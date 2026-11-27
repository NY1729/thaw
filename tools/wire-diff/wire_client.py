#!/usr/bin/env python3
"""Client-side wire differential: the bytes http.request/http.get send, Node vs thaw (one program, scene chosen by WIRE_SCENE)."""
import argparse, hashlib, json, os, socket, subprocess, threading, time
import wire_diff as w

S = [  # (name, statement inside main(); `port` is in scope)
 ("get", "http.request({ host: '127.0.0.1', port, path: '/a?x=1', method: 'GET' }, cb).end();"),
 ("headers-obj", "http.request({ host: '127.0.0.1', port, path: '/', headers: { 'X-A': '1', 'x-b': '2' } }, cb).end();"),
 ("post-write-end", "const r = http.request({ host: '127.0.0.1', port, method: 'POST', path: '/' }, cb); r.write('ab'); r.end('cd');"),
 ("post-end-data", "http.request({ host: '127.0.0.1', port, method: 'POST', path: '/' }, cb).end('data');"),
 ("post-explicit-cl", "const r = http.request({ host: '127.0.0.1', port, method: 'POST', path: '/', headers: { 'Content-Length': '4' } }, cb); r.end('data');"),
 ("post-te-chunked", "const r = http.request({ host: '127.0.0.1', port, method: 'POST', path: '/', headers: { 'Transfer-Encoding': 'chunked' } }, cb); r.write('ab'); r.end('cd');"),
 ("setheader", "const r = http.request({ host: '127.0.0.1', port, path: '/' }, cb); r.setHeader('X-A', '1'); r.setHeader('x-a', '2'); r.end();"),
 ("head", "http.request({ host: '127.0.0.1', port, method: 'HEAD', path: '/' }, cb).end();"),
 ("delete", "http.request({ host: '127.0.0.1', port, method: 'DELETE', path: '/x' }, cb).end();"),
 ("put-buffer", "http.request({ host: '127.0.0.1', port, method: 'PUT', path: '/' }, cb).end(Buffer.from('xyz'));"),
 ("agent-false", "http.request({ host: '127.0.0.1', port, path: '/', agent: false }, cb).end();"),
 ("get-url-string", "http.get('http://127.0.0.1:' + port + '/u?q=1', cb);"),
 ("path-unicode", "http.request({ host: '127.0.0.1', port, path: '/%E6%97%A5?a=b&c=d' }, cb).end();"),
 ("header-array", "http.request({ host: '127.0.0.1', port, path: '/', headers: { 'X-M': ['a', 'b'] } }, cb).end();"),
 ("host-localhost", "http.request({ host: 'localhost', port, path: '/' }, cb).end();"),
 ("auth", "http.request({ host: '127.0.0.1', port, path: '/', auth: 'user:pw' }, cb).end();"),
 ("custom-host", "http.request({ host: '127.0.0.1', port, path: '/', headers: { Host: 'example.test' } }, cb).end();"),
 ("keepalive-agent", "http.request({ host: '127.0.0.1', port, path: '/', agent: new http.Agent({ keepAlive: true }) }, cb).end();"),
 ("post-write-write-end", "const r = http.request({ host: '127.0.0.1', port, method: 'POST', path: '/' }, cb); r.write('a'); r.write('b'); r.end();"),
 ("options", "http.request({ host: '127.0.0.1', port, method: 'OPTIONS', path: '*' }, cb).end();"),
 ("url-and-opts", "http.request('http://127.0.0.1:' + port + '/z', { method: 'POST', headers: { 'X-B': '1' } }, cb).end('p');"),
 ("expect-100", "http.request({ host: '127.0.0.1', port, method: 'POST', path: '/', headers: { Expect: '100-continue', 'Content-Length': '2' } }, cb).end('hi');"),
 ("json-body", "const b = JSON.stringify({ a: 1 }); http.request({ host: '127.0.0.1', port, method: 'POST', path: '/', headers: { 'Content-Type': 'application/json', 'Content-Length': String(b.length) } }, cb).end(b);"),
 ("connection-close", "http.request({ host: '127.0.0.1', port, path: '/', headers: { Connection: 'close' } }, cb).end();"),
 ("lower-method", "http.request({ host: '127.0.0.1', port, method: 'post', path: '/' }, cb).end('q');"),
 ("nonascii-body", "http.request({ host: '127.0.0.1', port, method: 'POST', path: '/' }, cb).end('h\\u00e9\\u2713');"),
]

def program():
    out = ["import * as http from 'node:http';", "function main(): void {", "  const port = Number(process.env.WIRE_PORT);",
           "  const k = Number(process.env.WIRE_SCENE);", "  const cb = (res: http.IncomingMessage) => { res.resume(); };"]
    for i, (_, code) in enumerate(S):
        out.append(f"  if (k === {i}) {{ {code} return; }}")
    return "\n".join(out) + "\n}\n"

def capture(cmd, scene):
    srv = socket.socket(); srv.bind(("127.0.0.1", 0)); srv.listen(1); srv.settimeout(15); port = srv.getsockname()[1]
    p = subprocess.Popen(cmd, env=dict(os.environ, WIRE_PORT=str(port), WIRE_SCENE=str(scene)), stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    buf = b""
    try:
        c, _ = srv.accept(); c.settimeout(0.5)
        while True:
            try:
                d = c.recv(65536)
            except socket.timeout:
                break
            if not d: break
            buf += d
            if b"\r\n\r\n" in buf and b"Expect: 100" in buf and b"HTTP/1.1 100" not in buf:
                c.sendall(b"HTTP/1.1 100 Continue\r\n\r\n")
                buf += b"" ; continue
            if b"\r\n\r\n" in buf: break
        time.sleep(0.2)
        try:
            c.settimeout(0.3); buf += c.recv(65536)
        except Exception: pass
        c.sendall(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok"); c.close()
    except socket.timeout:
        buf = buf or None
    finally:
        p.kill(); p.wait(); srv.close()
    return buf.replace(str(port).encode(), b"<port>") if buf else buf

def main():
    ap = argparse.ArgumentParser(); ap.add_argument("--write-baseline", action="store_true"); ap.add_argument("--label", default="thaw")
    a = ap.parse_args(); a.scenes = len(S); a.seed = 0
    os.makedirs(w.WORK, exist_ok=True); tid = w.thaw_id(); src = program()
    d = os.path.join(w.WORK, "client"); os.makedirs(d, exist_ok=True)
    open(os.path.join(d, "c.ts"), "w").write(src + "main();\n")
    key = hashlib.sha256((tid + src).encode()).hexdigest()[:20]
    cfile = os.path.join(w.WORK, f"client-{key}.json")
    scenes = [dict(id=i, variant=n) for i, (n, _) in enumerate(S)]
    if os.path.exists(cfile):
        dd = json.load(open(cfile)); dec = lambda r: {int(k): (bytes.fromhex(v[0]) if v[0] else None, v[1]) for k, v in r.items()}
        node, thaw, failed = dec(dd["node"]), dec(dd["thaw"]), {int(k): v for k, v in dd["failed"].items()}
    else:
        node = {i: (capture(["node", "--experimental-strip-types", "--no-warnings", os.path.join(d, "c.ts")], i), "closed") for i in range(len(S))}
        bd = os.path.join(w.WORK, "build", "client-" + key); os.makedirs(bd, exist_ok=True)
        open(os.path.join(bd, "p.ts"), "w").write(src)
        binp = os.path.join(bd, "p.bin"); failed, thaw = {}, {}
        if not os.path.exists(binp):
            p = subprocess.run(w.WRAP + [w.THAW, "build", os.path.join(bd, "p.ts"), "-o", binp], cwd=bd, capture_output=True, text=True, timeout=w.BUILD_TIMEOUT)
            if p.returncode: failed = {i: (p.stdout + p.stderr)[-2000:] for i in range(len(S))}
        if not failed: thaw = {i: (capture([binp], i), "closed") for i in range(len(S))}
        enc = lambda r: {str(k): [v[0].hex() if v[0] else None, v[1]] for k, v in r.items()}
        json.dump({"node": enc(node), "thaw": enc(thaw), "failed": {str(k): v for k, v in failed.items()}}, open(cfile, "w"))
    rows = w.classify(scenes, node, thaw, failed)
    a.label = "client-" + a.label
    w.report(a, rows, tid, dims=("variant",), base_file="baseline-client.json")
    for r in rows:
        if r[1] != "SAME": print(r[1], r[0]["variant"], (r[4] if r[1] == "DIFF" else r[2][:100]))

if __name__ == "__main__": main()
