impl<'ctx> HirCompiler<'ctx> {
    fn compile_text_named_call(
        &mut self,
        name: &str,
        args: &[HirExpr],
    ) -> Option<Result<BasicValueEnum<'ctx>, String>> {
        let text_call = name.starts_with("__thaw_string_")
            || name.starts_with("__thaw_regex_")
            || name.starts_with("__thaw_encode_uri")
            || name.starts_with("__thaw_decode_uri")
            || matches!(
                name,
                "__thaw_bool_to_string"
                    | "__thaw_number_to_string"
                    | "__thaw_bool_to_number"
                    | "__thaw_parse_float"
                    | "__thaw_parse_int"
                    | "__thaw_array_is_null"
                    | "__thaw_number_to_fixed"
                    | "__thaw_number_to_precision"
                    | "__thaw_number_to_radix_string"
            );
        if !text_call {
            return None;
        }
        Some(self.compile_text_call(name, args))
    }

    fn compile_text_call(
        &mut self,
        name: &str,
        args: &[HirExpr],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        match name {
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
                    name,
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
            "__thaw_regex_exec_groups" => {
                let [matches] = args else {
                    return Err("RegExp groups expects one operand".into());
                };
                let matches = self.compile_expr(matches)?;
                return self
                    .builder
                    .build_call(
                        self.module.get_function("thaw_regex_exec_groups").unwrap(),
                        &[matches.into()],
                        "regex_exec_groups",
                    )
                    .map_err(|error| error.to_string())?
                    .try_as_basic_value()
                    .basic()
                    .ok_or("RegExp groups returned no value".into());
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
            _ => {}
        }
        unreachable!("text call name was checked before dispatch")
    }
}
