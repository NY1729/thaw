#!/usr/bin/env python3
"""Node-vs-thaw differential runner for tools/failure-conformance (non-gating).

For every case of cases.py: run it under Node (oracle, `node prog.mts <id>`), build the group's program once with
`thaw build prog.ts -o bin` (a failed group build is bisected so only the offending cases are BUILD-FAIL) and run
`bin <id>` as its own process. Compared per case: stdout, stderr (normalised, see norm()), exit status/signal.

Classes: MATCH, DIFF, BUILD-FAIL, RUN-FAIL (thaw hung, or died of SIGSEGV/SIGABRT/SIGBUS/SIGILL/SIGFPE where Node did
not), ORACLE-FAIL (Node itself hung; a harness problem, selftest.sh catches it). Results: report.tsv, summary.md,
results/<id>.json. Cases listed in known-gaps.json are expected non-MATCH ("known"); others are NEW; a known gap that
now MATCHes is FIXED.
"""
import argparse
import concurrent.futures as cf
import hashlib
import json
import os
import re
import shlex
import shutil
import signal
import subprocess
import sys
import threading
import time

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import gen  # noqa: E402

RUN_TIMEOUT = int(os.environ.get("FC_RUN_TIMEOUT", "20"))
BUILD_TIMEOUT = int(os.environ.get("FC_BUILD_TIMEOUT", "900"))
BUDGET = int(os.environ.get("FC_BUILD_BUDGET", "0"))  # max builds per program before unisolated failures are given up on (0 = no limit)
AS_LIMIT = int(os.environ.get("FC_RLIMIT_AS", str(8 << 30)))
CRASH = {signal.SIGSEGV, signal.SIGABRT, signal.SIGBUS, signal.SIGILL, signal.SIGFPE}
BIG = 8192

# ---------------------------------------------------------------- normalisation


def norm(text, paths):
    """Strip what is machine- or implementation-specific so that two correct runtimes compare equal: program paths,
    `:line:col`, `    at ...` stack frames (frame fidelity is pinned by stack tests elsewhere), Node's uncaught-error
    source excerpt + caret, `Node.js vX` banner and the `--trace-uncaught` hint; `[Error: m]` (a stack-less error
    as util.inspect prints it) is compared as `Error: m`. Everything else, including message text, own
    properties and `Emitted 'error' event` notes, is compared verbatim."""
    t = text
    for p in sorted(set(paths), key=len, reverse=True):
        t = t.replace("file://" + p, p).replace(p, "<P>")
    t = re.sub(r"fc-\d+", "fc-N", t)
    t = re.sub(r"\(node:\d+\)", "(node:PID)", t)
    t = re.sub(r"(127\.0\.0\.1:|:::|0\.0\.0\.0:)\d{4,5}", r"\1PORT", t)
    if len(t) > 65536:
        return big(t)
    t = re.sub(r"<P>(:\d+){1,2}", "<P>", t)
    lines, out, i, brace = t.split("\n"), [], 0, False
    while i < len(lines):
        l = lines[i]
        if re.match(r"^(<P>|node:[\w/.-]+?|<anonymous_script>|\[worker eval\])(:\d+)?$", l) and i + 2 < len(lines) and re.match(r"^\s*\^+\s*$", lines[i + 2]):
            i += 3
            continue
        if re.match(r"^\s+at ", l) or l.strip() == "Thrown at:":
            if l.endswith(" {") and out and not out[-1].endswith("{"):
                out[-1] += " {"
            i += 1
            continue
        if l.startswith("Node.js v") or l.startswith("(Use `node "):
            i += 1
            continue
        out.append(l.rstrip())
        i += 1
    t = "\n".join(out)
    t = re.sub(r"\[((?:[A-Za-z]+\.)?[A-Za-z]*(?:Error|Exception)(?::[^\[\]\n]*)?)\]", r"\1", t)
    t = re.sub(r"\n{3,}", "\n\n", t).strip()
    return big(t)


def big(t):
    if len(t) <= BIG:
        return t
    return t[:300] + f"\n...[{len(t)} chars sha256:{hashlib.sha256(t.encode()).hexdigest()[:12]}]"


# ---------------------------------------------------------------- process execution


def run_proc(argv, mode, merge, cwd, env, limit_as=False):
    """Run one process; returns {rc, out, err, timeout}. rc < 0 is -signal. Modes: run, signal:SIGX, epipe."""
    if limit_as and shutil.which("prlimit"):
        argv = ["prlimit", f"--as={AS_LIMIT}", "--"] + argv
    p = subprocess.Popen(argv, stdin=subprocess.DEVNULL, stdout=subprocess.PIPE,
                         stderr=subprocess.STDOUT if merge else subprocess.PIPE, cwd=cwd, env=env,
                         start_new_session=True)
    bufs = {"out": bytearray(), "err": bytearray()}

    def reader(f, key):
        try:
            while True:
                d = os.read(f.fileno(), 65536)
                if not d:
                    break
                bufs[key] += d
        except OSError:
            pass

    threads = []
    if mode == "epipe":
        bufs["out"] += os.read(p.stdout.fileno(), 1)
        p.stdout.close()
    else:
        threads.append(threading.Thread(target=reader, args=(p.stdout, "out"), daemon=True))
    if not merge:
        threads.append(threading.Thread(target=reader, args=(p.stderr, "err"), daemon=True))
    for th in threads:
        th.start()
    if mode.startswith("signal:"):
        end = time.time() + 10
        while b"READY" not in bufs["out"] and time.time() < end and p.poll() is None:
            time.sleep(0.02)
        time.sleep(0.2)
        try:
            os.kill(p.pid, getattr(signal, mode.split(":")[1]))
        except ProcessLookupError:
            pass
    timed_out = False
    try:
        p.wait(timeout=RUN_TIMEOUT)
    except subprocess.TimeoutExpired:
        timed_out = True
        try:
            os.killpg(p.pid, signal.SIGKILL)  # the session this harness started, nothing else
        except ProcessLookupError:
            pass
        p.wait()
    for th in threads:
        th.join(5)
    try:
        os.killpg(p.pid, signal.SIGKILL)  # stray grandchildren of the case (workers' children, `sleep`)
    except ProcessLookupError:
        pass
    return {"rc": p.returncode, "out": bytes(bufs["out"]).decode("utf-8", "replace"),
            "err": bytes(bufs["err"]).decode("utf-8", "replace"), "timeout": timed_out}


# ---------------------------------------------------------------- stages


class Cfg:
    pass


def case_env(cfg, name):
    env = dict(os.environ, TZ="UTC", LANG="C", NO_COLOR="1")
    env.pop("FORCE_COLOR", None)
    return env


def node_stage(cfg, cases):
    flags = [] if node_ok(cfg.node) else ["--experimental-strip-types"]
    def one(c):
        prog = os.path.join(cfg.programs, c["prog"] + ".mts")
        r = run_proc([cfg.node, *flags, prog, c["id"]], c["mode"], c["merge"], cfg.work, case_env(cfg, c["id"]))
        r["paths"] = [prog, cfg.work, cfg.out]
        return c["id"], r
    with cf.ThreadPoolExecutor(cfg.jobs * 2) as ex:
        return dict(ex.map(one, cases))


def node_ok(node):
    v = subprocess.run([node, "--version"], capture_output=True, text=True).stdout.strip()
    m = re.match(r"v(\d+)\.(\d+)", v)
    return (int(m.group(1)), int(m.group(2))) >= (22, 18)


DIAG_RE = re.compile(r"([\w./\-]+\.ts):(\d+):\d+")


def build(cfg, name, subset):
    """Build `subset` cases of a program; returns (ok, binary, src, diag). Cached by content hash of the program
    text (a binary or a `.diag` next to it), so a rerun with the same thaw only builds what changed."""
    wd = os.path.join(cfg.out, "thaw", "work", name)
    os.makedirs(wd, exist_ok=True)
    text = gen.render(subset[0]["id"].split(".")[0], subset)
    stem = "p" + hashlib.sha1(text.encode()).hexdigest()[:12]
    src, bin_ = os.path.join(wd, stem + ".ts"), os.path.join(wd, stem)
    if os.path.exists(bin_):
        return True, bin_, src, ""
    if os.path.exists(src + ".diag"):
        return False, bin_, src, open(src + ".diag", encoding="utf-8").read()
    with open(src, "w", encoding="utf-8") as fh:
        fh.write(text)
    try:
        p = subprocess.run(cfg.wrap + [cfg.thaw, "build", src, "-o", bin_], cwd=wd, env=case_env(cfg, name),
                           capture_output=True, text=True, timeout=BUILD_TIMEOUT, stdin=subprocess.DEVNULL)
        diag = (p.stdout + p.stderr).strip()
        ok = p.returncode == 0 and os.path.exists(bin_)
    except subprocess.TimeoutExpired:
        diag, ok = "error: build timed out", False
    if not ok:
        with open(src + ".diag", "w", encoding="utf-8") as fh:
            fh.write(diag)
    return ok, bin_, src, diag


def sites(diag, src, subset):
    """Case ids whose source lines are named by `file.ts:LINE:COL` locations in the diagnostic."""
    lines = open(src, encoding="utf-8").read().split("\n")
    owner, cur = {}, None
    for i, l in enumerate(lines, 1):
        m = re.match(r'^case "([^"]+)": \{$', l)
        if m:
            cur = m.group(1)
        elif l == "default:":
            cur = None
        owner[i] = cur
    ids = {c["id"] for c in subset}
    found, header = [], False
    for m in DIAG_RE.finditer(diag):
        if os.path.basename(m.group(1)) == os.path.basename(src):
            cid = owner.get(int(m.group(2)))
            header |= cid is None
            if cid in ids and cid not in found:
                found.append(cid)
    text = "\n".join(lines)
    for m in re.finditer(r"(?:source bytes |span: )(\d+)\.\.\d+", diag):
        ln = text.encode()[:int(m.group(1))].count(b"\n") + 1
        cid = owner.get(ln)
        if cid in ids and cid not in found:
            found.append(cid)
    return ["*"] if header and not found else found


def build_program(cfg, prog):
    """({case id: (binary or None, src, diag)}, builds). A failing group drops the cases its diagnostic points at
    (they become BUILD-FAIL) and rebuilds the rest; with no located case it is bisected."""
    res, nb = {}, [0]

    def go(subset):
        if not subset:
            return
        nb[0] += 1
        ok, bin_, src, diag = build(cfg, prog["name"], subset)
        if ok:
            for c in subset:
                res[c["id"]] = (bin_, src, "")
            return
        bad = sites(diag, src, subset)
        if bad == ["*"]:  # located in the shared header (imports, helpers): every case of the program fails the same way
            bad = [c["id"] for c in subset]
        if not bad and len(subset) > 1 and BUDGET and nb[0] >= BUDGET:
            # ponytail: local runs behind a contended build lock cap the isolation work; CI runs unlimited
            for c in subset:
                res[c["id"]] = (None, src, "(not isolated, FC_BUILD_BUDGET reached) " + diag)
            return
        if bad:
            for cid in bad:
                res[cid] = (None, src, diag)
            go([c for c in subset if c["id"] not in bad])
        elif len(subset) == 1:
            res[subset[0]["id"]] = (None, src, diag)
        else:
            # thaw's diagnostics name the construct, not the line: cases whose body contains the first quoted token are
            # the suspects. Build the others first (usually green), then bisect only the suspects.
            toks = re.findall(r"`([^`]+)`", diag)
            sus = [c for c in subset if toks and toks[0] in c["body"]]
            if 0 < len(sus) < len(subset):
                go([c for c in subset if c not in sus])
                go(sus)
            else:
                h = len(subset) // 2
                go(subset[:h])
                go(subset[h:])
    go(prog["cases"])
    return res, nb[0]


def thaw_stage(cfg, progs, cases_by_id):
    built, nbuilds = {}, 0
    with cf.ThreadPoolExecutor(cfg.jobs) as ex:
        for res, n in ex.map(lambda p: build_program(cfg, p), progs):
            built.update(res)
            nbuilds += n
    def one(cid):
        bin_, src, diag = built[cid]
        c = cases_by_id[cid]
        if bin_ is None:
            return cid, {"build_fail": diag, "paths": []}
        r = run_proc([bin_, cid], c["mode"], c["merge"], cfg.work, case_env(cfg, cid), limit_as=True)
        r["paths"] = [bin_, src, os.path.dirname(bin_), cfg.work, cfg.out]
        return cid, r
    with cf.ThreadPoolExecutor(cfg.jobs) as ex:
        return dict(ex.map(one, list(built))), nbuilds


# ---------------------------------------------------------------- classification


def fmt_rc(r):
    if r["rc"] is not None and r["rc"] < 0:
        return "SIG" + signal.Signals(-r["rc"]).name[3:]
    return str(r["rc"])


def first_line(text):
    return next((l for l in text.split("\n") if l.strip()), "")[:160]


def diag_line(d):
    """The `error:` line of a thaw diagnostic (stdout and stderr are concatenated, so cut a glued `Running as unit`)."""
    lines = [re.sub(r"Running as unit.*", "", l).strip() for l in d.splitlines()]
    pick = next((l for l in lines if l.startswith("error")), next((l for l in lines if "error" in l.lower()), "no diagnostic"))
    pick = re.sub(r"/\S+/work/\S+?\.ts:\d+:\d+: ", "", pick)
    return re.sub(r"/\S+", "<path>", pick)[:200]


def classify(node, thaw):
    """(class, detail, family) for one case."""
    if node["timeout"]:
        return "ORACLE-FAIL", "node timed out", "harness"
    if "build_fail" in thaw:
        return "BUILD-FAIL", diag_line(thaw["build_fail"]), "build-fail"
    n = {"out": norm(node["out"], node["paths"]), "err": norm(node["err"], node["paths"])}
    t = {"out": norm(thaw["out"], thaw["paths"]), "err": norm(thaw["err"], thaw["paths"])}
    node["norm"], thaw["norm"] = n, t
    diff = [k for k, v in (("stdout", "out"), ("stderr", "err")) if n[v] != t[v]]
    if node["rc"] != thaw["rc"]:
        diff.append(f"rc {fmt_rc(node)}->{fmt_rc(thaw)}")
    crashed = thaw["rc"] is not None and thaw["rc"] < 0 and -thaw["rc"] in {int(s) for s in CRASH} and node["rc"] != thaw["rc"]
    if thaw["timeout"] or crashed:
        return "RUN-FAIL", "hang" if thaw["timeout"] else "crash " + fmt_rc(thaw), "hang" if thaw["timeout"] else "crash"
    if not diff:
        return "MATCH", "", ""
    blob = thaw["out"] + thaw["err"]
    if "Uncaught opaque exception" in blob:
        fam = "opaque-exception"
    elif "stderr" in diff and not t["err"] and n["err"]:
        fam = "no-error-output"
    elif diff == [d for d in diff if d.startswith("rc")]:
        fam = "exit-status"
    elif "stderr" in diff:
        fam = "stderr-text"
    elif "stdout" in diff:
        fam = "stdout-text"
    else:
        fam = "exit-status"
    return "DIFF", ", ".join(diff), fam


# ---------------------------------------------------------------- main


def main(argv):
    ap = argparse.ArgumentParser()
    ap.add_argument("--out", required=True)
    ap.add_argument("--thaw", default=os.environ.get("THAW", ""))
    ap.add_argument("--node", default=os.environ.get("NODE", "node"))
    ap.add_argument("--jobs", type=int, default=int(os.environ.get("FC_JOBS", "3")))
    ap.add_argument("--only", default="", help="run only cases whose id contains this")
    ap.add_argument("--node-only", action="store_true", help="oracle check without thaw (selftest)")
    ap.add_argument("--clean", action="store_true", help="also discard earlier builds (reused when the program text is unchanged; pass after the thaw binary changes)")
    ap.add_argument("--write-baseline", action="store_true")
    ap.add_argument("--known", default=os.path.join(HERE, "known-gaps.json"))
    cfg = ap.parse_args(argv)
    # A harness started from a background job or nohup inherits SIG_IGN for these; children would inherit it too
    # (Node resets them itself at startup, a compiled binary does not), so start every child from the defaults.
    for sig in (signal.SIGINT, signal.SIGHUP, signal.SIGQUIT, signal.SIGTERM):
        signal.signal(sig, signal.SIG_DFL)
    cfg.wrap = shlex.split(os.environ.get("FC_BUILD_WRAP", ""))
    cfg.out = os.path.abspath(cfg.out)
    cfg.programs = os.path.join(cfg.out, "programs")
    cfg.work = os.path.join(cfg.out, "cwd")
    for d in ("programs", "results", "cwd") + (("thaw",) if cfg.clean else ()):
        shutil.rmtree(os.path.join(cfg.out, d), ignore_errors=True)
    os.makedirs(cfg.work)
    os.makedirs(os.path.join(cfg.out, "results"))
    for f in ("summary.md", "report.tsv"):
        if os.path.exists(os.path.join(cfg.out, f)):
            os.remove(os.path.join(cfg.out, f))
    gen.main([cfg.programs, "--only", cfg.only])
    man = json.load(open(os.path.join(cfg.programs, "manifest.json")))
    progs = gen.programs(only=cfg.only)
    cases = [{"id": c["id"], "mode": c["mode"], "merge": c["merge"], "prog": p["name"]}
             for p in man["programs"] for c in p["cases"]]
    cases_by_id = {c["id"]: c for c in cases}
    node = node_stage(cfg, cases)
    if cfg.node_only:
        bad = [i for i, r in node.items() if r["timeout"]]
        with open(os.path.join(cfg.out, "node-only.tsv"), "w") as fh:
            for i, r in node.items():
                fh.write("\t".join([i, fmt_rc(r), first_line(norm(r["err"], r["paths"])), first_line(norm(r["out"], r["paths"]))]) + "\n")
        print(f"node-only: {len(node)} cases, {len(bad)} timed out {bad[:5]}")
        return 1 if bad else 0
    if not cfg.thaw or not os.path.exists(cfg.thaw):
        print(f"harness error: thaw binary not found: {cfg.thaw!r}", file=sys.stderr)
        return 2
    thaw, nbuilds = thaw_stage(cfg, progs, cases_by_id)
    rows = []
    for c in cases:
        cls, detail, fam = classify(node[c["id"]], thaw[c["id"]])
        rows.append({"id": c["id"], "class": cls, "detail": detail, "family": fam, "node": node[c["id"]], "thaw": thaw[c["id"]]})
        with open(os.path.join(cfg.out, "results", c["id"] + ".json"), "w") as fh:
            json.dump({k: rows[-1][k] for k in ("id", "class", "detail", "family", "node", "thaw")}, fh, indent=1)
    return report(cfg, rows, nbuilds)


def report(cfg, rows, nbuilds):
    known = {g["id"] for g in json.load(open(cfg.known))["gaps"]} if os.path.exists(cfg.known) else set()
    counts = {}
    for r in rows:
        counts[r["class"]] = counts.get(r["class"], 0) + 1
        r["status"] = ("known" if r["id"] in known else "NEW") if r["class"] != "MATCH" else ("FIXED" if r["id"] in known else "")
    if cfg.write_baseline:
        gaps = [{"id": r["id"], "kind": r["class"]} for r in rows if r["class"] != "MATCH"]
        with open(cfg.known, "w") as fh:
            json.dump({"note": "expected non-MATCH cases; may only shrink. Regenerate: run.sh --write-baseline after a fix.",
                       "gaps": gaps}, fh, indent=1)
            fh.write("\n")
    with open(os.path.join(cfg.out, "report.tsv"), "w") as fh:
        fh.write("id\tclass\tstatus\tfamily\tdetail\tnode_rc\tthaw_rc\tnode_first\tthaw_first\n")
        for r in rows:
            n, t = r["node"], r["thaw"]
            nf = first_line(n.get("norm", {}).get("err", "")) or first_line(n.get("norm", {}).get("out", ""))
            tf = first_line(t.get("norm", {}).get("err", "")) or first_line(t.get("norm", {}).get("out", ""))
            tr = "build-fail" if "build_fail" in t else fmt_rc(t)
            fh.write("\t".join(x.replace("\t", " ").replace("\n", " ") for x in
                               [r["id"], r["class"], r["status"], r["family"], r["detail"], fmt_rc(n), tr, nf, tf]) + "\n")
    fams = {}
    for r in rows:
        if r["class"] != "MATCH":
            fams.setdefault(r["family"], []).append(r)
    L = ["# failure-conformance: thaw vs Node (NON-GATING)", "",
         f"{len(rows)} cases, {nbuilds} thaw builds. " + ", ".join(f"{k} {v}" for k, v in sorted(counts.items())), "",
         f"new (not in known-gaps): {sum(1 for r in rows if r['status'] == 'NEW')}, "
         f"fixed (known gap now MATCH): {sum(1 for r in rows if r['status'] == 'FIXED')}", "",
         "## Failure families", "", "| family | cases | first example (node -> thaw) |", "|---|---|---|"]
    for f, rs in sorted(fams.items(), key=lambda kv: -len(kv[1])):
        e = rs[0]
        nf = first_line(e["node"].get("norm", {}).get("err", "")) or first_line(e["node"].get("norm", {}).get("out", ""))
        tf = diag_line(e["thaw"]["build_fail"])[:80] if "build_fail" in e["thaw"] else (
            first_line(e["thaw"].get("norm", {}).get("err", "")) or first_line(e["thaw"].get("norm", {}).get("out", "")))
        L.append(f"| {f} | {len(rs)} | `{e['id']}`: `{nf}` -> `{tf}` |".replace("\n", " "))
    L += ["", "## Per group", "", "| group | MATCH | DIFF | BUILD-FAIL | RUN-FAIL |", "|---|---|---|---|---|"]
    groups = {}
    for r in rows:
        groups.setdefault(r["id"].split(".")[0], {}).setdefault(r["class"], 0)
        groups[r["id"].split(".")[0]][r["class"]] += 1
    for g, c in groups.items():
        L.append(f"| {g} | " + " | ".join(str(c.get(k, 0)) for k in ("MATCH", "DIFF", "BUILD-FAIL", "RUN-FAIL")) + " |")
    with open(os.path.join(cfg.out, "summary.md"), "w") as fh:
        fh.write("\n".join(L) + "\n")
    print("\n".join(L[:12]))
    print(f"summary: {cfg.out}/summary.md  report: {cfg.out}/report.tsv")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
