# Switch default dispatch flow plan — unit496

Source only, live candidate `/tmp/thaw-luna-jit-result-chain/candidate`. No execution or product edits. Read pins: analysis.rs929d98259c2271ee719ae15d06cfb3b3b36a24d3727e9d2979924936aef3eb6e; codegen.rsa58f8c2b48c71285b55a797d24c87ab977ef6304dd3689dd2f27980496811830; parser.rsf1282fef9badf6f0fa458234e5edacabb1fcbbc4694189dda19d4b50873be872. Managed baseline f4e2faf98b6973306ac634f2d78968904b846480.

## Exact control edges

Parser.rs416–421 maps switch/case/casebody/default/switchbreak/switchend directly to the six NumericValue markers. Case expression lies between CaseStart and CaseBody; the latter consumes its comparison boolean, preserving the full dispatch prefix including the discriminant.

Codegen.rs1985+ emits a body fallthrough jump at each CaseStart, then patches previous failed comparisons to the expression start. CaseBody2025+ records failed comparison exits and patches body fallthrough directly to the body, skipping the expression. Default2031+ records its body address but inserts a dispatch jump past the body. SwitchEnd2061+ patches the final unmatched dispatch to that saved default body; breaks target switch end. Thus default in first or middle textual position receives the state after ALL case expressions on unmatched dispatch. Default fallthrough separately receives the preceding body state and skips later comparisons when falling into their bodies.

Analyzer225–316 tracks failed-dispatch separately from pending body fallthrough, but Default279 uses dispatch_stack at its textual position. Later case expressions can mutate a stack/local representation before the final machine jump back to default. The current proof therefore analyzes the wrong default input. Default-last already receives final dispatch. No-default exits must retain final unmatched dispatch. Never reset failed comparison effects to the original switch base.

## Minimal implementation using current interpreter and retry machinery

Use a bounded discovery pass followed by one ordinary switch analysis, rather than a generic CFG framework. Store start index, entry full stack/reachability, and a small phase enum in CallbackSwitchState. Add a final-unmatched default seed consisting of full stack plus reachability. This seed is exclusively a default dispatch edge; it must never replace the initial dispatch stack.

Before ordinary bodies are analyzed, discovery walks the switch with the existing transfer match and structured-control stacks. On each top-level CaseStart restore discovery dispatch; interpret the case expression normally; on CaseBody pop the comparison and save full failed-dispatch state, then make the body unreachable for discovery. Default records its location and likewise suppresses its body. Body fallthrough, breaks and returns do not contribute during this phase. Case-expression throws/returns still kill their dispatch path; there is then no fictitious unmatched default edge. Discovery requires the same nested-marker matching as normal interpretation: markers of a nested switch in a suppressed body cannot act on the outer switch; a nested switch inside an evaluated expression uses its own phase/state. Maintain structural bookkeeping even while executable body transfers are suppressed.

At matching SwitchEnd, capture final dispatch state and replay from SwitchStart with original entry. At replay Default join that final unmatched seed with reachable preceding body fallthrough. All other CaseStart/CaseBody behavior stays as currently implemented. Normal SwitchEnd aggregates body end and breaks, excludes unmatched exit when a default exists, and pops the discriminant once. Default seed has the dispatch base length after comparison pop, not the temporary boolean slot.

Discovery must not publish effects to enclosing analysis records. Snapshot/restore enclosing results returns, loops exits/backedges/continue_edges and headers, tries exceptions, catches, branches/guards, outer switches and early_returns. Restore full records, not just vector lengths: an inner loop can mutate header states or target an outer continue. Existing loop retry (`index=start_index+1`) demonstrates the interpreter already supports replay, but its accumulator policy must not be blindly copied. A bounded private snapshot of existing records (Clone where needed) is sufficient; no new runtime carrier or universal CFG is needed. The ordinary pass publishes actual case-expression and body effects once.

Do not first analyze default with the stale state and retry at end: that pass can reject a valid operation or incompatible join before reaching SwitchEnd. Discovery must suppress body proof transfers from the beginning. Do not make discovery unreachable for case expressions merely because a prior body returned; dispatch is a separate edge.

For enclosing loop retries, recreate/recompute each switch discovery seed from the new loop header, rather than caching by index alone. Nested switch phases belong to their stack frame. This avoids stale seeds across widened loop input and preserves existing Maybe/Unknown joins.

## Unrun controls

Default first, middle and last with later failed comparisons mutating an aliased local from scalar to optional scalar: default reads must reflect final mutation. Include default fallthrough to later body while comparison expressions are skipped; earlier matching body fallthrough into default; no-default unmatched exit; breaks after mutated prefixes; case-expression throw and return; default throw/return; nested switch in a body and in an evaluated expression; enclosing loop retry and outer continue from a switch body. Compare full prefix and terminal proof, not only selected result type. These controls are unrun. Full presence/equality/callback gates remain separately open.

Marker: IMPLEMENTATION_PLAN_ONLY_SWITCH_DEFAULT_HOLD.
