# Native Return compile-repair CI diagnostics

This harness validates a disposable candidate built from the pinned product baseline `8b353a99d4995c8217b9e73cd308ea85d2d6e5d8` on the `validation/native-return` branch. It does not change product sources outside the candidate overlay or run broader repository CI.

## Pinned reconstruction

The harness preserves the existing four-stage chain and manifests at 175, 176, 179, and 180 files. It then verifies the exact paired repair base: those 180 files plus only `crates/thaw-std/src/json.rs`, for 181 files. The hash-pinned `compile-repairs-v1.patch` is applied as stage five and checked against the final 181-file manifest. The full checkout delta is pinned to 44 owners: 39 modified and five new. The earlier stage roots remain unchanged.

## Diagnostics

The runner performs independent locked `thaw-llvm` library checks for baseline and candidate. Candidate no-run gates remain LLVM first, then HIR, followed by `thaw-std --lib --no-run`. After those gates, the original 23 filters remain in order; the HIR `thaw_remaining_` filter and typed JSON decode-scope std filter are appended. The HIR filter must list and execute exactly its eight pinned test identifiers; the std filter must list and execute its one pinned identifier. The fully qualified selected and executed names are retained in evidence. Empty, substituted, duplicate, ignored, malformed-summary, partial, and wrong-count results fail closed.

This is still an LLVM-first lane: if LLVM compilation or its no-run gate fails, the later HIR/std compile and focused controls are blocked. It is not a standalone HIR-only diagnostic.

## Local harness tests

Run the standard-library-only harness tests with:

```sh
python3 -m unittest discover -s .ci/native-return-v1/tests -v
```

These tests use synthetic Git fixtures and do not run Rust/Cargo, install dependencies, download LLVM, or contact a network service. The GitHub workflow installs the pinned LLVM asset only on its runner. A green harness test result confirms harness behavior, not product compilation or runtime behavior.
