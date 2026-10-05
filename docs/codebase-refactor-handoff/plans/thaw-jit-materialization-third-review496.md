# JIT materialization third source review — unit 496

Bounded source review; full callback gate remains HOLD. No product execution, tests, parser, compiler, formatter or build was run. Mutable candidate at `/tmp/thaw-luna-jit-result-chain/candidate`; managed baseline unit496 `f4e2faf98b6973306ac634f2d78968904b846480`. No product source edited.

Read SHA-256 pins (paths relative to candidate `crates/thaw-jit/src/`):

- runtime/arrays/callbacks/jit.rs: `31c2188d94ebd210b31ad841fe42359c1df2ec00e52e0a78a50483f4a5cfad3b`
- runtime/globals.rs: `23a1b710e5724878dc0798a118643a54851598b979b0085ae29865d85bd0b691`
- compiler/machine.rs: `1c88f7c6482ac4f1800a2bc514f51089b71520a9da589cc0b4fe490f92a60952`
- compiler/program/codegen.rs: `b019a81651115e85fb1c48e37c51180920ac4b27fa9191d8a5ebb79e6174c8f8`
- compiler/program/analysis.rs: `90887a11cb994f5e9bb4ba964844f126d8388d96088bc730bbfda408f3bfbd04`

## Corrected bounded paths

Number reducer entry jit.rs646 snapshots incoming initial presence before callback compilation/capture preparation; Dynamic entry774 does likewise. The earlier misplaced Number binding is absent from this snapshot. Number accumulator retains word plus presence and passes accumulator/element metadata to callbacks; Dynamic seeded entry keeps the supplied numeric word plus presence rather than converting absence to an unrelated boxed-pointer representation. Both publish final accumulator presence, including no-callback supplied-initial cases.

Codegen Number reducer931+ and Dynamic1220+ publish the actual initial slot (depth-3 captured, depth-2 uncaptured) before the helper. After argument collapse both capture terminal state at depth-1, the live result slot. These arms do not subsequently overwrite it with generic present metadata. This closes the previously reported wrong-slot capture in the reviewed paths.

Primitive scan, including captured scan, forwards optional element presence into the untagged callback packet instead of rejecting every non-present element. The tagged callback path remains separately represented by tag/payload pairs. Captured packet source roots and sixteen-state copying remain as documented in the earlier packet review; this review makes no new claim that mixed capture families are proven.

AsBoolean and BooleanNot at codegen2898/2904 use `emit_number_boolean_with_presence`. Its machine emitter spills/restores the live prefix and RDI, passes numeric payload in XMM0, presence in EDI, negate in ESI, and marks the resulting Boolean present. The globals393 helper computes truth as presence==0, nonzero, non-NaN; therefore absent/null metadata cannot make stale nonzero payload truthy. Negation follows the normalized truth value. This is coherent for the numeric representation; it is not proof for String/Dynamic pointer operands.

No additional concrete defect was found in these corrected bounded reducer/scan/numeric Boolean paths by source inspection.

## Remaining dependencies and controls

Runtime callback compilation still calls `callback_result_kind_code()` at jit.rs166, rather than an actual-caller environment entry point. The analyzer environment API and mixed capture kind descriptor are not yet integrated producer evidence. Full operand-family proof remains required, including container element families and actual object/tuple backing layouts for field tokens; parser annotations alone cannot authorize pointer decoding. Existing runtime layout guards should be inspected before adding metadata. These are the existing HOLD gates, not a request to reject supported syntax.

Controls remain unrun: seeded Number and Dynamic reduce with undefined/null initial and empty input; right-reduce equivalents; captured variants; callback returning undefined/null and subsequent iteration; primitive scan explicit undefined and holes; numeric Boolean conversion/not of absent stale nonzero payload, null, zero, signed zero, NaN and nonzero. Broader equality, object/tuple guards, caller input environment and String/Dynamic lifetime closure are not accepted by this bounded review.

Marker: BOUNDED_CORRECTIONS_SOURCE_COHERENT_FULL_JIT_HOLD
