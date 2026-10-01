impl<'ctx> HirCompiler<'ctx> {
    fn compile_call(
        &mut self,
        callee: &HirExpr,
        args: &[HirExpr],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let HirExpr::Var(name) = callee else {
            return self.compile_expression_call(callee, args);
        };

        if let Some(result) = self.compile_dynamic_named_call(name, args) {
            return result;
        }
        if let Some(result) = self.compile_json_named_call(name, args) {
            return result;
        }
        if let Some(result) = self.compile_array_named_call(name, args) {
            return result;
        }
        if let Some(result) = self.compile_text_named_call(name, args) {
            return result;
        }

        match name.as_str() {
            "__thaw_object_has_accessor" => {
                let [object, property, setter] = args else {
                    return Err(
                        "object accessor query expects an object, property, and kind".into(),
                    );
                };
                let object = self.compile_expr(object)?.into_pointer_value();
                let property = self.compile_expr(property)?.into_pointer_value();
                let setter = self.compile_expr(setter)?.into_int_value();
                let accessor = self
                    .builder
                    .build_call(
                        self.module.get_function("thaw_object_accessor").unwrap(),
                        &[object.into(), property.into(), setter.into()],
                        "object_accessor_query",
                    )
                    .map_err(|error| error.to_string())?
                    .try_as_basic_value()
                    .basic()
                    .ok_or("object accessor query returned no value")?
                    .into_pointer_value();
                return self
                    .builder
                    .build_is_not_null(accessor, "object_has_accessor")
                    .map(Into::into)
                    .map_err(|error| error.to_string());
            }
            "__thaw_object_set_state"
            | "__thaw_object_set_state_and_return"
            | "__thaw_object_state" => {
                let [object, operation] = args else {
                    return Err(format!("{name} expects an object and an operation"));
                };
                let object = self.compile_expr(object)?;
                if !object.is_pointer_value() {
                    return Err("object integrity methods require a native object".into());
                }
                let operation = self.compile_expr(operation)?.into_float_value();
                let operation = self
                    .builder
                    .build_float_to_unsigned_int(
                        operation,
                        self.context.i8_type(),
                        "object_state_operation",
                    )
                    .map_err(|error| error.to_string())?;
                let runtime = if name == "__thaw_object_set_state_and_return" {
                    "object_set_state"
                } else {
                    name.trim_start_matches("__thaw_")
                };
                let result = self
                    .builder
                    .build_call(
                        self.module.get_function(&format!("thaw_{runtime}")).unwrap(),
                        &[object.into(), operation.into()],
                        "object_state",
                    )
                    .map_err(|error| error.to_string())?
                    .try_as_basic_value()
                    .basic()
                    .ok_or("object state operation returned no value".into());
                if name == "__thaw_object_set_state_and_return" {
                    result?;
                    return Ok(object);
                }
                return result;
            }
            "__thaw_array_has_index" => {
                let [array, index] = args else {
                    return Err("array presence check expects two operands".into());
                };
                let handle = self.compile_expr(array)?.into_pointer_value();
                let index = self.compile_expr(index)?.into_float_value();
                let index = self
                    .builder
                    .build_float_to_unsigned_int(index, self.context.i64_type(), "array_presence_index")
                    .map_err(|error| error.to_string())?;
                return Ok(self.compile_array_has_index(handle, index)?.into());
            }
            "__thaw_array_has_property" | "__thaw_array_has_own"
            | "__thaw_array_property_is_enumerable" => {
                let [array, key] = args else {
                    return Err("array property check expects two operands".into());
                };
                let handle = self.compile_expr(array)?.into_pointer_value();
                let key = self.compile_expr(key)?;
                let data = self.compile_array_data(handle)?;
                let presence = self.compile_array_presence(handle)?;
                let found = self.builder.build_call(
                    self.module.get_function("thaw_array_has_property").unwrap(),
                    &[
                        data.into(), presence.into(), key.into(),
                        self.context.i8_type().const_int(match name.as_str() {
                            "__thaw_array_has_own" => 1,
                            "__thaw_array_property_is_enumerable" => 2,
                            _ => 0,
                        }, false).into(),
                    ],
                    "array_has_property",
                ).map_err(|error| error.to_string())?.try_as_basic_value().basic()
                    .ok_or("array property check returned no value")?.into_int_value();
                return self.builder.build_int_compare(
                    IntPredicate::NE, found, self.context.i8_type().const_zero(), "array_has_property_bool",
                ).map(Into::into).map_err(|error| error.to_string());
            }
            "__thaw_array_index_state" => {
                let [array, index] = args else {
                    return Err("array state lookup expects two operands".into());
                };
                let handle = self.compile_expr(array)?.into_pointer_value();
                let index = self.compile_expr(index)?.into_float_value();
                let index = self
                    .builder
                    .build_float_to_unsigned_int(index, self.context.i64_type(), "array_state_index")
                    .map_err(|error| error.to_string())?;
                let state = self.compile_array_index_state(handle, index)?;
                return self
                    .builder
                    .build_unsigned_int_to_float(state, self.context.f64_type(), "array_state_number")
                    .map(Into::into)
                    .map_err(|error| error.to_string());
            }
            "__thaw_array_densify" => {
                let [array] = args else {
                    return Err("array densify expects one operand".into());
                };
                let handle = self.compile_expr(array)?.into_pointer_value();
                let presence = self.compile_array_presence(handle)?;
                let presence = self
                    .builder
                    .build_call(
                        self.module.get_function("thaw_array_presence_densify").unwrap(),
                        &[presence.into()],
                        "array_densify",
                    )
                    .map_err(|error| error.to_string())?
                    .try_as_basic_value()
                    .basic()
                    .ok_or("array densify returned no value")?
                    .into_pointer_value();
                self.compile_array_set_presence(handle, presence)?;
                return Ok(handle.into());
            }
            "__thaw_array_copy_index_state" => {
                let [target, target_index, source, source_index] = args else {
                    return Err("array state copy expects four operands".into());
                };
                let target = self.compile_expr(target)?.into_pointer_value();
                let target_index = self.compile_expr(target_index)?.into_float_value();
                let source = self.compile_expr(source)?.into_pointer_value();
                let source_index = self.compile_expr(source_index)?.into_float_value();
                let i64_type = self.context.i64_type();
                let target_index = self.builder.build_float_to_unsigned_int(
                    target_index, i64_type, "target_state_index",
                ).map_err(|error| error.to_string())?;
                let source_index = self.builder.build_float_to_unsigned_int(
                    source_index, i64_type, "source_state_index",
                ).map_err(|error| error.to_string())?;
                let state = self.compile_array_index_state(source, source_index)?;
                let target_data = self.compile_array_data(target)?;
                let length = self.builder.build_load(i64_type, target_data, "state_target_length")
                    .map_err(|error| error.to_string())?;
                let presence = self.compile_array_presence(target)?;
                let new_presence = self.builder.build_call(
                    self.module.get_function("thaw_array_presence_set_state").unwrap(),
                    &[presence.into(), length.into(), target_index.into(), state.into()],
                    "copy_index_state",
                ).map_err(|error| error.to_string())?.try_as_basic_value().basic()
                    .ok_or("array state copy returned no value")?.into_pointer_value();
                self.compile_array_set_presence(target, new_presence)?;
                return Ok(target.into());
            }
            "__thaw_array_set_undefined" | "__thaw_array_set_hole" => {
                let [array, index] = args else {
                    return Err("array undefined write expects two operands".into());
                };
                let Some(HirType::Array(element)) = self.expr_hir_type(array) else {
                    return Err("array undefined write requires a homogeneous array".into());
                };
                let target = self.compile_expr(array)?.into_pointer_value();
                let index = self.compile_expr(index)?.into_float_value();
                let i64_type = self.context.i64_type();
                let slot = self.builder.build_call(
                    self.module.get_function("thaw_array_ensure_index").unwrap(),
                    &[target.into(), i64_type.const_int(array_element_storage_bytes(&element), false).into(), index.into()],
                    "undefined_write_slot",
                ).map_err(|error| error.to_string())?.try_as_basic_value().basic()
                    .ok_or("array undefined write returned no slot")?.into_pointer_value();
                let write = self.context.append_basic_block(self.current_function(), "array_state_write_valid");
                let done = self.context.append_basic_block(self.current_function(), "array_state_write_done");
                let valid = self.builder.build_is_not_null(slot, "array_state_write_in_range")
                    .map_err(|error| error.to_string())?;
                self.builder.build_conditional_branch(valid, write, done)
                    .map_err(|error| error.to_string())?;
                self.builder.position_at_end(write);
                // `thaw_array_presence_set_state` (below) only marks the
                // presence *state*; unlike a real value write, it never
                // touches the slot's own bytes. For a scalar element
                // (F64/Bool/...) that's fine -- "undefined" is encoded
                // entirely by the state byte, and every reader of this
                // shape already goes through a state-aware wrapper before
                // ever looking at the slot. But `Json` (and any other
                // pointer-based element) has readers that don't: ordinary
                // `arr[i]` indexing and `.map()`'s "callback type already
                // matches element type" fast path both read the slot's
                // raw pointer directly. Left at whatever the arena
                // allocation zero-initialized it to, that's a null
                // pointer, dereferenced by the very next consumer (e.g.
                // `typeof`) -- a real, reproducible segfault, not just a
                // wrong value. Write a genuine, safe `Json` "undefined"
                // sentinel into the slot so a raw read is memory-safe too.
                if name == "__thaw_array_set_undefined" && *element == HirType::Json {
                    let undefined = self.builder.build_call(
                        self.module.get_function("thaw_json_undefined").unwrap(),
                        &[],
                        "array_undefined_sentinel",
                    ).map_err(|error| error.to_string())?
                        .try_as_basic_value().basic()
                        .ok_or("thaw_json_undefined returned no value")?;
                    self.builder.build_store(slot, undefined).map_err(|error| error.to_string())?;
                }
                let index = self.builder.build_float_to_unsigned_int(
                    index, i64_type, "undefined_write_index",
                ).map_err(|error| error.to_string())?;
                let data = self.compile_array_data(target)?;
                let length = self.builder.build_load(i64_type, data, "undefined_write_length")
                    .map_err(|error| error.to_string())?;
                let presence = self.compile_array_presence(target)?;
                let presence = self.builder.build_call(
                    self.module.get_function("thaw_array_presence_set_state").unwrap(),
                    &[presence.into(), length.into(), index.into(), self.context.i8_type().const_int(u64::from(name == "__thaw_array_set_undefined") * 2, false).into()],
                    "write_undefined_state",
                ).map_err(|error| error.to_string())?.try_as_basic_value().basic()
                    .ok_or("undefined write returned no value")?.into_pointer_value();
                self.compile_array_set_presence(target, presence)?;
                self.builder.build_unconditional_branch(done).map_err(|error| error.to_string())?;
                self.builder.position_at_end(done);
                return Ok(target.into());
            }
            "__thaw_array_resize" => {
                let [array, length] = args else {
                    return Err("array resize expects two operands".into());
                };
                let Some(HirType::Array(element)) = self.expr_hir_type(array) else {
                    return Err("array resize requires a homogeneous array".into());
                };
                let handle = self.compile_expr(array)?.into_pointer_value();
                let length = self.compile_expr(length)?.into_float_value();
                self.builder.build_call(
                    self.module.get_function("thaw_array_resize").unwrap(),
                    &[
                        handle.into(),
                        self.context.i64_type().const_int(array_element_storage_bytes(&element), false).into(),
                        length.into(),
                    ],
                    "array_resize",
                ).map_err(|error| error.to_string())?;
                return Ok(handle.into());
            }
            "__thaw_array_undefined_index_of" => {
                let [array, from_index, reverse, kind, include_holes, union_tag] = args else {
                    return Err("undefined array search expects six operands".into());
                };
                let handle = self.compile_expr(array)?.into_pointer_value();
                let from_index = self.compile_expr(from_index)?;
                let reverse = self.compile_expr(reverse)?.into_int_value();
                let kind = self.compile_expr(kind)?.into_float_value();
                let include_holes = self.compile_expr(include_holes)?.into_int_value();
                let union_tag = self.compile_expr(union_tag)?.into_float_value();
                let data = self.compile_array_data(handle)?;
                let presence = self.compile_array_presence(handle)?;
                let reverse = self.builder.build_int_z_extend(
                    reverse, self.context.i8_type(), "array_search_reverse",
                ).map_err(|error| error.to_string())?;
                let kind = self.builder.build_float_to_unsigned_int(
                    kind, self.context.i8_type(), "array_search_element_kind",
                ).map_err(|error| error.to_string())?;
                let include_holes = self.builder.build_int_z_extend(
                    include_holes, self.context.i8_type(), "array_search_holes",
                ).map_err(|error| error.to_string())?;
                let union_tag = self.builder.build_float_to_unsigned_int(
                    union_tag, self.context.i8_type(), "array_search_union_tag",
                ).map_err(|error| error.to_string())?;
                let result = self.builder.build_call(
                    self.module.get_function("thaw_array_undefined_index_of").unwrap(),
                    &[data.into(), presence.into(), from_index.into(), reverse.into(), kind.into(), include_holes.into(), union_tag.into()],
                    "undefined_index_of",
                ).map_err(|error| error.to_string())?.try_as_basic_value().basic()
                    .ok_or("undefined array search returned no value")?;
                return Ok(result);
            }
            "__thaw_tagged_array_index_of" => {
                let [array, needle, from_index, reverse, includes] = args else {
                    return Err("tagged array search expects five operands".into());
                };
                let Some(HirType::Array(element)) = self.expr_hir_type(array) else {
                    return Err("tagged search requires an array".into());
                };
                let (payload, value_tag, union_layout) = match element.as_ref() {
                    HirType::Optional(payload) | HirType::Nullable(payload) => (payload.as_ref(), 1, 0),
                    HirType::Nullish(payload) => (payload.as_ref(), 0, 0),
                    HirType::Union(members) => {
                        let needle_type = self.expr_hir_type(needle).ok_or("tagged needle has no type")?;
                        let index = members.iter().position(|member| member == &needle_type)
                            .ok_or("tagged needle is not a union member")?;
                        (members.get(index).unwrap(), index as u64, 1)
                    }
                    _ => return Err("tagged search requires tagged elements".into()),
                };
                let kind = match payload {
                    HirType::F64 => 0,
                    HirType::Str => 1,
                    HirType::Bool => 2,
                    HirType::Object(_) => 3,
                    _ => return Err("tagged search does not support this payload type".into()),
                };
                let needle_type = self.basic_type(payload)?;
                let needle_slot = self.builder.build_alloca(needle_type, "tagged_search_needle")
                    .map_err(|error| error.to_string())?;
                let array = self.compile_expr(array)?.into_pointer_value();
                let needle = self.compile_expr(needle)?;
                self.builder.build_store(needle_slot, needle).map_err(|error| error.to_string())?;
                let from_index = self.compile_expr(from_index)?;
                let reverse = self.compile_expr(reverse)?.into_int_value();
                let includes = self.compile_expr(includes)?.into_int_value();
                let i8_type = self.context.i8_type();
                let reverse = self.builder.build_int_z_extend(reverse, i8_type, "tagged_search_reverse")
                    .map_err(|error| error.to_string())?;
                let includes = self.builder.build_int_z_extend(includes, i8_type, "tagged_search_includes")
                    .map_err(|error| error.to_string())?;
                let data = self.compile_array_data(array)?;
                let presence = self.compile_array_presence(array)?;
                return self.builder.build_call(
                    self.module.get_function("thaw_tagged_array_index_of").unwrap(),
                    &[
                        data.into(), presence.into(), needle_slot.into(), from_index.into(),
                        reverse.into(), includes.into(), i8_type.const_int(kind, false).into(),
                        i8_type.const_int(value_tag, false).into(),
                        i8_type.const_int(union_layout, false).into(),
                    ],
                    "tagged_index_of",
                ).map_err(|error| error.to_string())?.try_as_basic_value().basic()
                    .ok_or("tagged array search returned no value".into());
            }
            "__thaw_array_copy_presence" | "__thaw_array_map_presence" => {
                let [target, source] = args else {
                    return Err("array presence copy expects two operands".into());
                };
                let target = self.compile_expr(target)?.into_pointer_value();
                let source = self.compile_expr(source)?.into_pointer_value();
                let target = self.compile_array_copy_presence(target, source)?;
                if name == "__thaw_array_map_presence" {
                    let presence = self.compile_array_presence(target)?;
                    self.builder.build_call(
                        self.module.get_function("thaw_array_presence_mapped").unwrap(),
                        &[presence.into()],
                        "array_mapped_presence",
                    ).map_err(|error| error.to_string())?;
                }
                return Ok(target.into());
            }
            "__thaw_array_compact_for_sort" => {
                let [array] = args else {
                    return Err("array sort compaction expects one operand".into());
                };
                let Some(HirType::Array(element)) = self.expr_hir_type(array) else {
                    return Err("array sort compaction requires an array".into());
                };
                let handle = self.compile_expr(array)?.into_pointer_value();
                let buffer = self.compile_array_data(handle)?;
                let mut presence = self.compile_array_presence(handle)?;
                let undefined_tag = match element.as_ref() {
                    HirType::Optional(_) => Some((0, 1)),
                    HirType::Nullish(_) => Some((2, 1)),
                    HirType::Nullable(_) => Some((0, 0)),
                    HirType::Undefined => Some((0, 2)),
                    HirType::Union(members) => Some(members.iter()
                        .position(|member| member == &HirType::Undefined)
                        .map_or((0, 0), |index| (index as u64, 1))),
                    _ => None,
                };
                if let Some((tag, mode)) = undefined_tag {
                    presence = self.builder.build_call(
                        self.module.get_function("thaw_array_presence_tagged_sort").unwrap(),
                        &[
                            buffer.into(), presence.into(),
                            self.context.i8_type().const_int(tag, false).into(),
                            self.context.i8_type().const_int(mode, false).into(),
                        ],
                        "array_tagged_sort_presence",
                    ).map_err(|error| error.to_string())?.try_as_basic_value().basic()
                        .ok_or("tagged sort presence returned no value")?.into_pointer_value();
                    self.compile_array_set_presence(handle, presence)?;
                }
                let length = self
                    .builder
                    .build_call(
                        self.module
                            .get_function("thaw_array_presence_compact")
                            .unwrap(),
                        &[
                            buffer.into(),
                            presence.into(),
                            self.context.i64_type().const_int(array_element_storage_bytes(&element), false).into(),
                        ],
                        "array_sort_present_length",
                    )
                    .map_err(|error| error.to_string())?
                    .try_as_basic_value()
                    .basic()
                    .ok_or("array presence compaction returned no value")?
                    .into_int_value();
                return self
                    .builder
                    .build_unsigned_int_to_float(
                        length,
                        self.context.f64_type(),
                        "array_sort_present_length_number",
                    )
                    .map(Into::into)
                    .map_err(|error| error.to_string());
            }
            "__thaw_array_to_spliced_presence" => {
                let [target, source, start, delete_count, insert_count] = args else {
                    return Err("array toSpliced presence expects five operands".into());
                };
                let target = self.compile_expr(target)?.into_pointer_value();
                let source = self.compile_expr(source)?.into_pointer_value();
                let buffer = self.compile_array_data(source)?;
                let old_len = self
                    .builder
                    .build_load(self.context.i64_type(), buffer, "to_spliced_old_length")
                    .map_err(|error| error.to_string())?
                    .into_int_value();
                let presence = self.compile_array_presence(source)?;
                let removed = self
                    .builder
                    .build_alloca(self.context.ptr_type(AddressSpace::default()), "to_spliced_removed_presence")
                    .map_err(|error| error.to_string())?;
                let start = self.compile_expr(start)?;
                let delete_count = self.compile_expr(delete_count)?;
                let insert_count = self.compile_expr(insert_count)?.into_float_value();
                let insert_count = self
                    .builder
                    .build_float_to_unsigned_int(
                        insert_count,
                        self.context.i64_type(),
                        "to_spliced_insert_count",
                    )
                    .map_err(|error| error.to_string())?;
                let new_presence = self.builder.build_call(
                    self.module.get_function("thaw_array_presence_splice").unwrap(),
                    &[
                        presence.into(), old_len.into(), start.into(),
                        delete_count.into(), insert_count.into(),
                        self.context.ptr_type(AddressSpace::default()).const_null().into(),
                        removed.into(),
                    ],
                    "to_spliced_presence",
                ).map_err(|error| error.to_string())?.try_as_basic_value().basic().ok_or("array presence splice returned no value")?.into_pointer_value();
                self.compile_array_set_presence(target, new_presence)?;
                return Ok(target.into());
            }
            "console.log" | "console.info" | "console.debug" => {
                return self.compile_console_log(args, false)
            }
            "console.warn" | "console.error" => return self.compile_console_log(args, true),
            "console.assert" => return self.compile_console_assert(args),
            "__thaw_date_now" => {
                if !args.is_empty() {
                    return Err("Date.now expects no operands".into());
                }
                return self
                    .builder
                    .build_call(
                        self.module.get_function("thaw_date_now").unwrap(),
                        &[],
                        "date_now",
                    )
                    .map_err(|error| error.to_string())?
                    .try_as_basic_value()
                    .basic()
                    .ok_or("Date.now returned no value".into());
            }
            "__thaw_performance_now" => {
                if !args.is_empty() {
                    return Err("performance.now expects no operands".into());
                }
                return self
                    .builder
                    .build_call(
                        self.module.get_function("thaw_performance_now").unwrap(),
                        &[],
                        "performance_now",
                    )
                    .map_err(|error| error.to_string())?
                    .try_as_basic_value()
                    .basic()
                    .ok_or("performance.now returned no value".into());
            }
            "__thaw_date_get_full_year"
            | "__thaw_date_get_month"
            | "__thaw_date_get_date"
            | "__thaw_date_get_day"
            | "__thaw_date_get_hours"
            | "__thaw_date_get_minutes"
            | "__thaw_date_get_seconds"
            | "__thaw_date_get_milliseconds"
            | "__thaw_date_get_local_full_year"
            | "__thaw_date_get_local_month"
            | "__thaw_date_get_local_date"
            | "__thaw_date_get_local_day"
            | "__thaw_date_get_local_hours"
            | "__thaw_date_get_local_minutes"
            | "__thaw_date_get_local_seconds"
            | "__thaw_date_get_local_milliseconds"
            | "__thaw_date_get_month_for_full_year"
            | "__thaw_date_get_date_for_full_year"
            | "__thaw_date_get_local_month_for_full_year"
            | "__thaw_date_get_local_date_for_full_year"
            | "__thaw_date_time_clip"
            | "__thaw_date_get_timezone_offset" => {
                let [timestamp] = args else {
                    return Err(format!("{name} expects one operand"));
                };
                let timestamp = self.compile_expr(timestamp)?;
                let runtime = name.trim_start_matches("__thaw_").to_string();
                let runtime = format!("thaw_{runtime}");
                return self
                    .builder
                    .build_call(
                        self.module.get_function(&runtime).unwrap(),
                        &[timestamp.into()],
                        "date_get",
                    )
                    .map_err(|error| error.to_string())?
                    .try_as_basic_value()
                    .basic()
                    .ok_or("Date getter returned no value".into());
            }
            "__thaw_date_to_iso_string"
            | "__thaw_date_to_date_string"
            | "__thaw_date_to_time_string"
            | "__thaw_date_to_string"
            | "__thaw_date_to_utc_string" => {
                let [timestamp] = args else {
                    return Err(format!("{name} expects one operand"));
                };
                let timestamp = self.compile_expr(timestamp)?;
                let runtime = name.trim_start_matches("__thaw_").to_string();
                let runtime = format!("thaw_{runtime}");
                return self
                    .builder
                    .build_call(
                        self.module.get_function(&runtime).unwrap(),
                        &[timestamp.into()],
                        "date_to_string",
                    )
                    .map_err(|error| error.to_string())?
                    .try_as_basic_value()
                    .basic()
                    .ok_or("Date string conversion returned no value".into());
            }
            "__thaw_bigint_decimal_cmp" => {
                let [left, right] = args else {
                    return Err("bigint comparison expects two operands".into());
                };
                let left = self.compile_expr(left)?;
                let right = self.compile_expr(right)?;
                return self
                    .builder
                    .build_call(
                        self.module.get_function("thaw_bigint_decimal_cmp").unwrap(),
                        &[left.into(), right.into()],
                        "bigint_decimal_cmp",
                    )
                    .map_err(|error| error.to_string())?
                    .try_as_basic_value()
                    .basic()
                    .ok_or("bigint comparison returned no value".into());
            }
            "__thaw_bigint_decimal_add"
            | "__thaw_bigint_decimal_sub"
            | "__thaw_bigint_decimal_mul"
            | "__thaw_bigint_decimal_div"
            | "__thaw_bigint_decimal_mod"
            | "__thaw_bigint_decimal_and"
            | "__thaw_bigint_decimal_or"
            | "__thaw_bigint_decimal_xor"
            | "__thaw_bigint_decimal_shl"
            | "__thaw_bigint_decimal_shr" => {
                let [left, right] = args else {
                    return Err(format!("{name} expects two operands"));
                };
                let left = self.compile_expr(left)?;
                let right = self.compile_expr(right)?;
                let runtime = name.trim_start_matches("__thaw_").to_string();
                let runtime = format!("thaw_{runtime}");
                return self
                    .builder
                    .build_call(
                        self.module.get_function(&runtime).unwrap(),
                        &[left.into(), right.into()],
                        "bigint_decimal_arithmetic",
                    )
                    .map_err(|error| error.to_string())?
                    .try_as_basic_value()
                    .basic()
                    .ok_or("bigint arithmetic returned no value".into());
            }
            "__thaw_temporal_now" | "__thaw_temporal_now_nanos" => {
                if !args.is_empty() {
                    return Err("Temporal.Now expects no operands".into());
                }
                let runtime = name.trim_start_matches("__thaw_").to_string();
                let runtime = format!("thaw_{runtime}");
                return self
                    .builder
                    .build_call(self.module.get_function(&runtime).unwrap(), &[], "temporal_now")
                    .map_err(|error| error.to_string())?
                    .try_as_basic_value()
                    .basic()
                    .ok_or("Temporal.Now returned no value".into());
            }
            "__thaw_temporal_time_zone_id" => {
                if !args.is_empty() {
                    return Err("Temporal.Now.timeZoneId expects no operands".into());
                }
                return self
                    .builder
                    .build_call(
                        self.module
                            .get_function("thaw_temporal_time_zone_id")
                            .unwrap(),
                        &[],
                        "temporal_time_zone_id",
                    )
                    .map_err(|error| error.to_string())?
                    .try_as_basic_value()
                    .basic()
                    .ok_or("Temporal.Now.timeZoneId returned no value".into());
            }
            "__thaw_temporal_instant_from_string"
            | "__thaw_temporal_instant_nanos_from_string"
            | "__thaw_temporal_plain_date_time_from_string"
            | "__thaw_temporal_plain_date_time_nanos_from_string"
            | "__thaw_temporal_plain_time_from_string"
            | "__thaw_temporal_plain_time_nanos_from_string"
            | "__thaw_temporal_plain_month_day_from_string"
            | "__thaw_temporal_duration_from_string"
            | "__thaw_temporal_duration_nanos_from_string" => {
                let [value] = args else {
                    return Err(format!("{name} expects one operand"));
                };
                let value = self.compile_expr(value)?;
                let runtime = name.trim_start_matches("__thaw_").to_string();
                let runtime = format!("thaw_{runtime}");
                return self
                    .builder
                    .build_call(
                        self.module.get_function(&runtime).unwrap(),
                        &[value.into()],
                        "temporal_from_string",
                    )
                    .map_err(|error| error.to_string())?
                    .try_as_basic_value()
                    .basic()
                    .ok_or("Temporal conversion returned no value".into());
            }
            "__thaw_temporal_instant_to_string"
            | "__thaw_temporal_plain_date_to_string"
            | "__thaw_temporal_plain_date_time_to_string"
            | "__thaw_temporal_plain_time_to_string"
            | "__thaw_temporal_plain_year_month_to_string"
            | "__thaw_temporal_plain_month_day_to_string"
            | "__thaw_temporal_duration_to_string"
            | "__thaw_temporal_epoch_nanoseconds" => {
                let [timestamp, nanoseconds] = args else {
                    return Err(format!("{name} expects two operands"));
                };
                let timestamp = self.compile_expr(timestamp)?;
                let nanoseconds = self.compile_expr(nanoseconds)?;
                let runtime = name.trim_start_matches("__thaw_").to_string();
                let runtime = format!("thaw_{runtime}");
                return self
                    .builder
                    .build_call(
                        self.module.get_function(&runtime).unwrap(),
                        &[timestamp.into(), nanoseconds.into()],
                        "temporal_to_string",
                    )
                    .map_err(|error| error.to_string())?
                    .try_as_basic_value()
                    .basic()
                    .ok_or("Temporal string conversion returned no value".into());
            }
            "__thaw_temporal_compare" => {
                let [left_ms, left_ns, right_ms, right_ns] = args else {
                    return Err(format!("{name} expects four operands"));
                };
                let left_ms = self.compile_expr(left_ms)?;
                let left_ns = self.compile_expr(left_ns)?;
                let right_ms = self.compile_expr(right_ms)?;
                let right_ns = self.compile_expr(right_ns)?;
                return self
                    .builder
                    .build_call(
                        self.module.get_function("thaw_temporal_compare").unwrap(),
                        &[
                            left_ms.into(),
                            left_ns.into(),
                            right_ms.into(),
                            right_ns.into(),
                        ],
                        "temporal_compare",
                    )
                    .map_err(|error| error.to_string())?
                    .try_as_basic_value()
                    .basic()
                    .ok_or("Temporal comparison returned no value".into());
            }
            "__thaw_temporal_round" => {
                let [total, increment, mode] = args else {
                    return Err(format!("{name} expects three operands"));
                };
                let total = self.compile_expr(total)?;
                let increment = self.compile_expr(increment)?;
                let mode = self.compile_expr(mode)?;
                return self
                    .builder
                    .build_call(
                        self.module.get_function("thaw_temporal_round").unwrap(),
                        &[total.into(), increment.into(), mode.into()],
                        "temporal_round",
                    )
                    .map_err(|error| error.to_string())?
                    .try_as_basic_value()
                    .basic()
                    .ok_or("Temporal rounding returned no value".into());
            }
            "__thaw_temporal_zone_valid" => {
                let [zone] = args else {
                    return Err(format!("{name} expects one operand"));
                };
                let zone = self.compile_expr(zone)?;
                return self
                    .builder
                    .build_call(
                        self.module.get_function("thaw_temporal_zone_valid").unwrap(),
                        &[zone.into()],
                        "temporal_zone_valid",
                    )
                    .map_err(|error| error.to_string())?
                    .try_as_basic_value()
                    .basic()
                    .ok_or("Temporal zone check returned no value".into());
            }
            "__thaw_temporal_duration_components_json"
            | "__thaw_temporal_duration_to_string_components" => {
                let [value] = args else {
                    return Err(format!("{name} expects one operand"));
                };
                let value = self.compile_expr(value)?;
                let runtime = name.trim_start_matches("__thaw_").to_string();
                let runtime = format!("thaw_{runtime}");
                return self
                    .builder
                    .build_call(
                        self.module.get_function(&runtime).unwrap(),
                        &[value.into()],
                        "temporal_duration_components",
                    )
                    .map_err(|error| error.to_string())?
                    .try_as_basic_value()
                    .basic()
                    .ok_or("Temporal duration components returned no value".into());
            }
            "__thaw_temporal_zoned_from_string"
            | "__thaw_temporal_zoned_nanos_from_string" => {
                let [text] = args else {
                    return Err(format!("{name} expects one operand"));
                };
                let text = self.compile_expr(text)?;
                let runtime = name.trim_start_matches("__thaw_").to_string();
                let runtime = format!("thaw_{runtime}");
                return self
                    .builder
                    .build_call(
                        self.module.get_function(&runtime).unwrap(),
                        &[text.into()],
                        "temporal_zoned_from_string",
                    )
                    .map_err(|error| error.to_string())?
                    .try_as_basic_value()
                    .basic()
                    .ok_or("Temporal zoned parse returned no value".into());
            }
            "__thaw_temporal_zoned_zone_from_string" => {
                let [text] = args else {
                    return Err(format!("{name} expects one operand"));
                };
                let text = self.compile_expr(text)?;
                return self
                    .builder
                    .build_call(
                        self.module
                            .get_function("thaw_temporal_zoned_zone_from_string")
                            .unwrap(),
                        &[text.into()],
                        "temporal_zoned_zone_from_string",
                    )
                    .map_err(|error| error.to_string())?
                    .try_as_basic_value()
                    .basic()
                    .ok_or("Temporal zoned zone parse returned no value".into());
            }
            "__thaw_temporal_zoned_start_of_day" | "__thaw_temporal_zoned_hours_in_day" => {
                let [milliseconds, nanoseconds, zone] = args else {
                    return Err(format!("{name} expects three operands"));
                };
                let milliseconds = self.compile_expr(milliseconds)?;
                let nanoseconds = self.compile_expr(nanoseconds)?;
                let zone = self.compile_expr(zone)?;
                return self
                    .builder
                    .build_call(
                        self.module.get_function(name.trim_start_matches("__")).unwrap(),
                        &[milliseconds.into(), nanoseconds.into(), zone.into()],
                        "temporal_zoned_number",
                    )
                    .map_err(|error| error.to_string())?
                    .try_as_basic_value()
                    .basic()
                    .ok_or("Temporal start of day returned no value".into());
            }
            "__thaw_temporal_zoned_transition" => {
                let [milliseconds, nanoseconds, zone, direction, part] = args else {
                    return Err(format!("{name} expects five operands"));
                };
                let values = [milliseconds, nanoseconds, zone, direction, part]
                    .into_iter()
                    .map(|value| self.compile_expr(value).map(Into::into))
                    .collect::<Result<Vec<_>, _>>()?;
                return self
                    .builder
                    .build_call(
                        self.module
                            .get_function("thaw_temporal_zoned_transition")
                            .unwrap(),
                        &values,
                        "temporal_zoned_transition",
                    )
                    .map_err(|error| error.to_string())?
                    .try_as_basic_value()
                    .basic()
                    .ok_or("Temporal time zone transition returned no value".into());
            }
            "__thaw_temporal_with_fields" => {
                if args.len() != 14 {
                    return Err(format!("{name} expects fourteen operands"));
                }
                let values = args
                    .iter()
                    .map(|value| self.compile_expr(value).map(Into::into))
                    .collect::<Result<Vec<_>, _>>()?;
                let result = self
                    .builder
                    .build_call(
                        self.module.get_function("thaw_temporal_with_fields").unwrap(),
                        &values,
                        "temporal_with_fields",
                    )
                    .map_err(|error| error.to_string())?
                    .try_as_basic_value()
                    .basic()
                    .ok_or("Temporal field replacement returned no value")?;
                let value = result.into_float_value();
                let invalid = self.builder
                    .build_float_compare(inkwell::FloatPredicate::UNO, value, value, "temporal_invalid_fields")
                    .map_err(|error| error.to_string())?;
                let function = self.current_function();
                let error_bb = self.context.append_basic_block(function, "temporal_fields_error");
                let continue_bb = self.context.append_basic_block(function, "temporal_fields_ok");
                self.builder.build_conditional_branch(invalid, error_bb, continue_bb)
                    .map_err(|error| error.to_string())?;
                self.builder.position_at_end(error_bb);
                let error = self.builder
                    .build_global_string_ptr("\u{1}RangeError\u{1}Invalid Temporal field", "temporal_fields_range_error")
                    .map_err(|error| error.to_string())?
                    .as_pointer_value();
                self.builder.build_store(self.pending_exception().as_pointer_value(), error)
                    .map_err(|error| error.to_string())?;
                self.builder.build_unconditional_branch(continue_bb)
                    .map_err(|error| error.to_string())?;
                self.builder.position_at_end(continue_bb);
                self.branch_on_pending_exception()?;
                return Ok(value.into());
            }
            "__thaw_temporal_zoned_to_string" | "__thaw_temporal_zoned_offset" => {
                let [milliseconds, nanoseconds, zone] = args else {
                    return Err(format!("{name} expects three operands"));
                };
                let milliseconds = self.compile_expr(milliseconds)?;
                let nanoseconds = self.compile_expr(nanoseconds)?;
                let zone = self.compile_expr(zone)?;
                let runtime = name.trim_start_matches("__thaw_").to_string();
                let runtime = format!("thaw_{runtime}");
                return self
                    .builder
                    .build_call(
                        self.module.get_function(&runtime).unwrap(),
                        &[milliseconds.into(), nanoseconds.into(), zone.into()],
                        "temporal_zoned_to_string",
                    )
                    .map_err(|error| error.to_string())?
                    .try_as_basic_value()
                    .basic()
                    .ok_or("Temporal zoned string conversion returned no value".into());
            }
            "__thaw_temporal_zoned_field"
            | "__thaw_temporal_zoned_plain_timestamp"
            | "__thaw_temporal_plain_to_zoned" => {
                let [milliseconds, nanoseconds, zone, field] = args else {
                    return Err(format!("{name} expects four operands"));
                };
                let milliseconds = self.compile_expr(milliseconds)?;
                let nanoseconds = self.compile_expr(nanoseconds)?;
                let zone = self.compile_expr(zone)?;
                let field = self.compile_expr(field)?;
                let runtime = name.trim_start_matches("__thaw_").to_string();
                let runtime = format!("thaw_{runtime}");
                return self
                    .builder
                    .build_call(
                        self.module.get_function(&runtime).unwrap(),
                        &[
                            milliseconds.into(),
                            nanoseconds.into(),
                            zone.into(),
                            field.into(),
                        ],
                        "temporal_zoned_field",
                    )
                    .map_err(|error| error.to_string())?
                    .try_as_basic_value()
                    .basic()
                    .ok_or("Temporal zoned field returned no value".into());
            }
            "__thaw_temporal_plain_date_field" => {
                let [milliseconds, field] = args else {
                    return Err(format!("{name} expects two operands"));
                };
                let milliseconds = self.compile_expr(milliseconds)?;
                let field = self.compile_expr(field)?;
                return self
                    .builder
                    .build_call(
                        self.module
                            .get_function("thaw_temporal_plain_date_field")
                            .unwrap(),
                        &[milliseconds.into(), field.into()],
                        "temporal_plain_date_field",
                    )
                    .map_err(|error| error.to_string())?
                    .try_as_basic_value()
                    .basic()
                    .ok_or("Temporal plain-date field returned no value".into());
            }
            "__thaw_temporal_calendar_valid" => {
                let [calendar] = args else {
                    return Err(format!("{name} expects one operand"));
                };
                let calendar = self.compile_expr(calendar)?;
                return self
                    .builder
                    .build_call(
                        self.module
                            .get_function("thaw_temporal_calendar_valid")
                            .unwrap(),
                        &[calendar.into()],
                        "temporal_calendar_valid",
                    )
                    .map_err(|error| error.to_string())?
                    .try_as_basic_value()
                    .basic()
                    .ok_or("Temporal calendar check returned no value".into());
            }
            "__thaw_temporal_calendar_from_string" => {
                let [text] = args else {
                    return Err(format!("{name} expects one operand"));
                };
                let text = self.compile_expr(text)?;
                return self
                    .builder
                    .build_call(
                        self.module
                            .get_function("thaw_temporal_calendar_from_string")
                            .unwrap(),
                        &[text.into()],
                        "temporal_calendar_from_string",
                    )
                    .map_err(|error| error.to_string())?
                    .try_as_basic_value()
                    .basic()
                    .ok_or("Temporal calendar parse returned no value".into());
            }
            "__thaw_temporal_calendar_field" => {
                let [milliseconds, calendar, field] = args else {
                    return Err(format!("{name} expects three operands"));
                };
                let milliseconds = self.compile_expr(milliseconds)?;
                let calendar = self.compile_expr(calendar)?;
                let field = self.compile_expr(field)?;
                return self
                    .builder
                    .build_call(
                        self.module
                            .get_function("thaw_temporal_calendar_field")
                            .unwrap(),
                        &[milliseconds.into(), calendar.into(), field.into()],
                        "temporal_calendar_field",
                    )
                    .map_err(|error| error.to_string())?
                    .try_as_basic_value()
                    .basic()
                    .ok_or("Temporal calendar field returned no value".into());
            }
            "__thaw_temporal_date_difference" | "__thaw_temporal_duration_balance" => {
                let [first, second, unit] = args else {
                    return Err(format!("{name} expects three operands"));
                };
                let first = self.compile_expr(first)?;
                let second = self.compile_expr(second)?;
                let unit = self.compile_expr(unit)?;
                let runtime = name.trim_start_matches("__thaw_").to_string();
                let runtime = format!("thaw_{runtime}");
                return self
                    .builder
                    .build_call(
                        self.module.get_function(&runtime).unwrap(),
                        &[first.into(), second.into(), unit.into()],
                        "temporal_three_arg",
                    )
                    .map_err(|error| error.to_string())?
                    .try_as_basic_value()
                    .basic()
                    .ok_or("Temporal operation returned no value".into());
            }
            "__thaw_temporal_calendar_month_code" | "__thaw_temporal_calendar_era" => {
                let [milliseconds, calendar] = args else {
                    return Err(format!("{name} expects two operands"));
                };
                let milliseconds = self.compile_expr(milliseconds)?;
                let calendar = self.compile_expr(calendar)?;
                let runtime = name.trim_start_matches("__thaw_").to_string();
                let runtime = format!("thaw_{runtime}");
                return self
                    .builder
                    .build_call(
                        self.module.get_function(&runtime).unwrap(),
                        &[milliseconds.into(), calendar.into()],
                        "temporal_calendar_string",
                    )
                    .map_err(|error| error.to_string())?
                    .try_as_basic_value()
                    .basic()
                    .ok_or("Temporal calendar string returned no value".into());
            }
            "__thaw_temporal_month_code" => {
                let [milliseconds] = args else {
                    return Err(format!("{name} expects one operand"));
                };
                let milliseconds = self.compile_expr(milliseconds)?;
                return self
                    .builder
                    .build_call(
                        self.module.get_function("thaw_temporal_month_code").unwrap(),
                        &[milliseconds.into()],
                        "temporal_month_code",
                    )
                    .map_err(|error| error.to_string())?
                    .try_as_basic_value()
                    .basic()
                    .ok_or("Temporal month code returned no value".into());
            }
            "__thaw_temporal_shift" | "__thaw_temporal_duration_component" => {
                let [left, right] = args else {
                    return Err(format!("{name} expects two operands"));
                };
                let left = self.compile_expr(left)?;
                let right = self.compile_expr(right)?;
                let runtime = name.trim_start_matches("__thaw_").to_string();
                let runtime = format!("thaw_{runtime}");
                return self
                    .builder
                    .build_call(
                        self.module.get_function(&runtime).unwrap(),
                        &[left.into(), right.into()],
                        "temporal_binary",
                    )
                    .map_err(|error| error.to_string())?
                    .try_as_basic_value()
                    .basic()
                    .ok_or("Temporal operation returned no value".into());
            }
            "__thaw_error_message"
            | "__thaw_error_name"
            | "__thaw_error_cause"
            | "__thaw_error_code"
            | "__thaw_error_stack"
            | "__thaw_error_suppressed_error"
            | "__thaw_error_suppressed"
            | "__thaw_error_is_error"
            | "__thaw_error_to_string" => {
                let [value] = args else {
                    return Err(format!("{name} expects one operand"));
                };
                let value = self.compile_expr(value)?;
                let runtime = name.trim_start_matches("__thaw_").to_string();
                let runtime = format!("thaw_{runtime}");
                return self
                    .builder
                    .build_call(
                        self.module.get_function(&runtime).unwrap(),
                        &[value.into()],
                        "error_property",
                    )
                    .map_err(|error| error.to_string())?
                    .try_as_basic_value()
                    .basic()
                    .ok_or("error property access returned no value".into());
            }
            "__thaw_error_suppress" => {
                let [error, suppressed, message] = args else {
                    return Err(format!("{name} expects three operands"));
                };
                let values = [
                    self.compile_expr(error)?.into(),
                    self.compile_expr(suppressed)?.into(),
                    self.compile_expr(message)?.into(),
                ];
                return self
                    .builder
                    .build_call(
                        self.module.get_function("thaw_error_suppress").unwrap(),
                        &values,
                        "suppressed_error",
                    )
                    .map_err(|error| error.to_string())?
                    .try_as_basic_value()
                    .basic()
                    .ok_or("SuppressedError construction returned no value".into());
            }
            "__thaw_error_property" => {
                let [receiver, key] = args else {
                    return Err("__thaw_error_property expects a receiver and a property name".into());
                };
                let receiver = self.compile_expr(receiver)?;
                let key = self.compile_expr(key)?;
                return self
                    .builder
                    .build_call(
                        self.module.get_function("thaw_error_property").unwrap(),
                        &[receiver.into(), key.into()],
                        "error_custom_property",
                    )
                    .map_err(|error| error.to_string())?
                    .try_as_basic_value()
                    .basic()
                    .ok_or("error custom property access returned no value".into());
            }
            "__thaw_i64_to_string" => {
                let [value] = args else {
                    return Err("bigint string conversion expects one operand".into());
                };
                let value = self.compile_expr(value)?;
                return self
                    .builder
                    .build_call(
                        self.module.get_function("thaw_i64_to_string").unwrap(),
                        &[value.into()],
                        "bigint_string",
                    )
                    .map_err(|error| error.to_string())?
                    .try_as_basic_value()
                    .basic()
                    .ok_or("bigint string conversion returned no value".into());
            }
            "__thaw_i64_from_number" | "__thaw_i64_from_string" => {
                let [value] = args else {
                    return Err(format!("{name} expects one operand"));
                };
                let value = self.compile_expr(value)?;
                let runtime = name.trim_start_matches("__thaw_").to_string();
                return self
                    .builder
                    .build_call(
                        self.module.get_function(&format!("thaw_{runtime}")).unwrap(),
                        &[value.into()],
                        "bigint_conversion",
                    )
                    .map_err(|error| error.to_string())?
                    .try_as_basic_value()
                    .basic()
                    .ok_or("bigint conversion returned no value".into());
            }
            "__thaw_i64_as_int_n"
            | "__thaw_i64_as_uint_n"
            | "__thaw_i64_to_radix_string" => {
                let [value, bits] = args else {
                    return Err(format!("{name} expects two operands"));
                };
                let value = self.compile_expr(value)?;
                let bits = self.compile_expr(bits)?;
                let runtime = name.trim_start_matches("__thaw_").to_string();
                return self
                    .builder
                    .build_call(
                        self.module.get_function(&format!("thaw_{runtime}")).unwrap(),
                        &[value.into(), bits.into()],
                        "bigint_radix",
                    )
                    .map_err(|error| error.to_string())?
                    .try_as_basic_value()
                    .basic()
                    .ok_or("bigint radix conversion returned no value".into());
            }
            "__thaw_symbol_new"
            | "__thaw_symbol_to_string"
            | "__thaw_symbol_key"
            | "__thaw_symbol_for"
            | "__thaw_symbol_key_for"
            | "__thaw_symbol_description" => {
                let [value] = args else {
                    return Err(format!("{name} expects one operand"));
                };
                let value = self.compile_expr(value)?;
                let runtime = name.trim_start_matches("__thaw_");
                return self
                    .builder
                    .build_call(
                        self.module.get_function(&format!("thaw_{runtime}")).unwrap(),
                        &[value.into()],
                        "symbol_call",
                    )
                    .map_err(|error| error.to_string())?
                    .try_as_basic_value()
                    .basic()
                    .ok_or("symbol call returned no value".into());
            }
            "__thaw_js_handle_to_string" => {
                let [value] = args else {
                    return Err("JsValue string conversion expects one operand".into());
                };
                let value = self.compile_expr(value)?;
                return self
                    .builder
                    .build_call(
                        self.module
                            .get_function("thaw_js_handle_to_string")
                            .unwrap(),
                        &[value.into()],
                        "js_value_to_string",
                    )
                    .map_err(|error| error.to_string())?
                    .try_as_basic_value()
                    .basic()
                    .ok_or("JsValue string conversion returned no value".into());
            }
            "__thaw_error_is_instance" => {
                let [value, class_name] = args else {
                    return Err(format!("{name} expects two operands"));
                };
                let value = self.compile_expr(value)?;
                let class_name = self.compile_expr(class_name)?;
                return self
                    .builder
                    .build_call(
                        self.module.get_function("thaw_error_is_instance").unwrap(),
                        &[value.into(), class_name.into()],
                        "error_is_instance",
                    )
                    .map_err(|error| error.to_string())?
                    .try_as_basic_value()
                    .basic()
                    .ok_or("error instanceof check returned no value".into());
            }
            "__thaw_set_pending_exception_object" => {
                let [value] = args else {
                    return Err(format!("{name} expects one operand"));
                };
                let value = self.compile_expr(value)?;
                self.builder
                    .build_store(self.pending_exception_object().as_pointer_value(), value)
                    .map_err(|error| error.to_string())?;
                self.builder
                    .build_store(
                        self.pending_exception_value(PENDING_EXCEPTION_VALUE_TAG_SYMBOL)
                            .as_pointer_value(),
                        self.context.i64_type().const_zero(),
                    )
                    .map_err(|error| error.to_string())?;
                return Ok(self.context.i32_type().const_int(0, false).into());
            }
            "__thaw_pending_exception_object" => {
                return self
                    .builder
                    .build_load(
                        self.context.ptr_type(AddressSpace::default()),
                        self.pending_exception_object().as_pointer_value(),
                        "pending_exception_object_value",
                    )
                    .map_err(|error| error.to_string());
            }
            "__thaw_exception_object_present" => {
                let [value] = args else {
                    return Err(format!("{name} expects one operand"));
                };
                let value = self.compile_expr(value)?.into_pointer_value();
                return self
                    .builder
                    .build_is_not_null(value, "exception_object_present")
                    .map(Into::into)
                    .map_err(|error| error.to_string());
            }
            "__thaw_pending_exception_tag"
            | "__thaw_pending_exception_f64"
            | "__thaw_pending_exception_i64"
            | "__thaw_pending_exception_bool" => {
                let (symbol, ty): (&str, BasicTypeEnum) = match name.as_str() {
                    "__thaw_pending_exception_tag" => (
                        PENDING_EXCEPTION_VALUE_TAG_SYMBOL,
                        self.context.i64_type().into(),
                    ),
                    "__thaw_pending_exception_f64" => {
                        (PENDING_EXCEPTION_F64_SYMBOL, self.context.f64_type().into())
                    }
                    "__thaw_pending_exception_i64" => {
                        (PENDING_EXCEPTION_I64_SYMBOL, self.context.i64_type().into())
                    }
                    _ => (PENDING_EXCEPTION_BOOL_SYMBOL, self.context.bool_type().into()),
                };
                return self
                    .builder
                    .build_load(
                        ty,
                        self.pending_exception_value(symbol).as_pointer_value(),
                        "pending_exception_value",
                    )
                    .map_err(|error| error.to_string());
            }
            "__thaw_set_pending_exception_f64"
            | "__thaw_set_pending_exception_i64"
            | "__thaw_set_pending_exception_bool" => {
                let [value] = args else {
                    return Err(format!("{name} expects one operand"));
                };
                let (symbol, tag) = match name.as_str() {
                    "__thaw_set_pending_exception_f64" => (PENDING_EXCEPTION_F64_SYMBOL, 1),
                    "__thaw_set_pending_exception_i64" => (PENDING_EXCEPTION_I64_SYMBOL, 2),
                    _ => (PENDING_EXCEPTION_BOOL_SYMBOL, 3),
                };
                let value = self.compile_expr(value)?;
                self.builder
                    .build_store(
                        self.pending_exception_value(symbol).as_pointer_value(),
                        value,
                    )
                    .map_err(|error| error.to_string())?;
                self.builder
                    .build_store(
                        self.pending_exception_value(PENDING_EXCEPTION_VALUE_TAG_SYMBOL)
                            .as_pointer_value(),
                        self.context.i64_type().const_int(tag, false),
                    )
                    .map_err(|error| error.to_string())?;
                return Ok(self.context.i32_type().const_int(0, false).into());
            }
            "__thaw_exception_typeof" => {
                let [tag] = args else {
                    return Err("exception typeof expects one tag".into());
                };
                let tag = self.compile_expr(tag)?.into_int_value();
                let choices = [
                    (1, "number"),
                    (2, "bigint"),
                    (3, "boolean"),
                    (4, "string"),
                    (5, "undefined"),
                ];
                let mut result = self.compile_expr(&HirExpr::Lit(HirLit::Str("object".into())))?;
                for (expected, label) in choices.into_iter().rev() {
                    let matches = self
                        .builder
                        .build_int_compare(
                            IntPredicate::EQ,
                            tag,
                            self.context.i64_type().const_int(expected, false),
                            "exception_type_matches",
                        )
                        .map_err(|error| error.to_string())?;
                    let label = self.compile_expr(&HirExpr::Lit(HirLit::Str(label.into())))?;
                    result = self
                        .builder
                        .build_select(matches, label, result, "exception_typeof")
                        .map_err(|error| error.to_string())?;
                }
                return Ok(result);
            }
            "__thaw_set_pending_exception_tag" => {
                let [tag] = args else {
                    return Err("exception tag setter expects one tag".into());
                };
                let tag = self.compile_expr(tag)?;
                self.builder
                    .build_store(
                        self.pending_exception_value(PENDING_EXCEPTION_VALUE_TAG_SYMBOL)
                            .as_pointer_value(),
                        tag,
                    )
                    .map_err(|error| error.to_string())?;
                return Ok(self.context.i32_type().const_int(0, false).into());
            }
            "__thaw_detach_promise" | "__thaw_detach_rejection" => {
                let [promise] = args else {
                    return Err("detach Promise expects one operand".into());
                };
                let promise = self.compile_expr(promise)?;
                let pending = if name == "__thaw_detach_rejection" {
                    self.pending_rejection()
                } else {
                    self.pending_exception()
                };
                return self
                    .builder
                    .build_call(
                        self.module.get_function("thaw_promise_detach").unwrap(),
                        &[
                            promise.into(),
                            pending.as_pointer_value().into(),
                        ],
                        "detach_promise",
                    )
                    .map_err(|error| error.to_string())?
                    .try_as_basic_value()
                    .basic()
                    .ok_or("thaw_promise_detach returned no value".into());
            }
            "__thaw_date_set_full_year"
            | "__thaw_date_set_local_full_year"
            | "__thaw_date_set_month"
            | "__thaw_date_set_local_month"
            | "__thaw_date_set_date"
            | "__thaw_date_set_local_date"
            | "__thaw_date_set_hours"
            | "__thaw_date_set_local_hours"
            | "__thaw_date_set_minutes"
            | "__thaw_date_set_local_minutes"
            | "__thaw_date_set_seconds"
            | "__thaw_date_set_local_seconds"
            | "__thaw_date_set_milliseconds"
            | "__thaw_date_set_local_milliseconds"
            | "__thaw_date_local"
            | "__thaw_date_utc" => {
                let mut compiled = Vec::with_capacity(args.len());
                for argument in args {
                    compiled.push(self.compile_expr(argument)?.into());
                }
                let runtime = name.trim_start_matches("__thaw_").to_string();
                let runtime = format!("thaw_{runtime}");
                return self
                    .builder
                    .build_call(
                        self.module.get_function(&runtime).unwrap(),
                        &compiled,
                        "date_set",
                    )
                    .map_err(|error| error.to_string())?
                    .try_as_basic_value()
                    .basic()
                    .ok_or("Date setter returned no value".into());
            }
            "__thaw_date_parse" => {
                let [text] = args else {
                    return Err("Date.parse expects one operand".into());
                };
                let text = self.compile_expr(text)?;
                return self
                    .builder
                    .build_call(
                        self.module.get_function("thaw_date_parse").unwrap(),
                        &[text.into()],
                        "date_parse",
                    )
                    .map_err(|error| error.to_string())?
                    .try_as_basic_value()
                    .basic()
                    .ok_or("Date.parse returned no value".into());
            }
            "__thaw_map_new" => {
                if !args.is_empty() {
                    return Err("Map/Set construction expects no operands".into());
                }
                return self
                    .builder
                    .build_call(
                        self.module.get_function("thaw_map_new").unwrap(),
                        &[],
                        "map_new",
                    )
                    .map_err(|error| error.to_string())?
                    .try_as_basic_value()
                    .basic()
                    .ok_or("Map/Set construction returned no value".into());
            }
            "__thaw_map_snapshot_keys"
            | "__thaw_map_snapshot_values"
            | "__thaw_map_snapshot_entries"
            | "__thaw_set_snapshot_entries" => {
                let [map] = args else {
                    return Err(format!("{name} expects one operand"));
                };
                let map = self.compile_expr(map)?;
                let runtime = name.trim_start_matches("__thaw_").to_string();
                let runtime = format!("thaw_{runtime}");
                let result = self
                    .builder
                    .build_call(
                        self.module.get_function(&runtime).unwrap(),
                        &[map.into()],
                        "map_snapshot",
                    )
                    .map_err(|error| error.to_string())?
                    .try_as_basic_value()
                    .basic()
                    .ok_or_else(|| format!("{name} returned no value"))?
                    .into_pointer_value();
                return Ok(self.compile_array_wrap(result)?.into());
            }
            "__thaw_map_iterator_next" => {
                let [map, cursor, mode] = args else {
                    return Err("map iterator next expects three operands".into());
                };
                let map = self.compile_expr(map)?;
                let cursor = self.compile_expr(cursor)?;
                let mode = self.compile_expr(mode)?;
                let result = self
                    .builder
                    .build_call(
                        self.module.get_function("thaw_map_iterator_next").unwrap(),
                        &[map.into(), cursor.into(), mode.into()],
                        "map_iterator_next",
                    )
                    .map_err(|error| error.to_string())?
                    .try_as_basic_value()
                    .basic()
                    .ok_or("map iterator next returned no value")?
                    .into_pointer_value();
                return Ok(self.compile_array_wrap(result)?.into());
            }
            "__thaw_map_size" => {
                let [map] = args else {
                    return Err("Map/Set size expects one operand".into());
                };
                let map = self.compile_expr(map)?;
                return self
                    .builder
                    .build_call(
                        self.module.get_function("thaw_map_size").unwrap(),
                        &[map.into()],
                        "map_size",
                    )
                    .map_err(|error| error.to_string())?
                    .try_as_basic_value()
                    .basic()
                    .ok_or("Map/Set size returned no value".into());
            }
            "__thaw_map_clear" => {
                let [map] = args else {
                    return Err("Map/Set clear expects one operand".into());
                };
                let map = self.compile_expr(map)?;
                self.builder
                    .build_call(
                        self.module.get_function("thaw_map_clear").unwrap(),
                        &[map.into()],
                        "map_clear",
                    )
                    .map_err(|error| error.to_string())?;
                return Ok(self.context.f64_type().const_zero().into());
            }
            "__thaw_map_num_has"
            | "__thaw_map_num_delete"
            | "__thaw_map_str_has"
            | "__thaw_map_str_delete"
            | "__thaw_map_ref_has"
            | "__thaw_map_ref_delete"
            | "__thaw_map_any_has"
            | "__thaw_map_any_delete" => {
                let [map, key] = args else {
                    return Err(format!("{name} expects two operands"));
                };
                let map = self.compile_expr(map)?;
                let key = self.compile_expr(key)?;
                let runtime = name.trim_start_matches("__thaw_").to_string();
                let runtime = format!("thaw_{runtime}");
                let result = self
                    .builder
                    .build_call(
                        self.module.get_function(&runtime).unwrap(),
                        &[map.into(), key.into()],
                        "map_bool_op",
                    )
                    .map_err(|error| error.to_string())?
                    .try_as_basic_value()
                    .basic()
                    .ok_or_else(|| format!("{name} returned no value"))?
                    .into_int_value();
                return self
                    .builder
                    .build_int_compare(
                        IntPredicate::NE,
                        result,
                        self.context.i8_type().const_zero(),
                        "map_bool_result",
                    )
                    .map(Into::into)
                    .map_err(|error| error.to_string());
            }
            "__thaw_map_num_get_f64"
            | "__thaw_map_num_get_bool"
            | "__thaw_map_num_get_ptr"
            | "__thaw_map_num_get_i64"
            | "__thaw_map_str_get_f64"
            | "__thaw_map_str_get_bool"
            | "__thaw_map_str_get_ptr"
            | "__thaw_map_str_get_i64"
            | "__thaw_map_ref_get_f64"
            | "__thaw_map_ref_get_bool"
            | "__thaw_map_ref_get_ptr"
            | "__thaw_map_ref_get_i64"
            | "__thaw_map_any_get_f64"
            | "__thaw_map_any_get_bool"
            | "__thaw_map_any_get_ptr"
            | "__thaw_map_any_get_i64" => {
                let [map, key] = args else {
                    return Err(format!("{name} expects two operands"));
                };
                let map = self.compile_expr(map)?;
                let key = self.compile_expr(key)?;
                let runtime = name.trim_start_matches("__thaw_").to_string();
                let runtime = format!("thaw_{runtime}");
                let result = self
                    .builder
                    .build_call(
                        self.module.get_function(&runtime).unwrap(),
                        &[map.into(), key.into()],
                        "map_get",
                    )
                    .map_err(|error| error.to_string())?
                    .try_as_basic_value()
                    .basic()
                    .ok_or_else(|| format!("{name} returned no value"))?;
                if name.ends_with("_get_bool") {
                    return self
                        .builder
                        .build_int_compare(
                            IntPredicate::NE,
                            result.into_int_value(),
                            self.context.i8_type().const_zero(),
                            "map_get_bool_result",
                        )
                        .map(Into::into)
                        .map_err(|error| error.to_string());
                }
                return Ok(result);
            }
            "__thaw_map_num_set"
            | "__thaw_map_str_set"
            | "__thaw_map_ref_set"
            | "__thaw_map_any_set" => {
                let [map, key, value] = args else {
                    return Err(format!("{name} expects three operands"));
                };
                let map = self.compile_expr(map)?;
                let key = self.compile_expr(key)?;
                let value = self.compile_expr(value)?;
                let word = self.encode_word(value)?;
                let runtime = name.trim_start_matches("__thaw_").to_string();
                let runtime = format!("thaw_{runtime}");
                self.builder
                    .build_call(
                        self.module.get_function(&runtime).unwrap(),
                        &[map.into(), key.into(), word.into()],
                        "map_set",
                    )
                    .map_err(|error| error.to_string())?;
                return Ok(map);
            }
            "__thaw_object_to_string" => return self.compile_object_to_string(args),
            "__thaw_number_is_nan" => return self.compile_number_predicate(args, false),
            "__thaw_number_is_finite" => return self.compile_number_predicate(args, true),
            "__thaw_number_is_integer" => return self.compile_integer_predicate(args, false),
            "__thaw_number_is_safe_integer" => return self.compile_integer_predicate(args, true),
            "__thaw_number_object_is" => {
                let [left, right] = args else {
                    return Err("Object.is number comparison expects two operands".to_string());
                };
                let left = self.compile_expr(left)?;
                let right = self.compile_expr(right)?;
                let result = self
                    .builder
                    .build_call(
                        self.module.get_function("thaw_number_object_is").unwrap(),
                        &[left.into(), right.into()],
                        "number_object_is",
                    )
                    .map_err(|error| error.to_string())?
                    .try_as_basic_value()
                    .basic()
                    .ok_or("Object.is number comparison returned no value")?
                    .into_int_value();
                return self
                    .builder
                    .build_int_compare(
                        IntPredicate::NE,
                        result,
                        self.context.i8_type().const_zero(),
                        "number_object_is_bool",
                    )
                    .map(Into::into)
                    .map_err(|error| error.to_string());
            }
            "__thaw_number_neg" => {
                let [value] = args else {
                    return Err("unary minus expects one operand".to_string());
                };
                let value = self.compile_expr(value)?.into_float_value();
                return self
                    .builder
                    .build_float_neg(value, "number_neg")
                    .map(Into::into)
                    .map_err(|error| error.to_string());
            }
            "__thaw_math_abs" => {
                return self.compile_single_arg_call("llvm.fabs.f64", args, "Math.abs")
            }
            "__thaw_math_floor" => {
                return self.compile_single_arg_call("llvm.floor.f64", args, "Math.floor")
            }
            "__thaw_math_ceil" => {
                return self.compile_single_arg_call("llvm.ceil.f64", args, "Math.ceil")
            }
            "__thaw_math_trunc" => {
                return self.compile_single_arg_call("llvm.trunc.f64", args, "Math.trunc")
            }
            "__thaw_math_sqrt" => {
                return self.compile_single_arg_call("llvm.sqrt.f64", args, "Math.sqrt")
            }
            "__thaw_math_exp" | "__thaw_math_log" | "__thaw_math_log2" | "__thaw_math_log10"
            | "__thaw_math_sin" | "__thaw_math_cos" => {
                let operation = name.trim_start_matches("__thaw_math_");
                let intrinsic = format!("llvm.{operation}.f64");
                return self.compile_single_arg_call(
                    &intrinsic,
                    args,
                    &format!("Math.{operation}"),
                );
            }
            "__thaw_math_tan" | "__thaw_math_asin" | "__thaw_math_acos" | "__thaw_math_atan"
            | "__thaw_math_sinh" | "__thaw_math_cosh" | "__thaw_math_tanh" | "__thaw_math_cbrt"
            | "__thaw_math_acosh" | "__thaw_math_asinh" | "__thaw_math_atanh"
            | "__thaw_math_expm1" | "__thaw_math_log1p" => {
                let operation = name.trim_start_matches("__thaw_math_");
                return self.compile_single_arg_call(operation, args, &format!("Math.{operation}"));
            }
            "__thaw_math_fround" | "__thaw_math_clz32" => {
                let operation = name.trim_start_matches("__thaw_math_");
                return self.compile_single_arg_call(
                    &format!("thaw_math_{operation}"),
                    args,
                    &format!("Math.{operation}"),
                );
            }
            "__thaw_math_sum_precise" => {
                let [array] = args else {
                    return Err("Math.sumPrecise expects one operand".to_string());
                };
                let handle = self.compile_expr(array)?.into_pointer_value();
                let buffer = self.compile_array_data(handle)?;
                return self
                    .builder
                    .build_call(
                        self.module.get_function("thaw_math_sum_precise").unwrap(),
                        &[buffer.into()],
                        "math_sum_precise",
                    )
                    .map_err(|error| error.to_string())?
                    .try_as_basic_value()
                    .basic()
                    .ok_or("Math.sumPrecise returned no value".into());
            }
            "__thaw_math_random" => {
                if !args.is_empty() {
                    return Err("Math.random expects no operands".to_string());
                }
                return self
                    .builder
                    .build_call(
                        self.module.get_function("thaw_math_random").unwrap(),
                        &[],
                        "math_random",
                    )
                    .map_err(|error| error.to_string())?
                    .try_as_basic_value()
                    .basic()
                    .ok_or("Math.random returned no value".to_string());
            }
            "__thaw_math_pow" => {
                let [left, right] = args else {
                    return Err("Math.pow expects two operands".to_string());
                };
                let left = self.compile_expr(left)?;
                let right = self.compile_expr(right)?;
                return self
                    .builder
                    .build_call(
                        self.module.get_function("pow").unwrap(),
                        &[left.into(), right.into()],
                        "math_pow",
                    )
                    .map_err(|error| error.to_string())?
                    .try_as_basic_value()
                    .basic()
                    .ok_or("pow returned no value".to_string());
            }
            "__thaw_math_atan2" => {
                let [left, right] = args else {
                    return Err("Math.atan2 expects two operands".to_string());
                };
                let left = self.compile_expr(left)?;
                let right = self.compile_expr(right)?;
                return self
                    .builder
                    .build_call(
                        self.module.get_function("atan2").unwrap(),
                        &[left.into(), right.into()],
                        "math_atan2",
                    )
                    .map_err(|error| error.to_string())?
                    .try_as_basic_value()
                    .basic()
                    .ok_or("atan2 returned no value".to_string());
            }
            "__thaw_math_imul" => {
                let [left, right] = args else {
                    return Err("Math.imul expects two operands".to_string());
                };
                let left = self.compile_expr(left)?;
                let right = self.compile_expr(right)?;
                return self
                    .builder
                    .build_call(
                        self.module.get_function("thaw_math_imul").unwrap(),
                        &[left.into(), right.into()],
                        "math_imul",
                    )
                    .map_err(|error| error.to_string())?
                    .try_as_basic_value()
                    .basic()
                    .ok_or("Math.imul returned no value".to_string());
            }
            "__thaw_math_min" => return self.compile_math_extreme(args, true),
            "__thaw_math_max" => return self.compile_math_extreme(args, false),
            "__thaw_math_hypot" => return self.compile_math_hypot(args),
            "__thaw_math_sign" => return self.compile_math_sign(args),
            "__thaw_math_round" => return self.compile_math_round(args),
            "fetch"
                if self
                    .module
                    .get_function(&Self::llvm_symbol_for(name))
                    .is_none() =>
            {
                return self.compile_single_arg_call("thaw_fetch_get", args, "fetch");
            }
            "sleep" => return self.compile_sleep(args),
            _ => {}
        }

        if let Some(result) = self.compile_variable_call(name, callee, args) {
            return result;
        }

        let symbol = Self::llvm_symbol_for(name);
        let function = self
            .module
            .get_function(&symbol)
            .ok_or_else(|| format!("call to undeclared function `{name}`"))?;

        self.build_call_with(function, args, name)
    }

}
