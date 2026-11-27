# tools/wire-diff: raw-bytes node:http differential test (Node vs thaw AOT)

Node 22 is the oracle. A scene is (server handler, client request). The generator samples scenes from the product of:

- response status: 100 101 200 201 204 301 304 400 404 500
- header style: `writeHead(s)`, `writeHead(s,obj)`, `writeHead(s,msg,obj)`, `setHeader`+`writeHead`, `setHeader` only,
  name-case variants, array values, repeated `setHeader`, explicit/wrong `Content-Length`, explicit `Transfer-Encoding`
- body: `end()`, `end(data)`, `end(Buffer)`, `write`+`end`, several `write`s (ended later), `end(undefined)`, `end('')`, 100 KB, non-ASCII
- request: method (GET HEAD POST PUT DELETE OPTIONS), connection (keep-alive, close, HTTP/1.0, HTTP/1.0 keep-alive,
  2 pipelined requests), request body (none, Content-Length, chunked), header name case (normal, lower, UPPER, duplicate)

All handlers of a run are routes (`/s<N>`) of ONE program, so one `thaw build` covers all scenes. If the build fails it is
bisected until the failing scene is isolated (BUILD-FAIL with the compiler diagnostic). The same program runs under Node
(`--experimental-strip-types`) and as the thaw binary; a raw socket sends the request bytes and reads the whole response.
Only the values of `Date` and `Server` are masked. Header order, name case and whitespace are compared.

Classes: `SAME`, `DIFF` (line diff and a cause signature, e.g. `extra-header:transfer-encoding`, `header-order`),
`BUILD-FAIL`, `HANG` (thaw returned no bytes), `NODE-NO-RESPONSE`.

## Run

    THAW=/path/to/thaw tools/wire-diff/run.sh [--scenes N] [--seed S] [--write-baseline] [--label L]

`build` runs under the `app-heavytests` slice (10G cap); set `WIRE_BUILD_WRAP` to override (e.g. prefix `cargo_slot.sh`)
and `CARGO_TARGET_DIR` to reuse a target. Work files and the cache live in `$WIRE_WORK`
(default `/home/fedora/work/thaw-tmp/wire-diff-work`). The cache key is thaw binary hash + program text + scenes: a rerun
with nothing changed does not build or run. The report is `$WIRE_WORK/report-<label>-<N>-<seed>.md`: counts, DIFF groups by
cause signature with per-dimension concentration (`value:count/total`) and example line diffs.

`baseline.json` records scenes, seed and the SAME/DIFF counts; a run with the same scenes+seed prints `REGRESSION`
when SAME drops. Not part of CI.

## Request side and client side

- `python3 wire_req.py`: 48 raw requests (bad request lines, duplicate headers, huge headers, chunked bodies, Expect, Upgrade,
  pipelines, no Host, ...) against one echo server. Baseline: `baseline-req.json`.
- `python3 wire_client.py`: the bytes `http.request`/`http.get` send (26 variants) into a raw socket. The Host port is masked.
  Baseline: `baseline-client.json`.
- Rebuild the thaw binary after merging main: a stale binary builds programs against a newer std and crashes at startup.
