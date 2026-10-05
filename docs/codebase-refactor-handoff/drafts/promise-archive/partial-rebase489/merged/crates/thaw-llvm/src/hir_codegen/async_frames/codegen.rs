impl<'ctx> HirCompiler<'ctx> {
<<<<<<< /tmp/thaw-luna-promise-chain-rebase489/base/crates/thaw-llvm/src/hir_codegen/async_frames/codegen.rs
=======
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

>>>>>>> /tmp/thaw-luna-promise-chain-successor/crates/thaw-llvm/src/hir_codegen/async_frames/codegen.rs
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

        let entry = self.context.append_basic_block(ramp, "entry");
        self.builder.position_at_end(entry);
        self.variables.clear();
        self.for_iteration_frame_slots.clear();
        self.catch_native_text.clear();
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
        let completion_missing = self.builder.build_is_null(completion, "async_completion_is_null")
            .map_err(|error| error.to_string())?;
        let completion_failed = self.context.append_basic_block(ramp, "async_completion_allocation_failed");
        let completion_ready = self.context.append_basic_block(ramp, "async_completion_allocation_ready");
        self.builder.build_conditional_branch(completion_missing, completion_failed, completion_ready)
            .map_err(|error| error.to_string())?;
        self.builder.position_at_end(completion_failed);
        self.compile_throw_type_error("Cannot allocate async completion")?;
        self.builder.position_at_end(completion_ready);
        let completion_slot =
            self.async_frame_field(frame, ASYNC_COMPLETION_OFFSET, "completion_slot")?;
        self.builder
            .build_store(completion_slot, completion)
            .map_err(|e| e.to_string())?;
        let waiting_slot = self.async_frame_field(frame, ASYNC_WAITING_OFFSET, "waiting_slot")?;
        self.builder
            .build_store(waiting_slot, ptr_ty.const_null())
            .map_err(|e| e.to_string())?;
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
            self.builder
                .build_store(slot, param_value)
                .map_err(|e| e.to_string())?;
        }
        self.emit_async_segment(&segments[0], frame, completion, resume, 1, true, plan, None)?;

        let resume_entry = self.context.append_basic_block(resume, "entry");
        self.builder.position_at_end(resume_entry);
        self.variables.clear();
        self.for_iteration_frame_slots.clear();
        self.catch_native_text.clear();
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
            let native_text = self.builder.build_call(
                self.module.get_function("thaw_promise_exception_native_text_copy").unwrap(),
                &[waiting.into()], "caught_promise_native_text",
            ).map_err(|e| e.to_string())?.try_as_basic_value().basic()
                .ok_or("native text copy returned no value")?.into_pointer_value();
            let has_native_text = self.builder.build_is_not_null(native_text, "caught_native_text_present")
                .map_err(|e| e.to_string())?;
            let caught_value = self.builder.build_select(
                has_native_text, native_text, resume_result, "caught_promise_value",
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
            for (suffix, getter) in [
                ("object", "thaw_promise_exception_object"),
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
        self.reject_promise_with_source_exception(
            resume_completion,
            resume_result,
            waiting,
            "reject_completion",
        )?;
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
            self.for_iteration_frame_slots.clear();
            self.catch_native_text.clear();
            self.variable_hir_types.clear();
            self.arena_variables.clear();
            self.catch_stack.clear();
            self.seed_global_variables();
            self.bind_async_frame_captures(resume_frame, plan)?;
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
            let waiting = match awaited {
                HirExpr::Call(callee, args) if matches!(callee.as_ref(), HirExpr::Var(name) if name == "fetch") => {
                    self.compile_single_arg_call("thaw_http_get_async", args, "async_fetch")
                }
                _ => self.compile_expr(awaited),
            };
            self.catch_stack.pop();
            let waiting = waiting?.into_pointer_value();
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
            let rejected = self
                .builder
                .build_call(
                    self.module.get_function("thaw_promise_new").unwrap(),
                    &[],
                    "await_rejected_promise",
                )
                .map_err(|e| e.to_string())?
                .try_as_basic_value()
                .basic()
                .ok_or("thaw_promise_new returned no value")?
                .into_pointer_value();
            self.reject_promise_with_pending_exception(rejected, error, "reject_await_operand")?;
            self.builder
                .build_store(self.pending_exception().as_pointer_value(), ptr_ty.const_null())
                .map_err(|e| e.to_string())?;
            self.clear_pending_native_text()?;
            self.builder
                .build_store(self.pending_exception_object().as_pointer_value(), ptr_ty.const_null())
                .map_err(|e| e.to_string())?;
            self.builder
                .build_store(
                    self.pending_exception_value(PENDING_EXCEPTION_VALUE_TAG_SYMBOL).as_pointer_value(),
                    self.context.i64_type().const_zero(),
                )
                .map_err(|e| e.to_string())?;
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
                            let error = self.compile_expr(error_expr)?.into_pointer_value();
                            self.reject_caught_or_opaque_value(
                                completion, error, error_expr, "reject_throw_value",
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
                        self.capture_pending_async_catch(frame, plan, &handler, error)?;
                        self.clear_pending_native_text()?;
                        self.builder.build_store(self.pending_exception_object().as_pointer_value(),
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
                        self.reject_promise_with_pending_exception(
                            completion, error, "rethrow_rejection",
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
                let error = self.compile_expr(error)?.into_pointer_value();
                self.reject_promise_with_pending_exception(
                    completion,
                    error,
                    "reject_throw_value",
                )?;
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
                let error = self.compile_throw_text(expr)?;
                self.reject_promise_with_pending_exception(
                    completion,
                    error,
                    "reject_completion",
                )?;
                return Ok(AsyncBlockExit::Rejected);
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

    fn capture_pending_async_catch(
        &self,
        frame: PointerValue<'ctx>,
        plan: &FrameAsyncPlan,
        handler: &AsyncRejectionHandler,
        pending: PointerValue<'ctx>,
    ) -> Result<(), String> {
        let ptr_ty = self.context.ptr_type(AddressSpace::default());
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
        let pending_metadata: [(&str, &str, BasicTypeEnum<'ctx>); 7] = [
            ("native_text", PENDING_EXCEPTION_NATIVE_TEXT_SYMBOL, ptr_ty.into()),
            ("object", PENDING_EXCEPTION_OBJECT_SYMBOL, ptr_ty.into()),
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
        self.capture_pending_async_catch(frame, plan, handler, pending)?;
        self.builder.build_store(self.pending_exception().as_pointer_value(), ptr_ty.const_null())
            .map_err(|e| e.to_string())?;
        self.clear_pending_native_text()?;
        self.builder.build_store(self.pending_exception_object().as_pointer_value(), ptr_ty.const_null())
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
        self.async_local_offset(plan, plan.locals.len())
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
