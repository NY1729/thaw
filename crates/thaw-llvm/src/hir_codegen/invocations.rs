impl<'ctx> HirCompiler<'ctx> {
    fn compile_call(
        &mut self,
        callee: &HirExpr,
        args: &[HirExpr],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let HirExpr::Var(name) = callee else {
            if let HirExpr::Lambda(_, params, ret, _) = callee {
                let parameter_types = params
                    .iter()
                    .map(|parameter| parameter.ty.clone())
                    .collect::<Vec<_>>();
                return self.compile_closure_call(
                    callee,
                    &parameter_types,
                    ret,
                    args,
                    "inline closure",
                );
            }
            if let HirExpr::PropAccess(_, HirType::Object(fields), field) = callee {
                if let Some((_, HirType::Function(params, ret))) =
                    fields.iter().find(|(name, _)| name == field)
                {
                    return self.compile_closure_call(
                        callee,
                        params,
                        ret,
                        args,
                        &format!("method `{field}`"),
                    );
                }
            }
            if let Some(HirType::Function(params, ret)) = self.expr_hir_type(callee) {
                return self.compile_closure_call(
                    callee,
                    &params,
                    ret.as_ref(),
                    args,
                    "function expression",
                );
            }
            if let Some(HirType::CallableFunction(mut params, _, rest, ret)) =
                self.expr_hir_type(callee)
            {
                if let Some(rest) = rest {
                    params.push(HirType::Array(rest));
                }
                return self.compile_closure_call(
                    callee,
                    &params,
                    ret.as_ref(),
                    args,
                    "callable function expression",
                );
            }
            return Err("call target is not a compiled function value".to_string());
        };

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
            "__thaw_number_array_to_string" => {
                return self.compile_single_arg_call(
                    "thaw_number_array_to_string",
                    args,
                    "String(number[])",
                )
            }
            "__thaw_string_array_to_string" => {
                return self.compile_single_arg_call(
                    "thaw_string_array_to_string",
                    args,
                    "String(string[])",
                )
            }
            "__thaw_bool_array_to_string" => {
                return self.compile_single_arg_call(
                    "thaw_bool_array_to_string",
                    args,
                    "String(boolean[])",
                )
            }
            "__thaw_object_array_to_string" => {
                return self.compile_single_arg_call(
                    "thaw_object_array_to_string",
                    args,
                    "String(object[])",
                )
            }
            "__thaw_number_array_join"
            | "__thaw_string_array_join"
            | "__thaw_bool_array_join"
            | "__thaw_object_array_join" => {
                let runtime = name.trim_start_matches("__thaw_");
                let runtime = format!("thaw_{runtime}");
                let [array, separator] = args else {
                    return Err("array join expects an array and separator".to_string());
                };
                let array = self.compile_expr(array)?;
                let separator = self.compile_expr(separator)?;
                return self
                    .builder
                    .build_call(
                        self.module.get_function(&runtime).unwrap(),
                        &[array.into(), separator.into()],
                        "array_join",
                    )
                    .map_err(|error| error.to_string())?
                    .try_as_basic_value()
                    .basic()
                    .ok_or("array join returned no value".to_string());
            }
            "__thaw_array_reverse" => {
                let [array] = args else {
                    return Err("array reverse expects one operand".to_string());
                };
                let Some(HirType::Array(element)) = self.expr_hir_type(array) else {
                    return Err("array reverse requires a homogeneous array".to_string());
                };
                let array = self.compile_expr(array)?;
                let width = self
                    .context
                    .i64_type()
                    .const_int(array_element_storage_bytes(&element), false);
                return self
                    .builder
                    .build_call(
                        self.module.get_function("thaw_array_reverse").unwrap(),
                        &[array.into(), width.into()],
                        "array_reverse",
                    )
                    .map_err(|error| error.to_string())?
                    .try_as_basic_value()
                    .basic()
                    .ok_or("array reverse returned no value".to_string());
            }
            "__thaw_array_copy_within" => {
                if args.len() != 4 {
                    return Err("array copyWithin expects four operands".to_string());
                }
                let Some(HirType::Array(element)) = self.expr_hir_type(&args[0]) else {
                    return Err("array copyWithin requires a homogeneous array".to_string());
                };
                let mut arguments = Vec::with_capacity(5);
                arguments.push(self.compile_expr(&args[0])?.into());
                arguments.push(
                    self.context
                        .i64_type()
                        .const_int(array_element_storage_bytes(&element), false)
                        .into(),
                );
                for argument in &args[1..] {
                    arguments.push(self.compile_expr(argument)?.into());
                }
                return self
                    .builder
                    .build_call(
                        self.module.get_function("thaw_array_copy_within").unwrap(),
                        &arguments,
                        "array_copy_within",
                    )
                    .map_err(|error| error.to_string())?
                    .try_as_basic_value()
                    .basic()
                    .ok_or("array copyWithin returned no value".to_string());
            }
            "__thaw_number_array_fill" | "__thaw_pointer_array_fill" | "__thaw_bool_array_fill" => {
                if args.len() != 4 {
                    return Err("array fill expects four operands".to_string());
                }
                let mut arguments = Vec::with_capacity(4);
                for (index, argument) in args.iter().enumerate() {
                    let mut value = self.compile_expr(argument)?;
                    if index == 1 && name == "__thaw_bool_array_fill" {
                        value = self
                            .builder
                            .build_int_z_extend(
                                value.into_int_value(),
                                self.context.i8_type(),
                                "array_fill_bool",
                            )
                            .map_err(|error| error.to_string())?
                            .into();
                    }
                    arguments.push(value.into());
                }
                let runtime = name.trim_start_matches("__thaw_");
                return self
                    .builder
                    .build_call(
                        self.module
                            .get_function(&format!("thaw_{runtime}"))
                            .unwrap(),
                        &arguments,
                        "array_fill",
                    )
                    .map_err(|error| error.to_string())?
                    .try_as_basic_value()
                    .basic()
                    .ok_or("array fill returned no value".to_string());
            }
            "__thaw_array_fill" => {
                if args.len() != 4 {
                    return Err("array fill expects four operands".to_string());
                }
                let Some(HirType::Array(element)) = self.expr_hir_type(&args[0]) else {
                    return Err("array fill requires a homogeneous array".to_string());
                };
                let array = self.compile_expr(&args[0])?;
                let value = self.compile_expr(&args[1])?;
                let value_slot = self
                    .builder
                    .build_alloca(value.get_type(), "array_fill_value")
                    .map_err(|error| error.to_string())?;
                self.builder
                    .build_store(value_slot, value)
                    .map_err(|error| error.to_string())?;
                let value_bytes = self
                    .builder
                    .build_pointer_cast(
                        value_slot,
                        self.context.ptr_type(AddressSpace::default()),
                        "array_fill_value_bytes",
                    )
                    .map_err(|error| error.to_string())?;
                let arguments = [
                    array.into(),
                    value_bytes.into(),
                    self.context
                        .i64_type()
                        .const_int(array_element_storage_bytes(&element), false)
                        .into(),
                    self.compile_expr(&args[2])?.into(),
                    self.compile_expr(&args[3])?.into(),
                ];
                return self
                    .builder
                    .build_call(
                        self.module.get_function("thaw_array_fill").unwrap(),
                        &arguments,
                        "array_fill",
                    )
                    .map_err(|error| error.to_string())?
                    .try_as_basic_value()
                    .basic()
                    .ok_or("array fill returned no value".to_string());
            }
            "__thaw_array_slice" => {
                if args.len() != 3 {
                    return Err("array slice expects three operands".to_string());
                }
                let Some(HirType::Array(element)) = self.expr_hir_type(&args[0]) else {
                    return Err("array slice requires a homogeneous array".to_string());
                };
                let mut arguments = Vec::with_capacity(4);
                arguments.push(self.compile_expr(&args[0])?.into());
                arguments.push(
                    self.context
                        .i64_type()
                        .const_int(array_element_storage_bytes(&element), false)
                        .into(),
                );
                for argument in &args[1..] {
                    arguments.push(self.compile_expr(argument)?.into());
                }
                return self
                    .builder
                    .build_call(
                        self.module.get_function("thaw_array_slice").unwrap(),
                        &arguments,
                        "array_slice",
                    )
                    .map_err(|error| error.to_string())?
                    .try_as_basic_value()
                    .basic()
                    .ok_or("array slice returned no value".to_string());
            }
            "__thaw_array_to_reversed" => {
                let [array] = args else {
                    return Err("array toReversed expects one operand".to_string());
                };
                let Some(HirType::Array(element)) = self.expr_hir_type(array) else {
                    return Err("array toReversed requires a homogeneous array".to_string());
                };
                let array = self.compile_expr(array)?;
                let width = self
                    .context
                    .i64_type()
                    .const_int(array_element_storage_bytes(&element), false);
                return self
                    .builder
                    .build_call(
                        self.module.get_function("thaw_array_to_reversed").unwrap(),
                        &[array.into(), width.into()],
                        "array_to_reversed",
                    )
                    .map_err(|error| error.to_string())?
                    .try_as_basic_value()
                    .basic()
                    .ok_or("array toReversed returned no value".to_string());
            }
            "__thaw_number_array_sort"
            | "__thaw_string_array_sort"
            | "__thaw_bool_array_sort"
            | "__thaw_object_array_sort"
            | "__thaw_number_array_to_sorted"
            | "__thaw_string_array_to_sorted"
            | "__thaw_bool_array_to_sorted"
            | "__thaw_object_array_to_sorted" => {
                let runtime = format!("thaw_{}", name.trim_start_matches("__thaw_"));
                return self.compile_single_arg_call(&runtime, args, "array sort");
            }
            "__thaw_number_array_index_of"
            | "__thaw_number_array_includes"
            | "__thaw_string_array_index_of"
            | "__thaw_string_array_includes"
            | "__thaw_bool_array_index_of"
            | "__thaw_bool_array_includes"
            | "__thaw_object_array_index_of"
            | "__thaw_object_array_includes"
            | "__thaw_number_array_last_index_of"
            | "__thaw_string_array_last_index_of"
            | "__thaw_bool_array_last_index_of"
            | "__thaw_object_array_last_index_of" => {
                let runtime = format!("thaw_{}", name.trim_start_matches("__thaw_"));
                if args.len() != 3 {
                    return Err("array search expects three operands".to_string());
                }
                let mut arguments = Vec::with_capacity(3);
                for (index, argument) in args.iter().enumerate() {
                    let mut value = self.compile_expr(argument)?;
                    if index == 1 && name.starts_with("__thaw_bool_array_") {
                        value = self
                            .builder
                            .build_int_z_extend(
                                value.into_int_value(),
                                self.context.i8_type(),
                                "array_bool_needle",
                            )
                            .map_err(|error| error.to_string())?
                            .into();
                    }
                    arguments.push(value.into());
                }
                let result = self
                    .builder
                    .build_call(
                        self.module.get_function(&runtime).unwrap(),
                        &arguments,
                        "array_search",
                    )
                    .map_err(|error| error.to_string())?
                    .try_as_basic_value()
                    .basic()
                    .ok_or("array search returned no value".to_string())?;
                if name.ends_with("_includes") {
                    return self
                        .builder
                        .build_int_compare(
                            IntPredicate::NE,
                            result.into_int_value(),
                            self.context.i8_type().const_zero(),
                            "array_includes_bool",
                        )
                        .map(Into::into)
                        .map_err(|error| error.to_string());
                }
                return Ok(result);
            }
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
            "__thaw_string_trim"
            | "__thaw_string_trim_start"
            | "__thaw_string_trim_end"
            | "__thaw_string_to_lower_case"
            | "__thaw_string_to_upper_case" => {
                let runtime = format!("thaw_{}", name.trim_start_matches("__thaw_"));
                return self.compile_single_arg_call(&runtime, args, "string transform");
            }
            "__thaw_string_to_array" => {
                return self.compile_single_arg_call(
                    "thaw_string_to_array",
                    args,
                    "string iterator array",
                )
            }
            "__thaw_string_split" => {
                let [value, separator, limit] = args else {
                    return Err("string split expects three operands".into());
                };
                let value = self.compile_expr(value)?;
                let separator = self.compile_expr(separator)?;
                let limit = self.compile_expr(limit)?;
                return self
                    .builder
                    .build_call(
                        self.module.get_function("thaw_string_split").unwrap(),
                        &[value.into(), separator.into(), limit.into()],
                        "string_split",
                    )
                    .map_err(|error| error.to_string())?
                    .try_as_basic_value()
                    .basic()
                    .ok_or("string split returned no value".into());
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
            "__thaw_date_to_iso_string" => {
                let [timestamp] = args else {
                    return Err("Date.toISOString expects one operand".into());
                };
                let timestamp = self.compile_expr(timestamp)?;
                return self
                    .builder
                    .build_call(
                        self.module.get_function("thaw_date_to_iso_string").unwrap(),
                        &[timestamp.into()],
                        "date_to_iso_string",
                    )
                    .map_err(|error| error.to_string())?
                    .try_as_basic_value()
                    .basic()
                    .ok_or("Date.toISOString returned no value".into());
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
            "__thaw_regex_exec" => {
                let [source, flags, value] = args else {
                    return Err("RegExp.exec expects three operands".into());
                };
                let source = self.compile_expr(source)?;
                let flags = self.compile_expr(flags)?;
                let value = self.compile_expr(value)?;
                return self
                    .builder
                    .build_call(
                        self.module.get_function("thaw_regex_exec").unwrap(),
                        &[source.into(), flags.into(), value.into()],
                        "regex_exec",
                    )
                    .map_err(|error| error.to_string())?
                    .try_as_basic_value()
                    .basic()
                    .ok_or("RegExp.exec returned no value".into());
            }
            "__thaw_regex_match" | "__thaw_regex_split" => {
                let [value, source, flags] = args else {
                    return Err("regex match/split expects three operands".into());
                };
                let value = self.compile_expr(value)?;
                let source = self.compile_expr(source)?;
                let flags = self.compile_expr(flags)?;
                let runtime = if name == "__thaw_regex_match" {
                    "thaw_regex_match"
                } else {
                    "thaw_regex_split"
                };
                return self
                    .builder
                    .build_call(
                        self.module.get_function(runtime).unwrap(),
                        &[value.into(), source.into(), flags.into()],
                        "regex_call",
                    )
                    .map_err(|error| error.to_string())?
                    .try_as_basic_value()
                    .basic()
                    .ok_or("regex match/split returned no value".into());
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
            "JSON.parse" => {
                return self.compile_single_arg_call("thaw_json_parse", args, "JSON.parse")
            }
            "JSON.stringify" => {
                return self.compile_single_arg_call("thaw_json_stringify", args, "JSON.stringify")
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
                let keys = self.compile_expr(keys)?;
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
                let keys = self.compile_expr(keys)?;
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
                return self.compile_single_arg_call("thaw_json_keys", args, "Object.keys")
            }
            "__thaw_array_keys" => {
                return self.compile_single_arg_call("thaw_array_keys", args, "Object.keys")
            }
            "__thaw_json_values" => {
                return self.compile_single_arg_call("thaw_json_values", args, "Object.values")
            }
            "__thaw_json_number_values"
            | "__thaw_json_string_values"
            | "__thaw_json_bool_values" => {
                return self.compile_single_arg_call(
                    name.trim_start_matches("__"),
                    args,
                    "Object.values",
                )
            }
            "__thaw_json_entries" => {
                return self.compile_single_arg_call("thaw_json_entries", args, "Object.entries")
            }
            "__thaw_json_number_entries"
            | "__thaw_json_string_entries"
            | "__thaw_json_bool_entries" => {
                return self.compile_single_arg_call(
                    name.trim_start_matches("__"),
                    args,
                    "Object.entries",
                )
            }
            "__thaw_json_object_from_number_entries"
            | "__thaw_json_object_from_string_entries"
            | "__thaw_json_object_from_bool_entries"
            | "__thaw_json_object_from_json_entries" => {
                return self.compile_single_arg_call(
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
            "loadScript" => return self.compile_load_script(args),
            "callDynamic" => return self.compile_call_dynamic(args),
            "getDynamicValue" => return self.compile_get_dynamic_value(args),
            "callDynamicValue" => return self.compile_call_dynamic_value(args),
            "callDynamicValueHandle" => return self.compile_call_dynamic_value_handle(args),
            "callDynamicValueWithValue" => return self.compile_call_dynamic_value_with_value(args),
            "releaseDynamicValue" => return self.compile_release_dynamic_value(args),
            "getDynamicProperty" => {
                return self.compile_dynamic_handle_operation("thaw_js_get_property_result", args)
            }
            "setDynamicProperty" => {
                let value = self
                    .compile_dynamic_handle_operation("thaw_js_set_property_result", args)?
                    .into_int_value();
                return self
                    .builder
                    .build_int_compare(
                        IntPredicate::NE,
                        value,
                        self.context.i64_type().const_zero(),
                        "dynamic_property_set",
                    )
                    .map(Into::into)
                    .map_err(|error| error.to_string());
            }
            "callDynamicMethod" => return self.compile_call_dynamic_method(args),
            "readDynamicValue" => return self.compile_read_dynamic_value(args),
            "callDynamicValueMixed" => return self.compile_call_dynamic_value_mixed(args),
            "constructDynamicValue" => return self.compile_construct_dynamic_value(args),
            "loadNativeAddon" => return self.compile_load_native_addon(args),
            "loadNativeAddonEmbedded" => return self.compile_load_embedded_native_addon(args),
            "callNativeAddon" => return self.compile_call_native_addon(args),
            "callNativeAddonWithCallback" => {
                return self.compile_call_native_addon_with_callback(args)
            }
            "pollNativeAddonEvents" => return self.compile_poll_native_addon_events(args),
            _ => {}
        }

        if let Some(HirType::Function(params, ret)) = self.variable_hir_types.get(name).cloned() {
            return self.compile_closure_call(callee, &params, &ret, args, name);
        }
        if let Some(HirType::CallableFunction(mut params, _, rest, ret)) =
            self.variable_hir_types.get(name).cloned()
        {
            if let Some(rest) = rest {
                params.push(HirType::Array(rest));
            }
            return self.compile_closure_call(callee, &params, &ret, args, name);
        }

        let symbol = Self::llvm_symbol_for(name);
        let function = self
            .module
            .get_function(&symbol)
            .ok_or_else(|| format!("call to undeclared function `{name}`"))?;

        self.build_call_with(function, args, name)
    }

    fn compile_this_argument_word(
        &mut self,
        expression: &HirExpr,
    ) -> Result<IntValue<'ctx>, String> {
        let value = self.compile_expr(expression)?;
        match value {
            BasicValueEnum::FloatValue(value) => self
                .builder
                .build_bit_cast(value, self.context.i64_type(), "this_number_word")
                .map(|value| value.into_int_value())
                .map_err(|error| error.to_string()),
            BasicValueEnum::IntValue(value) => {
                let width = value.get_type().get_bit_width();
                if width < 64 {
                    self.builder
                        .build_int_z_extend(value, self.context.i64_type(), "this_int_word")
                        .map_err(|error| error.to_string())
                } else if width > 64 {
                    self.builder
                        .build_int_truncate(value, self.context.i64_type(), "this_int_word")
                        .map_err(|error| error.to_string())
                } else {
                    Ok(value)
                }
            }
            BasicValueEnum::PointerValue(value) => self
                .builder
                .build_ptr_to_int(value, self.context.i64_type(), "this_pointer_word")
                .map_err(|error| error.to_string()),
            other => Err(format!(
                "explicit thisArg has unsupported native representation {other:?}"
            )),
        }
    }

    fn compile_function_call_with_this(
        &mut self,
        callee: &HirExpr,
        this_arg: &HirExpr,
        args: &[HirExpr],
        params: &[HirType],
        ret: &HirType,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let closure = self.compile_expr(callee)?.into_pointer_value();
        let this_entry_slot = unsafe {
            self.builder
                .build_in_bounds_gep(
                    self.context.i8_type(),
                    closure,
                    &[self
                        .context
                        .i64_type()
                        .const_int(CLOSURE_THIS_ENTRY_OFFSET, false)],
                    "closure_this_entry_slot",
                )
                .map_err(|error| error.to_string())?
        };
        let function_pointer = self
            .builder
            .build_load(
                self.context.ptr_type(AddressSpace::default()),
                this_entry_slot,
                "closure_this_entry",
            )
            .map_err(|error| error.to_string())?
            .into_pointer_value();
        let this_word = self.compile_this_argument_word(this_arg)?;
        let mut compiled_args = vec![
            BasicMetadataValueEnum::from(closure),
            BasicMetadataValueEnum::from(this_word),
        ];
        compiled_args.extend(
            args.iter()
                .map(|arg| self.compile_expr(arg).map(BasicMetadataValueEnum::from))
                .collect::<Result<Vec<_>, _>>()?,
        );
        let mut this_params = Vec::with_capacity(params.len() + 1);
        this_params.push(HirType::I64);
        this_params.extend_from_slice(params);
        let call = self
            .builder
            .build_indirect_call(
                self.function_type(&this_params, ret)?,
                function_pointer,
                &compiled_args,
                "closure_call_with_this",
            )
            .map_err(|error| error.to_string())?;
        let value = if *ret == HirType::Void {
            self.context.f64_type().const_zero().into()
        } else {
            call.try_as_basic_value()
                .basic()
                .ok_or("this-aware function value returned no value")?
        };
        self.branch_on_pending_exception()?;
        Ok(value)
    }

    fn compile_function_bind_this(
        &mut self,
        callee: &HirExpr,
        this_arg: &HirExpr,
        bound: &[HirExpr],
        params: &[HirType],
        ret: &HirType,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        if bound.len() > params.len() {
            return Err("bound function has more leading arguments than parameters".into());
        }
        let remaining = &params[bound.len()..];
        let name = format!("__thaw_bound_function_{}", self.next_lambda);
        self.next_lambda += 1;
        let code = self.module.add_function(
            &name,
            self.function_type(remaining, ret)?,
            Some(Linkage::Internal),
        );
        let parent = self.builder.get_insert_block().unwrap();
        let entry = self.context.append_basic_block(code, "entry");
        self.builder.position_at_end(entry);
        let environment = code.get_nth_param(0).unwrap().into_pointer_value();
        let i64_type = self.context.i64_type();
        let load_slot = |compiler: &mut Self, offset: u64, label: &str| unsafe {
            compiler
                .builder
                .build_in_bounds_gep(
                    compiler.context.i8_type(),
                    environment,
                    &[i64_type.const_int(offset, false)],
                    label,
                )
                .map_err(|error| error.to_string())
        };
        let source_slot = load_slot(self, BOUND_CLOSURE_SOURCE_OFFSET, "bound_source_slot")?;
        let source = self
            .builder
            .build_load(
                self.context.ptr_type(AddressSpace::default()),
                source_slot,
                "bound_source",
            )
            .map_err(|error| error.to_string())?
            .into_pointer_value();
        let source_this_slot = unsafe {
            self.builder
                .build_in_bounds_gep(
                    self.context.i8_type(),
                    source,
                    &[i64_type.const_int(CLOSURE_THIS_ENTRY_OFFSET, false)],
                    "bound_source_this_slot",
                )
                .map_err(|error| error.to_string())?
        };
        let source_this_entry = self
            .builder
            .build_load(
                self.context.ptr_type(AddressSpace::default()),
                source_this_slot,
                "bound_source_this_entry",
            )
            .map_err(|error| error.to_string())?
            .into_pointer_value();
        let this_slot = load_slot(self, BOUND_CLOSURE_THIS_OFFSET, "bound_this_slot")?;
        let this_word = self
            .builder
            .build_load(i64_type, this_slot, "bound_this")
            .map_err(|error| error.to_string())?;
        let mut arguments = vec![
            BasicMetadataValueEnum::from(source),
            BasicMetadataValueEnum::from(this_word),
        ];
        let mut offset = BOUND_CLOSURE_ARGUMENT_BASE;
        for (index, ty) in params[..bound.len()].iter().enumerate() {
            let slot = load_slot(self, offset, &format!("bound_argument_{index}_slot"))?;
            let value = self
                .builder
                .build_load(
                    self.basic_type(ty)?,
                    slot,
                    &format!("bound_argument_{index}"),
                )
                .map_err(|error| error.to_string())?;
            arguments.push(value.into());
            offset += object_field_storage_bytes(ty);
        }
        arguments.extend(
            code.get_param_iter()
                .skip(1)
                .map(BasicMetadataValueEnum::from),
        );
        let mut source_params = Vec::with_capacity(params.len() + 1);
        source_params.push(HirType::I64);
        source_params.extend_from_slice(params);
        let call = self
            .builder
            .build_indirect_call(
                self.function_type(&source_params, ret)?,
                source_this_entry,
                &arguments,
                "invoke_bound_function",
            )
            .map_err(|error| error.to_string())?;
        if *ret == HirType::Void {
            self.builder
                .build_return(None)
                .map_err(|error| error.to_string())?;
        } else {
            let value = call
                .try_as_basic_value()
                .basic()
                .ok_or("bound function returned no value")?;
            self.builder
                .build_return(Some(&value))
                .map_err(|error| error.to_string())?;
        }
        let ignored_this = self.compile_ignored_this_adapter(
            code,
            remaining,
            ret,
            &format!("{name}__thaw_this_adapter"),
        )?;
        self.builder.position_at_end(parent);
        let source = self.compile_expr(callee)?.into_pointer_value();
        let this_word = self.compile_this_argument_word(this_arg)?;
        let bound_values = bound
            .iter()
            .map(|value| self.compile_expr(value))
            .collect::<Result<Vec<_>, _>>()?;
        let payload_bytes = params[..bound.len()]
            .iter()
            .map(object_field_storage_bytes)
            .sum::<u64>();
        let closure = self
            .builder
            .build_call(
                self.module.get_function("thaw_arena_alloc").unwrap(),
                &[
                    i64_type
                        .const_int(BOUND_CLOSURE_ARGUMENT_BASE + payload_bytes, false)
                        .into(),
                    i64_type.const_int(8, false).into(),
                ],
                "bound_function_closure",
            )
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .ok_or("bound closure allocation returned no value")?
            .into_pointer_value();
        self.builder
            .build_store(closure, code.as_global_value().as_pointer_value())
            .map_err(|error| error.to_string())?;
        let slot = |compiler: &mut Self, offset: u64, label: &str| unsafe {
            compiler
                .builder
                .build_in_bounds_gep(
                    compiler.context.i8_type(),
                    closure,
                    &[i64_type.const_int(offset, false)],
                    label,
                )
                .map_err(|error| error.to_string())
        };
        let this_entry_slot = slot(self, CLOSURE_THIS_ENTRY_OFFSET, "bound_this_entry_slot")?;
        self.builder
            .build_store(
                this_entry_slot,
                ignored_this.as_global_value().as_pointer_value(),
            )
            .map_err(|error| error.to_string())?;
        let source_slot = slot(self, BOUND_CLOSURE_SOURCE_OFFSET, "bound_source_store")?;
        self.builder
            .build_store(source_slot, source)
            .map_err(|error| error.to_string())?;
        let bound_this_slot = slot(self, BOUND_CLOSURE_THIS_OFFSET, "bound_this_store")?;
        self.builder
            .build_store(bound_this_slot, this_word)
            .map_err(|error| error.to_string())?;
        let mut offset = BOUND_CLOSURE_ARGUMENT_BASE;
        for (index, (value, ty)) in bound_values.into_iter().zip(params).enumerate() {
            let argument_slot = slot(self, offset, &format!("bound_argument_{index}_store"))?;
            self.builder
                .build_store(argument_slot, value)
                .map_err(|error| error.to_string())?;
            offset += object_field_storage_bytes(ty);
        }
        Ok(closure.into())
    }

    fn compile_closure_call(
        &mut self,
        callee: &HirExpr,
        params: &[HirType],
        ret: &HirType,
        args: &[HirExpr],
        name: &str,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let function_type = self.function_type(params, ret)?;
        let closure = self.compile_expr(callee)?.into_pointer_value();
        let function_pointer = self
            .builder
            .build_load(
                self.context.ptr_type(AddressSpace::default()),
                closure,
                "closure_code",
            )
            .map_err(|error| error.to_string())?
            .into_pointer_value();
        let mut compiled_args = vec![BasicMetadataValueEnum::from(closure)];
        compiled_args.extend(
            args.iter()
                .map(|arg| self.compile_expr(arg).map(BasicMetadataValueEnum::from))
                .collect::<Result<Vec<_>, _>>()?,
        );
        let call = self
            .builder
            .build_indirect_call(
                function_type,
                function_pointer,
                &compiled_args,
                "closure_call",
            )
            .map_err(|error| error.to_string())?;
        let value = if *ret == HirType::Void {
            self.context.f64_type().const_zero().into()
        } else {
            call.try_as_basic_value()
                .basic()
                .ok_or_else(|| format!("function value `{name}` does not return a value"))?
        };
        self.branch_on_pending_exception()?;
        Ok(value)
    }

    fn allocate_special_closure(
        &mut self,
        code: FunctionValue<'ctx>,
        promise: PointerValue<'ctx>,
        name: &str,
    ) -> Result<PointerValue<'ctx>, String> {
        let i64_type = self.context.i64_type();
        let closure = self
            .builder
            .build_call(
                self.module.get_function("thaw_arena_alloc").unwrap(),
                &[
                    i64_type.const_int(CLOSURE_CAPTURE_BASE + 8, false).into(),
                    i64_type.const_int(8, false).into(),
                ],
                name,
            )
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .ok_or("closure allocation returned no value")?
            .into_pointer_value();
        self.builder
            .build_store(closure, code.as_global_value().as_pointer_value())
            .map_err(|error| error.to_string())?;
        let this_entry = unsafe {
            self.builder
                .build_in_bounds_gep(
                    self.context.i8_type(),
                    closure,
                    &[i64_type.const_int(CLOSURE_THIS_ENTRY_OFFSET, false)],
                    "special_closure_this_entry",
                )
                .map_err(|error| error.to_string())?
        };
        self.builder
            .build_store(
                this_entry,
                self.context.ptr_type(AddressSpace::default()).const_null(),
            )
            .map_err(|error| error.to_string())?;
        let promise_slot = unsafe {
            self.builder
                .build_in_bounds_gep(
                    self.context.i8_type(),
                    closure,
                    &[i64_type.const_int(CLOSURE_CAPTURE_BASE, false)],
                    "promise_capture",
                )
                .map_err(|error| error.to_string())?
        };
        self.builder
            .build_store(promise_slot, promise)
            .map_err(|error| error.to_string())?;
        Ok(closure)
    }
}
