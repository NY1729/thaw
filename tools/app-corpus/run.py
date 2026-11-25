#!/usr/bin/env python3
"""Application-style program corpus: Node 22 vs `thaw build` + run (non-gating).

For each program in programs/ (.mjs/.ts): node runs it in a fresh dir (stdout, stderr, exit code, post-run tree
snapshot); thaw builds it (one build per program) and the binary runs in another fresh dir with the same stdin/env.
Classes: MATCH, BUILD-FAIL (thaw build/add error), RUN-FAIL (thaw nonzero/timeout where node succeeds), DIFF
(output, exit code or tree differs), NODE-FAIL (harness problem: node itself failed unexpectedly or timed out).
A program may declare npm packages in its first lines: `// pkgs: semver@7.8.5 @types/semver@7.8.0` (installed with
`thaw add --prefix .`, versions as in tools/corpus/corpus/versions.txt).
Exit 0 when the harness ran (failures are reported, not gated); exit 2 on a harness error.
"""
import argparse, hashlib, os, re, resource, shlex, shutil, signal, subprocess, sys, time
from collections import Counter, OrderedDict

HERE = os.path.dirname(os.path.abspath(__file__))
ANSI = re.compile(r"\x1b\[[0-9;]*[A-Za-z]")
SKIP = {"node_modules", "thaw_modules", "package.json", "package-lock.json"}
CLASSES = ("MATCH", "BUILD-FAIL", "RUN-FAIL", "DIFF", "NODE-FAIL")


def sh(cmd, cwd, env, timeout, stdin=None, as_mb=0):
    """Run cmd in its own session; returns (exit, stdout, stderr, timed_out, seconds). Kills only its own group."""
    def pre():
        os.setsid()
        if as_mb:
            lim = as_mb << 20
            resource.setrlimit(resource.RLIMIT_AS, (lim, lim))
    t = time.time()
    f = open(stdin, "rb") if stdin else subprocess.DEVNULL
    p = subprocess.Popen(cmd, cwd=cwd, env=env, stdin=f, stdout=subprocess.PIPE, stderr=subprocess.PIPE, preexec_fn=pre)
    try:
        o, e = p.communicate(timeout=timeout)
        to = False
    except subprocess.TimeoutExpired:
        try:
            os.killpg(p.pid, signal.SIGKILL)
        except ProcessLookupError:
            pass
        o, e = p.communicate()
        to = True
    if stdin:
        f.close()
    return p.returncode, o.decode("utf-8", "replace"), e.decode("utf-8", "replace"), to, time.time() - t


def locked(wrap, inner_timeout, cmd):
    """Wrap cmd so the time limit starts after the wrapper (flock/systemd-run) got its turn."""
    return wrap + ["timeout", str(inner_timeout)] + cmd, 4 * 3600


def pkgs_of(path):
    for line in open(path, encoding="utf-8").read().splitlines()[:5]:
        m = re.match(r"//\s*pkgs:\s*(.+)", line)
        if m:
            return m.group(1).split()
    return []


def snapshot(d, prog):
    rows = []
    for root, dirs, files in os.walk(d):
        dirs[:] = sorted(x for x in dirs if x not in SKIP)
        rel = os.path.relpath(root, d)
        for x in dirs:
            p = os.path.join(root, x)
            rows.append(("l %s -> %s" % (os.path.normpath(os.path.join(rel, x)), os.readlink(p))) if os.path.islink(p)
                        else "d %s" % os.path.normpath(os.path.join(rel, x)))
        for x in sorted(files):
            p = os.path.join(root, x)
            r = os.path.normpath(os.path.join(rel, x))
            if r == prog or x in SKIP and rel == ".":
                continue
            if os.path.islink(p):
                rows.append("l %s -> %s" % (r, os.readlink(p)))
            else:
                rows.append("f %s %d %s" % (r, os.path.getsize(p), hashlib.sha1(open(p, "rb").read()).hexdigest()[:12]))
    return sorted(rows)


def norm(text, dirs):
    for d in dirs:
        text = text.replace(d, "<DIR>")
    text = ANSI.sub("", text)
    text = re.sub(r"((?:127\.0\.0\.1|localhost|0\.0\.0\.0|\[::1?\]):)\d{2,5}", r"\1<PORT>", text)
    text = re.sub(r'("port"\s*:\s*)\d{4,5}', r"\1<PORT>", text)
    out = []
    for line in text.splitlines():
        if re.match(r"\s+at ", line) or line.startswith("Node.js v"):
            continue
        out.append(line.rstrip())
    return "\n".join(out).strip("\n")


def first_diff(a, b, label):
    al, bl = a.splitlines(), b.splitlines()
    for i in range(max(len(al), len(bl))):
        x = al[i] if i < len(al) else "<eof>"
        y = bl[i] if i < len(bl) else "<eof>"
        if x != y:
            return "%s line %d: node=%s thaw=%s" % (label, i + 1, x[:70], y[:70])
    return label + " differs"


def first_error(text):
    lines = [ANSI.sub("", l).strip() for l in text.splitlines() if l.strip()]
    for l in lines:
        if re.match(r"error", l, re.I):
            return l[:200]
    for l in lines:
        if re.search(r"error|unsupported|panick|fatal|uncaught", l, re.I):
            return l[:200]
    return (lines[0] if lines else "(no output)")[:200]


def run_program(prog, a, env):
    name = prog.rsplit(".", 1)[0]
    ts = prog.endswith(".ts")
    work = os.path.join(a.out, "work", name)
    shutil.rmtree(work, ignore_errors=True)
    proj, ndir, tdir = (os.path.join(work, x) for x in ("proj", "node", "thaw"))
    logs = os.path.join(a.out, "logs")
    for d in (proj, ndir, tdir, logs, os.path.join(a.out, "bin")):
        os.makedirs(d, exist_ok=True)
    src = os.path.join(a.programs, prog)
    for d in (proj, ndir, tdir):
        shutil.copy(src, d)
    res = OrderedDict(program=prog, cls="", detail="", build_s=0.0, run_s=0.0)
    wrap = shlex.split(os.environ.get("APP_CORPUS_BUILD_WRAP", ""))
    pk = pkgs_of(src)
    if pk:
        open(os.path.join(proj, "package.json"), "w").write('{"name":"t","version":"1.0.0"}\n')
        for attempt in range(3):
            c, t = locked(wrap, 300, [a.thaw, "add"] + pk + ["--prefix", "."])
            rc, o, e, to, _ = sh(c, proj, env, t)
            if rc == 0:
                break
            time.sleep(3)
        if rc != 0:
            open(os.path.join(logs, name + ".build.log"), "w").write(o + e)
            res.update(cls="BUILD-FAIL", detail="thaw add failed: " + first_error(o + e))
            return res
        if not os.path.isdir(os.path.join(proj, "node_modules")):
            res.update(cls="BUILD-FAIL", detail="thaw add produced no node_modules")
            return res
        for d in (ndir, tdir):
            for m in ("node_modules", "thaw_modules"):
                if os.path.exists(os.path.join(proj, m)):
                    os.symlink(os.path.join(proj, m), os.path.join(d, m))
    stdin = os.path.join(a.out, "stdin.txt")
    open(stdin, "w").write("alpha beta\ngamma\n")
    ncmd = [a.node, "--no-warnings", "--experimental-strip-types", prog] if ts else [a.node, prog]
    nrc, no, ne, nto, _ = sh(ncmd, ndir, env, a.run_timeout, stdin)
    if nto:
        res.update(cls="NODE-FAIL", detail="node timed out")
        return res
    # build (one `thaw build` per program)
    binp = os.path.join(a.out, "bin", name)
    if os.path.exists(binp):
        os.remove(binp)
    c, t = locked(wrap, a.build_timeout, [a.thaw, "build", prog, "-o", binp])
    rc, bo, be, _, bs = sh(c, proj, env, t)
    bto = rc == 124
    res["build_s"] = round(bs, 1)
    open(os.path.join(logs, name + ".build.log"), "w").write(bo + be)
    if rc != 0 or not os.path.exists(binp):
        res.update(cls="BUILD-FAIL", detail="build timeout" if bto else first_error(bo + be))
        return res
    trc, to_, te, tto, rs = sh([binp], tdir, env, a.run_timeout, stdin, a.as_mb)
    res["run_s"] = round(rs, 1)
    for tag, o, e in (("node", no, ne), ("thaw", to_, te)):
        open(os.path.join(logs, "%s.%s.out" % (name, tag)), "w").write(o)
        open(os.path.join(logs, "%s.%s.err" % (name, tag)), "w").write(e)
    dirs = [ndir, tdir, os.path.realpath(ndir), os.path.realpath(tdir)]
    n_out, t_out = norm(no, dirs), norm(to_, dirs)
    n_err, t_err = norm(ne, dirs), norm(te, dirs)
    n_tree = "\n".join(snapshot(ndir, prog))
    t_tree = "\n".join(snapshot(tdir, prog))
    if tto:
        res.update(cls="RUN-FAIL", detail="timeout after %ds" % a.run_timeout)
    elif trc != 0 and nrc == 0:
        res.update(cls="RUN-FAIL", detail=first_error(te) if te.strip() else "exit %d" % trc)
    elif trc != nrc:
        res.update(cls="DIFF", detail="exit code node=%d thaw=%d" % (nrc, trc))
    elif n_out != t_out:
        res.update(cls="DIFF", detail=first_diff(n_out, t_out, "stdout"))
    elif n_err != t_err:
        res.update(cls="DIFF", detail=first_diff(n_err, t_err, "stderr"))
    elif n_tree != t_tree:
        res.update(cls="DIFF", detail=first_diff(n_tree, t_tree, "tree"))
    else:
        res.update(cls="MATCH")
    return res


def family(detail):
    return re.sub(r"\d+", "N", detail)[:120]


def load_known(path):
    known = {}
    if path and os.path.exists(path):
        for l in open(path, encoding="utf-8"):
            p = l.rstrip("\n").split("\t")
            if len(p) >= 2 and not l.startswith("#") and p[0] != "program":
                known[p[0]] = p[1]
    return known


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--programs", default=os.path.join(HERE, "programs"))
    ap.add_argument("--out", required=True)
    ap.add_argument("--thaw", required=True)
    ap.add_argument("--node", default="node")
    ap.add_argument("--only", default="", help="comma list of program names")
    ap.add_argument("--shard", default="1/1")
    ap.add_argument("--known", default=os.path.join(HERE, "known-gaps.tsv"))
    ap.add_argument("--write-known", action="store_true")
    ap.add_argument("--build-timeout", type=int, default=int(os.environ.get("APP_CORPUS_BUILD_TIMEOUT", "600")))
    ap.add_argument("--run-timeout", type=int, default=int(os.environ.get("APP_CORPUS_RUN_TIMEOUT", "30")))
    ap.add_argument("--as-mb", type=int, default=int(os.environ.get("APP_CORPUS_RUN_AS_MB", "8192")))
    a = ap.parse_args()
    a.out = os.path.abspath(a.out)
    a.programs = os.path.abspath(a.programs)
    if not os.access(a.thaw, os.X_OK):
        print("harness error: thaw binary not executable: " + a.thaw, file=sys.stderr)
        return 2
    progs = sorted(f for f in os.listdir(a.programs) if f.endswith((".mjs", ".ts")))
    if a.only:
        want = set(a.only.split(","))
        progs = [p for p in progs if p.rsplit(".", 1)[0] in want or p in want]
    i, n = map(int, a.shard.split("/"))
    progs = progs[i - 1::n]
    os.makedirs(a.out, exist_ok=True)
    env = dict(os.environ, TZ="UTC", LANG="C", NO_COLOR="1")
    env.pop("FORCE_COLOR", None)
    results = []
    for p in progs:
        r = run_program(p, a, env)
        results.append(r)
        print("%-11s %-28s %s" % (r["cls"], p, r["detail"]), flush=True)
    with open(os.path.join(a.out, "report.tsv"), "w") as f:
        f.write("program\tclass\tdetail\tbuild_s\trun_s\n")
        for r in results:
            f.write("\t".join(str(r[k]).replace("\t", " ") for k in ("program", "cls", "detail", "build_s", "run_s")) + "\n")
    known = load_known(a.known)
    cnt = Counter(r["cls"] for r in results)
    fails = [r for r in results if r["cls"] not in ("MATCH",)]
    new = [r for r in fails if r["program"] not in known]
    fixed = [p for p in known if any(r["program"] == p and r["cls"] == "MATCH" for r in results)]
    fam = Counter((r["cls"], family(r["detail"])) for r in fails)
    lines = ["programs: %d" % len(results)] + ["%-10s %d" % (c, cnt.get(c, 0)) for c in CLASSES]
    lines += ["", "BUILD-FAIL count: %d" % cnt.get("BUILD-FAIL", 0), "", "failure families:"]
    lines += ["  %3d  %-10s %s" % (v, c, d) for (c, d), v in fam.most_common()]
    lines += ["", "NEW failures vs known-gaps (%d):" % len(new)] + ["  %s %s %s" % (r["program"], r["cls"], r["detail"]) for r in new]
    lines += ["", "FIXED vs known-gaps (%d):" % len(fixed)] + ["  " + p for p in fixed]
    open(os.path.join(a.out, "summary.txt"), "w").write("\n".join(lines) + "\n")
    print("\n".join(lines))
    if a.write_known:
        with open(a.known, "w") as f:
            f.write("program\tclass\tfirst error line\n")
            for r in fails:
                f.write("%s\t%s\t%s\n" % (r["program"], r["cls"], r["detail"].replace("\t", " ")))
    return 2 if cnt.get("NODE-FAIL") else 0


if __name__ == "__main__":
    sys.exit(main())
