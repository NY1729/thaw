# Deferring a genuinely ambiguous Fallback overload call

## Problem

A bare call to an overloaded registry Fallback function (`class_methods.rs`'s
`self.functions` closure in `rewrite_external_class_methods_with_static`)
picks among same-arity candidates by scoring each argument's statically
known type against the candidate's declared parameter types
(`overload_type_score`). When an argument's type can't be determined at
all (an untyped/`Json`/`as any`-cast value), that argument contributes
nothing to the score — it's skipped, not disqualifying. If *every*
candidate ties at score 0, the code still picks one (the first
declared, via `.reduce`'s tie-preserving-`best` behavior): a pure
iteration-order guess.

For `ms` (`(value: number, options?): string` vs `(value: StringValue):
number`, both arity 1) this guess is actively wrong: the two overloads
declare genuinely different, semantically disjoint parameter types
(`F64` vs `Str`). A truly dynamic argument (`ms(value as any)`) gets
silently coerced through whichever one iteration order happened to
land on, producing a wrong runtime value (`ms(0 as any)`-shaped calls
print `"0ms"`) instead of falling through to the `typeof`-checking
runtime dispatcher that already exists for exactly this situation
(`union_overload_dispatch_declaration`).

## First attempt (reverted) and why it failed

"Defer (don't rewrite) whenever more than one candidate ties at score
0" is the obvious fix, and was tried first. It broke three real,
previously-fixed, currently-pinned packages the moment the *full*
gated npm-integration suite ran (`thaw-cli`'s own suite, and every
synthetic unit test, stayed green throughout — this is exactly the
gap the full suite exists to catch):

- **zod**: `z.string()` — `string(params?): ZodString` and a second,
  generic `string<T extends string>(params?): $ZodType<T,T>`, both
  arity 0-1, tied at 0 with no arguments. Deferring exposed a separate,
  pre-existing bug: `zod_string`'s own *default* binding (used when no
  rewrite applies) resolves to the generic overload, which needs 1
  argument — previously always masked because the argument-shape
  rewrite unconditionally overrode it.
- **pino**: same shape; its `.d.ts` bundles the *exact same* two
  overloads twice (a literal duplicate).
- **lodash**: `filter([1,2,3], value => value > 1)` — four same-
  arity-range generic overloads (list vs object collection, plain vs
  type-guard predicate). The callback needs `annotate_generic_
  callback_arguments` to run against *some* matching generic candidate
  to get `value`'s parameter type inferred at all; deferring means
  that never happens, a different failure ("needs an explicit type
  annotation") than a wrong overload being picked.

Two narrower heuristics ("prefer the sole non-generic candidate", then
"prefer any candidate when every tied one reduces to the same
`(min_arity, max_arity, param_types)` shape") each fixed what came
before but not the next case, and were abandoned in turn rather than
guessed at a third time.

## Root cause of why "always pick something" was safe until now

Every one of the surviving (non-`ms`) cases shares a property the
failed heuristics didn't check for directly: **the tied candidates'
declared parameter types render to `Json`/`JsValue` (an opaque,
never-coerced passthrough) at every position where they differ.**
Confirmed by direct inspection (`class_methods.rs`, instrumented
temporarily): zod's two `string` overloads both declare `[Json]`;
lodash's four `filter` overloads all declare `[Json, Json]`. Because
`Json`/`JsValue` never actually get coerced into anything more specific
downstream, it is *always* safe to route through any tied candidate
that only differs by an opaque type there — the real npm function gets
called with the same untouched value regardless of which symbol
carried it there.

`ms`'s two overloads are different in kind, not just contingently:
they declare genuinely distinct *native scalar* types (`F64` vs `Str`)
at the position that differs. Picking the wrong one there means
actively coercing the value (e.g. a string forced through an `F64`
slot), which can silently produce a wrong value rather than just an
imprecise static type.

## Plan

Keep the existing behavior unchanged for every case with at least one
positively-scored candidate (a real type match won it). Only change
what happens when every same-arity candidate ties at score 0 — instead
of unconditionally picking the first, or unconditionally deferring,
decide per this rule:

1. If any tied candidate declares **only** opaque (`Json`/`JsValue`)
   parameter types, pick it. An opaque type is never coerced into
   anything more specific regardless of the argument's real shape, so
   it's a universally safe representative for the *whole* tied set no
   matter how many other, differently-typed candidates also tie here —
   this is what actually covers zod (`string(params?)` alongside its
   generic `string<T>(params?)`, whose params both render to `[Json]`
   anyway, so this rule fires trivially), pino (a literal duplicate
   declaration), and lodash (all four `filter` overloads render to
   `[Json, Json]`).
2. Otherwise, if every tied candidate declares the exact same parameter
   types as every other, pick the first (matching today's tie rule
   exactly) — picking among several truly identical declarations
   changes nothing.
3. Otherwise (a tied pair's declared types genuinely disagree on a
   *native scalar* parameter with no opaque escape — `ms`'s own case),
   defer: don't rewrite this call, leaving it to fall through to
   whatever the name's default binding already is (the `typeof`-
   checking dispatcher for a union-dispatched name like `ms`, or the
   ordinary first-overload-wins default otherwise).

This only touches the "all zero" branch; every already-tested path
(a real score above 0 winning, a single arity-only match, the normal
first-wins default for a non-overloaded name) is untouched.

## Verification plan

- Reproduce all three previously-broken packages (zod `z.string()`
  with zero args, pino `pino()` with zero args, lodash `filter` with a
  callback) directly via `thaw build` against the real fetched
  package, confirming each still produces correct output.
- Reproduce `ms`'s own four scenarios (number literal, string literal,
  options object, typed variable) still narrow correctly, and the
  `as any` scenario now defers (build-time error, not a silently wrong
  value) rather than either silently misrouting or breaking a real
  package.
- Add network-free unit tests to `overload_inference.rs` covering: (a)
  two scalar-typed tied candidates (`ms`-shaped) still defer, (b) two
  Json/JsValue-typed tied candidates (zod/lodash-shaped) still get
  rewritten to the first one, (c) a mix (one scalar, one opaque)
  still conflicts and defers, since an opaque candidate provides no
  proof the scalar one is wrong to skip.
- Full `thaw-cli` suite (unit + registry-fallback tests).
- Full gated `THAW_RUN_NPM_INTEGRATION=1` suite, `--test-threads=1`,
  release mode — this is the one that actually caught the regression
  last time, so it is the real acceptance gate here, not a nice-to-have.
- `cargo fmt --check` / `cargo clippy -D warnings`.
- Commit only after all of the above are green.
