impl<'ctx> HirCompiler<'ctx> {
    fn compile_optional(
        &mut self,
        value: &HirExpr,
        payload: &HirType,
        present: bool,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let value = self.compile_expr(value)?;
        self.build_optional_value(value, payload, present)
    }

    fn build_union_value(
        &mut self,
        value: BasicValueEnum<'ctx>,
        index: usize,
        elements: &[HirType],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let member = elements
            .get(index)
            .ok_or_else(|| format!("union member index {index} is out of bounds"))?;
        let payload = match (member, value) {
            (HirType::F64, BasicValueEnum::FloatValue(value)) => self
                .builder
                .build_bit_cast(value, self.context.i64_type(), "union_float_bits")
                .map_err(|error| error.to_string())?
                .into_int_value(),
            (
                HirType::Bool | HirType::Undefined | HirType::Null,
                BasicValueEnum::IntValue(value),
            ) => self
                .builder
                .build_int_z_extend(value, self.context.i64_type(), "union_bool_bits")
                .map_err(|error| error.to_string())?,
            (HirType::I64 | HirType::JsValue, BasicValueEnum::IntValue(value)) => value,
            (_, BasicValueEnum::PointerValue(value)) => self
                .builder
                .build_ptr_to_int(value, self.context.i64_type(), "union_pointer_bits")
                .map_err(|error| error.to_string())?,
            _ => return Err(format!("cannot pack {member:?} into a union payload")),
        };
        let union_type = self
            .basic_type(&HirType::Union(elements.to_vec()))?
            .into_struct_type();
        let tagged = self
            .builder
            .build_insert_value(
                union_type.get_undef(),
                self.context.i8_type().const_int(index as u64, false),
                0,
                "union_with_tag",
            )
            .map_err(|error| error.to_string())?
            .into_struct_value();
        self.builder
            .build_insert_value(tagged, payload, 1, "union_with_payload")
            .map(|value| value.into_struct_value().into())
            .map_err(|error| error.to_string())
    }

    /// Packs an arbitrary compiled value into a 64-bit word, the same
    /// physical representation `Map`/`Set` values use -- an `f64`'s bits,
    /// a zero-extended `i1`/`i8` boolean, an `i64`/`JsValue` passed
    /// through, or a pointer's integer value. Unlike `build_union_value`,
    /// this needs no static `HirType` to disambiguate, since the LLVM
    /// value's own variant/width already says which case applies; the
    /// receiving native function decodes the word back with a type it
    /// already knows statically (chosen at HIR lowering time), so there's
    /// no tag to attach here.
    fn encode_word(&mut self, value: BasicValueEnum<'ctx>) -> Result<IntValue<'ctx>, String> {
        match value {
            BasicValueEnum::FloatValue(value) => self
                .builder
                .build_bit_cast(value, self.context.i64_type(), "word_float_bits")
                .map(|value| value.into_int_value())
                .map_err(|error| error.to_string()),
            BasicValueEnum::IntValue(value) if value.get_type().get_bit_width() < 64 => self
                .builder
                .build_int_z_extend(value, self.context.i64_type(), "word_int_bits")
                .map_err(|error| error.to_string()),
            BasicValueEnum::IntValue(value) => Ok(value),
            BasicValueEnum::PointerValue(value) => self
                .builder
                .build_ptr_to_int(value, self.context.i64_type(), "word_pointer_bits")
                .map_err(|error| error.to_string()),
            other => Err(format!("cannot pack {other:?} into a native word")),
        }
    }

    fn unpack_union_payload(
        &mut self,
        payload: IntValue<'ctx>,
        member: &HirType,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        match member {
            HirType::F64 => self
                .builder
                .build_bit_cast(payload, self.context.f64_type(), "union_float")
                .map_err(|error| error.to_string()),
            HirType::Bool | HirType::Undefined | HirType::Null => self
                .builder
                .build_int_truncate(payload, self.context.bool_type(), "union_bool")
                .map(Into::into)
                .map_err(|error| error.to_string()),
            HirType::I64 | HirType::JsValue => Ok(payload.into()),
            _ => self
                .builder
                .build_int_to_ptr(
                    payload,
                    self.context.ptr_type(AddressSpace::default()),
                    "union_pointer",
                )
                .map(Into::into)
                .map_err(|error| error.to_string()),
        }
    }

    fn compile_union_member_equality(
        &mut self,
        union: &HirExpr,
        member: &HirExpr,
        index: usize,
        elements: &[HirType],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let member_type = elements
            .get(index)
            .ok_or_else(|| format!("union member index {index} is out of bounds"))?;
        let union = self.compile_expr(union)?.into_struct_value();
        let member = self.compile_expr(member)?;
        let tag = self
            .builder
            .build_extract_value(union, 0, "union_equality_tag")
            .map_err(|error| error.to_string())?
            .into_int_value();
        let tag_matches = self
            .builder
            .build_int_compare(
                IntPredicate::EQ,
                tag,
                self.context.i8_type().const_int(index as u64, false),
                "union_equality_tag_matches",
            )
            .map_err(|error| error.to_string())?;
        let function = self.current_function();
        let matched = self
            .context
            .append_basic_block(function, "union_equality_matched");
        let mismatched = self
            .context
            .append_basic_block(function, "union_equality_mismatched");
        let merge = self
            .context
            .append_basic_block(function, "union_equality_merge");
        let result = self
            .builder
            .build_alloca(self.context.bool_type(), "union_equality_result")
            .map_err(|error| error.to_string())?;
        self.builder
            .build_conditional_branch(tag_matches, matched, mismatched)
            .map_err(|error| error.to_string())?;

        self.builder.position_at_end(mismatched);
        self.builder
            .build_store(result, self.context.bool_type().const_zero())
            .map_err(|error| error.to_string())?;
        self.builder
            .build_unconditional_branch(merge)
            .map_err(|error| error.to_string())?;

        self.builder.position_at_end(matched);
        let payload = self
            .builder
            .build_extract_value(union, 1, "union_equality_payload")
            .map_err(|error| error.to_string())?
            .into_int_value();
        let payload = self.unpack_union_payload(payload, member_type)?;
        let payload_matches = self.compile_native_strict_equality(payload, member, member_type)?;
        self.builder
            .build_store(result, payload_matches)
            .map_err(|error| error.to_string())?;
        self.builder
            .build_unconditional_branch(merge)
            .map_err(|error| error.to_string())?;

        self.builder.position_at_end(merge);
        self.builder
            .build_load(self.context.bool_type(), result, "union_member_equal")
            .map_err(|error| error.to_string())
    }

    fn compile_native_strict_equality(
        &mut self,
        left: BasicValueEnum<'ctx>,
        right: BasicValueEnum<'ctx>,
        ty: &HirType,
    ) -> Result<IntValue<'ctx>, String> {
        match (left, right) {
            (BasicValueEnum::FloatValue(left), BasicValueEnum::FloatValue(right)) => self
                .builder
                .build_float_compare(FloatPredicate::OEQ, left, right, "union_float_equal")
                .map_err(|error| error.to_string()),
            (BasicValueEnum::IntValue(left), BasicValueEnum::IntValue(right)) => self
                .builder
                .build_int_compare(IntPredicate::EQ, left, right, "union_int_equal")
                .map_err(|error| error.to_string()),
            (BasicValueEnum::PointerValue(left), BasicValueEnum::PointerValue(right)) => {
                if ty == &HirType::Str {
                    let compared = self
                        .builder
                        .build_call(
                            self.module.get_function("strcmp").unwrap(),
                            &[left.into(), right.into()],
                            "union_string_compare",
                        )
                        .map_err(|error| error.to_string())?
                        .try_as_basic_value()
                        .basic()
                        .ok_or("strcmp returned no union string comparison")?
                        .into_int_value();
                    self.builder
                        .build_int_compare(
                            IntPredicate::EQ,
                            compared,
                            self.context.i32_type().const_zero(),
                            "union_string_equal",
                        )
                        .map_err(|error| error.to_string())
                } else {
                    self.builder
                        .build_int_compare(IntPredicate::EQ, left, right, "union_pointer_equal")
                        .map_err(|error| error.to_string())
                }
            }
            _ => Err(format!("cannot compare union member with type {ty:?}")),
        }
    }

    fn compile_union_equality(
        &mut self,
        left: &HirExpr,
        right: &HirExpr,
        elements: &[HirType],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        if elements.is_empty() {
            return Err("cannot compare empty unions".into());
        }
        let left = self.compile_expr(left)?.into_struct_value();
        let right = self.compile_expr(right)?.into_struct_value();
        let left_tag = self
            .builder
            .build_extract_value(left, 0, "union_left_tag")
            .map_err(|error| error.to_string())?
            .into_int_value();
        let right_tag = self
            .builder
            .build_extract_value(right, 0, "union_right_tag")
            .map_err(|error| error.to_string())?
            .into_int_value();
        let tags_equal = self
            .builder
            .build_int_compare(IntPredicate::EQ, left_tag, right_tag, "union_tags_equal")
            .map_err(|error| error.to_string())?;
        let left_payload = self
            .builder
            .build_extract_value(left, 1, "union_left_payload")
            .map_err(|error| error.to_string())?
            .into_int_value();
        let right_payload = self
            .builder
            .build_extract_value(right, 1, "union_right_payload")
            .map_err(|error| error.to_string())?
            .into_int_value();
        let function = self.current_function();
        let dispatch = self
            .context
            .append_basic_block(function, "union_equal_dispatch");
        let mismatch = self
            .context
            .append_basic_block(function, "union_tags_differ");
        let merge = self
            .context
            .append_basic_block(function, "union_equal_merge");
        let result = self
            .builder
            .build_alloca(self.context.bool_type(), "union_equal_result")
            .map_err(|error| error.to_string())?;
        self.builder
            .build_conditional_branch(tags_equal, dispatch, mismatch)
            .map_err(|error| error.to_string())?;
        self.builder.position_at_end(mismatch);
        self.builder
            .build_store(result, self.context.bool_type().const_zero())
            .map_err(|error| error.to_string())?;
        self.builder
            .build_unconditional_branch(merge)
            .map_err(|error| error.to_string())?;

        self.builder.position_at_end(dispatch);
        for (index, member) in elements.iter().enumerate() {
            let matched = self
                .context
                .append_basic_block(function, "union_equal_member");
            let next = (index + 1 != elements.len()).then(|| {
                self.context
                    .append_basic_block(function, "union_equal_next")
            });
            if let Some(next) = next {
                let is_member = self
                    .builder
                    .build_int_compare(
                        IntPredicate::EQ,
                        left_tag,
                        self.context.i8_type().const_int(index as u64, false),
                        "union_equal_member_tag",
                    )
                    .map_err(|error| error.to_string())?;
                self.builder
                    .build_conditional_branch(is_member, matched, next)
                    .map_err(|error| error.to_string())?;
            } else {
                self.builder
                    .build_unconditional_branch(matched)
                    .map_err(|error| error.to_string())?;
            }
            self.builder.position_at_end(matched);
            let left = self.unpack_union_payload(left_payload, member)?;
            let right = self.unpack_union_payload(right_payload, member)?;
            let equal = self.compile_native_strict_equality(left, right, member)?;
            self.builder
                .build_store(result, equal)
                .map_err(|error| error.to_string())?;
            self.builder
                .build_unconditional_branch(merge)
                .map_err(|error| error.to_string())?;
            if let Some(next) = next {
                self.builder.position_at_end(next);
            }
        }
        self.builder.position_at_end(merge);
        self.builder
            .build_load(self.context.bool_type(), result, "union_equal")
            .map_err(|error| error.to_string())
    }

    fn compile_optional_none(&mut self, payload: &HirType) -> Result<BasicValueEnum<'ctx>, String> {
        let value = self.compile_zero_value(payload)?;
        self.build_optional_value(value, payload, false)
    }

    fn compile_zero_value(&mut self, ty: &HirType) -> Result<BasicValueEnum<'ctx>, String> {
        match ty {
            HirType::Array(element)
                if matches!(
                    element.as_ref(),
                    HirType::F64 | HirType::Bool | HirType::Str | HirType::JsValue
                ) =>
            {
                let i64_type = self.context.i64_type();
                let allocation = self
                    .builder
                    .build_call(
                        self.module.get_function("thaw_arena_alloc").unwrap(),
                        &[
                            i64_type.const_int(ARRAY_HEADER_BYTES, false).into(),
                            i64_type.const_int(OBJECT_FIELD_BYTES, false).into(),
                        ],
                        "zero_array_alloc",
                    )
                    .map_err(|error| error.to_string())?
                    .try_as_basic_value()
                    .basic()
                    .ok_or("thaw_arena_alloc returned no zero-array pointer")?
                    .into_pointer_value();
                self.builder
                    .build_store(allocation, i64_type.const_zero())
                    .map_err(|error| error.to_string())?;
                Ok(self.compile_array_wrap(allocation)?.into())
            }
            HirType::Object(fields) => {
                let i64_type = self.context.i64_type();
                let allocation = self
                    .builder
                    .build_call(
                        self.module.get_function("thaw_arena_alloc").unwrap(),
                        &[
                            i64_type
                                .const_int(object_storage_bytes(fields).max(1), false)
                                .into(),
                            i64_type.const_int(OBJECT_FIELD_BYTES, false).into(),
                        ],
                        "zero_object_alloc",
                    )
                    .map_err(|error| error.to_string())?
                    .try_as_basic_value()
                    .basic()
                    .ok_or("thaw_arena_alloc returned no zero-object pointer")?
                    .into_pointer_value();
                for (index, (_, field_type)) in fields.iter().enumerate() {
                    let field = self.compile_zero_value(field_type)?;
                    let pointer = unsafe {
                        self.builder
                            .build_in_bounds_gep(
                                self.context.i8_type(),
                                allocation,
                                &[i64_type.const_int(object_field_offset(fields, index), false)],
                                "zero_object_field",
                            )
                            .map_err(|error| error.to_string())?
                    };
                    self.builder
                        .build_store(pointer, field)
                        .map_err(|error| error.to_string())?;
                }
                Ok(allocation.into())
            }
            HirType::Optional(payload) | HirType::Nullable(payload) => {
                let value = self.compile_zero_value(payload)?;
                self.build_optional_value(value, payload, false)
            }
            HirType::Nullish(payload) => {
                let value = self.compile_zero_value(payload)?;
                self.build_nullish_value(value, payload, 2)
            }
            other => Ok(self.basic_type(other)?.const_zero()),
        }
    }

    fn build_optional_value(
        &mut self,
        value: BasicValueEnum<'ctx>,
        payload: &HirType,
        present: bool,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let optional_type = self
            .basic_type(&HirType::Optional(Box::new(payload.clone())))?
            .into_struct_type();
        let tagged = self
            .builder
            .build_insert_value(
                optional_type.get_undef(),
                self.context
                    .bool_type()
                    .const_int(u64::from(present), false),
                0,
                "optional_with_tag",
            )
            .map_err(|error| error.to_string())?
            .into_struct_value();
        self.builder
            .build_insert_value(tagged, value, 1, "optional_with_payload")
            .map(|value| value.into_struct_value().into())
            .map_err(|error| error.to_string())
    }

    fn compile_nullish_none(
        &mut self,
        payload: &HirType,
        tag: u64,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let value = self.compile_zero_value(payload)?;
        self.build_nullish_value(value, payload, tag)
    }

    fn build_nullish_value(
        &mut self,
        value: BasicValueEnum<'ctx>,
        payload: &HirType,
        tag: u64,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        self.build_nullish_tagged_value(
            value,
            payload,
            self.context.i8_type().const_int(tag, false),
        )
    }

    fn build_nullish_tagged_value(
        &mut self,
        value: BasicValueEnum<'ctx>,
        payload: &HirType,
        tag: IntValue<'ctx>,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let nullish_type = self
            .basic_type(&HirType::Nullish(Box::new(payload.clone())))?
            .into_struct_type();
        let tagged = self
            .builder
            .build_insert_value(nullish_type.get_undef(), tag, 0, "nullish_with_tag")
            .map_err(|error| error.to_string())?
            .into_struct_value();
        self.builder
            .build_insert_value(tagged, value, 1, "nullish_with_payload")
            .map(|value| value.into_struct_value().into())
            .map_err(|error| error.to_string())
    }

    fn compile_nullish_tag_test(
        &mut self,
        value: &HirExpr,
        expected: u64,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let nullish = self.compile_expr(value)?.into_struct_value();
        let tag = self
            .builder
            .build_extract_value(nullish, 0, "nullish_tag")
            .map_err(|error| error.to_string())?
            .into_int_value();
        self.builder
            .build_int_compare(
                IntPredicate::EQ,
                tag,
                self.context.i8_type().const_int(expected, false),
                "nullish_tag_matches",
            )
            .map(Into::into)
            .map_err(|error| error.to_string())
    }

}
