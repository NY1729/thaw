impl<'ctx> HirCompiler<'ctx> {
    fn compile_this_argument_word(
        &mut self,
        expression: &HirExpr,
    ) -> Result<IntValue<'ctx>, String> {
        let value = self.compile_expr(expression)?;
        match value {
            BasicValueEnum::FloatValue(value) => self
                .builder
                .build_bit_cast(value, self.context.i64_type(), "this_number_word")
                .map(|value| value.into_int_value())
                .map_err(|error| error.to_string()),
            BasicValueEnum::IntValue(value) => {
                let width = value.get_type().get_bit_width();
                if width < 64 {
                    self.builder
                        .build_int_z_extend(value, self.context.i64_type(), "this_int_word")
                        .map_err(|error| error.to_string())
                } else if width > 64 {
                    self.builder
                        .build_int_truncate(value, self.context.i64_type(), "this_int_word")
                        .map_err(|error| error.to_string())
                } else {
                    Ok(value)
                }
            }
            BasicValueEnum::PointerValue(value) => self
                .builder
                .build_ptr_to_int(value, self.context.i64_type(), "this_pointer_word")
                .map_err(|error| error.to_string()),
            other => Err(format!(
                "explicit thisArg has unsupported native representation {other:?}"
            )),
        }
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
        let mut this_params = Vec::with_capacity(params.len() + 1);
        this_params.push(HirType::I64);
        this_params.extend_from_slice(params);
        let call = self
            .builder
            .build_indirect_call(
                self.function_type(&this_params, ret)?,
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
            .build_load(i64_type, this_slot, "bound_this")
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
            offset += object_field_storage_bytes(ty);
        }
        arguments.extend(
            code.get_param_iter()
                .skip(1)
                .map(BasicMetadataValueEnum::from),
        );
        let mut source_params = Vec::with_capacity(params.len() + 1);
        source_params.push(HirType::I64);
        source_params.extend_from_slice(params);
        let call = self
            .builder
            .build_indirect_call(
                self.function_type(&source_params, ret)?,
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
        let this_word = self.compile_this_argument_word(this_arg)?;
        let bound_values = bound
            .iter()
            .map(|value| self.compile_expr(value))
            .collect::<Result<Vec<_>, _>>()?;
        let payload_bytes = params[..bound.len()]
            .iter()
            .map(object_field_storage_bytes)
            .sum::<u64>();
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
            offset += object_field_storage_bytes(ty);
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
        let function_pointer = self
            .builder
            .build_load(
                self.context.ptr_type(AddressSpace::default()),
                closure,
                "closure_code",
            )
            .map_err(|error| error.to_string())?
            .into_pointer_value();
        let mut compiled_args = vec![BasicMetadataValueEnum::from(closure)];
        compiled_args.extend(
            args.iter()
                .map(|arg| self.compile_expr(arg).map(BasicMetadataValueEnum::from))
                .collect::<Result<Vec<_>, _>>()?,
        );
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
        Ok(value)
    }

    fn allocate_special_closure(
        &mut self,
        code: FunctionValue<'ctx>,
        promise: PointerValue<'ctx>,
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
        self.builder
            .build_store(
                this_entry,
                self.context.ptr_type(AddressSpace::default()).const_null(),
            )
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
        Ok(closure)
    }
}
