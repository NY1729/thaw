# Task 3 report — canonical ABI and prologues for class methods

Commit: `38ac96a2` (`feat(hir): canonicalize class method omission ABI`).

Source instance and static methods now give optional/defaulted parameters undefined-capable physical HIR slots, while retaining the hidden instance receiver at physical index 0. Existing source omission masks remain in physical coordinates; the existing unbound projection removes and shifts the receiver once, and inherited signatures continue to clone the projected base signature. Constructors and initializers were left unchanged.

`lower_class_methods` now lowers parameter default and destructuring bindings before user statements for both bound and generated unbound method bodies. This reuses `lower_binding_pattern`, preserving left-to-right default evaluation and the existing undefined-only default behavior. The bound body retains `this` in scope for defaults. Transitional wrapper generation and selection remain in place.

Focused coverage checks tagged bound/unbound/static parameter slots and the default prologue in HIR. LLVM controls cover receiver-reading defaults, static defaults with a later default depending on an earlier parameter, destructuring defaults, explicit `undefined`, method `.bind()` callable invocation with explicit `undefined`, optional `null`, inherited/super calls, rest, and generic callable compatibility.

Validation:

- `cargo test --locked -p thaw-hir --lib` — 567 passed, 0 failed.
- `cargo test -p thaw-hir --lib lowers_native_method_defaults_with_tagged_bound_and_unbound_parameters` — 1 passed.
- `cargo test --locked -p thaw-llvm compiles_and_runs_native_class_default_parameters` — 1 passed.
- `cargo test --locked -p thaw-llvm compiles_inherited_and_super_native_class_default_parameters` — 1 passed.
- `cargo test --locked -p thaw-llvm compiles_native_class_optional_parameters` — 1 passed.
- `cargo test --locked -p thaw-llvm compiles_native_class_rest_parameters` — 1 passed.
- `cargo test --locked -p thaw-llvm generic_arrow_bind_preserves_omitted_optional_and_call_time_rest_values` — 1 passed.
- `cargo test --locked -p thaw-llvm named_callable_values_keep_optional_default_and_rest_shapes` — 1 passed.
- `cargo clippy --locked -p thaw-hir -p thaw-llvm --all-targets -- -D warnings` — passed.
- `cargo fmt --check` — passed.
- `git diff --check` — passed.

The existing transitional `.bind()` path accepts and correctly defaults an explicitly supplied `undefined`; omitting that argument through its generated lambda currently reports that a required argument was omitted. A direct optional function-type assignment of a class method also does not adapt cleanly through the transitional named-method path. These callable omission adapters remain scheduled for the later wrapper-cleanup task; this task did not broaden them. The full `thaw-llvm` suite and workspace suite were not run.
