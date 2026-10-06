# Native Return CI candidate diagnostics

This payload supports one isolated, disposable-branch diagnostic run for the frozen native Return candidate. The workflow replaces the repository's broad inherited CI only on `validation/native-return`; it does not change `main`, `refactor/codebase`, product sources, or any Cargo manifest or lockfile.

## What the runner does

1. Checks out the complete product baseline at `8b353a99d4995c8217b9e73cd308ea85d2d6e5d8` and the workflow/payload at the triggering commit.
2. Runs the Python-standard-library harness tests, verifies the baseline Git tree against every checked-out tracked blob, and reconstructs the candidate in a separate temporary directory.
3. Replays the exact lineage in this order: the existing native draft patch, the existing scope-v5 patch, `discard-v2.patch`, then `return-v1.patch`. It checks every source-stage manifest and the final 35-owner delta before running Cargo.
4. Installs `libuv1-dev` and the hash-pinned official LLVM 22.1.8 x64 asset on the GitHub runner only. It records runner, Rust, Cargo, LLVM, C-linker, and lockfile identity.
5. Runs independent locked `thaw-llvm` library checks for baseline and candidate. A baseline failure does not suppress the candidate check. Candidate LLVM/HIR no-run compiles and the 23 focused filters are gated on their required predecessor stages.
6. Writes separate command logs and atomic status summaries including the exact control commit, repository, branch ref, run ID, and baseline SHA. Failures, timeouts, blocked gates, and never-run controls stay distinguishable; evidence is uploaded even after failure.

The four source-stage manifests contain 175, 176, 179, and 180 files. The final candidate differs from the product baseline by 30 modified owners and five added owners. No source repairs, three-way patch application, runtime benchmarks, NPM/Postgres lanes, PR, merge, deployment, or local product compiler setup belong to this experiment.

## Important source-review limit

The candidate inherits the original 24-owner native draft, which remains unreviewed/HOLD. This experiment verifies source identity and records compiler/test diagnostics; it does not establish runtime reclamation, product integration, or broad semantic acceptance. Review prose is not bundled or checked by CI. A green focused diagnostic would apply only to the exact pinned source and toolchain recorded by that run.

## Local harness tests

These tests use Python's standard library and synthetic Git fixtures. They do not run Rust/Cargo, install dependencies, download LLVM, or contact a network service.

```sh
python3 -m unittest discover -s .ci/native-return-v1/tests -v
```

The `verify.py` runner CLI requires the complete pinned baseline checkout, a new candidate destination outside the checkout, this payload directory, and an independent evidence directory. It is invoked before dependency setup in the workflow. `run.py --initialize` prepares not-run evidence before setup; `run.py --finalize-if-incomplete` marks outstanding stages blocked after a workflow step fails.

## Immutable inputs

- The handoff manifest, original native patch, and scope-v5 patch are read from their existing paths in the pinned baseline; they are not duplicated here.
- `discard-v2.patch` and `return-v1.patch` are the two successor deltas. Their hashes, stage manifests, native base owners, final owner roster, v5 test plan, dependency identities, and filter order are listed in `pins.json` and also fixed in `verify.py`.
- `provenance_only_review_fingerprints` in `pins.json` holds informational digests only. The underlying prose is not present in the payload and is not checked by the workflow.
- `original-native.sha256`, `scope-v5.sha256`, `discard-v2.sha256`, and `return-v1.sha256` verify the ordered 175→176→179→180 source chain. `native-base.sha256` verifies the 24 inherited owner bases; `net-owners.txt` verifies the exact 30-modified/5-new product delta.

The baseline checkout and the reconstructed candidate remain separate. Cargo target directories and evidence live outside both source trees. This harness is diagnostic tooling, not a product change.
