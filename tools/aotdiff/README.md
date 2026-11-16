# aotdiff: AOT compiler vs Node differential probes

Compiles small TypeScript probe programs with `thaw build`, runs them, and
compares each probe's output with the same file run under `node` (the oracle).
Non-gating. It measures; it does not block CI.

## Run

```sh
python3 tools/aotdiff/run.py                      # all families
python3 tools/aotdiff/run.py --family numeric,json-date
python3 tools/aotdiff/run.py --batch 40 --jobs 2  # defaults
python3 -m unittest discover -s tools/aotdiff -p 'test_*.py'   # pure self-test, no thaw/node
```

Thaw binary: `--thaw PATH`, else `$THAW`, else `target/release/thaw` in the
repo, else a last-resort fallback path (see `FALLBACK_THAW` in `run.py`).
The report prints the thaw and node versions it used.

Report: `target/aotdiff/report.json` (or `--out`), with per-family counts and
every probe's outcome, node output and thaw output.

Scratch goes to `$TMPDIR` (python `tempfile`). Set `TMPDIR` to a directory
outside `/tmp` if needed. `--keep` keeps the work dir.

## Outcomes

| Outcome | Meaning |
|---|---|
| MATCH | compiled, ran, and the probe's output lines equal node's |
| COMPILE-REJECT | thaw refused to compile this probe (typed-subset restriction, explicit error). Reason = first `error:` line. Expected for dynamic idioms; tracked, not a correctness gap |
| MISMATCH | compiled and ran, but output or exit code differs from node. The only correctness gap class |

## How a run works

1. Each `probes/<family>.ts` is parsed: every probe is one line
   `  p("name", () => expr);` inside `function main(): void { ... }`.
2. Probes are grouped into batches of 40. Each batch is written as one program
   (family header + `main()` with the batch's probes) and built once.
3. If a batch fails to build, it is bisected by halves until each failing probe
   is alone. A probe that fails alone is COMPILE-REJECT.
4. Each built batch is run once. If it exits non-zero, each of its probes is
   rebuilt and rerun alone, so the crashing probe is attributed correctly.
5. Each family is also run once under node, with `main();` appended. Node
   output is split per probe by name and compared line-for-line.

Commands run under `timeout`, with `ulimit -v 8388608` for thaw, and under
`systemd-run --user --slice=app-heavytests.slice -p MemoryMax=8G` when
available (`--no-systemd` to disable). The node oracle is exempt from the
`ulimit`: node's type stripper (WASM) cannot start under an 8 GB address-space
cap.

Output normalisation is minimal: trailing whitespace and CRLF are dropped, and
trailing blank lines are removed. Exit code must match (node exits 0).

## Probe files

- `probes/<family>.ts`: one family per file, header helpers `P`, `Q`, `id(v: any)`, and
  `p(name, f)`. Probes are written in thaw's typed subset. `id()` launders a
  value through `any` to reach the dynamic path.
- A probe line must be complete on one line, and names must be unique across
  the family. The loader rejects anything else.
- Never add a `main();` call to a probe file. thaw auto-invokes `main()`; the
  node copy gets `main();` appended by the runner.

Current seed families (PR1, 191 probes): numeric, string-regexp, json-date,
any-control, class-closure, errors-collections. Async ordering probes are not
included yet (they need their own files, planned for PR3).

## Not in this PR

- No baseline and no gating (`known-gaps.json` and shrink-only checking are PR2).
- No CI job (PR4).
- No normalisation beyond whitespace; message-wording and stack-shape
  normalisation come with the baseline.
