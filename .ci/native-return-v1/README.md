# Native Return cumulative HIR CI diagnostics

This harness validates a disposable candidate built from the pinned product baseline `8b353a99d4995c8217b9e73cd308ea85d2d6e5d8` on the `validation/native-return` branch. It does not change product sources outside the candidate overlay or run broader repository CI.

## Pinned reconstruction

The first four stages and their component-ordered manifests remain unchanged at 175, 176, 179, and 180 files. The fifth stage preserves the exact paired repair base and `compile-repairs-v1.patch`; its final 181-file manifest remains lexical and retains its separate pinned `crates/thaw-std/src/json.rs` hash.

The sixth stage applies only `cumulative-hir-v1.patch`. Immediately before it, the verifier checks its lexical 181-file paired base and proves it is byte-identical to the fifth-stage final manifest and matches the candidate tree. The seventh stage applies only `next-hir-v1.patch`; its lexical 181-file paired base is checked immediately before application and must exactly match the sixth-stage final. Both stages retain the same stage-scoped JSON owner. The full checkout delta is pinned to 54 owners: 49 modified and five added. The subset roots are not globally expanded.

## Diagnostics

The runner performs independent locked `thaw-llvm` library checks for baseline and candidate. Candidate no-run gates remain LLVM first, then HIR, followed by `thaw-std --lib --no-run`. The existing 27 filters and their order are retained; three HIR filters are appended with exact pinned names and counts: `thaw_rethrow_cleanup_` selects seven names, `error_argument_staging_` selects seven names, and `existing_native_spread_staging_mode_false_is_unchanged` selects one name. All 30 groups (20 LLVM, nine HIR, one std) record fully qualified selected and executed names. Exact-name controls fail closed on empty, substituted, duplicate, ignored, malformed-summary, partial, or wrong-count results.

This remains an LLVM-first diagnostic lane: if LLVM compilation or its no-run gate fails, later HIR/std compile and focused controls are blocked. The inherited 24-owner native draft and product integration remain unreviewed and on HOLD. This lane does not establish runtime behavior or integration readiness.

## Local harness tests

Run the standard-library-only harness tests with:

```sh
python3 -m unittest discover -s .ci/native-return-v1/tests -v
```

These tests use synthetic Git fixtures and do not run Rust/Cargo, install dependencies, download LLVM, or contact a network service. The GitHub workflow installs the pinned LLVM asset only on its runner. A green harness test result confirms harness behavior, not product compilation or runtime behavior.
