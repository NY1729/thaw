#!/usr/bin/env python3
"""JIT coverage, stage 0 (PR 1): turns the logs of collect.sh into jit-coverage.tsv and jit-coverage.md.

usage: python3 -I tools/jit-coverage/run.py LOGS_DIR [--out-dir DIR]

One TSV row per program and variant (default = as shipped, infer = THAW_JS_INFER=1 at build).
The interpreter share is the thaw-js-profile-total line: interp_ms / wall_ms, where wall is the
time since the first interpreter entry of the profiled run. Stdlib only; the rendering is in
render(), which the unit test calls on the committed fixtures in testdata/.
"""

import argparse
import glob
import os
import re
import sys

VARIANTS = ("default", "infer")
SHARE_THRESHOLD = 0.40  # stage-0 note: programs whose interpreter share is under this are the "absorbable" set
TSV_COLUMNS = [
    "program", "variant", "build_ok", "build_ms", "stdout_ok", "profile_stdout_ok", "wall_ms",
    "interp_ms", "profile_wall_ms", "interp_share", "entry_rows", "typed_rows", "entry_calls",
    "typed_calls", "top_key", "top_calls", "top_ms", "top_share", "jit_exports", "jit_calls",
    "clones", "clones_direct", "clones_guarded", "clones_rejected",
]

JIT_DECL = re.compile(r"__thaw_typed_jit_[0-9a-f]+")


def parse_meta(text):
    """key=value lines (collect.sh) -> dict. Later lines win."""
    meta = {}
    for line in text.splitlines():
        key, sep, value = line.partition("=")
        if sep:
            meta[key.strip()] = value.strip()
    return meta


def parse_profile(text):
    """The counter lines of a THAW_JS_PROFILE=1 THAW_JIT_PROFILE=1 run (other stderr lines are ignored).

    thaw-js-profile<TAB><key><TAB><calls><TAB><ms>, rows sorted by ms (descending) by the binary;
    thaw-js-profile-total<TAB><interp_ms><TAB><wall_ms>; thaw-jit-profile<TAB><symbol><TAB><calls>.
    """
    entries, total, jit = [], None, []
    for line in text.splitlines():
        parts = line.split("\t")
        if parts[0] == "thaw-js-profile" and len(parts) == 4:
            try:
                entries.append((parts[1], int(parts[2]), float(parts[3])))
            except ValueError:
                pass
        elif parts[0] == "thaw-js-profile-total" and len(parts) == 3:
            try:
                total = (float(parts[1]), float(parts[2]))
            except ValueError:
                pass
        elif parts[0] == "thaw-jit-profile" and len(parts) == 3:
            try:
                jit.append((parts[1], int(parts[2])))
            except ValueError:
                pass
    entries.sort(key=lambda row: -row[2])
    return {"entries": entries, "total": total, "jit": jit}


def parse_inspect(text):
    """Counts from `thaw inspect` output: js-infer-aot lines (clones)."""
    counts = {"clones": 0, "clones_direct": 0, "clones_guarded": 0, "clones_rejected": 0}
    for line in text.splitlines():
        if line.startswith("js-infer-aot: "):
            counts["clones"] += 1
        elif line.startswith("js-infer-aot-direct: "):
            counts["clones_direct"] += 1
        elif line.startswith("js-infer-aot-guarded: "):
            counts["clones_guarded"] += 1
        elif line.startswith("js-infer-aot-rejected: "):
            counts["clones_rejected"] += 1
    return counts


def parse_shim(text):
    """Distinct typed JIT symbols declared by the build's registry shim."""
    return len(set(JIT_DECL.findall(text)))


def _read(path):
    try:
        with open(path, encoding="utf-8", errors="replace") as handle:
            return handle.read()
    except OSError:
        return ""


def variant_row(logs_dir, name, variant):
    """One TSV row (dict of column -> string) for a program and variant. Missing logs give blanks and 0."""
    base = os.path.join(logs_dir, f"{name}.{variant}")
    meta = parse_meta(_read(base + ".meta"))
    build_ok = meta.get("build_ok", "0") == "1"
    row = {column: "" for column in TSV_COLUMNS}
    row.update({"program": name, "variant": variant, "build_ok": "1" if build_ok else "0",
                "build_ms": meta.get("build_ms", ""), "stdout_ok": "1" if meta.get("stdout_ok") == "1" else "0",
                "profile_stdout_ok": "1" if meta.get("profile_stdout_ok") == "1" else "0",
                "wall_ms": meta.get("wall_ms", "")})
    if not build_ok:
        return row
    inspect = parse_inspect(_read(base + ".inspect"))
    row.update({key: str(value) for key, value in inspect.items()})
    row["jit_exports"] = str(parse_shim(_read(base + ".shim.ts")))

    profile = parse_profile(_read(base + ".profile"))
    entries = profile["entries"]
    row["entry_rows"] = str(sum(1 for key, _, _ in entries if key.startswith("entry:")))
    row["typed_rows"] = str(sum(1 for key, _, _ in entries if not key.startswith("entry:")))
    row["entry_calls"] = str(sum(calls for key, calls, _ in entries if key.startswith("entry:")))
    row["typed_calls"] = str(sum(calls for key, calls, _ in entries if not key.startswith("entry:")))
    row["jit_calls"] = str(sum(count for _, count in profile["jit"]))
    if entries:
        key, calls, ms = entries[0]
        row.update({"top_key": key, "top_calls": str(calls), "top_ms": f"{ms:.3f}"})
    if profile["total"] is not None:
        interp_ms, wall_ms = profile["total"]
        row["interp_ms"] = f"{interp_ms:.3f}"
        row["profile_wall_ms"] = f"{wall_ms:.3f}"
        if wall_ms > 0:
            row["interp_share"] = f"{interp_ms / wall_ms:.4f}"
            if entries:
                row["top_share"] = f"{entries[0][2] / wall_ms:.4f}"
    return row


def programs(logs_dir):
    """Program names from the `<name>.meta` files (names contain no dots), sorted."""
    names = []
    for path in glob.glob(os.path.join(logs_dir, "*.meta")):
        base = os.path.basename(path)[: -len(".meta")]
        if "." not in base:
            names.append(base)
    return sorted(names)


def _share(row):
    value = row["interp_share"]
    return float(value) if value else None


def summarize(rows):
    """Corpus-level numbers, per variant."""
    summary = {}
    for variant in VARIANTS:
        mine = [row for row in rows if row["variant"] == variant]
        built = [row for row in mine if row["build_ok"] == "1"]
        profiled = [row for row in built if row["interp_share"]]
        under = sorted(row["program"] for row in profiled if _share(row) < SHARE_THRESHOLD)
        zero_entry = sorted(row["program"] for row in profiled if row["entry_rows"] == "0")
        summary[variant] = {
            "programs": len(mine),
            "built": len(built),
            "stdout_ok": sum(1 for row in built if row["stdout_ok"] == "1"),
            "profiled": len(profiled),
            "under_share": under,
            "zero_entry": zero_entry,
            "clones_total": sum(int(row["clones"] or 0) for row in built),
            "programs_with_clones": sorted(row["program"] for row in built if int(row["clones"] or 0) > 0),
            "failed": sorted(row["program"] for row in mine if row["build_ok"] != "1" or row["stdout_ok"] != "1"),
        }
    return summary


def _fmt(value, places=3):
    return "" if value in (None, "") else f"{float(value):.{places}f}"


def render_tsv(rows):
    lines = ["\t".join(TSV_COLUMNS)]
    for row in rows:
        lines.append("\t".join(row[column] for column in TSV_COLUMNS))
    return "\n".join(lines) + "\n"


def render_md(rows, summary):
    by_key = {(row["program"], row["variant"]): row for row in rows}
    names = sorted({row["program"] for row in rows})
    out = [
        "# JIT coverage, stage 0 (PR 1)",
        "",
        "Non-gating. Question: how much of the interpreter time could a JIT or JS->HIR clone absorb?",
        "Interpreter share = thaw-js-profile-total interp_ms / wall_ms of the profiled run.",
        "default = as shipped; INFER = THAW_JS_INFER=1 at build (AOT clones).",
        "",
        "| program | interp share (default) | interp share (INFER) | clones (INFER) | top entry (default) | top calls (default) | top share (default) |",
        "|---|---|---|---|---|---|---|",
    ]
    for name in names:
        default = by_key.get((name, "default"), {})
        infer = by_key.get((name, "infer"), {})

        def share(row):
            if row.get("build_ok") != "1":
                return "build failed"
            return _fmt(row.get("interp_share"), 3) or "n/a"

        built = default.get("build_ok") == "1"
        out.append("| {} | {} | {} | {} | {} | {} | {} |".format(
            name,
            share(default),
            share(infer),
            infer.get("clones") or "0",
            (default.get("top_key") or "none") if built else "",
            (default.get("top_calls") or "0") if built else "",
            (_fmt(default.get("top_share"), 3) or "n/a") if built else "",
        ))
    out.append("")
    out.append("## Corpus level")
    out.append("")
    for variant in VARIANTS:
        s = summary[variant]
        out.append(f"### {variant}")
        out.append("")
        out.append(f"- programs: {s['programs']}, built: {s['built']}, stdout equal to Node: {s['stdout_ok']}")
        out.append(f"- profiled runs with a counter total: {s['profiled']}")
        out.append(f"- interpreter share under {SHARE_THRESHOLD:.0%}: {len(s['under_share'])}"
                   + (f" ({', '.join(s['under_share'])})" if s["under_share"] else ""))
        out.append(f"- programs with zero entry rows in the profiled run: {len(s['zero_entry'])}"
                   + (f" ({', '.join(s['zero_entry'])})" if s["zero_entry"] else ""))
        if variant == "infer":
            out.append(f"- AOT clones: {s['clones_total']} in {len(s['programs_with_clones'])} programs"
                       + (f" ({', '.join(s['programs_with_clones'])})" if s["programs_with_clones"] else ""))
        if s["failed"]:
            out.append(f"- not built or stdout differs from Node: {', '.join(s['failed'])}")
        out.append("")
    out.append("Stop rule (designs/jit-coverage-stage0.md): stop widening the JIT if under 20% of in-loop calls "
               "are absorbable on at least 18 programs. This table measures interpreter share only; "
               "absorbability needs the static classification of the hot calls.")
    return "\n".join(out) + "\n"


def render(logs_dir):
    """(tsv text, markdown text) for every program in logs_dir."""
    rows = [variant_row(logs_dir, name, variant) for name in programs(logs_dir) for variant in VARIANTS]
    return render_tsv(rows), render_md(rows, summarize(rows))


def main(argv):
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("logs_dir")
    parser.add_argument("--out-dir", default=None, help="where to write the two files (default: LOGS_DIR)")
    args = parser.parse_args(argv)
    if not os.path.isdir(args.logs_dir):
        print(f"not a directory: {args.logs_dir}", file=sys.stderr)
        return 2
    tsv, md = render(args.logs_dir)
    out_dir = args.out_dir or args.logs_dir
    os.makedirs(out_dir, exist_ok=True)
    with open(os.path.join(out_dir, "jit-coverage.tsv"), "w", encoding="utf-8") as handle:
        handle.write(tsv)
    with open(os.path.join(out_dir, "jit-coverage.md"), "w", encoding="utf-8") as handle:
        handle.write(md)
    print(md, end="")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
