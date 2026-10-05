# JIT materialization source review — unit496

Live source `/tmp/thaw-luna-jit-result-chain/candidate`; source only, no product edits/execution. Baseline managed f4e2faf98b6973306ac634f2d78968904b846480. Full callback acceptance remains HOLD.

Read hashes: callbacks/jit.rs initially960034d52ad083b43851dd44ac769caf62246b9e2ee8f0f9536d1c3fe678cd29, later d60b7769cb8f8958993a748d4668dee7de13f5cc33a13b96627e4ce7718d197b after author proof wiring; transforms.rs75d83c85b0416684bcb99908403ce519a768ecb704cdbba63bdad280ba6c00f5; values.rs16bb81964d98a7c52775f13c7263c7ade567d8d614d7ae84da30d9fc33be51cd; globals.rsb9bba9ed25624895d4e42c1e567dc44c993e3725d8b50ddc9c0986a688340867; machine.rs327cf927ecfa71e881d2e1f33f894fc53b0ba1af01cd0f1a03954869c7592edd; codegen.rs780409782bbb29b7d9c3737ecee69c129acdb61bcaab36eed5b2ac3c3a2b198c. Mutable snapshots, not frozen pins.

## Actual remaining gaps sent author

capture_arguments copies only raw words and loses array_index_state. call_jit_callback constructs words but publishes no parallel input presence packet, so generated entry defaults optional captures to present. Primitive map/scan explicitly reject state2 undefined unless DynamicArgument0 is used. This fails the supported untagged optional primitive input route. Carry words and state together; translate array state2 to frame state1 undefined, distinguish holes through traversal, publish scoped input pointer/count, restore prior TLS afterward. Retain the captures source root for the complete operation rather than only copying pointer words into a Rust Vec. Root pointer-valued input/accumulator according to proven kind across allocation/reentry.

initialize_recur_input_presence copies8 bytes and emit_presence_import_argument defaults indices>=8 present, while callbacks allow16 arguments. Permanent frame16..31 already accommodates16 input-state bytes with context starting32: copy16 and import indices<16 without increasing frame. The at-most8 live XMM stack/recur packed-argument limit is separate from direct callback input ABI; do not reduce supported16-input callbacks.

number_array_jit_reduce tests only array_index_present, so explicit undefined state2 becomes its raw placeholder word (typically zero). It also converts each callback result immediately to Number, losing optional accumulator state before the next callback and final publication. Carry accumulator JitCallbackResult/parallel state; import undefined/null into each callback, and allow numeric op emitters to materialize only when the operation requires coercion. Apply the same policy to dynamic reduction according to its actual accumulator contract. Preserve skip-hole behavior; first accumulator selection must inspect state, not merely present boolean.

## Corrected/source-coherent portions

Current compile_jit_callback now derives callback_result_kind_code from the full NumericProgram proof and rejects an explicit descriptor mismatch; the old terminal-token heuristic is removed in the later snapshot. This corrects that known caller wiring gap, not every input/representation proof gate.

Output wrapper snapshots CALL_PRESENT/CALL_ABSENCE immediately after callback and restores prior output TLS. normalize_jit_callback_result checks presence before String pointer decode; undefined→NaN/null→0 for Number coercion and false for Boolean are coherent conversion rules. String conversion produces textual undefined/null. Present String/Dynamic conversion roots the original pointer during conversion.

Map output maintains states0 hole,1 materialized value,2 explicit raw undefined, fixing the earlier Some(mapped)-only collapse for raw undefined results. Raw null takes the declared target conversion route. Dynamic boxed undefined/null are present Dynamic values and take dynamic conversion; exact map-result identity versus a declared primitive conversion must be checked against producer contract before claiming full JS map semantics. A typed target conversion is not itself proof that preserving raw null identity is unnecessary.

Mapped String words receive roots through final assembly. transforms mapped_array_result_with_states allocates data then presence before boxing roots both: if the supplied allocator can collect/reenter, root data immediately after allocation before the second allocation. Stock arena allocation does not itself prove arbitrary configured allocator behavior. This is an explicit dependency to source-check, not a claimed observed runtime failure.

## Unrun controls

Optional untagged String/Number/Boolean captures at argument indices8 and15, explicit undefined input versus hole, recursive and nested callback scopes, reducer returning undefined/null then comparing it next iteration, sparse first/last reduce accumulator, String/Dynamic input roots across reentrant allocation, map raw/boxed undefined/null conversion contract. All unrun.

Marker: MATERIALIZATION_HOLD_INPUT_PACKET_CAPTURE_AND_REDUCE_STATE.
