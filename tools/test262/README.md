# test262 conformance ratchet for thaw-js

Runs the tc39/test262 suite (`test/built-ins`, `test/language`, `test/annexB`; staging and intl402 are out of scope)
against `thaw-js-node` and fails if a test that passed in the committed baseline now fails.

| file | purpose |
|---|---|
| `TEST262_COMMIT` | pinned test262 commit; `fetch.sh DIR` fetches exactly that sha (depth 1) |
| `run.py` | runner: per-test jsonl `{path,status,message}`, status pass/fail/crash/timeout/skip; resumable |
| `shards.json` | disjoint directory shards (used by CI and by `run-shard.sh`) |
| `run-shard.sh` | `run-shard.sh SHARD|all OUT.jsonl [jobs]` |
| `compare.py` | ratchet: regressions, improvements, per-area deltas, `--update-baseline` |
| `baseline.json` | engine commit, per-area (two levels, e.g. `language/expressions`) pass/fail/skip counts, totals |
| `failing.txt.gz` | every failing test id of the baseline (~7k ids, ~60 KB gzipped; a pass list would be 5x larger) |

## Run locally

    cargo build --release -p thaw-js --bin thaw-js-node
    bash tools/test262/fetch.sh /path/to/test262
    export TEST262_DIR=/path/to/test262 THAW_JS_NODE=$PWD/target/release/thaw-js-node
    bash tools/test262/run-shard.sh lang-statements /tmp/r.jsonl 3     # or `all` (~1 h at 3 jobs)
    python3 -I tools/test262/compare.py /tmp/r.jsonl                   # exit 1 on regression
    python3 -I tools/test262/compare.py /tmp/r.jsonl --update-baseline # after a deliberate change; commit both files

`run.py --dirs test/built-ins/Array --out x.jsonl` runs any directory list. `compare.py` only compares the areas
present in the results, so partial runs are fine, and `--update-baseline` only rewrites those areas.

## Method

Per test: harness prelude (`print`, a `$262` stub with a native-Script `evalScript` (`__thaw_eval_script`)/`detachArrayBuffer`/`gc`), `assert.js`, `sta.js`
(+ `doneprintHandle.js` for async), the `includes`, then the test. Default flags run sloppy then strict, both must pass;
`raw`, `onlyStrict`, `noStrict` are honoured. Negative tests pass if the engine exits non-zero (parse phase) or prints
the expected error type. 20 s wall timeout and a 2 GB `RLIMIT_AS` per engine process. Skipped (counted as `skip`):
module flag, and features Temporal, Intl, ShadowRealm, Atomics/SharedArrayBuffer/agent, resizable-arraybuffer,
cross-realm, IsHTMLDDA, WeakRef/FinalizationRegistry, import-attributes, source-phase-imports, json-modules,
tail-call-optimization (see `SKIP` in `run.py`). The engine is built with default features (no `intl`).

Flaky tolerance: `compare.py` re-runs every would-be regression once and drops those that pass the second time.

## CI

`.github/workflows/test262.yml`: label `thaw-js-measure` or manual dispatch; one build job, one matrix job per shard,
a failing shard fails the `ratchet` aggregate job (the required check), details in the job summary. To bump test262, edit `TEST262_COMMIT` and regenerate
the baseline with `--update-baseline` in the same change.
