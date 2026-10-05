# Switch discovery/replay successor source review — unit496

Live candidate `/tmp/thaw-luna-jit-result-chain/candidate`; no execution or product edits. Managed baseline f4e2faf98b6973306ac634f2d78968904b846480. Full callback acceptance wiring remains open.

Read analysis.rs SHA a8ff14cbefa11f8691ba4c4e9fc45092bd611742adfe58943889fc773dcb4ec8. Inspected entry/main transfer, switch225–410, loop retry, exception/result exits, TagMaybe635–649, and snapshot/phase records. This is a mutable read pin, not frozen acceptance. Machine edge authority is the codegen source traced in /tmp/thaw-jit-switch-default-flow-plan496.md.

## Prior exact defects

TagMaybePrimitive now has an explicit reachable transfer. It checks present_callback_kind(actual) equals requested Number/String/Boolean/Dynamic family before consuming and producing Dynamic. Unknown or mismatched inputs cannot acquire a proof merely from the requested tag. The prior output-only proof defect is corrected in this snapshot. This does not certify missing source annotations or full producer/caller acceptance wiring.

Default now receives final failed-dispatch state through discovery/replay rather than textual-position state. Discover starts from the actual entry, evaluates case expressions, consumes comparison, stores full mutated dispatch prefix, and marks body unreachable. Default itself is suppressed during discovery. SwitchEnd captures final dispatch and restores all enclosing snapshot records before restarting at the first marker in Replay. Replay Default joins final unmatched seed with preceding body fallthrough; initial dispatch stays original. Thus later failed-comparison effects reach default-first/middle, while body fallthrough skips those expressions as machine code does. Default-last and no-default paths remain coherent.

## Nested and outer flow review

A switch reached in a discovery-suppressed body is Suppressed, owns its own marker frame, and remains unreachable through its end. Case/default markers act on that nested frame, not the outer discovery switch. A switch reached in an evaluated case expression gets its own Discover/Replay and returns its ordinary result to the expression. The snapshot of that inner switch includes the active outer discovery record, preserving its dispatch state.

The snapshot restores full branch/guard records, result returns, all switch records, loop headers/exits/backedges/continue edges, try exceptions, catch normals, early returns, stack, locals and reachability. This addresses mutations to enclosing loops/results/tries during discovery rather than only truncating vector lengths. Discovery throws/returns kill failed-dispatch reachability; replay republishes their contributions once after restoration. Suppressed inner loops have no exit edges and LoopEnd leaves them unreachable. Suppressed inner try catches with no exceptions are skipped; ordinary structured branch ends with two unreachable arms remain unreachable.

Each visit to SwitchStart creates discovery from the current enclosing loop state. No global seed cache survives a changed loop header. Nested scopes carry their own phase and snapshot. Breaks/end exits in replay remain separate from failed dispatch; discriminant pop occurs once on a reachable switch exit. Full prefix joins preserve local aliases because locals are reconstructed from stack afterward.

No new concrete CFG correctness defect was found in these bounded paths. This is source evidence only, not a claim that all supported syntax or opaque runtime errors are proved. Deep snapshots recursively clone enclosing switch snapshots; that can amplify work for extreme nested case-expression switches. It is an implementation cost observation, not a new correctness HOLD or reason to cut supported syntax.

## Unrun controls still required

Default first/middle/last with later failed case-expression local mutation; preceding matched body fallthrough into default; default falling through to later body without comparison effects; no-default unmatched exit; case-expression throw/early return; nested switch in suppressed body and evaluated expression; outer result-return; outer loop continue and changed-header replay; try exception contributions retained once. Matching/mismatching TagMaybe family controls must cover direct NumericProgram parsing and callback source annotations. All controls remain unrun.

Marker: BOUNDED_SOURCE_DEFECTS_CORRECTED_NO_NEW_SWITCH_CFG_DEFECT; FULL_JIT_HOLD.
