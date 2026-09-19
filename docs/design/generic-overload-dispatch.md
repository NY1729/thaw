# Generic-overload Fallback dispatch and tuple returns

**Status: done.** Motivating case: immer's own
`produceWithPatches(base, recipe): [nextState, patches, inversePatches]`,
which used to build but return an opaque `JsValue` whose `[0]` was
`undefined` (and, once a tuple was emitted at all, failed outright with
"unsupported JSON tuple element JsValue").

## Problem

`produceWithPatches` is declared as a callable-interface const with **all**
its overloads generic:

```ts
interface IProduceWithPatches {
  <Recipe extends AnyFunc>(recipe: Recipe): InferCurriedFromRecipe<Recipe, true>;
  <State, Args extends any[]>(recipe: (state: Draft<State>, ...args: Args) => ValidRecipeReturnType<State>, initialState: State): (state?: State, ...args: Args) => PatchesTuple<State>;
  <State>(recipe: (state: Draft<State>) => ValidRecipeReturnType<State>): (state: State) => PatchesTuple<State>;
  <State, Args extends any[]>(recipe: (state: Draft<State>, ...args: Args) => ValidRecipeReturnType<State>): (state: State, ...args: Args) => PatchesTuple<State>;
  <State, Recipe extends Function>(recipe: Recipe, initialState: State): InferCurriedFromInitialStateAndRecipe<State, Recipe, true>;
  <Base, D = Draft<Base>>(base: Base, recipe: (draft: D) => ValidRecipeReturnType<D>, listener?: PatchListener): PatchesTuple<Base>;
}
```

Two things had to line up for a real call to work:

1. **A generic return that mentions a type parameter is emitted as
   `JsValue`.** `mentions_any_type_param("PatchesTuple<Base>")` is true, so
   `typed_dynamic_declaration`'s generic branch collapses the return to
   `"JsValue"` even though the alias is a tuple.
2. **The tuple alias cannot be classified normally at all.** `Patch`'s own
   `path: (string | number)[]` has no native collection layout, so
   `resolve_generic_alias` fails on the whole alias
   (`array element type Union([Str, F64]) does not have a native
   collection layout`). Even with the type parameter substituted, the
   ordinary classifier has nothing decodable to give back.
   (A simplified `Patch { op: string; path: string[] }` *does* resolve --
   that was the source of the original, too-optimistic investigation
   note.)

Separately, the argument-shape dispatch in
`class_methods.rs` picks among the overloads whose parameter types all
render opaquely; for a 2-argument
`produceWithPatches(state, recipe)` it used to take the first such
candidate -- the curried `<State>(recipe, initialState)` overload, whose
return is a function/`JsValue`, not the base-first tuple overload.

## Scope

The bounded feature is: **render a generic Fallback function's declared
return type when that type resolves to a native *tuple* skeleton, by
substituting each own type parameter with the placeholder native type
`Json`, and flattening any other undecodable leaf (including nested array
and object elements) to `Json` as well.**

Deliberately tuple-only at the top level. A generic `T[]`/`{ field: T }`
return (lodash's `map`/`filter`/`reduce`, and countless others) must keep
its ordinary `JsValue` fallback: rendering it as `Array(Json)`/`Object(..)`
instead silently changes `const out: number[] = map(...)` from a coercible
opaque handle into a hard `Array(Json)`/`Array(F64)` mismatch. A tuple
differs in kind -- its fixed *arity* is what destructuring and `result[0]`
actually need, and no opaque handle can express it.

Concretely for immer: `PatchesTuple<Base>` →
`[Json, Json[], Json[]]` (a real tuple in the emitted declaration). `Base`
(the state type) becomes `Json`; the state itself is a plain object the
caller reads through ordinary `Json` access, and `patches`/`inversePatches`
become `Json[]` (their real `Patch` shape is unrepresentable anyway), while
the tuple *shape* is what makes `result[0]`/`result[1]`/`result[2]` decode.

This is narrower than full generic-overload resolution: the type parameter
is not inferred per call site for the *return* type, and only the tuple
skeleton is preserved. That is enough for immer's own practical call, and
for any package whose tuple return is a generic alias.

## Implementation

At the bridge layer, because the placeholder substitution has to happen
while the `.d.ts` AST (and its alias bodies) are still in scope:

1. `thaw_bridge::DtsGenericFunction` gains
   `tuple_return_type: Option<String>`. A helper
   (`generic_tuple_return_type`, `bridge/dts/types/generics.rs`)
   projects the declared return type with a substitution map
   `type_param -> HirType::Json`, keeping tuple structure (following
   generic aliases, `readonly` and parenthesized wrappers). Unsupported
   leaves become `Json`; opaque `JsValue` handles reject the projection
   so object identity is not lost. It renders and keeps only a top-level
   tuple, and skips projection when the return does not mention one of the
   function's type parameters. The four sites that build a `DtsGenericFunction`
   (`lower_dts_function`/`lower_dts_method_signature` in
   `interfaces.rs`, `lower_dts_call_signature`/`lower_dts_fn_type` in
   `exports.rs`) all populate it.
2. `crates/thaw-cli/src/registry_integration/dynamic_declarations.rs`'s
   `typed_dynamic_declaration`, generic branch: when the return type
   mentions a type parameter, use `generic.tuple_return_type` before falling back to
   `"JsValue"`. The ordinary `ret` is untouched, so every existing path
   keeps its old behavior.
3. `class_methods.rs`'s all-zero-tied overload dispatch: among the
   candidates whose provided parameters are all opaque
   (`Json`/`JsValue`) -- which all dispatch to the same runtime JS
   function, so the choice changes nothing at runtime -- prefer one whose
   `generic.tuple_return_type` is present, i.e. whose return decoded
   to a concrete aggregate. This is what steers immer's
   `(base, recipe)` call to the base-first overload instead of the earlier
   curried one.
4. No change to tuple decoding: the emitted declaration is a plain
   `declare function ...(...): [Json, Json[], Json[]]`, which thaw-hir
   already lowers as a `Tuple`.

The `Json` placeholder (rather than the `JsValue` the ordinary
classification degrades an unconstrained parameter to) is load-bearing: a
`JsValue` *tuple/array element* has no JSON marshaling at all.

## Verification

- Real immer @11.1.18, built through `thaw build --use immer`:
  `produceWithPatches({ count: 0, label: "start" }, (draft: any) => {
  draft.count = 1; })` returns `[next, patches, inverse]` with
  `next.count === 1`, `patches.length === 1`, `patches[0].op ===
  "replace"`, `patches[0].path[0] === "count"`, `inverse.length === 1`,
  matching real Node. (The recipe callback still needs an explicit
  parameter annotation; contextual typing of a generic callback parameter
  is a separate, pre-existing limitation.)
- `thaw-cli`'s
  `a_generic_alias_tuple_return_is_rendered_with_json_placeholder_params`
  (synthetic `pwp-kit`, real-immer-shaped `Patch`, plus a competing curried
  overload to exercise the dispatch tie-break).
- `thaw-bridge`'s `records_placeholder_native_return_for_a_generic_alias_aggregate`
  (all three top-level extraction shapes), `projects_undecodable_alias_leaves_to_json`,
  and `omits_placeholder_native_return_for_a_non_aggregate`.
- A gated `registry_add_runs_real_immer_produce_with_patches_when_enabled`
  (real `immer@11.1.18` fetched by `thaw_registry::add`).
- Full `cargo test --workspace`, `cargo fmt --check`,
  `cargo clippy --workspace --all-targets -D warnings`.
- Full gated `THAW_RUN_NPM_INTEGRATION=1` suite, `--test-threads=1`, release:
  501 passed, 2 failed -- both failing identically on unmodified `main` in
  this environment (`registry_add_builds_and_calls_real_lodash_when_enabled`
  and `registry_add_fetches_and_runs_utf8_validate_when_enabled`), so no
  regression here. The two `..._real_drizzle_orm_...` tests were skipped:
  they run for 10+ minutes and equally fail to finish on `main`.

A top-level array/object projection was tried first and reverted: it turned
generic `T[]` returns (lodash's `map`) into `Array(Json)`, breaking
`registry_add_builds_a_multi_package_project_when_enabled` with
`value has type Array(Json), expected Array(F64)`. Tuple-only is the
narrowest change that still fixes immer.
