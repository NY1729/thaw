# Reducer emitter and Boolean coercion source review — unit496

Live candidate `/tmp/thaw-luna-jit-result-chain/candidate`, baseline managed f4e2faf98b6973306ac634f2d78968904b846480. Source only; no execution/product edits. Read codegen.rsfa628f3b77d1a03e78980cfb6024459614f2bf28f927a0bf05943efff3ffb8e5; callbacks/jit.rs8378d21971d34ffd5c2520b56f03a8f23c1053fffdd9c8e6c03496c5b54a3312; globals.rs0c9fcd6d9b0050ef0ed610167e30f91e3207a9f6fdf95d52c2fc485ca910839b. Mutable read pins, not freeze acceptance.

## Reducer source status

NumberArrayJitReduce and DynamicArrayJitReduce now capture into depth-1 after collapsing operands, which is the actual live result slot. No subsequent generic present overwrite appears in those arms. Seeded captured call publishes original initial slot depth-3; noncaptured publishes depth-2 before native helper. Prefix XMMs and RDI are preserved by spill helpers.

Runtime reducers now carry accumulator word plus separate state, import accumulator/element presence into next callback and publish final accumulator state. This covers no-callback result if supplied initial state was correctly snapshotted. Dynamic seeded reducer no longer boxes only absent results into a numeric accumulator representation in the inspected successor. Its initial snapshot occurs at entry before callback compilation/capture processing.

Concrete Number snapshot defect remains in read: primitive_array_jit_map_impl272 has `let initial_presence = read_call_presence()`, while number_array_jit_reduce entry639+ lacks this declaration and references initial_presence685. This is a lexical source defect, not a compiler result. Move the binding to Number reducer entry before compile_jit_callback or any hook can affect output TLS; remove unused map binding. Root and author independently notified. Do not claim corrected until a successor read verifies it.

## Concrete Boolean coercion defect

AsBoolean2894 only clears metadata to present, leaving the raw payload unchanged. CLI append_boolean(Number) emits asbool. An undefined/null operand can carry arbitrary nonzero stale raw bits; clearing its state converts it into a truthy present value. Present NaN also stays NaN while downstream Boolean truthiness uses !=0, so classification does not normalize Boolean payload to0/1.

BooleanNot calls boolean_not(raw) before clearing state. The helper returns `(value==0 || value.is_nan())`; this is numeric negated truthiness only. An absent nonzero payload therefore yields false instead of true. Presence must be read before either operation consumes raw bits.

Minimal policy for numeric/Boolean AsBoolean and BooleanNot: state1 undefined or2 null yields false for ToBoolean, regardless of word. State0 present Number yields value!=0 && !is_nan. Emit normalized f64 0/1 into selected slot, then set state present. BooleanNot negates that normalized result. Existing numeric helper can be reused by materializing absent to zero before boolean_not; AsBoolean still needs actual numeric truthiness normalization (double boolean_not is valid but one helper or comparison sequence is smaller). Do not use Number coercion as a universal Boolean policy for String/Dynamic: those routes must use their existing string/dynamic truthiness helpers after an absence guard, avoiding absent pointer decode.

ConditionalStart, short-circuit and other truthy consumers must follow the same kind-aware contract; fixing these two operations alone does not close full coercion. Preserve legacy numeric comparator token semantics and supported syntax. Output proof should classify normalized operations Boolean only after payload and presence are coherent.

## Unrun controls

Number/Dynamic reducer with undefined/null initial and empty input; nested callback compilation between initial publication and runtime snapshot; callback returning optional then next-iteration comparison; result at nonzero base depth proving capture into depth-1; absent stale nonzero payload under asbool/not; present NaN, signed zero and ordinary Number; optional String/Dynamic truthiness without pointer decode. All controls unrun.

Marker: REDUCER_SLOT_SOURCE_CORRECTED_INITIAL_BINDING_AND_BOOLEAN_COERCION_HOLD.
