# Focused checks still required

These commands are a plan, not execution evidence. First restore a complete exact checkout and all original native draft prerequisites into a disposable copy, overlay the immutable reviewed candidate, and verify every source hash. Do not test an actively edited tree. Use the project's supported LLVM 22 development toolchain, Cargo.lock, a separate CARGO_TARGET_DIR, CARGO_BUILD_JOBS=2, and a verified LLVM_SYS_221_PREFIX. Resolve baseline compilation blockers separately rather than silently folding unrelated changes into this candidate.

## Compile before execution

- cargo check --locked -p thaw-llvm --lib
- cargo test --locked -p thaw-llvm --lib --no-run
- cargo test --locked -p thaw-hir --lib --no-run

Current environment stopped before package compilation on missing inkwell and has no LLVM 22 development installation. No result for any command above is claimed.

## HIR lowering filters

Run each with cargo test --locked -p thaw-hir --lib FILTER -- --test-threads=1:

- lowers_try_catch
- finally_separates_text_only_throws_from_fresh_published_tuples
- finally_snapshots_return_and_throw_values_before_mutation

## LLVM scope and provenance filters

Run each with cargo test --locked -p thaw-llvm --lib FILTER -- --test-threads=1:

- native_promise_scope_boundaries_codegen_regression_control
- native_promise_scope_tables_restore_after_codegen_errors
- reactive_preheader_promotions_preserve_exact_scope_and_successor_context
- reactive_preheader_codegen_error_restores_nonempty_scope_exactly
- stack_owner_live_merge_preserves_exact_branch_bindings
- hir_if_live_arm_keeps_sibling_stack_binding_and_runtime_flag
- native_promise_exception_descriptor_survives_all_cleanup_handoffs
- nested_codegen_scope_contexts_restore_exact_nonempty_state
- compile_lambda_restores_scope_after_real_inner_body_error
- compile_async_lambda_restores_scope_after_real_inner_body_error
- published_throw_keeps_fresh_native_exception_descriptor
- text_only_throw_clears_stale_native_exception_descriptor
- blocking_and_async_exception_handoffs_copy_native_descriptor_before_release
- plain_string_resolver_clears_native_descriptor_from_real_typed_publisher
- typed_native_reason_resolver_preserves_its_published_descriptor
- pending_rethrow_keeps_text_and_descriptor_on_their_own_channels

Several focused controls deliberately use dummy pointer operands or malformed codegen state to force Result restoration paths. Their generated IR must never be executed as live native code. Passing their host-side codegen assertions is not a runtime ownership proof.

## Separate broader gates

Run existing valid-IR native Promise runtime and integration coverage only after package compilation and the candidate's focused controls pass. Independently validate actual runtime release/retain ordering, aggregate/global/union relocation, JIT callback ABI, and creator/context lifetime under their own reviewed plans. Review and resolve the remote baseline CI parser/compilation failures as separate scoped work. No broad-gate pass follows from the controls above.
