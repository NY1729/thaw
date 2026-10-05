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

    /// Promotes `name` (a pre-existing, currently plain-stack variable) to
    /// a shared arena cell if it isn't one already -- reused by both a
    /// closure's own reactive first-capture promotion
    /// (`allocate_lambda_environment`, below) and `compile_while`'s eager
    /// pre-pass (see `thaw_hir::closure_captured_names_in_while`'s own
    /// doc comment for why a `while` loop needs this done for BOTH sides
    /// up front, not reactively on whichever side's closure compiles
    /// first).
    fn promote_variable_to_arena_cell(
        &mut self,
        name: &str,
        hir_ty: &HirType,
    ) -> Result<(), String> {
        self.promote_variable_to_arena_cell_with_mode(name, hir_ty, false)
    }

    fn prepromote_variable_to_arena_cell(
        &mut self,
        name: &str,
        hir_ty: &HirType,
    ) -> Result<(), String> {
        self.promote_variable_to_arena_cell_with_mode(name, hir_ty, true)
    }

    fn promote_variable_to_arena_cell_with_mode(
        &mut self,
        name: &str,
        hir_ty: &HirType,
        defer_js_retain: bool,
    ) -> Result<(), String> {
        let Some((variable_cell, ty)) = self.variables.get(name).copied() else {
            return Ok(());
        };
        let frame_backed = self.async_frame_cells.contains(&variable_cell);
        if self.arena_variables.contains(name)
            || self.global_variables.contains_key(name)
            || frame_backed
        {
            return Ok(());
        }
        let promotion_scope = self
            .loop_promotion_scopes
            .iter()
            .find(|(_, variables)| variables.contains(name))
            .map(|(preheader, _)| *preheader);
        let cell = if promotion_scope.is_none() && !self.loop_promotion_scopes.is_empty() {
            self.build_arena_cell(&self.builder, ty, name)?
        } else {
            self.allocate_arena_cell(ty, name)?
        };
        let promotion_builder = promotion_scope.map(|preheader| {
            let builder = self.context.create_builder();
            builder.position_before(&preheader.get_terminator().unwrap());
            builder
        });
        let builder = promotion_builder.as_ref().unwrap_or(&self.builder);
        let value = builder
            .build_load(ty, variable_cell, "captured_stack_value")
            .map_err(|error| error.to_string())?;
        if defer_js_retain && *hir_ty == HirType::JsValue {
            // The condition may be this function's first QuickJS operation.
            // Establish the handle dependency before it is compiled, since an
            // arena-promoted cell skips reactive promotion at closure creation.
            self.uses_quickjs = true;
            self.uses_quickjs_handles = true;
        }
        let pending_claim = if self.uses_quickjs_handles && *hir_ty == HirType::JsValue {
            if defer_js_retain {
                // Match the cell's allocation placement and initial-copy site.
                // In an outer loop this is a fresh cell and claim per iteration.
                let claim = if promotion_scope.is_none() && !self.loop_promotion_scopes.is_empty() {
                    self.build_arena_cell(&self.builder, self.context.i8_type().into(),
                        &format!("{name}_capture_claim"))?
                } else {
                    self.allocate_arena_cell(self.context.i8_type().into(),
                        &format!("{name}_capture_claim"))?
                };
                builder.build_store(claim, self.context.i8_type().const_zero())
                    .map_err(|error| error.to_string())?;
                Some(claim)
            } else {
                builder
                    .build_call(
                        self.module.get_function("thaw_js_retain_handle").unwrap(),
                        &[value.into()],
                        "retain_captured_js_handle",
                    )
                    .map_err(|error| error.to_string())?;
                None
            }
        } else {
            None
        };
        builder
            .build_store(cell, value)
            .map_err(|error| error.to_string())?;
        if matches!(hir_ty, HirType::Promise(_)) {
            if let Some(source) = self.promise_creator_cell_tickets.get(&variable_cell).copied() {
                let ticket_cell = if promotion_scope.is_none() && !self.loop_promotion_scopes.is_empty() {
                    self.build_arena_cell(&self.builder, self.context.i64_type().into(),
                        &format!("{name}_creator_ticket"))?
                } else {
                    self.allocate_arena_cell(self.context.i64_type().into(),
                        &format!("{name}_creator_ticket"))?
                };
                let ticket = builder.build_load(self.context.i64_type(), source,
                    "promoted_creator_ticket").map_err(|error| error.to_string())?;
                let registered = builder.build_call(
                    self.module.get_function("thaw_promise_register_capture_creator_cell").unwrap(),
                    &[ticket_cell.into(), ticket.into()], "track_promoted_creator_ticket",
                ).map_err(|error| error.to_string())?.try_as_basic_value().basic()
                    .ok_or("capture creator registration returned no status")?.into_int_value();
                let ready = self.context.append_basic_block(self.current_function(),
                    "capture_creator_registered");
                let failed = self.context.append_basic_block(self.current_function(),
                    "capture_creator_registration_oom");
                let registered = builder.build_int_compare(IntPredicate::NE, registered,
                    self.context.i8_type().const_zero(), "capture_creator_registered_status")
                    .map_err(|error| error.to_string())?;
                let preheader_target = if let Some(preheader) = promotion_scope {
                    // This copy is emitted into the loop preheader, whose old
                    // unconditional edge must now pass through the checked
                    // registration block. Keep later promotions in that new
                    // preheader tail, not before this ownership check.
                    let terminator = preheader.get_terminator()
                        .ok_or("Promise capture preheader has no terminator")?;
                    if terminator.is_conditional().map_err(|e| e.to_string())? {
                        return Err("Promise capture preheader has a conditional terminator".into());
                    }
                    let Some(inkwell::values::Operand::Block(target)) = terminator.get_operand(0) else {
                        return Err("Promise capture preheader has no branch target".into());
                    };
                    terminator.erase_from_basic_block();
                    builder.position_at_end(preheader);
                    Some(target)
                } else { None };
                builder.build_conditional_branch(registered, ready, failed)
                    .map_err(|error| error.to_string())?;
                let current = self.builder.get_insert_block().unwrap();
                self.builder.position_at_end(failed);
                self.compile_throw_type_error("Unable to track captured Promise creator")?;
                self.builder.position_at_end(current);
                if let Some(preheader) = promotion_scope {
                    promotion_builder.as_ref().unwrap().position_at_end(ready);
                    for (scope, _) in &mut self.loop_promotion_scopes {
                        if *scope == preheader { *scope = ready; }
                    }
                } else {
                    self.builder.position_at_end(ready);
                }
                let builder = promotion_builder.as_ref().unwrap_or(&self.builder);
                builder.build_call(
                    self.module.get_function("thaw_promise_update_capture_creator_cell").unwrap(),
                    &[source.into(), self.context.i64_type().const_zero().into()],
                    "untrack_moved_creator_source",
                ).map_err(|error| error.to_string())?;
                builder.build_store(ticket_cell, ticket).map_err(|error| error.to_string())?;
                builder.build_store(source, self.context.i64_type().const_zero())
                    .map_err(|error| error.to_string())?;
                if let Some(target) = preheader_target {
                    builder.build_unconditional_branch(target).map_err(|error| error.to_string())?;
                }
                self.promise_creator_cell_tickets.insert(cell, ticket_cell);
            }
        }
        if let Some(claim) = pending_claim {
            self.pending_js_capture_claims.insert(cell, claim);
        }
        self.variables.insert(name.to_string(), (cell, ty));
        self.arena_variables.insert(name.to_string());
        Ok(())
    }

    fn allocate_lambda_environment(
        &mut self,
        function: FunctionValue<'ctx>,
        this_adapter: FunctionValue<'ctx>,
        captures: &[HirParam],
    ) -> Result<PointerValue<'ctx>, String> {
        for capture in captures {
            self.promote_variable_to_arena_cell(&capture.name, &capture.ty)?;
        }
        for capture in captures {
            let (cell, ty) = self.variables.get(&capture.name).copied()
                .ok_or_else(|| format!("missing captured variable `{}`", capture.name))?;
            let Some(claim) = self.pending_js_capture_claims.get(&cell).copied() else {
                continue;
            };
            let function = self.current_function();
            let retain = self.context.append_basic_block(function, "capture_retain");
            let ready = self.context.append_basic_block(function, "capture_ready");
            let claimed = self.builder.build_load(self.context.i8_type(), claim, "capture_claimed")
                .map_err(|error| error.to_string())?.into_int_value();
            let unclaimed = self.builder.build_int_compare(
                IntPredicate::EQ, claimed, self.context.i8_type().const_zero(),
                "capture_unclaimed",
            ).map_err(|error| error.to_string())?;
            self.builder.build_conditional_branch(unclaimed, retain, ready)
                .map_err(|error| error.to_string())?;
            self.builder.position_at_end(retain);
            let handle = self.builder.build_load(ty, cell, "capture_current_js_handle")
                .map_err(|error| error.to_string())?;
            let did_retain = self.builder.build_call(
                self.module.get_function("thaw_js_retain_handle").unwrap(),
                &[handle.into()], "claim_captured_js_handle",
            ).map_err(|error| error.to_string())?.try_as_basic_value().basic()
                .ok_or("thaw_js_retain_handle returned no status")?;
            // A failed retain leaves the claim clear so a later construction
            // can retry; closure construction itself follows the old path.
            self.builder.build_store(claim, did_retain)
                .map_err(|error| error.to_string())?;
            self.builder.build_unconditional_branch(ready)
                .map_err(|error| error.to_string())?;
            self.builder.position_at_end(ready);
        }
        let i64_type = self.context.i64_type();
        let promise_captures = captures.iter().filter(|capture|
            matches!(&capture.ty, HirType::Promise(_))).count();
        if promise_captures != 0 {
            // Captured creator ticket cells must participate in arena
            // reachability even when the module has no Promise globals.
            self.tracks_native_promise_owners = true;
        }
        let closure = self
            .builder
            .build_call(
                self.module.get_function("thaw_arena_alloc").unwrap(),
                &[
                    i64_type
                        .const_int(CLOSURE_CAPTURE_BASE
                            + (captures.len() + promise_captures) as u64 * 8, false)
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
        let mut promise_index = 0_u64;
        for capture in captures {
            if !matches!(&capture.ty, HirType::Promise(_)) { continue; }
            let (value_cell, _) = self.variables.get(&capture.name).copied()
                .ok_or_else(|| format!("missing Promise capture `{}`", capture.name))?;
            let ticket_cell = if let Some(cell) = self.promise_creator_cell_tickets.get(&value_cell).copied() {
                cell
            } else {
                let cell = self.allocate_arena_cell(self.context.i64_type().into(),
                    &format!("{}_captured_creator_ticket", capture.name))?;
                self.builder.build_store(cell, i64_type.const_zero())
                    .map_err(|error| error.to_string())?;
                let registered = self.builder.build_call(
                    self.module.get_function("thaw_promise_register_capture_creator_cell").unwrap(),
                    &[cell.into(), i64_type.const_zero().into()],
                    "track_empty_captured_creator_ticket",
                ).map_err(|error| error.to_string())?.try_as_basic_value().basic()
                    .ok_or("empty capture creator registration returned no status")?.into_int_value();
                let ready = self.context.append_basic_block(self.current_function(),
                    "empty_capture_creator_registered");
                let failed = self.context.append_basic_block(self.current_function(),
                    "empty_capture_creator_registration_oom");
                let valid = self.builder.build_int_compare(IntPredicate::NE, registered,
                    self.context.i8_type().const_zero(), "empty_capture_creator_registered_status")
                    .map_err(|error| error.to_string())?;
                self.builder.build_conditional_branch(valid, ready, failed)
                    .map_err(|error| error.to_string())?;
                self.builder.position_at_end(failed);
                self.compile_throw_type_error("Unable to track captured Promise creator")?;
                self.builder.position_at_end(ready);
                self.promise_creator_cell_tickets.insert(value_cell, cell);
                cell
            };
            let offset = i64_type.const_int(CLOSURE_CAPTURE_BASE
                + (captures.len() as u64 + promise_index) * 8, false);
            let slot = unsafe { self.builder.build_in_bounds_gep(self.context.i8_type(),
                closure, &[offset], "captured_creator_ticket_slot")
                .map_err(|error| error.to_string())? };
            self.builder.build_store(slot, ticket_cell).map_err(|error| error.to_string())?;
            promise_index += 1;
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
        self.async_lambda_captures
            .insert(name.clone(), captures.to_vec());
        self.declare_function(&lifted)
            .map_err(|error| format!("async lambda `{name}`: {error}"))?;

        let saved_variables = std::mem::take(&mut self.variables);
        let saved_for_iteration_frame_slots = std::mem::take(&mut self.for_iteration_frame_slots);
        let saved_catch_native_text = std::mem::take(&mut self.catch_native_text);
        let saved_variable_hir_types = std::mem::take(&mut self.variable_hir_types);
        let saved_arena_variables = std::mem::take(&mut self.arena_variables);
        let saved_catch_stack = std::mem::take(&mut self.catch_stack);
        let saved_loop_stack = std::mem::take(&mut self.loop_stack);
        let saved_async_completion = self.active_async_completion.take();
        let compiled = self.compile_function_body(&lifted);
        self.variables = saved_variables;
        self.for_iteration_frame_slots = saved_for_iteration_frame_slots;
        self.catch_native_text = saved_catch_native_text;
        self.variable_hir_types = saved_variable_hir_types;
        self.arena_variables = saved_arena_variables;
        self.catch_stack = saved_catch_stack;
        self.loop_stack = saved_loop_stack;
        self.active_async_completion = saved_async_completion;
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
        for (index, _) in captures.iter().enumerate() {
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
            arguments.push(cell.into());
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
        let (entry, this_adapter) = if params.first().is_some_and(|param| param.name == "__thaw_this") {
            self.compile_non_arrow_entries(adapter, params, &promise_type, &adapter_name)?
        } else {
            (adapter, self.compile_ignored_this_adapter(adapter, &param_types, &promise_type,
                &format!("{adapter_name}__thaw_this_adapter"))?)
        };
        Ok(self.allocate_lambda_environment(entry, this_adapter, captures)?.into())
    }

    // Non-arrow closures expose a receiver-free ordinary entry and a tagged
    // entry. Both decode into the same hidden parameter ABI before calling
    // the one body; the ordinary entry supplies an undefined receiver.
    fn compile_non_arrow_entries(
        &mut self,
        target: FunctionValue<'ctx>,
        params: &[HirParam],
        ret: &HirType,
        name: &str,
    ) -> Result<(FunctionValue<'ctx>, FunctionValue<'ctx>), String> {
        let receiver = &params[0].ty;
        let visible = params[1..].iter().map(|param| param.ty.clone()).collect::<Vec<_>>();
        let parent = self.builder.get_insert_block().ok_or("non-arrow closure needs a parent block")?;
        let ordinary = self.module.add_function(
            &format!("{name}__ordinary"), self.function_type(&visible, ret)?, Some(Linkage::Internal));
        let entry = self.context.append_basic_block(ordinary, "entry");
        self.builder.position_at_end(entry);
        let undefined = if *receiver == HirType::JsValue {
            let name = self.builder.build_global_string_ptr("undefined", "ordinary_this_name")
                .map_err(|error| error.to_string())?;
            self.builder.build_call(self.module.get_function("thaw_js_get_global").unwrap(),
                &[name.as_pointer_value().into()], "ordinary_this_handle")
                .map_err(|error| error.to_string())?.try_as_basic_value().basic()
                .ok_or("undefined JsValue returned no value")?
        } else {
            self.compile_non_arrow_receiver_value(ordinary, receiver,
                self.context.i8_type().const_zero(), self.context.i64_type().const_zero())?
        };
        let mut args = vec![ordinary.get_nth_param(0).unwrap().into(), undefined.into()];
        args.extend(ordinary.get_param_iter().skip(1).map(BasicMetadataValueEnum::from));
        let call = self.builder.build_call(target, &args, "invoke_non_arrow_ordinary")
            .map_err(|error| error.to_string())?;
        if *ret == HirType::Void { self.builder.build_return(None).map_err(|error| error.to_string())?; }
        else { self.builder.build_return(Some(&call.try_as_basic_value().basic()
            .ok_or("non-arrow ordinary entry returned no value")?)).map_err(|error| error.to_string())?; }
        self.builder.position_at_end(parent);
        let explicit = self.compile_non_arrow_this_entry(target, &visible, receiver, ret, name)?;
        Ok((ordinary, explicit))
    }

    fn compile_non_arrow_this_entry(
        &mut self,
        target: FunctionValue<'ctx>,
        visible: &[HirType],
        receiver: &HirType,
        ret: &HirType,
        name: &str,
    ) -> Result<FunctionValue<'ctx>, String> {
        let parent = self.builder.get_insert_block().ok_or("this entry needs a parent block")?;
        let outer_catch_stack = std::mem::take(&mut self.catch_stack);
        let outer_async_completion = self.active_async_completion.take();
        let entry_fn = self.module.add_function(&format!("{name}__thaw_this_adapter"),
            self.this_entry_function_type(visible, ret)?, Some(Linkage::Internal));
        let entry = self.context.append_basic_block(entry_fn, "entry");
        self.builder.position_at_end(entry);
        let tagged = entry_fn.get_nth_param(1).unwrap().into_struct_value();
        let kind = self.builder.build_extract_value(tagged, 0, "receiver_kind")
            .map_err(|error| error.to_string())?.into_int_value();
        let word = self.builder.build_extract_value(tagged, 1, "receiver_word")
            .map_err(|error| error.to_string())?.into_int_value();
        let value = self.compile_non_arrow_receiver_value(entry_fn, receiver, kind, word)?;
        let mut args = vec![entry_fn.get_nth_param(0).unwrap().into(), value.into()];
        args.extend(entry_fn.get_param_iter().skip(2).map(BasicMetadataValueEnum::from));
        let call = self.builder.build_call(target, &args, "invoke_non_arrow_with_this")
            .map_err(|error| error.to_string())?;
        if *ret == HirType::Void { self.builder.build_return(None).map_err(|error| error.to_string())?; }
        else { self.builder.build_return(Some(&call.try_as_basic_value().basic()
            .ok_or("non-arrow this entry returned no value")?)).map_err(|error| error.to_string())?; }
        self.catch_stack = outer_catch_stack;
        self.active_async_completion = outer_async_completion;
        self.builder.position_at_end(parent);
        Ok(entry_fn)
    }

    /// Query only the private native-object wrapper metadata. The returned
    /// pointer is valid while the graph Host lease keeps the JS wrapper and
    /// its ArenaRoot guardian alive through the callback invocation.
    fn query_native_object_receiver(
        &mut self,
        json: PointerValue<'ctx>,
        fields: &[(String, HirType)],
    ) -> Result<(IntValue<'ctx>, IntValue<'ctx>), String> {
        let actual_fields = if fields.last().is_some_and(|(name, _)|
            name == "__thaw_object_method_receiver") {
            &fields[..fields.len() - 1]
        } else { fields };
        let expected = thaw_hir::native_object_layout_token(actual_fields);
        let expected = self.builder.build_global_string_ptr(&expected, "native_receiver_layout")
            .map_err(|error| error.to_string())?;
        let handle = self.builder.build_call(
            self.module.get_function("thaw_json_borrowed_handle_id").unwrap(),
            &[json.into()], "native_receiver_handle")
            .map_err(|error| error.to_string())?.try_as_basic_value().basic()
            .ok_or("native receiver has no handle result")?.into_int_value();
        let result = self.builder.build_call(
            self.module.get_function("thaw_js_native_object_pointer").unwrap(),
            &[handle.into(), expected.as_pointer_value().into()], "native_receiver_pointer")
            .map_err(|error| error.to_string())?.try_as_basic_value().basic()
            .ok_or("native receiver has no pointer result")?.into_struct_value();
        let pointer = self.builder.build_extract_value(result, 0, "native_receiver_pointer_word")
            .map_err(|error| error.to_string())?.into_int_value();
        let error = self.builder.build_extract_value(result, 1, "native_receiver_pointer_error")
            .map_err(|error| error.to_string())?.into_pointer_value();
        self.builder.build_call(self.module.get_function("thaw_cstring_destroy").unwrap(),
            &[error.into()], "destroy_native_receiver_query_error")
            .map_err(|error| error.to_string())?;
        let present = self.builder.build_int_compare(IntPredicate::NE, pointer,
            self.context.i64_type().const_zero(), "native_receiver_pointer_present")
            .map_err(|error| error.to_string())?;
        Ok((pointer, present))
    }

    fn non_arrow_receiver_tags(receiver: &HirType) -> Result<Vec<u64>, String> {
        let tags = match receiver {
            HirType::Undefined => vec![0],
            HirType::Null => vec![1],
            HirType::Bool => vec![2],
            HirType::F64 => vec![3],
            HirType::I64 => vec![4],
            HirType::Str | HirType::StrLiteral(_) => vec![5],
            HirType::Symbol => vec![6],
            HirType::Json => vec![0, 1, 2, 3, 5, 7, 8],
            HirType::Dictionary(_) => vec![7],
            HirType::JsValue => vec![8],
            HirType::Object(fields) if fields.first().is_some_and(|(name, _)|
                    name.starts_with("__thaw_class_identity_\u{1e}")) => vec![9],
            // A native fixed object crosses the graph callback boundary as
            // a borrowed Host Json. Its private wrapper token is checked at
            // call time before recovering the original arena pointer.
            HirType::Object(_) => vec![7],
            HirType::Optional(inner) => {
                let mut tags = Self::non_arrow_receiver_tags(inner)?;
                if !tags.contains(&0) { tags.push(0); }
                tags
            }
            HirType::Nullable(inner) => {
                let mut tags = Self::non_arrow_receiver_tags(inner)?;
                if !tags.contains(&1) { tags.push(1); }
                tags
            }
            HirType::Nullish(inner) => {
                let mut tags = Self::non_arrow_receiver_tags(inner)?;
                for tag in [0, 1] { if !tags.contains(&tag) { tags.push(tag); } }
                tags
            }
            HirType::Union(members) => {
                let mut tags = Vec::new();
                for member in members {
                    for tag in Self::non_arrow_receiver_tags(member)? {
                        if tags.contains(&tag) {
                            return Err("union receiver members have indistinguishable runtime tags".into());
                        }
                        tags.push(tag);
                    }
                }
                tags
            }
            other => return Err(format!("unsupported non-arrow this type {other:?}")),
        };
        Ok(tags)
    }

    fn reject_non_arrow_receiver(&mut self) -> Result<(), String> {
        let saved_catch_stack = std::mem::take(&mut self.catch_stack);
        let saved_async_completion = self.active_async_completion.take();
        self.compile_throw_type_error("Incompatible function receiver")?;
        self.catch_stack = saved_catch_stack;
        self.active_async_completion = saved_async_completion;
        Ok(())
    }

    fn compile_non_arrow_receiver_value(
        &mut self,
        entry_fn: FunctionValue<'ctx>,
        receiver: &HirType,
        kind: IntValue<'ctx>,
        word: IntValue<'ctx>,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        match receiver {
            HirType::Object(fields) if !fields.first().is_some_and(|(name, _)|
                    name.starts_with("__thaw_class_identity_\u{1e}")) => {
                let is_host = self.builder.build_int_compare(IntPredicate::EQ, kind,
                    self.context.i8_type().const_int(7, false), "native_receiver_is_host")
                    .map_err(|error| error.to_string())?;
                let host = self.context.append_basic_block(entry_fn, "native_receiver_host");
                let rejected = self.context.append_basic_block(entry_fn, "native_receiver_rejected");
                let accepted = self.context.append_basic_block(entry_fn, "native_receiver_accepted");
                self.builder.build_conditional_branch(is_host, host, rejected)
                    .map_err(|error| error.to_string())?;
                self.builder.position_at_end(host);
                let json = self.builder.build_int_to_ptr(word,
                    self.context.ptr_type(AddressSpace::default()), "native_receiver_json")
                    .map_err(|error| error.to_string())?;
                let (pointer, present) = self.query_native_object_receiver(json, fields)?;
                self.builder.build_conditional_branch(present, accepted, rejected)
                    .map_err(|error| error.to_string())?;
                self.builder.position_at_end(rejected);
                self.reject_non_arrow_receiver()?;
                self.builder.position_at_end(accepted);
                return self.builder.build_int_to_ptr(pointer,
                    self.context.ptr_type(AddressSpace::default()), "native_receiver_original_pointer")
                    .map(Into::into).map_err(|error| error.to_string());
            }
            HirType::Json => return self.compile_non_arrow_json_value(entry_fn, kind, word).map(Into::into),
            HirType::Optional(inner) | HirType::Nullable(inner) | HirType::Nullish(inner) => {
                let is_absent = match receiver {
                    HirType::Optional(_) => self.builder.build_int_compare(IntPredicate::EQ, kind,
                        self.context.i8_type().const_zero(), "receiver_optional_absent"),
                    HirType::Nullable(_) => self.builder.build_int_compare(IntPredicate::EQ, kind,
                        self.context.i8_type().const_int(1, false), "receiver_nullable_absent"),
                    _ => {
                        let is_undefined = self.builder.build_int_compare(IntPredicate::EQ, kind,
                            self.context.i8_type().const_zero(), "receiver_nullish_undefined")
                            .map_err(|error| error.to_string())?;
                        let is_null = self.builder.build_int_compare(IntPredicate::EQ, kind,
                            self.context.i8_type().const_int(1, false), "receiver_nullish_null")
                            .map_err(|error| error.to_string())?;
                        self.builder.build_or(is_undefined, is_null, "receiver_nullish_absent")
                    }
                }.map_err(|error| error.to_string())?;
                let absent = self.context.append_basic_block(entry_fn, "receiver_absent");
                let present = self.context.append_basic_block(entry_fn, "receiver_present");
                let join = self.context.append_basic_block(entry_fn, "receiver_join");
                self.builder.build_conditional_branch(is_absent, absent, present)
                    .map_err(|error| error.to_string())?;
                self.builder.position_at_end(absent);
                let zero = self.basic_type(inner)?.const_zero();
                let absent_value = if matches!(receiver, HirType::Nullish(_)) {
                    let tag = self.builder.build_select(
                        self.builder.build_int_compare(IntPredicate::EQ, kind,
                            self.context.i8_type().const_int(1, false), "receiver_absent_is_null")
                            .map_err(|error| error.to_string())?,
                        self.context.i8_type().const_int(1, false),
                        self.context.i8_type().const_int(2, false), "receiver_nullish_absent_tag")
                        .map_err(|error| error.to_string())?.into_int_value();
                    self.build_nullish_tagged_value(zero, inner, tag)?
                } else { self.build_optional_value(zero, inner, false)? };
                let absent_exit = self.builder.get_insert_block().unwrap();
                self.builder.build_unconditional_branch(join).map_err(|error| error.to_string())?;
                self.builder.position_at_end(present);
                let payload = self.compile_non_arrow_receiver_value(entry_fn, inner, kind, word)?;
                let present_value = if matches!(receiver, HirType::Nullish(_)) {
                    self.build_nullish_value(payload, inner, 0)?
                } else { self.build_optional_value(payload, inner, true)? };
                let present_exit = self.builder.get_insert_block().unwrap();
                self.builder.build_unconditional_branch(join).map_err(|error| error.to_string())?;
                self.builder.position_at_end(join);
                let selected = self.builder.build_phi(self.basic_type(receiver)?, "receiver_optional_value")
                    .map_err(|error| error.to_string())?;
                selected.add_incoming(&[
                    (&absent_value as &dyn inkwell::values::BasicValue<'ctx>, absent_exit),
                    (&present_value as &dyn inkwell::values::BasicValue<'ctx>, present_exit),
                ]);
                return Ok(selected.as_basic_value());
            }
            HirType::Union(members) => {
                let mut cases = Vec::new();
                let mut claimed = HashSet::new();
                let mut classes = Vec::new();
                let mut string_literals: Vec<(usize, String)> = Vec::new();
                let mut string_catchall = None;
                let mut json_fallback = None;
                for (index, member) in members.iter().enumerate() {
                    if matches!(member, HirType::Optional(_) | HirType::Nullable(_)
                        | HirType::Nullish(_) | HirType::Union(_)) {
                        return Err("nested tagged receiver union member exceeds the union word ABI".into());
                    }
                    // The caller sends the actual HIR value kind, not every
                    // kind a broad Json receiver can convert. Claim exact
                    // producers first, then use Json for otherwise unclaimed
                    // primitive kinds.
                    if *member == HirType::Json {
                        if json_fallback.replace(index).is_some() {
                            return Err("ambiguous Json receiver union".into());
                        }
                        continue;
                    }
                    // All string literals use source tag 5. Resolve their
                    // values before the broad string or Json member.
                    if let HirType::StrLiteral(literal) = member {
                        if string_literals.iter().any(|(_, existing)| existing == literal) {
                            return Err("duplicate string literal receiver union member".into());
                        }
                        string_literals.push((index, literal.clone()));
                        continue;
                    }
                    if *member == HirType::Str {
                        if string_catchall.replace(index).is_some() {
                            return Err("duplicate string receiver union member".into());
                        }
                        continue;
                    }
                    if let HirType::Object(fields) = member {
                        if let Some(marker) = fields.first().and_then(|(name, _)|
                            name.strip_prefix("__thaw_class_identity_\u{1e}")) {
                            let class = marker.split('\u{1f}').next().unwrap().to_string();
                            let depth = marker.split('\u{1f}').count();
                            if classes.iter().any(|(_, existing, _): &(usize, String, usize)| existing == &class) {
                                return Err("duplicate native class receiver union member".into());
                            }
                            classes.push((index, class, depth));
                            continue;
                        }
                    }
                    for tag in Self::non_arrow_receiver_tags(member)? {
                        if !claimed.insert(tag) {
                            return Err("union receiver members have indistinguishable runtime tags".into());
                        }
                        cases.push((index, tag, self.context.append_basic_block(entry_fn,
                            &format!("receiver_union_{index}_{tag}"))));
                    }
                }
                if let Some(index) = json_fallback {
                    if claimed.contains(&7) {
                        return Err("Json and dictionary receiver union members share the same source tag".into());
                    }
                    for tag in [0_u64, 1, 2, 3, 4, 5, 7, 8] {
                        if tag == 5 && (!string_literals.is_empty() || string_catchall.is_some()) {
                            continue;
                        }
                        if claimed.insert(tag) {
                            cases.push((index, tag, self.context.append_basic_block(entry_fn,
                                &format!("receiver_union_json_{tag}"))));
                        }
                    }
                }
                let rejected = self.context.append_basic_block(entry_fn, "receiver_union_rejected");
                let join = self.context.append_basic_block(entry_fn, "receiver_union_join");
                let class_dispatch = (!classes.is_empty()).then(||
                    self.context.append_basic_block(entry_fn, "receiver_union_native_classes"));
                let string_dispatch = (!string_literals.is_empty() || string_catchall.is_some()).then(||
                    self.context.append_basic_block(entry_fn, "receiver_union_strings"));
                let mut switches = cases.iter().map(|(_, tag, block)|
                    (self.context.i8_type().const_int(*tag, false), *block)).collect::<Vec<_>>();
                if let Some(block) = class_dispatch {
                    switches.push((self.context.i8_type().const_int(9, false), block));
                }
                if let Some(block) = string_dispatch {
                    switches.push((self.context.i8_type().const_int(5, false), block));
                }
                self.builder.build_switch(kind, rejected, &switches)
                    .map_err(|error| error.to_string())?;
                let mut incoming = Vec::new();
                for (index, _, block) in cases {
                    self.builder.position_at_end(block);
                    let value = self.compile_non_arrow_receiver_value(entry_fn, &members[index], kind, word)?;
                    let tagged = self.build_union_value(value, index, members)?;
                    let exit = self.builder.get_insert_block().unwrap();
                    self.builder.build_unconditional_branch(join).map_err(|error| error.to_string())?;
                    incoming.push((tagged, exit));
                }
                if let Some(block) = string_dispatch {
                    self.builder.position_at_end(block);
                    let actual = self.builder.build_int_to_ptr(word,
                        self.context.ptr_type(AddressSpace::default()), "receiver_union_string_pointer")
                        .map_err(|error| error.to_string())?;
                    for (index, literal) in string_literals {
                        let matched = self.context.append_basic_block(entry_fn,
                            &format!("receiver_union_literal_{index}"));
                        let next = self.context.append_basic_block(entry_fn,
                            &format!("receiver_union_next_literal_{index}"));
                        let expected = self.compile_raw_string_literal(literal.as_bytes())?.into_pointer_value();
                        let compared = self.builder.build_call(
                            self.module.get_function("thaw_string_compare").unwrap(),
                            &[actual.into(), expected.into()], "receiver_union_string_compare")
                            .map_err(|error| error.to_string())?.try_as_basic_value().basic()
                            .ok_or("string comparison returned no value")?.into_int_value();
                        let equal = self.builder.build_int_compare(IntPredicate::EQ, compared,
                            self.context.i32_type().const_zero(), "receiver_union_literal_matches")
                            .map_err(|error| error.to_string())?;
                        self.builder.build_conditional_branch(equal, matched, next)
                            .map_err(|error| error.to_string())?;
                        self.builder.position_at_end(matched);
                        let value = self.compile_non_arrow_receiver_value(entry_fn, &members[index], kind, word)?;
                        let tagged = self.build_union_value(value, index, members)?;
                        let exit = self.builder.get_insert_block().unwrap();
                        self.builder.build_unconditional_branch(join).map_err(|error| error.to_string())?;
                        incoming.push((tagged, exit));
                        self.builder.position_at_end(next);
                    }
                    if let Some(index) = string_catchall.or(json_fallback) {
                        let value = self.compile_non_arrow_receiver_value(entry_fn, &members[index], kind, word)?;
                        let tagged = self.build_union_value(value, index, members)?;
                        let exit = self.builder.get_insert_block().unwrap();
                        self.builder.build_unconditional_branch(join).map_err(|error| error.to_string())?;
                        incoming.push((tagged, exit));
                    } else {
                        self.builder.build_unconditional_branch(rejected).map_err(|error| error.to_string())?;
                    }
                }
                if let Some(block) = class_dispatch {
                    self.builder.position_at_end(block);
                    // A derived identity also matches its ancestors. Test
                    // the deepest declared class first so Base | Derived
                    // selects Derived for a Derived instance.
                    classes.sort_by_key(|(_, _, depth)| *depth);
                    for (index, class, _) in classes.into_iter().rev() {
                        let matched = self.context.append_basic_block(entry_fn,
                            &format!("receiver_union_class_{index}"));
                        let next = self.context.append_basic_block(entry_fn,
                            &format!("receiver_union_next_class_{index}"));
                        let pointer = self.builder.build_int_to_ptr(word,
                            self.context.ptr_type(AddressSpace::default()), "receiver_union_class_pointer")
                            .map_err(|error| error.to_string())?;
                        let expected = self.builder.build_global_string_ptr(&class, "receiver_union_class_name")
                            .map_err(|error| error.to_string())?;
                        let is_class = self.builder.build_call(
                            self.module.get_function("thaw_object_has_class_identity").unwrap(),
                            &[pointer.into(), expected.as_pointer_value().into()], "receiver_union_is_class")
                            .map_err(|error| error.to_string())?.try_as_basic_value().basic()
                            .ok_or("class identity returned no value")?.into_int_value();
                        self.builder.build_conditional_branch(is_class, matched, next)
                            .map_err(|error| error.to_string())?;
                        self.builder.position_at_end(matched);
                        let value = self.compile_non_arrow_receiver_value(entry_fn, &members[index], kind, word)?;
                        let tagged = self.build_union_value(value, index, members)?;
                        let exit = self.builder.get_insert_block().unwrap();
                        self.builder.build_unconditional_branch(join).map_err(|error| error.to_string())?;
                        incoming.push((tagged, exit));
                        self.builder.position_at_end(next);
                    }
                    self.builder.build_unconditional_branch(rejected).map_err(|error| error.to_string())?;
                }
                self.builder.position_at_end(rejected);
                self.reject_non_arrow_receiver()?;
                self.builder.position_at_end(join);
                let selected = self.builder.build_phi(self.basic_type(receiver)?, "receiver_union_value")
                    .map_err(|error| error.to_string())?;
                selected.add_incoming(&incoming.iter().map(|(value, block)|
                    (value as &dyn inkwell::values::BasicValue<'ctx>, *block)).collect::<Vec<_>>());
                return Ok(selected.as_basic_value());
            }
            _ => {}
        }
        let (expected, value): (u64, BasicValueEnum<'ctx>) = match receiver {
            HirType::Undefined => (0, self.context.bool_type().const_zero().into()),
            HirType::Null => (1, self.context.bool_type().const_int(1, false).into()),
            HirType::Dictionary(_) => (7, self.builder.build_int_to_ptr(word,
                self.context.ptr_type(AddressSpace::default()), "receiver_json")
                .map_err(|error| error.to_string())?.into()),
            HirType::JsValue => (8, word.into()),
            HirType::Bool => (2, self.builder.build_int_truncate(word, self.context.bool_type(), "receiver_bool")
                .map_err(|error| error.to_string())?.into()),
            HirType::F64 => (3, self.builder.build_bit_cast(word, self.context.f64_type(), "receiver_number")
                .map_err(|error| error.to_string())?),
            HirType::I64 => (4, word.into()),
            HirType::Str | HirType::StrLiteral(_) | HirType::Symbol => {
                let expected = if matches!(receiver, HirType::Symbol) { 6 } else { 5 };
                (expected, self.builder.build_int_to_ptr(word,
                    self.context.ptr_type(AddressSpace::default()), "receiver_string")
                    .map_err(|error| error.to_string())?.into())
            }
            HirType::Object(fields) if fields.first().is_some_and(|(name, _)|
                    name.starts_with("__thaw_class_identity_\u{1e}")) => (9,
                self.builder.build_int_to_ptr(word, self.context.ptr_type(AddressSpace::default()),
                    "receiver_native_class").map_err(|error| error.to_string())?.into()),
            other => return Err(format!("unsupported non-arrow this type {other:?}")),
        };
        let kind_matches = self.builder.build_int_compare(IntPredicate::EQ, kind,
            self.context.i8_type().const_int(expected, false), "receiver_kind_matches")
            .map_err(|error| error.to_string())?;
        let valid = if let HirType::Object(fields) = receiver {
            let marker = fields[0].0.strip_prefix("__thaw_class_identity_\u{1e}").unwrap();
            let class = marker.split('\u{1f}').next().unwrap();
            let expected_class = self.builder.build_global_string_ptr(class, "receiver_expected_class")
                .map_err(|error| error.to_string())?;
            let identity = self.builder.build_call(self.module.get_function("thaw_object_has_class_identity").unwrap(),
                &[value.into_pointer_value().into(), expected_class.as_pointer_value().into()],
                "receiver_class_identity").map_err(|error| error.to_string())?
                .try_as_basic_value().basic().ok_or("class identity returned no value")?.into_int_value();
            self.builder.build_and(kind_matches, identity, "receiver_valid_class")
                .map_err(|error| error.to_string())?
        } else { kind_matches };
        let accepted = self.context.append_basic_block(entry_fn, "receiver_accepted");
        let rejected = self.context.append_basic_block(entry_fn, "receiver_rejected");
        self.builder.build_conditional_branch(valid, accepted, rejected)
            .map_err(|error| error.to_string())?;
        self.builder.position_at_end(rejected);
        self.reject_non_arrow_receiver()?;
        self.builder.position_at_end(accepted);
        if let HirType::StrLiteral(literal) = receiver {
            // A literal receiver keeps the same string ABI but must not
            // silently accept another string passed through .call/.apply.
            let expected = self.compile_raw_string_literal(literal.as_bytes())?.into_pointer_value();
            let compared = self.builder.build_call(self.module.get_function("thaw_string_compare").unwrap(),
                &[value.into_pointer_value().into(), expected.into()], "receiver_literal_compare")
                .map_err(|error| error.to_string())?.try_as_basic_value().basic()
                .ok_or("string comparison returned no value")?.into_int_value();
            let same = self.builder.build_int_compare(IntPredicate::EQ, compared,
                self.context.i32_type().const_zero(), "receiver_literal_matches")
                .map_err(|error| error.to_string())?;
            let matched = self.context.append_basic_block(entry_fn, "receiver_literal_accepted");
            self.builder.build_conditional_branch(same, matched, rejected)
                .map_err(|error| error.to_string())?;
            self.builder.position_at_end(matched);
        }
        Ok(value)
    }

    // All scalar cases create an owned Json root, just like other Json
    // constructors. A lambda parameter is borrowed for its invocation and
    // may return or capture the root, so the adapter cannot destroy it here.
    fn compile_non_arrow_json_value(
        &mut self,
        entry_fn: FunctionValue<'ctx>,
        kind: IntValue<'ctx>,
        word: IntValue<'ctx>,
    ) -> Result<PointerValue<'ctx>, String> {
        self.tracks_owned_json_roots = true;
        let join = self.context.append_basic_block(entry_fn, "receiver_json_ready");
        let rejected = self.context.append_basic_block(entry_fn, "receiver_json_rejected");
        let cases = [0_u64, 1, 2, 3, 4, 5, 7, 8]
            .map(|tag| (tag, self.context.append_basic_block(entry_fn, &format!("receiver_json_{tag}"))));
        let branches = cases.iter().map(|(tag, block)|
            (self.context.i8_type().const_int(*tag, false), *block)).collect::<Vec<_>>();
        self.builder.build_switch(kind, rejected, &branches)
            .map_err(|error| error.to_string())?;
        let pointer = self.context.ptr_type(AddressSpace::default());
        let mut incoming = Vec::with_capacity(cases.len());
        for (tag, block) in cases {
            self.builder.position_at_end(block);
            let value = match tag {
                0 | 1 => self.builder.build_call(
                    self.module.get_function(if tag == 0 { "thaw_json_undefined" } else { "thaw_json_null" }).unwrap(),
                    &[], "receiver_json_nullish")
                    .map_err(|error| error.to_string())?.try_as_basic_value().basic()
                    .ok_or("nullish receiver constructor returned no value")?.into_pointer_value(),
                2 => {
                    let value = self.builder.build_int_truncate(word, self.context.i8_type(), "receiver_json_bool")
                        .map_err(|error| error.to_string())?;
                    self.builder.build_call(self.module.get_function("thaw_json_receiver_bool").unwrap(),
                        &[value.into()], "receiver_json_bool_root")
                        .map_err(|error| error.to_string())?.try_as_basic_value().basic()
                        .ok_or("bool receiver constructor returned no value")?.into_pointer_value()
                }
                3 => {
                    let value = self.builder.build_bit_cast(word, self.context.f64_type(), "receiver_json_number")
                        .map_err(|error| error.to_string())?;
                    self.builder.build_call(self.module.get_function("thaw_json_receiver_number").unwrap(),
                        &[value.into()], "receiver_json_number_root")
                        .map_err(|error| error.to_string())?.try_as_basic_value().basic()
                        .ok_or("number receiver constructor returned no value")?.into_pointer_value()
                }
                4 => self.builder.build_call(self.module.get_function("thaw_json_receiver_bigint").unwrap(),
                    &[word.into()], "receiver_json_bigint_root")
                    .map_err(|error| error.to_string())?.try_as_basic_value().basic()
                    .ok_or("BigInt receiver constructor returned no value")?.into_pointer_value(),
                5 => {
                    let value = self.builder.build_int_to_ptr(word, pointer, "receiver_json_string")
                        .map_err(|error| error.to_string())?;
                    self.builder.build_call(self.module.get_function("thaw_json_receiver_string").unwrap(),
                        &[value.into()], "receiver_json_string_root")
                        .map_err(|error| error.to_string())?.try_as_basic_value().basic()
                        .ok_or("string receiver constructor returned no value")?.into_pointer_value()
                }
                7 => self.builder.build_int_to_ptr(word, pointer, "receiver_json_existing")
                    .map_err(|error| error.to_string())?,
                8 => {
                    self.uses_quickjs = true;
                    self.uses_quickjs_handles = true;
                    self.compile_register_js_callback_host_operations()?;
                    let value = self.builder.build_call(
                        self.module.get_function("thaw_json_host_from_borrowed_handle").unwrap(),
                        &[word.into()], "receiver_json_live_host")
                        .map_err(|error| error.to_string())?
                        .try_as_basic_value().basic()
                        .ok_or("live receiver conversion returned no value")?;
                    self.compile_check_json_host_error(value, Some("thaw_json_destroy"))?
                        .into_pointer_value()
                }
                _ => unreachable!(),
            };
            let value = if tag == 7 || tag == 8 { value } else {
                let tracked = self.builder.build_call(
                    self.module.get_function("thaw_json_track_arena_owned_root").unwrap(),
                    &[value.into()], "track_receiver_json_root")
                    .map_err(|error| error.to_string())?
                    .try_as_basic_value().basic()
                    .ok_or("tracked receiver returned no value")?;
                self.compile_check_json_host_error(tracked, Some("thaw_json_destroy"))?
                    .into_pointer_value()
            };
            let exit = self.builder.get_insert_block().ok_or("receiver conversion has no exit")?;
            self.builder.build_unconditional_branch(join).map_err(|error| error.to_string())?;
            incoming.push((value, exit));
        }
        self.builder.position_at_end(rejected);
        let saved_catch_stack = std::mem::take(&mut self.catch_stack);
        let saved_async_completion = self.active_async_completion.take();
        self.compile_throw_type_error("Incompatible function receiver")?;
        self.catch_stack = saved_catch_stack;
        self.active_async_completion = saved_async_completion;
        self.builder.position_at_end(join);
        let selected = self.builder.build_phi(pointer, "receiver_json_value")
            .map_err(|error| error.to_string())?;
        selected.add_incoming(&incoming.iter().map(|(value, block)|
            (value as &dyn inkwell::values::BasicValue<'ctx>, *block)).collect::<Vec<_>>());
        Ok(selected.as_basic_value().into_pointer_value())
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
        let (entry, this_adapter) = if params.first().is_some_and(|param| param.name == "__thaw_this") {
            self.compile_non_arrow_entries(function, params, ret, &name)?
        } else {
            (function, self.compile_ignored_this_adapter(function, &param_types, ret,
                &format!("{name}__thaw_this_adapter"))?)
        };

        let i64_type = self.context.i64_type();
        let frame_captures = captures
            .iter()
            .filter(|capture| {
                self.variables
                    .get(&capture.name)
                    .is_some_and(|(cell, _)| self.async_frame_cells.contains(cell))
            })
            .map(|capture| capture.name.clone())
            .collect::<HashSet<_>>();
        // Closure captures retain their variable cells so mutations remain
        // visible when the function value is invoked later.
        let closure = self.allocate_lambda_environment(entry, this_adapter, captures)?;
        let arena_captures = captures
            .iter()
            .filter(|capture| self.arena_variables.contains(&capture.name))
            .map(|capture| capture.name.clone())
            .collect::<HashSet<_>>();

        let saved_variables = std::mem::take(&mut self.variables);
        let saved_for_iteration_frame_slots = std::mem::take(&mut self.for_iteration_frame_slots);
        let saved_catch_native_text = std::mem::take(&mut self.catch_native_text);
        let saved_variable_hir_types = std::mem::take(&mut self.variable_hir_types);
        let saved_arena_variables = std::mem::replace(&mut self.arena_variables, arena_captures);
        let saved_catch_stack = std::mem::take(&mut self.catch_stack);
        let saved_loop_stack = std::mem::take(&mut self.loop_stack);
        let saved_async_completion = self.active_async_completion.take();
        let saved_return_ticket = self.active_promise_return_ticket.take();
        if matches!(ret, HirType::Promise(_)) {
            self.active_promise_return_ticket = Some(function.get_last_param()
                .ok_or("Promise closure lacks creator ticket output")?.into_pointer_value());
        }
        let result = (|| -> Result<(), String> {
            let entry = self.context.append_basic_block(function, "entry");
            self.builder.position_at_end(entry);
            if let Some(out_ticket) = self.active_promise_return_ticket {
                self.builder.build_store(out_ticket, i64_type.const_zero())
                    .map_err(|error| error.to_string())?;
            }
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
                    .map_err(|error| error.to_string())?
                    .into_pointer_value();
                if frame_captures.contains(&capture.name) {
                    self.async_frame_cells.insert(variable_cell);
                }
                self.variables.insert(
                    capture.name.clone(),
                    (variable_cell, ty),
                );
                self.variable_hir_types
                    .insert(capture.name.clone(), capture.ty.clone());
            }
            let mut promise_index = 0_u64;
            for capture in captures {
                if !matches!(&capture.ty, HirType::Promise(_)) { continue; }
                let (value_cell, _) = self.variables.get(&capture.name).copied()
                    .ok_or_else(|| format!("missing captured Promise `{}`", capture.name))?;
                let offset = i64_type.const_int(CLOSURE_CAPTURE_BASE
                    + (captures.len() as u64 + promise_index) * 8, false);
                let slot = unsafe { self.builder.build_in_bounds_gep(self.context.i8_type(),
                    environment, &[offset], "captured_creator_ticket_entry")
                    .map_err(|error| error.to_string())? };
                let ticket_cell = self.builder.build_load(
                    self.context.ptr_type(AddressSpace::default()), slot,
                    "captured_creator_ticket_cell",
                ).map_err(|error| error.to_string())?.into_pointer_value();
                self.promise_creator_cell_tickets.insert(value_cell, ticket_cell);
                promise_index += 1;
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
                        if let Some(out_ticket) = self.active_promise_return_ticket {
                            let ticket = self.promise_creator_tickets
                                .get(&value.into_pointer_value()).copied()
                                .unwrap_or_else(|| i64_type.const_zero());
                            self.clear_promise_creator_ticket_source(value.into_pointer_value())?;
                            self.builder.build_store(out_ticket, ticket)
                                .map_err(|error| error.to_string())?;
                        }
                        self.builder
                            .build_return(Some(&value))
                            .map_err(|error| error.to_string())?;
                    }
                }
            }
            Ok(())
        })();
        self.variables = saved_variables;
        self.for_iteration_frame_slots = saved_for_iteration_frame_slots;
        self.catch_native_text = saved_catch_native_text;
        self.variable_hir_types = saved_variable_hir_types;
        self.arena_variables = saved_arena_variables;
        self.catch_stack = saved_catch_stack;
        self.loop_stack = saved_loop_stack;
        self.active_async_completion = saved_async_completion;
        self.active_promise_return_ticket = saved_return_ticket;
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
        receiver_class: Option<&str>,
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
        let this_entry = self.module.add_function(
            &format!("{ordinary_name}__thaw_this_adapter"),
            self.this_entry_function_type(params, ret)?,
            Some(Linkage::Internal),
        );
        let entry = self.context.append_basic_block(this_entry, "entry");
        self.builder.position_at_end(entry);
        let mut arguments = Vec::with_capacity(params.len() + usize::from(!is_static));
        if !is_static {
            let receiver_class = receiver_class.ok_or("instance method is missing its nominal receiver")?;
            let tagged = this_entry.get_nth_param(1).unwrap().into_struct_value();
            let kind = self.builder.build_extract_value(tagged, 0, "method_receiver_kind")
                .map_err(|error| error.to_string())?.into_int_value();
            let word = self.builder.build_extract_value(tagged, 1, "method_receiver_word")
                .map_err(|error| error.to_string())?.into_int_value();
            let receiver = self
                .builder
                .build_int_to_ptr(
                    word,
                    self.context.ptr_type(AddressSpace::default()),
                    "method_receiver",
                )
                .map_err(|error| error.to_string())?;
            let expected = self.builder.build_global_string_ptr(receiver_class, "expected_receiver_class")
                .map_err(|error| error.to_string())?;
            let actual_class = self.builder.build_call(
                self.module.get_function("thaw_object_has_class_identity").unwrap(),
                &[receiver.into(), expected.as_pointer_value().into()],
                "method_receiver_identity",
            ).map_err(|error| error.to_string())?.try_as_basic_value().basic()
                .ok_or("class identity check returned no value")?.into_int_value();
            let native_kind = self.builder.build_int_compare(
                IntPredicate::EQ, kind, self.context.i8_type().const_int(9, false),
                "method_receiver_is_native",
            ).map_err(|error| error.to_string())?;
            let valid = self.builder.build_and(native_kind, actual_class, "method_receiver_valid")
                .map_err(|error| error.to_string())?;
            let rejected = self.context.append_basic_block(this_entry, "method_receiver_rejected");
            let accepted = self.context.append_basic_block(this_entry, "method_receiver_accepted");
            self.builder.build_conditional_branch(valid, accepted, rejected)
                .map_err(|error| error.to_string())?;
            self.builder.position_at_end(rejected);
            let saved_catch_stack = std::mem::take(&mut self.catch_stack);
            let saved_async_completion = self.active_async_completion.take();
            self.compile_throw_type_error("Incompatible method receiver")?;
            self.catch_stack = saved_catch_stack;
            self.active_async_completion = saved_async_completion;
            self.builder.position_at_end(accepted);
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
