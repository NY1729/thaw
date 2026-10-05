# JIT callback result chain — scratch checkpoint (HOLD)

Candidate: `/tmp/thaw-luna-jit-result-chain/candidate`

Base reference: managed unit 495 (`bfe64fc8e9d4af054c4d800e9b18241d613c4cb3`). Authoritative managed HEAD is unit 496 (`f4e2faf98b6973306ac634f2d78968904b846480`); paired owner-byte comparison against 496 is still required before any freeze or upload. No managed or original checkout edits. No product execution/build/test/compiler/parser/formatter was run.

Current source changes cover callback result-kind proof, switch default discovery/replay and break-prefix handling, operand-family checks, explicit dynamic-pair input validation API, callback capture/presence packets, sparse map/scan result presence, reducer accumulator and initial/result presence transport, callback conversion roots, and presence-aware common scalar/dynamic conversions. Latest source addition makes Number/Boolean/String/Dynamic toString/toNumber conversions consume the runtime presence byte and preserve undefined/null results; analyzer accepts matching Maybe families for those conversions. Presence-aware conversion helpers root pointer-backed present inputs before converters that can allocate/re-enter.

Latest tracked source hashes:

- `analysis.rs`: `8d7be30c65d30d755faef03d2002ccc9889c67352b6fb736e137bef244c0c473`
- `codegen.rs`: `dee0c7234fe434302168623de05b5a3d67032774c7a2e537b5a7f86de39c3923`
- `machine.rs`: `9ebb715d84a00e32602db0f4cf7de4a88052fb2a3f00fe133060d2627fce63aa`
- `runtime/values.rs`: `c6ee6d2e584188d055c89238601d5ba448c08c4a239b6e74aeba61cd8ef49e7d`
- `runtime/arrays/callbacks/jit.rs`: `31c2188d94ebd210b31ad841fe42359c1df2ec00e52e0a78a50483f4a5cfad3b`
- `runtime/globals.rs`: `23a1b710e5724878dc0798a118643a54851598b979b0085ae29865d85bd0b691`
- `tests.rs`: `425084a430bb85f3b35f9304adfc7ff5646a605a54951b1a2050da6dd6dbf5c1`

`git diff --no-index --check base candidate` reported no whitespace errors. This is a static source check only, not validation of compilation or behavior.

Open gates before candidate can be called complete:

1. Wire `callback_result_kind_code_for_environment` into all real callers using exact builtin argument layouts, capture presence/kinds, and dynamic-pair starts. Current `compile_jit_callback` has analyzer proof but public caller environment validation is not wired.
2. Preserve mixed capture value families through the outer capture producer; `captureappend` currently loses family evidence through the legacy NumberArrayAppend representation. Do not trust callback token annotations as ABI proof.
3. Complete pointer consumer/container element-family checks (including captured arrays/dictionaries/field values) and cross-check producers against actual caller representations.
4. Audit all optional string/dynamic operations beyond the conversion set; absent receiver methods must retain their throwing contract, while valid Maybe coercions must remain supported.
5. Add unrun source controls for new optional coercion paths, then conduct paired source review against current HEAD 496. Do not run the controls or product tools under the current restriction.
6. No stable freeze, commit, or integration until the above gates are closed and paired owner hashes are confirmed.
