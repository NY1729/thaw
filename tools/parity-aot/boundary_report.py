#!/usr/bin/env python3
"""TSV + markdown report of a boundary-matrix run (gen_boundary.py programs, aot_run.py results).

Usage: boundary_report.py OUT_DIR   (reads OUT/programs/manifest.json, OUT/thaw-js/classes.json, OUT/node/*.out,
OUT/thaw-js/gaps.json; writes OUT/boundary.tsv and OUT/boundary.md)
Columns of boundary.tsv: module.fn, slot, shape, origin, class, sub, node first lines, thaw detail, new (1 when the gap is
not in known-gaps-boundary.json). A REJECT of a `direct` origin probe whose sub is `type` is shown as REJECT-TYPE: the
TS signature does not take that shape, so the reject is the expected answer, not a finding.
"""
import json
import os
import re
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.join(HERE, "..", "parity"))
import diff as pdiff  # noqa: E402

FAIL = ("WRONG-TYPE", "WRONG-VALUE", "LEAK", "MISSING", "CRASH", "HANG", "REJECT", "REJECT?")


def split_id(pid):
    mod, rest = pid.split(".", 1)
    fn, slot, shape, origin = rest.rsplit(".", 3)
    return mod, fn, slot, shape, origin


def first(block, n=2):
    return " / ".join((block or "").split("\n")[:n])[:120]


def main(out):
    man = json.load(open(os.path.join(out, "programs", "manifest.json")))
    classes = json.load(open(os.path.join(out, "thaw-js", "classes.json")))
    node = {}
    for prog in man["programs"]:
        path = os.path.join(out, "node", prog["name"] + ".out")
        if os.path.exists(path):
            node.update(pdiff.load_blocks(path))
    thaw = {}
    for corpus in {c["corpus"] for c in classes.values()}:
        path = os.path.join(out, "thaw-js", "corpus", "thaw", corpus + ".txt")
        if os.path.exists(path):
            thaw.update(pdiff.load_blocks(path))
    gp = os.path.join(out, "thaw-js", "gaps.json")
    new = set()
    if os.path.exists(gp):
        new = {g["id"] for g in json.load(open(gp))["gaps"]}
    rows = []
    for pid, c in sorted(classes.items()):
        mod, fn, slot, shape, origin = split_id(pid)
        cls = c["class"]
        if cls == "REJECT" and origin == "direct" and c.get("sub") == "type":
            cls = "REJECT-TYPE"
        nb, tb = node.get(pid), thaw.get(pid)
        # a value match with a different TREE is the stray-file signature
        stray = cls == "WRONG-VALUE" and nb and tb and nb.split("\nTREE\n")[0] == tb.split("\nTREE\n")[0]
        if stray:
            cls = "WRONG-TREE"
        rows.append([f"{mod}.{fn}", slot, shape, origin, cls, c.get("sub", ""), first(nb), (first(tb) if tb else c["detail"][:120]),
                     "1" if pid in new else "0"])
    with open(os.path.join(out, "boundary.tsv"), "w", encoding="utf-8") as fh:
        fh.write("fn\tslot\tshape\torigin\tclass\tsub\tnode\tthaw\tnew\n")
        for r in rows:
            fh.write("\t".join(x.replace("\t", " ") for x in r) + "\n")
    order = ["MATCH", "MSG", "WRONG-TYPE", "WRONG-VALUE", "WRONG-TREE", "LEAK", "MISSING", "CRASH", "HANG", "REJECT",
             "REJECT-TYPE", "REJECT-EXPECTED", "REJECT?", "UNVERIFIED"]
    md = ["# boundary-value matrix (non-gating)", "", f"{len(rows)} probes, {sum(r[8] == '1' for r in rows)} gaps not in "
          "known-gaps-boundary.json (new)", ""]
    for title, idx in (("module", lambda r: r[0].split(".")[0]), ("shape", lambda r: r[2]), ("origin", lambda r: r[3])):
        tab = {}
        for r in rows:
            tab.setdefault(idx(r), {}).setdefault(r[4], 0)
            tab[idx(r)][r[4]] += 1
        md += [f"## by {title}", "", f"| {title} | " + " | ".join(order) + " |", "|---" * (len(order) + 1) + "|"]
        for k in sorted(tab):
            md.append(f"| {k} | " + " | ".join(str(tab[k].get(c, 0)) for c in order) + " |")
        md.append("")
    fam = {}
    for r in rows:
        if r[4] in FAIL + ("WRONG-TREE",):
            key = (r[4], re.sub(r"\d+", "N", r[6])[:70], re.sub(r"\d+", "N", r[7])[:70])
            fam.setdefault(key, []).append(r)
    md += ["## failure families (class, node, thaw), by size", ""]
    for key, rs in sorted(fam.items(), key=lambda kv: -len(kv[1]))[:30]:
        ex = rs[0]
        md.append(f"- {len(rs)}x {key[0]}: node `{key[1]}` thaw `{key[2]}`; e.g. {ex[0]} {ex[2]}/{ex[3]}; shapes "
                  f"{sorted({r[2] for r in rs})[:6]}")
    md.append("")
    open(os.path.join(out, "boundary.md"), "w", encoding="utf-8").write("\n".join(md) + "\n")
    print("\n".join(md[:60]))
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1]))
