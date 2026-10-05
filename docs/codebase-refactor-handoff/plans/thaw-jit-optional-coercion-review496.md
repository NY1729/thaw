# Optional coercion source review — unit496

Verdict: HOLD for coercion closure. Source-only mutable scratch review; no product execution or edits. Candidate `/tmp/thaw-luna-jit-result-chain/candidate`, managed baseline unit496 `f4e2faf98b6973306ac634f2d78968904b846480`.

Read SHA256 pins:
- JIT compiler/program/analysis.rs `90887a11cb994f5e9bb4ba964844f126d8388d96088bc730bbfda408f3bfbd04`
- JIT compiler/program/codegen.rs `b019a81651115e85fb1c48e37c51180920ac4b27fa9191d8a5ebb79e6174c8f8`
- JIT compiler/program/parser.rs `9ac6122b5c9be174741abe4ec8c460b62ac6dcab6569468b78f296b1e1e1756a`
- JIT runtime/values.rs `16bb81964d98a7c52775f13c7263c7ade567d8d614d7ae84da30d9fc33be51cd`
- JIT compiler/machine.rs `1c88f7c6482ac4f1800a2bc514f51089b71520a9da589cc0b4fe490f92a60952`
- CLI registry_integration/jit/returns.rs `f162cadc990922319e6ac447af4225e6ae8fb6dc2ed0e18ad618402b866e2266`

## Concrete gap

Analyzer1259 groups StringToNumber and StringTruthy with String member operations and requires exact String. Analyzer1271 requires exact Dynamic for DynamicToNumber/String/Boolean. Consequently MaybeString and MaybeDynamic results (for example StringArrayAt or DynamicArrayAt followed by conversion) cannot retain supported coercion behavior under current callback proof. The emitter cannot simply relax those checks: codegen327 and394+ call helpers with raw payload, ignoring the parallel presence byte. `string_to_number` values1011 treats payload as C string; Dynamic helpers validate a dynamic descriptor. An absent slot can contain a stale payload, so direct decoding is wrong.

CLI returns.rs1383 append_string/1402 append_number emits numstr/boolstr/dynstr/strnum/dynnum based on expression family, without an explicit presence normalization step. This is actual producer plumbing, not a reason to reject optional syntax. Parser49–60 exposes the same legacy conversion tokens.

## Minimal coherent repair

Reuse `emit_dynamic_tag_with_presence` machine112 and `tag_primitive_with_presence` values173 for coercion-specific normalization. Kinds0/1/2/3 are Number/String/Boolean/Dynamic. State1 maps to boxed undefined, state2 to boxed null; present Dynamic validates/reuses its existing descriptor, present String is rooted while boxing. Then call existing dynamic_to_number/string/boolean and mark the conversion result present. Permit matching Maybe families only on those repaired coercion operations; retain strict representation mismatch rejection. No new carrier framework is needed.

Required policies:

| Operation | undefined | null | present value |
|---|---|---|---|
| Number conversion | NaN | 0 | existing family conversion |
| String conversion | "undefined" | "null" | existing family conversion |
| Boolean conversion | false | false | existing family truthiness |

Dynamic helpers already implement these boxed cases at values335/975/1026. Numeric Boolean emitter already handles absence correctly. NumberToString and BooleanToString also require presence-aware conversion: normalizing undefined to NaN or false first would produce the wrong string. StringTruthy should accept matching MaybeString with false for absent/null, using the same boxing/Boolean path or an equivalent checked presence branch. AsBoolean remains numeric/Boolean representation-specific; String/Dynamic frontend producers must use their matching conversion route, not decode their pointer word numerically.

String member access is a separate contract. StringLength, character methods, trim/case/normalize and receiver methods must throw on null/undefined receiver through existing error channels. They must not inherit coercion's null-to-string or false policy. If these optional receiver routes are supported by frontend lowering, add the checked receiver-presence error before any pointer decode rather than rejecting the supported route or silently coercing it. StringConcat/SameValue comparisons require their own producer semantics and are not blanket conversion aliases.

## Remaining dependencies

Preserve current spill/register ABI and check helper errors using existing call error handling. Retain String/Dynamic roots across the new boxing and conversion calls; the present String boxing helper already establishes a temporary root, and enclosing callback scope/root lifetime still needs end-to-end review. Actual caller environment, mixed capture metadata and typed container/field authority remain separate HOLD gates. No claim is made that relaxing Maybe checks alone fixes them.

Unrun controls: absent/null StringArrayAt→strnum/StringTruthy; absent/null DynamicArrayAt→dynnum/dynstr/dynbool; present empty/nonempty String; boxed undefined/null Dynamic; undefined/null number and Boolean string conversion; absent String member access must throw; representation mismatch must fail proof without pointer decoding. Captured and recursive variants must preserve the same state contract.

Marker: OPTIONAL_COERCION_NORMALIZATION_REQUIRED_FULL_JIT_HOLD
