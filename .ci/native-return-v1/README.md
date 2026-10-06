# Native Return cumulative HIR CI diagnostics

This harness validates a disposable candidate built from the pinned product baseline `8b353a99d4995c8217b9e73cd308ea85d2d6e5d8` on the `validation/native-return` branch. It does not change product sources outside the candidate overlay or run broader repository CI.

## Pinned reconstruction

The first four stages and their component-ordered manifests remain unchanged at 175, 176, 179, and 180 files. The fifth stage preserves the exact paired repair base and `compile-repairs-v1.patch`; its final 181-file manifest remains lexical and retains its separate pinned `crates/thaw-std/src/json.rs` hash.

The sixth stage applies only `cumulative-hir-v1.patch`. Immediately before it, the verifier checks its lexical 181-file paired base and proves it is byte-identical to the fifth-stage final manifest and matches the candidate tree. The seventh stage applies only `next-hir-v1.patch`; its lexical 181-file paired base is checked immediately before application and must exactly match the sixth-stage final. Both stages retain the same stage-scoped JSON owner. Through stage seven, the full-checkout delta is pinned to 54 owners: 49 modified and five added. The subset roots are not globally expanded.

## Diagnostics

After identity verification, the runner performs QuickJS and std test no-run preflights, then runs the nine new QuickJS/std groups before both baseline and candidate LLVM library checks. Those groups use their own package preflight and exact source identity; failures in the later LLVM/HIR lane do not gate them. The std no-run result is reused. The existing 30 filters retain their order and predecessor gating through the candidate LLVM check, LLVM no-run, HIR no-run, and the std no-run result. A failure in that old lane blocks its 30 groups without blocking the new nine. Exact fully qualified names are pinned for the 16 new tests: eight forced-root controls, six QuickJS regressions, and two std grant regressions. All 39 groups record selected and executed names, and exact-name checks fail closed on empty, substituted, duplicate, ignored, malformed-summary, partial, or wrong-count results.

The eighth stage applies only `forced-root-wire-v1.patch`. Its lexical 183-file paired base must match the seventh-stage final plus the two baseline QuickJS providers immediately before application; the final manifest keeps all prior-stage hashes and pins only those two provider changes. Stage-scoped extras preserve the earlier `crates/thaw-std/src/json.rs` handling without expanding the subset roots. This adds two modified owners, bringing the full-checkout roster to 56 owners: 51 modified and five added.

The workflow uses one job, build parallelism 2, a 90-minute job cap, and a 70-minute runner cap. Its pinned LLVM setup remains a prerequisite before the runner starts. The inherited 24-owner native draft and product integration remain unreviewed and on HOLD. A passing verifier confirms pinned reconstruction; control results record the selected tests’ observed outcomes. These results do not establish broader integration behavior or close separate runtime and semantic gates.

## Local harness tests

Run the standard-library-only harness tests with:

```sh
python3 -m unittest discover -s .ci/native-return-v1/tests -v
```

These tests use synthetic Git fixtures and do not run Rust/Cargo, install dependencies, download LLVM, or contact a network service. The GitHub workflow installs the pinned LLVM asset only on its runner. A green harness test result confirms harness behavior, not product compilation or runtime behavior.
