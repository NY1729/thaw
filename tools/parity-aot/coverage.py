#!/usr/bin/env python3
"""API coverage of the AOT probe manifest (no thaw build; reads manifest.json only).

API ids: `<owner>.<key>` as in crates/thaw-js/src/builtin_table.tsv (`Array.prototype.map`, `Math.max`,
`Map.prototype.size`), `global:<name>` for tools/parity/node_globals.json, `mod:<name>.<export>` for the
modules in modules.txt (Node's own Object.keys of require('node:<name>'), enumerated at run time).

The universe is computed at run time, so a new table row or Node export shows up as untouched instead of
being silently ignored. A (id, mode) pair is applicable unless it is json mode on a non-JSON receiver owner.
Touched means a probe exists for the pair; it does not mean the probe passes.

Ratchet: coverage-baseline.json lists the untouched `id#mode` pairs. Shrink-only: an untouched pair that is not
in the baseline (a new table row, or a removed probe) exits 1. Pairs that became touched are printed.
--write-baseline rewrites the list. The ratchet is skipped (with a warning) when node is not major 22.

Several manifests (gen_probes.py and gen_modules.py output) are scored together: repeat --manifest.
Usage: coverage.py --manifest MANIFEST.json [--manifest M2.json ...] [--out DIR] [--summary SUMMARY.md]
                   [--write-baseline] [--no-ratchet]
Exit: 0 ok, 1 ratchet violation, 2 harness error (bad manifest, unknown api id, node failure).
"""
import argparse
import json
import os
import subprocess
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.abspath(os.path.join(HERE, "..", ".."))
TABLE = os.path.join(REPO, "crates", "thaw-js", "src", "builtin_table.tsv")
GLOBALS = os.path.join(REPO, "tools", "parity", "node_globals.json")
MODULES = os.path.join(HERE, "modules.txt")
BASELINE = os.path.join(HERE, "coverage-baseline.json")
NODE_MAJOR = 22

MODES = ["typed", "any", "union", "unknown", "generic", "callback", "json", "module"]
JSON_OWNERS = {"Array", "Array.prototype", "String", "String.prototype", "Object", "Object.prototype",
               "Math", "Number", "Number.prototype"}
SKIP_DATA_KEYS = {"length", "name", "constructor"}  # function own data props; accessors such as length stay
SKIP_KEYS = {"@@toStringTag"}
NESTED_KEEP = {"%TypedArray%", "%TypedArray%.prototype"}
FAMILY_OF_OWNER = {}
for _fam, _owners in {
    "array": ["Array", "Array.prototype"],
    "string": ["String", "String.prototype"],
    "typedarray": ["%TypedArray%", "%TypedArray%.prototype"],
    "map-set-weak": ["Map", "Map.prototype", "Set", "Set.prototype", "WeakMap", "WeakMap.prototype",
                     "WeakSet", "WeakSet.prototype"],
    "promise": ["Promise", "Promise.prototype"],
    "json": ["JSON"],
    "object-reflect-proxy": ["Object", "Object.prototype", "Reflect", "Proxy"],
    "date": ["Date", "Date.prototype"],
    "regexp": ["RegExp", "RegExp.prototype"],
    "math": ["Math"],
    "number-bigint-symbol-error": ["Number", "Number.prototype", "BigInt", "BigInt.prototype", "Symbol",
                                   "Symbol.prototype", "Error", "Error.prototype"],
    "buffer": ["Buffer", "Buffer.prototype"],
}.items():
    for _o in _owners:
        FAMILY_OF_OWNER[_o] = _fam
FAMILIES = list(dict.fromkeys(list(FAMILY_OF_OWNER.values()) + ["globals", "modules", "hand", "other"]))


def family_of(api_id):
    if api_id.startswith("global:"):
        return "globals"
    if api_id.startswith("mod:"):
        return "modules"
    if api_id.startswith("hand:"):
        return "hand"
    return FAMILY_OF_OWNER.get(owner_of(api_id), "other")


def owner_of(api_id):
    if api_id.startswith(("global:", "mod:")):
        return None
    return api_id[:api_id.rfind(".")]


def applicable(api_id, mode):
    """json mode needs a JSON-expressible receiver owner; every other mode applies to every id."""
    return mode != "json" or owner_of(api_id) in JSON_OWNERS


def table_ids(path):
    with open(path, encoding="utf-8") as fh:
        header = fh.readline().rstrip("\n").split("\t")
        ids = []
        for line in fh:
            row = dict(zip(header, line.rstrip("\n").split("\t")))
            if not (row["kind"] == "accessor" or (row["kind"] == "data" and row["vtype"] == "function")):
                continue
            owner, key = row["owner"], row["key"]
            if (key in SKIP_KEYS or (key in SKIP_DATA_KEYS and row["kind"] == "data")
                    or ("%" in owner and owner not in NESTED_KEEP)):
                continue
            ids.append(f"{owner}.{key}")
    if len(ids) != len(set(ids)):
        raise SystemExit(f"coverage: duplicate ids in {path}")
    return ids


def module_ids(path, node):
    """mod:<name>.<export> for every module listed in path (names as written, minus the node: prefix)."""
    with open(path, encoding="utf-8") as fh:
        mods = [m.strip() for m in fh if m.strip() and not m.startswith("#")]
    if not mods:
        return [], None
    script = ("const out = {};"
              "for (const m of process.argv.slice(1)) {"
              "  try { out[m] = Object.keys(require(m)).sort(); }"
              "  catch (e) { console.error('require failed: ' + m + ': ' + e.message); process.exit(3); }"
              "}"
              "process.stdout.write(JSON.stringify(out));")
    res = subprocess.run([node, "-e", script, *mods], capture_output=True, text=True)
    if res.returncode != 0:
        raise SystemExit(f"coverage: node module enumeration failed: {res.stderr.strip()}")
    exports = json.loads(res.stdout)
    ids = [f"mod:{m.removeprefix('node:')}.{k}" for m in mods for k in exports[m]]
    ver = subprocess.run([node, "-p", "process.versions.node.split('.')[0]"], capture_output=True, text=True)
    return ids, int(ver.stdout.strip())


def universe(args):
    """U1 table rows, U2 globals, U3 module exports, U4 hand ids (gen_builtins.HAND_IDS unless --no-hand)."""
    with open(args.globals, encoding="utf-8") as fh:
        globals_ = [f"global:{name}" for name, _ in json.load(fh)]
    u1 = table_ids(args.table)
    u3, node_major = module_ids(args.modules, args.node)
    # u4: the documented hand ids of gen_builtins (API surfaces that are not table rows: indexing, Proxy traps)
    u4 = []
    if not args.no_hand:
        sys.path.insert(0, HERE)
        from gen_builtins import HAND_IDS
        u4 = list(HAND_IDS)
    ids = u1 + globals_ + u3 + u4
    if len(ids) != len(set(ids)):
        raise SystemExit("coverage: universe ids collide")
    return {"u1": u1, "u2": globals_, "u3": u3, "u4": u4}, node_major


def main(argv):
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("--manifest", required=True, action="append",
                    help="manifest.json of a generator; repeat to score several together")
    ap.add_argument("--out", help="directory for coverage.json (default: the manifest's directory)")
    ap.add_argument("--summary", help="append the markdown table to this summary.md")
    ap.add_argument("--table", default=TABLE)
    ap.add_argument("--globals", default=GLOBALS)
    ap.add_argument("--modules", default=MODULES)
    ap.add_argument("--baseline", default=BASELINE)
    ap.add_argument("--node", default=os.environ.get("NODE", "node"))
    ap.add_argument("--write-baseline", action="store_true")
    ap.add_argument("--no-ratchet", action="store_true", help="for subset runs (PARITY_AOT_FAMILIES)")
    ap.add_argument("--no-hand", action="store_true", help="leave U4 (gen_builtins hand ids) out of the universe")
    ap.add_argument("--allow-unknown", action="store_true",
                    help="count api ids outside the universe under unknown_api_ids instead of exit 2")
    args = ap.parse_args(argv)

    try:
        manifests = []
        for path in args.manifest:
            with open(path, encoding="utf-8") as fh:
                manifests.append(json.load(fh))
        uni, node_major = universe(args)
    except (OSError, ValueError, ImportError) as exc:
        print(f"coverage: harness error: {exc}", file=sys.stderr)
        return 2
    all_ids = uni["u1"] + uni["u2"] + uni["u3"] + uni["u4"]
    known = set(all_ids)

    touched = set()
    unknown = set()
    with_api = 0
    programs = [prog for manifest in manifests for prog in manifest["programs"]]
    for prog in programs:
        for probe in prog["probes"]:
            mode = probe.get("mode", prog.get("mode"))
            if mode not in MODES:
                print(f"coverage: harness error: probe {probe['name']} has mode {mode!r}", file=sys.stderr)
                return 2
            api = probe.get("api", [])
            with_api += bool(api)
            for api_id in api:
                if api_id not in known:
                    unknown.add(api_id)
                elif applicable(api_id, mode):
                    touched.add((api_id, mode))
    if unknown and not args.allow_unknown:
        print(f"coverage: harness error: {len(unknown)} api id(s) not in the universe: {sorted(unknown)[:20]}",
              file=sys.stderr)
        return 2

    pairs = [(i, m) for i in all_ids for m in MODES if applicable(i, m)]
    untouched_pairs = sorted(f"{i}#{m}" for i, m in pairs if (i, m) not in touched)
    touched_any = {i for i, _ in touched}

    def cell(n, t):
        return {"touched": n, "total": t, "pct": round(100.0 * n / t, 1) if t else 0.0}

    per_mode = {}
    for m in MODES:
        ids_m = [i for i in all_ids if applicable(i, m)]
        per_mode[m] = cell(sum((i, m) in touched for i in ids_m), len(ids_m))
    per_family = {}
    for fam in FAMILIES:
        ids_f = [i for i in all_ids if family_of(i) == fam]
        if not ids_f:
            continue
        entry = {m: cell(sum((i, m) in touched for i in ids_f if applicable(i, m)),
                         sum(applicable(i, m) for i in ids_f)) for m in MODES}
        entry["all"] = cell(sum(i in touched_any for i in ids_f), len(ids_f))
        per_family[fam] = entry
    overall = cell(len(touched_any), len(all_ids))
    untouched_any = sorted(i for i in all_ids if i not in touched_any)
    untouched_by_mode = {m: sorted(i for i in all_ids if applicable(i, m) and (i, m) not in touched)
                         for m in MODES}

    result = {
        "universe": {"u1": len(uni["u1"]), "u2": len(uni["u2"]), "u3": len(uni["u3"]), "u4": len(uni["u4"]),
                     "total": len(all_ids), "node_major": node_major, "pairs": len(pairs)},
        "probes": sum(m.get("probes", 0) for m in manifests), "probes_with_api": with_api,
        "unknown_api_ids": sorted(unknown),
        "overall": overall, "per_mode": per_mode, "per_family": per_family,
        "untouched_any_mode": untouched_any, "untouched_by_mode": untouched_by_mode,
    }

    if args.write_baseline:
        with open(args.baseline, "w", encoding="utf-8") as fh:
            json.dump({"note": "untouched id#mode pairs; shrink-only. Regenerate with coverage.py "
                               "--write-baseline only when a change knowingly adds untouched API surface.",
                       "untouched": untouched_pairs}, fh, indent=1)
            fh.write("\n")
        print(f"coverage: wrote baseline with {len(untouched_pairs)} untouched pairs to {args.baseline}")

    rc = 0
    became_touched, violations, stale = [], [], []
    if not args.no_ratchet and not args.write_baseline:
        if node_major not in (None, NODE_MAJOR):
            print(f"coverage: ratchet skipped, node major {node_major} (baseline is for node {NODE_MAJOR})")
        else:
            try:
                with open(args.baseline, encoding="utf-8") as fh:
                    base = set(json.load(fh)["untouched"])
            except (OSError, ValueError, KeyError) as exc:
                print(f"coverage: harness error: baseline {args.baseline}: {exc}", file=sys.stderr)
                return 2
            cur = set(untouched_pairs)
            violations = sorted(cur - base)
            became_touched = sorted(base - cur)
            stale = sorted(p for p in base - cur if p.split("#")[0] not in known)
            if violations:
                rc = 1
                print(f"coverage: RATCHET: {len(violations)} untouched pair(s) not in the baseline:")
                for p in violations[:50]:
                    print(f"  {p}")
            if became_touched:
                print(f"coverage: {len(became_touched)} baseline pair(s) became touched (drop them with "
                      "--write-baseline):")
                for p in became_touched[:50]:
                    print(f"  {p}")
            if stale:
                print(f"coverage: {len(stale)} baseline pair(s) name ids no longer in the universe")
    result["ratchet"] = {"violations": violations, "became_touched": became_touched, "rc": rc}

    out_dir = args.out or os.path.dirname(os.path.abspath(args.manifest[0]))
    os.makedirs(out_dir, exist_ok=True)
    with open(os.path.join(out_dir, "coverage.json"), "w", encoding="utf-8") as fh:
        json.dump(result, fh, indent=1, sort_keys=True)
        fh.write("\n")

    lines = [f"## API coverage (universe {len(all_ids)} ids: table {len(uni['u1'])}, globals {len(uni['u2'])}, "
             f"modules {len(uni['u3'])}, hand {len(uni['u4'])}; probes {result['probes']}, with api "
             f"{with_api})", "",
             "Cells: touched/applicable. `all` = touched in any mode.", "",
             "| family | " + " | ".join(MODES) + " | all |",
             "|---|" + "---:|" * (len(MODES) + 1)]
    for fam, entry in per_family.items():
        cells = [f"{entry[m]['touched']}/{entry[m]['total']}" for m in MODES] + [
            f"{entry['all']['touched']}/{entry['all']['total']}"]
        lines.append(f"| {fam} | " + " | ".join(cells) + " |")
    lines.append("| **total** | " + " | ".join(f"{per_mode[m]['touched']}/{per_mode[m]['total']}" for m in MODES)
                 + f" | {overall['touched']}/{overall['total']} |")
    lines.append("")
    if args.summary:
        with open(args.summary, "a", encoding="utf-8") as fh:
            fh.write("\n".join(lines) + "\n")
    print("\n".join(lines))
    print(f"coverage: {out_dir}/coverage.json (rc {rc})")
    return rc


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
