# Throw / native Promise ownership scratch checkpoint (unreviewed)

- Paired scratch: `/tmp/thaw-throw-provenance-head494/{base,candidate}`
- Base source is managed baseline HEAD `f4e2faf98b6973306ac634f2d78968904b846480`; do not treat this draft as integrated or reviewed.
- Candidate contains 25 changed source owners. Full SHA256 inventory is below. Product execution was not performed; tests/build/compiler/parser/formatter/runtime remain unrun by instruction.
- `git diff --no-index --check ../base .` returned no diagnostics. This is only a whitespace check, not source/type validation.

## Work present

Native thrown-value descriptor/tag transport and ownership work; runtime Promise external/internal owner accounting and arena-slot replacement; compiler Promise consume/store/discard helpers; direct and union return normalization, conditional branch splitting, basic stack owner flags; lambda exit cleanup; initial lexical block, catch, loop cleanup boundaries; supported aggregate ownership work already in the scratch. Regression controls in the candidate are source-only and unrun.

## Explicitly incomplete / handoff gates

1. The new `catch_promise_boundaries` and `loop_promise_boundaries` vectors are not yet saved/restored with every nested compiler context. `async_frames/codegen.rs` still clears `catch_stack` at three entry sites without clearing/isolating the boundary vector. Closure/function state save/restore sites and preheader promotion context also need paired isolation. Treat exception/loop cleanup as incomplete.
2. `compile_if` does not yet snapshot/restore `stack_promise_slots` per arm. A promotion removing a physical slot in one arm can affect codegen bookkeeping in the other arm/merge.
3. `compile_block` currently treats each nested statement list as an owner lifetime boundary. Confirm this against all HIR lowering scopes before accepting; do not infer full lexical lifetime coverage from this draft.
4. Promise-bearing union discard is still incomplete; current discard path handles direct Promise types and direct/conditional Promise forms only. Add active-member ownership cleanup without reinterpreting plain members.
5. Full stack cleanup, uncaught/caught throws, async rejection, capture promotion and returns still need whole-callsite source audit. Aggregate/global routes remain open; the supported-route addendum excludes direct `Dictionary<Promise>` as unsupported, but arrays, tuples, native object fields and globals remain in scope.
6. No stable/full-fix claim. Parent requested checkpoint and handoff; further implementation should continue from this candidate after review.

## Changed-owner SHA256

```
8ae6eaecfb9a7027efad2d6e1cd6264be9aa4d3372753d24c52f12537a339653 crates/thaw-hir/src/hir/types.rs
a6b645dfe302cb5be37a685b1d2697a9bb5f75def3cd83459523e0282d59f7f9 crates/thaw-hir/src/lower/promises.rs
5d7fb80839500db433dd857823a0ba09f68590fd20f8be882114f73c06faaf78 crates/thaw-hir/src/lower/control_flow.rs
03c839fb1c46519ea4811bf5876d573b3e2f153478b3b93f8190ec36e9abb528 crates/thaw-hir/src/lower/expressions/lowering.rs
949da50ab6d115d667fc42a55c30b7aac550cd5c94664a855a4c2a568b1d83c8 crates/thaw-hir/src/lower/tests/control_flow.rs
bd193e0a863d521e8975a1e11f7a8c2d43ae76fee0981383f6d2514627954bc4 crates/thaw-hir/src/lower/inference/types.rs
099842ace8b36e43f946e341a36f935f507e412c950c83f97ceee18e62184e69 crates/thaw-hir/src/lower/invocations/calls.rs
b8f3c78598e0195802218b84b6e0313a2f99b96ff8f9e9213930cf9c42e1c423 crates/thaw-hir/src/lower/statements/lowering.rs
42ccbbcf946b986b135d383d3c26254013af51c29b814d620b0423f9297583f0 crates/thaw-llvm/src/hir_codegen.rs
f30c349f31c38cb27ee95730ede80206f715f2d4e86dbac2222772f0c3f4905a crates/thaw-llvm/src/hir_codegen/runtime_declarations.rs
58d74d14d915428958e61587f8f721592a432c0e225d84dd97d222c7d05bb09f crates/thaw-llvm/src/hir_codegen/promises.rs
c4ca917db33794ca2104baa3e4466cc19a56c48a3b9d5c0749dc4bd57568fe3d crates/thaw-llvm/src/hir_codegen/statements.rs
d8a5b4f23a4627b88361a66a4f9a63b68dbc7e58f90faaf091d1200a355239d1 crates/thaw-llvm/src/hir_codegen/functions.rs
8bfefa80a5c47f3a7f1c3178afd36e1faeea351d2337ff4adba8c6bea9c8fe5c crates/thaw-llvm/src/hir_codegen/json_values.rs
a9d7763603e6a213a7f52c6808e797333409ee331503d3e146cb51536f339ccf crates/thaw-llvm/src/hir_codegen/async_frames/codegen.rs
db7c97898eebf2323c6a87441894edc902dca2baa3de10a0584eea4165b68a56 crates/thaw-llvm/src/hir_codegen/values/closures.rs
4168f60dbdbc42f80e5e0fe5273e107c00551c0fbe31d3f5bf2140543b769a93 crates/thaw-llvm/src/hir_codegen/values/expressions.rs
c88f9cada47bbfe0c4d0da6b75e98bf1c6da5e82d6115b9d8fea02f60bebaf25 crates/thaw-llvm/src/hir_codegen/values/unions.rs
aec037e3813f022fc2c57d86932ea3ee19c1bf731397e483b09514c35e6922b6 crates/thaw-llvm/src/hir_codegen/invocations/calls.rs
c420c55d2f9f045ce57ad2e2dc436bff618a55311a000aa361335cc854fae9a1 crates/thaw-llvm/src/hir_codegen/tests/async/expressions.rs
750fdd196261a33246813e262318d5086962b70477478d61a551b481ec97bf29 crates/thaw-llvm/src/hir_codegen/async_frames/planning/plan.rs
2077a6c4782983e484fd4f45661a211ee527ce8fd5e3d7e36c237c4fdbe69915 crates/thaw-runtime/src/tests.rs
a1d329dda73189b4d0f7fe2efe7dd01c51c70388d50a3d49085d2ecf1acb81aa crates/thaw-runtime/src/lib.rs
e322721353ad122e435466952416a6af213bf9861081a96d942643efadfc5330 crates/thaw-runtime/src/runtime/promises.rs
```
