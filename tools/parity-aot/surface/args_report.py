#!/usr/bin/env python3
"""args.md / args.json from an aot_run.py output dir run on omit/undef/null probes.
Usage: args_report.py OUT BASELINE.json [--write-baseline]
Per module and probe kind: probes, same (MATCH/MSG; same error name+code or same return kind), differ (WRONG-*/LEAK/MISSING),
build-fail (REJECT*), crash. Then the top causes (what Node did -> what thaw did) per module. Baseline = per-module
counts of `same`; REGRESSION is printed when it drops."""
import collections
import json
import os
import re
import sys

out, base_path = sys.argv[1], sys.argv[2]
man = json.load(open(os.path.join(out, "programs", "manifest.json")))
skips = json.load(open(os.path.join(out, "programs", "skips.json")))
cls = json.load(open(os.path.join(out, "thaw-js", "classes.json")))
BUCKET = {"MATCH": "same", "MSG": "same", "REJECT": "build-fail", "REJECT?": "build-fail", "REJECT-EXPECTED": "build-fail",
          "WRONG-TYPE": "differ", "WRONG-VALUE": "differ", "LEAK": "differ", "MISSING": "differ", "CRASH": "crash",
          "HANG": "crash", "UNVERIFIED": "unverified"}
COLS = ["same", "differ", "build-fail", "crash", "unverified"]
KINDS = ("omit", "undef", "null")


def node_first(prog, name):
    path = os.path.join(out, "node", prog + ".out")
    cur = None
    for line in open(path, encoding="utf-8", errors="replace"):
        line = line.rstrip("\n")
        if line.startswith("@@ "):
            cur = line[3:]
        elif cur == name:
            return line
    return ""


def norm(s):
    s = re.sub(r"'[^']*'|\"[^\"]*\"", "'X'", s)
    return re.sub(r"\d+", "N", s)[:110]


mods = collections.defaultdict(collections.Counter)
kinds = collections.defaultdict(collections.Counter)
causes = collections.defaultdict(collections.Counter)
examples = {}
rows = []
for prog in man["programs"]:
    mod = prog["corpus"].removeprefix("args_")
    for p in prog["probes"]:
        c = cls.get(p["name"])
        b = BUCKET.get(c["class"], "unverified") if c else "unverified"
        kind = next(k for k in KINDS if f".{k}." in p["name"])
        mods[mod][b] += 1
        kinds[kind][b] += 1
        if b in ("differ", "build-fail", "crash"):
            nf = norm(node_first(prog["name"], p["name"]))
            nf = nf.split(":")[0] if nf.startswith("THROW") else "returns " + nf.split(" ")[0]
            tf = norm((c or {}).get("detail", "")) if b != "differ" else norm((c or {}).get("detail", ""))
            key = (nf, b + ": " + tf)
            causes[mod][key] += 1
            examples.setdefault((mod, key), p["name"])
            rows.append({"probe": p["name"], "bucket": b, "node": nf, "thaw": tf})
for s in skips:
    mods[s["id"].split(".")[0]]["skipped"] += 1
tot = collections.Counter()
for m in mods.values():
    tot.update(m)
rate = lambda m: f"{100 * m['same'] / max(1, sum(m[c] for c in COLS)):.1f}%"
L = ["# omit / undef / null probes: Node vs thaw AOT", "", f"{man['probes']} probes. same = same return kind or same error name[code].", "",
     "| module | probes | same | differ | build-fail | crash | skipped (excluded/no instance) | same rate |", "|---|---|---|---|---|---|---|---|"]
for k in sorted(mods):
    m = mods[k]
    L.append(f"| {k} | {sum(m[c] for c in COLS)} | {m['same']} | {m['differ']} | {m['build-fail']} | {m['crash']} | {m['skipped']} | {rate(m)} |")
L.append(f"| **total** | {sum(tot[c] for c in COLS)} | {tot['same']} | {tot['differ']} | {tot['build-fail']} | {tot['crash']} | {tot['skipped']} | {rate(tot)} |")
L += ["", "| kind | probes | same | differ | build-fail | crash |", "|---|---|---|---|---|---|"]
for k in KINDS:
    m = kinds[k]
    L.append(f"| {k} | {sum(m[c] for c in COLS)} | {m['same']} | {m['differ']} | {m['build-fail']} | {m['crash']} |")
allc = collections.Counter()
for m, cs in causes.items():
    allc.update(cs)
L += ["", "## Top causes overall (node -> thaw)", ""]
for (nf, tf), n in allc.most_common(25):
    L.append(f"- {n}x node `{nf}` -> thaw `{tf}`")
for k in sorted(causes):
    L += ["", f"## {k}", ""] + [f"- {n}x node `{nf}` -> thaw `{tf}` (e.g. `{examples[(k, (nf, tf))]}`)" for (nf, tf), n in causes[k].most_common(8)]
open(os.path.join(out, "args.md"), "w").write("\n".join(L) + "\n")
counts = {k: dict(v) for k, v in sorted(mods.items())}
json.dump({"modules": counts, "kinds": {k: dict(v) for k, v in kinds.items()}, "total": dict(tot), "failures": rows},
          open(os.path.join(out, "args.json"), "w"), indent=1)
if "--write-baseline" in sys.argv:
    json.dump(counts, open(base_path, "w"), indent=1, sort_keys=True)
elif os.path.exists(base_path):
    old = json.load(open(base_path))
    for k, v in counts.items():
        if k in old and v.get("same", 0) < old[k].get("same", 0):
            print(f"REGRESSION {k}: same {old[k]['same']} -> {v.get('same', 0)}")
print("\n".join(L[:6 + len(mods)]))
