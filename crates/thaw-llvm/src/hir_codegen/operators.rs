impl<'ctx> HirCompiler<'ctx> {
    /// `process.env.NAME`, via libc `getenv`. Returns an empty string
    /// instead of a null pointer when the variable is unset, since Str
    /// values elsewhere (puts/printf %s) assume a valid C string.
    fn compile_env_var(&mut self, name: &str) -> Result<BasicValueEnum<'ctx>, String> {
        let name_global = self
            .builder
            .build_global_string_ptr(name, "envname")
            .map_err(|e| e.to_string())?;
        let getenv_fn = self.module.get_function("getenv").unwrap();
        let call = self
            .builder
            .build_call(
                getenv_fn,
                &[name_global.as_pointer_value().into()],
                "getenv_call",
            )
            .map_err(|e| e.to_string())?;
        let raw_ptr = call
            .try_as_basic_value()
            .basic()
            .ok_or("getenv did not return a value")?
            .into_pointer_value();

        let function = self.current_function();
        let null_bb = self.context.append_basic_block(function, "envnull");
        let notnull_bb = self.context.append_basic_block(function, "envnotnull");
        let merge_bb = self.context.append_basic_block(function, "envmerge");

        let is_null = self
            .builder
            .build_is_null(raw_ptr, "is_null")
            .map_err(|e| e.to_string())?;
        self.builder
            .build_conditional_branch(is_null, null_bb, notnull_bb)
            .map_err(|e| e.to_string())?;

        self.builder.position_at_end(null_bb);
        let empty = self
            .builder
            .build_global_string_ptr("", "envempty")
            .map_err(|e| e.to_string())?;
        self.builder
            .build_unconditional_branch(merge_bb)
            .map_err(|e| e.to_string())?;

        self.builder.position_at_end(notnull_bb);
        self.builder
            .build_unconditional_branch(merge_bb)
            .map_err(|e| e.to_string())?;

        self.builder.position_at_end(merge_bb);
        let ptr_ty = self.context.ptr_type(AddressSpace::default());
        let phi = self
            .builder
            .build_phi(ptr_ty, "envval")
            .map_err(|e| e.to_string())?;
        phi.add_incoming(&[(&empty.as_pointer_value(), null_bb), (&raw_ptr, notnull_bb)]);
        Ok(phi.as_basic_value())
    }

    fn expr_is_string(&self, expr: &HirExpr) -> bool {
        match expr {
            HirExpr::Lit(HirLit::Str(_)) | HirExpr::EnvVar(_) | HirExpr::JsonAsString(_) => true,
            HirExpr::Var(name) => self.variable_hir_types.get(name) == Some(&HirType::Str),
            HirExpr::TypedIndex(_, _, element) => element == &HirType::Str,
            HirExpr::PropAccess(_, HirType::Object(fields), field) => fields
                .iter()
                .any(|(name, ty)| name == field && ty == &HirType::Str),
            HirExpr::Assign(_, value) => self.expr_is_string(value),
            HirExpr::Call(callee, _) => match callee.as_ref() {
                HirExpr::Var(name) => {
                    self.function_return_types.get(name) == Some(&HirType::Str)
                        || matches!(
                            name.as_str(),
                            "__thaw_string_concat"
                                | "__thaw_bool_to_string"
                                | "__thaw_number_to_string"
                                | "__thaw_number_array_to_string"
                                | "__thaw_string_array_to_string"
                                | "__thaw_bool_array_to_string"
                                | "__thaw_object_array_to_string"
                                | "__thaw_object_to_string"
                                | "__thaw_number_array_join"
                                | "__thaw_string_array_join"
                                | "__thaw_bool_array_join"
                                | "__thaw_object_array_join"
                                | "__thaw_string_trim"
                                | "__thaw_string_trim_start"
                                | "__thaw_string_trim_end"
                                | "fetch"
                                | "JSON.stringify"
                        )
                }
                HirExpr::Lambda(_, _, ret, _) => ret == &HirType::Str,
                _ => false,
            },
            HirExpr::DynamicCall(signature, _) => signature.ret == HirType::Str,
            _ => false,
        }
    }

    fn expr_hir_type(&self, expr: &HirExpr) -> Option<HirType> {
        match expr {
            HirExpr::Lit(HirLit::F64(_)) => Some(HirType::F64),
            HirExpr::Lit(HirLit::Str(_)) => Some(HirType::Str),
            HirExpr::Lit(HirLit::Bool(_)) => Some(HirType::Bool),
            HirExpr::Lit(HirLit::Undefined) => Some(HirType::Undefined),
            HirExpr::Lit(HirLit::Null) => Some(HirType::Null),
            HirExpr::Var(name) => self.variable_hir_types.get(name).cloned(),
            HirExpr::Assign(_, value) => self.expr_hir_type(value),
            HirExpr::OptionalSome(_, payload) | HirExpr::OptionalNone(payload) => {
                Some(HirType::Optional(Box::new(payload.clone())))
            }
            HirExpr::OptionalIsNone(_, _) => Some(HirType::Bool),
            HirExpr::OptionalValue(_, payload) => Some(payload.clone()),
            HirExpr::NullableSome(_, payload) | HirExpr::NullableNone(payload) => {
                Some(HirType::Nullable(Box::new(payload.clone())))
            }
            HirExpr::NullableIsNone(_, _) => Some(HirType::Bool),
            HirExpr::NullableValue(_, payload) => Some(payload.clone()),
            HirExpr::NullishSome(_, payload)
            | HirExpr::NullishNull(payload)
            | HirExpr::NullishUndefined(payload) => {
                Some(HirType::Nullish(Box::new(payload.clone())))
            }
            HirExpr::NullishIsNull(_, _)
            | HirExpr::NullishIsUndefined(_, _)
            | HirExpr::NullishIsNone(_, _) => Some(HirType::Bool),
            HirExpr::NullishValue(_, payload) => Some(payload.clone()),
            HirExpr::UnionInject(_, _, elements) => Some(HirType::Union(elements.clone())),
            HirExpr::UnionTag(_, _) => Some(HirType::F64),
            HirExpr::UnionValue(_, index, elements) => elements.get(*index).cloned(),
            HirExpr::UnionMemberIsEqual(..) | HirExpr::UnionIsEqual(..) => Some(HirType::Bool),
            HirExpr::TypedIndex(_, _, element) => Some(element.clone()),
            HirExpr::PropAccess(_, HirType::Object(fields), field) => fields
                .iter()
                .find(|(name, _)| name == field)
                .map(|(_, ty)| ty.clone()),
            HirExpr::DynamicPropAccess(_, _, _, result) => Some(result.clone()),
            HirExpr::JsonObjectLit(_, element) => {
                Some(HirType::Dictionary(Box::new(element.clone())))
            }
            HirExpr::JsonKey(_, _) | HirExpr::JsonGet(_, _) | HirExpr::JsonIndex(_, _) => {
                Some(HirType::Json)
            }
            HirExpr::JsonSet(_, _, _, element) => Some(element.clone()),
            HirExpr::JsonIndexSet(_, _, _) => Some(HirType::Json),
            HirExpr::JsonDelete(_, _) => Some(HirType::Bool),
            HirExpr::EnumReverseLookup(_, _) => Some(HirType::Optional(Box::new(HirType::Str))),
            HirExpr::ArrayLit(elements) => Some(HirType::Array(Box::new(
                elements
                    .first()
                    .and_then(|element| self.expr_hir_type(element))
                    .unwrap_or(HirType::F64),
            ))),
            HirExpr::ArrayConcat(_, element) => Some(HirType::Array(Box::new(element.clone()))),
            HirExpr::ArrayAlloc(_, element) => Some(HirType::Array(Box::new(element.clone()))),
            HirExpr::ArraySetLen(_, _, element) => Some(HirType::Array(Box::new(element.clone()))),
            HirExpr::Lambda(_, params, ret, _) => Some(HirType::Function(
                params.iter().map(|param| param.ty.clone()).collect(),
                Box::new(ret.clone()),
            )),
            HirExpr::RecursiveClosure(_, ty, _) => Some(ty.clone()),
            HirExpr::TypedClosure(ty, _) => Some(ty.clone()),
            HirExpr::FunctionRef(_, params, ret) => {
                Some(HirType::Function(params.clone(), Box::new(ret.clone())))
            }
            HirExpr::MethodRef(_, _, params, ret, _) => {
                Some(HirType::Function(params.clone(), Box::new(ret.clone())))
            }
            HirExpr::Call(callee, arguments) => {
                if let HirExpr::Var(name) = callee.as_ref() {
                    match name.as_str() {
                        "JSON.parse" => return Some(HirType::Json),
                        "JSON.stringify"
                        | "__thaw_json_stringify_number_space"
                        | "__thaw_json_stringify_string_space"
                        | "__thaw_json_stringify_keys"
                        | "__thaw_json_stringify_keys_number_space"
                        | "__thaw_json_stringify_keys_string_space"
                        | "fetch" => return Some(HirType::Str),
                        _ => {}
                    }
                    if name == "__thaw_string_to_array" {
                        return Some(HirType::Array(Box::new(HirType::Str)));
                    }
                    if matches!(
                        name.as_str(),
                        "__thaw_array_reverse"
                            | "__thaw_array_copy_within"
                            | "__thaw_number_array_fill"
                            | "__thaw_pointer_array_fill"
                            | "__thaw_bool_array_fill"
                            | "__thaw_array_slice"
                            | "__thaw_array_to_reversed"
                    ) {
                        return arguments
                            .first()
                            .and_then(|argument| self.expr_hir_type(argument));
                    }
                    if let Some(ret) = self.frame_async_functions.get(name) {
                        return Some(HirType::Promise(Box::new(ret.clone())));
                    }
                    if let Some(ret) = self.function_return_types.get(name) {
                        return Some(ret.clone());
                    }
                }
                match self.expr_hir_type(callee)? {
                    HirType::Function(_, ret) => Some(*ret),
                    HirType::CallableFunction(_, _, _, ret) => Some(*ret),
                    _ => None,
                }
            }
            HirExpr::FunctionCallWithThis(_, _, _, _, ret) => Some(ret.clone()),
            HirExpr::FfiCall(signature, _) => Some(signature.ret.clone()),
            HirExpr::FunctionBindThis(_, _, bound, params, ret) => Some(HirType::Function(
                params[bound.len()..].to_vec(),
                Box::new(ret.clone()),
            )),
            HirExpr::AwaitPromise(_, resolved) => Some(resolved.clone()),
            HirExpr::ThrowValue(_, fallback) => self.expr_hir_type(fallback),
            _ => None,
        }
    }

    fn compile_binop(
        &mut self,
        op: BinOp,
        lhs: &HirExpr,
        rhs: &HirExpr,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let lhs_value = self.compile_expr(lhs)?;
        let rhs_value = self.compile_expr(rhs)?;

        if op == BinOp::EqEqEq {
            let string_operands = self.expr_is_string(lhs) && self.expr_is_string(rhs);
            return match (lhs_value, rhs_value) {
                (BasicValueEnum::FloatValue(lhs), BasicValueEnum::FloatValue(rhs)) => self
                    .builder
                    .build_float_compare(FloatPredicate::OEQ, lhs, rhs, "eqtmp")
                    .map(Into::into)
                    .map_err(|error| error.to_string()),
                (BasicValueEnum::IntValue(lhs), BasicValueEnum::IntValue(rhs)) => self
                    .builder
                    .build_int_compare(IntPredicate::EQ, lhs, rhs, "eqtmp")
                    .map(Into::into)
                    .map_err(|error| error.to_string()),
                (BasicValueEnum::PointerValue(lhs), BasicValueEnum::PointerValue(rhs)) => {
                    if string_operands {
                        let compared = self
                            .builder
                            .build_call(
                                self.module.get_function("strcmp").unwrap(),
                                &[lhs.into(), rhs.into()],
                                "strcmp",
                            )
                            .map_err(|error| error.to_string())?
                            .try_as_basic_value()
                            .basic()
                            .ok_or("strcmp returned no value")?
                            .into_int_value();
                        return self
                            .builder
                            .build_int_compare(
                                IntPredicate::EQ,
                                compared,
                                self.context.i32_type().const_zero(),
                                "string_eq",
                            )
                            .map(Into::into)
                            .map_err(|error| error.to_string());
                    }
                    self.builder
                        .build_int_compare(IntPredicate::EQ, lhs, rhs, "eqtmp")
                        .map(Into::into)
                        .map_err(|error| error.to_string())
                }
                _ => Err("strict equality operands have incompatible LLVM layouts".into()),
            };
        }

        let lhs_val = lhs_value.into_float_value();
        let rhs_val = rhs_value.into_float_value();

        match op {
            BinOp::Add => self
                .builder
                .build_float_add(lhs_val, rhs_val, "addtmp")
                .map(Into::into)
                .map_err(|e| e.to_string()),
            BinOp::Sub => self
                .builder
                .build_float_sub(lhs_val, rhs_val, "subtmp")
                .map(Into::into)
                .map_err(|e| e.to_string()),
            BinOp::Mul => self
                .builder
                .build_float_mul(lhs_val, rhs_val, "multmp")
                .map(Into::into)
                .map_err(|e| e.to_string()),
            BinOp::Div => self
                .builder
                .build_float_div(lhs_val, rhs_val, "divtmp")
                .map(Into::into)
                .map_err(|e| e.to_string()),
            BinOp::Mod => self
                .builder
                .build_float_rem(lhs_val, rhs_val, "modtmp")
                .map(Into::into)
                .map_err(|e| e.to_string()),
            BinOp::Exp => self
                .builder
                .build_call(
                    self.module.get_function("pow").unwrap(),
                    &[lhs_val.into(), rhs_val.into()],
                    "powtmp",
                )
                .map_err(|error| error.to_string())?
                .try_as_basic_value()
                .basic()
                .ok_or_else(|| "pow returned no value".to_string()),
            BinOp::BitOr
            | BinOp::BitXor
            | BinOp::BitAnd
            | BinOp::LShift
            | BinOp::RShift
            | BinOp::ZeroFillRShift => {
                let i32_type = self.context.i32_type();
                let lhs_int = self
                    .builder
                    .build_float_to_signed_int(lhs_val, i32_type, "bit_lhs")
                    .map_err(|error| error.to_string())?;
                let rhs_int = self
                    .builder
                    .build_float_to_signed_int(rhs_val, i32_type, "bit_rhs")
                    .map_err(|error| error.to_string())?;
                let shift = self
                    .builder
                    .build_and(rhs_int, i32_type.const_int(31, false), "shift_count")
                    .map_err(|error| error.to_string())?;
                let result = match op {
                    BinOp::BitOr => self.builder.build_or(lhs_int, rhs_int, "bitor"),
                    BinOp::BitXor => self.builder.build_xor(lhs_int, rhs_int, "bitxor"),
                    BinOp::BitAnd => self.builder.build_and(lhs_int, rhs_int, "bitand"),
                    BinOp::LShift => self.builder.build_left_shift(lhs_int, shift, "lshift"),
                    BinOp::RShift => self
                        .builder
                        .build_right_shift(lhs_int, shift, true, "rshift"),
                    BinOp::ZeroFillRShift => self
                        .builder
                        .build_right_shift(lhs_int, shift, false, "urshift"),
                    _ => unreachable!(),
                }
                .map_err(|error| error.to_string())?;
                if op == BinOp::ZeroFillRShift {
                    self.builder
                        .build_unsigned_int_to_float(result, self.context.f64_type(), "bit_number")
                        .map(Into::into)
                        .map_err(|error| error.to_string())
                } else {
                    self.builder
                        .build_signed_int_to_float(result, self.context.f64_type(), "bit_number")
                        .map(Into::into)
                        .map_err(|error| error.to_string())
                }
            }
            BinOp::Lt => self
                .builder
                .build_float_compare(FloatPredicate::OLT, lhs_val, rhs_val, "lttmp")
                .map(Into::into)
                .map_err(|e| e.to_string()),
            BinOp::Gt => self
                .builder
                .build_float_compare(FloatPredicate::OGT, lhs_val, rhs_val, "gttmp")
                .map(Into::into)
                .map_err(|e| e.to_string()),
            BinOp::LtEq => self
                .builder
                .build_float_compare(FloatPredicate::OLE, lhs_val, rhs_val, "letmp")
                .map(Into::into)
                .map_err(|e| e.to_string()),
            BinOp::GtEq => self
                .builder
                .build_float_compare(FloatPredicate::OGE, lhs_val, rhs_val, "getmp")
                .map(Into::into)
                .map_err(|e| e.to_string()),
            BinOp::EqEqEq => unreachable!(),
        }
    }
}
