# Task 3 report: structural assignability

## Result

DONE. Added recursive source-contract assignability for object width subtyping, arrays, tuples, unions, optional, nullable, and nullish types. Plain object targets ignore class identity, private, and hidden accessor fields; class-shaped targets still require the expected identity, including inherited identity markers. Missing and incompatible public fields report their field names. Dynamic, Json, and JsValue source values remain deferred to runtime checks.

The typed QuickJS argument path now validates the source contract, retains the physical class value, and lets the existing class-to-Json live projection handle marshalling. Its class-object case now takes precedence over the generic function-value branch, which had otherwise tried to coerce the physical Json-backed fields directly to the structural ABI.

## TDD evidence and verification

Before implementation, the focused external structural argument tests failed as expected for decorated and undecorated classes: `value` was seen as physical `Json`, expected `F64`; the missing/wrong field control passed.

Focused tests:

```sh
LLVM_SYS_221_PREFIX=/usr/lib/llvm-22 CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 CARGO_BUILD_JOBS=2 TMPDIR=$PWD/target/tmp-task3 cargo test --locked -p thaw-hir external_structural_argument -- --nocapture
```

Result: 7 passed. Coverage includes decorated/undecorated classes, missing/wrong field negatives, nested objects, nested class arrays, tuples, unions, optional absence, nullable/undefined fields, local aliases, and public-field diagnostics.

Full HIR suite:

```sh
LLVM_SYS_221_PREFIX=/usr/lib/llvm-22 CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 CARGO_BUILD_JOBS=2 TMPDIR=$PWD/target/tmp-task3 cargo test --locked -p thaw-hir
```

Result: 595 passed, 0 failed; doc tests passed (0 tests). `cargo fmt --all` and `git diff --check` passed.

## Files changed

- `crates/thaw-hir/src/lower/storage.rs`
- `crates/thaw-hir/src/lower/invocations/calls.rs`
- `crates/thaw-hir/src/lower/tests/values.rs`

## Self-review and concerns

The tuple-source-to-tuple-target fallback continues to use the existing expression-level coercion so fixed tuple spread handling retains its previous evidence. The Task 4 Registry runtime bridge and Task 5 runtime matrix were not run here; those remain for their assigned tasks.
