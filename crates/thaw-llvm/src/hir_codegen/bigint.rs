// Native arbitrary-precision `bigint` code generation. A `bigint` is one tagged
// `i64` handle owned by thaw-runtime (see its `bigint.rs`); compiled code never
// inspects it, every operation is an `extern "C"` call.
impl<'ctx> HirCompiler<'ctx> {
    fn bigint_call(
        &mut self,
        name: &str,
        arguments: &[inkwell::values::BasicMetadataValueEnum<'ctx>],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        self.builder
            .build_call(
                self.module.get_function(name).ok_or_else(|| format!("missing runtime function {name}"))?,
                arguments,
                "bigint_call",
            )
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .ok_or_else(|| format!("{name} returned no value"))
    }

    /// A `bigint` literal: inline constant when it fits the 62-bit inline range, else parsed at run time.
    fn compile_bigint_literal(&mut self, text: &str) -> Result<BasicValueEnum<'ctx>, String> {
        let i64_type = self.context.i64_type();
        if let Ok(value) = text.parse::<i64>() {
            if (-(1i64 << 62)..(1i64 << 62)).contains(&value) {
                return Ok(i64_type.const_int(((value << 1) | 1) as u64, false).into());
            }
        }
        let pointer = self.intern_string(text, "bigint_literal")
            .map_err(|error| error.to_string())?
            .as_pointer_value();
        self.bigint_call("thaw_i64_from_string", &[pointer.into()])
    }

    /// Throws `RangeError` when `thaw_bigint_binop_error` reports a failure code.
    fn compile_bigint_error_guard(&mut self, op_code: u64, lhs: IntValue<'ctx>, rhs: IntValue<'ctx>) -> Result<(), String> {
        let i32_type = self.context.i32_type();
        let code = self
            .bigint_call("thaw_bigint_binop_error", &[i32_type.const_int(op_code, false).into(), lhs.into(), rhs.into()])?
            .into_int_value();
        let failed = self
            .builder
            .build_int_compare(IntPredicate::NE, code, i32_type.const_zero(), "bigint_failed")
            .map_err(|error| error.to_string())?;
        let function = self.current_function();
        let error_bb = self.context.append_basic_block(function, "bigint_error");
        let continue_bb = self.context.append_basic_block(function, "bigint_ok");
        self.builder.build_conditional_branch(failed, error_bb, continue_bb).map_err(|error| error.to_string())?;
        self.builder.position_at_end(error_bb);
        let mut message = self.intern_string("Maximum BigInt size exceeded", "bigint_size_message")
            .map_err(|error| error.to_string())?.as_pointer_value();
        for (value, text) in [(2u64, "Exponent must be non-negative"), (1, "Division by zero")] {
            let text = self.intern_string(text, "bigint_error_message")
                .map_err(|error| error.to_string())?.as_pointer_value();
            let is = self.builder
                .build_int_compare(IntPredicate::EQ, code, i32_type.const_int(value, false), "bigint_code")
                .map_err(|error| error.to_string())?;
            message = self.builder.build_select(is, text, message, "bigint_message")
                .map_err(|error| error.to_string())?.into_pointer_value();
        }
        self.compile_throw_builtin_error_with("RangeError", message)?;
        self.builder.position_at_end(continue_bb);
        Ok(())
    }

    /// Every `bigint op bigint` operator; comparisons and `===` yield `i1`.
    fn compile_bigint_binop(
        &mut self,
        op: BinOp,
        lhs: IntValue<'ctx>,
        rhs: IntValue<'ctx>,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let arguments: [inkwell::values::BasicMetadataValueEnum<'ctx>; 2] = [lhs.into(), rhs.into()];
        let (function, guard) = match op {
            BinOp::Add => ("thaw_bigint_add", None),
            BinOp::Sub => ("thaw_bigint_sub", None),
            BinOp::Mul => ("thaw_bigint_mul", None),
            BinOp::Div => ("thaw_bigint_div", Some(0)),
            BinOp::Mod => ("thaw_bigint_mod", Some(1)),
            BinOp::Exp => ("thaw_bigint_pow", Some(2)),
            BinOp::BitAnd => ("thaw_bigint_and", None),
            BinOp::BitOr => ("thaw_bigint_or", None),
            BinOp::BitXor => ("thaw_bigint_xor", None),
            BinOp::LShift => ("thaw_bigint_shl", Some(3)),
            BinOp::RShift => ("thaw_bigint_shr", Some(4)),
            BinOp::ZeroFillRShift => {
                self.compile_throw_builtin_error("TypeError", "BigInts have no unsigned right shift, use >> instead")?;
                return Ok(self.context.i64_type().const_zero().into());
            }
            BinOp::Lt | BinOp::Gt | BinOp::LtEq | BinOp::GtEq | BinOp::EqEqEq => {
                let order = self.bigint_call("thaw_bigint_cmp", &arguments)?.into_int_value();
                let predicate = match op {
                    BinOp::Lt => IntPredicate::SLT,
                    BinOp::Gt => IntPredicate::SGT,
                    BinOp::LtEq => IntPredicate::SLE,
                    BinOp::GtEq => IntPredicate::SGE,
                    _ => IntPredicate::EQ,
                };
                return self
                    .builder
                    .build_int_compare(predicate, order, order.get_type().const_zero(), "bigint_compare")
                    .map(Into::into)
                    .map_err(|error| error.to_string());
            }
        };
        if let Some(code) = guard {
            self.compile_bigint_error_guard(code, lhs, rhs)?;
        }
        self.bigint_call(function, &arguments)
    }

    /// Json BigInt from a `bigint` handle, via exact decimal text.
    fn compile_json_bigint(&mut self, value: BasicValueEnum<'ctx>) -> Result<BasicValueEnum<'ctx>, String> {
        let decimal = self.bigint_call("thaw_i64_to_string", &[value.into()])?;
        self.bigint_call("thaw_json_receiver_bigint_decimal", &[decimal.into()])
    }

    /// `bigint` handle from a Json BigInt (host errors surface through the usual check).
    fn compile_json_as_bigint(&mut self, json: BasicValueEnum<'ctx>) -> Result<BasicValueEnum<'ctx>, String> {
        let decimal = self.bigint_call("thaw_json_as_bigint_decimal", &[json.into()])?;
        self.compile_check_json_host_error(json, None)?;
        self.bigint_call("thaw_i64_from_string", &[decimal.into()])
    }
}
