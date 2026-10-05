# Actual JIT callback input environment — unit496

Source only. Live `/tmp/thaw-luna-jit-result-chain/candidate`, managed baseline f4e2faf98b6973306ac634f2d78968904b846480. Read CLI callables.rs4727deab10b478a638199cbe953086e1e511913a0fb7f049c7d3006ee23345a5; expressions.rsb0857eb69250674cba10aa6299c5ad555d96e74408265ed45ee266969c30662f; runtime callbacks/jit.rs31c2188d94ebd210b31ad841fe42359c1df2ec00e52e0a78a50483f4a5cfad3b. Mutable pins, no execution/product edits.

## Ordered actual builtin words

N=Number, E=primitive source Number/Boolean/String with optional presence, A=raw source array with exact element family. T/P are paired dynamic tag/payload, NOT two freely interchangeable Number arguments. Captures follow the exact builtin count, up to total16.

| Caller | Actual slots before captures | Count |
|---|---|---|
| Primitive map/scan untagged | E, N index, A | 3 |
| Primitive map/scan tagged | element T, element P, N index, A | 4 |
| Dynamic map/scan | element T, element P, N index, array T, array P | 5 |
| Number seeded/unseeded reduce | numeric accumulator with optional state, numeric element with optional state, N index, Number A | 4 |
| Dynamic seeded reduce | numeric accumulator with optional state, element T, element P, N index, array T, array P | 6 |
| Dynamic unseeded reduce | accumulator T, accumulator P, element T, element P, N index, array T, array P | 7 |

Captured/noncaptured and left/right variants share their row. Unseeded first accumulator comes from actual first/last own array entry, including explicit undefined; holes are skipped. Dynamic source tags are validated Number/Boolean/String array tags. Tagged payload can contain numeric value, Boolean bits, String pointer or undefined's irrelevant zero depending on paired tag. Array payload is raw typed array reference, not numeric scalar.

CLI callables.rs785+ generates these offsets: dynamic map u0/a2/u3; dynamic seeded reduce a0/u1/a3/u4; dynamic unseeded u0/u2/a4/u5; primitive map/scan now uses tagged u0/a2/typedarray3; primitive reduction uses four raw slots. capture_offset is4/5/6/7 accordingly. Runtime tagged_arg0 detection must agree validated environment; merely finding DynamicArgument0 is not authorization to reinterpret a different packet.

## Minimal proof interface

Pass callback_result_kind analysis an actual argument environment from the caller. Check each Argument token assertion against that environment, including optional state and container family. DynamicArgument(i) requires the matching paired tag/payload slots and produces Dynamic from them; direct reads of a payload slot cannot infer Number when it may be a String pointer. Numeric tag word is Number; payload is paired representation evidence. Preserve array/dictionary family only where consumers need it. Runtime tag validation complements proof for genuinely dynamic pairs.

compile_jit_callback currently runs before some source/capture environment assembly. Move environment construction before proof, then invoke callback compiler with that environment. Runtime named callers above supply builtin kinds; captured packet supplies capture kinds. required_args<=16 stays supported. No changes to public f64 callback ABI are needed; environment belongs to compiler/proof and private capture metadata.

## Actual capture producer gap

encode_numeric_jit_callback in CLI callables.rs builds capture prefix from jit_expression_kind and array_prefix/dictionary_prefix, assigns offsets, selects used captures and returns original capture expression tokens. append_jit_captures915 emits arrayempty, each expression, captureappend. Parser currently aliases captureappend to NumberArrayAppend. Thus the physical capture buffer deliberately stores mixed Number/Boolean/String/container pointer words in one nominal numeric array, erasing per-entry family. Runtime capture_arguments can recover length/presence/raw words and root the buffer, but cannot infer each family's representation from raw bits. Callback token sN itself cannot be the authority for this erased layout.

Smallest producer evidence extension: give captureappend a distinct IR operation, retaining the analyzed input family's evidence from the actual outer expression before append. Transport a parallel per-entry kind descriptor with that capture buffer through the existing captured-call path, and carry it in CapturedArguments alongside presence. This is private capture packet metadata, not a generic runtime registry or new class. Choose a dedicated arena capture record/companion descriptor that leaves ordinary numeric arrays untouched; ensure array/source roots trace the record and child buffers. Define checked producer/callee ABI together. A descriptor appended to callback text is only a claim unless compared to actual capture producer evidence; do not solve by trusting matching user strings twice.

For legacy captured-call inputs built as an ordinary homogeneous array, use its actual declared element family (plus presence) where available. Mixed legacy capture producers require their existing source expression evidence to be preserved; do not silently assume all Number or reject mixed supported frontend captures. Array/dictionary captured subtype derives from its producer token/layout; dynamic capture was already unsupported in the inspected frontend, but no additional type cut is authorized.

## Owners and unrun controls

CLI callables.rs capture production and offset table; expressions.rs map/scan/reduce selected ABI; parser.rs distinct captureappend evidence; analysis.rs argument environment and container/pair requirements; codegen.rs capture descriptor transport; runtime capture packet/callback compiler actual caller environment. Reuse current root/presence packet and existing kind enum refinements.

Controls: callback annotation incompatible with actual primitive source; direct tagged payload misread; mixed Number/String/Boolean/array/dictionary captures; capture indices8/15; tagged primitive/dynamic map/scan offsets; both dynamic reducer rows; first undefined accumulator; recursive/nested capture environments and source roots. All unrun. Full producer/consumer proof remains HOLD until environment reaches every caller.

Marker: IMPLEMENTATION_ENVIRONMENT_PLAN_ONLY.
