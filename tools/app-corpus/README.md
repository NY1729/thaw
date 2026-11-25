# Application-style program corpus

Small probes pass while whole programs fail: a build script with `URL`-object fs paths, a static file server with `res.writeHead(405).end()`. This corpus holds realistic, self-contained Node-style programs (`programs/*.mjs`, a few `*.ts`) and compares each, differentially, against real Node 22.

Per program, in fresh temp dirs (stdin `alpha beta\ngamma\n`, `TZ=UTC LANG=C NO_COLOR=1`):

- Node runs the file: stdout, stderr, exit code, post-run directory tree (paths, sizes, sha1; the program file, `node_modules`, `package*.json` excluded).
- `thaw build <file> -o bin` (one build per program), then the binary runs with the same inputs, under `RLIMIT_AS` (`APP_CORPUS_RUN_AS_MB`, default 8192) and a timeout.
- Output is normalised: run dirs become `<DIR>`, `host:port` becomes `host:<PORT>`, stack frames (`    at ...`) and the `Node.js vX` line are dropped.

Classes: `MATCH`, `BUILD-FAIL` (first error line captured), `RUN-FAIL` (thaw exits nonzero or hangs where Node succeeds), `DIFF` (output, exit code or tree differs), `NODE-FAIL` (harness problem: Node itself timed out; exit 2).

## Run

```
tools/app-corpus/run.sh [--only a,b] [--shard 1/2] [--write-known]
```

Env: `THAW` (default `target/release/thaw`), `NODE`, `APP_CORPUS_OUT` (report.tsv, summary.txt, logs/), `APP_CORPUS_BUILD_WRAP` (prefix for `thaw build`/`thaw add`, e.g. a `flock ... systemd-run` memory cap), `APP_CORPUS_BUILD_TIMEOUT`, `APP_CORPUS_RUN_TIMEOUT`.

Outputs: `report.tsv` (program, class, detail, build_s, run_s) and `summary.txt` (counts, failure families, NEW failures and FIXED programs relative to `known-gaps.tsv`). `tools/app-corpus/selftest.sh` checks the driver against a fake `thaw`.

## known-gaps.tsv

`program<TAB>class<TAB>first error line` for every program that currently fails. Non-gating: the workflow (`app-corpus.yml`, label `thaw-js-measure`) reports NEW failures, FIXED programs and the BUILD-FAIL count in the step summary. Refresh with `run.sh --write-known` when a fix lands (then drop fixed rows).

## Adding a program

Deterministic, no external network, node builtins only; ports are `0` and requests go to itself. A program that needs npm packages declares them in its first lines, `// pkgs: semver@7.8.5 @types/semver@7.8.0` (installed with `thaw add --prefix .`; pins as in `tools/corpus/corpus/versions.txt`) and is a `.ts` file. `.ts` programs must run under Node's type stripping (no enums).
