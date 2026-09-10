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
}
