# `liveValue === undefined` always answering `false`

**Status: done for `JsValue`. A related `Json` gap remains open (see
below) -- deliberately not attempted here.**

## Symptom

Found while verifying [[project_npm_interop_gaps_6]]'s this-binding fix
against real joi: `result.error === undefined` (joi's own documented way
to check a `schema.validate(...)` call succeeded) came back `false` on
a valid input, even though `typeof result.error` correctly reported
`"undefined"` and `JSON.stringify(result.value)` round-tripped the
input correctly -- `result.error` genuinely *was* absent, but comparing
it to the literal `undefined` still said no.

## Root cause

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

## Fix

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

## A related, harder gap -- NOT fixed here

The exact same symptom also reproduces for a plain `Json` value (e.g.
`JSON.parse('{"a":1}').b === undefined` returns `false`, and `=== null`
on the same missing key returns `true`) -- but this is a *different*,
deeper bug, not a second instance of the one above. `Json`'s own
missing-key read (`thaw_json_get`, `crates/thaw-std/src/json.rs`) falls
back to plain `Value::Null` whenever a key isn't present -- collapsing
"the key is genuinely absent" and "the key is present with an explicit
`null`" into one indistinguishable representation. This is a documented,
deliberate limitation elsewhere in the codebase already (see the doc
comment on `HirExpr::JsonSet`, `crates/thaw-hir/src/hir/ir.rs`, which
calls out this exact `thaw_json_get`-reports-`null`-for-a-missing-key
behavior as a real, known sharp edge it has to route around for its own
narrower purpose).

Json *does* already have a way to represent a real `undefined` --  the
same `$__thaw_napi_undefined$`-tagged sentinel object
`__thaw_json_is_undefined` (the pre-existing `Json`/`Undefined` arm
right above this fix's new one) already checks for -- but it's only
ever produced deliberately, by the native-callback argument-marshaling
path (`compile_json_object_set_native_with_undefined` and its callers,
`crates/thaw-llvm/src/hir_codegen/json_bridge/encoding.rs`), never by a
plain missing-key read.

Making `thaw_json_get` (and `thaw_json_index`, which has the identical
issue for an out-of-range array read) produce that same sentinel
instead of `Value::Null` looks tempting, but is a genuinely
cross-cutting change, not a narrow one: several other functions in
`json.rs` already special-case the sentinel today for a *narrow*
purpose (`thaw_json_typeof`, `thaw_json_as_bool`) and several more do
not (`thaw_json_as_string` would stringify it as `{"$__thaw_napi_
undefined$":true}` instead of `"undefined"`; `thaw_json_stringify`/
`stringify_with_keys` have no notion of omitting it from output the way
a real `JSON.stringify` omits an `undefined`-valued property). Auditing
and safely updating every consumer, without breaking the many already-
passing packages whose code likely already depends on today's "a
missing key silently reads as something falsy/null-like" behavior
one way or another, is real, cross-cutting work with genuine regression
risk -- unlike the `this`-binding fix's own initial scare, there is no
equally narrow alternative found here. Left open; flagged for a
decision on priority before attempting it.

## Verification

- New network-free test:
  `a_live_dynamic_values_property_compares_equal_to_undefined_when_actually_absent`
  (`thaw-cli`, `tests/registry_fallback/results_and_exports.rs`) --
  a synthetic package modeled on joi's own `validate(...):
  ValidationResult<TSchema>` shape (a generic, unresolvable interface
  collapsing to one opaque `JsValue` handle), covering both operand
  orders and both `===`/`!==`.
- Full `thaw-hir` (258), `thaw-llvm` (571), `thaw-quickjs` (68), and
  `thaw-cli` (411) unit suites green; `cargo fmt`/`clippy -D warnings`
  clean; full gated `THAW_RUN_NPM_INTEGRATION=1 --release
  --test-threads=1` suite green.
- Confirmed against real joi: the original, previously-failing `jo.ts`
  repro (`result.error === undefined || result.error === null`, no
  explicit type annotation needed -- `schema.validate(...)`'s return
  already infers as `JsValue` on its own) now prints the expected
  `true`/`true`.
