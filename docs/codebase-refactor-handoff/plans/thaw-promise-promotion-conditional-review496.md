# Promise promotion and conditional consumption review — unit496

Live source-only throw candidate `/tmp/thaw-throw-provenance-head494/candidate`; baseline managed f4e2faf98b6973306ac634f2d78968904b846480. No product edits/execution. Read hashes: hir_codegen.rs c1bfea3dec35493b94f06de04ba9dc9c82311360e0dd755fd3fc8ccd809559a7; values/closures.rs e81954af055476a706988b8cf5c4d9304316ec873ccce37a1ae4b2ad9c8adc70; promises.rs15040db115a42e5ff5638e846525906094933cc931d17d56bc212398dbe38c31; statements.rs8cc5ad14e6f29f3af6bbb426d30e30dd5f549e59e0609061637f663af361762d. Live edits may supersede these snapshots.

## Concrete promotion API blocker

values/closures.rs128 uses self.builder.clone(). Installed Inkwell0.9/0.10 builder.rs declares Builder with Debug only and exposes no Clone implementation. This is a source API mismatch, not a compiler-run finding. Move the original Builder with std::mem::replace(&mut self.builder,promotion_builder), preserving its exact insertion point, then restore it after split_result. No builder-position reconstruction is required. Sent author immediately.

The intended split otherwise follows the scoped plan: initializes null and loads at preheader; detaches original terminator; temporarily installs saved preheader catch_stack; emits checked internal ownership store; moves original terminator into ready; updates active matching scope preheaders; restores discovery builder/catch stack before propagating Rust error. Only compile_while produces these scopes, and its memory-variable header contains no PHIs. Consecutive promotions therefore extend success edges without changing the header target. Non-Promise store is restored to the original promotion builder. Rust emission failure is an abort path; partial failed IR is not evidence of valid executable recovery, and no such claim should be made.

## Mixed conditional await — direct fix present

promises.rs69 compile_promise_for_consumption now recursively compiles a direct Conditional into separate branches. Each selected leaf is normalized before merge: borrowed leaf gets checked retain; owned leaf transfers its existing token. The pointer PHI uses actual post-retain ready blocks. This avoids unconditional merged retain for direct conditional Promise operands and evaluates only the selected arm/test once. Existing both-owned classifier alone would not suffice.

## Remaining union/wrapper gap and minimal extension

compile_await_promise_union still evaluates the merged union once, decodes its Promise arm and calls retain_borrowed_promise_for_consumption(inner,pointer) at1532. For a conditional combining an owned injected Promise and a borrowed injected Promise, the whole-expression ownership classifier is false; the selected owned arm is retained again and its original external token is not transferred to the consumer. UnionInject is not in the classifier. Wrappers hiding a Conditional also bypass direct recursive conditional normalization.

Extend normalization at the consumer's expression-tree boundary, not after losing branch provenance. A small compile_await_operand/compile_union_for_consumption helper should recursively distribute over Conditional and transparent representation wrappers; compile the test once, recurse only within selected branch, and merge the fully normalized result. For UnionInject, inspect its injected member type: evaluate the leaf once; on a Promise member normalize the leaf pointer according to its leaf ownership, then repack the original union tag/payload; on plain member preserve ordinary compilation. This gives every Promise-bearing selected path exactly one external consumable token before merge, while plain paths retain no Promise. The existing union await tag branch must then consume that normalized token without a second retain.

Do not blindly strip wrappers: OptionalValue/NullableValue/NullishValue/UnionValue have representation operations; reconstruct their tag/payload or extraction using existing lowering helpers after recursively normalizing the underlying Promise path. TypedClosure is transparent only to the extent its existing compile_expr contract confirms. Avoid compiling inner once for metadata and again for value.

For an opaque union variable/projection, the extracted Promise pointer is borrowed and must be retained inside the active Promise-tag branch only. For a function call returning a union containing Promise, determine the actual return/callee ownership ABI before deciding transfer; do not classify every union call borrowed or owned by its spelling. If source metadata is needed after an opaque merge, carry a local i1 ownership flag parallel to that value PHI, produced by the same branch evaluation; consume it under the Promise-tag branch. This is one local compiler ownership fact, not a runtime value registry. Callee return normalization is a prerequisite when its selected return arm can be owned or borrowed. Full global/aggregate alias contracts remain open.

Also store_arena_promise_expression currently releases adopted external tokens based on the whole expression's bool ownership classifier. Mixed conditionals stored into arena slots have the analogous owned-arm token leak; reuse normalized per-arm ownership or the selected ownership flag at that storage boundary. Do not fix await while claiming all consumers are closed.

## Unrun controls and bounded verdict

Controls: direct borrowed/owned conditional both selections; nested conditional under wrapper; typed union owned Promise/plain value; borrowed Promise/plain value; owned/borrowed Promise union selected arms; side-effecting condition and constructor evaluated once; alias remains valid after await; internal store releases only owned temporary; enclosing/nested catch promotion failures; consecutive and condition-side reactive captures. Check reference counts and active-owner removal after consumer final release. All controls unrun.

Marker: SOURCE_HOLD_BUILDER_API_AND_UNION_OWNERSHIP; no full native ownership acceptance.
