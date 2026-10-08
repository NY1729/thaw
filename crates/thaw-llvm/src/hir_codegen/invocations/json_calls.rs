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
                let [source] = args else {
                    return Err("JSON.parse expects one argument".into());
                };
                let source = self.compile_expr(source)?;
                let result = self.builder.build_call(
                    self.module.get_function("thaw_json_parse").unwrap(),
                    &[source.into()], "json_parse",
                ).map_err(|error| error.to_string())?.try_as_basic_value().basic()
                    .ok_or("JSON.parse returned no value")?;
                let error = self.builder.build_call(
                    self.module.get_function("thaw_json_take_parse_error").unwrap(),
                    &[], "json_parse_error",
                ).map_err(|error| error.to_string())?.try_as_basic_value().basic()
                    .ok_or("JSON parse status returned no value")?.into_int_value();
                let function = self.current_function();
                let invalid = self.context.append_basic_block(function, "json_parse_invalid");
                let valid = self.context.append_basic_block(function, "json_parse_valid");
                let failed = self.builder.build_int_compare(
                    IntPredicate::NE, error, self.context.i8_type().const_zero(), "json_parse_failed",
                ).map_err(|error| error.to_string())?;
                self.builder.build_conditional_branch(failed, invalid, valid)
                    .map_err(|error| error.to_string())?;
                self.builder.position_at_end(invalid);
                self.builder.build_call(
                    self.module.get_function("thaw_json_destroy").unwrap(),
                    &[result.into()], "discard_invalid_json",
                ).map_err(|error| error.to_string())?;
                self.compile_throw_builtin_error("SyntaxError", "Unexpected token in JSON")?;
                self.builder.position_at_end(valid);
                return Ok(result);
            }
            "JSON.stringify" => {
                // The `_public` variant omits/nulls a nested napi-
                // undefined sentinel the way real `JSON.stringify`
                // treats a real `undefined` -- safe here specifically
                // *because* this is the real, user-facing call, unlike
                // the internal argument/result marshaling paths that
                // still use the plain `thaw_json_stringify` and need the
                // sentinel preserved verbatim.
                let result = self.compile_single_arg_call(
                    "thaw_json_stringify_public",
                    args,
                    "JSON.stringify",
                )?;
                return self.compile_check_json_stringify_error(result);
            }
            "__thaw_json_typeof" => {
                let value = self.compile_single_arg_call("thaw_json_typeof", args, "JSON typeof")?;
                return self.compile_check_json_host_error(value, None);
            }
            "__thaw_json_as_bigint_i64" => {
                let value = self.compile_single_arg_call(
                    "thaw_json_as_bigint_i64", args, "checked Json BigInt assertion",
                )?;
                // Host-backed Json may need a reentrant query for the exact
                // decimal. Consume its error before another host operation.
                return self.compile_check_json_host_error(value, None);
            }
            "__thaw_json_borrowed_handle_id" => {
                return self.compile_single_arg_call(
                    "thaw_json_borrowed_handle_id", args, "borrowed JSON handle");
            }
            "__thaw_json_receiver_bigint" => {
                return self.compile_single_arg_call(
                    "thaw_json_receiver_bigint", args, "JSON bigint");
            }
            "__thaw_json_receiver_number" => {
                return self.compile_single_arg_call(
                    "thaw_json_receiver_number", args, "JSON number");
            }
            "__thaw_json_error_to_string" => {
                return self.compile_single_arg_call(
                    "thaw_json_error_to_string", args, "caught JSON toString");
            }
            "__thaw_json_error_stack" => {
                return self.compile_single_arg_call(
                    "thaw_json_error_stack", args, "caught JSON stack");
            }
            "__thaw_json_receiver_string" => {
                return self.compile_single_arg_call(
                    "thaw_json_receiver_string", args, "JSON string");
            }
            "__thaw_json_receiver_bool" => {
                let [value] = args else {
                    return Err("JSON bool expects one argument".into());
                };
                let value = self.compile_expr(value)?.into_int_value();
                let value = self.builder.build_int_z_extend(
                    value, self.context.i8_type(), "json_receiver_bool_u8",
                ).map_err(|error| error.to_string())?;
                let result = self.builder.build_call(
                    self.module.get_function("thaw_json_receiver_bool").unwrap(),
                    &[value.into()], "json_receiver_bool",
                ).map_err(|error| error.to_string())?.try_as_basic_value().basic()
                    .ok_or("JSON bool constructor returned no value")?;
                return self.compile_check_json_host_error(result, Some("thaw_json_destroy"));
            }
            "__thaw_json_undefined" | "__thaw_json_null" => {
                if !args.is_empty() {
                    return Err(format!("{name} expects no arguments"));
                }
                let symbol = if name == "__thaw_json_undefined" {
                    "thaw_json_undefined"
                } else {
                    "thaw_json_null"
                };
                let result = self.builder.build_call(
                    self.module.get_function(symbol).unwrap(), &[], "json_receiver_constant",
                ).map_err(|error| error.to_string())?.try_as_basic_value().basic()
                    .ok_or("JSON constant constructor returned no value")?;
                return self.compile_check_json_host_error(result, Some("thaw_json_destroy"));
            }
            "__thaw_json_track_owned" => {
                let result = self.compile_single_arg_call(
                    "thaw_json_track_arena_owned_root", args, "owned catch JSON root",
                )?;
                return self.compile_check_json_host_error(result, None);
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
                self.compile_check_json_host_error(value, None)?;
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
            "__thaw_json_is_live_iterable" => {
                let [value] = args else {
                    return Err("__thaw_json_is_live_iterable expects one argument".into());
                };
                let value = self.compile_expr(value)?;
                let function = self.module.get_function("thaw_json_is_live_iterable").unwrap();
                let call = self
                    .builder
                    .build_call(function, &[value.into()], "json_is_live_iterable_u8")
                    .map_err(|error| error.to_string())?;
                let u8_val = call
                    .try_as_basic_value()
                    .basic()
                    .ok_or("thaw_json_is_live_iterable did not return a value")?
                    .into_int_value();
                let zero = self.context.i8_type().const_int(0, false);
                return self
                    .builder
                    .build_int_compare(inkwell::IntPredicate::NE, u8_val, zero, "json_is_live_iterable")
                    .map(Into::into)
                    .map_err(|error| error.to_string());
            }
            "__thaw_json_date_timestamp" => {
                let [value] = args else { return Err("JSON date timestamp expects one argument".into()); };
                let value = self.compile_expr(value)?;
                let timestamp = self.builder.build_call(
                    self.module.get_function("thaw_json_date_timestamp").unwrap(),
                    &[value.into()], "JSON date timestamp",
                ).map_err(|error| error.to_string())?.try_as_basic_value().basic()
                    .ok_or("JSON date timestamp returned no value")?;
                self.compile_check_json_host_error(value, None)?;
                return Ok(timestamp);
            }
            "__thaw_json_date_set_timestamp" => {
                let [receiver, timestamp] = args else {
                    return Err("JSON Date setter expects receiver and timestamp".into());
                };
                let receiver = self.compile_expr(receiver)?;
                let timestamp = self.compile_expr(timestamp)?;
                let result = self.builder.build_call(
                    self.module.get_function("thaw_json_date_set_timestamp").unwrap(),
                    &[receiver.into(), timestamp.into()], "JSON Date setter",
                ).map_err(|error| error.to_string())?.try_as_basic_value().basic()
                    .ok_or("JSON Date setter returned no value")?;
                self.compile_check_json_host_error(receiver, None)?;
                return Ok(result);
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
                let result = self
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
                    .ok_or_else(|| String::from("JSON.stringify returned no value"))?;
                return self.compile_check_json_stringify_error(result);
            }
            "__thaw_json_stringify_string_space" => {
                let [value, space] = args else {
                    return Err("JSON.stringify expects value and string space".into());
                };
                let value = self.compile_expr(value)?;
                let space = self.compile_expr(space)?;
                let result = self
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
                    .ok_or_else(|| String::from("JSON.stringify returned no value"))?;
                return self.compile_check_json_stringify_error(result);
            }
            "__thaw_json_stringify_keys" => {
                let [value, keys] = args else {
                    return Err("JSON.stringify expects value and replacer keys".into());
                };
                let value = self.compile_expr(value)?;
                let keys_handle = self.compile_expr(keys)?.into_pointer_value();
                let keys = self.compile_array_data(keys_handle)?;
                let presence = self.compile_array_presence(keys_handle)?;
                let result = self
                    .builder
                    .build_call(
                        self.module.get_function("thaw_json_stringify_keys").unwrap(),
                        &[value.into(), keys.into(), presence.into()],
                        "json_stringify_keys",
                    )
                    .map_err(|error| error.to_string())?
                    .try_as_basic_value()
                    .basic()
                    .ok_or_else(|| String::from("JSON.stringify returned no value"))?;
                return self.compile_check_json_stringify_error(result);
            }
            "__thaw_json_stringify_keys_number_space"
            | "__thaw_json_stringify_keys_string_space" => {
                let [value, keys, space] = args else {
                    return Err("JSON.stringify expects value, replacer keys and space".into());
                };
                let value = self.compile_expr(value)?;
                let keys_handle = self.compile_expr(keys)?.into_pointer_value();
                let keys = self.compile_array_data(keys_handle)?;
                let presence = self.compile_array_presence(keys_handle)?;
                let space = self.compile_expr(space)?;
                let result = self
                    .builder
                    .build_call(
                        self.module
                            .get_function(name.trim_start_matches("__"))
                            .unwrap(),
                        &[value.into(), keys.into(), presence.into(), space.into()],
                        "json_stringify_keys_space",
                    )
                    .map_err(|error| error.to_string())?
                    .try_as_basic_value()
                    .basic()
                    .ok_or_else(|| String::from("JSON.stringify returned no value"))?;
                return self.compile_check_json_stringify_error(result);
            }
            "__thaw_json_array_join" => {
                let [value, separator] = args else {
                    return Err("Array.join expects a value and separator".to_string());
                };
                let value = self.compile_expr(value)?;
                let separator = self.compile_expr(separator)?;
                return self
                    .builder
                    .build_call(
                        self.module.get_function("thaw_json_array_join").unwrap(),
                        &[value.into(), separator.into()],
                        "json_array_join",
                    )
                    .map_err(|error| error.to_string())?
                    .try_as_basic_value()
                    .basic()
                    .ok_or_else(|| "thaw_json_array_join returned no value".to_string());
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
                self.compile_check_json_host_error(result.into(), None)?;
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
                self.compile_check_json_host_error(result.into(), None)?;
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
            "__thaw_json_brand_wrapper" => {
                return self.compile_single_arg_call("thaw_json_brand_wrapper", args, "JSON wrapper brand");
            }
            "__thaw_json_clone" => {
                return self.compile_single_arg_call(
                    "thaw_json_structured_clone",
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
                let result = self
                    .builder
                    .build_call(
                        self.module.get_function("thaw_json_set_prototype").unwrap(),
                        &[object.into(), prototype.into()],
                        "json_set_prototype",
                    )
                    .map_err(|error| error.to_string())?
                    .try_as_basic_value()
                    .basic()
                    .ok_or("thaw_json_set_prototype returned no value")?;
                let error = self.builder.build_call(
                    self.module.get_function("thaw_json_take_prototype_error").unwrap(),
                    &[], "json_prototype_error",
                ).map_err(|error| error.to_string())?.try_as_basic_value().basic()
                    .ok_or("JSON prototype error status returned no value")?.into_int_value();
                let function = self.current_function();
                let invalid = self.context.append_basic_block(function, "json_prototype_invalid");
                let valid = self.context.append_basic_block(function, "json_prototype_valid");
                let failed = self.builder.build_int_compare(
                    IntPredicate::NE, error, self.context.i8_type().const_zero(), "json_prototype_failed",
                ).map_err(|error| error.to_string())?;
                self.builder.build_conditional_branch(failed, invalid, valid)
                    .map_err(|error| error.to_string())?;
                self.builder.position_at_end(invalid);
                self.compile_throw_type_error("Invalid prototype")?;
                self.builder.position_at_end(valid);
                return Ok(result);
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
                return self.compile_json_enumeration(name.trim_start_matches("__"), args);
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
                return self.compile_json_enumeration("thaw_json_values", args);
            }
            "__thaw_json_number_values"
            | "__thaw_json_string_values"
            | "__thaw_json_bool_values" => {
                return self.compile_json_enumeration(name.trim_start_matches("__"), args);
            }
            "__thaw_json_entries" => {
                return self.compile_json_enumeration("thaw_json_entries", args);
            }
            "__thaw_json_number_entries"
            | "__thaw_json_string_entries"
            | "__thaw_json_bool_entries" => {
                return self.compile_json_enumeration(name.trim_start_matches("__"), args);
            }
            "__thaw_json_object_from_number_entries"
            | "__thaw_json_object_from_string_entries"
            | "__thaw_json_object_from_bool_entries"
            | "__thaw_json_object_from_json_entries" => {
                let [entries] = args else {
                    return Err("Object.fromEntries expects one argument".into());
                };
                let handle = self.compile_expr(entries)?.into_pointer_value();
                let buffer = self.compile_array_data(handle)?;
                let presence = self.compile_array_presence(handle)?;
                let result = self.builder.build_call(
                    self.module.get_function(name.trim_start_matches("__")).unwrap(),
                    &[buffer.into(), presence.into()], "object_from_entries",
                ).map_err(|error| error.to_string())?.try_as_basic_value().basic()
                    .ok_or("Object.fromEntries returned no value")?;
                let error = self.builder.build_call(
                    self.module.get_function("thaw_json_take_from_entries_error").unwrap(),
                    &[], "object_from_entries_error",
                ).map_err(|error| error.to_string())?.try_as_basic_value().basic()
                    .ok_or("Object.fromEntries error status returned no value")?.into_int_value();
                let function = self.current_function();
                let invalid = self.context.append_basic_block(function, "object_from_entries_invalid");
                let valid = self.context.append_basic_block(function, "object_from_entries_valid");
                let failed = self.builder.build_int_compare(
                    IntPredicate::NE, error, self.context.i8_type().const_zero(), "object_from_entries_failed",
                ).map_err(|error| error.to_string())?;
                self.builder.build_conditional_branch(failed, invalid, valid)
                    .map_err(|error| error.to_string())?;
                self.builder.position_at_end(invalid);
                self.compile_throw_type_error("Iterator value is not an entry object")?;
                self.builder.position_at_end(valid);
                return Ok(result);
            }
            "__thaw_json_object_assign" => {
                let [target, source] = args else {
                    return Err("Object.assign expects two internal operands".into());
                };
                let target = self.compile_expr(target)?;
                let source = self.compile_expr(source)?;
                self.compile_guard_json_non_nullish(target, "Cannot convert undefined or null to object")?;
                let assigned = self
                    .builder
                    .build_call(
                        self.module.get_function("thaw_json_object_assign").unwrap(),
                        &[target.into(), source.into()],
                        "object_assign",
                    )
                    .map_err(|error| error.to_string())?
                    .try_as_basic_value()
                    .basic()
                    .ok_or("Object.assign returned no value")?;
                let error = self.builder.build_call(
                    self.module.get_function("thaw_json_take_assign_error").unwrap(),
                    &[], "object_assign_error",
                ).map_err(|error| error.to_string())?.try_as_basic_value().basic()
                    .ok_or("Object.assign error status returned no value")?.into_int_value();
                let function = self.current_function();
                let blocked = self.context.append_basic_block(function, "object_assign_blocked");
                let allowed = self.context.append_basic_block(function, "object_assign_allowed");
                let failed = self.builder.build_int_compare(
                    IntPredicate::NE, error, self.context.i8_type().const_zero(), "object_assign_failed",
                ).map_err(|error| error.to_string())?;
                self.builder.build_conditional_branch(failed, blocked, allowed)
                    .map_err(|error| error.to_string())?;
                self.builder.position_at_end(blocked);
                self.compile_throw_type_error("Cannot assign to read only or non-extensible object")?;
                self.builder.position_at_end(allowed);
                return Ok(assigned);
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
            "__thaw_json_has_own" | "__thaw_json_property_is_enumerable" => {
                let predicate = if name == "__thaw_json_property_is_enumerable" {
                    "thaw_json_property_is_enumerable"
                } else {
                    "thaw_json_has_own"
                };
                let [value, key] = args else {
                    return Err("Object.hasOwn expects two operands".to_string());
                };
                let value = self.compile_expr(value)?;
                let key = self.compile_expr(key)?;
                self.compile_guard_json_non_nullish(value, "Cannot convert undefined or null to object")?;
                let result = self
                    .builder
                    .build_call(
                        self.module.get_function(predicate).unwrap(),
                        &[value.into(), key.into()],
                        "json_has_own",
                    )
                    .map_err(|error| error.to_string())?
                    .try_as_basic_value()
                    .basic()
                    .ok_or("thaw_json_has_own returned no value")?
                    .into_int_value();
                self.compile_check_json_host_error(result.into(), None)?;
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
                let [value, key] = args else {
                    return Err("JSON has expects two operands".into());
                };
                let value = self.compile_expr(value)?;
                let key = self.compile_expr(key)?;
                self.compile_guard_json_non_nullish(value, "Cannot use 'in' with undefined or null")?;
                let result = self.builder.build_call(
                    self.module.get_function("thaw_json_has").unwrap(),
                    &[value.into(), key.into()], "json_has",
                ).map_err(|error| error.to_string())?.try_as_basic_value().basic()
                    .ok_or("JSON has returned no value")?.into_int_value();
                self.compile_check_json_host_error(result.into(), None)?;
                return self.builder.build_int_compare(
                    IntPredicate::NE, result, self.context.i8_type().const_zero(), "json_has_bool",
                ).map(Into::into).map_err(|error| error.to_string());
            }
            "__thaw_json_object_delete_reflect" => {
                let [value, key] = args else {
                    return Err("Reflect.deleteProperty expects two operands".into());
                };
                let value = self.compile_expr(value)?;
                let key = self.compile_expr(key)?;
                let is_object = self.builder.build_call(
                    self.module.get_function("thaw_json_is_object_like").unwrap(),
                    &[value.into()], "json_reflect_receiver_object",
                ).map_err(|error| error.to_string())?.try_as_basic_value().basic()
                    .ok_or("Reflect.deleteProperty receiver check returned no value")?.into_int_value();
                let function = self.current_function();
                let invalid = self.context.append_basic_block(function, "json_reflect_receiver_invalid");
                let valid = self.context.append_basic_block(function, "json_reflect_receiver_valid");
                let is_object = self.builder.build_int_compare(
                    IntPredicate::NE, is_object, self.context.i8_type().const_zero(), "json_reflect_is_object",
                ).map_err(|error| error.to_string())?;
                self.builder.build_conditional_branch(is_object, valid, invalid)
                    .map_err(|error| error.to_string())?;
                self.builder.position_at_end(invalid);
                self.compile_throw_type_error("Reflect.deleteProperty requires an object")?;
                self.builder.position_at_end(valid);
                let result = self.builder.build_call(
                    self.module.get_function("thaw_json_object_delete").unwrap(),
                    &[value.into(), key.into()], "json_reflect_delete",
                ).map_err(|error| error.to_string())?.try_as_basic_value().basic()
                    .ok_or("Reflect.deleteProperty returned no value")?.into_int_value();
                self.compile_check_json_host_error(result.into(), None)?;
                return self.builder.build_int_compare(
                    IntPredicate::NE, result, self.context.i8_type().const_zero(), "json_reflect_deleted",
                ).map(Into::into).map_err(|error| error.to_string());
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

    fn compile_check_json_stringify_error(
        &mut self,
        result: BasicValueEnum<'ctx>,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        self.compile_check_json_stringify_error_with_cleanup(result, &[])
    }

    fn compile_check_json_stringify_error_with_cleanup(
        &mut self,
        result: BasicValueEnum<'ctx>,
        cleanup: &[BasicValueEnum<'ctx>],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        self.compile_check_json_stringify_error_with_inputs(result, cleanup, &[])
    }

    fn compile_check_json_stringify_error_with_inputs(
        &mut self,
        result: BasicValueEnum<'ctx>,
        cleanup: &[BasicValueEnum<'ctx>],
        graph_strings: &[BasicValueEnum<'ctx>],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        self.compile_check_json_stringify_error_with_inputs_and_handles(
            result, cleanup, graph_strings, &[], &[],
        )
    }

    fn compile_check_json_stringify_error_with_inputs_and_handles(
        &mut self,
        result: BasicValueEnum<'ctx>,
        cleanup: &[BasicValueEnum<'ctx>],
        graph_strings: &[BasicValueEnum<'ctx>],
        handles: &[PointerValue<'ctx>],
        direct_handles: &[IntValue<'ctx>],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let error = self.builder.build_call(
            self.module.get_function("thaw_json_take_stringify_error").unwrap(),
            &[], "json_stringify_error",
        ).map_err(|error| error.to_string())?.try_as_basic_value().basic()
            .ok_or("JSON stringify status returned no value")?.into_int_value();
        let function = self.current_function();
        let invalid = self.context.append_basic_block(function, "json_stringify_invalid");
        let valid = self.context.append_basic_block(function, "json_stringify_valid");
        let failed = self.builder.build_int_compare(
            IntPredicate::NE, error, self.context.i8_type().const_zero(), "json_stringify_failed",
        ).map_err(|error| error.to_string())?;
        self.builder.build_conditional_branch(failed, invalid, valid)
            .map_err(|error| error.to_string())?;
        self.builder.position_at_end(invalid);
        self.builder.build_call(
            self.module.get_function("thaw_cstring_destroy").unwrap(),
            &[result.into()], "discard_cyclic_json",
        ).map_err(|error| error.to_string())?;
        for value in cleanup {
            self.builder.build_call(
                self.module.get_function("thaw_json_destroy").unwrap(),
                &[(*value).into()], "destroy_cyclic_json_input",
            ).map_err(|error| error.to_string())?;
        }
        for string in graph_strings {
            self.builder.build_call(
                self.module.get_function("thaw_json_discard_graph_wire").unwrap(),
                &[(*string).into()], "release_unsent_graph_leases",
            ).map_err(|error| error.to_string())?;
            self.builder.build_call(
                self.module.get_function("thaw_cstring_destroy").unwrap(),
                &[(*string).into()], "destroy_cyclic_json_input_string",
            ).map_err(|error| error.to_string())?;
        }
        for handle_array in handles {
            self.builder.build_call(self.module.get_function("thaw_js_release_native_handle_array").unwrap(),
                &[(*handle_array).into()], "release_failed_projection_callbacks")
                .map_err(|error| error.to_string())?;
        }
        for handle in direct_handles {
            self.builder.build_call(self.module.get_function("thaw_js_release_handle").unwrap(),
                &[(*handle).into()], "release_failed_projection_builder")
                .map_err(|error| error.to_string())?;
        }
        self.compile_throw_builtin_error("TypeError", "Converting circular structure to JSON")?;
        self.builder.position_at_end(valid);
        let host_error = self.builder.build_call(
            self.module.get_function("thaw_json_take_host_error").unwrap(),
            &[], "json_stringify_host_error",
        ).map_err(|error| error.to_string())?.try_as_basic_value().basic()
            .ok_or("JSON host error query returned no value")?.into_pointer_value();
        let host_failed = self.context.append_basic_block(function, "json_stringify_host_failed");
        let host_valid = self.context.append_basic_block(function, "json_stringify_host_valid");
        let has_host_error = self.builder.build_is_not_null(host_error, "json_stringify_host_failed")
            .map_err(|error| error.to_string())?;
        self.builder.build_conditional_branch(has_host_error, host_failed, host_valid)
            .map_err(|error| error.to_string())?;
        self.builder.position_at_end(host_failed);
        self.builder.build_call(self.module.get_function("thaw_cstring_destroy").unwrap(),
            &[result.into()], "discard_failed_host_stringify")
            .map_err(|error| error.to_string())?;
        for value in cleanup {
            self.builder.build_call(self.module.get_function("thaw_json_destroy").unwrap(),
                &[(*value).into()], "destroy_failed_host_stringify_input")
                .map_err(|error| error.to_string())?;
        }
        for string in graph_strings {
            self.builder.build_call(self.module.get_function("thaw_json_discard_graph_wire").unwrap(),
                &[(*string).into()], "release_failed_host_graph_leases")
                .map_err(|error| error.to_string())?;
            self.builder.build_call(self.module.get_function("thaw_cstring_destroy").unwrap(),
                &[(*string).into()], "destroy_failed_host_stringify_string")
                .map_err(|error| error.to_string())?;
        }
        for handle_array in handles {
            self.builder.build_call(self.module.get_function("thaw_js_release_native_handle_array").unwrap(),
                &[(*handle_array).into()], "release_failed_projection_callbacks")
                .map_err(|error| error.to_string())?;
        }
        for handle in direct_handles {
            self.builder.build_call(self.module.get_function("thaw_js_release_handle").unwrap(),
                &[(*handle).into()], "release_failed_projection_builder")
                .map_err(|error| error.to_string())?;
        }
        self.builder.build_store(self.pending_exception().as_pointer_value(), host_error)
            .map_err(|error| error.to_string())?;
        self.clear_pending_native_text()?;
        self.mark_pending_native_text(host_error)?;
        self.branch_on_pending_exception()?;
        self.builder.build_unconditional_branch(host_valid).map_err(|error| error.to_string())?;
        self.builder.position_at_end(host_valid);
        Ok(result)
    }

    // Callback adapters return framed failures to the host instead of using
    // their caller's pending-exception slot.
    fn compile_check_json_stringify_callback_result(
        &mut self,
        result: BasicValueEnum<'ctx>,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let error = self.builder.build_call(
            self.module.get_function("thaw_json_take_stringify_error").unwrap(),
            &[], "callback_stringify_error",
        ).map_err(|error| error.to_string())?.try_as_basic_value().basic()
            .ok_or("callback stringify status returned no value")?.into_int_value();
        let function = self.current_function();
        let invalid = self.context.append_basic_block(function, "callback_stringify_invalid");
        let valid = self.context.append_basic_block(function, "callback_stringify_valid");
        let failed = self.builder.build_int_compare(
            IntPredicate::NE, error, self.context.i8_type().const_zero(), "callback_stringify_failed",
        ).map_err(|error| error.to_string())?;
        self.builder.build_conditional_branch(failed, invalid, valid)
            .map_err(|error| error.to_string())?;
        self.builder.position_at_end(invalid);
        self.builder.build_call(
            self.module.get_function("thaw_cstring_destroy").unwrap(),
            &[result.into()], "discard_cyclic_callback_json",
        ).map_err(|error| error.to_string())?;
        let message = self.builder.build_global_string_ptr(
            "\u{1}TypeError\u{1}Converting circular structure to JSON",
            "cyclic_callback_type_error",
        ).map_err(|error| error.to_string())?;
        let framed = self.builder.build_call(
            self.module.get_function("thaw_json_callback_error").unwrap(),
            &[message.as_pointer_value().into()], "cyclic_callback_error",
        ).map_err(|error| error.to_string())?.try_as_basic_value().basic()
            .ok_or("callback error framing returned no value")?;
        self.builder.build_return(Some(&framed)).map_err(|error| error.to_string())?;
        self.builder.position_at_end(valid);
        let host_error = self.builder.build_call(
            self.module.get_function("thaw_json_take_host_error").unwrap(),
            &[], "callback_stringify_host_error",
        ).map_err(|error| error.to_string())?.try_as_basic_value().basic()
            .ok_or("callback host error query returned no value")?.into_pointer_value();
        let host_failed = self.context.append_basic_block(function, "callback_host_failed");
        let host_valid = self.context.append_basic_block(function, "callback_host_valid");
        let has_host_error = self.builder.build_is_not_null(host_error, "callback_has_host_error")
            .map_err(|error| error.to_string())?;
        self.builder.build_conditional_branch(has_host_error, host_failed, host_valid)
            .map_err(|error| error.to_string())?;
        self.builder.position_at_end(host_failed);
        self.builder.build_call(self.module.get_function("thaw_cstring_destroy").unwrap(),
            &[result.into()], "discard_failed_host_callback_json")
            .map_err(|error| error.to_string())?;
        let framed = self.builder.build_call(
            self.module.get_function("thaw_json_callback_error").unwrap(),
            &[host_error.into()], "callback_host_error_result",
        ).map_err(|error| error.to_string())?.try_as_basic_value().basic()
            .ok_or("callback error framing returned no value")?;
        self.builder.build_call(self.module.get_function("thaw_cstring_destroy").unwrap(),
            &[host_error.into()], "destroy_host_callback_error")
            .map_err(|error| error.to_string())?;
        self.builder.build_return(Some(&framed)).map_err(|error| error.to_string())?;
        self.builder.position_at_end(host_valid);
        Ok(result)
    }

    fn compile_guard_json_non_nullish(
        &mut self,
        value: BasicValueEnum<'ctx>,
        message: &str,
    ) -> Result<(), String> {
        let nullish = self.builder.build_call(
            self.module.get_function("thaw_json_is_nullish").unwrap(),
            &[value.into()], "json_receiver_nullish",
        ).map_err(|error| error.to_string())?.try_as_basic_value().basic()
            .ok_or("JSON nullish check returned no value")?.into_int_value();
        let function = self.current_function();
        let invalid = self.context.append_basic_block(function, "json_receiver_invalid");
        let valid = self.context.append_basic_block(function, "json_receiver_valid");
        let is_nullish = self.builder.build_int_compare(
            IntPredicate::NE, nullish, self.context.i8_type().const_zero(), "json_is_nullish",
        ).map_err(|error| error.to_string())?;
        self.builder.build_conditional_branch(is_nullish, invalid, valid)
            .map_err(|error| error.to_string())?;
        self.builder.position_at_end(invalid);
        self.compile_throw_type_error(message)?;
        self.builder.position_at_end(valid);
        Ok(())
    }

    fn compile_json_enumeration(
        &mut self,
        runtime: &str,
        args: &[HirExpr],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let [value] = args else {
            return Err(format!("{runtime} expects one argument"));
        };
        let value = self.compile_expr(value)?;
        self.compile_guard_json_non_nullish(value, "Cannot convert undefined or null to object")?;
        let result = self.builder.build_call(
            self.module.get_function(runtime).unwrap(), &[value.into()], "json_enumeration",
        ).map_err(|error| error.to_string())?.try_as_basic_value().basic()
            .ok_or("JSON enumeration returned no value")?.into_pointer_value();
        self.compile_check_json_host_error(result.into(), None)?;
        Ok(self.compile_array_wrap(result)?.into())
    }
}
