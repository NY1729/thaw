impl<'ctx> HirCompiler<'ctx> {
    fn defer_chain_pending_failure(
        &mut self,
        cleanup: PointerValue<'ctx>,
        packet_offset: u64,
        token_offset: u64,
    ) -> Result<(), String> {
        let ptr = self.context.ptr_type(AddressSpace::default());
        let function = self.current_function();
        let pending = self.builder.build_load(ptr,
            self.pending_exception().as_pointer_value(), "chain_cause_primary")
            .map_err(|e| e.to_string())?.into_pointer_value();
        let has_primary = self.builder.build_is_not_null(pending, "chain_cause_has_primary")
            .map_err(|e| e.to_string())?;
        self.defer_reentrant_pending_exception_tuple_at(cleanup, packet_offset, token_offset)?;
        let report = self.context.append_basic_block(function, "report_chain_cause");
        let done = self.context.append_basic_block(function, "chain_cause_deferred");
        self.builder.build_conditional_branch(has_primary, report, done)
            .map_err(|e| e.to_string())?;
        self.builder.position_at_end(report);
        self.builder.build_call(self.module.get_function("thaw_http_mark_unhandled_error").unwrap(),
            &[], "mark_chain_causal_failure")
            .map_err(|e| e.to_string())?;
        self.builder.build_unconditional_branch(done).map_err(|e| e.to_string())?;
        self.builder.position_at_end(done);
        Ok(())
    }

    /// Preserve a Promise-chain callback throw across settlement and any
    /// reentrant Host cleanup. The chain subscription invokes this adapter
    /// once, so its rooted packet is the one-shot event authority; a settled
    /// output never proves that this independent throw was already accepted.
    fn reject_chain_callback_exception(
        &mut self,
        cleanup: PointerValue<'ctx>,
        completion: PointerValue<'ctx>,
        error: PointerValue<'ctx>,
        saved: &[(inkwell::values::GlobalValue<'ctx>, BasicValueEnum<'ctx>)],
        name: &str,
    ) -> Result<(), String> {
        let ptr = self.context.ptr_type(AddressSpace::default());
        let function = self.current_function();
        let original_field = self.async_frame_field(cleanup, 64, "chain_original_packet_field")?;
        let original = self.builder.build_load(ptr, original_field, "chain_original_packet")
            .map_err(|e| e.to_string())?.into_pointer_value();
        self.copy_pending_exception_to_deferred_packet(original)?;
        let original_value = self.async_frame_field(original, 8, "chain_original_value")?;
        self.builder.build_store(original_value, error).map_err(|e| e.to_string())?;
        let status = self.reject_promise_with_pending_exception_status_isolated(
            completion, error, name,
        )?;
        let accepted = self.builder.build_int_compare(IntPredicate::NE, status,
            status.get_type().const_zero(), "chain_throw_accepted")
            .map_err(|e| e.to_string())?;
        let settled = self.context.append_basic_block(function, "chain_throw_settled");
        let failed = self.context.append_basic_block(function, "chain_throw_unsettled");
        self.builder.build_conditional_branch(accepted, settled, failed)
            .map_err(|e| e.to_string())?;

        self.builder.position_at_end(failed);
        // Queue the independent causal failure first: the std packet list is
        // LIFO, so the original callback reason becomes the next head.
        self.defer_chain_pending_failure(cleanup, 96, 88)?;
        self.builder.build_store(original_field, ptr.const_null())
            .map_err(|e| e.to_string())?;
        self.builder.build_call(
            self.module.get_function("thaw_json_enqueue_deferred_exception_packet").unwrap(),
            &[original.into()], "queue_chain_original_failure",
        ).map_err(|e| e.to_string())?;
        self.builder.build_call(self.module.get_function("thaw_http_mark_unhandled_error").unwrap(),
            &[], "mark_chain_original_failure")
            .map_err(|e| e.to_string())?;
        self.restore_pending_exception_tuple(saved, cleanup)?;
        self.builder.build_return(None).map_err(|e| e.to_string())?;

        self.builder.position_at_end(settled);
        let owner_field = self.async_frame_field(original, 32, "chain_original_json_owner")?;
        let owner = self.builder.build_load(ptr, owner_field, "chain_original_owner")
            .map_err(|e| e.to_string())?.into_pointer_value();
        self.builder.build_store(owner_field, ptr.const_null()).map_err(|e| e.to_string())?;
        let token_field = self.async_frame_field(cleanup, 88, "chain_original_owner_token")?;
        let token = self.builder.build_load(ptr, token_field, "chain_original_owner_token_value")
            .map_err(|e| e.to_string())?.into_pointer_value();
        self.builder.build_call(self.module.get_function("thaw_json_enqueue_reserved_cleanup").unwrap(),
            &[owner.into(), token.into()], "defer_chain_original_owner")
            .map_err(|e| e.to_string())?;
        self.defer_chain_pending_failure(cleanup, 96, 56)?;
        self.destroy_reporter_owned_native_text(original)?;
        self.defer_chain_pending_failure(cleanup, 72, 104)?;
        self.restore_pending_exception_tuple(saved, cleanup)?;
        self.builder.build_return(None).map_err(|e| e.to_string())?;
        Ok(())
    }

    fn transfer_matching_async_catch_json_owner(
        &self, plan: &FrameAsyncPlan,
    ) -> Result<(), String> {
        let ptr = self.context.ptr_type(AddressSpace::default());
        let pending_owner = self.builder.build_load(ptr,
            self.pending_exception_json_owner().as_pointer_value(),
            "throw_pending_json_owner").map_err(|e| e.to_string())?
            .into_pointer_value();
        let owner_bits = self.builder.build_ptr_to_int(pending_owner,
            self.context.i64_type(), "throw_owner_bits")
            .map_err(|e| e.to_string())?;
        for binding in &plan.generated_catch_bindings {
            let name = format!("{binding}__thaw_exception_json_owner");
            let (slot, _) = self.variables.get(&name).copied()
                .ok_or_else(|| format!("missing async catch owner cell `{name}`"))?;
            let held = self.builder.build_load(ptr, slot,
                "throw_catch_json_owner").map_err(|e| e.to_string())?
                .into_pointer_value();
            let held_bits = self.builder.build_ptr_to_int(held,
                self.context.i64_type(), "throw_catch_owner_bits")
                .map_err(|e| e.to_string())?;
            let same = self.builder.build_int_compare(IntPredicate::EQ,
                held_bits, owner_bits, "throw_catch_owner_matches_pending")
                .map_err(|e| e.to_string())?;
            let nonnull = self.builder.build_is_not_null(pending_owner,
                "throw_pending_owner_present")
                .map_err(|e| e.to_string())?;
            let transfer = self.builder.build_and(same, nonnull,
                "throw_catch_owner_transferred")
                .map_err(|e| e.to_string())?;
            let retained = self.builder.build_select(transfer, ptr.const_null(), held,
                "throw_catch_owner_remaining")
                .map_err(|e| e.to_string())?;
            self.builder.build_store(slot, retained).map_err(|e| e.to_string())?;
        }
        Ok(())
    }

    /// Reject this frame's completion from an explicit throw. Snapshot the
    /// original reason before the runtime's tag-7 share can publish a new
    /// HOST_ERROR. A status-zero void resume has no return channel for that
    /// reason, so the frame's pre-reserved terminal authority reports it.
    /// Successful callers continue in the block this method leaves active.
    fn reject_frame_explicit_throw(
        &mut self, frame: PointerValue<'ctx>, completion: PointerValue<'ctx>,
        error: PointerValue<'ctx>, plan: &FrameAsyncPlan, name: &str,
    ) -> Result<(), String> {
        let ptr = self.context.ptr_type(AddressSpace::default());
        let function = self.current_function();
        let cleanup_slot = self.async_frame_field(frame,
            ASYNC_TERMINAL_CLEANUP_OFFSET, "explicit_throw_cleanup_slot")?;
        let cleanup = self.builder.build_load(ptr, cleanup_slot,
            "explicit_throw_cleanup").map_err(|e| e.to_string())?
            .into_pointer_value();
        let original_field = self.async_frame_field(cleanup, 64,
            "explicit_throw_original_field")?;
        let original = self.builder.build_load(ptr, original_field,
            "explicit_throw_original_packet").map_err(|e| e.to_string())?
            .into_pointer_value();
        let root = self.http_saved_exception_root();
        let previous = self.builder.build_load(ptr, root.as_pointer_value(),
            "explicit_throw_previous_root").map_err(|e| e.to_string())?;
        self.builder.build_store(cleanup, previous).map_err(|e| e.to_string())?;
        let frame_field = self.async_frame_field(cleanup, 80,
            "explicit_throw_rooted_frame")?;
        self.builder.build_store(frame_field, frame).map_err(|e| e.to_string())?;
        self.builder.build_store(root.as_pointer_value(), cleanup)
            .map_err(|e| e.to_string())?;
        // The frame's lexical catch may still name this same original Box.
        // Transfer its token before packet capture, leaving borrowed aliases.
        self.transfer_matching_async_catch_json_owner(plan)?;
        self.copy_pending_exception_to_deferred_packet(original)?;
        let original_value = self.async_frame_field(original, 8,
            "explicit_throw_original_value")?;
        self.builder.build_store(original_value, error).map_err(|e| e.to_string())?;
        self.builder.build_store(self.pending_exception_json_owner().as_pointer_value(),
            ptr.const_null()).map_err(|e| e.to_string())?;
        let status = self.reject_promise_with_pending_exception_status_isolated(
            completion, error, name)?;
        let settled = self.builder.build_int_compare(IntPredicate::NE, status,
            status.get_type().const_zero(), "explicit_throw_settled")
            .map_err(|e| e.to_string())?;
        let accepted = self.context.append_basic_block(function,
            "explicit_throw_accepted");
        let failed = self.context.append_basic_block(function,
            "explicit_throw_settlement_failed");
        self.builder.build_conditional_branch(settled, accepted, failed)
            .map_err(|e| e.to_string())?;
        self.builder.position_at_end(failed);
        self.builder.build_store(original_field, ptr.const_null())
            .map_err(|e| e.to_string())?;
        self.builder.build_call(self.module.get_function("thaw_json_enqueue_deferred_exception_packet").unwrap(),
            &[original.into()], "queue_explicit_throw_original")
            .map_err(|e| e.to_string())?;
        self.defer_reentrant_pending_exception_tuple_at(cleanup, 96, 88)?;
        self.builder.build_call(self.module.get_function("thaw_http_mark_unhandled_error").unwrap(),
            &[], "mark_explicit_throw_terminal")
            .map_err(|e| e.to_string())?;
        if function.get_type().get_return_type().is_some() {
            self.builder.build_return(Some(&completion)).map_err(|e| e.to_string())?;
        } else {
            self.builder.build_return(None).map_err(|e| e.to_string())?;
        }
        self.builder.position_at_end(accepted);
        let owner_field = self.async_frame_field(original, 32,
            "explicit_throw_owner_field")?;
        let owner = self.builder.build_load(ptr, owner_field,
            "explicit_throw_original_owner").map_err(|e| e.to_string())?
            .into_pointer_value();
        self.builder.build_store(owner_field, ptr.const_null())
            .map_err(|e| e.to_string())?;
        let token_field = self.async_frame_field(cleanup, 88,
            "explicit_throw_owner_token_field")?;
        let token = self.builder.build_load(ptr, token_field,
            "explicit_throw_owner_token").map_err(|e| e.to_string())?
            .into_pointer_value();
        self.builder.build_call(self.module.get_function("thaw_json_enqueue_reserved_cleanup").unwrap(),
            &[owner.into(), token.into()], "defer_explicit_throw_owner")
            .map_err(|e| e.to_string())?;
        self.destroy_reporter_owned_native_text(original)?;
        for offset in [8_u64, 16, 24, 40] {
            let field = self.async_frame_field(original, offset,
                "clear_handled_throw_alias")?;
            self.builder.build_store(field, ptr.const_null())
                .map_err(|e| e.to_string())?;
        }
        self.builder.build_store(original_field, ptr.const_null())
            .map_err(|e| e.to_string())?;
        // Frame-local Promise/catch retirement can invoke Host destructors.
        // Keep this registered snapshot linked through that retirement so a
        // second exact failure is captured in the separate cause packet.
        self.retire_all_async_frame_owners(plan, frame)?;
        let new_primary = self.builder.build_load(ptr,
            self.pending_exception().as_pointer_value(), "explicit_throw_new_primary")
            .map_err(|e| e.to_string())?.into_pointer_value();
        let new_json_owner = self.builder.build_load(ptr,
            self.pending_exception_json_owner().as_pointer_value(), "explicit_throw_new_json_owner")
            .map_err(|e| e.to_string())?.into_pointer_value();
        let new_text_owner = self.builder.build_load(ptr,
            self.pending_exception_native_text_owner().as_pointer_value(), "explicit_throw_new_text_owner")
            .map_err(|e| e.to_string())?.into_pointer_value();
        let a = self.builder.build_is_not_null(new_primary,
            "explicit_throw_has_primary_cause").map_err(|e| e.to_string())?;
        let b = self.builder.build_is_not_null(new_json_owner,
            "explicit_throw_has_json_cause").map_err(|e| e.to_string())?;
        let c = self.builder.build_is_not_null(new_text_owner,
            "explicit_throw_has_text_cause").map_err(|e| e.to_string())?;
        let ab = self.builder.build_or(a, b, "explicit_throw_has_primary_or_json_cause")
            .map_err(|e| e.to_string())?;
        let has_cause = self.builder.build_or(ab, c, "explicit_throw_has_cause")
            .map_err(|e| e.to_string())?;
        let cause = self.context.append_basic_block(function,
            "explicit_throw_accepted_cause");
        let clean = self.context.append_basic_block(function,
            "explicit_throw_clean");
        self.builder.build_conditional_branch(has_cause, cause, clean)
            .map_err(|e| e.to_string())?;
        self.builder.position_at_end(cause);
        self.defer_reentrant_pending_exception_tuple_at(cleanup, 96, 56)?;
        self.builder.build_call(self.module.get_function("thaw_http_mark_unhandled_error").unwrap(),
            &[], "mark_explicit_throw_cause_terminal")
            .map_err(|e| e.to_string())?;
        if function.get_type().get_return_type().is_some() {
            self.builder.build_return(Some(&completion)).map_err(|e| e.to_string())?;
        } else {
            self.builder.build_return(None).map_err(|e| e.to_string())?;
        }
        self.builder.position_at_end(clean);
        self.restore_pending_exception_tuple(&[], cleanup)?;
        Ok(())
    }

    /// Terminal fallback for a frame-local rejection that has no delivered
    /// source subscription to lend its share. The frame owns this packet
    /// from ramp initialization, before any Promise/user callback exists.
    /// The fatal process path keeps the exact packet rooted until exit.
    fn queue_frame_terminal_pending_exception(
        &self, frame: PointerValue<'ctx>,
    ) -> Result<(), String> {
        let ptr = self.context.ptr_type(AddressSpace::default());
        let original = self.builder.build_load(ptr,
            self.pending_exception().as_pointer_value(), "frame_terminal_original")
            .map_err(|error| error.to_string())?.into_pointer_value();
        let has_original = self.builder.build_is_not_null(original,
            "frame_terminal_has_exact_original")
            .map_err(|error| error.to_string())?;
        let function = self.current_function();
        let has_reason = self.context.append_basic_block(function,
            "frame_terminal_reason_present");
        let missing_reason = self.context.append_basic_block(function,
            "frame_terminal_reason_missing");
        self.builder.build_conditional_branch(has_original, has_reason, missing_reason)
            .map_err(|error| error.to_string())?;
        self.builder.position_at_end(missing_reason);
        self.builder.build_unreachable().map_err(|error| error.to_string())?;
        self.builder.position_at_end(has_reason);
        let cleanup_slot = self.async_frame_field(frame,
            ASYNC_TERMINAL_CLEANUP_OFFSET, "frame_terminal_cleanup_slot")?;
        let cleanup = self.builder.build_load(ptr, cleanup_slot,
            "frame_terminal_cleanup").map_err(|error| error.to_string())?
            .into_pointer_value();
        let packet_field = unsafe { self.builder.build_in_bounds_gep(self.context.i8_type(), cleanup,
            &[self.context.i64_type().const_int(64, false)], "frame_terminal_packet_field")
            .map_err(|error| error.to_string())? };
        let packet = self.builder.build_load(ptr, packet_field,
            "frame_terminal_packet").map_err(|error| error.to_string())?
            .into_pointer_value();
        // The reservation was checked in the ramp before publishing the
        // completion. A missing packet is a violated internal contract; it
        // must not turn a failed rejection into a silent void return.
        let present = self.builder.build_is_not_null(packet,
            "frame_terminal_packet_present").map_err(|error| error.to_string())?;
        let ready = self.context.append_basic_block(function, "frame_terminal_packet_ready");
        let broken = self.context.append_basic_block(function, "frame_terminal_packet_missing");
        self.builder.build_conditional_branch(present, ready, broken)
            .map_err(|error| error.to_string())?;
        self.builder.position_at_end(broken);
        self.builder.build_unreachable().map_err(|error| error.to_string())?;
        self.builder.position_at_end(ready);
        // A ramp can fail before its first subscription lends an ArenaRoot
        // to the frame. Keep borrowed catch/local carriers reachable until
        // the fatal main/Lambda exit even if an invocation reset occurs.
        // This cell has no later consumer: terminal callers return at once.
        let root = self.http_saved_exception_root();
        let previous = self.builder.build_load(ptr, root.as_pointer_value(),
            "frame_terminal_previous_root").map_err(|error| error.to_string())?
            .into_pointer_value();
        self.builder.build_store(cleanup, previous).map_err(|error| error.to_string())?;
        let frame_field = unsafe { self.builder.build_in_bounds_gep(self.context.i8_type(), cleanup,
            &[self.context.i64_type().const_int(80, false)], "frame_terminal_rooted_frame_field")
            .map_err(|error| error.to_string())? };
        self.builder.build_store(frame_field, frame).map_err(|error| error.to_string())?;
        self.builder.build_store(root.as_pointer_value(), cleanup)
            .map_err(|error| error.to_string())?;
        self.copy_pending_exception_to_deferred_packet(packet)?;
        self.builder.build_store(packet_field, ptr.const_null())
            .map_err(|error| error.to_string())?;
        self.clear_pending_exception_tuple_for_cleanup()?;
        self.builder.build_call(self.module.get_function("thaw_json_enqueue_deferred_exception_packet").unwrap(),
            &[packet.into()], "queue_frame_terminal_exact_exception")
            .map_err(|error| error.to_string())?;
        self.builder.build_call(self.module.get_function("thaw_http_mark_unhandled_error").unwrap(),
            &[], "mark_frame_terminal_exact_failure")
            .map_err(|error| error.to_string())?;
        Ok(())
    }

    /// A void resume cannot propagate a status-zero rejection through its
    /// return value. If completion remains pending, move the delivered
    /// subscription's independent source share to its pre-reserved packet;
    /// if reentry already settled completion, retain that state and leave
    /// the source with its subscription. Queue any exact causal pending
    /// tuple in the second packet in either case. The caller must only enter
    /// this path for the same delivered `waiting` and `completion` pair.
    fn queue_failed_delivered_source(
        &self, completion: PointerValue<'ctx>, waiting: PointerValue<'ctx>,
    ) -> Result<(), String> {
        let function = self.current_function();
        let ptr_ty = self.context.ptr_type(AddressSpace::default());
        let queued = self.builder.build_call(
            self.module.get_function("thaw_promise_queue_delivered_source_failure").unwrap(),
            &[completion.into(), waiting.into()], "queue_exact_failed_source",
        ).map_err(|error| error.to_string())?.try_as_basic_value().basic()
            .ok_or("terminal source handoff returned no status")?.into_int_value();
        let queued_ok = self.builder.build_int_compare(IntPredicate::NE,
            queued, queued.get_type().const_zero(), "exact_failed_source_disposed")
            .map_err(|error| error.to_string())?;
        let broken_contract = self.context.append_basic_block(function,
            "broken_delivered_source_contract");
        let queue_cause = self.context.append_basic_block(function,
            "preserve_exact_source_share_failure");
        let done = self.context.append_basic_block(function,
            "exact_delivered_source_failure_queued");
        self.builder.build_conditional_branch(queued_ok, queue_cause, broken_contract)
            .map_err(|error| error.to_string())?;
        self.builder.position_at_end(queue_cause);
        let cause = self.builder.build_load(ptr_ty,
            self.pending_exception().as_pointer_value(), "source_share_failure_pending")
            .map_err(|error| error.to_string())?.into_pointer_value();
        let has_cause = self.builder.build_is_not_null(cause,
            "source_share_failure_has_exact_cause").map_err(|error| error.to_string())?;
        let capture_cause = self.context.append_basic_block(function,
            "queue_exact_source_share_cause");
        self.builder.build_conditional_branch(has_cause, capture_cause, done)
            .map_err(|error| error.to_string())?;
        self.builder.position_at_end(capture_cause);
        let cause_packet = self.builder.build_call(
            self.module.get_function("thaw_promise_take_delivered_cause_packet").unwrap(),
            &[completion.into(), waiting.into()], "take_exact_share_cause_packet",
        ).map_err(|error| error.to_string())?.try_as_basic_value().basic()
            .ok_or("exact share cause packet returned no pointer")?.into_pointer_value();
        let cause_ready = self.context.append_basic_block(function,
            "exact_share_cause_packet_ready");
        let missing_cause = self.builder.build_is_null(cause_packet,
            "exact_share_cause_packet_missing").map_err(|error| error.to_string())?;
        self.builder.build_conditional_branch(missing_cause, broken_contract, cause_ready)
            .map_err(|error| error.to_string())?;
        self.builder.position_at_end(cause_ready);
        self.copy_pending_exception_to_deferred_packet(cause_packet)?;
        self.clear_pending_exception_tuple_for_cleanup()?;
        self.builder.build_call(
            self.module.get_function("thaw_json_enqueue_deferred_exception_packet").unwrap(),
            &[cause_packet.into()], "queue_exact_share_failure_cause",
        ).map_err(|error| error.to_string())?;
        self.builder.build_unconditional_branch(done).map_err(|error| error.to_string())?;
        self.builder.position_at_end(broken_contract);
        // All generated call sites have a live delivered subscription and
        // two pre-reserved packets. Returning void on a violated contract
        // would drop the last exact source claim and strand completion.
        self.builder.build_unreachable().map_err(|error| error.to_string())?;
        self.builder.position_at_end(done);
        Ok(())
    }

    fn consume_async_waiting_creator_ticket(
        &mut self, frame: PointerValue<'ctx>,
    ) -> Result<(), String> {
        let slot = self.async_frame_field(
            frame, ASYNC_WAITING_CREATOR_TICKET_OFFSET, "waiting_creator_ticket_slot",
        )?;
        let ticket = self.builder.build_load(
            self.context.i64_type(), slot, "waiting_creator_ticket",
        ).map_err(|error| error.to_string())?.into_int_value();
        let present = self.builder.build_int_compare(IntPredicate::NE, ticket,
            self.context.i64_type().const_zero(), "waiting_creator_ticket_present")
            .map_err(|error| error.to_string())?;
        let function = self.current_function();
        let retire = self.context.append_basic_block(function, "retire_waiting_creator_ticket");
        let done = self.context.append_basic_block(function, "waiting_creator_ticket_retired");
        self.builder.build_conditional_branch(present, retire, done)
            .map_err(|error| error.to_string())?;
        self.builder.position_at_end(retire);
        let cleanup_slot = self.async_frame_field(frame,
            ASYNC_WAITING_CREATOR_CLEANUP_OFFSET, "waiting_creator_cleanup_slot")?;
        let cleanup = self.builder.build_load(self.context.ptr_type(AddressSpace::default()),
            cleanup_slot, "waiting_creator_cleanup")
            .map_err(|error| error.to_string())?.into_pointer_value();
        let saved = self.activate_async_promise_local_cleanup(cleanup)?;
        // Clear the frame claim before a last-release destructor can reenter.
        self.builder.build_store(slot, self.context.i64_type().const_zero())
            .map_err(|error| error.to_string())?;
        self.builder.build_call(
            self.module.get_function("thaw_promise_destroy_creator_ticket").unwrap(),
            &[ticket.into()], "consume_waiting_creator_ticket",
        ).map_err(|error| error.to_string())?;
        self.defer_reentrant_pending_exception_tuple(cleanup)?;
        self.restore_pending_exception_tuple(&saved, cleanup)?;
        self.builder.build_unconditional_branch(done).map_err(|error| error.to_string())?;
        self.builder.position_at_end(done);
        Ok(())
    }

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
        self.active_promise_return_ticket = None;
        // A Promise subscription owns an ArenaRoot for its suspended frame.
        // Start tracing before the entrypoint can allocate that frame.
        self.tracks_owned_json_roots = true;
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
        let cancel = self.module.add_function(
            &format!("{symbol}.cancel"),
            self.context.void_type().fn_type(&[ptr_ty.into()], false),
            Some(Linkage::Internal),
        );

        let entry = self.context.append_basic_block(ramp, "entry");
        self.builder.position_at_end(entry);
        let ramp_ticket_out = ramp.get_last_param()
            .ok_or("async Promise ramp lacks its creator-ticket output")?
            .into_pointer_value();
        self.builder.build_store(ramp_ticket_out, self.context.i64_type().const_zero())
            .map_err(|error| error.to_string())?;
        self.variables.clear();
        self.for_iteration_frame_slots.clear();
        self.catch_native_text.clear();
        self.variable_hir_types.clear();
        self.arena_variables.clear();
        self.catch_stack.clear();
        self.catch_owner_roots.clear();
        self.seed_global_variables();

        let alloc = self.module.get_function("thaw_arena_alloc").unwrap();
        let frame = self
            .builder
            .build_call(
                alloc,
                &[
                    self.context
                        .i64_type()
                        .const_int(self.async_frame_size(plan)?, false)
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
        let frame_missing = self.builder.build_is_null(frame, "async_frame_is_null")
            .map_err(|error| error.to_string())?;
        let frame_failed = self.context.append_basic_block(ramp, "async_frame_allocation_failed");
        let frame_ready = self.context.append_basic_block(ramp, "async_frame_allocation_ready");
        self.builder.build_conditional_branch(frame_missing, frame_failed, frame_ready)
            .map_err(|error| error.to_string())?;
        self.builder.position_at_end(frame_failed);
        self.compile_throw_type_error("Cannot allocate async frame")?;
        self.builder.position_at_end(frame_ready);
        // A cancellation callback can be the frame's last live owner. Give
        // every generated catch an allocation-free deferred-cleanup node
        // before the first Promise or user callback can be created.
        for binding in &plan.generated_catch_bindings {
            let token_name = format!("{binding}__thaw_exception_cleanup_token");
            let index = plan.locals.iter().position(|(name, _)| name == &token_name)
                .ok_or_else(|| format!("missing async catch cleanup token `{token_name}`"))?;
            let slot = self.async_frame_field(frame, self.async_local_offset(plan, index)?,
                "async_catch_cleanup_token_slot")?;
            let token = self.builder.build_call(
                self.module.get_function("thaw_json_reserve_arena_owned_root").unwrap(),
                &[], "reserve_async_catch_cleanup_token",
            ).map_err(|error| error.to_string())?.try_as_basic_value().basic()
                .ok_or("async catch cleanup reservation returned no value")?.into_pointer_value();
            let ready = self.context.append_basic_block(ramp, "async_catch_token_ready");
            let failed = self.context.append_basic_block(ramp, "async_catch_token_oom");
            let missing = self.builder.build_is_null(token, "async_catch_token_missing")
                .map_err(|error| error.to_string())?;
            self.builder.build_conditional_branch(missing, failed, ready)
                .map_err(|error| error.to_string())?;
            self.builder.position_at_end(failed);
            self.compile_throw_type_error("Cannot reserve async catch cleanup")?;
            self.builder.position_at_end(ready);
            self.builder.build_store(slot, token).map_err(|error| error.to_string())?;
        }
        // The terminal path cannot allocate after a creator release starts:
        // Host destructors may reenter while this is the last frame root.
        // Reserve one frame-reachable snapshot/token/packet for each Promise
        // local before any Promise creator or source statement can run.
        let local_cleanup_oom = self.context.append_basic_block(ramp,
            "async_promise_local_cleanup_oom");
        for (index, (_, ty)) in plan.locals.iter().enumerate() {
            if !matches!(ty, HirType::Promise(_)) { continue; }
            let cleanup = self.reserve_async_promise_local_cleanup(local_cleanup_oom, false)?;
            let slot = self.async_frame_field(frame,
                self.async_promise_cleanup_offset(plan, index)?,
                "async_promise_local_cleanup_slot")?;
            self.builder.build_store(slot, cleanup).map_err(|error| error.to_string())?;
        }
        let local_cleanup_ready = self.builder.get_insert_block()
            .ok_or("async Promise cleanup reservation has no successor")?;
        self.builder.position_at_end(local_cleanup_oom);
        self.compile_throw_type_error("Cannot reserve async Promise cleanup")?;
        self.builder.position_at_end(local_cleanup_ready);
        // A rejection after the frame has resumed need not have a delivered
        // source Promise (for example, an explicit throw or an await operand
        // that throws before returning a Promise). Keep a separate terminal
        // snapshot with original/cause packets in the frame from before its
        // completion or any user callback exists. Local-owner retirement
        // spends different cells and cannot consume this authority.
        let terminal_oom = self.context.append_basic_block(ramp,
            "async_terminal_cleanup_oom");
        let terminal_cleanup = self.reserve_async_promise_local_cleanup(terminal_oom, true)?;
        let terminal_slot = self.async_frame_field(frame,
            ASYNC_TERMINAL_CLEANUP_OFFSET, "async_terminal_cleanup_slot")?;
        self.builder.build_store(terminal_slot, terminal_cleanup)
            .map_err(|error| error.to_string())?;
        let terminal_ready = self.builder.get_insert_block()
            .ok_or("async terminal reservation has no successor")?;
        self.builder.position_at_end(terminal_oom);
        self.compile_throw_type_error("Cannot reserve async terminal cleanup")?;
        self.builder.position_at_end(terminal_ready);
        // No newly created Promise exists yet. Preserve any exception that a
        // caller has already published, then reserve both cleanup packets
        // before finisher registration can fail and release Host leases.
        self.branch_on_pending_exception()?;
        let snapshot_oom = self.context.append_basic_block(ramp,
            "async_completion_snapshot_oom");
        let (saved_pending, pending_cell) =
            self.reserve_pending_exception_cleanup_snapshot(snapshot_oom)?;
        let completion = self
            .builder
            .build_call(
                self.module.get_function("thaw_promise_new_frame_with_creator_ticket").unwrap(),
                &[ramp_ticket_out.into()],
                "async_completion",
            )
            .map_err(|e| e.to_string())?
            .try_as_basic_value()
            .basic()
            .ok_or("promise allocator returned no value")?
            .into_pointer_value();
        let completion_missing = self.builder.build_is_null(completion, "async_completion_is_null")
            .map_err(|error| error.to_string())?;
        let completion_failed = self.context.append_basic_block(ramp, "async_completion_allocation_failed");
        let completion_ready = self.context.append_basic_block(ramp, "async_completion_allocation_ready");
        self.builder.build_conditional_branch(completion_missing, completion_failed, completion_ready)
            .map_err(|error| error.to_string())?;
        self.builder.position_at_end(snapshot_oom);
        self.compile_throw_type_error("Cannot reserve async completion cleanup")?;
        self.builder.position_at_end(completion_failed);
        self.restore_pending_exception_tuple(&saved_pending, pending_cell)?;
        self.compile_throw_type_error("Cannot allocate async completion")?;
        self.builder.position_at_end(completion_ready);
        let registration_failed = self.context.append_basic_block(ramp,
            "async_completion_registration_failed");
        let registration_ready = self.context.append_basic_block(ramp,
            "async_completion_registration_ready");
        self.catch_stack.push(registration_failed);
        self.compile_register_produced_promise_graph_finisher(completion, &plan.ret)?;
        self.catch_stack.pop();
        self.builder.build_unconditional_branch(registration_ready)
            .map_err(|error| error.to_string())?;
        self.builder.position_at_end(registration_failed);
        // The finisher's exact error becomes the owner of the first reserved
        // packet. Clear pending before the fresh Promise's two shares are
        // retired: their Host payload destruction may publish another error.
        let failure_packet = self.isolate_pending_exception_in_reserved_packet(pending_cell, 64)?;
        self.builder.build_store(ramp_ticket_out, self.context.i64_type().const_zero())
            .map_err(|error| error.to_string())?;
        self.builder.build_call(self.module.get_function("thaw_promise_destroy").unwrap(),
            &[completion.into()], "destroy_unregistered_async_completion")
            .map_err(|error| error.to_string())?;
        self.builder.build_call(self.module.get_function("thaw_promise_release_request_base").unwrap(),
            &[completion.into()], "release_unregistered_async_completion_base")
            .map_err(|error| error.to_string())?;
        self.defer_reentrant_pending_exception_tuple(pending_cell)?;
        self.restore_pending_exception_tuple(&saved_pending, pending_cell)?;
        self.publish_deferred_exception_packet(failure_packet)?;
        self.branch_on_pending_exception()?;
        self.builder.build_unreachable().map_err(|error| error.to_string())?;
        self.builder.position_at_end(registration_ready);
        self.restore_pending_exception_tuple(&saved_pending, pending_cell)?;
        let completion_slot =
            self.async_frame_field(frame, ASYNC_COMPLETION_OFFSET, "completion_slot")?;
        self.builder
            .build_store(completion_slot, completion)
            .map_err(|e| e.to_string())?;
        let waiting_slot = self.async_frame_field(frame, ASYNC_WAITING_OFFSET, "waiting_slot")?;
        self.builder
            .build_store(waiting_slot, ptr_ty.const_null())
            .map_err(|e| e.to_string())?;
        let waiting_ticket = self.async_frame_field(
            frame, ASYNC_WAITING_CREATOR_TICKET_OFFSET, "waiting_creator_ticket",
        )?;
        self.builder.build_store(waiting_ticket, self.context.i64_type().const_zero())
            .map_err(|error| error.to_string())?;
        let waiting_cleanup = self.async_frame_field(frame,
            ASYNC_WAITING_CREATOR_CLEANUP_OFFSET, "initial_waiting_creator_cleanup")?;
        self.builder.build_store(waiting_cleanup, ptr_ty.const_null())
            .map_err(|error| error.to_string())?;
        for (index, (name, _)) in plan.locals.iter().enumerate() {
            if name.starts_with("@@thaw_for_iteration_") {
                let slot = self.async_frame_field(
                    frame,
                    self.async_local_offset(plan, index)?,
                    &format!("frame_{name}"),
                )?;
                self.builder.build_store(slot, ptr_ty.const_null())
                    .map_err(|error| error.to_string())?;
            }
        }
        self.bind_async_frame_locals(frame, plan)?;
        for (index, (_, ty)) in plan.locals.iter().enumerate() {
            if matches!(ty, HirType::Promise(_)) {
                let ticket_slot = self.async_frame_field(
                    frame, self.async_promise_ticket_offset(plan, index)?,
                    "initial_local_creator_ticket",
                )?;
                self.builder.build_store(ticket_slot, self.context.i64_type().const_zero())
                    .map_err(|error| error.to_string())?;
            }
        }
        // Arena allocation is not zeroed outside tracing mode. Initialize
        // compiler-private provenance before any source statement executes.
        for (_, native_slot, original_slot, valid_slot) in self.catch_native_text.values() {
            for slot in [native_slot, original_slot] {
                self.builder.build_store(*slot, ptr_ty.const_null())
                    .map_err(|error| error.to_string())?;
            }
            self.builder.build_store(*valid_slot, self.context.bool_type().const_zero())
                .map_err(|error| error.to_string())?;
        }
        for binding in &plan.generated_catch_bindings {
            for suffix in ["json_owner", "native_text_owner"] {
                let name = format!("{binding}__thaw_exception_{suffix}");
                let (slot, _) = self.variables.get(&name)
                    .ok_or_else(|| format!("missing async catch owner cell `{name}`"))?;
                self.builder.build_store(*slot, ptr_ty.const_null())
                    .map_err(|error| error.to_string())?;
            }
        }
        // A checked Promise-valued frame write may synthesize a TypeError.
        // Give that failure the already-created completion before storing
        // any parameter; otherwise the ramp would return a null pointer and
        // leave this completion pending.
        let saved_async_completion = self.active_async_completion.replace(completion);
        for (index, (param_value, param)) in ramp.get_param_iter().zip(&func.params).enumerate() {
            if index < plan.captures.len() {
                let capture_slot = self.async_frame_field(
                    frame,
                    self.async_capture_offset(plan, index)?,
                    &format!("capture_{}", param.name),
                )?;
                self.builder
                    .build_store(capture_slot, param_value)
                    .map_err(|e| e.to_string())?;
                self.variables.insert(
                    param.name.clone(),
                    (param_value.into_pointer_value(), self.basic_type(&param.ty)?),
                );
                self.variable_hir_types
                    .insert(param.name.clone(), param.ty.clone());
                continue;
            }
            let (slot, _) = self.variables.get(&param.name).copied().ok_or_else(|| {
                format!("missing async frame parameter slot for `{}`", param.name)
            })?;
            self.retain_native_promise_cell_value(slot, &param.ty, param_value)?;
            self.builder
                .build_store(slot, param_value)
                .map_err(|e| e.to_string())?;
        }
        self.active_async_completion = saved_async_completion;
        self.emit_async_segment(&segments[0], frame, completion, resume, cancel, 1, true, plan, None)?;

        let resume_entry = self.context.append_basic_block(resume, "entry");
        self.builder.position_at_end(resume_entry);
        self.variables.clear();
        self.for_iteration_frame_slots.clear();
        self.catch_native_text.clear();
        self.variable_hir_types.clear();
        self.arena_variables.clear();
        self.catch_stack.clear();
        self.catch_owner_roots.clear();
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
        let resume_ok = self.context.append_basic_block(resume, "resume_fulfilled");
        let inspect_waiting = self.context.append_basic_block(resume, "inspect_waiting");
        let synchronous = self.builder.build_is_null(waiting, "synchronous_transition")
            .map_err(|e| e.to_string())?;
        self.builder.build_conditional_branch(synchronous, resume_ok, inspect_waiting)
            .map_err(|e| e.to_string())?;
        self.builder.position_at_end(inspect_waiting);
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
                    self.async_local_offset(plan, index)?,
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
                    self.async_local_offset(plan, index)?,
                    &format!("frame_{name}"),
                )?;
                self.builder
                    .build_store(slot, self.context.bool_type().const_zero())
                    .map_err(|e| e.to_string())?;
            }
            let binding_slot = self.catch_native_text.get(&handler.catch_binding)
                .map(|(binding, _, _, _)| *binding)
                .ok_or_else(|| format!("missing catch provenance for `{}`", handler.catch_binding))?;
            if let Some(source) = &handler.rethrow_source_binding {
                self.retire_async_catch_owner(source)?;
            }
            // A prior traversal may have left this catch abruptly via a loop
            // edge. Retire its old frame owner before replacing the slot.
            self.retire_async_catch_owner(&handler.catch_binding)?;
            // The waiting Promise may retire before this catch body runs.
            // Acquire a distinct arena-rooted tag-7 Box while it is live;
            // the ordinary object getter only returns a borrowed pointer.
            let transfer_failed = self.context.append_basic_block(resume,
                "caught_promise_transfer_failed");
            let cleanup_ready = self.ensure_async_catch_cleanup_token(&handler.catch_binding)?;
            let token_available = self.context.append_basic_block(resume,
                "caught_promise_cleanup_token_available");
            let token_missing = self.context.append_basic_block(resume,
                "caught_promise_cleanup_token_oom");
            self.builder.build_conditional_branch(cleanup_ready, token_available, token_missing)
                .map_err(|error| error.to_string())?;
            self.builder.position_at_end(token_missing);
            self.catch_stack.push(transfer_failed);
            self.compile_check_json_host_error(ptr_ty.const_null().into(), None)?;
            self.catch_stack.pop();
            self.compile_throw_type_error("Cannot reserve caught exception cleanup")?;
            self.builder.position_at_end(token_available);
            let tag = self.builder.build_call(
                self.module.get_function("thaw_promise_exception_tag").unwrap(),
                &[waiting.into()], "caught_promise_tag",
            ).map_err(|error| error.to_string())?.try_as_basic_value().basic()
                .ok_or("caught Promise tag returned no value")?.into_int_value();
            let is_json = self.builder.build_int_compare(inkwell::IntPredicate::EQ, tag,
                self.context.i64_type().const_int(7, false), "caught_promise_is_json")
                .map_err(|error| error.to_string())?;
            let json_transfer = self.context.append_basic_block(resume,
                "caught_promise_share_json");
            let native_object = self.context.append_basic_block(resume,
                "caught_promise_borrow_native_object");
            let transfer_ready = self.context.append_basic_block(resume,
                "caught_json_owner_ready");
            self.builder.build_conditional_branch(is_json, json_transfer, native_object)
                .map_err(|error| error.to_string())?;
            self.builder.position_at_end(json_transfer);
            self.catch_stack.push(transfer_failed);
            let transferred = self.builder.build_call(
                self.module.get_function("thaw_promise_exception_pending_object").unwrap(),
                &[waiting.into()], "caught_promise_owned_object",
            ).map_err(|error| error.to_string())?.try_as_basic_value().basic()
                .ok_or("caught Promise object transfer returned no value")?;
            let transferred = self.compile_check_json_host_error(
                transferred, Some("thaw_json_destroy"))?.into_pointer_value();
            let missing = self.builder.build_is_null(transferred, "caught_json_transfer_missing")
                .map_err(|error| error.to_string())?;
            let missing_block = self.context.append_basic_block(resume,
                "caught_json_owner_missing_error");
            let json_ready = self.context.append_basic_block(resume,
                "caught_json_share_ready");
            self.builder.build_conditional_branch(missing, missing_block, json_ready)
                .map_err(|error| error.to_string())?;
            self.builder.position_at_end(missing_block);
            self.compile_throw_type_error("Unable to retain caught JavaScript exception")?;
            self.builder.position_at_end(json_ready);
            self.catch_stack.pop();
            let json_from = self.builder.get_insert_block().ok_or("missing caught JSON branch")?;
            self.builder.build_unconditional_branch(transfer_ready).map_err(|error| error.to_string())?;
            self.builder.position_at_end(native_object);
            let borrowed = self.builder.build_call(
                self.module.get_function("thaw_promise_exception_object").unwrap(),
                &[waiting.into()], "caught_promise_native_object",
            ).map_err(|error| error.to_string())?.try_as_basic_value().basic()
                .ok_or("caught Promise object getter returned no value")?.into_pointer_value();
            self.builder.build_unconditional_branch(transfer_ready).map_err(|error| error.to_string())?;
            self.builder.position_at_end(transfer_ready);
            let selected = self.builder.build_phi(ptr_ty, "caught_promise_object_value")
                .map_err(|error| error.to_string())?;
            selected.add_incoming(&[(&transferred, json_from), (&borrowed, native_object)]);
            let transferred = selected.as_basic_value().into_pointer_value();
            let json_owner_name = format!("{}__thaw_exception_json_owner", handler.catch_binding);
            let (json_owner_slot, _) = self.variables.get(&json_owner_name).copied()
                .ok_or_else(|| format!("missing async catch owner cell `{json_owner_name}`"))?;
            let json_owner = self.builder.build_select(is_json, transferred, ptr_ty.const_null(),
                "caught_promise_json_owner").map_err(|error| error.to_string())?;
            self.builder.build_store(json_owner_slot, json_owner).map_err(|error| error.to_string())?;
            let native_text = self.builder.build_call(
                self.module.get_function("thaw_promise_exception_native_text_copy").unwrap(),
                &[waiting.into()], "caught_promise_native_text",
            ).map_err(|e| e.to_string())?.try_as_basic_value().basic()
                .ok_or("native text copy returned no value")?.into_pointer_value();
            let has_native_text = self.builder.build_is_not_null(native_text, "caught_native_text_present")
                .map_err(|e| e.to_string())?;
            let scalar_or_text = self.builder.build_select(
                has_native_text, native_text, resume_result, "caught_promise_scalar_or_text",
            ).map_err(|e| e.to_string())?;
            // A canonicalized Error can keep its original display text, but
            // tag 7 is the exact caught value and must win over that text.
            let caught_value = self.builder.build_select(
                is_json, transferred, scalar_or_text, "caught_promise_value",
            ).map_err(|e| e.to_string())?;
            self.builder.build_store(binding_slot, caught_value)
                .map_err(|e| e.to_string())?;
            let original_slot = self.catch_native_text.get(&handler.catch_binding)
                .map(|(_, _, original, _)| *original)
                .ok_or_else(|| format!("missing catch original for `{}`", handler.catch_binding))?;
            self.builder.build_store(original_slot, caught_value)
                .map_err(|e| e.to_string())?;
            let valid_slot = self.catch_native_text.get(&handler.catch_binding)
                .map(|(_, _, _, valid)| *valid)
                .ok_or_else(|| format!("missing catch validity for `{}`", handler.catch_binding))?;
            self.builder.build_store(valid_slot, self.context.bool_type().const_int(1, false))
                .map_err(|e| e.to_string())?;
            let native_slot = self.catch_native_text.get(&handler.catch_binding)
                .map(|(_, native, _, _)| *native)
                .ok_or_else(|| format!("missing catch provenance for `{}`", handler.catch_binding))?;
            self.builder.build_store(native_slot, native_text).map_err(|e| e.to_string())?;
            let object_name = format!("{}__thaw_exception_object", handler.catch_binding);
            let (object_slot, _) = self.variables.get(&object_name).copied()
                .ok_or_else(|| format!("missing async catch object cell `{object_name}`"))?;
            self.builder.build_store(object_slot, transferred).map_err(|error| error.to_string())?;
            for (suffix, getter) in [
                ("aggregate", "thaw_promise_exception_aggregate_errors"),
                ("tag", "thaw_promise_exception_tag"),
                ("f64", "thaw_promise_exception_f64"),
                ("i64", "thaw_promise_exception_i64"),
                ("bool", "thaw_promise_exception_bool"),
            ] {
                let name = format!("{}__thaw_exception_{suffix}", handler.catch_binding);
                let index = plan
                    .locals
                    .iter()
                    .position(|(local, _)| local == &name)
                    .ok_or_else(|| format!("missing async catch metadata `{name}`"))?;
                let slot = self.async_frame_field(
                    resume_frame,
                    self.async_local_offset(plan, index)?,
                    &format!("frame_{name}"),
                )?;
                let value = self
                    .builder
                    .build_call(
                        self.module.get_function(getter).unwrap(),
                        &[waiting.into()],
                        "caught_promise_exception_value",
                    )
                    .map_err(|e| e.to_string())?
                    .try_as_basic_value()
                    .basic()
                    .ok_or_else(|| format!("{getter} returned no value"))?;
                self.builder
                    .build_store(slot, value)
                    .map_err(|e| e.to_string())?;
            }
            self.builder
                .build_store(waiting_slot, ptr_ty.const_null())
                .map_err(|e| e.to_string())?;
            self.consume_async_waiting_creator_ticket(resume_frame)?;
            self.builder
                .build_call(
                    resume,
                    &[resume_frame.into(), resume_frame.into()],
                    "resume_catch",
                )
                .map_err(|e| e.to_string())?;
            self.builder.build_return(None).map_err(|e| e.to_string())?;
            self.builder.position_at_end(transfer_failed);
            let original_error = self.builder.build_load(ptr_ty,
                self.pending_exception().as_pointer_value(), "caught_transfer_error")
                .map_err(|error| error.to_string())?.into_pointer_value();
            // The exact HostError remains pending until the completion owns
            // it. On status zero, a void resume cannot return that failure;
            // transfer the delivered source share and the causal pending
            // tuple to the two packets reserved before subscription.
            let settled = self.reject_promise_with_pending_exception_status(
                resume_completion, original_error, "reject_caught_transfer_failure")?;
            self.discard_pending_owned_native_text_after_settlement(settled)?;
            let accepted = self.builder.build_int_compare(IntPredicate::NE,
                settled, settled.get_type().const_zero(), "caught_transfer_failure_settled")
                .map_err(|error| error.to_string())?;
            let queue_failed = self.context.append_basic_block(resume,
                "queue_caught_transfer_failure_source");
            let catch_done = self.context.append_basic_block(resume,
                "caught_transfer_failure_disposed");
            self.builder.build_conditional_branch(accepted, catch_done, queue_failed)
                .map_err(|error| error.to_string())?;
            self.builder.position_at_end(queue_failed);
            self.queue_failed_delivered_source(resume_completion, waiting)?;
            self.builder.build_unconditional_branch(catch_done)
                .map_err(|error| error.to_string())?;
            self.builder.position_at_end(catch_done);
            self.builder.build_store(waiting_slot, ptr_ty.const_null())
                .map_err(|error| error.to_string())?;
            self.consume_async_waiting_creator_ticket(resume_frame)?;
            self.builder.build_return(None).map_err(|error| error.to_string())?;
        }

        self.builder.position_at_end(propagate_rejection);
        let forwarded = self.builder.build_call(
            self.module.get_function("thaw_promise_forward_rejection").unwrap(),
            &[resume_completion.into(), waiting.into(), resume_result.into()],
            "reject_completion",
        ).map_err(|error| error.to_string())?.try_as_basic_value().basic()
            .ok_or("source rejection returned no settlement status")?.into_int_value();
        let source_forwarded = self.builder.build_int_compare(IntPredicate::NE,
            forwarded, forwarded.get_type().const_zero(), "source_rejection_forwarded")
            .map_err(|error| error.to_string())?;
        let borrow_source = self.context.append_basic_block(resume,
            "transfer_exact_rejection_source_after_share_failure");
        let source_ready = self.context.append_basic_block(resume,
            "rejection_source_disposition_ready");
        self.builder.build_conditional_branch(source_forwarded, source_ready, borrow_source)
            .map_err(|error| error.to_string())?;
        self.builder.position_at_end(borrow_source);
        // poll_one still owns the delivered subscription's source and
        // completion shares. If a Host Box share failed, first try moving
        // the source claim into the completion. On status zero, hand it to
        // the packet reserved before subscription publication; the void
        // resume must not return with a pending completion and drop the only
        // exact-reason claim.
        let transferred = self.builder.build_call(
            self.module.get_function("thaw_promise_reject_borrowed_source_from_subscription").unwrap(),
            &[resume_completion.into(), waiting.into()], "transfer_exact_rejection_source",
        ).map_err(|error| error.to_string())?.try_as_basic_value().basic()
            .ok_or("exact source transfer returned no status")?.into_int_value();
        let transfer_ok = self.builder.build_int_compare(IntPredicate::NE,
            transferred, transferred.get_type().const_zero(), "exact_source_transfer_ok")
            .map_err(|error| error.to_string())?;
        let queue_source = self.context.append_basic_block(resume,
            "queue_exact_source_after_transfer_failure");
        self.builder.build_conditional_branch(transfer_ok, source_ready, queue_source)
            .map_err(|error| error.to_string())?;
        self.builder.position_at_end(queue_source);
        self.queue_failed_delivered_source(resume_completion, waiting)?;
        self.builder.build_unconditional_branch(source_ready)
            .map_err(|error| error.to_string())?;
        self.builder.position_at_end(source_ready);
        self.builder
            .build_store(waiting_slot, ptr_ty.const_null())
            .map_err(|e| e.to_string())?;
        self.consume_async_waiting_creator_ticket(resume_frame)?;
        self.builder.build_return(None).map_err(|e| e.to_string())?;

        self.builder.position_at_end(resume_ok);
        self.builder
            .build_store(waiting_slot, ptr_ty.const_null())
            .map_err(|e| e.to_string())?;
        self.consume_async_waiting_creator_ticket(resume_frame)?;
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
            self.for_iteration_frame_slots.clear();
            self.catch_native_text.clear();
            self.variable_hir_types.clear();
            self.arena_variables.clear();
            self.catch_stack.clear();
            self.catch_owner_roots.clear();
            self.seed_global_variables();
            self.bind_async_frame_captures(resume_frame, plan)?;
            self.bind_async_frame_locals(resume_frame, plan)?;
            self.emit_async_segment(
                &segments[index + 1],
                resume_frame,
                resume_completion,
                resume,
                cancel,
                index + 2,
                false,
                plan,
                Some(resume_result),
            )?;
        }
        // PromiseSubscription::drop invokes cancel before dropping its
        // ArenaRoot and source/completion shares. The frame is therefore
        // still reachable while the same owner-retirement helper runs here.
        let cancel_entry = self.context.append_basic_block(cancel, "entry");
        self.builder.position_at_end(cancel_entry);
        self.variables.clear();
        self.for_iteration_frame_slots.clear();
        self.catch_native_text.clear();
        self.variable_hir_types.clear();
        self.arena_variables.clear();
        self.catch_stack.clear();
        self.catch_owner_roots.clear();
        self.seed_global_variables();
        let cancel_frame = cancel.get_nth_param(0).unwrap().into_pointer_value();
        self.bind_async_frame_locals(cancel_frame, plan)?;
        let cancel_waiting = self.async_frame_field(cancel_frame, ASYNC_WAITING_OFFSET,
            "cancel_waiting_slot")?;
        self.builder.build_store(cancel_waiting, ptr_ty.const_null())
            .map_err(|error| error.to_string())?;
        self.consume_async_waiting_creator_ticket(cancel_frame)?;
        self.retire_all_async_frame_owners(plan, cancel_frame)?;
        self.builder.build_return(None).map_err(|error| error.to_string())?;
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
        cancel: FunctionValue<'ctx>,
        next_state: usize,
        ramp: bool,
        plan: &FrameAsyncPlan,
        resume_result: Option<PointerValue<'ctx>>,
    ) -> Result<(), String> {
        let saved_async_completion = self.active_async_completion.replace(completion);
        if let Some((name, ty)) = &segment.resume_target {
            let llvm_ty = self.basic_type(ty)?;
            // Promise<void> settles with a null payload. The resumed HIR
            // binding still needs its normal frame slot, but no payload may
            // be loaded from the null pointer.
            let value = if *ty == HirType::Void {
                llvm_ty.const_zero()
            } else {
                let result_ptr = resume_result.ok_or("async resume result is unavailable")?;
                self.builder
                    .build_load(llvm_ty, result_ptr, &format!("awaited_{name}"))
                    .map_err(|e| e.to_string())?
            };
            let (slot, _) = self
                .variables
                .get(name)
                .copied()
                .ok_or_else(|| format!("missing async frame slot for `{name}`"))?;
            self.retain_native_promise_cell_value(slot, ty, value)?;
            self.builder
                .build_store(slot, value)
                .map_err(|e| e.to_string())?;
        }
        let block_result =
            self.compile_async_segment_block(&segment.stmts, frame, completion, plan);
        self.active_async_completion = saved_async_completion;
        match block_result? {
            AsyncBlockExit::Returned => {
                self.retire_all_async_frame_owners(plan, frame)?;
                self.resolve_async_completion(
                    completion,
                    self.async_completion_result(frame, plan)?,
                    ramp,
                )?;
                return Ok(());
            }
            exit @ (AsyncBlockExit::Rejected | AsyncBlockExit::RejectedRetired) => {
                if matches!(exit, AsyncBlockExit::Rejected) {
                    self.retire_all_async_frame_owners(plan, frame)?;
                }
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
            // Evaluating the operand of `await` can itself throw before it
            // produces a native Promise (notably when a QuickJS-backed call
            // returns a rejected JavaScript Promise). JavaScript turns that
            // into an awaited rejection, so route the existing exception
            // slot through the same resume/rejection machinery instead of
            // returning early from the async ramp or resume function.
            let function = self.current_function();
            let sync_rejection = self
                .context
                .append_basic_block(function, "await_operand_rejected");
            self.catch_stack.push(sync_rejection);
            // A frame can suspend and resume through more than one await.
            // Reserve a new original packet and a separate reentry packet
            // before each operand may mint a creator. The frame-header
            // terminal packet remains available for later frame states after
            // this awaited rejection succeeds and execution continues.
            let cleanup_oom = self.context.append_basic_block(function,
                "await_creator_cleanup_oom");
            let cleanup = self.reserve_async_promise_local_cleanup(cleanup_oom, true)?;
            let cleanup_slot = self.async_frame_field(frame,
                ASYNC_WAITING_CREATOR_CLEANUP_OFFSET, "waiting_creator_cleanup_reservation")?;
            self.builder.build_store(cleanup_slot, cleanup)
                .map_err(|error| error.to_string())?;
            let cleanup_ready = self.builder.get_insert_block()
                .ok_or("await cleanup reservation has no successor")?;
            self.builder.position_at_end(cleanup_oom);
            let cleanup_failure = self.context.append_basic_block(function,
                "await_cleanup_reservation_failed");
            self.catch_stack.push(cleanup_failure);
            self.compile_throw_type_error("Cannot reserve awaited Promise cleanup")?;
            self.catch_stack.pop();
            self.builder.position_at_end(cleanup_failure);
            // No new awaited Promise exists. Do not enter sync_rejection,
            // which would mint a creator using a previously consumed packet.
            let error = self.builder.build_load(
                self.context.ptr_type(AddressSpace::default()),
                self.pending_exception().as_pointer_value(),
                "await_cleanup_reservation_error",
            ).map_err(|error| error.to_string())?.into_pointer_value();
            let settled = self.reject_promise_with_pending_exception_status(
                completion, error, "reject_await_cleanup_reservation",
            )?;
            self.discard_pending_owned_native_text_after_settlement(settled)?;
            let accepted = self.builder.build_int_compare(IntPredicate::NE,
                settled, settled.get_type().const_zero(), "await_cleanup_failure_settled")
                .map_err(|error| error.to_string())?;
            let terminal = self.context.append_basic_block(function,
                "await_cleanup_rejection_terminal");
            let completed = self.context.append_basic_block(function,
                "await_cleanup_rejection_completed");
            self.builder.build_conditional_branch(accepted, completed, terminal)
                .map_err(|error| error.to_string())?;
            self.builder.position_at_end(terminal);
            // This generated TypeError has a borrowed, stable native-text
            // producer; it does not depend on a source Promise share. Leave
            // frame locals intact until the fatal exact packet is emitted.
            self.queue_frame_terminal_pending_exception(frame)?;
            if ramp {
                self.builder.build_return(Some(&completion)).map_err(|error| error.to_string())?;
            } else {
                self.builder.build_return(None).map_err(|error| error.to_string())?;
            }
            self.builder.position_at_end(completed);
            self.retire_all_async_frame_owners(plan, frame)?;
            if ramp {
                self.builder.build_return(Some(&completion)).map_err(|error| error.to_string())?;
            } else {
                self.builder.build_return(None).map_err(|error| error.to_string())?;
            }
            self.builder.position_at_end(cleanup_ready);
            let waiting = match awaited {
                HirExpr::Call(callee, args) if matches!(callee.as_ref(), HirExpr::Var(name) if name == "fetch") => {
                    self.compile_single_arg_call("thaw_http_get_async", args, "async_fetch")
                }
                _ => self.compile_expr(awaited),
            };
            self.catch_stack.pop();
            let waiting = waiting?.into_pointer_value();
            let fulfilled_ticket = self.promise_creator_tickets.get(&waiting).copied()
                .unwrap_or_else(|| self.context.i64_type().const_zero());
            self.clear_promise_creator_ticket_source(waiting)?;
            let fulfilled_block = self.builder.get_insert_block().unwrap();
            let await_ready = self.context.append_basic_block(function, "await_operand_ready");
            self.builder
                .build_unconditional_branch(await_ready)
                .map_err(|e| e.to_string())?;

            self.builder.position_at_end(sync_rejection);
            let ptr_ty = self.context.ptr_type(AddressSpace::default());
            let error = self
                .builder
                .build_load(
                    ptr_ty,
                    self.pending_exception().as_pointer_value(),
                    "await_operand_error",
                )
                .map_err(|e| e.to_string())?
                .into_pointer_value();
            let rejected_ticket_slot = self.builder.build_alloca(
                self.context.i64_type(), "rejected_await_creator_ticket",
            ).map_err(|error| error.to_string())?;
            let rejected = self
                .builder
                .build_call(
                    self.module.get_function("thaw_promise_new_with_creator_ticket").unwrap(),
                    &[rejected_ticket_slot.into()],
                    "await_rejected_promise",
                )
                .map_err(|e| e.to_string())?
                .try_as_basic_value()
                .basic()
                .ok_or("thaw_promise_new returned no value")?
                .into_pointer_value();
            let rejected_missing = self.builder.build_is_null(rejected,
                "rejected_await_promise_missing").map_err(|error| error.to_string())?;
            let rejected_ready = self.context.append_basic_block(function,
                "rejected_await_promise_ready");
            let rejected_oom = self.context.append_basic_block(function,
                "rejected_await_promise_oom");
            self.builder.build_conditional_branch(rejected_missing, rejected_oom, rejected_ready)
                .map_err(|error| error.to_string())?;
            self.builder.position_at_end(rejected_oom);
            // No creator was published. The original thrown operand is
            // still the pending exact tuple; its frame-terminal packet was
            // reserved before this allocation attempt and survives reset.
            self.queue_frame_terminal_pending_exception(frame)?;
            if ramp {
                self.builder.build_return(Some(&completion)).map_err(|error| error.to_string())?;
            } else {
                self.builder.build_return(None).map_err(|error| error.to_string())?;
            }
            self.builder.position_at_end(rejected_ready);
            let rejected_ticket = self.builder.build_load(
                self.context.i64_type(), rejected_ticket_slot, "rejected_await_ticket",
            ).map_err(|error| error.to_string())?.into_int_value();
            // The rejected Promise is not published yet. Capture its exact
            // operand throw before a tag-7 share or settlement callback can
            // replace pending with a new causal HOST_ERROR. The await's own
            // cell, not the frame-header terminal cell, owns this packet.
            let original_field = self.async_frame_field(cleanup, 64,
                "await_original_packet_field")?;
            let original_packet = self.builder.build_load(ptr_ty, original_field,
                "await_original_packet").map_err(|e| e.to_string())?
                .into_pointer_value();
            let root = self.http_saved_exception_root();
            let previous = self.builder.build_load(ptr_ty, root.as_pointer_value(),
                "await_previous_exception_root").map_err(|e| e.to_string())?;
            self.builder.build_store(cleanup, previous).map_err(|e| e.to_string())?;
            let frame_root = self.async_frame_field(cleanup, 80,
                "await_rooted_frame_field")?;
            self.builder.build_store(frame_root, frame).map_err(|e| e.to_string())?;
            self.builder.build_store(root.as_pointer_value(), cleanup)
                .map_err(|e| e.to_string())?;
            self.transfer_matching_async_catch_json_owner(plan)?;
            self.copy_pending_exception_to_deferred_packet(original_packet)?;
            let original_value = self.async_frame_field(original_packet, 8,
                "await_original_value_field")?;
            self.builder.build_store(original_value, error).map_err(|e| e.to_string())?;
            // Both tokens now belong to the rooted original packet; the
            // runtime still reads the borrowed value/typed channels below.
            self.builder.build_store(self.pending_exception_json_owner().as_pointer_value(),
                ptr_ty.const_null()).map_err(|e| e.to_string())?;
            let settled = self.reject_promise_with_pending_exception_status_isolated(
                rejected, error, "reject_await_operand")?;
            let settled_ok = self.builder.build_int_compare(IntPredicate::NE, settled,
                settled.get_type().const_zero(), "rejected_await_settled")
                .map_err(|e| e.to_string())?;
            let rejected_settled = self.context.append_basic_block(function,
                "rejected_await_settled_successfully");
            let rejected_failed = self.context.append_basic_block(function,
                "rejected_await_settlement_failed");
            self.builder.build_conditional_branch(settled_ok, rejected_settled, rejected_failed)
                .map_err(|e| e.to_string())?;
            self.builder.position_at_end(rejected_failed);
            self.builder.build_store(original_field, ptr_ty.const_null())
                .map_err(|e| e.to_string())?;
            self.builder.build_call(self.module.get_function("thaw_json_enqueue_deferred_exception_packet").unwrap(),
                &[original_packet.into()], "queue_original_failed_await")
                .map_err(|e| e.to_string())?;
            self.defer_reentrant_pending_exception_tuple_at(cleanup, 96, 88)?;
            // Only this unexposed fresh producer owns its creator/base. Keep
            // the registered cleanup cell linked while final release can
            // publish one further exact reentrant exception in byte 72.
            self.builder.build_call(self.module.get_function("thaw_http_mark_unhandled_error").unwrap(),
                &[], "mark_failed_await_terminal")
                .map_err(|e| e.to_string())?;
            self.builder.build_call(self.module.get_function("thaw_promise_destroy_creator_ticket").unwrap(),
                &[rejected_ticket.into()], "release_failed_await_creator_ticket")
                .map_err(|e| e.to_string())?;
            self.builder.build_call(self.module.get_function("thaw_promise_release_request_base").unwrap(),
                &[rejected.into()], "release_failed_await_request_base")
                .map_err(|e| e.to_string())?;
            self.defer_reentrant_pending_exception_tuple_at(cleanup, 72, 56)?;
            if ramp {
                self.builder.build_return(Some(&completion)).map_err(|e| e.to_string())?;
            } else {
                self.builder.build_return(None).map_err(|e| e.to_string())?;
            }
            self.builder.position_at_end(rejected_settled);
            // Settlement has an independent Json share and exact text copy.
            // Retire the producer's original Box at the deferred boundary;
            // byte 56 remains available for the later creator release.
            let owner_field = self.async_frame_field(original_packet, 32,
                "await_original_json_owner_field")?;
            let owner = self.builder.build_load(ptr_ty, owner_field,
                "await_original_json_owner").map_err(|e| e.to_string())?
                .into_pointer_value();
            self.builder.build_store(owner_field, ptr_ty.const_null())
                .map_err(|e| e.to_string())?;
            let owner_token_field = self.async_frame_field(cleanup, 88,
                "await_original_owner_token_field")?;
            let owner_token = self.builder.build_load(ptr_ty, owner_token_field,
                "await_original_owner_token").map_err(|e| e.to_string())?
                .into_pointer_value();
            self.builder.build_call(self.module.get_function("thaw_json_enqueue_reserved_cleanup").unwrap(),
                &[owner.into(), owner_token.into()], "defer_await_original_json_owner")
                .map_err(|e| e.to_string())?;
            self.destroy_reporter_owned_native_text(original_packet)?;
            // The packet is no longer a root for a handled rejection.
            for offset in [8_u64, 16, 24, 40] {
                let field = self.async_frame_field(original_packet, offset,
                    "clear_handled_await_alias")?;
                self.builder.build_store(field, ptr_ty.const_null())
                    .map_err(|e| e.to_string())?;
            }
            self.builder.build_store(original_field, ptr_ty.const_null())
                .map_err(|e| e.to_string())?;
            let post_settle_cause = self.builder.build_load(ptr_ty,
                self.pending_exception().as_pointer_value(), "accepted_await_new_cause")
                .map_err(|e| e.to_string())?.into_pointer_value();
            let post_settle_json_owner = self.builder.build_load(ptr_ty,
                self.pending_exception_json_owner().as_pointer_value(),
                "accepted_await_new_json_owner")
                .map_err(|e| e.to_string())?.into_pointer_value();
            let post_settle_text_owner = self.builder.build_load(ptr_ty,
                self.pending_exception_native_text_owner().as_pointer_value(),
                "accepted_await_new_text_owner")
                .map_err(|e| e.to_string())?.into_pointer_value();
            let has_primary = self.builder.build_is_not_null(post_settle_cause,
                "accepted_await_has_new_cause").map_err(|e| e.to_string())?;
            let has_json_owner = self.builder.build_is_not_null(post_settle_json_owner,
                "accepted_await_has_new_json_owner").map_err(|e| e.to_string())?;
            let has_text_owner = self.builder.build_is_not_null(post_settle_text_owner,
                "accepted_await_has_new_text_owner").map_err(|e| e.to_string())?;
            let has_any_owner = self.builder.build_or(has_json_owner, has_text_owner,
                "accepted_await_has_new_owner").map_err(|e| e.to_string())?;
            let has_post_settle_cause = self.builder.build_or(has_primary, has_any_owner,
                "accepted_await_has_new_exception").map_err(|e| e.to_string())?;
            let accepted_cause = self.context.append_basic_block(function,
                "accepted_await_settlement_cause");
            let accepted_clean = self.context.append_basic_block(function,
                "accepted_await_without_cause");
            self.builder.build_conditional_branch(has_post_settle_cause,
                accepted_cause, accepted_clean).map_err(|e| e.to_string())?;
            self.builder.position_at_end(accepted_cause);
            self.defer_reentrant_pending_exception_tuple_at(cleanup, 96, 56)?;
            self.builder.build_call(self.module.get_function("thaw_http_mark_unhandled_error").unwrap(),
                &[], "mark_accepted_await_cause_terminal")
                .map_err(|e| e.to_string())?;
            self.builder.build_call(self.module.get_function("thaw_promise_destroy_creator_ticket").unwrap(),
                &[rejected_ticket.into()], "release_accepted_await_creator_ticket")
                .map_err(|e| e.to_string())?;
            self.builder.build_call(self.module.get_function("thaw_promise_release_request_base").unwrap(),
                &[rejected.into()], "release_accepted_await_request_base")
                .map_err(|e| e.to_string())?;
            self.defer_reentrant_pending_exception_tuple_at(cleanup, 72, 104)?;
            if ramp {
                self.builder.build_return(Some(&completion)).map_err(|e| e.to_string())?;
            } else {
                self.builder.build_return(None).map_err(|e| e.to_string())?;
            }
            self.builder.position_at_end(accepted_clean);
            self.restore_pending_exception_tuple(&[], cleanup)?;
            let rejected_block = self.builder.get_insert_block().unwrap();
            self.builder
                .build_unconditional_branch(await_ready)
                .map_err(|e| e.to_string())?;

            self.builder.position_at_end(await_ready);
            let waiting_phi = self
                .builder
                .build_phi(ptr_ty, "await_operand")
                .map_err(|e| e.to_string())?;
            waiting_phi.add_incoming(&[(&waiting, fulfilled_block), (&rejected, rejected_block)]);
            let waiting = waiting_phi.as_basic_value().into_pointer_value();
            let ticket_phi = self.builder.build_phi(
                self.context.i64_type(), "await_creator_ticket",
            ).map_err(|error| error.to_string())?;
            ticket_phi.add_incoming(&[
                (&fulfilled_ticket, fulfilled_block), (&rejected_ticket, rejected_block),
            ]);
            let waiting_slot =
                self.async_frame_field(frame, ASYNC_WAITING_OFFSET, "waiting_slot")?;
            self.builder
                .build_store(waiting_slot, waiting)
                .map_err(|e| e.to_string())?;
            let ticket_slot = self.async_frame_field(
                frame, ASYNC_WAITING_CREATOR_TICKET_OFFSET, "await_creator_ticket_slot",
            )?;
            self.builder.build_store(ticket_slot, ticket_phi.as_basic_value())
                .map_err(|error| error.to_string())?;
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
            let subscribed = self.builder
                .build_call(
                    self.module.get_function("thaw_promise_subscribe_with_cancel_and_completion").unwrap(),
                    &[
                        waiting.into(),
                        resume.as_global_value().as_pointer_value().into(),
                        cancel.as_global_value().as_pointer_value().into(),
                        frame.into(),
                        completion.into(),
                    ],
                    "subscribe_resume",
                )
                .map_err(|e| e.to_string())?.try_as_basic_value().basic()
                .ok_or("Promise subscription returned no status")?.into_int_value();
            let subscription_ready = self.builder.build_int_compare(
                IntPredicate::NE, subscribed, self.context.i8_type().const_zero(),
                "async_subscription_ready",
            ).map_err(|e| e.to_string())?;
            let subscribe_failed = self.context.append_basic_block(function, "async_subscribe_failed");
            let subscribe_ready = self.context.append_basic_block(function, "async_subscribe_ready");
            self.builder.build_conditional_branch(subscription_ready, subscribe_ready, subscribe_failed)
                .map_err(|e| e.to_string())?;
            self.builder.position_at_end(subscribe_failed);
            // No subscription owns this frame. Retire only a producer-carried
            // creator ticket; a borrowed alias has a zero ticket.
            self.builder.build_store(waiting_slot, ptr_ty.const_null())
                .map_err(|e| e.to_string())?;
            self.consume_async_waiting_creator_ticket(frame)?;
            // No subscription will call either resume or cancel. Retire the
            // frame's catch owners before returning its completion pointer.
            self.retire_all_async_frame_owners(plan, frame)?;
            let message = self.builder.build_global_string_ptr(
                "Task is ending", "async_subscribe_failure_text",
            ).map_err(|e| e.to_string())?;
            self.builder.build_call(
                self.module.get_function("thaw_promise_reject_native_text").unwrap(),
                &[completion.into(), message.as_pointer_value().into()],
                "reject_failed_subscription",
            ).map_err(|e| e.to_string())?;
            if ramp {
                self.builder.build_return(Some(&completion)).map_err(|e| e.to_string())?;
            } else {
                self.builder.build_return(None).map_err(|e| e.to_string())?;
            }
            self.builder.position_at_end(subscribe_ready);
            if ramp {
                self.builder
                    .build_return(Some(&completion))
                    .map_err(|e| e.to_string())?;
            } else {
                self.builder.build_return(None).map_err(|e| e.to_string())?;
            }
        } else {
            if let Some(state) = segment.await_next {
                if !plan.segments.get(state).is_some_and(|next| next.resume_target.is_none()) {
                    return Err("synchronous frame transition needs a state without an await result".into());
                }
                // The resume entry treats a null waiting handle as a direct
                // state transition; no promise or scheduler tick is created.
                let state_slot = self.async_frame_field(frame, ASYNC_STATE_OFFSET, "state_slot")?;
                self.builder.build_store(
                    state_slot, self.context.i64_type().const_int(state as u64, false),
                ).map_err(|e| e.to_string())?;
                self.builder.build_call(
                    resume, &[frame.into(), frame.into()], "resume_without_await",
                ).map_err(|e| e.to_string())?;
                if ramp {
                    self.builder.build_return(Some(&completion)).map_err(|e| e.to_string())?;
                } else {
                    self.builder.build_return(None).map_err(|e| e.to_string())?;
                }
                return Ok(());
            }
            if plan.ret != HirType::Void {
                if plan.returns_on_all_paths {
                    self.builder
                        .build_unreachable()
                        .map_err(|error| error.to_string())?;
                    return Ok(());
                }
                return Err("value-returning frame-split async function does not return a value on all paths".to_string());
            }
            self.retire_all_async_frame_owners(plan, frame)?;
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
        self.catch_native_text.clear();
        for (index, (name, ty)) in plan.locals.iter().enumerate() {
            let slot = self.async_frame_field(
                frame,
                self.async_local_offset(plan, index)?,
                &format!("frame_{name}"),
            )?;
            self.async_frame_cells.insert(slot);
            if matches!(ty, HirType::Promise(_)) {
                let ticket_slot = self.async_frame_field(
                    frame, self.async_promise_ticket_offset(plan, index)?,
                    &format!("frame_{name}_creator_ticket"),
                )?;
                self.promise_creator_cell_tickets.insert(slot, ticket_slot);
            }
            if name.starts_with("@@thaw_for_iteration_") {
                let cell = self.builder.build_load(
                    self.context.ptr_type(AddressSpace::default()), slot,
                    &format!("current_{name}"),
                ).map_err(|error| error.to_string())?.into_pointer_value();
                self.for_iteration_frame_slots.insert(name.clone(), slot);
                self.arena_variables.insert(name.clone());
                self.variables.insert(name.clone(), (cell, self.basic_type(ty)?));
            } else {
                self.variables.insert(name.clone(), (slot, self.basic_type(ty)?));
            }
            self.variable_hir_types.insert(name.clone(), ty.clone());
        }
        for binding in &plan.generated_catch_bindings {
            let name = Self::async_catch_native_name(binding);
            let catch_slot = self.variables.get(binding).map(|(slot, _)| *slot)
                .ok_or_else(|| format!("missing async catch binding `{binding}`"))?;
            let native_slot = self.variables.get(&name).map(|(slot, _)| *slot)
                .ok_or_else(|| format!("missing async native text cell `{name}`"))?;
            let original_name = Self::async_catch_original_name(binding);
            let original_slot = self.variables.get(&original_name).map(|(slot, _)| *slot)
                .ok_or_else(|| format!("missing async catch original cell `{original_name}`"))?;
            let valid_name = Self::async_catch_valid_name(binding);
            let valid_slot = self.variables.get(&valid_name).map(|(slot, _)| *slot)
                .ok_or_else(|| format!("missing async catch validity cell `{valid_name}`"))?;
            self.catch_native_text.insert(binding.clone(), (catch_slot, native_slot, original_slot, valid_slot));
        }
        Ok(())
    }

    fn ensure_async_catch_cleanup_token(&self, binding: &str) -> Result<IntValue<'ctx>, String> {
        let ptr = self.context.ptr_type(AddressSpace::default());
        let name = format!("{binding}__thaw_exception_cleanup_token");
        let (slot, _) = self.variables.get(&name).copied()
            .ok_or_else(|| format!("missing async catch cleanup token cell `{name}`"))?;
        let existing = self.builder.build_load(ptr, slot, "async_catch_reserved_cleanup")
            .map_err(|error| error.to_string())?.into_pointer_value();
        let present = self.builder.build_is_not_null(existing, "async_catch_has_cleanup_token")
            .map_err(|error| error.to_string())?;
        let function = self.current_function();
        let reserve = self.context.append_basic_block(function, "replenish_async_catch_cleanup");
        let ready = self.context.append_basic_block(function, "async_catch_cleanup_available");
        self.builder.build_conditional_branch(present, ready, reserve)
            .map_err(|error| error.to_string())?;
        self.builder.position_at_end(reserve);
        let replacement = self.builder.build_call(
            self.module.get_function("thaw_json_reserve_arena_owned_root").unwrap(),
            &[], "replacement_async_catch_cleanup_token",
        ).map_err(|error| error.to_string())?.try_as_basic_value().basic()
            .ok_or("async catch replacement reservation returned no value")?.into_pointer_value();
        self.builder.build_store(slot, replacement).map_err(|error| error.to_string())?;
        self.builder.build_unconditional_branch(ready).map_err(|error| error.to_string())?;
        self.builder.position_at_end(ready);
        let available = self.builder.build_load(ptr, slot, "async_catch_cleanup_token")
            .map_err(|error| error.to_string())?.into_pointer_value();
        self.builder.build_is_not_null(available, "async_catch_cleanup_token_ready")
            .map_err(|error| error.to_string())
    }

    /// Retire the original Json Box captured for one generated async catch.
    /// The visible catch carrier takes a different arena-rooted share. An
    /// exact rethrow transfers this owner into the pending tuple instead.
    fn retire_async_catch_owner(
        &self,
        binding: &str,
    ) -> Result<(), String> {
        let ptr = self.context.ptr_type(AddressSpace::default());
        let name = format!("{binding}__thaw_exception_json_owner");
        let (slot, _) = self.variables.get(&name).copied()
            .ok_or_else(|| format!("missing async catch owner cell `{name}`"))?;
        let owner = self.builder.build_load(ptr, slot, "retire_async_catch_owner")
            .map_err(|error| error.to_string())?.into_pointer_value();
        let pending = self.builder.build_load(ptr,
            self.pending_exception_json_owner().as_pointer_value(),
            "retire_async_catch_pending_owner")
            .map_err(|error| error.to_string())?.into_pointer_value();
        let same = self.builder.build_int_compare(inkwell::IntPredicate::EQ,
            self.builder.build_ptr_to_int(owner, self.context.i64_type(), "async_catch_owner_bits")
                .map_err(|error| error.to_string())?,
            self.builder.build_ptr_to_int(pending, self.context.i64_type(), "async_pending_owner_bits")
                .map_err(|error| error.to_string())?,
            "async_catch_owner_transferred")
            .map_err(|error| error.to_string())?;
        let present = self.builder.build_is_not_null(owner, "async_catch_owner_present")
            .map_err(|error| error.to_string())?;
        let destroy = self.builder.build_and(present,
            self.builder.build_not(same, "async_catch_owner_not_pending")
                .map_err(|error| error.to_string())?,
            "async_catch_owner_needs_retirement")
            .map_err(|error| error.to_string())?;
        let function = self.current_function();
        let release = self.context.append_basic_block(function, "retire_async_catch_json");
        let transferred = self.context.append_basic_block(function, "transfer_async_catch_json");
        let done = self.context.append_basic_block(function, "async_catch_json_retired");
        self.builder.build_conditional_branch(destroy, release, transferred)
            .map_err(|error| error.to_string())?;
        self.builder.position_at_end(release);
        let released = self.destroy_catch_json_preserving_pending(owner, slot)?;
        let deferred = self.context.append_basic_block(function, "defer_async_catch_owner");
        self.builder.build_conditional_branch(released, done, deferred)
            .map_err(|error| error.to_string())?;
        self.builder.position_at_end(deferred);
        let token_name = format!("{binding}__thaw_exception_cleanup_token");
        let (token_slot, _) = self.variables.get(&token_name).copied()
            .ok_or_else(|| format!("missing async catch cleanup token cell `{token_name}`"))?;
        let token = self.builder.build_load(ptr, token_slot, "deferred_async_catch_token")
            .map_err(|error| error.to_string())?.into_pointer_value();
        // Every owner acquisition reserves a token first. Transfer both
        // pointers out of the frame before the subscription drops its sole
        // ArenaRoot; the registered queue head then keeps them reachable.
        self.builder.build_store(slot, ptr.const_null()).map_err(|error| error.to_string())?;
        self.builder.build_store(token_slot, ptr.const_null()).map_err(|error| error.to_string())?;
        self.builder.build_call(
            self.module.get_function("thaw_json_enqueue_reserved_cleanup").unwrap(),
            &[owner.into(), token.into()], "defer_async_catch_json_owner",
        ).map_err(|error| error.to_string())?;
        self.builder.build_unconditional_branch(done).map_err(|error| error.to_string())?;
        self.builder.position_at_end(transferred);
        // Null or exact rethrow needs no destructor. The pending tuple now
        // owns a transferred original, so do not leave a second frame token.
        // On an OOM-skipped release the arena root still owns the Box and
        // must remain in this slot for a later cleanup opportunity.
        let clear = self.builder.build_or(
            self.builder.build_not(present, "async_catch_no_owner")
                .map_err(|error| error.to_string())?,
            same, "async_catch_clear_transferred")
            .map_err(|error| error.to_string())?;
        let cleared = self.builder.build_select(clear, ptr.const_null(), owner,
            "async_catch_owner_after_retirement")
            .map_err(|error| error.to_string())?;
        self.builder.build_store(slot, cleared).map_err(|error| error.to_string())?;
        self.builder.build_unconditional_branch(done).map_err(|error| error.to_string())?;
        self.builder.position_at_end(done);
        Ok(())
    }

    fn retire_all_async_catch_owners(&self, plan: &FrameAsyncPlan) -> Result<(), String> {
        for binding in &plan.generated_catch_bindings {
            self.retire_async_catch_owner(binding)?;
        }
        Ok(())
    }

    fn retire_all_async_frame_owners(
        &self, plan: &FrameAsyncPlan, frame: PointerValue<'ctx>,
    ) -> Result<(), String> {
        self.retire_all_async_catch_owners(plan)?;
        let ptr = self.context.ptr_type(AddressSpace::default());
        let function = self.current_function();
        for (index, (_, ty)) in plan.locals.iter().enumerate().rev() {
            if !matches!(ty, HirType::Promise(_)) { continue; }
            let ticket_slot = self.async_frame_field(frame,
                self.async_promise_ticket_offset(plan, index)?,
                "terminal_promise_creator_ticket")?;
            let ticket = self.builder.build_load(self.context.i64_type(), ticket_slot,
                "terminal_promise_ticket")
                .map_err(|error| error.to_string())?.into_int_value();
            let present = self.builder.build_int_compare(IntPredicate::NE, ticket,
                self.context.i64_type().const_zero(), "terminal_promise_ticket_present")
                .map_err(|error| error.to_string())?;
            let retire = self.context.append_basic_block(function, "retire_frame_promise_creator");
            let done = self.context.append_basic_block(function, "frame_promise_creator_retired");
            self.builder.build_conditional_branch(present, retire, done)
                .map_err(|error| error.to_string())?;
            self.builder.position_at_end(retire);
            let cleanup_slot = self.async_frame_field(frame,
                self.async_promise_cleanup_offset(plan, index)?,
                "terminal_promise_cleanup_slot")?;
            let cleanup = self.builder.build_load(ptr, cleanup_slot,
                "terminal_promise_cleanup")
                .map_err(|error| error.to_string())?.into_pointer_value();
            let saved = self.activate_async_promise_local_cleanup(cleanup)?;
            // The ticket cannot be consumed twice if its final release
            // reenters the same frame. The registered snapshot protects the
            // original exact pending tuple throughout that release.
            self.builder.build_store(ticket_slot, self.context.i64_type().const_zero())
                .map_err(|error| error.to_string())?;
            self.builder.build_call(
                self.module.get_function("thaw_promise_destroy_creator_ticket").unwrap(),
                &[ticket.into()], "retire_terminal_promise_creator",
            ).map_err(|error| error.to_string())?;
            self.defer_reentrant_pending_exception_tuple(cleanup)?;
            self.restore_pending_exception_tuple(&saved, cleanup)?;
            self.builder.build_unconditional_branch(done).map_err(|error| error.to_string())?;
            self.builder.position_at_end(done);
        }
        Ok(())
    }

    fn store_for_iteration_cell(
        &mut self,
        name: &str,
        ty: &HirType,
        value: BasicValueEnum<'ctx>,
    ) -> Result<(), String> {
        let frame_slot = *self.for_iteration_frame_slots.get(name)
            .ok_or_else(|| format!("missing iteration frame slot for `{name}`"))?;
        let llvm_ty = self.basic_type(ty)?;
        let cell = self.build_arena_cell(&self.builder, llvm_ty, name)?;
        self.retain_native_promise_cell_value(cell, ty, value)?;
        self.builder.build_store(cell, value).map_err(|error| error.to_string())?;
        self.builder.build_store(frame_slot, cell).map_err(|error| error.to_string())?;
        self.variables.insert(name.to_string(), (cell, llvm_ty));
        self.variable_hir_types.insert(name.to_string(), ty.clone());
        self.arena_variables.insert(name.to_string());
        if *ty == HirType::JsValue {
            self.uses_quickjs = true;
            self.uses_quickjs_handles = true;
            self.builder.build_call(
                self.module.get_function("thaw_js_retain_handle").unwrap(),
                &[value.into()], "retain_iteration_js_handle",
            ).map_err(|error| error.to_string())?;
        }
        Ok(())
    }

    fn reload_for_iteration_cell(&mut self, name: &str, ty: &HirType) -> Result<(), String> {
        let slot = *self.for_iteration_frame_slots.get(name)
            .ok_or_else(|| format!("missing iteration frame slot for `{name}`"))?;
        let cell = self.builder.build_load(
            self.context.ptr_type(AddressSpace::default()), slot,
            &format!("current_{name}"),
        ).map_err(|error| error.to_string())?.into_pointer_value();
        self.variables.insert(name.to_string(), (cell, self.basic_type(ty)?));
        Ok(())
    }

    fn bind_async_frame_captures(
        &mut self,
        frame: PointerValue<'ctx>,
        plan: &FrameAsyncPlan,
    ) -> Result<(), String> {
        let ptr_ty = self.context.ptr_type(AddressSpace::default());
        for (index, (name, ty)) in plan.captures.iter().enumerate() {
            let slot = self.async_frame_field(
                frame,
                self.async_capture_offset(plan, index)?,
                &format!("capture_{name}"),
            )?;
            let cell = self
                .builder
                .build_load(ptr_ty, slot, &format!("capture_{name}_cell"))
                .map_err(|e| e.to_string())?
                .into_pointer_value();
            self.variables
                .insert(name.clone(), (cell, self.basic_type(ty)?));
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
            if let HirStmt::Expr(HirExpr::Call(callee, args)) = stmt {
                if matches!(callee.as_ref(), HirExpr::Var(name)
                    if name == "@@thaw_retire_async_catch_owner")
                {
                    let [HirExpr::Var(binding)] = args.as_slice() else {
                        return Err("generated async catch retirement needs one binding".into());
                    };
                    self.retire_async_catch_owner(binding)?;
                    continue;
                }
            }
            let synchronous_catch = match stmt {
                HirStmt::If(HirExpr::Var(guard), _, _) => plan
                    .guarded_rethrow_handlers
                    .get(guard)
                    .cloned()
                    .map(|handler| {
                        let function = self.current_function();
                        (
                            self.context.append_basic_block(function, "guarded_sync_catch"),
                            self.context.append_basic_block(function, "guarded_sync_next"),
                            handler,
                        )
                    }),
                _ => None,
            };
            if let Some((catch_block, _, _)) = &synchronous_catch {
                self.catch_stack.push(*catch_block);
            }
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
                    if self.for_iteration_frame_slots.contains_key(name) {
                        self.store_for_iteration_cell(name, ty, value)?;
                    } else {
                        let (slot, slot_ty) = self.variables.get(name).copied()
                            .ok_or_else(|| format!("missing async frame slot for `{name}`"))?;
                        if slot_ty != self.basic_type(ty)? {
                            return Err(format!("async frame local `{name}` changed type"));
                        }
                        self.retain_native_promise_cell_value(slot, ty, value)?;
                        self.builder.build_store(slot, value).map_err(|e| e.to_string())?;
                    }
                    self.builder
                        .build_unconditional_branch(continue_block)
                        .map_err(|e| e.to_string())?;
                    self.builder.position_at_end(continue_block);
                    if self.for_iteration_frame_slots.contains_key(name) {
                        self.reload_for_iteration_cell(name, ty)?;
                    }
                    self.finish_async_guarded_stmt(&synchronous_catch, frame, plan)?;
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
                        Some(HirExpr::ThrowValue(error_expr, _)) => {
                            let error = self.compile_throw_text(error_expr)?;
                            self.reject_frame_explicit_throw(
                                frame, completion, error, plan, "reject_guarded_throw_value",
                            )?;
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
                            self.finish_async_guarded_stmt(&synchronous_catch, frame, plan)?;
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
                    self.retire_all_async_frame_owners(plan, frame)?;
                    self.resolve_async_completion(
                        completion,
                        self.async_completion_result(frame, plan)?,
                        function.get_type().get_return_type().is_some(),
                    )?;
                    self.builder.position_at_end(continue_block);
                    self.finish_async_guarded_stmt(&synchronous_catch, frame, plan)?;
                    continue;
                }
                if let Some((HirStmt::Throw(error_expr), expected)) = guarded {
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
                    let error = self.compile_throw_text(error_expr)?;
                    if let Some(handler) = enclosing_handler {
                        if let Some(source) = &handler.rethrow_source_binding {
                            // Throw provenance has now moved the original
                            // owner into pending when this is an exact
                            // rethrow. A different throw retires the old
                            // catch owner before the outer catch takes over.
                            self.retire_async_catch_owner(source)?;
                        }
                        // The guard's synchronous catch is active while this
                        // statement is compiled. An arena-copy failure must
                        // bypass that same catch, or it would capture the
                        // unchanged heap-owned tuple again without progress.
                        let current_catch = self.catch_stack.pop()
                            .ok_or("guarded rethrow has no active synchronous catch")?;
                        self.capture_pending_async_catch(frame, plan, &handler, error)?;
                        self.catch_stack.push(current_catch);
                        self.builder.build_store(self.pending_exception_native_text_owner().as_pointer_value(),
                            self.context.ptr_type(AddressSpace::default()).const_null())
                            .map_err(|e| e.to_string())?;
                        self.clear_pending_native_text()?;
                        self.builder.build_store(self.pending_exception_object().as_pointer_value(),
                            self.context.ptr_type(AddressSpace::default()).const_null())
                            .map_err(|e| e.to_string())?;
                        // The catch frame now owns this captured Box token.
                        self.builder.build_store(self.pending_exception_json_owner().as_pointer_value(),
                            self.context.ptr_type(AddressSpace::default()).const_null())
                            .map_err(|e| e.to_string())?;
                        self.builder.build_store(
                            self.pending_exception_value(PENDING_EXCEPTION_VALUE_TAG_SYMBOL).as_pointer_value(),
                            self.context.i64_type().const_zero())
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
                                self.async_local_offset(plan, guard_index)?,
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
                        // compile_throw_text has already selected this throw's
                        // native/typed tuple. Reclassifying it as opaque here
                        // would erase a direct string or typed throw.
                        self.reject_frame_explicit_throw(
                            frame, completion, error, plan, "rethrow_rejection",
                        )?;
                        if function.get_type().get_return_type().is_some() {
                            self.builder
                                .build_return(Some(&completion))
                                .map_err(|e| e.to_string())?;
                        } else {
                            self.builder.build_return(None).map_err(|e| e.to_string())?;
                        }
                    }
                    self.builder.position_at_end(continue_block);
                    self.finish_async_guarded_stmt(&synchronous_catch, frame, plan)?;
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
                let error = self.compile_throw_text(error)?;
                self.reject_frame_explicit_throw(
                    frame, completion, error, plan, "reject_throw_value",
                )?;
                return Ok(AsyncBlockExit::RejectedRetired);
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
                let error = self.compile_throw_text(expr)?;
                self.reject_frame_explicit_throw(
                    frame, completion, error, plan, "reject_completion",
                )?;
                return Ok(AsyncBlockExit::RejectedRetired);
            }
            if let HirStmt::Let(name, ty, expr) = stmt {
                let value = self.compile_expr(expr)?;
                if self.for_iteration_frame_slots.contains_key(name) {
                    self.store_for_iteration_cell(name, ty, value)?;
                } else {
                    let index = plan.locals.iter().position(|(local, _)| local == name)
                        .ok_or_else(|| format!("missing async frame slot for `{name}`"))?;
                    let slot = self.async_frame_field(
                        frame,
                        self.async_local_offset(plan, index)?,
                        &format!("frame_{name}"),
                    )?;
                    self.async_frame_cells.insert(slot);
                    self.retain_native_promise_cell_value(slot, ty, value)?;
                    if matches!(ty, HirType::Promise(_)) {
                        self.store_promise_creator_cell_ticket(slot, value)?;
                    }
                    self.builder.build_store(slot, value).map_err(|e| e.to_string())?;
                    self.variables.insert(name.clone(), (slot, self.basic_type(ty)?));
                }
            } else if self.compile_stmt(stmt)? {
                self.finish_async_guarded_stmt(&synchronous_catch, frame, plan)?;
                return Ok(AsyncBlockExit::Returned);
            }
            self.finish_async_guarded_stmt(&synchronous_catch, frame, plan)?;
        }
        Ok(AsyncBlockExit::Continue)
    }

    fn promote_pending_async_catch_native_text(
        &mut self,
        pending: PointerValue<'ctx>,
    ) -> Result<PointerValue<'ctx>, String> {
        let ptr_ty = self.context.ptr_type(AddressSpace::default());
        let function = self.current_function();
        let owner = self.builder.build_load(
            ptr_ty,
            self.pending_exception_native_text_owner().as_pointer_value(),
            "async_catch_heap_text_owner",
        ).map_err(|e| e.to_string())?.into_pointer_value();
        let has_owner = self.builder.build_is_not_null(owner, "async_catch_has_heap_text")
            .map_err(|e| e.to_string())?;
        let copy_block = self.context.append_basic_block(function, "async_catch_copy_text");
        let ready_block = self.context.append_basic_block(function, "async_catch_text_ready");
        let from_block = self.builder.get_insert_block().ok_or("missing async catch entry")?;
        self.builder.build_conditional_branch(has_owner, copy_block, ready_block)
            .map_err(|e| e.to_string())?;
        self.builder.position_at_end(copy_block);
        let copy = self.builder.build_call(
            self.module.get_function("thaw_string_copy_to_arena")
                .ok_or("missing exact NativeStr arena copier")?,
            &[owner.into()],
            "copy_async_catch_exact_text",
        ).map_err(|e| e.to_string())?.try_as_basic_value().basic()
            .ok_or("async catch NativeStr copy returned no value")?.into_pointer_value();
        let missing = self.builder.build_is_null(copy, "async_catch_text_copy_oom")
            .map_err(|e| e.to_string())?;
        let failed = self.context.append_basic_block(function, "async_catch_text_copy_failed");
        let copied = self.context.append_basic_block(function, "async_catch_text_copied");
        self.builder.build_conditional_branch(missing, failed, copied)
            .map_err(|e| e.to_string())?;
        self.builder.position_at_end(failed);
        // Neither the pending value nor its owner has been changed. The
        // current source catch is already removed from catch_stack here.
        self.branch_on_pending_exception()?;
        self.builder.build_unreachable().map_err(|e| e.to_string())?;
        self.builder.position_at_end(copied);
        let same_primary = self.builder.build_int_compare(
            inkwell::IntPredicate::EQ,
            self.builder.build_ptr_to_int(pending, self.context.i64_type(), "async_catch_value_bits")
                .map_err(|e| e.to_string())?,
            self.builder.build_ptr_to_int(owner, self.context.i64_type(), "async_catch_owner_bits")
                .map_err(|e| e.to_string())?,
            "async_catch_value_is_heap_text",
        ).map_err(|e| e.to_string())?;
        let copied_value = self.builder.build_select(same_primary, copy, pending,
            "async_catch_arena_value").map_err(|e| e.to_string())?.into_pointer_value();
        self.builder.build_store(self.pending_exception().as_pointer_value(), copied_value)
            .map_err(|e| e.to_string())?;
        self.builder.build_store(self.pending_exception_native_text().as_pointer_value(), copy)
            .map_err(|e| e.to_string())?;
        let detached = self.take_pending_owned_native_text()?;
        self.builder.build_call(
            self.module.get_function("thaw_cstring_destroy")
                .ok_or("missing NativeStr destroy")?,
            &[detached.into()], "release_copied_async_catch_text",
        ).map_err(|e| e.to_string())?;
        self.builder.build_unconditional_branch(ready_block).map_err(|e| e.to_string())?;
        self.builder.position_at_end(ready_block);
        let result = self.builder.build_phi(ptr_ty, "async_catch_promoted_value")
            .map_err(|e| e.to_string())?;
        result.add_incoming(&[(&pending, from_block), (&copied_value, copied)]);
        Ok(result.as_basic_value().into_pointer_value())
    }

    fn capture_pending_async_catch(
        &mut self,
        frame: PointerValue<'ctx>,
        plan: &FrameAsyncPlan,
        handler: &AsyncRejectionHandler,
        pending: PointerValue<'ctx>,
    ) -> Result<(), String> {
        let ptr_ty = self.context.ptr_type(AddressSpace::default());
        // A catch binding may escape through a later return or Promise result.
        // Convert a transferred heap NativeStr into arena storage before any
        // frame cell is overwritten. The old pending tuple stays untouched on
        // allocation failure, so an enclosing catch receives its exact value.
        let pending = self.promote_pending_async_catch_native_text(pending)?;
        // A repeated catch (including a loop back-edge) must not overwrite
        // the previous owner token and leak its Host lease indefinitely.
        self.retire_async_catch_owner(&handler.catch_binding)?;
        let token_ready = self.ensure_async_catch_cleanup_token(&handler.catch_binding)?;
        let function = self.current_function();
        let token_available = self.context.append_basic_block(function,
            "async_catch_capture_token_available");
        let token_missing = self.context.append_basic_block(function,
            "async_catch_capture_token_oom");
        self.builder.build_conditional_branch(token_ready, token_available, token_missing)
            .map_err(|error| error.to_string())?;
        self.builder.position_at_end(token_missing);
        // The original pending exception has not been captured or cleared.
        // Propagate it exactly rather than install an owner with no cleanup
        // node for a later cancellation.
        self.branch_on_pending_exception()?;
        self.builder.build_unreachable().map_err(|error| error.to_string())?;
        self.builder.position_at_end(token_available);
        let binding_slot = self.catch_native_text.get(&handler.catch_binding)
            .map(|(binding, _, _, _)| *binding)
            .ok_or_else(|| format!("missing catch provenance for `{}`", handler.catch_binding))?;
        self.builder.build_store(binding_slot, pending).map_err(|e| e.to_string())?;
        let original_slot = self.catch_native_text.get(&handler.catch_binding)
            .map(|(_, _, original, _)| *original)
            .ok_or_else(|| format!("missing catch original for `{}`", handler.catch_binding))?;
        self.builder.build_store(original_slot, pending).map_err(|e| e.to_string())?;
        let valid_slot = self.catch_native_text.get(&handler.catch_binding)
            .map(|(_, _, _, valid)| *valid)
            .ok_or_else(|| format!("missing catch validity for `{}`", handler.catch_binding))?;
        self.builder.build_store(valid_slot, self.context.bool_type().const_int(1, false))
            .map_err(|e| e.to_string())?;
        let pending_metadata: [(&str, &str, BasicTypeEnum<'ctx>); 9] = [
            ("native_text", PENDING_EXCEPTION_NATIVE_TEXT_SYMBOL, ptr_ty.into()),
            ("native_text_owner", PENDING_EXCEPTION_NATIVE_TEXT_OWNER_SYMBOL, ptr_ty.into()),
            ("object", PENDING_EXCEPTION_OBJECT_SYMBOL, ptr_ty.into()),
            ("json_owner", PENDING_EXCEPTION_JSON_OWNER_SYMBOL, ptr_ty.into()),
            ("aggregate", PENDING_EXCEPTION_AGGREGATE_SYMBOL, ptr_ty.into()),
            ("tag", PENDING_EXCEPTION_VALUE_TAG_SYMBOL, self.context.i64_type().into()),
            ("f64", PENDING_EXCEPTION_F64_SYMBOL, self.context.f64_type().into()),
            ("i64", PENDING_EXCEPTION_I64_SYMBOL, self.context.i64_type().into()),
            ("bool", PENDING_EXCEPTION_BOOL_SYMBOL, self.context.bool_type().into()),
        ];
        for (suffix, symbol, ty) in pending_metadata {
            let name = if suffix == "native_text" {
                Self::async_catch_native_name(&handler.catch_binding)
            } else {
                format!("{}__thaw_exception_{suffix}", handler.catch_binding)
            };
            let index = plan.locals.iter().position(|(local, _)| local == &name)
                .ok_or_else(|| format!("missing async catch metadata `{name}`"))?;
            let slot = self.async_frame_field(
                frame,
                self.async_local_offset(plan, index)?,
                "caught_sync_metadata",
            )?;
            let global = self.module.get_global(symbol)
                .ok_or_else(|| format!("missing pending exception metadata `{symbol}`"))?;
            let value = self.builder.build_load(ty, global.as_pointer_value(), "pending_sync_metadata")
                .map_err(|e| e.to_string())?;
            self.builder.build_store(slot, value).map_err(|e| e.to_string())?;
        }
        Ok(())
    }

    fn finish_async_guarded_stmt(
        &mut self,
        boundary: &Option<(
            inkwell::basic_block::BasicBlock<'ctx>,
            inkwell::basic_block::BasicBlock<'ctx>,
            AsyncRejectionHandler,
        )>,
        frame: PointerValue<'ctx>,
        plan: &FrameAsyncPlan,
    ) -> Result<(), String> {
        let Some((catch_block, next_block, handler)) = boundary else {
            return Ok(());
        };
        self.catch_stack.pop();
        if self.builder.get_insert_block().unwrap().get_terminator().is_none() {
            self.builder.build_unconditional_branch(*next_block).map_err(|e| e.to_string())?;
        }
        self.builder.position_at_end(*catch_block);
        let ptr_ty = self.context.ptr_type(AddressSpace::default());
        let pending = self.builder.build_load(
            ptr_ty,
            self.pending_exception().as_pointer_value(),
            "caught_sync_exception",
        ).map_err(|e| e.to_string())?;
        for (name, enabled) in [(&handler.try_guard, false), (&handler.catch_guard, true)]
            .into_iter()
            .chain(handler.disable_guards.iter().map(|name| (name, false)))
        {
            let index = plan.locals.iter().position(|(local, _)| local == name)
                .ok_or_else(|| format!("missing async guard `{name}`"))?;
            let slot = self.async_frame_field(
                frame,
                self.async_local_offset(plan, index)?,
                "caught_sync_guard",
            )?;
            self.builder.build_store(slot, self.context.bool_type().const_int(enabled as u64, false))
                .map_err(|e| e.to_string())?;
        }
        if let Some(source) = &handler.rethrow_source_binding {
            self.retire_async_catch_owner(source)?;
        }
        self.capture_pending_async_catch(frame, plan, handler, pending)?;
        self.builder.build_store(self.pending_exception().as_pointer_value(), ptr_ty.const_null())
            .map_err(|e| e.to_string())?;
        // The synchronous catch frame has copied the heap allocation's owner
        // token. The frame, not the pending globals, now governs its release.
        self.builder.build_store(self.pending_exception_native_text_owner().as_pointer_value(),
            ptr_ty.const_null()).map_err(|e| e.to_string())?;
        self.clear_pending_native_text()?;
        self.builder.build_store(self.pending_exception_object().as_pointer_value(), ptr_ty.const_null())
            .map_err(|e| e.to_string())?;
        // The async catch frame retains the captured ownership token.
        self.builder.build_store(self.pending_exception_json_owner().as_pointer_value(), ptr_ty.const_null())
            .map_err(|e| e.to_string())?;
        self.builder.build_store(
            self.pending_exception_value(PENDING_EXCEPTION_VALUE_TAG_SYMBOL).as_pointer_value(),
            self.context.i64_type().const_zero(),
        ).map_err(|e| e.to_string())?;
        self.builder.build_unconditional_branch(*next_block).map_err(|e| e.to_string())?;
        self.builder.position_at_end(*next_block);
        Ok(())
    }

    fn async_return_width(&self, plan: &FrameAsyncPlan) -> Result<u64, String> {
        if plan.ret == HirType::Void { Ok(0) }
        else { Ok(ASYNC_SLOT_BYTES.max(arena_storage_bytes(&plan.ret)?)) }
    }

    fn async_capture_offset(&self, plan: &FrameAsyncPlan, index: usize) -> Result<u64, String> {
        checked_storage_add(
            checked_storage_add(ASYNC_FRAME_BYTES, self.async_return_width(plan)?)?,
            checked_storage_mul(ASYNC_SLOT_BYTES, index as u64)?,
        )
    }

    fn async_locals_offset(&self, plan: &FrameAsyncPlan) -> Result<u64, String> {
        self.async_capture_offset(plan, plan.captures.len())
    }

    fn async_local_offset(&self, plan: &FrameAsyncPlan, index: usize) -> Result<u64, String> {
        plan.locals[..index].iter().try_fold(self.async_locals_offset(plan)?,
            |offset, (_, ty)| checked_storage_add(
                offset, ASYNC_SLOT_BYTES.max(arena_storage_bytes(ty)?)))
    }

    fn async_frame_size(&self, plan: &FrameAsyncPlan) -> Result<u64, String> {
        checked_storage_add(
            self.async_local_offset(plan, plan.locals.len())?,
            checked_storage_mul(16, plan.locals.iter()
                .filter(|(_, ty)| matches!(ty, HirType::Promise(_))).count() as u64)?,
        )
    }

    fn async_promise_ticket_offset(&self, plan: &FrameAsyncPlan, index: usize) -> Result<u64, String> {
        checked_storage_add(
            self.async_local_offset(plan, plan.locals.len())?,
            checked_storage_mul(8, plan.locals[..index].iter()
                .filter(|(_, ty)| matches!(ty, HirType::Promise(_))).count() as u64)?,
        )
    }

    fn async_promise_cleanup_offset(&self, plan: &FrameAsyncPlan, index: usize) -> Result<u64, String> {
        let promise_count = plan.locals.iter()
            .filter(|(_, ty)| matches!(ty, HirType::Promise(_))).count() as u64;
        checked_storage_add(
            checked_storage_add(self.async_local_offset(plan, plan.locals.len())?,
                checked_storage_mul(8, promise_count)?)?,
            checked_storage_mul(8, plan.locals[..index].iter()
                .filter(|(_, ty)| matches!(ty, HirType::Promise(_))).count() as u64)?,
        )
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
