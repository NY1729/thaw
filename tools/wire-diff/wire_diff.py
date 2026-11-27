#!/usr/bin/env python3
"""Wire-level (raw bytes) differential test of node:http servers: Node 22 is the oracle, thaw (AOT) the subject.

Scenes = (server handler, client request bytes), sampled from a combinatorial space with a seed. All handlers
of one run live in ONE program (path /s<N> selects the handler) so one `thaw build` covers many scenes.
Classes per scene: SAME, DIFF (line diff recorded), BUILD-FAIL (thaw build error, bisected to the scene), HANG.
Only the Date and Server header values are masked. See README.md.
"""
import argparse, concurrent.futures as cf, difflib, hashlib, itertools, json, os, random, re, shlex, socket
import subprocess, sys, time, collections

HERE = os.path.dirname(os.path.abspath(__file__))
WORK = os.environ.get("WIRE_WORK", "/home/fedora/work/thaw-tmp/wire-diff-work")
THAW = os.environ.get("THAW", "thaw")
WRAP = shlex.split(os.environ.get("WIRE_BUILD_WRAP", ""))
BUILD_TIMEOUT = int(os.environ.get("WIRE_BUILD_TIMEOUT", "2400"))

# ---- server side dimensions -------------------------------------------------------------------------------
STATUSES = [100, 101, 200, 201, 204, 301, 304, 400, 404, 500]
BODIES = {  # name -> (code emitting the body, body text or None for "no body bytes")
    "end": ("res.end();", ""),
    "end-data": ("res.end('hello');", "hello"),
    "end-buffer": ("res.end(Buffer.from('hello'));", "hello"),
    "write-end": ("res.write('he'); res.end('llo');", "hello"),
    "write-multi": ("res.write('a'); res.write('b'); Promise.resolve().then(() => { res.end(); });", "ab"),
    "end-undefined": ("res.end(undefined);", ""),
    "end-empty": ("res.end('');", ""),
    "end-large": ("res.end('x'.repeat(100000));", "x" * 100000),
    "end-nonascii": ("res.end('h\\u00e9llo \\u2713 \\u65e5\\u672c');", "héllo ✓ 日本"),
}
def blen(text): return len(text.encode())
def hdr_styles(status, body):
    n = blen(body)
    s = str(status)
    return {
        "wh-status": f"res.writeHead({s});",
        "wh-obj": f"res.writeHead({s}, {{ 'X-A': '1', 'Content-Type': 'text/plain' }});",
        "wh-msg-obj": f"res.writeHead({s}, 'Custom Msg', {{ 'X-A': '1' }});",
        "set+wh": f"res.setHeader('X-A', '1'); res.setHeader('Content-Type', 'text/plain'); res.writeHead({s});",
        "set-only": f"res.statusCode = {s}; res.setHeader('X-A', '1'); res.setHeader('Content-Type', 'text/plain');",
        "name-case": f"res.writeHead({s}, {{ 'x-lower': '1', 'X-UPPER-NAME': '2', 'X-MiXeD': '3', 'content-type': 'text/plain' }});",
        "array-value": f"res.writeHead({s}, {{ 'Set-Cookie': ['a=1', 'b=2'], 'X-A': '1' }});",
        "set-twice": f"res.setHeader('X-A', '1'); res.setHeader('X-A', '2'); res.setHeader('x-a', '3'); res.writeHead({s});",
        "cl-explicit": f"res.writeHead({s}, {{ 'X-A': '1', 'Content-Length': '{n}', 'X-B': '2' }});",
        "cl-set-then-wh": f"res.setHeader('Content-Length', '{n}'); res.setHeader('X-A', '1'); res.writeHead({s}, {{ 'X-B': '2' }});",
        "cl-wrong": f"res.writeHead({s}, {{ 'Content-Length': '3' }});",
        "te-explicit": f"res.writeHead({s}, {{ 'Transfer-Encoding': 'chunked', 'X-A': '1' }});",
    }

def handler_code(n, status, hstyle, bstyle):
    body_code, body_text = BODIES[bstyle]
    head = hdr_styles(status, body_text)[hstyle]
    return f"      if (req.url === '/s{n}') {{\n        {head}\n        {body_code}\n        return;\n      }}"

# ---- client side dimensions -------------------------------------------------------------------------------
METHODS = ["GET", "HEAD", "POST", "PUT", "DELETE", "OPTIONS"]
CONNS = ["keepalive", "close", "http10", "http10-keepalive", "pipeline"]
REQBODY = ["none", "content-length", "chunked"]
REQHDR = ["normal", "lower", "upper", "dup"]

def client_bytes(method, path, conn, reqbody, reqhdr):
    ver = "1.0" if conn.startswith("http10") else "1.1"
    def hdr(name): return {"normal": name, "lower": name.lower(), "upper": name.upper(), "dup": name}[reqhdr]
    def one(last):
        h = [f"{method} {path} HTTP/{ver}"]
        if ver == "1.1": h.append(f"{hdr('Host')}: localhost")
        h.append(f"{hdr('User-Agent')}: wire")
        if reqhdr == "dup": h += ["X-Dup: a", "X-Dup: b"]
        c = {"close": "close", "http10-keepalive": "keep-alive", "keepalive": "keep-alive"}.get(conn)
        if conn == "pipeline": c = "close" if last else "keep-alive"
        if c: h.append(f"{hdr('Connection')}: {c}")
        body = b""
        if reqbody == "content-length":
            body = b"abcde"; h.append(f"{hdr('Content-Length')}: 5")
        elif reqbody == "chunked":
            body = b"3\r\nabc\r\n2\r\nde\r\n0\r\n\r\n"; h.append(f"{hdr('Transfer-Encoding')}: chunked")
        return ("\r\n".join(h) + "\r\n\r\n").encode() + body
    return one(False) + one(True) if conn == "pipeline" else one(True)

# ---- scene generation --------------------------------------------------------------------------------------
def gen_scenes(count, seed):
    rnd = random.Random(seed)
    hs = [(s, h, b) for s in STATUSES for h in hdr_styles(200, "").keys() for b in BODIES]
    cs = [(m, c, rb, rh) for m in METHODS for c in CONNS for rb in REQBODY for rh in REQHDR]
    rnd.shuffle(hs); rnd.shuffle(cs)
    scenes = []
    for i in range(count):
        s, h, b = hs[i % len(hs)]
        m, c, rb, rh = cs[(i * 7 + i // len(cs)) % len(cs)]
        scenes.append(dict(id=i, status=s, hstyle=h, bstyle=b, method=m, conn=c, reqbody=rb, reqhdr=rh))
    return scenes

def program(scenes):
    out = ["import * as http from 'node:http';"]
    out.append("function main(): void {\n  const srv = http.createServer((req, res) => {")
    out.append("    try {")
    # handlers are inlined: passing req/res to a typed helper function is itself a thaw gap (see report)
    out += [handler_code(s["id"], s["status"], s["hstyle"], s["bstyle"]) for s in scenes]
    out.append("      res.statusCode = 404; res.end('nf');")
    out.append("    } catch (e) { try { if (!res.headersSent) res.statusCode = 500; res.end('ERR ' + String(e)); } catch (e2) { } }")
    out.append("  });\n  srv.listen(Number(process.env.WIRE_PORT), '127.0.0.1');\n}")
    return "\n".join(out) + "\n"

# ---- running -----------------------------------------------------------------------------------------------
def free_port():
    s = socket.socket(); s.bind(("127.0.0.1", 0)); p = s.getsockname()[1]; s.close(); return p

def exchange(port, data, idle=0.35, total=6.0):
    t0 = time.time(); buf = b""
    try:
        s = socket.create_connection(("127.0.0.1", port), timeout=2)
    except OSError as e:
        return None, f"connect failed: {e}"
    s.sendall(data); s.settimeout(idle); closed = False
    while time.time() - t0 < total:
        try:
            d = s.recv(1 << 16)
        except socket.timeout:
            if buf: break
            if time.time() - t0 > 3: break
            continue
        except OSError:
            closed = True; break
        if not d: closed = True; break
        buf += d
    s.close()
    return buf, ("closed" if closed else "open")

def run_server(cmd, env, scenes):
    port = free_port()
    env = dict(env, WIRE_PORT=str(port))
    p = subprocess.Popen(cmd, env=env, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    for _ in range(100):
        try: socket.create_connection(("127.0.0.1", port), timeout=0.3).close(); break
        except OSError: time.sleep(0.1)
    def one(s):
        return s["id"], exchange(port, client_bytes(s["method"], f"/s{s['id']}", s["conn"], s["reqbody"], s["reqhdr"]))
    res = {}
    try:
        with cf.ThreadPoolExecutor(12) as ex:
            for i, (raw, state) in ex.map(one, scenes):
                res[i] = (raw, state)
    finally:
        p.kill(); p.wait()
    return res

MASK = re.compile(rb"(?im)^(Date|Server): [^\r\n]*")
def norm(raw): return MASK.sub(lambda m: m.group(1) + b": <masked>", raw)

def thaw_id():
    p = subprocess.run(["which", THAW], capture_output=True, text=True).stdout.strip() or THAW
    h = hashlib.sha256()
    with open(p, "rb") as f:
        for c in iter(lambda: f.read(1 << 22), b""): h.update(c)
    return h.hexdigest()[:16]

def build(scenes, tag, tid, depth=0):
    """Build `scenes` as one program; on failure bisect. Returns {id: diag} for failed, and list of (scenes, bin)."""
    src_text = program(scenes)
    key = hashlib.sha256((tid + src_text).encode()).hexdigest()[:20]
    d = os.path.join(WORK, "build", key); os.makedirs(d, exist_ok=True)
    src, binp = os.path.join(d, "p.ts"), os.path.join(d, "p.bin")
    open(src, "w").write(src_text)
    if not os.path.exists(binp):
        try:
            p = subprocess.run(WRAP + [THAW, "build", src, "-o", binp], cwd=d, capture_output=True, text=True, timeout=BUILD_TIMEOUT)
            diag = (p.stdout + p.stderr)[-4000:]; ok = p.returncode == 0 and os.path.exists(binp)
        except subprocess.TimeoutExpired:
            diag, ok = "error: build timed out", False
        if not ok:
            if len(scenes) == 1:
                return {scenes[0]["id"]: diag.strip()}, []
            m = len(scenes) // 2
            f1, b1 = build(scenes[:m], tag, tid, depth + 1); f2, b2 = build(scenes[m:], tag, tid, depth + 1)
            return {**f1, **f2}, b1 + b2
    return {}, [(scenes, binp)]

def build_all(scenes, tag, tid, chunk=100):
    """Probe every header style and body kind alone first (a failing build is cheap and says which construct is
    unsupported), mark scenes using a failing one BUILD-FAIL, then build the rest in chunks (bisecting as a fallback)."""
    failed, bad = {}, {}
    probes = [("hstyle", h) for h in sorted({s["hstyle"] for s in scenes})] + [("bstyle", b) for b in sorted({s["bstyle"] for s in scenes})]
    for dim, val in probes:
        base = dict(id=0, status=200, hstyle="wh-status", bstyle="end-data", method="GET", conn="close", reqbody="none", reqhdr="normal")
        base[dim] = val  # one dimension varies, the others are known-good
        f, _ = build([base], tag, tid)
        if f: bad[(dim, val)] = f[0]
    ok = []
    for s in scenes:
        hit = [(k, v) for k, v in bad.items() if s[k[0]] == k[1]]
        if hit: failed[s["id"]] = f"[probe {hit[0][0][0]}={hit[0][0][1]}] " + hit[0][1]
        else: ok.append(s)
    builds = []
    for i in range(0, len(ok), chunk):
        f, b = build(ok[i:i + chunk], tag, tid); failed.update(f); builds += b
    return failed, builds

def parse_resp(raw):
    """Split into responses by head; returns list of (status_line, [header lines], body bytes) for diff display."""
    return raw.decode("latin-1").split("\r\n")

def line_diff(a, b):
    la, lb = parse_resp(a), parse_resp(b)
    return [l for l in difflib.unified_diff(la, lb, "node", "thaw", lineterm="", n=0) if not l.startswith(("---", "+++", "@@"))]

def signature(a, b):
    """Cause signature of a DIFF from the head lines (node vs thaw)."""
    ha = a.split(b"\r\n\r\n", 1)[0].decode("latin-1").split("\r\n")
    hb = b.split(b"\r\n\r\n", 1)[0].decode("latin-1").split("\r\n")
    sig = []
    if ha[:1] != hb[:1]: sig.append("status-line")
    na = [l.split(":", 1)[0] for l in ha[1:]]; nb = [l.split(":", 1)[0] for l in hb[1:]]
    la = [x.lower() for x in na]; lb = [x.lower() for x in nb]
    for x in sorted(set(lb) - set(la)): sig.append(f"extra-header:{x}")
    for x in sorted(set(la) - set(lb)): sig.append(f"missing-header:{x}")
    common = [x for x in la if x in lb]
    if [x for x in la if x in common] != [x for x in lb if x in common]: sig.append("header-order")
    elif set(na) - set(nb) and set(la) == set(lb): sig.append("header-name-case")
    va = {l.lower().split(":", 1)[0]: l.split(":", 1)[1].strip() for l in ha[1:] if ":" in l}
    vb = {l.lower().split(":", 1)[0]: l.split(":", 1)[1].strip() for l in hb[1:] if ":" in l}
    for k in sorted(set(va) & set(vb)):
        if va[k] != vb[k]: sig.append(f"header-value:{k}")
    if not sig:
        if a.count(b"HTTP/1.") != b.count(b"HTTP/1."): sig.append("response-count")
        else: sig.append("body-or-whitespace")
    elif a.split(b"\r\n\r\n", 1)[1:] != b.split(b"\r\n\r\n", 1)[1:] and not any(s.startswith(("extra-header:transfer", "extra-header:content")) for s in sig):
        sig.append("body")
    return sig

def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--scenes", type=int, default=300); ap.add_argument("--seed", type=int, default=1)
    ap.add_argument("--write-baseline", action="store_true"); ap.add_argument("--label", default="thaw")
    a = ap.parse_args()
    os.makedirs(WORK, exist_ok=True)
    scenes = gen_scenes(a.scenes, a.seed)
    tid = thaw_id()
    # cache of raw responses keyed by (engine id, scene content): rerun without a rebuild when nothing changed
    ckey = hashlib.sha256((tid + program(scenes) + json.dumps(scenes)).encode()).hexdigest()[:20]
    cfile = os.path.join(WORK, f"run-{ckey}.json")
    if os.path.exists(cfile):
        data = json.load(open(cfile))
        node = {int(k): (bytes.fromhex(v[0]) if v[0] is not None else None, v[1]) for k, v in data["node"].items()}
        thaw = {int(k): (bytes.fromhex(v[0]) if v[0] is not None else None, v[1]) for k, v in data["thaw"].items()}
        failed = {int(k): v for k, v in data["failed"].items()}
        print("cache hit", cfile)
    else:
        os.makedirs(os.path.join(WORK, "node"), exist_ok=True)
        npath = os.path.join(WORK, "node", "server.ts")
        open(npath, "w").write(program(scenes) + "main();\n")  # thaw auto-invokes main(); Node does not
        # Node needs plain JS: strip type annotations with node's built-in type stripping (node >= 22.6)
        node = run_server(["node", "--experimental-strip-types", "--no-warnings", npath], os.environ, scenes)
        failed, builds = build_all(scenes, a.label, tid)
        thaw = {}
        for sub, binp in builds:
            thaw.update(run_server([binp], os.environ, sub))
        enc = lambda r: {str(k): [v[0].hex() if v[0] is not None else None, v[1]] for k, v in r.items()}
        json.dump({"node": enc(node), "thaw": enc(thaw), "failed": {str(k): v for k, v in failed.items()}}, open(cfile, "w"))
    rows = []
    for s in scenes:
        i = s["id"]
        if i in failed: rows.append((s, "BUILD-FAIL", failed[i], [])); continue
        nr, tr = node.get(i, (None, "")), thaw.get(i, (None, ""))
        if not nr[0] or not tr[0]:
            cls = "HANG" if (not tr[0]) else "NODE-NO-RESPONSE"
            rows.append((s, cls, tr[1] if not tr[0] else nr[1], [])); continue
        x, y = norm(nr[0]), norm(tr[0])
        if x == y and nr[1] == tr[1]: rows.append((s, "SAME", "", []))
        elif x == y: rows.append((s, "DIFF", "", [f"connection state: node={nr[1]} thaw={tr[1]}"], ["connection-state"]))
        else: rows.append((s, "DIFF", "", line_diff(x, y), signature(x, y)))
    report(a, rows, tid)

def report(a, rows, tid):
    cnt = collections.Counter(r[1] for r in rows)
    out = [f"# wire-diff report (scenes={a.scenes} seed={a.seed} thaw-id={tid})", ""]
    out.append("counts: " + ", ".join(f"{k}={cnt[k]}" for k in ("SAME", "DIFF", "BUILD-FAIL", "HANG", "NODE-NO-RESPONSE")))
    groups = collections.defaultdict(list)
    for r in rows:
        if r[1] == "DIFF": groups["; ".join(r[4])].append(r)
        elif r[1] != "SAME": groups[r[1] + ": " + (r[2].splitlines()[0][:100] if r[2] else "")].append(r)
    out.append("\n## DIFF groups by cause signature (largest first)\n")
    dims = ["status", "hstyle", "bstyle", "method", "conn", "reqbody", "reqhdr"]
    for sig, rs in sorted(groups.items(), key=lambda kv: -len(kv[1])):
        out.append(f"### [{len(rs)}] {sig}")
        for d in dims:
            c = collections.Counter(str(r[0][d]) for r in rs)
            tot = collections.Counter(str(r[0][d]) for r in rows)
            top = ", ".join(f"{k}:{v}/{tot[k]}" for k, v in c.most_common(4))
            out.append(f"- {d}: {top}")
        for r in rs[:3]:
            s = r[0]
            out.append(f"- example s{s['id']} {s['method']} status={s['status']} {s['hstyle']} {s['bstyle']} conn={s['conn']} reqbody={s['reqbody']} reqhdr={s['reqhdr']}")
            for l in (r[3] or [r[2]])[:6]: out.append("    " + l.replace("\r", "")[:160])
        out.append("")
    rp = os.path.join(WORK, f"report-{a.label}-{a.scenes}-{a.seed}.md")
    open(rp, "w").write("\n".join(out)); print("\n".join(out[:3])); print("report:", rp)
    base = {"scenes": a.scenes, "seed": a.seed, "same": cnt["SAME"], "diff": cnt["DIFF"], "build_fail": cnt["BUILD-FAIL"], "hang": cnt["HANG"]}
    bf = os.path.join(HERE, "baseline.json")
    if a.write_baseline: json.dump(base, open(bf, "w"), indent=1); print("baseline written")
    elif os.path.exists(bf):
        b = json.load(open(bf))
        if (b["scenes"], b["seed"]) == (a.scenes, a.seed):
            print(f"baseline SAME={b['same']} now SAME={cnt['SAME']}" + ("  REGRESSION" if cnt["SAME"] < b["same"] else ""))

if __name__ == "__main__":
    main()
