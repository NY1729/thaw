# Catch binding mutation and closure plan, HEAD492

Authority: managed source at `f15e942a0362f4ff8d5ad1d7225bccadb70c9277`. Source-only planning; no code edits, product execution, tests, build, compiler, parser or formatter. Historical bodies were read from `/tmp/thaw-ledger-reconcile-six-records-post365/base.json:12316` and `:12393`.

## Revalidation

- `01a0f23e-ae53-7393-992b-5e018f197e10:0`: the exact reported `throw 42; catch(e) { e = "replacement"; typeof e }` stale-tag path is covered by the new carrier source route. Catch binding is declared as `caught_exception_carrier_type` (`statements/lowering.rs:2686`); ordinary assignment coerces RHS to the declared union (`assignments/lowering.rs:1339–1344`, `inference/coercions.rs:1026`). typeof's original hidden-tag branch explicitly excludes this carrier (`expressions/lowering.rs:2442`); union typeof reads the current value/tag (`:2544`). `record_binding_write` invalidates flow narrowings. No runtime result was demonstrated, and this does not establish every mutation consumer.
- A remaining concrete mutation defect exists: `expressions/lowering.rs:1132–1168` still treats *all* catch scalar assertions as old synthetic payload reads. After `throw 42; catch(e) { e=7; e as number }`, assertion returns `e__thaw_exception_f64`, the original 42 slot, rather than current carrier member. bool/i64 share this path. Updating metadata only on assignment would retain two sources of truth and miss closures/other writes.
- `01a0f237-7e8e-7c52-8051-db3ef6350efc:2` remains HOLD for scalar assertions. Arrow body lowering inserts that synthetic payload name only after `saved_scope` is taken (`expressions/functions.rs:242`); capture selection at `:501–516` filters against saved_scope. Shorthand/function-expression routes similarly use saved_scope (`:607–618`, `:1232–1244`). Thus `()=>e as number` references a variable omitted from the closure capture list. LLVM's catch handler creates the synthetic slot only in the outer function (`statements.rs:740–789`), while closure body variable maps are rebuilt from declared captures (`values/closures.rs:1018–1048`). A body-only variable does not become available merely because the outer LLVM catch created it.
- Native object assertion for the new carrier already uses `UnionValue(Var(e),6,...)` with tag and class identity checks (`expressions/lowering.rs:1053–1080`), so it captures the pre-existing carrier and avoids the old synthetic object insertion. Legacy Str/Promise callback assertion routes still exist; their broader callback capture authority is separate and must not be declared resolved.

## Minimal Luna implementation

Change the lexical-carrier scalar assertion branch in `expressions/lowering.rs` before the legacy/promise suffix fallback. For carrier-typed source binding, derive the asserted scalar from that binding's current union member (F64 index 0, I64 index 1, Bool index 2; obtain indices from the shared carrier layout). Reuse the native-object assertion's checked conditional pattern for a mismatched active member, with a normal error and unreachable typed result. Read the carrier once if needed. Preserve legacy Str and Promise pending-channel behavior as a separate route; do not silently reinterpret them.

This avoids synthetic payload insertion for the new lexical carrier and makes saved_scope capture selection correct without broadening it to body locals. Do not replace saved_scope with self.scope: that admits closure-local temporaries as outer captures and hides the root cause. No assignment metadata synchronization, new global payload state, new closure ownership abstraction, or synthetic per-catch scalar cells are needed.

Before accepting, inspect scalar member behavior when the active carrier member is Json (exact QuickJS capture): decide consistently with existing assertion/coercion contracts whether checked Json scalar extraction is required. Do not return a stale native slot or reinterpret pointer bits. Ordinary `as number` remains an assertion rather than an implicit `Number(...)` conversion; wrong-member behavior must follow the established carrier checks or be explicitly retained as a bounded gate.

## Unrun controls to leave with the fix

1. Original mutation typeof control: numeric throw, replace with string, observe `string`; add bool/number/null transitions through same binding.
2. Scalar mutation assertion: throw 42, assign 7, read `e as number` and expect 7; bool and supported i64 variants. Include destructuring assignment to catch binding because it uses a sibling write route.
3. Escaped arrow and ordinary function returned from catch, asserting scalar, invoked after another nested catch overwrites pending channels; expect original captured current value.
4. Capture a closure first, mutate e afterward, invoke closure: expect the updated value. This exercises shared variable-cell capture rather than a snapshot of hidden scalar payload.
5. Escaped native object assertion closure, including mutation to another object of the same class; assert the current object's field/identity. Wrong member takes checked failure, not unknown variable or raw dereference.
6. Shadowed inner catch and outer catch closures retain distinct carriers; a source named like a compiler suffix must not acquire authority over another catch's internal payload.

Existing closure allocation promotes declared captures to shared arena cells (`values/closures.rs:124–133`), stores those cells in the closure, and rebinds body captures. The new path therefore follows the ordinary union capture route and its arena references. Verify the union cell's active Json/JsValue reference propagation by reading promotion, assignment reference registration and catch rooting consumers; broader escape/Promise/JIT lifetime gates remain unresolved. These controls are source fixtures only until execution is authorized.

Bounded recommendation: repair scalar assertions at their common carrier projection source, then independently review all generated references/capture lists. Do not mark both historical records fully resolved merely from unit492 or from a source fixture.
