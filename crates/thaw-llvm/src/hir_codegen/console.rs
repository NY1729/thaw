impl<'ctx> HirCompiler<'ctx> {
    fn compile_console_log(&mut self, args: &[HirExpr]) -> Result<BasicValueEnum<'ctx>, String> {
        if args.is_empty() {
            let empty = self
                .builder
                .build_global_string_ptr("", "console_empty")
                .map_err(|error| error.to_string())?;
            self.compile_console_text(empty.as_pointer_value(), true, "console_empty")?;
        }
        let values = args
            .iter()
            .map(|arg| Ok((self.expr_hir_type(arg), self.compile_expr(arg)?)))
            .collect::<Result<Vec<_>, String>>()?;
        for (index, (hir_type, value)) in values.into_iter().enumerate() {
            self.compile_console_arg(hir_type, value, index + 1 == args.len())?;
            if index + 1 != args.len() {
                let separator = self
                    .builder
                    .build_global_string_ptr(" ", "console_separator")
                    .map_err(|error| error.to_string())?;
                self.compile_console_text(
                    separator.as_pointer_value(),
                    false,
                    "console_separator",
                )?;
            }
        }

        let fflush_fn = self.module.get_function("fflush").unwrap();
        let null_ptr = self.context.ptr_type(AddressSpace::default()).const_null();
        self.builder
            .build_call(fflush_fn, &[null_ptr.into()], "fflush_call")
            .map_err(|error| error.to_string())?;
        Ok(self.context.i32_type().const_int(0, false).into())
    }

    fn compile_console_arg(
        &mut self,
        hir_type: Option<HirType>,
        value: BasicValueEnum<'ctx>,
        newline: bool,
    ) -> Result<(), String> {
        if let Some(HirType::Optional(payload)) = hir_type {
            self.compile_console_tagged(value.into_struct_value(), &payload, "undefined", newline)?;
        } else if let Some(HirType::Nullable(payload)) = hir_type {
            self.compile_console_tagged(value.into_struct_value(), &payload, "null", newline)?;
        } else if let Some(HirType::Nullish(payload)) = hir_type {
            self.compile_console_nullish(value.into_struct_value(), &payload, newline)?;
        } else if let Some(HirType::Union(elements)) = hir_type {
            self.compile_console_union(value.into_struct_value(), &elements, newline)?;
        } else if let Some(
            ty @ (HirType::Json
            | HirType::Dictionary(_)
            | HirType::Array(_)
            | HirType::Object(_)),
        ) = hir_type
        {
            self.compile_console_structured(value.into_pointer_value(), &ty, newline)?;
        } else if hir_type == Some(HirType::Undefined) {
            let undefined = self
                .builder
                .build_global_string_ptr("undefined", "undefined_value")
                .map_err(|error| error.to_string())?;
            self.compile_console_text(undefined.as_pointer_value(), newline, "undefined_value")?;
        } else if hir_type == Some(HirType::Null) {
            let null = self
                .builder
                .build_global_string_ptr("null", "null_value")
                .map_err(|error| error.to_string())?;
            self.compile_console_text(null.as_pointer_value(), newline, "null_value")?;
        } else {
            match value {
                BasicValueEnum::PointerValue(ptr) => {
                    self.compile_console_text(ptr, newline, "console_pointer")?;
                }
                BasicValueEnum::FloatValue(f) => {
                    self.compile_console_number(f, newline, "console_number")?;
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
    ) -> Result<(), String> {
        let format = self
            .builder
            .build_global_string_ptr(if newline { "%s\n" } else { "%s" }, &format!("{name}_fmt"))
            .map_err(|error| error.to_string())?;
        self.builder
            .build_call(
                self.module.get_function("printf").unwrap(),
                &[format.as_pointer_value().into(), value.into()],
                name,
            )
            .map_err(|error| error.to_string())?;
        Ok(())
    }

    fn compile_console_number(
        &mut self,
        value: FloatValue<'ctx>,
        newline: bool,
        name: &str,
    ) -> Result<(), String> {
        let format = self
            .builder
            .build_global_string_ptr(if newline { "%g\n" } else { "%g" }, &format!("{name}_fmt"))
            .map_err(|error| error.to_string())?;
        self.builder
            .build_call(
                self.module.get_function("printf").unwrap(),
                &[format.as_pointer_value().into(), value.into()],
                name,
            )
            .map_err(|error| error.to_string())?;
        Ok(())
    }

    fn compile_console_structured(
        &mut self,
        value: PointerValue<'ctx>,
        ty: &HirType,
        newline: bool,
    ) -> Result<(), String> {
        let json = match ty {
            HirType::Json | HirType::Dictionary(_) => value.into(),
            HirType::Array(element) => self.compile_native_array_to_json(value, element)?,
            HirType::Object(_) => self.compile_native_object_to_json(value, ty)?,
            _ => return Err(format!("console.log cannot serialize {ty:?}")),
        };
        let text = self
            .builder
            .build_call(
                self.module.get_function("thaw_json_stringify").unwrap(),
                &[json.into()],
                "console_json_stringify",
            )
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .ok_or("thaw_json_stringify returned no value")?
            .into_pointer_value();
        self.compile_console_text(text, newline, "console_structured")
    }

    fn compile_console_union(
        &mut self,
        value: StructValue<'ctx>,
        elements: &[HirType],
        newline: bool,
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
            self.compile_console_union_member(member_value, member, newline)?;
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
    ) -> Result<(), String> {
        match member {
            HirType::F64 => {
                self.compile_console_number(
                    value.into_float_value(),
                    newline,
                    "console_union_number",
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
                )?;
            }
            HirType::Str => {
                self.compile_console_text(
                    value.into_pointer_value(),
                    newline,
                    "console_union_string",
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
                )?;
            }
            HirType::Object(_)
            | HirType::Json
            | HirType::Dictionary(_)
            | HirType::Array(_) => {
                self.compile_console_structured(value.into_pointer_value(), member, newline)?;
            }
            HirType::Function(_, _) => {
                let object = self
                    .builder
                    .build_global_string_ptr("[object Object]", "union_object")
                    .map_err(|error| error.to_string())?;
                self.compile_console_text(
                    object.as_pointer_value(),
                    newline,
                    "console_union_object",
                )?;
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
        self.compile_console_text(absent.as_pointer_value(), newline, "console_tagged_absent")?;
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
                )?;
            }
            HirType::Str => {
                self.compile_console_text(
                    payload.into_pointer_value(),
                    newline,
                    "console_optional_string",
                )?;
            }
            HirType::Optional(inner) => {
                self.compile_console_tagged(
                    payload.into_struct_value(),
                    inner,
                    "undefined",
                    newline,
                )?;
            }
            HirType::Nullable(inner) => {
                self.compile_console_tagged(payload.into_struct_value(), inner, "null", newline)?;
            }
            HirType::Object(_)
            | HirType::Json
            | HirType::Dictionary(_)
            | HirType::Array(_) => {
                self.compile_console_structured(
                    payload.into_pointer_value(),
                    payload_type,
                    newline,
                )?;
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
        self.compile_console_tagged(tagged, payload_type, "null", newline)?;
        self.builder
            .build_unconditional_branch(merge_block)
            .map_err(|error| error.to_string())?;
        self.builder.position_at_end(merge_block);
        Ok(())
    }

}
