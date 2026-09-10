# `Promise.all` / `race` / `allSettled` / `any` over dynamic thenables

## Problem

`Promise.all([limit(fn), limit(fn)])` -- where each element is a
Fallback async function's result -- fails to lower:

```
Promise.all element 0 must be a Promise, got Json
```

`Promise.<combinator>`'s lowering (`crates/thaw-hir/src/lower/
invocations/promises.rs`) only ever produces the native `PromiseAll` /
`PromiseAllSettled` / `PromiseRace` / `PromiseAny` HIR nodes, which
combine `HirType::Promise(_)` values through thaw's own promise
runtime. A dynamic thenable is a `Json` / `JsValue` -- a live JS
`Promise` handle on the QuickJS side, not a `ThawPromise*` -- so it has
nowhere to go.

Real driver: **p-limit**. `LimitFunction`'s call signature is
`<Arguments, ReturnType>(fn, ...args): Promise<ReturnType>`; the
unresolved generic `ReturnType` collapses the return to bare `Json`.
Everything else about p-limit already works: `pLimit(2)`, a deferred
`limit(fn)`, `await limit(fn)`, `limit.activeCount`.

## Approach

When **every** element of a `Promise.<combinator>` call is dynamic
(`Json` / `JsValue`), don't build a native combinator node. Call
QuickJS's own `Promise.<combinator>` through the existing dynamic-host
intrinsics and return its `JsValue` result; the surrounding `await`
then resolves it the same way it resolves any other `JsValue`
(`resolveDynamicValue`, `Expr::Await`'s existing `JsValue` arm).

```
callDynamicMethodHandle(
    getDynamicValue("Promise"),      // the Promise constructor, a JsValue
    "<combinator>",                  // "all" | "race" | "allSettled" | "any"
    [ [ p0, p1, ... ] ],             // one arg: the array of thenables
)
```

- Each `pi` is passed through `coerce_to_declared(Json, pi)` -- for a
  `JsValue` that emits the `{"__thaw_js_handle_id__": N}` placeholder
  the QuickJS side turns back into the real handle (the same laundering
  `lower_dynamic_value_method_call` already does for a method
  argument); for a plain `Json` it's a no-op.
- The inner `[p0, p1, ...]` and the outer `[ inner ]` are each
  `coerce_to_declared(Json, ArrayLit(...))`, exactly as
  `lower_dynamic_value_method_call` builds its own argument array.
- `callDynamicMethodHandle` infers `JsValue`. `await Promise.all(...)`
  -> `Expr::Await` (`lowering.rs`) sees a `JsValue` -> emits
  `resolveDynamicValue(...)`, which drains the JS job queue and returns
  the settled value. `results[i]` is then an ordinary dynamic index.

This is *reuse*, not new infrastructure: `getDynamicValue`,
`callDynamicMethodHandle`, and the `Json`-array argument laundering all
already exist and are already exercised by zod/hono.

## Scope

- `Promise.all`, `Promise.allSettled`, `Promise.race`, `Promise.any`.
- Only the **array-literal** call form (`Promise.all([a, b, c])`) --
  the same form the current per-element loop handles. The
  non-literal-array form (`Promise.all(someArray)`) and the spread form
  stay on their existing paths; a `Json[]` / `JsValue[]` variable there
  is a separate, rarer case (add later if a package needs it).
- Trigger only when **all** elements are `Json` / `JsValue`. A mix of
  native `Promise` and dynamic elements keeps erroring for now
  (also rare; the native path can't hold a dynamic element and vice
  versa).
- Empty `Promise.all([])` keeps its existing native behaviour.

## Plan

1. **`promises.rs`** -- in `lower_promise_static_call`, for each of the
   four combinators' array-literal branch: after lowering the elements,
   if the list is non-empty and every element infers to
   `HirType::Json | HirType::JsValue`, build the
   `callDynamicMethodHandle` call described above and return it. A
   shared helper `lower_dynamic_promise_combinator(&mut self, method:
   &str, elements: Vec<HirExpr>) -> Result<HirExpr, String>` keeps the
   four call sites to one line each.
2. **No inference change** -- `callDynamicMethodHandle` already infers
   `JsValue`, and `Expr::Await` already handles `JsValue`.
3. **Test** -- `crates/thaw-cli/src/tests/native_addons/
   javascript_packages.rs`: a `THAW_RUN_NPM_INTEGRATION` pinned p-limit
   test that schedules three `limit(async () => n)` calls, `await
   Promise.all([...])`s them, and prints the results plus
   `limit.activeCount` / `pendingCount`. Also a network-free
   `crates/thaw-cli` or `crates/thaw-hir` test if the shape can be
   reproduced with `loadScript` + a Fallback-shaped async function.
4. **Revert-and-retest** the `promises.rs` change against the pinned
   test. Full `thaw-hir` / `thaw-cli` suites; `cargo fmt` / `clippy`.
5. Update memory (`project_npm_interop_gaps_5.md`) and mark this doc
   done.

## Risks / open questions

- Does `resolveDynamicValue` actually settle a still-pending p-limit
  promise (drain its queued `fn` call), or only unwrap an
  already-settled one? `await limit(fn)` already works, which routes a
  bound `Json` thenable through `Expr::Await` -- so the machinery to
  wait on one dynamic thenable exists; this extends it to a QuickJS
  `Promise.all` over several. If `Promise.all`'s own returned promise
  needs an extra queue turn, that surfaces as a hang or a `{}` result
  in step 3 and is dealt with then.
- `Promise.allSettled` returns `{status, value|reason}` objects --
  `results[i].status` is a dynamic property read on a `Json`, already
  supported. No special handling expected.
