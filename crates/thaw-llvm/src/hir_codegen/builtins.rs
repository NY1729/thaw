impl<'ctx> HirCompiler<'ctx> {
    /// Calls a declared extern function of shape `ptr (ptr)` with a single
    /// compiled argument -- the shared shape of `fetch`/`JSON.parse`/
    /// `JSON.stringify`.
    fn compile_single_arg_call(
        &mut self,
        fn_name: &str,
        args: &[HirExpr],
        source_name: &str,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let [arg] = args else {
            return Err(format!("`{source_name}` expects exactly one argument"));
        };
        let arg_val = self.compile_expr(arg)?;
        let function = self.module.get_function(fn_name).unwrap();
        let call = self
            .builder
            .build_call(function, &[arg_val.into()], "call")
            .map_err(|e| e.to_string())?;
        call.try_as_basic_value()
            .basic()
            .ok_or_else(|| format!("`{fn_name}` did not return a value"))
    }

    /// Like `compile_single_arg_call`, but for a function whose one
    /// argument is an array/tuple *handle* that the callee itself expects
    /// as a raw `[length][elem...]` buffer -- unwraps it first.
    fn compile_single_array_arg_call(
        &mut self,
        fn_name: &str,
        args: &[HirExpr],
        source_name: &str,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let [arg] = args else {
            return Err(format!("`{source_name}` expects exactly one argument"));
        };
        let handle = self.compile_expr(arg)?.into_pointer_value();
        let buffer = self.compile_array_data(handle)?;
        let function = self.module.get_function(fn_name).unwrap();
        let call = self
            .builder
            .build_call(function, &[buffer.into()], "call")
            .map_err(|e| e.to_string())?;
        call.try_as_basic_value()
            .basic()
            .ok_or_else(|| format!("`{fn_name}` did not return a value"))
    }

    fn compile_i8_predicate_call(
        &mut self,
        fn_name: &str,
        args: &[HirExpr],
        source_name: &str,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let arguments = args
            .iter()
            .map(|argument| self.compile_expr(argument).map(Into::into))
            .collect::<Result<Vec<_>, _>>()?;
        let result = self
            .builder
            .build_call(
                self.module.get_function(fn_name).unwrap(),
                &arguments,
                source_name,
            )
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .ok_or_else(|| format!("`{fn_name}` did not return a value"))?
            .into_int_value();
        self.builder
            .build_int_compare(
                IntPredicate::NE,
                result,
                self.context.i8_type().const_zero(),
                source_name,
            )
            .map(Into::into)
            .map_err(|error| error.to_string())
    }

    fn compile_string_concat(&mut self, args: &[HirExpr]) -> Result<BasicValueEnum<'ctx>, String> {
        let [left, right] = args else {
            return Err("string concatenation expects two operands".to_string());
        };
        let left = self.compile_expr(left)?.into_pointer_value();
        let right = self.compile_expr(right)?.into_pointer_value();
        let strlen = self.module.get_function("strlen").unwrap();
        let left_len = self
            .builder
            .build_call(strlen, &[left.into()], "left_len")
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .ok_or("strlen returned no value")?
            .into_int_value();
        let right_len = self
            .builder
            .build_call(strlen, &[right.into()], "right_len")
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .ok_or("strlen returned no value")?
            .into_int_value();
        let total = self
            .builder
            .build_int_add(left_len, right_len, "concat_len")
            .map_err(|error| error.to_string())?;
        let size = self
            .builder
            .build_int_add(
                total,
                self.context.i64_type().const_int(1, false),
                "concat_size",
            )
            .map_err(|error| error.to_string())?;
        let allocation = self
            .builder
            .build_call(
                self.module.get_function("thaw_arena_alloc").unwrap(),
                &[
                    size.into(),
                    self.context.i64_type().const_int(1, false).into(),
                ],
                "string_concat",
            )
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .ok_or("arena allocator returned no string")?
            .into_pointer_value();
        let memcpy = self.module.get_function("memcpy").unwrap();
        self.builder
            .build_call(
                memcpy,
                &[allocation.into(), left.into(), left_len.into()],
                "copy_left",
            )
            .map_err(|error| error.to_string())?;
        let right_destination = unsafe {
            self.builder
                .build_in_bounds_gep(
                    self.context.i8_type(),
                    allocation,
                    &[left_len],
                    "right_destination",
                )
                .map_err(|error| error.to_string())?
        };
        self.builder
            .build_call(
                memcpy,
                &[right_destination.into(), right.into(), right_len.into()],
                "copy_right",
            )
            .map_err(|error| error.to_string())?;
        let terminator = unsafe {
            self.builder
                .build_in_bounds_gep(
                    self.context.i8_type(),
                    allocation,
                    &[total],
                    "concat_terminator",
                )
                .map_err(|error| error.to_string())?
        };
        self.builder
            .build_store(terminator, self.context.i8_type().const_zero())
            .map_err(|error| error.to_string())?;
        Ok(allocation.into())
    }

    fn compile_bool_to_string(&mut self, args: &[HirExpr]) -> Result<BasicValueEnum<'ctx>, String> {
        let [value] = args else {
            return Err("boolean string conversion expects one operand".to_string());
        };
        let value = self.compile_expr(value)?.into_int_value();
        let true_value = self
            .builder
            .build_global_string_ptr("true", "bool_true")
            .map_err(|error| error.to_string())?
            .as_pointer_value();
        let false_value = self
            .builder
            .build_global_string_ptr("false", "bool_false")
            .map_err(|error| error.to_string())?
            .as_pointer_value();
        self.builder
            .build_select(value, true_value, false_value, "bool_string")
            .map_err(|error| error.to_string())
    }

    fn compile_bool_to_number(&mut self, args: &[HirExpr]) -> Result<BasicValueEnum<'ctx>, String> {
        let [value] = args else {
            return Err("boolean number conversion expects one operand".to_string());
        };
        let value = self.compile_expr(value)?.into_int_value();
        self.builder
            .build_select(
                value,
                self.context.f64_type().const_float(1.0),
                self.context.f64_type().const_zero(),
                "bool_number",
            )
            .map_err(|error| error.to_string())
    }

    fn compile_string_comparison(
        &mut self,
        args: &[HirExpr],
        predicate: IntPredicate,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let [left, right] = args else {
            return Err("string comparison expects two operands".to_string());
        };
        let left = self.compile_expr(left)?.into_pointer_value();
        let right = self.compile_expr(right)?.into_pointer_value();
        let compared = self
            .builder
            .build_call(
                self.module.get_function("thaw_string_compare").unwrap(),
                &[left.into(), right.into()],
                "string_compare",
            )
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .ok_or("strcmp returned no value")?
            .into_int_value();
        self.builder
            .build_int_compare(
                predicate,
                compared,
                self.context.i32_type().const_zero(),
                "string_relation",
            )
            .map(Into::into)
            .map_err(|error| error.to_string())
    }

    fn compile_object_to_string(
        &mut self,
        args: &[HirExpr],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let [value] = args else {
            return Err("object string conversion expects one operand".to_string());
        };
        let _ = self.compile_expr(value)?;
        self.builder
            .build_global_string_ptr("[object Object]", "object_string")
            .map(|value| value.as_pointer_value().into())
            .map_err(|error| error.to_string())
    }

    fn compile_number_predicate(
        &mut self,
        args: &[HirExpr],
        finite: bool,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let [value] = args else {
            return Err("number predicate expects one operand".to_string());
        };
        let value = self.compile_expr(value)?.into_float_value();
        if !finite {
            return self
                .builder
                .build_float_compare(FloatPredicate::UNO, value, value, "number_is_nan")
                .map(Into::into)
                .map_err(|error| error.to_string());
        }
        let at_most_max = self
            .builder
            .build_float_compare(
                FloatPredicate::OLE,
                value,
                self.context.f64_type().const_float(f64::MAX),
                "number_below_infinity",
            )
            .map_err(|error| error.to_string())?;
        let at_least_min = self
            .builder
            .build_float_compare(
                FloatPredicate::OGE,
                value,
                self.context.f64_type().const_float(-f64::MAX),
                "number_above_negative_infinity",
            )
            .map_err(|error| error.to_string())?;
        self.builder
            .build_and(at_most_max, at_least_min, "number_is_finite")
            .map(Into::into)
            .map_err(|error| error.to_string())
    }

    fn compile_integer_predicate(
        &mut self,
        args: &[HirExpr],
        safe: bool,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let [value] = args else {
            return Err("integer predicate expects one operand".to_string());
        };
        let value = self.compile_expr(value)?.into_float_value();
        let truncated = self
            .builder
            .build_call(
                self.module.get_function("llvm.trunc.f64").unwrap(),
                &[value.into()],
                "integer_truncated",
            )
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .ok_or("llvm.trunc returned no value")?
            .into_float_value();
        let integral = self
            .builder
            .build_float_compare(FloatPredicate::OEQ, value, truncated, "number_is_integral")
            .map_err(|error| error.to_string())?;
        let finite_upper = self
            .builder
            .build_float_compare(
                FloatPredicate::OLE,
                value,
                self.context.f64_type().const_float(f64::MAX),
                "integer_finite_upper",
            )
            .map_err(|error| error.to_string())?;
        let finite_lower = self
            .builder
            .build_float_compare(
                FloatPredicate::OGE,
                value,
                self.context.f64_type().const_float(-f64::MAX),
                "integer_finite_lower",
            )
            .map_err(|error| error.to_string())?;
        let finite = self
            .builder
            .build_and(finite_upper, finite_lower, "integer_finite")
            .map_err(|error| error.to_string())?;
        let mut result = self
            .builder
            .build_and(integral, finite, "number_is_integer")
            .map_err(|error| error.to_string())?;
        if safe {
            let safe_upper = self
                .builder
                .build_float_compare(
                    FloatPredicate::OLE,
                    value,
                    self.context.f64_type().const_float(9_007_199_254_740_991.0),
                    "safe_integer_upper",
                )
                .map_err(|error| error.to_string())?;
            let safe_lower = self
                .builder
                .build_float_compare(
                    FloatPredicate::OGE,
                    value,
                    self.context
                        .f64_type()
                        .const_float(-9_007_199_254_740_991.0),
                    "safe_integer_lower",
                )
                .map_err(|error| error.to_string())?;
            result = self
                .builder
                .build_and(result, safe_upper, "safe_integer_below_max")
                .and_then(|result| {
                    self.builder
                        .build_and(result, safe_lower, "number_is_safe_integer")
                })
                .map_err(|error| error.to_string())?;
        }
        Ok(result.into())
    }

    fn compile_math_extreme(
        &mut self,
        args: &[HirExpr],
        minimum: bool,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let mut result = self.context.f64_type().const_float(if minimum {
            f64::INFINITY
        } else {
            f64::NEG_INFINITY
        });
        let intrinsic = self
            .module
            .get_function(if minimum {
                "llvm.minimum.f64"
            } else {
                "llvm.maximum.f64"
            })
            .unwrap();
        for argument in args {
            let argument = self.compile_expr(argument)?.into_float_value();
            result = self
                .builder
                .build_call(intrinsic, &[result.into(), argument.into()], "math_extreme")
                .map_err(|error| error.to_string())?
                .try_as_basic_value()
                .basic()
                .ok_or("Math extrema intrinsic returned no value")?
                .into_float_value();
        }
        Ok(result.into())
    }

    fn compile_math_hypot(&mut self, args: &[HirExpr]) -> Result<BasicValueEnum<'ctx>, String> {
        let mut result = self.context.f64_type().const_zero();
        let hypot = self.module.get_function("hypot").unwrap();
        for argument in args {
            let argument = self.compile_expr(argument)?.into_float_value();
            result = self
                .builder
                .build_call(hypot, &[result.into(), argument.into()], "math_hypot")
                .map_err(|error| error.to_string())?
                .try_as_basic_value()
                .basic()
                .ok_or("hypot returned no value")?
                .into_float_value();
        }
        Ok(result.into())
    }

    fn compile_math_sign(&mut self, args: &[HirExpr]) -> Result<BasicValueEnum<'ctx>, String> {
        let [value] = args else {
            return Err("Math.sign expects one operand".to_string());
        };
        let value = self.compile_expr(value)?.into_float_value();
        let zero = self.context.f64_type().const_zero();
        let is_zero = self
            .builder
            .build_float_compare(FloatPredicate::OEQ, value, zero, "sign_zero")
            .map_err(|error| error.to_string())?;
        let is_nan = self
            .builder
            .build_float_compare(FloatPredicate::UNO, value, value, "sign_nan")
            .map_err(|error| error.to_string())?;
        let preserve = self
            .builder
            .build_or(is_zero, is_nan, "sign_preserve")
            .map_err(|error| error.to_string())?;
        let positive = self
            .builder
            .build_float_compare(FloatPredicate::OGT, value, zero, "sign_positive")
            .map_err(|error| error.to_string())?;
        let signed = self
            .builder
            .build_select(
                positive,
                self.context.f64_type().const_float(1.0),
                self.context.f64_type().const_float(-1.0),
                "sign_nonzero",
            )
            .map_err(|error| error.to_string())?
            .into_float_value();
        self.builder
            .build_select(preserve, value, signed, "math_sign")
            .map_err(|error| error.to_string())
    }

    fn compile_math_round(&mut self, args: &[HirExpr]) -> Result<BasicValueEnum<'ctx>, String> {
        let [value] = args else {
            return Err("Math.round expects one operand".to_string());
        };
        let value = self.compile_expr(value)?.into_float_value();
        let f64_type = self.context.f64_type();
        let zero = f64_type.const_zero();
        let floor = self
            .builder
            .build_call(
                self.module.get_function("llvm.floor.f64").unwrap(),
                &[value.into()],
                "round_floor",
            )
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .ok_or("floor returned no value")?
            .into_float_value();
        let fraction = self
            .builder
            .build_float_sub(value, floor, "round_fraction")
            .map_err(|error| error.to_string())?;
        let round_up = self
            .builder
            .build_float_compare(
                FloatPredicate::OGE,
                fraction,
                f64_type.const_float(0.5),
                "round_up",
            )
            .map_err(|error| error.to_string())?;
        let ceiling = self
            .builder
            .build_float_add(floor, f64_type.const_float(1.0), "round_ceiling")
            .map_err(|error| error.to_string())?;
        let rounded = self
            .builder
            .build_select(round_up, ceiling, floor, "round_nearest")
            .map_err(|error| error.to_string())?
            .into_float_value();
        let negative = self
            .builder
            .build_float_compare(FloatPredicate::OLT, value, zero, "round_negative")
            .map_err(|error| error.to_string())?;
        let at_least_half = self
            .builder
            .build_float_compare(
                FloatPredicate::OGE,
                value,
                f64_type.const_float(-0.5),
                "round_at_least_negative_half",
            )
            .map_err(|error| error.to_string())?;
        let negative_zero_range = self
            .builder
            .build_and(negative, at_least_half, "round_negative_zero_range")
            .map_err(|error| error.to_string())?;
        self.builder
            .build_select(
                negative_zero_range,
                f64_type.const_float(-0.0),
                rounded,
                "math_round",
            )
            .map_err(|error| error.to_string())
    }

}
