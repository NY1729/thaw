# QuickJS follow-up 2 draft

`quickjs-followup-2.patch` is the original five-file, ten-hunk patch, preserved byte for byte. SHA-256: `3246a5d75a444b5098fe37b2288565a9d405da9187f3806d2068d8ef3bcf5853`.

Apply only after reconstructing the baseline plus all 12 pinned stages described in the parent directory. The pristine roster of 186 files was checked against `quickjs-api-followup-v1.sha256`. This patch is not directly applicable to main or refactor/codebase. Check it against the reconstructed pristine tree with:

```sh
patch --dry-run --batch --fuzz=0 -p1 -d /path/to/pristine -i /absolute/path/to/quickjs-followup-2.patch
```

The patch changes QuickJS runtime wrappers and lazy Buffer.from access, synchronizes instance memories for table-derived Wasm funcref calls, and corrects test expectations in filesystem.rs, api.rs and tests.rs. It excludes the unresolved Symbol/nonlive-graph and forced_root_default_query issues, thaw-std/json.rs, diagnostic output and temporary tests.

`quickjs-followup-2.json` records the exact paired source hashes and patch checks. No tests or builds were run during this upload. The current CI workflow, verifier, pins and active patches are unchanged; this draft directory is not included in the verifier's named patch list, and the workflow push trigger is limited to validation/native-return. This commit adds archival files only.

## quickjs-followup-7.patch

`quickjs-followup-7.patch` supersedes `-2` (which is kept unchanged). It is a 13-file, 47-hunk patch against the same reconstructed pristine tree (baseline plus the 12 pinned stages; `patch -p1 --dry-run --batch --fuzz=0` succeeds). SHA-256: `d10173eb3748ecc33f5a5fc7401602c88e5343b696c2819bf624e06d55ce44a2`.

On top of `-2` it adds: a `thaw-std` graph-decode test aligned with the lease design, a detached-`hdl`-marker set so `thaw_json_borrowed_handle_id` returns 0 for markers decoded without HostOperations, the Symbol graph test split into a by-value roundtrip plus an explicit live-handle contract test, `thaw_arena::destroy_string` accepting `*const c_char`, three small `thaw-napi` library fixes (it did not build before), and `thaw-napi` test updates for the current API.

Measured locally on the reconstructed tree, not by CI: thaw-arena 11, thaw-runtime 221, thaw-std 85 and thaw-quickjs 278 tests pass; thaw-napi compiles and 197 of 218 tests pass (the rest need a graph ownership design decision; several are marked "Unrun" in source). thaw-hir still fails to compile (41 errors, definitions absent from every commit) and blocks thaw-bridge, thaw-llvm and thaw-cli. Nothing here changes the verifier, pins, workflow or active patches.
