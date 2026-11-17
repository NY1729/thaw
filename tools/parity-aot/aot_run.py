#!/usr/bin/env python3
"""Node reference + thaw AOT build/run + classification for the tools/parity-aot probe programs.

Driven by run.sh (which runs gen_probes.py first). For each program in the manifest:
  1. Node runs <program>.mts once (the oracle; a Node failure or a missing header is a harness error, exit 2).
  2. For each engine (THAW_JS_ENGINE value), thaw builds the program with `thaw build <file.ts> -o <bin>`.
     - Compile diagnostic: if it names a probe line, that probe is dropped and the rest rebuilt (REJECT, the
       diagnostic text is the reason). Otherwise the probe list is bisected by halves. At most EXTRA_CAP builds
       beyond the first per program; the rest is REJECT? (unisolated).
     - Run: `timeout RUN_TIMEOUT`. A nonzero exit or a timeout means the probes after the last printed header
       are unverified; the header-less tail is rerun one probe per binary (RERUN_CAP builds). The probe that
       still fails alone is CRASH or HANG.
  3. Classes per ran probe: MATCH, MSG (same THROW name, different message), WRONG-TYPE (THROW vs value or
     different THROW name), WRONG-VALUE, LEAK (engine handle marker in output), MISSING (no output block).
  4. The gate reuses tools/parity/diff.py (--blocks, baseline, waivers, skip) on the ran probes only.
     Exit 1 from diff.py means gaps not in the baseline: reported, not a failure. Exit 2 is a harness error.
"""
import argparse
import concurrent.futures as cf
import json
import os
import re
import shlex
import shutil
import subprocess
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.abspath(os.path.join(HERE, "..", ".."))
PARITY = os.path.join(REPO, "tools", "parity")
sys.path.insert(0, HERE)
sys.path.insert(0, PARITY)
import diff as pdiff  # noqa: E402  (tools/parity/diff.py: block loader, normaliser, gate)
import gen_probes  # noqa: E402

EXTRA_CAP = 25
RERUN_CAP = 40
BUILD_TIMEOUT = int(os.environ.get("PARITY_AOT_BUILD_TIMEOUT", "900"))
RUN_TIMEOUT = int(os.environ.get("PARITY_AOT_RUN_TIMEOUT", "20"))
MARKER = "__thaw_js_handle_id__"
DIAG_RE = re.compile(r"([\w./\-]+\.ts):(\d+):\d+")
PROBE_RE = re.compile(r'^\s*p\("([^"]+)", ')
GAP_CLASSES = ("WRONG-TYPE", "WRONG-VALUE", "LEAK", "MISSING")
CLASS_ORDER = ("MATCH", "MSG", "WRONG-TYPE", "WRONG-VALUE", "LEAK", "MISSING", "CRASH", "HANG",
               "REJECT", "REJECT?", "UNVERIFIED")


class HarnessError(Exception):
    pass


def safe(name):
    return name.replace(".", "_")


def node_version(node):
    out = subprocess.run([node, "--version"], capture_output=True, text=True).stdout.strip()
    m = re.match(r"v(\d+)\.(\d+)", out)
    if not m:
        raise HarnessError(f"cannot read node version: {out!r}")
    return int(m.group(1)), int(m.group(2))


def node_stage(cfg, manifest):
    """Run every program under node; returns {program: {probe: raw block}}. Exit 2 on any node failure."""
    flags = [] if node_version(cfg.node) >= (22, 18) else ["--experimental-strip-types"]
    os.makedirs(os.path.join(cfg.out, "node"), exist_ok=True)
    blocks = {}
    for prog in manifest["programs"]:
        if cfg.only and not prog["name"].startswith(cfg.only):
            continue
        path = os.path.join(cfg.programs, prog["name"] + ".mts")
        try:
            p = subprocess.run([cfg.node, *flags, path], cwd=cfg.programs, capture_output=True, text=True,
                               timeout=120)
        except subprocess.TimeoutExpired:
            raise HarnessError(f"node timed out on {prog['name']}")
        raw = os.path.join(cfg.out, "node", prog["name"] + ".out")
        with open(raw, "w", encoding="utf-8") as fh:
            fh.write(p.stdout)
        # stdout and the exit code are the oracle; stderr carries expected noise (e.g. DEP0005 for new Buffer()).
        if p.returncode != 0:
            raise HarnessError(f"node failed on {prog['name']} (rc {p.returncode}): {p.stderr.strip()[:300]}")
        got = pdiff.load_blocks(raw)
        want = [q["name"] for q in prog["probes"]]
        missing = [n for n in want if n not in got]
        if missing:
            raise HarnessError(f"node output of {prog['name']} lacks headers: {missing[:5]}")
        blocks[prog["name"]] = {n: got[n] for n in want}
    return blocks


def classify_ran(name, node_raw, thaw_raw):
    """(class, detail) for a probe that ran under both sides. node_raw is the oracle block."""
    if thaw_raw is None:
        return "MISSING", "no output block"
    if pdiff.block_normalise(node_raw) == pdiff.block_normalise(thaw_raw):
        n1 = node_raw.split("\n", 1)[0]
        t1 = thaw_raw.split("\n", 1)[0]
        if n1.startswith("THROW") and t1.startswith("THROW") and n1 != t1:
            return "MSG", t1[:160]
        return "MATCH", ""
    if MARKER in thaw_raw and MARKER not in node_raw:
        return "LEAK", thaw_raw.split("\n", 1)[0][:160]
    if node_raw.startswith("THROW") or thaw_raw.startswith("THROW"):
        return "WRONG-TYPE", thaw_raw.split("\n", 1)[0][:160]
    return "WRONG-VALUE", thaw_raw.split("\n", 1)[0][:160]


def diag_detail(diag):
    lines = [l.strip() for l in diag.splitlines() if l.strip()]
    pick = next((l for l in lines if "error" in l.lower()), lines[0] if lines else "build failed, no diagnostic")
    return re.sub(r"\d+", "N", re.sub(r"/\S+", "<path>", pick))[:160]


class Program:
    """Builds and runs one program for one engine. Budgets are per program."""

    def __init__(self, cfg, engine, prog):
        self.cfg, self.engine, self.prog = cfg, engine, prog
        self.line_of = {p["name"]: p["line"] for p in prog["probes"]}
        self.work = os.path.join(cfg.out, engine, "work", safe(prog["name"]))
        self.env = dict(os.environ, THAW_JS_ENGINE=engine, TZ="UTC", LANG="C")
        self.builds = 0
        self.reruns = 0
        self.n = 0
        self.rejects = {}
        self.unisolated = []
        os.makedirs(self.work, exist_ok=True)
        if prog["module"]:
            shutil.copy(os.path.join(cfg.programs, "lib.ts"), os.path.join(self.work, "lib.ts"))

    def build(self, names):
        """Build one group; returns (ok, diag, stem, texts)."""
        self.builds += 1
        self.n += 1
        stem = f"g{self.n}"
        texts = gen_probes.render([self.line_of[n] for n in names], self.prog["module"], node=False).split("\n")
        src = os.path.join(self.work, stem + ".ts")
        with open(src, "w", encoding="utf-8") as fh:
            fh.write("\n".join(texts))
        cmd = self.cfg.wrap + [self.cfg.thaw, "build", src, "-o", os.path.join(self.work, stem)]
        try:
            p = subprocess.run(cmd, cwd=self.work, env=self.env, capture_output=True, text=True,
                               timeout=BUILD_TIMEOUT)
            diag = p.stdout + p.stderr
            ok = p.returncode == 0 and os.path.exists(os.path.join(self.work, stem))
        except subprocess.TimeoutExpired:
            diag, ok = "error: build timed out", False
        return ok, diag, stem, texts

    def name_from_diag(self, diag, stem, texts, names):
        for m in DIAG_RE.finditer(diag):
            if os.path.basename(m.group(1)) != stem + ".ts":
                continue
            ln = int(m.group(2))
            if 1 <= ln <= len(texts):
                pm = PROBE_RE.match(texts[ln - 1])
                if pm and pm.group(1) in names:
                    return pm.group(1)
        return None

    def solve(self, names):
        """Compiled groups as [(names, stem)]. Rejected probes go to self.rejects."""
        if not names:
            return []
        if self.builds > EXTRA_CAP:
            self.unisolated.extend(names)
            return []
        ok, diag, stem, texts = self.build(names)
        if ok:
            return [(names, stem)]
        bad = self.name_from_diag(diag, stem, texts, names)
        if bad is not None:
            self.rejects[bad] = diag_detail(diag)
            return self.solve([n for n in names if n != bad])
        if len(names) == 1:
            self.rejects[names[0]] = diag_detail(diag)
            return []
        h = len(names) // 2
        return self.solve(names[:h]) + self.solve(names[h:])

    def run(self, stem):
        """(status, blocks, rc): status OK, CRASH or HANG; blocks is the parsed output (may be partial)."""
        binary = os.path.join(self.work, stem)
        out_path = os.path.join(self.work, stem + ".out")
        try:
            p = subprocess.run([binary], cwd=self.work, env=self.env, capture_output=True, timeout=RUN_TIMEOUT)
            status, rc, data = ("OK" if p.returncode == 0 else "CRASH"), p.returncode, p.stdout
        except subprocess.TimeoutExpired as e:
            status, rc, data = "HANG", None, e.stdout or b""
        with open(out_path, "wb") as fh:
            fh.write(data)
        return status, pdiff.load_blocks(out_path), rc


def run_program(cfg, engine, prog, node_blocks):
    """Thaw side of one program: {ran: [names], thaw: {name: raw}, classes: {name: (class, detail)}, rejects...}."""
    st = Program(cfg, engine, prog)
    names_all = [p["name"] for p in prog["probes"]]
    ran, thaw, classes = [], {}, {}
    for names, stem in st.solve(names_all):
        status, blocks, rc = st.run(stem)
        if status == "OK":
            ran += names
            thaw.update({n: blocks[n] for n in names if n in blocks})
            continue
        printed = [n for n in blocks if n in names]
        verified = printed[:-1]
        ran += verified
        thaw.update({n: blocks[n] for n in verified})
        for name in [n for n in names if n not in verified]:
            if st.reruns >= RERUN_CAP:
                classes[name] = ("UNVERIFIED", "rerun cap reached")
                continue
            st.reruns += 1
            ok, diag, rstem, _ = st.build([name])
            if not ok:
                st.rejects[name] = diag_detail(diag)
                continue
            s1, b1, rc1 = st.run(rstem)
            if s1 == "OK":
                ran.append(name)
                if name in b1:
                    thaw[name] = b1[name]
            elif s1 == "HANG":
                classes[name] = ("HANG", f"timeout {RUN_TIMEOUT}s")
            else:
                classes[name] = ("CRASH", f"exit {rc1}")
    for name, detail in st.rejects.items():
        classes[name] = ("REJECT", detail)
    for name in st.unisolated:
        classes[name] = ("REJECT?", "unisolated: build budget exhausted")
    order = {n: i for i, n in enumerate(names_all)}
    ran.sort(key=lambda n: order[n])
    for name in ran:
        classes[name] = classify_ran(name, node_blocks[name], thaw.get(name))
    return {"ran": ran, "thaw": thaw, "classes": classes, "builds": st.builds, "reruns": st.reruns}


def write_blocks(path, names, source):
    with open(path, "w", encoding="utf-8") as fh:
        for n in names:
            if n not in source:
                continue
            fh.write(f"@@ {n}\n{source[n]}\n")


def engine_stage(cfg, engine, manifest, node_blocks):
    progs = [p for p in manifest["programs"] if not cfg.only or p["name"].startswith(cfg.only)]
    results = {}
    with cf.ThreadPoolExecutor(max_workers=cfg.jobs) as ex:
        futs = {ex.submit(run_program, cfg, engine, p, node_blocks[p["name"]]): p for p in progs}
        for fut in cf.as_completed(futs):
            results[futs[fut]["name"]] = (futs[fut], fut.result())
    eng = os.path.join(cfg.out, engine)
    for sub in ("corpus/node", "corpus/thaw"):
        os.makedirs(os.path.join(eng, sub), exist_ok=True)
    corpora = {}
    for prog in progs:
        prog_, res = results[prog["name"]]
        corpora.setdefault(prog["corpus"], []).append((prog, res))
    classes = {}
    for corpus, rows in sorted(corpora.items()):
        ran, node_src, thaw_src = [], {}, {}
        for prog, res in rows:
            for n in res["ran"]:
                ran.append(n)
                node_src[n] = node_blocks[prog["name"]][n]
            thaw_src.update(res["thaw"])
            for n, (cls, detail) in res["classes"].items():
                classes[n] = {"corpus": corpus, "class": cls, "detail": detail}
        write_blocks(os.path.join(eng, "corpus/node", corpus + ".txt"), ran, node_src)
        write_blocks(os.path.join(eng, "corpus/thaw", corpus + ".txt"), ran, thaw_src)
        for n in ran:
            cls, detail = classify_ran(n, node_src[n], thaw_src.get(n))
            classes[n] = {"corpus": corpus, "class": cls, "detail": detail}
    cmd = [sys.executable, os.path.join(PARITY, "diff.py"),
           "--node-dir", os.path.join(eng, "corpus/node"), "--thaw-dir", os.path.join(eng, "corpus/thaw"),
           "--blocks", "--skip", os.path.join(HERE, "skip.txt"), "--waivers", os.path.join(HERE, "waivers.json"),
           "--baseline", os.path.join(HERE, "known-gaps.json"), "--gaps", os.path.join(eng, "gaps.json")]
    p = subprocess.run(cmd, capture_output=True, text=True)
    with open(os.path.join(eng, "diff.txt"), "w", encoding="utf-8") as fh:
        fh.write(p.stdout + p.stderr)
    if p.returncode == 2:
        raise HarnessError(f"diff.py harness error for {engine}: {p.stderr.strip()[:300]}")
    gaps = []
    if os.path.exists(os.path.join(eng, "gaps.json")):
        with open(os.path.join(eng, "gaps.json"), encoding="utf-8") as fh:
            gaps = json.load(fh)["gaps"]
    with open(os.path.join(eng, "classes.json"), "w", encoding="utf-8") as fh:
        json.dump(classes, fh, indent=1, sort_keys=True)
        fh.write("\n")
    return {"classes": classes, "gaps": gaps, "diff_rc": p.returncode,
            "builds": sum(r["builds"] for _, r in results.values()),
            "reruns": sum(r["reruns"] for _, r in results.values())}


def summarise(cfg, engines, out):
    lines = ["# tools/parity-aot results (non-gating)", ""]
    summary = {"engines": {}, "delta": []}
    for engine, res in out.items():
        counts = {}
        per_corpus = {}
        for n, c in res["classes"].items():
            per_corpus.setdefault(c["corpus"], {}).setdefault(c["class"], 0)
            per_corpus[c["corpus"]][c["class"]] += 1
            counts[c["class"]] = counts.get(c["class"], 0) + 1
        summary["engines"][engine] = {"counts": counts, "diff_rc": res["diff_rc"], "builds": res["builds"],
                                      "reruns": res["reruns"], "gaps": len(res["gaps"])}
        lines += [f"## engine `{engine}`", "", f"builds {res['builds']}, reruns {res['reruns']}, "
                  f"diff.py exit {res['diff_rc']} (1 = gaps not in known-gaps.json), gaps {len(res['gaps'])}", "",
                  "| corpus | " + " | ".join(CLASS_ORDER) + " |", "|---" * (len(CLASS_ORDER) + 1) + "|"]
        for corpus in sorted(per_corpus):
            row = per_corpus[corpus]
            lines.append(f"| {corpus} | " + " | ".join(str(row.get(c, 0)) for c in CLASS_ORDER) + " |")
        lines.append(f"| **total** | " + " | ".join(str(counts.get(c, 0)) for c in CLASS_ORDER) + " |")
        lines.append("")
        reject_groups = {}
        for n, c in res["classes"].items():
            if c["class"] == "REJECT":
                reject_groups[c["detail"]] = reject_groups.get(c["detail"], 0) + 1
        if reject_groups:
            lines += ["Top compile rejects (grouped by normalised diagnostic):", ""]
            for detail, k in sorted(reject_groups.items(), key=lambda kv: -kv[1])[:15]:
                lines.append(f"- {k}x `{detail}`")
            lines.append("")
        findings = sorted((n, c) for n, c in res["classes"].items() if c["class"] in GAP_CLASSES + ("CRASH", "HANG"))
        if findings:
            lines += ["Top findings (WRONG/LEAK/MISSING/CRASH/HANG):", ""]
            for n, c in findings[:40]:
                lines.append(f"- {c['class']} `{n}`: {c['detail']}")
            lines.append("")
    if len(engines) == 2:
        a, b = engines
        ca, cb = out[a]["classes"], out[b]["classes"]
        for n in sorted(set(ca) | set(cb)):
            ka = ca.get(n, {}).get("class", "-")
            kb = cb.get(n, {}).get("class", "-")
            if ka != kb:
                summary["delta"].append({"probe": n, a: ka, b: kb})
        lines += [f"## engine delta ({a} vs {b}): {len(summary['delta'])} probe(s) with a different class", ""]
        for d in summary["delta"][:60]:
            lines.append(f"- `{d['probe']}`: {a} {d[a]}, {b} {d[b]}")
        lines.append("")
    with open(os.path.join(cfg.out, "summary.md"), "w", encoding="utf-8") as fh:
        fh.write("\n".join(lines) + "\n")
    with open(os.path.join(cfg.out, "summary.json"), "w", encoding="utf-8") as fh:
        json.dump(summary, fh, indent=1, sort_keys=True)
        fh.write("\n")
    return "\n".join(lines)


def write_baseline(path, out):
    gaps = {}
    for res in out.values():
        for g in res["gaps"]:
            gaps[g["id"]] = g["kind"]
    with open(path, "w", encoding="utf-8") as fh:
        json.dump({"note": "gaps may only shrink; regenerate with --write-baseline only after a fix",
                   "gaps": [{"id": i, "kind": k} for i, k in sorted(gaps.items())]}, fh, indent=1)
        fh.write("\n")
    return len(gaps)


def main(argv):
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[1])
    ap.add_argument("--programs", required=True, help="directory written by gen_probes.py")
    ap.add_argument("--out", required=True)
    ap.add_argument("--thaw", default=os.path.join(REPO, "target", "release", "thaw"))
    ap.add_argument("--node", default="node")
    ap.add_argument("--engines", default="thaw-js,quickjs")
    ap.add_argument("--jobs", type=int, default=max(1, min(4, (os.cpu_count() or 2) // 2)))
    ap.add_argument("--only", default="", help="program name prefix (local debugging)")
    ap.add_argument("--node-only", action="store_true", help="run only the Node reference (selftest)")
    ap.add_argument("--write-baseline", action="store_true")
    ap.add_argument("--keep-work", action="store_true")
    cfg = ap.parse_args(argv)
    cfg.programs = os.path.abspath(cfg.programs)
    cfg.out = os.path.abspath(cfg.out)
    cfg.wrap = shlex.split(os.environ.get("PARITY_AOT_BUILD_WRAP", ""))
    engines = [e for e in cfg.engines.split(",") if e]
    try:
        with open(os.path.join(cfg.programs, "manifest.json"), encoding="utf-8") as fh:
            manifest = json.load(fh)
        os.makedirs(cfg.out, exist_ok=True)
        node_blocks = node_stage(cfg, manifest)
        if cfg.node_only:
            print(f"node reference ok: {len(node_blocks)} program(s)")
            return 0
        if not engines or not all(e in ("thaw-js", "quickjs") for e in engines):
            raise HarnessError("--engines takes thaw-js and/or quickjs")
        if not os.path.exists(cfg.thaw) and shutil.which(cfg.thaw) is None:
            raise HarnessError(f"thaw binary not found: {cfg.thaw} (set THAW=...)")
        out = {}
        for engine in engines:
            out[engine] = engine_stage(cfg, engine, manifest, node_blocks)
            if not cfg.keep_work:
                shutil.rmtree(os.path.join(cfg.out, engine, "work"), ignore_errors=True)
        print(summarise(cfg, engines, out))
        if cfg.write_baseline:
            n = write_baseline(os.path.join(HERE, "known-gaps.json"), out)
            print(f"wrote baseline with {n} gap(s)")
        return 0
    except HarnessError as e:
        print(f"harness error: {e}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
