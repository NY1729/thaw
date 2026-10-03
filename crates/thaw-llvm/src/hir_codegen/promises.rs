impl<'ctx> HirCompiler<'ctx> {
    fn compile_promise_resolver(
        &mut self,
        resolved: &HirType,
        reject: bool,
        assimilates: bool,
        typed_rejection: bool,
        mixed: bool,
    ) -> Result<FunctionValue<'ctx>, String> {
        let name = format!(
            "__thaw_promise_{}_{}",
            if reject { "reject" } else { "resolve" },
            self.next_lambda
        );
        self.next_lambda += 1;
        let param = if reject {
            HirType::Str
        } else if mixed && matches!(resolved, HirType::Void | HirType::Undefined) {
            HirType::Optional(Box::new(HirType::Promise(Box::new(resolved.clone()))))
        } else if mixed && resolved == &HirType::Null {
            HirType::Nullable(Box::new(HirType::Promise(Box::new(HirType::Null))))
        } else if mixed {
            HirType::Union(vec![
                resolved.clone(),
                HirType::Promise(Box::new(resolved.clone())),
            ])
        } else if assimilates {
            HirType::Promise(Box::new(resolved.clone()))
        } else {
            resolved.clone()
        };
        let params = if !reject && !assimilates && !mixed && resolved == &HirType::Void {
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
        let state_slot = unsafe {
            self.builder
                .build_in_bounds_gep(
                    self.context.i8_type(),
                    environment,
                    &[offset],
                    "resolver_state_capture",
                )
                .map_err(|error| error.to_string())?
        };
        let state = self
            .builder
            .build_load(
                self.context.ptr_type(AddressSpace::default()),
                state_slot,
                "resolver_state",
            )
            .map_err(|error| error.to_string())?
            .into_pointer_value();
        let called_slot = unsafe {
            self.builder
                .build_in_bounds_gep(
                    self.context.i8_type(),
                    state,
                    &[self.context.i64_type().const_int(8, false)],
                    "resolver_called_slot",
                )
                .map_err(|error| error.to_string())?
        };
        let called = self
            .builder
            .build_load(self.context.i8_type(), called_slot, "resolver_called")
            .map_err(|error| error.to_string())?
            .into_int_value();
        let first_call = self
            .context
            .append_basic_block(function, "resolver_first_call");
        let complete = self.context.append_basic_block(function, "resolver_complete");
        let uncalled = self
            .builder
            .build_int_compare(
                IntPredicate::EQ,
                called,
                self.context.i8_type().const_zero(),
                "resolver_uncalled",
            )
            .map_err(|error| error.to_string())?;
        self.builder
            .build_conditional_branch(uncalled, first_call, complete)
            .map_err(|error| error.to_string())?;
        self.builder.position_at_end(first_call);
        self.builder
            .build_store(called_slot, self.context.i8_type().const_int(1, false))
            .map_err(|error| error.to_string())?;
        let promise = self
            .builder
            .build_load(self.context.ptr_type(AddressSpace::default()), state, "promise")
            .map_err(|error| error.to_string())?;
        if mixed && !reject {
            self.compile_mixed_promise_settlement(function, promise.into_pointer_value(), resolved)?;
            self.builder
                .build_unconditional_branch(complete)
                .map_err(|error| error.to_string())?;
            self.builder.position_at_end(complete);
            self.builder.build_return(None).map_err(|error| error.to_string())?;
            self.builder.position_at_end(return_block);
            return Ok(function);
        }
        let payload = if !reject && !assimilates && resolved == &HirType::Void {
            self.context.ptr_type(AddressSpace::default()).const_null()
        } else if reject || assimilates {
            function.get_nth_param(1).unwrap().into_pointer_value()
        } else {
            let value = function.get_nth_param(1).unwrap();
            let value_type = self.basic_type(resolved)?;
            let slot = self.build_arena_cell(&self.builder, value_type, "promise_result")?;
            self.builder
                .build_store(slot, value)
                .map_err(|error| error.to_string())?;
            slot
        };
        let settle = if assimilates {
            "thaw_promise_adopt"
        } else {
            "thaw_promise_resolve"
        };
        if reject {
            if !typed_rejection {
                self.builder.build_store(self.pending_exception_aggregate_errors().as_pointer_value(),
                    self.context.ptr_type(AddressSpace::default()).const_null())
                    .map_err(|error| error.to_string())?;
                self.builder
                    .build_store(
                        self.pending_exception_value(PENDING_EXCEPTION_VALUE_TAG_SYMBOL)
                            .as_pointer_value(),
                        self.context.i64_type().const_int(4, false),
                    )
                    .map_err(|error| error.to_string())?;
                self.builder
                    .build_store(
                        self.pending_exception_object().as_pointer_value(),
                        self.context.ptr_type(AddressSpace::default()).const_null(),
                    )
                    .map_err(|error| error.to_string())?;
            }
            self.reject_promise_with_pending_exception(
                promise.into_pointer_value(),
                payload,
                "settle_promise",
            )?;
        } else {
            self.builder
                .build_call(
                    self.module.get_function(settle).unwrap(),
                    &[promise.into(), payload.into()],
                    "settle_promise",
                )
                .map_err(|error| error.to_string())?;
        }
        self.builder
            .build_unconditional_branch(complete)
            .map_err(|error| error.to_string())?;
        self.builder.position_at_end(complete);
        self.builder.build_return(None).map_err(|e| e.to_string())?;
        self.builder.position_at_end(return_block);
        Ok(function)
    }

    fn compile_mixed_promise_settlement(
        &mut self,
        function: FunctionValue<'ctx>,
        promise: PointerValue<'ctx>,
        resolved: &HirType,
    ) -> Result<(), String> {
        let argument = function.get_nth_param(1).unwrap().into_struct_value();
        let discriminator = self.builder
            .build_extract_value(argument, 0, "resolve_kind")
            .map_err(|error| error.to_string())?
            .into_int_value();
        let is_adoption = if matches!(resolved, HirType::Void | HirType::Undefined | HirType::Null) {
            discriminator
        } else {
            self.builder
                .build_int_compare(
                    IntPredicate::EQ,
                    discriminator,
                    discriminator.get_type().const_int(1, false),
                    "resolve_is_adoption",
                )
                .map_err(|error| error.to_string())?
        };
        let adopt = self.context.append_basic_block(function, "resolve_adopt");
        let plain = self.context.append_basic_block(function, "resolve_plain");
        let done = self.context.append_basic_block(function, "resolve_done");
        self.builder
            .build_conditional_branch(is_adoption, adopt, plain)
            .map_err(|error| error.to_string())?;

        self.builder.position_at_end(adopt);
        let adopted = if matches!(resolved, HirType::Void | HirType::Undefined | HirType::Null) {
            self.builder
                .build_extract_value(argument, 1, "resolve_promise")
                .map_err(|error| error.to_string())?
                .into_pointer_value()
        } else {
            let bits = self.builder
                .build_extract_value(argument, 1, "resolve_promise_bits")
                .map_err(|error| error.to_string())?
                .into_int_value();
            self.unpack_union_payload(bits, &HirType::Promise(Box::new(resolved.clone())))?
                .into_pointer_value()
        };
        self.builder
            .build_call(
                self.module.get_function("thaw_promise_adopt").unwrap(),
                &[promise.into(), adopted.into()],
                "adopt_resolve_value",
            )
            .map_err(|error| error.to_string())?;
        self.builder
            .build_unconditional_branch(done)
            .map_err(|error| error.to_string())?;

        self.builder.position_at_end(plain);
        let payload = if resolved == &HirType::Void {
            self.context.ptr_type(AddressSpace::default()).const_null()
        } else if matches!(resolved, HirType::Undefined | HirType::Null) {
            let slot = self.build_arena_cell(
                &self.builder, self.basic_type(resolved)?, "mixed_undefined",
            )?;
            let absence = if resolved == &HirType::Null {
                self.context.bool_type().const_int(1, false).into()
            } else {
                self.compile_zero_value(resolved)?
            };
            self.builder
                .build_store(slot, absence)
                .map_err(|error| error.to_string())?;
            slot
        } else {
            let bits = self.builder
                .build_extract_value(argument, 1, "resolve_value_bits")
                .map_err(|error| error.to_string())?
                .into_int_value();
            let value = self.unpack_union_payload(bits, resolved)?;
            let slot = self.build_arena_cell(&self.builder, self.basic_type(resolved)?, "mixed_result")?;
            self.builder
                .build_store(slot, value)
                .map_err(|error| error.to_string())?;
            slot
        };
        self.builder
            .build_call(
                self.module.get_function("thaw_promise_resolve").unwrap(),
                &[promise.into(), payload.into()],
                "settle_resolve_value",
            )
            .map_err(|error| error.to_string())?;
        self.builder
            .build_unconditional_branch(done)
            .map_err(|error| error.to_string())?;
        self.builder.position_at_end(done);
        Ok(())
    }

    fn compile_promise_new(
        &mut self,
        executor: &HirExpr,
        resolved: &HirType,
        assimilates: bool,
        typed_rejection: bool,
        mixed_resolver: Option<&HirType>,
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
        // The two escaping resolver closures and the executor's thrown-error path
        // must share the same first-call decision while adoption is still pending.
        let resolver_state = self
            .builder
            .build_call(
                self.module.get_function("thaw_arena_alloc").unwrap(),
                &[
                    self.context.i64_type().const_int(16, false).into(),
                    self.context.i64_type().const_int(8, false).into(),
                ],
                "resolver_state",
            )
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .ok_or("resolver state allocation returned no value")?
            .into_pointer_value();
        self.builder
            .build_store(resolver_state, promise)
            .map_err(|error| error.to_string())?;
        let called_slot = unsafe {
            self.builder
                .build_in_bounds_gep(
                    self.context.i8_type(),
                    resolver_state,
                    &[self.context.i64_type().const_int(8, false)],
                    "resolver_called_slot",
                )
                .map_err(|error| error.to_string())?
        };
        self.builder
            .build_store(called_slot, self.context.i8_type().const_zero())
            .map_err(|error| error.to_string())?;
        let executor = self.compile_expr(executor)?.into_pointer_value();
        let resolve_fn = self.compile_promise_resolver(
            resolved, false, assimilates, false, mixed_resolver.is_some(),
        )?;
        let reject_fn =
            self.compile_promise_resolver(resolved, true, false, typed_rejection, false)?;
        let resolve_params = if let Some(resolver) = mixed_resolver {
            match resolver {
                HirType::Function(params, _) | HirType::CallableFunction(params, _, _, _) => {
                    params.clone()
                }
                _ => return Err("mixed Promise resolver must be a function".into()),
            }
        } else if assimilates {
            vec![HirType::Promise(Box::new(resolved.clone()))]
        } else if resolved == &HirType::Void {
            Vec::new()
        } else {
            vec![resolved.clone()]
        };
        let resolve = self.allocate_special_closure(resolve_fn, resolver_state, &resolve_params, "resolve_closure")?;
        let reject = self.allocate_special_closure(reject_fn, resolver_state, &[HirType::Str], "reject_closure")?;
        let code = self
            .builder
            .build_load(
                self.context.ptr_type(AddressSpace::default()),
                executor,
                "executor_code",
            )
            .map_err(|error| error.to_string())?
            .into_pointer_value();
        let resolve_ty = mixed_resolver.cloned().unwrap_or_else(|| {
            HirType::Function(resolve_params, Box::new(HirType::Void))
        });
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
        let called = self
            .builder
            .build_load(self.context.i8_type(), called_slot, "executor_resolver_called")
            .map_err(|error| error.to_string())?
            .into_int_value();
        let reject_first = self.context.append_basic_block(function, "executor_reject_first");
        let clear_error = self.context.append_basic_block(function, "executor_clear_error");
        let uncalled = self
            .builder
            .build_int_compare(
                IntPredicate::EQ,
                called,
                self.context.i8_type().const_zero(),
                "executor_resolver_uncalled",
            )
            .map_err(|error| error.to_string())?;
        self.builder
            .build_conditional_branch(uncalled, reject_first, clear_error)
            .map_err(|error| error.to_string())?;
        self.builder.position_at_end(reject_first);
        self.builder
            .build_store(called_slot, self.context.i8_type().const_int(1, false))
            .map_err(|error| error.to_string())?;
        self.reject_promise_with_pending_exception(promise, pending, "reject_executor_throw")?;
        self.builder
            .build_unconditional_branch(clear_error)
            .map_err(|error| error.to_string())?;
        self.builder.position_at_end(clear_error);
        self.builder
            .build_store(
                pending_slot,
                self.context.ptr_type(AddressSpace::default()).const_null(),
            )
            .map_err(|error| error.to_string())?;
        self.clear_pending_native_text()?;
        self.builder
            .build_store(
                self.pending_exception_object().as_pointer_value(),
                self.context.ptr_type(AddressSpace::default()).const_null(),
            )
            .map_err(|error| error.to_string())?;
        self.builder
            .build_store(
                self.pending_exception_value(PENDING_EXCEPTION_VALUE_TAG_SYMBOL)
                    .as_pointer_value(),
                self.context.i64_type().const_zero(),
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
            .fn_type(&[ptr.into(), ptr.into(), ptr.into(), ptr.into()], false);
        let adapter =
            self.module
                .add_function(&adapter_name, adapter_type, Some(Linkage::Internal));
        let return_block = self.builder.get_insert_block().unwrap();
        let entry = self.context.append_basic_block(adapter, "entry");
        self.builder.position_at_end(entry);
        let context = adapter.get_nth_param(0).unwrap().into_pointer_value();
        let promise = adapter.get_nth_param(1).unwrap();
        let source_promise = adapter.get_nth_param(2).unwrap().into_pointer_value();
        let result = adapter.get_nth_param(3).unwrap().into_pointer_value();
        if on_rejected {
            for (getter, target) in [
                ("thaw_promise_exception_tag", self.pending_exception_value(PENDING_EXCEPTION_VALUE_TAG_SYMBOL)),
                ("thaw_promise_exception_f64", self.pending_exception_value(PENDING_EXCEPTION_F64_SYMBOL)),
                ("thaw_promise_exception_i64", self.pending_exception_value(PENDING_EXCEPTION_I64_SYMBOL)),
                ("thaw_promise_exception_bool", self.pending_exception_value(PENDING_EXCEPTION_BOOL_SYMBOL)),
                ("thaw_promise_exception_object", self.pending_exception_object()),
                ("thaw_promise_exception_aggregate_errors", self.pending_exception_aggregate_errors()),
            ] {
                let value = self.builder
                    .build_call(
                        self.module.get_function(getter).unwrap(),
                        &[source_promise.into()],
                        "catch_exception_value",
                    )
                    .map_err(|error| error.to_string())?
                    .try_as_basic_value()
                    .basic()
                    .ok_or_else(|| format!("{getter} returned no value"))?;
                self.builder
                    .build_store(target.as_pointer_value(), value)
                    .map_err(|error| error.to_string())?;
            }
        }
        let rejected_text = if on_rejected {
            let native_text = self.builder.build_call(
                self.module.get_function("thaw_promise_exception_native_text_copy").unwrap(),
                &[source_promise.into()], "chain_native_text",
            ).map_err(|error| error.to_string())?.try_as_basic_value().basic()
                .ok_or("native text copy returned no value")?.into_pointer_value();
            let has_text = self.builder.build_is_not_null(native_text, "chain_has_native_text")
                .map_err(|error| error.to_string())?;
            let text = self.builder.build_select(has_text, native_text, result, "chain_rejection_text")
                .map_err(|error| error.to_string())?;
            self.mark_pending_native_text(native_text)?;
            Some(text)
        } else { None };
        let callback_input = if on_rejected { &HirType::Str } else { input };
        let value = if !on_rejected && callback_input == &HirType::Void {
            None
        } else if on_rejected {
            Some(rejected_text.expect("rejection text was captured").into())
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
        self.reject_promise_with_pending_exception(
            promise.into_pointer_value(),
            pending,
            "reject_chain",
        )?;
        self.builder
            .build_store(pending_slot, ptr.const_null())
            .map_err(|error| error.to_string())?;
        self.clear_pending_native_text()?;
        self.builder.build_return(None).map_err(|e| e.to_string())?;
        self.builder.position_at_end(succeeded);
        if on_rejected {
            self.clear_pending_native_text()?;
        }
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
        let source_promise = self.compile_expr(source)?.into_pointer_value();
        let closure = self.compile_expr(callback)?.into_pointer_value();
        let name = format!("__thaw_promise_finally_{}", self.next_lambda);
        self.next_lambda += 1;
        let ptr = self.context.ptr_type(AddressSpace::default());
        let i8_type = self.context.i8_type();
        let adapter_type = self
            .context
            .void_type()
            .fn_type(
                &[
                    ptr.into(),
                    ptr.into(),
                    ptr.into(),
                    ptr.into(),
                    i8_type.into(),
                ],
                false,
            );
        let adapter = self
            .module
            .add_function(&name, adapter_type, Some(Linkage::Internal));
        let return_block = self.builder.get_insert_block().unwrap();
        let entry = self.context.append_basic_block(adapter, "entry");
        self.builder.position_at_end(entry);
        let context = adapter.get_nth_param(0).unwrap().into_pointer_value();
        let output = adapter.get_nth_param(1).unwrap();
        let source = adapter.get_nth_param(2).unwrap().into_pointer_value();
        let original = adapter.get_nth_param(3).unwrap();
        let original_rejected = adapter.get_nth_param(4).unwrap().into_int_value();
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
        self.reject_promise_with_pending_exception(
            output.into_pointer_value(),
            pending,
            "reject_finally",
        )?;
        self.builder
            .build_store(pending_slot, ptr.const_null())
            .map_err(|error| error.to_string())?;
        self.clear_pending_native_text()?;
        self.builder.build_return(None).map_err(|e| e.to_string())?;
        self.builder.position_at_end(succeeded);
        if matches!(callback_return, HirType::Promise(_)) {
            let returned = returned
                .ok_or("Promise-returning finally callback produced no value")?
                .into_pointer_value();
            let mut args: Vec<BasicMetadataValueEnum<'ctx>> = vec![
                output.into(),
                returned.into(),
                original.into(),
                original_rejected.into(),
            ];
            for getter in [
                "thaw_promise_exception_tag",
                "thaw_promise_exception_f64",
                "thaw_promise_exception_i64",
                "thaw_promise_exception_bool",
                "thaw_promise_exception_object",
            ] {
                args.push(
                    self.builder
                        .build_call(
                            self.module.get_function(getter).unwrap(),
                            &[source.into()],
                            "finally_exception_value",
                        )
                        .map_err(|error| error.to_string())?
                        .try_as_basic_value()
                        .basic()
                        .ok_or_else(|| format!("{getter} returned no value"))?
                        .into(),
                );
            }
            args.push(source.into());
            self.builder
                .build_call(
                    self.module
                        .get_function("thaw_promise_finally_adopt_with_source")
                        .unwrap(),
                    &args,
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
            self.reject_promise_with_source_exception(
                output.into_pointer_value(),
                original.into_pointer_value(),
                source,
                "forward_finally_rejection",
            )?;
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
                    source_promise.into(),
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
                        .const_int(array_element_storage_bytes(element)?, false)
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
                        .const_int(array_element_storage_bytes(element)?, false)
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
                    i64_type.const_int(array_element_storage_bytes(element)?, false),
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
                        .const_int(array_element_storage_bytes(element)?, false)
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
        // `await` on a `Union` where exactly one member is `Promise<T>`
        // and the other is already plain `T` (round25's own union-return-
        // type inference lets an unannotated function return a `Promise`
        // on one branch, a plain value on another -- e.g. `function pick
        // (flag: boolean) { return flag ? Promise.resolve(5) : 10; }`)
        // reaches this ordinary/blocking fallback because the async-frame
        // planner's own `is_frame_await_source` only recognizes a value
        // *known* to always be a `Promise<T>`, never "maybe a Promise" --
        // so it's never hoisted into a real coroutine suspend, and this
        // function's own "just return the value unchanged" behavior
        // below is only correct for an operand that provably never *is*
        // a Promise. Confirmed empirically: this genuinely was wrong,
        // printing the live `Promise` object itself instead of its
        // resolved payload. Blocking-drive it, exactly like `sleep`
        // already does just below -- not a true coroutine suspend, but a
        // real, narrow, already-accepted trade-off (this shape means the
        // *whole* enclosing async function blocks the thread while
        // waiting, rather than cooperatively yielding), matching `sleep`'s
        // own existing strategy precisely. A union with more than one
        // non-`Promise` member, or a non-`Promise` member that isn't
        // exactly the resolved payload type, falls through unchanged --
        // not attempted this round, no tracked case needs it.
        if let Some(HirType::Union(members)) = self.expr_hir_type(inner) {
            let promise_member = members.iter().enumerate().find_map(|(index, member)| {
                if let HirType::Promise(resolved) = member {
                    Some((index, resolved.as_ref().clone()))
                } else {
                    None
                }
            });
            if let Some((promise_index, resolved)) = promise_member {
                let other_members = members
                    .iter()
                    .enumerate()
                    .filter(|(index, _)| *index != promise_index)
                    .map(|(_, member)| member)
                    .collect::<Vec<_>>();
                if other_members.len() == 1 && other_members[0] == &resolved {
                    return self.compile_await_promise_union(inner, promise_index, &resolved);
                }
            }
        }
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

    /// `compile_await`'s own `Union` case: `inner` is a tagged-union value
    /// (the same `{tag, payload}` struct `HirExpr::UnionInject`/`UnionTag`/
    /// `UnionValue` already build/read) whose `promise_index`'th member is
    /// `Promise<resolved>` and whose one remaining member is already plain
    /// `resolved`. Evaluated once, then a genuine *runtime* branch on the
    /// union's own discriminant: the `Promise` arm blocking-drives it to
    /// completion (`drive_promise_to_resolved_value`, the same helper a
    /// statically-known `Promise<T>` await already uses outside a real
    /// coroutine frame); the other arm decodes the already-resolved payload
    /// directly (`unpack_union_payload`, the same decode `UnionValue`'s own
    /// codegen uses). Both arms merge into one native value of `resolved`'s
    /// own type via a phi node.
    fn compile_await_promise_union(
        &mut self,
        inner: &HirExpr,
        promise_index: usize,
        resolved: &HirType,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let union = self.compile_expr(inner)?.into_struct_value();
        let tag = self
            .builder
            .build_extract_value(union, 0, "await_union_tag")
            .map_err(|error| error.to_string())?
            .into_int_value();
        let payload = self
            .builder
            .build_extract_value(union, 1, "await_union_payload")
            .map_err(|error| error.to_string())?
            .into_int_value();
        let is_promise = self
            .builder
            .build_int_compare(
                IntPredicate::EQ,
                tag,
                tag.get_type().const_int(promise_index as u64, false),
                "await_union_is_promise",
            )
            .map_err(|error| error.to_string())?;

        let function = self.current_function();
        let promise_bb = self.context.append_basic_block(function, "await_union_promise");
        let value_bb = self.context.append_basic_block(function, "await_union_value");
        let merge_bb = self.context.append_basic_block(function, "await_union_merge");
        self.builder
            .build_conditional_branch(is_promise, promise_bb, value_bb)
            .map_err(|error| error.to_string())?;

        self.builder.position_at_end(promise_bb);
        let promise_ptr = self
            .unpack_union_payload(payload, &HirType::Promise(Box::new(resolved.clone())))?
            .into_pointer_value();
        let promise_result = self.drive_promise_to_resolved_value(promise_ptr, resolved)?;
        let promise_bb_end = self.builder.get_insert_block().unwrap();
        self.builder
            .build_unconditional_branch(merge_bb)
            .map_err(|error| error.to_string())?;

        self.builder.position_at_end(value_bb);
        let plain_result = self.unpack_union_payload(payload, resolved)?;
        let value_bb_end = self.builder.get_insert_block().unwrap();
        self.builder
            .build_unconditional_branch(merge_bb)
            .map_err(|error| error.to_string())?;

        self.builder.position_at_end(merge_bb);
        let result_type = self.basic_type(resolved)?;
        let phi = self
            .builder
            .build_phi(result_type, "await_union_result")
            .map_err(|error| error.to_string())?;
        phi.add_incoming(&[
            (&promise_result, promise_bb_end),
            (&plain_result, value_bb_end),
        ]);
        Ok(phi.as_basic_value())
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

    /// Preserve the full rejection reason before the settled Promise is destroyed.
    /// Both value-taking and void blocking awaits enter this only on failure.
    fn copy_blocking_promise_exception_metadata(
        &mut self,
        promise: PointerValue<'ctx>,
    ) -> Result<(), String> {
        for (getter, target) in [
            ("thaw_promise_exception_tag", self.pending_exception_value(PENDING_EXCEPTION_VALUE_TAG_SYMBOL)),
            ("thaw_promise_exception_f64", self.pending_exception_value(PENDING_EXCEPTION_F64_SYMBOL)),
            ("thaw_promise_exception_i64", self.pending_exception_value(PENDING_EXCEPTION_I64_SYMBOL)),
            ("thaw_promise_exception_bool", self.pending_exception_value(PENDING_EXCEPTION_BOOL_SYMBOL)),
            ("thaw_promise_exception_object", self.pending_exception_object()),
                ("thaw_promise_exception_aggregate_errors", self.pending_exception_aggregate_errors()),
        ] {
            let value = self.builder
                .build_call(
                    self.module.get_function(getter).unwrap(),
                    &[promise.into()],
                    "blocking_await_exception_value",
                )
                .map_err(|error| error.to_string())?
                .try_as_basic_value()
                .basic()
                .ok_or_else(|| format!("{getter} returned no value"))?;
            self.builder
                .build_store(target.as_pointer_value(), value)
                .map_err(|error| error.to_string())?;
        }
        Ok(())
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
        self.clear_pending_native_text()?;
        let native_text = self.builder.build_call(
            self.module.get_function("thaw_promise_exception_native_text_copy").unwrap(),
            &[promise.into()], "blocking_await_native_text",
        ).map_err(|error| error.to_string())?.try_as_basic_value().basic()
            .ok_or("native text copy returned no value")?.into_pointer_value();
        let has_native_text = self.builder.build_is_not_null(native_text, "blocking_await_native_text_present")
            .map_err(|error| error.to_string())?;
        let pending_value = self.builder.build_select(
            has_native_text, native_text, result, "blocking_await_native_text_pending",
        ).map_err(|error| error.to_string())?;
        self.builder.build_store(self.pending_exception().as_pointer_value(), pending_value)
            .map_err(|error| error.to_string())?;
        self.mark_pending_native_text(native_text)?;
        self.copy_blocking_promise_exception_metadata(promise)?;
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
        self.clear_pending_native_text()?;
        let native_text = self.builder.build_call(
            self.module.get_function("thaw_promise_exception_native_text_copy").unwrap(),
            &[promise.into()], "blocking_await_void_native_text",
        ).map_err(|error| error.to_string())?.try_as_basic_value().basic()
            .ok_or("native text copy returned no value")?.into_pointer_value();
        let has_native_text = self.builder.build_is_not_null(native_text, "blocking_await_void_native_text_present")
            .map_err(|error| error.to_string())?;
        let pending_value = self.builder.build_select(
            has_native_text, native_text, result, "blocking_await_void_native_text_pending",
        ).map_err(|error| error.to_string())?;
        self.builder.build_store(self.pending_exception().as_pointer_value(), pending_value)
            .map_err(|error| error.to_string())?;
        self.mark_pending_native_text(native_text)?;
        self.copy_blocking_promise_exception_metadata(promise)?;
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
