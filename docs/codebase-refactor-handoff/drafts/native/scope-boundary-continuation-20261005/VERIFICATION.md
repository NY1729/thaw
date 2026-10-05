# Verification and limits

## Verified source identity

The complete remote tree and restored inputs were read through GitHub at a6b4fad1723f41ff009b905d7ace2a9a2b5ce6bc and checked against Git blob identities. The original archived native owners matched their published paired snapshots. This continuation's immutable changed-owner base/candidate hashes, patch applicability, whitespace check, and exact replay equality passed. The remote ref was last rechecked at 2026-10-05 12:50 UTC before publication preparation.

Full incremental delta SHA-256: 1387f9a75e443e2030656ec7d4a3b2eb8f1d89573dc0e02c32f881b2b4b50e22.
Changed-candidate manifest SHA-256: 35d25b45ea1459ae29cc1c8b6c00efa8c6d223f308e08a7a5841c6498bc76108.

These are integrity checks, not compilation or runtime evidence. The final combined source review pins the exact reviewed candidate.

## Parallel compiler reductions

Using an existing rustc 1.96.1 (31fca3adb 2026-06-26), std-only reductions reproduced the ambiguous Result inference pattern (E0282/E0283) and the collect-before-clone pattern (E0282). The explicitly typed counterparts compiled successfully with Rust 2021 and --emit=metadata. The actual source now annotates the isolated scope macro Result error type and the relevant HashSet local. The reductions substitute standard-library stand-ins for LLVM types; they are not Thaw package compilation or LLVM validation.

## Package verification unavailable

A separate immutable v3 source copy, augmented with 23 exact baseline workspace/target inputs, passed `cargo metadata --offline --locked --no-deps`. `cargo check --offline --locked -p thaw-llvm --lib`, with two build jobs and isolated Cargo/target directories, exited 101 during dependency resolution because inkwell was unavailable. No package compilation, linking, or tests were reached. LLVM 22 development files were also absent. Authorized official LLVM/Cargo acquisition attempts and a bounded official alternative timed out without downloads or installations. No security/network settings were changed.

No Thaw package check or test was run on v4 or v5. rustfmt was unavailable, so there is no formatter/parser result. Exact generated-IR text assertions and dependency-specific Rust typing remain unexecuted. Required follow-up commands are in TEST-PLAN.md.

## Separate baseline CI failure

Existing run https://github.com/NY1729/thaw/actions/runs/37292963660 at the input remote head failed before this continuation was published. The test job's formatting stage reported a let-else unsafe-expression parser error at crates/thaw-std/src/json.rs:7192 and formatting differences, with later tests skipped. npm shard 0 and ARM logs reported 78 thaw-hir compilation errors, including moved-value and missing-match cases. The ARM performance comparison was skipped and x64 cancelled; no measured performance regression was established.

This existing failure is not attributed to the continuation. No unrelated baseline repair is included. Source acceptance does not certify a working build, runtime ownership, or the broader integration gates.
