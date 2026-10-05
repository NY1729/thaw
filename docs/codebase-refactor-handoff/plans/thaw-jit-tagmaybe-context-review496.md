# Optional equality boxing and nested context review — unit496

Live source review only of `/tmp/thaw-luna-jit-result-chain/candidate`, managed baseline f4e2faf98b6973306ac634f2d78968904b846480. No execution, product edits, or acceptance of the full JIT finding.

## Read hashes

machine.rs327cf927ecfa71e881d2e1f33f894fc53b0ba1af01cd0f1a03954869c7592edd
analysis.rsbcb0517a71320714c954907e334a128eb3d7749f47d5362cbf5ca9b4a958bfb7
codegen.rs780409782bbb29b7d9c3737ecee69c129acdb61bcaab36eed5b2ac3c3a2b198c
parser.rs9ac6122b5c9be174741abe4ec8c460b62ac6dcab6569468b78f296b1e1e1756a
ir.rse1b812086b4ddfa3c65f3c859548df48783236e498b287e15613ea8b4bfa6834
runtime/values.rs16bb81964d98a7c52775f13c7263c7ade567d8d614d7ae84da30d9fc33be51cd
runtime/globals.rsb9bba9ed25624895d4e42c1e567dc44c993e3725d8b50ddc9c0986a688340867
CLI conditions.rs4429a365c08289fa410ab3edf4b1aab49d4a24487ac2ed5135b9d8421974274f
CLI returns.rsf162cadc990922319e6ac447af4225e6ae8fb6dc2ed0e18ad618402b866e2266
CLI jit_validation/expression.rs22fbe9bb66df4a527b0966b7a66a6bb37594e327ced958526e8b8d1fb8ba4874

Author edits occurred during reads; hashes are read observations, not an immutable bundle verification.

## Concrete remaining proof defect

analysis.rs generic operation transfer consumes inputs and pushes callback_operation_kind's output. That output table groups TagMaybePrimitive(_) with Dynamic producers without validating the consumed kind. Consequently its callback result proof can classify a mismatched present scalar as Dynamic after a String tag operation. CLI jit_validation checks exact Number/String/Boolean/Dynamic families, but parser.rs74–77 also exposes the tokens directly to NumericProgram; that CLI check is not a complete producer/caller closure.

Required correction: explicit analyzer transfer for TagMaybePrimitive before the generic arm; accept matching Number/MaybeNumber, String/MaybeString, Boolean/MaybeBoolean, Dynamic/MaybeDynamic families. Use authoritative argument/local/field source annotations to resolve unknown inputs. Do not accept a wrong family or invent a type from the requested tag. Runtime values.rs173 helper trusts scalar payload representation and only validates existing Dynamic for kind3, so the proof matters. This is a concrete proof hole; it does not establish that every malformed token is currently executed by a public caller. Full proof wiring remains an explicit independent gate.

## Source-coherent changes

CLI conditions.rs preserves actual equality intent by emitting dyneq/dynne/dynseq/dynsne and append_dynamic now uses tagmaybe tokens. Parser stores explicit kind0..3. The runtime helper selects undefined/null from presence before decoding a present representation, so absent String payloads are not interpreted as String pointers. It roots a present String during wrapper allocation. The dynamic record's pointer payload is an aligned arena word, consistent with existing tracing. This does not prove all callback scopes/epochs or nested allocation roots across other operations; those remain in the full result lifetime plan.

machine emit_dynamic_tag_with_presence spills prefix XMMs/RDI, moves selected value to XMM0, sets kind XMM1, reads selected presence at spill-relative72+slot, converts it into XMM2, calls the three-f64 C helper, places result back in the selected XMM and restores prefix/RDI. Codegen marks the boxed result present and checks CALL_ERROR. This bounded instruction/ABI read found no new encoding error.

Nested context correction is coherent in the read source: permanent frame rounds 32+static PresentConditionalStart count to16; context offset is32+active context nesting; save occurs before the presence test, so both branches initialize it. Save/restore use unsigned disp32 byte memory encodings, avoiding disp8 sign-extension at large nesting offsets. PreserveAbsent selects the innermost context; ShortCircuitEnd pops the context only for its matching branch index, preserving enclosing ordinary branches. Matching lexical closures are still emitted after early return/throw, so compile-time context stacks can unwind although those runtime paths terminate. Loop reentry saves current metadata again. Frame-size epilogues cover error, uncaught throw, early and final return through the shared helper; call spill and recursive temporary-frame offsets remain relative to permanent-frame top and are not changed by its growth.

These observations are bounded: all supported structured paths still need parser/codegen/analyzer closure and unrun nested loop/try/return controls. Context correction alone is not whole presence acceptance.

## Unrun controls and open gates

Add matching and mismatching TagMaybe input-family proof controls including direct parser paths, optional cross-family strict equality, undefined versus null, present NaN/signed zero, optional String with deliberately irrelevant absent payload, and present String surviving nested callback allocation/reset scope. Add outer absent plus inner null contexts sharing selected slot0, intervening ordinary branch, loop reentry, inner throw/catch, result-return and early return. All are unrun.

Switch deferred-default dispatch, coercion families, full producer proof wiring, typed input/recur closure, scoped callback roots/epochs and materialization remain independently HOLD. This report does not reject supported syntax or redefine legacy scalar comparator tokens.

Marker: PARTIAL_SOURCE_REVIEW_HOLD_TAGMAYBE_INPUT_PROOF.
