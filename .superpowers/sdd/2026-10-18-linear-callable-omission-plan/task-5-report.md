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
