#!/usr/bin/env python3
"""omit / undef / null probe rows for gen_surface.py --args (see README). One call per probe, through `any`
(the path a JS-side adapter takes), arguments from optional-args.json (gen_optional_args.py), sandbox = cwd-relative
sb/ (reset before every probe) and loopback hosts. Functions in args-excludes.json are never called.

  <fn>.omit.<k>    first k of the n arguments (k = n-1 down to max(0, n-4))
  <fn>.undef.<i>   argument i is undefined, the others valid
  <fn>.null.<i>    argument i is null
n = nparams of the table row, at most MAX_N."""
import json
import os
import re

HERE = os.path.dirname(os.path.abspath(__file__))
MAX_N = 6
OMIT_LEVELS = 4
IDENT = re.compile(r"^[A-Za-z_$][\w$]*$")
EXCLUDES = json.load(open(os.path.join(HERE, "args-excludes.json")))["excludes"]


def excluded(mod, owner, name):
    for x in EXCLUDES:
        if re.fullmatch(x["module"], mod) and re.fullmatch(x["owner"], owner) and re.fullmatch(x["name"], name):
            return x["reason"]
    return None


def acc(base, name):
    return f"{base}.{name}" if IDENT.match(name) else f'{base}["{name}"]'


def arg_probes(base_args):
    n = min(len(base_args), MAX_N)
    a = base_args[:n]
    for k in range(n - 1, max(-1, n - 1 - OMIT_LEVELS), -1):
        yield f"omit.{k}", a[:k]
    for i in range(n):
        yield f"undef.{i}", a[:i] + ["undefined"] + a[i + 1:]
        yield f"null.{i}", a[:i] + ["null"] + a[i + 1:]


def arg_rows(mod, inv, table, recipes, add, skip, glob):
    """add(pid, body); skip(id, reason). table: rows of optional-args.json for this module."""
    by = {(r["owner"], r["name"], r["kind"]): r for r in table if r["module"] == mod}
    classes = {e["name"]: e for e in inv["exports"] if e["kind"] == "class"}
    for (owner, name, kind), r in sorted(by.items()):
        why = excluded(mod, owner, name)
        pid_base = (f"{owner}.static.{name}" if kind == "static" else f"{owner}.{name}") if owner else name
        if why:
            skip(f"{pid_base}", "excluded: " + why)
            continue
        if kind == "function":
            X = name if glob else acc("M", name)
            call = lambda args, X=X: f"const a: any = {X}; return a({', '.join(args)});"
        elif kind == "prop-fn":
            X = (acc("M", owner.split(".")[0]) if not glob else owner.split(".")[0])
            for part in owner.split(".")[1:]:
                X = acc(X, part)
            call = lambda args, X=X, name=name: f"const o: any = {X}; return o.{name}({', '.join(args)});" if IDENT.match(name) else f'const o: any = {X}; return o["{name}"]({", ".join(args)});'
        else:  # static / method
            c = classes.get(owner)
            CX = owner if glob else acc("M", owner)
            if kind == "static":
                call = lambda args, CX=CX, name=name: f"const o: any = {CX}; return o[{json.dumps(name)}]({', '.join(args)});"
            elif f"{mod}.{owner}" in recipes or not c or c.get("construct") != "ok":
                skip(pid_base, "no instance without a recipe")
                continue
            else:
                call = lambda args, CX=CX, name=name: f"const o: any = new {CX}(); return o[{json.dumps(name)}]({', '.join(args)});"
        for suffix, args in arg_probes(r["base_args"]):
            add(f"{pid_base}.{suffix}", call(args))
