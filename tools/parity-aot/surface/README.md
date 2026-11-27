# surface: Node 22 vs thaw AOT, member-level API matrix

Finds the API surface Node has and thaw's AOT lacks, mechanically. Not in CI; run by hand. Builds on `../aot_run.py`
(Node oracle, `thaw build`, bisect of rejects to the probe, classes) and `../gen_probes.py` (prelude, CTOR_ARGS).
Differs from `../gen_modules.py`: that one probes what thaw's own d.ts declares, so it cannot see a member thaw is
missing; this one is driven by what real Node has.

- `inv.js OUT [module...]`: Node 22 inventory, one JSON per module under OUT (all public `node:*` and `globals.json`):
  exports, class statics, prototype chain members, instance own properties (from a real instance), plain-object props
  (depth 2). Entry: name, kind (function/class/getter/object/value), typeof, arity.
- `recipes.json`: how user code reaches an instance with the type thaw infers (e.g. `createServer((req, res) => ...)`).
  An annotated parameter (`o: IncomingMessage`) is NOT checked by thaw, so it would hide the gaps; the recipe is.
- `gen_surface.py INV OUT [module...]`: probes `exists`, `arity`, `call` (safe modules only), `new`, `static.*`,
  `proto.*`, `inst.*`, `prop.*`. Classes needing ctor args with no recipe are skipped (skips.json).
- `report.py`: `surface.md` / `surface.json`: per module match / build-fail / output-diff / crash / skipped, top
  build-fail causes, failing members. match = MATCH+MSG; build-fail = REJECT; output-diff = WRONG-*/LEAK/MISSING.
- `baseline-counts.json`: per-module counts; the report prints REGRESSION when `match` drops. Refresh with `WRITE_BASELINE=1`.

Run: `tools/parity-aot/surface/run.sh http stream globals` (no args: every module). Env: `THAW`, `OUT` (default
thaw-tmp/surface-out), `JOBS` (2), `WRAP` (systemd-run heavytests slice, 10G), `INV` (reuse an inventory).
Committed inventory (all modules + globals, Node 22.23): `tools/parity-aot/inventory/`; regenerate with `node surface/inv.js ../inventory`.
Cache: `PARITY_AOT_CACHE` (default `$OUT/cache`), key = sha256(thaw binary, program text); a rerun with the same binary
does no builds. Build-fail causes show the first diagnostic line; the probe id says which member.
@types/node cross-check is not done (inventory is Node-only).

Declaration drift (not probes, run against the committed inventory; no thaw build needed):
- `arity_sync.py [--fix]`: declared `Function.length` of every registry d.ts function vs Node's (inventory). thaw's length is the
  number of non-rest parameters of the first overload, so the d.ts parameter lists must be shaped to Node's. `--fix` rewrites
  `resolution.rs`; callback-typed and overloaded signatures are only reported (an extra parameter breaks the native lowering /
  overload resolution; `f(a)` + `f(a, b)` pairs with one return type are merged into `f(a, b?)`).
- `gen_data.py [--check]`: Node's own constant tables (os/http2/zlib/perf_hooks/crypto/module constants, async_hooks.asyncWrapProviders,
  dns codes, process.config) -> `crates/thaw-registry/src/registry/builtins/surface_data.rs`, merged into `module.exports` by
  `builtins.rs`. Needs the pinned Node 22.23.
- `unsupported.json`: what is deliberately not implemented, with the reason.
