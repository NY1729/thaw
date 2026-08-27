impl<'a> FnLowerer<'a> {
    fn expect_type(
        &self,
        expected: &HirType,
        value: &HirExpr,
        context: &str,
    ) -> Result<(), String> {
        let actual = self.infer_expr_type(value)?;
        let callable_compatible = match (expected, &actual) {
            (
                HirType::CallableFunction(fixed, _, rest, expected_ret),
                HirType::Function(params, ret),
            ) => {
                let mut abi = fixed.clone();
                if let Some(rest) = rest {
                    abi.push(HirType::Array(rest.clone()));
                }
                abi == *params && expected_ret == ret
            }
            _ => false,
        };
        if actual == HirType::Dynamic
            || *expected == HirType::Dynamic
            || actual == *expected
            || callable_compatible
        {
            Ok(())
        } else {
            Err(format!(
                "{context} has type {actual:?}, expected {expected:?}"
            ))
        }
    }

    /// Infers the concrete native type of an expression. This is also the
    /// shared checker for assignments, returns, operators, indexes and call
    /// arguments, keeping unresolved/dynamic layouts out of LLVM lowering.
    fn infer_expr_type(&self, expr: &HirExpr) -> Result<HirType, String> {
        match expr {
            HirExpr::Lit(HirLit::F64(_)) => Ok(HirType::F64),
            HirExpr::Lit(HirLit::Str(_)) => Ok(HirType::Str),
            HirExpr::Lit(HirLit::Bool(_)) => Ok(HirType::Bool),
            HirExpr::Lit(HirLit::Undefined) => Ok(HirType::Undefined),
            HirExpr::Lit(HirLit::Null) => Ok(HirType::Null),
            HirExpr::Var(name) => self
                .scope
                .get(name)
                .cloned()
                .ok_or_else(|| format!("unknown variable `{name}`")),
            HirExpr::FunctionRef(_, params, ret) => {
                Ok(HirType::Function(params.clone(), Box::new(ret.clone())))
            }
            HirExpr::MethodRef(_, _, params, ret, _) => {
                Ok(HirType::Function(params.clone(), Box::new(ret.clone())))
            }
            HirExpr::OptionalSome(value, payload) => {
                self.expect_type(payload, value, "optional payload")?;
                Ok(HirType::Optional(Box::new(payload.clone())))
            }
            HirExpr::OptionalNone(payload) => Ok(HirType::Optional(Box::new(payload.clone()))),
            HirExpr::OptionalIsNone(value, payload) => {
                self.expect_type(
                    &HirType::Optional(Box::new(payload.clone())),
                    value,
                    "optional test",
                )?;
                Ok(HirType::Bool)
            }
            HirExpr::OptionalValue(value, payload) => {
                self.expect_type(
                    &HirType::Optional(Box::new(payload.clone())),
                    value,
                    "optional value extraction",
                )?;
                Ok(payload.clone())
            }
            HirExpr::NullableSome(value, payload) => {
                self.expect_type(payload, value, "nullable payload")?;
                Ok(HirType::Nullable(Box::new(payload.clone())))
            }
            HirExpr::NullableNone(payload) => Ok(HirType::Nullable(Box::new(payload.clone()))),
            HirExpr::NullableIsNone(value, payload) => {
                self.expect_type(
                    &HirType::Nullable(Box::new(payload.clone())),
                    value,
                    "nullable null check",
                )?;
                Ok(HirType::Bool)
            }
            HirExpr::NullableValue(value, payload) => {
                self.expect_type(
                    &HirType::Nullable(Box::new(payload.clone())),
                    value,
                    "nullable payload extraction",
                )?;
                Ok(payload.clone())
            }
            HirExpr::NullishSome(value, payload) => {
                self.expect_type(payload, value, "nullish payload")?;
                Ok(HirType::Nullish(Box::new(payload.clone())))
            }
            HirExpr::NullishNull(payload) | HirExpr::NullishUndefined(payload) => {
                Ok(HirType::Nullish(Box::new(payload.clone())))
            }
            HirExpr::NullishIsNull(value, payload)
            | HirExpr::NullishIsUndefined(value, payload)
            | HirExpr::NullishIsNone(value, payload) => {
                self.expect_type(
                    &HirType::Nullish(Box::new(payload.clone())),
                    value,
                    "nullish tag check",
                )?;
                Ok(HirType::Bool)
            }
            HirExpr::NullishValue(value, payload) => {
                self.expect_type(
                    &HirType::Nullish(Box::new(payload.clone())),
                    value,
                    "nullish payload extraction",
                )?;
                Ok(payload.clone())
            }
            HirExpr::UnionInject(value, index, elements) => {
                let member = elements
                    .get(*index)
                    .ok_or_else(|| format!("union member index {index} is out of bounds"))?;
                self.expect_type(member, value, "union payload")?;
                Ok(HirType::Union(elements.clone()))
            }
            HirExpr::UnionTag(value, elements) => {
                self.expect_type(&HirType::Union(elements.clone()), value, "union tag access")?;
                Ok(HirType::F64)
            }
            HirExpr::UnionValue(value, index, elements) => {
                self.expect_type(
                    &HirType::Union(elements.clone()),
                    value,
                    "union value extraction",
                )?;
                elements
                    .get(*index)
                    .cloned()
                    .ok_or_else(|| format!("union member index {index} is out of bounds"))
            }
            HirExpr::UnionMemberIsEqual(union, member, index, elements) => {
                self.expect_type(
                    &HirType::Union(elements.clone()),
                    union,
                    "union equality receiver",
                )?;
                let expected = elements
                    .get(*index)
                    .ok_or_else(|| format!("union member index {index} is out of bounds"))?;
                self.expect_type(expected, member, "union equality member")?;
                Ok(HirType::Bool)
            }
            HirExpr::UnionIsEqual(left, right, elements) => {
                let union = HirType::Union(elements.clone());
                self.expect_type(&union, left, "union equality left operand")?;
                self.expect_type(&union, right, "union equality right operand")?;
                Ok(HirType::Bool)
            }
            HirExpr::ArrayAlloc(length, element) => {
                self.expect_type(&HirType::F64, length, "array allocation length")?;
                Ok(HirType::Array(Box::new(element.clone())))
            }
            HirExpr::ArraySetLen(array, length, element) => {
                self.expect_type(
                    &HirType::Array(Box::new(element.clone())),
                    array,
                    "array length update receiver",
                )?;
                self.expect_type(&HirType::F64, length, "array length update")?;
                Ok(HirType::Array(Box::new(element.clone())))
            }
            HirExpr::Assign(name, value) => {
                let expected = self
                    .scope
                    .get(name)
                    .cloned()
                    .ok_or_else(|| format!("unknown variable `{name}`"))?;
                self.expect_type(&expected, value, &format!("assignment to `{name}`"))?;
                Ok(expected)
            }
            HirExpr::BinOp(op, left, right) => {
                let left_ty = self.infer_expr_type(left)?;
                let right_ty = self.infer_expr_type(right)?;
                match op {
                    BinOp::EqEqEq => {
                        if left_ty != HirType::Dynamic
                            && right_ty != HirType::Dynamic
                            && left_ty != right_ty
                        {
                            return Err(format!(
                                "strict equality compares incompatible types {left_ty:?} and {right_ty:?}"
                            ));
                        }
                        Ok(HirType::Bool)
                    }
                    BinOp::Lt | BinOp::Gt | BinOp::LtEq | BinOp::GtEq => {
                        if !matches!(left_ty, HirType::F64 | HirType::Dynamic)
                            || !matches!(right_ty, HirType::F64 | HirType::Dynamic)
                        {
                            return Err(format!(
                                "numeric comparison requires F64 operands, got {left_ty:?} and {right_ty:?}"
                            ));
                        }
                        Ok(HirType::Bool)
                    }
                    BinOp::Add
                    | BinOp::Sub
                    | BinOp::Mul
                    | BinOp::Div
                    | BinOp::Mod
                    | BinOp::Exp
                    | BinOp::BitOr
                    | BinOp::BitXor
                    | BinOp::BitAnd
                    | BinOp::LShift
                    | BinOp::RShift
                    | BinOp::ZeroFillRShift => {
                        if !matches!(left_ty, HirType::F64 | HirType::Dynamic)
                            || !matches!(right_ty, HirType::F64 | HirType::Dynamic)
                        {
                            return Err(format!(
                                "arithmetic requires F64 operands, got {left_ty:?} and {right_ty:?}"
                            ));
                        }
                        Ok(HirType::F64)
                    }
                }
            }
            HirExpr::Call(callee, args) => {
                let HirExpr::Var(name) = callee.as_ref() else {
                    let (params, ret) = match self.infer_expr_type(callee)? {
                        HirType::Function(params, ret) => (params, ret),
                        HirType::CallableFunction(mut params, _, rest, ret) => {
                            if let Some(rest) = rest {
                                params.push(HirType::Array(rest));
                            }
                            (params, ret)
                        }
                        _ => return Err("call target is not a function value".into()),
                    };
                    if params.len() != args.len() {
                        return Err(format!(
                            "function value expects {} argument(s), got {}",
                            params.len(),
                            args.len()
                        ));
                    }
                    return Ok(*ret);
                };
                match name.as_str() {
                    "console.log" | "console.info" | "console.debug" | "console.warn"
                    | "console.error" | "console.assert" => return Ok(HirType::Void),
                    "__thaw_string_concat" => {
                        if args.len() != 2 {
                            return Err("string concatenation expects two operands".into());
                        }
                        for argument in args {
                            self.expect_type(&HirType::Str, argument, "string concatenation")?;
                        }
                        return Ok(HirType::Str);
                    }
                    "__thaw_string_from_char_code" => {
                        let [argument] = args.as_slice() else {
                            return Err("String.fromCharCode expects one operand".into());
                        };
                        self.expect_type(&HirType::F64, argument, "String.fromCharCode")?;
                        return Ok(HirType::Str);
                    }
                    "__thaw_string_from_code_point" => {
                        let [argument] = args.as_slice() else {
                            return Err("String.fromCodePoint expects one operand".into());
                        };
                        self.expect_type(&HirType::F64, argument, "String.fromCodePoint")?;
                        return Ok(HirType::Str);
                    }
                    "__thaw_bool_to_string" => {
                        let [argument] = args.as_slice() else {
                            return Err("boolean string conversion expects one operand".into());
                        };
                        self.expect_type(&HirType::Bool, argument, "boolean string conversion")?;
                        return Ok(HirType::Str);
                    }
                    "__thaw_number_to_string" => {
                        let [argument] = args.as_slice() else {
                            return Err("number string conversion expects one operand".into());
                        };
                        self.expect_type(&HirType::F64, argument, "number string conversion")?;
                        return Ok(HirType::Str);
                    }
                    "__thaw_bool_to_number" => {
                        let [argument] = args.as_slice() else {
                            return Err("boolean number conversion expects one operand".into());
                        };
                        self.expect_type(&HirType::Bool, argument, "boolean number conversion")?;
                        return Ok(HirType::F64);
                    }
                    "__thaw_string_to_number" => {
                        let [argument] = args.as_slice() else {
                            return Err("string number conversion expects one operand".into());
                        };
                        self.expect_type(&HirType::Str, argument, "string number conversion")?;
                        return Ok(HirType::F64);
                    }
                    "__thaw_parse_float" => {
                        let [argument] = args.as_slice() else {
                            return Err("parseFloat expects one operand".into());
                        };
                        self.expect_type(&HirType::Str, argument, "parseFloat operand")?;
                        return Ok(HirType::F64);
                    }
                    "__thaw_parse_int" => {
                        let [text, radix] = args.as_slice() else {
                            return Err("parseInt expects text and radix operands".into());
                        };
                        self.expect_type(&HirType::Str, text, "parseInt text")?;
                        self.expect_type(&HirType::F64, radix, "parseInt radix")?;
                        return Ok(HirType::F64);
                    }
                    "__thaw_string_lt" | "__thaw_string_gt" | "__thaw_string_lte"
                    | "__thaw_string_gte" => {
                        if args.len() != 2 {
                            return Err("string comparison expects two operands".into());
                        }
                        for argument in args {
                            self.expect_type(&HirType::Str, argument, "string comparison")?;
                        }
                        return Ok(HirType::Bool);
                    }
                    "__thaw_number_array_to_string"
                    | "__thaw_string_array_to_string"
                    | "__thaw_bool_array_to_string"
                    | "__thaw_object_array_to_string"
                    | "__thaw_object_to_string" => return Ok(HirType::Str),
                    "__thaw_number_array_join"
                    | "__thaw_string_array_join"
                    | "__thaw_bool_array_join"
                    | "__thaw_object_array_join" => {
                        if args.len() != 2 {
                            return Err("array join expects two operands".into());
                        }
                        self.expect_type(&HirType::Str, &args[1], "array join separator")?;
                        return Ok(HirType::Str);
                    }
                    "__thaw_array_reverse" => {
                        let [array] = args.as_slice() else {
                            return Err("array reverse expects one operand".into());
                        };
                        let ty = self.infer_expr_type(array)?;
                        if !matches!(ty, HirType::Array(_)) {
                            return Err("array reverse requires a homogeneous array".into());
                        }
                        return Ok(ty);
                    }
                    "__thaw_array_copy_within" => {
                        if args.len() != 4 {
                            return Err("array copyWithin expects four operands".into());
                        }
                        let ty = self.infer_expr_type(&args[0])?;
                        if !matches!(ty, HirType::Array(_)) {
                            return Err("array copyWithin requires a homogeneous array".into());
                        }
                        for argument in &args[1..] {
                            self.expect_type(&HirType::F64, argument, "copyWithin index")?;
                        }
                        return Ok(ty);
                    }
                    "__thaw_array_fill"
                    | "__thaw_number_array_fill"
                    | "__thaw_pointer_array_fill"
                    | "__thaw_bool_array_fill" => {
                        if args.len() != 4 {
                            return Err("array fill expects four operands".into());
                        }
                        let ty = self.infer_expr_type(&args[0])?;
                        let HirType::Array(element) = &ty else {
                            return Err("array fill requires a homogeneous array".into());
                        };
                        self.expect_type(element, &args[1], "fill value")?;
                        for argument in &args[2..] {
                            self.expect_type(&HirType::F64, argument, "fill index")?;
                        }
                        return Ok(ty);
                    }
                    "__thaw_array_slice" => {
                        if args.len() != 3 {
                            return Err("array slice expects three operands".into());
                        }
                        let ty = self.infer_expr_type(&args[0])?;
                        if !matches!(ty, HirType::Array(_)) {
                            return Err("array slice requires a homogeneous array".into());
                        }
                        for argument in &args[1..] {
                            self.expect_type(&HirType::F64, argument, "slice index")?;
                        }
                        return Ok(ty);
                    }
                    "__thaw_array_to_reversed" => {
                        let [array] = args.as_slice() else {
                            return Err("array toReversed expects one operand".into());
                        };
                        let ty = self.infer_expr_type(array)?;
                        if !matches!(ty, HirType::Array(_)) {
                            return Err("array toReversed requires a homogeneous array".into());
                        }
                        return Ok(ty);
                    }
                    "__thaw_number_array_sort"
                    | "__thaw_string_array_sort"
                    | "__thaw_bool_array_sort"
                    | "__thaw_object_array_sort"
                    | "__thaw_number_array_to_sorted"
                    | "__thaw_string_array_to_sorted"
                    | "__thaw_bool_array_to_sorted"
                    | "__thaw_object_array_to_sorted" => {
                        let [array] = args.as_slice() else {
                            return Err("array sort expects one operand".into());
                        };
                        let ty = self.infer_expr_type(array)?;
                        if !matches!(ty, HirType::Array(_)) {
                            return Err("array sort requires a homogeneous array".into());
                        }
                        return Ok(ty);
                    }
                    "__thaw_number_array_index_of"
                    | "__thaw_string_array_index_of"
                    | "__thaw_bool_array_index_of"
                    | "__thaw_object_array_index_of"
                    | "__thaw_number_array_last_index_of"
                    | "__thaw_string_array_last_index_of"
                    | "__thaw_bool_array_last_index_of"
                    | "__thaw_object_array_last_index_of"
                    | "__thaw_number_array_includes"
                    | "__thaw_string_array_includes"
                    | "__thaw_bool_array_includes"
                    | "__thaw_object_array_includes" => {
                        if args.len() != 3 {
                            return Err("array search expects three operands".into());
                        }
                        self.expect_type(&HirType::F64, &args[2], "array search start")?;
                        return Ok(if name.ends_with("_includes") {
                            HirType::Bool
                        } else {
                            HirType::F64
                        });
                    }
                    "__thaw_string_index_of"
                    | "__thaw_string_last_index_of"
                    | "__thaw_string_includes"
                    | "__thaw_string_starts_with"
                    | "__thaw_string_ends_with" => {
                        if args.len() != 3 {
                            return Err("string search expects three operands".into());
                        }
                        self.expect_type(&HirType::Str, &args[0], "string search receiver")?;
                        self.expect_type(&HirType::Str, &args[1], "string search needle")?;
                        self.expect_type(&HirType::F64, &args[2], "string search position")?;
                        return Ok(
                            if matches!(
                                name.as_str(),
                                "__thaw_string_index_of" | "__thaw_string_last_index_of"
                            ) {
                                HirType::F64
                            } else {
                                HirType::Bool
                            },
                        );
                    }
                    "__thaw_string_trim"
                    | "__thaw_string_trim_start"
                    | "__thaw_string_trim_end"
                    | "__thaw_string_to_lower_case"
                    | "__thaw_string_to_upper_case" => {
                        let [argument] = args.as_slice() else {
                            return Err("string trim expects one operand".into());
                        };
                        self.expect_type(&HirType::Str, argument, "string trim receiver")?;
                        return Ok(HirType::Str);
                    }
                    "__thaw_encode_uri_component" => {
                        let [argument] = args.as_slice() else {
                            return Err("encodeURIComponent expects one operand".into());
                        };
                        self.expect_type(&HirType::Str, argument, "encodeURIComponent argument")?;
                        return Ok(HirType::Str);
                    }
                    "__thaw_encode_uri" => {
                        let [argument] = args.as_slice() else {
                            return Err("encodeURI expects one operand".into());
                        };
                        self.expect_type(&HirType::Str, argument, "encodeURI argument")?;
                        return Ok(HirType::Str);
                    }
                    "__thaw_decode_uri_component" => {
                        let [argument] = args.as_slice() else {
                            return Err("decodeURIComponent expects one operand".into());
                        };
                        self.expect_type(&HirType::Str, argument, "decodeURIComponent argument")?;
                        return Ok(HirType::Str);
                    }
                    "__thaw_decode_uri" => {
                        let [argument] = args.as_slice() else {
                            return Err("decodeURI expects one operand".into());
                        };
                        self.expect_type(&HirType::Str, argument, "decodeURI argument")?;
                        return Ok(HirType::Str);
                    }
                    "__thaw_string_is_null" => {
                        let [argument] = args.as_slice() else {
                            return Err("string null check expects one operand".into());
                        };
                        self.expect_type(&HirType::Str, argument, "string null check")?;
                        return Ok(HirType::Bool);
                    }
                    "__thaw_array_is_null" => {
                        let [argument] = args.as_slice() else {
                            return Err("array null check expects one operand".into());
                        };
                        if !matches!(self.infer_expr_type(argument)?, HirType::Array(_)) {
                            return Err("array null check requires an array operand".into());
                        }
                        return Ok(HirType::Bool);
                    }
                    "__thaw_string_to_array" => {
                        let [value] = args.as_slice() else {
                            return Err("string iterator conversion expects one operand".into());
                        };
                        self.expect_type(&HirType::Str, value, "string iterator source")?;
                        return Ok(HirType::Array(Box::new(HirType::Str)));
                    }
                    "__thaw_string_repeat" => {
                        let [value, count] = args.as_slice() else {
                            return Err("string repeat expects two operands".into());
                        };
                        self.expect_type(&HirType::Str, value, "string repeat receiver")?;
                        self.expect_type(&HirType::F64, count, "string repeat count")?;
                        return Ok(HirType::Str);
                    }
                    "__thaw_string_pad_start" | "__thaw_string_pad_end" => {
                        let [value, pad, length] = args.as_slice() else {
                            return Err("string pad expects three operands".into());
                        };
                        self.expect_type(&HirType::Str, value, "string pad receiver")?;
                        self.expect_type(&HirType::Str, pad, "string pad value")?;
                        self.expect_type(&HirType::F64, length, "string pad target length")?;
                        return Ok(HirType::Str);
                    }
                    "__thaw_number_to_fixed" => {
                        let [value, digits] = args.as_slice() else {
                            return Err("number toFixed expects two operands".into());
                        };
                        self.expect_type(&HirType::F64, value, "toFixed receiver")?;
                        self.expect_type(&HirType::F64, digits, "toFixed digits")?;
                        return Ok(HirType::Str);
                    }
                    "__thaw_number_to_precision" => {
                        let [value, digits] = args.as_slice() else {
                            return Err("number toPrecision expects two operands".into());
                        };
                        self.expect_type(&HirType::F64, value, "toPrecision receiver")?;
                        self.expect_type(&HirType::F64, digits, "toPrecision digits")?;
                        return Ok(HirType::Str);
                    }
                    "__thaw_string_length" => {
                        let [argument] = args.as_slice() else {
                            return Err("string length expects one operand".into());
                        };
                        self.expect_type(&HirType::Str, argument, "string length receiver")?;
                        return Ok(HirType::F64);
                    }
                    "__thaw_string_char_code_at" => {
                        let [value, index] = args.as_slice() else {
                            return Err("string charCodeAt expects two operands".into());
                        };
                        self.expect_type(&HirType::Str, value, "charCodeAt receiver")?;
                        self.expect_type(&HirType::F64, index, "charCodeAt index")?;
                        return Ok(HirType::F64);
                    }
                    "__thaw_string_locale_compare" => {
                        let [receiver, other] = args.as_slice() else {
                            return Err("string localeCompare expects two operands".into());
                        };
                        self.expect_type(&HirType::Str, receiver, "localeCompare receiver")?;
                        self.expect_type(&HirType::Str, other, "localeCompare argument")?;
                        return Ok(HirType::F64);
                    }
                    "__thaw_string_normalize" => {
                        let [receiver, form] = args.as_slice() else {
                            return Err("string normalize expects two operands".into());
                        };
                        self.expect_type(&HirType::Str, receiver, "normalize receiver")?;
                        self.expect_type(&HirType::Str, form, "normalize form")?;
                        return Ok(HirType::Str);
                    }
                    "__thaw_string_split" => {
                        let [receiver, separator, limit] = args.as_slice() else {
                            return Err("string split expects three operands".into());
                        };
                        self.expect_type(&HirType::Str, receiver, "split receiver")?;
                        self.expect_type(&HirType::Str, separator, "split separator")?;
                        self.expect_type(&HirType::F64, limit, "split limit")?;
                        return Ok(HirType::Array(Box::new(HirType::Str)));
                    }
                    "__thaw_string_replace" | "__thaw_string_replace_all" => {
                        let [receiver, search, replacement] = args.as_slice() else {
                            return Err("string replace expects three operands".into());
                        };
                        self.expect_type(&HirType::Str, receiver, "replace receiver")?;
                        self.expect_type(&HirType::Str, search, "replace search")?;
                        self.expect_type(&HirType::Str, replacement, "replace value")?;
                        return Ok(HirType::Str);
                    }
                    "__thaw_regex_test" => {
                        let [source, flags, value] = args.as_slice() else {
                            return Err("RegExp.test expects three operands".into());
                        };
                        self.expect_type(&HirType::Str, source, "RegExp.test source")?;
                        self.expect_type(&HirType::Str, flags, "RegExp.test flags")?;
                        self.expect_type(&HirType::Str, value, "RegExp.test value")?;
                        return Ok(HirType::Bool);
                    }
                    "__thaw_date_now" => {
                        if !args.is_empty() {
                            return Err("Date.now expects no operands".into());
                        }
                        return Ok(HirType::F64);
                    }
                    "__thaw_date_get_full_year"
                    | "__thaw_date_get_month"
                    | "__thaw_date_get_date"
                    | "__thaw_date_get_day"
                    | "__thaw_date_get_hours"
                    | "__thaw_date_get_minutes"
                    | "__thaw_date_get_seconds"
                    | "__thaw_date_get_milliseconds" => {
                        let [timestamp] = args.as_slice() else {
                            return Err(format!("{name} expects one operand"));
                        };
                        self.expect_type(&HirType::F64, timestamp, "Date getter timestamp")?;
                        return Ok(HirType::F64);
                    }
                    "__thaw_date_to_iso_string"
                    | "__thaw_date_to_date_string"
                    | "__thaw_date_to_time_string"
                    | "__thaw_date_to_string"
                    | "__thaw_date_to_utc_string" => {
                        let [timestamp] = args.as_slice() else {
                            return Err(format!("{name} expects one operand"));
                        };
                        self.expect_type(&HirType::F64, timestamp, "Date timestamp")?;
                        return Ok(HirType::Str);
                    }
                    "__thaw_date_set_full_year"
                    | "__thaw_date_set_month"
                    | "__thaw_date_set_date"
                    | "__thaw_date_set_hours"
                    | "__thaw_date_set_minutes"
                    | "__thaw_date_set_seconds"
                    | "__thaw_date_set_milliseconds"
                    | "__thaw_date_utc" => {
                        for (index, argument) in args.iter().enumerate() {
                            self.expect_type(
                                &HirType::F64,
                                argument,
                                &format!("{name} operand {index}"),
                            )?;
                        }
                        return Ok(HirType::F64);
                    }
                    "__thaw_date_parse" => {
                        let [text] = args.as_slice() else {
                            return Err("Date.parse expects one operand".into());
                        };
                        self.expect_type(&HirType::Str, text, "Date.parse argument")?;
                        return Ok(HirType::F64);
                    }
                    "__thaw_regex_search" => {
                        let [value, source, flags] = args.as_slice() else {
                            return Err("String.search expects three operands".into());
                        };
                        self.expect_type(&HirType::Str, value, "search value")?;
                        self.expect_type(&HirType::Str, source, "search source")?;
                        self.expect_type(&HirType::Str, flags, "search flags")?;
                        return Ok(HirType::F64);
                    }
                    "__thaw_regex_exec" => {
                        let [source, flags, value] = args.as_slice() else {
                            return Err("RegExp.exec expects three operands".into());
                        };
                        self.expect_type(&HirType::Str, source, "RegExp.exec source")?;
                        self.expect_type(&HirType::Str, flags, "RegExp.exec flags")?;
                        self.expect_type(&HirType::Str, value, "RegExp.exec value")?;
                        return Ok(HirType::Array(Box::new(HirType::Str)));
                    }
                    "__thaw_regex_match" => {
                        let [value, source, flags] = args.as_slice() else {
                            return Err("String.match expects three operands".into());
                        };
                        self.expect_type(&HirType::Str, value, "match receiver")?;
                        self.expect_type(&HirType::Str, source, "match source")?;
                        self.expect_type(&HirType::Str, flags, "match flags")?;
                        return Ok(HirType::Array(Box::new(HirType::Str)));
                    }
                    "__thaw_regex_replace" | "__thaw_regex_replace_all" => {
                        let [value, source, flags, replacement] = args.as_slice() else {
                            return Err("RegExp replace expects four operands".into());
                        };
                        self.expect_type(&HirType::Str, value, "replace receiver")?;
                        self.expect_type(&HirType::Str, source, "replace source")?;
                        self.expect_type(&HirType::Str, flags, "replace flags")?;
                        self.expect_type(&HirType::Str, replacement, "replace value")?;
                        return Ok(HirType::Str);
                    }
                    "__thaw_regex_split" => {
                        let [value, source, flags] = args.as_slice() else {
                            return Err("String.split expects three operands".into());
                        };
                        self.expect_type(&HirType::Str, value, "split receiver")?;
                        self.expect_type(&HirType::Str, source, "split source")?;
                        self.expect_type(&HirType::Str, flags, "split flags")?;
                        return Ok(HirType::Array(Box::new(HirType::Str)));
                    }
                    "__thaw_string_code_point_at" => {
                        let [value, index] = args.as_slice() else {
                            return Err("string codePointAt expects two operands".into());
                        };
                        self.expect_type(&HirType::Str, value, "codePointAt receiver")?;
                        self.expect_type(&HirType::F64, index, "codePointAt index")?;
                        return Ok(HirType::F64);
                    }
                    "__thaw_json_is_array" => {
                        let [value] = args.as_slice() else {
                            return Err("Array.isArray expects one operand".into());
                        };
                        self.expect_type(&HirType::Json, value, "Array.isArray JSON operand")?;
                        return Ok(HirType::Bool);
                    }
                    "__thaw_json_keys" => {
                        let [value] = args.as_slice() else {
                            return Err("Object.keys expects one operand".into());
                        };
                        let ty = self.infer_expr_type(value)?;
                        if !matches!(ty, HirType::Json | HirType::Dictionary(_)) {
                            return Err(format!(
                                "Object.keys expected a JSON value or dictionary, got {ty:?}"
                            ));
                        }
                        return Ok(HirType::Array(Box::new(HirType::Str)));
                    }
                    "__thaw_array_keys" => {
                        let [value] = args.as_slice() else {
                            return Err("Object.keys expects one operand".into());
                        };
                        let ty = self.infer_expr_type(value)?;
                        if !matches!(ty, HirType::Array(_) | HirType::Tuple(_)) {
                            return Err(format!("Object.keys expected an array, got {ty:?}"));
                        }
                        return Ok(HirType::Array(Box::new(HirType::Str)));
                    }
                    "__thaw_json_values" => {
                        let [value] = args.as_slice() else {
                            return Err("Object.values expects one operand".into());
                        };
                        let ty = self.infer_expr_type(value)?;
                        if !matches!(ty, HirType::Json)
                            && !matches!(
                                &ty,
                                HirType::Dictionary(element)
                                    if element.as_ref() == &HirType::Json
                            )
                        {
                            return Err(format!(
                                "Object.values expected JSON or a JSON-valued dictionary, got {ty:?}"
                            ));
                        }
                        return Ok(HirType::Array(Box::new(HirType::Json)));
                    }
                    "__thaw_json_number_values"
                    | "__thaw_json_string_values"
                    | "__thaw_json_bool_values" => {
                        let [value] = args.as_slice() else {
                            return Err("Object.values expects one operand".into());
                        };
                        let element = match name.as_str() {
                            "__thaw_json_number_values" => HirType::F64,
                            "__thaw_json_string_values" => HirType::Str,
                            _ => HirType::Bool,
                        };
                        self.expect_type(
                            &HirType::Dictionary(Box::new(element.clone())),
                            value,
                            "Object.values dictionary operand",
                        )?;
                        return Ok(HirType::Array(Box::new(element)));
                    }
                    "__thaw_json_entries" => {
                        let [value] = args.as_slice() else {
                            return Err("Object.entries expects one operand".into());
                        };
                        let ty = self.infer_expr_type(value)?;
                        if !matches!(ty, HirType::Json)
                            && !matches!(
                                &ty,
                                HirType::Dictionary(element)
                                    if element.as_ref() == &HirType::Json
                            )
                        {
                            return Err(format!(
                                "Object.entries expected JSON or a JSON-valued dictionary, got {ty:?}"
                            ));
                        }
                        return Ok(HirType::Array(Box::new(HirType::Tuple(vec![
                            HirType::Str,
                            HirType::Json,
                        ]))));
                    }
                    "__thaw_json_number_entries"
                    | "__thaw_json_string_entries"
                    | "__thaw_json_bool_entries" => {
                        let [value] = args.as_slice() else {
                            return Err("Object.entries expects one operand".into());
                        };
                        let element = match name.as_str() {
                            "__thaw_json_number_entries" => HirType::F64,
                            "__thaw_json_string_entries" => HirType::Str,
                            _ => HirType::Bool,
                        };
                        self.expect_type(
                            &HirType::Dictionary(Box::new(element.clone())),
                            value,
                            "Object.entries dictionary operand",
                        )?;
                        return Ok(HirType::Array(Box::new(HirType::Tuple(vec![
                            HirType::Str,
                            element,
                        ]))));
                    }
                    "__thaw_json_object_from_number_entries"
                    | "__thaw_json_object_from_string_entries"
                    | "__thaw_json_object_from_bool_entries"
                    | "__thaw_json_object_from_json_entries" => {
                        let [entries] = args.as_slice() else {
                            return Err("Object.fromEntries expects one operand".into());
                        };
                        let element = match name.as_str() {
                            "__thaw_json_object_from_number_entries" => HirType::F64,
                            "__thaw_json_object_from_string_entries" => HirType::Str,
                            "__thaw_json_object_from_bool_entries" => HirType::Bool,
                            _ => HirType::Json,
                        };
                        self.expect_type(
                            &HirType::Array(Box::new(HirType::Tuple(vec![
                                HirType::Str,
                                element.clone(),
                            ]))),
                            entries,
                            "Object.fromEntries operand",
                        )?;
                        return Ok(HirType::Dictionary(Box::new(element)));
                    }
                    "__thaw_json_object_assign" => {
                        let [target, source] = args.as_slice() else {
                            return Err("Object.assign expects two internal operands".into());
                        };
                        let target_type = self.infer_expr_type(target)?;
                        let source_type = self.infer_expr_type(source)?;
                        if target_type != source_type
                            || !matches!(target_type, HirType::Json | HirType::Dictionary(_))
                        {
                            return Err(format!(
                                "Object.assign requires matching JSON or dictionary operands, got {target_type:?} and {source_type:?}"
                            ));
                        }
                        return Ok(target_type);
                    }
                    "__thaw_json_has_own" => {
                        let [value, key] = args.as_slice() else {
                            return Err("Object.hasOwn expects two operands".into());
                        };
                        let ty = self.infer_expr_type(value)?;
                        if !matches!(ty, HirType::Json | HirType::Dictionary(_)) {
                            return Err(format!(
                                "Object.hasOwn expected a JSON value or dictionary, got {ty:?}"
                            ));
                        }
                        self.expect_type(&HirType::Str, key, "Object.hasOwn key")?;
                        return Ok(HirType::Bool);
                    }
                    "__thaw_json_is_null" => {
                        let [value] = args.as_slice() else {
                            return Err("JSON null check expects one operand".into());
                        };
                        self.expect_type(&HirType::Json, value, "JSON null check operand")?;
                        return Ok(HirType::Bool);
                    }
                    "__thaw_json_object_is"
                    | "__thaw_json_object_is_number"
                    | "__thaw_json_object_is_string"
                    | "__thaw_json_object_is_bool" => {
                        return Ok(HirType::Bool);
                    }
                    "__thaw_number_is_nan"
                    | "__thaw_number_is_finite"
                    | "__thaw_number_is_integer"
                    | "__thaw_number_is_safe_integer" => {
                        let [argument] = args.as_slice() else {
                            return Err("number predicate expects one operand".into());
                        };
                        self.expect_type(&HirType::F64, argument, "number predicate")?;
                        return Ok(HirType::Bool);
                    }
                    "__thaw_number_object_is" => {
                        let [left, right] = args.as_slice() else {
                            return Err("Object.is number comparison expects two operands".into());
                        };
                        self.expect_type(&HirType::F64, left, "Object.is left operand")?;
                        self.expect_type(&HirType::F64, right, "Object.is right operand")?;
                        return Ok(HirType::Bool);
                    }
                    "__thaw_number_neg" | "__thaw_math_abs" | "__thaw_math_floor"
                    | "__thaw_math_ceil" | "__thaw_math_trunc" | "__thaw_math_sqrt"
                    | "__thaw_math_sign" | "__thaw_math_round" | "__thaw_math_exp"
                    | "__thaw_math_log" | "__thaw_math_log2" | "__thaw_math_log10"
                    | "__thaw_math_sin" | "__thaw_math_cos" | "__thaw_math_tan"
                    | "__thaw_math_asin" | "__thaw_math_acos" | "__thaw_math_atan"
                    | "__thaw_math_sinh" | "__thaw_math_cosh" | "__thaw_math_tanh"
                    | "__thaw_math_cbrt" | "__thaw_math_acosh" | "__thaw_math_asinh"
                    | "__thaw_math_atanh" | "__thaw_math_expm1" | "__thaw_math_log1p" => {
                        let [argument] = args.as_slice() else {
                            return Err("unary Math function expects one operand".into());
                        };
                        self.expect_type(&HirType::F64, argument, "Math operand")?;
                        return Ok(HirType::F64);
                    }
                    "__thaw_math_fround" | "__thaw_math_clz32" => {
                        let [argument] = args.as_slice() else {
                            return Err("unary Math function expects one operand".into());
                        };
                        self.expect_type(&HirType::F64, argument, "Math operand")?;
                        return Ok(HirType::F64);
                    }
                    "__thaw_math_random" => {
                        if !args.is_empty() {
                            return Err("Math.random expects no operands".into());
                        }
                        return Ok(HirType::F64);
                    }
                    "__thaw_math_pow" => {
                        if args.len() != 2 {
                            return Err("Math.pow expects two operands".into());
                        }
                        for argument in args {
                            self.expect_type(&HirType::F64, argument, "Math.pow operand")?;
                        }
                        return Ok(HirType::F64);
                    }
                    "__thaw_math_atan2" => {
                        if args.len() != 2 {
                            return Err("Math.atan2 expects two operands".into());
                        }
                        for argument in args {
                            self.expect_type(&HirType::F64, argument, "Math.atan2 operand")?;
                        }
                        return Ok(HirType::F64);
                    }
                    "__thaw_math_imul" => {
                        if args.len() != 2 {
                            return Err("Math.imul expects two operands".into());
                        }
                        for argument in args {
                            self.expect_type(&HirType::F64, argument, "Math.imul operand")?;
                        }
                        return Ok(HirType::F64);
                    }
                    "__thaw_math_min" | "__thaw_math_max" | "__thaw_math_hypot" => {
                        for argument in args {
                            self.expect_type(&HirType::F64, argument, "Math extrema operand")?;
                        }
                        return Ok(HirType::F64);
                    }
                    "fetch" => return Ok(HirType::Str),
                    "sleep" => return Ok(HirType::Promise(Box::new(HirType::Void))),
                    "Promise.all" => {
                        for (index, arg) in args.iter().enumerate() {
                            match self.infer_expr_type(arg)? {
                                HirType::Promise(value) if *value == HirType::F64 => {}
                                HirType::F64
                                    if matches!(arg, HirExpr::Call(callee, _)
                                        if matches!(callee.as_ref(), HirExpr::Var(name)
                                            if self.signatures.get(name).is_some_and(|signature| signature.is_async && signature.ret == HirType::F64))) => {}
                                other => {
                                    return Err(format!(
                                        "Promise.all element {index} must be Promise<number>, got {other:?}"
                                    ))
                                }
                            }
                        }
                        return Ok(HirType::Promise(Box::new(HirType::Array(Box::new(
                            HirType::F64,
                        )))));
                    }
                    "JSON.parse" => return Ok(HirType::Json),
                    "JSON.stringify" => return Ok(HirType::Str),
                    "__thaw_json_stringify_number_space" => {
                        let [value, space] = args.as_slice() else {
                            return Err("JSON.stringify expects value and number space".into());
                        };
                        let value_type = self.infer_expr_type(value)?;
                        if !matches!(value_type, HirType::Json | HirType::Dictionary(_)) {
                            return Err(format!(
                                "JSON.stringify expected JSON or dictionary, got {value_type:?}"
                            ));
                        }
                        self.expect_type(&HirType::F64, space, "JSON.stringify space")?;
                        return Ok(HirType::Str);
                    }
                    "__thaw_json_stringify_string_space" => {
                        let [value, space] = args.as_slice() else {
                            return Err("JSON.stringify expects value and string space".into());
                        };
                        let value_type = self.infer_expr_type(value)?;
                        if !matches!(value_type, HirType::Json | HirType::Dictionary(_)) {
                            return Err(format!(
                                "JSON.stringify expected JSON or dictionary, got {value_type:?}"
                            ));
                        }
                        self.expect_type(&HirType::Str, space, "JSON.stringify space")?;
                        return Ok(HirType::Str);
                    }
                    "__thaw_json_stringify_keys" => {
                        let [value, keys] = args.as_slice() else {
                            return Err("JSON.stringify expects value and replacer keys".into());
                        };
                        let value_type = self.infer_expr_type(value)?;
                        if !matches!(value_type, HirType::Json | HirType::Dictionary(_)) {
                            return Err(format!(
                                "JSON.stringify expected JSON or dictionary, got {value_type:?}"
                            ));
                        }
                        self.expect_type(
                            &HirType::Array(Box::new(HirType::Str)),
                            keys,
                            "JSON.stringify replacer",
                        )?;
                        return Ok(HirType::Str);
                    }
                    "__thaw_json_stringify_keys_number_space"
                    | "__thaw_json_stringify_keys_string_space" => {
                        let [value, keys, space] = args.as_slice() else {
                            return Err(
                                "JSON.stringify expects value, replacer keys and space".into()
                            );
                        };
                        let value_type = self.infer_expr_type(value)?;
                        if !matches!(value_type, HirType::Json | HirType::Dictionary(_)) {
                            return Err(format!(
                                "JSON.stringify expected JSON or dictionary, got {value_type:?}"
                            ));
                        }
                        self.expect_type(
                            &HirType::Array(Box::new(HirType::Str)),
                            keys,
                            "JSON.stringify replacer",
                        )?;
                        let space_type = if name.ends_with("number_space") {
                            HirType::F64
                        } else {
                            HirType::Str
                        };
                        self.expect_type(&space_type, space, "JSON.stringify space")?;
                        return Ok(HirType::Str);
                    }
                    // QuickJS-NG fallback path (docs/design/bridge.md
                    // section 7): `loadScript` evaluates JS source into
                    // the global engine context; `callDynamic` calls a
                    // top-level function it defined, by name, with `Json`
                    // args in and a `Json` result out.
                    "loadScript" => return Ok(HirType::Bool),
                    "callDynamic" => return Ok(HirType::Json),
                    "getDynamicValue" => return Ok(HirType::JsValue),
                    "callDynamicValue" => return Ok(HirType::Json),
                    "callDynamicValueHandle" => return Ok(HirType::JsValue),
                    "callDynamicValueWithValue" => return Ok(HirType::Json),
                    "releaseDynamicValue" => return Ok(HirType::Bool),
                    "getDynamicProperty" => return Ok(HirType::JsValue),
                    "setDynamicProperty" => return Ok(HirType::Bool),
                    "callDynamicMethod" => return Ok(HirType::Json),
                    "readDynamicValue" => return Ok(HirType::Json),
                    "callDynamicValueMixed" => return Ok(HirType::Json),
                    "constructDynamicValue" => return Ok(HirType::JsValue),
                    "loadNativeAddon" => return Ok(HirType::Bool),
                    "loadNativeAddonEmbedded" => return Ok(HirType::Bool),
                    "callNativeAddon" => return Ok(HirType::Json),
                    "callNativeAddonWithCallback" => return Ok(HirType::Json),
                    "pollNativeAddonEvents" => return Ok(HirType::F64),
                    _ => {}
                }
                if let Some(HirType::Function(params, ret)) = self.scope.get(name) {
                    if params.len() != args.len() {
                        return Err(format!(
                            "function value `{name}` expects {} argument(s), got {}",
                            params.len(),
                            args.len()
                        ));
                    }
                    return Ok(ret.as_ref().clone());
                }
                if let Some(HirType::CallableFunction(params, _, rest, ret)) = self.scope.get(name)
                {
                    let abi_count = params.len() + usize::from(rest.is_some());
                    if abi_count != args.len() {
                        return Err(format!(
                            "callable function value `{name}` expects {abi_count} ABI argument(s), got {}",
                            args.len()
                        ));
                    }
                    return Ok(ret.as_ref().clone());
                }
                if let Some(return_type) = self.generic_call_returns.get(name) {
                    return Ok(return_type.clone());
                }
                let signature = self.signatures.get(name).or_else(|| {
                    name.split_once("__thaw_")
                        .and_then(|(base, _)| self.signatures.get(base))
                        .filter(|signature| !signature.generic_type_params.is_empty())
                });
                match signature {
                    Some(sig) => {
                        if !sig.generic_type_params.is_empty() {
                            let actual = args
                                .iter()
                                .map(|arg| self.infer_expr_type(arg))
                                .collect::<Result<Vec<_>, _>>()?;
                            let types = infer_generic_type_tuple(
                                sig,
                                &actual,
                                self.interfaces,
                                self.generic_interfaces,
                            )?;
                            let substitution = sig
                                .generic_type_params
                                .iter()
                                .cloned()
                                .zip(types)
                                .collect::<HashMap<_, _>>();
                            resolve_ts_type_with_substitution(
                                sig.generic_return_type
                                    .as_ref()
                                    .expect("generic return type"),
                                &substitution,
                                self.interfaces,
                                self.generic_interfaces,
                                &mut Vec::new(),
                            )
                        } else if sig.is_async {
                            Ok(HirType::Promise(Box::new(sig.ret.clone())))
                        } else {
                            Ok(sig.ret.clone())
                        }
                    }
                    None => Err(format!("call to unknown function `{name}`")),
                }
            }
            HirExpr::FunctionCallWithThis(_, _, _, _, ret) => Ok(ret.clone()),
            HirExpr::FunctionBindThis(_, _, bound, params, ret) => Ok(HirType::Function(
                params[bound.len()..].to_vec(),
                Box::new(ret.clone()),
            )),
            HirExpr::PromiseAll(_, element) => Ok(HirType::Promise(Box::new(HirType::Array(
                Box::new(element.clone()),
            )))),
            HirExpr::PromiseAllArray(_, element) => Ok(HirType::Promise(Box::new(HirType::Array(
                Box::new(element.clone()),
            )))),
            HirExpr::PromiseAllTuple(_, elements) => {
                Ok(HirType::Promise(Box::new(HirType::Tuple(elements.clone()))))
            }
            HirExpr::PromiseRace(_, element)
            | HirExpr::PromiseRaceArray(_, element)
            | HirExpr::PromiseAny(_, element)
            | HirExpr::PromiseAnyArray(_, element) => {
                Ok(HirType::Promise(Box::new(element.clone())))
            }
            HirExpr::PromiseAllSettled(_, element)
            | HirExpr::PromiseAllSettledArray(_, element) => Ok(HirType::Promise(Box::new(
                HirType::Array(Box::new(promise_settled_result_type(element.clone()))),
            ))),
            HirExpr::PromiseNew(_, resolved, _) => Ok(HirType::Promise(Box::new(resolved.clone()))),
            HirExpr::PromiseThen(_, _, _, output, _, _) => {
                Ok(HirType::Promise(Box::new(output.clone())))
            }
            HirExpr::PromiseFinally(_, _, input, _) => {
                Ok(HirType::Promise(Box::new(input.clone())))
            }
            HirExpr::DynamicCall(signature, _) => Ok(signature.ret.clone()),
            HirExpr::ArrayLit(values) => {
                if values.is_empty() {
                    return Ok(HirType::Array(Box::new(HirType::F64)));
                }
                let array_element_type =
                    |value: &HirExpr| -> Result<HirType, String> { self.infer_expr_type(value) };
                let elements = values
                    .iter()
                    .map(array_element_type)
                    .collect::<Result<Vec<_>, _>>()?;
                if elements.iter().all(|element| element == &elements[0]) {
                    Ok(HirType::Array(Box::new(elements[0].clone())))
                } else {
                    Ok(HirType::Tuple(elements))
                }
            }
            HirExpr::ArrayConcat(_, element) => Ok(HirType::Array(Box::new(element.clone()))),
            HirExpr::Index(arr, index) => {
                self.expect_type(&HirType::F64, index, "array index")?;
                match self.infer_expr_type(arr)? {
                    HirType::Array(elem) => Ok(*elem),
                    other => Err(format!("cannot index into a value of type {other:?}")),
                }
            }
            HirExpr::TypedIndex(_, _, element) => Ok(element.clone()),
            HirExpr::IndexAssign(arr, index, value) => {
                self.expect_type(&HirType::F64, index, "array index")?;
                let HirType::Array(element) = self.infer_expr_type(arr)? else {
                    return Err("index assignment target is not an array".into());
                };
                self.expect_type(&element, value, "array assignment")?;
                Ok(*element)
            }
            HirExpr::ArrayLen(_) => Ok(HirType::F64),
            HirExpr::EnumReverseLookup(_, _) => Ok(HirType::Optional(Box::new(HirType::Str))),
            HirExpr::EnvVar(_) => Ok(HirType::Str),
            HirExpr::ObjectLit(fields) => {
                let fields = fields
                    .iter()
                    .map(|(name, value)| Ok((name.clone(), self.infer_expr_type(value)?)))
                    .collect::<Result<Vec<_>, String>>()?;
                Ok(HirType::Object(fields))
            }
            HirExpr::ObjectAlloc(ty @ HirType::Object(_)) => Ok(ty.clone()),
            HirExpr::ObjectAlloc(other) => Err(format!(
                "object allocation requires an object type, got {other:?}"
            )),
            HirExpr::PropAccess(_, object_ty, field) => match object_ty {
                HirType::Object(fields) => fields
                    .iter()
                    .find(|(name, _)| name == field)
                    .map(|(_, ty)| ty.clone())
                    .ok_or_else(|| format!("object has no field `{field}`")),
                other => Err(format!(
                    "cannot access `.{field}` on a value of type {other:?}"
                )),
            },
            HirExpr::DynamicPropAccess(_, _, _, result) => Ok(result.clone()),
            HirExpr::PropAssign(_, _, _, value) => self.infer_expr_type(value),
            HirExpr::JsonObjectLit(_, element) => {
                Ok(HirType::Dictionary(Box::new(element.clone())))
            }
            HirExpr::JsonGet(_, _) | HirExpr::JsonIndex(_, _) | HirExpr::JsonKey(_, _) => {
                Ok(HirType::Json)
            }
            HirExpr::JsonSet(_, _, _, element) => Ok(element.clone()),
            HirExpr::JsonIndexSet(_, _, _) => Ok(HirType::Json),
            HirExpr::JsonDelete(_, _) => Ok(HirType::Bool),
            HirExpr::JsonAsNumber(_) => Ok(HirType::F64),
            HirExpr::JsonAsString(_) => Ok(HirType::Str),
            HirExpr::JsonAsBool(_) => Ok(HirType::Bool),
            HirExpr::JsonAsNative(_, ty) => Ok(ty.clone()),
            HirExpr::FfiCall(sig, _) => Ok(sig.ret.clone()),
            HirExpr::Await(inner) | HirExpr::AwaitPromise(inner, _) => {
                match self.infer_expr_type(inner)? {
                    HirType::Promise(value) => Ok(*value),
                    // Legacy/direct await sources can already expose their
                    // resolved type to the surrounding expression.
                    other => Ok(other),
                }
            }
            // The Lambda node now preserves typed parameters and its body,
            // but function values do not have a native ABI until the next
            // callback-lowering phase. Keep the enclosing local dynamic
            // instead of discarding or pretending to know that ABI.
            HirExpr::RecursiveClosure(_, ty, _) => Ok(ty.clone()),
            HirExpr::TypedClosure(ty, _) => Ok(ty.clone()),
            HirExpr::Lambda(_, params, ret, _) => Ok(HirType::Function(
                params.iter().map(|param| param.ty.clone()).collect(),
                Box::new(ret.clone()),
            )),
            HirExpr::ThrowValue(_, fallback) => self.infer_expr_type(fallback),
            HirExpr::Block(stmts) => self.infer_return_type(stmts),
        }
    }
}
