# Native scope v5: final combined Sol source review

## Verdict

**ACCEPT the bounded prerequisite source gate for these exact frozen bytes. Overall product integration remains HOLD.** All scoped I1–I3, F1–F3, extra tuple fixes, and v4 R1–R3 are source-addressed. No remaining Critical, Important or Minor finding was established in this correction. This accepts the source design and authored controls; it does not establish Thaw compilation, successful tests, valid emitted IR or runtime ownership.

V5 changes only two test files from v4. I read the exact correction and affected registration/adapter, async-stage, resolver and test-include flows; the complete candidate comparison confirms every other source byte is unchanged. Prior combined production conclusions therefore carry forward only at the matching hashes below. This report supersedes v4's bounded HOLD for v5, not for mutable or earlier candidates.

The patch base is the archived held native draft, not the remote product checkout. Broader native aggregate/union/global ownership, variable/arena dominance, JIT ABI and Promise creator/context lifetime remain HOLD. Historical broad findings and the handoff's open count are not resolved by this review.

All source locations below are relative to `frozen-native-scope-v5/candidate/`.

## Finding dispositions and evidence

### I1: ADDRESSED

`hir_codegen.rs:301–307` constrains the isolated closure to `Result<_, String>` and restores the scope before returning it. The four callback/N-API adapter result annotations and their production restoration paths are unchanged from reviewed v3/v4. Earlier separate std-only compiler controls support the inference repair, not package compilation.

### I2(a), I2(b), I2(c): ADDRESSED at source level

- **Explicit native throw:** the HIR producer publishes its complete packet before emitting the published-text marker; `hir_codegen/statements.rs:295–309,389–417` preserves only that fresh packet before the enclosing Throw/cleanup route. Plain trusted bytes are treated separately, as detailed under F1(a).
- **Active async rejection:** `hir_codegen.rs:1219–1295` loads the native descriptor and supplies it as the final argument of the ten-argument provenance-aware rejection call. The declared i8 ABI and existing runtime implementation agree; the reviewed escaping path attaches the reason before pending clears and local-owner cleanup. This is unchanged production source.
- **Blocking rejection transfer:** `hir_codegen/promises.rs:1932–1960` includes `thaw_promise_exception_native` in the metadata copy. Both value-taking and void blocking failure callers copy before destroying the settled Promise, as reviewed in v3/v4. The direct control now checks getter SSA → exact pending store → explicit destroy in the same entry block (`tests/async/promises.rs:2500–2518`). Its active-async counterpart checks the actual rejection call before clears and the reachable release block (`2520–2543`).

### F1(a): ADDRESSED

HIR `lower/statements/lowering.rs:275–296` emits `@@thaw_published_exception_text` only after provenance clear and active-member setters. `lower/control_flow.rs:641–658` snapshots its complete ten-channel tuple into the existing pending-rethrow route. Plain trusted markers/raw strings use the bytes-only path at `660–687`; finalizer execution order and handler suspension at `560–566` are unchanged. Inference `lower/inference/types.rs:1571–1573` recognizes the new private marker as Str.

LLVM `hir_codegen/statements.rs:295–309,389–417` recognizes both display forms and preserves the tuple only for the published form. The marker-consumer audit and unchanged forwarding flows remain the v4 compatibility evidence. No public ABI, HIR variant, general registry or broad creator redesign was added.

### F1(b): ADDRESSED

`hir_codegen/promises.rs:562–588` clears native provenance only for the untyped string resolver, before String tag/object/text publication and the shared rejection helper. The typed resolver retains its real publisher tuple. The new paired source controls require a real native-array publisher and inspect defined resolver instructions, not module declarations (`tests/async/resolver_provenance.rs:25–125`).

### Extra tuple fixes: ADDRESSED

- **Ten-channel operands:** `hir_codegen/statements.rs:342–376` distinguishes args[2] native text from args[9] native descriptor, preserving the existing same-binding selection and each destination. `tests/async/resolver_provenance.rs:152–254` checks distinct pointer selects/stores. Dummy sentinel pointers are codegen-only.
- **Text-only String packet:** `hir_codegen/statements.rs:392–417` clears native/text/aggregate pointers, nulls object, sets tag4 and zeros the scalar channels before installing current text. Caught/pending-rethrow early returns remain separate. Exact fresh-pointer and stale-complete-packet controls at `tests/async/promises.rs:2329–2457` cover the repaired source contract.

### F2 and v4 R2/R3: ADDRESSED

The source fixture is unchanged. Its no-await `asyncThrow` now checks the ramp `@asyncThrow(` (`tests/async/promises.rs:2213–2219`), consistent with segment0 emission and direct Throw handling in `async_frames/codegen.rs:162,1102–1108`; resume emits later segments at `436–459`.

The outgoing callback control now follows the actual QuickJS chain (`tests/async/promises.rs:2221–2261`):

1. `__thaw_typed_js_*` selects QuickJs (`thaw-hir/src/lower.rs:54–55`); typed marshalling invokes `compile_quickjs_callback_argument` (`dynamic_host/typed_calls.rs:853–855`).
2. `callbacks.rs:1698–1705,1789–1821` compiles the native closure adapter in deferred-Promise mode. The Promise return chooses a callback guard and a separate finisher (`1069–1075`); the finisher itself receives a guard at `1553`.
3. The real seven-argument registrar call (`1855–1873`, declaration `runtime_declarations.rs:2826–2840`) passes callback guard at argument0 and distinct finisher at argument5. The new test extracts those exact operands and requires distinct guard symbols.
4. It inspects that argument0 guard's definition for the direct raw-adapter call, matching `callbacks.rs:1355–1359`, then locates the raw adapter definition. The internal `__thaw_napi_value_callback_*` name is backend-agnostic here.
5. Registration must precede the actual `thaw_js_call_graph_result` call in the main resume, consistent with `typed_calls.rs:1158–1164`.

The fixture's registrar operands are pointer symbols/SSA and scalar constants, so the narrow comma selector matches this call's shape. The test does not assume lambda suffix numbers or accidentally follow the finisher guard. Exact LLVM printer output remains unexecuted.

Resolver v4 R3 is corrected at `tests/async/resolver_provenance.rs:52–67`: both selection and ordering use the formatted `PENDING_EXCEPTION_VALUE_TAG_SYMBOL`, whose actual spelling is defined at `hir_codegen.rs:78` and used by the producer at `promises.rs:570–574`. `tests.rs:1` imports the parent namespace, so the constant and pointer types are in scope. The prior exact descriptor delimiter, non-null text publication and i8-call corrections remain present.

### F3, v4 R1 and I3 overall: ADDRESSED at source level

The ambiguous collect-before-clone local is now explicitly `HashSet<PointerValue<'ctx>>` (`tests/async/promises.rs:1900–1905`). This removes the particular unconstrained receiver pattern reproduced by the separate std-only control. No package type-check result is claimed.

The preserved controls meaningfully exercise the requested repairs:

- Reactive preheader success/error controls (`1945–2060`) install nonempty outer/inner catch, loop and physical-owner state, require consecutive splits and original successor preservation, and force a real Result error after saved-context installation.
- Live-merge helper coverage (`2062–2106`) compares physical slot-to-flag bindings; real `compile_if` coverage (`2110–2160`) introduces a then-only Promise through Let, then Return, with a live empty else. Removing sibling restoration would leave the first-arm physical record in the final map.
- Real `compile_lambda` and `compile_async_lambda` controls (`2288–2325`) call the production methods with a missing Var after a complete nonempty scope seed, require the exact inner-body error and emitted inner function/ramp, and compare the complete relevant snapshot (`1812–1930`). Production isolation/restoration is at `values/closures.rs:1092,1197` and `347,354` respectively.
- The narrow host-adapter macro control (`2265–2284`) remains additional coverage; it is no longer substituted for the actual lambda error paths.
- Fresh/stale packet and direct call/block-local handoff controls described above replace the defective broad declaration/substr checks.

The catch/loop paired records, shared context helper (`statements.rs:63–113`), generic escaping-exception paths (`hir_codegen.rs:1315` onward), preheader promotion and per-arm owner restoration/merge remain the reviewed unchanged production implementation. No new source inconsistency was established in the fix diff or its load-bearing callers.

## Test plan and quality

`TEST-PLAN.md:17–19,25–40` includes all 16 added LLVM scoped test functions and the three required HIR filters, including the changed lowering control, new freshness control and existing ten-channel/finalizer snapshot control. No required scoped filter omission was found. Existing broader regression suites remain separate gates rather than being implicitly accepted by these filters.

The compilation-first commands, exact immutable overlay/prerequisite requirement and baseline separation are explicit (`3–11`). The dummy-pointer/malformed-state no-runtime warning is preserved at `42`. The resolver include occurs once (`tests/async.rs:4`); its uniquely prefixed helpers do not collide with sibling includes. The new local collection annotation matches `LoopScope.promise_boundary`; new test string/iterator usage and helper visibility are source-consistent. These assessments do not replace dependency-specific Rust/API validation.

No new Critical/Important/Minor source finding remains. Printed-IR selectors are deliberately narrow to these fixtures. Their eventual execution must still confirm actual lowering, printer spelling and host-side assertions.

## Verification and remaining blockers

This reviewer used source reads, hashes, file comparisons and git patch/whitespace checks only; no product compiler, parser, formatter, verifier, test or runtime probe was run. Calls to parse/compile/module.verify inside test source are authored controls, not results obtained here.

Independently verified here: all 18 candidate and 17 existing-base manifest entries; explicit absence of the new resolver file at base; full archived-base patch applicability (exit0); complete candidate/replay equality (exit0); and no whitespace diagnostics. The no-index whitespace comparison returns exit1 because base/candidate differ, not because diagnostics occurred. Complete v4/v5 comparison identifies only the two declared test files. These checks prove identity/applicability, not behavior.

Separate verification evidence reports reduced Rust inference controls only. Package checking stopped before compilation on missing Inkwell; LLVM 22 development files and rustfmt are unavailable, and authorized official acquisition timed out. No package test pass, package type-check result, formatter result or runtime proof exists. Existing baseline CI parser/compilation failures predate this candidate and remain separate. The marker audit was read and its unchanged local routes checked; this reviewer did not repeat its external blob retrieval.

Next permitted acceptance work is the compilation-first immutable test plan once its toolchain/dependencies and baseline prerequisites are available. This source acceptance does not authorize applying the archived-draft delta directly to the remote product tree, merging, or clearing broad ownership/JIT/creator gates.

## Frozen pins and read-source hashes

- Full delta: `1387f9a75e443e2030656ec7d4a3b2eb8f1d89573dc0e02c32f881b2b4b50e22`
- Full review package: `c6fac1a22a6b6cc5dc9207851710b83f0b3fb8e75f3410a551a07aa9d168dcfe`
- V4→V5 fix review: `da1762017027523199e0f08ed12c4982ac4c7cf1ac90f54d4bb75a5ba5c36d59`
- Candidate manifest: `35d25b45ea1459ae29cc1c8b6c00efa8c6d223f308e08a7a5841c6498bc76108`
- Base manifest: `f9ad742a33be453857eba2b7215f89caa303ecba668e7b0f56d7f8791cd03134`
- Base absence: `bbb950823a30bb92c9527880e10435cbb1f3bed5d1e1faed4cb6be5e27b29bfd`
- CHECKPOINT: `7c143b35edced0e44031e925e8f10bd39d0397e95fca9489d96ee683e4306410`
- TEST-PLAN: `9eb740f8f418de5e7f8c7cd88607d5b269c7495a34600d937bf8653f7e55ae81`
- Prior combined v4 review: `a83ad916e10c4ebdf11bf1da20ed144884fd2f1e53742291c21c4f8c444aeb20`
- Marker audit: `85301456bb8eaafcab07f1270713828ec9c731813a60c8c53dccb3f621ebddd0`

### Candidate changed owners

```text
7343c78c4deed380bd2a0394223ac3aebf227a497c64991e82273bebd46c0c35  crates/thaw-hir/src/lower/control_flow.rs
cbc078c2cd2534f17d13c3d06a5d799fef829667a91c902383e9c2d411aac5a4  crates/thaw-hir/src/lower/inference/types.rs
cec03b82c561d5e9e1f19a1363acb2b7a2178d927053efc9e14b64b3c842c3e0  crates/thaw-hir/src/lower/statements/lowering.rs
1db0ffbdc40f3a43899000d0e2608574d121452f3c421ef68bcf1145bc8851a5  crates/thaw-hir/src/lower/tests/control_flow.rs
d375dbd15a448a2383f4499fb9609c2f6aa9e2e2d3ced08c6aefc99f77385934  crates/thaw-llvm/src/hir_codegen.rs
b6beee0d2f139798452b87fc2bf4dee2303b6a42d721add21eead2fa2529626b  crates/thaw-llvm/src/hir_codegen/async_frames/codegen.rs
546d3af6385158c6cd9540a9d33e7be9551da51dc7d0833249baae2a7b3b0b82  crates/thaw-llvm/src/hir_codegen/console.rs
4585204d7f3954c6c5362b6b4f6eeb5c60607b90cfc9c0c42c6f9be94a78df31  crates/thaw-llvm/src/hir_codegen/dynamic_host/callbacks.rs
360d261f891299dde644ea2c3d2dc36701818b62d94e04bca9cd8fd1f43faa8d  crates/thaw-llvm/src/hir_codegen/dynamic_host/napi.rs
66efebeb91332b05795dd5b56dd3bac819ba127f60fc653064105d7c00fa464a  crates/thaw-llvm/src/hir_codegen/dynamic_host/quickjs.rs
c2b137151ebaade21d8dfef2b2b2804433be6e13474b7a10ea2f9baf99b52917  crates/thaw-llvm/src/hir_codegen/promises.rs
07f6e34199eb5c488076b0ff766c31473c5e0d39b26596d34dc550286542a994  crates/thaw-llvm/src/hir_codegen/runtime_declarations.rs
ae6d2bc3bf271d9a31a156a512315b937f3b1618d02608fd2a377797e69ef0a9  crates/thaw-llvm/src/hir_codegen/statements.rs
18a7048068070e770c3663c23b622f050eb96017404ac576d2c7387b0e3966a6  crates/thaw-llvm/src/hir_codegen/tests/async.rs
2a92750d1417350f01b7511b36cab233f88528f163f6e3a5175df29f1194f848  crates/thaw-llvm/src/hir_codegen/tests/async/promises.rs
80304f982295050402ffa5daf569264b0c9818bdda358e362870c560ac912389  crates/thaw-llvm/src/hir_codegen/tests/async/resolver_provenance.rs
3e546a37b0d48a5bdeed31b06fa1359c248bff74a6428cb2e3bf45265890067a  crates/thaw-llvm/src/hir_codegen/values/closures.rs
4e646b220782c65a752d28fc80fdb0f2d00633a7ad2b904835edd7282a1cdba2  crates/thaw-llvm/src/hir_codegen/values/expressions.rs
```

### Supplementary affected source, unchanged

```text
013fe13e55f67b4c0e72d99d368e2fd12870535065c8e596bf7f50ec7b58b659  crates/thaw-hir/src/lower.rs
75254b8d7545b4c44619bdda601c4d16173a2b788411c39fdfe702bcd8559882  crates/thaw-llvm/src/hir_codegen/dynamic_host/typed_calls.rs
750fdd196261a33246813e262318d5086962b70477478d61a551b481ec97bf29  crates/thaw-llvm/src/hir_codegen/async_frames/planning/plan.rs
d8a5b4f23a4627b88361a66a4f9a63b68dbc7e58f90faaf091d1200a355239d1  crates/thaw-llvm/src/hir_codegen/functions.rs
aec037e3813f022fc2c57d86932ea3ee19c1bf731397e483b09514c35e6922b6  crates/thaw-llvm/src/hir_codegen/invocations/calls.rs
f447b001c842f1711f6c949b5496e7d721e11ba57ba880e9b46d45a8cbe3b7ed  crates/thaw-llvm/src/hir_codegen/tests.rs
e322721353ad122e435466952416a6af213bf9861081a96d942643efadfc5330  crates/thaw-runtime/src/runtime/promises.rs
```
