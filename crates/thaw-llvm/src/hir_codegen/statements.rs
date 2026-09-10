impl<'ctx> HirCompiler<'ctx> {
    fn compile_conditional_value(
        &mut self,
        test: &HirExpr,
        consequent: &HirExpr,
        alternate: &HirExpr,
        ty: &HirType,
    ) -> Result<BasicValueEnum<'ctx>, String> {
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

    fn compile_stmt(&mut self, stmt: &HirStmt) -> Result<bool, String> {
        match stmt {
            HirStmt::Expr(expr) => {
                if let HirExpr::FfiCall(sig, args) = expr {
                    if sig.ret == HirType::Void {
                        self.compile_ffi_call(sig, args)?;
                        return Ok(false);
                    }
                }
                self.compile_expr(expr)?;
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
                let val = self.compile_expr(expr)?;
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

            HirStmt::Try(body, catch_name, catch_body) => {
                self.compile_try(body, catch_name, catch_body)
            }
        }
    }

    fn compile_if(
        &mut self,
        cond: &HirExpr,
        then_branch: &[HirStmt],
        else_branch: &[HirStmt],
    ) -> Result<bool, String> {
        let function = self.current_function();
        let cond_val = self.compile_expr(cond)?.into_int_value();

        let then_bb = self.context.append_basic_block(function, "then");
        let else_bb = self.context.append_basic_block(function, "else");
        let merge_bb = self.context.append_basic_block(function, "ifcont");

        self.builder
            .build_conditional_branch(cond_val, then_bb, else_bb)
            .map_err(|e| e.to_string())?;

        let variables_before_branches = self.variables.clone();
        let arena_variables_before_branches = self.arena_variables.clone();
        self.builder.position_at_end(then_bb);
        let then_terminated = self.compile_block(then_branch)?;
        let then_variables = self.variables.clone();
        let then_arena_variables = self.arena_variables.clone();
        if !then_terminated {
            self.builder
                .build_unconditional_branch(merge_bb)
                .map_err(|e| e.to_string())?;
        } else {
            self.variables = variables_before_branches.clone();
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

        let preheader_bb = self.builder.get_insert_block().unwrap();

        self.builder
            .build_unconditional_branch(header_bb)
            .map_err(|e| e.to_string())?;

        let variables_before_body = self.variables.clone();
        let variable_types_before_body = self.variable_hir_types.clone();
        let arena_variables_before_body = self.arena_variables.clone();
        self.builder.position_at_end(body_bb);
        self.loop_stack.push((header_bb, after_bb));
        self.loop_promotion_scopes.push((
            preheader_bb,
            variables_before_body.keys().cloned().collect(),
        ));
        let body_terminated = self.compile_block(body)?;
        self.loop_promotion_scopes.pop();
        self.loop_stack.pop();
        if !body_terminated {
            self.builder
                .build_unconditional_branch(header_bb)
                .map_err(|e| e.to_string())?;
        }

        let body_variables = std::mem::replace(&mut self.variables, variables_before_body.clone());
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
        let cond_val = self.compile_expr(cond)?.into_int_value();
        self.builder
            .build_conditional_branch(cond_val, body_bb, after_bb)
            .map_err(|e| e.to_string())?;

        self.builder.position_at_end(after_bb);
        // `after_bb` is always reachable (the condition can be false on the
        // first check), so a `while` never terminates its enclosing block.
        Ok(false)
    }

    fn compile_try(
        &mut self,
        body: &[HirStmt],
        catch_name: &str,
        catch_body: &[HirStmt],
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
        self.builder
            .build_store(
                self.pending_exception().as_pointer_value(),
                ptr_ty.const_null(),
            )
            .map_err(|e| e.to_string())?;
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
        self.variables
            .insert(catch_name.to_string(), (catch_slot, str_ty));
        self.variables.insert(
            format!("{catch_name}__thaw_exception_object"),
            (object_slot, str_ty),
        );
        let catch_terminated = self.compile_block(catch_body)?;
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
