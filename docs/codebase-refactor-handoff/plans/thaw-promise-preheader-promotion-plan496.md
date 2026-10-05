# Checked Promise preheader promotion — unit496

Source-only plan; no product edits or execution. Live throw scratch candidate baseline managed f4e2faf98b6973306ac634f2d78968904b846480. Read closures.rs b2b214335b3ff8f61c8d462da9cdcb5b4f3a4346a9dacb871ef780ad2d97f874; statements.rs71cc4dde548e522c84829b7468339d76b8d858d69edfcd748c5a353d11c800a5. These are live read pins, not a freeze.

## Current trigger

values/closures.rs47+ promote_variable_to_arena_cell_with_mode selects an enclosing loop preheader from loop_promotion_scopes. A separate builder inserts the stack load before that preheader's existing terminator. Non-Promise stores must remain on that builder. The current Promise fallback at119+ null-initializes then raw-stores the pointer: this publishes an untracked borrowed alias and must be replaced with a checked ownership call in the actual preheader.

statements.rs468 compile_while is the sole producer of these promotion scopes. It emits an unconditional preheader→whilecond branch, pushes that exact preheader into the scope, compiles body and then condition, and pops scope. Its header uses memory variables, not PHIs. Eager closure_captured_names_in_while prepromotion runs before the branch, and already refreshes preheader_bb after ownership checks. That handles discoverable captures but is not proof that every reactive capture was prepromoted; retain the supported reactive path.

## Smallest safe CFG change

For Promise promotion with promotion_scope present:

1. Save the caller self.builder insertion point before changing it, including instruction-relative position if applicable. Preserve compiler catch_stack. Obtain the existing terminator and its successor; the current scope contract is an unconditional branch to that loop header.
2. Existing allocation placement, stack load and null initialization stay before this terminator using the promotion builder. Do not load at a nested body location or null-initialize per loop iteration accidentally.
3. Detach the terminator with InstructionValue::remove_from_basic_block (not erase; retain it for insertion). Position self.builder at the now unterminated preheader end. Invoke store_arena_promise_slot(cell,cell,hir_ty,value). It emits checked call, condition, failure block and ready block; compile_throw_type_error terminates failure, and the method leaves the builder at ready.
4. Insert the detached original terminator into ready with Builder::insert_instruction(&terminator,None). Preserve its target rather than reconstructing an assumed branch. Old preheader now has only the check branch; ready has only the original branch. Never append a second terminator.
5. Replace every active loop_promotion_scopes preheader equal to the old preheader with ready, preserving its variable set. This is necessary for subsequent captures to extend the success chain instead of inserting on the predecessor of an existing check. Multiple checks then form old→ready1→ready2→header.
6. Restore caller builder insertion position and catch_stack on success and Rust error paths. Publish variable→cell and arena_variables only after successful emission. The generated runtime failure path never stores a borrowed pointer; initialized null remains untracked on failure.

Inkwell source inspected locally exposes InstructionValue::remove_from_basic_block, Builder::insert_instruction, PhiValue::count_incoming/get_incoming/add_incoming/replace_all_uses_with/as_instruction, and instruction erase_from_basic_block. Do not rely on an unverified set_incoming_block API.

## Exception context is part of the split

values/expressions.rs36 compile_throw_type_error delegates to compile_throw_builtin_error, which selects catch_stack.last(). A capture discovered in a nested body try must not make a preheader failure jump into that nested catch: its entry values may not dominate and it is not active at runtime before loop entry. Record the preheader catch_stack when compile_while creates the scope, alongside preheader/variable-set (small dedicated scope struct or extra field). Temporarily restore that saved stack while emitting the ownership failure, then restore the discovery-site stack. Nested loops choose the catch context of the selected containing preheader. Eager prepromotion at the current builder needs no context substitution.

## Successor PHIs

The current selected scope successor is compile_while's whilecond, whose source builds loads and a condition branch and no PHIs. Thus the exact scoped implementation can preserve current behavior without a general PHI framework. Document this invariant explicitly. If implementation supports arbitrary existing terminators or future PHI-bearing headers, moving the edge changes predecessor from old to ready: rebuild each successor PHI with identical incoming values except incoming block old replaced by ready, replace all uses, erase old PHI. Insert rebuilt PHIs before the old PHIs; preserve non-old backedges and self references through RAUW. Do not globally replace uses of the old basic block: the ownership check still needs it. Handle all original successors if generalized. No supported user syntax should be rejected to avoid this internal compiler dependency.

## Minimal owners and unrun controls

Owners: values/closures.rs checked split; statements.rs and hir_codegen.rs scope declaration/construction for saved catch context; existing statements.rs store emitter reused. No new runtime ABI is needed.

Controls: reactive Promise capture discovered from loop body and condition; two captures requiring consecutive splits; nested loops capturing an outer variable; nested try in body versus outer preheader try; closure mutates Promise binding across iterations; failure preserves old pointer and targets correct enclosing catch; non-Promise captures retain original load/store placement. Source inspect one terminator per block and dominance, no tests or verifier run. Full native Promise alias ownership remains separately HOLD.

Marker: IMPLEMENTATION_READY_PREHEADER_PLAN_ONLY.
