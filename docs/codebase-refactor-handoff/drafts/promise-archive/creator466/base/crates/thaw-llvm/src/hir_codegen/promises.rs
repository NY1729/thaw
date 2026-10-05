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
            // The reject closure's argument is declared HirType::Str. Record
            // that producer-proven NativeStr before settlement, including
            // computed strings and values converted by Promise.reject.
            // Public runtime rejection pointers remain opaque.
            self.mark_pending_native_text(payload)?;
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
        // `new Promise(nonCallable)` throws synchronously. Evaluate the
        // executor expression once before allocating the Promise, and do not
        // dereference an absent Function closure.
        let executor = self.compile_expr(executor)?.into_pointer_value();
        self.guard_callable_closure(executor, "Promise executor is not a function")?;
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
        let function = self.current_function();
        let missing_promise = self.context.append_basic_block(function, "promise_new_failed");
        let have_promise = self.context.append_basic_block(function, "promise_new_ready");
        let promise_is_null = self.builder.build_is_null(promise, "promise_new_missing")
            .map_err(|error| error.to_string())?;
        self.builder.build_conditional_branch(promise_is_null, missing_promise, have_promise)
            .map_err(|error| error.to_string())?;
        self.builder.position_at_end(missing_promise);
        self.compile_throw_builtin_error("Error", "Promise allocation failed")?;
        self.builder.position_at_end(have_promise);
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
        let missing_state = self.context.append_basic_block(function, "resolver_state_failed");
        let have_state = self.context.append_basic_block(function, "resolver_state_ready");
        let state_is_null = self.builder.build_is_null(resolver_state, "resolver_state_missing")
            .map_err(|error| error.to_string())?;
        self.builder.build_conditional_branch(state_is_null, missing_state, have_state)
            .map_err(|error| error.to_string())?;
        self.builder.position_at_end(missing_state);
        self.builder.build_call(
            self.module.get_function("thaw_promise_destroy").unwrap(),
            &[promise.into()], "release_unescaped_promise",
        ).map_err(|error| error.to_string())?;
        // No resolver or subscriber can have observed this Promise yet.
        // `destroy` drops its creator share; release the request-base share
        // as well instead of retaining a failed construction until reset.
        self.builder.build_call(
            self.module.get_function("thaw_promise_release_request_base").unwrap(),
            &[promise.into()], "release_unescaped_promise_base",
        ).map_err(|error| error.to_string())?;
        self.compile_throw_builtin_error("Error", "Promise resolver allocation failed")?;
        self.builder.position_at_end(have_state);
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
        let missing_resolve = self.builder.build_is_null(resolve, "missing_resolve_closure")
            .map_err(|error| error.to_string())?;
        let resolve_failed = self.context.append_basic_block(function, "promise_resolve_closure_failed");
        let resolve_ready = self.context.append_basic_block(function, "promise_resolve_closure_ready");
        self.builder.build_conditional_branch(missing_resolve, resolve_failed, resolve_ready)
            .map_err(|error| error.to_string())?;
        self.builder.position_at_end(resolve_failed);
        self.builder.build_call(self.module.get_function("thaw_promise_destroy").unwrap(),
            &[promise.into()], "release_failed_resolve_promise_creator")
            .map_err(|error| error.to_string())?;
        self.builder.build_call(self.module.get_function("thaw_promise_release_request_base").unwrap(),
            &[promise.into()], "release_failed_resolve_promise_base")
            .map_err(|error| error.to_string())?;
        self.compile_throw_builtin_error("Error", "Promise resolve closure allocation failed")?;
        self.builder.position_at_end(resolve_ready);
        let reject = self.allocate_special_closure(reject_fn, resolver_state, &[HirType::Str], "reject_closure")?;
        let missing_reject = self.builder.build_is_null(reject, "missing_reject_closure")
            .map_err(|error| error.to_string())?;
        let reject_failed = self.context.append_basic_block(function, "promise_reject_closure_failed");
        let reject_ready = self.context.append_basic_block(function, "promise_reject_closure_ready");
        self.builder.build_conditional_branch(missing_reject, reject_failed, reject_ready)
            .map_err(|error| error.to_string())?;
        self.builder.position_at_end(reject_failed);
        self.builder.build_call(self.module.get_function("thaw_promise_destroy").unwrap(),
            &[promise.into()], "release_failed_reject_promise_creator")
            .map_err(|error| error.to_string())?;
        self.builder.build_call(self.module.get_function("thaw_promise_release_request_base").unwrap(),
            &[promise.into()], "release_failed_reject_promise_base")
            .map_err(|error| error.to_string())?;
        self.compile_throw_builtin_error("Error", "Promise reject closure allocation failed")?;
        self.builder.position_at_end(reject_ready);
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

    fn compile_promise_fulfillment_as_type(
        &mut self,
        promise: PointerValue<'ctx>,
        result: PointerValue<'ctx>,
        resolved: &HirType,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let llvm_type = self.basic_type(resolved)?;
        let kind = self.builder.build_call(
            self.module.get_function("thaw_promise_result_kind").unwrap(),
            &[promise.into()], "promise_result_kind",
        ).map_err(|error| error.to_string())?.try_as_basic_value().basic()
            .ok_or("Promise result kind returned no value")?.into_int_value();
        let is_native = self.builder.build_int_compare(
            IntPredicate::EQ, kind,
            self.context.i8_type().const_zero(), "promise_native_origin",
        ).map_err(|error| error.to_string())?;
        let is_js_origin = self.builder.build_int_compare(
            IntPredicate::EQ, kind,
            self.context.i8_type().const_int(1, false), "promise_js_origin",
        ).map_err(|error| error.to_string())?;
        let function = self.current_function();
        let valid_kind = self.builder.build_or(is_native, is_js_origin, "promise_valid_origin")
            .map_err(|error| error.to_string())?;
        let has_result = self.builder.build_is_not_null(result, "promise_result_present")
            .map_err(|error| error.to_string())?;
        let valid = self.builder.build_and(valid_kind, has_result, "promise_valid_payload")
            .map_err(|error| error.to_string())?;
        let invalid_bb = self.context.append_basic_block(function, "promise_invalid_origin");
        let dispatch_bb = self.context.append_basic_block(function, "promise_origin_dispatch");
        self.builder.build_conditional_branch(valid, dispatch_bb, invalid_bb)
            .map_err(|error| error.to_string())?;
        self.builder.position_at_end(invalid_bb);
        self.compile_throw_builtin_error("TypeError", "Invalid Promise result or origin")?;
        self.builder.position_at_end(dispatch_bb);
        let native_bb = self.context.append_basic_block(function, "promise_native_result");
        let js_bb = self.context.append_basic_block(function, "promise_js_result");
        let done_bb = self.context.append_basic_block(function, "promise_projected_result");
        self.builder.build_conditional_branch(is_js_origin, js_bb, native_bb)
            .map_err(|error| error.to_string())?;
        self.builder.position_at_end(native_bb);
        let native = self.builder.build_load(llvm_type, result, "promise_native_value")
            .map_err(|error| error.to_string())?;
        self.builder.build_unconditional_branch(done_bb)
            .map_err(|error| error.to_string())?;
        self.builder.position_at_end(js_bb);
        let cloned = self.builder.build_call(
            self.module.get_function("thaw_json_share").unwrap(),
            &[result.into()], "promise_contextual_json_clone",
        ).map_err(|error| error.to_string())?.try_as_basic_value().basic()
            .ok_or("Promise Json clone returned no value")?;
        let cloned = cloned.into_pointer_value();
        let absent = self.builder.build_is_null(cloned, "promise_json_clone_failed")
            .map_err(|error| error.to_string())?;
        let failed_bb = self.context.append_basic_block(function, "promise_json_clone_error");
        let decode_bb = self.context.append_basic_block(function, "promise_json_decode");
        self.builder.build_conditional_branch(absent, failed_bb, decode_bb)
            .map_err(|error| error.to_string())?;
        self.builder.position_at_end(failed_bb);
        // thaw_json_share reports the shared HostError contract when the
        // source Box is missing. Consume it through the active catch/async
        // channel before falling back for a null return without HostError.
        self.compile_check_json_host_error(cloned.into(), None)?;
        self.compile_throw_builtin_error("RangeError", "Unable to retain Promise result")?;
        self.builder.position_at_end(decode_bb);
        let contextual = self.compile_typed_dynamic_result(cloned.into(), resolved)?;
        let js_end = self.builder.get_insert_block()
            .ok_or("Promise Json projection has no block")?;
        self.builder.build_unconditional_branch(done_bb)
            .map_err(|error| error.to_string())?;
        self.builder.position_at_end(done_bb);
        let phi = self.builder.build_phi(llvm_type, "promise_contextual_value")
            .map_err(|error| error.to_string())?;
        phi.add_incoming(&[(&native, native_bb), (&contextual, js_end)]);
        Ok(phi.as_basic_value())
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
        let adapter = self.compile_promise_chain_adapter(input, output, on_rejected, flatten)?;
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

    /// Emits the selected settlement adapter without subscribing a source.
    /// Both-handler `.then` uses this twice, then makes one native chain.
    fn compile_promise_chain_adapter(
        &mut self,
        input: &HirType,
        output: &HirType,
        on_rejected: bool,
        flatten: bool,
    ) -> Result<FunctionValue<'ctx>, String> {
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
        // This is the same active value that lexical catch and allSettled
        // observe. Its Json member has an independent arena-rooted Box before
        // the source Promise may be retired by the chain subscription.
        let rejected_carrier = if on_rejected {
            let saved_completion = self.active_async_completion;
            self.active_async_completion = Some(promise.into_pointer_value());
            let carrier = self.compile_promise_rejection_carrier(source_promise);
            self.active_async_completion = saved_completion;
            Some(carrier?)
        } else { None };
        let caught_type = thaw_hir::caught_exception_carrier_type();
        let callback_input = if on_rejected { &caught_type } else { input };
        let value = if !on_rejected && callback_input == &HirType::Void {
            None
        } else if on_rejected {
            Some(rejected_carrier.expect("rejection carrier was captured").into())
        } else {
            let saved_completion = self.active_async_completion;
            self.active_async_completion = Some(promise.into_pointer_value());
            let projected = self.compile_promise_fulfillment_as_type(
                source_promise, result, callback_input,
            );
            self.active_async_completion = saved_completion;
            Some(projected?)
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
        self.builder.build_store(self.pending_exception_object().as_pointer_value(),
            ptr.const_null()).map_err(|error| error.to_string())?;
        self.builder.build_store(
            self.pending_exception_value(PENDING_EXCEPTION_VALUE_TAG_SYMBOL).as_pointer_value(),
            self.context.i64_type().const_zero(),
        ).map_err(|error| error.to_string())?;
        self.builder.build_store(self.pending_exception_aggregate_errors().as_pointer_value(),
            ptr.const_null()).map_err(|error| error.to_string())?;
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
        Ok(adapter)
    }

    fn compile_promise_then_both(
        &mut self,
        source: &HirExpr,
        fulfilled: Option<&HirExpr>,
        rejected: &HirExpr,
        input: &HirType,
        output: &HirType,
        flatten_fulfilled: bool,
        flatten_rejected: bool,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        // These source-visible expressions run once, in member-call order,
        // before either adapter subscribes to the original promise.
        let source = self.compile_expr(source)?.into_pointer_value();
        let fulfilled_closure = fulfilled
            .map(|callback| self.compile_expr(callback).map(BasicValueEnum::into_pointer_value))
            .transpose()?;
        let rejected_closure = self.compile_expr(rejected)?.into_pointer_value();
        let fulfilled_adapter = if fulfilled_closure.is_some() {
            Some(self.compile_promise_chain_adapter(input, output, false, flatten_fulfilled)?)
        } else {
            None
        };
        let rejected_adapter =
            self.compile_promise_chain_adapter(input, output, true, flatten_rejected)?;
        let ptr = self.context.ptr_type(AddressSpace::default());
        let fulfilled_code = fulfilled_adapter
            .map(|adapter| adapter.as_global_value().as_pointer_value())
            .unwrap_or(ptr.const_null());
        let fulfilled_context = fulfilled_closure.unwrap_or(ptr.const_null());
        self.builder.build_call(
            self.module.get_function("thaw_promise_chain_both").unwrap(),
            &[
                source.into(),
                fulfilled_code.into(),
                fulfilled_context.into(),
                rejected_adapter.as_global_value().as_pointer_value().into(),
                rejected_closure.into(),
            ],
            "promise_chain_both",
        ).map_err(|error| error.to_string())?.try_as_basic_value().basic()
            .ok_or_else(|| "thaw_promise_chain_both returned no value".to_string())
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
        let promise = self.builder
            .build_call(sleep, &[milliseconds.into()], "sleep_promise")
            .map_err(|e| e.to_string())?
            .try_as_basic_value()
            .basic()
            .ok_or_else(|| "thaw_sleep_ms did not return a promise".to_string())?;
        let promise = promise.into_pointer_value();
        let function = self.current_function();
        let missing = self.builder.build_is_null(promise, "sleep_promise_allocation_failed")
            .map_err(|error| error.to_string())?;
        let allocation_failed = self.context.append_basic_block(function,
            "sleep_promise_allocation_failed");
        let allocation_ready = self.context.append_basic_block(function,
            "sleep_promise_allocation_ready");
        self.builder.build_conditional_branch(missing, allocation_failed, allocation_ready)
            .map_err(|error| error.to_string())?;
        self.builder.position_at_end(allocation_failed);
        self.compile_throw_type_error("Cannot allocate sleep Promise")?;
        self.builder.position_at_end(allocation_ready);
        let registration_failed = self.context.append_basic_block(function,
            "sleep_promise_registration_failed");
        let registration_ready = self.context.append_basic_block(function,
            "sleep_promise_registration_ready");
        self.catch_stack.push(registration_failed);
        self.compile_register_produced_promise_graph_finisher(
            promise, &HirType::Void,
        )?;
        self.catch_stack.pop();
        self.builder.build_unconditional_branch(registration_ready)
            .map_err(|error| error.to_string())?;
        self.builder.position_at_end(registration_failed);
        // The timer queue still holds its borrowed pointer; retiring the
        // creator and base shares removes that entry if no subscriber owns it.
        self.builder.build_call(self.module.get_function("thaw_promise_destroy").unwrap(),
            &[promise.into()], "destroy_unregistered_sleep_promise")
            .map_err(|error| error.to_string())?;
        self.builder.build_call(self.module.get_function("thaw_promise_release_request_base").unwrap(),
            &[promise.into()], "release_unregistered_sleep_base")
            .map_err(|error| error.to_string())?;
        self.branch_on_pending_exception()?;
        self.builder.build_unreachable().map_err(|error| error.to_string())?;
        self.builder.position_at_end(registration_ready);
        Ok(promise.into())
    }

    fn compile_promise_input_words(
        &mut self, words: usize, name: &str,
    ) -> Result<PointerValue<'ctx>, String> {
        let pointer = self.context.ptr_type(AddressSpace::default());
        if words == 0 { return Ok(pointer.const_null()); }
        let bytes = words.checked_mul(8).ok_or("Promise input storage size overflow")?;
        let i64_type = self.context.i64_type();
        let storage = self.builder.build_call(
            self.module.get_function("thaw_arena_alloc").unwrap(),
            &[i64_type.const_int(bytes as u64, false).into(), i64_type.const_int(8, false).into()],
            name,
        ).map_err(|error| error.to_string())?.try_as_basic_value().basic()
            .ok_or("Promise input allocation returned void")?.into_pointer_value();
        let function = self.current_function();
        let failed = self.context.append_basic_block(function, "promise_input_allocation_failed");
        let ready = self.context.append_basic_block(function, "promise_input_allocation_ready");
        let missing = self.builder.build_is_null(storage, "promise_input_missing")
            .map_err(|error| error.to_string())?;
        self.builder.build_conditional_branch(missing, failed, ready)
            .map_err(|error| error.to_string())?;
        self.builder.position_at_end(failed);
        self.compile_throw_builtin_error("Error", "Promise input allocation failed")?;
        self.builder.position_at_end(ready);
        Ok(storage)
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
            let storage = self.compile_promise_input_words(args.len(), "promise_all_storage")?;
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
        let promises = self.compile_promise_input_words(args.len(), "promise_all_tuple_promises")?;
        let sizes = self.compile_promise_input_words(elements.len(), "promise_all_tuple_sizes")?;
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
            let storage = self.compile_promise_input_words(args.len(), "promise_all_settled_storage")?;
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

    fn compile_settled_reason_decoder(&mut self) -> Result<FunctionValue<'ctx>, String> {
        let pointer = self.context.ptr_type(AddressSpace::default());
        let word = self.context.i64_type();
        let name = format!("__thaw_settled_reason_decoder_{}", self.next_lambda);
        self.next_lambda += 1;
        let decoder = self.module.add_function(&name,
            word.fn_type(&[pointer.into(), pointer.into()], false), Some(Linkage::Internal));
        let caller = self.builder.get_insert_block().ok_or("settled decoder has no caller")?;
        let entry = self.context.append_basic_block(decoder, "entry");
        let failed = self.context.append_basic_block(decoder, "failed");
        let saved_catch = std::mem::take(&mut self.catch_stack);
        let saved_catch_owner_roots = std::mem::take(&mut self.catch_owner_roots);
        let saved_completion = self.active_async_completion.take();
        self.catch_stack.push(failed);
        let generated = (|| -> Result<(), String> {
            self.builder.position_at_end(entry);
            let source = decoder.get_nth_param(0).unwrap().into_pointer_value();
            let borrowed = self.compile_promise_rejection_borrowed_json(source)?;
            let owned = self.builder.build_call(
                self.module.get_function("thaw_json_share").unwrap(), &[borrowed.into()],
                "share_settled_original_reason",
            ).map_err(|error| error.to_string())?.try_as_basic_value().basic()
                .ok_or("settled reason share returned no value")?;
            let owned = self.compile_check_json_host_error(owned, Some("thaw_json_destroy"))?
                .into_pointer_value();
            let handle = self.compile_retain_settled_reason_json(owned)?;
            // The constructor records this temporary before checking pending
            // exception, including a retain result carrying both fields.
            self.builder.build_return(Some(&handle)).map_err(|error| error.to_string())?;
            self.builder.position_at_end(failed);
            self.builder.build_return(Some(&word.const_zero())).map_err(|error| error.to_string())?;
            Ok(())
        })();
        self.catch_stack = saved_catch;
        self.catch_owner_roots = saved_catch_owner_roots.clone();
        self.active_async_completion = saved_completion;
        self.builder.position_at_end(caller);
        generated?;
        Ok(decoder)
    }

    fn compile_promise_settled_constructor(
        &mut self,
        input: &HirType,
        reason_decoder: FunctionValue<'ctx>,
    ) -> Result<FunctionValue<'ctx>, String> {
        let pointer = self.context.ptr_type(AddressSpace::default());
        if reason_decoder.get_type() != self.context.i64_type().fn_type(
            &[pointer.into(), pointer.into()], false) {
            return Err("settled reason decoder must return JsValue from source and result".into());
        }
        let name = format!("__thaw_settled_constructor_{}", self.next_lambda);
        self.next_lambda += 1;
        let constructor = self.module.add_function(&name,
            pointer.fn_type(&[pointer.into(), pointer.into(), pointer.into(), pointer.into()], false),
            Some(Linkage::Internal));
        let return_block = self.builder.get_insert_block().ok_or("settled caller has no block")?;
        let entry = self.context.append_basic_block(constructor, "entry");
        let fulfilled = self.context.append_basic_block(constructor, "fulfilled");
        let rejected = self.context.append_basic_block(constructor, "rejected");
        let failed = self.context.append_basic_block(constructor, "construction_failed");
        let saved_catch = std::mem::take(&mut self.catch_stack);
        let saved_catch_owner_roots = std::mem::take(&mut self.catch_owner_roots);
        let saved_completion = self.active_async_completion.take();
        self.catch_stack.push(failed);
        let generated = (|| -> Result<(), String> {
            self.builder.position_at_end(entry);
            let source = constructor.get_nth_param(0).unwrap().into_pointer_value();
            let result = constructor.get_nth_param(1).unwrap().into_pointer_value();
            let destination = constructor.get_nth_param(2).unwrap().into_pointer_value();
            let output = constructor.get_nth_param(3).unwrap().into_pointer_value();
            let temporary_reason = self.builder.build_alloca(self.context.i64_type(),
                "settled_temporary_reason").map_err(|error| error.to_string())?;
            self.builder.build_store(temporary_reason, self.context.i64_type().const_zero())
                .map_err(|error| error.to_string())?;
            let value_type = if *input == HirType::Void { HirType::Undefined } else { input.clone() };
            let HirType::Union(members) = thaw_hir::promise_settled_result_type(value_type.clone()) else {
                return Err("settled result type must be a union".into());
            };
            let state = self.builder.build_call(self.module.get_function("thaw_promise_state").unwrap(),
                &[source.into()], "settled_source_state")
                .map_err(|error| error.to_string())?.try_as_basic_value().basic()
                .ok_or("source state returned no value")?.into_int_value();
            let success = self.builder.build_int_compare(IntPredicate::EQ, state,
                state.get_type().const_int(1, false), "settled_source_fulfilled")
                .map_err(|error| error.to_string())?;
            self.builder.build_conditional_branch(success, fulfilled, rejected)
                .map_err(|error| error.to_string())?;
            self.builder.position_at_end(fulfilled);
            let value = if *input == HirType::Void {
                self.compile_zero_value(&HirType::Undefined)?
            } else {
                self.compile_promise_fulfillment_as_type(source, result, input)?
            };
            self.branch_on_pending_exception()?;
            let object = self.build_promise_settled_result_member(destination, &members, 0,
                "fulfilled", value)?;
            self.builder.build_return(Some(&object)).map_err(|error| error.to_string())?;
            self.builder.position_at_end(rejected);
            let reason = self.builder.build_call(reason_decoder, &[source.into(), result.into()],
                "settled_original_reason")
                .map_err(|error| error.to_string())?.try_as_basic_value().basic()
                .ok_or("reason decoder returned no value")?;
            self.builder.build_store(temporary_reason, reason)
                .map_err(|error| error.to_string())?;
            self.branch_on_pending_exception()?;
            let object = self.build_promise_settled_result_member(destination, &members, 1,
                "rejected", reason)?;
            self.builder.build_call(self.module.get_function("thaw_js_release_handle").unwrap(),
                &[reason.into()], "release_settled_reason_temporary")
                .map_err(|error| error.to_string())?;
            self.builder.build_store(temporary_reason, self.context.i64_type().const_zero())
                .map_err(|error| error.to_string())?;
            self.builder.build_return(Some(&object)).map_err(|error| error.to_string())?;
            self.builder.position_at_end(failed);
            let error = self.builder.build_load(pointer, self.pending_exception().as_pointer_value(),
                "settled_construction_error").map_err(|error| error.to_string())?.into_pointer_value();
            self.reject_promise_with_pending_exception(output, error, "reject_settled_construction")?;
            // Preserve the original typed rejection before releasing a
            // temporary registry reference that may retire native callbacks.
            let temporary = self.builder.build_load(self.context.i64_type(), temporary_reason,
                "failed_settled_reason").map_err(|error| error.to_string())?;
            self.builder.build_call(self.module.get_function("thaw_js_release_handle").unwrap(),
                &[temporary.into()], "release_failed_settled_reason")
                .map_err(|error| error.to_string())?;
            self.builder.build_store(self.pending_exception().as_pointer_value(), pointer.const_null())
                .map_err(|error| error.to_string())?;
            self.clear_pending_native_text()?;
            // The output Promise now owns the rejection tuple. This callback
            // consumed the exception; clear every companion before another
            // child conversion runs on the same thread.
            for (slot, zero) in [
                (self.pending_exception_object().as_pointer_value(), pointer.const_null().into()),
                (self.pending_exception_value(PENDING_EXCEPTION_VALUE_TAG_SYMBOL).as_pointer_value(),
                    self.context.i64_type().const_zero().into()),
                (self.pending_exception_value(PENDING_EXCEPTION_F64_SYMBOL).as_pointer_value(),
                    self.context.f64_type().const_zero().into()),
                (self.pending_exception_value(PENDING_EXCEPTION_I64_SYMBOL).as_pointer_value(),
                    self.context.i64_type().const_zero().into()),
                (self.pending_exception_value(PENDING_EXCEPTION_BOOL_SYMBOL).as_pointer_value(),
                    self.context.bool_type().const_zero().into()),
            ] {
                let zero: BasicValueEnum<'ctx> = zero;
                self.builder.build_store(slot, zero).map_err(|error| error.to_string())?;
            }
            self.builder.build_return(Some(&pointer.const_null())).map_err(|error| error.to_string())?;
            Ok(())
        })();
        self.catch_stack = saved_catch;
        self.catch_owner_roots = saved_catch_owner_roots.clone();
        self.active_async_completion = saved_completion;
        self.builder.position_at_end(return_block);
        generated?;
        Ok(constructor)
    }

    // Consumes one owned Json value and returns an independently retained
    // JavaScript handle. Graph transport preserves primitive kinds and live
    // object identity; the callback must attach the handle to its result owner.
    fn compile_retain_settled_reason_json(
        &mut self,
        json: PointerValue<'ctx>,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        self.uses_quickjs = true;
        self.uses_quickjs_handles = true;
        self.compile_register_js_callback_host_operations()?;
        let wire = self.builder.build_call(
            self.module.get_function("thaw_json_graph_encode").unwrap(),
            &[json.into()], "settled_reason_graph",
        ).map_err(|error| error.to_string())?.try_as_basic_value().basic()
            .ok_or("reason graph encoder returned no value")?;
        let wire = self.compile_check_json_stringify_error_with_cleanup(wire, &[json])?;
        let result = self.builder.build_call(
            self.module.get_function("thaw_js_retain_graph_result").unwrap(),
            &[wire.into()], "retain_settled_reason",
        ).map_err(|error| error.to_string())?.try_as_basic_value().basic()
            .ok_or("reason graph retention returned no value")?.into_struct_value();
        let handle = self.builder.build_extract_value(result, 0, "settled_reason_handle")
            .map_err(|error| error.to_string())?;
        let error = self.builder.build_extract_value(result, 1, "settled_reason_error")
            .map_err(|error| error.to_string())?;
        self.builder.build_call(self.module.get_function("thaw_cstring_destroy").unwrap(),
            &[wire.into()], "destroy_settled_reason_graph")
            .map_err(|error| error.to_string())?;
        self.builder.build_call(self.module.get_function("thaw_json_destroy").unwrap(),
            &[json.into()], "destroy_settled_reason_json")
            .map_err(|error| error.to_string())?;
        self.builder.build_store(self.pending_exception().as_pointer_value(), error)
            .map_err(|error| error.to_string())?;
        self.clear_pending_native_text()?;
        self.mark_pending_owned_native_text(error)?;
        // As with native projection, publish the temporary handle to the
        // constructor before its pending-exception cleanup branch.
        Ok(handle)
    }

    fn build_promise_settled_result_member(
        &mut self,
        destination: PointerValue<'ctx>,
        members: &[HirType],
        index: usize,
        status: &str,
        payload: BasicValueEnum<'ctx>,
    ) -> Result<PointerValue<'ctx>, String> {
        let member = members.get(index).ok_or("missing settled result member")?;
        let HirType::Object(fields) = member else {
            return Err("settled result member must be an object".into());
        };
        let payload_name = match status {
            "fulfilled" => "value",
            "rejected" => "reason",
            _ => return Err("invalid settled result status".into()),
        };
        if fields.len() != 2 || fields[0] != ("status".into(), HirType::Str)
            || fields[1].0 != payload_name {
            return Err("settled result member has an inconsistent shape".into());
        }
        if payload.get_type() != self.basic_type(&fields[1].1)? {
            return Err("settled result payload does not match its declared type".into());
        }
        let object = self.compile_object_alloc(member, false)?.into_pointer_value();
        // The runtime roots this destination array. Publish the zero-initialized
        // owner before handle retention can reenter host code or reset the arena.
        let result = self.build_union_value(object.into(), index, members)?;
        self.builder.build_store(destination, result).map_err(|error| error.to_string())?;
        if fields[1].1 == HirType::JsValue {
            self.tracks_owned_json_roots = true;
            self.compile_register_js_callback_host_operations()?;
            let retained = self.builder.build_call(
                self.module.get_function("thaw_json_retain_handle_for_owner").unwrap(),
                &[object.into(), payload.into()], "retain_settled_field_handle")
                .map_err(|error| error.to_string())?.try_as_basic_value().basic()
                .ok_or("settled field ownership returned no value")?;
            self.compile_check_json_host_error(retained, None)?;
        }
        let status = self.builder.build_global_string_ptr(status, "settled_status")
            .map_err(|error| error.to_string())?.as_pointer_value();
        self.builder.build_store(object, status).map_err(|error| error.to_string())?;
        let payload_slot = unsafe {
            self.builder.build_in_bounds_gep(
                self.context.i8_type(), object,
                &[self.context.i64_type().const_int(object_field_offset(fields, 1)?, false)],
                "settled_payload_slot",
            ).map_err(|error| error.to_string())?
        };
        self.builder.build_store(payload_slot, payload).map_err(|error| error.to_string())?;
        Ok(object)
    }

    fn build_promise_all_settled_call(
        &mut self,
        promises: PointerValue<'ctx>,
        len: IntValue<'ctx>,
        element: &HirType,
        name: &str,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let i64_type = self.context.i64_type();
        let reason_decoder = self.compile_settled_reason_decoder()?;
        let constructor = self.compile_promise_settled_constructor(element, reason_decoder)?;
        let result_type = thaw_hir::promise_settled_result_type(element.clone());
        let input_storage = if *element == HirType::Void {
            HirType::Undefined
        } else {
            element.clone()
        };
        self.builder
            .build_call(
                self.module
                    .get_function("thaw_promise_all_settled_constructed")
                    .unwrap(),
                &[
                    promises.into(),
                    len.into(),
                    i64_type
                        .const_int(array_element_storage_bytes(&input_storage)?, false)
                        .into(),
                    i64_type.const_int(array_element_storage_bytes(&result_type)?, false).into(),
                    constructor.as_global_value().as_pointer_value().into(),
                ],
                name,
            )
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .ok_or_else(|| "thaw_promise_all_settled_constructed did not return a promise".to_string())
    }

    fn compile_promise_race(&mut self, args: &[HirExpr]) -> Result<BasicValueEnum<'ctx>, String> {
        self.compile_first_settled_combinator(args, "thaw_promise_race", "promise_race")
    }

    fn compile_promise_any(&mut self, args: &[HirExpr]) -> Result<BasicValueEnum<'ctx>, String> {
        // Promise.any owns graph reason boxes across asynchronous callbacks;
        // tracing must start before any reason buffer is allocated.
        self.tracks_owned_json_roots = true;
        self.compile_first_settled_combinator(args, "thaw_promise_any_with_projector", "promise_any")
    }

    fn compile_first_settled_combinator(
        &mut self,
        args: &[HirExpr],
        runtime_symbol: &str,
        label: &str,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let ptr_type = self.context.ptr_type(AddressSpace::default());
        let i64_type = self.context.i64_type();
        let storage = self.compile_promise_input_words(args.len(), &format!("{label}_storage"))?;
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
        let mut call_args: Vec<BasicMetadataValueEnum<'ctx>> = vec![
            storage.into(), i64_type.const_int(args.len() as u64, false).into(),
        ];
        if runtime_symbol == "thaw_promise_any_with_projector" {
            let projector = self.compile_promise_any_projector()?;
            call_args.push(projector.as_global_value().as_pointer_value().into());
        }
        self.builder
            .build_call(
                self.module.get_function(runtime_symbol).unwrap(),
                &call_args,
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
        self.tracks_owned_json_roots = true;
        self.compile_first_settled_array(array, "thaw_promise_any_with_projector", "promise_any")
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
        let mut call_args: Vec<BasicMetadataValueEnum<'ctx>> = vec![promises.into(), len.into()];
        if runtime_symbol == "thaw_promise_any_with_projector" {
            let projector = self.compile_promise_any_projector()?;
            call_args.push(projector.as_global_value().as_pointer_value().into());
        }
        self.builder
            .build_call(
                self.module.get_function(runtime_symbol).unwrap(),
                &call_args,
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

    /// Capture one rejected Promise as the ordinary active catch value while
    /// the source is live. Tag 7 is independently shared by the pending-object
    /// accessor; native Error text is canonicalized once on the source.
    fn compile_promise_rejection_carrier(
        &mut self,
        promise: PointerValue<'ctx>,
    ) -> Result<StructValue<'ctx>, String> {
        self.tracks_owned_json_roots = true;
        let mut read = |symbol: &str| -> Result<BasicValueEnum<'ctx>, String> {
            self.builder.build_call(
                self.module.get_function(symbol).unwrap(), &[promise.into()], symbol,
            ).map_err(|error| error.to_string())?.try_as_basic_value().basic()
                .ok_or_else(|| format!("{symbol} returned no value"))
        };
        let tag = read("thaw_promise_exception_tag")?.into_int_value();
        let number = read("thaw_promise_exception_f64")?.into_float_value();
        let bigint = read("thaw_promise_exception_i64")?.into_int_value();
        let boolean = read("thaw_promise_exception_bool")?.into_int_value();
        let text = read("thaw_promise_exception_native_text_copy")?.into_pointer_value();
        let owner = read("thaw_promise_exception_pending_object")?.into_pointer_value();
        let aggregate = read("thaw_promise_exception_aggregate_errors")?.into_pointer_value();

        // The generic result slot is opaque. Only native_text_copy proves a
        // NativeStr. Tags 0 without an owner and 4 cannot use result fallback.
        let i64_ty = self.context.i64_type();
        let tag_zero = self.builder.build_int_compare(IntPredicate::EQ, tag,
            i64_ty.const_zero(), "promise_reason_native_kind")
            .map_err(|error| error.to_string())?;
        let tag_string = self.builder.build_int_compare(IntPredicate::EQ, tag,
            i64_ty.const_int(4, false), "promise_reason_string_kind")
            .map_err(|error| error.to_string())?;
        let owner_missing = self.builder.build_is_null(owner, "promise_reason_owner_missing")
            .map_err(|error| error.to_string())?;
        let text_missing = self.builder.build_is_null(text, "promise_reason_text_missing")
            .map_err(|error| error.to_string())?;
        let needs_text = self.builder.build_or(
            self.builder.build_and(tag_zero, owner_missing, "promise_reason_legacy_text")
                .map_err(|error| error.to_string())?,
            tag_string, "promise_reason_requires_text",
        ).map_err(|error| error.to_string())?;
        let invalid_text = self.builder.build_and(needs_text, text_missing,
            "promise_reason_untrusted_text").map_err(|error| error.to_string())?;
        let tag_json = self.builder.build_int_compare(IntPredicate::EQ, tag,
            i64_ty.const_int(7, false), "promise_reason_json_kind")
            .map_err(|error| error.to_string())?;
        let invalid_json = self.builder.build_and(tag_json, owner_missing,
            "promise_reason_missing_json_share").map_err(|error| error.to_string())?;
        // Raw tag-8 handles and unknown tags have no owned carrier lifetime.
        // Promise.any can also attach an aggregate-errors buffer to tag 0;
        // its AggregateError must be projected, never recast as text.
        let unowned_tag = self.builder.build_int_compare(IntPredicate::UGE, tag,
            i64_ty.const_int(8, false), "promise_reason_unowned_tag")
            .map_err(|error| error.to_string())?;
        let has_aggregate = self.builder.build_is_not_null(aggregate,
            "promise_reason_has_aggregate").map_err(|error| error.to_string())?;
        let invalid = self.builder.build_or(
            self.builder.build_or(
                self.builder.build_or(invalid_text, invalid_json,
                    "promise_reason_missing_value").map_err(|error| error.to_string())?,
                unowned_tag, "promise_reason_invalid_tag",
            ).map_err(|error| error.to_string())?,
            has_aggregate, "promise_reason_needs_aggregate_projector",
        ).map_err(|error| error.to_string())?;
        let function = self.current_function();
        let failure = self.context.append_basic_block(function, "promise_reason_missing_authority");
        let ready = self.context.append_basic_block(function, "promise_reason_authorized");
        self.builder.build_conditional_branch(invalid, failure, ready)
            .map_err(|error| error.to_string())?;
        self.builder.position_at_end(failure);
        // A tag-7 transfer failure carries a HostError from the owner helper.
        // For opaque legacy text, report the missing authority explicitly.
        let host_error = self.builder.build_call(
            self.module.get_function("thaw_json_take_host_error").unwrap(),
            &[], "promise_reason_transfer_error",
        ).map_err(|error| error.to_string())?.try_as_basic_value().basic()
            .ok_or("Promise reason HostError returned no value")?.into_pointer_value();
        let has_host_error = self.builder.build_is_not_null(host_error,
            "promise_reason_has_host_error").map_err(|error| error.to_string())?;
        let host_failure = self.context.append_basic_block(function, "promise_reason_host_failure");
        let opaque_failure = self.context.append_basic_block(function, "promise_reason_opaque_failure");
        self.builder.build_conditional_branch(has_host_error, host_failure, opaque_failure)
            .map_err(|error| error.to_string())?;
        self.builder.position_at_end(host_failure);
        self.builder.build_store(self.pending_exception().as_pointer_value(), host_error)
            .map_err(|error| error.to_string())?;
        self.clear_pending_native_text()?;
        self.mark_pending_owned_native_text(host_error)?;
        self.branch_on_pending_exception()?;
        self.builder.build_unconditional_branch(opaque_failure)
            .map_err(|error| error.to_string())?;
        self.builder.position_at_end(opaque_failure);
        self.compile_throw_type_error("Promise reason requires trusted original-value projection")?;
        self.builder.position_at_end(ready);

        let native_text_reason = self.builder.build_and(tag_zero, owner_missing,
            "promise_reason_native_error_text").map_err(|error| error.to_string())?;
        let native_block = self.context.append_basic_block(function, "promise_native_error_reason");
        let direct_block = self.context.append_basic_block(function, "promise_direct_reason");
        let joined = self.context.append_basic_block(function, "promise_original_reason_ready");
        self.builder.build_conditional_branch(native_text_reason, native_block, direct_block)
            .map_err(|error| error.to_string())?;
        self.builder.position_at_end(native_block);
        let _canonical_native = self.compile_native_error_for_promise_source(promise, text)?;
        // The canonical Box belongs to the source Promise. A callback can
        // return, capture, or rethrow its parameter after that source retires.
        // Transfer an independent arena-rooted share before invoking it.
        let shared_native = self.builder.build_call(
            self.module.get_function("thaw_promise_exception_pending_object").unwrap(),
            &[promise.into()], "chain_native_error_share",
        ).map_err(|error| error.to_string())?.try_as_basic_value().basic()
            .ok_or("native Error pending share returned no value")?;
        let shared_native = self.compile_check_json_host_error(shared_native, None)?
            .into_pointer_value();
        let share_missing = self.builder.build_is_null(shared_native,
            "chain_native_error_share_missing").map_err(|error| error.to_string())?;
        let function = self.current_function();
        let share_failed = self.context.append_basic_block(function, "chain_native_share_failed");
        let share_ready = self.context.append_basic_block(function, "chain_native_share_ready");
        self.builder.build_conditional_branch(share_missing, share_failed, share_ready)
            .map_err(|error| error.to_string())?;
        self.builder.position_at_end(share_failed);
        self.compile_throw_type_error("Unable to retain canonical Promise Error")?;
        self.builder.position_at_end(share_ready);
        let native_carrier = self.build_caught_exception_carrier(
            text, shared_native,
            i64_ty.const_int(7, false), number, bigint, boolean, false,
        )?;
        let native_from = self.builder.get_insert_block().ok_or("missing native Promise reason")?;
        self.builder.build_unconditional_branch(joined).map_err(|error| error.to_string())?;
        self.builder.position_at_end(direct_block);
        let direct_carrier = self.build_caught_exception_carrier(
            text, owner, tag, number, bigint, boolean, false,
        )?;
        let direct_from = self.builder.get_insert_block().ok_or("missing direct Promise reason")?;
        self.builder.build_unconditional_branch(joined).map_err(|error| error.to_string())?;
        self.builder.position_at_end(joined);
        let selected = self.builder.build_phi(
            self.basic_type(&thaw_hir::caught_exception_carrier_type())?,
            "promise_original_carrier",
        ).map_err(|error| error.to_string())?;
        selected.add_incoming(&[(&native_carrier, native_from), (&direct_carrier, direct_from)]);
        Ok(selected.as_basic_value().into_struct_value())
    }

    /// Materialize a borrowed, arena-rooted Json value from the same active
    /// carrier passed to Promise rejection callbacks and lexical catches.
    fn compile_promise_rejection_borrowed_json(
        &mut self,
        promise: PointerValue<'ctx>,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let carrier = self.compile_promise_rejection_carrier(promise)?;
        let carrier_ty = thaw_hir::caught_exception_carrier_type();
        let llvm_ty = self.basic_type(&carrier_ty)?;
        let slot = self.builder.build_alloca(llvm_ty, "promise_reason_carrier")
            .map_err(|error| error.to_string())?;
        self.builder.build_store(slot, carrier).map_err(|error| error.to_string())?;
        let name = format!("__thaw_promise_reason_carrier_{}", self.next_lambda);
        let prior = self.variables.insert(name.clone(), (slot, llvm_ty));
        let prior_type = self.variable_hir_types.insert(name.clone(), carrier_ty);
        let expression = HirExpr::Call(
            Box::new(thaw_hir::caught_exception_json_adapter()),
            vec![HirExpr::Var(name.clone())],
        );
        let result = self.compile_expr(&expression);
        if let Some(previous) = prior { self.variables.insert(name.clone(), previous); }
        else { self.variables.remove(&name); }
        if let Some(previous) = prior_type { self.variable_hir_types.insert(name, previous); }
        else { self.variable_hir_types.remove(&name); }
        result
    }

    /// Native tagged text has one source-Promise-owned Error identity. Read
    /// the canonical Host first; on a miss, use the captured constructor and
    /// install one independent Promise-owned share while the source is held
    /// alive by the active reason callback. The returned Box is borrowed.
    fn compile_native_error_for_promise_source(
        &mut self,
        source: PointerValue<'ctx>,
        text: PointerValue<'ctx>,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let ptr = self.context.ptr_type(AddressSpace::default());
        let function = self.current_function();
        let valid_source = self.builder.build_is_not_null(source, "any_native_error_source_valid")
            .map_err(|error| error.to_string())?;
        let invalid_source = self.context.append_basic_block(function, "any_native_error_missing_source");
        let source_ready = self.context.append_basic_block(function, "any_native_error_source_ready");
        self.builder.build_conditional_branch(valid_source, source_ready, invalid_source)
            .map_err(|error| error.to_string())?;
        self.builder.position_at_end(invalid_source);
        self.compile_throw_type_error("Native Error lacks source Promise authority")?;
        self.builder.position_at_end(source_ready);
        let existing = self.builder.build_call(
            self.module.get_function("thaw_promise_source_canonical_error").unwrap(),
            &[source.into(), ptr.const_null().into()], "any_cached_native_error",
        ).map_err(|error| error.to_string())?.try_as_basic_value().basic()
            .ok_or("Promise.any cached Error returned no value")?.into_pointer_value();
        let has_existing = self.builder.build_is_not_null(existing, "any_has_canonical_error")
            .map_err(|error| error.to_string())?;
        let create = self.context.append_basic_block(function, "any_create_native_error");
        let join = self.context.append_basic_block(function, "any_native_error_ready");
        let cached = self.builder.get_insert_block().ok_or("missing cached Error block")?;
        self.builder.build_conditional_branch(has_existing, join, create)
            .map_err(|error| error.to_string())?;
        self.builder.position_at_end(create);
        let valid_text = self.builder.build_is_not_null(text, "any_native_error_text_valid")
            .map_err(|error| error.to_string())?;
        let invalid_text = self.context.append_basic_block(function, "any_native_error_missing_text");
        let text_ready = self.context.append_basic_block(function, "any_native_error_text_ready");
        self.builder.build_conditional_branch(valid_text, text_ready, invalid_text)
            .map_err(|error| error.to_string())?;
        self.builder.position_at_end(invalid_text);
        self.compile_throw_type_error("Native Error lacks trusted text")?;
        self.builder.position_at_end(text_ready);
        let construction = self.builder.build_call(
            self.module.get_function("thaw_js_new_error_from_tagged_result").unwrap(),
            &[text.into()], "construct_canonical_native_error",
        ).map_err(|error| error.to_string())?.try_as_basic_value().basic()
            .ok_or("Native Error construction returned no result")?.into_struct_value();
        let handle = self.builder.build_extract_value(construction, 0, "native_error_handle")
            .map_err(|error| error.to_string())?;
        let thrown = self.builder.build_extract_value(construction, 1, "native_error_throw")
            .map_err(|error| error.to_string())?.into_int_value();
        let infrastructure = self.builder.build_extract_value(construction, 3, "native_error_failure")
            .map_err(|error| error.to_string())?.into_pointer_value();
        let has_infrastructure = self.builder.build_is_not_null(infrastructure, "native_error_infrastructure_failed")
            .map_err(|error| error.to_string())?;
        let infrastructure_block = self.context.append_basic_block(function, "native_error_infrastructure");
        let check_throw = self.context.append_basic_block(function, "native_error_check_throw");
        self.builder.build_conditional_branch(has_infrastructure, infrastructure_block, check_throw)
            .map_err(|error| error.to_string())?;
        self.builder.position_at_end(infrastructure_block);
        self.builder.build_call(self.module.get_function("thaw_cstring_destroy").unwrap(),
            &[infrastructure.into()], "release_native_error_failure")
            .map_err(|error| error.to_string())?;
        self.compile_throw_type_error("Native Error construction failed")?;
        self.builder.position_at_end(check_throw);
        let did_throw = self.builder.build_int_compare(IntPredicate::NE, thrown,
            self.context.i64_type().const_zero(), "native_error_constructor_threw")
            .map_err(|error| error.to_string())?;
        let throw_block = self.context.append_basic_block(function, "native_error_original_throw");
        let success = self.context.append_basic_block(function, "native_error_constructed");
        self.builder.build_conditional_branch(did_throw, throw_block, success)
            .map_err(|error| error.to_string())?;
        self.builder.position_at_end(throw_block);
        self.compile_throw_original_quickjs_exception_handle(thrown.into(), true)?;
        self.builder.build_unconditional_branch(success).map_err(|error| error.to_string())?;
        self.builder.position_at_end(success);
        let candidate = self.compile_original_quickjs_exception_handle(handle, true)?
            .into_pointer_value();
        let installed = self.builder.build_call(
            self.module.get_function("thaw_promise_source_canonical_error").unwrap(),
            &[source.into(), candidate.into()], "install_canonical_native_error",
        ).map_err(|error| error.to_string())?.try_as_basic_value().basic()
            .ok_or("Promise.any canonical Error install returned no value")?;
        let installed = self.compile_check_json_host_error(installed, None)?.into_pointer_value();
        let valid_install = self.builder.build_is_not_null(installed,
            "native_error_install_valid").map_err(|error| error.to_string())?;
        let rejected_install = self.context.append_basic_block(function, "native_error_install_rejected");
        let install_ready = self.context.append_basic_block(function, "native_error_install_ready");
        self.builder.build_conditional_branch(valid_install, install_ready, rejected_install)
            .map_err(|error| error.to_string())?;
        self.builder.position_at_end(rejected_install);
        self.compile_throw_type_error("Native Error source changed during construction")?;
        self.builder.position_at_end(install_ready);
        let installed_from = self.builder.get_insert_block().ok_or("missing installed Error block")?;
        self.builder.build_unconditional_branch(join).map_err(|error| error.to_string())?;
        self.builder.position_at_end(join);
        let result = self.builder.build_phi(ptr, "any_canonical_native_error")
            .map_err(|error| error.to_string())?;
        result.add_incoming(&[(&existing, cached), (&installed, installed_from)]);
        Ok(result.as_basic_value())
    }

    fn compile_promise_any_native_error_borrowed_json(
        &mut self,
        reasons: PointerValue<'ctx>,
        index: IntValue<'ctx>,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        // Both accessors require the active, serial-validated reason scope;
        // the generic materializer also validates the live source pointer.
        let source = self.builder.build_call(
            self.module.get_function("thaw_promise_any_reason_record_source").unwrap(),
            &[reasons.into(), index.into()], "any_native_error_source",
        ).map_err(|error| error.to_string())?.try_as_basic_value().basic()
            .ok_or("Promise.any native Error source returned no value")?.into_pointer_value();
        let text = self.builder.build_call(
            self.module.get_function("thaw_promise_any_reason_record_text").unwrap(),
            &[reasons.into(), index.into()], "any_native_error_frame",
        ).map_err(|error| error.to_string())?.try_as_basic_value().basic()
            .ok_or("Promise.any native Error text returned no value")?.into_pointer_value();
        self.compile_native_error_for_promise_source(source, text)
    }

    /// A legacy nested Promise.any record owns its original source Promise
    /// and ordered child buffer for this callback. Recurse while that source
    /// is live, then install one Promise-owned AggregateError Host so a
    /// repeated or later aggregation observes the same exact Error object.
    fn compile_promise_any_nested_error_borrowed_json(
        &mut self,
        reasons: PointerValue<'ctx>,
        index: IntValue<'ctx>,
        materializer: FunctionValue<'ctx>,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let ptr = self.context.ptr_type(AddressSpace::default());
        let function = self.current_function();
        let source = self.builder.build_call(
            self.module.get_function("thaw_promise_any_reason_record_source").unwrap(),
            &[reasons.into(), index.into()], "nested_aggregate_source",
        ).map_err(|error| error.to_string())?.try_as_basic_value().basic()
            .ok_or("nested AggregateError source returned no value")?.into_pointer_value();
        let valid_source = self.builder.build_is_not_null(source, "nested_source_valid")
            .map_err(|error| error.to_string())?;
        let invalid_source = self.context.append_basic_block(function, "nested_source_missing");
        let source_ready = self.context.append_basic_block(function, "nested_source_ready");
        self.builder.build_conditional_branch(valid_source, source_ready, invalid_source)
            .map_err(|error| error.to_string())?;
        self.builder.position_at_end(invalid_source);
        self.compile_throw_type_error("Nested AggregateError lacks source authority")?;
        self.builder.position_at_end(source_ready);
        let existing = self.builder.build_call(
            self.module.get_function("thaw_promise_any_source_canonical_error").unwrap(),
            &[source.into(), ptr.const_null().into()], "nested_cached_aggregate",
        ).map_err(|error| error.to_string())?.try_as_basic_value().basic()
            .ok_or("nested cached AggregateError returned no value")?.into_pointer_value();
        let has_existing = self.builder.build_is_not_null(existing, "nested_has_canonical_aggregate")
            .map_err(|error| error.to_string())?;
        let cached = self.builder.get_insert_block().ok_or("missing nested cache block")?;
        let create = self.context.append_basic_block(function, "nested_materialize_aggregate");
        let joined = self.context.append_basic_block(function, "nested_aggregate_ready");
        self.builder.build_conditional_branch(has_existing, joined, create)
            .map_err(|error| error.to_string())?;
        self.builder.position_at_end(create);
        let children = self.builder.build_call(
            self.module.get_function("thaw_promise_any_reason_nested_errors").unwrap(),
            &[reasons.into(), index.into()], "nested_reason_owner",
        ).map_err(|error| error.to_string())?.try_as_basic_value().basic()
            .ok_or("nested AggregateError reasons returned no value")?.into_pointer_value();
        let valid_children = self.builder.build_is_not_null(children, "nested_reasons_valid")
            .map_err(|error| error.to_string())?;
        let invalid_children = self.context.append_basic_block(function, "nested_reasons_missing");
        let children_ready = self.context.append_basic_block(function, "nested_reasons_ready");
        self.builder.build_conditional_branch(valid_children, children_ready, invalid_children)
            .map_err(|error| error.to_string())?;
        self.builder.position_at_end(invalid_children);
        self.compile_throw_type_error("Nested AggregateError lacks ordered reason authority")?;
        self.builder.position_at_end(children_ready);
        let candidate = self.builder.build_call(materializer, &[children.into()],
            "materialize_nested_aggregate")
            .map_err(|error| error.to_string())?.try_as_basic_value().basic()
            .ok_or("nested AggregateError materializer returned no value")?.into_pointer_value();
        let valid_candidate = self.builder.build_is_not_null(candidate, "nested_aggregate_constructed")
            .map_err(|error| error.to_string())?;
        let rejected = self.context.append_basic_block(function, "nested_materialization_failed");
        let candidate_ready = self.context.append_basic_block(function, "nested_materialization_ready");
        self.builder.build_conditional_branch(valid_candidate, candidate_ready, rejected)
            .map_err(|error| error.to_string())?;
        self.builder.position_at_end(rejected);
        self.branch_on_pending_exception()?;
        self.compile_throw_type_error("Nested AggregateError materialization failed")?;
        self.builder.position_at_end(candidate_ready);
        let installed = self.builder.build_call(
            self.module.get_function("thaw_promise_any_source_canonical_error").unwrap(),
            &[source.into(), candidate.into()], "install_nested_aggregate",
        ).map_err(|error| error.to_string())?.try_as_basic_value().basic()
            .ok_or("nested AggregateError install returned no value")?;
        let installed = self.compile_check_json_host_error(installed, None)?.into_pointer_value();
        let valid_install = self.builder.build_is_not_null(installed, "nested_install_valid")
            .map_err(|error| error.to_string())?;
        let install_rejected = self.context.append_basic_block(function, "nested_install_rejected");
        let install_ready = self.context.append_basic_block(function, "nested_install_ready");
        self.builder.build_conditional_branch(valid_install, install_ready, install_rejected)
            .map_err(|error| error.to_string())?;
        self.builder.position_at_end(install_rejected);
        self.compile_throw_type_error("Nested AggregateError source changed during construction")?;
        self.builder.position_at_end(install_ready);
        let installed_from = self.builder.get_insert_block().ok_or("missing nested install block")?;
        self.builder.build_unconditional_branch(joined).map_err(|error| error.to_string())?;
        self.builder.position_at_end(joined);
        let selected = self.builder.build_phi(ptr, "canonical_nested_aggregate")
            .map_err(|error| error.to_string())?;
        selected.add_incoming(&[(&existing, cached), (&installed, installed_from)]);
        Ok(selected.as_basic_value())
    }

    /// The Promise.any buffer has a private tag map, distinct from pending
    /// exception tags. Read only through the runtime's callback-scoped,
    /// serial-validated accessors while its reason owner remains live. The
    /// returned Json is arena-managed and borrowed by the runtime projector.
    fn compile_promise_any_reason_borrowed_json(
        &mut self,
        reasons: PointerValue<'ctx>,
        index: IntValue<'ctx>,
        materializer: FunctionValue<'ctx>,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        self.tracks_owned_json_roots = true;
        let word = self.context.i64_type();
        let ptr = self.context.ptr_type(AddressSpace::default());
        let private_tag = self.builder.build_call(
            self.module.get_function("thaw_promise_any_reason_tag").unwrap(),
            &[reasons.into(), index.into()], "any_reason_private_tag",
        ).map_err(|error| error.to_string())?.try_as_basic_value().basic()
            .ok_or("Promise.any reason tag returned no value")?.into_int_value();
        let payload = self.builder.build_call(
            self.module.get_function("thaw_promise_any_reason_payload").unwrap(),
            &[reasons.into(), index.into()], "any_reason_private_payload",
        ).map_err(|error| error.to_string())?.try_as_basic_value().basic()
            .ok_or("Promise.any reason payload returned no value")?.into_int_value();
        // Private tags 0..6 map to pending 1..6,0; 10 is an independently
        // owned Json Box and maps to pending 7. Tag 9 canonicalizes native
        // Error text at its source Promise; nested 7 recursively constructs
        // and caches its AggregateError. Opaque 8 has no value authority.
        let simple = self.builder.build_int_compare(IntPredicate::ULE, private_tag,
            self.context.i8_type().const_int(6, false), "any_simple_reason")
            .map_err(|error| error.to_string())?;
        let json = self.builder.build_int_compare(IntPredicate::EQ, private_tag,
            self.context.i8_type().const_int(10, false), "any_json_reason")
            .map_err(|error| error.to_string())?;
        let native_error = self.builder.build_int_compare(IntPredicate::EQ, private_tag,
            self.context.i8_type().const_int(9, false), "any_native_error_reason")
            .map_err(|error| error.to_string())?;
        let nested_error = self.builder.build_int_compare(IntPredicate::EQ, private_tag,
            self.context.i8_type().const_int(7, false), "any_nested_error_reason")
            .map_err(|error| error.to_string())?;
        let direct = self.builder.build_or(simple, json, "any_direct_reason")
            .map_err(|error| error.to_string())?;
        let constructed = self.builder.build_or(native_error, nested_error,
            "any_constructed_reason").map_err(|error| error.to_string())?;
        let authorized = self.builder.build_or(direct, constructed, "any_reason_authorized")
            .map_err(|error| error.to_string())?;
        let function = self.current_function();
        let invalid = self.context.append_basic_block(function, "any_reason_unsupported");
        let ready = self.context.append_basic_block(function, "any_reason_ready");
        self.builder.build_conditional_branch(authorized, ready, invalid)
            .map_err(|error| error.to_string())?;
        self.builder.position_at_end(invalid);
        self.compile_throw_type_error("Promise.any reason requires trusted original-value projection")?;
        self.builder.position_at_end(ready);
        let native_block = self.context.append_basic_block(function, "any_native_error_reason");
        let nested_block = self.context.append_basic_block(function, "any_nested_error_reason");
        let constructed_block = self.context.append_basic_block(function, "any_constructed_reason");
        let direct_block = self.context.append_basic_block(function, "any_direct_reason");
        let joined = self.context.append_basic_block(function, "any_reason_json_ready");
        self.builder.build_conditional_branch(constructed, constructed_block, direct_block)
            .map_err(|error| error.to_string())?;
        self.builder.position_at_end(constructed_block);
        self.builder.build_conditional_branch(native_error, native_block, nested_block)
            .map_err(|error| error.to_string())?;
        self.builder.position_at_end(native_block);
        let native_value = self.compile_promise_any_native_error_borrowed_json(reasons, index)?;
        let native_from = self.builder.get_insert_block().ok_or("missing native Error reason block")?;
        self.builder.build_unconditional_branch(joined).map_err(|error| error.to_string())?;
        self.builder.position_at_end(nested_block);
        let nested_value = self.compile_promise_any_nested_error_borrowed_json(
            reasons, index, materializer)?;
        let nested_from = self.builder.get_insert_block().ok_or("missing nested Error reason block")?;
        self.builder.build_unconditional_branch(joined).map_err(|error| error.to_string())?;
        self.builder.position_at_end(direct_block);

        let mut exception_tag = word.const_zero();
        for (private, pending) in [(0, 1), (1, 2), (2, 3), (3, 4),
            (4, 5), (5, 6), (10, 7)] {
            let selected = self.builder.build_int_compare(IntPredicate::EQ, private_tag,
                self.context.i8_type().const_int(private, false), "any_reason_kind")
                .map_err(|error| error.to_string())?;
            exception_tag = self.builder.build_select(selected,
                word.const_int(pending, false), exception_tag, "any_pending_kind")
                .map_err(|error| error.to_string())?.into_int_value();
        }
        let text = self.builder.build_int_to_ptr(payload, ptr, "any_reason_text")
            .map_err(|error| error.to_string())?;
        let owner = self.builder.build_int_to_ptr(payload, ptr, "any_reason_owner")
            .map_err(|error| error.to_string())?;
        let number = self.builder.build_bit_cast(payload, self.context.f64_type(),
            "any_reason_number").map_err(|error| error.to_string())?.into_float_value();
        let boolean = self.builder.build_int_compare(IntPredicate::NE, payload,
            word.const_zero(), "any_reason_boolean").map_err(|error| error.to_string())?;
        // Tag 10 is borrowed from the AggregateReasonOwner. The packer takes
        // its own shallow, arena-rooted share before the callback may unwind.
        let carrier = self.build_caught_exception_carrier(
            text, owner, exception_tag, number, payload, boolean, true,
        )?;
        let carrier_ty = thaw_hir::caught_exception_carrier_type();
        let llvm_ty = self.basic_type(&carrier_ty)?;
        let slot = self.builder.build_alloca(llvm_ty, "any_reason_carrier")
            .map_err(|error| error.to_string())?;
        self.builder.build_store(slot, carrier).map_err(|error| error.to_string())?;
        let name = format!("__thaw_any_reason_carrier_{}", self.next_lambda);
        let previous = self.variables.insert(name.clone(), (slot, llvm_ty));
        let previous_ty = self.variable_hir_types.insert(name.clone(), carrier_ty);
        let expression = HirExpr::Call(Box::new(thaw_hir::caught_exception_json_adapter()),
            vec![HirExpr::Var(name.clone())]);
        let result = self.compile_expr(&expression);
        if let Some(old) = previous { self.variables.insert(name.clone(), old); }
        else { self.variables.remove(&name); }
        if let Some(old) = previous_ty { self.variable_hir_types.insert(name, old); }
        else { self.variable_hir_types.remove(&name); }
        let direct_value = result?;
        let direct_from = self.builder.get_insert_block().ok_or("missing direct reason block")?;
        self.builder.build_unconditional_branch(joined).map_err(|error| error.to_string())?;
        self.builder.position_at_end(joined);
        let selected = self.builder.build_phi(ptr, "any_original_reason_json")
            .map_err(|error| error.to_string())?;
        selected.add_incoming(&[
            (&native_value.into_pointer_value(), native_from),
            (&nested_value.into_pointer_value(), nested_from),
            (&direct_value.into_pointer_value(), direct_from),
        ]);
        Ok(selected.as_basic_value())
    }

    /// One canonical AggregateError is constructed before Promise.any settles.
    /// The callback is invoked with the reason owner active and returns a
    /// borrowed, arena-rooted Json Box; the runtime takes its own share.
    fn compile_promise_any_projector(&mut self) -> Result<FunctionValue<'ctx>, String> {
        let pointer = self.context.ptr_type(AddressSpace::default());
        let word = self.context.i64_type();
        let name = format!("__thaw_promise_any_projector_{}", self.next_lambda);
        self.next_lambda += 1;
        // This internal function is recursive for legacy nested Promise.any
        // reasons. The public runtime callback additionally settles its output
        // Promise when materialization returns null with a pending exception.
        let materializer = self.module.add_function(&format!("{name}_materialize"),
            pointer.fn_type(&[pointer.into()], false), Some(Linkage::Internal));
        let callback = self.module.add_function(&name,
            pointer.fn_type(&[pointer.into(), pointer.into()], false),
            Some(Linkage::Internal));
        let caller = self.builder.get_insert_block().ok_or("Promise.any has no caller block")?;
        let entry = self.context.append_basic_block(materializer, "entry");
        let loop_head = self.context.append_basic_block(materializer, "reason_head");
        let loop_body = self.context.append_basic_block(materializer, "reason_body");
        let complete = self.context.append_basic_block(materializer, "construct_aggregate");
        let failed = self.context.append_basic_block(materializer, "aggregate_failed");
        let saved_catch = std::mem::take(&mut self.catch_stack);
        let saved_catch_owner_roots = std::mem::take(&mut self.catch_owner_roots);
        let saved_completion = self.active_async_completion.take();
        self.catch_stack.push(failed);
        let generated = (|| -> Result<(), String> {
            self.builder.position_at_end(entry);
            // This may be the first QuickJS-backed value in the program.
            // Register Host retain/release before any reason conversion or
            // graph decode can create a Host lease.
            self.uses_quickjs = true;
            self.uses_quickjs_handles = true;
            self.tracks_owned_json_roots = true;
            self.compile_register_js_callback_host_operations()?;
            let reasons = materializer.get_nth_param(0).unwrap().into_pointer_value();
            let array = self.builder.build_call(
                self.module.get_function("thaw_json_array_new").unwrap(), &[], "any_errors_array",
            ).map_err(|error| error.to_string())?.try_as_basic_value().basic()
                .ok_or("Promise.any errors array allocation returned no value")?.into_pointer_value();
            // A reason getter, Host clone, or graph encoder can unwind. Root
            // the array now so every path retains its children until reset;
            // an ordinary successful path explicitly destroys it after wire
            // encoding, and thaw_json_destroy untracks that root.
            let array = self.builder.build_call(
                self.module.get_function("thaw_json_track_arena_owned_root").unwrap(),
                &[array.into()], "root_any_errors_array",
            ).map_err(|error| error.to_string())?.try_as_basic_value().basic()
                .ok_or("Promise.any errors array root returned no value")?;
            let array = self.compile_check_json_host_error(array, None)?.into_pointer_value();
            let count = self.builder.build_call(
                self.module.get_function("thaw_promise_any_reason_count").unwrap(),
                &[reasons.into()], "any_reason_count",
            ).map_err(|error| error.to_string())?.try_as_basic_value().basic()
                .ok_or("Promise.any reason count returned no value")?.into_int_value();
            let index_slot = self.builder.build_alloca(word, "any_reason_index")
                .map_err(|error| error.to_string())?;
            self.builder.build_store(index_slot, word.const_zero()).map_err(|error| error.to_string())?;
            self.builder.build_unconditional_branch(loop_head).map_err(|error| error.to_string())?;
            self.builder.position_at_end(loop_head);
            let index = self.builder.build_load(word, index_slot, "any_reason_cursor")
                .map_err(|error| error.to_string())?.into_int_value();
            let more = self.builder.build_int_compare(IntPredicate::ULT, index, count,
                "any_reason_more").map_err(|error| error.to_string())?;
            self.builder.build_conditional_branch(more, loop_body, complete)
                .map_err(|error| error.to_string())?;
            self.builder.position_at_end(loop_body);
            let value = self.compile_promise_any_reason_borrowed_json(reasons, index, materializer)?;
            self.builder.build_call(self.module.get_function("thaw_json_array_push_json").unwrap(),
                &[array.into(), value.into()], "append_any_reason")
                .map_err(|error| error.to_string())?;
            self.compile_check_json_host_error(array.into(), Some("thaw_json_destroy"))?;
            let next = self.builder.build_int_add(index, word.const_int(1, false),
                "next_any_reason").map_err(|error| error.to_string())?;
            self.builder.build_store(index_slot, next).map_err(|error| error.to_string())?;
            self.builder.build_unconditional_branch(loop_head).map_err(|error| error.to_string())?;
            self.builder.position_at_end(complete);
            let graph = self.builder.build_call(
                self.module.get_function("thaw_json_graph_encode").unwrap(),
                &[array.into()], "encode_any_errors",
            ).map_err(|error| error.to_string())?.try_as_basic_value().basic()
                .ok_or("Promise.any errors graph returned no value")?;
            let graph = self.compile_check_json_stringify_error_with_cleanup(graph, &[array.into()])?
                .into_pointer_value();
            self.builder.build_call(self.module.get_function("thaw_json_destroy").unwrap(),
                &[array.into()], "release_any_errors_array")
                .map_err(|error| error.to_string())?;
            let constructed = self.builder.build_call(
                self.module.get_function("thaw_js_new_aggregate_error_graph_result").unwrap(),
                &[graph.into()], "construct_real_aggregate_error",
            ).map_err(|error| error.to_string())?.try_as_basic_value().basic()
                .ok_or("AggregateError constructor returned no result")?.into_struct_value();
            self.builder.build_call(self.module.get_function("thaw_cstring_destroy").unwrap(),
                &[graph.into()], "release_any_errors_graph")
                .map_err(|error| error.to_string())?;
            let handle = self.builder.build_extract_value(constructed, 0, "aggregate_error_handle")
                .map_err(|error| error.to_string())?;
            let thrown = self.builder.build_extract_value(constructed, 1, "aggregate_constructor_throw")
                .map_err(|error| error.to_string())?.into_int_value();
            let error = self.builder.build_extract_value(constructed, 3, "aggregate_constructor_error")
                .map_err(|error| error.to_string())?.into_pointer_value();
            let error_present = self.builder.build_is_not_null(error, "aggregate_infrastructure_error")
                .map_err(|error| error.to_string())?;
            let infrastructure = self.context.append_basic_block(materializer, "aggregate_infrastructure_failure");
            let check_throw = self.context.append_basic_block(materializer, "aggregate_check_throw");
            self.builder.build_conditional_branch(error_present, infrastructure, check_throw)
                .map_err(|error| error.to_string())?;
            self.builder.position_at_end(infrastructure);
            self.builder.build_call(self.module.get_function("thaw_cstring_destroy").unwrap(),
                &[error.into()], "release_aggregate_infrastructure_error")
                .map_err(|error| error.to_string())?;
            self.compile_throw_type_error("AggregateError construction failed")?;
            self.builder.position_at_end(check_throw);
            let did_throw = self.builder.build_int_compare(IntPredicate::NE, thrown,
                word.const_zero(), "aggregate_constructor_threw")
                .map_err(|error| error.to_string())?;
            let exact_throw = self.context.append_basic_block(materializer, "aggregate_exact_throw");
            let success = self.context.append_basic_block(materializer, "aggregate_success");
            self.builder.build_conditional_branch(did_throw, exact_throw, success)
                .map_err(|error| error.to_string())?;
            self.builder.position_at_end(exact_throw);
            self.compile_throw_original_quickjs_exception_handle(thrown.into(), true)?;
            self.builder.build_unconditional_branch(failed).map_err(|error| error.to_string())?;
            self.builder.position_at_end(success);
            let original = self.compile_original_quickjs_exception_handle(handle, true)?;
            self.builder.build_return(Some(&original)).map_err(|error| error.to_string())?;
            self.builder.position_at_end(failed);
            self.builder.build_return(Some(&pointer.const_null()))
                .map_err(|error| error.to_string())?;
            Ok(())
        })();
        self.catch_stack = saved_catch;
        self.catch_owner_roots = saved_catch_owner_roots.clone();
        self.active_async_completion = saved_completion;
        self.builder.position_at_end(caller);
        generated?;
        let outer_entry = self.context.append_basic_block(callback, "entry");
        let outer_failed = self.context.append_basic_block(callback, "materialization_failed");
        let outer_success = self.context.append_basic_block(callback, "materialization_ready");
        self.builder.position_at_end(outer_entry);
        let reasons = callback.get_nth_param(0).unwrap().into_pointer_value();
        let output = callback.get_nth_param(1).unwrap().into_pointer_value();
        let result = self.builder.build_call(materializer, &[reasons.into()],
            "materialize_aggregate_error").map_err(|error| error.to_string())?
            .try_as_basic_value().basic()
            .ok_or("AggregateError materializer returned no value")?.into_pointer_value();
        let has_result = self.builder.build_is_not_null(result, "aggregate_materialized")
            .map_err(|error| error.to_string())?;
        self.builder.build_conditional_branch(has_result, outer_success, outer_failed)
            .map_err(|error| error.to_string())?;
        self.builder.position_at_end(outer_failed);
        let pending = self.builder.build_load(pointer,
            self.pending_exception().as_pointer_value(), "aggregate_pending_value")
            .map_err(|error| error.to_string())?.into_pointer_value();
        self.reject_promise_with_pending_exception(output, pending, "reject_aggregate_construction")?;
        // The output has its own rejection share. This compiler-private
        // callback is invoked from the runtime, so no enclosing generated
        // caller will clear the thread's pending tuple for it.
        self.builder.build_store(self.pending_exception().as_pointer_value(),
            pointer.const_null()).map_err(|error| error.to_string())?;
        self.builder.build_store(self.pending_exception_object().as_pointer_value(),
            pointer.const_null()).map_err(|error| error.to_string())?;
        self.builder.build_store(
            self.pending_exception_value(PENDING_EXCEPTION_VALUE_TAG_SYMBOL).as_pointer_value(),
            word.const_zero(),
        ).map_err(|error| error.to_string())?;
        self.builder.build_store(self.pending_exception_aggregate_errors().as_pointer_value(),
            pointer.const_null()).map_err(|error| error.to_string())?;
        self.clear_pending_native_text()?;
        self.builder.build_return(Some(&pointer.const_null())).map_err(|error| error.to_string())?;
        self.builder.position_at_end(outer_success);
        self.builder.build_return(Some(&result)).map_err(|error| error.to_string())?;
        self.builder.position_at_end(caller);
        Ok(callback)
    }

    /// Transfer a rejected Promise's payload before the Promise is destroyed.
    /// Tag 7 owns a JSON Box; the runtime gives pending/catch an independent
    /// arena-rooted share. Other tags retain their original pointer ABI.
    fn copy_blocking_promise_exception_metadata(
        &mut self,
        promise: PointerValue<'ctx>,
    ) -> Result<(), String> {
        // A tag-7 rejection can create an arena-owned pending JSON share even
        // when no other expression projects or owns JSON values.
        self.tracks_owned_json_roots = true;
        for (getter, target) in [
            ("thaw_promise_exception_tag", self.pending_exception_value(PENDING_EXCEPTION_VALUE_TAG_SYMBOL)),
            ("thaw_promise_exception_f64", self.pending_exception_value(PENDING_EXCEPTION_F64_SYMBOL)),
            ("thaw_promise_exception_i64", self.pending_exception_value(PENDING_EXCEPTION_I64_SYMBOL)),
            ("thaw_promise_exception_bool", self.pending_exception_value(PENDING_EXCEPTION_BOOL_SYMBOL)),
            ("thaw_promise_exception_aggregate_errors", self.pending_exception_aggregate_errors()),
        ] {
            let value = self.builder.build_call(
                self.module.get_function(getter).unwrap(), &[promise.into()],
                "blocking_await_exception_value",
            ).map_err(|error| error.to_string())?.try_as_basic_value().basic()
                .ok_or_else(|| format!("{getter} returned no value"))?;
            self.builder.build_store(target.as_pointer_value(), value)
                .map_err(|error| error.to_string())?;
        }
        let tag = self.builder.build_load(
            self.context.i64_type(),
            self.pending_exception_value(PENDING_EXCEPTION_VALUE_TAG_SYMBOL).as_pointer_value(),
            "blocking_await_pending_tag",
        ).map_err(|error| error.to_string())?.into_int_value();
        let object = self.builder.build_call(
            self.module.get_function("thaw_promise_exception_pending_object").unwrap(),
            &[promise.into()], "blocking_await_transferred_object",
        ).map_err(|error| error.to_string())?.try_as_basic_value().basic()
            .ok_or("Promise pending object transfer returned no value")?.into_pointer_value();
        let native_text = self.builder.build_call(
            self.module.get_function("thaw_promise_exception_native_text_copy").unwrap(),
            &[promise.into()], "blocking_canonical_native_text",
        ).map_err(|error| error.to_string())?.try_as_basic_value().basic()
            .ok_or("Promise native Error text returned no value")?.into_pointer_value();
        let aggregate = self.builder.build_load(
            self.context.ptr_type(AddressSpace::default()),
            self.pending_exception_aggregate_errors().as_pointer_value(),
            "blocking_aggregate_reason_owner",
        ).map_err(|error| error.to_string())?.into_pointer_value();
        let is_native = self.builder.build_int_compare(IntPredicate::EQ, tag,
            self.context.i64_type().const_zero(), "blocking_native_error_kind")
            .map_err(|error| error.to_string())?;
        let missing_owner = self.builder.build_is_null(object, "blocking_native_error_owner_missing")
            .map_err(|error| error.to_string())?;
        let has_text = self.builder.build_is_not_null(native_text, "blocking_native_error_text_present")
            .map_err(|error| error.to_string())?;
        let without_aggregate = self.builder.build_is_null(aggregate, "blocking_native_error_not_aggregate")
            .map_err(|error| error.to_string())?;
        let native = self.builder.build_and(is_native, missing_owner,
            "blocking_native_no_object").map_err(|error| error.to_string())?;
        let native = self.builder.build_and(native, has_text, "blocking_native_trusted_text")
            .map_err(|error| error.to_string())?;
        let native = self.builder.build_and(native, without_aggregate,
            "blocking_native_plain_error").map_err(|error| error.to_string())?;
        let function = self.current_function();
        let native_branch = self.context.append_basic_block(function, "blocking_materialize_native_error");
        let direct_branch = self.context.append_basic_block(function, "blocking_transfer_direct_reason");
        let done = self.context.append_basic_block(function, "blocking_transfer_done");
        self.builder.build_conditional_branch(native, native_branch, direct_branch)
            .map_err(|error| error.to_string())?;
        self.builder.position_at_end(native_branch);
        // Constructor/capture/install can reenter user code. A local catch
        // owns failure cleanup so an exception never skips the source release.
        let native_failed = self.context.append_basic_block(function, "blocking_native_error_failed");
        self.catch_stack.push(native_failed);
        let canonical = self.compile_native_error_for_promise_source(promise, native_text);
        if canonical.is_err() { self.catch_stack.pop(); }
        let _canonical = canonical?.into_pointer_value();
        let transferred = self.builder.build_call(
            self.module.get_function("thaw_promise_exception_pending_object").unwrap(),
            &[promise.into()], "blocking_canonical_error_share",
        ).map_err(|error| error.to_string())?.try_as_basic_value().basic()
            .ok_or("canonical pending Error transfer returned no value")?;
        let transferred = self.compile_check_json_host_error(transferred, None)?.into_pointer_value();
        let share_valid = self.builder.build_is_not_null(transferred, "blocking_canonical_share_valid")
            .map_err(|error| error.to_string())?;
        let share_failed = self.context.append_basic_block(function, "blocking_canonical_share_missing");
        let share_ready = self.context.append_basic_block(function, "blocking_canonical_share_ready");
        self.builder.build_conditional_branch(share_valid, share_ready, share_failed)
            .map_err(|error| error.to_string())?;
        self.builder.position_at_end(share_failed);
        self.compile_throw_type_error("Native Error pending share was not retained")?;
        self.builder.position_at_end(share_ready);
        self.builder.build_store(self.pending_exception().as_pointer_value(), transferred)
            .map_err(|error| error.to_string())?;
        self.builder.build_store(self.pending_exception_object().as_pointer_value(), transferred)
            .map_err(|error| error.to_string())?;
        self.builder.build_store(self.pending_exception_json_owner().as_pointer_value(), transferred)
            .map_err(|error| error.to_string())?;
        self.builder.build_store(
            self.pending_exception_value(PENDING_EXCEPTION_VALUE_TAG_SYMBOL).as_pointer_value(),
            self.context.i64_type().const_int(7, false),
        ).map_err(|error| error.to_string())?;
        self.clear_pending_native_text()?;
        self.builder.build_unconditional_branch(done).map_err(|error| error.to_string())?;
        self.catch_stack.pop();
        self.builder.position_at_end(native_failed);
        self.builder.build_call(self.module.get_function("thaw_promise_destroy").unwrap(),
            &[promise.into()], "destroy_failed_native_error_source")
            .map_err(|error| error.to_string())?;
        self.branch_on_pending_exception()?;
        self.compile_throw_type_error("Native Error materialization failed without exception")?;
        self.builder.position_at_end(direct_branch);
        let is_json = self.builder.build_int_compare(
            IntPredicate::EQ, tag, self.context.i64_type().const_int(7, false),
            "blocking_await_json_rejection",
        ).map_err(|error| error.to_string())?;
        let missing = self.builder.build_is_null(object, "blocking_await_json_transfer_missing")
            .map_err(|error| error.to_string())?;
        let failed = self.builder.build_and(is_json, missing, "blocking_await_json_transfer_failed")
            .map_err(|error| error.to_string())?;
        let failure = self.context.append_basic_block(function, "blocking_await_transfer_failure");
        let transferred = self.context.append_basic_block(function, "blocking_await_transfer_success");
        self.builder.build_conditional_branch(failed, failure, transferred)
            .map_err(|error| error.to_string())?;
        self.builder.position_at_end(transferred);
        self.builder.build_store(self.pending_exception_object().as_pointer_value(), object)
            .map_err(|error| error.to_string())?;
        let selected_owner = self.builder.build_select(is_json, object,
            self.context.ptr_type(AddressSpace::default()).const_null(),
            "blocking_await_owned_json")
            .map_err(|error| error.to_string())?;
        self.builder.build_store(self.pending_exception_json_owner().as_pointer_value(), selected_owner)
            .map_err(|error| error.to_string())?;
        let json_primary = self.context.append_basic_block(function, "blocking_await_json_primary");
        self.builder.build_conditional_branch(is_json, json_primary, done)
            .map_err(|error| error.to_string())?;
        self.builder.position_at_end(json_primary);
        self.builder.build_store(self.pending_exception().as_pointer_value(), object)
            .map_err(|error| error.to_string())?;
        self.builder.build_unconditional_branch(done).map_err(|error| error.to_string())?;
        self.builder.position_at_end(failure);
        // Consume HostError before the common typed/void caller destroys the
        // Promise; destruction may itself release reentrant Host values.
        let error = self.builder.build_call(
            self.module.get_function("thaw_json_take_host_error").unwrap(),
            &[], "blocking_await_transfer_host_error",
        ).map_err(|error| error.to_string())?.try_as_basic_value().basic()
            .ok_or("Promise pending transfer HostError returned no value")?.into_pointer_value();
        self.builder.build_store(self.pending_exception().as_pointer_value(), error)
            .map_err(|error| error.to_string())?;
        self.builder.build_store(
            self.pending_exception_value(PENDING_EXCEPTION_VALUE_TAG_SYMBOL).as_pointer_value(),
            self.context.i64_type().const_zero(),
        ).map_err(|error| error.to_string())?;
        self.builder.build_store(
            self.pending_exception_object().as_pointer_value(),
            self.context.ptr_type(AddressSpace::default()).const_null(),
        ).map_err(|error| error.to_string())?;
        self.builder.build_store(self.pending_exception_json_owner().as_pointer_value(),
            self.context.ptr_type(AddressSpace::default()).const_null())
            .map_err(|error| error.to_string())?;
        self.clear_pending_native_text()?;
        self.mark_pending_owned_native_text(error)?;
        self.builder.build_unconditional_branch(done).map_err(|error| error.to_string())?;
        self.builder.position_at_end(done);
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
    /// Project one settled fulfillment into this read's contextual T. Native
    /// producers still expose their established physical T bytes. A realm
    /// Promise keeps a canonical Json value, so decode a fresh owned clone;
    /// the Promise's own payload remains valid for later, differently typed
    /// aliases and for forwarding subscribers.
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
        let failed_incoming = self.builder.get_insert_block().unwrap();
        let default = llvm_type.const_zero();
        self.builder
            .build_unconditional_branch(merge)
            .map_err(|error| error.to_string())?;
        self.builder.position_at_end(succeeded);
        let projection_failed = self.context.append_basic_block(function, "blocking_projection_failed");
        self.catch_stack.push(projection_failed);
        let projected = self.compile_promise_fulfillment_as_type(promise, result, resolved);
        self.catch_stack.pop();
        let value = projected?;
        let succeeded_end = self.builder.get_insert_block()
            .ok_or("blocking Promise projection has no end block")?;
        self.builder
            .build_unconditional_branch(merge)
            .map_err(|error| error.to_string())?;
        self.builder.position_at_end(projection_failed);
        self.builder.build_call(
            self.module.get_function("thaw_promise_destroy").unwrap(),
            &[promise.into()], "destroy_failed_blocking_projection",
        ).map_err(|error| error.to_string())?;
        self.branch_on_pending_exception()?;
        self.builder.build_unreachable().map_err(|error| error.to_string())?;
        self.builder.position_at_end(merge);
        let phi = self
            .builder
            .build_phi(llvm_type, "blocking_await_value")
            .map_err(|error| error.to_string())?;
        phi.add_incoming(&[(&default, failed_incoming), (&value, succeeded_end)]);
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
