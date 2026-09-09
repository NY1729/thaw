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
            "callDynamicValue" => self.compile_call_dynamic_value(args),
            "callNativeAddonValue" => self.compile_call_native_addon_value(args),
            "callDynamicValueHandle" => self.compile_call_dynamic_value_handle(args),
            "callDynamicValueWithValue" => self.compile_call_dynamic_value_with_value(args),
            "releaseDynamicValue" => self.compile_release_dynamic_value(args),
            "getDynamicProperty" => {
                self.compile_dynamic_handle_operation("thaw_js_get_property_result", args)
            }
            "setDynamicProperty" => self.compile_set_dynamic_property(args),
            "setDynamicPropertyJson" => self.compile_set_dynamic_property_json(args),
            "callDynamicMethod" => self.compile_call_dynamic_method(args),
            "callDynamicMethodHandle" => self.compile_call_dynamic_method_handle(args, false),
            "callDynamicMethodHandleRaw" => self.compile_call_dynamic_method_handle(args, true),
            "readDynamicValue" => self.compile_read_dynamic_value(args),
            "resolveDynamicValue" => self.compile_dynamic_handle_operation(
                "thaw_js_resolve_handle_handle_result",
                args,
            ),
            "callDynamicValueMixed" => self.compile_call_dynamic_value_mixed(args),
            "constructDynamicValue" => self.compile_construct_dynamic_value(args),
            "loadNativeAddon" => self.compile_load_native_addon(args),
            "loadNativeAddonEmbedded" => self.compile_load_embedded_native_addon(args),
            "loadNativeSharedLibraryEmbedded" => self.compile_load_embedded_native_dependency(args),
            "loadNativeSharedLibrary" => self.compile_load_native_dependency(args),
            "callNativeAddon" => self.compile_call_native_addon(args),
            "callNativeAddonWithCallback" => self.compile_call_native_addon_with_callback(args),
            "pollNativeAddonEvents" => self.compile_poll_native_addon_events(args),
            "registerNativeCallback" => self.compile_register_native_callback(args),
            _ => return None,
        })
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
                self.module.get_function("thaw_json_stringify").unwrap(),
                &[array.into()],
                "dynamic_set_args_json",
            )
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .unwrap();
        let result = self
            .builder
            .build_call(
                self.module
                    .get_function("thaw_js_set_property_json_result")
                    .unwrap(),
                &[receiver.into(), name.into(), args_json.into()],
                "dynamic_property_json_set",
            )
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .unwrap()
            .into_struct_value();
        let error = self
            .builder
            .build_extract_value(result, 1, "dynamic_property_json_set_error")
            .map_err(|error| error.to_string())?;
        self.builder
            .build_store(self.pending_exception().as_pointer_value(), error)
            .map_err(|error| error.to_string())?;
        self.branch_on_pending_exception()?;
        Ok(self.context.bool_type().const_int(1, false).into())
    }
}
