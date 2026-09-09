impl<'ctx> HirCompiler<'ctx> {
    fn compile_console_log(
        &mut self,
        args: &[HirExpr],
        stderr: bool,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let descriptor = if stderr { 2 } else { 1 };
        let values = args
            .iter()
            .map(|arg| Ok((self.expr_hir_type(arg), self.compile_expr(arg)?)))
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
            | HirType::Tuple(_)
            | HirType::Object(_)),
        ) = hir_type
        {
            self.compile_console_structured(value.into_pointer_value(), &ty, newline, descriptor)?;
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
                // Our only first-class `IntValue` is `i1` (`HirType::Bool`) --
                // nothing else reaches console.log as a raw `IntValue`.
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
        let format = self
            .builder
            .build_global_string_ptr(if newline { "%s\n" } else { "%s" }, &format!("{name}_fmt"))
            .map_err(|error| error.to_string())?;
        let (function, arguments) = if descriptor == 1 {
            (
                self.module.get_function("printf").unwrap(),
                vec![format.as_pointer_value().into(), value.into()],
            )
        } else {
            (
                self.module.get_function("dprintf").unwrap(),
                vec![
                    self.context.i32_type().const_int(descriptor, false).into(),
                    format.as_pointer_value().into(),
                    value.into(),
                ],
            )
        };
        self.builder
            .build_call(function, &arguments, name)
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
                self.module.get_function("thaw_number_to_string").unwrap(),
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
        let json = match ty {
            HirType::Json | HirType::Dictionary(_) => value.into(),
            HirType::Array(element) => self.compile_native_array_to_json(value, element)?,
            HirType::Tuple(elements) => self.compile_native_tuple_to_json(value, elements)?,
            HirType::Object(_) => self.compile_native_object_to_json(value, ty)?,
            _ => return Err(format!("console.log cannot serialize {ty:?}")),
        };
        let formatter = if matches!(ty, HirType::Json) {
            "thaw_json_console_string"
        } else {
            "thaw_json_stringify"
        };
        let text = self
            .builder
            .build_call(
                self.module.get_function(formatter).unwrap(),
                &[json.into()],
                "console_json_format",
            )
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .ok_or("JSON console formatter returned no value")?
            .into_pointer_value();
        self.compile_console_text(text, newline, "console_structured", descriptor)
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
                self.module.get_function("thaw_js_handle_to_string").unwrap(),
                &[handle.into()],
                "console_js_value",
            )
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .ok_or("thaw_js_handle_to_string returned no value")?
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
                self.compile_console_text(
                    value.into_pointer_value(),
                    newline,
                    "console_union_string",
                    descriptor,
                )?;
            }
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
            other => return Err(format!("console.log cannot print union member {other:?}")),
        }
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
