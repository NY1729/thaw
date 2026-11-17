# tools/parity-aot: AOT-compiled thaw vs Node probes

Node 22 is the oracle. The thaw CLI compiles generated probe programs (`thaw build <file.ts> -o <bin>`), each
binary runs, and its output is diffed per probe with `tools/parity/diff.py`. Slice 1 covers Buffer and
Uint8Array across typing modes, Buffer statics, and the constructibility of every `Ctor` in
`crates/thaw-hir/src/lower/platform_globals.rs`. Non-gating: findings are reported, never failed on.

Plan: `designs/plan-q9-aot-parity-harness.md` (PR 1). Scope is in its section 5.

## Files

- `gen_probes.py`: writes the programs, a `lib.ts`/`lib.mts` module fixture, and `manifest.json` (probes, findings, counts).
- `aot_run.py`: Node reference, thaw build/isolation/run per engine, classification, diff, summary.
- `run.sh`: the entry point (gen, aot_run, coverage). `selftest.sh`: determinism, Node run of every program, classifier and coverage checks.
- `coverage.py`: API coverage of `manifest.json` and the shrink-only ratchet. `coverage-baseline.json`: the untouched `id#mode` pairs. `modules.txt`: the 55 thaw-js builtin modules (copied from `THAW_JS_BUILTIN_MODULES` in `crates/thaw-cli/src/registry_integration/thaw_js_routing.rs`; selftest checks they match).
- `known-gaps.json`, `waivers.json`: same schema as `tools/parity`. `skip.txt`: probe-name regexes with a reason
line each. gen_probes.py drops them at generation (manifest `skipped` counts them) and diff.py drops them from both sides.

## Probes

Each probe is one line, `p("<name>", () => { <setup> return <expr>; });`, printed with the shared `show()` helper.
A throw prints `THROW <name>: <message>`. Expected output comes from Node only.

Families and modes (`<family>.<mode>.cNN` programs, at most 40 probes each):

- `buffer`, `u8`: receiver `Buffer.from([1, 2, 3, 250])` or `new Uint8Array(...)` in modes `typed`, `any`,
  `union`, `unknown`, `generic`, `callback`, `json` (JSON-expressible ops only), `module` (value from `lib`).
  29 ops per mode: methods, index get/set/out-of-range/negative, spread, for..of, destructuring, `in`, `delete`,
  `typeof`, `instanceof`, `String()`, `JSON.stringify`, `Array.isArray`, loose equality.
- `static`: `Buffer.concat`, `alloc`, `from`, `isBuffer`, `byteLength`, `compare`, ... in modes `typed`, `any`, `callback`.
- `globals`: `new X(args)` (typed and any), `instanceof`, `typeof` for each `Ctor` in PLATFORM_GLOBALS. Constructor
  arguments come from `CTOR_ARGS` in gen_probes.py. Names in `skip.txt` (Node's "Illegal constructor", DisposableStack,
  BroadcastChannel and WebSocket) keep only their `typeof` probe.
- builtin namespaces (`array.typed`, `math.any`, ...): a namespace is called directly in `typed` mode (`Math.abs(2)`),
  with no `typeof Math` annotation and no `union` mode (both were probe artefacts). `typeofann`: one
  `const b: typeof <ns> = <ns>` probe per namespace, the one family that keeps the annotation form, so that gap stays measured.
- `tag`: `Object.prototype.toString` on each native kind (Map, Set, typed arrays, Buffer, Promise, ...), printed by the
  tag printer (`pt` probes, `showTag` in the prelude). The value families print with `show()`, which prints the
  constructor name and never calls `Object.prototype.toString`, so a tag gap cannot hide a value diff.

## Classes

Per probe, per engine: `MATCH`; `MSG` (same THROW name, different message); `WRONG-TYPE` (THROW vs value, or a
different THROW name); `WRONG-VALUE`; `LEAK` (thaw output has `__thaw_js_handle_id__`, Node does not); `MISSING`
(no output block); `CRASH` / `HANG` (a probe that kills or hangs its binary when rerun alone); `REJECT` (compile
diagnostic naming the probe, or no location after bisect); `REJECT-EXPECTED` (a `REJECT` where Node also throws a
TypeError for a non-function callback: `(1 as any)` to an array or promise method, documented in `aot_run.py`);
`REJECT?` (build budget exhausted); `UNVERIFIED` (rerun budget exhausted).

MSG is gated by error name only (`diff.py`). The summary lists the text differences in a separate non-gating
section, grouped by thrown name.

Isolation: a failing build names its error sites (`error` blocks of the diagnostic only). Every named probe is
dropped in one pass and the rest rebuilt. A diagnostic with no named site bisects the probe list. At most 120 builds
per program (`EXTRA_CAP`). After a crash or hang, the probes after the last printed header are rerun one per binary
(at most 40 per program).

List drift: the three lists in `platform_globals.rs` (PLATFORM_GLOBALS Ctors, DYNAMIC_NEW_CTORS, DYNAMIC_OPAQUE_CTORS)
are compared. Differences are `LIST-DRIFT` findings. Method names used by ops must exist in
`crates/thaw-js/src/builtin_table.tsv` (`OP-DRIFT`).

## Coverage

`coverage.py` reads only `manifest.json` (no thaw). Each probe lists its API ids in `api` (`<owner>.<key>` as in
`crates/thaw-js/src/builtin_table.tsv`, e.g. `Array.prototype.map`, `Map.prototype.size`; `global:<name>` for
`tools/parity/node_globals.json`; `mod:<name>.<export>` for the modules in `modules.txt`) and a `mode`.

Universe, computed at run time: table function and accessor rows (except `length`, `name`, `constructor`,
`@@toStringTag` and nested `%X%` owners other than `%TypedArray%`), the 172 globals, and the `Object.keys` of each
module under node. A pair `id#mode` is applicable unless it is `json` mode on a non-JSON receiver owner.
Touched means a probe exists for the pair; it does not mean the probe passes.

Ratchet: every untouched applicable pair must be in `coverage-baseline.json`. A new table row, a removed probe
or a new module export adds an untouched pair and exits 1. Pairs that became touched are listed and can be
dropped with `coverage.py --manifest M --write-baseline` (only when the baseline should shrink or a change knowingly
adds surface). The ratchet is skipped with a warning when node is not major 22.

Output: `coverage.json` (universe sizes, per-mode and per-family cells, untouched lists) in the OUT directory, and a
markdown table (families by modes) appended to `summary.md`. `run.sh` runs it after the probes and never fails on it.

## Run

    tools/parity-aot/selftest.sh                 # no thaw build; determinism, node run, classifier checks
    THAW=/path/to/thaw tools/parity-aot/run.sh [OUT_DIR]
    tools/parity-aot/run.sh --write-baseline     # records current gaps into known-gaps.json (after a fix only)

Build command per program and engine (`THAW_JS_ENGINE` is `thaw-js` or `quickjs`):

    THAW_JS_ENGINE=<engine> thaw build <program>.ts -o <binary>      (cwd: the program's work dir)

Env: `THAW` (default `<repo>/target/release/thaw`), `NODE`, `PARITY_AOT_ENGINES` (default `thaw-js,quickjs`),
`PARITY_AOT_JOBS`, `PARITY_AOT_BUILD_WRAP` (command prefix for each build, e.g. a systemd-run memory cap),
`PARITY_AOT_FAMILIES` (comma list; generate only those families, skips the coverage ratchet), `PARITY_AOT_RUN_TIMEOUT` (20 s), `PARITY_AOT_BUILD_TIMEOUT` (900 s), `PARITY_AOT_SCRATCH` (default
`/home/fedora/work/thaw-tmp/parity-aot-scratch`), `PARITY_AOT_OUT`.

Output (OUT_DIR): `summary.md` / `summary.json` (counts per corpus and class, top findings, engine delta),
`<engine>/gaps.json`, `<engine>/diff.txt`, `<engine>/classes.json`, `node/*.out`, `programs/`.

Exit: 0 when the harness ran (gaps are reported only). 2 on a harness error (node failure, missing header,
missing thaw binary, diff.py harness error). `diff.py` exit 1 means gaps not in `known-gaps.json`; the run still exits 0.

## CI

`.github/workflows/parity-aot.yml` runs on `workflow_dispatch` or when the `parity-aot` label is added to a PR.
Job `build` builds the release thaw CLI, runs `selftest.sh`, writes the API coverage (full manifest, no thaw),
and uploads the binary as `thaw-release-bin`. Job `shards` (matrix s1 to s4, split by family) downloads the binary and
runs both engines per shard, writing `parity-aot-results-<shard>` artifacts. Both jobs are `continue-on-error`.

## Module surface

`modules.txt` (the 57 thaw-js builtin modules) is inventoried by `module_inv.js` under Node 22 into
`module_table.tsv` (module, export, kind, arity, vtype, alias; kinds function, class, constant, primitive, object,
accessor). `check_module_table.sh` regenerates it and fails on a format error or a drifted row; rows that exist
in only one of the two (version-dependent exports) print WARN. Regenerate with
`node tools/parity-aot/module_inv.js tools/parity-aot/module_table.tsv`.

`dts_parse.py` parses the ambient d.ts of each module (dumped by `crates/thaw-registry/tests/dump_builtin_dts.rs`,
run with `THAW_DUMP_DTS_DIR=<dir> cargo test -p thaw-registry --test dump_builtin_dts -- --ignored`) into export
signatures: names, kind, overloads, parameter and return types. Fragments it cannot parse are counted and listed,
never dropped. `selftest_parse.sh [DUMP_DIR]` runs its inline fixtures and, given a dump directory, parses all of it.

`gen_modules.py` generates the module probes (shapes 1-3: sync functions, constants/primitives/objects/accessors,
classes with construct, methods, static methods and properties). It needs the d.ts dump:

    THAW_DUMP_DTS_DIR=<dir> cargo test -p thaw-registry --test dump_builtin_dts -- --ignored
    PARITY_AOT_DTS_DIR=<dir> python3 tools/parity-aot/gen_modules.py <OUT_DIR>      # or --dts <dir>

Types come only from the d.ts. Arguments come from the fixed POOL in the script (one expression per TS type; an
unpoolable type is a skip, never a guess). Functions with a callback parameter or a Promise return are deferred
to shapes 4-6 (gen_async.py) and console output to the inspect family. Non-deterministic exports (host state,
clock, randomness, stream handles with uninitialised memory) still run, but print only `[typeof, length or key
count]` (the `shape` rule table in the script, with a reason per rule). Modes: `typed` and `any`.

Output: `mod_<module>.<mode>.cNN.ts` / `.mts` (one module per program, at most 40 probes, the gen_probes `@@` format),
`manifest.json` (gen_probes schema, plus `imports` per program and `arg_types` per probe), `skips.json`
(`exports`: id, module, export, category, reason; `members`: member-level skips; `dts_status`; `unused_rules`).
The categories are in `CATEGORIES` in the script. The stdout summary is a per-module table of exports probed/skipped
by kind. Probe ids are `mod:<module without node:>.<export>`, the coverage.py universe (`Object.keys` of the module);
a table row outside that list is NOT-EXPORT, a listed export without a table row is NOT-IN-TABLE.

Not yet wired into run.sh or gen_probes.py. aot_run.py renders each thaw program with gen_probes.render, which has no
`import * as M` line, so a module program needs aot_run to render `prog["imports"]` before PRELUDE. Until then the
thaw build of every module program fails, and the programs are for node runs and coverage only.

`testdata/dts/` holds seven d.ts (node:path, events, os, querystring, string_decoder, buffer, url) for selftest only,
copied from a dump. Other modules are NO-DTS-FILE skips in that run. `selftest_gen.sh` checks determinism, the
universe accounting (every Object.keys id probed or skipped), the argument pool, valid TypeScript and node runs of
the programs, and the coverage of module ids.
