# A stateful default-exported namespace object losing `this`

**Status: done.**

## Starting premise (too pessimistic)

Round 6 of the npm bug hunt found that joi and handlebars both crash on
a genuinely instance-method-shaped call: `Joi.string()` fails joi's own
`assert(root, 'Must be invoked on a Joi instance.')`, and
`Handlebars.registerHelper(...)` fails with "cannot read property
'helpers' of undefined". Both packages default-export one *live,
stateful* object (joi's internal `root`, handlebars' `Environment`
instance), and both failures happen inside a real method on that object
that reads `this`.

The first read of this looked like it would require reworking core
dynamic-call *compilation* (thaw-hir/thaw-llvm): route every namespace-
qualified call on a value-exported object through method-call semantics
(`callDynamicMethod`/`invoke_method`, which already correctly does
`call_args.this(object)`) instead of `rewrite_qualified_calls`' plain
"free function" treatment -- a real, moderate-to-large change with
regression risk to every package that already works through the
existing path (zod, ms, kleur, commander, ...), because it is the exact
same path they use too.

## Root cause, precisely

Confirmed via `RUST_BACKTRACE=1` + a temporary panic, and a debug print
in `invoke_method` (`thaw-quickjs/src/quickjs/api.rs`) that never fired
even in a freshly rebuilt binary: `Joi.string()` does **not** go through
`callDynamicMethod`/`invoke_method` at all. Both packages' interface-
declared methods (`Root.string()`, `Environment.registerHelper()`) get
extracted by `extract_interface_method_decls` into detached, package-
level Fallback functions -- the exact same mechanism that turns every
one of lodash's ~300 methods, or zod's free functions, into a callable
global. `rewrite_qualified_calls` (`crates/thaw-cli/src/
registry_integration/calls.rs`) then rewrites the *source text* of
`Joi.string()` into a call to that detached function
(`joi_string()`) *before* thaw-hir/thaw-llvm ever see it -- correct for
a real namespace of free functions (zod's `z.string()`, no `this`
dependency), silently wrong for a real instance method whose
implementation depends on `this` referring back to the live object it
was declared on.

So the actual defect isn't in dynamic-call compilation at all -- it's
upstream of it, in how the compiled *JS glue code* detaches a method
from its owning object when copying it out to become a global. Two
separate binding mechanisms do this copy, both in `crates/thaw-bridge/
src/bridge/generation.rs`:

1. `wrap_as_commonjs_module`'s `module.exports` -> `globalThis` copy
   loop (the generic "every export becomes a global" step every
   package's bundle goes through).
2. `generate_module_init`'s `qualified_aliases` capture snippet (reads
   `globalThis.module.exports.{bare_name}` into a package-qualified key
   like `"joi::string"`, used to disambiguate same-named exports across
   packages -- confirmed via `strings` on a compiled test binary to be
   the mechanism the compiled program's runtime lookup actually goes
   through, not (1)).

Neither preserved the function's receiver; both were, in effect, doing
`const f = obj.method; globalThis.f = f;` -- exactly what silently
drops `this` in plain JS too.

## Fix

`.bind()` a copied function value to its own original owner
(`module.exports`) before handing it to `globalThis`, in both places.
A stateless function (the overwhelming majority of real packages'
methods -- lodash's `_.chunk`, zod's `z.string()`) is unaffected, since
it ignores whatever receiver it's bound to; a `this`-dependent one
(joi's `Root.string()`, handlebars' `registerHelper`) now keeps working.
Zero changes to any compilation or type-inference logic -- the fix is
entirely in generated JS text.

Both parts were necessary: applying only (1) against a synthetic
"statekit" reproduction (`interface StateKit { count(): number; }`,
`count()` reading `this.value`) still failed, because the compiled
program's actual lookup path runs through (2), not (1) -- found by
inspecting the compiled binary's embedded JS text directly with
`strings` after (1) alone proved insufficient.

## A separate bug found while verifying, fixed separately

Verifying joi's full `schema.validate(...)` flow against real input
surfaced `result.error === undefined` returning `false` even when
`typeof result.error` correctly reports `"undefined"`. Unrelated to
this-binding -- a pre-existing, general bug in equality comparison
against the literal `undefined` for a live `JsValue`. Fixed in
[[project_npm_interop_gaps_6]]'s follow-up, see
`docs/design/dynamic-value-undefined-equality.md`. The pinned joi test
below still uses `typeof x === "undefined"`, unaffected either way and
left as-is.

## Verification

- New network-free test:
  `a_stateful_namespace_objects_method_keeps_its_receiver_when_extracted`
  (`thaw-cli`, `tests/registry_fallback/results_and_exports.rs`) --
  the isolated "statekit" shape.
- New pinned tests (`THAW_RUN_NPM_INTEGRATION=1`, `thaw-cli`, `tests/
  native_addons/javascript_packages.rs`):
  `registry_add_validates_with_real_joi_when_enabled` (a real nested-
  schema build and validate, both a valid and an invalid input) and
  `registry_add_registers_helpers_with_real_handlebars_when_enabled`
  (a registered custom helper through a real `{{#each}}` block).
- Full `thaw-bridge` and `thaw-cli` unit suites green (410 tests);
  `cargo fmt`/`clippy -D warnings` clean; full gated
  `THAW_RUN_NPM_INTEGRATION=1 --release --test-threads=1` suite green.
