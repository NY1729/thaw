impl<'ctx> HirCompiler<'ctx> {
    fn compile_promise_resolver(
        &mut self,
        resolved: &HirType,
        reject: bool,
        assimilates: bool,
    ) -> Result<FunctionValue<'ctx>, String> {
        let name = format!(
            "__thaw_promise_{}_{}",
            if reject { "reject" } else { "resolve" },
            self.next_lambda
        );
        self.next_lambda += 1;
        let param = if reject {
            HirType::Str
        } else if assimilates {
            HirType::Promise(Box::new(resolved.clone()))
        } else {
            resolved.clone()
        };
        let params = if !reject && !assimilates && resolved == &HirType::Void {
            &[][..]
        } else {
            std::slice::from_ref(&param)
        };
        let function_type = self.function_type(params, &HirType::Void)?;
        let function = self
            .module
            .add_function(&name, function_type, Some(Linkage::Internal));
        let return_block = self.builder.get_insert_block().unwrap();
        let entry = self.context.append_basic_block(function, "entry");
        self.builder.position_at_end(entry);
        let environment = function.get_nth_param(0).unwrap().into_pointer_value();
        let offset = self
            .context
            .i64_type()
            .const_int(CLOSURE_CAPTURE_BASE, false);
        let promise_slot = unsafe {
            self.builder
                .build_in_bounds_gep(
                    self.context.i8_type(),
                    environment,
                    &[offset],
                    "promise_capture",
                )
                .map_err(|error| error.to_string())?
        };
        let promise = self
            .builder
            .build_load(
                self.context.ptr_type(AddressSpace::default()),
                promise_slot,
                "promise",
            )
            .map_err(|error| error.to_string())?;
        let payload = if !reject && !assimilates && resolved == &HirType::Void {
            self.context.ptr_type(AddressSpace::default()).const_null()
        } else if reject || assimilates {
            function.get_nth_param(1).unwrap().into_pointer_value()
        } else {
            let value = function.get_nth_param(1).unwrap();
            let slot = self.allocate_arena_cell(self.basic_type(resolved)?, "promise_result")?;
            self.builder
                .build_store(slot, value)
                .map_err(|error| error.to_string())?;
            slot
        };
        let settle = if reject {
            "thaw_promise_reject"
        } else if assimilates {
            "thaw_promise_adopt"
        } else {
            "thaw_promise_resolve"
        };
        self.builder
            .build_call(
                self.module.get_function(settle).unwrap(),
                &[promise.into(), payload.into()],
                "settle_promise",
            )
            .map_err(|error| error.to_string())?;
        self.builder.build_return(None).map_err(|e| e.to_string())?;
        self.builder.position_at_end(return_block);
        Ok(function)
    }

    fn compile_promise_new(
        &mut self,
        executor: &HirExpr,
        resolved: &HirType,
        assimilates: bool,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let promise = self
            .builder
            .build_call(
                self.module.get_function("thaw_promise_new").unwrap(),
                &[],
                "promise_new",
            )
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .unwrap()
            .into_pointer_value();
        let executor = self.compile_expr(executor)?.into_pointer_value();
        let resolve_fn = self.compile_promise_resolver(resolved, false, assimilates)?;
        let reject_fn = self.compile_promise_resolver(resolved, true, false)?;
        let resolve = self.allocate_special_closure(resolve_fn, promise, "resolve_closure")?;
        let reject = self.allocate_special_closure(reject_fn, promise, "reject_closure")?;
        let code = self
            .builder
            .build_load(
                self.context.ptr_type(AddressSpace::default()),
                executor,
                "executor_code",
            )
            .map_err(|error| error.to_string())?
            .into_pointer_value();
        let resolve_params = if assimilates {
            vec![HirType::Promise(Box::new(resolved.clone()))]
        } else if resolved == &HirType::Void {
            Vec::new()
        } else {
            vec![resolved.clone()]
        };
        let resolve_ty = HirType::Function(resolve_params, Box::new(HirType::Void));
        let reject_ty = HirType::Function(vec![HirType::Str], Box::new(HirType::Void));
        let executor_type = self.function_type(&[resolve_ty, reject_ty], &HirType::Void)?;
        self.builder
            .build_indirect_call(
                executor_type,
                code,
                &[executor.into(), resolve.into(), reject.into()],
                "invoke_promise_executor",
            )
            .map_err(|error| error.to_string())?;
        let pending_slot = self.pending_exception().as_pointer_value();
        let pending = self
            .builder
            .build_load(
                self.context.ptr_type(AddressSpace::default()),
                pending_slot,
                "executor_error",
            )
            .map_err(|error| error.to_string())?
            .into_pointer_value();
        let function = self
            .builder
            .get_insert_block()
            .unwrap()
            .get_parent()
            .unwrap();
        let rejected = self
            .context
            .append_basic_block(function, "executor_rejected");
        let complete = self
            .context
            .append_basic_block(function, "executor_complete");
        let has_error = self
            .builder
            .build_is_not_null(pending, "executor_has_error")
            .map_err(|error| error.to_string())?;
        self.builder
            .build_conditional_branch(has_error, rejected, complete)
            .map_err(|error| error.to_string())?;
        self.builder.position_at_end(rejected);
        self.builder
            .build_call(
                self.module.get_function("thaw_promise_reject").unwrap(),
                &[promise.into(), pending.into()],
                "reject_executor_throw",
            )
            .map_err(|error| error.to_string())?;
        self.builder
            .build_store(
                pending_slot,
                self.context.ptr_type(AddressSpace::default()).const_null(),
            )
            .map_err(|error| error.to_string())?;
        self.builder
            .build_unconditional_branch(complete)
            .map_err(|error| error.to_string())?;
        self.builder.position_at_end(complete);
        Ok(promise.into())
    }

    fn compile_promise_then(
        &mut self,
        source: &HirExpr,
        callback: &HirExpr,
        input: &HirType,
        output: &HirType,
        on_rejected: bool,
        flatten: bool,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let source = self.compile_expr(source)?.into_pointer_value();
        let closure = self.compile_expr(callback)?.into_pointer_value();
        let adapter_name = format!("__thaw_promise_chain_{}", self.next_lambda);
        self.next_lambda += 1;
        let ptr = self.context.ptr_type(AddressSpace::default());
        let adapter_type = self
            .context
            .void_type()
            .fn_type(&[ptr.into(), ptr.into(), ptr.into()], false);
        let adapter =
            self.module
                .add_function(&adapter_name, adapter_type, Some(Linkage::Internal));
        let return_block = self.builder.get_insert_block().unwrap();
        let entry = self.context.append_basic_block(adapter, "entry");
        self.builder.position_at_end(entry);
        let context = adapter.get_nth_param(0).unwrap().into_pointer_value();
        let promise = adapter.get_nth_param(1).unwrap();
        let result = adapter.get_nth_param(2).unwrap().into_pointer_value();
        let callback_input = if on_rejected { &HirType::Str } else { input };
        let value = if !on_rejected && callback_input == &HirType::Void {
            None
        } else if on_rejected {
            Some(result.into())
        } else {
            Some(
                self.builder
                    .build_load(self.basic_type(callback_input)?, result, "chain_input")
                    .map_err(|error| error.to_string())?,
            )
        };
        let code = self
            .builder
            .build_load(ptr, context, "chain_code")
            .map_err(|error| error.to_string())?
            .into_pointer_value();
        let callback_return = if flatten {
            HirType::Promise(Box::new(output.clone()))
        } else {
            output.clone()
        };
        let callback_params = if callback_input == &HirType::Void {
            &[][..]
        } else {
            std::slice::from_ref(callback_input)
        };
        let callback_type = self.function_type(callback_params, &callback_return)?;
        let mut callback_args = vec![context.into()];
        if let Some(value) = value {
            callback_args.push(value.into());
        }
        let transformed = self
            .builder
            .build_indirect_call(callback_type, code, &callback_args, "invoke_chain_callback")
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic();
        if transformed.is_none() && (flatten || output != &HirType::Void) {
            return Err("Promise callback must return a value".into());
        }
        let pending_slot = self.pending_exception().as_pointer_value();
        let pending = self
            .builder
            .build_load(ptr, pending_slot, "chain_error")
            .map_err(|error| error.to_string())?
            .into_pointer_value();
        let failed = self.context.append_basic_block(adapter, "callback_failed");
        let succeeded = self
            .context
            .append_basic_block(adapter, "callback_succeeded");
        let has_error = self
            .builder
            .build_is_not_null(pending, "callback_has_error")
            .map_err(|error| error.to_string())?;
        self.builder
            .build_conditional_branch(has_error, failed, succeeded)
            .map_err(|error| error.to_string())?;
        self.builder.position_at_end(failed);
        self.builder
            .build_call(
                self.module.get_function("thaw_promise_reject").unwrap(),
                &[promise.into(), pending.into()],
                "reject_chain",
            )
            .map_err(|error| error.to_string())?;
        self.builder
            .build_store(pending_slot, ptr.const_null())
            .map_err(|error| error.to_string())?;
        self.builder.build_return(None).map_err(|e| e.to_string())?;
        self.builder.position_at_end(succeeded);
        if flatten {
            self.builder
                .build_call(
                    self.module.get_function("thaw_promise_adopt").unwrap(),
                    &[promise.into(), transformed.unwrap().into()],
                    "adopt_chain",
                )
                .map_err(|error| error.to_string())?;
        } else if output == &HirType::Void {
            self.builder
                .build_call(
                    self.module.get_function("thaw_promise_resolve").unwrap(),
                    &[promise.into(), ptr.const_null().into()],
                    "resolve_void_chain",
                )
                .map_err(|error| error.to_string())?;
        } else {
            let output_type = self.basic_type(output)?;
            let output_slot = self.allocate_arena_cell(output_type, "chain_output")?;
            self.builder
                .build_store(output_slot, transformed.unwrap())
                .map_err(|error| error.to_string())?;
            self.builder
                .build_call(
                    self.module.get_function("thaw_promise_resolve").unwrap(),
                    &[promise.into(), output_slot.into()],
                    "resolve_chain",
                )
                .map_err(|error| error.to_string())?;
        }
        self.builder.build_return(None).map_err(|e| e.to_string())?;
        self.builder.position_at_end(return_block);
        self.builder
            .build_call(
                self.module.get_function("thaw_promise_chain").unwrap(),
                &[
                    source.into(),
                    adapter.as_global_value().as_pointer_value().into(),
                    closure.into(),
                    self.context
                        .i8_type()
                        .const_int(u64::from(on_rejected), false)
                        .into(),
                ],
                "promise_chain",
            )
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .ok_or_else(|| "thaw_promise_chain returned no value".to_string())
    }

    fn compile_promise_finally(
        &mut self,
        source: &HirExpr,
        callback: &HirExpr,
        _input: &HirType,
        callback_return: &HirType,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let source = self.compile_expr(source)?.into_pointer_value();
        let closure = self.compile_expr(callback)?.into_pointer_value();
        let name = format!("__thaw_promise_finally_{}", self.next_lambda);
        self.next_lambda += 1;
        let ptr = self.context.ptr_type(AddressSpace::default());
        let i8_type = self.context.i8_type();
        let adapter_type = self
            .context
            .void_type()
            .fn_type(&[ptr.into(), ptr.into(), ptr.into(), i8_type.into()], false);
        let adapter = self
            .module
            .add_function(&name, adapter_type, Some(Linkage::Internal));
        let return_block = self.builder.get_insert_block().unwrap();
        let entry = self.context.append_basic_block(adapter, "entry");
        self.builder.position_at_end(entry);
        let context = adapter.get_nth_param(0).unwrap().into_pointer_value();
        let output = adapter.get_nth_param(1).unwrap();
        let original = adapter.get_nth_param(2).unwrap();
        let original_rejected = adapter.get_nth_param(3).unwrap().into_int_value();
        let code = self
            .builder
            .build_load(ptr, context, "finally_code")
            .map_err(|error| error.to_string())?
            .into_pointer_value();
        let callback_type = self.function_type(&[], callback_return)?;
        let call = self
            .builder
            .build_indirect_call(callback_type, code, &[context.into()], "invoke_finally")
            .map_err(|error| error.to_string())?;
        let returned = call.try_as_basic_value().basic();
        let pending_slot = self.pending_exception().as_pointer_value();
        let pending = self
            .builder
            .build_load(ptr, pending_slot, "finally_error")
            .map_err(|error| error.to_string())?
            .into_pointer_value();
        let failed = self.context.append_basic_block(adapter, "finally_failed");
        let succeeded = self
            .context
            .append_basic_block(adapter, "finally_succeeded");
        let has_error = self
            .builder
            .build_is_not_null(pending, "finally_has_error")
            .map_err(|error| error.to_string())?;
        self.builder
            .build_conditional_branch(has_error, failed, succeeded)
            .map_err(|error| error.to_string())?;
        self.builder.position_at_end(failed);
        self.builder
            .build_call(
                self.module.get_function("thaw_promise_reject").unwrap(),
                &[output.into(), pending.into()],
                "reject_finally",
            )
            .map_err(|error| error.to_string())?;
        self.builder
            .build_store(pending_slot, ptr.const_null())
            .map_err(|error| error.to_string())?;
        self.builder.build_return(None).map_err(|e| e.to_string())?;
        self.builder.position_at_end(succeeded);
        if matches!(callback_return, HirType::Promise(_)) {
            let returned = returned
                .ok_or("Promise-returning finally callback produced no value")?
                .into_pointer_value();
            self.builder
                .build_call(
                    self.module
                        .get_function("thaw_promise_finally_adopt")
                        .unwrap(),
                    &[
                        output.into(),
                        returned.into(),
                        original.into(),
                        original_rejected.into(),
                    ],
                    "wait_finally_promise",
                )
                .map_err(|error| error.to_string())?;
            self.builder.build_return(None).map_err(|e| e.to_string())?;
        } else {
            let reject = self
                .context
                .append_basic_block(adapter, "forward_rejection");
            let resolve = self
                .context
                .append_basic_block(adapter, "forward_fulfillment");
            let rejected = self
                .builder
                .build_int_compare(
                    IntPredicate::NE,
                    original_rejected,
                    i8_type.const_zero(),
                    "original_rejected",
                )
                .map_err(|error| error.to_string())?;
            self.builder
                .build_conditional_branch(rejected, reject, resolve)
                .map_err(|error| error.to_string())?;
            self.builder.position_at_end(reject);
            self.builder
                .build_call(
                    self.module.get_function("thaw_promise_reject").unwrap(),
                    &[output.into(), original.into()],
                    "forward_finally_rejection",
                )
                .map_err(|error| error.to_string())?;
            self.builder.build_return(None).map_err(|e| e.to_string())?;
            self.builder.position_at_end(resolve);
            self.builder
                .build_call(
                    self.module.get_function("thaw_promise_resolve").unwrap(),
                    &[output.into(), original.into()],
                    "forward_finally_fulfillment",
                )
                .map_err(|error| error.to_string())?;
            self.builder.build_return(None).map_err(|e| e.to_string())?;
        }
        self.builder.position_at_end(return_block);
        self.builder
            .build_call(
                self.module.get_function("thaw_promise_finally").unwrap(),
                &[
                    source.into(),
                    adapter.as_global_value().as_pointer_value().into(),
                    closure.into(),
                ],
                "promise_finally",
            )
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .ok_or_else(|| "thaw_promise_finally returned no value".to_string())
    }

    fn compile_sleep(&mut self, args: &[HirExpr]) -> Result<BasicValueEnum<'ctx>, String> {
        let [milliseconds] = args else {
            return Err("sleep expects exactly one millisecond argument".to_string());
        };
        let milliseconds = self.compile_expr(milliseconds)?.into_float_value();
        let milliseconds = self
            .builder
            .build_float_to_unsigned_int(milliseconds, self.context.i64_type(), "sleep_ms")
            .map_err(|e| e.to_string())?;
        let sleep = self.module.get_function("thaw_sleep_ms").unwrap();
        self.builder
            .build_call(sleep, &[milliseconds.into()], "sleep_promise")
            .map_err(|e| e.to_string())?
            .try_as_basic_value()
            .basic()
            .ok_or_else(|| "thaw_sleep_ms did not return a promise".to_string())
    }

    fn compile_promise_all(
        &mut self,
        args: &[HirExpr],
        element: &HirType,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let ptr_type = self.context.ptr_type(AddressSpace::default());
        let i64_type = self.context.i64_type();
        let promises = if args.is_empty() {
            ptr_type.const_null()
        } else {
            let arena_alloc = self.module.get_function("thaw_arena_alloc").unwrap();
            let storage = self
                .builder
                .build_call(
                    arena_alloc,
                    &[
                        i64_type.const_int((args.len() * 8) as u64, false).into(),
                        i64_type.const_int(8, false).into(),
                    ],
                    "promise_all_storage",
                )
                .map_err(|error| error.to_string())?
                .try_as_basic_value()
                .basic()
                .unwrap()
                .into_pointer_value();
            for (index, arg) in args.iter().enumerate() {
                let promise = self.compile_expr(arg)?.into_pointer_value();
                let slot = unsafe {
                    self.builder
                        .build_in_bounds_gep(
                            ptr_type,
                            storage,
                            &[i64_type.const_int(index as u64, false)],
                            "promise_all_slot",
                        )
                        .map_err(|error| error.to_string())?
                };
                self.builder
                    .build_store(slot, promise)
                    .map_err(|error| error.to_string())?;
            }
            storage
        };
        self.builder
            .build_call(
                self.module.get_function("thaw_promise_all_slots").unwrap(),
                &[
                    promises.into(),
                    i64_type.const_int(args.len() as u64, false).into(),
                    i64_type
                        .const_int(array_element_storage_bytes(element), false)
                        .into(),
                ],
                "promise_all",
            )
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .ok_or_else(|| "thaw_promise_all_slots did not return a promise".to_string())
    }

    fn compile_promise_all_array(
        &mut self,
        array: &HirExpr,
        element: &HirType,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let i64_type = self.context.i64_type();
        let handle = self.compile_expr(array)?.into_pointer_value();
        let base = self.compile_array_data(handle)?;
        let len = self
            .builder
            .build_load(i64_type, base, "promise_all_len")
            .map_err(|error| error.to_string())?
            .into_int_value();
        let promises = unsafe {
            self.builder
                .build_in_bounds_gep(
                    self.context.i8_type(),
                    base,
                    &[i64_type.const_int(ARRAY_HEADER_BYTES, false)],
                    "promise_all_values",
                )
                .map_err(|error| error.to_string())?
        };
        self.builder
            .build_call(
                self.module.get_function("thaw_promise_all_slots").unwrap(),
                &[
                    promises.into(),
                    len.into(),
                    i64_type
                        .const_int(array_element_storage_bytes(element), false)
                        .into(),
                ],
                "promise_all_array",
            )
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .ok_or_else(|| "thaw_promise_all_slots did not return a promise".to_string())
    }

    fn compile_promise_all_tuple(
        &mut self,
        args: &[HirExpr],
        elements: &[HirType],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        if args.len() != elements.len() {
            return Err("Promise.all tuple value/type arity mismatch".into());
        }
        let ptr_type = self.context.ptr_type(AddressSpace::default());
        let i64_type = self.context.i64_type();
        let arena_alloc = self.module.get_function("thaw_arena_alloc").unwrap();
        let allocate_words = |compiler: &mut Self, words: usize, name: &str| {
            compiler
                .builder
                .build_call(
                    arena_alloc,
                    &[
                        i64_type.const_int((words * 8) as u64, false).into(),
                        i64_type.const_int(8, false).into(),
                    ],
                    name,
                )
                .map_err(|error| error.to_string())?
                .try_as_basic_value()
                .basic()
                .ok_or_else(|| format!("{name} allocation returned void"))
                .map(|value| value.into_pointer_value())
        };
        let promises = allocate_words(self, args.len(), "promise_all_tuple_promises")?;
        let sizes = allocate_words(self, elements.len(), "promise_all_tuple_sizes")?;
        for (index, (arg, element)) in args.iter().zip(elements).enumerate() {
            let offset = i64_type.const_int(index as u64, false);
            let promise_slot = unsafe {
                self.builder
                    .build_in_bounds_gep(ptr_type, promises, &[offset], "promise_tuple_slot")
                    .map_err(|error| error.to_string())?
            };
            let promise = self.compile_expr(arg)?.into_pointer_value();
            self.builder
                .build_store(promise_slot, promise)
                .map_err(|error| error.to_string())?;
            let size_slot = unsafe {
                self.builder
                    .build_in_bounds_gep(i64_type, sizes, &[offset], "promise_tuple_size")
                    .map_err(|error| error.to_string())?
            };
            self.builder
                .build_store(
                    size_slot,
                    i64_type.const_int(array_element_storage_bytes(element), false),
                )
                .map_err(|error| error.to_string())?;
        }
        self.builder
            .build_call(
                self.module.get_function("thaw_promise_all_typed").unwrap(),
                &[
                    promises.into(),
                    sizes.into(),
                    i64_type.const_int(args.len() as u64, false).into(),
                ],
                "promise_all_tuple",
            )
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .ok_or_else(|| "thaw_promise_all_typed did not return a promise".to_string())
    }

    fn compile_promise_all_settled(
        &mut self,
        args: &[HirExpr],
        element: &HirType,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let ptr_type = self.context.ptr_type(AddressSpace::default());
        let i64_type = self.context.i64_type();
        let promises = if args.is_empty() {
            ptr_type.const_null()
        } else {
            let arena_alloc = self.module.get_function("thaw_arena_alloc").unwrap();
            let storage = self
                .builder
                .build_call(
                    arena_alloc,
                    &[
                        i64_type.const_int((args.len() * 8) as u64, false).into(),
                        i64_type.const_int(8, false).into(),
                    ],
                    "promise_all_settled_storage",
                )
                .map_err(|error| error.to_string())?
                .try_as_basic_value()
                .basic()
                .ok_or("Promise.allSettled storage allocation returned void")?
                .into_pointer_value();
            for (index, arg) in args.iter().enumerate() {
                let slot = unsafe {
                    self.builder
                        .build_in_bounds_gep(
                            ptr_type,
                            storage,
                            &[i64_type.const_int(index as u64, false)],
                            "promise_all_settled_slot",
                        )
                        .map_err(|error| error.to_string())?
                };
                let promise = self.compile_expr(arg)?.into_pointer_value();
                self.builder
                    .build_store(slot, promise)
                    .map_err(|error| error.to_string())?;
            }
            storage
        };
        self.build_promise_all_settled_call(
            promises,
            i64_type.const_int(args.len() as u64, false),
            element,
            "promise_all_settled",
        )
    }

    fn compile_promise_all_settled_array(
        &mut self,
        array: &HirExpr,
        element: &HirType,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let i64_type = self.context.i64_type();
        let handle = self.compile_expr(array)?.into_pointer_value();
        let base = self.compile_array_data(handle)?;
        let len = self
            .builder
            .build_load(i64_type, base, "promise_all_settled_len")
            .map_err(|error| error.to_string())?
            .into_int_value();
        let promises = unsafe {
            self.builder
                .build_in_bounds_gep(
                    self.context.i8_type(),
                    base,
                    &[i64_type.const_int(ARRAY_HEADER_BYTES, false)],
                    "promise_all_settled_values",
                )
                .map_err(|error| error.to_string())?
        };
        self.build_promise_all_settled_call(promises, len, element, "promise_all_settled_array")
    }

    fn build_promise_all_settled_call(
        &mut self,
        promises: PointerValue<'ctx>,
        len: IntValue<'ctx>,
        element: &HirType,
        name: &str,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let i64_type = self.context.i64_type();
        self.builder
            .build_call(
                self.module
                    .get_function("thaw_promise_all_settled")
                    .unwrap(),
                &[
                    promises.into(),
                    len.into(),
                    i64_type
                        .const_int(array_element_storage_bytes(element), false)
                        .into(),
                ],
                name,
            )
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .ok_or_else(|| "thaw_promise_all_settled did not return a promise".to_string())
    }

    fn compile_promise_race(&mut self, args: &[HirExpr]) -> Result<BasicValueEnum<'ctx>, String> {
        self.compile_first_settled_combinator(args, "thaw_promise_race", "promise_race")
    }

    fn compile_promise_any(&mut self, args: &[HirExpr]) -> Result<BasicValueEnum<'ctx>, String> {
        self.compile_first_settled_combinator(args, "thaw_promise_any", "promise_any")
    }

    fn compile_first_settled_combinator(
        &mut self,
        args: &[HirExpr],
        runtime_symbol: &str,
        label: &str,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let ptr_type = self.context.ptr_type(AddressSpace::default());
        let i64_type = self.context.i64_type();
        let arena_alloc = self.module.get_function("thaw_arena_alloc").unwrap();
        let storage = self
            .builder
            .build_call(
                arena_alloc,
                &[
                    i64_type.const_int((args.len() * 8) as u64, false).into(),
                    i64_type.const_int(8, false).into(),
                ],
                &format!("{label}_storage"),
            )
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .ok_or_else(|| format!("{label} storage allocation returned void"))?
            .into_pointer_value();
        for (index, arg) in args.iter().enumerate() {
            let slot = unsafe {
                self.builder
                    .build_in_bounds_gep(
                        ptr_type,
                        storage,
                        &[i64_type.const_int(index as u64, false)],
                        &format!("{label}_slot"),
                    )
                    .map_err(|error| error.to_string())?
            };
            let promise = self.compile_expr(arg)?.into_pointer_value();
            self.builder
                .build_store(slot, promise)
                .map_err(|error| error.to_string())?;
        }
        self.builder
            .build_call(
                self.module.get_function(runtime_symbol).unwrap(),
                &[
                    storage.into(),
                    i64_type.const_int(args.len() as u64, false).into(),
                ],
                label,
            )
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .ok_or_else(|| format!("{runtime_symbol} did not return a promise"))
    }

    fn compile_promise_race_array(
        &mut self,
        array: &HirExpr,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        self.compile_first_settled_array(array, "thaw_promise_race", "promise_race")
    }

    fn compile_promise_any_array(
        &mut self,
        array: &HirExpr,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        self.compile_first_settled_array(array, "thaw_promise_any", "promise_any")
    }

    fn compile_first_settled_array(
        &mut self,
        array: &HirExpr,
        runtime_symbol: &str,
        label: &str,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let i64_type = self.context.i64_type();
        let handle = self.compile_expr(array)?.into_pointer_value();
        let base = self.compile_array_data(handle)?;
        let len = self
            .builder
            .build_load(i64_type, base, &format!("{label}_len"))
            .map_err(|error| error.to_string())?
            .into_int_value();
        let promises = unsafe {
            self.builder
                .build_in_bounds_gep(
                    self.context.i8_type(),
                    base,
                    &[i64_type.const_int(ARRAY_HEADER_BYTES, false)],
                    &format!("{label}_values"),
                )
                .map_err(|error| error.to_string())?
        };
        self.builder
            .build_call(
                self.module.get_function(runtime_symbol).unwrap(),
                &[promises.into(), len.into()],
                &format!("{label}_array"),
            )
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .ok_or_else(|| format!("{runtime_symbol} did not return a promise"))
    }

    fn compile_await(&mut self, inner: &HirExpr) -> Result<BasicValueEnum<'ctx>, String> {
        let is_sleep = matches!(
            inner,
            HirExpr::Call(callee, _) if matches!(callee.as_ref(), HirExpr::Var(name) if name == "sleep")
        );
        let value = self.compile_expr(inner)?;
        if !is_sleep {
            // User-defined async functions still use the V1 synchronous ABI.
            return Ok(value);
        }

        let promise = value.into_pointer_value();
        let run_until = self
            .module
            .get_function("thaw_runtime_run_until_resolved")
            .unwrap();
        let result = self
            .builder
            .build_call(run_until, &[promise.into()], "await_result")
            .map_err(|e| e.to_string())?
            .try_as_basic_value()
            .basic()
            .ok_or("thaw_runtime_run_until_resolved did not return a value")?
            .into_pointer_value();
        let resolved = self
            .builder
            .build_is_not_null(result, "await_resolved")
            .map_err(|e| e.to_string())?;
        Ok(resolved.into())
    }

    /// Drives a typed Promise that remains inside a synchronous closure (for
    /// example the selected branch of a short-circuit expression) and loads
    /// its native payload. Ordinary async-function awaits are frame-split
    /// before reaching this fallback.
    fn compile_typed_blocking_await(
        &mut self,
        inner: &HirExpr,
        resolved: &HirType,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let promise = self.compile_expr(inner)?.into_pointer_value();
        self.drive_promise_to_resolved_value(promise, resolved)
    }

    /// The value-taking half of `compile_typed_blocking_await`, for a
    /// caller that already has a compiled `Promise<T>` pointer in hand
    /// (rather than an `HirExpr` to compile) -- e.g. a native callback's
    /// raw return value from an indirect call
    /// (`compile_napi_value_callback`). Drives it to completion, checks
    /// fulfilled/rejected state, loads the resolved payload as
    /// `resolved`'s native representation (propagating a rejection as a
    /// pending thaw exception the same way an ordinary `await` does), and
    /// destroys the promise.
    fn drive_promise_to_resolved_value(
        &mut self,
        promise: PointerValue<'ctx>,
        resolved: &HirType,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let result = self
            .builder
            .build_call(
                self.module
                    .get_function("thaw_runtime_run_until_resolved")
                    .unwrap(),
                &[promise.into()],
                "await_typed_result",
            )
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .ok_or("typed Promise did not settle")?
            .into_pointer_value();
        let state = self
            .builder
            .build_call(
                self.module.get_function("thaw_promise_state").unwrap(),
                &[promise.into()],
                "blocking_await_state",
            )
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .ok_or("typed Promise has no state")?
            .into_int_value();
        let rejected = self
            .builder
            .build_int_compare(
                IntPredicate::EQ,
                state,
                self.context.i8_type().const_int(2, false),
                "blocking_await_rejected",
            )
            .map_err(|error| error.to_string())?;
        let function = self.current_function();
        let failed = self
            .context
            .append_basic_block(function, "blocking_await_failed");
        let succeeded = self
            .context
            .append_basic_block(function, "blocking_await_succeeded");
        let merge = self
            .context
            .append_basic_block(function, "blocking_await_merge");
        self.builder
            .build_conditional_branch(rejected, failed, succeeded)
            .map_err(|error| error.to_string())?;

        let llvm_type = self.basic_type(resolved)?;
        self.builder.position_at_end(failed);
        self.builder
            .build_store(self.pending_exception().as_pointer_value(), result)
            .map_err(|error| error.to_string())?;
        let default = llvm_type.const_zero();
        self.builder
            .build_unconditional_branch(merge)
            .map_err(|error| error.to_string())?;
        self.builder.position_at_end(succeeded);
        let value = self
            .builder
            .build_load(llvm_type, result, "await_typed_value")
            .map_err(|error| error.to_string())?;
        self.builder
            .build_unconditional_branch(merge)
            .map_err(|error| error.to_string())?;
        self.builder.position_at_end(merge);
        let phi = self
            .builder
            .build_phi(llvm_type, "blocking_await_value")
            .map_err(|error| error.to_string())?;
        phi.add_incoming(&[(&default, failed), (&value, succeeded)]);
        self.builder
            .build_call(
                self.module.get_function("thaw_promise_destroy").unwrap(),
                &[promise.into()],
                "destroy_blocking_await",
            )
            .map_err(|error| error.to_string())?;
        self.branch_on_pending_exception()?;
        Ok(phi.as_basic_value())
    }

    /// The `Promise<void>` counterpart to `drive_promise_to_resolved_value`,
    /// for a resolved type with no native representation to `build_load`
    /// (`void` carries no payload). Drives the promise to completion,
    /// propagates a rejection as a pending thaw exception the same way,
    /// and destroys the promise -- without attempting to load a value.
    fn drive_promise_to_completion(&mut self, promise: PointerValue<'ctx>) -> Result<(), String> {
        let result = self
            .builder
            .build_call(
                self.module
                    .get_function("thaw_runtime_run_until_resolved")
                    .unwrap(),
                &[promise.into()],
                "await_void_result",
            )
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .ok_or("typed Promise did not settle")?
            .into_pointer_value();
        let state = self
            .builder
            .build_call(
                self.module.get_function("thaw_promise_state").unwrap(),
                &[promise.into()],
                "blocking_await_void_state",
            )
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .ok_or("typed Promise has no state")?
            .into_int_value();
        let rejected = self
            .builder
            .build_int_compare(
                IntPredicate::EQ,
                state,
                self.context.i8_type().const_int(2, false),
                "blocking_await_void_rejected",
            )
            .map_err(|error| error.to_string())?;
        let function = self.current_function();
        let failed = self
            .context
            .append_basic_block(function, "blocking_await_void_failed");
        let merge = self
            .context
            .append_basic_block(function, "blocking_await_void_merge");
        self.builder
            .build_conditional_branch(rejected, failed, merge)
            .map_err(|error| error.to_string())?;
        self.builder.position_at_end(failed);
        self.builder
            .build_store(self.pending_exception().as_pointer_value(), result)
            .map_err(|error| error.to_string())?;
        self.builder
            .build_unconditional_branch(merge)
            .map_err(|error| error.to_string())?;
        self.builder.position_at_end(merge);
        self.builder
            .build_call(
                self.module.get_function("thaw_promise_destroy").unwrap(),
                &[promise.into()],
                "destroy_blocking_await_void",
            )
            .map_err(|error| error.to_string())?;
        self.branch_on_pending_exception()
    }
}
