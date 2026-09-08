impl<'ctx> HirCompiler<'ctx> {
    fn compile_call(
        &mut self,
        callee: &HirExpr,
        args: &[HirExpr],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let HirExpr::Var(name) = callee else {
            return self.compile_expression_call(callee, args);
        };

        if let Some(result) = self.compile_dynamic_named_call(name, args) {
            return result;
        }
        if let Some(result) = self.compile_json_named_call(name, args) {
            return result;
        }
        if let Some(result) = self.compile_array_named_call(name, args) {
            return result;
        }

        match name.as_str() {
            "console.log" | "console.info" | "console.debug" => {
                return self.compile_console_log(args, false)
            }
            "console.warn" | "console.error" => return self.compile_console_log(args, true),
            "console.assert" => return self.compile_console_assert(args),
            "__thaw_string_concat" => return self.compile_string_concat(args),
            "__thaw_string_from_char_code" => {
                return self.compile_single_arg_call(
                    "thaw_string_from_char_code",
                    args,
                    "String.fromCharCode",
                )
            }
            "__thaw_string_from_code_point" => {
                return self.compile_single_arg_call(
                    "thaw_string_from_code_point",
                    args,
                    "String.fromCodePoint",
                )
            }
            "__thaw_bool_to_string" => return self.compile_bool_to_string(args),
            "__thaw_number_to_string" => {
                return self.compile_single_arg_call(
                    "thaw_number_to_string",
                    args,
                    "String(number)",
                )
            }
            "__thaw_bool_to_number" => return self.compile_bool_to_number(args),
            "__thaw_string_to_number" => {
                return self.compile_single_arg_call(
                    "thaw_string_to_number",
                    args,
                    "Number(string)",
                )
            }
            "__thaw_parse_float" => {
                return self.compile_single_arg_call("thaw_parse_float", args, "parseFloat")
            }
            "__thaw_parse_int" => {
                let [text, radix] = args else {
                    return Err("parseInt expects text and radix operands".to_string());
                };
                let text = self.compile_expr(text)?;
                let radix = self.compile_expr(radix)?;
                return self
                    .builder
                    .build_call(
                        self.module.get_function("thaw_parse_int").unwrap(),
                        &[text.into(), radix.into()],
                        "parseInt",
                    )
                    .map_err(|error| error.to_string())?
                    .try_as_basic_value()
                    .basic()
                    .ok_or("parseInt returned no value".to_string());
            }
            "__thaw_string_lt" => return self.compile_string_comparison(args, IntPredicate::SLT),
            "__thaw_string_gt" => return self.compile_string_comparison(args, IntPredicate::SGT),
            "__thaw_string_lte" => return self.compile_string_comparison(args, IntPredicate::SLE),
            "__thaw_string_gte" => return self.compile_string_comparison(args, IntPredicate::SGE),
            "__thaw_string_index_of"
            | "__thaw_string_last_index_of"
            | "__thaw_string_includes"
            | "__thaw_string_starts_with"
            | "__thaw_string_ends_with" => {
                if args.len() != 3 {
                    return Err("string search expects three operands".to_string());
                }
                let mut arguments = Vec::with_capacity(3);
                for argument in args {
                    arguments.push(self.compile_expr(argument)?.into());
                }
                let runtime = name.trim_start_matches("__thaw_");
                let runtime = format!("thaw_{runtime}");
                let result = self
                    .builder
                    .build_call(
                        self.module.get_function(&runtime).unwrap(),
                        &arguments,
                        "string_search",
                    )
                    .map_err(|error| error.to_string())?
                    .try_as_basic_value()
                    .basic()
                    .ok_or("string search returned no value".to_string())?;
                if !matches!(
                    name.as_str(),
                    "__thaw_string_index_of" | "__thaw_string_last_index_of"
                ) {
                    return self
                        .builder
                        .build_int_compare(
                            IntPredicate::NE,
                            result.into_int_value(),
                            self.context.i8_type().const_zero(),
                            "string_search_bool",
                        )
                        .map(Into::into)
                        .map_err(|error| error.to_string());
                }
                return Ok(result);
            }
            "__thaw_string_slice" => {
                let arguments = args
                    .iter()
                    .map(|argument| self.compile_expr(argument).map(Into::into))
                    .collect::<Result<Vec<_>, _>>()?;
                return self
                    .builder
                    .build_call(
                        self.module.get_function("thaw_string_slice").unwrap(),
                        &arguments,
                        "string_slice",
                    )
                    .map_err(|error| error.to_string())?
                    .try_as_basic_value()
                    .basic()
                    .ok_or("string slice returned no value".to_string());
            }
            "__thaw_string_trim"
            | "__thaw_string_trim_start"
            | "__thaw_string_trim_end"
            | "__thaw_string_to_lower_case"
            | "__thaw_string_to_upper_case" => {
                let runtime = format!("thaw_{}", name.trim_start_matches("__thaw_"));
                return self.compile_single_arg_call(&runtime, args, "string transform");
            }
            "__thaw_string_to_array" => {
                let result = self
                    .compile_single_arg_call("thaw_string_to_array", args, "string iterator array")?
                    .into_pointer_value();
                return Ok(self.compile_array_wrap(result)?.into());
            }
            "__thaw_string_split" => {
                let [value, separator, limit] = args else {
                    return Err("string split expects three operands".into());
                };
                let value = self.compile_expr(value)?;
                let separator = self.compile_expr(separator)?;
                let limit = self.compile_expr(limit)?;
                let result = self
                    .builder
                    .build_call(
                        self.module.get_function("thaw_string_split").unwrap(),
                        &[value.into(), separator.into(), limit.into()],
                        "string_split",
                    )
                    .map_err(|error| error.to_string())?
                    .try_as_basic_value()
                    .basic()
                    .ok_or("string split returned no value")?
                    .into_pointer_value();
                return Ok(self.compile_array_wrap(result)?.into());
            }
            "__thaw_string_replace" | "__thaw_string_replace_all" => {
                let [value, search, replacement] = args else {
                    return Err("string replace expects three operands".into());
                };
                let value = self.compile_expr(value)?;
                let search = self.compile_expr(search)?;
                let replacement = self.compile_expr(replacement)?;
                let runtime = if name == "__thaw_string_replace" {
                    "thaw_string_replace"
                } else {
                    "thaw_string_replace_all"
                };
                return self
                    .builder
                    .build_call(
                        self.module.get_function(runtime).unwrap(),
                        &[value.into(), search.into(), replacement.into()],
                        "string_replace",
                    )
                    .map_err(|error| error.to_string())?
                    .try_as_basic_value()
                    .basic()
                    .ok_or("string replace returned no value".into());
            }
            "__thaw_regex_test" => {
                let [source, flags, value] = args else {
                    return Err("RegExp.test expects three operands".into());
                };
                let source = self.compile_expr(source)?;
                let flags = self.compile_expr(flags)?;
                let value = self.compile_expr(value)?;
                let result = self
                    .builder
                    .build_call(
                        self.module.get_function("thaw_regex_test").unwrap(),
                        &[source.into(), flags.into(), value.into()],
                        "regex_test",
                    )
                    .map_err(|error| error.to_string())?
                    .try_as_basic_value()
                    .basic()
                    .ok_or("RegExp.test returned no value")?
                    .into_int_value();
                return self
                    .builder
                    .build_int_compare(
                        IntPredicate::NE,
                        result,
                        self.context.i8_type().const_zero(),
                        "regex_test_bool",
                    )
                    .map(Into::into)
                    .map_err(|error| error.to_string());
            }
            "__thaw_regex_search" => {
                let [value, source, flags] = args else {
                    return Err("RegExp search expects three operands".into());
                };
                let value = self.compile_expr(value)?;
                let source = self.compile_expr(source)?;
                let flags = self.compile_expr(flags)?;
                return self
                    .builder
                    .build_call(
                        self.module.get_function("thaw_regex_search").unwrap(),
                        &[value.into(), source.into(), flags.into()],
                        "regex_search",
                    )
                    .map_err(|error| error.to_string())?
                    .try_as_basic_value()
                    .basic()
                    .ok_or("RegExp search returned no value".into());
            }
            "__thaw_date_now" => {
                if !args.is_empty() {
                    return Err("Date.now expects no operands".into());
                }
                return self
                    .builder
                    .build_call(
                        self.module.get_function("thaw_date_now").unwrap(),
                        &[],
                        "date_now",
                    )
                    .map_err(|error| error.to_string())?
                    .try_as_basic_value()
                    .basic()
                    .ok_or("Date.now returned no value".into());
            }
            "__thaw_performance_now" => {
                if !args.is_empty() {
                    return Err("performance.now expects no operands".into());
                }
                return self
                    .builder
                    .build_call(
                        self.module.get_function("thaw_performance_now").unwrap(),
                        &[],
                        "performance_now",
                    )
                    .map_err(|error| error.to_string())?
                    .try_as_basic_value()
                    .basic()
                    .ok_or("performance.now returned no value".into());
            }
            "__thaw_date_get_full_year"
            | "__thaw_date_get_month"
            | "__thaw_date_get_date"
            | "__thaw_date_get_day"
            | "__thaw_date_get_hours"
            | "__thaw_date_get_minutes"
            | "__thaw_date_get_seconds"
            | "__thaw_date_get_milliseconds" => {
                let [timestamp] = args else {
                    return Err(format!("{name} expects one operand"));
                };
                let timestamp = self.compile_expr(timestamp)?;
                let runtime = name.trim_start_matches("__thaw_").to_string();
                let runtime = format!("thaw_{runtime}");
                return self
                    .builder
                    .build_call(
                        self.module.get_function(&runtime).unwrap(),
                        &[timestamp.into()],
                        "date_get",
                    )
                    .map_err(|error| error.to_string())?
                    .try_as_basic_value()
                    .basic()
                    .ok_or("Date getter returned no value".into());
            }
            "__thaw_date_to_iso_string"
            | "__thaw_date_to_date_string"
            | "__thaw_date_to_time_string"
            | "__thaw_date_to_string"
            | "__thaw_date_to_utc_string" => {
                let [timestamp] = args else {
                    return Err(format!("{name} expects one operand"));
                };
                let timestamp = self.compile_expr(timestamp)?;
                let runtime = name.trim_start_matches("__thaw_").to_string();
                let runtime = format!("thaw_{runtime}");
                return self
                    .builder
                    .build_call(
                        self.module.get_function(&runtime).unwrap(),
                        &[timestamp.into()],
                        "date_to_string",
                    )
                    .map_err(|error| error.to_string())?
                    .try_as_basic_value()
                    .basic()
                    .ok_or("Date string conversion returned no value".into());
            }
            "__thaw_error_message" | "__thaw_error_name" => {
                let [value] = args else {
                    return Err(format!("{name} expects one operand"));
                };
                let value = self.compile_expr(value)?;
                let runtime = name.trim_start_matches("__thaw_").to_string();
                let runtime = format!("thaw_{runtime}");
                return self
                    .builder
                    .build_call(
                        self.module.get_function(&runtime).unwrap(),
                        &[value.into()],
                        "error_property",
                    )
                    .map_err(|error| error.to_string())?
                    .try_as_basic_value()
                    .basic()
                    .ok_or("error property access returned no value".into());
            }
            "__thaw_error_is_instance" => {
                let [value, class_name] = args else {
                    return Err(format!("{name} expects two operands"));
                };
                let value = self.compile_expr(value)?;
                let class_name = self.compile_expr(class_name)?;
                return self
                    .builder
                    .build_call(
                        self.module.get_function("thaw_error_is_instance").unwrap(),
                        &[value.into(), class_name.into()],
                        "error_is_instance",
                    )
                    .map_err(|error| error.to_string())?
                    .try_as_basic_value()
                    .basic()
                    .ok_or("error instanceof check returned no value".into());
            }
            "__thaw_set_pending_exception_object" => {
                let [value] = args else {
                    return Err(format!("{name} expects one operand"));
                };
                let value = self.compile_expr(value)?;
                self.builder
                    .build_store(self.pending_exception_object().as_pointer_value(), value)
                    .map_err(|error| error.to_string())?;
                return Ok(self.context.i32_type().const_int(0, false).into());
            }
            "__thaw_date_set_full_year"
            | "__thaw_date_set_month"
            | "__thaw_date_set_date"
            | "__thaw_date_set_hours"
            | "__thaw_date_set_minutes"
            | "__thaw_date_set_seconds"
            | "__thaw_date_set_milliseconds"
            | "__thaw_date_utc" => {
                let mut compiled = Vec::with_capacity(args.len());
                for argument in args {
                    compiled.push(self.compile_expr(argument)?.into());
                }
                let runtime = name.trim_start_matches("__thaw_").to_string();
                let runtime = format!("thaw_{runtime}");
                return self
                    .builder
                    .build_call(
                        self.module.get_function(&runtime).unwrap(),
                        &compiled,
                        "date_set",
                    )
                    .map_err(|error| error.to_string())?
                    .try_as_basic_value()
                    .basic()
                    .ok_or("Date setter returned no value".into());
            }
            "__thaw_date_parse" => {
                let [text] = args else {
                    return Err("Date.parse expects one operand".into());
                };
                let text = self.compile_expr(text)?;
                return self
                    .builder
                    .build_call(
                        self.module.get_function("thaw_date_parse").unwrap(),
                        &[text.into()],
                        "date_parse",
                    )
                    .map_err(|error| error.to_string())?
                    .try_as_basic_value()
                    .basic()
                    .ok_or("Date.parse returned no value".into());
            }
            "__thaw_map_new" => {
                if !args.is_empty() {
                    return Err("Map/Set construction expects no operands".into());
                }
                return self
                    .builder
                    .build_call(
                        self.module.get_function("thaw_map_new").unwrap(),
                        &[],
                        "map_new",
                    )
                    .map_err(|error| error.to_string())?
                    .try_as_basic_value()
                    .basic()
                    .ok_or("Map/Set construction returned no value".into());
            }
            "__thaw_map_snapshot_keys"
            | "__thaw_map_snapshot_values"
            | "__thaw_map_snapshot_entries"
            | "__thaw_set_snapshot_entries" => {
                let [map] = args else {
                    return Err(format!("{name} expects one operand"));
                };
                let map = self.compile_expr(map)?;
                let runtime = name.trim_start_matches("__thaw_").to_string();
                let runtime = format!("thaw_{runtime}");
                let result = self
                    .builder
                    .build_call(
                        self.module.get_function(&runtime).unwrap(),
                        &[map.into()],
                        "map_snapshot",
                    )
                    .map_err(|error| error.to_string())?
                    .try_as_basic_value()
                    .basic()
                    .ok_or_else(|| format!("{name} returned no value"))?
                    .into_pointer_value();
                return Ok(self.compile_array_wrap(result)?.into());
            }
            "__thaw_map_size" => {
                let [map] = args else {
                    return Err("Map/Set size expects one operand".into());
                };
                let map = self.compile_expr(map)?;
                return self
                    .builder
                    .build_call(
                        self.module.get_function("thaw_map_size").unwrap(),
                        &[map.into()],
                        "map_size",
                    )
                    .map_err(|error| error.to_string())?
                    .try_as_basic_value()
                    .basic()
                    .ok_or("Map/Set size returned no value".into());
            }
            "__thaw_map_clear" => {
                let [map] = args else {
                    return Err("Map/Set clear expects one operand".into());
                };
                let map = self.compile_expr(map)?;
                self.builder
                    .build_call(
                        self.module.get_function("thaw_map_clear").unwrap(),
                        &[map.into()],
                        "map_clear",
                    )
                    .map_err(|error| error.to_string())?;
                return Ok(self.context.f64_type().const_zero().into());
            }
            "__thaw_map_num_has"
            | "__thaw_map_num_delete"
            | "__thaw_map_str_has"
            | "__thaw_map_str_delete"
            | "__thaw_map_ref_has"
            | "__thaw_map_ref_delete" => {
                let [map, key] = args else {
                    return Err(format!("{name} expects two operands"));
                };
                let map = self.compile_expr(map)?;
                let key = self.compile_expr(key)?;
                let runtime = name.trim_start_matches("__thaw_").to_string();
                let runtime = format!("thaw_{runtime}");
                let result = self
                    .builder
                    .build_call(
                        self.module.get_function(&runtime).unwrap(),
                        &[map.into(), key.into()],
                        "map_bool_op",
                    )
                    .map_err(|error| error.to_string())?
                    .try_as_basic_value()
                    .basic()
                    .ok_or_else(|| format!("{name} returned no value"))?
                    .into_int_value();
                return self
                    .builder
                    .build_int_compare(
                        IntPredicate::NE,
                        result,
                        self.context.i8_type().const_zero(),
                        "map_bool_result",
                    )
                    .map(Into::into)
                    .map_err(|error| error.to_string());
            }
            "__thaw_map_num_get_f64"
            | "__thaw_map_num_get_bool"
            | "__thaw_map_num_get_ptr"
            | "__thaw_map_str_get_f64"
            | "__thaw_map_str_get_bool"
            | "__thaw_map_str_get_ptr"
            | "__thaw_map_ref_get_f64"
            | "__thaw_map_ref_get_bool"
            | "__thaw_map_ref_get_ptr" => {
                let [map, key] = args else {
                    return Err(format!("{name} expects two operands"));
                };
                let map = self.compile_expr(map)?;
                let key = self.compile_expr(key)?;
                let runtime = name.trim_start_matches("__thaw_").to_string();
                let runtime = format!("thaw_{runtime}");
                let result = self
                    .builder
                    .build_call(
                        self.module.get_function(&runtime).unwrap(),
                        &[map.into(), key.into()],
                        "map_get",
                    )
                    .map_err(|error| error.to_string())?
                    .try_as_basic_value()
                    .basic()
                    .ok_or_else(|| format!("{name} returned no value"))?;
                if name.ends_with("_get_bool") {
                    return self
                        .builder
                        .build_int_compare(
                            IntPredicate::NE,
                            result.into_int_value(),
                            self.context.i8_type().const_zero(),
                            "map_get_bool_result",
                        )
                        .map(Into::into)
                        .map_err(|error| error.to_string());
                }
                return Ok(result);
            }
            "__thaw_map_num_set" | "__thaw_map_str_set" | "__thaw_map_ref_set" => {
                let [map, key, value] = args else {
                    return Err(format!("{name} expects three operands"));
                };
                let map = self.compile_expr(map)?;
                let key = self.compile_expr(key)?;
                let value = self.compile_expr(value)?;
                let word = self.encode_word(value)?;
                let runtime = name.trim_start_matches("__thaw_").to_string();
                let runtime = format!("thaw_{runtime}");
                self.builder
                    .build_call(
                        self.module.get_function(&runtime).unwrap(),
                        &[map.into(), key.into(), word.into()],
                        "map_set",
                    )
                    .map_err(|error| error.to_string())?;
                return Ok(map);
            }
            "__thaw_regex_exec" => {
                let [source, flags, value, last_index] = args else {
                    return Err("RegExp.exec expects four operands".into());
                };
                let source = self.compile_expr(source)?;
                let flags = self.compile_expr(flags)?;
                let value = self.compile_expr(value)?;
                let last_index = self.compile_expr(last_index)?;
                let result = self
                    .builder
                    .build_call(
                        self.module.get_function("thaw_regex_exec").unwrap(),
                        &[source.into(), flags.into(), value.into(), last_index.into()],
                        "regex_exec",
                    )
                    .map_err(|error| error.to_string())?
                    .try_as_basic_value()
                    .basic()
                    .ok_or("RegExp.exec returned no value")?
                    .into_pointer_value();
                return Ok(self.compile_array_wrap_nullable(result)?.into());
            }
            "__thaw_regex_exec_advance" => {
                let [value, source, flags, last_index] = args else {
                    return Err("RegExp.exec lastIndex advance expects four operands".into());
                };
                let value = self.compile_expr(value)?;
                let source = self.compile_expr(source)?;
                let flags = self.compile_expr(flags)?;
                let last_index = self.compile_expr(last_index)?;
                return self
                    .builder
                    .build_call(
                        self.module.get_function("thaw_regex_exec_advance").unwrap(),
                        &[value.into(), source.into(), flags.into(), last_index.into()],
                        "regex_exec_advance",
                    )
                    .map_err(|error| error.to_string())?
                    .try_as_basic_value()
                    .basic()
                    .ok_or("RegExp.exec lastIndex advance returned no value".into());
            }
            "__thaw_regex_match" | "__thaw_regex_match_all" => {
                let [value, source, flags] = args else {
                    return Err(format!("{name} expects three operands"));
                };
                let value = self.compile_expr(value)?;
                let source = self.compile_expr(source)?;
                let flags = self.compile_expr(flags)?;
                let runtime = name.trim_start_matches("__thaw_").to_string();
                let runtime = format!("thaw_{runtime}");
                let result = self
                    .builder
                    .build_call(
                        self.module.get_function(&runtime).unwrap(),
                        &[value.into(), source.into(), flags.into()],
                        "regex_call",
                    )
                    .map_err(|error| error.to_string())?
                    .try_as_basic_value()
                    .basic()
                    .ok_or_else(|| format!("{name} returned no value"))?
                    .into_pointer_value();
                return Ok(self.compile_array_wrap_nullable(result)?.into());
            }
            "__thaw_regex_split" => {
                let [value, source, flags, limit] = args else {
                    return Err(format!("{name} expects four operands"));
                };
                let value = self.compile_expr(value)?;
                let source = self.compile_expr(source)?;
                let flags = self.compile_expr(flags)?;
                let limit = self.compile_expr(limit)?;
                let result = self
                    .builder
                    .build_call(
                        self.module.get_function("thaw_regex_split").unwrap(),
                        &[value.into(), source.into(), flags.into(), limit.into()],
                        "regex_call",
                    )
                    .map_err(|error| error.to_string())?
                    .try_as_basic_value()
                    .basic()
                    .ok_or_else(|| format!("{name} returned no value"))?
                    .into_pointer_value();
                return Ok(self.compile_array_wrap_nullable(result)?.into());
            }
            "__thaw_regex_replace" | "__thaw_regex_replace_all" => {
                let [value, source, flags, replacement] = args else {
                    return Err("regex replace expects four operands".into());
                };
                let value = self.compile_expr(value)?;
                let source = self.compile_expr(source)?;
                let flags = self.compile_expr(flags)?;
                let replacement = self.compile_expr(replacement)?;
                let runtime = if name == "__thaw_regex_replace" {
                    "thaw_regex_replace"
                } else {
                    "thaw_regex_replace_all"
                };
                return self
                    .builder
                    .build_call(
                        self.module.get_function(runtime).unwrap(),
                        &[value.into(), source.into(), flags.into(), replacement.into()],
                        "regex_replace",
                    )
                    .map_err(|error| error.to_string())?
                    .try_as_basic_value()
                    .basic()
                    .ok_or("regex replace returned no value".into());
            }
            "__thaw_encode_uri_component" => {
                return self.compile_single_arg_call(
                    "thaw_encode_uri_component",
                    args,
                    "encodeURIComponent",
                )
            }
            "__thaw_encode_uri" => {
                return self.compile_single_arg_call("thaw_encode_uri", args, "encodeURI")
            }
            "__thaw_decode_uri_component" => {
                return self.compile_single_arg_call(
                    "thaw_decode_uri_component",
                    args,
                    "decodeURIComponent",
                )
            }
            "__thaw_decode_uri" => {
                return self.compile_single_arg_call("thaw_decode_uri", args, "decodeURI")
            }
            "__thaw_string_is_null" => {
                let [value] = args else {
                    return Err("string null check expects one operand".into());
                };
                let value = self.compile_expr(value)?.into_pointer_value();
                return self
                    .builder
                    .build_is_null(value, "string_is_null")
                    .map(Into::into)
                    .map_err(|error| error.to_string());
            }
            "__thaw_array_is_null" => {
                let [value] = args else {
                    return Err("array null check expects one operand".into());
                };
                let value = self.compile_expr(value)?.into_pointer_value();
                return self
                    .builder
                    .build_is_null(value, "array_is_null")
                    .map(Into::into)
                    .map_err(|error| error.to_string());
            }
            "__thaw_string_repeat" => {
                let [value, count] = args else {
                    return Err("string repeat expects two operands".into());
                };
                let value = self.compile_expr(value)?;
                let count = self.compile_expr(count)?;
                return self
                    .builder
                    .build_call(
                        self.module.get_function("thaw_string_repeat").unwrap(),
                        &[value.into(), count.into()],
                        "string_repeat",
                    )
                    .map_err(|error| error.to_string())?
                    .try_as_basic_value()
                    .basic()
                    .ok_or("string repeat returned no value".into());
            }
            "__thaw_string_at" => {
                let [value, index] = args else {
                    return Err("string at expects two operands".into());
                };
                let value = self.compile_expr(value)?;
                let index = self.compile_expr(index)?;
                return self
                    .builder
                    .build_call(
                        self.module.get_function("thaw_string_at").unwrap(),
                        &[value.into(), index.into()],
                        "string_at",
                    )
                    .map_err(|error| error.to_string())?
                    .try_as_basic_value()
                    .basic()
                    .ok_or("string at returned no value".into());
            }
            "__thaw_string_pad_start" | "__thaw_string_pad_end" => {
                let [value, pad, length] = args else {
                    return Err("string pad expects three operands".into());
                };
                let value = self.compile_expr(value)?;
                let pad = self.compile_expr(pad)?;
                let length = self.compile_expr(length)?;
                let runtime = if name == "__thaw_string_pad_start" {
                    "thaw_string_pad_start"
                } else {
                    "thaw_string_pad_end"
                };
                return self
                    .builder
                    .build_call(
                        self.module.get_function(runtime).unwrap(),
                        &[value.into(), pad.into(), length.into()],
                        "string_pad",
                    )
                    .map_err(|error| error.to_string())?
                    .try_as_basic_value()
                    .basic()
                    .ok_or("string pad returned no value".into());
            }
            "__thaw_number_to_fixed" => {
                let [value, digits] = args else {
                    return Err("number toFixed expects two operands".into());
                };
                let value = self.compile_expr(value)?;
                let digits = self.compile_expr(digits)?;
                return self
                    .builder
                    .build_call(
                        self.module.get_function("thaw_number_to_fixed").unwrap(),
                        &[value.into(), digits.into()],
                        "number_to_fixed",
                    )
                    .map_err(|error| error.to_string())?
                    .try_as_basic_value()
                    .basic()
                    .ok_or("number toFixed returned no value".into());
            }
            "__thaw_number_to_precision" => {
                let [value, digits] = args else {
                    return Err("number toPrecision expects two operands".into());
                };
                let value = self.compile_expr(value)?;
                let digits = self.compile_expr(digits)?;
                return self
                    .builder
                    .build_call(
                        self.module.get_function("thaw_number_to_precision").unwrap(),
                        &[value.into(), digits.into()],
                        "number_to_precision",
                    )
                    .map_err(|error| error.to_string())?
                    .try_as_basic_value()
                    .basic()
                    .ok_or("number toPrecision returned no value".into());
            }
            "__thaw_number_to_radix_string" => {
                let [value, radix] = args else {
                    return Err("number toString radix expects two operands".into());
                };
                let value = self.compile_expr(value)?;
                let radix = self.compile_expr(radix)?;
                return self
                    .builder
                    .build_call(
                        self.module.get_function("thaw_number_to_radix_string").unwrap(),
                        &[value.into(), radix.into()],
                        "number_to_radix_string",
                    )
                    .map_err(|error| error.to_string())?
                    .try_as_basic_value()
                    .basic()
                    .ok_or("number toString radix returned no value".into());
            }
            "__thaw_string_length" => {
                return self.compile_single_arg_call("thaw_string_length", args, "string length")
            }
            "__thaw_string_char_code_at" => {
                let [value, index] = args else {
                    return Err("string charCodeAt expects two operands".to_string());
                };
                let value = self.compile_expr(value)?;
                let index = self.compile_expr(index)?;
                return self
                    .builder
                    .build_call(
                        self.module
                            .get_function("thaw_string_char_code_at")
                            .unwrap(),
                        &[value.into(), index.into()],
                        "string_char_code_at",
                    )
                    .map_err(|error| error.to_string())?
                    .try_as_basic_value()
                    .basic()
                    .ok_or("charCodeAt returned no value".to_string());
            }
            "__thaw_string_locale_compare" => {
                let [receiver, other] = args else {
                    return Err("string localeCompare expects two operands".to_string());
                };
                let receiver = self.compile_expr(receiver)?;
                let other = self.compile_expr(other)?;
                let compared = self
                    .builder
                    .build_call(
                        self.module.get_function("thaw_string_compare").unwrap(),
                        &[receiver.into(), other.into()],
                        "locale_compare",
                    )
                    .map_err(|error| error.to_string())?
                    .try_as_basic_value()
                    .basic()
                    .ok_or("localeCompare returned no value")?
                    .into_int_value();
                return self
                    .builder
                    .build_signed_int_to_float(
                        compared,
                        self.context.f64_type(),
                        "locale_compare_f64",
                    )
                    .map(Into::into)
                    .map_err(|error| error.to_string());
            }
            "__thaw_string_normalize" => {
                let [value, form] = args else {
                    return Err("string normalize expects two operands".to_string());
                };
                let value = self.compile_expr(value)?;
                let form = self.compile_expr(form)?;
                return self
                    .builder
                    .build_call(
                        self.module.get_function("thaw_string_normalize").unwrap(),
                        &[value.into(), form.into()],
                        "string_normalize",
                    )
                    .map_err(|error| error.to_string())?
                    .try_as_basic_value()
                    .basic()
                    .ok_or("normalize returned no value".into());
            }
            "__thaw_string_code_point_at" => {
                let [value, index] = args else {
                    return Err("string codePointAt expects two operands".to_string());
                };
                let value = self.compile_expr(value)?;
                let index = self.compile_expr(index)?;
                return self
                    .builder
                    .build_call(
                        self.module
                            .get_function("thaw_string_code_point_at")
                            .unwrap(),
                        &[value.into(), index.into()],
                        "string_code_point_at",
                    )
                    .map_err(|error| error.to_string())?
                    .try_as_basic_value()
                    .basic()
                    .ok_or("codePointAt returned no value".to_string());
            }
            "__thaw_object_to_string" => return self.compile_object_to_string(args),
            "__thaw_number_is_nan" => return self.compile_number_predicate(args, false),
            "__thaw_number_is_finite" => return self.compile_number_predicate(args, true),
            "__thaw_number_is_integer" => return self.compile_integer_predicate(args, false),
            "__thaw_number_is_safe_integer" => return self.compile_integer_predicate(args, true),
            "__thaw_number_object_is" => {
                let [left, right] = args else {
                    return Err("Object.is number comparison expects two operands".to_string());
                };
                let left = self.compile_expr(left)?;
                let right = self.compile_expr(right)?;
                let result = self
                    .builder
                    .build_call(
                        self.module.get_function("thaw_number_object_is").unwrap(),
                        &[left.into(), right.into()],
                        "number_object_is",
                    )
                    .map_err(|error| error.to_string())?
                    .try_as_basic_value()
                    .basic()
                    .ok_or("Object.is number comparison returned no value")?
                    .into_int_value();
                return self
                    .builder
                    .build_int_compare(
                        IntPredicate::NE,
                        result,
                        self.context.i8_type().const_zero(),
                        "number_object_is_bool",
                    )
                    .map(Into::into)
                    .map_err(|error| error.to_string());
            }
            "__thaw_number_neg" => {
                let [value] = args else {
                    return Err("unary minus expects one operand".to_string());
                };
                let value = self.compile_expr(value)?.into_float_value();
                return self
                    .builder
                    .build_float_neg(value, "number_neg")
                    .map(Into::into)
                    .map_err(|error| error.to_string());
            }
            "__thaw_math_abs" => {
                return self.compile_single_arg_call("llvm.fabs.f64", args, "Math.abs")
            }
            "__thaw_math_floor" => {
                return self.compile_single_arg_call("llvm.floor.f64", args, "Math.floor")
            }
            "__thaw_math_ceil" => {
                return self.compile_single_arg_call("llvm.ceil.f64", args, "Math.ceil")
            }
            "__thaw_math_trunc" => {
                return self.compile_single_arg_call("llvm.trunc.f64", args, "Math.trunc")
            }
            "__thaw_math_sqrt" => {
                return self.compile_single_arg_call("llvm.sqrt.f64", args, "Math.sqrt")
            }
            "__thaw_math_exp" | "__thaw_math_log" | "__thaw_math_log2" | "__thaw_math_log10"
            | "__thaw_math_sin" | "__thaw_math_cos" => {
                let operation = name.trim_start_matches("__thaw_math_");
                let intrinsic = format!("llvm.{operation}.f64");
                return self.compile_single_arg_call(
                    &intrinsic,
                    args,
                    &format!("Math.{operation}"),
                );
            }
            "__thaw_math_tan" | "__thaw_math_asin" | "__thaw_math_acos" | "__thaw_math_atan"
            | "__thaw_math_sinh" | "__thaw_math_cosh" | "__thaw_math_tanh" | "__thaw_math_cbrt"
            | "__thaw_math_acosh" | "__thaw_math_asinh" | "__thaw_math_atanh"
            | "__thaw_math_expm1" | "__thaw_math_log1p" => {
                let operation = name.trim_start_matches("__thaw_math_");
                return self.compile_single_arg_call(operation, args, &format!("Math.{operation}"));
            }
            "__thaw_math_fround" | "__thaw_math_clz32" => {
                let operation = name.trim_start_matches("__thaw_math_");
                return self.compile_single_arg_call(
                    &format!("thaw_math_{operation}"),
                    args,
                    &format!("Math.{operation}"),
                );
            }
            "__thaw_math_random" => {
                if !args.is_empty() {
                    return Err("Math.random expects no operands".to_string());
                }
                return self
                    .builder
                    .build_call(
                        self.module.get_function("thaw_math_random").unwrap(),
                        &[],
                        "math_random",
                    )
                    .map_err(|error| error.to_string())?
                    .try_as_basic_value()
                    .basic()
                    .ok_or("Math.random returned no value".to_string());
            }
            "__thaw_math_pow" => {
                let [left, right] = args else {
                    return Err("Math.pow expects two operands".to_string());
                };
                let left = self.compile_expr(left)?;
                let right = self.compile_expr(right)?;
                return self
                    .builder
                    .build_call(
                        self.module.get_function("pow").unwrap(),
                        &[left.into(), right.into()],
                        "math_pow",
                    )
                    .map_err(|error| error.to_string())?
                    .try_as_basic_value()
                    .basic()
                    .ok_or("pow returned no value".to_string());
            }
            "__thaw_math_atan2" => {
                let [left, right] = args else {
                    return Err("Math.atan2 expects two operands".to_string());
                };
                let left = self.compile_expr(left)?;
                let right = self.compile_expr(right)?;
                return self
                    .builder
                    .build_call(
                        self.module.get_function("atan2").unwrap(),
                        &[left.into(), right.into()],
                        "math_atan2",
                    )
                    .map_err(|error| error.to_string())?
                    .try_as_basic_value()
                    .basic()
                    .ok_or("atan2 returned no value".to_string());
            }
            "__thaw_math_imul" => {
                let [left, right] = args else {
                    return Err("Math.imul expects two operands".to_string());
                };
                let left = self.compile_expr(left)?;
                let right = self.compile_expr(right)?;
                return self
                    .builder
                    .build_call(
                        self.module.get_function("thaw_math_imul").unwrap(),
                        &[left.into(), right.into()],
                        "math_imul",
                    )
                    .map_err(|error| error.to_string())?
                    .try_as_basic_value()
                    .basic()
                    .ok_or("Math.imul returned no value".to_string());
            }
            "__thaw_math_min" => return self.compile_math_extreme(args, true),
            "__thaw_math_max" => return self.compile_math_extreme(args, false),
            "__thaw_math_hypot" => return self.compile_math_hypot(args),
            "__thaw_math_sign" => return self.compile_math_sign(args),
            "__thaw_math_round" => return self.compile_math_round(args),
            "fetch" => return self.compile_single_arg_call("thaw_fetch_get", args, "fetch"),
            "sleep" => return self.compile_sleep(args),
            _ => {}
        }

        if let Some(result) = self.compile_variable_call(name, callee, args) {
            return result;
        }

        let symbol = Self::llvm_symbol_for(name);
        let function = self
            .module
            .get_function(&symbol)
            .ok_or_else(|| format!("call to undeclared function `{name}`"))?;

        self.build_call_with(function, args, name)
    }

}
