#!/usr/bin/env python3
"""Differential driver: run each case in cases.src.json on real Node and on thaw-js.

usage: run_diff.py <corpus-dir with <pkg>.js bundles> <projects-dir with <pkg>/node_modules> <thaw-js-run binary> [--write cases.json]
Each case is {pkg, path, args}; path "" calls the module itself, "a.b" a property path, args null reads the property.
"""
import json, subprocess, sys, os

corpus, projs, runner = [os.path.abspath(p) for p in sys.argv[1:4]]
write = sys.argv[5] if len(sys.argv) > 5 and sys.argv[4] == "--write" else None
cases = json.load(open(os.path.join(os.path.dirname(__file__), "cases.src.json")))
NODE = r"""
const [pkgdir, path, argsjson] = process.argv.slice(1);
process.chdir(pkgdir);
let m = require(require.resolve(process.env.PKG, {paths:[pkgdir]}));
let args = JSON.parse(argsjson);
let t = m, thisv = m;
for (const p of path.split('.').filter(Boolean)) { thisv = t; t = t[p]; }
let r = args === null ? t : t.apply(thisv, args);
const norm = v => v === undefined ? null : v;
console.log(JSON.stringify(norm(r), (k, v) => v === undefined ? null : (typeof v === 'number' && !isFinite(v) ? null : v)));
"""
PKGDIR = {"tj": "tj"}
bad = 0
out = []
for c in cases:
    pkg = c["pkg"]
    d = os.path.join(projs, PKGDIR.get(pkg, pkg))
    argsj = json.dumps(c["args"])
    env = dict(os.environ, PKG={"jsyaml": "js-yaml", "datefns": "date-fns"}.get(pkg, pkg))
    n = subprocess.run(["node", "-e", NODE, d, c["path"], argsj], capture_output=True, text=True, env=env, cwd=d)
    node_out = n.stdout.strip() if n.returncode == 0 else "ERROR"
    if n.returncode != 0 and os.environ.get("DEBUG"): print(n.stderr[-300:])
    t = subprocess.run([runner, os.path.join(corpus, pkg + ".js"), c["path"], argsj if c["args"] is not None else "null"], capture_output=True, text=True)
    thaw_out = t.stdout.strip()
    ok = thaw_out == node_out or (node_out == "ERROR" and thaw_out.startswith("ERROR"))
    if not ok:
        bad += 1
        print("DIFF", pkg, c["path"], argsj[:80], "\n   node:", node_out[:200], "\n   thaw:", thaw_out[:200], t.stderr.strip()[:200])
    out.append(dict(c, expect=node_out))
print(f"{len(cases)} cases, {bad} differ")
if write:
    json.dump(out, open(write, "w"), indent=1)
