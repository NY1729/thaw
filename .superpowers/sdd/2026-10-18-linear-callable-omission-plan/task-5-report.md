# Task 5 report: canonical class omission ABI

Removed class constructor, initializer, instance/static method, unbound-method, and inherited default-arity/omitted-mask wrapper generation. Source omission metadata now pads the full canonical ABI (including receiver offsets and native rest packing) and rejects omitted required arguments even when their type includes `undefined`. Saved `.bind()` closures retain shifted omission metadata, and compatible named class method references adapt directly to their canonical signature. Incompatible or named-`this` callable shapes still decline adaptation; no recursive mask dispatcher or omitted-symbol lookup remains.

Added HIR physical-signature assertions and a bounded growth guard for 8, 17, and 65 optional/default constructor and method parameters, with a named callable value in each case. HIR remains at four functions for each size. Added negative checks for required `number | undefined` and generic `T | undefined` constructor/method parameters, plus LLVM regressions for omitted `.bind()` arguments and optional callable assignments from both static and bound instance methods. The `.bind()` regression was first run red (`function bound expects argument 1, but it was omitted`) before the closure-mask fix.

Verification passed:

- `cargo test --locked -p thaw-hir --lib` — 569 passed.
- HIR growth/canonical-signature/required-undefined focused tests passed, including generic required-undefined cases.
- LLVM class controls: 23 `native_class_` tests, plus focused bound/static callable, super/inheritance defaults, rest, typed-tuple bind, extracted and explicit-receiver methods, receiver identity, constructor properties/defaults, generic specialization/inheritance, and optional-void adapter tests passed.
- `cargo test --locked -p thaw-hir --lib storage_preserves_native_ffi_class_layout` passed.
- `cargo clippy --locked -p thaw-hir -p thaw-llvm --all-targets -- -D warnings`, `cargo fmt --all -- --check`, and `git diff --check` passed.

Repository audit found no remaining `omitted_parameter_symbol`, `default_arity_symbol`, subset-mask generator/parser, class wrapper generator, old selector name, or omission-parameter cap. The only remaining 16-byte limit is the unrelated direct aggregate FFI register-layout constraint in `crates/thaw-hir/src/hir/program.rs`; `HirOptionalMask` remains as canonical omission metadata. No publication or full LLVM suite was performed.

## Follow-up: bound native method rest arguments

An additional review found that specialized instance/static class `.bind()` paths treated the physical native rest-array slot as fixed and dropped rest metadata from their returned callable. Both regressions were confirmed red before the fix with a bind arity rejection. The bind paths now split fixed parameters from the native rest element, coerce bound rest-prefix values by element type, preserve the returned callable's rest type, and concatenate bound prefix values with invocation-time rest values. Instance and static controls cover `sum(first, base = 1, ...values)`, bound prefix `[2, 3]` with omitted `base`, and no-rest-prefix binds that retain the optional `base` mask. They pass with output `11\n20\n14\n6\n`.

Follow-up verification passed: full `thaw-hir` (569), LLVM class filter (23), both new rest-prefix controls, prior bind/call/apply/rest/inheritance/super/receiver/generic controls, FFI layout control, Clippy, fmt, and diff checks.
