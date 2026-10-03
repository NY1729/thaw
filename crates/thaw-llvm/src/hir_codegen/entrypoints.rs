impl<'ctx> HirCompiler<'ctx> {
    /// Emits `int main(void) { thaw_user_main(); return 0; }`, the real
    /// process entry point that the system linker/CRT expects, for
    /// ordinary (non-Lambda) programs that define `main`.
    fn emit_c_main_entry(&mut self) {
        let user_main = self.module.get_function(USER_MAIN_SYMBOL);

        let (main_fn, entry) = self.new_c_main();
        let cleanup = self.context.append_basic_block(main_fn, "entry_cleanup");
        self.builder.position_at_end(entry);
        let quickjs_failure = self
            .builder
            .build_alloca(self.context.i32_type(), "quickjs_event_loop_status")
            .unwrap();
        self.builder
            .build_store(quickjs_failure, self.context.i32_type().const_zero())
            .unwrap();
        // Live native-object projections can retain arena allocations from
        // ordinary `main` through a QuickJS wrapper. Enable tracking before
        // module initialization creates any such object, and keep native
        // globals and pending exceptions rooted across arena resets.
        if self.projects_native_objects {
            let pointer = self.context.ptr_type(AddressSpace::default());
            let enable = self.module.add_function("thaw_arena_enable_tracing",
                self.context.void_type().fn_type(&[], false), Some(Linkage::External));
            self.builder.build_call(enable, &[], "enable_native_object_tracing").unwrap();
            let register = self.module.add_function("thaw_arena_register_root",
                self.context.void_type().fn_type(&[pointer.into()], false),
                Some(Linkage::External));
            for (slot, ty, _) in self.global_variables.values().cloned().collect::<Vec<_>>() {
                self.register_arena_root_slots(register, slot, ty).unwrap();
            }
            for slot in &self.module_exception_roots {
                self.builder.build_call(register, &[(*slot).into()],
                    "register_native_object_exception_root").unwrap();
            }
        }
        // Module initialization can fail after loading an addon. Every exit
        // must have the process reporters installed before reaching cleanup.
        self.configure_unhandled_rejection_reporter();
        self.call_module_init_if_present(cleanup);
        if self.uses_quickjs {
            let status = self
                .builder
                .build_call(
                    self.module.get_function("thaw_js_run_cli").unwrap(),
                    &[],
                    "run_quickjs_cli",
                )
                .unwrap()
                .try_as_basic_value()
                .basic()
                .unwrap()
                .into_int_value();
            self.builder.build_store(quickjs_failure, status).unwrap();
            let handled = self
                .builder
                .build_int_compare(
                    IntPredicate::SGE,
                    status,
                    status.get_type().const_zero(),
                    "quickjs_cli_handled",
                )
                .unwrap();
            let ordinary_entry = self.context.append_basic_block(main_fn, "ordinary_entry");
            self.builder
                .build_conditional_branch(handled, cleanup, ordinary_entry)
                .unwrap();
            self.builder.position_at_end(ordinary_entry);
            self.builder
                .build_store(quickjs_failure, self.context.i32_type().const_zero())
                .unwrap();
        }
        let main_returns_promise = self.frame_async_functions.contains_key("main")
            || self.promise_returning_functions.contains("main");
        let completion = user_main.and_then(|user_main| {
            self.builder
                .build_call(user_main, &[], "call_thaw_user_main")
                .unwrap()
                .try_as_basic_value()
                .basic()
        });
        if let Some(completion) = completion.filter(|_| main_returns_promise) {
            let completion = completion.into_pointer_value();
            self.builder
                .build_call(
                    self.module
                        .get_function("thaw_runtime_run_until_resolved")
                        .unwrap(),
                    &[completion.into()],
                    "run_async_main",
                )
                .unwrap();
            if self.uses_napi {
                self.builder
                    .build_call(
                        self.module
                                .get_function("thaw_napi_run_async_work")
                                .unwrap(),
                        &[],
                        "drain_napi_for_async_main",
                    )
                    .unwrap();
                self.builder
                    .build_call(
                        self.module
                            .get_function("thaw_runtime_run_until_resolved")
                            .unwrap(),
                        &[completion.into()],
                        "resume_async_main_after_napi",
                    )
                    .unwrap();
            }
            if self.uses_quickjs {
                self.builder
                    .build_call(
                        self.module
                            .get_function("thaw_js_run_until_native_resolved")
                            .unwrap(),
                        &[completion.into()],
                        "interleave_quickjs_for_async_main",
                    )
                    .unwrap();
            }
            if self.uses_napi && self.uses_quickjs {
                self.builder
                    .build_call(
                        self.module
                            .get_function("thaw_napi_run_async_work")
                            .unwrap(),
                        &[],
                        "drain_napi_after_quickjs",
                    )
                    .unwrap();
                self.builder
                    .build_call(
                        self.module
                            .get_function("thaw_runtime_run_until_resolved")
                            .unwrap(),
                        &[completion.into()],
                        "resume_async_main_after_napi_quickjs",
                    )
                    .unwrap();
            }
            self.builder
                .build_call(
                    self.module.get_function("thaw_promise_destroy").unwrap(),
                    &[completion.into()],
                    "destroy_async_main",
                )
                .unwrap();
            self.builder
                .build_call(
                    self.module
                        .get_function("thaw_runtime_drain_detached")
                        .unwrap(),
                    &[],
                    "drain_detached_promises",
                )
                .unwrap();
        }
        self.builder
            .build_call(
                self.module
                    .get_function("thaw_runtime_run_until_idle")
                    .unwrap(),
                &[],
                "drain_native_microtasks",
            )
            .unwrap();
        if self.uses_quickjs {
            let status = self
                .builder
                .build_call(
                    self.module.get_function("thaw_js_run_event_loop").unwrap(),
                    &[],
                    "run_quickjs_event_loop",
                )
                .unwrap()
                .try_as_basic_value()
                .basic()
                .unwrap();
            self.builder.build_store(quickjs_failure, status).unwrap();
        }
        if self.module.get_function("createServer").is_some() {
            self.builder
                .build_call(
                    self.module.get_function("thaw_http_run_servers").unwrap(),
                    &[],
                    "run_http_servers",
                )
                .unwrap();
        }
        if self.uses_napi {
            self.builder
                .build_call(
                    self.module
                        .get_function("thaw_napi_run_async_work")
                        .unwrap(),
                    &[],
                    "drain_napi_async_work",
                )
                .unwrap();
            if self.uses_quickjs {
                let status = self
                    .builder
                    .build_call(
                        self.module.get_function("thaw_js_run_event_loop").unwrap(),
                        &[],
                        "run_quickjs_after_napi",
                    )
                    .unwrap()
                    .try_as_basic_value()
                    .basic()
                    .unwrap()
                    .into_int_value();
                let previous = self
                    .builder
                    .build_load(self.context.i32_type(), quickjs_failure, "previous_quickjs_status")
                    .unwrap()
                    .into_int_value();
                let has_status = self
                    .builder
                    .build_int_compare(
                        IntPredicate::NE,
                        status,
                        status.get_type().const_zero(),
                        "has_quickjs_status",
                    )
                    .unwrap();
                let combined = self
                    .builder
                    .build_select(has_status, status, previous, "combined_quickjs_status")
                    .unwrap();
                self.builder.build_store(quickjs_failure, combined).unwrap();
            }
        }
        self.builder.build_unconditional_branch(cleanup).unwrap();
        self.builder.position_at_end(cleanup);
        self.finish_c_main(Some(quickjs_failure));
    }

    /// Calls `__thaw_module_init` before user code, if the program defines
    /// one (see `MODULE_INIT_SYMBOL`). A no-op for programs with no
    /// registry packages that ship a `bundle.js`.
    fn call_module_init_if_present(&self, cleanup: BasicBlock<'ctx>) {
        for (symbol, call_name) in [
            (NATIVE_MODULE_INIT_SYMBOL, "call_thaw_native_module_init"),
            (MODULE_INIT_SYMBOL, "call_thaw_module_init"),
            (TOP_LEVEL_INIT_SYMBOL, "call_thaw_top_level_init"),
        ] {
            if let Some(init_fn) = self.module.get_function(symbol) {
                self.builder.build_call(init_fn, &[], call_name).unwrap();
                let function = self
                    .builder
                    .get_insert_block()
                    .and_then(|block| block.get_parent())
                    .unwrap();
                let continue_block = self
                    .context
                    .append_basic_block(function, &format!("{call_name}_ok"));
                let pending = self
                    .builder
                    .build_load(
                        self.context.ptr_type(AddressSpace::default()),
                        self.pending_exception().as_pointer_value(),
                        "init_pending_exception",
                    )
                    .unwrap()
                    .into_pointer_value();
                let failed = self
                    .builder
                    .build_is_not_null(pending, "init_failed")
                    .unwrap();
                self.builder
                    .build_conditional_branch(failed, cleanup, continue_block)
                    .unwrap();
                self.builder.position_at_end(continue_block);
            }
        }
    }

    fn emit_json_handler_adapter(
        &mut self,
        handler_fn: FunctionValue<'ctx>,
        is_frame_async: bool,
    ) -> Result<FunctionValue<'ctx>, String> {
        let ptr_ty = self.context.ptr_type(AddressSpace::default());
        let adapter = self.module.add_function(
            "__thaw_json_handler_adapter",
            ptr_ty.fn_type(&[ptr_ty.into()], false),
            Some(Linkage::Internal),
        );
        let entry = self.context.append_basic_block(adapter, "entry");
        self.builder.position_at_end(entry);
        let event_text = adapter.get_first_param().unwrap().into_pointer_value();
        let parse = self.module.get_function("thaw_json_parse").unwrap();
        let event = self
            .builder
            .build_call(parse, &[event_text.into()], "lambda_event_json")
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .ok_or("thaw_json_parse did not return a value")?;
        let call = self
            .builder
            .build_call(handler_fn, &[event.into()], "call_json_handler")
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .ok_or("JSON handler did not return a value")?;

        let result = if is_frame_async {
            let promise = call.into_pointer_value();
            let result_slot = self
                .builder
                .build_call(
                    self.module
                        .get_function("thaw_runtime_run_until_resolved")
                        .unwrap(),
                    &[promise.into()],
                    "await_json_handler",
                )
                .map_err(|error| error.to_string())?
                .try_as_basic_value()
                .basic()
                .ok_or("async JSON handler did not settle")?
                .into_pointer_value();
            let promise_state = self
                .builder
                .build_call(
                    self.module.get_function("thaw_promise_state").unwrap(),
                    &[promise.into()],
                    "json_handler_state",
                )
                .map_err(|error| error.to_string())?
                .try_as_basic_value()
                .basic()
                .unwrap()
                .into_int_value();
            let rejected = self
                .builder
                .build_int_compare(
                    IntPredicate::EQ,
                    promise_state,
                    self.context.i8_type().const_int(2, false),
                    "json_handler_rejected",
                )
                .map_err(|error| error.to_string())?;
            let failed = self.context.append_basic_block(adapter, "handler_rejected");
            let succeeded = self
                .context
                .append_basic_block(adapter, "handler_fulfilled");
            self.builder
                .build_conditional_branch(rejected, failed, succeeded)
                .map_err(|error| error.to_string())?;
            self.builder.position_at_end(failed);
            self.builder
                .build_store(self.pending_exception().as_pointer_value(), result_slot)
                .map_err(|error| error.to_string())?;
            self.clear_pending_native_text()?;
            let native_text = self.builder.build_call(
                self.module.get_function("thaw_promise_exception_native_text_copy").unwrap(),
                &[promise.into()], "json_handler_native_text",
            ).map_err(|error| error.to_string())?.try_as_basic_value().basic()
                .ok_or("native text copy returned no value")?.into_pointer_value();
            let has_native_text = self.builder.build_is_not_null(native_text, "json_handler_native_text_present")
                .map_err(|error| error.to_string())?;
            let pending_value = self.builder.build_select(
                has_native_text, native_text, result_slot, "json_handler_pending_value",
            ).map_err(|error| error.to_string())?;
            self.builder.build_store(self.pending_exception().as_pointer_value(), pending_value)
                .map_err(|error| error.to_string())?;
            self.mark_pending_native_text(native_text)?;
            let aggregate = self.builder.build_call(
                self.module.get_function("thaw_promise_exception_aggregate_errors").unwrap(),
                &[promise.into()], "json_handler_aggregate_errors",
            ).map_err(|error| error.to_string())?.try_as_basic_value().basic()
                .ok_or("aggregate errors getter returned no value")?;
            self.builder.build_store(self.pending_exception_aggregate_errors().as_pointer_value(), aggregate)
                .map_err(|error| error.to_string())?;
            self.builder
                .build_call(
                    self.module.get_function("thaw_promise_destroy").unwrap(),
                    &[promise.into()],
                    "destroy_rejected_json_handler",
                )
                .map_err(|error| error.to_string())?;
            self.builder
                .build_return(Some(&ptr_ty.const_null()))
                .map_err(|error| error.to_string())?;

            self.builder.position_at_end(succeeded);
            let result = self
                .builder
                .build_load(ptr_ty, result_slot, "json_handler_result")
                .map_err(|error| error.to_string())?;
            self.builder
                .build_call(
                    self.module.get_function("thaw_promise_destroy").unwrap(),
                    &[promise.into()],
                    "destroy_json_handler",
                )
                .map_err(|error| error.to_string())?;
            result
        } else {
            call
        };

        self.branch_on_pending_exception()?;
        let stringify = self.module.get_function("thaw_json_stringify").unwrap();
        let text = self
            .builder
            .build_call(stringify, &[result.into()], "lambda_response_json")
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .ok_or("thaw_json_stringify did not return a value")?;
        let text = self.compile_check_json_stringify_error(text)?;
        self.builder
            .build_return(Some(&text))
            .map_err(|error| error.to_string())?;
        Ok(adapter)
    }

    /// Emits `int main(void) { thaw_runtime_run(&handler); return 0; }` for
    /// programs that define `handler` instead of `main` -- `thaw_runtime_run`
    /// (see thaw-runtime) never actually returns; it polls the Lambda
    /// Runtime API forever.
    fn emit_lambda_entry(&mut self, handler_hir: &HirFunction) -> Result<(), String> {
        let string_signature = handler_hir.params.len() == 1
            && handler_hir.params[0].ty == HirType::Str
            && handler_hir.ret == HirType::Str;
        let json_signature = handler_hir.params.len() == 1
            && handler_hir.params[0].ty == HirType::Json
            && handler_hir.ret == HirType::Json;
        if !string_signature && !json_signature {
            return Err("`handler` must have the signature `(event: string): string` or `(event: Json): Json`".to_string());
        }
        let handler_fn = self.module.get_function("handler").unwrap();
        let handler_fn = if json_signature {
            self.emit_json_handler_adapter(
                handler_fn,
                self.frame_async_functions.contains_key("handler"),
            )?
        } else {
            handler_fn
        };

        let i8_ptr = self.context.ptr_type(AddressSpace::default());
        // Only a QuickJS-backed Lambda needs to refresh the JavaScript
        // process.env snapshot. This direct call also retains the helper in
        // the linked archive without adding a runtime-to-QuickJS dependency.
        let handler_fn = if self.uses_quickjs || self.uses_quickjs_handles {
            let sync = self.module.add_function(
                "thaw_js_sync_lambda_trace_from_env",
                self.context.i8_type().fn_type(&[], false),
                Some(Linkage::External),
            );
            let wrapper = self.module.add_function(
                "__thaw_lambda_trace_handler",
                i8_ptr.fn_type(&[i8_ptr.into()], false),
                Some(Linkage::Internal),
            );
            let entry = self.context.append_basic_block(wrapper, "entry");
            let failed = self.context.append_basic_block(wrapper, "trace_sync_failed");
            let ready = self.context.append_basic_block(wrapper, "trace_sync_ready");
            self.builder.position_at_end(entry);
            let synced = self.builder.build_call(sync, &[], "sync_lambda_trace")
                .map_err(|error| error.to_string())?
                .try_as_basic_value().basic()
                .ok_or("Lambda trace sync returned no value")?
                .into_int_value();
            let ok = self.builder.build_int_compare(
                IntPredicate::NE, synced, self.context.i8_type().const_zero(), "lambda_trace_synced",
            ).map_err(|error| error.to_string())?;
            self.builder.build_conditional_branch(ok, ready, failed)
                .map_err(|error| error.to_string())?;
            self.builder.position_at_end(failed);
            let message = self.builder.build_global_string_ptr(
                "\u{1}ThawError\u{1}failed to synchronize Lambda trace environment",
                "lambda_trace_sync_error",
            ).map_err(|error| error.to_string())?;
            self.clear_pending_native_text()?;
            self.builder.build_store(self.pending_exception().as_pointer_value(), message.as_pointer_value())
                .map_err(|error| error.to_string())?;
            self.builder.build_return(Some(&i8_ptr.const_null()))
                .map_err(|error| error.to_string())?;
            self.builder.position_at_end(ready);
            let event = wrapper.get_first_param().unwrap();
            let result = self.builder.build_call(handler_fn, &[event.into()], "call_lambda_handler")
                .map_err(|error| error.to_string())?
                .try_as_basic_value().basic()
                .ok_or("Lambda handler returned no value")?;
            self.builder.build_return(Some(&result))
                .map_err(|error| error.to_string())?;
            wrapper
        } else {
            handler_fn
        };
        let run_type = self
            .context
            .void_type()
            .fn_type(&[i8_ptr.into(), i8_ptr.into()], false);
        let run_fn =
            self.module
                .add_function("thaw_runtime_run", run_type, Some(Linkage::External));

        let (main_fn, entry) = self.new_c_main();
        let cleanup = self.context.append_basic_block(main_fn, "entry_cleanup");
        self.builder.position_at_end(entry);
        let pointer = self.context.ptr_type(AddressSpace::default());
        let enable = self.module.add_function("thaw_arena_enable_tracing",
            self.context.void_type().fn_type(&[], false), Some(Linkage::External));
        self.builder.build_call(enable, &[], "enable_invocation_tracing").map_err(|error| error.to_string())?;
        let register = self.module.add_function(
            "thaw_arena_register_root",
            self.context.void_type().fn_type(&[pointer.into()], false),
            Some(Linkage::External),
        );
        for (slot, ty, _) in self.global_variables.values().cloned().collect::<Vec<_>>() {
            self.register_arena_root_slots(register, slot, ty)?;
        }
        for slot in &self.module_exception_roots {
            self.builder.build_call(register, &[(*slot).into()], "register_module_exception_root")
                .map_err(|error| error.to_string())?;
        }
        self.configure_unhandled_rejection_reporter();
        self.call_module_init_if_present(cleanup);
        let handler_ptr = handler_fn.as_global_value().as_pointer_value();
        self.builder
            .build_call(
                run_fn,
                &[
                    handler_ptr.into(),
                    self.pending_exception().as_pointer_value().into(),
                ],
                "call_thaw_runtime_run",
            )
            .map_err(|e| e.to_string())?;
        self.builder
            .build_unconditional_branch(cleanup)
            .map_err(|error| error.to_string())?;
        self.builder.position_at_end(cleanup);
        self.finish_c_main(None);
        Ok(())
    }

    fn register_arena_root_slots(
        &self,
        register: FunctionValue<'ctx>,
        slot: PointerValue<'ctx>,
        ty: BasicTypeEnum<'ctx>,
    ) -> Result<(), String> {
        match ty {
            BasicTypeEnum::PointerType(_) => {
                self.builder.build_call(register, &[slot.into()], "register_global_root")
                    .map_err(|error| error.to_string())?;
            }
            BasicTypeEnum::StructType(structure) => {
                for (index, field) in structure.get_field_types().into_iter().enumerate() {
                    let field_slot = self.builder.build_struct_gep(structure, slot, index as u32, "global_root_field")
                        .map_err(|error| error.to_string())?;
                    // Union payloads are represented as initialized i64 bits.
                    if field == self.context.i64_type().into() {
                        self.builder.build_call(register, &[field_slot.into()], "register_global_union_root")
                            .map_err(|error| error.to_string())?;
                    } else {
                        self.register_arena_root_slots(register, field_slot, field)?;
                    }
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn new_c_main(&mut self) -> (FunctionValue<'ctx>, BasicBlock<'ctx>) {
        let i32_type = self.context.i32_type();
        let main_type = i32_type.fn_type(&[], false);
        let main_fn = self.module.add_function("main", main_type, None);
        let entry = self.context.append_basic_block(main_fn, "entry");
        (main_fn, entry)
    }

    fn configure_unhandled_rejection_reporter(&self) {
        let reporter = if self.uses_quickjs_handles {
            self.module
                .get_function("thaw_js_emit_unhandled_rejection_result")
                .unwrap()
                .as_global_value()
                .as_pointer_value()
        } else {
            self.context.ptr_type(AddressSpace::default()).const_null()
        };
        self.builder
            .build_call(
                self.module
                    .get_function("thaw_promise_set_unhandled_reporter_text_result")
                    .unwrap(),
                &[reporter.into()],
                "configure_unhandled_rejection_reporter",
            )
            .unwrap();
        let handled_reporter = if self.uses_quickjs_handles {
            self.module
                .get_function("thaw_js_emit_rejection_handled_result")
                .unwrap()
                .as_global_value()
                .as_pointer_value()
        } else {
            self.context.ptr_type(AddressSpace::default()).const_null()
        };
        self.builder
            .build_call(
                self.module
                    .get_function("thaw_promise_set_rejection_handled_reporter_result")
                    .unwrap(),
                &[handled_reporter.into()],
                "configure_rejection_handled_reporter",
            )
            .unwrap();
    }

    // Consume the complete exception tuple before entering a process listener.
    // A listener may call compiled code that sets a new tuple, so leaving the
    // original object/tag/payload here would attach stale type information to
    // that new exception. The already-loaded text remains live through the
    // synchronous notification; its producer owns the underlying allocation.
    fn clear_terminal_exception(&self) {
        let ptr_ty = self.context.ptr_type(AddressSpace::default());
        for slot in [
            self.pending_exception(),
            self.pending_exception_native_text(),
            self.pending_exception_object(),
            self.pending_exception_aggregate_errors(),
        ] {
            self.builder.build_store(slot.as_pointer_value(), ptr_ty.const_null()).unwrap();
        }
        for (symbol, ty) in [
            (PENDING_EXCEPTION_VALUE_TAG_SYMBOL, BasicTypeEnum::from(self.context.i64_type())),
            (PENDING_EXCEPTION_F64_SYMBOL, BasicTypeEnum::from(self.context.f64_type())),
            (PENDING_EXCEPTION_I64_SYMBOL, BasicTypeEnum::from(self.context.i64_type())),
            (PENDING_EXCEPTION_BOOL_SYMBOL, BasicTypeEnum::from(self.context.bool_type())),
        ] {
            self.builder.build_store(self.pending_exception_value(symbol).as_pointer_value(), ty.const_zero()).unwrap();
        }
    }

    // Snapshot a report-safe owned string before listener reentry. The native
    // helper reads the pointer only when exact producer provenance matches;
    // otherwise it uses typed scalar channels or an opaque diagnostic.
    fn terminal_report_text(
        &self, value: PointerValue<'ctx>, native_text: PointerValue<'ctx>, name: &str,
    ) -> PointerValue<'ctx> {
        let mut args: Vec<BasicMetadataValueEnum<'ctx>> = vec![value.into(), native_text.into()];
        for (symbol, ty) in [
            (PENDING_EXCEPTION_VALUE_TAG_SYMBOL, BasicTypeEnum::from(self.context.i64_type())),
            (PENDING_EXCEPTION_F64_SYMBOL, BasicTypeEnum::from(self.context.f64_type())),
            (PENDING_EXCEPTION_I64_SYMBOL, BasicTypeEnum::from(self.context.i64_type())),
            (PENDING_EXCEPTION_BOOL_SYMBOL, BasicTypeEnum::from(self.context.bool_type())),
        ] {
            let field = self.builder.build_load(
                ty, self.pending_exception_value(symbol).as_pointer_value(),
                &format!("{name}_typed_field"),
            ).unwrap();
            args.push(field.into());
        }
        self.builder.build_call(
            self.module.get_function("thaw_runtime_exception_report_text").unwrap(),
            &args, &format!("{name}_owned_report_text"),
        ).unwrap().try_as_basic_value().basic().unwrap().into_pointer_value()
    }

    // A checkpoint consumes its direct exception slots and returns only a
    // failure flag. Listener and cleanup work may call it again before unload.
    fn report_c_main(&mut self) -> IntValue<'ctx> {
        let pending = self
            .builder
            .build_load(
                self.context.ptr_type(AddressSpace::default()),
                self.pending_exception().as_pointer_value(),
                "process_pending_exception",
            )
            .unwrap()
            .into_pointer_value();
        let original_native_text = self.builder.build_load(
            pending.get_type(), self.pending_exception_native_text().as_pointer_value(),
            "original_exception_native_text",
        ).unwrap().into_pointer_value();
        let pending_report = self.terminal_report_text(pending, original_native_text, "original_exception");
        self.clear_terminal_exception();
        let has_exception = self
            .builder
            .build_is_not_null(pending, "process_exception_failed")
            .unwrap();
        let (exception_failure, reported, native_handler_error, listener_error) = if self.uses_quickjs_handles {
            let emission = self.builder.build_call(
                self.module.get_function("thaw_js_emit_uncaught_result").unwrap(),
                &[pending_report.into()],
                "emit_process_uncaught_exception",
            ).unwrap().try_as_basic_value().basic().unwrap().into_struct_value();
            let handled = self.builder.build_extract_value(emission, 0, "process_exception_handled_value")
                .unwrap().into_int_value();
            let listener_error = self.builder.build_extract_value(emission, 1, "process_exception_listener_error")
                .unwrap().into_pointer_value();
            let listener_failed = self.builder.build_is_not_null(listener_error, "process_listener_failed").unwrap();
            let handled = self
                .builder
                .build_int_compare(
                    IntPredicate::NE,
                    handled,
                    handled.get_type().const_zero(),
                    "process_exception_handled",
                )
                .unwrap();
            let handler_exception = self
                .builder
                .build_load(
                    self.context.ptr_type(AddressSpace::default()),
                    self.pending_exception().as_pointer_value(),
                    "process_handler_exception",
                )
                .unwrap()
                .into_pointer_value();
            let handler_native_text = self.builder.build_load(
                pending.get_type(), self.pending_exception_native_text().as_pointer_value(),
                "listener_exception_native_text",
            ).unwrap().into_pointer_value();
            let handler_report = self.terminal_report_text(
                handler_exception, handler_native_text, "listener_exception",
            );
            self.clear_terminal_exception();
            let handler_failed = self
                .builder
                .build_is_not_null(handler_exception, "process_handler_failed")
                .unwrap();
            let original_unhandled = self
                .builder
                .build_and(
                    has_exception,
                    self.builder.build_not(handled, "process_exception_unhandled").unwrap(),
                    "process_original_exception_unhandled",
                )
                .unwrap();
            let failure = self
                .builder
                .build_or(
                    original_unhandled,
                    handler_failed,
                    "process_exception_failed_unhandled",
                )
                .and_then(|failed| self.builder.build_or(failed, listener_failed, "process_listener_exception_failed"))
                .unwrap();
            let original_report = self
                .builder
                .build_select(
                    original_unhandled,
                    pending_report,
                    pending.get_type().const_null(),
                    "original_uncaught_exception",
                )
                .unwrap()
                .into_pointer_value();
            (failure, original_report, handler_report, listener_error)
        } else {
            let null = pending.get_type().const_null();
            (has_exception, pending_report, null, null)
        };
        self.builder
            .build_call(
                self.module
                    .get_function("thaw_runtime_report_uncaught")
                    .unwrap(),
                &[reported.into()],
                "report_uncaught_exception",
            )
            .unwrap();
        self.builder.build_call(
            self.module.get_function("thaw_runtime_report_uncaught").unwrap(),
            &[native_handler_error.into()],
            "report_native_handler_exception",
        ).unwrap();
        self.builder.build_call(
            self.module.get_function("thaw_runtime_report_uncaught").unwrap(),
            &[listener_error.into()],
            "report_js_listener_exception",
        ).unwrap();
        self.builder.build_call(
            self.module.get_function("thaw_cstring_destroy").unwrap(),
            &[listener_error.into()],
            "destroy_js_listener_exception",
        ).unwrap();
        self.builder.build_call(
            self.module.get_function("thaw_cstring_destroy").unwrap(),
            &[pending_report.into()], "destroy_original_exception_report",
        ).unwrap();
        self.builder.build_call(
            self.module.get_function("thaw_cstring_destroy").unwrap(),
            &[native_handler_error.into()], "destroy_native_handler_exception_report",
        ).unwrap();
        let rejection = self
            .builder
            .build_load(
                self.context.ptr_type(AddressSpace::default()),
                self.pending_rejection().as_pointer_value(),
                "process_pending_rejection",
            )
            .unwrap()
            .into_pointer_value();
        self.builder.build_store(
            self.pending_rejection().as_pointer_value(), rejection.get_type().const_null(),
        ).unwrap();
        let has_rejection = self
            .builder
            .build_is_not_null(rejection, "process_rejection_failed")
            .unwrap();
        let (rejection_failure, reported_rejection, rejection_listener_error) = if self.uses_quickjs_handles {
            let emission = self.builder.build_call(
                self.module.get_function("thaw_js_emit_unhandled_rejection_result").unwrap(),
                &[rejection.into()],
                "emit_process_unhandled_rejection",
            ).unwrap().try_as_basic_value().basic().unwrap().into_struct_value();
            let handled = self.builder.build_extract_value(emission, 0, "process_rejection_handled_value")
                .unwrap().into_int_value();
            let listener_error = self.builder.build_extract_value(emission, 1, "process_rejection_listener_error")
                .unwrap().into_pointer_value();
            let listener_failed = self.builder.build_is_not_null(listener_error, "process_rejection_listener_failed").unwrap();
            let handled = self
                .builder
                .build_int_compare(
                    IntPredicate::NE,
                    handled,
                    handled.get_type().const_zero(),
                    "process_rejection_handled",
                )
                .unwrap();
            let unhandled = self
                .builder
                .build_not(handled, "process_rejection_unhandled")
                .unwrap();
            let failure = self.builder.build_and(
                has_rejection, unhandled, "process_rejection_failed_unhandled",
            ).and_then(|failed| self.builder.build_or(failed, listener_failed, "process_rejection_handler_failed"))
                .unwrap();
            let reported = self
                .builder
                .build_select(
                    handled,
                    rejection.get_type().const_null(),
                    rejection,
                    "reported_unhandled_rejection",
                )
                .unwrap()
                .into_pointer_value();
            (failure, reported, listener_error)
        } else {
            (has_rejection, rejection, rejection.get_type().const_null())
        };
        self.builder
            .build_call(
                self.module
                    .get_function("thaw_runtime_report_uncaught")
                    .unwrap(),
                &[reported_rejection.into()],
                "report_unhandled_rejection",
            )
            .unwrap();
        self.builder.build_call(
            self.module.get_function("thaw_runtime_report_uncaught").unwrap(),
            &[rejection_listener_error.into()],
            "report_rejection_listener_exception",
        ).unwrap();
        self.builder.build_call(
            self.module.get_function("thaw_cstring_destroy").unwrap(),
            &[rejection_listener_error.into()],
            "destroy_rejection_listener_exception",
        ).unwrap();
        self.builder.build_call(
            self.module.get_function("thaw_cstring_destroy").unwrap(),
            &[rejection.into()],
            "destroy_detached_rejection_report",
        ).unwrap();
        let rejection_reporter = if self.uses_quickjs_handles {
            self.module
                .get_function("thaw_js_emit_unhandled_rejection_result")
                .unwrap()
                .as_global_value()
                .as_pointer_value()
        } else {
            self.context.ptr_type(AddressSpace::default()).const_null()
        };
        let stored_rejection_failure = self
            .builder
            .build_call(
                self.module
                    .get_function("thaw_promise_drain_unhandled_text_result")
                    .unwrap(),
                &[rejection_reporter.into()],
                "drain_stored_unhandled_rejections",
            )
            .unwrap()
            .try_as_basic_value()
            .basic()
            .unwrap()
            .into_int_value();
        let stored_rejection_failure = self
            .builder
            .build_int_compare(
                IntPredicate::NE,
                stored_rejection_failure,
                stored_rejection_failure.get_type().const_zero(),
                "stored_rejection_failed",
            )
            .unwrap();
        let checkpoint_rejection_failure = self
            .builder
            .build_call(
                self.module
                    .get_function("thaw_promise_take_unhandled_failure")
                    .unwrap(),
                &[],
                "take_checkpoint_rejection_failure",
            )
            .unwrap()
            .try_as_basic_value()
            .basic()
            .unwrap()
            .into_int_value();
        let checkpoint_rejection_failure = self
            .builder
            .build_int_compare(
                IntPredicate::NE,
                checkpoint_rejection_failure,
                checkpoint_rejection_failure.get_type().const_zero(),
                "checkpoint_rejection_failed",
            )
            .unwrap();
        let http_failure = if self.module.get_function("createServer").is_some() {
            let status = self
                .builder
                .build_call(
                    self.module
                        .get_function("thaw_http_take_unhandled_error")
                        .unwrap(),
                    &[],
                    "take_http_unhandled_error",
                )
                .unwrap()
                .try_as_basic_value()
                .basic()
                .unwrap()
                .into_int_value();
            self.builder
                .build_int_compare(
                    IntPredicate::NE,
                    status,
                    status.get_type().const_zero(),
                    "http_failed",
                )
                .unwrap()
        } else {
            self.context.bool_type().const_zero()
        };
        let process_failure = self
            .builder
            .build_or(exception_failure, rejection_failure, "direct_script_failed")
            .and_then(|direct_failure| {
                self.builder.build_or(
                    direct_failure,
                    stored_rejection_failure,
                    "exit_script_failed",
                )
            })
            .and_then(|exit_failure| {
                self.builder.build_or(
                    exit_failure,
                    checkpoint_rejection_failure,
                    "script_failed",
                )
            })
            .and_then(|script_failure| {
                self.builder
                    .build_or(script_failure, http_failure, "process_failed")
            })
            .unwrap();
        let process_failure = if self.uses_napi {
            let fatal_status = self
                .builder
                .build_call(
                    self.module
                        .get_function("thaw_napi_take_fatal_exception")
                        .unwrap(),
                    &[],
                    "take_napi_fatal_exception",
                )
                .unwrap()
                .try_as_basic_value()
                .basic()
                .unwrap()
                .into_int_value();
            let fatal = self
                .builder
                .build_int_compare(
                    IntPredicate::NE,
                    fatal_status,
                    fatal_status.get_type().const_zero(),
                    "napi_fatal",
                )
                .unwrap();
            self
                .builder
                .build_or(fatal, process_failure, "napi_process_failed")
                .unwrap()
        } else {
            process_failure
        };
        process_failure
    }

    fn latch_terminal_failure(&self, slot: PointerValue<'ctx>, failed: IntValue<'ctx>) {
        let previous = self.builder.build_load(self.context.bool_type(), slot, "previous_terminal_failure")
            .unwrap().into_int_value();
        let combined = self.builder.build_or(previous, failed, "terminal_failure_latched").unwrap();
        self.builder.build_store(slot, combined).unwrap();
    }

    // One pass through every callback-capable queue. A zero native drain alone
    // says nothing about QuickJS jobs or N-API completions, so check all three.
    fn drive_terminal_work(
        &mut self,
        failure: PointerValue<'ctx>,
        quickjs_failure: Option<PointerValue<'ctx>>,
        before_shutdown: bool,
    ) -> IntValue<'ctx> {
        let reported_failure = self.report_c_main();
        self.latch_terminal_failure(failure, reported_failure);
        let native_before = self.builder.build_call(
            self.module.get_function("thaw_runtime_run_until_idle").unwrap(), &[],
            "terminal_native_before_js",
        ).unwrap().try_as_basic_value().basic().unwrap().into_int_value();
        let mut progressed = self.builder.build_int_compare(
            IntPredicate::NE, native_before, native_before.get_type().const_zero(),
            "terminal_native_before_progress",
        ).unwrap();
        if self.uses_quickjs || self.uses_quickjs_handles {
            let status = self.builder.build_call(
                self.module.get_function("thaw_js_run_event_loop").unwrap(), &[],
                "terminal_quickjs_jobs",
            ).unwrap().try_as_basic_value().basic().unwrap().into_int_value();
            let failed = self.builder.build_int_compare(
                IntPredicate::NE, status, status.get_type().const_zero(), "terminal_quickjs_failed",
            ).unwrap();
            if let Some(slot) = quickjs_failure {
                let previous = self.builder.build_load(self.context.i32_type(), slot, "terminal_previous_quickjs_status")
                    .unwrap().into_int_value();
                let combined = self.builder.build_select(failed, status, previous, "terminal_quickjs_status")
                    .unwrap();
                self.builder.build_store(slot, combined).unwrap();
            } else {
                self.latch_terminal_failure(failure, failed);
            }
            let pending = self.builder.build_call(
                self.module.get_function("thaw_js_terminal_work_pending").unwrap(), &[],
                "terminal_quickjs_pending_query",
            ).unwrap().try_as_basic_value().basic().unwrap().into_int_value();
            let pending = self.builder.build_int_compare(
                IntPredicate::NE, pending, pending.get_type().const_zero(),
                "terminal_quickjs_work_pending",
            ).unwrap();
            progressed = self.builder.build_or(progressed, pending, "terminal_quickjs_more_work").unwrap();
        }
        if self.uses_napi && before_shutdown {
            let count = self.builder.build_call(
                self.module.get_function("thaw_napi_poll_async_work").unwrap(), &[],
                "terminal_napi_callbacks",
            ).unwrap().try_as_basic_value().basic().unwrap().into_int_value();
            let ran = self.builder.build_int_compare(
                IntPredicate::NE, count, count.get_type().const_zero(), "terminal_napi_progress",
            ).unwrap();
            progressed = self.builder.build_or(progressed, ran, "terminal_host_progress").unwrap();
            let pending = self.builder.build_call(
                self.module.get_function("thaw_napi_async_work_pending").unwrap(), &[],
                "terminal_napi_pending",
            ).unwrap().try_as_basic_value().basic().unwrap().into_int_value();
            let pending = self.builder.build_int_compare(
                IntPredicate::NE, pending, pending.get_type().const_zero(), "terminal_napi_pending_work",
            ).unwrap();
            progressed = self.builder.build_or(progressed, pending, "terminal_host_pending").unwrap();
        }
        let native_after = self.builder.build_call(
            self.module.get_function("thaw_runtime_run_until_idle").unwrap(), &[],
            "terminal_native_after_js",
        ).unwrap().try_as_basic_value().basic().unwrap().into_int_value();
        let ran = self.builder.build_int_compare(
            IntPredicate::NE, native_after, native_after.get_type().const_zero(),
            "terminal_native_after_progress",
        ).unwrap();
        progressed = self.builder.build_or(progressed, ran, "terminal_native_progress").unwrap();
        let reports = self.builder.build_call(
            self.module.get_function("thaw_promise_take_report_activity").unwrap(), &[],
            "terminal_promise_notifications",
        ).unwrap().try_as_basic_value().basic().unwrap().into_int_value();
        let reported = self.builder.build_int_compare(
            IntPredicate::NE, reports, reports.get_type().const_zero(),
            "terminal_promise_reported",
        ).unwrap();
        progressed = self.builder.build_or(progressed, reported, "terminal_reporting_progress").unwrap();
        for (slot, name) in [
            (self.pending_exception().as_pointer_value(), "terminal_new_exception"),
            (self.pending_rejection().as_pointer_value(), "terminal_new_rejection"),
        ] {
            let pending = self.builder.build_load(
                self.context.ptr_type(AddressSpace::default()), slot, name,
            ).unwrap().into_pointer_value();
            let pending = self.builder.build_is_not_null(pending, "terminal_pending_notification").unwrap();
            progressed = self.builder.build_or(progressed, pending, "terminal_more_notifications").unwrap();
        }
        let pending_native = self.builder.build_call(
            self.module.get_function("thaw_runtime_async_work_pending").unwrap(), &[],
            "terminal_pending_native_async",
        ).unwrap().try_as_basic_value().basic().unwrap().into_int_value();
        let pending_native = self.builder.build_int_compare(
            IntPredicate::NE, pending_native, pending_native.get_type().const_zero(),
            "terminal_native_async_pending",
        ).unwrap();
        progressed = self.builder.build_or(progressed, pending_native, "terminal_native_async_progress").unwrap();
        progressed
    }

    // Poll/cleanup errors are owned strings. Report and free each while the
    // addon and its Env are still mapped; listener errors also latch failure.
    fn report_napi_shutdown_errors(
        &mut self, failure: PointerValue<'ctx>, seen: PointerValue<'ctx>,
    ) -> IntValue<'ctx> {
        let function = self.builder.get_insert_block().unwrap().get_parent().unwrap();
        let check = self.context.append_basic_block(function, "shutdown_error_check");
        let report = self.context.append_basic_block(function, "shutdown_error_report");
        let done = self.context.append_basic_block(function, "shutdown_errors_drained");
        self.builder.build_store(seen, self.context.bool_type().const_zero()).unwrap();
        self.builder.build_unconditional_branch(check).unwrap();
        self.builder.position_at_end(check);
        let error = self.builder.build_call(
            self.module.get_function("thaw_napi_take_shutdown_error").unwrap(), &[],
            "take_shutdown_error",
        ).unwrap().try_as_basic_value().basic().unwrap().into_pointer_value();
        let has_error = self.builder.build_is_not_null(error, "has_shutdown_error").unwrap();
        self.builder.build_conditional_branch(has_error, report, done).unwrap();
        self.builder.position_at_end(report);
        self.builder.build_store(seen, self.context.bool_type().const_int(1, false)).unwrap();
        if self.uses_quickjs_handles {
            let result = self.builder.build_call(
                self.module.get_function("thaw_js_emit_uncaught_result").unwrap(),
                &[error.into()], "emit_shutdown_exception",
            ).unwrap().try_as_basic_value().basic().unwrap().into_struct_value();
            let handled = self.builder.build_extract_value(result, 0, "shutdown_exception_handled")
                .unwrap().into_int_value();
            let listener_error = self.builder.build_extract_value(result, 1, "shutdown_listener_error")
                .unwrap().into_pointer_value();
            let unhandled = self.builder.build_int_compare(
                IntPredicate::EQ, handled, handled.get_type().const_zero(), "shutdown_exception_unhandled",
            ).unwrap();
            let listener_failed = self.builder.build_is_not_null(listener_error, "shutdown_listener_failed").unwrap();
            let failed = self.builder.build_or(unhandled, listener_failed, "shutdown_exception_failed").unwrap();
            self.latch_terminal_failure(failure, failed);
            let report_error = self.builder.build_select(
                unhandled, error, error.get_type().const_null(), "unhandled_shutdown_exception",
            ).unwrap().into_pointer_value();
            self.builder.build_call(
                self.module.get_function("thaw_runtime_report_uncaught").unwrap(),
                &[report_error.into()], "report_shutdown_exception",
            ).unwrap();
            self.builder.build_call(
                self.module.get_function("thaw_runtime_report_uncaught").unwrap(),
                &[listener_error.into()], "report_shutdown_listener_error",
            ).unwrap();
            self.builder.build_call(
                self.module.get_function("thaw_cstring_destroy").unwrap(),
                &[listener_error.into()], "destroy_shutdown_listener_error",
            ).unwrap();
        } else {
            self.latch_terminal_failure(failure, self.context.bool_type().const_int(1, false));
            self.builder.build_call(
                self.module.get_function("thaw_runtime_report_uncaught").unwrap(),
                &[error.into()], "report_shutdown_exception",
            ).unwrap();
        }
        self.builder.build_call(
            self.module.get_function("thaw_cstring_destroy").unwrap(),
            &[error.into()], "destroy_shutdown_error",
        ).unwrap();
        self.builder.build_unconditional_branch(check).unwrap();
        self.builder.position_at_end(done);
        self.builder.build_load(self.context.bool_type(), seen, "reported_shutdown_error")
            .unwrap().into_int_value()
    }

    fn release_terminal_handles(&self) -> IntValue<'ctx> {
        if !self.uses_quickjs_handles {
            return self.context.bool_type().const_zero();
        }
        let count = self.builder.build_call(
            self.module.get_function("thaw_js_release_all_handles").unwrap(), &[],
            "release_terminal_handles",
        ).unwrap().try_as_basic_value().basic().unwrap().into_int_value();
        self.builder.build_int_compare(
            IntPredicate::NE, count, count.get_type().const_zero(), "released_terminal_handles",
        ).unwrap()
    }

    fn finish_c_main(&mut self, quickjs_failure: Option<PointerValue<'ctx>>) {
        let i32_type = self.context.i32_type();
        let failure = self.builder.build_alloca(self.context.bool_type(), "terminal_failure").unwrap();
        let shutdown_error_seen = self.builder.build_alloca(self.context.bool_type(), "shutdown_error_seen").unwrap();
        self.builder.build_store(failure, self.context.bool_type().const_zero()).unwrap();
        let function = self.builder.get_insert_block().unwrap().get_parent().unwrap();
        let ordinary = self.context.append_basic_block(function, "terminal_ordinary_notifications");
        let begin = self.context.append_basic_block(function, "terminal_begin_shutdown");
        let wait = self.context.append_basic_block(function, "terminal_wait_for_work");
        let finished = self.context.append_basic_block(function, "terminal_finished");
        self.builder.build_unconditional_branch(ordinary).unwrap();
        self.builder.position_at_end(ordinary);
        let progressed = self.drive_terminal_work(failure, quickjs_failure, true);
        if self.uses_napi {
            let errors = self.report_napi_shutdown_errors(failure, shutdown_error_seen);
            let after_errors = self.context.append_basic_block(function, "terminal_after_ordinary_errors");
            self.builder.build_conditional_branch(errors, ordinary, after_errors).unwrap();
            self.builder.position_at_end(after_errors);
        }
        self.builder.build_conditional_branch(progressed, wait, begin).unwrap();
        self.builder.position_at_end(wait);
        self.builder.build_call(
            self.module.get_function("thaw_runtime_shutdown_wait").unwrap(), &[],
            "wait_for_terminal_work",
        ).unwrap();
        self.builder.build_unconditional_branch(ordinary).unwrap();
        self.builder.position_at_end(begin);
        if self.uses_napi {
            let poll = self.context.append_basic_block(function, "terminal_shutdown_poll");
            let release = self.context.append_basic_block(function, "terminal_shutdown_release_handles");
            let close = self.context.append_basic_block(function, "terminal_shutdown_close");
            let started = self.builder.build_call(
                self.module.get_function("thaw_napi_begin_shutdown").unwrap(), &[],
                "begin_napi_shutdown",
            ).unwrap().try_as_basic_value().basic().unwrap().into_int_value();
            let started = self.builder.build_int_compare(
                IntPredicate::NE, started, started.get_type().const_zero(), "napi_shutdown_started",
            ).unwrap();
            self.builder.build_conditional_branch(started, poll, wait).unwrap();
            self.builder.position_at_end(poll);
            // A live unreferenced handle can keep the phase barrier closed.
            // Retry with its Env and library retained until its owner closes it.
            self.builder.build_call(
                self.module.get_function("thaw_runtime_shutdown_wait").unwrap(), &[],
                "wait_for_napi_shutdown",
            ).unwrap();
            self.builder.build_call(
                self.module.get_function("thaw_napi_poll_shutdown").unwrap(), &[],
                "poll_napi_shutdown",
            ).unwrap();
            self.report_napi_shutdown_errors(failure, shutdown_error_seen);
            self.drive_terminal_work(failure, quickjs_failure, false);
            let ready = self.builder.build_call(
                self.module.get_function("thaw_napi_poll_shutdown").unwrap(), &[],
                "confirm_napi_shutdown",
            ).unwrap().try_as_basic_value().basic().unwrap().into_int_value();
            let ready = self.builder.build_int_compare(
                IntPredicate::NE, ready, ready.get_type().const_zero(), "napi_shutdown_ready",
            ).unwrap();
            let new_errors = self.report_napi_shutdown_errors(failure, shutdown_error_seen);
            let new_work = self.drive_terminal_work(failure, quickjs_failure, false);
            let unsettled = self.builder.build_or(new_errors, new_work, "shutdown_more_work").unwrap();
            let quiescent = self.builder.build_not(unsettled, "shutdown_quiescent").unwrap();
            let may_close = self.builder.build_and(ready, quiescent, "shutdown_can_close").unwrap();
            self.builder.build_conditional_branch(may_close, release, poll).unwrap();
            self.builder.position_at_end(release);
            let released_handles = self.release_terminal_handles();
            self.builder.build_conditional_branch(released_handles, poll, close).unwrap();
            self.builder.position_at_end(close);
            let released = self.builder.build_call(
                self.module.get_function("thaw_napi_finish_shutdown").unwrap(), &[],
                "finish_napi_shutdown",
            ).unwrap().try_as_basic_value().basic().unwrap().into_int_value();
            let released = self.builder.build_int_compare(
                IntPredicate::NE, released, released.get_type().const_zero(), "napi_shutdown_finished",
            ).unwrap();
            self.builder.build_conditional_branch(released, finished, poll).unwrap();
        } else {
            let released_handles = self.release_terminal_handles();
            self.builder.build_conditional_branch(released_handles, wait, finished).unwrap();
        }
        self.builder.position_at_end(finished);
        let process_failure = self.builder.build_load(
            self.context.bool_type(), failure, "terminal_failure_status",
        ).unwrap().into_int_value();
        let quickjs_status = quickjs_failure.map_or_else(
            || i32_type.const_zero(),
            |failure| {
                self
                    .builder
                    .build_load(i32_type, failure, "quickjs_event_loop_status")
                    .unwrap()
                    .into_int_value()
            },
        );
        let failure_status = self
            .builder
            .build_int_z_extend(process_failure, i32_type, "failure_exit_status")
            .unwrap();
        let status = self
            .builder
            .build_select(
                process_failure,
                failure_status,
                quickjs_status,
                "process_exit_status",
            )
            .unwrap();
        self.builder.build_return(Some(&status)).unwrap();
    }


}
