impl<'ctx> HirCompiler<'ctx> {
    fn prepromote_branch_captures(
        &mut self,
        condition: &HirExpr,
        branches: &[HirStmt],
    ) -> Result<(), String> {
        // Allocation and the initial copy must both dominate dispatch.
        for name in thaw_hir::closure_captured_names_in_while(condition, branches) {
            if let Some(ty) = self.variable_hir_types.get(&name).cloned() {
                self.prepromote_variable_to_arena_cell(&name, &ty)?;
            }
        }
        Ok(())
    }

    fn compile_conditional_value(
        &mut self,
        test: &HirExpr,
        consequent: &HirExpr,
        alternate: &HirExpr,
        ty: &HirType,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        self.prepromote_branch_captures(test, &[
            HirStmt::Expr(consequent.clone()),
            HirStmt::Expr(alternate.clone()),
        ])?;
        let function = self.current_function();
        let consequent_block = self.context.append_basic_block(function, "conditional_then");
        let alternate_block = self.context.append_basic_block(function, "conditional_else");
        let merge_block = self.context.append_basic_block(function, "conditional_end");
        let condition = self.compile_expr(test)?.into_int_value();
        self.builder
            .build_conditional_branch(condition, consequent_block, alternate_block)
            .map_err(|error| error.to_string())?;

        self.builder.position_at_end(consequent_block);
        let consequent = self.compile_expr(consequent)?;
        let consequent_end = self.builder.get_insert_block().unwrap();
        self.builder
            .build_unconditional_branch(merge_block)
            .map_err(|error| error.to_string())?;

        self.builder.position_at_end(alternate_block);
        let alternate = self.compile_expr(alternate)?;
        let alternate_end = self.builder.get_insert_block().unwrap();
        self.builder
            .build_unconditional_branch(merge_block)
            .map_err(|error| error.to_string())?;

        self.builder.position_at_end(merge_block);
        let phi = self
            .builder
            .build_phi(self.basic_type(ty)?, "conditional_value")
            .map_err(|error| error.to_string())?;
        phi.add_incoming(&[
            (&consequent, consequent_end),
            (&alternate, alternate_end),
        ]);
        Ok(phi.as_basic_value())
    }

    /// Compiles a statement list, stopping early if one of them
    /// unconditionally leaves the block (`return`/`throw`, or an `if` whose
    /// branches all do). Returns `true` when that happened, so callers know
    /// not to fall through past this block.
    fn compile_block(&mut self, stmts: &[HirStmt]) -> Result<bool, String> {
        for stmt in stmts {
            if self.compile_stmt(stmt)? {
                return Ok(true);
            }
        }
        Ok(false)
    }

    fn compile_throw_text(&mut self, expr: &HirExpr) -> Result<PointerValue<'ctx>, String> {
        // This marker is created by source lowering only after conversion to
        // native text. Its name cannot occur in a TypeScript identifier.
        let trusted_text = if let HirExpr::Call(callee, args) = expr {
            if matches!(callee.as_ref(), HirExpr::Var(name) if name == "@@thaw_trusted_exception_text")
                && args.len() == 1
            {
                Some(&args[0])
            } else {
                None
            }
        } else {
            None
        };
        let pending_rethrow = if let HirExpr::Call(callee, args) = expr {
            if matches!(callee.as_ref(), HirExpr::Var(name) if name == "@@thaw_rethrow_pending_exception")
                && args.len() == 9 { Some(args.as_slice()) } else { None }
        } else { None };
        // The exit injector has already evaluated the visible catch binding.
        // Keep its original binding only as provenance for the caught tuple;
        // evaluating it again after `finally` would observe later mutation.
        let snapshot_caught = if let HirExpr::Call(callee, args) = expr {
            if matches!(callee.as_ref(), HirExpr::Var(name) if name == "@@thaw_snapshot_caught_exception")
                && matches!(args.as_slice(), [_, HirExpr::Lit(HirLit::Str(_))]) {
                Some(args.as_slice())
            } else { None }
        } else { None };
        let val = self.compile_expr(pending_rethrow.map(|args| &args[0])
            .or(trusted_text)
            .or_else(|| snapshot_caught.map(|args| &args[0]))
            .unwrap_or(expr))?.into_pointer_value();
        if let Some(args) = pending_rethrow {
            // Snapshot fields belong to this callback parameter, so a handled
            // inner exception cannot replace the original rejection metadata.
            let ptr = self.context.ptr_type(AddressSpace::default());
            let original = self.compile_expr(&args[1])?.into_pointer_value();
            let value_word = self.builder.build_ptr_to_int(val, self.context.i64_type(),
                "rethrow_value_word").map_err(|error| error.to_string())?;
            let original_word = self.builder.build_ptr_to_int(original, self.context.i64_type(),
                "rethrow_original_word").map_err(|error| error.to_string())?;
            let same = self.builder.build_int_compare(inkwell::IntPredicate::EQ,
                value_word, original_word, "same_rejection_binding")
                .map_err(|error| error.to_string())?;
            let native = self.compile_expr(&args[2])?.into_pointer_value();
            let aggregate = self.compile_expr(&args[3])?.into_pointer_value();
            let tag = self.compile_expr(&args[4])?.into_int_value();
            let f64_value = self.compile_expr(&args[5])?.into_float_value();
            let i64_value = self.compile_expr(&args[6])?.into_int_value();
            let bool_value = self.compile_expr(&args[7])?.into_int_value();
            let object = self.compile_expr(&args[8])?.into_pointer_value();
            self.clear_pending_native_text()?;
            for (slot, value) in [
                (self.pending_exception_native_text().as_pointer_value(),
                    self.builder.build_select(same, native, ptr.const_null(), "rethrow_native_text")
                        .map_err(|error| error.to_string())?),
                (self.pending_exception_aggregate_errors().as_pointer_value(),
                    self.builder.build_select(same, aggregate, ptr.const_null(), "rethrow_aggregate")
                        .map_err(|error| error.to_string())?),
                (self.pending_exception_object().as_pointer_value(),
                    self.builder.build_select(same, object, ptr.const_null(), "rethrow_object")
                        .map_err(|error| error.to_string())?),
            ] {
                self.builder.build_store(slot, value).map_err(|error| error.to_string())?;
            }
            let tag = self.builder.build_select(same, tag, self.context.i64_type().const_int(4, false),
                "rethrow_tag").map_err(|error| error.to_string())?;
            self.builder.build_store(self.pending_exception_value(PENDING_EXCEPTION_VALUE_TAG_SYMBOL).as_pointer_value(), tag)
                .map_err(|error| error.to_string())?;
            self.builder.build_store(self.pending_exception_value(PENDING_EXCEPTION_F64_SYMBOL).as_pointer_value(), f64_value)
                .map_err(|error| error.to_string())?;
            self.builder.build_store(self.pending_exception_value(PENDING_EXCEPTION_I64_SYMBOL).as_pointer_value(), i64_value)
                .map_err(|error| error.to_string())?;
            self.builder.build_store(self.pending_exception_value(PENDING_EXCEPTION_BOOL_SYMBOL).as_pointer_value(), bool_value)
                .map_err(|error| error.to_string())?;
            return Ok(val);
        }
        let snapshot_origin = snapshot_caught.and_then(|args| {
            if let HirExpr::Lit(HirLit::Str(name)) = &args[1] {
                Some(HirExpr::Var(name.clone()))
            } else { None }
        });
        let provenance_expr = snapshot_origin.as_ref().unwrap_or(expr);
        if self.restore_caught_exception_tuple(val, provenance_expr)? {
            return Ok(val);
        }
        self.clear_pending_native_text()?;
        if trusted_text.is_some() || matches!(expr, HirExpr::Lit(HirLit::Str(_) | HirLit::Wtf8(_))) {
            self.mark_pending_native_text(val)?;
        } else if let HirExpr::Var(name) = provenance_expr {
            if let Some((catch_slot, native_slot, _, _)) = self.catch_native_text.get(name) {
                if self.variables.get(name).map(|(slot, _)| slot) == Some(catch_slot) {
                    let provenance = self.builder.build_load(
                        self.context.ptr_type(AddressSpace::default()), *native_slot,
                        "rethrown_native_text_provenance",
                    ).map_err(|error| error.to_string())?;
                    self.mark_pending_native_text(provenance)?;
                    if let Some((aggregate_slot, _)) = self.variables.get(&format!("{name}__thaw_exception_aggregate")) {
                        let aggregate = self.builder.build_load(
                            self.context.ptr_type(AddressSpace::default()), *aggregate_slot,
                            "rethrown_aggregate_errors",
                        ).map_err(|error| error.to_string())?;
                        self.builder.build_store(self.pending_exception_aggregate_errors().as_pointer_value(), aggregate)
                            .map_err(|error| error.to_string())?;
                    }
                }
            }
        }
        Ok(val)
    }

    fn compile_stmt(&mut self, stmt: &HirStmt) -> Result<bool, String> {
        match stmt {
            HirStmt::Expr(expr) => {
                if let HirExpr::FfiCall(sig, args) = expr {
                    if sig.ret == HirType::Void {
                        self.compile_ffi_call(sig, args)?;
                        return Ok(false);
                    }
                }
                let value = self.compile_expr(expr)?;
                if matches!(expr, HirExpr::Call(..) | HirExpr::DynamicCall(..))
                    && self.expr_hir_type(expr) == Some(HirType::JsValue)
                {
                    self.builder
                        .build_call(
                            self.module.get_function("thaw_js_release_handle").unwrap(),
                            &[value.into()],
                            "release_discarded_dynamic_value",
                        )
                        .map_err(|error| error.to_string())?;
                }
                Ok(false)
            }

            HirStmt::Return(value) => {
                match value {
                    Some(expr) => {
                        let val = self.compile_expr(expr)?;
                        self.builder
                            .build_return(Some(&val))
                            .map_err(|e| e.to_string())?;
                    }
                    None => {
                        self.builder.build_return(None).map_err(|e| e.to_string())?;
                    }
                }
                Ok(true)
            }

            HirStmt::Let(name, ty, expr) => {
                let val = self.compile_expr(expr)?;
                if self.for_iteration_frame_slots.contains_key(name) {
                    self.store_for_iteration_cell(name, ty, val)?;
                    return Ok(false);
                }
                let llvm_ty = self.basic_type(ty)?;
                // A `Let` for a name that's already bound to an async-frame
                // slot (`bind_async_frame_locals` sets this up before a
                // segment's own statements run -- see `async_frames/
                // codegen.rs`) must store into that *same* slot rather than
                // allocating a fresh one: this exact statement is the
                // declaration inside a loop's body, re-executed on every
                // iteration/resume, and the whole point of frame-backing it
                // is for its storage to survive the suspend/resume boundary
                // between iterations. Allocating a new ordinary stack cell
                // here instead (as this code used to do unconditionally)
                // silently rebinds `self.variables[name]` away from the
                // frame slot to one hoisted into the *current* physical
                // function's entry block -- valid only for that one
                // invocation of `resume`, not across the separate
                // invocation the next loop iteration's resume causes. Any
                // closure that captured `name` between this point and the
                // next iteration (real example: the closure `&&`/`||`
                // desugars into, `wrap_call_argument_bindings`) ends up
                // holding a dangling pointer into a stack frame that's
                // already gone, reading garbage on the next iteration --
                // silently wrong values, or a crash, depending on what
                // happens to occupy that stack slot afterward.
                let existing_frame_cell = self
                    .variables
                    .get(name)
                    .filter(|(cell, _)| self.async_frame_cells.contains(cell))
                    .map(|(cell, _)| *cell);
                let slot = match existing_frame_cell {
                    Some(cell) => cell,
                    None => self.allocate_variable_cell(llvm_ty, name)?,
                };
                if existing_frame_cell.is_some() {
                    self.retain_native_promise_cell_value(slot, ty, val)?;
                }
                self.builder
                    .build_store(slot, val)
                    .map_err(|e| e.to_string())?;
                self.variables.insert(name.clone(), (slot, llvm_ty));
                self.variable_hir_types.insert(name.clone(), ty.clone());
                Ok(false)
            }

            HirStmt::If(cond, then_branch, else_branch) => {
                self.compile_if(cond, then_branch, else_branch)
            }

            HirStmt::While(cond, body) => self.compile_while(cond, body),

            HirStmt::Break => {
                let (_, break_target) = self
                    .loop_stack
                    .last()
                    .copied()
                    .ok_or("`break` used outside a loop")?;
                self.builder
                    .build_unconditional_branch(break_target)
                    .map_err(|e| e.to_string())?;
                Ok(true)
            }

            HirStmt::BreakDepth(depth) => {
                let index = self
                    .loop_stack
                    .len()
                    .checked_sub(depth + 1)
                    .ok_or("labeled `break` target is outside the active loop stack")?;
                let (_, break_target) = self.loop_stack[index];
                self.builder
                    .build_unconditional_branch(break_target)
                    .map_err(|e| e.to_string())?;
                Ok(true)
            }

            HirStmt::Continue => {
                let (continue_target, _) = self
                    .loop_stack
                    .last()
                    .copied()
                    .ok_or("`continue` used outside a loop")?;
                self.builder
                    .build_unconditional_branch(continue_target)
                    .map_err(|e| e.to_string())?;
                Ok(true)
            }

            HirStmt::ContinueDepth(depth) => {
                let index = self
                    .loop_stack
                    .len()
                    .checked_sub(depth + 1)
                    .ok_or("labeled `continue` target is outside the active loop stack")?;
                let (continue_target, _) = self.loop_stack[index];
                self.builder
                    .build_unconditional_branch(continue_target)
                    .map_err(|e| e.to_string())?;
                Ok(true)
            }

            HirStmt::Throw(expr) => {
                let val = self.compile_throw_text(expr)?;
                self.builder
                    .build_store(self.pending_exception().as_pointer_value(), val)
                    .map_err(|e| e.to_string())?;
                if let Some(catch_bb) = self.catch_stack.last().copied() {
                    self.builder
                        .build_unconditional_branch(catch_bb)
                        .map_err(|e| e.to_string())?;
                } else {
                    self.build_default_return()?;
                }
                Ok(true)
            }

            HirStmt::Try(body, catch_name, catch_body, hidden_tag) => {
                self.compile_try(body, catch_name, catch_body, hidden_tag.as_deref())
            }
            HirStmt::Finally(body, suspended) => {
                if *suspended > self.catch_stack.len() {
                    return Err("finally suspends more lexical catches than are active".into());
                }
                let at = self.catch_stack.len() - suspended;
                let handlers = self.catch_stack.split_off(at);
                let result = self.compile_block(body);
                // Nested compilation may return an error before its own pop.
                // Restore the lexical stack exactly on both paths.
                self.catch_stack.truncate(at);
                self.catch_stack.extend(handlers);
                result
            }
        }
    }

    fn compile_if(
        &mut self,
        cond: &HirExpr,
        then_branch: &[HirStmt],
        else_branch: &[HirStmt],
    ) -> Result<bool, String> {
        let mut branches = then_branch.to_vec();
        branches.extend_from_slice(else_branch);
        self.prepromote_branch_captures(cond, &branches)?;
        let function = self.current_function();
        let cond_val = self.compile_expr(cond)?.into_int_value();

        let then_bb = self.context.append_basic_block(function, "then");
        let else_bb = self.context.append_basic_block(function, "else");
        let merge_bb = self.context.append_basic_block(function, "ifcont");

        self.builder
            .build_conditional_branch(cond_val, then_bb, else_bb)
            .map_err(|e| e.to_string())?;

        let variables_before_branches = self.variables.clone();
        let native_text_before_branches = self.catch_native_text.clone();
        let arena_variables_before_branches = self.arena_variables.clone();
        self.builder.position_at_end(then_bb);
        let then_terminated = self.compile_block(then_branch)?;
        let then_variables = self.variables.clone();
        let then_native_text = self.catch_native_text.clone();
        let then_arena_variables = self.arena_variables.clone();
        if !then_terminated {
            self.builder
                .build_unconditional_branch(merge_bb)
                .map_err(|e| e.to_string())?;
        } else {
            self.variables = variables_before_branches.clone();
            self.catch_native_text = native_text_before_branches.clone();
            self.arena_variables = arena_variables_before_branches.clone();
        }

        self.builder.position_at_end(else_bb);
        let else_terminated = self.compile_block(else_branch)?;
        if !else_terminated {
            self.builder
                .build_unconditional_branch(merge_bb)
                .map_err(|e| e.to_string())?;
        } else if !then_terminated {
            self.variables = then_variables;
            self.catch_native_text = then_native_text;
            self.arena_variables = then_arena_variables;
        }

        self.builder.position_at_end(merge_bb);
        if then_terminated && else_terminated {
            // merge_bb has no predecessors; give it a terminator so the
            // module stays valid IR, then report that this whole
            // if-statement terminates its enclosing block.
            self.builder
                .build_unreachable()
                .map_err(|e| e.to_string())?;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    fn compile_while(&mut self, cond: &HirExpr, body: &[HirStmt]) -> Result<bool, String> {
        let function = self.current_function();

        let header_bb = self.context.append_basic_block(function, "whilecond");
        let body_bb = self.context.append_basic_block(function, "whilebody");
        let after_bb = self.context.append_basic_block(function, "whileend");

        // Eagerly promote every pre-loop variable EITHER side's own
        // closure would otherwise reactively promote on first capture --
        // done here, before either side is compiled and before this
        // loop's own `loop_promotion_scopes` entry is pushed, so
        // `promote_variable_to_arena_cell`'s existing logic (no outer
        // loop scope -> `allocate_arena_cell`, the function entry block;
        // an outer loop scope active -> `build_arena_cell` at the
        // current builder position, still `preheader_bb` here, before
        // its terminator) places the cell correctly regardless of
        // nesting. Without this, whichever side's own closure compiles
        // first (currently always the body) reactively promotes the
        // variable, leaving the *other* side -- compiled from a
        // `variables_before_body` snapshot taken before that promotion
        // -- reading a stale, un-promoted cell (round24's `.test()`/
        // `.lastIndex` staleness bug). See `closure_captured_names_in_
        // while`'s own doc comment for why simply reordering cond/body
        // compilation instead (tried in round38, reverted) isn't safe:
        // it just moves the identical bug to the mirror-image case
        // (`for...of` over a sparse array desugars to this same `while`
        // shape, with its own index-normalization closure promoting from
        // the body side).
        for name in thaw_hir::closure_captured_names_in_while(cond, body) {
            let Some(hir_ty) = self.variable_hir_types.get(&name).cloned() else {
                continue;
            };
            self.prepromote_variable_to_arena_cell(&name, &hir_ty)?;
        }

        // A checked native Promise capture can split the incoming block
        // into retain-failed and retain-ready successors. Later loop
        // promotions must use the actual successor that branches to the
        // header, not the predecessor of that ownership check.
        let preheader_bb = self.builder.get_insert_block().unwrap();

        self.builder
            .build_unconditional_branch(header_bb)
            .map_err(|e| e.to_string())?;

        let variables_before_body = self.variables.clone();
        let native_text_before_body = self.catch_native_text.clone();
        let variable_types_before_body = self.variable_hir_types.clone();
        let arena_variables_before_body = self.arena_variables.clone();
        self.builder.position_at_end(body_bb);
        self.loop_stack.push((header_bb, after_bb));
        self.loop_promotion_scopes.push((
            preheader_bb,
            variables_before_body.keys().cloned().collect(),
        ));
        let body_terminated = self.compile_block(body)?;
        self.loop_stack.pop();
        if !body_terminated {
            self.builder
                .build_unconditional_branch(header_bb)
                .map_err(|e| e.to_string())?;
        }

        let body_variables = std::mem::replace(&mut self.variables, variables_before_body.clone());
        self.catch_native_text = native_text_before_body;
        let body_arena_variables =
            std::mem::replace(&mut self.arena_variables, arena_variables_before_body);
        self.variable_hir_types = variable_types_before_body;
        for name in variables_before_body.keys() {
            if body_arena_variables.contains(name) {
                if let Some(variable) = body_variables.get(name) {
                    self.variables.insert(name.clone(), *variable);
                    self.arena_variables.insert(name.clone());
                }
            }
        }

        self.builder.position_at_end(header_bb);
        // The condition block, like the body, is re-entered on every
        // iteration via the backward branch below -- so a closure built
        // while compiling `cond` that captures a pre-loop variable needs
        // the same loop-promotion treatment as one built in the body
        // (`loop_promotion_scopes` must still be active here). Otherwise
        // a mutating capture (e.g. `any`-typed RegExp `.exec()`'s
        // `lastIndex` write-back) gets re-snapshotted from the
        // still-stale original variable at the top of every iteration,
        // discarding the previous iteration's write before it's ever
        // read back.
        let cond_val = self.compile_expr(cond)?.into_int_value();
        self.loop_promotion_scopes.pop();
        self.builder
            .build_conditional_branch(cond_val, body_bb, after_bb)
            .map_err(|e| e.to_string())?;

        self.builder.position_at_end(after_bb);
        // `after_bb` is always reachable (the condition can be false on the
        // first check), so a `while` never terminates its enclosing block.
        Ok(false)
    }

    /// Pack the original exception tuple into the ordinary catch value.
    /// Used both by lexical try/catch and by split async catch frames.
    fn build_caught_exception_carrier(
        &mut self,
        text: PointerValue<'ctx>,
        owner: PointerValue<'ctx>,
        exception_tag: IntValue<'ctx>,
        number: FloatValue<'ctx>,
        bigint: IntValue<'ctx>,
        boolean: IntValue<'ctx>,
        share_json: bool,
    ) -> Result<StructValue<'ctx>, String> {
        let i64_ty = self.context.i64_type();
        let i8_ty = self.context.i8_type();
        let number_bits = self.builder.build_bit_cast(number, i64_ty, "caught_number_bits")
            .map_err(|e| e.to_string())?.into_int_value();
        let bool_bits = self.builder.build_int_z_extend(boolean, i64_ty, "caught_bool_bits")
            .map_err(|e| e.to_string())?;
        let text_bits = self.builder.build_ptr_to_int(text, i64_ty, "caught_text_bits")
            .map_err(|e| e.to_string())?;
        let owner_bits = self.builder.build_ptr_to_int(owner, i64_ty, "caught_owner_bits")
            .map_err(|e| e.to_string())?;
        // Lexical catches may receive a borrowed JSON Box and must share it.
        // A split async catch uses the runtime pending-transfer API before its
        // source Promise dies; that Box is already independently arena-rooted.
        let selected_owner = if share_json {
            let is_json = self.builder.build_int_compare(inkwell::IntPredicate::EQ,
                exception_tag, i64_ty.const_int(7, false), "caught_is_json")
                .map_err(|e| e.to_string())?;
            let function = self.current_function();
            let share_block = self.context.append_basic_block(function, "caught_share_json");
            let passthrough_block = self.context.append_basic_block(function, "caught_keep_native_owner");
            let join_block = self.context.append_basic_block(function, "caught_owner_ready");
            self.builder.build_conditional_branch(is_json, share_block, passthrough_block)
                .map_err(|e| e.to_string())?;
            self.builder.position_at_end(share_block);
            let shared = self.builder.build_call(
                self.module.get_function("thaw_json_share").unwrap(),
                &[owner.into()], "caught_json_share",
            ).map_err(|e| e.to_string())?.try_as_basic_value().basic()
                .ok_or("JSON exception share returned no value")?;
            let rooted = self.builder.build_call(
                self.module.get_function("thaw_json_track_arena_owned_root").unwrap(),
                &[shared.into()], "caught_json_arena_root",
            ).map_err(|e| e.to_string())?.try_as_basic_value().basic()
                .ok_or("JSON exception root returned no value")?;
            // track_arena_owned_root destroys its input on failure itself.
            let rooted = self.compile_check_json_host_error(rooted, None)?.into_pointer_value();
            let share_end = self.builder.get_insert_block().ok_or("missing JSON share block")?;
            self.builder.build_unconditional_branch(join_block).map_err(|e| e.to_string())?;
            self.builder.position_at_end(passthrough_block);
            self.builder.build_unconditional_branch(join_block).map_err(|e| e.to_string())?;
            self.builder.position_at_end(join_block);
            let selected = self.builder.build_phi(self.context.ptr_type(AddressSpace::default()),
                "caught_owned_json_or_native").map_err(|e| e.to_string())?;
            selected.add_incoming(&[(&rooted, share_end), (&owner, passthrough_block)]);
            selected.as_basic_value().into_pointer_value()
        } else {
            owner
        };
        let json_bits = self.builder.build_ptr_to_int(selected_owner,
            i64_ty, "caught_json_bits").map_err(|e| e.to_string())?;
        // Exception tags 1..8 map to union members 0..5,7,8.
        // Tag 0 with an owner pointer is the native Object member (6);
        // legacy text-only tag 0 remains a String member (3).
        let owner_present = self.builder.build_is_not_null(owner, "caught_owner_present")
            .map_err(|e| e.to_string())?;
        let mut union_tag = self.builder.build_select(owner_present,
            i8_ty.const_int(6, false), i8_ty.const_int(3, false), "caught_default_tag")
            .map_err(|e| e.to_string())?.into_int_value();
        let mut payload = self.builder.build_select(owner_present, owner_bits,
            text_bits, "caught_default_payload")
            .map_err(|e| e.to_string())?.into_int_value();
        for (exception_kind, union_kind, bits) in [
            (1u64, 0u64, number_bits),
            (2, 1, bigint),
            (3, 2, bool_bits),
            (4, 3, text_bits),
            (5, 4, i64_ty.const_zero()),
            (6, 5, i64_ty.const_zero()),
            (7, 7, json_bits),
            (8, 8, bigint),
        ] {
            let active = self.builder.build_int_compare(inkwell::IntPredicate::EQ,
                exception_tag, i64_ty.const_int(exception_kind, false), "caught_kind")
                .map_err(|e| e.to_string())?;
            union_tag = self.builder.build_select(active, i8_ty.const_int(union_kind, false),
                union_tag, "select_caught_tag").map_err(|e| e.to_string())?.into_int_value();
            payload = self.builder.build_select(active, bits, payload, "select_caught_payload")
                .map_err(|e| e.to_string())?.into_int_value();
        }
        let union_ty = self.basic_type(&thaw_hir::caught_exception_carrier_type())?.into_struct_type();
        let tagged = self.builder.build_insert_value(union_ty.get_undef(), union_tag,
            0, "caught_union_tag").map_err(|e| e.to_string())?.into_struct_value();
        self.builder.build_insert_value(tagged, payload,
            1, "caught_union_payload").map(|value| value.into_struct_value())
            .map_err(|e| e.to_string())
    }

    fn compile_try(
        &mut self,
        body: &[HirStmt],
        catch_name: &str,
        catch_body: &[HirStmt],
        hidden_tag: Option<&str>,
    ) -> Result<bool, String> {
        let function = self.current_function();

        let try_bb = self.context.append_basic_block(function, "try");
        let catch_bb = self.context.append_basic_block(function, "catch");
        let merge_bb = self.context.append_basic_block(function, "trycont");

        let ptr_ty = self.context.ptr_type(AddressSpace::default());
        let str_ty: BasicTypeEnum = ptr_ty.into();
        let catch_slot = self
            .builder
            .build_alloca(str_ty, "catch_slot")
            .map_err(|e| e.to_string())?;

        self.builder
            .build_unconditional_branch(try_bb)
            .map_err(|e| e.to_string())?;

        self.builder.position_at_end(try_bb);
        self.catch_stack.push(catch_bb);
        let try_terminated = self.compile_block(body)?;
        self.catch_stack.pop();
        if !try_terminated {
            self.builder
                .build_unconditional_branch(merge_bb)
                .map_err(|e| e.to_string())?;
        }

        self.builder.position_at_end(catch_bb);
        let thrown = self
            .builder
            .build_load(
                ptr_ty,
                self.pending_exception().as_pointer_value(),
                "caught_exception",
            )
            .map_err(|e| e.to_string())?;
        self.builder
            .build_store(catch_slot, thrown)
            .map_err(|e| e.to_string())?;
        let valid_slot = self.builder.build_alloca(self.context.bool_type(), "catch_original_valid_slot")
            .map_err(|e| e.to_string())?;
        self.builder.build_store(valid_slot, self.context.bool_type().const_int(1, false))
            .map_err(|e| e.to_string())?;
        let original_slot = self.builder.build_alloca(str_ty, "catch_original_value_slot")
            .map_err(|e| e.to_string())?;
        self.builder.build_store(original_slot, thrown).map_err(|e| e.to_string())?;
        let native_text_slot = self.builder.build_alloca(str_ty, "catch_native_text_slot")
            .map_err(|e| e.to_string())?;
        let native_text = self.builder.build_load(
            ptr_ty, self.pending_exception_native_text().as_pointer_value(),
            "caught_native_text_provenance",
        ).map_err(|e| e.to_string())?;
        self.builder.build_store(native_text_slot, native_text)
            .map_err(|e| e.to_string())?;
        let aggregate = self.builder.build_load(
            ptr_ty, self.pending_exception_aggregate_errors().as_pointer_value(),
            "caught_aggregate_errors",
        ).map_err(|e| e.to_string())?;
        let previous_native_text = self.catch_native_text.insert(
            catch_name.to_string(), (catch_slot, native_text_slot, original_slot, valid_slot),
        );
        self.builder
            .build_store(
                self.pending_exception().as_pointer_value(),
                ptr_ty.const_null(),
            )
            .map_err(|e| e.to_string())?;
        self.clear_pending_native_text()?;
        // Companion to `catch_slot` for the parallel object channel (see
        // `docs/design/exceptions.md` section 3): always captured and
        // cleared alongside the string, even though it is usually null
        // (nothing but a real Error-family class instance throw ever
        // populates it -- see `HirStmt::Throw`'s codegen). An explicit
        // `(e as MyError)` cast, lowered in thaw-hir to
        // `Var("<catch_name>__thaw_exception_object")`, is what actually
        // reads this slot; a plain `catch (e) { e.message }` never
        // references it at all.
        let object_slot = self
            .builder
            .build_alloca(str_ty, "catch_object_slot")
            .map_err(|e| e.to_string())?;
        let thrown_object = self
            .builder
            .build_load(
                ptr_ty,
                self.pending_exception_object().as_pointer_value(),
                "caught_exception_object",
            )
            .map_err(|e| e.to_string())?;
        self.builder
            .build_store(object_slot, thrown_object)
            .map_err(|e| e.to_string())?;
        self.builder
            .build_store(
                self.pending_exception_object().as_pointer_value(),
                ptr_ty.const_null(),
            )
            .map_err(|e| e.to_string())?;
        let carrier_type = thaw_hir::caught_exception_carrier_type();
        let carrier_slot = if self.variable_hir_types.get(catch_name) == Some(&carrier_type) {
            self.variables.get(catch_name).copied().map(|(slot, _)| slot)
                .ok_or("caught value has no predeclared carrier slot")?
        } else {
            self.variables.insert(catch_name.to_string(), (catch_slot, str_ty));
            catch_slot
        };
        self.variables.insert(
            format!("{catch_name}__thaw_exception_object"),
            (object_slot, str_ty),
        );
        let aggregate_slot = self.builder.build_alloca(str_ty, "catch_aggregate_slot")
            .map_err(|e| e.to_string())?;
        self.builder.build_store(aggregate_slot, aggregate).map_err(|e| e.to_string())?;
        self.variables.insert(format!("{catch_name}__thaw_exception_aggregate"), (aggregate_slot, str_ty));
        for (suffix, symbol, ty) in [
            (
                "tag",
                PENDING_EXCEPTION_VALUE_TAG_SYMBOL,
                self.context.i64_type().into(),
            ),
            ("f64", PENDING_EXCEPTION_F64_SYMBOL, self.context.f64_type().into()),
            ("i64", PENDING_EXCEPTION_I64_SYMBOL, self.context.i64_type().into()),
            ("bool", PENDING_EXCEPTION_BOOL_SYMBOL, self.context.bool_type().into()),
        ] {
            let slot = self
                .builder
                .build_alloca(ty, &format!("catch_{suffix}_slot"))
                .map_err(|e| e.to_string())?;
            let value = self
                .builder
                .build_load(
                    ty,
                    self.pending_exception_value(symbol).as_pointer_value(),
                    &format!("caught_exception_{suffix}"),
                )
                .map_err(|e| e.to_string())?;
            self.builder
                .build_store(slot, value)
                .map_err(|e| e.to_string())?;
            self.variables.insert(
                format!("{catch_name}__thaw_exception_{suffix}"),
                (slot, ty),
            );
            self.variable_hir_types.insert(
                format!("{catch_name}__thaw_exception_{suffix}"),
                match suffix {
                    "f64" => HirType::F64,
                    "bool" => HirType::Bool,
                    _ => HirType::I64,
                },
            );
        }
        if self.variable_hir_types.get(catch_name) == Some(&carrier_type) {
            let i64_ty = self.context.i64_type();
            let stored_tag = self.variables.get(&format!("{catch_name}__thaw_exception_tag"))
                .ok_or("catch tag snapshot missing")?.0;
            let stored_f64 = self.variables.get(&format!("{catch_name}__thaw_exception_f64"))
                .ok_or("catch f64 snapshot missing")?.0;
            let stored_i64 = self.variables.get(&format!("{catch_name}__thaw_exception_i64"))
                .ok_or("catch i64 snapshot missing")?.0;
            let stored_bool = self.variables.get(&format!("{catch_name}__thaw_exception_bool"))
                .ok_or("catch boolean snapshot missing")?.0;
            let exception_tag = self.builder.build_load(i64_ty, stored_tag, "caught_value_tag")
                .map_err(|e| e.to_string())?.into_int_value();
            let number = self.builder.build_load(self.context.f64_type(), stored_f64, "caught_value_f64")
                .map_err(|e| e.to_string())?.into_float_value();
            let bigint = self.builder.build_load(i64_ty, stored_i64, "caught_value_i64")
                .map_err(|e| e.to_string())?.into_int_value();
            let boolean = self.builder.build_load(self.context.bool_type(), stored_bool, "caught_value_bool")
                .map_err(|e| e.to_string())?.into_int_value();
            let tagged = self.build_caught_exception_carrier(
                thrown, thrown_object, exception_tag, number, bigint, boolean, true,
            )?;
            self.builder.build_store(carrier_slot, tagged).map_err(|e| e.to_string())?;
        }
        self.builder
            .build_store(
                self.pending_exception_value(PENDING_EXCEPTION_VALUE_TAG_SYMBOL)
                    .as_pointer_value(),
                self.context.i64_type().const_zero(),
            )
            .map_err(|e| e.to_string())?;
        // The hidden tag is the same caught string under a private name,
        // for a catch body (or nested handler) that reads the raw tagged
        // error rather than the materialized object. Bound to the same
        // `catch_slot` storage.
        if let Some(hidden_tag) = hidden_tag {
            self.variables
                .insert(hidden_tag.to_string(), (catch_slot, str_ty));
        }
        let catch_terminated = self.compile_block(catch_body)?;
        if let Some(previous) = previous_native_text {
            self.catch_native_text.insert(catch_name.to_string(), previous);
        } else {
            self.catch_native_text.remove(catch_name);
        }
        if !catch_terminated {
            self.builder
                .build_unconditional_branch(merge_bb)
                .map_err(|e| e.to_string())?;
        }

        self.builder.position_at_end(merge_bb);
        if try_terminated && catch_terminated {
            self.builder
                .build_unreachable()
                .map_err(|e| e.to_string())?;
            Ok(true)
        } else {
            Ok(false)
        }
    }
}
