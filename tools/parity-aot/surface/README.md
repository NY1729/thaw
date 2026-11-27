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

## Optional-argument probes (omit / undef / null)

Run: `ARGS=1 tools/parity-aot/surface/run.sh fs path ...` (report `$OUT/args.md`; counts baseline `args-counts.json`, separate
from `baseline-counts.json`). Calls every function and method through `any` (the path a JS-side adapter takes) with
arguments from `optional-args.json`, and compares what Node and thaw return (kind) or throw (`name[code]`):
- `omit.<k>`: first k of the n arguments (k = n-1 .. n-4);  `undef.<i>` / `null.<i>`: argument i replaced.
- `gen_optional_args.py --inv inventory --types <@types/node dir> --out optional-args.json [--md]` builds the table (per
  function: `Function.length`, @types/node `?` per overload, `omit_from`, sample `base_args`). Get @types/node with
  `npm install --ignore-scripts @types/node@22` in a fresh empty directory.
- Sandbox: cwd-relative `sb/` (file `sb/f`, dir `sb/d`) is recreated before every probe; hosts/urls are loopback.
  `args-excludes.json` lists the only functions never called (process exit/kill, spawn, DNS, ...), each with its reason.
- Boundary with the wrong-type probes (r145): these probes only pass `undefined`/`null`/fewer arguments.
