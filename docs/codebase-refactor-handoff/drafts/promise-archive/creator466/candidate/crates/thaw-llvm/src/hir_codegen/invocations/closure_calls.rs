impl<'ctx> HirCompiler<'ctx> {
    // A Function-typed value can carry JavaScript `undefined` as a null
    // closure pointer. This guards local LLVM dereferences; HIR prebindings
    // can still evaluate source operands before this check.
    fn guard_callable_closure(
        &mut self,
        closure: PointerValue<'ctx>,
        message: &str,
    ) -> Result<(), String> {
        let absent = self.builder.build_is_null(closure, "callable_is_undefined")
            .map_err(|error| error.to_string())?;
        let function = self.current_function();
        let rejected = self.context.append_basic_block(function, "undefined_callable");
        let accepted = self.context.append_basic_block(function, "present_callable");
        self.builder.build_conditional_branch(absent, rejected, accepted)
            .map_err(|error| error.to_string())?;
        self.builder.position_at_end(rejected);
        self.compile_throw_type_error(message)?;
        self.builder.position_at_end(accepted);
        Ok(())
    }

    fn compile_this_argument_word(
        &mut self,
        expression: &HirExpr,
    ) -> Result<StructValue<'ctx>, String> {
        let value = self.compile_expr(expression)?;
        let ty = self
            .expr_hir_type(expression)
            .ok_or("explicit thisArg is missing its HIR type")?;
        let (tag, payload) = self.compile_receiver_parts(value, &ty)?;
        let receiver = self
            .builder
            .build_insert_value(
                self.receiver_type().get_undef(),
                tag,
                0,
                "receiver_with_kind",
            )
            .map_err(|error| error.to_string())?
            .into_struct_value();
        self.builder
            .build_insert_value(receiver, payload, 1, "receiver_with_payload")
            .map(|value| value.into_struct_value())
            .map_err(|error| error.to_string())
    }

    // The physical layout is shared with a two-field native Union, but these
    // tags are canonical across every closure call (Union member indices are
    // local to one static union and cannot be passed through unchanged).
    fn receiver_type(&self) -> inkwell::types::StructType<'ctx> {
        self.context.struct_type(
            &[self.context.i8_type().into(), self.context.i64_type().into()],
            false,
        )
    }

    fn this_entry_function_type(
        &self,
        params: &[HirType],
        ret: &HirType,
    ) -> Result<FunctionType<'ctx>, String> {
        let mut lowered = vec![BasicMetadataTypeEnum::from(
            self.context.ptr_type(AddressSpace::default()),
        )];
        lowered.push(self.receiver_type().into());
        lowered.extend(
            params
                .iter()
                .map(|ty| self.basic_type(ty).map(BasicMetadataTypeEnum::from))
                .collect::<Result<Vec<_>, _>>()?,
        );
        if matches!(ret, HirType::Promise(_)) {
            lowered.push(self.context.ptr_type(AddressSpace::default()).into());
        }
        if *ret == HirType::Void {
            Ok(self.context.void_type().fn_type(&lowered, false))
        } else {
            Ok(self.basic_type(ret)?.fn_type(&lowered, false))
        }
    }

    fn compile_receiver_parts(
        &mut self,
        value: BasicValueEnum<'ctx>,
        ty: &HirType,
    ) -> Result<(IntValue<'ctx>, IntValue<'ctx>), String> {
        let i8_type = self.context.i8_type();
        let i64_type = self.context.i64_type();
        match ty {
            HirType::Optional(inner) | HirType::Nullable(inner) => {
                let tagged = value.into_struct_value();
                let present = self.builder.build_extract_value(tagged, 0, "receiver_present")
                    .map_err(|error| error.to_string())?.into_int_value();
                let payload = self.builder.build_extract_value(tagged, 1, "receiver_optional_value")
                    .map_err(|error| error.to_string())?;
                let (tag, word) = self.compile_receiver_parts(payload, inner)?;
                let absent = if matches!(ty, HirType::Optional(_)) { 0 } else { 1 };
                let tag = self.builder.build_select(present, tag, i8_type.const_int(absent, false), "receiver_optional_kind")
                    .map_err(|error| error.to_string())?.into_int_value();
                let word = self.builder.build_select(present, word, i64_type.const_zero(), "receiver_optional_word")
                    .map_err(|error| error.to_string())?.into_int_value();
                return Ok((tag, word));
            }
            HirType::Nullish(inner) => {
                let tagged = value.into_struct_value();
                let source_tag = self.builder.build_extract_value(tagged, 0, "receiver_nullish_tag")
                    .map_err(|error| error.to_string())?.into_int_value();
                let payload = self.builder.build_extract_value(tagged, 1, "receiver_nullish_value")
                    .map_err(|error| error.to_string())?;
                let (present_tag, present_word) = self.compile_receiver_parts(payload, inner)?;
                let is_present = self.builder.build_int_compare(IntPredicate::EQ, source_tag, i8_type.const_zero(), "receiver_nullish_present")
                    .map_err(|error| error.to_string())?;
                let is_null = self.builder.build_int_compare(IntPredicate::EQ, source_tag, i8_type.const_int(1, false), "receiver_nullish_null")
                    .map_err(|error| error.to_string())?;
                let absent_tag = self.builder.build_select(is_null, i8_type.const_int(1, false), i8_type.const_zero(), "receiver_nullish_absent")
                    .map_err(|error| error.to_string())?.into_int_value();
                let tag = self.builder.build_select(is_present, present_tag, absent_tag, "receiver_nullish_kind")
                    .map_err(|error| error.to_string())?.into_int_value();
                let word = self.builder.build_select(is_present, present_word, i64_type.const_zero(), "receiver_nullish_word")
                    .map_err(|error| error.to_string())?.into_int_value();
                return Ok((tag, word));
            }
            HirType::Union(members) => {
                let tagged = value.into_struct_value();
                let member_tag = self.builder.build_extract_value(tagged, 0, "receiver_union_member")
                    .map_err(|error| error.to_string())?.into_int_value();
                let payload = self.builder.build_extract_value(tagged, 1, "receiver_union_payload")
                    .map_err(|error| error.to_string())?.into_int_value();
                let mut result_tag = i8_type.const_zero();
                let mut result_word = i64_type.const_zero();
                for (index, member) in members.iter().enumerate() {
                    let member_value = self.unpack_union_payload(payload, member)?;
                    let (tag, word) = self.compile_receiver_parts(member_value, member)?;
                    let matches = self.builder.build_int_compare(IntPredicate::EQ, member_tag, i8_type.const_int(index as u64, false), "receiver_union_matches")
                        .map_err(|error| error.to_string())?;
                    result_tag = self.builder.build_select(matches, tag, result_tag, "receiver_union_kind")
                        .map_err(|error| error.to_string())?.into_int_value();
                    result_word = self.builder.build_select(matches, word, result_word, "receiver_union_word")
                        .map_err(|error| error.to_string())?.into_int_value();
                }
                return Ok((result_tag, result_word));
            }
            _ => {}
        }
        let kind = match ty {
            HirType::Undefined | HirType::Void => 0,
            HirType::Null => 1,
            HirType::Bool => 2,
            HirType::F64 => 3,
            HirType::I64 => 4,
            HirType::Str | HirType::StrLiteral(_) => 5,
            HirType::Symbol => 6,
            HirType::Json | HirType::Dictionary(_) => 7,
            HirType::JsValue => 8,
            HirType::Array(_) | HirType::Tuple(_) | HirType::Object(_)
            | HirType::Bytes | HirType::Map(_, _) | HirType::WeakMap(_, _)
            | HirType::Set(_) | HirType::WeakSet(_) | HirType::Function(_, _)
            | HirType::CallableFunction(..) | HirType::Promise(_) => 9,
            other => return Err(format!("unsupported explicit receiver type {other:?}")),
        };
        let word = if matches!(ty, HirType::Undefined | HirType::Null | HirType::Void) {
            i64_type.const_zero()
        } else {
            self.encode_word(value)?
        };
        Ok((i8_type.const_int(kind, false), word))
    }

    fn compile_function_call_with_this(
        &mut self,
        callee: &HirExpr,
        this_arg: &HirExpr,
        args: &[HirExpr],
        params: &[HirType],
        ret: &HirType,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let closure = self.compile_expr(callee)?.into_pointer_value();
        self.guard_callable_closure(closure, "Cannot read a function property of undefined")?;
        let this_entry_slot = unsafe {
            self.builder
                .build_in_bounds_gep(
                    self.context.i8_type(),
                    closure,
                    &[self
                        .context
                        .i64_type()
                        .const_int(CLOSURE_THIS_ENTRY_OFFSET, false)],
                    "closure_this_entry_slot",
                )
                .map_err(|error| error.to_string())?
        };
        let function_pointer = self
            .builder
            .build_load(
                self.context.ptr_type(AddressSpace::default()),
                this_entry_slot,
                "closure_this_entry",
            )
            .map_err(|error| error.to_string())?
            .into_pointer_value();
        let this_word = self.compile_this_argument_word(this_arg)?;
        let mut compiled_args = vec![
            BasicMetadataValueEnum::from(closure),
            BasicMetadataValueEnum::from(this_word),
        ];
        compiled_args.extend(
            args.iter()
                .map(|arg| self.compile_expr(arg).map(BasicMetadataValueEnum::from))
                .collect::<Result<Vec<_>, _>>()?,
        );
        let ticket_slot = if matches!(ret, HirType::Promise(_)) {
            let slot = self.builder.build_alloca(
                self.context.i64_type(), "closure_creator_ticket_out",
            ).map_err(|error| error.to_string())?;
            self.builder.build_store(slot, self.context.i64_type().const_zero())
                .map_err(|error| error.to_string())?;
            compiled_args.push(slot.into());
            Some(slot)
        } else { None };
        let call = self
            .builder
            .build_indirect_call(
                self.this_entry_function_type(params, ret)?,
                function_pointer,
                &compiled_args,
                "closure_call_with_this",
            )
            .map_err(|error| error.to_string())?;
        let value = if *ret == HirType::Void {
            self.context.f64_type().const_zero().into()
        } else {
            call.try_as_basic_value()
                .basic()
                .ok_or("this-aware function value returned no value")?
        };
        self.branch_on_pending_exception()?;
        if let Some(slot) = ticket_slot {
            let ticket = self.builder.build_load(
                self.context.i64_type(), slot, "closure_creator_ticket",
            ).map_err(|error| error.to_string())?.into_int_value();
            self.promise_creator_tickets.insert(value.into_pointer_value(), ticket);
        }
        Ok(value)
    }

    fn compile_function_bind_this(
        &mut self,
        callee: &HirExpr,
        this_arg: &HirExpr,
        bound: &[HirExpr],
        params: &[HirType],
        ret: &HirType,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        if bound.len() > params.len() {
            return Err("bound function has more leading arguments than parameters".into());
        }
        let remaining = &params[bound.len()..];
        let name = format!("__thaw_bound_function_{}", self.next_lambda);
        self.next_lambda += 1;
        let code = self.module.add_function(
            &name,
            self.function_type(remaining, ret)?,
            Some(Linkage::Internal),
        );
        let parent = self.builder.get_insert_block().unwrap();
        let entry = self.context.append_basic_block(code, "entry");
        self.builder.position_at_end(entry);
        let environment = code.get_nth_param(0).unwrap().into_pointer_value();
        let i64_type = self.context.i64_type();
        let load_slot = |compiler: &mut Self, offset: u64, label: &str| unsafe {
            compiler
                .builder
                .build_in_bounds_gep(
                    compiler.context.i8_type(),
                    environment,
                    &[i64_type.const_int(offset, false)],
                    label,
                )
                .map_err(|error| error.to_string())
        };
        let source_slot = load_slot(self, BOUND_CLOSURE_SOURCE_OFFSET, "bound_source_slot")?;
        let source = self
            .builder
            .build_load(
                self.context.ptr_type(AddressSpace::default()),
                source_slot,
                "bound_source",
            )
            .map_err(|error| error.to_string())?
            .into_pointer_value();
        let source_this_slot = unsafe {
            self.builder
                .build_in_bounds_gep(
                    self.context.i8_type(),
                    source,
                    &[i64_type.const_int(CLOSURE_THIS_ENTRY_OFFSET, false)],
                    "bound_source_this_slot",
                )
                .map_err(|error| error.to_string())?
        };
        let source_this_entry = self
            .builder
            .build_load(
                self.context.ptr_type(AddressSpace::default()),
                source_this_slot,
                "bound_source_this_entry",
            )
            .map_err(|error| error.to_string())?
            .into_pointer_value();
        let this_slot = load_slot(self, BOUND_CLOSURE_THIS_OFFSET, "bound_this_slot")?;
        let this_word = self
            .builder
            .build_load(self.receiver_type(), this_slot, "bound_this")
            .map_err(|error| error.to_string())?;
        let mut arguments = vec![
            BasicMetadataValueEnum::from(source),
            BasicMetadataValueEnum::from(this_word),
        ];
        let mut offset = BOUND_CLOSURE_ARGUMENT_BASE;
        for (index, ty) in params[..bound.len()].iter().enumerate() {
            let slot = load_slot(self, offset, &format!("bound_argument_{index}_slot"))?;
            let value = self
                .builder
                .build_load(
                    self.basic_type(ty)?,
                    slot,
                    &format!("bound_argument_{index}"),
                )
                .map_err(|error| error.to_string())?;
            arguments.push(value.into());
            offset = checked_storage_add(offset, object_field_storage_bytes(ty)?)?;
        }
        arguments.extend(
            code.get_param_iter()
                .skip(1)
                .map(BasicMetadataValueEnum::from),
        );
        let call = self
            .builder
            .build_indirect_call(
                self.this_entry_function_type(params, ret)?,
                source_this_entry,
                &arguments,
                "invoke_bound_function",
            )
            .map_err(|error| error.to_string())?;
        if *ret == HirType::Void {
            self.builder
                .build_return(None)
                .map_err(|error| error.to_string())?;
        } else {
            let value = call
                .try_as_basic_value()
                .basic()
                .ok_or("bound function returned no value")?;
            self.builder
                .build_return(Some(&value))
                .map_err(|error| error.to_string())?;
        }
        let ignored_this = self.compile_ignored_this_adapter(
            code,
            remaining,
            ret,
            &format!("{name}__thaw_this_adapter"),
        )?;
        self.builder.position_at_end(parent);
        let source = self.compile_expr(callee)?.into_pointer_value();
        self.guard_callable_closure(source, "Cannot read properties of undefined (reading 'bind')")?;
        let this_word = self.compile_this_argument_word(this_arg)?;
        let bound_values = bound
            .iter()
            .map(|value| self.compile_expr(value))
            .collect::<Result<Vec<_>, _>>()?;
        let payload_bytes = params[..bound.len()].iter().try_fold(0, |total, ty| {
            checked_storage_add(total, object_field_storage_bytes(ty)?)
        })?;
        let closure = self
            .builder
            .build_call(
                self.module.get_function("thaw_arena_alloc").unwrap(),
                &[
                    i64_type
                        .const_int(BOUND_CLOSURE_ARGUMENT_BASE + payload_bytes, false)
                        .into(),
                    i64_type.const_int(8, false).into(),
                ],
                "bound_function_closure",
            )
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .ok_or("bound closure allocation returned no value")?
            .into_pointer_value();
        self.builder
            .build_store(closure, code.as_global_value().as_pointer_value())
            .map_err(|error| error.to_string())?;
        let slot = |compiler: &mut Self, offset: u64, label: &str| unsafe {
            compiler
                .builder
                .build_in_bounds_gep(
                    compiler.context.i8_type(),
                    closure,
                    &[i64_type.const_int(offset, false)],
                    label,
                )
                .map_err(|error| error.to_string())
        };
        let this_entry_slot = slot(self, CLOSURE_THIS_ENTRY_OFFSET, "bound_this_entry_slot")?;
        self.builder
            .build_store(
                this_entry_slot,
                ignored_this.as_global_value().as_pointer_value(),
            )
            .map_err(|error| error.to_string())?;
        let source_slot = slot(self, BOUND_CLOSURE_SOURCE_OFFSET, "bound_source_store")?;
        self.builder
            .build_store(source_slot, source)
            .map_err(|error| error.to_string())?;
        let bound_this_slot = slot(self, BOUND_CLOSURE_THIS_OFFSET, "bound_this_store")?;
        self.builder
            .build_store(bound_this_slot, this_word)
            .map_err(|error| error.to_string())?;
        let mut offset = BOUND_CLOSURE_ARGUMENT_BASE;
        for (index, (value, ty)) in bound_values.into_iter().zip(params).enumerate() {
            let argument_slot = slot(self, offset, &format!("bound_argument_{index}_store"))?;
            self.builder
                .build_store(argument_slot, value)
                .map_err(|error| error.to_string())?;
            offset = checked_storage_add(offset, object_field_storage_bytes(ty)?)?;
        }
        Ok(closure.into())
    }

    fn compile_closure_call(
        &mut self,
        callee: &HirExpr,
        params: &[HirType],
        ret: &HirType,
        args: &[HirExpr],
        name: &str,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let function_type = self.function_type(params, ret)?;
        let closure = self.compile_expr(callee)?.into_pointer_value();
        let mut compiled_args = vec![BasicMetadataValueEnum::from(closure)];
        compiled_args.extend(
            args.iter()
                .map(|arg| self.compile_expr(arg).map(BasicMetadataValueEnum::from))
                .collect::<Result<Vec<_>, _>>()?,
        );
        let ticket_slot = if matches!(ret, HirType::Promise(_)) {
            let slot = self.builder.build_alloca(
                self.context.i64_type(), "closure_creator_ticket_out",
            ).map_err(|error| error.to_string())?;
            self.builder.build_store(slot, self.context.i64_type().const_zero())
                .map_err(|error| error.to_string())?;
            compiled_args.push(slot.into());
            Some(slot)
        } else { None };
        self.guard_callable_closure(closure, "Value is not a function")?;
        let function_pointer = self
            .builder
            .build_load(
                self.context.ptr_type(AddressSpace::default()),
                closure,
                "closure_code",
            )
            .map_err(|error| error.to_string())?
            .into_pointer_value();
        let call = self
            .builder
            .build_indirect_call(
                function_type,
                function_pointer,
                &compiled_args,
                "closure_call",
            )
            .map_err(|error| error.to_string())?;
        let value = if *ret == HirType::Void {
            self.context.f64_type().const_zero().into()
        } else {
            call.try_as_basic_value()
                .basic()
                .ok_or_else(|| format!("function value `{name}` does not return a value"))?
        };
        self.branch_on_pending_exception()?;
        if let Some(slot) = ticket_slot {
            let ticket = self.builder.build_load(
                self.context.i64_type(), slot, "closure_creator_ticket",
            ).map_err(|error| error.to_string())?.into_int_value();
            self.promise_creator_tickets.insert(value.into_pointer_value(), ticket);
        }
        Ok(value)
    }

    fn allocate_special_closure(
        &mut self,
        code: FunctionValue<'ctx>,
        promise: PointerValue<'ctx>,
        params: &[HirType],
        name: &str,
    ) -> Result<PointerValue<'ctx>, String> {
        let i64_type = self.context.i64_type();
        let closure = self
            .builder
            .build_call(
                self.module.get_function("thaw_arena_alloc").unwrap(),
                &[
                    i64_type.const_int(CLOSURE_CAPTURE_BASE + 8, false).into(),
                    i64_type.const_int(8, false).into(),
                ],
                name,
            )
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .ok_or("closure allocation returned no value")?
            .into_pointer_value();
        let function = self.current_function();
        let missing = self.context.append_basic_block(function, "special_closure_missing");
        let ready = self.context.append_basic_block(function, "special_closure_ready");
        let done = self.context.append_basic_block(function, "special_closure_done");
        let failed = self.builder.build_is_null(closure, "special_closure_allocation_failed")
            .map_err(|error| error.to_string())?;
        self.builder.build_conditional_branch(failed, missing, ready)
            .map_err(|error| error.to_string())?;
        self.builder.position_at_end(missing);
        self.builder.build_unconditional_branch(done).map_err(|error| error.to_string())?;
        self.builder.position_at_end(ready);
        self.builder
            .build_store(closure, code.as_global_value().as_pointer_value())
            .map_err(|error| error.to_string())?;
        let this_entry = unsafe {
            self.builder
                .build_in_bounds_gep(
                    self.context.i8_type(),
                    closure,
                    &[i64_type.const_int(CLOSURE_THIS_ENTRY_OFFSET, false)],
                    "special_closure_this_entry",
                )
                .map_err(|error| error.to_string())?
        };
        let adapter = self.compile_ignored_this_adapter(
            code,
            params,
            &HirType::Void,
            &format!("{name}__thaw_this_adapter"),
        )?;
        self.builder
            .build_store(this_entry, adapter.as_global_value().as_pointer_value())
            .map_err(|error| error.to_string())?;
        let promise_slot = unsafe {
            self.builder
                .build_in_bounds_gep(
                    self.context.i8_type(),
                    closure,
                    &[i64_type.const_int(CLOSURE_CAPTURE_BASE, false)],
                    "promise_capture",
                )
                .map_err(|error| error.to_string())?
        };
        self.builder
            .build_store(promise_slot, promise)
            .map_err(|error| error.to_string())?;
        self.builder.build_unconditional_branch(done).map_err(|error| error.to_string())?;
        self.builder.position_at_end(done);
        let selected = self.builder.build_phi(
            self.context.ptr_type(AddressSpace::default()), "special_closure_or_null",
        ).map_err(|error| error.to_string())?;
        selected.add_incoming(&[
            (&self.context.ptr_type(AddressSpace::default()).const_null(), missing),
            (&closure, ready),
        ]);
        Ok(selected.as_basic_value().into_pointer_value())
    }
}
