# Held Promise creator/context handoff inventory

Read-only inventory, 2026-10-05. No implementation resumed, product edited, or tests/build/compiler/runtime executed. These artifacts are HOLD drafts, not integrated patches or an acceptance claim.

## Minimal complete archive

1. Preserve `/tmp/thaw-promise-creator-origin-draft466/base/` and `candidate/` as the paired creator ABI/deferred reporter substrate. Preserve its `STATUS.md` and four checkpoint/audit Markdown files: `capture-registration-and-status0-head476.md`, `checkpoint-generic-drain-and-replacement-head472.md`, `checkpoint-discarded-closure-cells-head472.md`, `discarded-closure-ticket-owner-audit-head472.md`. The STATUS explicitly describes mixed old HTTP/Error base, hidden creator tickets, capture-cell retirement and native/QuickJS drain dependencies. Do not include the loose `merged-runtime-promises.preview.rs` as authoritative source; paired candidate is the useful checkpoint.
2. Preserve `/tmp/thaw-luna-promise-chain-successor/crates/` in full: 21 source owners, comprising the seven changed chain ABI/context owners and fourteen retained draft466 dependencies recorded in the independent review. This is the latest useful chain overlay, not a standalone managed tree. Its runtime/promises.rs current SHA is `5e3b796833a445bcd550d70b08e92868c80e4dea960b7a2416c53ed20ceaa87e`, which differs from the historical review's pinned bytes.
3. Preserve `/tmp/thaw-luna-promise-chain-rebase489/base/` and `merged/` (seven paired owners) plus `/tmp/thaw-luna-promise-chain-rebase489-callgraph.md`. This is the later partial semantic-rebase attempt, not a ready patch. Literal merge conflict markers remain in runtime/promises.rs, LLVM promises.rs and async_frames/codegen.rs. The runtime reserve function includes tracing-before-first-root, but sits within unresolved merge text; do not infer integrated correctness from it. Current merged runtime SHA `11ce4a490e22385ce96be1e4982219c3e5934328c59cd987b8d51e6bdd3f39c3`.

The closure needs runtime callback ABI, cancellation/subscriber shares, chain context reserve/root, std Json exact packet/token queue, LLVM pending tuple/adapter retirement, C-main/Lambda/HTTP drain/fatal disposition, arena roots and source controls. A seven-owner chain-only archive omitting the fourteen substrate dependencies is incomplete. Preserve paired old bytes for semantic rebase; do not whole-file apply them over current managed changes.

## Essential reports not present in managed plans

Managed `/home/fedora/.codex/worktrees/review-remediation/thaw/docs/codebase-refactor-handoff/plans` already has `thaw-promise-chain-ownership-plan488.md` and `...488-v2.md`, plus the unit496 native alias/slot/stack/aggregate/scope plans. Do not duplicate them as missing documents.

Add these missing focused artifacts:

- `/tmp/thaw-luna-promise-chain-rebase489-callgraph.md`, SHA `cef08f9d576b99609969f5f5464b226c32bae63759becd3006b0c646c85ff317`: dependency/caller closure, baseline `bd4565fa048400309f923681614839b3951192a6`, rebase priorities.
- `/tmp/thaw-promise-chain-successor-independent-review489.md`, SHA `379f6bbf1dfb3f3907b4607b7e555f3c4d79f5ffe74b76d0f33b88fc470d4849`: historical bounded five-argument/one-shot source review, root timing HOLD, exact report/fatal distinction and rebase gates. Review pins are older than current successor; retain as history, not current-byte acceptance.
- `/tmp/thaw-promise-creator-async-retirement-bounded-review.md`: historical missing non-HTTP drain finding; latest draft STATUS says generic native drivers were subsequently staged, still unreviewed. Read together rather than treating the old defect as automatically current.
- Draft466 STATUS and four notes above, currently absent from managed plans. STATUS SHA `59ec72cf8e11fa6471de718740ee02b144eca36ffdb39a9bf9e4aef9ae5f5a08`.

The older `/tmp/thaw-promise-chain-source-review489.md` is superseded by the independent review for the same topic and need not be mandatory archive reading. Avoid copying obsolete draft465/earlier creator iterations; draft466 paired sources plus successor and partial rebase preserve the useful dependency chain.

## Receiving-agent instructions

Start from managed handoff/current HEAD. Treat these as archived source evidence. Resolve conflict-bearing rebase semantically against current owners, preserving newer catch/native Promise/Worker/JIT work. Re-audit creator tickets and current intrusive Promise ownership compatibility before applying old hidden-ticket ABI. Full creator/context/report closure remains HOLD; tests and source controls are unrun. No GitHub upload or publication was performed by this inventory task.
