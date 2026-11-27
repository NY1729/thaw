#!/usr/bin/env python3
"""Request-side wire differential: odd/malformed/edge raw requests against one echo server, Node vs thaw (see README)."""
import argparse, hashlib, json, os, sys
import wire_diff as w

ECHO = """import * as http from 'node:http';
function main(): void {
  const srv = http.createServer((req, res) => {
    let b = '';
    req.on('data', (c: any) => { b += String(c); });
    req.on('end', () => {
      res.writeHead(200, { 'Content-Type': 'text/plain' });
      res.end(req.method + ' ' + req.url + ' HTTP/' + req.httpVersion + '\\n' + JSON.stringify(req.headers) + '\\nbody=' + b);
    });
  });
  srv.listen(Number(process.env.WIRE_PORT), '127.0.0.1');
}
"""
H = "Host: x\r\n"
def req(line, hdrs=H, body=""): return f"{line}\r\n{hdrs}\r\n{body}"
V = {
 "no-target": "GET\r\n\r\n",
 "bad-version": req("GET /q HTTP/9.9"),
 "lower-method": req("get /q HTTP/1.1"),
 "no-host-11": req("GET /q HTTP/1.1", ""),
 "no-host-10": req("GET /q HTTP/1.0", ""),
 "dup-host": req("GET /q HTTP/1.1", H + H),
 "dup-cl-same": req("POST /q HTTP/1.1", H + "Content-Length: 5\r\nContent-Length: 5\r\n", "abcde"),
 "dup-cl-conflict": req("POST /q HTTP/1.1", H + "Content-Length: 5\r\nContent-Length: 6\r\n", "abcde"),
 "cl-and-te": req("POST /q HTTP/1.1", H + "Content-Length: 5\r\nTransfer-Encoding: chunked\r\n", "3\r\nabc\r\n0\r\n\r\n"),
 "dup-custom": req("GET /q HTTP/1.1", H + "X-Dup: a\r\nX-Dup: b\r\nx-dup: c\r\n"),
 "dup-cookie": req("GET /q HTTP/1.1", H + "Cookie: a=1\r\nCookie: b=2\r\n"),
 "dup-set-cookie-req": req("GET /q HTTP/1.1", H + "Set-Cookie: a=1\r\nSet-Cookie: b=2\r\n"),
 "dup-user-agent": req("GET /q HTTP/1.1", H + "User-Agent: a\r\nUser-Agent: b\r\n"),
 "header-16k": req("GET /q HTTP/1.1", H + "X-Big: " + "a" * 16000 + "\r\n"),
 "header-100k": req("GET /q HTTP/1.1", H + "X-Big: " + "a" * 100000 + "\r\n"),
 "headers-300": req("GET /q HTTP/1.1", H + "".join(f"X-H{i}: v\r\n" for i in range(300))),
 "url-20k": req("GET /" + "a" * 20000 + " HTTP/1.1"),
 "chunked-body": req("POST /q HTTP/1.1", H + "Transfer-Encoding: chunked\r\n", "3\r\nabc\r\n2\r\nde\r\n0\r\n\r\n"),
 "chunked-ext": req("POST /q HTTP/1.1", H + "Transfer-Encoding: chunked\r\n", "3;ext=1\r\nabc\r\n0\r\n\r\n"),
 "chunked-trailer": req("POST /q HTTP/1.1", H + "Transfer-Encoding: chunked\r\n", "3\r\nabc\r\n0\r\nX-T: 1\r\n\r\n"),
 "chunked-bad-size": req("POST /q HTTP/1.1", H + "Transfer-Encoding: chunked\r\n", "zz\r\nabc\r\n0\r\n\r\n"),
 "chunked-upper-hex": req("POST /q HTTP/1.1", H + "Transfer-Encoding: chunked\r\n", "A\r\n0123456789\r\n0\r\n\r\n"),
 "chunked-then-get": req("POST /q HTTP/1.1", H + "Transfer-Encoding: chunked\r\n", "3\r\nabc\r\n0\r\n\r\n") + req("GET /q2 HTTP/1.1", H + "Connection: close\r\n"),
 "expect-100": req("POST /q HTTP/1.1", H + "Expect: 100-continue\r\nContent-Length: 5\r\n", "abcde"),
 "expect-other": req("POST /q HTTP/1.1", H + "Expect: foo\r\nContent-Length: 5\r\n", "abcde"),
 "upgrade-ws": req("GET /q HTTP/1.1", H + "Connection: Upgrade\r\nUpgrade: websocket\r\n"),
 "upgrade-h2c": req("GET /q HTTP/1.1", H + "Connection: Upgrade, HTTP2-Settings\r\nUpgrade: h2c\r\nHTTP2-Settings: AAMAAABkAAQAAP__\r\n"),
 "pipeline-3": req("GET /q1 HTTP/1.1") + req("POST /q2 HTTP/1.1", H + "Content-Length: 3\r\n", "abc") + req("GET /q3 HTTP/1.1", H + "Connection: close\r\n"),
 "pipeline-close-mid": req("GET /q1 HTTP/1.1", H + "Connection: close\r\n") + req("GET /q2 HTTP/1.1"),
 "obs-fold": req("GET /q HTTP/1.1", H + "X-F: a\r\n  b\r\n"),
 "space-before-colon": req("GET /q HTTP/1.1", H + "X-S : a\r\n"),
 "header-no-colon": req("GET /q HTTP/1.1", H + "garbage\r\n"),
 "bare-lf": "GET /q HTTP/1.1\nHost: x\n\n",
 "absolute-uri": req("GET http://y/q HTTP/1.1"),
 "options-star": req("OPTIONS * HTTP/1.1"),
 "connect": req("CONNECT x:80 HTTP/1.1"),
 "head-with-body": req("HEAD /q HTTP/1.1", H + "Content-Length: 3\r\n", "abc"),
 "get-with-body": req("GET /q HTTP/1.1", H + "Content-Length: 3\r\n", "abc"),
 "cl-negative": req("POST /q HTTP/1.1", H + "Content-Length: -1\r\n", "abc"),
 "cl-not-number": req("POST /q HTTP/1.1", H + "Content-Length: abc\r\n", "abc"),
 "te-identity": req("POST /q HTTP/1.1", H + "Transfer-Encoding: identity\r\nContent-Length: 3\r\n", "abc"),
 "te-gzip-chunked": req("POST /q HTTP/1.1", H + "Transfer-Encoding: gzip, chunked\r\n", "3\r\nabc\r\n0\r\n\r\n"),
 "conn-keepalive-close": req("GET /q HTTP/1.1", H + "Connection: keep-alive, close\r\n"),
 "header-case-mix": req("GET /q HTTP/1.1", "hOsT: x\r\nX-MiXeD-CaSe: V\r\nACCEPT: */*\r\n"),
 "empty-header-value": req("GET /q HTTP/1.1", H + "X-E:\r\nX-F:   \r\n"),
 "nul-in-value": req("GET /q HTTP/1.1", H + "X-N: a\x00b\r\n"),
 "utf8-in-value": req("GET /q HTTP/1.1", H + "X-U: caf\xc3\xa9\r\n"),
 "leading-crlf": "\r\n\r\nGET /q HTTP/1.1\r\nHost: x\r\n\r\n",
}

def main():
    ap = argparse.ArgumentParser(); ap.add_argument("--write-baseline", action="store_true"); ap.add_argument("--label", default="thaw")
    a = ap.parse_args(); a.scenes = len(V); a.seed = 0
    os.makedirs(w.WORK, exist_ok=True)
    scenes = [dict(id=i, variant=n, raw=r) for i, (n, r) in enumerate(V.items())]
    tid = w.thaw_id()
    ckey = hashlib.sha256((tid + ECHO + json.dumps(scenes)).encode()).hexdigest()[:20]
    cfile = os.path.join(w.WORK, f"req-{ckey}.json")
    if os.path.exists(cfile):
        d = json.load(open(cfile)); dec = lambda r: {int(k): (bytes.fromhex(v[0]) if v[0] else None, v[1]) for k, v in r.items()}
        node, thaw, failed = dec(d["node"]), dec(d["thaw"]), {int(k): v for k, v in d["failed"].items()}
    else:
        d = os.path.join(w.WORK, "req"); os.makedirs(d, exist_ok=True)
        open(os.path.join(d, "echo.ts"), "w").write(ECHO + "main();\n")
        node = w.run_server(["node", "--experimental-strip-types", "--no-warnings", os.path.join(d, "echo.ts")], os.environ, scenes)
        key = hashlib.sha256((tid + ECHO).encode()).hexdigest()[:20]
        bd = os.path.join(w.WORK, "build", "req-" + key); os.makedirs(bd, exist_ok=True)
        open(os.path.join(bd, "p.ts"), "w").write(ECHO)
        binp = os.path.join(bd, "p.bin"); failed, thaw = {}, {}
        if not os.path.exists(binp):
            import subprocess
            p = subprocess.run(w.WRAP + [w.THAW, "build", os.path.join(bd, "p.ts"), "-o", binp], cwd=bd, capture_output=True, text=True, timeout=w.BUILD_TIMEOUT)
            if p.returncode: failed = {s["id"]: (p.stdout + p.stderr)[-2000:] for s in scenes}
        if not failed: thaw = w.run_server([binp], os.environ, scenes)
        enc = lambda r: {str(k): [v[0].hex() if v[0] else None, v[1]] for k, v in r.items()}
        json.dump({"node": enc(node), "thaw": enc(thaw), "failed": {str(k): v for k, v in failed.items()}}, open(cfile, "w"))
    rows = w.classify(scenes, node, thaw, failed)
    a.label = "req-" + a.label
    w.report(a, rows, tid, dims=("variant",), base_file="baseline-req.json")
    for r in rows:
        if r[1] != "SAME": print(r[1], r[0]["variant"], (r[4] if r[1] == "DIFF" else r[2][:80]))

if __name__ == "__main__": main()
