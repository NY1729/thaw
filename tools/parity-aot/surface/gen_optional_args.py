#!/usr/bin/env python3
"""Optional-argument table: which trailing parameters of which Node API may be omitted.

Usage: gen_optional_args.py --inv INV_DIR --types NODE_TYPES_DIR --out optional-args.json [--md optional-args.md]
  INV_DIR         tools/parity-aot/inventory (Node 22 inventory: Function.length per function and method)
  NODE_TYPES_DIR  an @types/node package dir (npm install --ignore-scripts @types/node@22 in a fresh empty dir)

For every inventory function (exports, class statics, class prototype methods, functions in plain-object exports) the
row records the parameter lists of its @types/node overloads (parsed with ../dts_parse.py, heritage followed by class
name) and the inventory `Function.length`:
  required   = min over overloads of the leading parameters that are not `?`/rest (what @types/node demands)
  jslen      = Function.length in Node
  nparams    = max positional parameters over overloads
  omit_from  = min(required, jslen): parameters at index >= omit_from may be left off (`optional` lists them)
  base_args  = JS argument expressions for one valid-looking call (used by gen_surface.py omit/undef/null probes)
A function with no @types/node declaration (inherited from an untyped base, runtime-only) gets nparams = jslen and
src = "len". The table is data for gen_surface.py (probes) and for the compiler's optional_native_args table.
"""
import argparse
import json
import os
import re
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.dirname(HERE))
import dts_parse  # noqa: E402

PATH_NAMES = {"path", "file", "filename", "src", "dest", "oldPath", "newPath", "target", "existingPath", "from", "to",
              "destination", "pathname", "filepath", "cwd", "p", "link", "linkPath"}
DIR_NAMES = {"dir", "directory", "prefix", "dirname", "dirPath"}
BUF = re.compile(r"\b(Buffer|Uint8Array|ArrayBufferView|ArrayBuffer|TypedArray|DataView|NonSharedBuffer|SharedArrayBuffer)\b")
# @types/node files that are not a module (skipped) and sub-path module aliases
SKIP_FILES = {"index.d.ts", "globals.d.ts", "globals.typedarray.d.ts", "buffer.buffer.d.ts", "inspector.generated.d.ts",
              "sea.d.ts", "sqlite.d.ts", "test.d.ts"}
ALIAS = {"path/posix": "path", "path/win32": "path", "assert/strict": "assert", "sys": "util", "inspector/promises": "inspector"}


def top_split(t, sep):
    out, depth, cur = [], 0, ""
    for ch in t:
        if ch in "<([{":
            depth += 1
        elif ch in ">)]}":
            depth -= 1
        if ch == sep and depth == 0:
            out.append(cur)
            cur = ""
        else:
            cur += ch
    return out + [cur]


def sample(t, name):
    """JS expression for a parameter of declared type text t."""
    t = " ".join(t.split()).lstrip("|& ").strip()
    if name in PATH_NAMES or re.search(r"PathLike|PathOrFileDescriptor", t):
        return '"sb/f"'
    if name in DIR_NAMES:
        return '"sb/d"'
    if name in ("host", "hostname", "address"):
        return '"127.0.0.1"'
    if name in ("url", "href", "origin"):
        return '"http://127.0.0.1:1/"'
    if name == "fd":
        return "999"
    if "=>" in t or re.search(r"\bFunction\b", t):
        return "() => {}"
    alts = [a.strip() for a in top_split(t, "|") if a.strip() not in ("undefined", "null", "void")]
    for a in alts or [t]:
        if a in ("string", "String"):
            return '"a"'
        if a in ("number", "Number"):
            return "1"
        if a == "boolean":
            return "true"
        if a == "bigint":
            return "1n"
        if a == "BufferEncoding":
            return '"utf8"'
        if re.fullmatch(r'"[^"]*"|\'[^\']*\'', a):
            return json.dumps(a[1:-1])
        if re.fullmatch(r"-?\d+", a):
            return a
        if BUF.search(a):
            return 'Buffer.from("ab")'
        if a.endswith("[]") or a.startswith("Array<") or a.startswith("ReadonlyArray<") or a.startswith("readonly "):
            return "[]"
        if a in ("any", "unknown"):
            return '"a"'
        if a == "URL":
            return 'new URL("file:///a")'
        if a.startswith("{") or a.startswith("Record<") or a in ("object", "Object"):
            return "{}"
    return "{}"


# ---------------------------------------------------------------- @types/node

def module_files(types_dir):
    for dp, dn, fn in os.walk(types_dir):
        dn[:] = [d for d in dn if d not in ("ts5.6", "compatibility", "web-globals", "node_modules")]
        for f in fn:
            if f.endswith(".d.ts") and f not in SKIP_FILES:
                yield os.path.join(dp, f)


def load_dts(types_dir):
    """{module: {"fns": {name: [sig]}, "classes": {name: {"heritage": [..], "members": {name: [(static, sigs)]}}}}}"""
    mods = {}

    def add_members(cls, d):
        for m in d.get("members", []):
            if m["kind"] in ("method", "function", "property") and m.get("sigs"):
                cls["members"].setdefault(m["name"], []).append((m.get("static", False), m["sigs"]))
        h = d.get("heritage", "")
        for part in re.split(r"\b(?:extends|implements)\b", h):
            for nm in top_split(part, ","):
                nm = re.sub(r"<.*", "", nm.strip()).split(".")[-1].strip()
                if nm:
                    cls["heritage"].append(nm)

    def walk(children, mod, prefix="", flat=False):
        # flat: namespace named like the module (`declare module "assert" { namespace assert {..} }`): its members are
        # also the module's own exports. Classes are registered by bare name too so heritage lookups find them.
        for d in children:
            bare = d.get("name", "")
            nm = prefix + bare
            if d["kind"] == "function" and d.get("sigs"):
                mod["fns"].setdefault(nm, []).extend(d["sigs"])
                if flat:
                    mod["fns"].setdefault(bare, []).extend(d["sigs"])
            elif d["kind"] in ("class", "interface"):
                for key in {nm, bare}:
                    c = mod["classes"].setdefault(key, {"heritage": [], "members": {}})
                    add_members(c, d)
            elif d["kind"] == "namespace":
                walk(d["children"], mod, nm + ".", flat or bare == mod["last"])

    for path in module_files(types_dir):
        src = re.sub(r"`[^`]*`", "string", open(path, encoding="utf-8").read())  # template literal types
        try:
            decls = dts_parse.Parser(src).statements(None)
        except ValueError as e:  # template literal types: dts_parse cannot tokenize the file
            print(f"gen_optional_args: skipped {os.path.relpath(path)} ({e})", file=sys.stderr)
            continue
        for d in decls:
            if d["kind"] == "ambient-module" and not d["name"].startswith("node:"):
                mod = mods.setdefault(d["name"], {"fns": {}, "classes": {}, "last": d["name"].split("/")[-1]})
                walk(d["children"], mod)
    for a, b in ALIAS.items():
        if b in mods and not mods.get(a, {}).get("fns"):
            mods[a] = mods[b]
    return mods


def find_class(mods, mod, name, seen=None):
    seen = seen or set()
    if (mod, name) in seen:
        return None
    seen.add((mod, name))
    c = mods.get(mod, {}).get("classes", {}).get(name)
    return c


def class_sigs(mods, mod, cls, member, static, seen=None):
    """sigs of cls.member, following heritage (same module first, then any module)."""
    seen = seen or set()
    if (mod, cls) in seen:
        return None
    seen.add((mod, cls))
    c = mods.get(mod, {}).get("classes", {}).get(cls)
    if c:
        out = [s for st, sigs in c["members"].get(member, []) if st == static for s in sigs]
        if out:
            return out
        for h in c["heritage"]:
            r = class_sigs(mods, mod, h, member, static, seen)
            if r:
                return r
            for m2 in mods:
                if h in mods[m2]["classes"]:
                    r = class_sigs(mods, m2, h, member, static, seen)
                    if r:
                        return r
    return None


# ---------------------------------------------------------------- table

def analyse(sigs, jslen):
    sigs = [s for s in (sigs or []) if s is not None]
    if sigs:
        lists = [[p for p in s["params"] if not p["rest"]] for s in sigs]
        nparams = max(len(p) for p in lists)

        def req(ps):
            n = 0
            for p in ps:
                if p["optional"]:
                    break
                n += 1
            return n
        required = min(req(p) for p in lists)
        best = max(lists, key=lambda ps: (req(ps), len(ps)))
        params = [{"name": p["name"], "type": " ".join(p["type"].split())[:80], "optional": p["optional"]} for p in best]
        base = [sample(p["type"], p["name"]) for p in best]
        src = "dts"
    else:
        nparams, required, params, src = (jslen or 0), (jslen or 0), [], "len"
        base = ['"a"'] * nparams
    omit_from = required if jslen is None else min(required, jslen)
    return {"nparams": nparams, "required": required, "jslen": jslen, "omit_from": omit_from,
            "optional": list(range(omit_from, nparams)), "params": params, "base_args": base, "src": src}


def build(inv_dir, types_dir):
    mods = load_dts(types_dir)
    rows = []
    for fn in sorted(os.listdir(inv_dir)):
        if not fn.endswith(".json"):
            continue
        inv = json.load(open(os.path.join(inv_dir, fn)))
        mod = inv["module"]
        short = mod.removeprefix("node:")
        dts = mods.get(short, {"fns": {}, "classes": {}})

        def row(owner, name, kind, jslen, sigs):
            r = {"module": mod, "owner": owner, "name": name, "kind": kind}
            r.update(analyse(sigs, jslen))
            rows.append(r)

        def walk_props(props, path):
            for m in props:
                p = path + [m["name"]]
                if m["kind"] == "function":
                    sigs = dts["fns"].get(".".join(p))
                    row(".".join(path), m["name"], "prop-fn", m.get("arity"), sigs)
                if m.get("props"):
                    walk_props(m["props"], p)

        for e in inv["exports"]:
            n = e["name"]
            if e["kind"] == "function":
                row("", n, "function", e.get("arity"), dts["fns"].get(n))
            elif e["kind"] == "object":
                walk_props(e.get("props", []), [n])
            elif e["kind"] == "class":
                for part, static in (("static", True), ("proto", False)):
                    for m in e.get(part, []):
                        if m["kind"] != "function" or m.get("from"):
                            continue
                        row(n, m["name"], "static" if static else "method", m.get("arity"),
                            class_sigs(mods, short, n, m["name"], static))
    return rows


def to_md(rows):
    by = {}
    for r in rows:
        by.setdefault(r["module"], []).append(r)
    L = ["# Optional arguments (generated by gen_optional_args.py)", "",
         "| module | functions | with optional params | src=dts | Function.length < @types required |", "|---|---|---|---|---|"]
    for m in sorted(by):
        rs = by[m]
        L.append(f"| {m} | {len(rs)} | {sum(1 for r in rs if r['optional'])} | {sum(1 for r in rs if r['src'] == 'dts')} | "
                 f"{sum(1 for r in rs if r['jslen'] is not None and r['jslen'] < r['required'])} |")
    return "\n".join(L) + "\n"


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--inv", required=True)
    ap.add_argument("--types", required=True)
    ap.add_argument("--out", required=True)
    ap.add_argument("--md")
    a = ap.parse_args()
    rows = build(a.inv, a.types)
    with open(a.out, "w", encoding="utf-8") as fh:
        json.dump({"rows": rows}, fh, indent=0, sort_keys=True)
        fh.write("\n")
    if a.md:
        open(a.md, "w", encoding="utf-8").write(to_md(rows))
    print(f"gen_optional_args: {len(rows)} functions, {sum(1 for r in rows if r['optional'])} with omittable params, "
          f"{sum(1 for r in rows if r['src'] == 'dts')} typed from @types/node")


if __name__ == "__main__":
    main()
