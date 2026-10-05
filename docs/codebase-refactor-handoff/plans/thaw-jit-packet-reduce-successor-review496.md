# JIT captured packet/reduce successor — unit496

Live candidate `/tmp/thaw-luna-jit-result-chain/candidate`, baseline managed f4e2faf98b6973306ac634f2d78968904b846480. Source only; no product edits/execution. Read pins callbacks/jit.rs125433ea2cc7a31c7a51e0d411c5fa83b611541b1a9dcc9c601dd7871b808b83; globals.rs0c9fcd6d9b0050ef0ed610167e30f91e3207a9f6fdf95d52c2fc485ca910839b; machine.rs9758adfb711e89784ca6132c5fbecc0def523280260568594102ef55f5360978; analysis.rsb6eaa31615cf899ff3ffaeb34a1f25d4682277ced28f0e1508b8a28ccaf1dd14. Mutable reads, not freeze.

## Source corrections confirmed

CapturedArguments carries words, parallel presence and a source ArenaRoot. Captured state1 becomes frame present0; array holes0 and explicit undefined2 become callback undefined1 with irrelevant raw word0. This is coherent capture lookup semantics and avoids absent String pointer reads. The source root survives all callback iterations and roots pointer payloads via the source allocation graph.

call_jit_callback_with_presence combines builtin and capture metadata into a16-byte stack packet, publishes pointer/count, then restores prior RECUR_INPUT_PRESENCE after callback. Generated prologue copies16 bytes into permanent frame16..31 and clears input TLS at entry. Direct Argument indices<16 import metadata; this corrects the previous >=8 loss while retaining max8 live value slots. Output CALL_PRESENT/CALL_ABSENCE is captured and restored separately. Nested callbacks have their own scoped packet whose lifetime encloses the call; restored outer pointer still belongs to an active outer stack frame.

Primitive map untagged optional input uses the packet rather than rejecting explicit undefined. SwitchBreak analyzer now guards base length and truncates the mutated exit prefix before storing it, correcting the extra-temporary break mismatch. These corrections are bounded source evidence.

## Remaining current defects/dependencies

Primitive scan and captured scan still filter state==1 on untagged path, rejecting undefined and holes visited by find-style scans, and use call_jit_callback without builtin presence. They must match map's with_presence route, preserving scan mode's skip/visit-hole behavior instead of cutting optional inputs.

Number reducer in this read still uses raw f64 accumulator, array_index_present boolean and per-iteration normalize-to-Number. Explicit undefined array entries/first accumulator lose state and callback optional result state is lost before next iteration. Maintain accumulator word+presence, import both accumulator and element state, and publish final presence. Supplied initial presence must arrive from actual codegen initial-slot producer, not be assumed present. Empty-with-initial/no-callback result must publish that initial state too. Source element state2 is visited undefined; state0 is skipped.

Dynamic seeded reducer requires result_kind Number, preserves present results as raw numbers, but replaces absent results with dynamic_from_parts allocated pointer before passing/returning accumulator. This mixes Number and Dynamic representation on different branches of the same ABI. Keep raw Number word+presence for the declared numeric route, or consistently change the complete producer/proof/arg0/result contract to Dynamic if that is its actual semantics. Boxing only absent results is incoherent. Sent author immediately.

Unseeded dynamic reducer reconstructs DynamicPrimitive tags/payloads and roots String accumulator payload; final arena_dynamic allocation preserves that payload while the accumulator root is alive. Its callback result kind and initial active-tag policies still need full producer verification. No full reduction acceptance follows from this one route.

Final publication must be immediate from the helper's accumulator state and survive emitter capture; a generic call that forces result present after helper would erase it. Audit numeric/dynamic reduce codegen sites for reset-before/capture-after and initial metadata transport. Rooting must cover source, capture source, present String/Dynamic accumulator and result conversion, with no absent raw pointer root/decode.

## Unrun controls

Captured undefined/hole at indices8/15, nested callbacks restoring outer packet, optional untagged scan mode2/find plus skip-hole modes, reduce undefined first entry, optional callback output inspected next iteration, supplied undefined/null initial with empty array, dynamic seeded absent result followed by numeric comparison, final optional result publication, String/Dynamic accumulator under reentrant allocation. All unrun. Full callback proof/producer/operand closure remains HOLD.

Marker: PACKET_SOURCE_CORRECTED_SCAN_REDUCE_REPRESENTATION_HOLD.
