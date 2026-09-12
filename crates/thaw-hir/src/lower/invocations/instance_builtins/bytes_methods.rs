impl<'a> FnLowerer<'a> {
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
        let receiver = self.lower_expr(&member.obj)?;
        if self.infer_expr_type_inner(&receiver)? != HirType::Bytes {
            return Err(format!(
                "`.{}()` is only supported on a Buffer / Uint8Array",
                property.sym
            ));
        }
        let (arguments, bindings) =
            self.lower_native_spread_values(&call.args, &format!("Buffer.{}", property.sym))?;
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
            HirExpr::Call(
                Box::new(HirExpr::Var("__thaw_bytes_write".to_string())),
                vec![receiver, offset, value, width_lit, kind_lit, le_lit],
            )
        } else {
            let [offset] = arguments.as_slice() else {
                return Err(format!(
                    "`Buffer.prototype.{}` expects an offset",
                    property.sym
                ));
            };
            let offset = number(self, offset.clone())?;
            HirExpr::Call(
                Box::new(HirExpr::Var("__thaw_bytes_read".to_string())),
                vec![receiver, offset, width_lit, kind_lit, le_lit],
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
        let receiver = self.lower_expr(&member.obj)?;
        if self.infer_expr_type_inner(&receiver)? != HirType::Bytes {
            return Err(format!(
                "`.{}()` is only supported on a Buffer / Uint8Array",
                property.sym
            ));
        }
        let (arguments, bindings) =
            self.lower_native_spread_values(&call.args, &format!("Buffer.{}", property.sym))?;
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
            HirExpr::Call(
                Box::new(HirExpr::Var("__thaw_bytes_write".to_string())),
                vec![receiver, offset, value, byte_length, kind_lit, le_lit],
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
            HirExpr::Call(
                Box::new(HirExpr::Var("__thaw_bytes_read".to_string())),
                vec![receiver, offset, byte_length, kind_lit, le_lit],
            )
        };
        self.wrap_call_argument_bindings(result, &bindings)
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
        let (_signed, big_endian, write) = Self::bytes_bigint_accessor(property.sym.as_ref())
            .expect("dispatch checked bytes_bigint_accessor");
        let receiver = self.lower_expr(&member.obj)?;
        if self.infer_expr_type_inner(&receiver)? != HirType::Bytes {
            return Err(format!(
                "`.{}()` is only supported on a Buffer / Uint8Array",
                property.sym
            ));
        }
        let (arguments, bindings) =
            self.lower_native_spread_values(&call.args, &format!("Buffer.{}", property.sym))?;
        let le_lit = HirExpr::Lit(HirLit::F64(if big_endian { 0.0 } else { 1.0 }));
        let result = if write {
            let [value, offset] = arguments.as_slice() else {
                return Err(format!(
                    "`Buffer.prototype.{}` expects a value and an offset",
                    property.sym
                ));
            };
            self.expect_type(
                &HirType::I64,
                value,
                &format!("`Buffer.prototype.{}` value", property.sym),
            )?;
            let offset = self.coerce_primitive_to_number(offset.clone())?;
            HirExpr::Call(
                Box::new(HirExpr::Var("__thaw_bytes_write_i64".to_string())),
                vec![receiver, offset, value.clone(), le_lit],
            )
        } else {
            let [offset] = arguments.as_slice() else {
                return Err(format!(
                    "`Buffer.prototype.{}` expects an offset",
                    property.sym
                ));
            };
            let offset = self.coerce_primitive_to_number(offset.clone())?;
            HirExpr::Call(
                Box::new(HirExpr::Var("__thaw_bytes_read_i64".to_string())),
                vec![receiver, offset, le_lit],
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
        let receiver = self.lower_expr(&member.obj)?;
        if self.infer_expr_type_inner(&receiver)? != HirType::Bytes {
            return Err("`.copy()` is only supported on a Buffer / Uint8Array".into());
        }
        let (arguments, bindings) =
            self.lower_native_spread_values(&call.args, "Buffer.copy")?;
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
        spread_bindings: Vec<(Symbol, HirType, HirExpr)>,
    ) -> Result<HirExpr, String> {
        let needle = arguments[0].clone();
        let needle_type = self.infer_expr_type(&needle)?;
        let needle_bytes = match &needle_type {
            HirType::Str => HirExpr::Call(
                Box::new(HirExpr::Var("__thaw_bytes_from_string".to_string())),
                vec![needle, HirExpr::Lit(HirLit::Str("utf8".to_string()))],
            ),
            HirType::F64 => HirExpr::ArrayLit(vec![needle]),
            HirType::Array(inner) if **inner == HirType::F64 => needle,
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
}
