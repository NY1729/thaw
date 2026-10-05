# Worker terminal threadId plan (Sol source-only)

Managed HEAD: 22c425d9a172e05e3eb2d54d5ffc622890b2b8d1, refactor/codebase.
Finding: 01a0f1d6-4026-7a42-998a-d9cac9a63f91:0.

Current source in crates/thaw-registry/src/registry/builtins/system/workers.rs has two _finish definitions: the first sets _exited and schedules exit; the later shared wrapper deletes threadReceivers by the live threadId before calling the first. Native exit polling separately deletes nativeWorkers and nativeWorkersByThread by the live id before _finish. Both fallback and native instances share EvalWorker.prototype. No terminal threadId assignment exists.

Luna implementation: inspect every _finish caller and every threadId routing consumer first. Set public threadId to -1 in the shared terminal implementation after routing cleanup and before stream shutdown/exit notification. Keep live id available until all native and fallback maps are removed; never use -1 for deletion. Preserve repeated _finish idempotence and exit promise ordering. Do not alter child module threadId while running.

Add the smallest unrun regression using existing Worker fixture patterns: positive live id, fallback normal exit observer sees -1, terminate promise observes -1, repeated terminate remains -1, native mocked poll exit preserves removal of old routing key and public -1. Confirm native termination currently waits for poll before finishing. Do not invent broad lifecycle abstractions or claim native ref/unref repair.

Only scratch product/test edits by Luna. No tests/build/compiler/parser/formatter/runtime execution. Parent Sol reviews immutable paired owners and integrates exact reviewed patches; managed/original checkout are not implementation scratch.
