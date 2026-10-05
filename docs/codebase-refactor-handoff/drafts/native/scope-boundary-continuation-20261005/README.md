# Native Promise scope-boundary continuation, 2026-10-05

## Status

**Bounded prerequisite source review: ACCEPT. Product integration: HOLD.** No Thaw package build or test pass is claimed. This archive records an incremental continuation of the existing held native draft, not an integrated product fix or a replacement for the broader native/JIT/creator plans.

Repository: NY1729/thaw. Target branch: refactor/codebase. Input remote head: a6b4fad1723f41ff009b905d7ace2a9a2b5ce6bc. Product checkpoint: f4e2faf98b6973306ac634f2d78968904b846480 (unit 496).

## Contents and application base

- `base/` and `candidate/` hold exact paired changed-owner snapshots: 17 existing base files and 18 candidate files. The new resolver test is recorded in `base-absent.txt`; an empty base is not fabricated.
- `delta.patch` changes only these 18 owners, relative to the already-applied archived native draft. It is not a patch directly against the remote product checkpoint.
- `manifest.json`, `base.sha256` and `candidate.sha256` pin the source identities and integrity checks.
- `REVIEW.md` records the final immutable source review.
- `TEST-PLAN.md` lists the compile-first checks and 16 LLVM/3 HIR focused controls still required.
- `VERIFICATION.md` distinguishes executed source/synthetic checks, unavailable package tests, and the unrelated failing remote baseline.

To reconstruct, start from the exact product checkpoint and the original `drafts/native/` paired archive, respecting its manifest and held prerequisites. Verify that the resulting files match this continuation's `base/`, then apply this delta in an isolated draft. Do not overwrite newer user files with these snapshots. The other original native draft owners remain unchanged. Complete repository/dependency closure is required before compilation; this changed-owner archive is not a complete runnable checkout.

## What changed

1. Catch and loop targets carry their Promise cleanup boundaries through nested code-generation, module initialization, async and host-adapter contexts. Scope restoration runs on Result errors as well as successful generation.
2. Reactive preheader promotion installs and restores the complete physical-owner context. If-arm siblings start from isolated maps, and live continuation ownership records are merged without treating bookkeeping alone as runtime ownership.
3. Escaping synchronous/async exception cleanup preserves the native reason before clearing pending state and releasing Promise owners. Blocking Promise-to-pending and pending-to-async-completion handoffs preserve the native descriptor.
4. A private published-tuple marker is distinguished from trusted text-only bytes. Finally snapshots a complete ten-channel tuple only for the published form. Plain strings normalize stale typed fields, and pending rethrow keeps native-text and native-descriptor operands separate.
5. Focused controls exercise actual sibling-map mutation, actual sync/async lambda body errors, preheader contexts, real QuickJS callback handoffs, and typed/string rejection provenance. These controls have not been package-compiled or executed.

## Remaining integration gates

Variable/arena dominance, supported aggregate/union/global Promise ownership, JIT callback ABI, and Promise creator/context lifetime still require their own plan closure and immutable review. Direct Dictionary<Promise> remains unsupported and out of scope. The scope-boundary work is a prerequisite for these gates, not their completion.

Historical findings `01a0f303-d687-7ab2-96c8-2817d096efed:0` and `01a0f0ac-6416-7ba1-b37b-abffc59a0b44:0` remain open. No ledger statuses or the recorded 748-open count were changed. All applicable HOLD gates must be addressed before product integration.
