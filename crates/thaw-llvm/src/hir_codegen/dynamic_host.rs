fn napi_constructor_export_name(symbol: &str) -> Option<&str> {
    let constructor = symbol.strip_prefix("$new$")?;
    Some(
        constructor
            .rsplit_once("$arity")
            .map_or(constructor, |(name, _)| name),
    )
}

fn dynamic_json_collection_element_supported(ty: &HirType) -> bool {
    match ty {
        HirType::F64 | HirType::Str | HirType::Bool | HirType::Json => true,
        HirType::Array(element) => dynamic_json_collection_element_supported(element),
        HirType::Tuple(elements) => elements
            .iter()
            .all(dynamic_json_collection_element_supported),
        HirType::Object(fields) => fields
            .iter()
            .all(|(_, field)| dynamic_json_collection_element_supported(field)),
        _ => false,
    }
}

impl<'ctx> HirCompiler<'ctx> {
    /// `loadScript(source): boolean`, via thaw-quickjs's `thaw_js_load`.
    /// Same `i8` -> `i1` conversion as `compile_json_as_bool` and for the
    /// same reason (the extern function avoids relying on `bool`'s C ABI
    /// shape).
    fn compile_load_script(&mut self, args: &[HirExpr]) -> Result<BasicValueEnum<'ctx>, String> {
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
        if !(1..=2).contains(&args.len()) {
            return Err("loadNativeAddon expects a path and optional root export name".to_string());
        }
        let path = self.compile_expr(&args[0])?;
        let mut call_args = vec![path.into()];
        let symbol = if args.len() == 2 {
            call_args.push(self.compile_expr(&args[1])?.into());
            "thaw_napi_load_named"
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
        if args.len() != 2 {
            return Err(
                "loadNativeAddonEmbedded expects addon bytes and a root export name".into(),
            );
        }
        let bytes = self.compile_expr(&args[0])?;
        let root = self.compile_expr(&args[1])?;
        let function = self
            .module
            .get_function("thaw_napi_load_embedded_hex")
            .unwrap();
        let loaded = self
            .builder
            .build_call(
                function,
                &[bytes.into(), root.into()],
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

    /// `callDynamic(name, args): Json` -- the QuickJS-NG fallback path
    /// (docs/design/bridge.md section 7). Composes thaw-std's
    /// `thaw_json_stringify`/`thaw_json_parse` with thaw-quickjs's
    /// `thaw_js_call` so a `Json` value flows in and out without this
    /// module needing to know thaw-quickjs's internals (or vice versa).
    fn compile_call_dynamic(&mut self, args: &[HirExpr]) -> Result<BasicValueEnum<'ctx>, String> {
        self.compile_json_backend_call(args, "thaw_js_call_result", "callDynamic")
    }

    fn compile_get_dynamic_value(
        &mut self,
        args: &[HirExpr],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        self.uses_quickjs_handles = true;
        self.compile_single_arg_call("thaw_js_get_global", args, "getDynamicValue")
    }

    fn compile_call_dynamic_value(
        &mut self,
        args: &[HirExpr],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        self.compile_json_backend_call(args, "thaw_js_call_handle_result", "callDynamicValue")
    }

    fn compile_call_dynamic_value_handle(
        &mut self,
        args: &[HirExpr],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        self.uses_quickjs_handles = true;
        let [handle, call_args] = args else {
            return Err("callDynamicValueHandle expects exactly two arguments".into());
        };
        let handle = self.compile_expr(handle)?;
        let call_args = self.compile_expr(call_args)?;
        let args_json = self
            .builder
            .build_call(
                self.module.get_function("thaw_json_stringify").unwrap(),
                &[call_args.into()],
                "dynamic_handle_args",
            )
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .unwrap();
        let result = self
            .builder
            .build_call(
                self.module
                    .get_function("thaw_js_call_handle_handle_result")
                    .unwrap(),
                &[handle.into(), args_json.into()],
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
            .build_store(self.pending_exception().as_pointer_value(), error)
            .map_err(|error| error.to_string())?;
        self.branch_on_pending_exception()?;
        Ok(value)
    }

    fn compile_call_dynamic_value_with_value(
        &mut self,
        args: &[HirExpr],
    ) -> Result<BasicValueEnum<'ctx>, String> {
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
                    .get_function("thaw_js_call_handle_value_result")
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
        self.branch_on_pending_exception()?;
        self.builder
            .build_call(
                self.module.get_function("thaw_json_parse").unwrap(),
                &[value.into()],
                "dynamic_value_argument_parsed",
            )
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .ok_or_else(|| "thaw_json_parse returned no value".into())
    }

    fn compile_release_dynamic_value(
        &mut self,
        args: &[HirExpr],
    ) -> Result<BasicValueEnum<'ctx>, String> {
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
        self.branch_on_pending_exception()?;
        Ok(value)
    }

    fn compile_call_dynamic_method(
        &mut self,
        args: &[HirExpr],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        self.uses_quickjs_handles = true;
        let [handle, name, call_args] = args else {
            return Err("callDynamicMethod expects exactly three arguments".into());
        };
        let handle = self.compile_expr(handle)?;
        let name = self.compile_expr(name)?;
        let call_args = self.compile_expr(call_args)?;
        let args_json = self
            .builder
            .build_call(
                self.module.get_function("thaw_json_stringify").unwrap(),
                &[call_args.into()],
                "dynamic_method_args",
            )
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .unwrap();
        let result = self
            .builder
            .build_call(
                self.module
                    .get_function("thaw_js_call_method_result")
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
            .build_store(self.pending_exception().as_pointer_value(), error)
            .map_err(|error| error.to_string())?;
        self.branch_on_pending_exception()?;
        self.builder
            .build_call(
                self.module.get_function("thaw_json_parse").unwrap(),
                &[value.into()],
                "dynamic_method_parsed",
            )
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .ok_or_else(|| "thaw_json_parse returned no value".into())
    }

    fn compile_read_dynamic_value(
        &mut self,
        args: &[HirExpr],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        self.uses_quickjs_handles = true;
        let [handle] = args else {
            return Err("readDynamicValue expects exactly one argument".into());
        };
        let handle = self.compile_expr(handle)?;
        let result = self
            .builder
            .build_call(
                self.module
                    .get_function("thaw_js_resolve_handle_result")
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
        self.branch_on_pending_exception()?;
        self.builder
            .build_call(
                self.module.get_function("thaw_json_parse").unwrap(),
                &[value.into()],
                "read_dynamic_parsed",
            )
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .ok_or_else(|| "thaw_json_parse returned no value".into())
    }

    fn compile_call_dynamic_value_mixed(
        &mut self,
        args: &[HirExpr],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        self.uses_quickjs_handles = true;
        let [callable, json_args, handles] = args else {
            return Err(
                "callDynamicValueMixed expects callable, JSON arguments, and handle array".into(),
            );
        };
        let callable = self.compile_expr(callable)?;
        let json_args = self.compile_expr(json_args)?;
        let handles = self.compile_expr(handles)?;
        let text = self
            .builder
            .build_call(
                self.module.get_function("thaw_json_stringify").unwrap(),
                &[json_args.into()],
                "mixed_args_json",
            )
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .unwrap();
        let result = self
            .builder
            .build_call(
                self.module
                    .get_function("thaw_js_call_handle_mixed_result")
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
        self.builder
            .build_store(self.pending_exception().as_pointer_value(), error)
            .map_err(|error| error.to_string())?;
        self.branch_on_pending_exception()?;
        self.builder
            .build_call(
                self.module.get_function("thaw_json_parse").unwrap(),
                &[value.into()],
                "mixed_dynamic_parsed",
            )
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .ok_or_else(|| "thaw_json_parse returned no value".into())
    }

    fn compile_construct_dynamic_value(
        &mut self,
        args: &[HirExpr],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        self.uses_quickjs_handles = true;
        let [constructor, json_args] = args else {
            return Err("constructDynamicValue expects constructor and JSON arguments".into());
        };
        let constructor = self.compile_expr(constructor)?;
        let json_args = self.compile_expr(json_args)?;
        let text = self
            .builder
            .build_call(
                self.module.get_function("thaw_json_stringify").unwrap(),
                &[json_args.into()],
                "constructor_args_json",
            )
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .unwrap();
        let result = self
            .builder
            .build_call(
                self.module
                    .get_function("thaw_js_construct_handle_result")
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
            .build_store(self.pending_exception().as_pointer_value(), error)
            .map_err(|error| error.to_string())?;
        self.branch_on_pending_exception()?;
        Ok(value)
    }

    fn compile_typed_dynamic_call(
        &mut self,
        signature: &DynamicSignature,
        args: &[HirExpr],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        if signature.backend == DynamicBackend::Napi
            && (signature.symbol.starts_with("$getter$")
                || signature.symbol.starts_with("$staticgetter$"))
        {
            return self.compile_typed_napi_getter(signature, args);
        }
        if signature.backend == DynamicBackend::Napi
            && (signature.symbol.starts_with("$setter$")
                || signature.symbol.starts_with("$staticsetter$"))
        {
            return self.compile_typed_napi_setter(signature, args);
        }
        if signature.backend == DynamicBackend::Napi
            && (signature.symbol.starts_with("$method$")
                || signature.symbol.starts_with("$methodvoid$")
                || signature.symbol.starts_with("$staticmethod$")
                || signature.symbol.starts_with("$staticmethodvoid$"))
        {
            return self.compile_typed_napi_method(signature, args);
        }
        let array = self
            .builder
            .build_call(
                self.module.get_function("thaw_json_array_new").unwrap(),
                &[],
                "dynamic_args",
            )
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .unwrap();
        for (index, (arg, ty)) in args.iter().zip(&signature.params).enumerate() {
            let mut value = self.compile_expr(arg)?;
            let push = match ty {
                HirType::F64 => "thaw_json_array_push_number",
                HirType::Str => "thaw_json_array_push_string",
                HirType::Bool => {
                    let widened = self
                        .builder
                        .build_int_z_extend(
                            value.into_int_value(),
                            self.context.i8_type(),
                            &format!("dynamic_bool_{index}"),
                        )
                        .map_err(|error| error.to_string())?;
                    value = widened.into();
                    "thaw_json_array_push_bool"
                }
                HirType::Json => "thaw_json_array_push_json",
                HirType::Array(element) if **element == HirType::F64 => {
                    value = self
                        .builder
                        .build_call(
                            self.module
                                .get_function("thaw_json_from_number_array")
                                .unwrap(),
                            &[value.into()],
                            "marshal_number_array",
                        )
                        .map_err(|error| error.to_string())?
                        .try_as_basic_value()
                        .basic()
                        .unwrap();
                    "thaw_json_array_push_json"
                }
                HirType::Array(element) if dynamic_json_collection_element_supported(element) =>
                {
                    value = self.compile_native_array_to_json(
                        value.into_pointer_value(),
                        element,
                    )?;
                    "thaw_json_array_push_json"
                }
                HirType::Tuple(elements) => {
                    value = self.compile_native_tuple_to_json(
                        value.into_pointer_value(),
                        elements,
                    )?;
                    "thaw_json_array_push_json"
                }
                HirType::Object(_) => {
                    value = self.compile_native_object_to_json(value.into_pointer_value(), ty)?;
                    "thaw_json_array_push_json"
                }
                other => {
                    return Err(format!(
                        "typed dynamic argument {} does not support {other:?} yet",
                        index + 1
                    ))
                }
            };
            self.builder
                .build_call(
                    self.module.get_function(push).unwrap(),
                    &[array.into(), value.into()],
                    &format!("marshal_dynamic_arg_{index}"),
                )
                .map_err(|error| error.to_string())?;
        }
        let name = self
            .builder
            .build_global_string_ptr(&signature.symbol, "dynamic_symbol")
            .map_err(|error| error.to_string())?;
        if signature.backend == DynamicBackend::Napi && signature.ret == HirType::JsValue {
            let constructor_name = napi_constructor_export_name(&signature.symbol).ok_or_else(|| {
                "N-API JsValue return is reserved for class constructors".to_string()
            })?;
            let constructor_name = self
                .builder
                .build_global_string_ptr(constructor_name, "napi_constructor_name")
                .map_err(|error| error.to_string())?;
            let constructor = self
                .builder
                .build_call(
                    self.module.get_function("thaw_napi_get_export").unwrap(),
                    &[constructor_name.as_pointer_value().into()],
                    "napi_constructor",
                )
                .map_err(|error| error.to_string())?
                .try_as_basic_value()
                .basic()
                .unwrap();
            let args_json = self
                .builder
                .build_call(
                    self.module.get_function("thaw_json_stringify").unwrap(),
                    &[array.into()],
                    "napi_constructor_args",
                )
                .map_err(|error| error.to_string())?
                .try_as_basic_value()
                .basic()
                .unwrap();
            let result = self
                .builder
                .build_call(
                    self.module
                        .get_function("thaw_napi_construct_handle_result")
                        .unwrap(),
                    &[constructor.into(), args_json.into()],
                    "napi_construct_result",
                )
                .map_err(|error| error.to_string())?
                .try_as_basic_value()
                .basic()
                .unwrap()
                .into_struct_value();
            let value = self
                .builder
                .build_extract_value(result, 0, "napi_constructed_value")
                .map_err(|error| error.to_string())?;
            let error = self
                .builder
                .build_extract_value(result, 1, "napi_construct_error")
                .map_err(|error| error.to_string())?;
            self.builder
                .build_store(self.pending_exception().as_pointer_value(), error)
                .map_err(|error| error.to_string())?;
            self.branch_on_pending_exception()?;
            return Ok(value);
        }
        if signature.backend == DynamicBackend::QuickJs && signature.ret == HirType::JsValue {
            self.uses_quickjs_handles = true;
            let callable = self
                .builder
                .build_call(
                    self.module.get_function("thaw_js_get_global").unwrap(),
                    &[name.as_pointer_value().into()],
                    "typed_dynamic_callable",
                )
                .map_err(|error| error.to_string())?
                .try_as_basic_value()
                .basic()
                .unwrap();
            let args_json = self
                .builder
                .build_call(
                    self.module.get_function("thaw_json_stringify").unwrap(),
                    &[array.into()],
                    "typed_dynamic_callable_args",
                )
                .map_err(|error| error.to_string())?
                .try_as_basic_value()
                .basic()
                .unwrap();
            let result = self
                .builder
                .build_call(
                    self.module
                        .get_function("thaw_js_call_handle_handle_result")
                        .unwrap(),
                    &[callable.into(), args_json.into()],
                    "typed_dynamic_callable_result",
                )
                .map_err(|error| error.to_string())?
                .try_as_basic_value()
                .basic()
                .unwrap()
                .into_struct_value();
            let value = self
                .builder
                .build_extract_value(result, 0, "typed_dynamic_callable_value")
                .map_err(|error| error.to_string())?;
            let error = self
                .builder
                .build_extract_value(result, 1, "typed_dynamic_callable_error")
                .map_err(|error| error.to_string())?;
            self.builder
                .build_store(self.pending_exception().as_pointer_value(), error)
                .map_err(|error| error.to_string())?;
            self.branch_on_pending_exception()?;
            return Ok(value);
        }
        let backend = match signature.backend {
            DynamicBackend::QuickJs => "thaw_js_call_result",
            DynamicBackend::Napi => "thaw_napi_call_result",
        };
        let json =
            self.compile_json_backend_values(name.as_pointer_value().into(), array, backend)?;
        match signature.ret {
            HirType::Json => Ok(json),
            HirType::F64 => self.compile_json_as_value(json, "thaw_json_as_number"),
            HirType::Str => self.compile_json_as_value(json, "thaw_json_as_string"),
            HirType::Bool => self.compile_json_as_bool_value(json),
            HirType::Array(ref element) if **element == HirType::F64 => self
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
                .ok_or_else(|| "thaw_json_to_number_array returned no value".to_string()),
            HirType::Array(ref element) if dynamic_json_collection_element_supported(element) => {
                self.compile_json_to_native_array(json, element)
            }
            HirType::Tuple(ref elements) => self.compile_json_to_native_tuple(json, elements),
            HirType::Object(_) => self.compile_json_to_native_object(json, &signature.ret),
            ref other => Err(format!(
                "typed dynamic return does not support {other:?} yet"
            )),
        }
    }

    fn compile_typed_napi_setter(
        &mut self,
        signature: &DynamicSignature,
        args: &[HirExpr],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let is_static = signature.symbol.starts_with("$staticsetter$");
        let (receiver_expr, assigned) = if is_static {
            let [assigned] = args else {
                return Err("typed N-API static setter expects one value".into());
            };
            (None, assigned)
        } else {
            let [receiver, assigned] = args else {
                return Err("typed N-API setter expects a receiver and value".into());
            };
            (Some(receiver), assigned)
        };
        let receiver = if let Some(receiver) = receiver_expr {
            self.compile_expr(receiver)?
        } else {
            let class = signature
                .symbol
                .split('$')
                .nth(2)
                .ok_or("invalid typed N-API static setter symbol")?;
            let class = self
                .builder
                .build_global_string_ptr(class, "napi_static_setter_class_name")
                .map_err(|error| error.to_string())?;
            self.builder
                .build_call(
                    self.module.get_function("thaw_napi_get_export").unwrap(),
                    &[class.as_pointer_value().into()],
                    "napi_static_setter_class",
                )
                .map_err(|error| error.to_string())?
                .try_as_basic_value()
                .basic()
                .unwrap()
        };
        let assigned_type = signature
            .params
            .get(usize::from(!is_static))
            .ok_or("typed N-API setter is missing its value type")?;
        let mut assigned_value = self.compile_expr(assigned)?;
        let array = self
            .builder
            .build_call(
                self.module.get_function("thaw_json_array_new").unwrap(),
                &[],
                "napi_setter_args",
            )
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .unwrap();
        let push = match assigned_type {
            HirType::F64 => "thaw_json_array_push_number",
            HirType::Str => "thaw_json_array_push_string",
            HirType::Bool => {
                assigned_value = self
                    .builder
                    .build_int_z_extend(
                        assigned_value.into_int_value(),
                        self.context.i8_type(),
                        "napi_setter_bool",
                    )
                    .map_err(|error| error.to_string())?
                    .into();
                "thaw_json_array_push_bool"
            }
            HirType::Json => "thaw_json_array_push_json",
            HirType::Array(element) if **element == HirType::F64 => {
                assigned_value = self
                    .builder
                    .build_call(
                        self.module
                            .get_function("thaw_json_from_number_array")
                            .unwrap(),
                        &[assigned_value.into()],
                        "marshal_napi_setter_number_array",
                    )
                    .map_err(|error| error.to_string())?
                    .try_as_basic_value()
                    .basic()
                    .unwrap();
                "thaw_json_array_push_json"
            }
            HirType::Array(element) if dynamic_json_collection_element_supported(element) => {
                assigned_value = self.compile_native_array_to_json(
                    assigned_value.into_pointer_value(),
                    element,
                )?;
                "thaw_json_array_push_json"
            }
            HirType::Tuple(elements) => {
                assigned_value = self.compile_native_tuple_to_json(
                    assigned_value.into_pointer_value(),
                    elements,
                )?;
                "thaw_json_array_push_json"
            }
            HirType::Object(_) => {
                assigned_value = self.compile_native_object_to_json(
                    assigned_value.into_pointer_value(),
                    assigned_type,
                )?;
                "thaw_json_array_push_json"
            }
            other => return Err(format!("N-API setter value does not support {other:?}")),
        };
        self.builder
            .build_call(
                self.module.get_function(push).unwrap(),
                &[array.into(), assigned_value.into()],
                "marshal_napi_setter_value",
            )
            .map_err(|error| error.to_string())?;
        let property = signature
            .symbol
            .split('$')
            .nth(3)
            .ok_or("invalid typed N-API setter symbol")?;
        let property = self
            .builder
            .build_global_string_ptr(property, "napi_setter_name")
            .map_err(|error| error.to_string())?;
        let args_json = self
            .builder
            .build_call(
                self.module.get_function("thaw_json_stringify").unwrap(),
                &[array.into()],
                "napi_setter_args_json",
            )
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .unwrap();
        let result = self
            .builder
            .build_call(
                self.module
                    .get_function("thaw_napi_set_property_result")
                    .unwrap(),
                &[
                    receiver.into(),
                    property.as_pointer_value().into(),
                    args_json.into(),
                ],
                "napi_setter_result",
            )
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .unwrap()
            .into_struct_value();
        let value = self
            .builder
            .build_extract_value(result, 0, "napi_setter_json")
            .map_err(|error| error.to_string())?;
        let error = self
            .builder
            .build_extract_value(result, 1, "napi_setter_error")
            .map_err(|error| error.to_string())?;
        self.builder
            .build_store(self.pending_exception().as_pointer_value(), error)
            .map_err(|error| error.to_string())?;
        self.branch_on_pending_exception()?;
        let json = self
            .builder
            .build_call(
                self.module.get_function("thaw_json_parse").unwrap(),
                &[value.into()],
                "napi_setter_parsed",
            )
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .ok_or_else(|| "thaw_json_parse returned no setter value".to_string())?;
        match signature.ret {
            HirType::Json => Ok(json),
            HirType::F64 => self.compile_json_as_value(json, "thaw_json_as_number"),
            HirType::Str => self.compile_json_as_value(json, "thaw_json_as_string"),
            HirType::Bool => self.compile_json_as_bool_value(json),
            HirType::Array(ref element) if **element == HirType::F64 => self
                .builder
                .build_call(
                    self.module
                        .get_function("thaw_json_to_number_array")
                        .unwrap(),
                    &[json.into()],
                    "napi_setter_number_array_result",
                )
                .map_err(|error| error.to_string())?
                .try_as_basic_value()
                .basic()
                .ok_or_else(|| "thaw_json_to_number_array returned no value".to_string()),
            HirType::Array(ref element) if dynamic_json_collection_element_supported(element) => {
                self.compile_json_to_native_array(json, element)
            }
            HirType::Tuple(ref elements) => self.compile_json_to_native_tuple(json, elements),
            HirType::Object(_) => self.compile_json_to_native_object(json, &signature.ret),
            ref other => Err(format!(
                "typed N-API setter return does not support {other:?} yet"
            )),
        }
    }

    fn compile_typed_napi_getter(
        &mut self,
        signature: &DynamicSignature,
        args: &[HirExpr],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let is_static = signature.symbol.starts_with("$staticgetter$");
        let receiver = if is_static {
            if !args.is_empty() {
                return Err("typed N-API static getter expects no arguments".into());
            }
            let class = signature
                .symbol
                .split('$')
                .nth(2)
                .ok_or("invalid typed N-API static getter symbol")?;
            let class = self
                .builder
                .build_global_string_ptr(class, "napi_static_getter_class_name")
                .map_err(|error| error.to_string())?;
            self.builder
                .build_call(
                    self.module.get_function("thaw_napi_get_export").unwrap(),
                    &[class.as_pointer_value().into()],
                    "napi_static_getter_class",
                )
                .map_err(|error| error.to_string())?
                .try_as_basic_value()
                .basic()
                .unwrap()
        } else {
            let [receiver] = args else {
                return Err("typed N-API getter expects one receiver".into());
            };
            self.compile_expr(receiver)?
        };
        let property = signature
            .symbol
            .split('$')
            .nth(3)
            .ok_or("invalid typed N-API getter symbol")?;
        let property = self
            .builder
            .build_global_string_ptr(property, "napi_getter_name")
            .map_err(|error| error.to_string())?;
        let result = self
            .builder
            .build_call(
                self.module
                    .get_function("thaw_napi_get_property_result")
                    .unwrap(),
                &[receiver.into(), property.as_pointer_value().into()],
                "napi_getter_result",
            )
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .unwrap()
            .into_struct_value();
        let value = self
            .builder
            .build_extract_value(result, 0, "napi_getter_json")
            .map_err(|error| error.to_string())?;
        let error = self
            .builder
            .build_extract_value(result, 1, "napi_getter_error")
            .map_err(|error| error.to_string())?;
        self.builder
            .build_store(self.pending_exception().as_pointer_value(), error)
            .map_err(|error| error.to_string())?;
        self.branch_on_pending_exception()?;
        let json = self
            .builder
            .build_call(
                self.module.get_function("thaw_json_parse").unwrap(),
                &[value.into()],
                "napi_getter_parsed",
            )
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .ok_or_else(|| "thaw_json_parse returned no getter value".to_string())?;
        match signature.ret {
            HirType::Json => Ok(json),
            HirType::F64 => self.compile_json_as_value(json, "thaw_json_as_number"),
            HirType::Str => self.compile_json_as_value(json, "thaw_json_as_string"),
            HirType::Bool => self.compile_json_as_bool_value(json),
            HirType::Array(ref element) if **element == HirType::F64 => self
                .builder
                .build_call(
                    self.module
                        .get_function("thaw_json_to_number_array")
                        .unwrap(),
                    &[json.into()],
                    "napi_getter_number_array_result",
                )
                .map_err(|error| error.to_string())?
                .try_as_basic_value()
                .basic()
                .ok_or_else(|| "thaw_json_to_number_array returned no value".to_string()),
            HirType::Array(ref element) if dynamic_json_collection_element_supported(element) => {
                self.compile_json_to_native_array(json, element)
            }
            HirType::Tuple(ref elements) => self.compile_json_to_native_tuple(json, elements),
            HirType::Object(_) => self.compile_json_to_native_object(json, &signature.ret),
            ref other => Err(format!(
                "typed N-API getter return does not support {other:?} yet"
            )),
        }
    }

    fn compile_typed_napi_method(
        &mut self,
        signature: &DynamicSignature,
        args: &[HirExpr],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let is_static = signature.symbol.starts_with("$staticmethod$")
            || signature.symbol.starts_with("$staticmethodvoid$");
        let (receiver_expr, method_args) = if is_static {
            (None, args)
        } else {
            let [receiver, method_args @ ..] = args else {
                return Err("typed N-API method expects a receiver".into());
            };
            (Some(receiver), method_args)
        };
        let callback = matches!(signature.params.last(), Some(HirType::Function(_, _)))
            .then(|| method_args.last())
            .flatten();
        let marshalled_args = if callback.is_some() {
            &method_args[..method_args.len() - 1]
        } else {
            method_args
        };
        let receiver = if let Some(receiver) = receiver_expr {
            self.compile_expr(receiver)?
        } else {
            let class = signature
                .symbol
                .split('$')
                .nth(2)
                .ok_or("invalid typed N-API static method symbol")?;
            let class = self
                .builder
                .build_global_string_ptr(class, "napi_static_class_name")
                .map_err(|error| error.to_string())?;
            self.builder
                .build_call(
                    self.module.get_function("thaw_napi_get_export").unwrap(),
                    &[class.as_pointer_value().into()],
                    "napi_static_class",
                )
                .map_err(|error| error.to_string())?
                .try_as_basic_value()
                .basic()
                .unwrap()
        };
        let array = self
            .builder
            .build_call(
                self.module.get_function("thaw_json_array_new").unwrap(),
                &[],
                "napi_method_args",
            )
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .unwrap();
        for (index, (argument, ty)) in marshalled_args
            .iter()
            .zip(signature.params.iter().skip(usize::from(!is_static)))
            .enumerate()
        {
            let mut value = self.compile_expr(argument)?;
            let push = match ty {
                HirType::F64 => "thaw_json_array_push_number",
                HirType::Str => "thaw_json_array_push_string",
                HirType::Bool => {
                    value = self
                        .builder
                        .build_int_z_extend(
                            value.into_int_value(),
                            self.context.i8_type(),
                            &format!("napi_method_bool_{index}"),
                        )
                        .map_err(|error| error.to_string())?
                        .into();
                    "thaw_json_array_push_bool"
                }
                HirType::Json => "thaw_json_array_push_json",
                HirType::Array(element) if **element == HirType::F64 => {
                    value = self
                        .builder
                        .build_call(
                            self.module
                                .get_function("thaw_json_from_number_array")
                                .unwrap(),
                            &[value.into()],
                            "marshal_napi_method_number_array",
                        )
                        .map_err(|error| error.to_string())?
                        .try_as_basic_value()
                        .basic()
                        .unwrap();
                    "thaw_json_array_push_json"
                }
                HirType::Array(element) if dynamic_json_collection_element_supported(element) =>
                {
                    value = self.compile_native_array_to_json(
                        value.into_pointer_value(),
                        element,
                    )?;
                    "thaw_json_array_push_json"
                }
                HirType::Tuple(elements) => {
                    value = self.compile_native_tuple_to_json(
                        value.into_pointer_value(),
                        elements,
                    )?;
                    "thaw_json_array_push_json"
                }
                HirType::Object(_) => {
                    value = self.compile_native_object_to_json(value.into_pointer_value(), ty)?;
                    "thaw_json_array_push_json"
                }
                other => return Err(format!("N-API method argument does not support {other:?}")),
            };
            self.builder
                .build_call(
                    self.module.get_function(push).unwrap(),
                    &[array.into(), value.into()],
                    &format!("marshal_napi_method_arg_{index}"),
                )
                .map_err(|error| error.to_string())?;
        }
        let method = signature
            .symbol
            .split('$')
            .nth(3)
            .ok_or("invalid typed N-API method symbol")?;
        let method = self
            .builder
            .build_global_string_ptr(method, "napi_method_name")
            .map_err(|error| error.to_string())?;
        let args_json = self
            .builder
            .build_call(
                self.module.get_function("thaw_json_stringify").unwrap(),
                &[array.into()],
                "napi_method_args_json",
            )
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .unwrap();
        let result = if let Some(callback) = callback {
            self.compile_typed_napi_method_callback(
                receiver,
                method.as_pointer_value(),
                args_json,
                callback,
                signature.symbol.starts_with("$methodvoid$")
                    || signature.symbol.starts_with("$staticmethodvoid$"),
            )?
        } else {
            self.builder
                .build_call(
                    self.module
                        .get_function("thaw_napi_call_method_result")
                        .unwrap(),
                    &[
                        receiver.into(),
                        method.as_pointer_value().into(),
                        args_json.into(),
                    ],
                    "napi_method_result",
                )
                .map_err(|error| error.to_string())?
                .try_as_basic_value()
                .basic()
                .unwrap()
                .into_struct_value()
        };
        let value = self
            .builder
            .build_extract_value(result, 0, "napi_method_json")
            .map_err(|error| error.to_string())?;
        let error = self
            .builder
            .build_extract_value(result, 1, "napi_method_error")
            .map_err(|error| error.to_string())?;
        self.builder
            .build_store(self.pending_exception().as_pointer_value(), error)
            .map_err(|error| error.to_string())?;
        self.branch_on_pending_exception()?;
        let json = self
            .builder
            .build_call(
                self.module.get_function("thaw_json_parse").unwrap(),
                &[value.into()],
                "napi_method_parsed",
            )
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .ok_or_else(|| "thaw_json_parse returned no method value".to_string())?;
        match signature.ret {
            HirType::Json => Ok(json),
            HirType::F64 => self.compile_json_as_value(json, "thaw_json_as_number"),
            HirType::Str => self.compile_json_as_value(json, "thaw_json_as_string"),
            HirType::Bool => self.compile_json_as_bool_value(json),
            HirType::Array(ref element) if **element == HirType::F64 => self
                .builder
                .build_call(
                    self.module
                        .get_function("thaw_json_to_number_array")
                        .unwrap(),
                    &[json.into()],
                    "napi_method_number_array_result",
                )
                .map_err(|error| error.to_string())?
                .try_as_basic_value()
                .basic()
                .ok_or_else(|| "thaw_json_to_number_array returned no value".to_string()),
            HirType::Array(ref element) if dynamic_json_collection_element_supported(element) => {
                self.compile_json_to_native_array(json, element)
            }
            HirType::Tuple(ref elements) => self.compile_json_to_native_tuple(json, elements),
            HirType::Object(_) => self.compile_json_to_native_object(json, &signature.ret),
            ref other => Err(format!(
                "typed N-API method return does not support {other:?} yet"
            )),
        }
    }

    fn compile_typed_napi_method_callback(
        &mut self,
        receiver: BasicValueEnum<'ctx>,
        method: PointerValue<'ctx>,
        args_json: BasicValueEnum<'ctx>,
        callback: &HirExpr,
        discard_result: bool,
    ) -> Result<StructValue<'ctx>, String> {
        let callback_type = match callback {
            HirExpr::Lambda(_, params, ret, _) => HirType::Function(
                params.iter().map(|param| param.ty.clone()).collect(),
                Box::new(ret.clone()),
            ),
            HirExpr::Var(name) => self
                .variable_hir_types
                .get(name)
                .cloned()
                .ok_or_else(|| format!("unknown callback `{name}`"))?,
            _ => return Err("typed N-API method callback must be a function value".into()),
        };
        let HirType::Function(params, ret) = callback_type else {
            return Err("typed N-API method callback must be a function".into());
        };
        if params.len() > 2
            || params.iter().any(|param| *param != HirType::Json)
            || !matches!(*ret, HirType::Json | HirType::Void)
        {
            return Err(
                "native addon method callback must take zero to two Json arguments and return Json or void"
                    .into(),
            );
        }
        let closure = self.compile_expr(callback)?.into_pointer_value();
        let callback_name = format!("__thaw_napi_method_callback_{}", self.next_lambda);
        self.next_lambda += 1;
        let ptr_type = self.context.ptr_type(AddressSpace::default());
        let adapter_type = self
            .context
            .void_type()
            .fn_type(&[ptr_type.into(), ptr_type.into(), ptr_type.into()], false);
        let adapter =
            self.module
                .add_function(&callback_name, adapter_type, Some(Linkage::Internal));
        let return_block = self.builder.get_insert_block().unwrap();
        let adapter_entry = self.context.append_basic_block(adapter, "entry");
        self.builder.position_at_end(adapter_entry);
        let context = adapter.get_nth_param(0).unwrap().into_pointer_value();
        let error_string = adapter.get_nth_param(1).unwrap();
        let result_string = adapter.get_nth_param(2).unwrap();
        let parse = self.module.get_function("thaw_json_parse").unwrap();
        let error_json = self
            .builder
            .build_call(parse, &[error_string.into()], "method_callback_error")
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .unwrap();
        let result_json = self
            .builder
            .build_call(parse, &[result_string.into()], "method_callback_result")
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .unwrap();
        let code = self
            .builder
            .build_load(ptr_type, context, "method_callback_code")
            .map_err(|error| error.to_string())?
            .into_pointer_value();
        let closure_type = self.function_type(&params, &ret)?;
        let mut callback_args = vec![context.into()];
        if !params.is_empty() {
            callback_args.push(error_json.into());
        }
        if params.len() == 2 {
            callback_args.push(result_json.into());
        }
        self.builder
            .build_indirect_call(
                closure_type,
                code,
                &callback_args,
                "invoke_thaw_method_callback",
            )
            .map_err(|error| error.to_string())?;
        self.builder.build_return(None).map_err(|e| e.to_string())?;
        self.builder.position_at_end(return_block);
        self.builder
            .build_call(
                self.module
                    .get_function("thaw_napi_call_method_with_callback_result")
                    .unwrap(),
                &[
                    receiver.into(),
                    method.into(),
                    args_json.into(),
                    adapter.as_global_value().as_pointer_value().into(),
                    closure.into(),
                    self.context
                        .i8_type()
                        .const_int(discard_result as u64, false)
                        .into(),
                ],
                "napi_method_callback_result",
            )
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .map(|value| value.into_struct_value())
            .ok_or_else(|| "native method callback returned no result".into())
    }

    fn compile_call_native_addon(
        &mut self,
        args: &[HirExpr],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        self.compile_json_backend_call(args, "thaw_napi_call_result", "callNativeAddon")
    }

    fn compile_poll_native_addon_events(
        &mut self,
        args: &[HirExpr],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        if !args.is_empty() {
            return Err("pollNativeAddonEvents expects no arguments".into());
        }
        self.uses_napi = true;
        let count = self
            .builder
            .build_call(
                self.module
                    .get_function("thaw_napi_poll_async_work")
                    .unwrap(),
                &[],
                "poll_napi_events",
            )
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .unwrap()
            .into_int_value();
        self.builder
            .build_unsigned_int_to_float(count, self.context.f64_type(), "napi_event_count")
            .map(BasicValueEnum::FloatValue)
            .map_err(|error| error.to_string())
    }

    fn compile_call_native_addon_with_callback(
        &mut self,
        args: &[HirExpr],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let [name, call_args, callback] = args else {
            return Err(
                "callNativeAddonWithCallback expects a name, Json args, and callback".into(),
            );
        };
        let callback_type = match callback {
            HirExpr::Lambda(_, params, ret, _) => HirType::Function(
                params.iter().map(|param| param.ty.clone()).collect(),
                Box::new(ret.clone()),
            ),
            HirExpr::Var(name) => self
                .variable_hir_types
                .get(name)
                .cloned()
                .ok_or_else(|| format!("unknown callback `{name}`"))?,
            _ => {
                return Err("callNativeAddonWithCallback callback must be a function value".into())
            }
        };
        let HirType::Function(params, ret) = callback_type else {
            return Err("callNativeAddonWithCallback third argument must be a function".into());
        };
        if params != vec![HirType::Json, HirType::Json] || *ret != HirType::Json {
            return Err("native addon callback must have type (Json, Json) => Json".into());
        }

        self.uses_napi = true;
        let name = self.compile_expr(name)?;
        let args_json = self.compile_expr(call_args)?;
        let closure = self.compile_expr(callback)?.into_pointer_value();
        let stringify = self.module.get_function("thaw_json_stringify").unwrap();
        let args_string = self
            .builder
            .build_call(stringify, &[args_json.into()], "napi_callback_args")
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .unwrap();

        let callback_name = format!("__thaw_napi_callback_{}", self.next_lambda);
        self.next_lambda += 1;
        let ptr_type = self.context.ptr_type(AddressSpace::default());
        let adapter_type = self
            .context
            .void_type()
            .fn_type(&[ptr_type.into(), ptr_type.into(), ptr_type.into()], false);
        let adapter =
            self.module
                .add_function(&callback_name, adapter_type, Some(Linkage::Internal));
        let return_block = self.builder.get_insert_block().unwrap();
        let adapter_entry = self.context.append_basic_block(adapter, "entry");
        self.builder.position_at_end(adapter_entry);
        let context = adapter.get_nth_param(0).unwrap().into_pointer_value();
        let error_string = adapter.get_nth_param(1).unwrap();
        let result_string = adapter.get_nth_param(2).unwrap();
        let parse = self.module.get_function("thaw_json_parse").unwrap();
        let error_json = self
            .builder
            .build_call(parse, &[error_string.into()], "callback_error")
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .unwrap();
        let result_json = self
            .builder
            .build_call(parse, &[result_string.into()], "callback_result")
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .unwrap();
        let code = self
            .builder
            .build_load(ptr_type, context, "callback_code")
            .map_err(|error| error.to_string())?
            .into_pointer_value();
        let closure_type = self.function_type(&params, &ret)?;
        self.builder
            .build_indirect_call(
                closure_type,
                code,
                &[context.into(), error_json.into(), result_json.into()],
                "invoke_thaw_callback",
            )
            .map_err(|error| error.to_string())?;
        self.builder.build_return(None).map_err(|e| e.to_string())?;
        self.builder.position_at_end(return_block);

        let call = self
            .builder
            .build_call(
                self.module
                    .get_function("thaw_napi_call_with_callback_result")
                    .unwrap(),
                &[
                    name.into(),
                    args_string.into(),
                    adapter.as_global_value().as_pointer_value().into(),
                    closure.into(),
                ],
                "call_napi_with_callback",
            )
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .unwrap()
            .into_struct_value();
        let value = self
            .builder
            .build_extract_value(call, 0, "napi_callback_value")
            .map_err(|error| error.to_string())?
            .into_pointer_value();
        let error = self
            .builder
            .build_extract_value(call, 1, "napi_callback_error")
            .map_err(|error| error.to_string())?
            .into_pointer_value();
        self.builder
            .build_store(self.pending_exception().as_pointer_value(), error)
            .map_err(|error| error.to_string())?;
        self.branch_on_pending_exception()?;
        self.builder
            .build_call(
                self.module.get_function("thaw_json_parse").unwrap(),
                &[value.into()],
                "napi_callback_queued_result",
            )
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .ok_or_else(|| "thaw_json_parse did not return a value".into())
    }
}
