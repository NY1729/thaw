# Task 4 report — canonical ABI for class constructors and initializers

Implementation commit: `feat(hir): canonicalize class constructor omission ABI`.

Source constructors with optional/defaulted parameters now use undefined-capable ABI slots while retaining physical source omission masks. Generated initializer signatures shift those masks past the receiver. The initializer resolves defaults and destructuring once in source order before constructor statements; this places derived defaults before `super(...)`, with instance fields and parameter-property writes after `super`. Parameter properties read the resolved binding and coerce to the declared field type. Constructor and initializer adapter symbols temporarily forward canonical slots; Task 5 removes these adapters.

The LLVM control covers required/defaulted/optional parameters, omitted versus explicit `undefined`, explicit `null`, parameter properties, a default expression evaluated once, a destructuring getter read once, rest arguments, explicit `super`, two implicit inheritance levels, and base/derived field timing. The initial red run failed with `unknown variable x` for constructor destructuring. Existing HIR super-order coverage was updated for the new parameter prologue.

Validation:

- `cargo test --locked -p thaw-hir --lib` — 567 passed.
- `cargo test --locked -p thaw-llvm --lib constructor -- --nocapture` — 35 passed.
- `cargo test --locked -p thaw-llvm --lib compiles_inherited_and_super_native_class_default_parameters` — passed.
- `cargo test --locked -p thaw-llvm --lib compiles_native_class_rest_parameters` — passed.
- `cargo test --locked -p thaw-llvm --lib constructor_defaults_and_parameter_properties_use_resolved_arguments` — passed.
- `cargo clippy --locked -p thaw-hir -p thaw-llvm --all-targets -- -D warnings` — passed.
- `cargo fmt --check` and `git diff --check` — passed.

No constructor growth check was added; that remains scheduled for Task 5. The full LLVM suite was not run.
