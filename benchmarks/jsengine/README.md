# JS engine microbenchmarks

Ten small deterministic scripts (fib, loop sum, object/array/string/JSON, class and closure calls,
regexp, Map/Set). Each prints one checksum line so a run can be verified.

```
benchmarks/jsengine/run.sh [-n RUNS] [-o OUT.tsv]
```

- Builds `thaw-js-node` in release unless `THAW_JS_RUN` points at a prebuilt runner. It runs plain
  scripts like `node` does; `thaw-js-run` only loads registry bundles.
- `QJS=/path/to/qjs` adds the QuickJS reference column. Unset, the column is `-`.
- Times are best-of-`RUNS` (default 3) wall-clock milliseconds per script.
- Output is TSV `benchmark  thaw_js_ms  quickjs_ms  ratio`, then a `geomean` row (thaw-js / QuickJS).
- A checksum mismatch between the two engines is reported on stderr.
