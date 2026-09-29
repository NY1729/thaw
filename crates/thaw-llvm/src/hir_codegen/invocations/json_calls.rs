impl<'ctx> HirCompiler<'ctx> {
    fn compile_json_named_call(
        &mut self,
        name: &str,
        args: &[HirExpr],
    ) -> Option<Result<BasicValueEnum<'ctx>, String>> {
        if name != "JSON.parse"
            && name != "JSON.stringify"
            && name != "__thaw_array_keys"
            && !name.starts_with("__thaw_json_")
        {
            return None;
        }

        Some(self.compile_json_call(name, args))
    }

    fn compile_json_call(
        &mut self,
        name: &str,
        args: &[HirExpr],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        match name {
            "JSON.parse" => {
                return self.compile_single_arg_call("thaw_json_parse", args, "JSON.parse")
            }
            "JSON.stringify" => {
                // The `_public` variant omits/nulls a nested napi-
                // undefined sentinel the way real `JSON.stringify`
                // treats a real `undefined` -- safe here specifically
                // *because* this is the real, user-facing call, unlike
                // the internal argument/result marshaling paths that
                // still use the plain `thaw_json_stringify` and need the
                // sentinel preserved verbatim.
                return self.compile_single_arg_call(
                    "thaw_json_stringify_public",
                    args,
                    "JSON.stringify",
                )
            }
            "__thaw_json_typeof" => {
                return self.compile_single_arg_call("thaw_json_typeof", args, "JSON typeof")
            }
            "__thaw_json_is_date_shape" => {
                let [value] = args else {
                    return Err("__thaw_json_is_date_shape expects one argument".into());
                };
                let value = self.compile_expr(value)?;
                let function = self.module.get_function("thaw_json_is_date_shape").unwrap();
                let call = self
                    .builder
                    .build_call(function, &[value.into()], "json_is_date_shape_u8")
                    .map_err(|error| error.to_string())?;
                let u8_val = call
                    .try_as_basic_value()
                    .basic()
                    .ok_or("thaw_json_is_date_shape did not return a value")?
                    .into_int_value();
                let zero = self.context.i8_type().const_int(0, false);
                return self
                    .builder
                    .build_int_compare(inkwell::IntPredicate::NE, u8_val, zero, "json_is_date_shape")
                    .map(Into::into)
                    .map_err(|error| error.to_string());
            }
            "__thaw_json_is_buffer_shape" => {
                let [value] = args else {
                    return Err("__thaw_json_is_buffer_shape expects one argument".into());
                };
                let value = self.compile_expr(value)?;
                let function = self.module.get_function("thaw_json_is_buffer_shape").unwrap();
                let call = self
                    .builder
                    .build_call(function, &[value.into()], "json_is_buffer_shape_u8")
                    .map_err(|error| error.to_string())?;
                let u8_val = call
                    .try_as_basic_value()
                    .basic()
                    .ok_or("thaw_json_is_buffer_shape did not return a value")?
                    .into_int_value();
                let zero = self.context.i8_type().const_int(0, false);
                return self
                    .builder
                    .build_int_compare(
                        inkwell::IntPredicate::NE,
                        u8_val,
                        zero,
                        "json_is_buffer_shape",
                    )
                    .map(Into::into)
                    .map_err(|error| error.to_string());
            }
            "__thaw_json_date_timestamp" => {
                return self.compile_single_arg_call(
                    "thaw_json_date_timestamp",
                    args,
                    "JSON date timestamp",
                )
            }
            "__thaw_json_has_wrapper_key" => {
                let [value, key] = args else {
                    return Err("__thaw_json_has_wrapper_key expects two arguments".into());
                };
                let value = self.compile_expr(value)?;
                let key = self.compile_expr(key)?;
                let function = self.module.get_function("thaw_json_has_wrapper_key").unwrap();
                let call = self
                    .builder
                    .build_call(function, &[value.into(), key.into()], "json_has_wrapper_key_u8")
                    .map_err(|error| error.to_string())?;
                let u8_val = call
                    .try_as_basic_value()
                    .basic()
                    .ok_or("thaw_json_has_wrapper_key did not return a value")?
                    .into_int_value();
                let zero = self.context.i8_type().const_int(0, false);
                return self
                    .builder
                    .build_int_compare(inkwell::IntPredicate::NE, u8_val, zero, "json_has_wrapper_key")
                    .map(Into::into)
                    .map_err(|error| error.to_string());
            }
            "__thaw_json_map_or_set_entries" => {
                return self.compile_single_arg_call(
                    "thaw_json_map_or_set_entries",
                    args,
                    "JSON map/set entries",
                )
            }
            "__thaw_json_map_or_set_keys" => {
                let result = self
                    .compile_single_arg_call("thaw_json_map_or_set_keys", args, "Map/Set keys")?
                    .into_pointer_value();
                return Ok(self.compile_array_wrap(result)?.into());
            }
            "__thaw_json_map_or_set_values" => {
                let result = self
                    .compile_single_arg_call("thaw_json_map_or_set_values", args, "Map/Set values")?
                    .into_pointer_value();
                return Ok(self.compile_array_wrap(result)?.into());
            }
            "__thaw_json_map_or_set_entries_view" => {
                let result = self
                    .compile_single_arg_call(
                        "thaw_json_map_or_set_entries_view",
                        args,
                        "Map/Set entries",
                    )?
                    .into_pointer_value();
                return Ok(self.compile_array_wrap(result)?.into());
            }
            "__thaw_json_map_or_set_get" => {
                let [value, key] = args else {
                    return Err("__thaw_json_map_or_set_get expects two arguments".into());
                };
                let value = self.compile_expr(value)?;
                let key = self.compile_expr(key)?;
                let function = self.module.get_function("thaw_json_map_or_set_get").unwrap();
                return self
                    .builder
                    .build_call(function, &[value.into(), key.into()], "json_map_or_set_get")
                    .map_err(|error| error.to_string())?
                    .try_as_basic_value()
                    .basic()
                    .ok_or_else(|| "thaw_json_map_or_set_get did not return a value".into());
            }
            "__thaw_json_map_or_set_has" => {
                let [value, key] = args else {
                    return Err("__thaw_json_map_or_set_has expects two arguments".into());
                };
                let value = self.compile_expr(value)?;
                let key = self.compile_expr(key)?;
                let function = self.module.get_function("thaw_json_map_or_set_has").unwrap();
                let call = self
                    .builder
                    .build_call(function, &[value.into(), key.into()], "json_map_or_set_has_u8")
                    .map_err(|error| error.to_string())?;
                let u8_val = call
                    .try_as_basic_value()
                    .basic()
                    .ok_or("thaw_json_map_or_set_has did not return a value")?
                    .into_int_value();
                let zero = self.context.i8_type().const_int(0, false);
                return self
                    .builder
                    .build_int_compare(inkwell::IntPredicate::NE, u8_val, zero, "json_map_or_set_has")
                    .map(Into::into)
                    .map_err(|error| error.to_string());
            }
            "__thaw_json_map_or_set_set" => {
                let [value, key, new_value] = args else {
                    return Err("__thaw_json_map_or_set_set expects three arguments".into());
                };
                let value = self.compile_expr(value)?;
                let key = self.compile_expr(key)?;
                let new_value = self.compile_expr(new_value)?;
                let function = self.module.get_function("thaw_json_map_or_set_set").unwrap();
                return self
                    .builder
                    .build_call(
                        function,
                        &[value.into(), key.into(), new_value.into()],
                        "json_map_or_set_set",
                    )
                    .map_err(|error| error.to_string())?
                    .try_as_basic_value()
                    .basic()
                    .ok_or_else(|| "thaw_json_map_or_set_set did not return a value".into());
            }
            "__thaw_json_map_or_set_add" => {
                let [value, element] = args else {
                    return Err("__thaw_json_map_or_set_add expects two arguments".into());
                };
                let value = self.compile_expr(value)?;
                let element = self.compile_expr(element)?;
                let function = self.module.get_function("thaw_json_map_or_set_add").unwrap();
                return self
                    .builder
                    .build_call(function, &[value.into(), element.into()], "json_map_or_set_add")
                    .map_err(|error| error.to_string())?
                    .try_as_basic_value()
                    .basic()
                    .ok_or_else(|| "thaw_json_map_or_set_add did not return a value".into());
            }
            "__thaw_json_map_or_set_delete" => {
                let [value, key] = args else {
                    return Err("__thaw_json_map_or_set_delete expects two arguments".into());
                };
                let value = self.compile_expr(value)?;
                let key = self.compile_expr(key)?;
                let function = self.module.get_function("thaw_json_map_or_set_delete").unwrap();
                return self
                    .builder
                    .build_call(function, &[value.into(), key.into()], "json_map_or_set_delete")
                    .map_err(|error| error.to_string())?
                    .try_as_basic_value()
                    .basic()
                    .ok_or_else(|| "thaw_json_map_or_set_delete did not return a value".into());
            }
            "__thaw_json_map_or_set_clear" => {
                return self.compile_single_arg_call(
                    "thaw_json_map_or_set_clear",
                    args,
                    "Map/Set clear",
                )
            }
            "__thaw_json_stringify_number_space" => {
                let [value, space] = args else {
                    return Err("JSON.stringify expects value and number space".into());
                };
                let value = self.compile_expr(value)?;
                let space = self.compile_expr(space)?;
                return self
                    .builder
                    .build_call(
                        self.module
                            .get_function("thaw_json_stringify_number_space")
                            .unwrap(),
                        &[value.into(), space.into()],
                        "json_stringify_number_space",
                    )
                    .map_err(|error| error.to_string())?
                    .try_as_basic_value()
                    .basic()
                    .ok_or("JSON.stringify returned no value".into());
            }
            "__thaw_json_stringify_string_space" => {
                let [value, space] = args else {
                    return Err("JSON.stringify expects value and string space".into());
                };
                let value = self.compile_expr(value)?;
                let space = self.compile_expr(space)?;
                return self
                    .builder
                    .build_call(
                        self.module
                            .get_function("thaw_json_stringify_string_space")
                            .unwrap(),
                        &[value.into(), space.into()],
                        "json_stringify_string_space",
                    )
                    .map_err(|error| error.to_string())?
                    .try_as_basic_value()
                    .basic()
                    .ok_or("JSON.stringify returned no value".into());
            }
            "__thaw_json_stringify_keys" => {
                let [value, keys] = args else {
                    return Err("JSON.stringify expects value and replacer keys".into());
                };
                let value = self.compile_expr(value)?;
                let keys_handle = self.compile_expr(keys)?.into_pointer_value();
                let keys = self.compile_array_data(keys_handle)?;
                return self
                    .builder
                    .build_call(
                        self.module.get_function("thaw_json_stringify_keys").unwrap(),
                        &[value.into(), keys.into()],
                        "json_stringify_keys",
                    )
                    .map_err(|error| error.to_string())?
                    .try_as_basic_value()
                    .basic()
                    .ok_or("JSON.stringify returned no value".into());
            }
            "__thaw_json_stringify_keys_number_space"
            | "__thaw_json_stringify_keys_string_space" => {
                let [value, keys, space] = args else {
                    return Err("JSON.stringify expects value, replacer keys and space".into());
                };
                let value = self.compile_expr(value)?;
                let keys_handle = self.compile_expr(keys)?.into_pointer_value();
                let keys = self.compile_array_data(keys_handle)?;
                let space = self.compile_expr(space)?;
                return self
                    .builder
                    .build_call(
                        self.module
                            .get_function(name.trim_start_matches("__"))
                            .unwrap(),
                        &[value.into(), keys.into(), space.into()],
                        "json_stringify_keys_space",
                    )
                    .map_err(|error| error.to_string())?
                    .try_as_basic_value()
                    .basic()
                    .ok_or("JSON.stringify returned no value".into());
            }
            "__thaw_json_is_array" => {
                let [value] = args else {
                    return Err("Array.isArray expects one operand".to_string());
                };
                let value = self.compile_expr(value)?;
                let result = self
                    .builder
                    .build_call(
                        self.module.get_function("thaw_json_is_array").unwrap(),
                        &[value.into()],
                        "json_is_array",
                    )
                    .map_err(|error| error.to_string())?
                    .try_as_basic_value()
                    .basic()
                    .ok_or("thaw_json_is_array returned no value")?
                    .into_int_value();
                return self
                    .builder
                    .build_int_compare(
                        IntPredicate::NE,
                        result,
                        self.context.i8_type().const_zero(),
                        "array_is_array",
                    )
                    .map(Into::into)
                    .map_err(|error| error.to_string());
            }
            "__thaw_json_is_buffer" => {
                let [value] = args else {
                    return Err("Buffer.isBuffer expects one operand".to_string());
                };
                let value = self.compile_expr(value)?;
                let result = self
                    .builder
                    .build_call(
                        self.module.get_function("thaw_json_is_buffer").unwrap(),
                        &[value.into()],
                        "json_is_buffer",
                    )
                    .map_err(|error| error.to_string())?
                    .try_as_basic_value()
                    .basic()
                    .ok_or("thaw_json_is_buffer returned no value")?
                    .into_int_value();
                return self
                    .builder
                    .build_int_compare(
                        IntPredicate::NE,
                        result,
                        self.context.i8_type().const_zero(),
                        "buffer_is_buffer",
                    )
                    .map(Into::into)
                    .map_err(|error| error.to_string());
            }
            "__thaw_json_clone" => {
                return self.compile_single_arg_call(
                    name.trim_start_matches("__"),
                    args,
                    "structuredClone",
                );
            }
            "__thaw_json_set_prototype" => {
                let [object, prototype] = args else {
                    return Err("__thaw_json_set_prototype expects two operands".into());
                };
                let object = self.compile_expr(object)?;
                let prototype = self.compile_expr(prototype)?;
                return self
                    .builder
                    .build_call(
                        self.module.get_function("thaw_json_set_prototype").unwrap(),
                        &[object.into(), prototype.into()],
                        "json_set_prototype",
                    )
                    .map_err(|error| error.to_string())?
                    .try_as_basic_value()
                    .basic()
                    .ok_or_else(|| "thaw_json_set_prototype returned no value".to_string());
            }
            "__thaw_json_get_prototype" => {
                return self.compile_single_arg_call(
                    name.trim_start_matches("__"),
                    args,
                    "Object.getPrototypeOf",
                );
            }
            "__thaw_json_get_mut" => {
                let [object, key] = args else {
                    return Err("__thaw_json_get_mut expects two operands".into());
                };
                let object = self.compile_expr(object)?;
                let key = self.compile_expr(key)?;
                return self
                    .builder
                    .build_call(
                        self.module.get_function("thaw_json_get_mut").unwrap(),
                        &[object.into(), key.into()],
                        "json_get_mut",
                    )
                    .map_err(|error| error.to_string())?
                    .try_as_basic_value()
                    .basic()
                    .ok_or_else(|| "thaw_json_get_mut returned no value".to_string());
            }
            "__thaw_json_index_get_mut" => {
                let [object, index] = args else {
                    return Err("__thaw_json_index_get_mut expects two operands".into());
                };
                let object = self.compile_expr(object)?;
                let index = self.compile_expr(index)?;
                return self
                    .builder
                    .build_call(
                        self.module.get_function("thaw_json_index_get_mut").unwrap(),
                        &[object.into(), index.into()],
                        "json_index_get_mut",
                    )
                    .map_err(|error| error.to_string())?
                    .try_as_basic_value()
                    .basic()
                    .ok_or_else(|| "thaw_json_index_get_mut returned no value".to_string());
            }
            "__thaw_json_keys" | "__thaw_json_own_keys" => {
                let result = self
                    .compile_single_arg_call(name.trim_start_matches("__"), args, "Object.keys")?
                    .into_pointer_value();
                return Ok(self.compile_array_wrap(result)?.into());
            }
            "__thaw_array_keys" => {
                let [array, include_length] = args else {
                    return Err("array keys expects an array and length flag".into());
                };
                let handle = self.compile_expr(array)?.into_pointer_value();
                let include_length = self.compile_expr(include_length)?.into_int_value();
                let array = self.compile_array_data(handle)?;
                let presence = self.compile_array_presence(handle)?;
                let include_length = self.builder.build_int_z_extend(
                    include_length, self.context.i8_type(), "array_keys_include_length",
                ).map_err(|error| error.to_string())?;
                let result = self.builder.build_call(
                    self.module.get_function("thaw_array_keys").unwrap(),
                    &[array.into(), presence.into(), include_length.into()],
                    "array_keys",
                ).map_err(|error| error.to_string())?.try_as_basic_value().basic()
                    .ok_or("array keys returned no value")?.into_pointer_value();
                return Ok(self.compile_array_wrap(result)?.into());
            }
            "__thaw_json_values" => {
                let result = self
                    .compile_single_arg_call("thaw_json_values", args, "Object.values")?
                    .into_pointer_value();
                return Ok(self.compile_array_wrap(result)?.into());
            }
            "__thaw_json_number_values"
            | "__thaw_json_string_values"
            | "__thaw_json_bool_values" => {
                let result = self
                    .compile_single_arg_call(
                        name.trim_start_matches("__"),
                        args,
                        "Object.values",
                    )?
                    .into_pointer_value();
                return Ok(self.compile_array_wrap(result)?.into());
            }
            "__thaw_json_entries" => {
                let result = self
                    .compile_single_arg_call("thaw_json_entries", args, "Object.entries")?
                    .into_pointer_value();
                return Ok(self.compile_array_wrap(result)?.into());
            }
            "__thaw_json_number_entries"
            | "__thaw_json_string_entries"
            | "__thaw_json_bool_entries" => {
                let result = self
                    .compile_single_arg_call(
                        name.trim_start_matches("__"),
                        args,
                        "Object.entries",
                    )?
                    .into_pointer_value();
                return Ok(self.compile_array_wrap(result)?.into());
            }
            "__thaw_json_object_from_number_entries"
            | "__thaw_json_object_from_string_entries"
            | "__thaw_json_object_from_bool_entries"
            | "__thaw_json_object_from_json_entries" => {
                return self.compile_single_array_arg_call(
                    name.trim_start_matches("__"),
                    args,
                    "Object.fromEntries",
                )
            }
            "__thaw_json_object_assign" => {
                let [target, source] = args else {
                    return Err("Object.assign expects two internal operands".into());
                };
                let target = self.compile_expr(target)?;
                let source = self.compile_expr(source)?;
                return self
                    .builder
                    .build_call(
                        self.module.get_function("thaw_json_object_assign").unwrap(),
                        &[target.into(), source.into()],
                        "object_assign",
                    )
                    .map_err(|error| error.to_string())?
                    .try_as_basic_value()
                    .basic()
                    .ok_or("Object.assign returned no value".into());
            }
            "__thaw_json_array_slice" => {
                let [value, start] = args else {
                    return Err("dynamic array rest expects two operands".into());
                };
                let value = self.compile_expr(value)?;
                let start = self.compile_expr(start)?.into_float_value();
                let start = self
                    .builder
                    .build_float_to_signed_int(
                        start,
                        self.context.i64_type(),
                        "json_array_slice_start",
                    )
                    .map_err(|error| error.to_string())?;
                return self
                    .builder
                    .build_call(
                        self.module.get_function("thaw_json_array_slice").unwrap(),
                        &[value.into(), start.into()],
                        "json_array_slice",
                    )
                    .map_err(|error| error.to_string())?
                    .try_as_basic_value()
                    .basic()
                    .ok_or("thaw_json_array_slice returned no value".into());
            }
            "__thaw_json_has_own" => {
                let [value, key] = args else {
                    return Err("Object.hasOwn expects two operands".to_string());
                };
                let value = self.compile_expr(value)?;
                let key = self.compile_expr(key)?;
                let result = self
                    .builder
                    .build_call(
                        self.module.get_function("thaw_json_has_own").unwrap(),
                        &[value.into(), key.into()],
                        "json_has_own",
                    )
                    .map_err(|error| error.to_string())?
                    .try_as_basic_value()
                    .basic()
                    .ok_or("thaw_json_has_own returned no value")?
                    .into_int_value();
                return self
                    .builder
                    .build_int_compare(
                        IntPredicate::NE,
                        result,
                        self.context.i8_type().const_zero(),
                        "json_has_own_bool",
                    )
                    .map(Into::into)
                    .map_err(|error| error.to_string());
            }
            "__thaw_json_has" => {
                return self.compile_i8_predicate_call("thaw_json_has", args, "json_has");
            }
            "__thaw_json_is_null" => {
                return self.compile_i8_predicate_call(
                    "thaw_json_is_null",
                    args,
                    "json_is_null",
                );
            }
            "__thaw_json_is_nullish" => {
                return self.compile_i8_predicate_call(
                    "thaw_json_is_nullish",
                    args,
                    "json_is_nullish",
                );
            }
            "__thaw_json_is_undefined" => {
                let [value] = args else {
                    return Err("JSON undefined check expects one operand".into());
                };
                let value = self.compile_expr(value)?;
                return self.compile_json_is_napi_undefined(value).map(Into::into);
            }
            "__thaw_json_object_is"
            | "__thaw_json_object_is_number"
            | "__thaw_json_object_is_string"
            | "__thaw_json_object_is_bool" => {
                let runtime = name.trim_start_matches("__thaw_");
                return self.compile_i8_predicate_call(
                    &format!("thaw_{runtime}"),
                    args,
                    "json_object_is",
                );
            }
            _ => {}
        }
        unreachable!("JSON call name was checked before dispatch")
    }
}
