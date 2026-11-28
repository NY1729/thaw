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
 ("seq-default", "http.get({ host: '127.0.0.1', port, path: '/1' }, (r) => { r.resume(); r.on('end', () => { http.get({ host: '127.0.0.1', port, path: '/2' }, (q) => { q.resume(); q.on('end', () => { console.log('done'); }); }); }); });"),
 ("seq-agent-false", "http.get({ host: '127.0.0.1', port, path: '/1', agent: false }, (r) => { r.resume(); r.on('end', () => { http.get({ host: '127.0.0.1', port, path: '/2', agent: false }, (q) => { q.resume(); q.on('end', () => { console.log('done'); }); }); }); });"),
 ("seq-keepalive-false", "const ag = new http.Agent({ keepAlive: false }); http.get({ host: '127.0.0.1', port, path: '/1', agent: ag }, (r) => { r.resume(); r.on('end', () => { http.get({ host: '127.0.0.1', port, path: '/2', agent: ag }, (q) => { q.resume(); q.on('end', () => { console.log('done'); }); }); }); });"),
 ("conc-default", "let n = 0; const fin = () => { n++; if (n === 2) console.log('done'); }; http.get({ host: '127.0.0.1', port, path: '/1' }, (r) => { r.resume(); r.on('end', fin); }); http.get({ host: '127.0.0.1', port, path: '/2' }, (r) => { r.resume(); r.on('end', fin); });"),
 ("conc-maxsockets-1", "const ag = new http.Agent({ keepAlive: true, maxSockets: 1 }); let n = 0; const fin = () => { n++; if (n === 2) console.log('done'); }; http.get({ host: '127.0.0.1', port, path: '/1', agent: ag }, (r) => { r.resume(); r.on('end', fin); }); http.get({ host: '127.0.0.1', port, path: '/2', agent: ag }, (r) => { r.resume(); r.on('end', fin); });"),
 ("seq-maxfreesockets-0", "const ag = new http.Agent({ keepAlive: true, maxFreeSockets: 0 }); http.get({ host: '127.0.0.1', port, path: '/1', agent: ag }, (r) => { r.resume(); r.on('end', () => { http.get({ host: '127.0.0.1', port, path: '/2', agent: ag }, (q) => { q.resume(); q.on('end', () => { console.log('done'); }); }); }); });"),
 ("seq-server-close", "http.get({ host: '127.0.0.1', port, path: '/close' }, (r) => { r.resume(); r.on('end', () => { setTimeout(() => { http.get({ host: '127.0.0.1', port, path: '/2' }, (q) => { q.resume(); q.on('end', () => { console.log('done'); }); }); }, 50); }); });"),
 ("agent-destroy-then-request", "const ag = new http.Agent({ keepAlive: true }); http.get({ host: '127.0.0.1', port, path: '/1', agent: ag }, (r) => { r.resume(); r.on('end', () => { ag.destroy(); http.get({ host: '127.0.0.1', port, path: '/2', agent: ag }, (q) => { q.resume(); q.on('end', () => { console.log('done'); }); }); }); });"),
 ("free-counts", "const ag = new http.Agent({ keepAlive: true }); http.get({ host: '127.0.0.1', port, path: '/1', agent: ag }, (r) => { r.resume(); r.on('end', () => { setTimeout(() => { console.log(Object.keys(ag.sockets).length, Object.keys(ag.freeSockets).length); }, 20); }); });"),
 ("request-timeout", "const q = http.request({ host: '127.0.0.1', port, path: '/hang', timeout: 100 }, (r) => { r.resume(); }); q.on('timeout', () => { console.log('timeout'); q.destroy(); }); q.on('error', (e: any) => { console.log('error', e.code); }); q.end();"),
 ("agent-scheduling-fifo", "const ag = new http.Agent({ keepAlive: true, scheduling: 'fifo' }); let n = 0; const next = () => { http.get({ host: '127.0.0.1', port, path: '/3', agent: ag }, (r) => { r.resume(); r.on('end', () => { console.log('done'); }); }); }; const fin = () => { n++; if (n === 2) setTimeout(next, 20); }; http.get({ host: '127.0.0.1', port, path: '/1', agent: ag }, (r) => { r.resume(); r.on('end', fin); }); http.get({ host: '127.0.0.1', port, path: '/2', agent: ag }, (r) => { r.resume(); r.on('end', fin); });"),
 ("agent-scheduling-lifo", "const ag = new http.Agent({ keepAlive: true, scheduling: 'lifo' }); let n = 0; const next = () => { http.get({ host: '127.0.0.1', port, path: '/3', agent: ag }, (r) => { r.resume(); r.on('end', () => { console.log('done'); }); }); }; const fin = () => { n++; if (n === 2) setTimeout(next, 20); }; http.get({ host: '127.0.0.1', port, path: '/1', agent: ag }, (r) => { r.resume(); r.on('end', fin); }); http.get({ host: '127.0.0.1', port, path: '/2', agent: ag }, (r) => { r.resume(); r.on('end', fin); });"),
]

def program():
    out = ["import * as http from 'node:http';", "function main(): void {", "  const port = Number(process.env.WIRE_PORT);",
           "  const k = Number(process.env.WIRE_SCENE);", "  const cb = (res: http.IncomingMessage) => { res.resume(); };"]
    for i, (_, code) in enumerate(S):
        out.append(f"  if (k === {i}) {{ {code} return; }}")
    return "\n".join(out) + "\n}\n"

def capture(cmd, scene):
    """Runs the program against a small multi-connection server; the record is every connection's request bytes plus stdout."""
    srv = socket.socket(); srv.bind(("127.0.0.1", 0)); srv.listen(16); srv.settimeout(0.2); port = srv.getsockname()[1]
    p = subprocess.Popen(cmd, env=dict(os.environ, WIRE_PORT=str(port), WIRE_SCENE=str(scene)), stdout=subprocess.PIPE, stderr=subprocess.DEVNULL)
    conns, bufs, lock = [], {}, threading.Lock()
    def serve(c, idx):
        c.settimeout(0.3); buf = b""; answered = 0
        while True:
            try:
                d = c.recv(65536)
            except socket.timeout:
                d = None
            except OSError:
                break
            if d == b"": break
            if d: buf += d; bufs[idx] = buf
            while True:  # answer every complete request head in the buffer
                end = buf.find(b"\r\n\r\n", answered)
                if end < 0: break
                head = buf[answered:end]; start = answered
                if b"Expect: 100" in head and not head.endswith(b"\0"):
                    c.sendall(b"HTTP/1.1 100 Continue\r\n\r\n")
                path = head.split(b" ")[1] if b" " in head else b""
                answered = end + 4
                cl = 0
                for line in head.split(b"\r\n"):
                    if line.lower().startswith(b"content-length:"): cl = int(line.split(b":")[1])
                answered += cl
                if b"transfer-encoding: chunked" in head.lower():
                    stop = buf.find(b"0\r\n\r\n", answered)
                    if stop < 0:
                        answered = start  # the rest of the body has not arrived; look again later
                        break
                    answered = stop + 5
                if path == b"/hang": continue
                close = path == b"/close"
                c.sendall(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n" + (b"Connection: close\r\n" if close else b"") + b"\r\nok")
                if close:
                    c.close(); return
        c.close()
    deadline = time.time() + 12; last = time.time() + 3; threads = []  # the program may need a few seconds to start
    while time.time() < deadline and time.time() - last < 1.2:
        try:
            c, _ = srv.accept(); idx = len(threads); threads.append(threading.Thread(target=serve, args=(c, idx), daemon=True)); threads[-1].start(); last = time.time()
        except socket.timeout:
            pass
        for i in list(bufs): last = max(last, time.time() - 0.9) if False else last
        if p.poll() is not None and time.time() - last > 0.6: break
    exited = p.poll() is not None
    p.kill(); out = p.stdout.read(); p.wait(); srv.close()
    if not bufs and not threads: return None
    rec = b"\n---conn---\n".join(bufs.get(i, b"") for i in range(len(threads))) + b"\n---stdout---\n" + out + (b"" if exited else b"\n---still running---")
    return rec.replace(str(port).encode(), b"<port>")

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
