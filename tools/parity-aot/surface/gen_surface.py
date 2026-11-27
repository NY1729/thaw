#!/usr/bin/env python3
"""Probe programs from the Node inventory (inv.js output). Usage: gen_surface.py INV_DIR OUT_DIR [module ...]

One probe per inventory member, in the gen_modules manifest format, so aot_run.py runs, bisects and classifies them:
  exists / arity / call / new   export level          static.<m>   class statics
  proto.<m> / inst.<m>          instance members, reached the way user code reaches them (recipes.json) so the
                                compiler's own idea of the type is what gets checked (an annotated parameter is not).
  prop.<a>.<b>                  members of plain-object exports
Skips (with a reason) go to skips.json. At most PER_PROGRAM probes per program.
"""
import json, subprocess
import os
import re
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.dirname(HERE))
import gen_probes  # noqa: E402  (render, CTOR_ARGS)

PER_PROGRAM = 40
IDENT = re.compile(r"^[A-Za-z_$][\w$]*$")
# Modules whose functions are called with no arguments (deterministic, no host side effects).
SAFE_CALL = {"node:path", "node:path/posix", "node:path/win32", "node:url", "node:querystring", "node:string_decoder",
             "node:buffer", "node:punycode"}
RECIPES = json.load(open(os.path.join(HERE, "recipes.json")))


def acc(base, name):
    return f"{base}.{name}" if IDENT.match(name) else f'{base}["{name}"]'


def key(mod):
    return mod.removeprefix("node:").replace("/", "_")


def gen(inv_dir, out_dir, only):
    programs, skips, total = [], [], 0
    for fn in sorted(os.listdir(inv_dir)):
        if not fn.endswith(".json"):
            continue
        inv = json.load(open(os.path.join(inv_dir, fn)))
        mod = inv["module"]
        if only and mod not in only and key(mod) not in only:
            continue
        glob = mod == "globals"
        rows = []  # (id, body)
        # An ESM namespace (`import * as M`) lacks the module object's non-enumerable properties (zlib.DEFLATE,
        # crypto.fips/rng/prng, ...); Node itself reports undefined for them, so they are not probed.
        ns = None
        if not glob:
            r = subprocess.run(["node", "--input-type=module", "-e", f"import('{mod}').then(n=>console.log(JSON.stringify(Object.keys(n))))"], capture_output=True, text=True)
            ns = set(json.loads(r.stdout)) if r.returncode == 0 and r.stdout.strip() else None

        def add(pid, body):
            rows.append((f"{key(mod)}.{pid}", body))

        for e in inv["exports"]:
            n, kind = e["name"], e["kind"]
            if ns is not None and n not in ns:
                continue
            X = n if glob else acc("M", n)
            if kind in ("function", "class", "getter", "object", "value"):
                add(f"{n}.exists", f"return typeof {X};")
            if kind == "function":
                add(f"{n}.arity", f"return {X}.length;")
                if mod in SAFE_CALL:
                    add(f"{n}.call", f"const f: any = {X}; return f();")
            if kind == "class":
                args = gen_probes.CTOR_ARGS.get(n, "") if glob else ""
                r = RECIPES.get(f"{mod}.{n}")
                can_new = e.get("construct") == "ok" or n in gen_probes.CTOR_ARGS and glob
                if can_new:
                    add(f"{n}.new", f"const C: any = {X}; return new C({args});")
                for m in e.get("static", []):
                    if m.get("from"):  # inherited statics (EventEmitter.*, Stream.*) are noise on every subclass
                        continue
                    add(f"{n}.static.{m['name']}", f"return typeof {acc(X, m['name'])};")
                for part, label in (("proto", "proto"), ("instance", "inst")):
                    for m in e.get(part, []):
                        mem = m["name"]
                        if r:
                            body = r["ts"].replace("$", f"void typeof {acc('o', mem)}") + ' return "built";'
                        elif can_new:
                            body = f"const o = new {X}({args}); return typeof {acc('o', mem)};"
                        else:
                            skips.append({"id": f"{key(mod)}.{n}.{label}.{mem}", "reason": "no instance recipe (new throws without args)"})
                            continue
                        add(f"{n}.{label}.{mem}", body)
            if kind == "object":
                def walk(props, path):
                    for m in props:
                        # Node's start-up module list differs run to run (which internals are loaded), so its elements are no oracle
                        if n == "moduleLoadList" and m["name"].isdigit():
                            continue
                        p = acc(path, m["name"])
                        add(f"{n}.prop{p[len(X):]}".replace('"', "'"), f"return typeof {p};")
                        if m.get("props"):
                            walk(m["props"], p)
                walk(e.get("props", []), X)
        names = [r[0] for r in rows]
        assert len(names) == len(set(names)), f"duplicate probe id in {mod}"
        rows.sort()
        for i in range(0, len(rows), PER_PROGRAM):
            chunk = rows[i:i + PER_PROGRAM]
            programs.append({
                "name": f"surf_{key(mod)}.c{i // PER_PROGRAM:03d}", "corpus": f"surf_{key(mod)}", "family": f"surf_{key(mod)}",
                "mode": "typed", "module": False,
                "imports": [] if glob else [f'import * as M from "{mod}";'],
                "probes": [{"name": pid, "api": [pid], "mode": "typed", "line": f'p({json.dumps(pid)}, () => {{ {body} }});',
                            "arg_types": [], "shape": False} for pid, body in chunk]})
        total += len(rows)
    os.makedirs(out_dir, exist_ok=True)
    for prog in programs:
        lines = [p["line"] for p in prog["probes"]]
        for suffix, node in ((".ts", False), (".mts", True)):
            with open(os.path.join(out_dir, prog["name"] + suffix), "w", encoding="utf-8") as fh:
                text = gen_probes.render(lines, False, node)
                if prog["imports"]:
                    head, rest = text.split("\n", 1)
                    text = head + "\n" + prog["imports"][0] + "\n" + rest
                fh.write(text)
    counts = {}
    for prog in programs:
        counts[prog["corpus"]] = counts.get(prog["corpus"], 0) + len(prog["probes"])
    json.dump({"per_program": PER_PROGRAM, "probes": total, "skipped": len(skips), "counts": counts, "findings": [],
               "programs": programs}, open(os.path.join(out_dir, "manifest.json"), "w"), indent=1, sort_keys=True)
    json.dump(skips, open(os.path.join(out_dir, "skips.json"), "w"), indent=1)
    print(f"gen_surface: {total} probes in {len(programs)} programs, {len(skips)} skipped")


if __name__ == "__main__":
    if len(sys.argv) < 3:
        sys.exit(__doc__)
    gen(sys.argv[1], sys.argv[2], set(sys.argv[3:]))
