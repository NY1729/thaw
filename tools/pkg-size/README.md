# pkg-size

What an imported npm package costs: for each of the 24 corpus packages (`tools/corpus/corpus/versions.txt`),
`measure.sh` builds an import-only program (`import "<pkg>"; console.log("ok");`) and records one TSV row.
A `_hello` row (no import) is the floor.

Columns: `build_ok`, `build_first_ms` (first build: registry bundling, new runtime variants),
`build_warm_ms` (second build, everything cached: compare this one), `size_bytes`, `start_ms` (median of
`MEASURE_RUNS`, default 10), `max_rss_kb`.

    cargo build --release -p thaw-cli
    tools/pkg-size/measure.sh -o /tmp/pkg-size.tsv [--only NAME]

`baseline.tsv` was recorded on one machine (release build of this commit); build times depend on the host and
on whether the runtime variant archives were already built, so compare sizes and RSS strictly and times loosely.
`.github/workflows/pkg-size.yml` runs it on `workflow_dispatch` or on a PR with the `thaw-js-measure` label;
it is non-gating and only prints the table next to the baseline.
