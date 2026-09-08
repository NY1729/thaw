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
            "callDynamicMethod" => self.compile_call_dynamic_method(args),
            "callDynamicMethodHandle" => self.compile_call_dynamic_method_handle(args, false),
            "callDynamicMethodHandleRaw" => self.compile_call_dynamic_method_handle(args, true),
            "readDynamicValue" => self.compile_read_dynamic_value(args),
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
}
