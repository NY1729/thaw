# Promise owned-return successor source review — unit496

Live candidate `/tmp/thaw-throw-provenance-head494/candidate`, managed baseline f4e2faf98b6973306ac634f2d78968904b846480. Source only; no product edits/execution. Read pins promises.rs4733f1908e82971ba6a858ba6062855d39fea273064c1e37780f8775f384fc74; statements.rs34a0584e39b52b3218747ca2f128bc871a9c3ccd47efa115c1e6bbf2e6faa08e; closures.rsee403d2acb420a9d7151963e5103ffb691ed0ac5a75e837977e2908238c5141c. Author is actively editing; no freeze verdict.

## Confirmed source corrections

split_promise_conditional distributes Conditional through TypedClosure, UnionInject/UnionValue and Optional/Nullable/NullishValue by reconstructing each wrapper around the selected arm. It clones syntax, not runtime evaluation: consumers compile one test and one selected arm, recursively. Direct wrapped conditional consumption is no longer merged before ownership normalization.

compile_await_promise_union first distributes wrapped conditional; explicit UnionInject Promise arm calls compile_promise_for_consumption on the leaf and drives it, while explicit plain arm compiles its value. Opaque union path still evaluates once and branches on active tag before unpacking a Promise pointer. Explicit path corrections should not be confused with opaque-call ABI closure.

store_arena_promise_slot_with_token releases an owned temporary on both checked failure and ready paths. On failure the internal acquisition did not publish; release the temporary before throwing. On success internal slot token exists before external temporary release. Borrowed promotion stores pass false. compile_and_store_arena_promise_expression now splits mixed owned/borrowed conditional before evaluating/storing, so only selected leaf chooses token adoption. Let/Assign/frame paths found calling it; this corrects the prior whole-mixed-expression bool store gap for those paths. Assignment returns its stored pointer borrowed after adoption; existing classifier does not incorrectly mark Assign owned.

HirStmt::Return now normalizes exact Promise expressions through compile_promise_for_consumption. Lambda non-block expression returns also do this when declared ret is Promise. Block lambdas use shared Return lowering. Thus named/closure exact Promise borrowed returns receive an external token, while constructor/call returns transfer it. Adapters forwarding these corrected targets must transfer their returned token without another retain. The prior Builder clone mismatch is separately reported fixed by moving builders, not by this return review.

## Remaining concrete union-return ABI gap

Return and lambda normalization currently recognizes exact Promise only. A Promise-bearing union return still uses compile_expr. The callee can therefore return an injected owned constructor token or a borrowed alias pointer under the same union ABI. The caller's opaque union await decodes its active Promise arm and uses the whole expression ownership classifier; union-return Call is not exact Promise and classifies borrowed. It acquires an additional token even for the owned constructor-return arm, leaving the original token unreleased. Changing classifier to owned without normalizing borrowed callee arms would instead consume the caller's borrowed alias owner.

Minimal complete owner contract: native return of any supported type containing a direct Promise union member supplies one external token iff that member is active. At Return and lambda expression return, recurse through Conditional/representation wrappers without reevaluation, normalize injected Promise leaves using existing consumption helper, preserve injected plain leaves unchanged, and rebuild the original union representation. For opaque borrowed union variable/projection, compile once, extract tag/payload, branch on the actual Promise member tag, checked-retain that pointer only on the active branch, rebuild payload unchanged, merge with untouched plain union. For an already owned union call-return under the normalized ABI, transfer its active token. Multiple Promise members require tag-specific normalization; do not guess one member or decode inactive payload.

Then annotate/classify native named/closure call-return ownership from actual return ABI, including FunctionCallWithThis, rather than asserting owned solely because syntax is Call. Keep external/host call contracts separate. Caller opaque await consumes active owned token without a second retain. Caller storage into arena-owned aggregate/union must similarly acquire internal token then release the transferred external token. Borrowed parameters remain borrowed and require no entry retain absent an escape; return is an escape.

Use one shared compiler return-normalization helper for shared Return and lambda expression returns. No new runtime registry is necessary. Active-member guarded normalization and a local ownership fact for selected paths suffice; existing union pack/unpack and checked-retain ABI are reusable. Maintain rollback/failure cleanup for owned nested values if later wrapper packing/guard work fails; do not claim every aggregate path complete from union normalization.

## Unrun controls and verdict

Named and closure returns of borrowed exact Promise (source alias survives await); constructor exact return (one final release); conditional union borrowed Promise/plain, owned Promise/plain, mixed borrowed/owned Promise union; wrapped condition with side effect counted once; function reference/this adapter transfer; inactive union payload never decoded; retain failure and slot failure release owned temporary once. Root full alias/aggregate/global gates remain open. All controls unrun.

Marker: EXPLICIT_PATHS_SOURCE_CORRECTED_OPAQUE_UNION_RETURN_OWNERSHIP_HOLD.
