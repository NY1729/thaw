impl<'ctx> HirCompiler<'ctx> {
    /// Emits a `thaw_arena_poll()` call at the top of a loop header. Whether it survives is decided
    /// once the call graph is known (`prune_arena_polls`).
    fn emit_arena_poll(&mut self) -> Result<usize, String> {
        let poll = self.module.get_function("thaw_arena_poll").unwrap_or_else(|| {
            self.module.add_function(
                "thaw_arena_poll",
                self.context.void_type().fn_type(&[], false),
                Some(Linkage::External),
            )
        });
        self.builder.build_call(poll, &[], "arena_poll").map_err(|e| e.to_string())?;
        let call = self.builder.get_insert_block().and_then(|block| block.get_last_instruction());
        let index = self.arena_polls.len();
        self.arena_polls.extend(call);
        Ok(index)
    }

    /// Drops the poll `emit_arena_poll` put in the header of a finished loop when nothing from
    /// the header on calls a function: such a loop allocates nothing, and the poll's call would
    /// only slow it down.
    fn drop_idle_arena_poll(&mut self, index: usize, header: BasicBlock<'ctx>) {
        let Some(function) = header.get_parent() else { return };
        let calls = function
            .get_basic_blocks()
            .into_iter()
            .skip_while(|block| *block != header)
            .any(|block| {
                std::iter::successors(block.get_first_instruction(), |i| i.get_next_instruction())
                    .any(|instruction| {
                        if instruction.get_opcode() != InstructionOpcode::Call {
                            return false;
                        }
                        let last = instruction.get_num_operands() - 1;
                        let callee = match instruction.get_operand(last) {
                            Some(Operand::Value(BasicValueEnum::PointerValue(pointer))) => {
                                FunctionValue::try_from(inkwell::values::AnyValueEnum::from(BasicValueEnum::PointerValue(pointer))).ok()
                            }
                            _ => None,
                        };
                        callee.is_none_or(|callee| {
                            let name = callee.get_name().to_string_lossy();
                            name != "thaw_arena_poll" && !name.starts_with("llvm.")
                        })
                    })
            });
        if !calls && index < self.arena_polls.len() {
            self.arena_polls.remove(index).erase_from_basic_block();
        }
    }

    /// True when every use of `function` is a direct call (callee operand) from a function in `safe`.
    fn only_called_from(
        function: FunctionValue<'ctx>,
        safe: &HashSet<FunctionValue<'ctx>>,
        root: bool,
    ) -> bool {
        let mut next = function.as_global_value().as_pointer_value().get_first_use();
        let mut any = false;
        while let Some(usage) = next {
            next = usage.get_next_use();
            let user = match usage.get_user() {
                inkwell::values::AnyValueEnum::InstructionValue(user) => user,
                // The `Error.stack` symbol table row `{ fn, name, file, line, column, via }` only reads the
                // address. ponytail: matched by its printed form; any other constant user escapes.
                inkwell::values::AnyValueEnum::StructValue(row)
                    if inkwell::values::AnyValue::print_to_string(&row)
                        .to_string()
                        .starts_with("{ ptr, ptr, ptr, i32, i32, ptr } { ptr @") =>
                {
                    continue
                }
                // A call that returns a value is typed by its result.
                inkwell::values::AnyValueEnum::FloatValue(value) => match value.as_instruction() {
                    Some(user) => user,
                    None => return false,
                },
                inkwell::values::AnyValueEnum::IntValue(value) => match value.as_instruction() {
                    Some(user) => user,
                    None => return false,
                },
                inkwell::values::AnyValueEnum::PointerValue(value) => match value.as_instruction() {
                    Some(user) => user,
                    None => return false,
                },
                _ => return false,
            };
            if user.get_opcode() != InstructionOpcode::Call {
                return false;
            }
            any = true;
            let last = user.get_num_operands() - 1;
            let is_callee = matches!(user.get_operand(last),
                Some(Operand::Value(value)) if value.as_value_ref() == function.as_value_ref());
            let as_argument = (0..last).any(|i| matches!(user.get_operand(i),
                Some(Operand::Value(value)) if value.as_value_ref() == function.as_value_ref()));
            let caller = user.get_parent().and_then(|block| block.get_parent());
            if !is_callee || as_argument || !caller.is_some_and(|caller| safe.contains(&caller)) {
                return false;
            }
        }
        any || root
    }

    /// Keeps a poll only in a function that no runtime frame can sit below: reachable from the C
    /// `main` through direct calls alone and never address-taken, so the stack between that
    /// `main` and the poll holds generated frames only (which the conservative stack scan covers).
    /// The functions the C `main` calls directly are the roots (its calls are not emitted yet).
    /// With `enabled` false every poll is removed. Returns whether any poll remains.
    fn prune_arena_polls(&mut self, enabled: bool) -> bool {
        let polls = std::mem::take(&mut self.arena_polls);
        let mut safe: HashSet<FunctionValue<'ctx>> = HashSet::new();
        let mut roots: HashSet<FunctionValue<'ctx>> = HashSet::new();
        if enabled {
            for symbol in [USER_MAIN_SYMBOL, NATIVE_MODULE_INIT_SYMBOL, MODULE_INIT_SYMBOL, TOP_LEVEL_INIT_SYMBOL] {
                roots.extend(self.module.get_function(symbol));
            }
            for function in self.module.get_functions() {
                // An external function can also be called by symbol name from outside the module
                // (the runtime's `thaw_*` entry points, the C `main`, `handler`; the generated
                // `__thaw_*` init functions are roots or direct-call only).
                let name = function.get_name().to_string_lossy();
                let local = roots.contains(&function)
                    || name.starts_with("__thaw_lazy_module_")
                    || !(name.starts_with("thaw_") || name.starts_with("llvm.")
                        || name == "main" || name == "handler");
                if function.count_basic_blocks() > 0 && local {
                    safe.insert(function);
                }
            }
            // Greatest fixed point: drop functions with a use outside the safe set.
            loop {
                let before = safe.len();
                let snapshot = safe.clone();
                safe.retain(|function| {
                    Self::only_called_from(*function, &snapshot, roots.contains(function))
                });
                if safe.len() == before {
                    break;
                }
            }
        }
        let mut kept = false;
        for call in polls {
            let parent = call.get_parent().and_then(|block| block.get_parent());
            if parent.is_some_and(|function| safe.contains(&function)) {
                kept = true;
            } else {
                call.erase_from_basic_block();
            }
        }
        kept
    }
}
