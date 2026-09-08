impl<'ctx> HirCompiler<'ctx> {
    fn async_frame_field(
        &self,
        frame: PointerValue<'ctx>,
        offset: u64,
        name: &str,
    ) -> Result<PointerValue<'ctx>, String> {
        let offset = self.context.i64_type().const_int(offset, false);
        unsafe {
            self.builder
                .build_in_bounds_gep(self.context.i8_type(), frame, &[offset], name)
                .map_err(|e| e.to_string())
        }
    }

    fn compile_frame_async_function(
        &mut self,
        func: &HirFunction,
        plan: &FrameAsyncPlan,
    ) -> Result<(), String> {
        let segments = &plan.segments;
        let symbol = Self::llvm_symbol_for(&func.name);
        let ramp = self.module.get_function(&symbol).unwrap();
        let ptr_ty = self.context.ptr_type(AddressSpace::default());
        let resume_ty = self
            .context
            .void_type()
            .fn_type(&[ptr_ty.into(), ptr_ty.into()], false);
        let resume = self.module.add_function(
            &format!("{symbol}.resume"),
            resume_ty,
            Some(Linkage::Internal),
        );

        let entry = self.context.append_basic_block(ramp, "entry");
        self.builder.position_at_end(entry);
        self.variables.clear();
        self.variable_hir_types.clear();
        self.arena_variables.clear();
        self.catch_stack.clear();
        self.seed_global_variables();

        let alloc = self.module.get_function("thaw_arena_alloc").unwrap();
        let frame = self
            .builder
            .build_call(
                alloc,
                &[
                    self.context
                        .i64_type()
                        .const_int(self.async_frame_size(plan), false)
                        .into(),
                    self.context.i64_type().const_int(8, false).into(),
                ],
                "async_frame",
            )
            .map_err(|e| e.to_string())?
            .try_as_basic_value()
            .basic()
            .ok_or("arena allocator returned no frame")?
            .into_pointer_value();
        let completion = self
            .builder
            .build_call(
                self.module.get_function("thaw_promise_new").unwrap(),
                &[],
                "async_completion",
            )
            .map_err(|e| e.to_string())?
            .try_as_basic_value()
            .basic()
            .ok_or("promise allocator returned no value")?
            .into_pointer_value();
        let completion_slot =
            self.async_frame_field(frame, ASYNC_COMPLETION_OFFSET, "completion_slot")?;
        self.builder
            .build_store(completion_slot, completion)
            .map_err(|e| e.to_string())?;
        let waiting_slot = self.async_frame_field(frame, ASYNC_WAITING_OFFSET, "waiting_slot")?;
        self.builder
            .build_store(waiting_slot, ptr_ty.const_null())
            .map_err(|e| e.to_string())?;
        self.bind_async_frame_locals(frame, plan)?;
        for (param_value, param) in ramp.get_param_iter().zip(&func.params) {
            let (slot, _) = self.variables.get(&param.name).copied().ok_or_else(|| {
                format!("missing async frame parameter slot for `{}`", param.name)
            })?;
            self.builder
                .build_store(slot, param_value)
                .map_err(|e| e.to_string())?;
        }
        self.emit_async_segment(&segments[0], frame, completion, resume, 1, true, plan, None)?;

        let resume_entry = self.context.append_basic_block(resume, "entry");
        self.builder.position_at_end(resume_entry);
        self.variables.clear();
        self.variable_hir_types.clear();
        self.arena_variables.clear();
        self.catch_stack.clear();
        self.seed_global_variables();
        let resume_frame = resume.get_nth_param(0).unwrap().into_pointer_value();
        let resume_result = resume.get_nth_param(1).unwrap().into_pointer_value();
        let waiting_slot =
            self.async_frame_field(resume_frame, ASYNC_WAITING_OFFSET, "waiting_slot")?;
        let waiting = self
            .builder
            .build_load(ptr_ty, waiting_slot, "waiting_promise")
            .map_err(|e| e.to_string())?
            .into_pointer_value();
        let completion_slot =
            self.async_frame_field(resume_frame, ASYNC_COMPLETION_OFFSET, "completion_slot")?;
        let resume_completion = self
            .builder
            .build_load(ptr_ty, completion_slot, "completion")
            .map_err(|e| e.to_string())?
            .into_pointer_value();
        let state_slot = self.async_frame_field(resume_frame, ASYNC_STATE_OFFSET, "state_slot")?;
        let state = self
            .builder
            .build_load(self.context.i64_type(), state_slot, "async_state")
            .map_err(|e| e.to_string())?
            .into_int_value();
        let promise_state = self
            .builder
            .build_call(
                self.module.get_function("thaw_promise_state").unwrap(),
                &[waiting.into()],
                "waiting_state",
            )
            .map_err(|e| e.to_string())?
            .try_as_basic_value()
            .basic()
            .ok_or("thaw_promise_state returned no value")?
            .into_int_value();
        let is_rejected = self
            .builder
            .build_int_compare(
                IntPredicate::EQ,
                promise_state,
                self.context.i8_type().const_int(2, false),
                "waiting_rejected",
            )
            .map_err(|e| e.to_string())?;
        let rejected = self.context.append_basic_block(resume, "handle_rejection");
        let resume_ok = self.context.append_basic_block(resume, "resume_fulfilled");
        self.builder
            .build_conditional_branch(is_rejected, rejected, resume_ok)
            .map_err(|e| e.to_string())?;

        self.builder.position_at_end(rejected);
        let propagate_rejection = self
            .context
            .append_basic_block(resume, "propagate_rejection");
        let rejection_handlers = segments
            .iter()
            .enumerate()
            .skip(1)
            .filter_map(|(index, segment)| {
                segment.rejection_handler.as_ref().map(|handler| {
                    (
                        index,
                        handler.clone(),
                        self.context
                            .append_basic_block(resume, &format!("catch_rejection_{index}")),
                    )
                })
            })
            .collect::<Vec<_>>();
        let rejection_cases = rejection_handlers
            .iter()
            .map(|(index, _, block)| {
                (
                    self.context.i64_type().const_int(*index as u64, false),
                    *block,
                )
            })
            .collect::<Vec<_>>();
        self.builder
            .build_switch(state, propagate_rejection, &rejection_cases)
            .map_err(|e| e.to_string())?;

        for (_, handler, block) in rejection_handlers {
            self.builder.position_at_end(block);
            for name in [handler.try_guard.as_str(), handler.catch_guard.as_str()] {
                let index = plan
                    .locals
                    .iter()
                    .position(|(local, _)| local == name)
                    .ok_or_else(|| format!("missing async rejection guard `{name}`"))?;
                let slot = self.async_frame_field(
                    resume_frame,
                    self.async_locals_offset(plan) + ASYNC_SLOT_BYTES * index as u64,
                    &format!("frame_{name}"),
                )?;
                let enabled = name == handler.catch_guard;
                self.builder
                    .build_store(
                        slot,
                        self.context.bool_type().const_int(enabled as u64, false),
                    )
                    .map_err(|e| e.to_string())?;
            }
            for name in &handler.disable_guards {
                let index = plan
                    .locals
                    .iter()
                    .position(|(local, _)| local == name)
                    .ok_or_else(|| format!("missing async disabled guard `{name}`"))?;
                let slot = self.async_frame_field(
                    resume_frame,
                    self.async_locals_offset(plan) + ASYNC_SLOT_BYTES * index as u64,
                    &format!("frame_{name}"),
                )?;
                self.builder
                    .build_store(slot, self.context.bool_type().const_zero())
                    .map_err(|e| e.to_string())?;
            }
            let binding_index = plan
                .locals
                .iter()
                .position(|(local, _)| local == &handler.catch_binding)
                .ok_or_else(|| {
                    format!("missing async catch binding `{}`", handler.catch_binding)
                })?;
            let binding_slot = self.async_frame_field(
                resume_frame,
                self.async_locals_offset(plan) + ASYNC_SLOT_BYTES * binding_index as u64,
                &format!("frame_{}", handler.catch_binding),
            )?;
            self.builder
                .build_store(binding_slot, resume_result)
                .map_err(|e| e.to_string())?;
            self.builder
                .build_call(
                    self.module.get_function("thaw_promise_destroy").unwrap(),
                    &[waiting.into()],
                    "destroy_caught_waiting",
                )
                .map_err(|e| e.to_string())?;
            self.builder
                .build_store(waiting_slot, ptr_ty.const_null())
                .map_err(|e| e.to_string())?;
            self.builder
                .build_call(
                    resume,
                    &[resume_frame.into(), resume_frame.into()],
                    "resume_catch",
                )
                .map_err(|e| e.to_string())?;
            self.builder.build_return(None).map_err(|e| e.to_string())?;
        }

        self.builder.position_at_end(propagate_rejection);
        self.builder
            .build_call(
                self.module.get_function("thaw_promise_reject").unwrap(),
                &[resume_completion.into(), resume_result.into()],
                "reject_completion",
            )
            .map_err(|e| e.to_string())?;
        self.builder
            .build_call(
                self.module.get_function("thaw_promise_destroy").unwrap(),
                &[waiting.into()],
                "destroy_rejected_waiting",
            )
            .map_err(|e| e.to_string())?;
        self.builder
            .build_store(waiting_slot, ptr_ty.const_null())
            .map_err(|e| e.to_string())?;
        self.builder.build_return(None).map_err(|e| e.to_string())?;

        self.builder.position_at_end(resume_ok);
        self.builder
            .build_call(
                self.module.get_function("thaw_promise_destroy").unwrap(),
                &[waiting.into()],
                "destroy_waiting",
            )
            .map_err(|e| e.to_string())?;
        self.builder
            .build_store(waiting_slot, ptr_ty.const_null())
            .map_err(|e| e.to_string())?;
        let invalid = self.context.append_basic_block(resume, "invalid_state");
        let case_blocks = (1..segments.len())
            .map(|index| {
                self.context
                    .append_basic_block(resume, &format!("state_{index}"))
            })
            .collect::<Vec<_>>();
        let cases = case_blocks
            .iter()
            .enumerate()
            .map(|(index, block)| {
                (
                    self.context.i64_type().const_int((index + 1) as u64, false),
                    *block,
                )
            })
            .collect::<Vec<_>>();
        self.builder
            .build_switch(state, invalid, &cases)
            .map_err(|e| e.to_string())?;
        self.builder.position_at_end(invalid);
        self.builder.build_return(None).map_err(|e| e.to_string())?;

        for (index, block) in case_blocks.into_iter().enumerate() {
            self.builder.position_at_end(block);
            self.variables.clear();
            self.variable_hir_types.clear();
            self.arena_variables.clear();
            self.catch_stack.clear();
            self.seed_global_variables();
            self.bind_async_frame_locals(resume_frame, plan)?;
            self.emit_async_segment(
                &segments[index + 1],
                resume_frame,
                resume_completion,
                resume,
                index + 2,
                false,
                plan,
                Some(resume_result),
            )?;
        }
        Ok(())
    }

    // Segment emission needs the coroutine frame plus all exceptional and
    // normal successors as distinct LLVM values.
    #[allow(clippy::too_many_arguments)]
    fn emit_async_segment(
        &mut self,
        segment: &AsyncSegment,
        frame: PointerValue<'ctx>,
        completion: PointerValue<'ctx>,
        resume: FunctionValue<'ctx>,
        next_state: usize,
        ramp: bool,
        plan: &FrameAsyncPlan,
        resume_result: Option<PointerValue<'ctx>>,
    ) -> Result<(), String> {
        if let Some((name, ty)) = &segment.resume_target {
            let result_ptr = resume_result.ok_or("async resume result is unavailable")?;
            let llvm_ty = self.basic_type(ty)?;
            let value = self
                .builder
                .build_load(llvm_ty, result_ptr, &format!("awaited_{name}"))
                .map_err(|e| e.to_string())?;
            let (slot, _) = self
                .variables
                .get(name)
                .copied()
                .ok_or_else(|| format!("missing async frame slot for `{name}`"))?;
            self.builder
                .build_store(slot, value)
                .map_err(|e| e.to_string())?;
        }
        let saved_async_completion = self.active_async_completion.replace(completion);
        let block_result =
            self.compile_async_segment_block(&segment.stmts, frame, completion, plan);
        self.active_async_completion = saved_async_completion;
        match block_result? {
            AsyncBlockExit::Returned => {
                self.resolve_async_completion(
                    completion,
                    self.async_completion_result(frame, plan)?,
                    ramp,
                )?;
                return Ok(());
            }
            AsyncBlockExit::Rejected => {
                if ramp {
                    self.builder
                        .build_return(Some(&completion))
                        .map_err(|e| e.to_string())?;
                } else {
                    self.builder.build_return(None).map_err(|e| e.to_string())?;
                }
                return Ok(());
            }
            AsyncBlockExit::Continue => {}
        }
        if let Some(awaited) = &segment.awaited {
            if let Some((guard_name, expected)) = &segment.await_guard {
                let function = self.current_function();
                let schedule = self
                    .context
                    .append_basic_block(function, "await_guard_true");
                let skip = self
                    .context
                    .append_basic_block(function, "await_guard_skip");
                let (guard_slot, guard_ty) = self
                    .variables
                    .get(guard_name)
                    .copied()
                    .ok_or_else(|| format!("missing async branch guard `{guard_name}`"))?;
                let mut condition = self
                    .builder
                    .build_load(guard_ty, guard_slot, guard_name)
                    .map_err(|e| e.to_string())?
                    .into_int_value();
                if !expected {
                    condition = self
                        .builder
                        .build_not(condition, "inverse_branch_guard")
                        .map_err(|e| e.to_string())?;
                }
                self.builder
                    .build_conditional_branch(condition, schedule, skip)
                    .map_err(|e| e.to_string())?;

                self.builder.position_at_end(skip);
                let state_slot = self.async_frame_field(frame, ASYNC_STATE_OFFSET, "state_slot")?;
                self.builder
                    .build_store(
                        state_slot,
                        self.context.i64_type().const_int(next_state as u64, false),
                    )
                    .map_err(|e| e.to_string())?;
                self.builder
                    .build_call(resume, &[frame.into(), frame.into()], "skip_guarded_await")
                    .map_err(|e| e.to_string())?;
                if ramp {
                    self.builder
                        .build_return(Some(&completion))
                        .map_err(|e| e.to_string())?;
                } else {
                    self.builder.build_return(None).map_err(|e| e.to_string())?;
                }
                self.builder.position_at_end(schedule);
            }
            let waiting = match awaited {
                HirExpr::Call(callee, args) if matches!(callee.as_ref(), HirExpr::Var(name) if name == "fetch") => {
                    self.compile_single_arg_call("thaw_http_get_async", args, "async_fetch")?
                        .into_pointer_value()
                }
                _ => self.compile_expr(awaited)?.into_pointer_value(),
            };
            let waiting_slot =
                self.async_frame_field(frame, ASYNC_WAITING_OFFSET, "waiting_slot")?;
            self.builder
                .build_store(waiting_slot, waiting)
                .map_err(|e| e.to_string())?;
            let state_slot = self.async_frame_field(frame, ASYNC_STATE_OFFSET, "state_slot")?;
            let scheduled_state = segment.await_next.unwrap_or(next_state);
            self.builder
                .build_store(
                    state_slot,
                    self.context
                        .i64_type()
                        .const_int(scheduled_state as u64, false),
                )
                .map_err(|e| e.to_string())?;
            self.builder
                .build_call(
                    self.module.get_function("thaw_promise_subscribe").unwrap(),
                    &[
                        waiting.into(),
                        resume.as_global_value().as_pointer_value().into(),
                        frame.into(),
                    ],
                    "subscribe_resume",
                )
                .map_err(|e| e.to_string())?;
            if ramp {
                self.builder
                    .build_return(Some(&completion))
                    .map_err(|e| e.to_string())?;
            } else {
                self.builder.build_return(None).map_err(|e| e.to_string())?;
            }
        } else {
            if plan.ret != HirType::Void {
                if plan.returns_on_all_paths {
                    self.builder
                        .build_unreachable()
                        .map_err(|error| error.to_string())?;
                    return Ok(());
                }
                return Err("value-returning frame-split async function does not return a value on all paths".to_string());
            }
            self.resolve_async_completion(completion, frame, ramp)?;
        }
        Ok(())
    }

    fn resolve_async_completion(
        &self,
        completion: PointerValue<'ctx>,
        result: PointerValue<'ctx>,
        ramp: bool,
    ) -> Result<(), String> {
        self.builder
            .build_call(
                self.module.get_function("thaw_promise_resolve").unwrap(),
                &[completion.into(), result.into()],
                "resolve_completion",
            )
            .map_err(|e| e.to_string())?;
        if ramp {
            self.builder
                .build_return(Some(&completion))
                .map_err(|e| e.to_string())?;
        } else {
            self.builder.build_return(None).map_err(|e| e.to_string())?;
        }
        Ok(())
    }

    fn bind_async_frame_locals(
        &mut self,
        frame: PointerValue<'ctx>,
        plan: &FrameAsyncPlan,
    ) -> Result<(), String> {
        for (index, (name, ty)) in plan.locals.iter().enumerate() {
            let slot = self.async_frame_field(
                frame,
                self.async_locals_offset(plan) + ASYNC_SLOT_BYTES * index as u64,
                &format!("frame_{name}"),
            )?;
            self.async_frame_cells.insert(slot);
            self.variables
                .insert(name.clone(), (slot, self.basic_type(ty)?));
            self.variable_hir_types.insert(name.clone(), ty.clone());
        }
        Ok(())
    }

    fn compile_async_segment_block(
        &mut self,
        stmts: &[HirStmt],
        frame: PointerValue<'ctx>,
        completion: PointerValue<'ctx>,
        plan: &FrameAsyncPlan,
    ) -> Result<AsyncBlockExit, String> {
        for stmt in stmts {
            if let HirStmt::If(guard, then_body, else_body) = stmt {
                let guarded = match (then_body.as_slice(), else_body.as_slice()) {
                    ([only], []) => Some((only, true)),
                    ([], [only]) => Some((only, false)),
                    _ => None,
                };
                if let Some((HirStmt::Let(name, ty, expr), expected)) = guarded {
                    let function = self.current_function();
                    let initialize = self.context.append_basic_block(function, "guarded_let");
                    let continue_block = self.context.append_basic_block(function, "let_skipped");
                    let mut condition = self.compile_expr(guard)?.into_int_value();
                    if !expected {
                        condition = self
                            .builder
                            .build_not(condition, "inverse_let_guard")
                            .map_err(|e| e.to_string())?;
                    }
                    self.builder
                        .build_conditional_branch(condition, initialize, continue_block)
                        .map_err(|e| e.to_string())?;
                    self.builder.position_at_end(initialize);
                    let value = self.compile_expr(expr)?;
                    let (slot, slot_ty) = self
                        .variables
                        .get(name)
                        .copied()
                        .ok_or_else(|| format!("missing async frame slot for `{name}`"))?;
                    if slot_ty != self.basic_type(ty)? {
                        return Err(format!("async frame local `{name}` changed type"));
                    }
                    self.builder
                        .build_store(slot, value)
                        .map_err(|e| e.to_string())?;
                    self.builder
                        .build_unconditional_branch(continue_block)
                        .map_err(|e| e.to_string())?;
                    self.builder.position_at_end(continue_block);
                    continue;
                }
                if let Some((HirStmt::Return(value), expected)) = guarded {
                    let function = self.current_function();
                    let return_block = self.context.append_basic_block(function, "guarded_return");
                    let continue_block =
                        self.context.append_basic_block(function, "return_skipped");
                    let mut condition = self.compile_expr(guard)?.into_int_value();
                    if !expected {
                        condition = self
                            .builder
                            .build_not(condition, "inverse_return_guard")
                            .map_err(|e| e.to_string())?;
                    }
                    self.builder
                        .build_conditional_branch(condition, return_block, continue_block)
                        .map_err(|e| e.to_string())?;
                    self.builder.position_at_end(return_block);
                    match value {
                        Some(HirExpr::ThrowValue(error, _)) => {
                            let error = self.compile_expr(error)?.into_pointer_value();
                            self.builder
                                .build_call(
                                    self.module.get_function("thaw_promise_reject").unwrap(),
                                    &[completion.into(), error.into()],
                                    "reject_throw_value",
                                )
                                .map_err(|error| error.to_string())?;
                            if function.get_type().get_return_type().is_some() {
                                self.builder
                                    .build_return(Some(&completion))
                                    .map_err(|error| error.to_string())?;
                            } else {
                                self.builder
                                    .build_return(None)
                                    .map_err(|error| error.to_string())?;
                            }
                            self.builder.position_at_end(continue_block);
                            continue;
                        }
                        Some(expr) if plan.ret != HirType::Void => {
                            let result = self.compile_expr(expr)?;
                            let result_slot =
                                self.async_frame_field(frame, ASYNC_RESULT_OFFSET, "async_result")?;
                            self.builder
                                .build_store(result_slot, result)
                                .map_err(|e| e.to_string())?;
                        }
                        None if plan.ret == HirType::Void => {}
                        Some(_) => {
                            return Err(
                                "void frame-split async function cannot return a value".to_string()
                            )
                        }
                        None => {
                            return Err(
                                "value-returning frame-split async function cannot use `return;`"
                                    .to_string(),
                            )
                        }
                    }
                    self.resolve_async_completion(
                        completion,
                        self.async_completion_result(frame, plan)?,
                        function.get_type().get_return_type().is_some(),
                    )?;
                    self.builder.position_at_end(continue_block);
                    continue;
                }
                if let Some((HirStmt::Throw(error), expected)) = guarded {
                    let enclosing_handler = match guard {
                        HirExpr::Var(name) => plan.guarded_rethrow_handlers.get(name).cloned(),
                        _ => None,
                    };
                    let function = self.current_function();
                    let reject = self.context.append_basic_block(function, "guarded_rethrow");
                    let continue_block =
                        self.context.append_basic_block(function, "rethrow_skipped");
                    let mut condition = self.compile_expr(guard)?.into_int_value();
                    if !expected {
                        condition = self
                            .builder
                            .build_not(condition, "inverse_throw_guard")
                            .map_err(|e| e.to_string())?;
                    }
                    self.builder
                        .build_conditional_branch(condition, reject, continue_block)
                        .map_err(|e| e.to_string())?;
                    self.builder.position_at_end(reject);
                    let error = self.compile_expr(error)?.into_pointer_value();
                    if let Some(handler) = enclosing_handler {
                        let catch_index = plan
                            .locals
                            .iter()
                            .position(|(name, _)| name == &handler.catch_binding)
                            .ok_or_else(|| {
                                format!("missing async catch binding `{}`", handler.catch_binding)
                            })?;
                        let catch_slot = self.async_frame_field(
                            frame,
                            self.async_locals_offset(plan) + ASYNC_SLOT_BYTES * catch_index as u64,
                            "outer_catch_binding",
                        )?;
                        self.builder
                            .build_store(catch_slot, error)
                            .map_err(|e| e.to_string())?;
                        for (guard_name, value) in
                            [(&handler.try_guard, false), (&handler.catch_guard, true)]
                                .into_iter()
                                .chain(handler.disable_guards.iter().map(|name| (name, false)))
                        {
                            let guard_index = plan
                                .locals
                                .iter()
                                .position(|(name, _)| name == guard_name)
                                .ok_or_else(|| format!("missing async guard `{guard_name}`"))?;
                            let guard_slot = self.async_frame_field(
                                frame,
                                self.async_locals_offset(plan)
                                    + ASYNC_SLOT_BYTES * guard_index as u64,
                                "outer_catch_guard",
                            )?;
                            self.builder
                                .build_store(
                                    guard_slot,
                                    self.context.bool_type().const_int(value as u64, false),
                                )
                                .map_err(|e| e.to_string())?;
                        }
                        self.builder
                            .build_unconditional_branch(continue_block)
                            .map_err(|e| e.to_string())?;
                    } else {
                        self.builder
                            .build_call(
                                self.module.get_function("thaw_promise_reject").unwrap(),
                                &[completion.into(), error.into()],
                                "rethrow_rejection",
                            )
                            .map_err(|e| e.to_string())?;
                        if function.get_type().get_return_type().is_some() {
                            self.builder
                                .build_return(Some(&completion))
                                .map_err(|e| e.to_string())?;
                        } else {
                            self.builder.build_return(None).map_err(|e| e.to_string())?;
                        }
                    }
                    self.builder.position_at_end(continue_block);
                    continue;
                }
            }
            if matches!(stmt, HirStmt::Return(None)) {
                if plan.ret != HirType::Void {
                    return Err(
                        "value-returning frame-split async function cannot use `return;`"
                            .to_string(),
                    );
                }
                return Ok(AsyncBlockExit::Returned);
            }
            if let HirStmt::Return(Some(HirExpr::ThrowValue(error, _))) = stmt {
                let error = self.compile_expr(error)?.into_pointer_value();
                self.builder
                    .build_call(
                        self.module.get_function("thaw_promise_reject").unwrap(),
                        &[completion.into(), error.into()],
                        "reject_throw_value",
                    )
                    .map_err(|error| error.to_string())?;
                return Ok(AsyncBlockExit::Rejected);
            }
            if let HirStmt::Return(Some(expr)) = stmt {
                if plan.ret == HirType::Void {
                    return Err("void frame-split async function cannot return a value".to_string());
                }
                let value = self.compile_expr(expr)?;
                let result_slot =
                    self.async_frame_field(frame, ASYNC_RESULT_OFFSET, "async_result")?;
                self.builder
                    .build_store(result_slot, value)
                    .map_err(|e| e.to_string())?;
                return Ok(AsyncBlockExit::Returned);
            }
            if let HirStmt::Throw(expr) = stmt {
                let error = self.compile_expr(expr)?.into_pointer_value();
                self.builder
                    .build_call(
                        self.module.get_function("thaw_promise_reject").unwrap(),
                        &[completion.into(), error.into()],
                        "reject_completion",
                    )
                    .map_err(|e| e.to_string())?;
                return Ok(AsyncBlockExit::Rejected);
            }
            if let HirStmt::Let(name, ty, expr) = stmt {
                let value = self.compile_expr(expr)?;
                let index = plan
                    .locals
                    .iter()
                    .position(|(local, _)| local == name)
                    .ok_or_else(|| format!("missing async frame slot for `{name}`"))?;
                let slot = self.async_frame_field(
                    frame,
                    self.async_locals_offset(plan) + ASYNC_SLOT_BYTES * index as u64,
                    &format!("frame_{name}"),
                )?;
                self.async_frame_cells.insert(slot);
                self.builder
                    .build_store(slot, value)
                    .map_err(|e| e.to_string())?;
                self.variables
                    .insert(name.clone(), (slot, self.basic_type(ty)?));
            } else if self.compile_stmt(stmt)? {
                return Ok(AsyncBlockExit::Returned);
            }
        }
        Ok(AsyncBlockExit::Continue)
    }

    fn async_locals_offset(&self, plan: &FrameAsyncPlan) -> u64 {
        ASYNC_FRAME_BYTES
            + if plan.ret == HirType::Void {
                0
            } else {
                ASYNC_SLOT_BYTES
            }
    }

    fn async_frame_size(&self, plan: &FrameAsyncPlan) -> u64 {
        self.async_locals_offset(plan) + ASYNC_SLOT_BYTES * plan.locals.len() as u64
    }

    fn async_completion_result(
        &self,
        frame: PointerValue<'ctx>,
        plan: &FrameAsyncPlan,
    ) -> Result<PointerValue<'ctx>, String> {
        if plan.ret == HirType::Void {
            Ok(frame)
        } else {
            self.async_frame_field(frame, ASYNC_RESULT_OFFSET, "async_result")
        }
    }

}
