impl<'ctx> HirCompiler<'ctx> {
    fn compile_console_log(
        &mut self,
        args: &[HirExpr],
        stderr: bool,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let descriptor = if stderr { 2 } else { 1 };
        let values = args
            .iter()
            .map(|arg| match arg {
                // HIR tags a byte buffer (erased to `number[]`) so it prints as `<Buffer ..>`.
                HirExpr::Call(callee, inner) if matches!(callee.as_ref(), HirExpr::Var(name) if name == "__thaw_console_bytes") => {
                    Ok((Some(HirType::Bytes), self.compile_expr(&inner[0])?))
                }
                _ => Ok((self.expr_hir_type(arg), self.compile_expr(arg)?)),
            })
            .collect::<Result<Vec<_>, String>>()?;
        self.compile_console_values(values, descriptor)?;
        Ok(self.context.i32_type().const_int(0, false).into())
    }

    fn compile_console_assert(
        &mut self,
        args: &[HirExpr],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let mut values = args
            .iter()
            .map(|arg| Ok((self.expr_hir_type(arg), self.compile_expr(arg)?)))
            .collect::<Result<Vec<_>, String>>()?;
        let condition = if values.is_empty() {
            self.context.bool_type().const_zero()
        } else {
            values.remove(0).1.into_int_value()
        };
        let function = self.current_function();
        let failed = self.context.append_basic_block(function, "console_assert_failed");
        let done = self.context.append_basic_block(function, "console_assert_done");
        self.builder
            .build_conditional_branch(condition, done, failed)
            .map_err(|error| error.to_string())?;

        self.builder.position_at_end(failed);
        let prefix = self
            .builder
            .build_global_string_ptr(
                if values.is_empty() {
                    "Assertion failed"
                } else {
                    "Assertion failed: "
                },
                "console_assert_prefix",
            )
            .map_err(|error| error.to_string())?;
        self.compile_console_text(
            prefix.as_pointer_value(),
            values.is_empty(),
            "console_assert_prefix",
            2,
        )?;
        if !values.is_empty() {
            self.compile_console_values(values, 2)?;
        } else {
            self.flush_console()?;
        }
        self.builder
            .build_unconditional_branch(done)
            .map_err(|error| error.to_string())?;
        self.builder.position_at_end(done);
        Ok(self.context.i32_type().const_int(0, false).into())
    }

    fn compile_console_values(
        &mut self,
        values: Vec<(Option<HirType>, BasicValueEnum<'ctx>)>,
        descriptor: u64,
    ) -> Result<(), String> {
        if values.is_empty() {
            let empty = self
                .builder
                .build_global_string_ptr("", "console_empty")
                .map_err(|error| error.to_string())?;
            self.compile_console_text(empty.as_pointer_value(), true, "console_empty", descriptor)?;
        }
        // `console.log("%s is %d", ...)`: a leading string with further arguments follows
        // `util.format`, so the whole argument list goes through one runtime formatter.
        if values.len() > 1
            && matches!(values[0].0, Some(HirType::Str))
            && values.iter().all(|(ty, _)| matches!(ty,
                Some(HirType::Str | HirType::F64 | HirType::Bool | HirType::Json
                    | HirType::Array(_) | HirType::Tuple(_) | HirType::Object(_))))
        {
            return self.compile_console_format(values, descriptor);
        }
        let value_count = values.len();
        for (index, (hir_type, value)) in values.into_iter().enumerate() {
            self.compile_console_arg(hir_type, value, index + 1 == value_count, descriptor)?;
            if index + 1 != value_count {
                let separator = self
                    .builder
                    .build_global_string_ptr(" ", "console_separator")
                    .map_err(|error| error.to_string())?;
                self.compile_console_text(
                    separator.as_pointer_value(),
                    false,
                    "console_separator",
                    descriptor,
                )?;
            }
        }

        self.flush_console()
    }

    fn compile_console_format(
        &mut self,
        values: Vec<(Option<HirType>, BasicValueEnum<'ctx>)>,
        descriptor: u64,
    ) -> Result<(), String> {
        let call = |this: &mut Self, name: &str, args: &[inkwell::values::BasicMetadataValueEnum<'ctx>]| {
            this.builder.build_call(this.module.get_function(name).unwrap(), args, name)
                .map_err(|error| error.to_string())
        };
        call(self, "thaw_json_typed_decode_scope_begin", &[])?;
        let failed = self.context.append_basic_block(self.current_function(), "console_format_failed");
        self.push_catch_target(failed);
        let omit_before = std::mem::replace(&mut self.console_omit_absent_fields, true);
        let array = call(self, "thaw_json_array_new", &[])?.try_as_basic_value().basic().unwrap();
        let mut pushed = Ok(());
        for (ty, value) in values {
            pushed = self.compile_json_array_push_native_with_undefined(array, value, &ty.unwrap(), true);
            if pushed.is_err() { break; }
        }
        self.console_omit_absent_fields = omit_before;
        self.pop_catch_target();
        pushed?;
        call(self, "thaw_json_typed_decode_scope_end", &[self.context.i8_type().const_zero().into()])?;
        let text = call(self, "thaw_console_format", &[array.into()])?
            .try_as_basic_value().basic().unwrap().into_pointer_value();
        self.compile_console_text(text, true, "console_format", descriptor)?;
        call(self, "thaw_json_destroy", &[array.into()])?;
        let done = self.builder.get_insert_block().ok_or("console log lost its success block")?;
        self.builder.position_at_end(failed);
        self.compile_discard_typed_decode_scope()?;
        self.branch_on_pending_exception()?;
        self.builder.build_unreachable().map_err(|error| error.to_string())?;
        self.builder.position_at_end(done);
        self.flush_console()
    }

    fn flush_console(&mut self) -> Result<(), String> {
        let fflush_fn = self.module.get_function("fflush").unwrap();
        let null_ptr = self.context.ptr_type(AddressSpace::default()).const_null();
        self.builder
            .build_call(fflush_fn, &[null_ptr.into()], "fflush_call")
            .map_err(|error| error.to_string())?;
        Ok(())
    }

    fn compile_console_arg(
        &mut self,
        hir_type: Option<HirType>,
        value: BasicValueEnum<'ctx>,
        newline: bool,
        descriptor: u64,
    ) -> Result<(), String> {
        if let Some(HirType::Optional(payload)) = hir_type {
            self.compile_console_tagged(value.into_struct_value(), &payload, "undefined", newline, descriptor)?;
        } else if let Some(HirType::Nullable(payload)) = hir_type {
            self.compile_console_tagged(value.into_struct_value(), &payload, "null", newline, descriptor)?;
        } else if let Some(HirType::Nullish(payload)) = hir_type {
            self.compile_console_nullish(value.into_struct_value(), &payload, newline, descriptor)?;
        } else if let Some(HirType::Union(elements)) = hir_type {
            self.compile_console_union(value.into_struct_value(), &elements, newline, descriptor)?;
        } else if let Some(
            ty @ (HirType::Json
            | HirType::Dictionary(_)
            | HirType::Array(_)
            | HirType::Bytes
            | HirType::Tuple(_)
            | HirType::Object(_)),
        ) = hir_type
        {
            self.compile_console_structured(value.into_pointer_value(), &ty, newline, descriptor)?;
        } else if let Some(ty @ (HirType::Map(_, _) | HirType::Set(_))) = hir_type {
            self.compile_console_structured(value.into_pointer_value(), &ty, newline, descriptor)?;
        } else if matches!(hir_type, Some(HirType::WeakMap(_, _))) {
            self.compile_console_literal(
                "WeakMap { <items unknown> }", newline, "console_weak_map", descriptor,
            )?;
        } else if matches!(hir_type, Some(HirType::WeakSet(_))) {
            self.compile_console_literal(
                "WeakSet { <items unknown> }", newline, "console_weak_set", descriptor,
            )?;
        } else if matches!(
            hir_type,
            Some(HirType::Function(_, _) | HirType::CallableFunction(..))
        ) {
            self.compile_console_literal("[Function]", newline, "console_function", descriptor)?;
        } else if matches!(hir_type, Some(HirType::Promise(_))) {
            self.compile_console_literal(
                "Promise { <pending> }",
                newline,
                "console_promise",
                descriptor,
            )?;
        } else if hir_type == Some(HirType::JsValue) {
            self.compile_console_js_value(value.into_int_value(), newline, descriptor)?;
        } else if hir_type == Some(HirType::I64) {
            // A real `i64`, not the `i1` the fallback `IntValue` arm below
            // assumes -- reachable now that `Buffer.prototype.
            // readBigInt64LE`/`readBigUInt64LE` &c. hand back a genuine
            // `HirType::I64` value that a caller may log directly.
            let rendered = self
                .builder
                .build_call(
                    self.module.get_function("thaw_i64_to_bigint_string").unwrap(),
                    &[value.into()],
                    "console_i64",
                )
                .map_err(|error| error.to_string())?
                .try_as_basic_value()
                .basic()
                .ok_or("thaw_i64_to_string returned no value")?;
            self.compile_console_text(
                rendered.into_pointer_value(),
                newline,
                "console_i64",
                descriptor,
            )?;
        } else if hir_type == Some(HirType::Symbol) {
            let rendered = self
                .builder
                .build_call(
                    self.module.get_function("thaw_symbol_to_string").unwrap(),
                    &[value.into()],
                    "console_symbol",
                )
                .map_err(|error| error.to_string())?
                .try_as_basic_value()
                .basic()
                .ok_or("symbol conversion returned no value")?;
            self.compile_console_text(
                rendered.into_pointer_value(),
                newline,
                "console_symbol",
                descriptor,
            )?;
        } else if hir_type == Some(HirType::Undefined) {
            let undefined = self
                .builder
                .build_global_string_ptr("undefined", "undefined_value")
                .map_err(|error| error.to_string())?;
            self.compile_console_text(undefined.as_pointer_value(), newline, "undefined_value", descriptor)?;
        } else if hir_type == Some(HirType::Null) {
            let null = self
                .builder
                .build_global_string_ptr("null", "null_value")
                .map_err(|error| error.to_string())?;
            self.compile_console_text(null.as_pointer_value(), newline, "null_value", descriptor)?;
        } else {
            match value {
                BasicValueEnum::PointerValue(ptr) => {
                    // Every other explicitly-typed case above has already
                    // taken its own branch, so hir_type is either untracked
                    // (as it is for a `catch` binding, which codegen only
                    // ever gives a raw LLVM slot, not an entry in
                    // `variable_hir_types`) or genuinely `HirType::Str` --
                    // both were already printed as a plain C string before
                    // this call existed. A string thrown via
                    // `new Error(...)`/`new TypeError(...)`/etc. carries its
                    // class name ahead of the message behind a marker byte
                    // (see `thaw_hir::lower::expressions::lowering`); this
                    // strips that back off before printing, so
                    // `console.log(e)` shows just the message, exactly as
                    // it did before that tagging existed. An ordinary
                    // string without the marker passes through unchanged.
                    let message = self
                        .builder
                        .build_call(
                            self.module.get_function("thaw_error_message").unwrap(),
                            &[ptr.into()],
                            "console_error_message",
                        )
                        .map_err(|error| error.to_string())?
                        .try_as_basic_value()
                        .basic()
                        .ok_or("thaw_error_message returned no value")?;
                    self.compile_console_text(
                        message.into_pointer_value(),
                        newline,
                        "console_error_message",
                        descriptor,
                    )?;
                }
                BasicValueEnum::FloatValue(f) => {
                    self.compile_console_number(f, newline, "console_number", descriptor)?;
                }
                // `HirType::I64` already took its own branch above, so the
                // only `IntValue` left untyped/reaching here is `i1`
                // (`HirType::Bool`).
                BasicValueEnum::IntValue(b) => {
                    let true_str = self
                        .builder
                        .build_global_string_ptr("true", "true_str")
                        .map_err(|e| e.to_string())?;
                    let false_str = self
                        .builder
                        .build_global_string_ptr("false", "false_str")
                        .map_err(|e| e.to_string())?;
                    let selected = self
                        .builder
                        .build_select(
                            b,
                            true_str.as_pointer_value(),
                            false_str.as_pointer_value(),
                            "bool_str",
                        )
                        .map_err(|e| e.to_string())?;
                    self.compile_console_text(
                        selected.into_pointer_value(),
                        newline,
                        "console_bool",
                        descriptor,
                    )?;
                }
                other => {
                    return Err(format!(
                        "console.log does not support values of this kind yet: {other:?}"
                    ))
                }
            }
        }

        Ok(())
    }

    fn compile_console_text(
        &mut self,
        value: PointerValue<'ctx>,
        newline: bool,
        name: &str,
        descriptor: u64,
    ) -> Result<(), String> {
        // Render WTF-8 lossily: a lone surrogate prints as one U+FFFD (Node's
        // own terminal output) instead of leaking its 3 raw bytes.
        let value = self
            .builder
            .build_call(
                self.module.get_function("thaw_string_to_display").unwrap(),
                &[value.into()],
                "console_display",
            )
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .ok_or("thaw_string_to_display returned no value")?
            .into_pointer_value();
        self.builder
            .build_call(self.module.get_function("thaw_console_write").unwrap(), &[
                value.into(), self.context.i32_type().const_int(descriptor, false).into(),
                self.context.bool_type().const_int(newline as u64, false).into(),
            ], name)
            .map_err(|error| error.to_string())?;
        Ok(())
    }

    fn compile_console_number(
        &mut self,
        value: FloatValue<'ctx>,
        newline: bool,
        name: &str,
        descriptor: u64,
    ) -> Result<(), String> {
        // `%g` truncates to C's default six significant digits and picks
        // fixed/exponential notation by C's own rules, not JavaScript's --
        // it silently mis-prints e.g. `Number.MAX_SAFE_INTEGER` as
        // `9.0072e+15` and hides `0.1 + 0.2`'s imprecision by rounding it
        // to `0.3`. `thaw_number_to_string` already implements the correct
        // shortest-round-trip conversion (the same one `String(number)`,
        // template literals and `+` concatenation already use), so printing
        // through it as a string keeps `console.log` consistent with them.
        let text = self
            .builder
            .build_call(
                self.module
                    .get_function("thaw_number_to_console_string")
                    .unwrap(),
                &[value.into()],
                &format!("{name}_text"),
            )
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .ok_or("thaw_number_to_string returned no value")?
            .into_pointer_value();
        self.compile_console_text(text, newline, name, descriptor)
    }

    fn compile_console_structured(
        &mut self,
        value: PointerValue<'ctx>,
        ty: &HirType,
        newline: bool,
        descriptor: u64,
    ) -> Result<(), String> {
        let generated = !matches!(ty, HirType::Json | HirType::Dictionary(_));
        let rollback = if generated {
            self.builder.build_call(
                self.module.get_function("thaw_json_typed_decode_scope_begin").unwrap(),
                &[], "begin_console_json_scope",
            ).map_err(|error| error.to_string())?;
            let failed = self.context.append_basic_block(self.current_function(), "console_marshal_failed");
            self.push_catch_target(failed);
            Some(failed)
        } else { None };
        let omit_before = std::mem::replace(&mut self.console_omit_absent_fields, true);
        let json_result = match ty {
            HirType::Json | HirType::Dictionary(_) => Ok(value.into()),
            // `undefined` is kept as the napi-undefined sentinel (not folded into `null`) so the
            // inspector can print it like Node does.
            HirType::Array(element) => self.compile_native_array_to_json_with_undefined(value, element, true),
            HirType::Bytes => self.compile_native_array_to_json_with_undefined(value, &HirType::F64, true),
            HirType::Tuple(elements) => self.compile_native_tuple_to_json_with_undefined(value, elements, true),
            HirType::Object(_) => self.compile_native_object_to_json_with_undefined(value, ty, true),
            HirType::Map(..) | HirType::Set(_) => self.compile_native_collection_to_json(value, ty, true),
            _ => Err(format!("console.log cannot serialize {ty:?}")),
        };
        self.console_omit_absent_fields = omit_before;
        if generated {
            self.pop_catch_target();
        }
        let json = json_result?;
        if generated {
            self.builder.build_call(
                self.module.get_function("thaw_json_typed_decode_scope_end").unwrap(),
                &[self.context.i8_type().const_zero().into()], "finish_console_json_scope",
            ).map_err(|error| error.to_string())?;
        }
        // Typed values are inspected like Node's console.log (util.inspect rules), not
        // JSON-stringified: every converted shape goes through the faithful formatter.
        let text = self.builder.build_call(
            self.module.get_function(if *ty == HirType::Bytes { "thaw_json_buffer_inspect" } else { "thaw_json_console_string" }).unwrap(),
            &[json.into()], "console_json_format",
        ).map_err(|error| error.to_string())?
            .try_as_basic_value().basic()
            .ok_or("JSON console formatter returned no value")?.into_pointer_value();
        self.compile_console_text(text, newline, "console_structured", descriptor)?;
        if generated {
            self.builder.build_call(
                self.module.get_function("thaw_json_destroy").unwrap(),
                &[json.into()], "destroy_console_json_root",
            ).map_err(|error| error.to_string())?;
        }
        if let Some(failed) = rollback {
            let done = self.builder.get_insert_block().ok_or("console log lost its success block")?;
            self.builder.position_at_end(failed);
            self.compile_discard_typed_decode_scope()?;
            self.branch_on_pending_exception()?;
            self.builder.build_unreachable().map_err(|error| error.to_string())?;
            self.builder.position_at_end(done);
        }
        Ok(())
    }

    fn compile_console_collection(
        &mut self,
        value: PointerValue<'ctx>,
        ty: &HirType,
        newline: bool,
        descriptor: u64,
    ) -> Result<(), String> {
        let label = if matches!(ty, HirType::Map(_, _)) {
            "Map { size: "
        } else {
            "Set { size: "
        };
        self.compile_console_literal(label, false, "console_collection_label", descriptor)?;
        let size = self
            .builder
            .build_call(
                self.module.get_function("thaw_map_size").unwrap(),
                &[value.into()],
                "console_collection_size",
            )
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .ok_or("thaw_map_size returned no value")?
            .into_float_value();
        self.compile_console_number(size, false, "console_collection_size", descriptor)?;
        self.compile_console_literal(" }", newline, "console_collection_end", descriptor)
    }

    fn compile_console_literal(
        &mut self,
        text: &str,
        newline: bool,
        name: &str,
        descriptor: u64,
    ) -> Result<(), String> {
        let value = self
            .builder
            .build_global_string_ptr(text, name)
            .map_err(|error| error.to_string())?;
        self.compile_console_text(value.as_pointer_value(), newline, name, descriptor)
    }

    fn compile_console_js_value(
        &mut self,
        handle: IntValue<'ctx>,
        newline: bool,
        descriptor: u64,
    ) -> Result<(), String> {
        let text = self
            .builder
            .build_call(
                self.module.get_function("thaw_js_handle_to_console_string").unwrap(),
                &[handle.into()],
                "console_js_value",
            )
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .ok_or("thaw_js_handle_to_console_string returned no value")?
            .into_pointer_value();
        let probe = self
            .builder
            .build_call(
                self.module.get_function("thaw_js_handle_typed_array_probe").unwrap(),
                &[handle.into()],
                "console_js_typed_probe",
            )
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .ok_or("thaw_js_handle_typed_array_probe returned no value")?;
        let text = self
            .builder
            .build_call(
                self.module.get_function("thaw_console_typed_or").unwrap(),
                &[probe.into(), text.into()],
                "console_js_typed_text",
            )
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .ok_or("thaw_console_typed_or returned no value")?
            .into_pointer_value();
        self.compile_console_text(text, newline, "console_js_value", descriptor)
    }

    fn compile_console_union(
        &mut self,
        value: StructValue<'ctx>,
        elements: &[HirType],
        newline: bool,
        descriptor: u64,
    ) -> Result<(), String> {
        if elements.is_empty() {
            return Err("console.log cannot print an empty union".into());
        }
        let tag = self
            .builder
            .build_extract_value(value, 0, "console_union_tag")
            .map_err(|error| error.to_string())?
            .into_int_value();
        let payload = self
            .builder
            .build_extract_value(value, 1, "console_union_payload")
            .map_err(|error| error.to_string())?
            .into_int_value();
        let function = self.current_function();
        let merge = self.context.append_basic_block(function, "union_printed");
        for (index, member) in elements.iter().enumerate() {
            let matched = self
                .context
                .append_basic_block(function, "union_print_member");
            if index + 1 == elements.len() {
                self.builder
                    .build_unconditional_branch(matched)
                    .map_err(|error| error.to_string())?;
            } else {
                let next = self
                    .context
                    .append_basic_block(function, "union_print_next");
                let is_match = self
                    .builder
                    .build_int_compare(
                        IntPredicate::EQ,
                        tag,
                        self.context.i8_type().const_int(index as u64, false),
                        "union_print_tag_match",
                    )
                    .map_err(|error| error.to_string())?;
                self.builder
                    .build_conditional_branch(is_match, matched, next)
                    .map_err(|error| error.to_string())?;
                self.builder.position_at_end(next);
            }
            self.builder.position_at_end(matched);
            let member_value = self.unpack_union_payload(payload, member)?;
            self.compile_console_union_member(member_value, member, newline, descriptor)?;
            self.builder
                .build_unconditional_branch(merge)
                .map_err(|error| error.to_string())?;
            if index + 1 != elements.len() {
                let next = matched
                    .get_next_basic_block()
                    .ok_or("union console dispatch lost its next comparison block")?;
                self.builder.position_at_end(next);
            }
        }
        self.builder.position_at_end(merge);
        Ok(())
    }

    fn compile_console_union_member(
        &mut self,
        value: BasicValueEnum<'ctx>,
        member: &HirType,
        newline: bool,
        descriptor: u64,
    ) -> Result<(), String> {
        match member {
            HirType::F64 => {
                self.compile_console_number(
                    value.into_float_value(),
                    newline,
                    "console_union_number",
                    descriptor,
                )?;
            }
            HirType::Bool => {
                let yes = self
                    .builder
                    .build_global_string_ptr("true", "union_true")
                    .map_err(|error| error.to_string())?;
                let no = self
                    .builder
                    .build_global_string_ptr("false", "union_false")
                    .map_err(|error| error.to_string())?;
                let selected = self
                    .builder
                    .build_select(
                        value.into_int_value(),
                        yes.as_pointer_value(),
                        no.as_pointer_value(),
                        "union_bool_string",
                    )
                    .map_err(|error| error.to_string())?;
                self.compile_console_text(
                    selected.into_pointer_value(),
                    newline,
                    "console_union_bool",
                    descriptor,
                )?;
            }
            HirType::Str => {
                // A catch carrier's string member may be a tagged/framed Error text;
                // strip the framing like the bare-pointer path does (plain strings pass through).
                let message = self
                    .builder
                    .build_call(
                        self.module.get_function("thaw_error_message").unwrap(),
                        &[value.into_pointer_value().into()],
                        "console_union_error_message",
                    )
                    .map_err(|error| error.to_string())?
                    .try_as_basic_value()
                    .basic()
                    .ok_or("thaw_error_message returned no value")?;
                self.compile_console_text(
                    message.into_pointer_value(),
                    newline,
                    "console_union_string",
                    descriptor,
                )?;
            }
            // Same path as a bare I64 (`123n`); catch carriers can hold bigint members.
            HirType::I64 => self.compile_console_arg(Some(HirType::I64), value, newline, descriptor)?,
            HirType::Undefined => {
                let undefined = self
                    .builder
                    .build_global_string_ptr("undefined", "union_undefined")
                    .map_err(|error| error.to_string())?;
                self.compile_console_text(
                    undefined.as_pointer_value(),
                    newline,
                    "console_union_undefined",
                    descriptor,
                )?;
            }
            HirType::Null => {
                let null = self
                    .builder
                    .build_global_string_ptr("null", "union_null")
                    .map_err(|error| error.to_string())?;
                self.compile_console_text(
                    null.as_pointer_value(),
                    newline,
                    "console_union_null",
                    descriptor,
                )?;
            }
            // A caught native object (the carrier's layout-less `Object([])` member) has no
            // fields to walk; print its describer/projection as Json like Node prints it.
            HirType::Object(fields) if fields.is_empty() => {
                let ptr_ty = self.context.ptr_type(inkwell::AddressSpace::default());
                let slot = self.builder.build_alloca(ptr_ty, "console_caught_owner_slot")
                    .map_err(|error| error.to_string())?;
                self.builder.build_store(slot, value).map_err(|error| error.to_string())?;
                let name = "__thaw_console_caught_owner".to_string();
                self.variables.insert(name.clone(), (slot, ptr_ty.into()));
                self.variable_hir_types.insert(name.clone(), member.clone());
                let json = self.compile_expr(&thaw_hir::caught_native_object_console_json(
                    thaw_hir::HirExpr::Var(name),
                ))?;
                self.compile_console_structured(
                    json.into_pointer_value(), &HirType::Json, newline, descriptor,
                )?;
            }
            HirType::Object(_)
            | HirType::Json
            | HirType::Dictionary(_)
            | HirType::Array(_)
            | HirType::Tuple(_) => {
                self.compile_console_structured(
                    value.into_pointer_value(),
                    member,
                    newline,
                    descriptor,
                )?;
            }
            HirType::Map(_, _) | HirType::Set(_) => {
                self.compile_console_collection(
                    value.into_pointer_value(), member, newline, descriptor,
                )?;
            }
            HirType::WeakMap(_, _) => self.compile_console_literal(
                "WeakMap { <items unknown> }", newline, "console_union_weak_map", descriptor,
            )?,
            HirType::WeakSet(_) => self.compile_console_literal(
                "WeakSet { <items unknown> }", newline, "console_union_weak_set", descriptor,
            )?,
            HirType::Function(_, _) | HirType::CallableFunction(..) => self
                .compile_console_literal(
                    "[Function]",
                    newline,
                    "console_union_function",
                    descriptor,
                )?,
            HirType::Promise(_) => self.compile_console_literal(
                "Promise { <pending> }",
                newline,
                "console_union_promise",
                descriptor,
            )?,
            HirType::JsValue => {
                self.compile_console_js_value(value.into_int_value(), newline, descriptor)?
            }
            HirType::NativeException => {
                self.compile_console_native_exception(value.into_pointer_value(), newline, descriptor)?
            }
            other => return Err(format!("console.log cannot print union member {other:?}")),
        }
        Ok(())
    }

    /// A caught native value (Symbol, Array, Map, Promise, ...) is a private runtime descriptor.
    /// Print a fixed label chosen by its category tag; never read the contents or run user code.
    // ponytail: no contents (Array/Map items) until the snapshot-at-throw design is approved.
    fn compile_console_native_exception(
        &mut self,
        descriptor_ptr: PointerValue<'ctx>,
        newline: bool,
        descriptor: u64,
    ) -> Result<(), String> {
        const LABELS: [(u64, &str); 10] = [
            (20, "Symbol()"),
            (21, "[ <items unknown> ]"),
            (22, "<Buffer>"),
            (23, "Map { <items unknown> }"),
            (24, "WeakMap { <items unknown> }"),
            (25, "Set { <items unknown> }"),
            (26, "WeakSet { <items unknown> }"),
            (27, "[ <items unknown> ]"),
            (28, "[Function]"),
            (29, "Promise { <pending> }"),
        ];
        let tag = self
            .builder
            .build_call(
                self.module.get_function("thaw_exception_native_tag").unwrap(),
                &[descriptor_ptr.into()],
                "console_native_exception_tag",
            )
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .ok_or("native exception tag returned no value")?
            .into_int_value();
        let function = self.current_function();
        let merge = self.context.append_basic_block(function, "console_native_exception_done");
        let fallback = self.context.append_basic_block(function, "console_native_exception_other");
        let cases = LABELS
            .iter()
            .map(|(code, _)| (self.context.i64_type().const_int(*code, false),
                self.context.append_basic_block(function, "console_native_exception_case")))
            .collect::<Vec<_>>();
        self.builder
            .build_switch(tag, fallback, &cases)
            .map_err(|error| error.to_string())?;
        for ((_, block), (_, label)) in cases.iter().zip(LABELS) {
            self.builder.position_at_end(*block);
            self.compile_console_literal(label, newline, "console_native_exception_label", descriptor)?;
            self.builder.build_unconditional_branch(merge).map_err(|error| error.to_string())?;
        }
        self.builder.position_at_end(fallback);
        self.compile_console_literal(
            "[native exception]", newline, "console_native_exception_unknown", descriptor,
        )?;
        self.builder.build_unconditional_branch(merge).map_err(|error| error.to_string())?;
        self.builder.position_at_end(merge);
        Ok(())
    }

    fn compile_console_tagged(
        &mut self,
        value: StructValue<'ctx>,
        payload_type: &HirType,
        absent_text: &str,
        newline: bool,
        descriptor: u64,
    ) -> Result<(), String> {
        let present = self
            .builder
            .build_extract_value(value, 0, "console_optional_present")
            .map_err(|error| error.to_string())?
            .into_int_value();
        let payload = self
            .builder
            .build_extract_value(value, 1, "console_optional_payload")
            .map_err(|error| error.to_string())?;
        let function = self.current_function();
        let present_block = self.context.append_basic_block(function, "optional_value");
        let absent_block = self
            .context
            .append_basic_block(function, "optional_undefined");
        let merge_block = self
            .context
            .append_basic_block(function, "optional_printed");
        self.builder
            .build_conditional_branch(present, present_block, absent_block)
            .map_err(|error| error.to_string())?;

        self.builder.position_at_end(absent_block);
        let absent = self
            .builder
            .build_global_string_ptr(absent_text, "tagged_absent_string")
            .map_err(|error| error.to_string())?;
        self.compile_console_text(
            absent.as_pointer_value(),
            newline,
            "console_tagged_absent",
            descriptor,
        )?;
        self.builder
            .build_unconditional_branch(merge_block)
            .map_err(|error| error.to_string())?;

        self.builder.position_at_end(present_block);
        match payload_type {
            HirType::I64 => {
                self.compile_console_arg(Some(HirType::I64), payload, newline, descriptor)?;
            }
            HirType::F64 => {
                self.compile_console_number(
                    payload.into_float_value(),
                    newline,
                    "console_optional_number",
                    descriptor,
                )?;
            }
            HirType::Bool => {
                let true_string = self
                    .builder
                    .build_global_string_ptr("true", "optional_true")
                    .map_err(|error| error.to_string())?;
                let false_string = self
                    .builder
                    .build_global_string_ptr("false", "optional_false")
                    .map_err(|error| error.to_string())?;
                let selected = self
                    .builder
                    .build_select(
                        payload.into_int_value(),
                        true_string.as_pointer_value(),
                        false_string.as_pointer_value(),
                        "optional_bool_string",
                    )
                    .map_err(|error| error.to_string())?;
                self.compile_console_text(
                    selected.into_pointer_value(),
                    newline,
                    "console_optional_bool",
                    descriptor,
                )?;
            }
            HirType::Str => {
                self.compile_console_text(
                    payload.into_pointer_value(),
                    newline,
                    "console_optional_string",
                    descriptor,
                )?;
            }
            HirType::Optional(inner) => {
                self.compile_console_tagged(
                    payload.into_struct_value(),
                    inner,
                    "undefined",
                    newline,
                    descriptor,
                )?;
            }
            HirType::Nullable(inner) => {
                self.compile_console_tagged(
                    payload.into_struct_value(),
                    inner,
                    "null",
                    newline,
                    descriptor,
                )?;
            }
            HirType::Union(elements) => {
                self.compile_console_union(payload.into_struct_value(), elements, newline, descriptor)?;
            }
            HirType::Nullish(inner) => {
                self.compile_console_nullish(payload.into_struct_value(), inner, newline, descriptor)?;
            }
            HirType::Object(_)
            | HirType::Json
            | HirType::Dictionary(_)
            | HirType::Array(_)
            | HirType::Tuple(_) => {
                self.compile_console_structured(
                    payload.into_pointer_value(),
                    payload_type,
                    newline,
                    descriptor,
                )?;
            }
            HirType::Function(_, _) | HirType::CallableFunction(..) => self
                .compile_console_literal(
                    "[Function]",
                    newline,
                    "console_optional_function",
                    descriptor,
                )?,
            HirType::Promise(_) => self.compile_console_literal(
                "Promise { <pending> }",
                newline,
                "console_optional_promise",
                descriptor,
            )?,
            HirType::JsValue => {
                self.compile_console_js_value(payload.into_int_value(), newline, descriptor)?
            }
            other => {
                return Err(format!(
                    "console.log does not support optional payload {other:?} yet"
                ))
            }
        }
        self.builder
            .build_unconditional_branch(merge_block)
            .map_err(|error| error.to_string())?;
        self.builder.position_at_end(merge_block);
        Ok(())
    }

    fn compile_console_nullish(
        &mut self,
        value: StructValue<'ctx>,
        payload_type: &HirType,
        newline: bool,
        descriptor: u64,
    ) -> Result<(), String> {
        let tag = self
            .builder
            .build_extract_value(value, 0, "console_nullish_tag")
            .map_err(|error| error.to_string())?
            .into_int_value();
        let payload = self
            .builder
            .build_extract_value(value, 1, "console_nullish_payload")
            .map_err(|error| error.to_string())?;
        let function = self.current_function();
        let undefined_block = self
            .context
            .append_basic_block(function, "nullish_undefined");
        let value_or_null_block = self
            .context
            .append_basic_block(function, "nullish_value_or_null");
        let merge_block = self.context.append_basic_block(function, "nullish_printed");
        let is_undefined = self
            .builder
            .build_int_compare(
                IntPredicate::EQ,
                tag,
                self.context.i8_type().const_int(2, false),
                "nullish_is_undefined",
            )
            .map_err(|error| error.to_string())?;
        self.builder
            .build_conditional_branch(is_undefined, undefined_block, value_or_null_block)
            .map_err(|error| error.to_string())?;

        self.builder.position_at_end(undefined_block);
        let undefined = self
            .builder
            .build_global_string_ptr("undefined", "nullish_undefined_string")
            .map_err(|error| error.to_string())?;
        self.compile_console_text(
            undefined.as_pointer_value(),
            newline,
            "console_nullish_undefined",
            descriptor,
        )?;
        self.builder
            .build_unconditional_branch(merge_block)
            .map_err(|error| error.to_string())?;

        self.builder.position_at_end(value_or_null_block);
        let present = self
            .builder
            .build_int_compare(
                IntPredicate::EQ,
                tag,
                self.context.i8_type().const_zero(),
                "nullish_has_value",
            )
            .map_err(|error| error.to_string())?;
        let tagged_type = self
            .basic_type(&HirType::Nullable(Box::new(payload_type.clone())))?
            .into_struct_type();
        let tagged = self
            .builder
            .build_insert_value(tagged_type.get_undef(), present, 0, "nullish_console_tag")
            .map_err(|error| error.to_string())?
            .into_struct_value();
        let tagged = self
            .builder
            .build_insert_value(tagged, payload, 1, "nullish_console_payload")
            .map_err(|error| error.to_string())?
            .into_struct_value();
        self.compile_console_tagged(tagged, payload_type, "null", newline, descriptor)?;
        self.builder
            .build_unconditional_branch(merge_block)
            .map_err(|error| error.to_string())?;
        self.builder.position_at_end(merge_block);
        Ok(())
    }

}
