impl<'ctx> HirCompiler<'ctx> {
    fn compile_typed_dynamic_call(
        &mut self,
        signature: &DynamicSignature,
        args: &[HirExpr],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        if signature.backend == DynamicBackend::QuickJs {
            self.uses_quickjs = true;
        }
        if signature.backend == DynamicBackend::Napi {
            self.uses_napi = true;
        }
        if signature.backend == DynamicBackend::Napi
            && (signature.symbol.starts_with("$getter$")
                || signature.symbol.starts_with("$staticgetter$"))
        {
            return self.compile_typed_napi_getter(signature, args);
        }
        if signature.backend == DynamicBackend::Napi
            && (signature.symbol.starts_with("$setter$")
                || signature.symbol.starts_with("$staticsetter$"))
        {
            return self.compile_typed_napi_setter(signature, args);
        }
        if (signature.backend == DynamicBackend::Napi || signature.backend == DynamicBackend::QuickJs)
            && (signature.symbol.starts_with("$method$")
                || signature.symbol.starts_with("$methodvoid$")
                || signature.symbol.starts_with("$staticmethod$")
                || signature.symbol.starts_with("$staticmethodvoid$"))
        {
            return self.compile_typed_napi_method(signature, args);
        }
        if signature.backend == DynamicBackend::Jit {
            let return_type = match &signature.ret {
                HirType::Optional(payload)
                | HirType::Nullable(payload)
                | HirType::Nullish(payload)
                    if matches!(payload.as_ref(), HirType::F64 | HirType::Bool | HirType::Str)
                        || matches!(payload.as_ref(), HirType::Array(element) if jit_array_result_element_supported(element))
                        || matches!(payload.as_ref(), HirType::Dictionary(element) if matches!(element.as_ref(), HirType::F64 | HirType::Bool | HirType::Str))
                        || matches!(payload.as_ref(), HirType::Union(elements) if jit_tagged_union(elements)) =>
                {
                    payload.as_ref()
                }
                ty @ (HirType::F64 | HirType::Bool | HirType::Str) => ty,
                ty @ HirType::Union(elements)
                    if jit_tagged_union(elements) => ty,
                ty @ HirType::Array(element)
                    if jit_array_result_element_supported(element) => ty,
                ty @ HirType::Dictionary(element)
                    if matches!(element.as_ref(), HirType::F64 | HirType::Bool | HirType::Str) => ty,
                _ => {
                    return Err(format!(
                        "JIT calls currently return supported primitives, tagged unions, arrays, dictionaries, or optional supported values, not {:?}",
                        signature.ret
                    ));
                }
            };
            let argument_slots = signature
                .params
                .iter()
                .map(jit_parameter_slots)
                .collect::<Option<Vec<_>>>()
                .ok_or("JIT calls require supported primitive, tagged union, array, dictionary, object, or tuple arguments")?
                .into_iter()
                .sum::<usize>();
            if argument_slots > 16
                || args.len() != signature.params.len()
            {
                return Err(
                    "JIT calls currently require supported arguments fitting 16 ABI slots"
                        .into(),
                );
            }
            let name = self
                .builder
                .build_global_string_ptr(&signature.symbol, "jit_symbol")
                .map_err(|error| error.to_string())?;
            let argument_storage = self
                .builder
                .build_alloca(
                    self.context
                        .i8_type()
                        .array_type((argument_slots * 8) as u32),
                    "jit_arguments",
                )
                .map_err(|error| error.to_string())?;
            let mut argument_values = Vec::with_capacity(argument_slots);
            for (index, argument) in args.iter().enumerate() {
                let value = self.compile_expr(argument)?;
                if let HirType::Optional(payload)
                | HirType::Nullable(payload)
                | HirType::Nullish(payload) = &signature.params[index]
                {
                    let value = value.into_struct_value();
                    let tag = self
                        .builder
                        .build_extract_value(value, 0, "jit_tagged_argument_tag")
                        .map_err(|error| error.to_string())?
                        .into_int_value();
                    let present = if matches!(&signature.params[index], HirType::Nullish(_)) {
                        self.builder
                            .build_int_compare(
                                inkwell::IntPredicate::EQ,
                                tag,
                                tag.get_type().const_zero(),
                                "jit_nullish_argument_present",
                            )
                            .map_err(|error| error.to_string())?
                    } else {
                        tag
                    };
                    argument_values.push(
                        self.builder
                            .build_unsigned_int_to_float(
                                tag,
                                self.context.f64_type(),
                                "jit_tagged_argument_tag_slot",
                            )
                            .map_err(|error| error.to_string())?
                            .into(),
                    );
                    let payload_value = self
                        .builder
                        .build_extract_value(value, 1, "jit_tagged_argument_payload")
                        .map_err(|error| error.to_string())?;
                    if matches!(payload.as_ref(), HirType::Object(_) | HirType::Tuple(_)) {
                        let function = self.current_function();
                        let present_block = self
                            .context
                            .append_basic_block(function, "jit_optional_aggregate_present");
                        let absent_block = self
                            .context
                            .append_basic_block(function, "jit_optional_aggregate_absent");
                        let merge_block = self
                            .context
                            .append_basic_block(function, "jit_optional_aggregate_merge");
                        self.builder
                            .build_conditional_branch(present, present_block, absent_block)
                            .map_err(|error| error.to_string())?;

                        self.builder.position_at_end(present_block);
                        let mut present_values = Vec::new();
                        self.compile_jit_argument_slots(
                            payload_value,
                            payload,
                            &format!("jit_optional_argument_{index}"),
                            &mut present_values,
                        )?;
                        let present_end = self.builder.get_insert_block().unwrap();
                        self.builder
                            .build_unconditional_branch(merge_block)
                            .map_err(|error| error.to_string())?;

                        self.builder.position_at_end(absent_block);
                        let absent_values = present_values
                            .iter()
                            .map(|value| match value {
                                BasicValueEnum::FloatValue(value) => {
                                    Ok(value.get_type().const_zero().into())
                                }
                                BasicValueEnum::IntValue(value) => {
                                    Ok(value.get_type().const_zero().into())
                                }
                                BasicValueEnum::PointerValue(value) => {
                                    Ok(value.get_type().const_null().into())
                                }
                                _ => Err("JIT aggregate slots require scalar or pointer leaves"),
                            })
                            .collect::<Result<Vec<BasicValueEnum<'ctx>>, _>>()?;
                        let absent_end = self.builder.get_insert_block().unwrap();
                        self.builder
                            .build_unconditional_branch(merge_block)
                            .map_err(|error| error.to_string())?;

                        self.builder.position_at_end(merge_block);
                        for (present_value, absent_value) in
                            present_values.iter().zip(&absent_values)
                        {
                            let phi = self
                                .builder
                                .build_phi(present_value.get_type(), "jit_optional_aggregate_slot")
                                .map_err(|error| error.to_string())?;
                            phi.add_incoming(&[
                                (present_value, present_end),
                                (absent_value, absent_end),
                            ]);
                            argument_values.push(phi.as_basic_value());
                        }
                        continue;
                    }
                    self.compile_jit_argument_slots(
                        payload_value,
                        payload,
                        &format!("jit_optional_argument_{index}"),
                        &mut argument_values,
                    )?;
                    continue;
                }
                self.compile_jit_argument_slots(
                    value,
                    &signature.params[index],
                    &format!("jit_argument_{index}"),
                    &mut argument_values,
                )?;
            }
            for (index, value) in argument_values.into_iter().enumerate() {
                let slot = unsafe {
                    self.builder
                        .build_in_bounds_gep(
                            self.context.i8_type(),
                            argument_storage,
                            &[self.context.i64_type().const_int((index * 8) as u64, false)],
                            "jit_argument",
                        )
                        .map_err(|error| error.to_string())?
                };
                self.builder
                    .build_store(slot, value)
                    .map_err(|error| error.to_string())?;
            }
            let result = self
                .builder
                .build_call(
                    self.module.get_function("thaw_jit_call_f64").unwrap(),
                    &[
                        name.as_pointer_value().into(),
                        argument_storage.into(),
                        self.context
                            .i64_type()
                            .const_int(argument_slots as u64, false)
                            .into(),
                        self.module
                            .get_function("thaw_arena_alloc")
                            .unwrap()
                            .as_global_value()
                            .as_pointer_value()
                            .into(),
                        self.module
                            .get_function("thaw_number_to_string")
                            .unwrap()
                            .as_global_value()
                            .as_pointer_value()
                            .into(),
                        self.module
                            .get_function("thaw_string_to_number")
                            .unwrap()
                            .as_global_value()
                            .as_pointer_value()
                            .into(),
                        self.module
                            .get_function("thaw_parse_float")
                            .unwrap()
                            .as_global_value()
                            .as_pointer_value()
                            .into(),
                        self.module
                            .get_function("thaw_parse_int")
                            .unwrap()
                            .as_global_value()
                            .as_pointer_value()
                            .into(),
                        self.module
                            .get_function("thaw_jit_format_number")
                            .unwrap()
                            .as_global_value()
                            .as_pointer_value()
                            .into(),
                        self.module
                            .get_function("thaw_jit_array_search")
                            .unwrap()
                            .as_global_value()
                            .as_pointer_value()
                            .into(),
                        self.module
                            .get_function("thaw_jit_array_format")
                            .unwrap()
                            .as_global_value()
                            .as_pointer_value()
                            .into(),
                        self.module
                            .get_function("thaw_string_normalize")
                            .unwrap()
                            .as_global_value()
                            .as_pointer_value()
                            .into(),
                        self.module
                            .get_function("thaw_string_split")
                            .unwrap()
                            .as_global_value()
                            .as_pointer_value()
                            .into(),
                        self.module
                            .get_function("thaw_array_slice")
                            .unwrap()
                            .as_global_value()
                            .as_pointer_value()
                            .into(),
                        self.module
                            .get_function("thaw_array_concat")
                            .unwrap()
                            .as_global_value()
                            .as_pointer_value()
                            .into(),
                        self.module
                            .get_function("thaw_jit_array_append")
                            .unwrap()
                            .as_global_value()
                            .as_pointer_value()
                            .into(),
                        self.module
                            .get_function("thaw_array_to_reversed")
                            .unwrap()
                            .as_global_value()
                            .as_pointer_value()
                            .into(),
                        self.module
                            .get_function("thaw_jit_array_to_sorted")
                            .unwrap()
                            .as_global_value()
                            .as_pointer_value()
                            .into(),
                        self.module
                            .get_function("thaw_array_reverse")
                            .unwrap()
                            .as_global_value()
                            .as_pointer_value()
                            .into(),
                        self.module
                            .get_function("thaw_jit_array_sort")
                            .unwrap()
                            .as_global_value()
                            .as_pointer_value()
                            .into(),
                        self.module
                            .get_function("thaw_jit_array_fill")
                            .unwrap()
                            .as_global_value()
                            .as_pointer_value()
                            .into(),
                        self.module
                            .get_function("thaw_array_copy_within")
                            .unwrap()
                            .as_global_value()
                            .as_pointer_value()
                            .into(),
                        self.module
                            .get_function("thaw_jit_array_push")
                            .unwrap()
                            .as_global_value()
                            .as_pointer_value()
                            .into(),
                        self.module
                            .get_function("thaw_jit_array_unshift")
                            .unwrap()
                            .as_global_value()
                            .as_pointer_value()
                            .into(),
                        self.module
                            .get_function("thaw_jit_array_remove")
                            .unwrap()
                            .as_global_value()
                            .as_pointer_value()
                            .into(),
                        self.module
                            .get_function("thaw_jit_array_splice")
                            .unwrap()
                            .as_global_value()
                            .as_pointer_value()
                            .into(),
                        self.module
                            .get_function("thaw_jit_array_set")
                            .unwrap()
                            .as_global_value()
                            .as_pointer_value()
                            .into(),
                        self.module
                            .get_function("thaw_jit_array_with")
                            .unwrap()
                            .as_global_value()
                            .as_pointer_value()
                            .into(),
                        self.module
                            .get_function("thaw_math_random")
                            .unwrap()
                            .as_global_value()
                            .as_pointer_value()
                            .into(),
                        self.module
                            .get_function("thaw_date_now")
                            .unwrap()
                            .as_global_value()
                            .as_pointer_value()
                            .into(),
                        self.module
                            .get_function("thaw_performance_now")
                            .unwrap()
                            .as_global_value()
                            .as_pointer_value()
                            .into(),
                        self.module
                            .get_function("thaw_process_pid")
                            .unwrap()
                            .as_global_value()
                            .as_pointer_value()
                            .into(),
                        self.module
                            .get_function("thaw_process_ppid")
                            .unwrap()
                            .as_global_value()
                            .as_pointer_value()
                            .into(),
                        self.module
                            .get_function("thaw_string_to_array")
                            .unwrap()
                            .as_global_value()
                            .as_pointer_value()
                            .into(),
                        self.module
                            .get_function("thaw_string_from_char_code")
                            .unwrap()
                            .as_global_value()
                            .as_pointer_value()
                            .into(),
                        self.module
                            .get_function("thaw_string_from_code_point")
                            .unwrap()
                            .as_global_value()
                            .as_pointer_value()
                            .into(),
                        self.module
                            .get_function("thaw_jit_dictionary_get")
                            .unwrap()
                            .as_global_value()
                            .as_pointer_value()
                            .into(),
                        self.module
                            .get_function("thaw_jit_dictionary_mutate")
                            .unwrap()
                            .as_global_value()
                            .as_pointer_value()
                            .into(),
                        self.module
                            .get_function("thaw_jit_dictionary_query")
                            .unwrap()
                            .as_global_value()
                            .as_pointer_value()
                            .into(),
                    ],
                    "jit_numeric_result",
                )
                .map_err(|error| error.to_string())?
                .try_as_basic_value()
                .basic()
                .unwrap()
                .into_struct_value();
            let value = self
                .builder
                .build_extract_value(result, 0, "jit_numeric_value")
                .map_err(|error| error.to_string())?;
            let error = self
                .builder
                .build_extract_value(result, 1, "jit_numeric_error")
                .map_err(|error| error.to_string())?;
            let error = error.into_pointer_value();
            let status = self
                .builder
                .build_ptr_to_int(
                    error,
                    self.context.i64_type(),
                    "jit_status",
                )
                .map_err(|error| error.to_string())?;
            let undefined = self
                .builder
                .build_int_compare(
                    inkwell::IntPredicate::EQ,
                    status,
                    self.context.i64_type().const_int(1, false),
                    "jit_value_absent",
                )
                .map_err(|error| error.to_string())?;
            let null = self
                .builder
                .build_int_compare(
                    inkwell::IntPredicate::EQ,
                    status,
                    self.context.i64_type().const_int(2, false),
                    "jit_value_null",
                )
                .map_err(|error| error.to_string())?;
            let absent = self
                .builder
                .build_or(undefined, null, "jit_value_nullish")
                .map_err(|error| error.to_string())?;
            let error = self
                .builder
                .build_select(
                    absent,
                    self.context.ptr_type(inkwell::AddressSpace::default()).const_null(),
                    error,
                    "jit_error_without_absence_tag",
                )
                .map_err(|error| error.to_string())?;
            self.builder
                .build_store(self.pending_exception().as_pointer_value(), error)
                .map_err(|error| error.to_string())?;
            self.branch_on_pending_exception()?;
            let value = if *return_type == HirType::Bool {
                self.builder
                    .build_float_compare(
                        FloatPredicate::ONE,
                        value.into_float_value(),
                        self.context.f64_type().const_zero(),
                        "jit_boolean_value",
                    )
                    .map(BasicValueEnum::from)
                    .map_err(|error| error.to_string())?
            } else if let HirType::Union(elements) = return_type {
                if !jit_tagged_union(elements) {
                    return Err(format!(
                        "JIT tagged result has unsupported members {return_type:?}"
                    ));
                }
                let bits = self
                    .builder
                    .build_bit_cast(
                        value.into_float_value(),
                        self.context.i64_type(),
                        "jit_dynamic_bits",
                    )
                    .map_err(|error| error.to_string())?
                    .into_int_value();
                let pointer = self
                    .builder
                    .build_int_to_ptr(
                        bits,
                        self.context.ptr_type(inkwell::AddressSpace::default()),
                        "jit_dynamic_value",
                    )
                    .map_err(|error| error.to_string())?;
                let pointer = if matches!(
                    signature.ret,
                    HirType::Optional(_) | HirType::Nullable(_) | HirType::Nullish(_)
                ) {
                    let storage_type = self.context.i64_type().array_type(2);
                    let absent_storage = self
                        .builder
                        .build_alloca(storage_type, "jit_absent_dynamic_value")
                        .map_err(|error| error.to_string())?;
                    self.builder
                        .build_store(absent_storage, storage_type.const_zero())
                        .map_err(|error| error.to_string())?;
                    self.builder
                        .build_select(
                            absent,
                            absent_storage,
                            pointer,
                            "jit_present_dynamic_value",
                        )
                        .map_err(|error| error.to_string())?
                        .into_pointer_value()
                } else {
                    pointer
                };
                let payload_pointer = unsafe {
                    self.builder
                        .build_in_bounds_gep(
                            self.context.i64_type(),
                            pointer,
                            &[self.context.i64_type().const_int(1, false)],
                            "jit_dynamic_payload_pointer",
                        )
                        .map_err(|error| error.to_string())?
                };
                let runtime_tag = self
                    .builder
                    .build_load(self.context.i64_type(), pointer, "jit_dynamic_tag")
                    .map_err(|error| error.to_string())?
                    .into_int_value();
                let mut tag = self.context.i8_type().const_zero();
                for (member, ty) in elements.iter().enumerate() {
                    let runtime = jit_union_member_tag(ty).unwrap();
                    let selected = self
                        .builder
                        .build_int_compare(
                            inkwell::IntPredicate::EQ,
                            runtime_tag,
                            self.context.i64_type().const_int(runtime, false),
                            &format!("jit_dynamic_is_{runtime}"),
                        )
                        .map_err(|error| error.to_string())?;
                    tag = self
                        .builder
                        .build_select(
                            selected,
                            self.context.i8_type().const_int(member as u64, false),
                            tag,
                            "jit_union_tag",
                        )
                        .map_err(|error| error.to_string())?
                        .into_int_value();
                }
                let payload = self
                    .builder
                    .build_load(
                        self.context.i64_type(),
                        payload_pointer,
                        "jit_union_payload",
                    )
                    .map_err(|error| error.to_string())?;
                let union_type = self.basic_type(return_type)?.into_struct_type();
                let union = self
                    .builder
                    .build_insert_value(union_type.get_undef(), tag, 0, "jit_union_with_tag")
                    .map_err(|error| error.to_string())?
                    .into_struct_value();
                self.builder
                    .build_insert_value(union, payload, 1, "jit_union_with_payload")
                    .map_err(|error| error.to_string())?
                    .into_struct_value()
                    .into()
            } else if matches!(
                return_type,
                HirType::Str | HirType::Array(_) | HirType::Dictionary(_)
            ) {
                let bits = self
                    .builder
                    .build_bit_cast(
                        value.into_float_value(),
                        self.context.i64_type(),
                        "jit_string_bits",
                    )
                    .map_err(|error| error.to_string())?
                    .into_int_value();
                let pointer = self.builder
                    .build_int_to_ptr(
                        bits,
                        self.context.ptr_type(inkwell::AddressSpace::default()),
                        "jit_string_value",
                    )
                    .map_err(|error| error.to_string())?;
                if matches!(return_type, HirType::Array(_)) {
                    self.compile_array_wrap(pointer)?.into()
                } else {
                    pointer.into()
                }
            } else {
                value
            };
            if let HirType::Optional(payload) | HirType::Nullable(payload) = &signature.ret {
                let tagged_type = self.basic_type(&signature.ret)?.into_struct_type();
                let present = self
                    .builder
                    .build_not(absent, "jit_optional_present")
                    .map_err(|error| error.to_string())?;
                let tagged = self
                    .builder
                    .build_insert_value(tagged_type.get_undef(), present, 0, "jit_optional_tag")
                    .map_err(|error| error.to_string())?
                    .into_struct_value();
                return self
                    .builder
                    .build_insert_value(tagged, value, 1, "jit_optional_payload")
                    .map(|value| value.into_struct_value().into())
                    .map_err(|error| format!("JIT optional {payload:?} result: {error}"));
            }
            if let HirType::Nullish(payload) = &signature.ret {
                let tagged_type = self.basic_type(&signature.ret)?.into_struct_type();
                let absent_tag = self
                    .builder
                    .build_select(
                        null,
                        self.context.i8_type().const_int(1, false),
                        self.context.i8_type().const_int(2, false),
                        "jit_nullish_absent_tag",
                    )
                    .map_err(|error| error.to_string())?
                    .into_int_value();
                let tag = self
                    .builder
                    .build_select(
                        absent,
                        absent_tag,
                        self.context.i8_type().const_zero(),
                        "jit_nullish_tag",
                    )
                    .map_err(|error| error.to_string())?;
                let tagged = self
                    .builder
                    .build_insert_value(tagged_type.get_undef(), tag, 0, "jit_nullish_with_tag")
                    .map_err(|error| error.to_string())?
                    .into_struct_value();
                return self
                    .builder
                    .build_insert_value(tagged, value, 1, "jit_nullish_with_payload")
                    .map(|value| value.into_struct_value().into())
                    .map_err(|error| format!("JIT nullish {payload:?} result: {error}"));
            }
            return Ok(value);
        }
        let function_argument = signature
            .params
            .iter()
            .take(args.len())
            .enumerate()
            .filter_map(|(index, ty)| match ty {
                HirType::Function(params, ret) => {
                    Some((index, params.clone(), ret.as_ref().clone(), false, None))
                }
                HirType::CallableFunction(params, _, rest, ret) => {
                    let mut abi_params = params.clone();
                    let rest_start = rest.as_ref().map(|_| abi_params.len());
                    if let Some(rest) = rest {
                        abi_params.push(HirType::Array(rest.clone()));
                    }
                    Some((index, abi_params, ret.as_ref().clone(), false, rest_start))
                }
                HirType::Optional(inner) => match inner.as_ref() {
                    HirType::Function(params, ret) => {
                        Some((index, params.clone(), ret.as_ref().clone(), true, None))
                    }
                    HirType::CallableFunction(params, _, rest, ret) => {
                        let mut abi_params = params.clone();
                        let rest_start = rest.as_ref().map(|_| abi_params.len());
                        if let Some(rest) = rest {
                            abi_params.push(HirType::Array(rest.clone()));
                        }
                        Some((index, abi_params, ret.as_ref().clone(), true, rest_start))
                    }
                    _ => None,
                },
                _ => None,
            })
            .collect::<Vec<_>>();
        let array = self
            .builder
            .build_call(
                self.module.get_function("thaw_json_array_new").unwrap(),
                &[],
                "dynamic_args",
            )
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .unwrap();
        // Saved and restored around the loop (rather than just cleared
        // afterward) because `compile_expr(arg)` below can itself recurse
        // into another dynamic call (e.g. zod's `optional(string())`,
        // where `string()`'s own call is compiled while marshaling
        // `optional`'s argument list) -- without this, that nested call
        // (or one for an unrelated backend) would leave the flag in
        // whatever state its own call left it, rather than this call's.
        // See `compile_dynamic_value_placeholder` (json_bridge.rs), which
        // reads the flag while marshaling each argument below.
        let outer_compiling_quickjs_dynamic_arguments = self.compiling_quickjs_dynamic_arguments;
        self.compiling_quickjs_dynamic_arguments = signature.backend == DynamicBackend::QuickJs;
        for (index, (arg, ty)) in args.iter().zip(&signature.params).enumerate() {
            if signature.backend == DynamicBackend::Napi
                && function_argument
                    .iter()
                    .any(|(function_index, ..)| *function_index == index)
            {
                continue;
            }
            let (value, marshalled_type) = if signature.backend == DynamicBackend::QuickJs
                && quickjs_callback_type(ty)
            {
                (
                    self.compile_register_native_callback(std::slice::from_ref(arg))?,
                    &HirType::JsValue,
                )
            } else {
                (self.compile_expr(arg)?, ty)
            };
            self.compile_typed_dynamic_argument(array, value, marshalled_type)
                .map_err(|error| {
                    format!("typed dynamic argument {} ({ty:?}): {error}", index + 1)
                })?;
        }
        self.compiling_quickjs_dynamic_arguments = outer_compiling_quickjs_dynamic_arguments;
        let name = self
            .builder
            .build_global_string_ptr(&signature.symbol, "dynamic_symbol")
            .map_err(|error| error.to_string())?;
        if signature.backend == DynamicBackend::Napi && signature.ret == HirType::JsValue {
            let Some(constructor_name) = napi_constructor_export_name(&signature.symbol) else {
                let args_json = self
                    .builder
                    .build_call(
                        self.module.get_function("thaw_json_stringify").unwrap(),
                        &[array.into()],
                        "napi_handle_args",
                    )
                    .map_err(|error| error.to_string())?
                    .try_as_basic_value()
                    .basic()
                    .unwrap();
                let result = if !function_argument.is_empty() {
                    let functions =
                        self.compile_napi_function_arguments(args, &function_argument)?;
                    self.builder.build_call(
                        self.module
                            .get_function(
                                "thaw_napi_call_export_handle_with_functions_typed_result",
                            )
                            .unwrap(),
                        &[
                            name.as_pointer_value().into(),
                            args_json.into(),
                            functions.into(),
                            self.context
                                .i64_type()
                                .const_int(function_argument.len() as u64, false)
                                .into(),
                        ],
                        "napi_export_function_handle_result",
                    )
                } else {
                    self.builder.build_call(
                        self.module
                            .get_function("thaw_napi_call_export_handle_typed_result")
                            .unwrap(),
                        &[name.as_pointer_value().into(), args_json.into()],
                        "napi_export_handle_result",
                    )
                }
                    .map_err(|error| error.to_string())?
                    .try_as_basic_value()
                    .basic()
                    .unwrap()
                    .into_struct_value();
                let value = self
                    .builder
                    .build_extract_value(result, 0, "napi_export_handle")
                    .map_err(|error| error.to_string())?;
                let error = self
                    .builder
                    .build_extract_value(result, 1, "napi_export_handle_error")
                    .map_err(|error| error.to_string())?;
                self.builder
                    .build_store(self.pending_exception().as_pointer_value(), error)
                    .map_err(|error| error.to_string())?;
                self.branch_on_pending_exception()?;
                return Ok(value);
            };
            let constructor_name = self
                .builder
                .build_global_string_ptr(constructor_name, "napi_constructor_name")
                .map_err(|error| error.to_string())?;
            let constructor = self
                .builder
                .build_call(
                    self.module.get_function("thaw_napi_get_export").unwrap(),
                    &[constructor_name.as_pointer_value().into()],
                    "napi_constructor",
                )
                .map_err(|error| error.to_string())?
                .try_as_basic_value()
                .basic()
                .unwrap();
            let args_json = self
                .builder
                .build_call(
                    self.module.get_function("thaw_json_stringify").unwrap(),
                    &[array.into()],
                    "napi_constructor_args",
                )
                .map_err(|error| error.to_string())?
                .try_as_basic_value()
                .basic()
                .unwrap();
            let result = self
                .builder
                .build_call(
                    self.module
                        .get_function("thaw_napi_construct_handle_typed_result")
                        .unwrap(),
                    &[constructor.into(), args_json.into()],
                    "napi_construct_result",
                )
                .map_err(|error| error.to_string())?
                .try_as_basic_value()
                .basic()
                .unwrap()
                .into_struct_value();
            let value = self
                .builder
                .build_extract_value(result, 0, "napi_constructed_value")
                .map_err(|error| error.to_string())?;
            let error = self
                .builder
                .build_extract_value(result, 1, "napi_construct_error")
                .map_err(|error| error.to_string())?;
            self.builder
                .build_store(self.pending_exception().as_pointer_value(), error)
                .map_err(|error| error.to_string())?;
            self.branch_on_pending_exception()?;
            return Ok(value);
        }
        if signature.backend == DynamicBackend::QuickJs && signature.ret == HirType::JsValue {
            self.uses_quickjs_handles = true;
            // `napi_constructor_export_name` is plain `"$new$Class..."`
            // runtime-key parsing, not actually NAPI-specific despite the
            // name -- reused here for a Fallback (QuickJS-NG) class's own
            // `new Class(...)` construction (real example: hono's
            // `Hono`), the same way the `DynamicBackend::Napi` branch
            // above uses it for a native addon's.
            if let Some(constructor_name) = napi_constructor_export_name(&signature.symbol) {
                let constructor_name = self
                    .builder
                    .build_global_string_ptr(constructor_name, "typed_dynamic_constructor_name")
                    .map_err(|error| error.to_string())?;
                let constructor = self
                    .builder
                    .build_call(
                        self.module.get_function("thaw_js_get_global").unwrap(),
                        &[constructor_name.as_pointer_value().into()],
                        "typed_dynamic_constructor",
                    )
                    .map_err(|error| error.to_string())?
                    .try_as_basic_value()
                    .basic()
                    .unwrap();
                let args_json = self
                    .builder
                    .build_call(
                        self.module.get_function("thaw_json_stringify").unwrap(),
                        &[array.into()],
                        "typed_dynamic_construct_args",
                    )
                    .map_err(|error| error.to_string())?
                    .try_as_basic_value()
                    .basic()
                    .unwrap();
                let result = self
                    .builder
                    .build_call(
                        self.module
                            .get_function("thaw_js_construct_handle_result")
                            .unwrap(),
                        &[constructor.into(), args_json.into()],
                        "typed_dynamic_construct_result",
                    )
                    .map_err(|error| error.to_string())?
                    .try_as_basic_value()
                    .basic()
                    .unwrap()
                    .into_struct_value();
                let value = self
                    .builder
                    .build_extract_value(result, 0, "typed_dynamic_constructed_value")
                    .map_err(|error| error.to_string())?;
                let error = self
                    .builder
                    .build_extract_value(result, 1, "typed_dynamic_construct_error")
                    .map_err(|error| error.to_string())?;
                self.builder
                    .build_store(self.pending_exception().as_pointer_value(), error)
                    .map_err(|error| error.to_string())?;
                self.branch_on_pending_exception()?;
                return Ok(value);
            }
            let callable = self
                .builder
                .build_call(
                    self.module.get_function("thaw_js_get_global").unwrap(),
                    &[name.as_pointer_value().into()],
                    "typed_dynamic_callable",
                )
                .map_err(|error| error.to_string())?
                .try_as_basic_value()
                .basic()
                .unwrap();
            let args_json = self
                .builder
                .build_call(
                    self.module.get_function("thaw_json_stringify").unwrap(),
                    &[array.into()],
                    "typed_dynamic_callable_args",
                )
                .map_err(|error| error.to_string())?
                .try_as_basic_value()
                .basic()
                .unwrap();
            let result = self
                .builder
                .build_call(
                    self.module
                        .get_function("thaw_js_call_handle_handle_result")
                        .unwrap(),
                    &[callable.into(), args_json.into()],
                    "typed_dynamic_callable_result",
                )
                .map_err(|error| error.to_string())?
                .try_as_basic_value()
                .basic()
                .unwrap()
                .into_struct_value();
            let value = self
                .builder
                .build_extract_value(result, 0, "typed_dynamic_callable_value")
                .map_err(|error| error.to_string())?;
            let error = self
                .builder
                .build_extract_value(result, 1, "typed_dynamic_callable_error")
                .map_err(|error| error.to_string())?;
            self.builder
                .build_store(self.pending_exception().as_pointer_value(), error)
                .map_err(|error| error.to_string())?;
            self.branch_on_pending_exception()?;
            return Ok(value);
        }
        if signature.backend == DynamicBackend::Napi && !function_argument.is_empty() {
            let functions = self.compile_napi_function_arguments(args, &function_argument)?;
            let count = self
                .context
                .i64_type()
                .const_int(function_argument.len() as u64, false);
            let json = self.compile_json_backend_values_with_extra(
                name.as_pointer_value().into(),
                array,
                "thaw_napi_call_with_functions_typed_result",
                &[functions.into(), count.into()],
            )?;
            return self.compile_typed_dynamic_result(json, &signature.ret);
        }
        let backend = match signature.backend {
            DynamicBackend::Jit => unreachable!("JIT calls return before JSON marshalling"),
            DynamicBackend::QuickJs => "thaw_js_call_result",
            DynamicBackend::Napi => "thaw_napi_call_typed_result",
        };
        let json =
            self.compile_json_backend_values(name.as_pointer_value().into(), array, backend)?;
        self.compile_typed_dynamic_result(json, &signature.ret)
    }

}
