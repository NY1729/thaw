impl<'ctx> HirCompiler<'ctx> {
    fn compile_recursive_closure(
        &mut self,
        name: &str,
        ty: &HirType,
        closure: &HirExpr,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let llvm_ty = self.basic_type(ty)?;
        let cell = self.allocate_arena_cell(llvm_ty, name)?;
        self.arena_variables.insert(name.to_string());
        self.variables.insert(name.to_string(), (cell, llvm_ty));
        self.variable_hir_types.insert(name.to_string(), ty.clone());
        let value = self.compile_expr(closure)?;
        self.builder
            .build_store(cell, value)
            .map_err(|error| error.to_string())?;
        Ok(value)
    }

    fn allocate_lambda_environment(
        &mut self,
        function: FunctionValue<'ctx>,
        this_adapter: FunctionValue<'ctx>,
        captures: &[HirParam],
    ) -> Result<PointerValue<'ctx>, String> {
        for capture in captures {
            if self.arena_variables.contains(&capture.name)
                || self.global_variables.contains_key(&capture.name)
                || self.active_async_completion.is_some()
            {
                continue;
            }
            let Some((stack, ty)) = self.variables.get(&capture.name).copied() else {
                continue;
            };
            let cell = self.allocate_arena_cell(ty, &capture.name)?;
            let value = self
                .builder
                .build_load(ty, stack, "captured_stack_value")
                .map_err(|error| error.to_string())?;
            self.builder
                .build_store(cell, value)
                .map_err(|error| error.to_string())?;
            self.variables.insert(capture.name.clone(), (cell, ty));
            self.arena_variables.insert(capture.name.clone());
        }
        let i64_type = self.context.i64_type();
        let closure = self
            .builder
            .build_call(
                self.module.get_function("thaw_arena_alloc").unwrap(),
                &[
                    i64_type
                        .const_int(CLOSURE_CAPTURE_BASE + captures.len() as u64 * 8, false)
                        .into(),
                    i64_type.const_int(8, false).into(),
                ],
                "closure_alloc",
            )
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .ok_or("thaw_arena_alloc did not return a closure")?
            .into_pointer_value();
        self.builder
            .build_store(closure, function.as_global_value().as_pointer_value())
            .map_err(|error| error.to_string())?;
        let this_entry = unsafe {
            self.builder
                .build_in_bounds_gep(
                    self.context.i8_type(),
                    closure,
                    &[i64_type.const_int(CLOSURE_THIS_ENTRY_OFFSET, false)],
                    "closure_this_entry",
                )
                .map_err(|error| error.to_string())?
        };
        self.builder
            .build_store(
                this_entry,
                this_adapter.as_global_value().as_pointer_value(),
            )
            .map_err(|error| error.to_string())?;
        for (index, capture) in captures.iter().enumerate() {
            let (variable_cell, _) = self
                .variables
                .get(&capture.name)
                .copied()
                .ok_or_else(|| format!("missing captured variable `{}`", capture.name))?;
            let offset = i64_type.const_int(CLOSURE_CAPTURE_BASE + index as u64 * 8, false);
            let slot = unsafe {
                self.builder
                    .build_in_bounds_gep(self.context.i8_type(), closure, &[offset], "capture_slot")
                    .map_err(|error| error.to_string())?
            };
            self.builder
                .build_store(slot, variable_cell)
                .map_err(|error| error.to_string())?;
        }
        Ok(closure)
    }

    fn compile_async_lambda(
        &mut self,
        captures: &[HirParam],
        params: &[HirParam],
        resolved: &HirType,
        body: &HirExpr,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let parent_block = self
            .builder
            .get_insert_block()
            .ok_or("async lambda must be emitted inside a function")?;
        let name = format!("__thaw_async_lambda_{}", self.next_lambda);
        self.next_lambda += 1;
        let mut lifted_params = captures.to_vec();
        lifted_params.extend_from_slice(params);
        let lifted = HirFunction {
            name: name.clone(),
            params: lifted_params,
            ret: resolved.clone(),
            is_async: true,
            body: match body {
                HirExpr::Block(statements) => statements.clone(),
                expression => vec![HirStmt::Return(Some(expression.clone()))],
            },
        };
        self.frame_async_functions
            .insert(name.clone(), resolved.clone());
        self.declare_function(&lifted)
            .map_err(|error| format!("async lambda `{name}`: {error}"))?;

        let saved_variables = std::mem::take(&mut self.variables);
        let saved_variable_hir_types = std::mem::take(&mut self.variable_hir_types);
        let saved_catch_stack = std::mem::take(&mut self.catch_stack);
        let saved_loop_stack = std::mem::take(&mut self.loop_stack);
        let compiled = self.compile_function_body(&lifted);
        self.variables = saved_variables;
        self.variable_hir_types = saved_variable_hir_types;
        self.catch_stack = saved_catch_stack;
        self.loop_stack = saved_loop_stack;
        self.builder.position_at_end(parent_block);
        compiled.map_err(|error| format!("async lambda `{name}`: {error}"))?;

        let promise_type = HirType::Promise(Box::new(resolved.clone()));
        let param_types = params
            .iter()
            .map(|parameter| parameter.ty.clone())
            .collect::<Vec<_>>();
        let adapter_name = format!("{name}__closure");
        let adapter = self.module.add_function(
            &adapter_name,
            self.function_type(&param_types, &promise_type)?,
            Some(Linkage::Internal),
        );
        let entry = self.context.append_basic_block(adapter, "entry");
        self.builder.position_at_end(entry);
        let environment = adapter.get_first_param().unwrap().into_pointer_value();
        let i64_type = self.context.i64_type();
        let mut arguments = Vec::with_capacity(captures.len() + params.len());
        for (index, capture) in captures.iter().enumerate() {
            let offset = i64_type.const_int(CLOSURE_CAPTURE_BASE + index as u64 * 8, false);
            let capture_slot = unsafe {
                self.builder
                    .build_in_bounds_gep(
                        self.context.i8_type(),
                        environment,
                        &[offset],
                        "async_capture",
                    )
                    .map_err(|error| error.to_string())?
            };
            let cell = self
                .builder
                .build_load(
                    self.context.ptr_type(AddressSpace::default()),
                    capture_slot,
                    "async_capture_cell",
                )
                .map_err(|error| error.to_string())?
                .into_pointer_value();
            arguments.push(
                self.builder
                    .build_load(self.basic_type(&capture.ty)?, cell, "async_capture_value")
                    .map_err(|error| error.to_string())?
                    .into(),
            );
        }
        arguments.extend(
            adapter
                .get_param_iter()
                .skip(1)
                .map(BasicMetadataValueEnum::from),
        );
        let target = self.module.get_function(&name).unwrap();
        let promise = self
            .builder
            .build_call(target, &arguments, "invoke_async_lambda")
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .ok_or("async lambda did not return a promise")?;
        self.builder
            .build_return(Some(&promise))
            .map_err(|error| error.to_string())?;
        self.builder.position_at_end(parent_block);
        let this_adapter = self.compile_ignored_this_adapter(
            adapter,
            &param_types,
            &promise_type,
            &format!("{adapter_name}__thaw_this_adapter"),
        )?;
        Ok(self
            .allocate_lambda_environment(adapter, this_adapter, captures)?
            .into())
    }

    fn compile_lambda(
        &mut self,
        captures: &[HirParam],
        params: &[HirParam],
        ret: &HirType,
        body: &HirExpr,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        if let HirType::Promise(resolved) = ret {
            let frame_functions = self
                .frame_async_functions
                .keys()
                .cloned()
                .collect::<std::collections::HashSet<_>>();
            let suspends = match body {
                HirExpr::Block(statements) => statements
                    .iter()
                    .any(|statement| Self::stmt_awaits_frame_source(statement, &frame_functions)),
                expression => Self::expr_awaits_frame_source(expression, &frame_functions),
            };
            if suspends {
                return self.compile_async_lambda(captures, params, resolved, body);
            }
        }
        let parent_block = self
            .builder
            .get_insert_block()
            .ok_or("lambda must be emitted inside a function")?;
        let param_types = params
            .iter()
            .map(|param| param.ty.clone())
            .collect::<Vec<_>>();
        let function_type = self.function_type(&param_types, ret)?;
        let name = format!("__thaw_lambda_{}", self.next_lambda);
        self.next_lambda += 1;
        let function = self
            .module
            .add_function(&name, function_type, Some(Linkage::Internal));
        let this_adapter = self.compile_ignored_this_adapter(
            function,
            &param_types,
            ret,
            &format!("{name}__thaw_this_adapter"),
        )?;

        let i64_type = self.context.i64_type();
        // Closure captures retain their variable cells so mutations remain
        // visible when the function value is invoked later.
        let closure = self.allocate_lambda_environment(function, this_adapter, captures)?;

        let saved_variables = std::mem::take(&mut self.variables);
        let saved_variable_hir_types = std::mem::take(&mut self.variable_hir_types);
        let saved_catch_stack = std::mem::take(&mut self.catch_stack);
        let saved_loop_stack = std::mem::take(&mut self.loop_stack);
        let result = (|| -> Result<(), String> {
            let entry = self.context.append_basic_block(function, "entry");
            self.builder.position_at_end(entry);
            let environment = function
                .get_first_param()
                .ok_or("closure function is missing its environment")?
                .into_pointer_value();
            for (index, capture) in captures.iter().enumerate() {
                let ty = self.basic_type(&capture.ty)?;
                let offset = i64_type.const_int(CLOSURE_CAPTURE_BASE + (index as u64 * 8), false);
                let capture_slot = unsafe {
                    self.builder
                        .build_in_bounds_gep(
                            self.context.i8_type(),
                            environment,
                            &[offset],
                            "captured",
                        )
                        .map_err(|error| error.to_string())?
                };
                let variable_cell = self
                    .builder
                    .build_load(
                        self.context.ptr_type(AddressSpace::default()),
                        capture_slot,
                        "capture_cell",
                    )
                    .map_err(|error| error.to_string())?;
                self.variables.insert(
                    capture.name.clone(),
                    (variable_cell.into_pointer_value(), ty),
                );
                self.variable_hir_types
                    .insert(capture.name.clone(), capture.ty.clone());
            }
            for (value, param) in function.get_param_iter().skip(1).zip(params) {
                let ty = self.basic_type(&param.ty)?;
                let slot = self.allocate_variable_cell(ty, &param.name)?;
                self.builder
                    .build_store(slot, value)
                    .map_err(|error| error.to_string())?;
                self.variables.insert(param.name.clone(), (slot, ty));
                self.variable_hir_types
                    .insert(param.name.clone(), param.ty.clone());
            }

            match body {
                HirExpr::Block(stmts) => {
                    let terminated = self.compile_block(stmts)?;
                    if !terminated {
                        if *ret == HirType::Void {
                            self.builder
                                .build_return(None)
                                .map_err(|error| error.to_string())?;
                        } else {
                            return Err(format!(
                                "lambda `{name}` does not return a value on all paths"
                            ));
                        }
                    }
                }
                expr => {
                    if *ret == HirType::Void {
                        self.compile_expr(expr)?;
                        self.builder
                            .build_return(None)
                            .map_err(|error| error.to_string())?;
                    } else {
                        let value = self.compile_expr(expr)?;
                        self.builder
                            .build_return(Some(&value))
                            .map_err(|error| error.to_string())?;
                    }
                }
            }
            Ok(())
        })();
        self.variables = saved_variables;
        self.variable_hir_types = saved_variable_hir_types;
        self.catch_stack = saved_catch_stack;
        self.loop_stack = saved_loop_stack;
        self.builder.position_at_end(parent_block);
        result?;
        Ok(closure.into())
    }

    fn compile_function_ref(
        &mut self,
        name: &str,
        params: &[HirType],
        ret: &HirType,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let target = self
            .module
            .get_function(&Self::llvm_symbol_for(name))
            .ok_or_else(|| format!("function value `{name}` is not declared"))?;
        let adapter_name = format!("__thaw_function_ref_{}", self.next_lambda);
        self.next_lambda += 1;
        let adapter = self.module.add_function(
            &adapter_name,
            self.function_type(params, ret)?,
            Some(Linkage::Internal),
        );
        let parent = self.builder.get_insert_block().unwrap();
        let entry = self.context.append_basic_block(adapter, "entry");
        self.builder.position_at_end(entry);
        let args = adapter
            .get_param_iter()
            .skip(1)
            .map(BasicMetadataValueEnum::from)
            .collect::<Vec<_>>();
        let call = self
            .builder
            .build_call(target, &args, "invoke_function_ref")
            .map_err(|error| error.to_string())?;
        if *ret == HirType::Void {
            self.builder.build_return(None).map_err(|e| e.to_string())?;
        } else {
            let value = call
                .try_as_basic_value()
                .basic()
                .ok_or("function reference returned no value")?;
            self.builder
                .build_return(Some(&value))
                .map_err(|e| e.to_string())?;
        }
        self.builder.position_at_end(parent);
        let this_adapter = self.compile_ignored_this_adapter(
            adapter,
            params,
            ret,
            &format!("{adapter_name}__thaw_this_adapter"),
        )?;
        let closure = self
            .builder
            .build_call(
                self.module.get_function("thaw_arena_alloc").unwrap(),
                &[
                    self.context
                        .i64_type()
                        .const_int(CLOSURE_CAPTURE_BASE, false)
                        .into(),
                    self.context.i64_type().const_int(8, false).into(),
                ],
                "function_ref_closure",
            )
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .ok_or("function reference closure allocation returned no value")?
            .into_pointer_value();
        self.builder
            .build_store(closure, adapter.as_global_value().as_pointer_value())
            .map_err(|error| error.to_string())?;
        let this_entry = unsafe {
            self.builder
                .build_in_bounds_gep(
                    self.context.i8_type(),
                    closure,
                    &[self
                        .context
                        .i64_type()
                        .const_int(CLOSURE_THIS_ENTRY_OFFSET, false)],
                    "function_ref_this_entry",
                )
                .map_err(|error| error.to_string())?
        };
        self.builder
            .build_store(
                this_entry,
                this_adapter.as_global_value().as_pointer_value(),
            )
            .map_err(|error| error.to_string())?;
        Ok(closure.into())
    }

    /// Allocates `[i64 length][f64 elem0]...[f64 elemN-1]` from the arena
    /// and returns a pointer to the start of the buffer (the array value).
    fn compile_method_ref(
        &mut self,
        unbound: &str,
        explicit: &str,
        params: &[HirType],
        ret: &HirType,
        is_static: bool,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let unbound_target = self
            .module
            .get_function(&Self::llvm_symbol_for(unbound))
            .ok_or_else(|| format!("unbound method entry `{unbound}` is not declared"))?;
        let explicit_target = self
            .module
            .get_function(&Self::llvm_symbol_for(explicit))
            .ok_or_else(|| format!("explicit method entry `{explicit}` is not declared"))?;
        let ordinary_name = format!("__thaw_method_ref_{}", self.next_lambda);
        self.next_lambda += 1;
        let ordinary = self.module.add_function(
            &ordinary_name,
            self.function_type(params, ret)?,
            Some(Linkage::Internal),
        );
        let parent = self.builder.get_insert_block().unwrap();
        let entry = self.context.append_basic_block(ordinary, "entry");
        self.builder.position_at_end(entry);
        let arguments = ordinary
            .get_param_iter()
            .skip(1)
            .map(BasicMetadataValueEnum::from)
            .collect::<Vec<_>>();
        let call = self
            .builder
            .build_call(unbound_target, &arguments, "invoke_unbound_method")
            .map_err(|error| error.to_string())?;
        if *ret == HirType::Void {
            self.builder
                .build_return(None)
                .map_err(|error| error.to_string())?;
        } else {
            let value = call
                .try_as_basic_value()
                .basic()
                .ok_or("unbound method entry returned no value")?;
            self.builder
                .build_return(Some(&value))
                .map_err(|error| error.to_string())?;
        }
        let mut this_params = Vec::with_capacity(params.len() + 1);
        this_params.push(HirType::I64);
        this_params.extend_from_slice(params);
        let this_entry = self.module.add_function(
            &format!("{ordinary_name}__thaw_this_adapter"),
            self.function_type(&this_params, ret)?,
            Some(Linkage::Internal),
        );
        let entry = self.context.append_basic_block(this_entry, "entry");
        self.builder.position_at_end(entry);
        let mut arguments = Vec::with_capacity(params.len() + usize::from(!is_static));
        if !is_static {
            let receiver = this_entry.get_nth_param(1).unwrap().into_int_value();
            let receiver = self
                .builder
                .build_int_to_ptr(
                    receiver,
                    self.context.ptr_type(AddressSpace::default()),
                    "method_receiver",
                )
                .map_err(|error| error.to_string())?;
            arguments.push(BasicMetadataValueEnum::from(receiver));
        }
        arguments.extend(
            this_entry
                .get_param_iter()
                .skip(2)
                .map(BasicMetadataValueEnum::from),
        );
        let call = self
            .builder
            .build_call(explicit_target, &arguments, "invoke_method_with_this")
            .map_err(|error| error.to_string())?;
        if *ret == HirType::Void {
            self.builder
                .build_return(None)
                .map_err(|error| error.to_string())?;
        } else {
            let value = call
                .try_as_basic_value()
                .basic()
                .ok_or("explicit method entry returned no value")?;
            self.builder
                .build_return(Some(&value))
                .map_err(|error| error.to_string())?;
        }
        self.builder.position_at_end(parent);
        let closure = self
            .builder
            .build_call(
                self.module.get_function("thaw_arena_alloc").unwrap(),
                &[
                    self.context
                        .i64_type()
                        .const_int(CLOSURE_CAPTURE_BASE, false)
                        .into(),
                    self.context.i64_type().const_int(8, false).into(),
                ],
                "method_ref_closure",
            )
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .ok_or("method reference closure allocation returned no value")?
            .into_pointer_value();
        self.builder
            .build_store(closure, ordinary.as_global_value().as_pointer_value())
            .map_err(|error| error.to_string())?;
        let this_slot = unsafe {
            self.builder
                .build_in_bounds_gep(
                    self.context.i8_type(),
                    closure,
                    &[self
                        .context
                        .i64_type()
                        .const_int(CLOSURE_THIS_ENTRY_OFFSET, false)],
                    "method_ref_this_entry",
                )
                .map_err(|error| error.to_string())?
        };
        self.builder
            .build_store(this_slot, this_entry.as_global_value().as_pointer_value())
            .map_err(|error| error.to_string())?;
        Ok(closure.into())
    }
}
