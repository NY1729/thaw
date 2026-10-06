# Integrated validation patch set (2026-10-07)

All patches apply to the *reconstructed pristine tree* (baseline `8b353a99` + the 12 pinned stages, built with `.ci/native-return-v1/verify.py`; use a plain `git clone` for the baseline, not `git worktree`). They are archival drafts: nothing here is wired into the verifier, pins or workflow, and none of it is on `main`.

## Apply order

| patch | sha256 (16) | files | hunks |
|---|---|---|---|
| `quickjs-followup-7.patch` | `d10173eb3748ecc3` | 13 | 47 |
| `par-registry.patch` | `69582352b2d56eb3` | 11 | 58 |
| `par-hir.patch` | `d80d8782d29f037d` | 6 | 18 |
| `par-hirtests.patch` | `8440e6dcd52e8505` | 8 | 21 |
| `par-bridge.patch` | `ed94d1424f15204f` | 7 | 22 |
| `par-llvm.patch` | `f099616762a09316` | 12 | 25 |
| `par-cli.patch` | `5347e808f64767a6` | 4 | 14 |
| `par-llvm2.patch` | `54eaf4edc98f55ab` | 5 | 10 |
| `par-stdshare-std.patch` | `9425fa82170f018b` | 1 | 2 |
| `par-stdshare-llvm.patch` | `46f28e9f4630ed37` | 1 | 1 |
| `mono-cli-experiment2.patch` | `5ef448b50911b2c7` | 0 | 1 |

How to apply (verified with `--dry-run --fuzz=0`; headers differ per patch):

1. `quickjs-followup-7.patch`: `patch -p1` from the tree root.
2. `par-registry.patch`: `patch -p0` from the tree root.
3. `par-hir.patch`: `patch -p1` from the tree root.
4. `par-hirtests.patch`, `par-bridge.patch`, `par-llvm.patch`, `par-cli.patch`, `par-llvm2.patch`, `par-stdshare-std.patch`, `par-stdshare-llvm.patch`: `patch -p3` from inside the matching `crates/<crate>` directory (hirtests = thaw-hir, bridge, llvm/llvm2/stdshare-llvm = thaw-llvm, cli, stdshare-std = thaw-std), in the order listed.
5. `mono-cli-experiment2.patch` is an experiment (cli external getters without typed targets) and is NOT part of the set.

## Measured on the integrated tree (not yet by CI)

* `cargo check --workspace --tests`: 0 errors.
* CI-pinned test filters (58): thaw-hir 9/9, thaw-quickjs 11/11, thaw-runtime 10/10, thaw-std 8/8, thaw-llvm 16/20. The 4 llvm failures: `native_eval_then_return_mapped_throw_producers_bypass_effect_and_discard`, `discarded_native_promise_union_eval_then_and_errors` (native Promise scope-boundary IR shape, a held design area), `native_promise_scope_boundaries_codegen_regression_control` (`console.log cannot print union member I64`), `native_promise_exception_descriptor_survives_all_cleanup_handoffs` (`unbound this` because the static-this feature was rolled back to the f15 parent behaviour).
* Full suites: quickjs 278, std 86, runtime 221, arena 11 (all pass); bridge 220/220; registry 589/651; hir 358/401; napi 197/218; cli 353/678; llvm 718/1159.

## SAFETY

Never run the thaw-cli or thaw-llvm test binary as one process: memory accumulates (35 GB RSS, host OOM-killed). Run one process per test (`--list` + `--exact`) with `RUST_MIN_STACK=64MB` under a memory cap (`systemd-run --user --scope -p MemoryMax=4G -p MemorySwapMax=0 timeout 60`), at most 2 in parallel.

## Open design decisions (not decided in these patches)

1. cli `module_graph.rs` namespace getters return call-only typed externs (about 250 cli failures). Decision taken by the owner: generate them lazily, only for modules referenced as values (not yet in these patches).
2. llvm async frames register the catch variable as `Str`, HIR emits `Let error: Union(..)` (about 60 llvm tests).
3. `HirType::NativeException` Json projection (hir/ir.rs `caught_exception_json_adapter`, index 9; explicit-error form now).
4. `HirExpr::FunctionRefThis` codegen (explicit error) and `.bind` lowering.
5. llvm `console.log` of I64 union members.
6. Static `this` / `class_value_token` / `Target::ParameterCell|StaticProperty` were restored to the f15 parent behaviour (minimal repair).
7. napi graph ownership tests (`// Unrun:` in source), registry ESM-mixed path, HIR object physical-reorder tests.
