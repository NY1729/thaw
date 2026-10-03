impl<'ctx> HirCompiler<'ctx> {
    fn compile_dynamic_named_call(
        &mut self,
        name: &str,
        args: &[HirExpr],
    ) -> Option<Result<BasicValueEnum<'ctx>, String>> {
        Some(match name {
            "loadScript" => self.compile_load_script(args),
            "callDynamic" => self.compile_call_dynamic(args),
            "getDynamicValue" => self.compile_get_dynamic_value(args),
            "newDynamicFunction" => self.compile_new_dynamic_function(args),
            "callDynamicValue" => self.compile_call_dynamic_value(args),
            "callNativeAddonValue" => self.compile_call_native_addon_value(args),
            "callDynamicValueHandle" => self.compile_call_dynamic_value_handle(args),
            "callDynamicValueWithValue" => self.compile_call_dynamic_value_with_value(args),
            "releaseDynamicValue" => self.compile_release_dynamic_value(args),
            "getDynamicProperty" => {
                self.compile_dynamic_handle_operation("thaw_js_get_property_result", args)
            }
            "deleteDynamicProperty" => self.compile_dynamic_property_predicate(
                "thaw_js_delete_property_result",
                args,
            ),
            "hasDynamicProperty" => {
                self.compile_dynamic_property_predicate("thaw_js_has_property_result", args)
            }
            "setDynamicProperty" => self.compile_set_dynamic_property(args),
            "setDynamicPropertyJson" => self.compile_set_dynamic_property_json(args),
            "callDynamicMethod" => self.compile_call_dynamic_method(args),
            "__thaw_call_selected_dynamic_method" => self.compile_selected_call_dynamic_method(args),
            "__thaw_call_selected_dynamic_method_handle" => self.compile_selected_call_dynamic_method_handle(args, false),
            "__thaw_call_selected_dynamic_method_raw" => self.compile_selected_call_dynamic_method_handle(args, true),
            "callDynamicMethodHandle" => self.compile_call_dynamic_method_handle(args, false),
            "callDynamicMethodHandleRaw" => self.compile_call_dynamic_method_handle(args, true),
            "readDynamicValue" => self.compile_read_dynamic_value(args),
            "retainDynamicJson" => self.compile_retain_dynamic_json(args),
            "__thaw_json_host_from_dynamic" => self.compile_host_json_from_dynamic(args),
            "__thaw_lookup_native_object" => self.compile_lookup_native_object(args),
            "__thaw_lookup_native_projector" => self.compile_lookup_native_projector(args),
            "__thaw_register_native_object_projector" =>
                self.compile_register_native_object_projector(args),
            "__thaw_register_native_object_layout" =>
                self.compile_register_native_object_layout(args),
            "__thaw_require_native_owner" => self.compile_require_native_owner(args),
            "__thaw_release_native_projection_callbacks" =>
                self.compile_release_native_projection_callbacks(args),
            "__thaw_intern_native_object" => self.compile_intern_native_object(args),
            "resolveDynamicValue" => self.compile_dynamic_handle_operation(
                "thaw_js_resolve_handle_handle_result",
                args,
            ),
            "callDynamicValueMixed" => self.compile_call_dynamic_value_mixed(args),
            "callDynamicValueMixedNativeJson" => self.compile_call_dynamic_value_mixed_native_json(args),
            "callDynamicValueMixedHandle" => {
                self.compile_call_dynamic_value_mixed_handle(args)
            }
            "__thaw_build_native_object_wrapper" => {
                self.compile_build_native_object_wrapper(args)
            }
            "constructDynamicValue" => self.compile_construct_dynamic_value(args),
            "loadNativeAddon" => self.compile_load_native_addon(args),
            "loadNativeAddonEmbedded" => self.compile_load_embedded_native_addon(args),
            "embedExecutable" => self.compile_embed_executable(args),
            "setProcessEnv" => self.compile_set_process_env(args),
            "loadNativeSharedLibraryEmbedded" => self.compile_load_embedded_native_dependency(args),
            "loadNativeSharedLibrary" => self.compile_load_native_dependency(args),
            "callNativeAddon" => self.compile_call_native_addon(args),
            "callNativeAddonWithCallback" => self.compile_call_native_addon_with_callback(args),
            "pollNativeAddonEvents" => self.compile_poll_native_addon_events(args),
            "registerNativeCallback" => self.compile_register_native_callback(args, false),
            "registerNativeCallbackGraph" => self.compile_register_native_callback(args, true),
            "__thaw_register_native_method_callback_graph" => {
                self.compile_register_native_method_callback_graph(args)
            }
            _ => return None,
        })
    }

    fn compile_dynamic_property_predicate(
        &mut self,
        symbol: &str,
        args: &[HirExpr],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let value = self
            .compile_dynamic_handle_operation(symbol, args)?
            .into_int_value();
        self.builder
            .build_int_compare(
                IntPredicate::NE,
                value,
                self.context.i64_type().const_zero(),
                "dynamic_property_predicate",
            )
            .map(Into::into)
            .map_err(|error| error.to_string())
    }

    fn compile_set_dynamic_property(
        &mut self,
        args: &[HirExpr],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let value = self
            .compile_dynamic_handle_operation("thaw_js_set_property_result", args)?
            .into_int_value();
        self.builder
            .build_int_compare(
                IntPredicate::NE,
                value,
                self.context.i64_type().const_zero(),
                "dynamic_property_set",
            )
            .map(Into::into)
            .map_err(|error| error.to_string())
    }

    fn compile_host_json_from_dynamic(
        &mut self,
        args: &[HirExpr],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let [source] = args else {
            return Err("live JSON host conversion expects one handle".into());
        };
        self.uses_quickjs = true;
        self.uses_quickjs_handles = true;
        self.tracks_owned_json_roots = true;
        let handle = self.compile_expr(source)?.into_int_value();
        let json = self.builder.build_call(
            self.module.get_function("thaw_json_host_from_borrowed_handle").unwrap(),
            &[handle.into()], "live_json_host",
        ).map_err(|error| error.to_string())?.try_as_basic_value().basic()
            .ok_or("live JSON host conversion returned no value")?;
        // The Host lease retained its own reference. Discard the temporary
        // handle even when the host conversion reported an error.
        self.builder.build_call(self.module.get_function("thaw_js_release_handle").unwrap(),
            &[handle.into()], "release_live_json_builder_handle")
            .map_err(|error| error.to_string())?;
        self.compile_check_json_host_error(json, Some("thaw_json_destroy"))
    }

    fn compile_retain_dynamic_json(
        &mut self,
        args: &[HirExpr],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let [value] = args else {
            return Err("retainDynamicJson expects exactly one argument".into());
        };
        self.uses_quickjs = true;
        self.uses_quickjs_handles = true;
        let compiling_dynamic_arguments = self.compiling_quickjs_dynamic_arguments;
        self.compiling_quickjs_dynamic_arguments = true;
        // A caller may reach this intrinsic with a bare native object
        // literal (`retainDynamicJson({})`, an internal-builtin call some
        // tests exercise directly) rather than the already-`Json`-coerced
        // argument the compiler's own emit sites pass -- compile such a
        // literal through the JSON builder so the runtime sees a real
        // shared `Json` value, not a fixed-layout native object.
        let value = match value {
            HirExpr::ObjectLit(fields) => self.compile_json_object_lit(fields, &HirType::Json),
            _ => self.compile_expr(value),
        };
        self.compiling_quickjs_dynamic_arguments = compiling_dynamic_arguments;
        let value = value?.into_pointer_value();
        let json = self
            .builder
            .build_call(
                self.module.get_function("thaw_json_stringify").unwrap(),
                &[value.into()],
                "retained_dynamic_json",
            )
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .unwrap();
        let json = self.compile_check_json_stringify_error(json)?;
        let result = self
            .builder
            .build_call(
                self.module.get_function("thaw_js_retain_json_result").unwrap(),
                &[json.into()],
                "retain_dynamic_json_result",
            )
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .unwrap()
            .into_struct_value();
        let retained = self
            .builder
            .build_extract_value(result, 0, "retained_dynamic_handle")
            .map_err(|error| error.to_string())?;
        let error = self
            .builder
            .build_extract_value(result, 1, "retain_dynamic_json_error")
            .map_err(|error| error.to_string())?;
        self.builder
            .build_call(
                self.module.get_function("thaw_cstring_destroy").unwrap(),
                &[json.into()],
                "destroy_retained_dynamic_json",
            )
            .map_err(|error| error.to_string())?;
        self.builder
            .build_store(self.pending_exception().as_pointer_value(), error)
            .map_err(|error| error.to_string())?;
        self.clear_pending_native_text()?;
        self.mark_pending_native_text(error)?;
        self.branch_on_pending_exception()?;
        Ok(retained)
    }

    fn compile_set_dynamic_property_json(
        &mut self,
        args: &[HirExpr],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let [receiver, name, value] = args else {
            return Err("setDynamicPropertyJson expects receiver, name, and value".into());
        };
        self.uses_quickjs = true;
        self.uses_quickjs_handles = true;
        let receiver = self.compile_expr(receiver)?.into_int_value();
        let name = self.compile_expr(name)?.into_pointer_value();
        let value = self.compile_expr(value)?.into_pointer_value();
        let array = self
            .builder
            .build_call(
                self.module.get_function("thaw_json_array_new").unwrap(),
                &[],
                "dynamic_set_args",
            )
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .unwrap()
            .into_pointer_value();
        self.builder
            .build_call(
                self.module
                    .get_function("thaw_json_array_push_json")
                    .unwrap(),
                &[array.into(), value.into()],
                "push_dynamic_set_value",
            )
            .map_err(|error| error.to_string())?;
        let args_json = self
            .builder
            .build_call(
                self.module.get_function("thaw_json_graph_encode").unwrap(),
                &[array.into()],
                "dynamic_set_args_json",
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
                &[receiver.into(), name.into(), args_json.into()],
                "dynamic_property_json_set",
            )
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .unwrap()
            .into_struct_value();
        let returned = self
            .builder
            .build_extract_value(result, 0, "dynamic_property_json_set_value")
            .map_err(|error| error.to_string())?;
        self.builder
            .build_call(
                self.module.get_function("thaw_cstring_destroy").unwrap(),
                &[returned.into()],
                "destroy_dynamic_set_result",
            )
            .map_err(|error| error.to_string())?;
        let error = self
            .builder
            .build_extract_value(result, 1, "dynamic_property_json_set_error")
            .map_err(|error| error.to_string())?;
        self.builder.build_call(
            self.module.get_function("thaw_cstring_destroy").unwrap(),
            &[args_json.into()], "destroy_dynamic_set_args_string",
        ).map_err(|error| error.to_string())?;
        self.builder.build_call(
            self.module.get_function("thaw_json_destroy").unwrap(),
            &[array.into()], "destroy_dynamic_set_args_json",
        ).map_err(|error| error.to_string())?;
        self.builder
            .build_store(self.pending_exception().as_pointer_value(), error)
            .map_err(|error| error.to_string())?;
        self.clear_pending_native_text()?;
        self.mark_pending_native_text(error)?;
        self.branch_on_pending_exception()?;
        Ok(self.context.bool_type().const_int(1, false).into())
    }

    fn compile_lookup_native_object(
        &mut self,
        args: &[HirExpr],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let [source, HirExpr::Lit(HirLit::Str(layout))] = args else {
            return Err("native object lookup expects object and static layout".into());
        };
        self.uses_quickjs = true;
        self.uses_quickjs_handles = true;
        let source = self.compile_expr(source)?.into_pointer_value();
        let layout = self.builder.build_global_string_ptr(layout, "cached_native_object_layout")
            .map_err(|error| error.to_string())?;
        let result = self.builder.build_call(
            self.module.get_function("thaw_js_lookup_native_object").unwrap(),
            &[source.into(), layout.as_pointer_value().into()], "lookup_native_object",
        ).map_err(|error| error.to_string())?.try_as_basic_value().basic()
            .ok_or("native object lookup returned no result")?.into_struct_value();
        let handle = self.builder.build_extract_value(result, 0, "cached_native_object_handle")
            .map_err(|error| error.to_string())?.into_int_value();
        let error = self.builder.build_extract_value(result, 1, "cached_native_object_error")
            .map_err(|error| error.to_string())?;
        self.builder.build_store(self.pending_exception().as_pointer_value(), error)
            .map_err(|error| error.to_string())?;
        self.clear_pending_native_text()?;
        self.mark_pending_native_text(error)?;
        self.branch_on_pending_exception()?;
        let present = self.builder.build_int_compare(IntPredicate::NE, handle,
            self.context.i64_type().const_zero(), "cached_native_object_present")
            .map_err(|error| error.to_string())?;
        let ty = self.basic_type(&HirType::Optional(Box::new(HirType::JsValue)))?
            .into_struct_type();
        let tagged = self.builder.build_insert_value(ty.get_undef(), present, 0,
            "cached_native_object_tag")
            .map_err(|error| error.to_string())?.into_struct_value();
        self.builder.build_insert_value(tagged, handle, 1, "cached_native_object_value")
            .map(Into::into).map_err(|error| error.to_string())
    }

    fn compile_require_native_owner(
        &mut self, args: &[HirExpr],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let [owner] = args else {
            return Err("native owner check expects one object".into());
        };
        let owner = self.compile_expr(owner)?.into_pointer_value();
        let valid = self.builder.build_call(
            self.module.get_function("thaw_arena_contains_allocation").unwrap(),
            &[owner.into()], "native_owner_allocation",
        ).map_err(|error| error.to_string())?.try_as_basic_value().basic()
            .ok_or("native owner check returned no value")?.into_int_value();
        let missing = self.builder.build_int_compare(
            inkwell::IntPredicate::EQ, valid, self.context.i8_type().const_zero(),
            "native_owner_missing",
        ).map_err(|error| error.to_string())?;
        let function = self.current_function();
        let error_block = self.context.append_basic_block(function, "native_owner_error");
        let continuation = self.context.append_basic_block(function, "native_owner_ready");
        self.builder.build_conditional_branch(missing, error_block, continuation)
            .map_err(|error| error.to_string())?;
        self.builder.position_at_end(error_block);
        self.compile_throw_builtin_error("TypeError", "Live native object requires arena-owned storage")?;
        self.builder.position_at_end(continuation);
        Ok(self.context.i8_type().const_zero().into())
    }

    fn compile_register_native_object_layout(
        &mut self, args: &[HirExpr],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let [owner, HirExpr::Lit(HirLit::Bool(required))] = args else {
            return Err("native layout registration expects owner and required flag".into());
        };
        let Some(HirType::Object(fields)) = self.expr_hir_type(owner) else {
            return Err("native layout registration requires a typed object".into());
        };
        let mut layout = String::new();
        for (index, (name, _)) in fields.iter().enumerate() {
            layout.push_str(&format!("{}:", object_field_offset(&fields, index)?));
            for byte in name.as_bytes() {
                layout.push_str(&format!("{byte:02x}"));
            }
            layout.push(';');
        }
        let descriptor = thaw_hir::native_object_layout_token(&fields);
        let owner = self.compile_expr(owner)?.into_pointer_value();
        let layout = self.builder.build_global_string_ptr(&layout, "native_field_offsets")
            .map_err(|error| error.to_string())?;
        let descriptor = self.builder.build_global_string_ptr(&descriptor, "native_field_descriptor")
            .map_err(|error| error.to_string())?;
        let accepted = self.builder.build_call(
            self.module.get_function("thaw_object_register_field_offsets").unwrap(),
            &[owner.into(), layout.as_pointer_value().into(), descriptor.as_pointer_value().into()],
            "register_native_field_offsets",
        ).map_err(|error| error.to_string())?.try_as_basic_value().basic()
            .ok_or("native layout registration returned no value")?.into_int_value();
        if *required {
            let rejected = self.builder.build_not(accepted, "native_layout_rejected")
                .map_err(|error| error.to_string())?;
            let function = self.current_function();
            let invalid = self.context.append_basic_block(function, "native_layout_invalid");
            let ready = self.context.append_basic_block(function, "native_layout_ready");
            self.builder.build_conditional_branch(rejected, invalid, ready)
                .map_err(|error| error.to_string())?;
            self.builder.position_at_end(invalid);
            self.compile_throw_builtin_error("TypeError", "Nonprefix native object requires arena-owned storage")?;
            self.builder.position_at_end(ready);
        }
        Ok(accepted.into())
    }

    fn compile_register_native_object_projector(
        &mut self, args: &[HirExpr],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let [owner, HirExpr::Lit(HirLit::Str(layout)), factory] = args else {
            return Err("native projector registration expects owner, layout, and factory".into());
        };
        let owner = self.compile_expr(owner)?.into_pointer_value();
        if !self.projects_native_objects {
            // The full typed factory is never emitted into native-only
            // programs, even though HIR records an erasure site.
            return Ok(self.context.bool_type().const_int(1, false).into());
        }
        let layout = self.builder.build_global_string_ptr(layout, "native_projector_layout")
            .map_err(|error| error.to_string())?;
        let factory = self.compile_expr(factory)?.into_pointer_value();
        let accepted = self.builder.build_call(
            self.module.get_function("thaw_object_register_projector").unwrap(),
            &[owner.into(), layout.as_pointer_value().into(), factory.into()],
            "register_native_projector",
        ).map_err(|error| error.to_string())?.try_as_basic_value().basic()
            .ok_or("native projector registration returned no value")?.into_int_value();
        let failed = self.builder.build_not(accepted, "native_projector_failed")
            .map_err(|error| error.to_string())?;
        let function = self.current_function();
        let error_block = self.context.append_basic_block(function, "native_projector_error");
        let continuation = self.context.append_basic_block(function, "native_projector_ready");
        self.builder.build_conditional_branch(failed, error_block, continuation)
            .map_err(|error| error.to_string())?;
        self.builder.position_at_end(error_block);
        self.compile_throw_builtin_error("TypeError", "Incompatible native object projection")?;
        self.builder.position_at_end(continuation);
        Ok(accepted.into())
    }

    fn compile_release_native_projection_callbacks(
        &mut self, args: &[HirExpr],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let [callbacks] = args else {
            return Err("native projection cleanup expects its callback array".into());
        };
        let callbacks = self.compile_expr(callbacks)?.into_pointer_value();
        let callbacks = self.compile_array_data(callbacks)?;
        self.builder.build_call(
            self.module.get_function("thaw_js_release_native_handle_array").unwrap(),
            &[callbacks.into()], "release_unbuilt_native_projection",
        ).map_err(|error| error.to_string())?;
        Ok(self.context.i8_type().const_zero().into())
    }

    fn compile_lookup_native_projector(
        &mut self, args: &[HirExpr],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let [owner, HirExpr::Lit(HirLit::Str(layout))] = args else {
            return Err("native projector lookup expects owner and static layout".into());
        };
        let owner = self.compile_expr(owner)?.into_pointer_value();
        let layout = self.builder.build_global_string_ptr(layout, "native_projector_query")
            .map_err(|error| error.to_string())?;
        let projector = self.builder.build_call(
            self.module.get_function("thaw_object_projector").unwrap(),
            &[owner.into(), layout.as_pointer_value().into()], "lookup_native_projector",
        ).map_err(|error| error.to_string())?.try_as_basic_value().basic()
            .ok_or("native projector lookup returned no value")?.into_pointer_value();
        let present = self.builder.build_is_not_null(projector, "native_projector_present")
            .map_err(|error| error.to_string())?;
        let ty = self.basic_type(&HirType::Optional(Box::new(HirType::Function(
            Vec::new(), Box::new(HirType::JsValue)))))?.into_struct_type();
        let tagged = self.builder.build_insert_value(ty.get_undef(), present, 0,
            "native_projector_tag").map_err(|error| error.to_string())?.into_struct_value();
        self.builder.build_insert_value(tagged, projector, 1, "native_projector_value")
            .map(Into::into).map_err(|error| error.to_string())
    }

    fn compile_intern_native_object(
        &mut self,
        args: &[HirExpr],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let [source, HirExpr::Lit(HirLit::Str(layout)), candidate] = args else {
            return Err("native object interning expects object, static layout, and wrapper".into());
        };
        self.uses_quickjs = true;
        self.uses_quickjs_handles = true;
        let source = self.compile_expr(source)?.into_pointer_value();
        let layout = self.builder.build_global_string_ptr(layout, "native_object_layout")
            .map_err(|error| error.to_string())?;
        let candidate = self.compile_expr(candidate)?.into_int_value();
        let result = self.builder.build_call(
            self.module.get_function("thaw_js_intern_native_object").unwrap(),
            &[source.into(), layout.as_pointer_value().into(), candidate.into()],
            "intern_native_object",
        ).map_err(|error| error.to_string())?.try_as_basic_value().basic()
            .ok_or("native object interning returned no value")?.into_struct_value();
        let handle = self.builder.build_extract_value(result, 0, "interned_native_object_handle")
            .map_err(|error| error.to_string())?;
        let error = self.builder.build_extract_value(result, 1, "interned_native_object_error")
            .map_err(|error| error.to_string())?;
        self.builder.build_store(self.pending_exception().as_pointer_value(), error)
            .map_err(|error| error.to_string())?;
        self.clear_pending_native_text()?;
        self.mark_pending_native_text(error)?;
        self.branch_on_pending_exception()?;
        Ok(handle)
    }
}
