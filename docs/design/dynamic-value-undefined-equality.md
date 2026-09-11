# `liveValue === undefined` always answering `false`

**Status: done.** Part 1 (below) fixed `JsValue`. Part 2 closes the
`Json` gap left open there, plus two sibling gaps found while fixing it
(`JsValue === null`, and `==`/`!=` against `null`/`undefined` for both
`Json` and `JsValue`).

## Part 1: `JsValue === undefined`

### Symptom

Found while verifying [[project_npm_interop_gaps_6]]'s this-binding fix
against real joi: `result.error === undefined` (joi's own documented way
to check a `schema.validate(...)` call succeeded) came back `false` on
a valid input, even though `typeof result.error` correctly reported
`"undefined"` and `JSON.stringify(result.value)` round-tripped the
input correctly -- `result.error` genuinely *was* absent, but comparing
it to the literal `undefined` still said no.

### Root cause

`lower_optional_undefined_equality` (`crates/thaw-hir/src/lower/
expressions/coercions.rs`) special-cases every type pairing that can
mean "absent" against `HirType::Undefined` -- `Optional`, `Json`,
`Nullable`, `Nullish`, matching `Union` members -- each decoding the
comparison the right way for its own representation. It had no arm at
all for `HirType::JsValue` (a live QuickJS handle, e.g. `const x:
JsValue = someDynamicCall()`), so every such comparison fell through to
the function's own catch-all: "one operand's type is literally
`Undefined` and nothing more specific matched -> `false`" -- answering
unconditionally wrong for a live value that genuinely is `undefined`,
never actually asking the live engine what the value is.

### Fix

There's no local representation to compare a live handle against --
unlike `Json`'s sentinel-tagged-object trick (below), a `JsValue` is
just an opaque handle into QuickJS's own heap. Ask the engine directly,
reusing the exact mechanism `typeof` on a `JsValue` already uses
(`UnaryOp::TypeOf`'s own `JsValue` arm, same file): a new bootstrap-
registered global, `__thaw_is_undefined_dynamic_value = value => value
=== undefined` (`crates/thaw-quickjs/src/quickjs/platform_globals/
runtime.js`, right next to the pre-existing `__thaw_typeof_dynamic_
value`), invoked through the same `callDynamicValueWithValue` mechanism
and decoded as a `JsonAsBool`. Added for both operand orders
(`(JsValue, Undefined)` and `(Undefined, JsValue)`); `!==` already
falls out for free, since the caller negates whatever this function
returns.

### Verification (Part 1)

- Network-free test:
  `a_live_dynamic_values_property_compares_equal_to_undefined_when_actually_absent`
  (`thaw-cli`, `tests/registry_fallback/results_and_exports.rs`) --
  a synthetic package modeled on joi's own `validate(...):
  ValidationResult<TSchema>` shape (a generic, unresolvable interface
  collapsing to one opaque `JsValue` handle), covering both operand
  orders and both `===`/`!==`.
- Confirmed against real joi: the original, previously-failing `jo.ts`
  repro (`result.error === undefined || result.error === null`, no
  explicit type annotation needed -- `schema.validate(...)`'s return
  already infers as `JsValue` on its own) now prints the expected
  `true`/`true`.

## Part 2: the `Json` gap, plus two siblings found while fixing it

### Symptom

The exact same symptom as Part 1 also reproduced for a plain `Json`
value (e.g. `JSON.parse('{"a":1}').b === undefined` returned `false`,
and `=== null` on the same missing key returned `true`) -- a
*different*, deeper bug, not a second instance of Part 1's. Researching
it surfaced two more, closely related gaps in the same neighborhood:
**`JsValue === null`** was *also* still broken (Part 1 added a
`(JsValue, Undefined)` arm but no `(JsValue, Null)` one), and **`x ==
undefined`/`x == null`** (loose equality) **crashed at build time**, not
just answered wrong, for `Json` and `JsValue` alike -- confirmed
empirically: `JSON.parse('{}').missingKey == undefined` failed to build
with `"numeric conversion is not defined for native type Undefined"`.

### Root cause

`Json`'s own missing-key read (`thaw_json_get`, `crates/thaw-std/src/
json.rs`) fell back to plain `Value::Null` whenever a key wasn't
present -- collapsing "the key is genuinely absent" and "the key is
present with an explicit `null`" into one indistinguishable
representation. This was a documented, deliberate limitation elsewhere
in the codebase already (see the doc comment on `HirExpr::JsonSet`,
`crates/thaw-hir/src/hir/ir.rs`, which called out this exact
`thaw_json_get`-reports-`null`-for-a-missing-key behavior as a known
sharp edge it had to route around for its own narrower purpose).

Json already has a way to represent a real `undefined` -- a
`$__thaw_napi_undefined$`-tagged sentinel object
(`is_napi_undefined`/`__thaw_json_is_undefined`, the pre-existing
`Json`/`Undefined` equality arm from Part 1) -- but it was only ever
produced deliberately, by the native-callback argument-marshaling path
(`compile_json_object_set_native_with_undefined` and its callers,
`crates/thaw-llvm/src/hir_codegen/json_bridge/encoding.rs`), never by a
plain missing-key read. `thaw_json_is_null` is a *strict* `Value::Null`
match, so the existing `(Json, Null)`/`(Json, Undefined)` equality arms
started working correctly the moment the runtime stopped conflating the
two -- **this needed zero compiler changes for `===`/`!==`**, only a
runtime-representation fix.

`lower_loose_equality` (`==`/`!=`), by contrast, had no `Json`/`JsValue`
-aware arms at all -- only `Optional`/`Nullable`/`Nullish` and a literal
`(Null, Undefined)` pair -- so it fell through to
`coerce_primitive_to_number` on both operands, which has an explicit
`Err` arm for a bare `Undefined`/`Null` operand. A real, empirically
confirmed build-time crash, not just a wrong answer.

### Fix

- **`crates/thaw-std/src/json.rs`**: `thaw_json_get`/`thaw_json_index`
  synthesize the napi-undefined sentinel (new `napi_undefined_value()`)
  instead of `Value::Null` for a genuinely missing key/index.
  `thaw_json_as_string`/`thaw_json_console_string` special-case it
  (`"undefined"`, matching real `String(undefined)`/
  `console.log(undefined)`). New `thaw_json_is_nullish` (`is_null ||
  is_napi_undefined`). `thaw_json_as_number` deliberately stays
  unchanged (`0.0`, not `NaN`, for a missing key) to avoid rippling into
  arithmetic results for existing code -- documented at the top of the
  file, along with the one resulting, pre-existing (not newly
  introduced) inconsistency this leaves: `missing == 0` still reads
  `true`.
- **`crates/thaw-llvm`** (`runtime_declarations.rs`,
  `invocations/json_calls.rs`) and **`crates/thaw-hir/src/lower/
  inference/types.rs`**: wire up `__thaw_json_is_nullish` the same way
  `__thaw_json_is_null` already is.
- **`crates/thaw-hir/src/lower/expressions/coercions.rs`**: refactored
  `dynamic_value_is_undefined` into a shared `dynamic_value_check(global,
  value)`, reused by two new siblings, `dynamic_value_is_null` and
  `dynamic_value_is_nullish`. Added `(JsValue, Null)`/`(Null, JsValue)`
  to `lower_optional_undefined_equality` (closing the `JsValue === null`
  sibling gap). Added `(Json, Null)`/`(Json, Undefined)` (+ reverse) and
  `(JsValue, Null)`/`(JsValue, Undefined)` (+ reverse) to
  `lower_loose_equality`, calling `__thaw_json_is_nullish`/
  `dynamic_value_is_nullish` respectively.
- **`crates/thaw-quickjs/src/quickjs/platform_globals/runtime.js`**: two
  more bootstrap globals, `__thaw_is_null_dynamic_value = value => value
  === null` and `__thaw_is_nullish_dynamic_value = value => value ==
  null`, next to Part 1's `__thaw_is_undefined_dynamic_value`.

### Two regressions found mid-fix, both fixed in the same effort

1. **A `Nullable` dictionary read decoded a missing key as garbage
   instead of `null`.** `Record<string, number | null>`'s own
   desugaring (`lower_nullable_dictionary_value`,
   `crates/thaw-hir/src/lower/objects.rs`) checked *only* `is_null` to
   decide "none" vs. "some" -- correct before this fix (a missing key
   read as bare `Value::Null` too), but wrong after: the sentinel isn't
   literally `null`, so a missing key was now treated as "present" and
   decoded as a real payload value (`0` for a numeric field, instead of
   `null`). `Nullable` has no third "undefined" state to represent a
   missing key with, so both real `null` and the sentinel now map to
   "none" there -- but *only* for `Nullable` (not `Nullish`, which has a
   real `undefined` state, and whose own caller already gates on
   `has_own` before ever reaching this function, so an already-present
   sentinel value there is a much rarer, pre-existing, unrelated edge
   case, deliberately left alone). Caught by the existing test
   `dictionary_reads_restore_optional_nullable_and_nullish_values`
   (`thaw-llvm`).
2. **`ordered_json`/`filtered_json` (backing `JSON.stringify`) must NOT
   omit/null a nested sentinel value**, even though that would match
   real `JSON.stringify`'s treatment of a real `undefined` -- an earlier
   version of this fix added exactly that, and it broke two existing
   tests (`a_bare_undefined_literal_can_be_passed_as_a_dynamic_call_
   argument` and its `Optional`-typed sibling, plus a native-addon
   class-method test, all in `thaw-cli`). Root cause:
   `thaw_json_stringify` isn't only reached by user-facing
   `JSON.stringify(...)` -- it's *also* how several dynamic-call
   argument/result marshaling paths internally serialize a value (often
   nested inside a fresh single-element array,
   e.g. `compile_set_dynamic_property_json`, `crates/thaw-llvm/src/
   hir_codegen/invocations/dynamic_calls.rs`) before handing it to the
   live QuickJS engine, whose own deserializer looks for the sentinel's
   *exact* shape to revive a real `undefined` argument -- nulling/
   omitting it there silently corrupted that unrelated, pervasive
   mechanism. Reverted; a *nested* sentinel value's `JSON.stringify`
   treatment stays as it always has been (own raw JSON shape leaks
   through) -- a real, narrower, pre-existing rough edge, left open (see
   `ordered_json`'s own doc comment in `json.rs` for the full
   reasoning).

### Verification (Part 2)

- New tests: `a_missing_key_or_index_is_distinguishable_from_an_explicit_
  null` and `stringify_...` extensions (`thaw-std`, network-free);
  `a_missing_json_key_or_index_is_distinguishable_from_an_explicit_null`
  (`thaw-llvm`); `a_live_dynamic_values_property_compares_equal_to_
  null_or_via_loose_equality` (`thaw-cli`, network-free).
- Full `thaw-hir` (258), `thaw-std` (29), `thaw-quickjs` (68),
  `thaw-llvm` (572), and `thaw-cli` (412) unit suites green; `cargo
  fmt`/`clippy -D warnings` clean; full gated
  `THAW_RUN_NPM_INTEGRATION=1 --release --test-threads=1` suite green.
- Confirmed against real joi and a hand-written synthetic package
  (mirroring both the strict- and loose-equality matrices, both operand
  orders) end to end via direct `thaw build`/run, not just the pinned
  test suite.

### Deliberately left open

- `thaw_json_as_number` stays `0.0` (not `NaN`) for a missing key.
- A nested sentinel value's `JSON.stringify` treatment (see regression 2
  above).
- `thaw_json_index_set`'s array-hole padding stays real `Value::Null`
  (not the sentinel) -- a written hole and a missing key are different
  representations; a hole still reads back as `null` instead of
  `undefined`, unchanged from before this fix.
- `compile_json_is_napi_undefined`'s laxer strictness (checks the
  sentinel key's mere presence via `thaw_json_has_own`, not that its
  value is `true`, unlike the Rust-side `is_napi_undefined`) -- an
  existing, narrow, exotic-input-only mismatch, not newly introduced.
