impl<'a> FnLowerer<'a> {
    /// `value` as a byte-buffer record: a `Buffer` passes through, a `number[]`
    /// is narrowed (copied) first.
    fn as_bytes_value(&self, value: HirExpr) -> Result<HirExpr, String> {
        Ok(if self.infer_expr_type_inner(&value)? == HirType::Bytes {
            value
        } else {
            HirExpr::Call(Box::new(HirExpr::Var("__thaw_bytes_from_array".into())), vec![value])
        })
    }

    /// `project_native_base` for a spread source, also widening a `Buffer`. A separate,
    /// never-inlined function so `lower_expr` (deeply recursive) keeps its stack frame.
    #[inline(never)]
    fn project_native_base_widening_bytes(&mut self, value: HirExpr) -> Result<HirExpr, String> {
        let value = self.project_native_base(value)?;
        self.bytes_to_f64_array(value)
    }

    /// Whether `new <class>(args)` is `new Uint8Array(buf.buffer, byteOffset, length?)` over a
    /// native `ArrayBuffer` view (a native view sharing the memory).
    #[inline(never)]
    fn is_buffer_view_constructor(
        &mut self,
        class: &str,
        args: &[swc_ecma_ast::ExprOrSpread],
    ) -> Result<bool, String> {
        if class != "Uint8Array" || !(1..=3).contains(&args.len()) {
            return Ok(false);
        }
        let source = self.lower_expr(&args[0].expr)?;
        Ok(self.infer_expr_type_inner(&source)? == HirType::Bytes)
    }

    #[inline(never)]
    fn lower_buffer_view_constructor(
        &mut self,
        args: &[swc_ecma_ast::ExprOrSpread],
    ) -> Result<HirExpr, String> {
        let source = self.lower_expr(&args[0].expr)?;
        let offset = match args.get(1) {
            Some(argument) => {
                let offset = self.lower_expr(&argument.expr)?;
                self.coerce_primitive_to_number(offset)?
            }
            None => HirExpr::Lit(HirLit::F64(f64::NAN)),
        };
        let length = match args.get(2) {
            Some(argument) => {
                let length = self.lower_expr(&argument.expr)?;
                self.coerce_primitive_to_number(length)?
            }
            None => HirExpr::Lit(HirLit::F64(f64::NAN)),
        };
        Ok(HirExpr::Call(
            Box::new(HirExpr::Var("__thaw_bytes_from_buffer_like".to_string())),
            vec![source, offset, length, HirExpr::Lit(HirLit::F64(0.0))],
        ))
    }

    /// A `Buffer` operand of `+` as its UTF-8 text; any other operand unchanged.
    #[inline(never)]
    fn bytes_operand_as_text(&self, value: HirExpr) -> Result<HirExpr, String> {
        Ok(if self.infer_expr_type_inner(&value)? == HirType::Bytes {
            HirExpr::Call(
                Box::new(HirExpr::Var("__thaw_bytes_to_string".into())),
                vec![value, HirExpr::Lit(HirLit::Str("utf8".into()))],
            )
        } else {
            value
        })
    }

    /// `value` as an `f64`-slot array: a `Buffer` is widened (copied), any other
    /// array passes through.
    fn bytes_to_f64_array(&self, value: HirExpr) -> Result<HirExpr, String> {
        Ok(if self.infer_expr_type_inner(&value)? == HirType::Bytes {
            HirExpr::Call(Box::new(HirExpr::Var("__thaw_bytes_to_array".into())), vec![value])
        } else {
            value
        })
    }

    /// Whether `expr` is a byte buffer, judged without lowering it: a binding or property
    /// typed `Bytes`, a `Buffer.from(..)`-style constructor call, or a `subarray`/`slice` of one.
    fn expr_is_bytes_syntax(&self, expr: &Expr) -> bool {
        match expr {
            Expr::Paren(paren) => self.expr_is_bytes_syntax(&paren.expr),
            Expr::TsNonNull(assertion) => self.expr_is_bytes_syntax(&assertion.expr),
            Expr::Call(call) => {
                let Callee::Expr(callee) = &call.callee else { return false };
                let Expr::Member(member) = callee.as_ref() else { return false };
                let MemberProp::Ident(property) = &member.prop else { return false };
                match member.obj.as_ref() {
                    Expr::Ident(object) if object.sym == *"Buffer" => matches!(
                        property.sym.as_ref(),
                        "from" | "alloc" | "allocUnsafe" | "allocUnsafeSlow" | "concat"
                    ),
                    Expr::Ident(object) if object.sym == *"Uint8Array" => {
                        matches!(property.sym.as_ref(), "from" | "fromHex" | "fromBase64")
                    }
                    object => {
                        matches!(
                            property.sym.as_ref(),
                            "subarray" | "slice" | "map" | "filter" | "toSorted" | "toReversed"
                                | "with" | "fill" | "reverse" | "sort" | "copyWithin"
                        ) && self.expr_is_bytes_syntax(object)
                    }
                }
            }
            other => self.peek_type_without_lowering(other) == Some(HirType::Bytes),
        }
    }

    /// A generic `Array.prototype` method on a `Buffer` receiver: the method runs on a
    /// widened `number[]` copy, and a mutator (`fill`/`reverse`/`sort`/`copyWithin`)
    /// narrows the copy back into the buffer, which is also the call's value.
    fn lower_bytes_generic_array_method(
        &mut self,
        member: &MemberExpr,
        property: &swc_ecma_ast::IdentName,
        call: &CallExpr,
    ) -> Result<Option<HirExpr>, String> {
        let name = property.sym.as_ref();
        if !matches!(
            name,
            "map" | "filter" | "forEach" | "reduce" | "reduceRight" | "some" | "every" | "find"
                | "findIndex" | "findLast" | "findLastIndex" | "join" | "at" | "flat" | "flatMap"
                | "fill" | "reverse" | "sort" | "toSorted" | "toReversed" | "copyWithin"
                | "entries" | "keys" | "values" | "with" | "toSpliced"
        ) || !self.expr_is_bytes_syntax(&member.obj)
        {
            return Ok(None);
        }
        let receiver = self.lower_required_member_receiver(&member.obj, name)?;
        if self.infer_expr_type_inner(&receiver)? != HirType::Bytes {
            return Ok(None);
        }
        if name == "fill" {
            return Ok(Some(self.lower_native_bytes_fill(receiver, call)?));
        }
        // `reverse()` and a comparator-less `sort()` (numeric on a typed array) run in place.
        if (name == "reverse" || name == "sort") && call.args.is_empty() {
            let mut bindings = Vec::new();
            let receiver = self.bind_native_bytes_receiver(receiver, &mut bindings);
            let order = if name == "reverse" { 0.0 } else { 1.0 };
            let result = HirExpr::Block(vec![
                HirStmt::Expr(HirExpr::Call(
                    Box::new(HirExpr::Var("__thaw_bytes_reorder".into())),
                    vec![receiver.clone(), HirExpr::Lit(HirLit::F64(order))],
                )),
                HirStmt::Return(Some(receiver)),
            ]);
            return Ok(Some(self.wrap_call_argument_bindings(result, &bindings)?));
        }
        let id = self.next_binding;
        self.next_binding += 1;
        let (bytes_name, array_name) = (
            format!("__thaw_bytes_generic_receiver_{id}"),
            format!("__thaw_bytes_generic_array_{id}"),
        );
        let array_type = HirType::Array(Box::new(HirType::F64));
        self.scope.insert(bytes_name.clone(), HirType::Bytes);
        self.scope.insert(array_name.clone(), array_type.clone());
        let mut synthetic_member = member.clone();
        *synthetic_member.obj = Expr::Ident(swc_ecma_ast::Ident::new_no_ctxt(
            array_name.clone().into(),
            member.span,
        ));
        let mut synthetic_call = call.clone();
        synthetic_call.callee = Callee::Expr(Box::new(Expr::Member(synthetic_member.clone())));
        let result = self.lower_native_instance_builtin(&synthetic_member, property, &synthetic_call)?;
        let widened = HirExpr::Call(
            Box::new(HirExpr::Var("__thaw_bytes_to_array".into())),
            vec![HirExpr::Var(bytes_name.clone())],
        );
        let bindings = [
            (bytes_name.clone(), HirType::Bytes, receiver),
            (array_name.clone(), array_type, widened),
        ];
        // `map` / `filter` / `toSorted` / `toReversed` / `with` give a buffer back, with the
        // elements wrapped modulo 256.
        let result = if matches!(name, "map" | "filter" | "toSorted" | "toReversed" | "with")
            && self.infer_expr_type(&result)? == HirType::Array(Box::new(HirType::F64))
        {
            HirExpr::Call(Box::new(HirExpr::Var("__thaw_bytes_from_array".into())), vec![result])
        } else {
            result
        };
        let result = if matches!(name, "fill" | "reverse" | "sort" | "copyWithin") {
            HirExpr::Block(vec![
                HirStmt::Expr(result),
                HirStmt::Expr(HirExpr::Call(
                    Box::new(HirExpr::Var("__thaw_bytes_store_array".into())),
                    vec![HirExpr::Var(bytes_name.clone()), HirExpr::Var(array_name)],
                )),
                HirStmt::Return(Some(HirExpr::Var(bytes_name))),
            ])
        } else {
            result
        };
        Ok(Some(self.wrap_call_argument_bindings(result, &bindings)?))
    }

    /// `buf.fill(value, start?, end?)` / `buf.fill(string, encoding)`: the value is encoded
    /// to a byte pattern that the runtime repeats over the range, in place.
    fn lower_native_bytes_fill(&mut self, receiver: HirExpr, call: &CallExpr) -> Result<HirExpr, String> {
        let (arguments, mut bindings) = self.lower_native_spread_values(&call.args, "Buffer.fill")?;
        let receiver = self.bind_native_bytes_receiver(receiver, &mut bindings);
        let value = arguments.first().cloned().ok_or("`Buffer.prototype.fill` expects a value")?;
        let number = |lowerer: &mut Self, index: usize, default: f64| -> Result<HirExpr, String> {
            match arguments.get(index) {
                Some(value) => lowerer.coerce_primitive_to_number(value.clone()),
                None => Ok(HirExpr::Lit(HirLit::F64(default))),
            }
        };
        let encoding_second = arguments.get(1).map(|value| self.infer_expr_type(value)).transpose()? == Some(HirType::Str);
        let (start, end, encoding) = if encoding_second {
            (HirExpr::Lit(HirLit::F64(0.0)), HirExpr::Lit(HirLit::F64(f64::INFINITY)), arguments.get(1).cloned())
        } else {
            (number(self, 1, 0.0)?, number(self, 2, f64::INFINITY)?, arguments.get(3).cloned())
        };
        let pattern = match self.infer_expr_type(&value)? {
            HirType::Str => HirExpr::Call(
                Box::new(HirExpr::Var("__thaw_bytes_from_string".into())),
                vec![value, encoding.unwrap_or_else(|| HirExpr::Lit(HirLit::Str("utf8".into())))],
            ),
            HirType::Array(element) if *element == HirType::F64 => self.as_bytes_value(value)?,
            _ => {
                let number = self.coerce_primitive_to_number(value)?;
                HirExpr::Call(
                    Box::new(HirExpr::Var("__thaw_bytes_from_array".into())),
                    vec![HirExpr::ArrayLit(vec![number])],
                )
            }
        };
        let result = HirExpr::Block(vec![
            HirStmt::Expr(HirExpr::Call(
                Box::new(HirExpr::Var("__thaw_bytes_fill".into())),
                vec![receiver.clone(), pattern, start, end],
            )),
            HirStmt::Return(Some(receiver)),
        ]);
        self.wrap_call_argument_bindings(result, &bindings)
    }

    /// Binds `expr` once as an `f64` local so range checks and the access share it.
    fn bind_bytes_number(
        &mut self,
        prefix: &str,
        expr: HirExpr,
        bindings: &mut Vec<LoweredBinding>,
    ) -> HirExpr {
        let name = format!("{prefix}_{}", self.next_binding);
        self.next_binding += 1;
        self.scope.insert(name.clone(), HirType::F64);
        bindings.push((name.clone(), HirType::F64, expr));
        HirExpr::Var(name)
    }

    /// `throw new RangeError(<parts>)` carrying a Node `code`.
    fn bytes_range_throw(parts: Vec<HirExpr>, code: &str) -> HirExpr {
        let text = |value: &str| HirExpr::Lit(HirLit::Str(value.to_string()));
        let mut all = vec![text("\u{1}RangeError\u{1}")];
        all.extend(parts);
        if !code.is_empty() {
            all.push(text(&format!("\u{3}{code}")));
        }
        let message = all
            .into_iter()
            .reduce(|left, right| {
                HirExpr::Call(Box::new(HirExpr::Var("__thaw_string_concat".into())), vec![left, right])
            })
            .expect("non-empty message");
        HirExpr::ThrowValue(Box::new(message), Box::new(HirExpr::Lit(HirLit::F64(0.0))))
    }

    /// Node's `byteLength` check for the 1-6 byte `readUInt*`/`writeInt*` forms.
    fn bytes_byte_length_check(byte_length: &HirExpr) -> HirExpr {
        let number = |value: f64| HirExpr::Lit(HirLit::F64(value));
        let error = Self::bytes_range_throw(
            vec![
                HirExpr::Lit(HirLit::Str(
                    "The value of \"byteLength\" is out of range. It must be >= 1 and <= 6. Received ".to_string(),
                )),
                HirExpr::Call(Box::new(HirExpr::Var("__thaw_number_to_string".into())), vec![byte_length.clone()]),
            ],
            "ERR_OUT_OF_RANGE",
        );
        let bin = |op: BinOp, right: f64| {
            Box::new(HirExpr::BinOp(op, Box::new(byte_length.clone()), Box::new(number(right))))
        };
        let branch = |cond: Box<HirExpr>, then: HirExpr, other: HirExpr| {
            HirExpr::Conditional(cond, Box::new(then), Box::new(other), HirType::F64)
        };
        let integer = Box::new(HirExpr::BinOp(BinOp::EqEqEq, bin(BinOp::Mod, 1.0), Box::new(number(0.0))));
        branch(
            bin(BinOp::GtEq, 1.0),
            branch(bin(BinOp::LtEq, 6.0), branch(integer, number(0.0), error.clone()), error.clone()),
            error,
        )
    }

    /// Node's `ERR_OUT_OF_RANGE` / `ERR_BUFFER_OUT_OF_BOUNDS` checks for a
    /// fixed-width read or write at `offset` (and, for a write of an integer
    /// type, `value` within `[min, max]`). Evaluates to `0` when valid.
    fn bytes_access_check(
        receiver: &HirExpr,
        offset: &HirExpr,
        width: &HirExpr,
        value_range: Option<(&HirExpr, f64, f64)>,
    ) -> HirExpr {
        let number = |value: f64| HirExpr::Lit(HirLit::F64(value));
        let text = |value: &str| HirExpr::Lit(HirLit::Str(value.to_string()));
        let to_text = |value: &HirExpr| {
            HirExpr::Call(Box::new(HirExpr::Var("__thaw_number_to_string".into())), vec![value.clone()])
        };
        let binary = |op: BinOp, left: &HirExpr, right: HirExpr| {
            HirExpr::BinOp(op, Box::new(left.clone()), Box::new(right))
        };
        let branch = |cond: HirExpr, then: HirExpr, other: HirExpr| {
            HirExpr::Conditional(Box::new(cond), Box::new(then), Box::new(other), HirType::F64)
        };
        let available = binary(BinOp::Sub, &HirExpr::ArrayLen(Box::new(receiver.clone())), width.clone());
        let range_error = Self::bytes_range_throw(
            vec![
                text("The value of \"offset\" is out of range. It must be >= 0 and <= "),
                to_text(&available),
                text(". Received "),
                to_text(offset),
            ],
            "ERR_OUT_OF_RANGE",
        );
        let bounds = branch(
            binary(BinOp::Lt, &available, number(0.0)),
            Self::bytes_range_throw(
                vec![text("Attempt to access memory outside buffer bounds")],
                "ERR_BUFFER_OUT_OF_BOUNDS",
            ),
            branch(
                binary(BinOp::Lt, offset, number(0.0)),
                range_error.clone(),
                branch(binary(BinOp::Gt, offset, available), range_error, number(0.0)),
            ),
        );
        let integer = branch(
            binary(BinOp::EqEqEq, &binary(BinOp::Mod, offset, number(1.0)), number(0.0)),
            bounds,
            Self::bytes_range_throw(
                vec![
                    text("The value of \"offset\" is out of range. It must be an integer. Received "),
                    to_text(offset),
                ],
                "ERR_OUT_OF_RANGE",
            ),
        );
        let Some((value, min, max)) = value_range else {
            return integer;
        };
        let value_error = Self::bytes_range_throw(
            vec![
                text(&format!(
                    "The value of \"value\" is out of range. It must be >= {min} and <= {max}. Received "
                )),
                to_text(value),
            ],
            "ERR_OUT_OF_RANGE",
        );
        branch(
            binary(BinOp::Lt, value, number(min)),
            value_error.clone(),
            branch(binary(BinOp::Gt, value, number(max)), value_error, integer),
        )
    }

    // Bind the byte buffer before argument bindings; use the raw Bytes type
    // so chained byte methods retain their native dispatch identity.
    fn bind_native_bytes_receiver(
        &mut self,
        receiver: HirExpr,
        bindings: &mut Vec<LoweredBinding>,
    ) -> HirExpr {
        let name = format!("__thaw_bytes_receiver_{}", self.next_binding);
        self.next_binding += 1;
        self.scope.insert(name.clone(), HirType::Bytes);
        bindings.insert(0, (name.clone(), HirType::Bytes, receiver));
        HirExpr::Var(name)
    }

    /// `buf.readUInt16BE(offset)` / `buf.writeInt32LE(value, offset)` /
    /// `buf.readDoubleLE(offset)` &c. -- fixed-width integer / float
    /// reads and writes over the native byte layout, dispatched by
    /// `bytes_numeric_accessor`'s `(width, signed, float, big-endian,
    /// write)` decode. Reads return the value; writes mutate the buffer
    /// in place and return `offset + width` (Node's contract). A
    /// non-`Bytes` receiver, or an out-of-range offset at run time, is an
    /// error / a `0` respectively rather than a `RangeError` (thaw can't
    /// throw across this boundary).
    fn lower_native_bytes_accessor(
        &mut self,
        member: &MemberExpr,
        property: &swc_ecma_ast::IdentName,
        call: &CallExpr,
    ) -> Result<HirExpr, String> {
        let (width, signed, float, big_endian, write) =
            Self::bytes_numeric_accessor(property.sym.as_ref())
                .expect("dispatch checked bytes_numeric_accessor");
        let receiver = self.lower_required_member_receiver(&member.obj, property.sym.as_ref())?;
        if self.infer_expr_type_inner(&receiver)? != HirType::Bytes {
            return Err(format!(
                "`.{}()` is only supported on a Buffer / Uint8Array",
                property.sym
            ));
        }
        let (arguments, mut bindings) =
            self.lower_native_spread_values(&call.args, &format!("Buffer.{}", property.sym))?;
        let receiver = self.bind_native_bytes_receiver(receiver, &mut bindings);
        let kind = if float {
            2.0
        } else if signed {
            1.0
        } else {
            0.0
        };
        let width_lit = HirExpr::Lit(HirLit::F64(width as f64));
        let kind_lit = HirExpr::Lit(HirLit::F64(kind));
        let le_lit = HirExpr::Lit(HirLit::F64(if big_endian { 0.0 } else { 1.0 }));
        let number = |lowerer: &mut Self, value: HirExpr| lowerer.coerce_primitive_to_number(value);
        let result = if write {
            let [value, offset] = arguments.as_slice() else {
                return Err(format!(
                    "`Buffer.prototype.{}` expects a value and an offset",
                    property.sym
                ));
            };
            let value = number(self, value.clone())?;
            let offset = number(self, offset.clone())?;
            let value = self.bind_bytes_number("__thaw_bytes_write_value", value, &mut bindings);
            let offset = self.bind_bytes_number("__thaw_bytes_write_offset", offset, &mut bindings);
            // Integer writes also reject a value outside the type's range.
            let bits = (width * 8) as i32;
            let value_range = (!float && width <= 4).then(|| {
                if signed {
                    (-(2f64.powi(bits - 1)), 2f64.powi(bits - 1) - 1.0)
                } else {
                    (0.0, 2f64.powi(bits) - 1.0)
                }
            });
            let check = Self::bytes_access_check(
                &receiver,
                &offset,
                &width_lit,
                value_range.as_ref().map(|(min, max)| (&value, *min, *max)),
            );
            HirExpr::EvalThen(
                Box::new(check),
                Box::new(HirExpr::Call(
                    Box::new(HirExpr::Var("__thaw_bytes_write".to_string())),
                    vec![receiver, offset, value, width_lit, kind_lit, le_lit],
                )),
            )
        } else {
            let [offset] = arguments.as_slice() else {
                return Err(format!(
                    "`Buffer.prototype.{}` expects an offset",
                    property.sym
                ));
            };
            let offset = number(self, offset.clone())?;
            let offset = self.bind_bytes_number("__thaw_bytes_read_offset", offset, &mut bindings);
            let check = Self::bytes_access_check(&receiver, &offset, &width_lit, None);
            HirExpr::EvalThen(
                Box::new(check),
                Box::new(HirExpr::Call(
                    Box::new(HirExpr::Var("__thaw_bytes_read".to_string())),
                    vec![receiver, offset, width_lit, kind_lit, le_lit],
                )),
            )
        };
        self.wrap_call_argument_bindings(result, &bindings)
    }

    /// `buf.readUIntLE(offset, byteLength)` / `buf.writeIntBE(value,
    /// offset, byteLength)` &c. -- the variable-width sibling of
    /// `lower_native_bytes_accessor`: `byteLength` is a real runtime
    /// argument (clamped to 1-6 by `thaw_bytes_read`/`thaw_bytes_write`
    /// themselves; anything else reads as `0`/is a no-op, matching this
    /// API's existing "no thrown `RangeError` across this boundary"
    /// convention) rather than a name-derived compile-time constant.
    fn lower_native_bytes_variable_width_accessor(
        &mut self,
        member: &MemberExpr,
        property: &swc_ecma_ast::IdentName,
        call: &CallExpr,
    ) -> Result<HirExpr, String> {
        let (signed, big_endian, write) =
            Self::bytes_variable_width_accessor(property.sym.as_ref())
                .expect("dispatch checked bytes_variable_width_accessor");
        let receiver = self.lower_required_member_receiver(&member.obj, property.sym.as_ref())?;
        if self.infer_expr_type_inner(&receiver)? != HirType::Bytes {
            return Err(format!(
                "`.{}()` is only supported on a Buffer / Uint8Array",
                property.sym
            ));
        }
        let (arguments, mut bindings) =
            self.lower_native_spread_values(&call.args, &format!("Buffer.{}", property.sym))?;
        let receiver = self.bind_native_bytes_receiver(receiver, &mut bindings);
        let kind_lit = HirExpr::Lit(HirLit::F64(if signed { 1.0 } else { 0.0 }));
        let le_lit = HirExpr::Lit(HirLit::F64(if big_endian { 0.0 } else { 1.0 }));
        let number = |lowerer: &mut Self, value: HirExpr| lowerer.coerce_primitive_to_number(value);
        let result = if write {
            let [value, offset, byte_length] = arguments.as_slice() else {
                return Err(format!(
                    "`Buffer.prototype.{}` expects a value, an offset, and a byte length",
                    property.sym
                ));
            };
            let value = number(self, value.clone())?;
            let offset = number(self, offset.clone())?;
            let byte_length = number(self, byte_length.clone())?;
            let offset = self.bind_bytes_number("__thaw_bytes_write_offset", offset, &mut bindings);
            let byte_length = self.bind_bytes_number("__thaw_bytes_write_width", byte_length, &mut bindings);
            let check = HirExpr::EvalThen(
                Box::new(Self::bytes_byte_length_check(&byte_length)),
                Box::new(Self::bytes_access_check(&receiver, &offset, &byte_length, None)),
            );
            HirExpr::EvalThen(
                Box::new(check),
                Box::new(HirExpr::Call(
                    Box::new(HirExpr::Var("__thaw_bytes_write".to_string())),
                    vec![receiver, offset, value, byte_length, kind_lit, le_lit],
                )),
            )
        } else {
            let [offset, byte_length] = arguments.as_slice() else {
                return Err(format!(
                    "`Buffer.prototype.{}` expects an offset and a byte length",
                    property.sym
                ));
            };
            let offset = number(self, offset.clone())?;
            let byte_length = number(self, byte_length.clone())?;
            let offset = self.bind_bytes_number("__thaw_bytes_read_offset", offset, &mut bindings);
            let byte_length = self.bind_bytes_number("__thaw_bytes_read_width", byte_length, &mut bindings);
            let check = HirExpr::EvalThen(
                Box::new(Self::bytes_byte_length_check(&byte_length)),
                Box::new(Self::bytes_access_check(&receiver, &offset, &byte_length, None)),
            );
            HirExpr::EvalThen(
                Box::new(check),
                Box::new(HirExpr::Call(
                    Box::new(HirExpr::Var("__thaw_bytes_read".to_string())),
                    vec![receiver, offset, byte_length, kind_lit, le_lit],
                )),
            )
        };
        self.wrap_call_argument_bindings(result, &bindings)
    }

    /// `new ArrayBuffer(n)` / `new DataView(buffer, byteOffset?, byteLength?)` over a native
    /// byte view; `None` for any other shape (the dynamic constructor path handles those).
    #[inline(never)]
    fn lower_native_arraybuffer_constructor(
        &mut self,
        class: &str,
        args: &[swc_ecma_ast::ExprOrSpread],
    ) -> Result<Option<HirExpr>, String> {
        let number = |value: f64| HirExpr::Lit(HirLit::F64(value));
        let text = |value: &str| HirExpr::Lit(HirLit::Str(value.to_string()));
        let to_text = |value: &HirExpr| {
            HirExpr::Call(Box::new(HirExpr::Var("__thaw_number_to_string".into())), vec![value.clone()])
        };
        let branch = |cond: HirExpr, then: HirExpr, other: HirExpr| {
            HirExpr::Conditional(Box::new(cond), Box::new(then), Box::new(other), HirType::F64)
        };
        let mut bindings = Vec::new();
        let result = match (class, args.len()) {
            ("ArrayBuffer", 0 | 1) => {
                let size = match args.first() {
                    Some(argument) => {
                        let size = self.lower_expr(&argument.expr)?;
                        self.coerce_primitive_to_number(size)?
                    }
                    None => number(0.0),
                };
                let size = self.bind_bytes_number("__thaw_arraybuffer_size", size, &mut bindings);
                let check = branch(
                    HirExpr::BinOp(BinOp::Lt, Box::new(size.clone()), Box::new(number(0.0))),
                    Self::bytes_range_throw(vec![text("Invalid array buffer length")], ""),
                    number(0.0),
                );
                HirExpr::EvalThen(
                    Box::new(check),
                    Box::new(HirExpr::Call(Box::new(HirExpr::Var("__thaw_arraybuffer_alloc".into())), vec![size])),
                )
            }
            ("DataView", 1..=3) => {
                let source = self.lower_expr(&args[0].expr)?;
                if self.infer_expr_type_inner(&source)? != HirType::Bytes {
                    return Ok(None);
                }
                let mut operand = |lowerer: &mut Self, index: usize, default: f64| -> Result<HirExpr, String> {
                    let value = match args.get(index) {
                        Some(argument) => {
                            let value = lowerer.lower_expr(&argument.expr)?;
                            lowerer.coerce_primitive_to_number(value)?
                        }
                        None => number(default),
                    };
                    Ok(lowerer.bind_bytes_number("__thaw_dataview_arg", value, &mut bindings))
                };
                let offset = operand(self, 1, 0.0)?;
                let length = operand(self, 2, f64::NAN)?;
                let source = self.bind_native_bytes_receiver(source, &mut bindings);
                let total = HirExpr::ArrayLen(Box::new(source.clone()));
                let compare = |op: BinOp, left: HirExpr, right: HirExpr| HirExpr::BinOp(op, Box::new(left), Box::new(right));
                let start_error = Self::bytes_range_throw(
                    vec![text("Start offset "), to_text(&offset), text(" is outside the bounds of the buffer")],
                    "",
                );
                let length_error =
                    Self::bytes_range_throw(vec![text("Invalid DataView length "), to_text(&length)], "");
                let check = branch(
                    compare(BinOp::Lt, offset.clone(), number(0.0)),
                    start_error.clone(),
                    branch(
                        compare(BinOp::Gt, offset.clone(), total.clone()),
                        start_error,
                        branch(
                            compare(
                                BinOp::Gt,
                                compare(BinOp::Add, offset.clone(), length.clone()),
                                total,
                            ),
                            length_error.clone(),
                            branch(compare(BinOp::Lt, length.clone(), number(0.0)), length_error, number(0.0)),
                        ),
                    ),
                );
                HirExpr::EvalThen(
                    Box::new(check),
                    Box::new(HirExpr::Call(
                        Box::new(HirExpr::Var("__thaw_dataview_new".into())),
                        vec![source, offset, length],
                    )),
                )
            }
            // `const u = new Uint8Array(length)`: a native
            // byte store (O(1) element access) instead of a JS-realm typed array whose every
            // element access would cross the bridge.
            ("Uint8Array", 1) if self.native_u8_args.contains(&(args.as_ptr() as usize)) => {
                let as_u8 = |buffer: HirExpr| {
                    HirExpr::Call(Box::new(HirExpr::Var("__thaw_bytes_as_u8".into())), vec![buffer])
                };
                let alloc = |size: HirExpr| HirExpr::Call(Box::new(HirExpr::Var("__thaw_bytes_alloc".into())), vec![size]);
                let argument = &args[0];
                let value = self.lower_expr(&argument.expr)?;
                match self.infer_expr_type_inner(&value)? {
                    HirType::F64 => {
                        let size = self.bind_bytes_number("__thaw_u8_length", value, &mut bindings);
                        let error =
                            Self::bytes_range_throw(vec![text("Invalid typed array length: "), to_text(&size)], "");
                        let check = branch(
                            HirExpr::BinOp(BinOp::Lt, Box::new(size.clone()), Box::new(number(0.0))),
                            error.clone(),
                            branch(
                                HirExpr::BinOp(BinOp::Gt, Box::new(size.clone()), Box::new(number(9007199254740991.0))),
                                error,
                                number(0.0),
                            ),
                        );
                        as_u8(alloc(HirExpr::EvalThen(Box::new(check), Box::new(size))))
                    }
                    _ => return Ok(None),
                }
            }
            _ => return Ok(None),
        };
        Ok(Some(self.wrap_call_argument_bindings(result, &bindings)?))
    }

    /// `ArrayBuffer.isView(value)` for a native byte view (everything but an `ArrayBuffer`).
    #[inline(never)]
    fn lower_native_array_buffer_is_view(&mut self, call: &CallExpr) -> Result<Option<HirExpr>, String> {
        let [argument] = call.args.as_slice() else {
            return Ok(None);
        };
        if argument.spread.is_some() {
            return Ok(None);
        }
        let value = self.lower_expr(&argument.expr)?;
        if self.infer_expr_type_inner(&value)? != HirType::Bytes {
            return Ok(None);
        }
        let name = format!("__thaw_is_view_source_{}", self.next_binding);
        self.next_binding += 1;
        self.scope.insert(name.clone(), HirType::Bytes);
        let is_array_buffer = HirExpr::BinOp(
            BinOp::EqEqEq,
            Box::new(HirExpr::Call(Box::new(HirExpr::Var("__thaw_bytes_kind".into())), vec![HirExpr::Var(name.clone())])),
            Box::new(HirExpr::Lit(HirLit::F64(1.0))),
        );
        let result = HirExpr::Conditional(
            Box::new(is_array_buffer),
            Box::new(HirExpr::Lit(HirLit::Bool(false))),
            Box::new(HirExpr::Lit(HirLit::Bool(true))),
            HirType::Bool,
        );
        Ok(Some(self.wrap_call_argument_bindings(result, &[(name, HirType::Bytes, value)])?))
    }

    /// `(kind, big, write)` for `DataView.prototype.get*` / `set*`; `kind` as `thaw_dataview_get`.
    fn dataview_accessor(property: &str) -> Option<(f64, bool, bool)> {
        let (write, rest) = match property.strip_prefix("set") {
            Some(rest) => (true, rest),
            None => (false, property.strip_prefix("get")?),
        };
        let (kind, big) = match rest {
            "Int8" => (-1.0, false),
            "Uint8" => (1.0, false),
            "Int16" => (-2.0, false),
            "Uint16" => (2.0, false),
            "Int32" => (-4.0, false),
            "Uint32" => (4.0, false),
            "Float16" => (-3.0, false),
            "Float32" => (-5.0, false),
            "Float64" => (-9.0, false),
            "BigInt64" | "BigUint64" => (8.0, true),
            _ => return None,
        };
        Some((kind, big, write))
    }

    /// Methods only a native `ArrayBuffer` / `DataView` has, so only claimed for a `Bytes` receiver.
    fn is_native_view_only_method(property: &str) -> bool {
        Self::dataview_accessor(property).is_some() || matches!(property, "transfer" | "transferToFixedLength")
    }

    /// `ab.transfer(newLength?)` / `ab.transferToFixedLength(newLength?)`.
    fn lower_native_arraybuffer_transfer(
        &mut self,
        member: &MemberExpr,
        property: &swc_ecma_ast::IdentName,
        call: &CallExpr,
    ) -> Result<HirExpr, String> {
        let receiver = self.lower_required_member_receiver(&member.obj, property.sym.as_ref())?;
        let (arguments, mut bindings) = self.lower_native_spread_values(&call.args, &format!("ArrayBuffer.{}", property.sym))?;
        let receiver = self.bind_native_bytes_receiver(receiver, &mut bindings);
        let length = match arguments.as_slice() {
            [] => HirExpr::Lit(HirLit::F64(f64::NAN)),
            [length] => self.coerce_primitive_to_number(length.clone())?,
            _ => return Err(format!("`ArrayBuffer.prototype.{}` takes at most one argument", property.sym)),
        };
        let result = HirExpr::Call(Box::new(HirExpr::Var("__thaw_arraybuffer_transfer".into())), vec![receiver, length]);
        self.wrap_call_argument_bindings(result, &bindings)
    }

    /// `dv.getInt32(offset, littleEndian?)` / `dv.setFloat64(offset, value, littleEndian?)`.
    fn lower_native_dataview_access(
        &mut self,
        member: &MemberExpr,
        property: &swc_ecma_ast::IdentName,
        call: &CallExpr,
    ) -> Result<HirExpr, String> {
        let (kind, big, write) = Self::dataview_accessor(property.sym.as_ref()).expect("dispatch checked dataview_accessor");
        let width = match kind.abs() as u8 {
            _ if big => 8.0,
            3 => 2.0,
            5 => 4.0,
            9 => 8.0,
            other => f64::from(other),
        };
        let number = |value: f64| HirExpr::Lit(HirLit::F64(value));
        let receiver = self.lower_required_member_receiver(&member.obj, property.sym.as_ref())?;
        let (arguments, mut bindings) = self.lower_native_spread_values(&call.args, &format!("DataView.{}", property.sym))?;
        let receiver = self.bind_native_bytes_receiver(receiver, &mut bindings);
        let operands = if write { 2 } else { 1 };
        if arguments.len() < operands || arguments.len() > operands + 1 {
            return Err(format!("`DataView.prototype.{}` has the wrong number of arguments", property.sym));
        }
        let offset = self.coerce_primitive_to_number(arguments[0].clone())?;
        let offset = self.bind_bytes_number("__thaw_dataview_offset", offset, &mut bindings);
        let little = match arguments.get(operands) {
            Some(flag) => HirExpr::Conditional(Box::new(flag.clone()), Box::new(number(1.0)), Box::new(number(0.0)), HirType::F64),
            None => number(0.0),
        };
        let out_of_range = Self::bytes_range_throw(vec![HirExpr::Lit(HirLit::Str("Offset is outside the bounds of the DataView".into()))], "");
        let end = HirExpr::BinOp(BinOp::Add, Box::new(offset.clone()), Box::new(number(width)));
        let check = HirExpr::Conditional(
            Box::new(HirExpr::BinOp(BinOp::Lt, Box::new(offset.clone()), Box::new(number(0.0)))),
            Box::new(out_of_range.clone()),
            Box::new(HirExpr::Conditional(
                Box::new(HirExpr::BinOp(BinOp::Gt, Box::new(end), Box::new(HirExpr::ArrayLen(Box::new(receiver.clone()))))),
                Box::new(out_of_range),
                Box::new(number(0.0)),
                HirType::F64,
            )),
            HirType::F64,
        );
        let call_intrinsic = |name: &str, args: Vec<HirExpr>| HirExpr::Call(Box::new(HirExpr::Var(name.to_string())), args);
        let access = match (write, big) {
            (false, false) => call_intrinsic("__thaw_dataview_get", vec![receiver, number(kind), offset, little]),
            (false, true) => {
                let read = call_intrinsic("__thaw_bytes_read_i64", vec![receiver, offset, little]);
                if property.sym.contains("Uint") { Self::unsigned_bigint_read(read) } else { read }
            }
            (true, true) => {
                self.expect_type(&HirType::BigInt, &arguments[1], "`DataView.prototype.setBigInt64` value")?;
                call_intrinsic("__thaw_bytes_write_i64", vec![receiver, offset, arguments[1].clone(), little])
            }
            (true, false) => {
                let value = self.coerce_primitive_to_number(arguments[1].clone())?;
                call_intrinsic("__thaw_dataview_set", vec![receiver, number(kind), offset, value, little])
            }
        };
        self.wrap_call_argument_bindings(HirExpr::EvalThen(Box::new(check), Box::new(access)), &bindings)
    }


    /// `BigInt.asUintN(64, read)`: unsigned 64-bit reads (`readBigUInt64*`, `getBigUint64`).
    fn unsigned_bigint_read(read: HirExpr) -> HirExpr {
        HirExpr::Call(
            Box::new(HirExpr::Var("__thaw_i64_as_uint_n".to_string())),
            vec![read, HirExpr::Lit(HirLit::F64(64.0))],
        )
    }

    /// `buf.readBigInt64LE(offset)` / `buf.writeBigUInt64BE(value,
    /// offset)` &c. -- the full-precision 64-bit sibling of
    /// `lower_native_bytes_accessor`: always exactly 8 bytes, and the
    /// value itself is a real `HirType::I64` (a genuine 64-bit integer,
    /// i.e. a TS `bigint`) rather than the `F64` every other accessor
    /// here uses -- `f64` can only exactly represent 53 bits of
    /// integer, not a full 64-bit range. Signed and unsigned share the
    /// exact same bit pattern (`__thaw_bytes_read_i64`/`_write_i64` have
    /// no separate `kind` parameter): a `BigUInt64` value `>= 2^63`
    /// round-trips correctly through a buffer but displays as negative
    /// via `.toString()`/`console.log` -- a deliberate, documented
    /// practical-subset limitation (no genuine unsigned 64-bit type
    /// exists), not a silent miscalculation.
    fn lower_native_bytes_bigint_accessor(
        &mut self,
        member: &MemberExpr,
        property: &swc_ecma_ast::IdentName,
        call: &CallExpr,
    ) -> Result<HirExpr, String> {
        let (signed, big_endian, write) = Self::bytes_bigint_accessor(property.sym.as_ref())
            .expect("dispatch checked bytes_bigint_accessor");
        let receiver = self.lower_required_member_receiver(&member.obj, property.sym.as_ref())?;
        if self.infer_expr_type_inner(&receiver)? != HirType::Bytes {
            return Err(format!(
                "`.{}()` is only supported on a Buffer / Uint8Array",
                property.sym
            ));
        }
        let (arguments, mut bindings) =
            self.lower_native_spread_values(&call.args, &format!("Buffer.{}", property.sym))?;
        let receiver = self.bind_native_bytes_receiver(receiver, &mut bindings);
        let le_lit = HirExpr::Lit(HirLit::F64(if big_endian { 0.0 } else { 1.0 }));
        let result = if write {
            let [value, offset] = arguments.as_slice() else {
                return Err(format!(
                    "`Buffer.prototype.{}` expects a value and an offset",
                    property.sym
                ));
            };
            self.expect_type(
                &HirType::BigInt,
                value,
                &format!("`Buffer.prototype.{}` value", property.sym),
            )?;
            let offset = self.coerce_primitive_to_number(offset.clone())?;
            let offset = self.bind_bytes_number("__thaw_bytes_write_offset", offset, &mut bindings);
            let check = Self::bytes_access_check(&receiver, &offset, &HirExpr::Lit(HirLit::F64(8.0)), None);
            HirExpr::EvalThen(
                Box::new(check),
                Box::new(HirExpr::Call(
                    Box::new(HirExpr::Var("__thaw_bytes_write_i64".to_string())),
                    vec![receiver, offset, value.clone(), le_lit],
                )),
            )
        } else {
            let [offset] = arguments.as_slice() else {
                return Err(format!(
                    "`Buffer.prototype.{}` expects an offset",
                    property.sym
                ));
            };
            let offset = self.coerce_primitive_to_number(offset.clone())?;
            let offset = self.bind_bytes_number("__thaw_bytes_read_offset", offset, &mut bindings);
            let check = Self::bytes_access_check(&receiver, &offset, &HirExpr::Lit(HirLit::F64(8.0)), None);
            let read = HirExpr::Call(
                Box::new(HirExpr::Var("__thaw_bytes_read_i64".to_string())),
                vec![receiver, offset, le_lit],
            );
            HirExpr::EvalThen(
                Box::new(check),
                Box::new(if signed { read } else { Self::unsigned_bigint_read(read) }),
            )
        };
        self.wrap_call_argument_bindings(result, &bindings)
    }

    /// `source.copy(target, targetStart?, sourceStart?, sourceEnd?)` --
    /// blits bytes into an existing byte buffer in place and returns the
    /// count copied (Node's contract). Both receiver and `target` must be
    /// `Bytes`; the three offsets default to `0`, `0`, and the source
    /// length (`-1` sentinel, resolved in the runtime).
    fn lower_native_bytes_copy(
        &mut self,
        member: &MemberExpr,
        call: &CallExpr,
    ) -> Result<HirExpr, String> {
        let receiver = self.lower_required_member_receiver(&member.obj, "copy")?;
        if self.infer_expr_type_inner(&receiver)? != HirType::Bytes {
            return Err("`.copy()` is only supported on a Buffer / Uint8Array".into());
        }
        let (arguments, mut bindings) =
            self.lower_native_spread_values(&call.args, "Buffer.copy")?;
        let receiver = self.bind_native_bytes_receiver(receiver, &mut bindings);
        if arguments.is_empty() || arguments.len() > 4 {
            return Err(
                "`Buffer.prototype.copy` expects a target and up to three offsets".into(),
            );
        }
        let target = arguments[0].clone();
        self.expect_type(
            &HirType::Array(Box::new(HirType::F64)),
            &target,
            "`.copy()` target",
        )?;
        let target = self.as_bytes_value(target)?;
        let offset = |lowerer: &mut Self, index: usize, default: f64| -> Result<HirExpr, String> {
            match arguments.get(index) {
                Some(value) => lowerer.coerce_primitive_to_number(value.clone()),
                None => Ok(HirExpr::Lit(HirLit::F64(default))),
            }
        };
        let target_start = offset(self, 1, 0.0)?;
        let source_start = offset(self, 2, 0.0)?;
        let source_end = offset(self, 3, -1.0)?;
        let result = HirExpr::Call(
            Box::new(HirExpr::Var("__thaw_bytes_copy".to_string())),
            vec![receiver, target, target_start, source_start, source_end],
        );
        self.wrap_call_argument_bindings(result, &bindings)
    }

    /// `Uint8Array.prototype.set(source, offset?)` -- copies `source`
    /// (a numeric array or byte buffer) into the receiver starting at
    /// `offset` (default 0). The inverse of `Buffer.prototype.copy`:
    /// the receiver is the copy *target*, so it reuses the same in-place
    /// copy with the operands swapped.
    fn lower_native_bytes_set(
        &mut self,
        receiver: HirExpr,
        call: &CallExpr,
    ) -> Result<HirExpr, String> {
        let (arguments, mut bindings) =
            self.lower_native_spread_values(&call.args, "Uint8Array.set")?;
        let receiver = self.bind_native_bytes_receiver(receiver, &mut bindings);
        let (source, offset) = match arguments.as_slice() {
            [source] => (source.clone(), HirExpr::Lit(HirLit::F64(0.0))),
            [source, offset] => (source.clone(), offset.clone()),
            _ => return Err("native `.set()` expects a source and an optional offset".into()),
        };
        let offset = self.coerce_primitive_to_number(offset)?;
        // A live typed array from the JS realm (`new Uint8Array([9])`) is read as a `number[]`.
        let source = if self.infer_expr_type(&source)? == HirType::JsValue {
            self.coerce_to_declared(&HirType::Array(Box::new(HirType::F64)), source)?
        } else {
            source
        };
        let source = self.as_bytes_value(source)?;
        // Source and offset are read twice (range check, then copy): bind both once.
        let source_type = self.infer_expr_type(&source)?;
        let source_name = format!("__thaw_bytes_set_source_{}", self.next_binding);
        let offset_name = format!("__thaw_bytes_set_offset_{}", self.next_binding);
        self.next_binding += 1;
        self.scope.insert(source_name.clone(), source_type.clone());
        self.scope.insert(offset_name.clone(), HirType::F64);
        bindings.push((source_name.clone(), source_type.clone(), source));
        bindings.push((offset_name.clone(), HirType::F64, offset));
        let source = HirExpr::Var(source_name);
        let offset = HirExpr::Var(offset_name);
        // A byte buffer shares the array layout, so both lengths read the same header.
        let source_length = HirExpr::ArrayLen(Box::new(source.clone()));
        let receiver_length = HirExpr::ArrayLen(Box::new(receiver.clone()));
        // A source that does not fit raises RangeError before any byte is written.
        let overflow = HirExpr::BinOp(
            BinOp::Gt,
            Box::new(HirExpr::BinOp(
                BinOp::Add,
                Box::new(offset.clone()),
                Box::new(source_length),
            )),
            Box::new(receiver_length),
        );
        let negative = HirExpr::BinOp(
            BinOp::Lt,
            Box::new(offset.clone()),
            Box::new(HirExpr::Lit(HirLit::F64(0.0))),
        );
        let out_of_range = HirExpr::Conditional(
            Box::new(negative),
            Box::new(HirExpr::Lit(HirLit::Bool(true))),
            Box::new(overflow),
            HirType::Bool,
        );
        let range_check = HirExpr::Conditional(
            Box::new(out_of_range),
            Box::new(HirExpr::ThrowValue(
                Box::new(HirExpr::Lit(HirLit::Str(
                    "\u{1}RangeError\u{1}offset is out of bounds".into(),
                ))),
                Box::new(HirExpr::Lit(HirLit::F64(0.0))),
            )),
            Box::new(HirExpr::Lit(HirLit::F64(0.0))),
            HirType::F64,
        );
        let copy = HirExpr::Call(
            Box::new(HirExpr::Var("__thaw_bytes_copy".to_string())),
            vec![
                source,
                receiver,
                offset,
                HirExpr::Lit(HirLit::F64(0.0)),
                HirExpr::Lit(HirLit::F64(-1.0)),
            ],
        );
        // `Uint8Array.prototype.set` mutates the receiver but returns undefined.
        // Keep `__thaw_bytes_copy`'s numeric result for Buffer.copy callers.
        let result = HirExpr::EvalThen(
            Box::new(HirExpr::EvalThen(Box::new(range_check), Box::new(copy))),
            Box::new(HirExpr::Lit(HirLit::Undefined)),
        );
        self.wrap_call_argument_bindings(result, &bindings)
    }

    /// `buf.indexOf` / `includes` / `lastIndexOf` with a string or
    /// sub-buffer needle -- a byte-subsequence search. The needle is
    /// normalised to a byte buffer (a string decodes as `utf8`, a number
    /// becomes a one-byte buffer), then `__thaw_bytes_index_of` scans
    /// `haystack` from `from` (`last` = 1 reverses). `includes` is that
    /// index `>= 0`. An empty needle matches at the clamped `from`, like
    /// Node. A negative `from` clamps to 0 (Node's from-the-end offset
    /// isn't modelled).
    fn lower_native_bytes_search(
        &mut self,
        receiver: HirExpr,
        method: &str,
        arguments: Vec<HirExpr>,
        mut spread_bindings: Vec<(Symbol, HirType, HirExpr)>,
    ) -> Result<HirExpr, String> {
        let receiver = self.bind_native_bytes_receiver(receiver, &mut spread_bindings);
        let needle = arguments[0].clone();
        let needle_type = self.infer_expr_type(&needle)?;
        let needle_bytes = match &needle_type {
            HirType::Str => HirExpr::Call(
                Box::new(HirExpr::Var("__thaw_bytes_from_string".to_string())),
                vec![needle, HirExpr::Lit(HirLit::Str("utf8".to_string()))],
            ),
            HirType::F64 => HirExpr::Call(
                Box::new(HirExpr::Var("__thaw_bytes_from_array".into())),
                vec![HirExpr::ArrayLit(vec![needle])],
            ),
            HirType::Array(inner) if **inner == HirType::F64 => self.as_bytes_value(needle)?,
            other => {
                return Err(format!(
                    "`Buffer.prototype.{method}` needle must be a string, a byte buffer, or a number (got {other:?})"
                ))
            }
        };
        let last = method == "lastIndexOf";
        let from = match arguments.get(1) {
            Some(value) => self.coerce_primitive_to_number(value.clone())?,
            None if last => HirExpr::Lit(HirLit::F64(f64::INFINITY)),
            None => HirExpr::Lit(HirLit::F64(0.0)),
        };
        let index = HirExpr::Call(
            Box::new(HirExpr::Var("__thaw_bytes_index_of".to_string())),
            vec![
                receiver,
                needle_bytes,
                from,
                HirExpr::Lit(HirLit::F64(if last { 1.0 } else { 0.0 })),
            ],
        );
        let result = if method == "includes" {
            HirExpr::BinOp(
                BinOp::GtEq,
                Box::new(index),
                Box::new(HirExpr::Lit(HirLit::F64(0.0))),
            )
        } else {
            index
        };
        self.wrap_call_argument_bindings(result, &spread_bindings)
    }

    /// `Uint8Array.prototype.toHex()` / `.toBase64()` and
    /// `.setFromHex(text)` / `.setFromBase64(text)` (ECMAScript 2026). The
    /// `set*` variants write into the buffer from offset 0 and return
    /// `{ read, written }` from the native capacity-bounded decoder.
    fn lower_native_bytes_encoding_method(
        &mut self,
        member: &MemberExpr,
        property: &swc_ecma_ast::IdentName,
        call: &CallExpr,
    ) -> Result<HirExpr, String> {
        let receiver = self.lower_required_member_receiver(&member.obj, property.sym.as_ref())?;
        if self.infer_expr_type_inner(&receiver)? != HirType::Bytes {
            return Err(format!(
                "`.{}()` is only supported on a Buffer / Uint8Array",
                property.sym
            ));
        }
        let encoding = if matches!(property.sym.as_ref(), "toHex" | "setFromHex") {
            "hex"
        } else {
            "base64"
        };
        let encoding_lit = HirExpr::Lit(HirLit::Str(encoding.to_string()));
        let (arguments, mut bindings) = self
            .lower_native_spread_values(&call.args, &format!("Uint8Array.{}", property.sym))?;
        let receiver = self.bind_native_bytes_receiver(receiver, &mut bindings);
        if matches!(property.sym.as_ref(), "toHex" | "toBase64") {
            if !arguments.is_empty() {
                return Err(format!("`{}` expects no arguments", property.sym));
            }
            return self.wrap_call_argument_bindings(
                HirExpr::Call(
                    Box::new(HirExpr::Var("__thaw_bytes_to_string".to_string())),
                    vec![receiver, encoding_lit],
                ),
                &bindings,
            );
        }
        let [source] = arguments.as_slice() else {
            return Err(format!("`{}` expects exactly one argument", property.sym));
        };
        let source = self.coerce_primitive_to_string(source.clone())?;
        let result = HirExpr::Call(
            Box::new(HirExpr::Var("__thaw_bytes_set_from_string".to_string())),
            vec![receiver, source, encoding_lit],
        );
        self.wrap_call_argument_bindings(result, &bindings)
    }
}
