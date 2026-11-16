#!/usr/bin/env python3
"""AOT-compiler vs Node differential harness.

Each probes/<family>.ts holds one-line `p(name, () => expr)` probes inside
`function main(): void { ... }`. Probes are compiled in batches (one thaw
binary per BATCH probes), run, and compared with the same file run under node.

Outcomes per probe: MATCH | COMPILE-REJECT | MISMATCH (only MISMATCH is a
correctness gap). Plain runs only report; --check gates shrink-only against
known-gaps.json. See README.md.
"""
import argparse
import concurrent.futures as cf
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

HERE = Path(__file__).resolve().parent
REPO = HERE.parent.parent
PROBE_DIR = HERE / "probes"
FALLBACK_THAW = "/home/fedora/work/thaw/target/debug/thaw"  # stale debug build, last resort

BATCH = 40
JOBS = 2
BUILD_TIMEOUT = 300
RUN_TIMEOUT = 60
ULIMIT_VM_KB = 8 * 1024 * 1024  # ulimit -v 8388608
SLICE = "app-heavytests.slice"
MEM_MAX = "8G"

MATCH = "MATCH"
COMPILE_REJECT = "COMPILE-REJECT"
MISMATCH = "MISMATCH"

# Kind of a MISMATCH (informational; the gate counts every MISMATCH the same).
TYPE = "TYPE"            # error constructor name differs (or one side threw, the other did not)
MSG = "MSG"              # same error constructor, message wording differs
OUTPUT = "OUTPUT"        # ordinary output line differs
EXIT = "EXIT"            # thaw exited non-zero
NO_OUTPUT = "NO-OUTPUT"  # thaw printed no line for the probe

BASELINE_SCHEMA = 1
DEFAULT_BASELINE = HERE / "known-gaps.json"
GAP_CLASSES = ("MISMATCH", "REJECT")
WAIVER_ID_RE = re.compile(r"^W\d+$")

MAIN_LINE = "function main(): void {"
PROBE_RE = re.compile(r'^  p\("([A-Za-z0-9_]+)", .*\);$')
PROBE_START_RE = re.compile(r'^  p\(')
OUT_RE = re.compile(r"^([A-Za-z0-9_]+) (=>|THROW) ")
THROW_RE = re.compile(r"^\S+ THROW (\S+)(?: (.*))?$", re.S)
STACK_RE = re.compile(r"^\s*at \S")

# Output normalisers, applied to both the node and the thaw block before comparing.
# Keep this list short: every entry hides a difference, so each needs a reason.
NORMALISERS = [
    ("abs-path", re.compile(r"(?<![\w.@-])/(?:[\w.@+-]+/){2,}[\w.@+-]*"), "<path>",
     "absolute paths (two or more segments) -> <path>; the build dir is scratch"),
    ("null-proto", re.compile(r"\[Object: null prototype\] "), "",
     "node prints `[Object: null prototype] {..}`, thaw prints `{..}`: formatting only, prefix dropped"),
    ("timing", re.compile(r"\b\d+(?:\.\d+)?ms\b"), "<ms>", "elapsed-time figures"),
    ("address", re.compile(r"\b0x[0-9a-fA-F]{6,}\b"), "<addr>",
     "object addresses (__thaw_js_handle_id__ is NOT normalised: it is a bug)"),
]


class HarnessError(Exception):
    pass


class Family:
    def __init__(self, name, header, probes):
        self.name = name
        self.header = header  # lines before `function main()`
        self.probes = probes  # list of (probe name, source line)

    @property
    def names(self):
        return [n for n, _ in self.probes]

    def render(self, probes, node=False):
        lines = list(self.header) + [MAIN_LINE] + [src for _, src in probes] + ["}"]
        if node:
            lines.append("main();")  # thaw auto-invokes main(); node does not
        return "\n".join(lines) + "\n"


def parse_family(path):
    lines = path.read_text(encoding="utf-8").split("\n")
    if MAIN_LINE not in lines:
        raise HarnessError(f"{path.name}: no `{MAIN_LINE}` line")
    start = lines.index(MAIN_LINE)
    try:
        end = lines.index("}", start)
    except ValueError:
        raise HarnessError(f"{path.name}: main() is not closed by a column-0 `}}`")
    probes, seen = [], set()
    for line in lines[start + 1 : end]:
        if not line.strip():
            continue
        m = PROBE_RE.match(line)
        if not m:
            if PROBE_START_RE.match(line):
                raise HarnessError(f"{path.name}: probe is not a one-line p(): {line[:80]}")
            raise HarnessError(f"{path.name}: stray line in main(): {line[:80]}")
        if m.group(1) in seen:
            raise HarnessError(f"{path.name}: duplicate probe name {m.group(1)}")
        seen.add(m.group(1))
        probes.append((m.group(1), line))
    return Family(path.stem, lines[:start], probes)


def load_families(only):
    fams = [parse_family(p) for p in sorted(PROBE_DIR.glob("*.ts"))]
    if only:
        fams = [f for f in fams if f.name in only]
        missing = only - {f.name for f in fams}
        if missing:
            raise HarnessError(f"unknown family: {', '.join(sorted(missing))}")
    return fams


def chunked(seq, n):
    return [seq[i : i + n] for i in range(0, len(seq), n)]


def bisect_compile(probes, build):
    """build(probes) -> (ok, err, binary). Returns (groups, rejects).

    groups: [(probes, binary)] for every sub-chunk that compiled.
    rejects: [(probe name, reason)] for each probe that failed alone.
    """
    ok, err, binary = build(probes)
    if ok:
        return [(probes, binary)], []
    if len(probes) == 1:
        return [], [(probes[0][0], err)]
    mid = len(probes) // 2
    g1, r1 = bisect_compile(probes[:mid], build)
    g2, r2 = bisect_compile(probes[mid:], build)
    return g1 + g2, r1 + r2


def split_outputs(text, names):
    """Map probe name -> normalised block of output lines (None if absent)."""
    blocks, cur = {}, None
    for line in text.split("\n"):
        m = OUT_RE.match(line)
        if m and m.group(1) in names:
            cur = m.group(1)
            blocks[cur] = [line]
        elif cur is not None:
            blocks[cur].append(line)
    return {k: normalise("\n".join(v)) for k, v in blocks.items()}


def normalise_stack(lines):
    """Stack frames after the header line become `    at <frame>`; repeats collapse to one."""
    out = []
    for i, ln in enumerate(lines):
        if i > 0 and STACK_RE.match(ln):
            ln = "    at <frame>"
            if out and out[-1] == ln:
                continue
        out.append(ln)
    return out


def normalise(text):
    lines = [ln.rstrip() for ln in text.replace("\r\n", "\n").split("\n")]
    lines = normalise_stack(lines)
    text = "\n".join(lines)
    for _, rx, repl, _ in NORMALISERS:
        text = rx.sub(repl, text)
    lines = text.split("\n")
    while lines and not lines[-1]:
        lines.pop()
    return "\n".join(lines)


def error_kind(node_text, thaw_text):
    """Kind of an output difference: error type, error wording, or plain output."""
    n, t = THROW_RE.match(node_text), THROW_RE.match(thaw_text)
    if n is None and t is None:
        return OUTPUT
    if n is None or t is None or n.group(1) != t.group(1):
        return TYPE
    if (n.group(2) or "") != (t.group(2) or ""):
        return MSG
    return OUTPUT


def verdict(node_text, thaw_text, rc):
    """Classify one probe that compiled and ran. Returns (outcome, reason, kind)."""
    if rc != 0:
        return MISMATCH, f"exit code {rc} (node 0)", EXIT
    if thaw_text is None:
        return MISMATCH, "no output line", NO_OUTPUT
    if thaw_text == node_text:
        return MATCH, "", ""
    return MISMATCH, "output differs", error_kind(node_text, thaw_text)


def first_error(out):
    lines = [ln for ln in out.split("\n") if ln.strip()]
    for ln in lines:
        if ln.lower().startswith("error"):
            return ln.strip()[:200]
    return lines[-1].strip()[:200] if lines else "build failed"


def wrap(argv, timeout, systemd, cwd, tmpdir, vm_cap=True):
    # node's type stripper (amaro, WASM) cannot start under `ulimit -v`, so the
    # node oracle is exempt; it keeps the timeout and the systemd MemoryMax cap.
    inner = ["timeout", "-k", "5", str(timeout)]
    if vm_cap:
        inner += ["sh", "-c", f'ulimit -v {ULIMIT_VM_KB} && exec "$@"', "sh"]
    inner += list(argv)
    if not systemd:
        return inner
    return [
        "systemd-run", "--user", f"--slice={SLICE}", "-p", f"MemoryMax={MEM_MAX}",
        "--wait", "--pipe", "--collect",
        f"--working-directory={cwd}", f"--setenv=TMPDIR={tmpdir}",
        *inner,
    ]


class Runner:
    def __init__(self, thaw, work, systemd):
        self.thaw = thaw
        self.work = Path(work)
        self.systemd = systemd
        self.tmpdir = os.environ.get("TMPDIR") or tempfile.gettempdir()

    def exec(self, argv, timeout, vm_cap=True):
        return subprocess.run(
            wrap(argv, timeout, self.systemd, self.work, self.tmpdir, vm_cap),
            capture_output=True, text=True, errors="replace", cwd=self.work,
        )

    def build_fn(self, fam):
        """Return build(probes) -> (ok, err, binary) for bisect_compile."""
        def build(ps):
            tag = f"{fam.name}-{ps[0][0]}-{len(ps)}"
            src = self.work / f"{tag}.ts"
            out = self.work / f"{tag}.bin"
            src.write_text(fam.render(ps), encoding="utf-8")
            try:
                r = self.exec([str(self.thaw), "build", str(src), "-o", str(out)], BUILD_TIMEOUT)
            except subprocess.TimeoutExpired:
                return False, "build timeout", None
            if r.returncode == 0 and out.exists():
                return True, "", str(out)
            return False, first_error(r.stdout + r.stderr), None
        return build

    def run_bin(self, binary):
        r = self.exec([binary], RUN_TIMEOUT)
        return r.returncode, r.stdout, r.stderr

    def node_oracle(self, fam):
        src = self.work / f"node-{fam.name}.ts"
        src.write_text(fam.render(fam.probes, node=True), encoding="utf-8")
        r = self.exec(["node", str(src)], RUN_TIMEOUT, vm_cap=False)
        if r.returncode != 0:
            raise HarnessError(f"node oracle failed for {fam.name} (rc {r.returncode}): {r.stderr[-300:]}")
        blocks = split_outputs(r.stdout, set(fam.names))
        missing = [n for n in fam.names if n not in blocks]
        if missing:
            raise HarnessError(f"node printed no line for {fam.name} probes {missing[:5]}")
        return blocks

    def chunk_results(self, fam, chunk, node_blocks):
        """Compile (bisecting), run, and classify one chunk of probes."""
        build = self.build_fn(fam)
        groups, rejects = bisect_compile(chunk, build)
        results = [
            {"name": n, "outcome": COMPILE_REJECT, "reason": why, "node": None, "thaw": None}
            for n, why in rejects
        ]
        for probes, binary in groups:
            names = {n for n, _ in probes}
            rc, out, _ = self.run_bin(binary)
            if rc == 0:
                blocks = split_outputs(out, names)
                for n, _ in probes:
                    outcome, why, kind = verdict(node_blocks[n], blocks.get(n), 0)
                    results.append(self.entry(n, outcome, why, node_blocks[n], blocks.get(n), kind))
                continue
            # Batch crashed or failed: rerun each probe alone to attribute it.
            for probe in probes:
                single, why_rej = bisect_compile([probe], build)
                if not single:
                    results.append(self.entry(probe[0], COMPILE_REJECT, why_rej[0][1], None, None))
                    continue
                prc, pout, _ = self.run_bin(single[0][1])
                text = split_outputs(pout, {probe[0]}).get(probe[0])
                outcome, why, kind = verdict(node_blocks[probe[0]], text, prc)
                results.append(self.entry(probe[0], outcome, why, node_blocks[probe[0]], text, kind))
        return results

    @staticmethod
    def entry(name, outcome, why, node_text, thaw_text, kind=""):
        return {"name": name, "outcome": outcome, "reason": why, "kind": kind,
                "node": node_text, "thaw": thaw_text}


def tool_version(runner, argv):
    try:
        r = runner.exec(argv, 60)
        return (r.stdout or r.stderr).strip().split("\n")[0]
    except (subprocess.TimeoutExpired, OSError) as e:
        return f"unavailable ({e})"


def resolve_thaw(arg):
    if arg:
        return Path(arg)
    env = os.environ.get("THAW")
    if env:
        return Path(env)
    repo_bin = REPO / "target" / "release" / "thaw"
    if repo_bin.exists():
        return repo_bin
    print(f"note: no target/release/thaw; falling back to {FALLBACK_THAW}", file=sys.stderr)
    return Path(FALLBACK_THAW)


def probe_id(family, name):
    return f"{family}/{name}"


def current_state(pairs):
    """pairs: [(family name, results)]. Returns (ids run, gap entries by id).

    Gap entries exist only for MISMATCH and COMPILE-REJECT probes (class MISMATCH/REJECT).
    """
    ids, gaps = set(), {}
    for fam, results in pairs:
        for r in results:
            pid = probe_id(fam, r["name"])
            ids.add(pid)
            if r["outcome"] == MISMATCH:
                gaps[pid] = {"class": "MISMATCH", "kind": r["kind"], "reason": r["reason"]}
            elif r["outcome"] == COMPILE_REJECT:
                gaps[pid] = {"class": "REJECT", "reason": r["reason"]}
    return ids, gaps


def validate_gaps(gaps):
    if not isinstance(gaps, dict):
        raise HarnessError("gaps must be an object")
    for pid, e in gaps.items():
        where = f"{pid}"
        if not isinstance(e, dict) or e.get("class") not in GAP_CLASSES:
            raise HarnessError(f"{where}: class must be one of {GAP_CLASSES}")
        if not isinstance(e.get("reason"), str) or not e["reason"]:
            raise HarnessError(f"{where}: reason must be a non-empty string")
        if e["class"] == "MISMATCH" and e.get("kind") not in (TYPE, MSG, OUTPUT, EXIT, NO_OUTPUT):
            raise HarnessError(f"{where}: MISMATCH needs a kind (TYPE|MSG|OUTPUT|EXIT|NO-OUTPUT)")
        w = e.get("waiver")
        if w is not None:
            if not isinstance(w, dict) or not WAIVER_ID_RE.match(str(w.get("id", ""))):
                raise HarnessError(f"{where}: waiver.id must match W<number>")
            if not isinstance(w.get("reason"), str) or not w["reason"]:
                raise HarnessError(f"{where}: waiver.reason must be a non-empty string")


def load_baseline(path):
    try:
        doc = json.loads(Path(path).read_text(encoding="utf-8"))
    except (OSError, ValueError) as e:
        raise HarnessError(f"cannot read baseline {path}: {e}")
    if doc.get("schema") != BASELINE_SCHEMA:
        raise HarnessError(f"baseline {path}: schema must be {BASELINE_SCHEMA}")
    validate_gaps(doc.get("gaps", {}))
    return doc["gaps"]


def write_baseline(path, gaps, thaw_ver, node_ver):
    validate_gaps(gaps)
    doc = {"schema": BASELINE_SCHEMA, "thaw_version": thaw_ver, "node_version": node_ver,
           "gaps": {k: gaps[k] for k in sorted(gaps)}}
    Path(path).parent.mkdir(parents=True, exist_ok=True)
    Path(path).write_text(json.dumps(doc, indent=1, ensure_ascii=False) + "\n", encoding="utf-8")


def merge_baseline(old, cur, run_fams):
    """Entries of families not run are kept; run families take the current state.

    A waiver survives a rerun only while the probe keeps the same gap class.
    """
    out = {i: e for i, e in old.items() if i.split("/", 1)[0] not in run_fams}
    for i, e in cur.items():
        e = dict(e)
        w = old.get(i, {}).get("waiver")
        if w and old[i]["class"] == e["class"]:
            e["waiver"] = w
        out[i] = e
    return out


def gate(base, cur, run_fams):
    """Shrink-only check. Only a new MISMATCH fails; REJECT is informational.

    base: baseline gap entries. cur: current gap entries (probes that now match are absent).
    fixed: baseline entries of run families whose class changed or vanished. They pass.
    """
    new_mismatch = sorted(i for i, e in cur.items()
                          if e["class"] == "MISMATCH" and base.get(i, {}).get("class") != "MISMATCH")
    new_reject = sorted(i for i, e in cur.items()
                        if e["class"] == "REJECT" and base.get(i, {}).get("class") != "REJECT")
    fixed = []
    for i, e in sorted(base.items()):
        if i.split("/", 1)[0] not in run_fams:
            continue
        now = cur.get(i, {}).get("class")
        if now != e["class"]:
            fixed.append((i, e["class"], now))
    open_m = [i for i, e in cur.items() if e["class"] == "MISMATCH" and "waiver" not in base.get(i, {})]
    waived = [i for i, e in cur.items() if e["class"] == "MISMATCH" and "waiver" in base.get(i, {})]
    return {"pass": not new_mismatch, "new_mismatch": new_mismatch, "new_reject": new_reject,
            "fixed": fixed, "open_mismatch": len(open_m), "waived": len(waived)}


def summarise(families):
    rows, totals = [], {MATCH: 0, COMPILE_REJECT: 0, MISMATCH: 0}
    for fam, results in families:
        counts = {MATCH: 0, COMPILE_REJECT: 0, MISMATCH: 0}
        for r in results:
            counts[r["outcome"]] += 1
        for k in totals:
            totals[k] += counts[k]
        rows.append((fam.name, len(results), counts))
    return rows, totals


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--thaw", help="thaw binary (default: $THAW, then repo target/release/thaw, then fallback)")
    ap.add_argument("--family", action="append", default=[], help="family name (repeatable, comma ok)")
    ap.add_argument("--batch", type=int, default=BATCH)
    ap.add_argument("--jobs", type=int, default=JOBS)
    ap.add_argument("--out", help="report JSON path (default: target/aotdiff/report.json)")
    ap.add_argument("--keep", action="store_true", help="keep the scratch work dir")
    ap.add_argument("--no-systemd", action="store_true", help="do not wrap commands in systemd-run")
    mode = ap.add_mutually_exclusive_group()
    mode.add_argument("--check", action="store_true",
                      help="shrink-only gate vs --baseline: exit 1 if a MISMATCH is not in the baseline")
    mode.add_argument("--update-baseline", action="store_true",
                      help="write --baseline from this run (run against a CURRENT build only)")
    ap.add_argument("--baseline", default=str(DEFAULT_BASELINE), help="known-gaps.json path")
    args = ap.parse_args(argv)

    only = {f for spec in args.family for f in spec.split(",") if f}
    try:
        fams = load_families(only)
    except HarnessError as e:
        print(f"error: {e}", file=sys.stderr)
        return 2
    thaw = resolve_thaw(args.thaw)
    if not thaw.exists():
        print(f"error: thaw binary not found: {thaw}", file=sys.stderr)
        return 2
    if args.update_baseline and thaw == Path(FALLBACK_THAW) and not (args.thaw or os.environ.get("THAW")):
        print("error: refusing --update-baseline with the stale fallback binary; pass --thaw", file=sys.stderr)
        return 2
    if args.check and not Path(args.baseline).exists():
        print(f"error: no baseline at {args.baseline} (generate it with --update-baseline)", file=sys.stderr)
        return 2
    systemd = not args.no_systemd and shutil.which("systemd-run") is not None
    tmp_root = os.environ.get("TMPDIR") or tempfile.gettempdir()
    work = Path(tempfile.mkdtemp(prefix="aotdiff-", dir=tmp_root))
    runner = Runner(thaw, work, systemd)
    out_path = Path(args.out) if args.out else REPO / "target" / "aotdiff" / "report.json"

    try:
        thaw_ver = tool_version(runner, [str(thaw), "--version"])
        node_ver = tool_version(runner, ["node", "--version"])
        print(f"thaw: {thaw} ({thaw_ver})\nnode: {node_ver}\nbatch: {args.batch}, jobs: {args.jobs}, systemd: {systemd}")
        with cf.ThreadPoolExecutor(max_workers=args.jobs) as pool:
            node_maps = dict(zip([f.name for f in fams],
                                 pool.map(runner.node_oracle, fams)))
            jobs = [(f, c) for f in fams for c in chunked(f.probes, args.batch)]
            futs = {pool.submit(runner.chunk_results, f, c, node_maps[f.name]): f for f, c in jobs}
            per_fam = {f.name: [] for f in fams}
            for fut in cf.as_completed(futs):
                per_fam[futs[fut].name].extend(fut.result())
        families = [(f, sorted(per_fam[f.name], key=lambda r: f.names.index(r["name"]))) for f in fams]
    except HarnessError as e:
        print(f"error: {e}", file=sys.stderr)
        return 2
    finally:
        if args.keep:
            print(f"work dir kept: {work}")
        else:
            shutil.rmtree(work, ignore_errors=True)

    rows, totals = summarise(families)
    print()
    print(f"{'family':<20}{'probes':>8}{'MATCH':>8}{'COMPILE-REJECT':>16}{'MISMATCH':>10}")
    for name, n, c in rows:
        print(f"{name:<20}{n:>8}{c[MATCH]:>8}{c[COMPILE_REJECT]:>16}{c[MISMATCH]:>10}")
    print(f"{'TOTAL':<20}{sum(r[1] for r in rows):>8}{totals[MATCH]:>8}{totals[COMPILE_REJECT]:>16}{totals[MISMATCH]:>10}")
    print("\nMISMATCH (correctness gaps):")
    for fam, results in families:
        for r in results:
            if r["outcome"] == MISMATCH:
                print(f"  {fam.name}/{r['name']} [{r['kind']}]: {r['reason']}")
                print(f"    node: {(r['node'] or '')[:160]!r}")
                print(f"    thaw: {(r['thaw'] or '')[:160]!r}")

    report = {
        "thaw": str(thaw), "thaw_version": thaw_ver, "node_version": node_ver,
        "batch": args.batch, "totals": totals,
        "families": {
            fam.name: {"probes": len(results),
                       **{k: sum(1 for r in results if r["outcome"] == k) for k in totals},
                       "results": results}
            for fam, results in families
        },
    }
    out_path.parent.mkdir(parents=True, exist_ok=True)
    out_path.write_text(json.dumps(report, ensure_ascii=False, indent=1) + "\n", encoding="utf-8")
    print(f"\nreport: {out_path}")

    pairs = [(f.name, results) for f, results in families]
    run_fams = {f.name for f in fams}
    ids, cur = current_state(pairs)
    base_path = Path(args.baseline)
    if args.update_baseline:
        old = load_baseline(base_path) if base_path.exists() else {}
        merged = merge_baseline(old, cur, run_fams)
        write_baseline(base_path, merged, thaw_ver, node_ver)
        print(f"baseline: {base_path} ({len(merged)} gaps)")
        return 0
    if args.check:
        base = load_baseline(base_path)
        g = gate(base, cur, run_fams)
        print_gate(g, base_path)
        return 0 if g["pass"] else 1
    return 0


def print_gate(g, base_path):
    print(f"\ncheck vs {base_path}")
    for i in g["new_mismatch"]:
        print(f"  NEW MISMATCH (gating): {i}")
    for i, was, now in g["fixed"]:
        print(f"  fixed since baseline: {i} ({was} -> {now or 'MATCH'}); delete it with --update-baseline")
    print(f"  new COMPILE-REJECT (not gating): {len(g['new_reject'])}")
    print(f"  open MISMATCH: {g['open_mismatch']}, waived: {g['waived']}")
    print("RESULT: " + ("PASS" if g["pass"] else "FAIL"))


if __name__ == "__main__":
    sys.exit(main())
