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
                return self.compile_single_arg_call("thaw_json_stringify", args, "JSON.stringify")
            }
            "__thaw_json_typeof" => {
                return self.compile_single_arg_call("thaw_json_typeof", args, "JSON typeof")
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
            "__thaw_json_keys" => {
                let result = self
                    .compile_single_arg_call("thaw_json_keys", args, "Object.keys")?
                    .into_pointer_value();
                return Ok(self.compile_array_wrap(result)?.into());
            }
            "__thaw_array_keys" => {
                let result = self
                    .compile_single_array_arg_call("thaw_array_keys", args, "Object.keys")?
                    .into_pointer_value();
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
            "__thaw_json_is_null" => {
                return self.compile_i8_predicate_call(
                    "thaw_json_is_null",
                    args,
                    "json_is_null",
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
