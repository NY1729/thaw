# aotdiff: AOT compiler vs Node differential probes

Compiles small TypeScript probe programs with `thaw build`, runs them, and
compares each probe's output with the same file run under `node` (the oracle).
By default it only measures. `--check` gates shrink-only against
`known-gaps.json` (see Baseline). CI wiring is not part of this tool yet.

## Run

```sh
python3 tools/aotdiff/run.py                      # all families
python3 tools/aotdiff/run.py --family numeric,json-date
python3 tools/aotdiff/run.py --batch 40 --jobs 2  # defaults
python3 tools/aotdiff/run.py --check                # shrink-only gate vs known-gaps.json
python3 tools/aotdiff/run.py --update-baseline --thaw PATH   # regenerate the baseline
python3 -m unittest discover -s tools/aotdiff -p 'test_*.py'   # pure self-test, no thaw/node
```

`--check` and `--update-baseline` are exclusive. `--baseline PATH` picks the
file (default `tools/aotdiff/known-gaps.json`). `--update-baseline` refuses the
stale fallback binary unless `--thaw` or `$THAW` is given.

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
| COMPILE-REJECT | thaw refused to compile this probe (typed-subset restriction, explicit error). Reason = first `error:` line. Expected for dynamic idioms; tracked, not a correctness gap. Recorded in the baseline as class REJECT, never gating |
| MISMATCH | compiled and ran, but output or exit code differs from node. The only correctness gap class; the only one `--check` gates |

A MISMATCH carries a kind, so wording-only gaps can be told apart from type gaps:

| Kind | Meaning |
|---|---|
| TYPE | error constructor name differs (or only one side threw) |
| MSG | same error constructor, message wording differs |
| OUTPUT | ordinary output line differs |
| EXIT | thaw exited non-zero |
| NO-OUTPUT | thaw printed no line for the probe |

## Output normalisation

Applied to both the node and the thaw block of each probe before comparing
(`NORMALISERS` in `run.py`):

| Normaliser | Rewrites | Why |
|---|---|---|
| trailing whitespace, CRLF, trailing blank lines | dropped | formatting |
| stack-shape | after the header line, every `    at ...` frame becomes `    at <frame>`; consecutive repeats collapse | frame text differs by build |
| abs-path | `/a/b/...` (two or more segments) -> `<path>` | build dir is scratch |
| null-proto | `[Object: null prototype] ` prefix dropped | node prints it, thaw prints plain `{..}`: formatting, not a gap |
| timing | `12.5ms` -> `<ms>` | elapsed time |
| address | `0x7f3a2b1c4d` (6+ hex digits) -> `<addr>` | object addresses |

Not normalised, on purpose: `__thaw_js_handle_id__` (a leaked handle id is a
bug), error messages (their wording is part of compat; split by kind instead).
Exit code must match (node exits 0).

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

## Baseline: known-gaps.json

`tools/aotdiff/known-gaps.json` is the committed baseline. It is **not** in
this PR. It is generated by a documented command against a current build,
to be run in CI later:

```sh
python3 tools/aotdiff/run.py --thaw <current release thaw> --update-baseline
```

Never generate it from a stale binary. The fallback path is refused.
`--update-baseline` keeps entries of families it did not run, and keeps a
waiver while its probe stays in the same class.

Schema (version 1):

```json
{
  "schema": 1,
  "thaw_version": "...",
  "node_version": "...",
  "gaps": {
    "numeric/bigCmp": {"class": "MISMATCH", "kind": "OUTPUT", "reason": "output differs",
                        "waiver": {"id": "W1", "reason": "by design: ..."}},
    "numeric/big1":   {"class": "REJECT", "reason": "build failed"}
  }
}
```

- key: `<family>/<probe>`.
- `class`: `MISMATCH` or `REJECT`. `kind` is required for MISMATCH.
- `reason`: non-empty.
- `waiver` (optional): `{"id": "W<n>", "reason": "..."}`. A waived MISMATCH
  is counted as waived, not open. Waivers are for by-design restrictions and
  are written by hand, then reviewed. A waiver never gates.

## Check mode (`--check`)

Shrink-only. Exit 1 only when a probe is a MISMATCH that the baseline does not
list as a MISMATCH. Otherwise exit 0:

- REJECT is reported as a count (new REJECTs do not gate).
- A baseline gap that now matches, or changed class, or whose probe is gone, is
  printed as `fixed since baseline` and passes. Remove it with `--update-baseline`.
- Families not run are ignored.
- A missing baseline is an error (exit 2).

## Not in this PR

- No CI job (PR4).
- No real baseline file (generated in CI against a current build).
