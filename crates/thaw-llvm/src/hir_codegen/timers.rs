impl<'ctx> HirCompiler<'ctx> {
    /// Emits `void adapter(ptr closure)`: runs a zero-argument native closure and, if it left a
    /// pending exception, drops all queued async work so no later timer runs (the terminal
    /// checkpoint then reports the uncaught error and exits 1, as Node does).
    fn native_timer_adapter(&mut self) -> Result<FunctionValue<'ctx>, String> {
        if let Some(adapter) = self.module.get_function("__thaw_timer_adapter") {
            return Ok(adapter);
        }
        let ptr = self.context.ptr_type(AddressSpace::default());
        let adapter = self.module.add_function(
            "__thaw_timer_adapter",
            self.context.void_type().fn_type(&[ptr.into()], false),
            Some(Linkage::Internal),
        );
        let return_block = self.builder.get_insert_block();
        let entry = self.context.append_basic_block(adapter, "entry");
        let failed = self.context.append_basic_block(adapter, "failed");
        let done = self.context.append_basic_block(adapter, "done");
        self.builder.position_at_end(entry);
        let closure = adapter.get_first_param().unwrap().into_pointer_value();
        let code = self
            .builder
            .build_load(ptr, closure, "timer_code")
            .map_err(|error| error.to_string())?
            .into_pointer_value();
        // The return value of a scalar-returning callback is ignored by the caller.
        let callback_type = self.function_type(&[], &HirType::Void)?;
        self.builder
            .build_indirect_call(callback_type, code, &[closure.into()], "invoke_timer")
            .map_err(|error| error.to_string())?;
        let pending = self
            .builder
            .build_load(ptr, self.pending_exception().as_pointer_value(), "timer_pending_exception")
            .map_err(|error| error.to_string())?
            .into_pointer_value();
        let threw = self
            .builder
            .build_is_not_null(pending, "timer_threw")
            .map_err(|error| error.to_string())?;
        self.builder
            .build_conditional_branch(threw, failed, done)
            .map_err(|error| error.to_string())?;
        self.builder.position_at_end(failed);
        self.builder
            .build_call(
                self.module.get_function("thaw_runtime_abort_pending_work").unwrap(),
                &[],
                "abort_after_timer_exception",
            )
            .map_err(|error| error.to_string())?;
        self.builder
            .build_unconditional_branch(done)
            .map_err(|error| error.to_string())?;
        self.builder.position_at_end(done);
        self.builder.build_return(None).map_err(|error| error.to_string())?;
        if let Some(block) = return_block {
            self.builder.position_at_end(block);
        }
        Ok(adapter)
    }

    fn compile_native_timer(&mut self, name: &str, args: &[HirExpr]) -> Result<BasicValueEnum<'ctx>, String> {
        self.uses_native_timers = true;
        let i64_type = self.context.i64_type();
        let i8_type = self.context.i8_type();
        let runtime = |this: &Self, symbol: &str| {
            this.module
                .get_function(symbol)
                .ok_or_else(|| format!("{symbol} is not declared"))
        };
        match (name, args) {
            ("__thaw_timer_set", [HirExpr::Lit(HirLit::F64(kind)), delay, callback]) => {
                let delay = self.compile_expr(delay)?;
                let closure = self.compile_expr(callback)?;
                let adapter = self.native_timer_adapter()?;
                let set = runtime(self, "thaw_timer_set")?;
                self.builder
                    .build_call(
                        set,
                        &[
                            i8_type.const_int(*kind as u64, false).into(),
                            delay.into(),
                            adapter.as_global_value().as_pointer_value().into(),
                            closure.into(),
                        ],
                        "native_timer_set",
                    )
                    .map_err(|error| error.to_string())?
                    .try_as_basic_value()
                    .basic()
                    .ok_or_else(|| "thaw_timer_set returned no value".to_string())
            }
            ("__thaw_timer_clear", [handle]) => {
                let handle = self.compile_expr(handle)?;
                let clear = runtime(self, "thaw_timer_clear")?;
                self.builder
                    .build_call(clear, &[handle.into()], "native_timer_clear")
                    .map_err(|error| error.to_string())?;
                Ok(i64_type.const_zero().into())
            }
            ("__thaw_timer_ref" | "__thaw_timer_unref" | "__thaw_timer_refresh", [handle]) => {
                let handle = self.compile_expr(handle)?;
                let symbol = name.replace("__thaw_timer", "thaw_timer");
                let function = runtime(self, &symbol)?;
                self.builder
                    .build_call(function, &[handle.into()], "native_timer_update")
                    .map_err(|error| error.to_string())?;
                Ok(handle)
            }
            ("__thaw_timer_has_ref", [handle]) => {
                let handle = self.compile_expr(handle)?;
                let has_ref = runtime(self, "thaw_timer_has_ref")?;
                let flag = self
                    .builder
                    .build_call(has_ref, &[handle.into()], "native_timer_has_ref")
                    .map_err(|error| error.to_string())?
                    .try_as_basic_value()
                    .basic()
                    .ok_or_else(|| "thaw_timer_has_ref returned no value".to_string())?
                    .into_int_value();
                self.builder
                    .build_int_compare(IntPredicate::NE, flag, i8_type.const_zero(), "native_timer_has_ref_bool")
                    .map(Into::into)
                    .map_err(|error| error.to_string())
            }
            _ => Err(format!("malformed native timer builtin `{name}`")),
        }
    }
}
