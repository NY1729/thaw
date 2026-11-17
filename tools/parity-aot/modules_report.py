#!/usr/bin/env python3
"""Module-surface report for tools/parity-aot (non-gating): joins the outputs of a module run into modules.md.

Inputs: shard directories, each with programs/manifest.json and programs/skips.json (gen_modules.py) and
<engine>/classes.json (aot_run.py). A directory without programs/manifest.json is scanned for shard subdirectories
(CI downloads one artifact per shard under one parent). A shard without classes.json did not run: its probes count
as unclassified.

Table: one row per module. exports, probed, hand and skipped count exports (an export is probed when it has at
least one probe; hand when a probe has an api id `hand:...`; skipped when skip.txt or gen_modules gave it a
category). The class columns count probes. REJECT is split into missing-export / type / other (aot_run.reject_kind).
Diagnostic families: the most common (class, normalised diagnostic) pairs over the non-matching probes.

Usage: modules_report.py SHARD_OR_PARENT_DIR... [--engine thaw-js] [--out modules.md] [--json-out FILE]
                         [--merge-manifest FILE] [--coverage coverage.json]
--merge-manifest writes the shards' programs as one manifest (input of coverage.py --no-ratchet in CI).
"""
import argparse
import collections
import json
import os
import re
import sys

IMPORT_RE = re.compile(r'^import \* as M from "([^"]+)";$')
NOT_EXPORT = {"NOT-EXPORT", "NOT-IN-TABLE"}
REJECT_KINDS = ("missing-export", "type", "other")
TOP_N = 10


def find_shards(roots):
    found = []
    for root in roots:
        if os.path.exists(os.path.join(root, "programs", "manifest.json")):
            found.append(root)
            continue
        if not os.path.isdir(root):
            continue
        for name in sorted(os.listdir(root)):
            sub = os.path.join(root, name)
            if os.path.exists(os.path.join(sub, "programs", "manifest.json")):
                found.append(sub)
    if not found:
        sys.exit(f"modules_report: no shard with programs/manifest.json under {', '.join(roots)}")
    return found


def load_json(path, default):
    if not os.path.exists(path):
        return default
    with open(path, encoding="utf-8") as fh:
        return json.load(fh)


def export_of(module, api):
    """'mod:fs/promises.readFile' under module 'node:fs/promises' -> 'readFile'."""
    name = module.removeprefix("node:")
    body = api.removeprefix("mod:")
    if not body.startswith(name + "."):
        raise ValueError(f"api id {api!r} does not belong to {module}")
    return body[len(name) + 1:]


def classify(c):
    """Column of one probe class entry (None = unclassified)."""
    if c is None:
        return "unclassified"
    cls = c["class"]
    if cls == "REJECT":
        return "REJECT:" + c.get("sub", "other")
    if cls in ("MATCH", "WRONG-VALUE", "CRASH", "HANG", "REJECT?"):
        return {"CRASH": "CRASH+HANG", "HANG": "CRASH+HANG"}.get(cls, cls)
    return "other"


def build(shards, engine):
    """All the numbers the report prints, as plain dicts (also the JSON output)."""
    mods = collections.OrderedDict()
    skip_reasons = collections.Counter()
    diag = collections.defaultdict(lambda: {"n": 0, "probes": []})
    merged = {"per_program": None, "probes": 0, "programs": [], "skipped": 0, "counts": {}, "findings": []}
    shard_info = []
    unclassified = 0
    for sdir in shards:
        prog_dir = os.path.join(sdir, "programs")
        manifest = load_json(os.path.join(prog_dir, "manifest.json"), None)
        skips = load_json(os.path.join(prog_dir, "skips.json"), {"exports": []})["exports"]
        classes = load_json(os.path.join(sdir, engine, "classes.json"), None)
        shard_name = os.path.basename(os.path.normpath(sdir))
        shard_info.append({"shard": shard_name, "programs": len(manifest["programs"]),
                           "probes": manifest["probes"], "classes": classes is not None})
        merged["per_program"] = manifest.get("per_program", merged["per_program"])
        merged["programs"].extend(manifest["programs"])
        merged["probes"] += manifest["probes"]
        merged["skipped"] += manifest.get("skipped", 0)
        for k, v in manifest.get("counts", {}).items():
            merged["counts"][k] = merged["counts"].get(k, 0) + v
        for s in skips:
            m = mods.setdefault(s["module"], {"exports": set(), "probed": set(), "hand": set(), "skipped": set(),
                                              "cols": collections.Counter(), "cats": collections.Counter()})
            if s["category"] in NOT_EXPORT:
                m["cats"][s["category"]] += 1
                continue
            m["exports"].add(s["export"])
            m["skipped"].add(s["export"])
            m["cats"][s["category"]] += 1
            skip_reasons[(s["category"], s["reason"])] += 1
        for prog in manifest["programs"]:
            mm = IMPORT_RE.match(prog["imports"][0]) if prog.get("imports") else None
            if mm is None:
                raise SystemExit(f"modules_report: program {prog['name']} has no `import * as M` line")
            module = mm.group(1)
            m = mods.setdefault(module, {"exports": set(), "probed": set(), "hand": set(), "skipped": set(),
                                         "cols": collections.Counter(), "cats": collections.Counter()})
            for p in prog["probes"]:
                export = export_of(module, p["api"][0])
                m["exports"].add(export)
                m["probed"].add(export)
                if p["api"][0].startswith("hand:"):
                    m["hand"].add(export)
                c = classes.get(p["name"]) if classes is not None else None
                if c is None:
                    unclassified += 1
                m["cols"][classify(c)] += 1
                if c is not None and c["class"] != "MATCH":
                    key = f"{c['class']}{' ' + c['sub'] if c['class'] == 'REJECT' else ''}: {c['detail']}"
                    diag[key]["n"] += 1
                    if len(diag[key]["probes"]) < 2:
                        diag[key]["probes"].append(p["name"])
    for m in mods.values():
        m["skipped"] -= m["probed"]
    return {"mods": mods, "skip_reasons": skip_reasons, "diag": diag, "shards": shard_info,
            "unclassified": unclassified, "merged": merged}


def cells(m):
    cols = m["cols"]
    reject = {k: cols.get("REJECT:" + k, 0) for k in REJECT_KINDS}
    known = {"MATCH", "WRONG-VALUE", "CRASH+HANG", "REJECT?", "unclassified"}
    other = sum(v for k, v in cols.items() if k not in known and not k.startswith("REJECT:"))
    return {"exports": len(m["exports"]), "probed": len(m["probed"]), "hand": len(m["hand"]),
            "skipped": len(m["skipped"]), "MATCH": cols.get("MATCH", 0), "WRONG-VALUE": cols.get("WRONG-VALUE", 0),
            "REJECT": reject, "CRASH+HANG": cols.get("CRASH+HANG", 0), "REJECT?": cols.get("REJECT?", 0),
            "other": other, "unclassified": cols.get("unclassified", 0)}


def markdown(res, engine, coverage):
    lines = [f"# Module surface: {engine} vs Node (NON-GATING)", ""]
    shards = res["shards"]
    lines.append(f"Shards: " + ", ".join(
        f"`{s['shard']}` ({s['programs']} programs, {s['probes']} probes" +
        ("" if s["classes"] else ", did not run") + ")" for s in shards) + ".")
    lines.append("")
    head = ["module", "exports", "probed", "hand", "skipped", "MATCH", "WRONG-VALUE",
            "REJECT (missing-export / type / other)", "CRASH+HANG", "REJECT?", "other"]
    lines += ["| " + " | ".join(head) + " |", "|" + "---|" * len(head)]
    total = collections.Counter()
    rej_total = collections.Counter()
    for mod in sorted(res["mods"]):
        c = cells(res["mods"][mod])
        for k in ("exports", "probed", "hand", "skipped", "MATCH", "WRONG-VALUE", "CRASH+HANG", "REJECT?", "other"):
            total[k] += c[k]
        for k in REJECT_KINDS:
            rej_total[k] += c["REJECT"][k]
        lines.append(f"| {mod} | {c['exports']} | {c['probed']} | {c['hand']} | {c['skipped']} | {c['MATCH']} | "
                     f"{c['WRONG-VALUE']} | {fmt_reject(c['REJECT'])} | {c['CRASH+HANG']} | {c['REJECT?']} | "
                     f"{c['other']} |")
    lines.append("| **total** | " + " | ".join(str(total[k]) for k in
                 ("exports", "probed", "hand", "skipped", "MATCH", "WRONG-VALUE")) + " | "
                 + fmt_reject(rej_total) + f" | {total['CRASH+HANG']} | {total['REJECT?']} | {total['other']} |")
    lines += ["", "Counts are probes except exports, probed, hand and skipped (exports). `other` = WRONG-TYPE, LEAK, "
              "MISSING, MSG, UNVERIFIED, REJECT-EXPECTED.", ""]
    if res["unclassified"]:
        lines += [f"**{res['unclassified']} probe(s) have no class entry** (a shard did not run or its output is "
                  "incomplete).", ""]
    if coverage is not None:
        mc = coverage.get("per_family", {}).get("modules", {}).get("all")
        if mc:
            lines += [f"API coverage (coverage.py, merged manifest): module exports touched {mc['touched']} of "
                      f"{mc['total']} ({mc['pct']}%).", ""]
    lines += ["## Top diagnostic families", ""]
    top = sorted(res["diag"].items(), key=lambda kv: (-kv[1]["n"], kv[0]))[:TOP_N]
    if top:
        lines += ["| probes | class | diagnostic (normalised) | e.g. |", "|---|---|---|---|"]
        for key, v in top:
            cls, _, detail = key.partition(": ")
            lines.append(f"| {v['n']} | {cls} | `{detail}` | {', '.join(f'`{p}`' for p in v['probes'])} |")
    else:
        lines.append("None.")
    lines += ["", "## Skipped exports by reason", ""]
    if res["skip_reasons"]:
        lines += ["| category | exports | reason |", "|---|---|---|"]
        for (cat, reason), n in sorted(res["skip_reasons"].items(), key=lambda kv: (-kv[1], kv[0])):
            lines.append(f"| {cat} | {n} | {reason} |")
    else:
        lines.append("None.")
    if total_not_in_table(res):
        lines += ["", f"NOT-IN-TABLE: {total_not_in_table(res)} export(s) listed by Object.keys without a table row "
                  "(module_table.tsv is stale)."]
    return "\n".join(lines) + "\n"


def total_not_in_table(res):
    return sum(m["cats"].get("NOT-IN-TABLE", 0) for m in res["mods"].values())


def fmt_reject(r):
    return f"{sum(r.values())} ({r['missing-export']} / {r['type']} / {r['other']})"


def main(argv):
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[1])
    ap.add_argument("roots", nargs="+")
    ap.add_argument("--engine", default="thaw-js")
    ap.add_argument("--out")
    ap.add_argument("--json-out")
    ap.add_argument("--merge-manifest")
    ap.add_argument("--coverage")
    args = ap.parse_args(argv)
    shards = find_shards(args.roots)
    res = build(shards, args.engine)
    coverage = load_json(args.coverage, None) if args.coverage else None
    md = markdown(res, args.engine, coverage)
    if args.out:
        with open(args.out, "w", encoding="utf-8") as fh:
            fh.write(md)
    if args.json_out:
        data = {"engine": args.engine, "shards": res["shards"], "unclassified": res["unclassified"],
                "modules": {m: cells(v) for m, v in res["mods"].items()},
                "diagnostics": [{"key": k, "n": v["n"], "probes": v["probes"]}
                                for k, v in sorted(res["diag"].items(), key=lambda kv: (-kv[1]["n"], kv[0]))]}
        with open(args.json_out, "w", encoding="utf-8") as fh:
            json.dump(data, fh, indent=1, sort_keys=True)
            fh.write("\n")
    if args.merge_manifest:
        with open(args.merge_manifest, "w", encoding="utf-8") as fh:
            json.dump(res["merged"], fh, indent=1, sort_keys=True)
            fh.write("\n")
    sys.stdout.write(md)
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
