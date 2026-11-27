# Node's own test suite as a Node-vs-thaw(AOT) yardstick

Runs eligible tests of `nodejs/node` `test/parallel` (pinned in `TEST_COMMIT`, tag v22.23.1) as `thaw build` programs and
ratchets the per-module pass counts, like `tools/test262`. Manual only, not wired into CI.

| file | purpose |
|---|---|
| `TEST_COMMIT` | pinned nodejs/node commit; `fetch.sh DIR` sparse-fetches `test/parallel`, `test/common`, `test/fixtures` (88 MB) |
| `run.py` | `select` (eligibility) and `run` (build + run, cached by thaw-binary hash + shim + test source) |
| `shim/` | ESM/TS ports of `test/common` (`index.ts`, `countdown.ts`, `fixtures.ts`), same contract as upstream |
| `eligible.txt` / `skip.txt` | selected tests / every excluded test with its reason (counts in the `#` header) |
| `shards.json`, `run-shard.sh` | prefix shards (`http`, `stream`, `buffer`, `whatwg-fetch`, `events`, `path-url`) |
| `compare.py`, `baseline.json`, `failing.txt.gz` | ratchet: regressions = a test not in the baseline failing list that now fails |

## Run

    bash tools/node-tests/fetch.sh ~/.cache/node-tests-src
    export NODE_TESTS_DIR=~/.cache/node-tests-src NODE_TESTS_WORK=/path/to/scratch THAW=/path/to/thaw
    python3 -I tools/node-tests/run.py select --jobs 8          # only after a TEST_COMMIT bump or a transform/shim change
    bash tools/node-tests/run-shard.sh all /tmp/r.jsonl 3       # build is heavy: use the heavytests slice
    python3 -I tools/node-tests/compare.py /tmp/r.jsonl [--update-baseline]

## Method

thaw does not support CommonJS `require`, so each test is mechanically rewritten to ESM (top-level `const x = require('m')`,
`const {a} = require('m')`, bare `require('m')` become imports; `__filename`/`__dirname` are provided) and run as
`WORK/test/parallel/<name>.js` (`"type":"module"`), with `../common` resolving to `shim/`. Eligibility (`run.py select`):
upstream source passes twice standalone on Node 22, no `// Flags:`, no internals, child processes, cluster/worker, tls/crypto,
external network, CJS-only APIs, un-shimmed `common.*`, and the transformed ESM version also passes twice on Node.
Statuses: pass / build-fail (normalised `thaw build` error) / runtime-fail (first assertion line) / crash (signal) / timeout.

## Shim limits (AOT)

The shim keeps upstream function bodies; deviations forced by the AOT compiler are marked `ponytail:` in `shim/index.ts`:
`mustCall(fn, n)` needs a function first argument (`mustCall()`/`mustCall(n)` build-fail), the wrapper does not forward
`this` and does not copy `name`/`length`, and `expectWarning` is omitted (its users are excluded).
