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
        if let Some(result) = self.compile_text_named_call(name, args) {
            return result;
        }

        match name.as_str() {
            "console.log" | "console.info" | "console.debug" => {
                return self.compile_console_log(args, false)
            }
            "console.warn" | "console.error" => return self.compile_console_log(args, true),
            "console.assert" => return self.compile_console_assert(args),
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
            "__thaw_error_message"
            | "__thaw_error_name"
            | "__thaw_error_cause"
            | "__thaw_error_code"
            | "__thaw_error_to_string" => {
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
            "__thaw_i64_to_string" => {
                let [value] = args else {
                    return Err("bigint string conversion expects one operand".into());
                };
                let value = self.compile_expr(value)?;
                return self
                    .builder
                    .build_call(
                        self.module.get_function("thaw_i64_to_string").unwrap(),
                        &[value.into()],
                        "bigint_string",
                    )
                    .map_err(|error| error.to_string())?
                    .try_as_basic_value()
                    .basic()
                    .ok_or("bigint string conversion returned no value".into());
            }
            "__thaw_symbol_new" | "__thaw_symbol_to_string" | "__thaw_symbol_key" => {
                let [value] = args else {
                    return Err(format!("{name} expects one operand"));
                };
                let value = self.compile_expr(value)?;
                let runtime = name.trim_start_matches("__thaw_");
                return self
                    .builder
                    .build_call(
                        self.module.get_function(&format!("thaw_{runtime}")).unwrap(),
                        &[value.into()],
                        "symbol_call",
                    )
                    .map_err(|error| error.to_string())?
                    .try_as_basic_value()
                    .basic()
                    .ok_or("symbol call returned no value".into());
            }
            "__thaw_js_handle_to_string" => {
                let [value] = args else {
                    return Err("JsValue string conversion expects one operand".into());
                };
                let value = self.compile_expr(value)?;
                return self
                    .builder
                    .build_call(
                        self.module
                            .get_function("thaw_js_handle_to_string")
                            .unwrap(),
                        &[value.into()],
                        "js_value_to_string",
                    )
                    .map_err(|error| error.to_string())?
                    .try_as_basic_value()
                    .basic()
                    .ok_or("JsValue string conversion returned no value".into());
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
            "__thaw_detach_promise" | "__thaw_detach_rejection" => {
                let [promise] = args else {
                    return Err("detach Promise expects one operand".into());
                };
                let promise = self.compile_expr(promise)?;
                let pending = if name == "__thaw_detach_rejection" {
                    self.pending_rejection()
                } else {
                    self.pending_exception()
                };
                return self
                    .builder
                    .build_call(
                        self.module.get_function("thaw_promise_detach").unwrap(),
                        &[
                            promise.into(),
                            pending.as_pointer_value().into(),
                        ],
                        "detach_promise",
                    )
                    .map_err(|error| error.to_string())?
                    .try_as_basic_value()
                    .basic()
                    .ok_or("thaw_promise_detach returned no value".into());
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
