# Independent source review: Luna Promise-chain successor

Decision: **HOLD for integration; bounded local acceptance of the five-argument callback ABI and the one-shot callback-event shape.** This is a static source review only. I did not run a parser, compiler, formatter, test, build, or product runtime, and did not edit product/test sources. The successor is based on the mutable draft466 candidate, not the current managed HEAD; all overlapping owners still need a semantic rebase and a new paired review.

Reviewed `/tmp/thaw-luna-promise-chain-successor` against `/tmp/thaw-promise-creator-origin-draft466/candidate`, `/tmp/thaw-promise-chain-ownership-plan488-v2.md` (SHA-256 `5689b72f60b8e796d745279290224b76c9b4a868a9047ab844a05c27c1dc28ee`) and the earlier partial review (SHA-256 `3907790fa06553c4c8aa7c6db477c3f309f19f79afed2d6973fb27bfc48b920d`).

## Concrete HOLD

`runtime/promises.rs:1842–1862` constructs `ArenaRoot::new(cell)` **before** the first reserved token/packet calls `thaw_arena_register_root`. `ArenaRoot::new` stores zero when tracing is off (`thaw-arena/src/lib.rs:36–43`), whereas `thaw_json_reserve_arena_owned_root`/`thaw_json_reserve_deferred_exception_packet` register their queue roots and enable tracing only afterward (`std/json.rs:1175–1184,1218–1225`). A chain-only module need not trigger the compiler's pre-entry tracing predicates (`hir_codegen.rs:543–547`, `entrypoints.rs:37–45`); local Promise captures and globals are separate predicates. In that case the 112-byte context root remains inert. The later `PromiseChainState` root does not register the `exception_context.cell` as a child: its `replace_reference` loop publishes callback closure contexts only (`runtime/promises.rs:2051–2064`). The six reserved child pointers therefore have no proven root across an arena reset while the subscription is pending. Establish tracing before the cell root is made (or add an independently registered cell root), and add a source control for a chain-only module with no other tracing trigger. The earlier partial-review root timing issue is otherwise improved: the cell root is now created immediately after allocation and each child is written before the next reservation.

`hir_codegen/promises.rs:652–709` handles an adopt/resolve status zero by querying output state and trying a static native-text rejection if still pending. `settle_promise` returns zero only for a null or already-settled Promise (`runtime/promises.rs:3831–3840`), so the final still-pending branch is unreachable for a valid single-threaded output under these current runtime semantics. If that assumption is ever violated, however, the branch only calls `thaw_http_mark_unhandled_error`: it neither queues an exact packet nor attaches a rejection to the still-pending completion. The C-main terminal loop consumes this flag into a failure exit (`entrypoints.rs:1170–1210`), and Lambda checks the flag before the next invocation (`runtime/lambda.rs:25–52`), but this is **exit/fatal signaling, not a proof that the callback failure was reported exactly**. Keep that terminal fallback explicitly conditional on the `settle_promise` invariant; do not claim it repairs a pending output or preserves a cause. An unrun control should assert the status/state invariant or exercise an injected status-zero/pending failure with a reportable disposition.

## Bounded source findings that look coherent

- `runtime/abi.rs` defines the compiler-private five-argument callback type; `runtime/promises.rs:1986–2008` and LLVM declarations/calls at `runtime_declarations.rs:3503–3523`, `hir_codegen/promises.rs:619–636,713–737,919–935` agree on pointer-sized arguments. The public four-argument callback exports remain present. `resume_promise_chain` consumes its Box once; cancellation consumes it only for a subscription not delivered (`runtime/lib.rs:215–228,690–701`). This supports one-shot event authority without an extra ticket for this specific callback delivery.
- Context allocation and six reserves precede output publication and subscription (`runtime/promises.rs:2012–2070`). The subscriber independently retains source and completion, and its own two terminal packets are reserved before enqueue (`runtime/promises.rs:869–945`). The runtime source-transfer fallback uses the delivery's retained source share; the chain-specific variant does not suppress an independent source reason merely because reentry settled output (`runtime/promises.rs:1597–1632`). Its cause packet is distinct (`1634–1655`), and a moved source is released only after a deferred report (`1684–1705`). These are source-shape observations, subject to the root defect above and the draft466 reporter dependency.
- The LLVM adapter captures the original typed pending tuple in packet64 before isolated settlement, distinguishes status zero from success, and queues status-zero original plus causal failure separately (`async_frames/codegen.rs:33–90`). The successful path moves Json cleanup to a reserved token and defers reentrant text/Json cleanup before restoring the outer tuple. Callback projection failures route through the same active chain context (`hir_codegen.rs:3475–3484`). These helpers share draft466's ten-channel packet and token contract; their full reentry and reporter behavior is not accepted by this review.

## Remaining integration and evidence gates

1. Rebase the successor's seven byte-changed owners and its fourteen unchanged draft466 dependencies against the current managed owners. In particular, preserve later managed Promise/runtime, LLVM, std/json, HTTP, entrypoint, and test changes; no whole-file copy from this old baseline is safe.
2. Audit the full deferred reporter with a queue containing an original packet, a causal packet, and an independently retained source packet. Confirm one pop, one owner release, and terminal fatal handling for each, including Lambda and C-main paths. The present LLVM test is an unrun IR string-shape check (`tests/async/promises.rs:854–877`), not ownership/reentry evidence.
3. Prove the initial chain-context reservation failure path: it takes HostError, creates a fresh output, and rejects it from copied native text (`runtime/promises.rs:2017–2034`), but a null output remains a null return without a separate report. This is not an exact callback-throw path and must not be used as its fidelity proof.
4. Preserve the draft466 source/result-kind, Promise creator-ticket, pending NativeStr/Json owner, subscription cancellation, and terminal-driver contracts during rebase. This review does not accept draft466 as a whole.

## Pinned source hashes (SHA-256)

| Owner | Successor hash |
|---|---|
| `runtime/promises.rs` | `5431b4110d0240b9f5d20dc815bf9992cac1b9ce4aadee538ed1d0633857dcc9` |
| `runtime/abi.rs` | `1fad4bba5a8c2d4a1fd6bdbfe2877fc27092fb63064752c410c82a3df0bd6381` |
| `llvm/hir_codegen/promises.rs` | `681768a0f1c7c4d325e8ee2bedd1be2375f1fd0f279ad5efd59f10b989b08633` |
| `llvm/hir_codegen/async_frames/codegen.rs` | `35439e892cdd2a558e09d0ab72afcf23e9e9745ea2d5b2f16acbadbf9a616b59` |
| `llvm/hir_codegen.rs` | `70087222cdd7fcae09c7b80eada518481ddd70c8db67bed9730a9187b053d1d8` |
| `llvm/hir_codegen/runtime_declarations.rs` | `8d3fe0ce1b42d31dc4fd53cffbe33142f7f9712f583ce4bb49cc391c08a34d2e` |
| `llvm/hir_codegen/tests/async/promises.rs` | `ff3424cd9720633f5b6fc7bcd70e564e0bc934e7f391c0251e32d2d7836f7b32` |
| dependency `runtime/lib.rs` | `93bb0b3085bf58b5cffd6a9e96efdc25ae96ef4efdd50c4a8a7d7c8d9eb8373f` |
| dependency `std/json.rs` | `c1f0da4e01d7f09742ea3f21b68be883f5b05ccfb3acb6cc1f3322577b832ad2` |
| dependency `std/http.rs` | `24bd7647921cc0bc8052ea5a427c6cf3204edf65fdb627d198cfee241f391470` |
| dependency `llvm/hir_codegen/entrypoints.rs` | `1a01fb2e2e38d41e8082020b6e2376171cf595f2c486c565ace500d17e62a8d5` |

Paths in this table are relative to `crates/thaw-*` within `/tmp/thaw-luna-promise-chain-successor`; the exact absolute source paths are the corresponding files under that directory.
