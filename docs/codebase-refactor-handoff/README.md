# Codebase refactor handoff

## Authority and restrictions

Branch: `refactor/codebase`. Product checkpoint: `f4e2faf98b6973306ac634f2d78968904b846480` (unit 496).
The documentation commit containing this handoff adds no product changes.
Planning and review use Sol; implementation uses Luna. All source files may be edited, including the original 41 files.
Tests are performed by the user. Do not run tests, builds, compilers, formatters, parsers, runtime probes, Cargo or npm under the current authorization. Static source review and git patch checks are allowed. No force push or main merge is requested.

## Findings ledger

`findings.json` contains all 1,548 historical findings with status, source location and evidence. At this checkpoint 748 findings remain open; counts describe findings, not files. `fixed_static_reviewed` does not mean execution-tested. Preserve withdrawn and already-resolved statuses. Revalidate old findings against current source before changing them. Do not mark full findings resolved for a partial prerequisite.

## Work to continue

1. JIT callback ABI: wire caller input environments using actual builtin argument layouts; preserve mixed capture kinds at their producer; validate container element/field representations before pointer consumers. Review the latest optional coercion changes and roots, reachability, switch replay, reduce presence and all 16 argument imports.
2. Native exception / Promise ownership: pair catch and loop boundaries across module initialization, function isolation and preheader promotion; route generic pending exceptions through token cleanup; preserve branch ownership snapshots. Complete supported native aggregate slot relocation and global ownership. Direct Dictionary<Promise> is not currently supported; do not expand the feature scope to implement it.
3. Promise creator/context successor: archived dependency sources are in `drafts/promise-archive/` (including an explicitly conflicted partial rebase). Resume only after JIT and native ownership prerequisites are reviewed. Rebase older drafts semantically against the current checkpoint; do not replace files with stale whole-file copies.
4. Continue the remaining ledger across all domains. The two current drafts do not cover all remaining findings.

## Drafts and review evidence

`drafts/jit/candidate.patch` and `drafts/native/candidate.patch` are saved work in progress, not integrated product fixes. Each draft includes exact paired base/candidate source snapshots; `git apply --check` against that paired base succeeds. Some draft base files differ from unit 496: consult `base_matches_checkpoint` in the manifest and rebase those changes explicitly. `manifest.json` records exact owner and patch hashes. The author STATUS files may retain old unit labels; the manifest and this README are authoritative.

`plans/` contains source-backed plans and reviews. References to `/tmp` describe this session; use the copied documents here where available. Latest native scope review: `plans/thaw-promise-scope-boundary-review496.md`. All HOLD findings must be addressed and the resulting immutable patch reviewed before integration.

For continuation, reconstruct separate scratch candidates from this checkpoint, compare the paired snapshots, and rebase each draft there. Keep JIT and native ownership edits separate. Review complete affected caller flows, freeze exact reviewed bytes, commit suitable completed units with `feat(scope): ...`, and update the ledger with evidence. Preserve unrelated current user edits. Static patch checks do not prove compilation or behavior.
