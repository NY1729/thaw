impl<'ctx> HirCompiler<'ctx> {
    fn compile_js_callback_from_json(
        &mut self,
        json: BasicValueEnum<'ctx>,
        params: &[HirType],
        ret: &HirType,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let retained_handle = self
            .builder
            .build_call(
                self.module.get_function("thaw_json_handle_id").unwrap(),
                &[json.into()],
                "js_callback_handle",
            )
            .map_err(|error| error.to_string())?
            .try_as_basic_value().basic()
            .ok_or("thaw_json_handle_id returned no callback handle")?;

        let parent = self
            .builder
            .get_insert_block()
            .ok_or("JS callback must be decoded inside a function")?;
        let name = format!("__thaw_js_callback_{}", self.next_lambda);
        self.next_lambda += 1;
        let adapter = self.module.add_function(
            &name,
            self.function_type(params, ret)?,
            Some(Linkage::Internal),
        );
        // This adapter is a separate LLVM function. Its exception branch
        // must not target a catch block in the function that creates it.
        let outer_catch_stack = std::mem::take(&mut self.catch_stack);
        let outer_async_completion = self.active_async_completion.take();
        let entry = self.context.append_basic_block(adapter, "entry");
        self.builder.position_at_end(entry);
        let environment = adapter
            .get_first_param()
            .ok_or("JS callback adapter is missing its environment")?
            .into_pointer_value();
        let handle_slot = unsafe {
            self.builder.build_in_bounds_gep(
                self.context.i8_type(),
                environment,
                &[self.context.i64_type().const_int(CLOSURE_CAPTURE_BASE, false)],
                "js_callback_handle_slot",
            )
        }
        .map_err(|error| error.to_string())?;
        let adapter_handle = self
            .builder
            .build_load(self.context.i64_type(), handle_slot, "js_callback_handle")
            .map_err(|error| error.to_string())?;
        let arguments = self
            .builder
            .build_call(
                self.module.get_function("thaw_json_array_new").unwrap(),
                &[],
                "js_callback_arguments",
            )
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .ok_or("thaw_json_array_new did not return callback arguments")?;
        for (index, ty) in params.iter().enumerate() {
            let value = adapter
                .get_nth_param((index + 1) as u32)
                .ok_or("JS callback adapter is missing an argument")?;
            self.compile_json_array_push_native(arguments, value, ty)?;
        }
        let arguments_json = self
            .builder
            .build_call(
                self.module.get_function("thaw_json_graph_encode").unwrap(),
                &[arguments.into()],
                "js_callback_arguments_json",
            )
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .ok_or("thaw_json_stringify did not return callback arguments")?;
        let arguments_json = self.compile_check_json_stringify_error_with_cleanup(arguments_json, &[arguments])?;
        let result = self
            .builder
            .build_call(
                self.module.get_function("thaw_js_call_handle_graph_result").unwrap(),
                &[adapter_handle.into(), arguments_json.into()],
                "js_callback_result",
            )
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .ok_or("thaw_js_call_handle_result did not return a result")?
            .into_struct_value();
        let error = self
            .builder
            .build_extract_value(result, 1, "js_callback_error")
            .map_err(|error| error.to_string())?;
        let value = self
            .builder
            .build_extract_value(result, 0, "js_callback_value")
            .map_err(|error| error.to_string())?;
        self.builder
            .build_call(
                self.module.get_function("thaw_cstring_destroy").unwrap(),
                &[arguments_json.into()],
                "destroy_js_callback_arguments_string",
            )
            .map_err(|error| error.to_string())?;
        self.builder
            .build_call(
                self.module.get_function("thaw_json_destroy").unwrap(),
                &[arguments.into()],
                "destroy_js_callback_arguments",
            )
            .map_err(|error| error.to_string())?;
        self.builder
            .build_store(self.pending_exception().as_pointer_value(), error)
            .map_err(|error| error.to_string())?;
        self.clear_pending_native_text()?;
        self.mark_pending_native_text(error)?;
        self.branch_on_pending_exception()?;
        if *ret == HirType::Void {
            self.builder
                .build_call(
                    self.module.get_function("thaw_cstring_destroy").unwrap(),
                    &[value.into()],
                    "destroy_js_callback_result_string",
                )
                .map_err(|error| error.to_string())?;
            self.builder
                .build_return(None)
                .map_err(|error| error.to_string())?;
        } else {
            let result_json = self.compile_decode_quickjs_graph(value.into())?;

            let decoded = self.compile_typed_dynamic_result(result_json, ret)?;
            self.builder
                .build_return(Some(&decoded))
                .map_err(|error| error.to_string())?;
        }
        self.catch_stack = outer_catch_stack;
        self.active_async_completion = outer_async_completion;
        self.builder.position_at_end(parent);
        let this_adapter = self.compile_ignored_this_adapter(
            adapter,
            params,
            ret,
            &format!("{name}__thaw_this_adapter"),
        )?;

        let closure = self
            .builder
            .build_call(
                self.module.get_function("thaw_arena_alloc").unwrap(),
                &[
                    self.context
                        .i64_type()
                        .const_int(CLOSURE_CAPTURE_BASE + 8, false)
                        .into(),
                    self.context.i64_type().const_int(8, false).into(),
                ],
                "js_callback_closure",
            )
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .ok_or("thaw_arena_alloc did not return a callback closure")?
            .into_pointer_value();
        self.builder
            .build_store(closure, adapter.as_global_value().as_pointer_value())
            .map_err(|error| error.to_string())?;
        for offset in [CLOSURE_THIS_ENTRY_OFFSET, CLOSURE_CAPTURE_BASE] {
            let slot = unsafe {
                self.builder.build_in_bounds_gep(
                    self.context.i8_type(),
                    closure,
                    &[self.context.i64_type().const_int(offset, false)],
                    "js_callback_closure_slot",
                )
            }
            .map_err(|error| error.to_string())?;
            if offset == CLOSURE_CAPTURE_BASE {
                self.builder
                    .build_store(slot, retained_handle)
                    .map_err(|error| error.to_string())?;
            } else {
                self.builder
                    .build_store(slot, this_adapter.as_global_value().as_pointer_value())
                    .map_err(|error| error.to_string())?;
            }
        }
        Ok(closure.into())
    }

    fn compile_jit_argument_slots(
        &mut self,
        value: BasicValueEnum<'ctx>,
        ty: &HirType,
        path: &str,
        output: &mut Vec<BasicValueEnum<'ctx>>,
    ) -> Result<(), String> {
        if *ty == HirType::JsValue {
            output.push(
                self.builder
                    .build_bit_cast(
                        value.into_int_value(),
                        self.context.f64_type(),
                        &format!("{path}_handle_slot"),
                    )
                    .map_err(|error| error.to_string())?,
            );
            return Ok(());
        }
        if let HirType::Union(elements) = ty {
            if !jit_argument_tagged_union(elements) {
                return Err(format!("unsupported JIT union argument {ty:?}"));
            }
            let union = value.into_struct_value();
            let tag = self
                .builder
                .build_extract_value(union, 0, &format!("{path}_tag"))
                .map_err(|error| error.to_string())?
                .into_int_value();
            let payload = self
                .builder
                .build_extract_value(union, 1, &format!("{path}_payload"))
                .map_err(|error| error.to_string())?
                .into_int_value();
            let mut runtime_tag = self.context.i8_type().const_zero();
            for (index, member) in elements.iter().enumerate() {
                let semantic = jit_union_member_tag(member).unwrap();
                let selected = self
                    .builder
                    .build_int_compare(
                        inkwell::IntPredicate::EQ,
                        tag,
                        self.context.i8_type().const_int(index as u64, false),
                        &format!("{path}_is_{index}"),
                    )
                    .map_err(|error| error.to_string())?;
                runtime_tag = self
                    .builder
                    .build_select(
                        selected,
                        self.context.i8_type().const_int(semantic, false),
                        runtime_tag,
                        &format!("{path}_runtime_tag"),
                    )
                    .map_err(|error| error.to_string())?
                    .into_int_value();
            }
            output.push(
                self.builder
                    .build_unsigned_int_to_float(
                        runtime_tag,
                        self.context.f64_type(),
                        &format!("{path}_tag_slot"),
                    )
                    .map_err(|error| error.to_string())?
                    .into(),
            );
            output.push(
                self.builder
                    .build_bit_cast(
                        payload,
                        self.context.f64_type(),
                        &format!("{path}_payload_slot"),
                    )
                    .map_err(|error| error.to_string())?,
            );
            return Ok(());
        }
        if let HirType::Object(fields) = ty {
            let object = value.into_pointer_value();
            let mut offset = 0u64;
            for (field, field_type) in fields {
                let field_path = format!("{path}_{field}");
                let pointer = unsafe {
                    self.builder
                        .build_in_bounds_gep(
                            self.context.i8_type(),
                            object,
                            &[self.context.i64_type().const_int(offset, false)],
                            &field_path,
                        )
                        .map_err(|error| error.to_string())?
                };
                let field_value = self
                    .builder
                    .build_load(
                        self.basic_type(field_type)?,
                        pointer,
                        &format!("{field_path}_value"),
                    )
                    .map_err(|error| error.to_string())?;
                self.compile_jit_argument_slots(field_value, field_type, &field_path, output)?;
                offset = checked_storage_add(offset, object_field_storage_bytes(field_type)?)?;
            }
            return Ok(());
        }
        if let HirType::Tuple(elements) = ty {
            let tuple = self.compile_array_data(value.into_pointer_value())?;
            let stride = tuple_element_storage_bytes(elements)?;
            for (index, element_type) in elements.iter().enumerate() {
                let element_path = format!("{path}_{index}");
                let pointer = unsafe {
                    self.builder
                        .build_in_bounds_gep(
                            self.context.i8_type(),
                            tuple,
                            &[self.context.i64_type().const_int(
                                checked_array_offset(stride, index)?,
                                false,
                            )],
                            &element_path,
                        )
                        .map_err(|error| error.to_string())?
                };
                let element = self
                    .builder
                    .build_load(
                        self.basic_type(element_type)?,
                        pointer,
                        &format!("{element_path}_value"),
                    )
                    .map_err(|error| error.to_string())?;
                self.compile_jit_argument_slots(
                    element,
                    element_type,
                    &element_path,
                    output,
                )?;
            }
            return Ok(());
        }
        output.push(if *ty == HirType::Bool {
            self.builder
                .build_unsigned_int_to_float(
                    value.into_int_value(),
                    self.context.f64_type(),
                    "jit_boolean_slot",
                )
                .map_err(|error| error.to_string())?
                .into()
        } else {
            value
        });
        Ok(())
    }

    fn compile_napi_value_callback_from_closure(
        &mut self,
        closure: PointerValue<'ctx>,
        params: &[HirType],
        ret: &HirType,
        rest_start: Option<usize>,
    ) -> Result<(PointerValue<'ctx>, PointerValue<'ctx>), String> {
        let (adapter, closure, _) =
            self.compile_value_callback_from_closure(closure, params, ret, false, rest_start, false, true)?;
        Ok((adapter, closure))
    }

    fn compile_napi_function_arguments(
        &mut self,
        callbacks: &[BasicValueEnum<'ctx>],
        functions: &[NapiFunctionArgument],
    ) -> Result<PointerValue<'ctx>, String> {
        let ptr_type = self.context.ptr_type(AddressSpace::default());
        if functions.is_empty() {
            return Ok(ptr_type.const_null());
        }
        let descriptor_type = self.context.struct_type(
            &[
                self.context.i64_type().into(),
                ptr_type.into(),
                ptr_type.into(),
            ],
            false,
        );
        let descriptors = self
            .builder
            .build_alloca(
                descriptor_type.array_type(functions.len() as u32),
                "napi_function_arguments",
            )
            .map_err(|error| error.to_string())?;
        for (slot, (index, params, ret, optional, rest_start)) in functions.iter().enumerate() {
            let callback = callbacks[slot];
            let (adapter, context) = if *optional {
                let callback = callback.into_struct_value();
                let present = self
                    .builder
                    .build_extract_value(callback, 0, "optional_napi_callback_present")
                    .map_err(|error| error.to_string())?
                    .into_int_value();
                let closure = self
                    .builder
                    .build_extract_value(callback, 1, "optional_napi_callback")
                    .map_err(|error| error.to_string())?
                    .into_pointer_value();
                let (adapter, context) =
                    self.compile_napi_value_callback_from_closure(
                        closure,
                        params,
                        ret,
                        *rest_start,
                    )?;
                let adapter = self
                    .builder
                    .build_select(present, adapter, ptr_type.const_null(), "napi_callback")
                    .map_err(|error| error.to_string())?
                    .into_pointer_value();
                (adapter, context)
            } else {
                self.compile_napi_value_callback_from_closure(
                    callback.into_pointer_value(),
                    params,
                    ret,
                    *rest_start,
                )?
            };
            let descriptor = self
                .builder
                .build_insert_value(
                    descriptor_type.get_undef(),
                    self.context.i64_type().const_int(*index as u64, false),
                    0,
                    "napi_function_argument_index",
                )
                .map_err(|error| error.to_string())?
                .into_struct_value();
            let descriptor = self
                .builder
                .build_insert_value(descriptor, adapter, 1, "napi_function_argument_callback")
                .map_err(|error| error.to_string())?
                .into_struct_value();
            let descriptor = self
                .builder
                .build_insert_value(descriptor, context, 2, "napi_function_argument_context")
                .map_err(|error| error.to_string())?;
            let target = unsafe {
                self.builder
                    .build_in_bounds_gep(
                        descriptor_type,
                        descriptors,
                        &[self.context.i64_type().const_int(slot as u64, false)],
                        "napi_function_argument",
                    )
                    .map_err(|error| error.to_string())?
            };
            self.builder
                .build_store(target, descriptor)
                .map_err(|error| error.to_string())?;
        }
        Ok(descriptors)
    }

    fn compile_value_callback_from_closure(
        &mut self,
        closure: PointerValue<'ctx>,
        params: &[HirType],
        ret: &HirType,
        defer_promise: bool,
        rest_start: Option<usize>,
        graph_mode: bool,
        graph_wire: bool,
    ) -> Result<
        (
            PointerValue<'ctx>,
            PointerValue<'ctx>,
            Option<PointerValue<'ctx>>,
        ),
        String,
    > {
        let callback_name = format!("__thaw_napi_value_callback_{}", self.next_lambda);
        self.next_lambda += 1;
        let ptr_type = self.context.ptr_type(AddressSpace::default());
        let adapter_type = ptr_type.fn_type(&[ptr_type.into(), ptr_type.into()], false);
        let adapter = self
            .module
            .add_function(&callback_name, adapter_type, Some(Linkage::Internal));
        let return_block = self.builder.get_insert_block().unwrap();
        let entry = self.context.append_basic_block(adapter, "entry");
        self.builder.position_at_end(entry);
        // `adapter`'s own body is a *separate* LLVM function from whatever
        // is currently being compiled -- if that outer compilation is
        // itself inside a user `try` block (real example: a database call
        // wrapped in `try { ... } catch { ... }`, awaited inside an async
        // function that also registers an async callback via this same
        // adapter), `self.catch_stack` still holds the *outer* function's
        // own catch block. `branch_on_pending_exception` (called below,
        // via `drive_promise_to_resolved_value`/`drive_promise_to_
        // completion` for a `Promise`-returning callback) would otherwise
        // branch straight into that unrelated block from inside this
        // adapter -- an illegal cross-function edge ("Referring to a basic
        // block in another function!", an LLVM module-verification
        // failure). `adapter` has no enclosing `try` of its own, so it
        // must see an empty catch stack (propagate/default-return on
        // failure, matching a top-level function with no active catch).
        let outer_catch_stack = std::mem::take(&mut self.catch_stack);
        let outer_async_completion = self.active_async_completion.take();
        let context = adapter.get_nth_param(0).unwrap().into_pointer_value();
        let args_string = adapter.get_nth_param(1).unwrap();
        let args_json = self
            .builder
            .build_call(
                self.module.get_function(if graph_wire { "thaw_json_graph_decode" } else { "thaw_json_parse" }).unwrap(),
                &[args_string.into()],
                "napi_value_callback_args",
            )
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .unwrap();
        if graph_wire {
            let status = self.builder.build_call(
                self.module.get_function("thaw_json_take_graph_error").unwrap(),
                &[], "native_callback_graph_error",
            ).map_err(|error| error.to_string())?.try_as_basic_value().basic()
                .ok_or("graph decode status returned no value")?.into_int_value();
            let invalid = self.context.append_basic_block(adapter, "native_callback_graph_invalid");
            let valid = self.context.append_basic_block(adapter, "native_callback_graph_valid");
            let failed = self.builder.build_int_compare(
                inkwell::IntPredicate::NE, status, self.context.i8_type().const_zero(), "invalid_callback_graph",
            ).map_err(|error| error.to_string())?;
            self.builder.build_conditional_branch(failed, invalid, valid)
                .map_err(|error| error.to_string())?;
            self.builder.position_at_end(invalid);
            self.builder.build_call(self.module.get_function("thaw_json_destroy").unwrap(),
                &[args_json.into()], "destroy_invalid_callback_graph")
                .map_err(|error| error.to_string())?;
            let message = self.builder.build_global_string_ptr(
                "\u{1}TypeError\u{1}Invalid native callback graph", "invalid_callback_graph_message",
            ).map_err(|error| error.to_string())?;
            let invalid_result = if defer_promise && matches!(ret, HirType::Promise(_)) {
                let promise = self.builder.build_call(
                    self.module.get_function("thaw_promise_new").unwrap(),
                    &[], "invalid_callback_graph_promise",
                ).map_err(|error| error.to_string())?.try_as_basic_value().basic()
                    .ok_or("callback graph rejection has no Promise")?;
                self.builder.build_call(
                    self.module.get_function("thaw_promise_reject_native_text").unwrap(),
                    &[promise.into(), message.as_pointer_value().into()],
                    "reject_invalid_callback_graph",
                ).map_err(|error| error.to_string())?;
                promise
            } else {
                self.builder.build_call(
                    self.module.get_function("thaw_json_callback_error").unwrap(),
                    &[message.as_pointer_value().into()], "invalid_callback_graph_result",
                ).map_err(|error| error.to_string())?.try_as_basic_value().basic()
                    .ok_or("callback error framing returned no value")?
            };
            self.builder.build_return(Some(&invalid_result)).map_err(|error| error.to_string())?;
            self.builder.position_at_end(valid);
        }
        let null_key = ptr_type.const_null();
        // Graph callbacks carry the JavaScript caller's holder at index zero.
        // Keeping it in the same graph as the arguments preserves aliases
        // between a JSON.stringify replacer's `this` and the current value.
        let holder_json = if graph_mode {
            Some(self.builder.build_call(
                self.module.get_function("thaw_json_index").unwrap(),
                &[args_json.into(), self.context.f64_type().const_zero().into(), null_key.into()],
                "native_callback_holder",
            ).map_err(|error| error.to_string())?.try_as_basic_value().basic()
                .ok_or("graph callback holder returned no value")?.into_pointer_value())
        } else {
            None
        };
        // A dynamic argument conversion may invoke a live Host getter and
        // branch to a catch target before the callback itself is called.
        // Track only values already acquired at each such branch: SSA values
        // for later arguments do not dominate an earlier failure edge.
        let failed_block = self.context.append_basic_block(adapter, "callback_failed");
        let conversion_failed = self.context.append_basic_block(adapter, "callback_conversion_failed");
        let mut argument_slots = Vec::with_capacity(params.len() + usize::from(graph_mode));
        for _ in 0..params.len() + usize::from(graph_mode) {
            let slot = self.builder.build_alloca(ptr_type, "callback_argument_json_slot")
                .map_err(|error| error.to_string())?;
            self.builder.build_store(slot, ptr_type.const_null())
                .map_err(|error| error.to_string())?;
            argument_slots.push(slot);
        }
        let mut handle_slots = Vec::with_capacity(params.len());
        for param in params {
            let slot = if *param == HirType::JsValue {
                let slot = self.builder.build_alloca(self.context.i64_type(), "callback_argument_handle_slot")
                    .map_err(|error| error.to_string())?;
                self.builder.build_store(slot, self.context.i64_type().const_zero())
                    .map_err(|error| error.to_string())?;
                Some(slot)
            } else { None };
            handle_slots.push(slot);
        }
        let mut callback_args = vec![context.into()];
        let mut argument_json = Vec::with_capacity(params.len() + usize::from(graph_mode));
        if let Some(holder) = holder_json {
            argument_json.push(holder);
            self.builder.build_store(argument_slots[0], holder)
                .map_err(|error| error.to_string())?;
        }
        self.builder.build_call(
            self.module.get_function("thaw_json_typed_decode_scope_begin").unwrap(),
            &[], "begin_callback_conversion_scope",
        ).map_err(|error| error.to_string())?;
        for (index, param) in params.iter().enumerate() {
            let graph_index = index + usize::from(graph_mode);
            let argument = if rest_start == Some(index) {
                self.builder.build_call(
                    self.module.get_function("thaw_json_array_slice").unwrap(),
                    &[
                        args_json.into(),
                        self.context.i64_type().const_int(graph_index as u64, false).into(),
                    ],
                    "napi_value_callback_rest",
                )
            } else {
                self.builder.build_call(
                    self.module.get_function("thaw_json_index").unwrap(),
                    &[
                        args_json.into(),
                        self.context.f64_type().const_float(graph_index as f64).into(),
                        null_key.into(),
                    ],
                    "napi_value_callback_argument",
                )
            }
                .map_err(|error| error.to_string())?
                .try_as_basic_value()
                .basic()
                .unwrap();
            argument_json.push(argument);
            self.builder.build_store(argument_slots[graph_index], argument)
                .map_err(|error| error.to_string())?;
            self.catch_stack.push(conversion_failed);
            let converted = self.compile_json_value_to_native(argument, param);
            self.catch_stack.pop();
            let converted = converted?;
            if let Some(slot) = handle_slots[index] {
                self.builder.build_store(slot, converted)
                    .map_err(|error| error.to_string())?;
            }
            callback_args.push(converted.into());
        }
        self.builder.build_call(
            self.module.get_function("thaw_json_typed_decode_scope_end").unwrap(),
            &[self.context.i8_type().const_zero().into()], "finish_callback_conversion_scope",
        ).map_err(|error| error.to_string())?;
        let code_slot = if graph_mode { unsafe {
            self.builder.build_in_bounds_gep(
                self.context.i8_type(), context,
                &[self.context.i64_type().const_int(CLOSURE_THIS_ENTRY_OFFSET, false)],
                "native_callback_this_entry_slot",
            ).map_err(|error| error.to_string())?
        }} else { context };
        let code = self
            .builder
            .build_load(ptr_type, code_slot, "napi_value_callback_code")
            .map_err(|error| error.to_string())?
            .into_pointer_value();
        let closure_type = if graph_mode {
            let holder = holder_json.expect("graph callback has a holder");
            let word = self.builder.build_ptr_to_int(holder, self.context.i64_type(), "callback_holder_word")
                .map_err(|error| error.to_string())?;
            let receiver = self.builder.build_insert_value(
                self.receiver_type().get_undef(), self.context.i8_type().const_int(7, false),
                0, "callback_holder_kind",
            ).map_err(|error| error.to_string())?.into_struct_value();
            let receiver = self.builder.build_insert_value(receiver, word, 1, "callback_holder_receiver")
                .map_err(|error| error.to_string())?.into_struct_value();
            callback_args.insert(1, receiver.into());
            self.this_entry_function_type(params, ret)?
        } else {
            self.function_type(params, ret)?
        };
        let call = self
            .builder
            .build_indirect_call(closure_type, code, &callback_args, "invoke_napi_value_callback")
            .map_err(|error| error.to_string())?;
        let pending_slot = self.pending_exception().as_pointer_value();
        let pending = self
            .builder
            .build_load(self.context.ptr_type(AddressSpace::default()), pending_slot, "callback_exception")
            .map_err(|error| error.to_string())?
            .into_pointer_value();
        let failed = self
            .builder
            .build_is_not_null(pending, "callback_failed")
            .map_err(|error| error.to_string())?;
        let success_block = self.context.append_basic_block(adapter, "callback_succeeded");
        self.builder
            .build_conditional_branch(failed, failed_block, success_block)
            .map_err(|error| error.to_string())?;

        self.builder.position_at_end(conversion_failed);
        self.compile_discard_typed_decode_scope()?;
        // The scope released all acquired children and independent handles.
        // The common failure block still owns the holder and root graph.
        for slot in argument_slots.iter().skip(usize::from(graph_mode)) {
            self.builder.build_store(*slot, ptr_type.const_null())
                .map_err(|error| error.to_string())?;
        }
        for slot in handle_slots.iter().flatten() {
            self.builder.build_store(*slot, self.context.i64_type().const_zero())
                .map_err(|error| error.to_string())?;
        }
        self.builder.build_unconditional_branch(failed_block)
            .map_err(|error| error.to_string())?;

        self.builder.position_at_end(failed_block);
        let pending = self.builder.build_load(ptr_type, pending_slot, "callback_failed_exception")
            .map_err(|error| error.to_string())?.into_pointer_value();
        let framed = if defer_promise && matches!(ret, HirType::Promise(_)) {
            // A deferred callback has a Promise-pointer ABI even when its
            // synchronous body throws before producing a Promise.
            let rejected = self.builder.build_call(
                self.module.get_function("thaw_promise_new").unwrap(),
                &[], "callback_synchronous_failure_promise",
            ).map_err(|error| error.to_string())?.try_as_basic_value().basic()
                .ok_or("callback rejection has no Promise")?;
            self.reject_promise_with_pending_exception(
                rejected.into_pointer_value(), pending,
                "reject_synchronous_callback_failure",
            )?;
            rejected
        } else {
            self.builder.build_call(
                self.module.get_function("thaw_json_callback_error").unwrap(),
                &[pending.into()], "callback_error_result",
            ).map_err(|error| error.to_string())?.try_as_basic_value().basic()
                .ok_or("thaw_json_callback_error returned no value")?
        };
        self.builder
            .build_store(
                pending_slot,
                self.context.ptr_type(AddressSpace::default()).const_null(),
            )
            .map_err(|error| error.to_string())?;
        for slot in handle_slots.iter().flatten() {
            let handle = self.builder.build_load(self.context.i64_type(), *slot, "failed_callback_handle")
                .map_err(|error| error.to_string())?;
            self.builder.build_call(self.module.get_function("thaw_js_release_handle").unwrap(),
                &[handle.into()], "release_failed_callback_argument")
                .map_err(|error| error.to_string())?;
        }
        for slot in &argument_slots {
            let value = self.builder.build_load(ptr_type, *slot, "failed_callback_json")
                .map_err(|error| error.to_string())?;
            self.builder.build_call(self.module.get_function("thaw_json_destroy").unwrap(),
                &[value.into()], "destroy_failed_callback_argument")
                .map_err(|error| error.to_string())?;
        }
        self.builder.build_call(self.module.get_function("thaw_json_destroy").unwrap(),
            &[args_json.into()], "destroy_failed_callback_args")
            .map_err(|error| error.to_string())?;
        self.builder
            .build_return(Some(&framed))
            .map_err(|error| error.to_string())?;
        self.builder.position_at_end(success_block);
        if defer_promise && matches!(ret, HirType::Promise(_)) {
            let promise = call
                .try_as_basic_value()
                .basic()
                .ok_or("async value callback must return a Promise")?
                .into_pointer_value();
            for (index, param) in params.iter().enumerate() {
                if *param == HirType::JsValue {
                    self.builder.build_call(
                        self.module.get_function("thaw_js_release_handle").unwrap(),
                        &[callback_args[index + 1 + usize::from(graph_mode)]],
                        "release_deferred_callback_argument",
                    ).map_err(|error| error.to_string())?;
                }
            }
            for value in argument_json.iter().copied().chain(std::iter::once(args_json)) {
                self.builder.build_call(
                    self.module.get_function("thaw_json_destroy").unwrap(),
                    &[value.into()],
                    "destroy_deferred_callback_argument",
                ).map_err(|error| error.to_string())?;
            }
            self.builder
                .build_return(Some(&promise))
                .map_err(|error| error.to_string())?;
            self.catch_stack = outer_catch_stack;
            self.active_async_completion = outer_async_completion;
            self.builder.position_at_end(return_block);
            let HirType::Promise(resolved) = ret else {
                unreachable!()
            };
            let guarded = self.compile_host_callback_pending_guard(adapter, true)?;
            let finish = self.compile_native_promise_callback_finisher(resolved)?;
            return Ok((
                guarded,
                closure,
                Some(finish),
            ));
        }
        // A `void`-returning closure (real example: zod's own
        // `superRefine((val, ctx) => { ctx.addIssue(...); })` -- the
        // predicate mutates `ctx` and returns nothing at all) has no
        // return value to marshal; encoded as a bare JSON `null` result
        // instead of running it through the ordinary array-wrap-then-
        // index dance below, which requires a real value to push.
        // `undefined` is a real callback result; only `void` means there is
        // no value for the JavaScript wrapper to reconstruct.
        let mut result_array = None;
        let result_json = if matches!(ret, HirType::Undefined) {
            self.compile_napi_undefined_json()?
        } else if matches!(ret, HirType::Void) {
            self.compile_json_null()?
        } else {
            let result = call
                .try_as_basic_value()
                .basic()
                .ok_or("N-API value callback must return a value")?;
            // An `async` callback's declared return type is always
            // `Promise<T>` here (see `discover_frame_async_functions`'s
            // "every async function must expose the same Promise-handle
            // ABI" invariant), so `result` is a genuine, resolvable
            // `ThawPromise` pointer -- never a raw unwrapped value in
            // disguise. Drive it to its resolved value (propagating a
            // rejection as a pending thaw exception, same as an ordinary
            // `await`) before marshaling `T`, not `Promise<T>`, to JSON.
            let (result, ret) = match ret {
                HirType::Promise(resolved) if **resolved == HirType::Void => {
                    self.drive_promise_to_completion(result.into_pointer_value())?;
                    (result, resolved.as_ref())
                }
                HirType::Promise(resolved) => {
                    let resolved_value = self.drive_promise_to_resolved_value(
                        result.into_pointer_value(),
                        resolved,
                    )?;
                    (resolved_value, resolved.as_ref())
                }
                other => (result, other),
            };
            if matches!(ret, HirType::Undefined | HirType::Void) {
                self.compile_json_null()?
            } else {
                let result_json = self
                    .builder
                    .build_call(
                        self.module.get_function("thaw_json_array_new").unwrap(),
                        &[],
                        "napi_value_callback_result_array",
                    )
                    .map_err(|error| error.to_string())?
                    .try_as_basic_value()
                    .basic()
                    .unwrap();
                result_array = Some(result_json);
                // `preserve_undefined: true` -- this is a standalone
                // return value being wrapped, not an object field that
                // can legitimately omit itself (see `wrap_native_value_
                // as_json`, thaw-hir, for the identical reasoning): a
                // callback whose own inferred return type is `Optional
                // (Json)`/`Nullish(Json)` (real trigger: `qs`'s own
                // `filter: (prefix, value) => (cond ? undefined :
                // value)`, a ternary unifying an `Undefined` branch with
                // a `Json` one) must still tell a real `undefined` apart
                // from a real `null` in the JSON this produces -- the
                // `false` this used to pass silently collapsed the
                // "absent" case to a plain JSON `null` every time,
                // indistinguishable from an explicit `null` return.
                if *ret == HirType::JsValue {
                    let result = self.compile_dynamic_value_placeholder_unchecked(result)?;
                    self.builder
                        .build_call(
                            self.module
                                .get_function("thaw_json_array_push_json")
                                .unwrap(),
                            &[result_json.into(), result.into()],
                            "native_callback_dynamic_result",
                        )
                        .map_err(|error| error.to_string())?;
                } else {
                    self.compile_json_array_push_native_with_undefined(
                        result_json,
                        result,
                        ret,
                        true,
                    )?;
                }
                self.builder
                    .build_call(
                        self.module.get_function("thaw_json_index").unwrap(),
                        &[
                            result_json.into(),
                            self.context.f64_type().const_zero().into(),
                            null_key.into(),
                        ],
                        "napi_value_callback_result",
                    )
                    .map_err(|error| error.to_string())?
                    .try_as_basic_value()
                    .basic()
                    .unwrap()
            }
        };
        // Cleans up every decoded argument (the `Json` values from
        // `thaw_json_index`/`thaw_json_array_slice`, and any retained
        // `JsValue` parameter handle) only *after* `result_json` has
        // been fully built above -- not right after `call` returns.
        // `compile_json_array_push_native`/`thaw_json_array_push_json`
        // (used to build `result_json` from the closure's own return
        // value) clone defensively, but only if the source they clone
        // from is still alive: a callback that returns one of its own
        // parameters unchanged (a real, common pattern -- e.g. `qs`'s
        // own `filter: (prefix, value) => value`) makes the return
        // value alias one of these same `argument_json` pointers.
        // Destroying them first (the original ordering here) freed
        // that memory before the clone ever read it -- a real,
        // reproducible segfault, not just a leak. Moving cleanup to
        // here instead means nothing downstream ever needs the
        // original argument pointers again, so this is always safe
        // regardless of aliasing (including a *nested* alias, e.g.
        // `(x) => ({ wrapped: x })` -- `serde_json::Value::clone` is a
        // real recursive clone, so the nested copy is independent too).
        {
            for (index, param) in params.iter().enumerate() {
                if *param == HirType::JsValue {
                    self.builder
                        .build_call(
                            self.module.get_function("thaw_js_release_handle").unwrap(),
                            &[callback_args[index + 1 + usize::from(graph_mode)]],
                            "release_native_callback_argument",
                        )
                        .map_err(|error| error.to_string())?;
                }
            }
            for value in argument_json.into_iter().chain(std::iter::once(args_json)) {
                self.builder
                    .build_call(
                        self.module.get_function("thaw_json_destroy").unwrap(),
                        &[value.into()],
                        "destroy_native_callback_json",
                    )
                    .map_err(|error| error.to_string())?;
            }
        }
        let result = self
            .builder
            .build_call(
                self.module.get_function(if graph_wire { "thaw_json_graph_encode" } else { "thaw_json_stringify" }).unwrap(),
                &[result_json.into()],
                "napi_value_callback_result_string",
            )
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .unwrap();
        self.builder
            .build_call(
                self.module.get_function("thaw_json_destroy").unwrap(),
                &[result_json.into()],
                "destroy_native_callback_result_json",
            )
            .map_err(|error| error.to_string())?;
        if let Some(array) = result_array {
            self.builder.build_call(
                self.module.get_function("thaw_json_destroy").unwrap(),
                &[array.into()],
                "destroy_native_callback_result_array",
            ).map_err(|error| error.to_string())?;
        }
        let result = self.compile_check_json_stringify_callback_result(result)?;
        self.builder
            .build_return(Some(&result))
            .map_err(|error| error.to_string())?;
        self.catch_stack = outer_catch_stack;
        self.active_async_completion = outer_async_completion;
        self.builder.position_at_end(return_block);
        let guarded = self.compile_host_callback_pending_guard(adapter, false)?;
        Ok((guarded, closure, None))
    }

    /// A host may invoke this adapter while an outer generated frame has an
    /// unrelated pending exception. Isolate the callback's slot for its whole
    /// body, including compiler-generated early returns from Promise driving.
    fn compile_host_callback_pending_guard(
        &mut self,
        body: FunctionValue<'ctx>,
        returns_promise: bool,
    ) -> Result<PointerValue<'ctx>, String> {
        let parent = self.builder.get_insert_block().ok_or("callback guard has no parent")?;
        let ptr_type = self.context.ptr_type(AddressSpace::default());
        let name = format!("__thaw_callback_pending_guard_{}", self.next_lambda);
        self.next_lambda += 1;
        let wrapper = self.module.add_function(
            &name,
            ptr_type.fn_type(&[ptr_type.into(), ptr_type.into()], false),
            Some(Linkage::Internal),
        );
        let entry = self.context.append_basic_block(wrapper, "entry");
        let failed = self.context.append_basic_block(wrapper, "callback_raised");
        let succeeded = self.context.append_basic_block(wrapper, "callback_returned");
        self.builder.position_at_end(entry);
        // The exception string, native Error object, and typed thrown-value
        // fields are one logical state. Snapshot them together before invoking
        // foreign code so a callback cannot consume an outer exception or
        // replace its typed payload.
        let exception_slots = [
            (self.pending_exception(), BasicTypeEnum::from(ptr_type)),
            (self.pending_exception_native_text(), BasicTypeEnum::from(ptr_type)),
            (self.pending_exception_object(), BasicTypeEnum::from(ptr_type)),
            (self.pending_exception_aggregate_errors(), BasicTypeEnum::from(ptr_type)),
            (self.pending_exception_value(PENDING_EXCEPTION_VALUE_TAG_SYMBOL), BasicTypeEnum::from(self.context.i64_type())),
            (self.pending_exception_value(PENDING_EXCEPTION_F64_SYMBOL), BasicTypeEnum::from(self.context.f64_type())),
            (self.pending_exception_value(PENDING_EXCEPTION_I64_SYMBOL), BasicTypeEnum::from(self.context.i64_type())),
            (self.pending_exception_value(PENDING_EXCEPTION_BOOL_SYMBOL), BasicTypeEnum::from(self.context.bool_type())),
        ];
        let mut previous = Vec::with_capacity(exception_slots.len());
        for (slot, ty) in exception_slots {
            let value = self.builder.build_load(ty, slot.as_pointer_value(), "outer_exception_field")
                .map_err(|error| error.to_string())?;
            previous.push((slot, value));
            self.builder.build_store(slot.as_pointer_value(), ty.const_zero())
                .map_err(|error| error.to_string())?;
        }
        let pending_slot = self.pending_exception().as_pointer_value();
        let result = self.builder.build_call(body,
            &[wrapper.get_nth_param(0).unwrap().into(), wrapper.get_nth_param(1).unwrap().into()],
            "run_isolated_host_callback")
            .map_err(|error| error.to_string())?.try_as_basic_value().basic()
            .ok_or("host callback returned no value")?.into_pointer_value();
        let raised = self.builder.build_load(ptr_type, pending_slot, "own_callback_exception")
            .map_err(|error| error.to_string())?.into_pointer_value();
        let has_error = self.builder.build_is_not_null(raised, "callback_has_own_exception")
            .map_err(|error| error.to_string())?;
        self.builder.build_conditional_branch(has_error, failed, succeeded)
            .map_err(|error| error.to_string())?;
        self.builder.position_at_end(failed);
        // The body's implicit exception return is a null pointer. Do not
        // destroy a speculative non-null graph result or Promise here: its
        // ownership and subscribers belong to the body/host ABI.
        let fallback = if returns_promise {
            let rejected = self.builder.build_call(self.module.get_function("thaw_promise_new").unwrap(),
                &[], "rejected_callback_promise")
                .map_err(|error| error.to_string())?.try_as_basic_value().basic()
                .ok_or("thaw_promise_new returned no value")?.into_pointer_value();
            self.reject_promise_with_pending_exception(
                rejected, raised, "reject_callback_exception",
            )?;
            rejected
        } else {
            self.builder.build_call(self.module.get_function("thaw_json_callback_error").unwrap(),
                &[raised.into()], "frame_isolated_callback_exception")
                .map_err(|error| error.to_string())?.try_as_basic_value().basic()
                .ok_or("thaw_json_callback_error returned no value")?.into_pointer_value()
        };
        for (slot, value) in &previous {
            self.builder.build_store(slot.as_pointer_value(), *value)
                .map_err(|error| error.to_string())?;
        }
        self.builder.build_return(Some(&fallback)).map_err(|error| error.to_string())?;
        self.builder.position_at_end(succeeded);
        for (slot, value) in &previous {
            self.builder.build_store(slot.as_pointer_value(), *value)
                .map_err(|error| error.to_string())?;
        }
        self.builder.build_return(Some(&result)).map_err(|error| error.to_string())?;
        self.builder.position_at_end(parent);
        Ok(wrapper.as_global_value().as_pointer_value())
    }

    fn compile_native_promise_callback_finisher(
        &mut self,
        resolved: &HirType,
    ) -> Result<PointerValue<'ctx>, String> {
        let name = format!("__thaw_native_promise_finish_{}", self.next_lambda);
        self.next_lambda += 1;
        let ptr_type = self.context.ptr_type(AddressSpace::default());
        let function = self.module.add_function(
            &name,
            ptr_type.fn_type(&[ptr_type.into(), ptr_type.into()], false),
            Some(Linkage::Internal),
        );
        let return_block = self.builder.get_insert_block().unwrap();
        let outer_catch_stack = std::mem::take(&mut self.catch_stack);
        let outer_async_completion = self.active_async_completion.take();
        let entry = self.context.append_basic_block(function, "entry");
        self.builder.position_at_end(entry);
        let promise = function.get_nth_param(0).unwrap().into_pointer_value();
        let state = self
            .builder
            .build_call(
                self.module.get_function("thaw_promise_state").unwrap(),
                &[promise.into()],
                "native_promise_state",
            )
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .ok_or("thaw_promise_state returned no value")?
            .into_int_value();
        let pending = self
            .builder
            .build_int_compare(
                IntPredicate::EQ,
                state,
                self.context.i8_type().const_zero(),
                "native_promise_pending",
            )
            .map_err(|error| error.to_string())?;
        let pending_block = self.context.append_basic_block(function, "pending");
        let settled_block = self.context.append_basic_block(function, "settled");
        self.builder
            .build_conditional_branch(pending, pending_block, settled_block)
            .map_err(|error| error.to_string())?;
        self.builder.position_at_end(pending_block);
        self.builder
            .build_return(Some(&ptr_type.const_null()))
            .map_err(|error| error.to_string())?;
        self.builder.position_at_end(settled_block);
        let rejected = self
            .builder
            .build_int_compare(
                IntPredicate::EQ,
                state,
                self.context.i8_type().const_int(2, false),
                "native_promise_rejected",
            )
            .map_err(|error| error.to_string())?;
        let rejected_block = self.context.append_basic_block(function, "rejected");
        let fulfilled_block = self.context.append_basic_block(function, "fulfilled");
        self.builder
            .build_conditional_branch(rejected, rejected_block, fulfilled_block)
            .map_err(|error| error.to_string())?;
        self.builder.position_at_end(rejected_block);
        let error = self
            .builder
            .build_call(
                self.module
                    .get_function("thaw_runtime_run_until_resolved")
                    .unwrap(),
                &[promise.into()],
                "native_promise_error",
            )
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .ok_or("rejected native Promise has no error")?;
        self.builder
            .build_call(
                self.module.get_function("thaw_promise_destroy").unwrap(),
                &[promise.into()],
                "destroy_rejected_native_promise",
            )
            .map_err(|error| error.to_string())?;
        self.builder
            .build_return(Some(&error))
            .map_err(|error| error.to_string())?;
        self.builder.position_at_end(fulfilled_block);
        let mut result_array = None;
        let result_json = if *resolved == HirType::Void {
            self.drive_promise_to_completion(promise)?;
            self.compile_json_null()?
        } else {
            let value = self.drive_promise_to_resolved_value(promise, resolved)?;
            let array = self
                .builder
                .build_call(
                    self.module.get_function("thaw_json_array_new").unwrap(),
                    &[],
                    "native_promise_result_array",
                )
                .map_err(|error| error.to_string())?
                .try_as_basic_value()
                .basic()
                .unwrap();
            result_array = Some(array);
            // See the identical `preserve_undefined: true` reasoning at
            // this same function's synchronous counterpart above -- a
            // resolved `Promise<Optional<Json>>`/`Promise<Nullish<Json>>`
            // value needs the same real-`undefined`-vs-`null` distinction.
            self.compile_json_array_push_native_with_undefined(array, value, resolved, true)?;
            self.builder
                .build_call(
                    self.module.get_function("thaw_json_index").unwrap(),
                    &[
                        array.into(),
                        self.context.f64_type().const_zero().into(),
                        ptr_type.const_null().into(),
                    ],
                    "native_promise_result",
                )
                .map_err(|error| error.to_string())?
                .try_as_basic_value()
                .basic()
                .unwrap()
        };
        let result = self
            .builder
            .build_call(
                self.module.get_function("thaw_json_stringify").unwrap(),
                &[result_json.into()],
                "native_promise_result_string",
            )
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .unwrap();
        self.builder.build_call(
            self.module.get_function("thaw_json_destroy").unwrap(),
            &[result_json.into()],
            "destroy_native_promise_result_json",
        ).map_err(|error| error.to_string())?;
        if let Some(array) = result_array {
            self.builder.build_call(
                self.module.get_function("thaw_json_destroy").unwrap(),
                &[array.into()],
                "destroy_native_promise_result_array",
            ).map_err(|error| error.to_string())?;
        }
        let result = self.compile_check_json_stringify_callback_result(result)?;
        self.builder
            .build_return(Some(&result))
            .map_err(|error| error.to_string())?;
        self.catch_stack = outer_catch_stack;
        self.active_async_completion = outer_async_completion;
        self.builder.position_at_end(return_block);
        self.compile_host_callback_pending_guard(function, false)
    }

    /// Wraps a real compiled (native) closure as a live, retained QuickJS
    /// value the Fallback dynamic-call path can pass around like any
    /// other `JsValue` -- real example: zod's `z.number().refine((n:
    /// number) => n > 0, {...})`, whose predicate has nowhere to go
    /// without this (`coerce_to_declared`'s own doc comment on the
    /// thaw-hir side explains why a bare pass-through, the way `JsValue`
    /// itself gets, doesn't work here: there is no existing *live* QuickJS
    /// value yet, only a native function pointer that needs bridging into
    /// one first).
    ///
    /// Reuses `compile_napi_value_callback` as-is for the hard part (a
    /// generic `(context, args_json) -> result_json` adapter around the
    /// real closure, entirely backend-agnostic despite the name) rather
    /// than duplicating it -- the *only* new piece is handing that
    /// `(adapter, closure)` pair to QuickJS-NG (`thaw_js_register_native_
    /// callback`, thaw-quickjs) instead of to N-API, which wraps it in a
    /// real `rquickjs::Function` backed by a Rust closure that marshals a
    /// JS call into exactly the JSON-string-in/JSON-string-out shape the
    /// adapter expects, then retains and returns it the same way any
    /// other live value gets a permanent handle.
    fn compile_register_native_callback(
        &mut self,
        args: &[HirExpr],
        graph_mode: bool,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        self.uses_quickjs = true;
        self.uses_quickjs_handles = true;
        let [closure_expr] = args else {
            return Err("registerNativeCallback expects exactly one argument".into());
        };
        let (params, ret, optional, has_rest) = match self.expr_hir_type(closure_expr) {
            Some(HirType::Function(params, ret)) => (params, *ret, false, false),
            Some(HirType::CallableFunction(mut params, _, rest, ret)) => {
                let has_rest = rest.is_some();
                if let Some(rest) = rest {
                    params.push(HirType::Array(rest));
                }
                (params, *ret, false, has_rest)
            }
            Some(HirType::Optional(inner)) => match *inner {
                HirType::Function(params, ret) => (params, *ret, true, false),
                HirType::CallableFunction(mut params, _, rest, ret) => {
                    let has_rest = rest.is_some();
                    if let Some(rest) = rest {
                        params.push(HirType::Array(rest));
                    }
                    (params, *ret, true, has_rest)
                }
                _ => {
                    return Err(
                        "registerNativeCallback: could not determine the callback's own function type"
                            .into(),
                    );
                }
            },
            _ => {
                return Err(
                    "registerNativeCallback: could not determine the callback's own function type"
                        .into(),
                );
            }
        };
        // Tells the JS-side wrapper (`thaw_js_register_native_callback`,
        // thaw-quickjs) which argument positions to retain as a live
        // handle (encoded as the same `{"__thaw_js_handle_id__": N}`
        // marker `compile_dynamic_value_placeholder` builds for the
        // opposite direction) instead of naively `JSON.stringify`-ing --
        // see `compile_json_value_to_native`'s own new `HirType::JsValue`
        // case, the matching native-side decoder. A `u64` is plenty (32
        // real params would already be an extraordinary callback), and
        // JS's own bitwise operators only ever work on 32 bits anyway.
        let closure = self.compile_expr(closure_expr)?;
        if optional {
            let closure = closure.into_struct_value();
            let present = self.builder
                .build_extract_value(closure, 0, "optional_native_callback_present")
                .map_err(|error| error.to_string())?
                .into_int_value();
            let payload = self.builder
                .build_extract_value(closure, 1, "optional_native_callback")
                .map_err(|error| error.to_string())?
                .into_pointer_value();
            let function = self.current_function();
            let registered = self.context.append_basic_block(function, "native_callback_present");
            let absent = self.context.append_basic_block(function, "native_callback_absent");
            let done = self.context.append_basic_block(function, "native_callback_done");
            self.builder.build_conditional_branch(present, registered, absent)
                .map_err(|error| error.to_string())?;
            self.builder.position_at_end(registered);
            let value = self.compile_register_native_callback_from_closure_with_rest(
                payload, &params, &ret, has_rest, graph_mode,
            )?;
            let registered_end = self.builder.get_insert_block().unwrap();
            self.builder.build_unconditional_branch(done)
                .map_err(|error| error.to_string())?;
            self.builder.position_at_end(absent);
            let undefined = self.builder.build_global_string_ptr("undefined", "native_callback_undefined")
                .map_err(|error| error.to_string())?;
            let undefined = self.builder.build_call(
                self.module.get_function("thaw_js_get_global").unwrap(),
                &[undefined.as_pointer_value().into()],
                "native_callback_undefined_handle",
            ).map_err(|error| error.to_string())?.try_as_basic_value().basic()
                .ok_or("thaw_js_get_global returned no undefined handle")?;
            let absent_end = self.builder.get_insert_block().unwrap();
            self.builder.build_unconditional_branch(done)
                .map_err(|error| error.to_string())?;
            self.builder.position_at_end(done);
            let result = self.builder.build_phi(value.get_type(), "optional_native_callback_handle")
                .map_err(|error| error.to_string())?;
            result.add_incoming(&[(&value, registered_end), (&undefined, absent_end)]);
            Ok(result.as_basic_value())
        } else {
            self.compile_register_native_callback_from_closure_with_rest(
                closure.into_pointer_value(), &params, &ret, has_rest, graph_mode,
            )
        }
    }

    fn compile_quickjs_callback_argument(
        &mut self,
        array: BasicValueEnum<'ctx>,
        argument: &HirExpr,
    ) -> Result<(), String> {
        let Some(HirType::Optional(inner)) = self.expr_hir_type(argument) else {
            let value = self.compile_register_native_callback(std::slice::from_ref(argument), false)?;
            return self.compile_typed_dynamic_argument(array, value, &HirType::JsValue);
        };
        let (mut params, rest, ret) = match inner.as_ref() {
            HirType::Function(params, ret) => (params.clone(), None, ret.as_ref()),
            HirType::CallableFunction(params, _, rest, ret) => {
                (params.clone(), rest.as_ref(), ret.as_ref())
            }
            _ => return Err("optional QuickJS callback must be a function".into()),
        };
        let has_rest = rest.is_some();
        if let Some(rest) = rest {
            params.push(HirType::Array(rest.clone()));
        }
        let tagged = self.compile_expr(argument)?.into_struct_value();
        let present = self.builder
            .build_extract_value(tagged, 0, "quickjs_callback_present")
            .map_err(|error| error.to_string())?
            .into_int_value();
        let closure = self.builder
            .build_extract_value(tagged, 1, "quickjs_callback_payload")
            .map_err(|error| error.to_string())?
            .into_pointer_value();
        let function = self.current_function();
        let present_block = self.context.append_basic_block(function, "quickjs_callback_present");
        let absent_block = self.context.append_basic_block(function, "quickjs_callback_absent");
        let done = self.context.append_basic_block(function, "quickjs_callback_done");
        self.builder.build_conditional_branch(present, present_block, absent_block)
            .map_err(|error| error.to_string())?;

        self.builder.position_at_end(present_block);
        let callback = self.compile_register_native_callback_from_closure_with_rest(
            closure, &params, ret, has_rest, false,
        )?;
        self.compile_typed_dynamic_argument(array, callback, &HirType::JsValue)?;
        self.builder.build_unconditional_branch(done)
            .map_err(|error| error.to_string())?;

        self.builder.position_at_end(absent_block);
        let undefined = self.compile_napi_undefined_json()?;
        self.compile_json_array_push_owned(array, undefined)?;
        self.builder.build_unconditional_branch(done)
            .map_err(|error| error.to_string())?;
        self.builder.position_at_end(done);
        Ok(())
    }

    pub(super) fn compile_register_native_callback_from_closure(
        &mut self,
        closure: PointerValue<'ctx>,
        params: &[HirType],
        ret: &HirType,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        self.compile_register_native_callback_from_closure_with_rest(closure, params, ret, false, false)
    }

    /// Like `compile_register_native_callback_from_closure`, but for a
    /// callback whose *last* `params` entry (always `HirType::Array
    /// (element)` here -- callers already flattened a real
    /// `HirType::CallableFunction`'s rest parameter into this exact
    /// shape) should collect every *actual* trailing JS argument at call
    /// time, not just the one at that position. Without `has_rest`, the
    /// JS-side wrapper (`thaw_js_register_native_callback`, thaw-quickjs)
    /// sliced `arguments` down to a fixed `param_count` unconditionally
    /// -- for a rest callback called with more real arguments than the
    /// flattened ABI's param count (real trigger: better-sqlite3's own
    /// `DatabaseOptions.verbose?: (message?, ...rest) => void`, called
    /// with 2+ arguments), every argument past the cutoff was silently
    /// dropped, and the one argument landing in the rest slot was passed
    /// through as a bare scalar instead of the array the native decoder
    /// expects -- a real, reproducible crash/misdecoding, not just lost
    /// data.
    pub(super) fn compile_register_native_callback_from_closure_with_rest(
        &mut self,
        closure: PointerValue<'ctx>,
        params: &[HirType],
        ret: &HirType,
        has_rest: bool,
        graph_mode: bool,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        self.uses_quickjs = true;
        self.uses_quickjs_handles = true;
        let absent = self.builder.build_is_null(closure, "native_callback_is_undefined")
            .map_err(|error| error.to_string())?;
        let function = self.current_function();
        let present_block = self.context.append_basic_block(function, "native_callback_present_pointer");
        let absent_block = self.context.append_basic_block(function, "native_callback_absent_pointer");
        let done = self.context.append_basic_block(function, "native_callback_pointer_done");
        self.builder.build_conditional_branch(absent, absent_block, present_block)
            .map_err(|error| error.to_string())?;
        self.builder.position_at_end(present_block);
        let jsvalue_param_mask: u64 = params
            .iter()
            .enumerate()
            .filter(|(_, param)| {
                matches!(
                    param,
                    HirType::JsValue | HirType::Function(_, _) | HirType::CallableFunction(..)
                )
            })
            .map(|(index, _)| 1u64 << index)
            .sum();
        let (adapter, closure, finish) =
            self.compile_value_callback_from_closure(closure, params, ret, true, None, graph_mode, graph_mode)?;
        let jsvalue_param_mask = self
            .context
            .i64_type()
            .const_int(jsvalue_param_mask, false);
        let param_count = self
            .context
            .i64_type()
            .const_int(params.len() as u64, false);
        let void_result = matches!(ret, HirType::Void)
            || matches!(ret, HirType::Promise(value) if **value == HirType::Void);
        let void_result = self.context.i8_type().const_int(void_result as u64, false);
        let has_rest = self.context.i8_type().const_int(has_rest as u64, false);
        if graph_mode {
            // The graph callback decodes live JS handles into Json::Host.
            // Install the existing QuickJS operations on this callback thread
            // before any decoder or compiled replacer can touch them.
            let operations = [
                "thaw_js_retain_handle",
                "thaw_js_release_handle",
                "thaw_js_get_property_json_key_result",
                "thaw_js_host_query_result",
                "thaw_js_host_date_set_result",
                "thaw_js_set_property_graph_result",
                "thaw_js_property_predicate_json_key_result",
                "thaw_js_host_enumerate_result",
            ].iter().map(|name| self.module.get_function(name).unwrap()
                .as_global_value().as_pointer_value().into())
                .collect::<Vec<BasicMetadataValueEnum<'ctx>>>();
            self.builder.build_call(
                self.module.get_function("thaw_json_register_host_operations").unwrap(),
                &operations, "register_json_host_operations",
            ).map_err(|error| error.to_string())?;
        }
        let result = self
            .builder
            .build_call(
                self.module
                    .get_function(if graph_mode { "thaw_js_register_native_callback_graph" } else { "thaw_js_register_native_callback" })
                    .unwrap(),
                &[
                    adapter.into(),
                    closure.into(),
                    jsvalue_param_mask.into(),
                    param_count.into(),
                    void_result.into(),
                    finish.unwrap_or_else(|| self.context.ptr_type(AddressSpace::default()).const_null()).into(),
                    has_rest.into(),
                ],
                "register_native_callback",
            )
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .unwrap()
            .into_struct_value();
        let value = self
            .builder
            .build_extract_value(result, 0, "register_native_callback_value")
            .map_err(|error| error.to_string())?;
        let error = self
            .builder
            .build_extract_value(result, 1, "register_native_callback_error")
            .map_err(|error| error.to_string())?;
        self.builder
            .build_store(self.pending_exception().as_pointer_value(), error)
            .map_err(|error| error.to_string())?;
        self.clear_pending_native_text()?;
        self.mark_pending_native_text(error)?;
        self.branch_on_pending_exception()?;
        let present_end = self.builder.get_insert_block().unwrap();
        self.builder.build_unconditional_branch(done).map_err(|error| error.to_string())?;

        self.builder.position_at_end(absent_block);
        let undefined_name = self.builder.build_global_string_ptr("undefined", "native_callback_undefined_name")
            .map_err(|error| error.to_string())?;
        let undefined = self.builder.build_call(
            self.module.get_function("thaw_js_get_global").unwrap(),
            &[undefined_name.as_pointer_value().into()], "native_callback_undefined_handle",
        ).map_err(|error| error.to_string())?.try_as_basic_value().basic()
            .ok_or("thaw_js_get_global returned no undefined handle")?;
        let absent_end = self.builder.get_insert_block().unwrap();
        self.builder.build_unconditional_branch(done).map_err(|error| error.to_string())?;

        self.builder.position_at_end(done);
        let result = self.builder.build_phi(value.get_type(), "native_callback_handle")
            .map_err(|error| error.to_string())?;
        result.add_incoming(&[(&value, present_end), (&undefined, absent_end)]);
        Ok(result.as_basic_value())
    }

    fn compile_napi_undefined_json(&mut self) -> Result<BasicValueEnum<'ctx>, String> {
        let json = self
            .builder
            .build_call(
                self.module.get_function("thaw_json_object_new").unwrap(),
                &[],
                "napi_undefined_json",
            )
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .ok_or("thaw_json_object_new returned no value")?;
        let key = self
            .builder
            .build_global_string_ptr("$__thaw_napi_undefined$", "napi_undefined_key")
            .map_err(|error| error.to_string())?;
        self.builder
            .build_call(
                self.module
                    .get_function("thaw_json_object_set_bool")
                    .unwrap(),
                &[
                    json.into(),
                    key.as_pointer_value().into(),
                    self.context.i8_type().const_int(1, false).into(),
                ],
                "set_napi_undefined_tag",
            )
            .map_err(|error| error.to_string())?;
        let branded = self.builder.build_call(
            self.module.get_function("thaw_json_brand_wrapper").unwrap(),
            &[json.into()], "brand_napi_undefined",
        ).map_err(|error| error.to_string())?.try_as_basic_value().basic()
            .ok_or("thaw_json_brand_wrapper returned no value")?;
        Ok(branded)
    }

    fn compile_json_is_napi_undefined(
        &mut self,
        json: BasicValueEnum<'ctx>,
    ) -> Result<IntValue<'ctx>, String> {
        // Delegate to thaw-std's own exact predicate (`is_napi_undefined`:
        // the sentinel key must map to `true`) rather than merely checking
        // the key's presence via `thaw_json_has_own` -- an exotic input
        // like `{"$__thaw_napi_undefined$": false}` is a real object, not
        // `undefined`.
        let tagged = self
            .builder
            .build_call(
                self.module.get_function("thaw_json_is_undefined").unwrap(),
                &[json.into()],
                "json_is_napi_undefined",
            )
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .ok_or("thaw_json_is_undefined returned no value")?
            .into_int_value();
        self.builder
            .build_int_compare(
                IntPredicate::NE,
                tagged,
                self.context.i8_type().const_zero(),
                "json_is_napi_undefined_bool",
            )
            .map_err(|error| error.to_string())
    }

    fn compile_typed_dynamic_tagged_argument(
        &mut self,
        array: BasicValueEnum<'ctx>,
        tagged: StructValue<'ctx>,
        payload_type: &HirType,
        three_state: bool,
    ) -> Result<(), String> {
        let tag = self
            .builder
            .build_extract_value(tagged, 0, "napi_argument_tag")
            .map_err(|error| error.to_string())?
            .into_int_value();
        let payload = self
            .builder
            .build_extract_value(tagged, 1, "napi_argument_payload")
            .map_err(|error| error.to_string())?;
        let present = if three_state {
            self.builder
                .build_int_compare(
                    IntPredicate::EQ,
                    tag,
                    self.context.i8_type().const_zero(),
                    "napi_argument_has_value",
                )
                .map_err(|error| error.to_string())?
        } else {
            tag
        };
        let function = self.current_function();
        let value_block = self.context.append_basic_block(function, "napi_argument_value");
        let absent_block = self.context.append_basic_block(function, "napi_argument_absent");
        let done = self.context.append_basic_block(function, "napi_argument_done");
        self.builder
            .build_conditional_branch(present, value_block, absent_block)
            .map_err(|error| error.to_string())?;

        self.builder.position_at_end(value_block);
        self.compile_json_array_push_native_with_undefined(array, payload, payload_type, true)?;
        self.builder
            .build_unconditional_branch(done)
            .map_err(|error| error.to_string())?;

        self.builder.position_at_end(absent_block);
        let absent = if three_state {
            let null_block = self.context.append_basic_block(function, "napi_argument_null");
            let undefined_block = self
                .context
                .append_basic_block(function, "napi_argument_undefined");
            let absent_done = self
                .context
                .append_basic_block(function, "napi_argument_absent_done");
            let is_null = self
                .builder
                .build_int_compare(
                    IntPredicate::EQ,
                    tag,
                    self.context.i8_type().const_int(1, false),
                    "napi_argument_is_null",
                )
                .map_err(|error| error.to_string())?;
            self.builder
                .build_conditional_branch(is_null, null_block, undefined_block)
                .map_err(|error| error.to_string())?;
            self.builder.position_at_end(null_block);
            let null = self.compile_json_null()?;
            self.builder
                .build_unconditional_branch(absent_done)
                .map_err(|error| error.to_string())?;
            let null_end = self.builder.get_insert_block().ok_or("lost N-API null block")?;
            self.builder.position_at_end(undefined_block);
            let undefined = self.compile_napi_undefined_json()?;
            self.builder
                .build_unconditional_branch(absent_done)
                .map_err(|error| error.to_string())?;
            let undefined_end = self
                .builder
                .get_insert_block()
                .ok_or("lost N-API undefined block")?;
            self.builder.position_at_end(absent_done);
            let result = self
                .builder
                .build_phi(self.context.ptr_type(AddressSpace::default()), "napi_absent_json")
                .map_err(|error| error.to_string())?;
            result.add_incoming(&[(&null, null_end), (&undefined, undefined_end)]);
            result.as_basic_value()
        } else {
            self.compile_napi_undefined_json()?
        };
        self.builder
            .build_call(
                self.module
                    .get_function("thaw_json_array_push_json")
                    .unwrap(),
                &[array.into(), absent.into()],
                "push_napi_absent_argument",
            )
            .map_err(|error| error.to_string())?;
        self.builder
            .build_unconditional_branch(done)
            .map_err(|error| error.to_string())?;
        self.builder.position_at_end(done);
        Ok(())
    }

    fn compile_typed_dynamic_argument(
        &mut self,
        array: BasicValueEnum<'ctx>,
        value: BasicValueEnum<'ctx>,
        ty: &HirType,
    ) -> Result<(), String> {
        match ty {
            HirType::Optional(payload) => self.compile_typed_dynamic_tagged_argument(
                array,
                value.into_struct_value(),
                payload,
                false,
            ),
            HirType::Nullish(payload) => self.compile_typed_dynamic_tagged_argument(
                array,
                value.into_struct_value(),
                payload,
                true,
            ),
            HirType::Array(element) => {
                let json = self.compile_native_array_to_json_with_undefined(
                    value.into_pointer_value(),
                    element,
                    true,
                )?;
                self.compile_json_array_push_owned(array, json)
            }
            HirType::Bytes => {
                let object = self
                    .builder
                    .build_call(
                        self.module.get_function("thaw_json_object_new").unwrap(),
                        &[],
                        "dynamic_bytes",
                    )
                    .map_err(|error| error.to_string())?
                    .try_as_basic_value()
                    .basic()
                    .unwrap();
                let ty_key = self.builder.build_global_string_ptr("type", "bytes_type_key").map_err(|error| error.to_string())?;
                let ty_value = self.builder.build_global_string_ptr("Buffer", "bytes_type_value").map_err(|error| error.to_string())?;
                self.builder.build_call(
                    self.module.get_function("thaw_json_object_set_string").unwrap(),
                    &[object.into(), ty_key.as_pointer_value().into(), ty_value.as_pointer_value().into()],
                    "set_bytes_type",
                ).map_err(|error| error.to_string())?;
                let data = self.compile_native_array_to_json_with_undefined(
                    value.into_pointer_value(),
                    &HirType::F64,
                    true,
                )?;
                let data_key = self.builder.build_global_string_ptr("data", "bytes_data_key").map_err(|error| error.to_string())?;
                self.builder.build_call(
                    self.module.get_function("thaw_json_object_set_json").unwrap(),
                    &[object.into(), data_key.as_pointer_value().into(), data.into()],
                    "set_bytes_data",
                ).map_err(|error| error.to_string())?;
                self.builder
                    .build_call(
                        self.module.get_function("thaw_json_destroy").unwrap(),
                        &[data.into()],
                        "destroy_dynamic_bytes_data",
                    )
                    .map_err(|error| error.to_string())?;
                self.compile_json_array_push_owned(array, object)
            }
            HirType::Tuple(elements) => {
                let json = self.compile_native_tuple_to_json_with_undefined(
                    value.into_pointer_value(),
                    elements,
                    true,
                )?;
                self.compile_json_array_push_owned(array, json)
            }
            HirType::Object(_) => {
                let json = self.compile_native_object_to_json_with_undefined(
                    value.into_pointer_value(),
                    ty,
                    true,
                )?;
                self.compile_json_array_push_owned(array, json)
            }
            // A handle id pushed as a bare number reaches JS as one, so a
            // package's own `any`-declared parameter gets a number rather
            // than the live object (`invalid or released dynamic value
            // handle` the moment it is used). Encode it as the same
            // `{"__thaw_js_handle_id__": N}` placeholder the sibling
            // `Json`/`Optional(Json)` paths produce.
            HirType::JsValue | HirType::Dynamic => {
                let placeholder = self.compile_dynamic_value_placeholder_unchecked(value)?;
                self.compile_json_array_push_owned(array, placeholder)
            }
            _ => self.compile_json_array_push_native(array, value, ty),
        }
    }

    fn compile_json_array_push_owned(
        &mut self,
        array: BasicValueEnum<'ctx>,
        value: BasicValueEnum<'ctx>,
    ) -> Result<(), String> {
        self.compile_json_array_push_native(array, value, &HirType::Json)?;
        self.builder
            .build_call(
                self.module.get_function("thaw_json_destroy").unwrap(),
                &[value.into()],
                "destroy_marshaled_dynamic_argument",
            )
            .map_err(|error| error.to_string())?;
        Ok(())
    }

    fn compile_napi_optional_result_container(
        &mut self,
        json: BasicValueEnum<'ctx>,
    ) -> Result<(BasicValueEnum<'ctx>, PointerValue<'ctx>), String> {
        let object = self
            .builder
            .build_call(
                self.module.get_function("thaw_json_object_new").unwrap(),
                &[],
                "napi_optional_result_object",
            )
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .ok_or("thaw_json_object_new returned no value")?;
        self.builder.build_call(
            self.module.get_function("thaw_json_typed_decode_scope_own").unwrap(),
            &[object.into()], "own_optional_result_container",
        ).map_err(|error| error.to_string())?;
        let key = self
            .builder
            .build_global_string_ptr("value", "napi_optional_result_key")
            .map_err(|error| error.to_string())?
            .as_pointer_value();
        let is_undefined = self.compile_json_is_napi_undefined(json)?;
        let function = self.current_function();
        let absent = self.context.append_basic_block(function, "napi_result_undefined");
        let present = self.context.append_basic_block(function, "napi_result_present");
        let done = self.context.append_basic_block(function, "napi_result_optional_done");
        self.builder
            .build_conditional_branch(is_undefined, absent, present)
            .map_err(|error| error.to_string())?;
        self.builder.position_at_end(absent);
        self.builder
            .build_unconditional_branch(done)
            .map_err(|error| error.to_string())?;
        self.builder.position_at_end(present);
        self.builder
            .build_call(
                self.module
                    .get_function("thaw_json_object_set_json")
                    .unwrap(),
                &[object.into(), key.into(), json.into()],
                "set_napi_optional_result",
            )
            .map_err(|error| error.to_string())?;
        self.builder
            .build_unconditional_branch(done)
            .map_err(|error| error.to_string())?;
        self.builder.position_at_end(done);
        Ok((object, key))
    }

    /// Decodes a dynamic call's raw JSON result into a declared `Union`
    /// return type -- real examples: validator's `normalizeEmail(...):
    /// string | false` and protobufjs's `number | Long`. Every member must
    /// have a distinct runtime `typeof` category; ambiguous members use the
    /// first declared match. Picks
    /// the member whose runtime JS-visible category the JSON value
    /// actually has (via `thaw_json_typeof`/the napi-undefined sentinel
    /// check, the same primitives already used elsewhere in this file --
    /// `Null`/`Undefined` first, since `typeof null === "object"` would
    /// otherwise be indistinguishable from a real object), not the
    /// declared *order* -- a dynamic call's result is only ever known as
    /// a raw JSON value at this point, so there is no compile-time
    /// evidence to prefer one candidate member over another beyond what
    /// the value itself reports.
    /// A Union decoder borrows its JSON input even when a selected member is
    /// Promise<T>. Build the resolved Promise from a borrowed input; the outer
    /// Union alone releases that JSON Box after selecting a member.
    fn compile_borrowed_json_promise_member(
        &mut self,
        json: BasicValueEnum<'ctx>,
        resolved: &HirType,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let promise = self.builder
            .build_call(
                self.module.get_function("thaw_promise_new").unwrap(),
                &[],
                "dynamic_union_promise",
            )
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .ok_or("thaw_promise_new returned no value")?
            .into_pointer_value();
        let value = if *resolved == HirType::Null {
            self.context.bool_type().const_int(1, false).into()
        } else if *resolved == HirType::Undefined {
            self.context.bool_type().const_zero().into()
        } else {
            self.compile_json_value_to_native(json, resolved)?
        };
        let slot = self.allocate_arena_cell(self.basic_type(resolved)?, "dynamic_union_promise_result")?;
        self.builder
            .build_store(slot, value)
            .map_err(|error| error.to_string())?;
        self.builder
            .build_call(
                self.module.get_function("thaw_promise_resolve").unwrap(),
                &[promise.into(), slot.into()],
                "resolve_dynamic_union_promise",
            )
            .map_err(|error| error.to_string())?;
        Ok(promise.into())
    }

    fn compile_json_to_union_result(
        &mut self,
        json: BasicValueEnum<'ctx>,
        elements: &[HirType],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        // A `Promise<T>` member is real (e.g. ejs's own `render(...): string
        // | Promise<string>`, whose sync-vs-async return depends on an
        // `opts.async` flag no overload-dispatch narrows away) but needs no
        // special runtime discrimination here: `resolve_value_impl`/
        // `resolve_promise_value` (thaw-quickjs) already fully resolve any
        // real Promise a dynamic call's result carries *before* it reaches
        // this decoder, so `json` is always the final, already-awaited
        // value -- the same raw shape a plain `T` member would have. Only
        // `T` itself needs to be a supported scalar; the wrapping back into
        // a genuine (already-resolved) native Promise object reuses
        // `compile_typed_dynamic_result`'s own existing `HirType::Promise`
        // branch, just below.
        for element in elements {
            let scalar = match element {
                HirType::Promise(resolved) => resolved.as_ref(),
                other => other,
            };
            if !matches!(
                scalar,
                HirType::F64
                    | HirType::Str
                    | HirType::Bool
                    | HirType::Null
                    | HirType::Undefined
                    | HirType::Object(_)
                    | HirType::Array(_)
                    | HirType::Tuple(_)
                    | HirType::Function(..)
                    | HirType::CallableFunction(..)
            ) {
                return Err(format!(
                    "typed dynamic union return does not support member {element:?} yet"
                ));
            }
        }
        let function = self.current_function();
        let union_type = self.basic_type(&HirType::Union(elements.to_vec()))?;
        let result_slot = self
            .builder
            .build_alloca(union_type, "dynamic_union_result")
            .map_err(|error| error.to_string())?;
        let done = self.context.append_basic_block(function, "dynamic_union_done");
        let is_undefined = self.compile_json_is_napi_undefined(json)?;
        let is_null = self.compile_json_is_null_value(json)?;
        let typeof_string = self
            .builder
            .build_call(
                self.module.get_function("thaw_json_typeof").unwrap(),
                &[json.into()],
                "dynamic_union_typeof",
            )
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .ok_or("thaw_json_typeof returned no value")?
            .into_pointer_value();
        // An array/tuple member is discriminated the same way an object is
        // (`typeof === "object"`), so when a union carries one, `Array.isArray`
        // (the `thaw_json_is_array` runtime) is computed once and used both
        // to match that member and to keep an *object* member from claiming a
        // genuine array (real Node: an array is an object, but a union that
        // names both must resolve to its array member).
        let has_array_member = elements.iter().any(|element| {
            let scalar = match element {
                HirType::Promise(resolved) => resolved.as_ref(),
                other => other,
            };
            matches!(scalar, HirType::Array(_) | HirType::Tuple(_))
        });
        let is_array_json = if has_array_member {
            let raw = self
                .builder
                .build_call(
                    self.module.get_function("thaw_json_is_array").unwrap(),
                    &[json.into()],
                    "dynamic_union_is_array",
                )
                .map_err(|error| error.to_string())?
                .try_as_basic_value()
                .basic()
                .ok_or("thaw_json_is_array returned no value")?
                .into_int_value();
            Some(
                self.builder
                    .build_int_compare(
                        IntPredicate::NE,
                        raw,
                        raw.get_type().const_zero(),
                        "dynamic_union_is_array_bool",
                    )
                    .map_err(|error| error.to_string())?,
            )
        } else {
            None
        };
        let mut remaining: Option<inkwell::basic_block::BasicBlock<'ctx>> = None;
        for (index, element) in elements.iter().enumerate() {
            if let Some(block) = remaining {
                self.builder.position_at_end(block);
            }
            let scalar = match element {
                HirType::Promise(resolved) => resolved.as_ref(),
                other => other,
            };
            let matches = match scalar {
                HirType::Undefined => is_undefined,
                HirType::Null => is_null,
                HirType::F64 => self.compile_typeof_matches(typeof_string, "number")?,
                HirType::Str => self.compile_typeof_matches(typeof_string, "string")?,
                HirType::Bool => self.compile_typeof_matches(typeof_string, "boolean")?,
                HirType::Object(_) => {
                    let is_object = self.compile_typeof_matches(typeof_string, "object")?;
                    let is_not_null = self
                        .builder
                        .build_not(is_null, "dynamic_union_object_not_null")
                        .map_err(|error| error.to_string())?;
                    let base = self
                        .builder
                        .build_and(is_object, is_not_null, "dynamic_union_is_object")
                        .map_err(|error| error.to_string())?;
                    // Only exclude arrays when the union also names an
                    // array/tuple member (otherwise an object member keeps
                    // accepting an array, as before).
                    match is_array_json {
                        Some(is_array) => {
                            let not_array = self
                                .builder
                                .build_not(is_array, "dynamic_union_object_not_array")
                                .map_err(|error| error.to_string())?;
                            self.builder
                                .build_and(base, not_array, "dynamic_union_is_plain_object")
                                .map_err(|error| error.to_string())?
                        }
                        None => base,
                    }
                }
                HirType::Array(_) | HirType::Tuple(_) => {
                    let is_object = self.compile_typeof_matches(typeof_string, "object")?;
                    let is_not_null = self
                        .builder
                        .build_not(is_null, "dynamic_union_array_not_null")
                        .map_err(|error| error.to_string())?;
                    let base = self
                        .builder
                        .build_and(is_object, is_not_null, "dynamic_union_is_object_for_array")
                        .map_err(|error| error.to_string())?;
                    let is_array = is_array_json
                        .ok_or("array union member requires array discrimination")?;
                    self.builder
                        .build_and(base, is_array, "dynamic_union_is_array_member")
                        .map_err(|error| error.to_string())?
                }
                // A returned JS function's own `typeof` can't tell apart
                // two callable members that differ only in *their own*
                // return type (real example: ejs's own `compile(...):
                // TemplateFunction | AsyncTemplateFunction`, `(data?:
                // Data) => string | Promise<string>` depending on a flag
                // no overload-dispatch narrows away) -- whichever callable
                // member is declared first always wins for a real JS
                // function value, matching this whole codebase's existing
                // "first declared wins when the runtime value alone can't
                // disambiguate" convention.
                HirType::Function(..) | HirType::CallableFunction(..) => {
                    self.compile_typeof_matches(typeof_string, "function")?
                }
                _ => unreachable!("checked above"),
            };
            let matched_block = self
                .context
                .append_basic_block(function, "dynamic_union_matched");
            let next_block = self
                .context
                .append_basic_block(function, "dynamic_union_next");
            self.builder
                .build_conditional_branch(matches, matched_block, next_block)
                .map_err(|error| error.to_string())?;
            self.builder.position_at_end(matched_block);
            let native = if let HirType::Promise(resolved) = element {
                self.compile_borrowed_json_promise_member(json, resolved)?
            } else {
                self.compile_json_value_to_native(json, element)?
            };
            let value = self.build_union_value(native, index, elements)?;
            self.builder
                .build_store(result_slot, value)
                .map_err(|error| error.to_string())?;
            self.builder
                .build_unconditional_branch(done)
                .map_err(|error| error.to_string())?;
            remaining = Some(next_block);
        }
        // Nothing matched (a value genuinely outside every declared
        // member -- shouldn't happen for an honestly-typed package, but
        // this function never panics/aborts elsewhere either): falls
        // back to the *last* declared member, decoding the same raw
        // value against it regardless of its actual runtime shape
        // (matching every individual scalar decoder's own "degrade to a
        // default instead of crashing" philosophy, `thaw-std`'s
        // `json.rs`).
        if let Some(block) = remaining {
            self.builder.position_at_end(block);
            let index = elements.len() - 1;
            let native = if let HirType::Promise(resolved) = &elements[index] {
                self.compile_borrowed_json_promise_member(json, resolved)?
            } else {
                self.compile_json_value_to_native(json, &elements[index])?
            };
            let value = self.build_union_value(native, index, elements)?;
            self.builder
                .build_store(result_slot, value)
                .map_err(|error| error.to_string())?;
            self.builder
                .build_unconditional_branch(done)
                .map_err(|error| error.to_string())?;
        }
        self.builder.position_at_end(done);
        self.builder
            .build_load(union_type, result_slot, "dynamic_union_value")
            .map_err(|error| error.to_string())
    }

    /// `strcmp(typeof_string, expected) == 0` -- the same primitive
    /// `compile_dynamic_prop_access` already uses to compare a runtime
    /// key against a known name, reused here to compare `typeof`'s own
    /// result against a known category name.
    fn compile_typeof_matches(
        &mut self,
        typeof_string: PointerValue<'ctx>,
        expected: &str,
    ) -> Result<IntValue<'ctx>, String> {
        let expected = self
            .builder
            .build_global_string_ptr(expected, "dynamic_union_typeof_expected")
            .map_err(|error| error.to_string())?;
        let comparison = self
            .builder
            .build_call(
                self.module.get_function("strcmp").unwrap(),
                &[typeof_string.into(), expected.as_pointer_value().into()],
                "dynamic_union_typeof_compare",
            )
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .ok_or("strcmp returned no value")?
            .into_int_value();
        self.builder
            .build_int_compare(
                IntPredicate::EQ,
                comparison,
                comparison.get_type().const_zero(),
                "dynamic_union_typeof_is_match",
            )
            .map_err(|error| error.to_string())
    }

    fn compile_discard_typed_decode_scope(&mut self) -> Result<(), String> {
        // Dropping a Host child may re-enter compiled code. The original
        // exception tuple must win over any cleanup callback's own state.
        let ptr = self.context.ptr_type(AddressSpace::default());
        let globals = [
            (self.pending_exception().as_pointer_value(), ptr.into(), ptr.const_null().into()),
            (self.pending_exception_native_text().as_pointer_value(), ptr.into(), ptr.const_null().into()),
            (self.pending_exception_object().as_pointer_value(), ptr.into(), ptr.const_null().into()),
            (self.pending_exception_aggregate_errors().as_pointer_value(), ptr.into(), ptr.const_null().into()),
            (self.pending_exception_value(PENDING_EXCEPTION_VALUE_TAG_SYMBOL).as_pointer_value(),
                self.context.i64_type().into(), self.context.i64_type().const_zero().into()),
            (self.pending_exception_value(PENDING_EXCEPTION_F64_SYMBOL).as_pointer_value(),
                self.context.f64_type().into(), self.context.f64_type().const_zero().into()),
            (self.pending_exception_value(PENDING_EXCEPTION_I64_SYMBOL).as_pointer_value(),
                self.context.i64_type().into(), self.context.i64_type().const_zero().into()),
            (self.pending_exception_value(PENDING_EXCEPTION_BOOL_SYMBOL).as_pointer_value(),
                self.context.bool_type().into(), self.context.bool_type().const_zero().into()),
        ];
        let mut saved = Vec::with_capacity(globals.len());
        for (global, ty, empty) in globals {
            saved.push((global, self.builder.build_load(ty, global, "decode_exception_state")
                .map_err(|error| error.to_string())?));
            self.builder.build_store(global, empty).map_err(|error| error.to_string())?;
        }
        self.builder.build_call(
            self.module.get_function("thaw_json_typed_decode_scope_end").unwrap(),
            &[self.context.i8_type().const_int(1, false).into()], "discard_typed_decode_scope",
        ).map_err(|error| error.to_string())?;
        let cleanup_native = self.builder.build_load(ptr,
            self.pending_exception_native_text().as_pointer_value(), "cleanup_native_text")
            .map_err(|error| error.to_string())?.into_pointer_value();
        let original_native = saved[1].1.into_pointer_value();
        let new_native = self.builder.build_is_not_null(cleanup_native, "has_cleanup_native_text")
            .map_err(|error| error.to_string())?;
        let distinct = self.builder.build_int_compare(IntPredicate::NE,
            self.builder.build_ptr_to_int(cleanup_native, self.context.i64_type(), "cleanup_native_word")
                .map_err(|error| error.to_string())?,
            self.builder.build_ptr_to_int(original_native, self.context.i64_type(), "original_native_word")
                .map_err(|error| error.to_string())?, "new_cleanup_native_text")
            .map_err(|error| error.to_string())?;
        let dispose = self.builder.build_and(new_native, distinct, "dispose_cleanup_error")
            .map_err(|error| error.to_string())?;
        let function = self.current_function();
        let release = self.context.append_basic_block(function, "release_cleanup_error");
        let restore = self.context.append_basic_block(function, "restore_decode_exception");
        self.builder.build_conditional_branch(dispose, release, restore)
            .map_err(|error| error.to_string())?;
        self.builder.position_at_end(release);
        self.builder.build_call(self.module.get_function("thaw_cstring_destroy").unwrap(),
            &[cleanup_native.into()], "destroy_cleanup_native_error")
            .map_err(|error| error.to_string())?;
        self.builder.build_unconditional_branch(restore).map_err(|error| error.to_string())?;
        self.builder.position_at_end(restore);
        for (global, value) in saved {
            self.builder.build_store(global, value).map_err(|error| error.to_string())?;
        }
        Ok(())
    }

    fn typed_result_embeds_input_json(ty: &HirType) -> bool {
        match ty {
            HirType::Json | HirType::Dictionary(_) => true,
            HirType::Optional(payload) | HirType::Nullable(payload)
            | HirType::Nullish(payload) => Self::typed_result_embeds_input_json(payload),
            _ => false,
        }
    }

    fn typed_result_null_is_absent(ty: &HirType) -> bool {
        match ty {
            HirType::Nullable(_) | HirType::Nullish(_) => true,
            HirType::Optional(payload) => Self::typed_result_null_is_absent(payload),
            _ => false,
        }
    }

    fn compile_typed_input_absent(
        &mut self,
        json: BasicValueEnum<'ctx>,
        ty: &HirType,
    ) -> Result<IntValue<'ctx>, String> {
        let is_undefined = self.compile_json_is_napi_undefined(json)?;
        if Self::typed_result_null_is_absent(ty) {
            let is_null = self.compile_json_is_null_value(json)?;
            self.builder.build_or(is_null, is_undefined, "typed_json_input_absent")
                .map_err(|error| error.to_string())
        } else { Ok(is_undefined) }
    }

    fn compile_destroy_typed_input_if_detached(
        &mut self,
        json: BasicValueEnum<'ctx>,
        payload: &HirType,
        absent: Option<IntValue<'ctx>>,
    ) -> Result<(), String> {
        if Self::typed_result_embeds_input_json(payload) {
            let Some(absent) = absent else { return Ok(()) };
            let function = self.current_function();
            let release = self.context.append_basic_block(function, "release_absent_typed_input");
            let done = self.context.append_basic_block(function, "retain_present_typed_input");
            self.builder.build_conditional_branch(absent, release, done)
                .map_err(|error| error.to_string())?;
            self.builder.position_at_end(release);
            self.builder.build_call(self.module.get_function("thaw_json_destroy").unwrap(),
                &[json.into()], "destroy_absent_typed_input")
                .map_err(|error| error.to_string())?;
            self.builder.build_unconditional_branch(done).map_err(|error| error.to_string())?;
            self.builder.position_at_end(done);
        } else {
            self.builder.build_call(self.module.get_function("thaw_json_destroy").unwrap(),
                &[json.into()], "destroy_detached_typed_input")
                .map_err(|error| error.to_string())?;
        }
        Ok(())
    }

    fn compile_typed_dynamic_result(
        &mut self,
        json: BasicValueEnum<'ctx>,
        ty: &HirType,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let function = self.current_function();
        let cleanup = self.context.append_basic_block(function, "typed_decode_failed");
        let done = self.context.append_basic_block(function, "typed_decode_done");
        self.builder.build_call(
            self.module.get_function("thaw_json_typed_decode_scope_begin").unwrap(),
            &[], "begin_typed_decode_scope",
        ).map_err(|error| error.to_string())?;
        self.builder.build_call(
            self.module.get_function("thaw_json_typed_decode_scope_own").unwrap(),
            &[json.into()], "own_typed_decode_input",
        ).map_err(|error| error.to_string())?;
        self.catch_stack.push(cleanup);
        let value = self.compile_typed_dynamic_result_inner(json, ty);
        self.catch_stack.pop();
        let value = value?;
        self.builder.build_call(
            self.module.get_function("thaw_json_typed_decode_scope_end").unwrap(),
            &[self.context.i8_type().const_int(2, false).into()], "merge_typed_decode_scope",
        ).map_err(|error| error.to_string())?;
        self.builder.build_unconditional_branch(done).map_err(|error| error.to_string())?;
        self.builder.position_at_end(cleanup);
        self.compile_discard_typed_decode_scope()?;
        self.branch_on_pending_exception()?;
        self.builder.build_unreachable().map_err(|error| error.to_string())?;
        self.builder.position_at_end(done);
        Ok(value)
    }

    fn compile_typed_dynamic_result_inner(
        &mut self,
        json: BasicValueEnum<'ctx>,
        ty: &HirType,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        match ty {
            HirType::F64 | HirType::Str | HirType::Bool => {
                let value = match ty {
                    HirType::F64 => self.compile_json_as_value(json, "thaw_json_as_number")?,
                    HirType::Str => self.compile_json_as_value(json, "thaw_json_as_string")?,
                    HirType::Bool => self.compile_json_as_bool_value(json)?,
                    _ => unreachable!(),
                };
                self.builder
                    .build_call(
                        self.module.get_function("thaw_json_destroy").unwrap(),
                        &[json.into()],
                        "destroy_typed_dynamic_scalar",
                    )
                    .map_err(|error| error.to_string())?;
                Ok(value)
            }
            HirType::Json => Ok(json),
            HirType::Dictionary(_) => Ok(json),
            HirType::Bytes => {
                let key = self.builder.build_global_string_ptr("data", "bytes_result_data_key").map_err(|error| error.to_string())?;
                let data = self.builder.build_call(
                    self.module.get_function("thaw_json_get").unwrap(),
                    &[json.into(), key.as_pointer_value().into()],
                    "dynamic_bytes_data",
                ).map_err(|error| error.to_string())?.try_as_basic_value().basic().unwrap();
                self.compile_check_json_host_error(data, Some("thaw_json_destroy"))?;
                let result = self.builder.build_call(
                    self.module.get_function("thaw_json_to_number_array").unwrap(),
                    &[data.into()],
                    "dynamic_bytes_result",
                ).map_err(|error| error.to_string())?.try_as_basic_value().basic().ok_or("thaw_json_to_number_array returned no value")?.into_pointer_value();
                let wrapped = self.compile_array_wrap(result)?;
                self.builder.build_call(self.module.get_function("thaw_json_destroy").unwrap(),
                    &[data.into()], "destroy_dynamic_bytes_data")
                    .map_err(|error| error.to_string())?;
                self.compile_destroy_typed_input_if_detached(json, ty, None)?;
                Ok(wrapped.into())
            }
            // A real, declared union return -- e.g. validator's own
            // `normalizeEmail(...): string | false`. Every member here
            // must be a plain scalar (`F64`/`Str`/`Bool`/`Null`/
            // `Undefined`) this function already knows how to decode
            // individually just above/below -- `compile_json_to_union_
            // result` picks the one the *runtime* JSON value's own
            // JS-visible type actually matches (via `thaw_json_typeof`,
            // the same primitive `compile_dynamic_prop_access` already
            // uses to compare against a known set), not the compile-time
            // declared shape, since a dynamic call's result is only ever
            // known as a raw JSON value at this point. Found via
            // validator: without this, *any* Fallback function declared
            // with such a union return crashed the whole build outright
            // (not just a call to it -- every declared Fallback function
            // gets compiled unconditionally, whether the user's own code
            // ever calls it or not).
            HirType::Union(elements) => {
                let value = self.compile_json_to_union_result(json, elements)?;
                self.compile_destroy_decoded_owned_json(json, ty)?;
                Ok(value)
            },
            HirType::Optional(payload) => {
                let (object, key) = self.compile_napi_optional_result_container(json)?;
                let value = self.compile_json_to_optional_field(object, key, json, payload, false)?;
                self.builder.build_call(self.module.get_function("thaw_json_destroy").unwrap(),
                    &[object.into()], "destroy_optional_result_container")
                    .map_err(|error| error.to_string())?;
                let absent = Self::typed_result_embeds_input_json(payload)
                    .then(|| self.compile_typed_input_absent(json, ty)).transpose()?;
                self.compile_destroy_typed_input_if_detached(json, payload, absent)?;
                Ok(value)
            }
            HirType::Nullable(payload) => {
                let value = self.compile_json_to_nullable_field(json, payload)?;
                let absent = Self::typed_result_embeds_input_json(payload)
                    .then(|| self.compile_typed_input_absent(json, ty)).transpose()?;
                self.compile_destroy_typed_input_if_detached(json, payload, absent)?;
                Ok(value)
            }
            HirType::Nullish(payload) => {
                let (object, key) = self.compile_napi_optional_result_container(json)?;
                let value = self.compile_json_to_nullish_field(object, key, json, payload)?;
                self.builder.build_call(self.module.get_function("thaw_json_destroy").unwrap(),
                    &[object.into()], "destroy_nullish_result_container")
                    .map_err(|error| error.to_string())?;
                let absent = Self::typed_result_embeds_input_json(payload)
                    .then(|| self.compile_typed_input_absent(json, ty)).transpose()?;
                self.compile_destroy_typed_input_if_detached(json, payload, absent)?;
                Ok(value)
            }
            HirType::Array(element) if **element == HirType::F64 => {
                let result = self
                    .builder
                    .build_call(
                        self.module
                            .get_function("thaw_json_to_number_array")
                            .unwrap(),
                        &[json.into()],
                        "dynamic_number_array_result",
                    )
                    .map_err(|error| error.to_string())?
                    .try_as_basic_value()
                    .basic()
                    .ok_or_else(|| "thaw_json_to_number_array returned no value".to_string())?
                    .into_pointer_value();
                let wrapped = self.compile_array_wrap(result)?;
                self.compile_destroy_typed_input_if_detached(json, ty, None)?;
                Ok(wrapped.into())
            }
            HirType::Array(element) if dynamic_json_collection_element_supported(element) => {
                let value = self.compile_json_to_native_array(json, element)?;
                self.compile_destroy_typed_input_if_detached(json, ty, None)?;
                Ok(value)
            }
            HirType::Tuple(elements) => {
                let value = self.compile_json_to_native_tuple(json, elements)?;
                self.compile_destroy_typed_input_if_detached(json, ty, None)?;
                Ok(value)
            }
            HirType::Object(_) => {
                let value = self.compile_json_to_native_object(json, ty)?;
                self.compile_destroy_typed_input_if_detached(json, ty, None)?;
                Ok(value)
            },
            HirType::Promise(resolved) => {
                let promise = self
                    .builder
                    .build_call(
                        self.module.get_function("thaw_promise_new").unwrap(),
                        &[],
                        "dynamic_promise",
                    )
                    .map_err(|error| error.to_string())?
                    .try_as_basic_value()
                    .basic()
                    .ok_or("thaw_promise_new returned no value")?
                    .into_pointer_value();
                let payload = if **resolved == HirType::Void {
                    self.compile_destroy_typed_input_if_detached(json, resolved, None)?;
                    self.context.ptr_type(AddressSpace::default()).const_null()
                } else {
                    let value = self.compile_typed_dynamic_result(json, resolved)?;
                    let slot = self.allocate_arena_cell(self.basic_type(resolved)?, "dynamic_promise_result")?;
                    self.builder
                        .build_store(slot, value)
                        .map_err(|error| error.to_string())?;
                    slot
                };
                self.builder
                    .build_call(
                        self.module.get_function("thaw_promise_resolve").unwrap(),
                        &[promise.into(), payload.into()],
                        "resolve_dynamic_promise",
                    )
                    .map_err(|error| error.to_string())?;
                Ok(promise.into())
            }
            // A `void`-returning dynamic call still marshals a JSON result
            // back across the boundary (there's no "no value" JSON
            // representation to special-case on the JS side), but the
            // caller has nothing to do with it: a `void`-declared
            // function's own return codegen discards whatever value its
            // return expression produced and emits a bare `build_return
            // (None)` regardless (see `compile_ignored_this_adapter` for
            // the same pattern), so any placeholder value is fine here.
            HirType::Void => {
                self.builder
                    .build_call(
                        self.module.get_function("thaw_json_destroy").unwrap(),
                        &[json.into()],
                        "destroy_typed_dynamic_void",
                    )
                    .map_err(|error| error.to_string())?;
                Ok(self.context.i32_type().const_zero().into())
            }
            other => Err(format!("typed dynamic return does not support {other:?} yet")),
        }
    }

}
