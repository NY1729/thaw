impl<'ctx> HirCompiler<'ctx> {
    fn begin_owned_json_discard_frame(&mut self) {
        let function_name = self.current_function().get_name().to_string_lossy().into_owned();
        self.owned_json_discard_frames.push(OwnedJsonDiscardFrame {
            function_name,
            candidates: Vec::new(),
        });
    }

    /// Record only successful fresh-box producers. The transfer slot remains
    /// false until a successful store consumes this exact SSA value.
    fn mark_fresh_owned_json_result(&mut self, value: BasicValueEnum<'ctx>) -> Result<(), String> {
        let function = self.current_function();
        let function_name = function.get_name().to_string_lossy().into_owned();
        let Some(frame) = self.owned_json_discard_frames.last() else { return Ok(()) };
        if frame.function_name != function_name
            || frame.candidates.iter().any(|candidate| candidate.value.as_value_ref() == value.as_value_ref())
        {
            return Ok(());
        }

        // Keep the slot in the function entry block so producers inside loops
        // reuse stack storage rather than growing the stack on every iteration.
        let entry = function.get_first_basic_block().ok_or("JSON discard producer has no entry block")?;
        let entry_builder = self.context.create_builder();
        if let Some(first) = entry.get_first_instruction() {
            entry_builder.position_before(&first);
        } else {
            entry_builder.position_at_end(entry);
        }
        let transferred = entry_builder.build_alloca(self.context.bool_type(), "owned_json_transferred")
            .map_err(|error| error.to_string())?;
        self.builder.build_store(transferred, self.context.bool_type().const_zero())
            .map_err(|error| error.to_string())?;
        self.owned_json_discard_frames.last_mut().unwrap().candidates.push(
            OwnedJsonDiscardCandidate { value, transferred },
        );
        Ok(())
    }

    /// Mark an exact candidate as transferred only on the path where a
    /// binding/property/index write succeeds. Active outer frames are included
    /// because wrappers may return the very same SSA value.
    fn revoke_fresh_owned_json_result(&mut self, value: BasicValueEnum<'ctx>) -> Result<(), String> {
        let function_name = self.current_function().get_name().to_string_lossy().into_owned();
        let slots: Vec<_> = self.owned_json_discard_frames.iter()
            .filter(|frame| frame.function_name == function_name)
            .flat_map(|frame| frame.candidates.iter())
            .filter(|candidate| candidate.value.as_value_ref() == value.as_value_ref())
            .map(|candidate| candidate.transferred)
            .collect();
        for slot in slots {
            self.builder.build_store(slot, self.context.bool_type().const_int(1, false))
                .map_err(|error| error.to_string())?;
        }
        Ok(())
    }

    fn discard_owned_json_statement_result(
        &mut self,
        expression: &HirExpr,
        value: BasicValueEnum<'ctx>,
        result_type: Option<HirType>,
    ) -> Result<(), String> {
        let Some(frame) = self.owned_json_discard_frames.pop() else { return Ok(()) };
        let current_function_name = self.current_function().get_name().to_string_lossy().into_owned();
        if frame.function_name != current_function_name
            || matches!(expression, HirExpr::Assign(_, _))
            || !matches!(result_type, Some(HirType::Json | HirType::Dictionary(_)))
        {
            return Ok(());
        }
        let Some(candidate) = frame.candidates.iter()
            .find(|candidate| candidate.value.as_value_ref() == value.as_value_ref()) else {
            return Ok(());
        };
        let transferred = self.builder.build_load(self.context.bool_type(), candidate.transferred,
            "discarded_json_was_transferred").map_err(|error| error.to_string())?.into_int_value();
        let should_destroy = self.builder.build_not(transferred, "discarded_json_still_owned")
            .map_err(|error| error.to_string())?;
        let function = self.current_function();
        let destroy = self.context.append_basic_block(function, "destroy_untransferred_json_result");
        let done = self.context.append_basic_block(function, "discarded_json_cleanup_done");
        self.builder.build_conditional_branch(should_destroy, destroy, done)
            .map_err(|error| error.to_string())?;
        self.builder.position_at_end(destroy);
        self.builder.build_call(
            self.module.get_function("thaw_json_destroy").unwrap(),
            &[value.into()],
            "destroy_discarded_fresh_json_result",
        ).map_err(|error| error.to_string())?;
        self.builder.build_unconditional_branch(done).map_err(|error| error.to_string())?;
        self.builder.position_at_end(done);
        Ok(())
    }
}
