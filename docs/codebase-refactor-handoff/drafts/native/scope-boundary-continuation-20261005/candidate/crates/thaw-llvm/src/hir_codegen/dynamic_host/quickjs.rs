impl<'ctx> HirCompiler<'ctx> {
    fn compile_typed_quickjs_setter(
        &mut self,
        signature: &DynamicSignature,
        args: &[HirExpr],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let [receiver, assigned] = args else {
            return Err("typed QuickJS setter expects a receiver and value".into());
        };
        let receiver = self.compile_expr(receiver)?;
        let assigned_type = signature
            .params
            .get(1)
            .ok_or("typed QuickJS setter is missing its value type")?;
        let array = self
            .builder
            .build_call(
                self.module.get_function("thaw_json_array_new").unwrap(),
                &[],
                "quickjs_setter_args",
            )
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .unwrap();
        let assigned = self.compile_expr(assigned)?;
        self.compile_typed_dynamic_argument(array, assigned, assigned_type)?;
        let property = dynamic_member_name(&signature.symbol, false)
            .ok_or("invalid typed QuickJS setter symbol")?;
        let property = self
            .builder
            .build_global_string_ptr(&property, "quickjs_setter_name")
            .map_err(|error| error.to_string())?;
        let args_json = self
            .builder
            .build_call(
                self.module.get_function("thaw_json_graph_encode").unwrap(),
                &[array.into()],
                "quickjs_setter_args_json",
            )
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .unwrap();
        let args_json = self.compile_check_json_stringify_error_with_cleanup(args_json, &[array])?;
        let result = self
            .builder
            .build_call(
                self.module
                    .get_function("thaw_js_set_property_json_graph_result")
                    .unwrap(),
                &[
                    receiver.into(),
                    property.as_pointer_value().into(),
                    args_json.into(),
                ],
                "quickjs_setter_result",
            )
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .unwrap()
            .into_struct_value();
        let value = self
            .builder
            .build_extract_value(result, 0, "quickjs_setter_json")
            .map_err(|error| error.to_string())?;
        let error = self
            .builder
            .build_extract_value(result, 1, "quickjs_setter_error")
            .map_err(|error| error.to_string())?;
        self.destroy_typed_host_arguments(array, args_json)?;
        self.builder
            .build_store(self.pending_exception().as_pointer_value(), error)
            .map_err(|error| error.to_string())?;
        self.clear_pending_native_text()?;
        self.mark_pending_native_text(error)?;
        self.branch_on_pending_exception()?;
        let json = self.compile_decode_quickjs_graph(value.into())?;

        self.compile_typed_dynamic_result(json, &signature.ret)
    }

    /// `loadScript(source): boolean`, via thaw-quickjs's `thaw_js_load`.
    /// Same `i8` -> `i1` conversion as `compile_json_as_bool` and for the
    /// same reason (the extern function avoids relying on `bool`'s C ABI
    /// shape).
    fn compile_load_script(&mut self, args: &[HirExpr]) -> Result<BasicValueEnum<'ctx>, String> {
        self.uses_quickjs = true;
        let [source] = args else {
            return Err("loadScript expects exactly one argument".to_string());
        };
        let source_val = self.compile_expr(source)?;
        let function = self.module.get_function("thaw_js_load").unwrap();
        let call = self
            .builder
            .build_call(function, &[source_val.into()], "load_script_u8")
            .map_err(|e| e.to_string())?;
        let u8_val = call
            .try_as_basic_value()
            .basic()
            .ok_or("thaw_js_load did not return a value")?
            .into_int_value();
        let zero = self.context.i8_type().const_int(0, false);
        self.builder
            .build_int_compare(inkwell::IntPredicate::NE, u8_val, zero, "load_script_ok")
            .map(Into::into)
            .map_err(|e| e.to_string())
    }

    fn compile_load_native_addon(
        &mut self,
        args: &[HirExpr],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        self.uses_napi = true;
        if !(1..=3).contains(&args.len()) {
            return Err("loadNativeAddon expects a path, optional root export name, and optional package name".to_string());
        }
        let path = self.compile_expr(&args[0])?;
        let mut call_args = vec![path.into()];
        let symbol = if args.len() >= 2 {
            call_args.push(self.compile_expr(&args[1])?.into());
            if args.len() == 3 {
                call_args.push(self.compile_expr(&args[2])?.into());
                "thaw_napi_load_named_qualified"
            } else {
                "thaw_napi_load_named"
            }
        } else {
            "thaw_napi_load"
        };
        let function = self.module.get_function(symbol).unwrap();
        let loaded = self
            .builder
            .build_call(function, &call_args, "load_napi_u8")
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .ok_or("thaw_napi_load did not return a value")?
            .into_int_value();
        self.builder
            .build_int_compare(
                IntPredicate::NE,
                loaded,
                self.context.i8_type().const_zero(),
                "load_napi_ok",
            )
            .map(Into::into)
            .map_err(|error| error.to_string())
    }

    fn compile_load_embedded_native_addon(
        &mut self,
        args: &[HirExpr],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        self.uses_napi = true;
        if !(2..=3).contains(&args.len()) {
            return Err(
                "loadNativeAddonEmbedded expects addon bytes and a root export name".into(),
            );
        }
        let bytes = self.compile_expr(&args[0])?;
        let root = self.compile_expr(&args[1])?;
        let package = args.get(2).map(|package| self.compile_expr(package)).transpose()?;
        let function = self
            .module
            .get_function(if package.is_some() { "thaw_napi_load_embedded_hex_qualified" } else { "thaw_napi_load_embedded_hex" })
            .unwrap();
        let loaded = self
            .builder
            .build_call(
                function,
                &if let Some(package) = package { vec![bytes.into(), root.into(), package.into()] } else { vec![bytes.into(), root.into()] },
                "load_embedded_napi_u8",
            )
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .ok_or("thaw_napi_load_embedded_hex did not return a value")?
            .into_int_value();
        self.builder
            .build_int_compare(
                IntPredicate::NE,
                loaded,
                self.context.i8_type().const_zero(),
                "load_embedded_napi_ok",
            )
            .map(Into::into)
            .map_err(|error| error.to_string())
    }

    fn compile_embed_executable(
        &mut self,
        args: &[HirExpr],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        self.uses_napi = true;
        let [bytes] = args else {
            return Err("embedExecutable expects executable bytes".into());
        };
        let bytes = self.compile_expr(bytes)?;
        self.builder
            .build_call(
                self.module
                    .get_function("thaw_napi_embed_executable_hex")
                    .unwrap(),
                &[bytes.into()],
                "embedded_executable_path",
            )
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .ok_or("thaw_napi_embed_executable_hex returned no value".into())
    }

    fn compile_set_process_env(
        &mut self,
        args: &[HirExpr],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let [name, value] = args else {
            return Err("setProcessEnv expects a name and value".into());
        };
        let name = self.compile_expr(name)?;
        let value = self.compile_expr(value)?;
        self.builder
            .build_call(
                self.module.get_function("thaw_js_set_process_env").unwrap(),
                &[name.into(), value.into()],
                "set_process_env",
            )
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .ok_or("thaw_js_set_process_env returned no value".into())
    }

    fn compile_load_embedded_native_dependency(
        &mut self,
        args: &[HirExpr],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        self.uses_napi = true;
        if args.len() != 1 {
            return Err("loadNativeSharedLibraryEmbedded expects shared library bytes".into());
        }
        let bytes = self.compile_expr(&args[0])?;
        let loaded = self
            .builder
            .build_call(
                self.module
                    .get_function("thaw_napi_load_embedded_shared_hex")
                    .unwrap(),
                &[bytes.into()],
                "load_embedded_native_dependency_u8",
            )
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .ok_or("thaw_napi_load_embedded_shared_hex did not return a value")?
            .into_int_value();
        self.builder
            .build_int_compare(
                IntPredicate::NE,
                loaded,
                self.context.i8_type().const_zero(),
                "load_embedded_native_dependency_ok",
            )
            .map(Into::into)
            .map_err(|error| error.to_string())
    }

    fn compile_load_native_dependency(
        &mut self,
        args: &[HirExpr],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        self.uses_napi = true;
        let [path] = args else {
            return Err("loadNativeSharedLibrary expects a path".into());
        };
        let path = self.compile_expr(path)?;
        let loaded = self
            .builder
            .build_call(
                self.module.get_function("thaw_napi_load_shared").unwrap(),
                &[path.into()],
                "load_native_dependency_u8",
            )
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .ok_or("thaw_napi_load_shared did not return a value")?
            .into_int_value();
        self.builder
            .build_int_compare(
                IntPredicate::NE,
                loaded,
                self.context.i8_type().const_zero(),
                "load_native_dependency_ok",
            )
            .map(Into::into)
            .map_err(|error| error.to_string())
    }

    /// `callDynamic(name, args): Json` -- the QuickJS-NG fallback path
    /// (docs/design/bridge.md section 7). Composes thaw-std's
    /// `thaw_json_stringify`/`thaw_json_parse` with thaw-quickjs's
    /// `thaw_js_call` so a `Json` value flows in and out without this
    /// module needing to know thaw-quickjs's internals (or vice versa).
    fn compile_call_dynamic(&mut self, args: &[HirExpr]) -> Result<BasicValueEnum<'ctx>, String> {
        self.compile_json_backend_call(args, "thaw_js_call_graph_result", "callDynamic")
    }

    fn compile_get_dynamic_value(
        &mut self,
        args: &[HirExpr],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        self.uses_quickjs = true;
        self.uses_quickjs_handles = true;
        self.compile_single_arg_call("thaw_js_get_global", args, "getDynamicValue")
    }

    fn compile_call_dynamic_value(
        &mut self,
        args: &[HirExpr],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        self.compile_json_backend_call(args, "thaw_js_call_handle_graph_result", "callDynamicValue")
    }

    /// `newDynamicFunction(args_json): JsValue` -- `new Function(...)` via
    /// the JS realm, with a source-keyed cache in `thaw_js_new_function`.
    fn compile_new_dynamic_function(
        &mut self,
        args: &[HirExpr],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        self.uses_quickjs = true;
        self.uses_quickjs_handles = true;
        let [call_args] = args else {
            return Err("newDynamicFunction expects exactly one argument".into());
        };
        let call_args = self.compile_expr(call_args)?;
        let args_json = self
            .builder
            .build_call(
                self.module.get_function("thaw_json_stringify").unwrap(),
                &[call_args.into()],
                "new_function_args",
            )
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .unwrap();
        let args_json = self.compile_check_json_stringify_error_with_cleanup(args_json, &[call_args])?;
        let result = self
            .builder
            .build_call(
                self.module.get_function("thaw_js_new_function").unwrap(),
                &[args_json.into()],
                "new_function_result",
            )
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .unwrap()
            .into_struct_value();
        let value = self
            .builder
            .build_extract_value(result, 0, "new_function_value")
            .map_err(|error| error.to_string())?;
        let error = self
            .builder
            .build_extract_value(result, 1, "new_function_error")
            .map_err(|error| error.to_string())?;
        self.builder
            .build_call(
                self.module.get_function("thaw_cstring_destroy").unwrap(),
                &[args_json.into()],
                "destroy_new_function_args_string",
            )
            .map_err(|error| error.to_string())?;
        self.builder
            .build_call(
                self.module.get_function("thaw_json_destroy").unwrap(),
                &[call_args.into()],
                "destroy_new_function_args",
            )
            .map_err(|error| error.to_string())?;
        self.builder
            .build_store(self.pending_exception().as_pointer_value(), error)
            .map_err(|error| error.to_string())?;
        self.clear_pending_native_text()?;
        self.mark_pending_native_text(error)?;
        self.branch_on_pending_exception()?;
        Ok(value)
    }

    fn compile_call_native_addon_value(
        &mut self,
        args: &[HirExpr],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        self.uses_napi = true;
        self.compile_json_backend_call(
            args,
            "thaw_napi_call_handle_typed_graph_result",
            "callNativeAddonValue",
        )
    }

    fn compile_call_dynamic_value_handle(
        &mut self,
        args: &[HirExpr],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        self.uses_quickjs = true;
        self.uses_quickjs_handles = true;
        let [handle, call_args] = args else {
            return Err("callDynamicValueHandle expects exactly two arguments".into());
        };
        let handle = self.compile_expr(handle)?;
        let outer_compiling_quickjs_dynamic_arguments = self.compiling_quickjs_dynamic_arguments;
        self.compiling_quickjs_dynamic_arguments = true;
        let call_args = self.compile_expr(call_args);
        self.compiling_quickjs_dynamic_arguments = outer_compiling_quickjs_dynamic_arguments;
        let call_args = call_args?;
        let args_json = self
            .builder
            .build_call(
                self.module.get_function("thaw_json_graph_encode").unwrap(),
                &[call_args.into()],
                "dynamic_handle_args",
            )
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .unwrap();
        let args_json = self.compile_check_json_stringify_error_with_cleanup(args_json, &[call_args])?;
        let result = self
            .builder
            .build_call(
                self.module
                    .get_function("thaw_js_call_handle_handle_graph_args_result")
                    .unwrap(),
                &[
                    handle.into(),
                    args_json.into(),
                    self.context.bool_type().const_zero().into(),
                ],
                "dynamic_handle_result",
            )
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .unwrap()
            .into_struct_value();
        let value = self
            .builder
            .build_extract_value(result, 0, "dynamic_handle_value")
            .map_err(|error| error.to_string())?;
        let error = self
            .builder
            .build_extract_value(result, 1, "dynamic_handle_error")
            .map_err(|error| error.to_string())?;
        self.builder
            .build_call(
                self.module.get_function("thaw_cstring_destroy").unwrap(),
                &[args_json.into()],
                "destroy_dynamic_handle_args_string",
            )
            .map_err(|error| error.to_string())?;
        self.builder
            .build_call(
                self.module.get_function("thaw_json_destroy").unwrap(),
                &[call_args.into()],
                "destroy_dynamic_handle_args",
            )
            .map_err(|error| error.to_string())?;
        self.builder
            .build_store(self.pending_exception().as_pointer_value(), error)
            .map_err(|error| error.to_string())?;
        self.clear_pending_native_text()?;
        self.mark_pending_native_text(error)?;
        self.branch_on_pending_exception()?;
        Ok(value)
    }

    fn compile_call_dynamic_value_with_value(
        &mut self,
        args: &[HirExpr],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        self.uses_quickjs = true;
        self.uses_quickjs_handles = true;
        let [callable, argument] = args else {
            return Err("callDynamicValueWithValue expects exactly two arguments".into());
        };
        let callable = self.compile_expr(callable)?;
        let argument = self.compile_expr(argument)?;
        let result = self
            .builder
            .build_call(
                self.module
                    .get_function("thaw_js_call_handle_value_graph_result")
                    .unwrap(),
                &[callable.into(), argument.into()],
                "dynamic_value_argument_result",
            )
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .unwrap()
            .into_struct_value();
        let value = self
            .builder
            .build_extract_value(result, 0, "dynamic_value_argument_json")
            .map_err(|error| error.to_string())?;
        let error = self
            .builder
            .build_extract_value(result, 1, "dynamic_value_argument_error")
            .map_err(|error| error.to_string())?;
        self.builder
            .build_store(self.pending_exception().as_pointer_value(), error)
            .map_err(|error| error.to_string())?;
        self.clear_pending_native_text()?;
        self.mark_pending_native_text(error)?;
        self.branch_on_pending_exception()?;
        self.compile_decode_quickjs_graph(value.into())
    }

    fn compile_release_dynamic_value(
        &mut self,
        args: &[HirExpr],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        self.uses_quickjs = true;
        self.uses_quickjs_handles = true;
        let released = self
            .compile_single_arg_call("thaw_js_release_handle", args, "releaseDynamicValue")?
            .into_int_value();
        self.builder
            .build_int_compare(
                IntPredicate::NE,
                released,
                self.context.i8_type().const_zero(),
                "released_dynamic_value",
            )
            .map(Into::into)
            .map_err(|error| error.to_string())
    }

    fn compile_dynamic_handle_operation(
        &mut self,
        symbol: &str,
        args: &[HirExpr],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        self.uses_quickjs = true;
        self.uses_quickjs_handles = true;
        let values = args
            .iter()
            .map(|argument| self.compile_expr(argument).map(Into::into))
            .collect::<Result<Vec<_>, _>>()?;
        let result = self
            .builder
            .build_call(self.module.get_function(symbol).unwrap(), &values, symbol)
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .unwrap()
            .into_struct_value();
        let value = self
            .builder
            .build_extract_value(result, 0, "dynamic_operation_value")
            .map_err(|error| error.to_string())?;
        let error = self
            .builder
            .build_extract_value(result, 1, "dynamic_operation_error")
            .map_err(|error| error.to_string())?;
        self.builder
            .build_store(self.pending_exception().as_pointer_value(), error)
            .map_err(|error| error.to_string())?;
        self.clear_pending_native_text()?;
        self.mark_pending_native_text(error)?;
        self.branch_on_pending_exception()?;
        Ok(value)
    }

    /// Compiles a `.method()` call's receiver expression, used by both
    /// `compile_call_dynamic_method` and `compile_call_dynamic_method_handle`.
    /// A receiver that is itself a `.method()` call is always lowered as
    /// the `"callDynamicMethodHandle"` intrinsic (`lower_dynamic_value_
    /// method_call` always requests `JsValue` for a receiver, regardless
    /// of what the *outer* call's own dispatch ends up being) -- so this
    /// structurally detects "I am not the terminal link in this chain" and
    /// compiles that inner call with `chain_intermediate: true`, meaning
    /// its own result must not have a pending thenable resolved early
    /// (see `compile_call_dynamic_method_handle`'s doc comment and the
    /// plan this fix came from: eagerly resolving every chain link, not
    /// just the terminal one, invokes a lazy query-builder's `.then` --
    /// e.g. drizzle-orm's -- before later links like `.where(...)` apply).
    fn compile_dynamic_method_receiver(
        &mut self,
        handle: &HirExpr,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        match handle {
            HirExpr::Call(callee, inner_args)
                if matches!(callee.as_ref(), HirExpr::Var(n) if n == "__thaw_call_selected_dynamic_method_handle") =>
            {
                self.compile_selected_call_dynamic_method_handle(inner_args, true)
            }
            HirExpr::Call(callee, inner_args)
                if matches!(callee.as_ref(), HirExpr::Var(n) if n == "callDynamicMethodHandle") =>
            {
                self.compile_call_dynamic_method_handle(inner_args, true)
            }
            _ => self.compile_expr(handle),
        }
    }

    // Capture the member before evaluating argument expressions. The returned
    // registry reference remains owned until invocation or an argument failure.
    fn compile_select_dynamic_method(
        &mut self, receiver: BasicValueEnum<'ctx>, name: BasicValueEnum<'ctx>,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        // Keep the receiver's original value alive even if an argument
        // releases the caller's own handle after the member has been read.
        let retained = self.builder.build_call(self.module.get_function("thaw_js_retain_handle").unwrap(),
            &[receiver.into()], "retain_selected_method_receiver")
            .map_err(|error| error.to_string())?.try_as_basic_value().basic()
            .ok_or("receiver retain returned no value")?.into_int_value();
        let valid = self.builder.build_int_compare(inkwell::IntPredicate::NE,
            retained, self.context.i8_type().const_zero(), "selected_receiver_retained")
            .map_err(|error| error.to_string())?;
        let ready = self.context.append_basic_block(self.current_function(), "selected_receiver_ready");
        let invalid = self.context.append_basic_block(self.current_function(), "selected_receiver_invalid");
        self.builder.build_conditional_branch(valid, ready, invalid)
            .map_err(|error| error.to_string())?;
        self.builder.position_at_end(invalid);
        self.compile_throw_type_error("Invalid JavaScript method receiver")?;
        self.builder.position_at_end(ready);
        let result = self.builder.build_call(
            self.module.get_function("thaw_js_get_property_result").unwrap(),
            &[receiver.into(), name.into()], "selected_method",
        ).map_err(|error| error.to_string())?.try_as_basic_value().basic().unwrap().into_struct_value();
        let selected = self.builder.build_extract_value(result, 0, "selected_method_handle")
            .map_err(|error| error.to_string())?;
        let error = self.builder.build_extract_value(result, 1, "selected_method_error")
            .map_err(|error| error.to_string())?;
        let failed = self.builder.build_is_not_null(error.into_pointer_value(), "selected_lookup_failed")
            .map_err(|error| error.to_string())?;
        let release = self.context.append_basic_block(self.current_function(), "release_selected_receiver_on_lookup_failure");
        let continue_block = self.context.append_basic_block(self.current_function(), "selected_lookup_checked");
        self.builder.build_conditional_branch(failed, release, continue_block)
            .map_err(|error| error.to_string())?;
        self.builder.position_at_end(release);
        self.builder.build_call(self.module.get_function("thaw_js_release_handle").unwrap(),
            &[receiver.into()], "release_failed_selected_receiver")
            .map_err(|error| error.to_string())?;
        self.builder.build_unconditional_branch(continue_block)
            .map_err(|error| error.to_string())?;
        self.builder.position_at_end(continue_block);
        self.builder.build_store(self.pending_exception().as_pointer_value(), error)
            .map_err(|error| error.to_string())?;
        self.clear_pending_native_text()?;
        self.mark_pending_native_text(error)?;
        self.branch_on_pending_exception()?;
        Ok(selected)
    }

    // A getter's result is retained across arbitrary argument expressions.
    // Intercept their pending-exception branches locally so it is released
    // before propagation to an enclosing catch or async completion.
    fn compile_selected_dynamic_arguments(
        &mut self, expression: &HirExpr, selected: BasicValueEnum<'ctx>,
        receiver: BasicValueEnum<'ctx>, label: &str,
    ) -> Result<(BasicValueEnum<'ctx>, BasicValueEnum<'ctx>), String> {
        let cleanup = self.context.append_basic_block(self.current_function(), "selected_method_arg_cleanup");
        self.push_catch_target(cleanup);
        let outer = self.compiling_quickjs_dynamic_arguments;
        self.compiling_quickjs_dynamic_arguments = true;
        let compiled = (|| -> Result<_, String> {
            let array = self.compile_expr(expression)?;
            let encoded = self.builder.build_call(
                self.module.get_function("thaw_json_graph_encode").unwrap(), &[array.into()], label,
            ).map_err(|error| error.to_string())?.try_as_basic_value().basic().unwrap();
            let encoded = self.compile_check_json_stringify_error_with_cleanup(encoded, &[array])?;
            Ok((array, encoded))
        })();
        self.compiling_quickjs_dynamic_arguments = outer;
        self.pop_catch_target();
        let success = self.builder.get_insert_block().unwrap();
        self.builder.position_at_end(cleanup);
        self.builder.build_call(self.module.get_function("thaw_js_release_handle").unwrap(),
            &[selected.into()], "release_selected_method_on_argument_error")
            .map_err(|error| error.to_string())?;
        self.builder.build_call(self.module.get_function("thaw_js_release_handle").unwrap(),
            &[receiver.into()], "release_selected_receiver_on_argument_error")
            .map_err(|error| error.to_string())?;
        self.branch_on_pending_exception()?;
        self.builder.position_at_end(success);
        compiled
    }

    fn compile_call_dynamic_method(
        &mut self,
        args: &[HirExpr],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        self.uses_quickjs = true;
        self.uses_quickjs_handles = true;
        let [handle, name, call_args] = args else {
            return Err("callDynamicMethod expects exactly three arguments".into());
        };
        let handle = self.compile_dynamic_method_receiver(handle)?;
        let name = self.compile_expr(name)?;
        // Set for the same reason `compile_typed_dynamic_call` sets it
        // around its own args-marshaling: `call_args` (the pre-built
        // `HirExpr::ArrayLit` of already-Json-coerced arguments -- see
        // `lower_dynamic_value_method_call`) can itself contain a
        // `JsValue`-typed element (a live handle nested in the arg array,
        // not just a bare top-level arg -- real example: zod's `.refine(
        // (n: number) => n > 0, ...)`, whose predicate becomes a `JsValue`
        // via `registerNativeCallback`), and `compile_dynamic_value_
        // placeholder` refuses to encode one unless this flag is set.
        // Method-call args previously never set it at all (only the typed
        // ambient-declaration dispatch path did), so a `JsValue` nested in
        // a method call's own arguments -- as opposed to a top-level
        // function call's -- failed outright until now.
        let outer_compiling_quickjs_dynamic_arguments = self.compiling_quickjs_dynamic_arguments;
        self.compiling_quickjs_dynamic_arguments = true;
        let call_args = self.compile_expr(call_args);
        self.compiling_quickjs_dynamic_arguments = outer_compiling_quickjs_dynamic_arguments;
        let call_args = call_args?;
        let args_json = self
            .builder
            .build_call(
                self.module.get_function("thaw_json_graph_encode").unwrap(),
                &[call_args.into()],
                "dynamic_method_args",
            )
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .unwrap();
        let args_json = self.compile_check_json_stringify_error_with_cleanup(args_json, &[call_args])?;
        let result = self
            .builder
            .build_call(
                self.module
                    .get_function("thaw_js_call_method_graph_result")
                    .unwrap(),
                &[handle.into(), name.into(), args_json.into()],
                "dynamic_method_result",
            )
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .unwrap()
            .into_struct_value();
        let value = self
            .builder
            .build_extract_value(result, 0, "dynamic_method_json")
            .map_err(|error| error.to_string())?;
        let error = self
            .builder
            .build_extract_value(result, 1, "dynamic_method_error")
            .map_err(|error| error.to_string())?;
        self.builder
            .build_call(
                self.module.get_function("thaw_cstring_destroy").unwrap(),
                &[args_json.into()],
                "destroy_dynamic_method_args_string",
            )
            .map_err(|error| error.to_string())?;
        self.builder
            .build_call(
                self.module.get_function("thaw_json_destroy").unwrap(),
                &[call_args.into()],
                "destroy_dynamic_method_args",
            )
            .map_err(|error| error.to_string())?;
        self.builder
            .build_store(self.pending_exception().as_pointer_value(), error)
            .map_err(|error| error.to_string())?;
        self.clear_pending_native_text()?;
        self.mark_pending_native_text(error)?;
        self.branch_on_pending_exception()?;
        let parsed = self.compile_decode_quickjs_graph(value.into())?;

        Ok(parsed)
    }

    fn compile_selected_call_dynamic_method(
        &mut self,
        args: &[HirExpr],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        self.uses_quickjs = true;
        self.uses_quickjs_handles = true;
        let [handle, name, call_args] = args else {
            return Err("callDynamicMethod expects exactly three arguments".into());
        };
        let handle = self.compile_dynamic_method_receiver(handle)?;
        let name = self.compile_expr(name)?;
        let selected = self.compile_select_dynamic_method(handle, name)?;
        let (call_args, args_json) =
            self.compile_selected_dynamic_arguments(call_args, selected, handle, "dynamic_method_args")?;
        let result = self
            .builder
            .build_call(
                self.module
                    .get_function("thaw_js_call_selected_method_graph_result")
                    .unwrap(),
                &[handle.into(), selected.into(), name.into(), args_json.into()],
                "dynamic_method_result",
            )
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .unwrap()
            .into_struct_value();
        let value = self
            .builder
            .build_extract_value(result, 0, "dynamic_method_json")
            .map_err(|error| error.to_string())?;
        let error = self
            .builder
            .build_extract_value(result, 1, "dynamic_method_error")
            .map_err(|error| error.to_string())?;
        self.builder
            .build_call(
                self.module.get_function("thaw_cstring_destroy").unwrap(),
                &[args_json.into()],
                "destroy_dynamic_method_args_string",
            )
            .map_err(|error| error.to_string())?;
        self.builder
            .build_call(
                self.module.get_function("thaw_json_destroy").unwrap(),
                &[call_args.into()],
                "destroy_dynamic_method_args",
            )
            .map_err(|error| error.to_string())?;
        self.builder.build_call(
            self.module.get_function("thaw_js_release_handle").unwrap(),
            &[selected.into()], "release_selected_method",
        ).map_err(|error| error.to_string())?;
        self.builder.build_call(self.module.get_function("thaw_js_release_handle").unwrap(),
            &[handle.into()], "release_selected_receiver")
            .map_err(|error| error.to_string())?;
        self.builder
            .build_store(self.pending_exception().as_pointer_value(), error)
            .map_err(|error| error.to_string())?;
        self.clear_pending_native_text()?;
        self.mark_pending_native_text(error)?;
        self.branch_on_pending_exception()?;
        let parsed = self.compile_decode_quickjs_graph(value.into())?;

        Ok(parsed)
    }

    /// Like `compile_call_dynamic_method`, but for a method whose own
    /// result is itself a `JsValue` (e.g. a schema instance's chained
    /// method returning another schema instance) rather than plain data:
    /// retains the result as a handle via `thaw_js_call_method_handle_result`
    /// instead of JSON-decoding it. See `callDynamicMethodHandle`'s doc
    /// comment (thaw-hir's `inference/types.rs`) for how a call chooses
    /// between the two.
    fn compile_call_dynamic_method_handle(
        &mut self,
        args: &[HirExpr],
        chain_intermediate: bool,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        self.uses_quickjs = true;
        self.uses_quickjs_handles = true;
        let [handle, name, call_args] = args else {
            return Err("callDynamicMethodHandle expects exactly three arguments".into());
        };
        let handle = self.compile_dynamic_method_receiver(handle)?;
        let name = self.compile_expr(name)?;
        // See the matching comment in `compile_call_dynamic_method`.
        let outer_compiling_quickjs_dynamic_arguments = self.compiling_quickjs_dynamic_arguments;
        self.compiling_quickjs_dynamic_arguments = true;
        let call_args = self.compile_expr(call_args);
        self.compiling_quickjs_dynamic_arguments = outer_compiling_quickjs_dynamic_arguments;
        let call_args = call_args?;
        let args_json = self
            .builder
            .build_call(
                self.module.get_function("thaw_json_graph_encode").unwrap(),
                &[call_args.into()],
                "dynamic_method_handle_args",
            )
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .unwrap();
        let args_json = self.compile_check_json_stringify_error_with_cleanup(args_json, &[call_args])?;
        let chain_intermediate_flag = self
            .context
            .bool_type()
            .const_int(chain_intermediate as u64, false);
        let result = self
            .builder
            .build_call(
                self.module
                    .get_function("thaw_js_call_method_handle_graph_args_result")
                    .unwrap(),
                &[
                    handle.into(),
                    name.into(),
                    args_json.into(),
                    chain_intermediate_flag.into(),
                ],
                "dynamic_method_handle_result",
            )
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .unwrap()
            .into_struct_value();
        let value = self
            .builder
            .build_extract_value(result, 0, "dynamic_method_handle_value")
            .map_err(|error| error.to_string())?;
        let error = self
            .builder
            .build_extract_value(result, 1, "dynamic_method_handle_error")
            .map_err(|error| error.to_string())?;
        self.builder
            .build_call(
                self.module.get_function("thaw_cstring_destroy").unwrap(),
                &[args_json.into()],
                "destroy_dynamic_method_handle_args_string",
            )
            .map_err(|error| error.to_string())?;
        self.builder
            .build_call(
                self.module.get_function("thaw_json_destroy").unwrap(),
                &[call_args.into()],
                "destroy_dynamic_method_handle_args",
            )
            .map_err(|error| error.to_string())?;
        self.builder
            .build_store(self.pending_exception().as_pointer_value(), error)
            .map_err(|error| error.to_string())?;
        self.clear_pending_native_text()?;
        self.mark_pending_native_text(error)?;
        self.branch_on_pending_exception()?;
        Ok(value)
    }

    fn compile_selected_call_dynamic_method_handle(
        &mut self,
        args: &[HirExpr],
        chain_intermediate: bool,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        self.uses_quickjs = true;
        self.uses_quickjs_handles = true;
        let [handle, name, call_args] = args else {
            return Err("callDynamicMethodHandle expects exactly three arguments".into());
        };
        let handle = self.compile_dynamic_method_receiver(handle)?;
        let name = self.compile_expr(name)?;
        let selected = self.compile_select_dynamic_method(handle, name)?;
        let (call_args, args_json) =
            self.compile_selected_dynamic_arguments(call_args, selected, handle, "dynamic_method_handle_args")?;
        let chain_intermediate_flag = self.context.bool_type()
            .const_int(chain_intermediate as u64, false);
        let result = self
            .builder
            .build_call(
                self.module
                    .get_function("thaw_js_call_selected_method_handle_graph_args_result")
                    .unwrap(),
                &[
                    handle.into(),
                    selected.into(),
                    name.into(),
                    args_json.into(),
                    chain_intermediate_flag.into(),
                ],
                "dynamic_method_handle_result",
            )
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .unwrap()
            .into_struct_value();
        let value = self
            .builder
            .build_extract_value(result, 0, "dynamic_method_handle_value")
            .map_err(|error| error.to_string())?;
        let error = self
            .builder
            .build_extract_value(result, 1, "dynamic_method_handle_error")
            .map_err(|error| error.to_string())?;
        self.builder
            .build_call(
                self.module.get_function("thaw_cstring_destroy").unwrap(),
                &[args_json.into()],
                "destroy_dynamic_method_handle_args_string",
            )
            .map_err(|error| error.to_string())?;
        self.builder
            .build_call(
                self.module.get_function("thaw_json_destroy").unwrap(),
                &[call_args.into()],
                "destroy_dynamic_method_handle_args",
            )
            .map_err(|error| error.to_string())?;
        self.builder.build_call(
            self.module.get_function("thaw_js_release_handle").unwrap(),
            &[selected.into()], "release_selected_method",
        ).map_err(|error| error.to_string())?;
        self.builder.build_call(self.module.get_function("thaw_js_release_handle").unwrap(),
            &[handle.into()], "release_selected_receiver")
            .map_err(|error| error.to_string())?;
        self.builder
            .build_store(self.pending_exception().as_pointer_value(), error)
            .map_err(|error| error.to_string())?;
        self.clear_pending_native_text()?;
        self.mark_pending_native_text(error)?;
        self.branch_on_pending_exception()?;
        Ok(value)
    }

    fn compile_read_dynamic_value(
        &mut self,
        args: &[HirExpr],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        self.uses_quickjs = true;
        self.uses_quickjs_handles = true;
        let [handle] = args else {
            return Err("readDynamicValue expects exactly one argument".into());
        };
        let handle = self.compile_expr(handle)?;
        let result = self
            .builder
            .build_call(
                self.module
                    .get_function("thaw_js_resolve_handle_graph_result")
                    .unwrap(),
                &[handle.into()],
                "read_dynamic_value",
            )
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .unwrap()
            .into_struct_value();
        let value = self
            .builder
            .build_extract_value(result, 0, "read_dynamic_json")
            .map_err(|error| error.to_string())?;
        let error = self
            .builder
            .build_extract_value(result, 1, "read_dynamic_error")
            .map_err(|error| error.to_string())?;
        self.builder
            .build_store(self.pending_exception().as_pointer_value(), error)
            .map_err(|error| error.to_string())?;
        self.clear_pending_native_text()?;
        self.mark_pending_native_text(error)?;
        self.branch_on_pending_exception()?;
        self.compile_decode_quickjs_graph(value.into())
    }

    fn compile_call_dynamic_value_mixed_exact(
        &mut self,
        args: &[HirExpr],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        self.uses_quickjs = true;
        self.uses_quickjs_handles = true;
        let [callable, json_args, handles] = args else {
            return Err("callDynamicValueMixedExact expects callable, JSON arguments, and owned handle array".into());
        };
        // This path can be the first QuickJS operation on the thread. Json
        // arguments may carry Host/function/Symbol leases, so install the
        // retained-value callbacks before graph encoding or decoding them.
        self.compile_register_js_callback_host_operations()?;
        let HirExpr::ArrayLit(direct) = handles else {
            return Err("callDynamicValueMixedExact requires a compiler-built direct-handle array".into());
        };
        // A regular ArrayLit evaluates every element before publishing its
        // buffer. If a later element throws, earlier owned JS handles are
        // invisible to the consuming FFI and leak. Stage them separately in
        // local cells so the pending-exception path can retire exactly those
        // already acquired. The final raw array exists only for this call.
        let i64_type = self.context.i64_type();
        let ptr_type = self.context.ptr_type(inkwell::AddressSpace::default());
        let direct_cells = direct.iter().map(|_| {
            let cell = self.builder.build_alloca(i64_type, "exact_mixed_direct_cell")
                .map_err(|error| error.to_string())?;
            self.builder.build_store(cell, i64_type.const_zero())
                .map_err(|error| error.to_string())?;
            Ok::<_, String>(cell)
        }).collect::<Result<Vec<_>, _>>()?;
        let json_cell = self.builder.build_alloca(ptr_type, "exact_mixed_json_cell")
            .map_err(|error| error.to_string())?;
        self.builder.build_store(json_cell, ptr_type.const_null())
            .map_err(|error| error.to_string())?;
        let callable_cell = self.builder.build_alloca(i64_type, "exact_mixed_callable_cell")
            .map_err(|error| error.to_string())?;
        self.builder.build_store(callable_cell, i64_type.const_zero())
            .map_err(|error| error.to_string())?;
        let cleanup = self.context.append_basic_block(self.current_function(),
            "exact_mixed_staging_failed");
        self.push_catch_target(cleanup);
        let staged = (|| -> Result<_, String> {
            let callable = self.compile_expr(callable)?.into_int_value();
            self.builder.build_store(callable_cell, callable)
                .map_err(|error| error.to_string())?;
            let json = self.compile_expr(json_args)?.into_pointer_value();
            self.builder.build_store(json_cell, json)
                .map_err(|error| error.to_string())?;
            for (expression, cell) in direct.iter().zip(&direct_cells) {
                let value = self.compile_expr(expression)?.into_int_value();
                self.builder.build_store(*cell, value)
                    .map_err(|error| error.to_string())?;
            }
            Ok((callable, json))
        })();
        self.pop_catch_target();
        let success = self.builder.get_insert_block().unwrap();
        self.builder.position_at_end(cleanup);
        for cell in &direct_cells {
            let value = self.builder.build_load(i64_type, *cell, "failed_direct_handle")
                .map_err(|error| error.to_string())?.into_int_value();
            self.builder.build_call(self.module.get_function("thaw_js_release_handle").unwrap(),
                &[value.into()], "release_staged_direct_handle")
                .map_err(|error| error.to_string())?;
        }
        let staged_callable = self.builder.build_load(i64_type, callable_cell,
            "failed_mixed_callable").map_err(|error| error.to_string())?.into_int_value();
        self.builder.build_call(self.module.get_function("thaw_js_release_handle").unwrap(),
            &[staged_callable.into()], "release_staged_callable")
            .map_err(|error| error.to_string())?;
        let staged_json = self.builder.build_load(ptr_type, json_cell, "failed_mixed_json")
            .map_err(|error| error.to_string())?.into_pointer_value();
        let has_json = self.builder.build_is_not_null(staged_json, "has_staged_mixed_json")
            .map_err(|error| error.to_string())?;
        let destroy_json = self.context.append_basic_block(self.current_function(),
            "destroy_staged_mixed_json");
        let propagate = self.context.append_basic_block(self.current_function(),
            "propagate_exact_mixed_staging_failure");
        self.builder.build_conditional_branch(has_json, destroy_json, propagate)
            .map_err(|error| error.to_string())?;
        self.builder.position_at_end(destroy_json);
        self.builder.build_call(self.module.get_function("thaw_json_destroy").unwrap(),
            &[staged_json.into()], "destroy_staged_mixed_json")
            .map_err(|error| error.to_string())?;
        self.builder.build_unconditional_branch(propagate)
            .map_err(|error| error.to_string())?;
        self.builder.position_at_end(propagate);
        self.branch_on_pending_exception()?;
        self.builder.position_at_end(success);
        let (callable, json_args) = staged?;
        let raw = self.builder.build_alloca(i64_type.array_type((direct.len() + 1) as u32),
            "exact_mixed_direct_array").map_err(|error| error.to_string())?;
        self.builder.build_store(raw, i64_type.const_int(direct.len() as u64, false))
            .map_err(|error| error.to_string())?;
        for (index, cell) in direct_cells.iter().enumerate() {
            let value = self.builder.build_load(i64_type, *cell, "exact_mixed_direct")
                .map_err(|error| error.to_string())?;
            let slot = unsafe { self.builder.build_in_bounds_gep(i64_type, raw,
                &[i64_type.const_int((index + 1) as u64, false)], "exact_mixed_direct_slot") }
                .map_err(|error| error.to_string())?;
            self.builder.build_store(slot, value).map_err(|error| error.to_string())?;
        }
        let handles = raw;
        let text = self.builder.build_call(
            self.module.get_function("thaw_json_graph_encode").unwrap(),
            &[json_args.into()], "exact_mixed_args_json",
        ).map_err(|error| error.to_string())?
            .try_as_basic_value().basic().unwrap();
        // Before the consuming call, an encoder failure must retire every
        // staged JS reference as well as the temporary Json graph.
        let text = self.compile_check_json_stringify_error_with_inputs_and_handles(
            text, &[json_args.into()], &[], &[handles], &[callable],
        )?;
        let result = self.builder.build_call(
            self.module.get_function("thaw_js_call_handle_mixed_exact_consuming_result").unwrap(),
            &[callable.into(), text.into(), handles.into()], "exact_mixed_result",
        ).map_err(|error| error.to_string())?
            .try_as_basic_value().basic()
            .ok_or("exact mixed call returned no result")?.into_struct_value();
        self.builder.build_call(
            self.module.get_function("thaw_cstring_destroy").unwrap(),
            &[text.into()], "destroy_exact_mixed_args_string",
        ).map_err(|error| error.to_string())?;
        self.builder.build_call(
            self.module.get_function("thaw_json_destroy").unwrap(),
            &[json_args.into()], "destroy_exact_mixed_args_json",
        ).map_err(|error| error.to_string())?;
        let value = self.compile_exact_iterator_handle_result(result, "exact_mixed_call")?;
        self.compile_original_quickjs_exception_handle(value, true)
    }

    fn compile_call_dynamic_value_mixed(
        &mut self,
        args: &[HirExpr],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        self.uses_quickjs = true;
        self.uses_quickjs_handles = true;
        let [callable, json_args, handles] = args else {
            return Err(
                "callDynamicValueMixed expects callable, JSON arguments, and handle array".into(),
            );
        };
        let callable = self.compile_expr(callable)?;
        let json_args = self.compile_expr(json_args)?;
        let handles = self.compile_expr(handles)?.into_pointer_value();
        let handles = self.compile_array_data(handles)?;
        let text = self
            .builder
            .build_call(
                self.module.get_function("thaw_json_graph_encode").unwrap(),
                &[json_args.into()],
                "mixed_args_json",
            )
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .unwrap();
        let text = self.compile_check_json_stringify_error_with_cleanup(text, &[json_args])?;
        let result = self
            .builder
            .build_call(
                self.module
                    .get_function("thaw_js_call_handle_mixed_graph_result")
                    .unwrap(),
                &[callable.into(), text.into(), handles.into()],
                "mixed_dynamic_result",
            )
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .unwrap()
            .into_struct_value();
        let value = self
            .builder
            .build_extract_value(result, 0, "mixed_dynamic_json")
            .map_err(|error| error.to_string())?;
        let error = self
            .builder
            .build_extract_value(result, 1, "mixed_dynamic_error")
            .map_err(|error| error.to_string())?;
        self.builder.build_call(
            self.module.get_function("thaw_cstring_destroy").unwrap(),
            &[text.into()], "destroy_mixed_args_string",
        ).map_err(|error| error.to_string())?;
        self.builder.build_call(
            self.module.get_function("thaw_json_destroy").unwrap(),
            &[json_args.into()], "destroy_mixed_args_json",
        ).map_err(|error| error.to_string())?;
        self.builder
            .build_store(self.pending_exception().as_pointer_value(), error)
            .map_err(|error| error.to_string())?;
        self.clear_pending_native_text()?;
        self.mark_pending_native_text(error)?;
        self.branch_on_pending_exception()?;
        self.compile_decode_quickjs_graph(value.into())
    }

    fn compile_call_dynamic_value_mixed_native_json(
        &mut self,
        args: &[HirExpr],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        self.uses_quickjs = true;
        self.uses_quickjs_handles = true;
        let [callable, value, space, handles] = args else {
            return Err("callDynamicValueMixedNativeJson expects callable, value, space, and handles".into());
        };
        let callable = self.compile_expr(callable)?;
        let value = self.compile_expr(value)?;
        let space = self.compile_expr(space)?;
        let handles = self.compile_expr(handles)?.into_pointer_value();
        let handles = self.compile_array_data(handles)?;
        let graph = self.builder.build_call(
            self.module.get_function("thaw_json_graph_encode").unwrap(),
            &[value.into()], "replacer_native_graph",
        ).map_err(|error| error.to_string())?.try_as_basic_value().basic()
            .ok_or("native JSON graph encoder returned no value")?;
        let space_text = self.builder.build_call(
            self.module.get_function("thaw_json_stringify").unwrap(),
            &[space.into()], "replacer_space_json",
        ).map_err(|error| error.to_string())?.try_as_basic_value().basic()
            .ok_or("replacer space serializer returned no value")?;
        let space_text = self.compile_check_json_stringify_error_with_inputs(space_text, &[space], &[graph])?;
        let result = self.builder.build_call(
            self.module.get_function("thaw_js_call_handle_mixed_native_json_graph_result").unwrap(),
            &[callable.into(), graph.into(), space_text.into(), handles.into()],
            "mixed_native_json_result",
        ).map_err(|error| error.to_string())?.try_as_basic_value().basic()
            .ok_or("mixed native JSON call returned no value")?.into_struct_value();
        let result_text = self.builder.build_extract_value(result, 0, "mixed_native_json_text")
            .map_err(|error| error.to_string())?;
        let error = self.builder.build_extract_value(result, 1, "mixed_native_json_error")
            .map_err(|error| error.to_string())?;
        for text in [graph, space_text] {
            self.builder.build_call(
                self.module.get_function("thaw_cstring_destroy").unwrap(),
                &[text.into()], "destroy_mixed_native_json_input",
            ).map_err(|error| error.to_string())?;
        }
        self.builder.build_call(
            self.module.get_function("thaw_json_destroy").unwrap(),
            &[space.into()], "destroy_replacer_space_json",
        ).map_err(|error| error.to_string())?;
        self.builder.build_store(self.pending_exception().as_pointer_value(), error)
            .map_err(|error| error.to_string())?;
        self.clear_pending_native_text()?;
        self.mark_pending_native_text(error)?;
        self.branch_on_pending_exception()?;
        let parsed = self.compile_decode_quickjs_graph(result_text.into())?;

        Ok(parsed)
    }

    fn compile_call_dynamic_value_mixed_handle(
        &mut self,
        args: &[HirExpr],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        self.compile_call_dynamic_value_mixed_handle_impl(args, false)
    }

    fn compile_build_native_object_wrapper(
        &mut self,
        args: &[HirExpr],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        self.compile_call_dynamic_value_mixed_handle_impl(args, true)
    }

    fn compile_call_dynamic_value_mixed_handle_impl(
        &mut self,
        args: &[HirExpr],
        consume_handles: bool,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        self.uses_quickjs = true;
        self.uses_quickjs_handles = true;
        let [callable, json_args, handles] = args else {
            return Err(
                "callDynamicValueMixedHandle expects callable, JSON arguments, and handle array"
                    .into(),
            );
        };
        let callable = self.compile_expr(callable)?.into_int_value();
        let json_args = self.compile_expr(json_args)?;
        let handles = self.compile_expr(handles)?.into_pointer_value();
        let handles = self.compile_array_data(handles)?;
        let text = self
            .builder
            .build_call(
                self.module.get_function("thaw_json_graph_encode").unwrap(),
                &[json_args.into()],
                "mixed_handle_args_json",
            )
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .unwrap();
        let text = if consume_handles {
            self.compile_check_json_stringify_error_with_inputs_and_handles(
                text, &[json_args], &[], &[handles], &[callable],
            )?
        } else {
            self.compile_check_json_stringify_error_with_cleanup(text, &[json_args])?
        };
        let result = self
            .builder
            .build_call(
                self.module
                    .get_function(if consume_handles {
                        "thaw_js_call_handle_mixed_handle_graph_args_consuming_result"
                    } else { "thaw_js_call_handle_mixed_handle_graph_args_result" })
                    .unwrap(),
                &[callable.into(), text.into(), handles.into()],
                "mixed_handle_result",
            )
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .unwrap()
            .into_struct_value();
        let value = self
            .builder
            .build_extract_value(result, 0, "mixed_handle_value")
            .map_err(|error| error.to_string())?;
        let error = self
            .builder
            .build_extract_value(result, 1, "mixed_handle_error")
            .map_err(|error| error.to_string())?;
        self.builder.build_call(
            self.module.get_function("thaw_cstring_destroy").unwrap(),
            &[text.into()], "destroy_mixed_args_string",
        ).map_err(|error| error.to_string())?;
        self.builder.build_call(
            self.module.get_function("thaw_json_destroy").unwrap(),
            &[json_args.into()], "destroy_mixed_args_json",
        ).map_err(|error| error.to_string())?;
        self.builder
            .build_store(self.pending_exception().as_pointer_value(), error)
            .map_err(|error| error.to_string())?;
        self.clear_pending_native_text()?;
        self.mark_pending_native_text(error)?;
        self.branch_on_pending_exception()?;
        Ok(value)
    }

    fn compile_construct_dynamic_value(
        &mut self,
        args: &[HirExpr],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        self.uses_quickjs = true;
        self.uses_quickjs_handles = true;
        let [constructor, json_args] = args else {
            return Err("constructDynamicValue expects constructor and JSON arguments".into());
        };
        let constructor = self.compile_expr(constructor)?;
        let outer_compiling_quickjs_dynamic_arguments = self.compiling_quickjs_dynamic_arguments;
        self.compiling_quickjs_dynamic_arguments = true;
        let json_args = self.compile_expr(json_args);
        self.compiling_quickjs_dynamic_arguments = outer_compiling_quickjs_dynamic_arguments;
        let json_args = json_args?;
        let text = self
            .builder
            .build_call(
                self.module.get_function("thaw_json_graph_encode").unwrap(),
                &[json_args.into()],
                "constructor_args_json",
            )
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .unwrap();
        let text = self.compile_check_json_stringify_error_with_cleanup(text, &[json_args])?;
        let result = self
            .builder
            .build_call(
                self.module
                    .get_function("thaw_js_construct_handle_graph_args_result")
                    .unwrap(),
                &[constructor.into(), text.into()],
                "construct_dynamic_result",
            )
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .unwrap()
            .into_struct_value();
        let value = self
            .builder
            .build_extract_value(result, 0, "constructed_dynamic_value")
            .map_err(|error| error.to_string())?;
        let error = self
            .builder
            .build_extract_value(result, 1, "construct_dynamic_error")
            .map_err(|error| error.to_string())?;
        self.builder
            .build_call(
                self.module.get_function("thaw_cstring_destroy").unwrap(),
                &[text.into()],
                "destroy_constructor_args_string",
            )
            .map_err(|error| error.to_string())?;
        self.builder
            .build_call(
                self.module.get_function("thaw_json_destroy").unwrap(),
                &[json_args.into()],
                "destroy_constructor_args",
            )
            .map_err(|error| error.to_string())?;
        self.builder
            .build_store(self.pending_exception().as_pointer_value(), error)
            .map_err(|error| error.to_string())?;
        self.clear_pending_native_text()?;
        self.mark_pending_native_text(error)?;
        self.branch_on_pending_exception()?;
        Ok(value)
    }

}
