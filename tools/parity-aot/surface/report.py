#!/usr/bin/env python3
"""surface.md / surface.json from an aot_run.py output dir. Usage: report.py OUT BASELINE.json [--write-baseline]
Classes: match = MATCH+MSG; build-fail = REJECT/REJECT?; output-diff = WRONG-*/LEAK/MISSING; crash = CRASH/HANG; skipped = gen skips."""
import collections
import json
import os
import re
import sys

out, base_path = sys.argv[1], sys.argv[2]
man = json.load(open(os.path.join(out, "programs", "manifest.json")))
skips = json.load(open(os.path.join(out, "programs", "skips.json")))
cls = json.load(open(os.path.join(out, "thaw-js", "classes.json")))
BUCKET = {"MATCH": "match", "MSG": "match", "REJECT": "build-fail", "REJECT?": "build-fail", "REJECT-EXPECTED": "build-fail",
          "WRONG-TYPE": "output-diff", "WRONG-VALUE": "output-diff", "LEAK": "output-diff", "MISSING": "output-diff",
          "CRASH": "crash", "HANG": "crash", "UNVERIFIED": "unverified"}
COLS = ["match", "build-fail", "output-diff", "crash", "unverified"]
mods = collections.defaultdict(collections.Counter)
causes = collections.defaultdict(collections.Counter)
fails = collections.defaultdict(list)
for prog in man["programs"]:
    mod = prog["corpus"].removeprefix("surf_")
    for p in prog["probes"]:
        c = cls.get(p["name"])
        b = BUCKET.get(c["class"], "unverified") if c else "unverified"
        mods[mod][b] += 1
        if b != "match":
            if b == "build-fail":
                d = re.sub(r"\d+", "N", c["detail"].split("\n")[0])[:160]
                causes[mod][d] += 1
            fails[mod].append((p["name"], b, (c or {}).get("detail", "")[:100]))
for s in skips:
    mods[s["id"].split(".")[0]]["skipped"] += 1
tot = collections.Counter()
for m in mods.values():
    tot.update(m)
rate = lambda m: f"{100 * m['match'] / max(1, sum(m[c] for c in COLS)):.1f}%"
L = ["# Node vs thaw AOT: API surface matrix", "", f"{man['probes']} probes. match rate = match / probed (skipped excluded).", "",
     "| module | probes | match | build-fail | output-diff | crash | skipped | match rate |", "|---|---|---|---|---|---|---|---|"]
for k in sorted(mods):
    m = mods[k]
    L.append(f"| {k} | {sum(m[c] for c in COLS)} | {m['match']} | {m['build-fail']} | {m['output-diff']} | {m['crash']} | {m['skipped']} | {rate(m)} |")
L.append(f"| **total** | {sum(tot[c] for c in COLS)} | {tot['match']} | {tot['build-fail']} | {tot['output-diff']} | {tot['crash']} | {tot['skipped']} | {rate(tot)} |")
for k in sorted(mods):
    if not causes[k] and not fails[k]:
        continue
    L += ["", f"## {k}", ""]
    if causes[k]:
        L += ["Top build-fail causes:"] + [f"- {n}x `{d}`" for d, n in causes[k].most_common(8)] + [""]
    L += ["Failing members (first 60):"] + [f"- {b} `{n}` {d}" for n, b, d in sorted(fails[k])[:60]]
open(os.path.join(out, "surface.md"), "w").write("\n".join(L) + "\n")
counts = {k: dict(v) for k, v in sorted(mods.items())}
json.dump({"modules": counts, "total": dict(tot), "failures": {k: v for k, v in sorted(fails.items())}},
          open(os.path.join(out, "surface.json"), "w"), indent=1)
if "--write-baseline" in sys.argv:
    old = json.load(open(base_path)) if os.path.exists(base_path) else {}
    old.update(counts)
    json.dump(old, open(base_path, "w"), indent=1, sort_keys=True)
elif os.path.exists(base_path):
    old = json.load(open(base_path))
    for k, v in counts.items():
        if k in old and v.get("match", 0) < old[k].get("match", 0):
            print(f"REGRESSION {k}: match {old[k]['match']} -> {v.get('match', 0)}")
print("\n".join(L[:4 + len(mods) + 3]))
