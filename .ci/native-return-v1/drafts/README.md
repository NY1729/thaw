# QuickJS follow-up 2 draft

`quickjs-followup-2.patch` is the original five-file, ten-hunk patch, preserved byte for byte. SHA-256: `3246a5d75a444b5098fe37b2288565a9d405da9187f3806d2068d8ef3bcf5853`.

Apply only after reconstructing the baseline plus all 12 pinned stages described in the parent directory. The pristine roster of 186 files was checked against `quickjs-api-followup-v1.sha256`. This patch is not directly applicable to main or refactor/codebase. Check it against the reconstructed pristine tree with:

```sh
patch --dry-run --batch --fuzz=0 -p1 -d /path/to/pristine -i /absolute/path/to/quickjs-followup-2.patch
```

The patch changes QuickJS runtime wrappers and lazy Buffer.from access, synchronizes instance memories for table-derived Wasm funcref calls, and corrects test expectations in filesystem.rs, api.rs and tests.rs. It excludes the unresolved Symbol/nonlive-graph and forced_root_default_query issues, thaw-std/json.rs, diagnostic output and temporary tests.

`quickjs-followup-2.json` records the exact paired source hashes and patch checks. No tests or builds were run during this upload. The current CI workflow, verifier, pins and active patches are unchanged; this draft directory is not included in the verifier's named patch list, and the workflow push trigger is limited to validation/native-return. This commit adds archival files only.
