# Task 1 implementation report

Implemented named function value normalization at the `Expr::Ident` source-value boundary. The raw `FunctionRef` remains physically typed in inference. Compatible callable ABIs use `TypedClosure`; ABI-changing named defaults use the existing callable adapter and `__thaw_closure_alias`. `.bind` now consumes that normalized callable metadata. Nullable default adapters preserve `null` while forwarding non-null values.

Native strict equality now resolves closure aliases only when both operands are native function pointers. Union function members use the same resolver. Invocation still uses the adapted function pointer. No runtime API was added; LLVM declares the existing `thaw_closure_alias_resolve` symbol.

The optional fixed-prefix plus rest regression exposed four `split_off(params.len())` paths that could panic when fewer fixed arguments were supplied. Each now splits at `min(arguments.len(), params.len())`. Coverage exercises direct invocation, `.call`, `.apply` with supplied values, object fields, aliases, and immediate/retained `.bind` results.

## RED evidence

- Before normalization, `CARGO_TARGET_DIR=/tmp/thaw-generic-bind-target cargo test -p thaw-llvm named_callable_values_keep_optional_default_and_rest_shapes -- --nocapture` failed during lowering with `function `a` expects argument 1, but it was omitted`.
- After normalization, that case reached invocation and exposed the `split_off` panic for an omitted optional prefix with rest arguments. The same boundary existed in `.call`/`.apply`, immediate bound invocation, and object callable invocation; all four paths now clamp the split position.
- The nullable named default initially failed with `callable adapter for `nullable` cannot convert parameter 1 from Nullish(F64) to Nullable(F64)`. Added a present-value conversion that preserves null and unwraps non-null payloads.
- The first ABI-changing identity control returned false for repeated defaulted function references; native comparisons now resolve registered aliases. Defaulted function equality controls pass through repeated reads, object/array fields, an unannotated factory, and an exact required callback type.

## GREEN evidence

- `CARGO_TARGET_DIR=/tmp/thaw-generic-bind-target cargo test -p thaw-hir`: 558 passed, 0 failed.
- Focused LLVM tests passed: named callable shapes/identity/`any` projection (3), optional alias identity (1), nullable default (1), generic optional/rest bind control (1), and generic optional value control (1).
- `CARGO_TARGET_DIR=/tmp/thaw-generic-bind-target cargo clippy --workspace --all-targets -- -D warnings` passed.
- `cargo fmt --all -- --check` and `git diff --check` passed.
- No full LLVM suite was run in this worker, per the task instructions; CI is expected to cover the final branch head.

## Scope and limits

The required `number | undefined` parameter remains required through a named alias. Generic template values, generator handling, omitted-array `.apply` behavior, and later callable stages remain deferred as planned. `.apply` coverage uses supplied arguments; zero-argument `.apply` belongs to the separately deferred apply-omission stage.
