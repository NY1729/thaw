# JIT coverage (stage 0, PR 1)

Question: how much of the interpreter time could a JIT or a JS->HIR clone absorb?

For each of the 24 corpus programs (`tools/quickjs-removal/corpus/pk`), the workflow builds the program
twice with the thaw CLI and profiles both binaries:

- `default`: as shipped (thaw-js is the default engine).
- `infer`: `THAW_JS_INFER=1` at build time, which adds AOT clones from the JS->HIR inference.

Each binary is timed over `MEASURE_RUNS` rounds, then run once more with `THAW_JS_PROFILE=1 THAW_JIT_PROFILE=1`.
The counters give, per entry key (`pkg::export` for a typed call, `entry:<pkg>@<file>:<line>` for an interpreter
entry from AOT code) the call count and time, plus the total interpreter time and the wall time of the run.
Interpreter share is `interp_ms / wall_ms` of that total line.

## Files

- `collect.sh OUT_DIR`: fetch (`thaw add`), Node reference, both builds, timed rounds, profiled run. Its fetch
  and comparison steps are those of `tools/quickjs-removal/measure.sh`; it is a separate script so the shared
  harness is not changed.
- `run.py LOGS_DIR [--out-dir DIR]`: stdlib only. Writes `jit-coverage.tsv` (one row per program and variant)
  and `jit-coverage.md` (per-program table and corpus-level numbers), and prints the Markdown.
- `test_run.py`: unit tests on the fixtures in `testdata/` (the yup default profile is the real output of the
  counters PR; the other fixtures are hand-written in the same formats).
- `.github/workflows/jit-coverage.yml`: non-gating job that runs both on demand.

## Stop rule

From `designs/jit-coverage-stage0.md`: stop widening the JIT if under 20% of in-loop calls are absorbable on at
least 18 programs. This tool measures interpreter share only. Absorbability needs the static classification of
the hot calls, which is a later step.

The "programs with zero entry rows" count is over the whole profiled run: a program with no `entry:` row made no
interpreter entry from AOT code at all.

## Run it

Build the CLI and fetch the packages (needs network and LLVM 22, as in the workflow):

    cargo build --release -p thaw-cli
    THAW=$PWD/target/release/thaw SCRATCH=/path/to/scratch ONLY=yup MEASURE_RUNS=3 \
      bash tools/jit-coverage/collect.sh /path/to/logs

Then summarise (no network or thaw needed):

    python3 -I tools/jit-coverage/run.py /path/to/logs --out-dir /path/to/out

Selftest:

    cd tools/jit-coverage && python3 -m unittest

In CI the workflow runs on `workflow_dispatch` (inputs `runs`, `only`) or on a pull request labelled
`jit-coverage-measure`. Results are in the step summary and in the `jit-coverage-tsv` and `jit-coverage-logs`
artifacts.
