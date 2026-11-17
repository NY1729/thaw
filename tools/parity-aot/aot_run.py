#!/usr/bin/env python3
"""Node reference + thaw AOT build/run + classification for the tools/parity-aot probe programs.

Driven by run.sh (which runs gen_probes.py first). For each program in the manifest:
  1. Node runs <program>.mts once (the oracle; a Node failure or a missing header is a harness error, exit 2).
  2. For each engine (THAW_JS_ENGINE value), thaw builds the program with `thaw build <file.ts> -o <bin>`.
     - Compile diagnostic: every probe line named at an error site is dropped in one pass and the rest rebuilt
       (REJECT, the diagnostic text is the reason; REJECT-EXPECTED when Node also throws, see expected_reject).
       A diagnostic with no named site bisects the probe list by halves. At most EXTRA_CAP builds per program;
       the rest is REJECT? (unisolated).
     - Run: `timeout RUN_TIMEOUT`. A nonzero exit or a timeout means the probes after the last printed header
       are unverified; the header-less tail is rerun one probe per binary (RERUN_CAP builds). The probe that
       still fails alone is CRASH or HANG.
  3. Classes per ran probe: MATCH, MSG (same THROW name, different message), WRONG-TYPE (THROW vs value or
     different THROW name), WRONG-VALUE, LEAK (engine handle marker in output), MISSING (no output block).
  4. The gate reuses tools/parity/diff.py (--blocks, baseline, waivers, skip) on the ran probes only.
     Exit 1 from diff.py means gaps not in the baseline: reported, not a failure. Exit 2 is a harness error.
Module programs (gen_modules.py, manifest `imports`): each program renders its `import * as M from "node:x"` lines
(render_program). Every run gets PA_TMP=<out>/pa-tmp/<program>, the SAME path on both sides, and module programs run
with that directory as cwd on both sides: path.resolve and cwd-derived values then print identically (diff.py masks the
path to <tmp>). Module programs run under TZ=UTC LANG=C on the Node side too.
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

# Builds per program (the first build included). One build per error site plus one clean build when the diagnostics
# name their sites, so 120 covers about 100 rejected probes in one 40-probe program. Budget = CI wall clock, see README.
EXTRA_CAP = 120
RERUN_CAP = 40
BUILD_TIMEOUT = int(os.environ.get("PARITY_AOT_BUILD_TIMEOUT", "900"))
RUN_TIMEOUT = int(os.environ.get("PARITY_AOT_RUN_TIMEOUT", "20"))
MARKER = "__thaw_js_handle_id__"
DIAG_RE = re.compile(r"([\w./\-]+\.ts):(\d+):\d+")
PROBE_RE = re.compile(r'^\s*pt?\("([^"]+)", ')
GAP_CLASSES = ("WRONG-TYPE", "WRONG-VALUE", "LEAK", "MISSING")
CLASS_ORDER = ("MATCH", "MSG", "WRONG-TYPE", "WRONG-VALUE", "LEAK", "MISSING", "CRASH", "HANG",
               "REJECT", "REJECT-EXPECTED", "REJECT?", "UNVERIFIED")
# Compile rejects that Node reproduces as a runtime TypeError: a non-function callback (`(1 as any)`) to an array or
# promise method. The AOT cannot express that call, so the reject is the expected answer, classed apart from REJECT.
EXPECTED_REJECT_RE = re.compile(r"must be an arrow or function value|is not a function value")
# REJECT sub-kinds for the module report (details are normalised: digits become N). Heuristic on the diagnostic text;
# anything unmatched is "other". missing-export: the d.ts or lowering lacks a member Node has. type: a type or
# arity mismatch on a Node-valid call. other: unsupported syntax or lowering.
REJECT_MISSING_RE = re.compile(r"has no exported member|does not exist on type|Cannot find name|Property '[^']*' does not")
REJECT_TYPE_RE = re.compile(r"not assignable|Argument of type|No overload matches|not callable|cannot be used as|"
                            r"has no call signatures|expected N arguments?|Type '")


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
        env = dict(os.environ, PA_TMP=pa_tmp_dir(cfg, prog))
        os.makedirs(env["PA_TMP"], exist_ok=True)
        cwd = cfg.programs
        if prog.get("imports"):
            env.update(TZ="UTC", LANG="C")
            cwd = env["PA_TMP"]
        try:
            p = subprocess.run([cfg.node, *flags, path], cwd=cwd, env=env, capture_output=True, text=True,
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
            return "MSG", f"node {n1[:80]} | thaw {t1[:80]}"
        return "MATCH", ""
    if MARKER in thaw_raw and MARKER not in node_raw:
        return "LEAK", thaw_raw.split("\n", 1)[0][:160]
    if node_raw.startswith("THROW") or thaw_raw.startswith("THROW"):
        return "WRONG-TYPE", thaw_raw.split("\n", 1)[0][:160]
    return "WRONG-VALUE", thaw_raw.split("\n", 1)[0][:160]


def expected_reject(node_raw, detail):
    """True when Node throws and the compile reject is the documented non-function callback reject."""
    return node_raw.startswith("THROW") and EXPECTED_REJECT_RE.search(detail) is not None


def reject_kind(detail):
    """missing-export / type / other for a REJECT detail (see REJECT_MISSING_RE)."""
    if REJECT_MISSING_RE.search(detail):
        return "missing-export"
    if REJECT_TYPE_RE.search(detail):
        return "type"
    return "other"


def render_program(prog, lines, node):
    """Program text for probe lines. Module programs carry their `import * as M` lines in prog["imports"], which
    gen_probes.render does not write; they go right after the header line."""
    text = gen_probes.render(lines, prog["module"], node)
    imports = prog.get("imports") or []
    if imports:
        head, rest = text.split("\n", 1)
        text = head + "\n" + "\n".join(imports) + "\n" + rest
    return text


def pa_tmp_dir(cfg, prog):
    """Per-program scratch directory exported as PA_TMP. The same path for Node and the engine, so that a value
    derived from the path (length included) cannot differ between the two sides. diff.py masks it to <tmp>."""
    return os.path.join(cfg.out, "pa-tmp", prog["name"])


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
        self.cwd = self.work
        self.env = dict(os.environ, THAW_JS_ENGINE=engine, TZ="UTC", LANG="C",
                        PA_TMP=pa_tmp_dir(cfg, prog))
        os.makedirs(self.env["PA_TMP"], exist_ok=True)
        if prog.get("imports"):
            self.cwd = self.env["PA_TMP"]   # same cwd as the Node side, see the module docstring
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
        texts = render_program(self.prog, [self.line_of[n] for n in names], node=False).split("\n")
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

    def sites_from_diag(self, diag, stem, texts, names):
        """Probe names at the error sites of one diagnostic, in order. Only `error` blocks count: a note or warning may
        point at another line. With no error block, the first located probe of any block counts (old behaviour)."""
        error_lines, in_error = [], False
        for line in diag.splitlines():
            if line and not line[0].isspace():
                in_error = line.lower().startswith("error")
            if in_error:
                error_lines.append(line)
        found = []
        for line in error_lines:
            for m in DIAG_RE.finditer(line):
                if os.path.basename(m.group(1)) != stem + ".ts":
                    continue
                ln = int(m.group(2))
                if 1 <= ln <= len(texts):
                    pm = PROBE_RE.match(texts[ln - 1])
                    if pm and pm.group(1) in names and pm.group(1) not in found:
                        found.append(pm.group(1))
        if found:
            return found
        first = self.name_from_diag(diag, stem, texts, names)
        return [first] if first else []

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
        bad = self.sites_from_diag(diag, stem, texts, names)
        if bad:
            for name in bad:
                self.rejects[name] = diag_detail(diag)
            return self.solve([n for n in names if n not in bad])
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
            p = subprocess.run([binary], cwd=self.cwd, env=self.env, capture_output=True, timeout=RUN_TIMEOUT)
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
        cls = "REJECT-EXPECTED" if expected_reject(node_blocks[name], detail) else "REJECT"
        classes[name] = (cls, detail)
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
                if cls == "REJECT":
                    classes[n]["sub"] = reject_kind(detail)
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
        # MSG probes are gated by error name only (diff.py); the text differences are listed here, not gated.
        by_name = {}
        for n, c in res["classes"].items():
            if c["class"] == "MSG":
                thrown = c["detail"].split(" | thaw ", 1)[-1].replace("THROW ", "").split(":", 1)[0]
                by_name.setdefault(thrown, []).append((n, c["detail"]))
        if by_name:
            lines += ["Error text differences (non-gating, same error name on both sides):", ""]
            for thrown, rows in sorted(by_name.items(), key=lambda kv: -len(kv[1])):
                lines.append(f"- {thrown}: {len(rows)} probe(s), e.g. `{rows[0][0]}` {rows[0][1]}")
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
        if not cfg.keep_work:
            shutil.rmtree(os.path.join(cfg.out, "pa-tmp"), ignore_errors=True)
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
