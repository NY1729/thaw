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
        self.call_module_init_if_present(cleanup);
        self.configure_unhandled_rejection_reporter();
        let completion = user_main.and_then(|user_main| {
            self.builder
                .build_call(user_main, &[], "call_thaw_user_main")
                .unwrap()
                .try_as_basic_value()
                .basic()
        });
        if let Some(completion) = completion {
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
            self.builder
                .build_call(
                    self.module.get_function("thaw_napi_unload_all").unwrap(),
                    &[],
                    "unload_napi_addons",
                )
                .unwrap();
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
        self.call_module_init_if_present(cleanup);
        self.configure_unhandled_rejection_reporter();
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
                .get_function("thaw_js_emit_unhandled_rejection")
                .unwrap()
                .as_global_value()
                .as_pointer_value()
        } else {
            self.context.ptr_type(AddressSpace::default()).const_null()
        };
        self.builder
            .build_call(
                self.module
                    .get_function("thaw_promise_set_unhandled_reporter")
                    .unwrap(),
                &[reporter.into()],
                "configure_unhandled_rejection_reporter",
            )
            .unwrap();
        let handled_reporter = if self.uses_quickjs_handles {
            self.module
                .get_function("thaw_js_emit_rejection_handled")
                .unwrap()
                .as_global_value()
                .as_pointer_value()
        } else {
            self.context.ptr_type(AddressSpace::default()).const_null()
        };
        self.builder
            .build_call(
                self.module
                    .get_function("thaw_promise_set_rejection_handled_reporter")
                    .unwrap(),
                &[handled_reporter.into()],
                "configure_rejection_handled_reporter",
            )
            .unwrap();
    }

    fn finish_c_main(&mut self, quickjs_failure: Option<PointerValue<'ctx>>) {
        let i32_type = self.context.i32_type();
        let pending = self
            .builder
            .build_load(
                self.context.ptr_type(AddressSpace::default()),
                self.pending_exception().as_pointer_value(),
                "process_pending_exception",
            )
            .unwrap()
            .into_pointer_value();
        let has_exception = self
            .builder
            .build_is_not_null(pending, "process_exception_failed")
            .unwrap();
        let (exception_failure, reported) = if self.uses_quickjs_handles {
            self.builder
                .build_store(
                    self.pending_exception().as_pointer_value(),
                    pending.get_type().const_null(),
                )
                .unwrap();
            let handled = self
                .builder
                .build_call(
                    self.module.get_function("thaw_js_emit_uncaught").unwrap(),
                    &[pending.into()],
                    "emit_process_uncaught_exception",
                )
                .unwrap()
                .try_as_basic_value()
                .basic()
                .unwrap()
                .into_int_value();
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
                .unwrap();
            let original_report = self
                .builder
                .build_select(
                    original_unhandled,
                    pending,
                    pending.get_type().const_null(),
                    "original_uncaught_exception",
                )
                .unwrap()
                .into_pointer_value();
            let reported = self
                .builder
                .build_select(
                    handler_failed,
                    handler_exception,
                    original_report,
                    "reported_uncaught_exception",
                )
                .unwrap()
                .into_pointer_value();
            (failure, reported)
        } else {
            (has_exception, pending)
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
        let rejection = self
            .builder
            .build_load(
                self.context.ptr_type(AddressSpace::default()),
                self.pending_rejection().as_pointer_value(),
                "process_pending_rejection",
            )
            .unwrap()
            .into_pointer_value();
        let has_rejection = self
            .builder
            .build_is_not_null(rejection, "process_rejection_failed")
            .unwrap();
        let (rejection_failure, reported_rejection) = if self.uses_quickjs_handles {
            let handled = self
                .builder
                .build_call(
                    self.module
                        .get_function("thaw_js_emit_unhandled_rejection")
                        .unwrap(),
                    &[rejection.into()],
                    "emit_process_unhandled_rejection",
                )
                .unwrap()
                .try_as_basic_value()
                .basic()
                .unwrap()
                .into_int_value();
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
            let failure = self
                .builder
                .build_and(
                    has_rejection,
                    unhandled,
                    "process_rejection_failed_unhandled",
                )
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
            (failure, reported)
        } else {
            (has_rejection, rejection)
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
        let rejection_reporter = if self.uses_quickjs_handles {
            self.module
                .get_function("thaw_js_emit_unhandled_rejection")
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
                    .get_function("thaw_promise_drain_unhandled")
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
        if self.uses_quickjs_handles {
            self.builder
                .build_call(
                    self.module
                        .get_function("thaw_js_release_all_handles")
                        .unwrap(),
                    &[],
                    "release_javascript_handles",
                )
                .unwrap();
        }
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
