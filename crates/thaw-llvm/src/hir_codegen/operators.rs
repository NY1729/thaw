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
        self.expr_hir_type(expr) == Some(HirType::Str)
    }

    fn expr_hir_type(&self, expr: &HirExpr) -> Option<HirType> {
        match expr {
            HirExpr::Lit(HirLit::F64(_)) => Some(HirType::F64),
            HirExpr::Lit(HirLit::I64(_)) => Some(HirType::I64),
            HirExpr::Lit(HirLit::Str(_) | HirLit::Wtf8(_))
            | HirExpr::EnvVar(_) | HirExpr::JsonAsString(_) => Some(HirType::Str),
            HirExpr::Lit(HirLit::Bool(_)) => Some(HirType::Bool),
            HirExpr::Lit(HirLit::Undefined | HirLit::ArrayHole) => Some(HirType::Undefined),
            HirExpr::Lit(HirLit::Null) => Some(HirType::Null),
            HirExpr::Var(name) => self.variable_hir_types.get(name).cloned(),
            HirExpr::Assign(_, value) => self.expr_hir_type(value),
            HirExpr::PostfixUpdate(_, _) => Some(HirType::F64),
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
            HirExpr::JsonSet(_, _, _, element, _) => Some(element.clone()),
            HirExpr::JsonIndexSet(_, _, _) => Some(HirType::Json),
            HirExpr::JsonDelete(_, _) => Some(HirType::Bool),
            HirExpr::EnumReverseLookup(_, _) => Some(HirType::Optional(Box::new(HirType::Str))),
            HirExpr::ArrayLen(_) => Some(HirType::F64),
            HirExpr::Conditional(_, _, _, ty) => Some(ty.clone()),
            HirExpr::BinOp(op, left, right) => Some(match op {
                BinOp::Lt | BinOp::Gt | BinOp::LtEq | BinOp::GtEq | BinOp::EqEqEq => HirType::Bool,
                // A nested bigint expression (e.g. `-9223372036854775807n
                // - 1n`, or any chained bigint arithmetic beyond a single
                // literal/variable operand) previously fell through to
                // `HirType::F64` unconditionally here, regardless of its
                // real operand types -- this mirrors `compile_binop`'s own
                // `I64`-vs-`F64` dispatch check just below, and thaw-hir's
                // identical check in `lower/inference/types.rs`, so the
                // three stay in agreement.
                _ if self.expr_hir_type(left) == Some(HirType::I64)
                    && self.expr_hir_type(right) == Some(HirType::I64) =>
                {
                    HirType::I64
                }
                _ => HirType::F64,
            }),
            HirExpr::ObjectLit(fields) => fields
                .iter()
                .map(|(name, value)| self.expr_hir_type(value).map(|ty| (name.clone(), ty)))
                .collect::<Option<Vec<_>>>()
                .map(HirType::Object),
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
                        "JSON.parse"
                        | "__thaw_json_object_from_number_entries"
                        | "__thaw_json_object_from_string_entries"
                        | "__thaw_json_object_from_bool_entries"
                        | "__thaw_json_object_from_json_entries"
                        | "__thaw_json_object_assign"
                        | "__thaw_json_array_slice"
                        | "__thaw_json_clone"
                        | "__thaw_json_get_mut"
                        | "__thaw_json_index_get_mut"
                        | "__thaw_json_get_prototype"
                        | "__thaw_json_set_prototype"
                        | "__thaw_json_map_or_set_entries"
                        | "__thaw_json_map_or_set_get"
                        | "__thaw_json_map_or_set_set"
                        | "__thaw_json_map_or_set_add"
                        | "__thaw_json_map_or_set_delete"
                        | "__thaw_json_map_or_set_clear" => return Some(HirType::Json),
                        // `Object.keys`/`Object.getOwnPropertyNames`/
                        // `Reflect.ownKeys` on a `Json`/`Dictionary`/
                        // array receiver, and the array-receiver variant
                        // of the same (thaw-hir's own `infer_expr_type`,
                        // `lower/inference/types.rs`, already has these
                        // -- this mirror was simply never extended to
                        // match). Missing here meant `expr_hir_type`
                        // returned `None` for e.g. `console.log(Object.
                        // keys(x))` used inline (not through a `let`
                        // first), and `compile_console_arg`'s `None`
                        // fallback treated the resulting array handle as
                        // an *error* value to report via
                        // `thaw_error_message`, not a value to format --
                        // observed printing garbage/replacement-character
                        // bytes instead of the array's own text.
                        "__thaw_json_keys" | "__thaw_json_own_keys" | "__thaw_array_keys"
                        | "__thaw_template_strings_raw" => {
                            return Some(HirType::Array(Box::new(HirType::Str)))
                        }
                        "__thaw_json_values"
                        | "__thaw_json_map_or_set_keys"
                        | "__thaw_json_map_or_set_values"
                        | "__thaw_json_map_or_set_entries_view" => {
                            return Some(HirType::Array(Box::new(HirType::Json)))
                        }
                        "__thaw_json_number_values" => {
                            return Some(HirType::Array(Box::new(HirType::F64)))
                        }
                        "__thaw_json_string_values" => {
                            return Some(HirType::Array(Box::new(HirType::Str)))
                        }
                        "__thaw_json_bool_values" => {
                            return Some(HirType::Array(Box::new(HirType::Bool)))
                        }
                        "__thaw_json_entries" => {
                            return Some(HirType::Array(Box::new(HirType::Tuple(vec![
                                HirType::Str,
                                HirType::Json,
                            ]))))
                        }
                        "__thaw_json_number_entries" => {
                            return Some(HirType::Array(Box::new(HirType::Tuple(vec![
                                HirType::Str,
                                HirType::F64,
                            ]))))
                        }
                        "__thaw_json_string_entries" => {
                            return Some(HirType::Array(Box::new(HirType::Tuple(vec![
                                HirType::Str,
                                HirType::Str,
                            ]))))
                        }
                        "__thaw_json_bool_entries" => {
                            return Some(HirType::Array(Box::new(HirType::Tuple(vec![
                                HirType::Str,
                                HirType::Bool,
                            ]))))
                        }
                        "JSON.stringify"
                        | "__thaw_json_stringify_number_space"
                        | "__thaw_json_stringify_string_space"
                        | "__thaw_json_stringify_keys"
                        | "__thaw_json_stringify_keys_number_space"
                        | "__thaw_json_stringify_keys_string_space"
                        | "fetch"
                        | "__thaw_string_concat"
                        | "__thaw_bool_to_string"
                        | "__thaw_number_to_string"
                        | "__thaw_number_array_to_string"
                        | "__thaw_string_array_to_string"
                        | "__thaw_bool_array_to_string"
                        | "__thaw_object_array_to_string"
                        | "__thaw_bytes_to_string"
                        | "__thaw_object_to_string"
                        | "__thaw_number_array_join"
                        | "__thaw_string_array_join"
                        | "__thaw_bool_array_join"
                        | "__thaw_object_array_join"
                        | "__thaw_string_trim"
                        | "__thaw_string_trim_start"
                        | "__thaw_string_trim_end" => return Some(HirType::Str),
                        "__thaw_i64_to_string"
                        | "__thaw_i64_to_bigint_string"
                        | "__thaw_i64_to_radix_string" => return Some(HirType::Str),
                        "__thaw_i64_from_number"
                        | "__thaw_i64_from_string"
                        | "__thaw_i64_as_int_n"
                        | "__thaw_i64_as_uint_n"
                        | "__thaw_bytes_read_i64" => return Some(HirType::I64),
                        "__thaw_any_array_flat" => {
                            return Some(HirType::Array(Box::new(HirType::Json)))
                        }
                        "__thaw_symbol_new" | "__thaw_symbol_for" => {
                            return Some(HirType::Symbol)
                        }
                        "__thaw_symbol_key_for" | "__thaw_symbol_description" => {
                            return Some(HirType::Str)
                        }
                        "__thaw_symbol_to_string" => return Some(HirType::Str),
                        "__thaw_symbol_key" => return Some(HirType::Str),
                        "__thaw_string_length"
                        | "__thaw_string_to_number"
                        | "__thaw_bool_to_number"
                        | "__thaw_parse_int"
                        | "__thaw_parse_float" => return Some(HirType::F64),
                        // The manual dynamic-value escape hatches
                        // (thaw-hir's `infer_expr_type` special-cases these
                        // same names identically, in
                        // `lower/inference/types.rs`) -- previously missing
                        // here entirely, so an un-bound inline call to one
                        // of these (e.g. `console.log(schema.safeParse(x))`,
                        // which `lower_dynamic_value_method_call` lowers to
                        // an inline `callDynamicMethod` call) fell through
                        // to `None`, and `compile_console_values` mishandled
                        // an argument with no known type -- not just wrong
                        // output, but observed to segfault.
                        "getDynamicValue"
                        | "constructDynamicValue"
                        | "callDynamicMethodHandle"
                        | "callDynamicMethodHandleRaw"
                        | "registerNativeCallback"
                        | "getDynamicProperty"
                        | "callDynamicValueHandle"
                        | "resolveDynamicValue" => return Some(HirType::JsValue),
                        "readDynamicValue"
                        | "callDynamicMethod"
                        | "callDynamicValueMixed"
                        | "callDynamic"
                        | "callDynamicValue"
                        | "callDynamicValueWithValue" => return Some(HirType::Json),
                        "loadNativeAddon" | "loadNativeAddonEmbedded" | "loadNativeSharedLibrary" | "loadNativeSharedLibraryEmbedded" | "loadScript"
                        | "releaseDynamicValue"
                        | "setDynamicProperty"
                        | "setDynamicPropertyJson"
                        | "deleteDynamicProperty"
                        | "hasDynamicProperty"
                        | "__thaw_error_is_error"
                        | "__thaw_error_is_instance" => {
                            return Some(HirType::Bool)
                        }
                        "callNativeAddon" | "callNativeAddonWithCallback" | "callNativeAddonValue" => {
                            return Some(HirType::Json)
                        }
                        "pollNativeAddonEvents" => return Some(HirType::F64),
                        "sleep" => return Some(HirType::Promise(Box::new(HirType::Void))),
                        _ => {}
                    }
                    if name == "__thaw_string_to_array" {
                        return Some(HirType::Array(Box::new(HirType::Str)));
                    }
                    if matches!(
                        name.as_str(),
                        "__thaw_number_neg"
                            | "__thaw_math_abs"
                            | "__thaw_math_floor"
                            | "__thaw_math_ceil"
                            | "__thaw_math_trunc"
                            | "__thaw_math_sqrt"
                            | "__thaw_math_sign"
                            | "__thaw_math_round"
                            | "__thaw_math_exp"
                            | "__thaw_math_log"
                            | "__thaw_math_log2"
                            | "__thaw_math_log10"
                            | "__thaw_math_sin"
                            | "__thaw_math_cos"
                            | "__thaw_math_tan"
                            | "__thaw_math_asin"
                            | "__thaw_math_acos"
                            | "__thaw_math_atan"
                            | "__thaw_math_sinh"
                            | "__thaw_math_cosh"
                            | "__thaw_math_tanh"
                            | "__thaw_math_cbrt"
                            | "__thaw_math_acosh"
                            | "__thaw_math_asinh"
                            | "__thaw_math_atanh"
                            | "__thaw_math_expm1"
                            | "__thaw_math_log1p"
                            | "__thaw_math_fround"
                            | "__thaw_math_clz32"
                            | "__thaw_math_random"
                            | "__thaw_math_sum_precise"
                            | "__thaw_math_pow"
                            | "__thaw_math_min"
                            | "__thaw_math_max"
                            | "__thaw_math_atan2"
                            | "__thaw_math_hypot"
                            | "__thaw_math_imul"
                    ) {
                        return Some(HirType::F64);
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
                            | "__thaw_array_splice"
                    ) {
                        return arguments
                            .first()
                            .and_then(|argument| self.expr_hir_type(argument));
                    }
                    if matches!(name.as_str(), "__thaw_array_push" | "__thaw_array_unshift") {
                        return Some(HirType::F64);
                    }
                    if matches!(name.as_str(), "__thaw_array_pop" | "__thaw_array_shift") {
                        return arguments.first().and_then(|argument| {
                            match self.expr_hir_type(argument)? {
                                HirType::Array(element) => Some(*element),
                                _ => None,
                            }
                        });
                    }
                    if matches!(name.as_str(), "__thaw_array_pop_optional" | "__thaw_array_shift_optional") {
                        return arguments.first().and_then(|argument| {
                            let HirType::Array(element) = self.expr_hir_type(argument)? else {
                                return None;
                            };
                            Some(match *element {
                                HirType::Nullable(payload) => HirType::Nullish(payload),
                                HirType::Union(mut members) => {
                                    if !members.contains(&HirType::Undefined) {
                                        members.push(HirType::Undefined);
                                    }
                                    HirType::Union(members)
                                }
                                element @ (HirType::Optional(_) | HirType::Nullish(_) | HirType::Undefined) => element,
                                element => HirType::Optional(Box::new(element)),
                            })
                        });
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
            HirExpr::DynamicCall(signature, _) => Some(signature.ret.clone()),
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
            // A symbol is represented as the same tagged string a `Str` is,
            // so it compares by content (identity): `Symbol.for("x")` is
            // one symbol, while two `Symbol("x")` calls differ by their
            // own id. Without this, symbol operands fell to a raw pointer
            // compare against two separate arena allocations.
            let string_like = |expr: &HirExpr| {
                self.expr_is_string(expr)
                    || matches!(self.expr_hir_type(expr), Some(HirType::Str | HirType::Symbol))
            };
            let string_operands = string_like(lhs) && string_like(rhs);
            // `Json` (`any`-typed) operands are the one pointer-shaped
            // case where the value at that pointer can be a primitive
            // (number/string/boolean/`undefined`/`null`), not just an
            // object/array -- real Strict Equality compares those by
            // value, unlike the raw-pointer fallback below, which is only
            // correct for a genuine object/array/other reference type.
            // `thaw_json_strict_equal` (thaw-std) implements the real
            // per-kind algorithm, including reference equality for its
            // own array/object case, so this doesn't change behavior for
            // those.
            let json_operands = self.expr_hir_type(lhs) == Some(HirType::Json)
                && self.expr_hir_type(rhs) == Some(HirType::Json);
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
                    if json_operands {
                        let compared = self
                            .builder
                            .build_call(
                                self.module.get_function("thaw_json_strict_equal").unwrap(),
                                &[lhs.into(), rhs.into()],
                                "json_strict_eq",
                            )
                            .map_err(|error| error.to_string())?
                            .try_as_basic_value()
                            .basic()
                            .ok_or("thaw_json_strict_equal returned no value")?
                            .into_int_value();
                        return self
                            .builder
                            .build_int_compare(
                                IntPredicate::NE,
                                compared,
                                self.context.i8_type().const_zero(),
                                "json_eq",
                            )
                            .map(Into::into)
                            .map_err(|error| error.to_string());
                    }
                    if string_operands {
                        let compared = self
                            .builder
                            .build_call(
                                self.module.get_function("thaw_string_compare").unwrap(),
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

        if self.expr_hir_type(lhs) == Some(HirType::I64)
            && self.expr_hir_type(rhs) == Some(HirType::I64)
        {
            let lhs = lhs_value.into_int_value();
            let rhs = rhs_value.into_int_value();
            return match op {
                BinOp::Add => self.builder.build_int_add(lhs, rhs, "bigint_add"),
                BinOp::Sub => self.builder.build_int_sub(lhs, rhs, "bigint_sub"),
                BinOp::Mul => self.builder.build_int_mul(lhs, rhs, "bigint_mul"),
                BinOp::Div => self.builder.build_int_signed_div(lhs, rhs, "bigint_div"),
                BinOp::Mod => self.builder.build_int_signed_rem(lhs, rhs, "bigint_mod"),
                BinOp::BitOr => self.builder.build_or(lhs, rhs, "bigint_or"),
                BinOp::BitXor => self.builder.build_xor(lhs, rhs, "bigint_xor"),
                BinOp::BitAnd => self.builder.build_and(lhs, rhs, "bigint_and"),
                BinOp::LShift => self.builder.build_left_shift(lhs, rhs, "bigint_lshift"),
                BinOp::RShift => self.builder.build_right_shift(lhs, rhs, true, "bigint_rshift"),
                BinOp::Lt => self.builder.build_int_compare(IntPredicate::SLT, lhs, rhs, "bigint_lt"),
                BinOp::Gt => self.builder.build_int_compare(IntPredicate::SGT, lhs, rhs, "bigint_gt"),
                BinOp::LtEq => self.builder.build_int_compare(IntPredicate::SLE, lhs, rhs, "bigint_le"),
                BinOp::GtEq => self.builder.build_int_compare(IntPredicate::SGE, lhs, rhs, "bigint_ge"),
                BinOp::Exp => return Err("bigint exponentiation is not implemented".into()),
                BinOp::ZeroFillRShift | BinOp::EqEqEq => unreachable!(),
            }
            .map(Into::into)
            .map_err(|error| error.to_string());
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
            BinOp::Exp => self.compile_js_pow(lhs_val, rhs_val),
            BinOp::BitOr
            | BinOp::BitXor
            | BinOp::BitAnd
            | BinOp::LShift
            | BinOp::RShift
            | BinOp::ZeroFillRShift => {
                let i32_type = self.context.i32_type();
                let lhs_int = self.compile_to_uint32(lhs_val)?;
                let rhs_int = self.compile_to_uint32(rhs_val)?;
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

    fn compile_js_pow(
        &mut self,
        base: FloatValue<'ctx>,
        exponent: FloatValue<'ctx>,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let float = self.context.f64_type();
        let exponent_nan = self.builder.build_float_compare(
            FloatPredicate::UNO, exponent, exponent, "pow_exponent_nan",
        ).map_err(|error| error.to_string())?;
        let exponent_abs = self.builder.build_call(
            self.module.get_function("llvm.fabs.f64").unwrap(),
            &[exponent.into()], "pow_exponent_abs",
        ).map_err(|error| error.to_string())?.try_as_basic_value().basic()
            .ok_or("fabs returned no value")?.into_float_value();
        let exponent_infinite = self.builder.build_float_compare(
            FloatPredicate::OEQ, exponent_abs, float.const_float(f64::INFINITY), "pow_exponent_infinite",
        ).map_err(|error| error.to_string())?;
        let base_abs = self.builder.build_call(
            self.module.get_function("llvm.fabs.f64").unwrap(),
            &[base.into()], "pow_base_abs",
        ).map_err(|error| error.to_string())?.try_as_basic_value().basic()
            .ok_or("fabs returned no value")?.into_float_value();
        let base_unit = self.builder.build_float_compare(
            FloatPredicate::OEQ, base_abs, float.const_float(1.0), "pow_base_unit",
        ).map_err(|error| error.to_string())?;
        let unit_to_infinite = self.builder.build_and(
            exponent_infinite, base_unit, "pow_unit_to_infinite",
        ).map_err(|error| error.to_string())?;
        let special = self.builder.build_or(
            exponent_nan, unit_to_infinite, "pow_special",
        ).map_err(|error| error.to_string())?;
        let function = self.current_function();
        let special_block = self.context.append_basic_block(function, "pow_nan");
        let regular_block = self.context.append_basic_block(function, "pow_regular");
        let merge_block = self.context.append_basic_block(function, "pow_done");
        self.builder.build_conditional_branch(special, special_block, regular_block)
            .map_err(|error| error.to_string())?;
        self.builder.position_at_end(special_block);
        self.builder.build_unconditional_branch(merge_block)
            .map_err(|error| error.to_string())?;
        self.builder.position_at_end(regular_block);
        let regular = self.builder.build_call(
            self.module.get_function("pow").unwrap(),
            &[base.into(), exponent.into()], "powtmp",
        ).map_err(|error| error.to_string())?.try_as_basic_value().basic()
            .ok_or("pow returned no value")?.into_float_value();
        self.builder.build_unconditional_branch(merge_block)
            .map_err(|error| error.to_string())?;
        self.builder.position_at_end(merge_block);
        let result = self.builder.build_phi(float, "pow_result")
            .map_err(|error| error.to_string())?;
        result.add_incoming(&[
            (&float.const_float(f64::NAN), special_block),
            (&regular, regular_block),
        ]);
        Ok(result.as_basic_value())
    }

    // ECMAScript ToUint32's bit pattern is also ToInt32's bit pattern.
    fn compile_to_uint32(&mut self, value: FloatValue<'ctx>) -> Result<IntValue<'ctx>, String> {
        let float = self.context.f64_type();
        let zero = float.const_zero();
        let finite_low = self.builder.build_float_compare(
            FloatPredicate::OGT, value, float.const_float(f64::NEG_INFINITY), "bit_finite_low",
        ).map_err(|error| error.to_string())?;
        let finite_high = self.builder.build_float_compare(
            FloatPredicate::OLT, value, float.const_float(f64::INFINITY), "bit_finite_high",
        ).map_err(|error| error.to_string())?;
        let finite = self.builder.build_and(finite_low, finite_high, "bit_finite")
            .map_err(|error| error.to_string())?;
        let safe = self.builder.build_select(finite, value, zero, "bit_finite_value")
            .map_err(|error| error.to_string())?.into_float_value();
        let truncated = self.builder.build_call(
            self.module.get_function("llvm.trunc.f64").unwrap(), &[safe.into()], "bit_truncated",
        ).map_err(|error| error.to_string())?.try_as_basic_value().basic()
            .ok_or("llvm.trunc returned no value")?.into_float_value();
        let modulus = float.const_float(4_294_967_296.0);
        let remainder = self.builder.build_float_rem(truncated, modulus, "bit_remainder")
            .map_err(|error| error.to_string())?;
        let negative = self.builder.build_float_compare(
            FloatPredicate::OLT, remainder, zero, "bit_negative",
        ).map_err(|error| error.to_string())?;
        let wrapped = self.builder.build_float_add(remainder, modulus, "bit_wrapped")
            .map_err(|error| error.to_string())?;
        let normalized = self.builder.build_select(negative, wrapped, remainder, "bit_normalized")
            .map_err(|error| error.to_string())?.into_float_value();
        self.builder.build_float_to_unsigned_int(normalized, self.context.i32_type(), "bit_uint32")
            .map_err(|error| error.to_string())
    }
}
