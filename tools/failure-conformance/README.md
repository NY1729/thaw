# tools/failure-conformance: process-level failure behaviour, thaw vs Node

Node 22 is the oracle. Small programs are compiled with `thaw build` (the AOT executable; modules that thaw routes to
thaw-js, such as `worker_threads`, take that path automatically) and run next to `node prog.mts`. Per case the harness
compares stdout, stderr (normalised), the exit code and the terminating signal. Non-gating: gaps are reported, never failed on.

Motivation: a compiled app printed `Uncaught: Uncaught opaque exception` instead of Node's error output. This suite
looks for the whole class: what a process prints and returns when something goes wrong.

## Files

- `cases.py`: the case table. `C(group, id, body)` adds one case; the body is TypeScript. Edit this to add probes.
- `gen.py`: renders the table into programs (`<group>.<n>.mts` for Node, the same text as `.ts` for thaw), at most 40
  cases per program in one `switch`. The case id is the last argv entry, so every case is its own process run.
- `run.py`: Node stage, thaw build stage, thaw run stage, classification, reports. `run.sh` is the entry point.
- `selftest.sh`: no-thaw checks (generation determinism, unique ids, Node runs every case twice with the same result,
  normaliser and classifier unit checks including the `Uncaught opaque exception` family).
- `known-gaps.json`: the expected non-MATCH cases (`{"id", "kind"}`), same shape as `tools/parity`. `run.sh --write-baseline`
  rewrites it from the current run; it may only shrink as fixes land.

## Running

    THAW=target/release/thaw tools/failure-conformance/run.sh [OUT_DIR] [--only SUBSTR] [--write-baseline]

Environment: `NODE`, `FC_BUILD_BUDGET` (max builds per program spent isolating build failures, 0 = unlimited; a local run behind a contended build lock may cap it, the remaining cases then read `not isolated`), `FC_JOBS` (default 3), `FC_RUN_TIMEOUT` (20 s), `FC_BUILD_TIMEOUT` (900 s), `FC_RLIMIT_AS` (8 GiB, applied
to thaw binaries through `prlimit`), `FC_BUILD_WRAP` (command prefix for each `thaw build`, e.g. a `systemd-run` slice).
Output in OUT_DIR: `summary.md` (counts, failure families, per group), `report.tsv` (one row per case), `results/<id>.json`
(raw and normalised streams for both sides).

A group build that fails is narrowed (source offsets in the diagnostic, then the cases that contain the quoted construct, then bisection), so only the cases that fail to compile alone are BUILD-FAIL.

## Classes

- MATCH: stdout, normalised stderr and exit status/signal all equal.
- DIFF: they differ; `detail` says which. `family` clusters them: `opaque-exception` (thaw printed `Uncaught opaque
  exception`), `no-error-output` (Node wrote to stderr, thaw nothing), `exit-status`, `stderr-text`, `stdout-text`.
- BUILD-FAIL: `thaw build` rejected the program (detail is the first diagnostic line).
- RUN-FAIL: thaw hung, or died of SIGSEGV/SIGABRT/SIGBUS/SIGILL/SIGFPE where Node did not.
- ORACLE-FAIL: Node hung (harness problem).

Status column: `known` (non-MATCH and in known-gaps), `NEW` (non-MATCH, not in known-gaps), `FIXED` (MATCH but listed).

## Normalisation (run.py `norm`)

Removed: program/work paths, `:line:col`, `    at ...` frames (frame fidelity belongs to the stack tests; a trailing ` {`
of the last frame is kept), Node's uncaught-error source excerpt and caret, the `Node.js vX` banner, the `(Use
\`node --trace-uncaught\`)` hint, ports in `127.0.0.1:N` / `:::N`, `(node:PID)`. A stack-less error that util.inspect prints
as `[Error: m]` compares as `Error: m`. Output longer than 8 KiB compares by length and sha256. Everything else, message
text and own properties included, is compared verbatim.

## Case modes

`run` (default); `signal:SIGX` (the program prints `READY`, the harness waits, sends the signal, records exit
status or signal); `epipe` (the harness reads 1 byte of stdout and closes the pipe, like `| head -c 1`); `merge=True`
(stderr into the stdout pipe, to compare write ordering).

## Groups

`uncaught` (sync throw of every value kind), `async` (throws in timers, microtasks, promises, emitters, streams),
`handlers` (uncaughtException/unhandledRejection/exit/beforeExit, exit codes), `signals`, `stdio` (flush, ordering,
EPIPE), `inspect` (console.error/util.inspect of errors), `syserr` (fs/JSON/URL error shapes), `messages` (V8 message
texts), `limits` (stack overflow, huge sizes), `assert` (node:assert output), `worker`, `idioms` (http/child_process/fetch).

CI: `.github/workflows/failure-conformance.yml`, non-gating, runs on a PR labelled `thaw-js-measure` or on dispatch.

Builds and `.diag` files are cached by program text under OUT_DIR/thaw; pass `--clean` after the thaw binary changes.

The committed known-gaps.json was recorded locally against a release thaw built from this branch's base (553 cases).
