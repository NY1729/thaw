# tools/parity-aot: AOT-compiled thaw vs Node probes

Node 22 is the oracle. The thaw CLI compiles generated probe programs (`thaw build <file.ts> -o <bin>`), each
binary runs, and its output is diffed per probe with `tools/parity/diff.py`. Slice 1 covers Buffer and
Uint8Array across typing modes, Buffer statics, and the constructibility of every `Ctor` in
`crates/thaw-hir/src/lower/platform_globals.rs`. Non-gating: findings are reported, never failed on.

Plan: `designs/plan-q9-aot-parity-harness.md` (PR 1). Scope is in its section 5.

## Files

- `gen_probes.py`: writes the programs, a `lib.ts`/`lib.mts` module fixture, and `manifest.json` (probes, findings, counts).
- `aot_run.py`: Node reference, thaw build/isolation/run per engine, classification, diff, summary.
- `run.sh`: the entry point (gen, then aot_run). `selftest.sh`: determinism, Node run of every program, classifier checks.
- `known-gaps.json`, `waivers.json`, `skip.txt`: same schema as `tools/parity`. Gaps and waivers are empty at slice 1.

## Probes

Each probe is one line, `p("<name>", () => { <setup> return <expr>; });`, printed with the shared `show()` helper.
A throw prints `THROW <name>: <message>`. Expected output comes from Node only.

Families and modes (`<family>.<mode>.cNN` programs, at most 40 probes each):

- `buffer`, `u8`: receiver `Buffer.from([1, 2, 3, 250])` or `new Uint8Array(...)` in modes `typed`, `any`,
  `union`, `unknown`, `generic`, `callback`, `json` (JSON-expressible ops only), `module` (value from `lib`).
  29 ops per mode: methods, index get/set/out-of-range/negative, spread, for..of, destructuring, `in`, `delete`,
  `typeof`, `instanceof`, `String()`, `JSON.stringify`, `Array.isArray`, loose equality.
- `static`: `Buffer.concat`, `alloc`, `from`, `isBuffer`, `byteLength`, `compare`, ... in modes `typed`, `any`, `callback`.
- `globals`: `new X(args)` (typed and any), `instanceof`, `typeof` for each `Ctor` in PLATFORM_GLOBALS.

## Classes

Per probe, per engine: `MATCH`; `MSG` (same THROW name, different message); `WRONG-TYPE` (THROW vs value, or a
different THROW name); `WRONG-VALUE`; `LEAK` (thaw output has `__thaw_js_handle_id__`, Node does not); `MISSING`
(no output block); `CRASH` / `HANG` (a probe that kills or hangs its binary when rerun alone); `REJECT` (compile
diagnostic naming the probe, or no location after bisect); `REJECT?` (build budget exhausted); `UNVERIFIED`
(rerun budget exhausted).

Isolation: a build that fails on a named probe line drops that probe and rebuilds. Without a line the probe list
is bisected. At most 25 extra builds per program. After a crash or hang, the probes after the last printed header
are rerun one per binary (at most 40 per program).

List drift: the three lists in `platform_globals.rs` (PLATFORM_GLOBALS Ctors, DYNAMIC_NEW_CTORS, DYNAMIC_OPAQUE_CTORS)
are compared. Differences are `LIST-DRIFT` findings. Method names used by ops must exist in
`crates/thaw-js/src/builtin_table.tsv` (`OP-DRIFT`).

## Run

    tools/parity-aot/selftest.sh                 # no thaw build; determinism, node run, classifier checks
    THAW=/path/to/thaw tools/parity-aot/run.sh [OUT_DIR]
    tools/parity-aot/run.sh --write-baseline     # records current gaps into known-gaps.json (after a fix only)

Build command per program and engine (`THAW_JS_ENGINE` is `thaw-js` or `quickjs`):

    THAW_JS_ENGINE=<engine> thaw build <program>.ts -o <binary>      (cwd: the program's work dir)

Env: `THAW` (default `<repo>/target/release/thaw`), `NODE`, `PARITY_AOT_ENGINES` (default `thaw-js,quickjs`),
`PARITY_AOT_JOBS`, `PARITY_AOT_BUILD_WRAP` (command prefix for each build, e.g. a systemd-run memory cap),
`PARITY_AOT_RUN_TIMEOUT` (20 s), `PARITY_AOT_BUILD_TIMEOUT` (900 s), `PARITY_AOT_SCRATCH` (default
`/home/fedora/work/thaw-tmp/parity-aot-scratch`), `PARITY_AOT_OUT`.

Output (OUT_DIR): `summary.md` / `summary.json` (counts per corpus and class, top findings, engine delta),
`<engine>/gaps.json`, `<engine>/diff.txt`, `<engine>/classes.json`, `node/*.out`, `programs/`.

Exit: 0 when the harness ran (gaps are reported only). 2 on a harness error (node failure, missing header,
missing thaw binary, diff.py harness error). `diff.py` exit 1 means gaps not in `known-gaps.json`; the run still exits 0.

## CI

`.github/workflows/parity-aot.yml` runs on `workflow_dispatch` or when the `parity-aot` label is added to a PR.
It builds the release thaw CLI, runs `selftest.sh`, runs both engines, and writes the summary and the
`parity-aot-results` artifact. The job is `continue-on-error`.
