fn run_conformance_program(name: &str, source: &str) -> (String, String, Option<i32>) {
    let dir = std::env::temp_dir().join(format!("thaw-cli-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(&entry, source).unwrap();
    let output = dir.join("app");
    build(&entry, &output, &[], &[], &[], &dir.join("registry"), &[]).unwrap();
    let result = Command::new(output).output().unwrap();
    let _ = std::fs::remove_dir_all(dir);
    (
        String::from_utf8_lossy(&result.stdout).into_owned(),
        String::from_utf8_lossy(&result.stderr).into_owned(),
        result.status.code(),
    )
}

/// `Promise.reject()` rejects with `undefined` (Node 22: `caught undefined`, then `UR undefined`).
#[test]
fn conformance_promise_reject_without_argument_rejects_with_undefined() {
    let (stdout, stderr, code) = run_conformance_program(
        "fc-promise-reject",
        r#"process.on("unhandledRejection", (r: any) => { console.log("UR", r); });
Promise.reject();
Promise.reject().catch((e: any) => console.log("caught", e));
"#,
    );
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "caught undefined\nUR undefined\n");
}

/// A block-bodied callback without a value `return` satisfies a `() => any` parameter (it yields `undefined`).
#[test]
fn conformance_void_callback_satisfies_any_result() {
    let (stdout, stderr, code) = run_conformance_program(
        "fc-void-callback",
        r#"function t(f: () => any): void {
  try { const r = f(); console.log("OK", String(r)); } catch (e: any) { console.log("ERR", e.constructor.name); }
}
const o: any = undefined;
t(() => { o.x = 1; });
t(() => { return; });
t(() => "x".repeat(-1));
t(() => new ArrayBuffer(-1));
"#,
    );
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(
        stdout,
        "ERR TypeError\nOK undefined\nERR RangeError\nERR RangeError\n"
    );
}

/// `(0, eval)(code)` is an indirect eval, like `eval(code)`.
#[test]
fn conformance_indirect_eval_through_a_sequence_expression() {
    let (stdout, stderr, code) = run_conformance_program(
        "fc-indirect-eval",
        r#"function t(f: () => any): void {
  try { const r = f(); console.log("OK", String(r)); } catch (e: any) { console.log("ERR", e.constructor.name); }
}
t(() => (0, eval)("1+1"));
t(() => (0, eval)("nopeVar"));
"#,
    );
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "OK 2\nERR ReferenceError\n");
}

/// A `new Promise` executor that cannot resolve (no parameters, or `resolve` unused) is `Promise<void>`.
#[test]
fn conformance_new_promise_without_a_resolver_builds() {
    let (stdout, stderr, code) = run_conformance_program(
        "fc-promise-no-resolver",
        r#"const never = new Promise(() => {});
const rejOnly = new Promise((_, r) => setTimeout(() => r(new Error("slow")), 5));
never.then(() => console.log("never"));
rejOnly.catch((e: any) => console.log("rejected", e.message));
"#,
    );
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "rejected slow\n");
}

/// `http.createServer()` without a listener, and `res.writeHead(n).end()` without a chunk.
#[test]
fn conformance_http_createserver_without_listener_and_chained_end() {
    let (stdout, stderr, code) = run_conformance_program(
        "fc-http-optional",
        r#"import * as http from 'node:http';
const bare = http.createServer();
bare.listen(0, () => { bare.close(); });
const srv = http.createServer((req: any, res: any) => { res.writeHead(405).end(); });
srv.listen(0, '127.0.0.1', () => {
  http.get({ port: (srv.address() as any).port, host: '127.0.0.1' }, (r: any) => {
    r.on('data', () => {});
    r.on('end', () => { console.log(r.statusCode); srv.close(); });
  });
});
"#,
    );
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert_eq!(stdout, "405\n");
}

const STATIC_SERVER_SOURCE: &str = r#"import {createServer} from 'node:http';
import {readFile, stat} from 'node:fs/promises';
import {resolve, sep} from 'node:path';
import {fileURLToPath} from 'node:url';
const root = resolve(fileURLToPath(new URL('dist/', import.meta.url)));
const types = {'.html':'text/html; charset=utf-8', '.js':'text/javascript; charset=utf-8', '.css':'text/css; charset=utf-8', '.svg':'image/svg+xml', '.txt':'text/plain; charset=utf-8'};
export function createStaticServer() {
  return createServer(async (req, res) => {
    if (!['GET', 'HEAD'].includes(req.method)) {res.writeHead(405).end(); return;}
    try {
      const pathname = decodeURIComponent(new URL(req.url, 'http://localhost').pathname);
      if (pathname.includes('\0') || pathname.split('/').some(p => p.startsWith('.'))) {res.writeHead(404).end(); return;}
      const path = resolve(root, '.' + (pathname === '/' ? '/index.html' : pathname));
      if (!path.startsWith(root + sep) || !(await stat(path)).isFile()) {res.writeHead(404).end(); return;}
      const extension = path.slice(path.lastIndexOf('.'));
      res.writeHead(200, {'Content-Type': types[extension] || 'application/octet-stream', 'Cache-Control':'no-store', 'X-Content-Type-Options':'nosniff'});
      res.end(req.method === 'HEAD' ? undefined : await readFile(path));
    } catch {res.writeHead(404).end('Not found');}
  });
}
if (process.argv[1] === fileURLToPath(import.meta.url)) {
  const port = Number(process.env.PORT || 3173);
  const server = createStaticServer();
  server.on('error', error => {console.error(error.message); process.exitCode = 1;});
  server.listen(port, '0.0.0.0', () => console.log(`Listening: http://0.0.0.0:${server.address().port} (local: http://localhost:${server.address().port})`));
}
"#;

/// Builds `STATIC_SERVER_SOURCE` next to a `dist/` directory and returns `(directory, binary)`.
fn build_static_server(name: &str) -> (std::path::PathBuf, std::path::PathBuf) {
    let dir = std::env::temp_dir().join(format!("thaw-cli-{name}-{}", std::process::id()));
    std::fs::create_dir_all(dir.join("dist/src")).unwrap();
    std::fs::write(dir.join("dist/index.html"), "<html>ok</html>\n").unwrap();
    std::fs::write(dir.join("dist/src/a.js"), "console.log(1)\n").unwrap();
    let entry = dir.join("server.mjs");
    std::fs::write(&entry, STATIC_SERVER_SOURCE).unwrap();
    let output = dir.join("server");
    build(&entry, &output, &[], &[], &[], &dir.join("registry"), &[]).unwrap();
    (dir, output)
}

/// One raw HTTP/1.1 exchange; returns `status line | sorted headers (minus framing) || decoded body`.
fn static_server_exchange(port: u16, method: &str, path: &str) -> String {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
    stream.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
    write!(stream, "{method} {path} HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n").unwrap();
    let mut raw = Vec::new();
    stream.read_to_end(&mut raw).unwrap();
    let split = raw.windows(4).position(|window| window == b"\r\n\r\n").unwrap();
    let head = String::from_utf8_lossy(&raw[..split]).into_owned();
    let mut body = &raw[split + 4..];
    let chunked = head.to_ascii_lowercase().contains("transfer-encoding: chunked");
    let mut decoded = Vec::new();
    if chunked {
        loop {
            let end = body.windows(2).position(|window| window == b"\r\n").unwrap();
            let size = usize::from_str_radix(std::str::from_utf8(&body[..end]).unwrap(), 16).unwrap();
            if size == 0 {
                break;
            }
            decoded.extend_from_slice(&body[end + 2..end + 2 + size]);
            body = &body[end + 4 + size..];
        }
    } else {
        decoded.extend_from_slice(body);
    }
    let mut lines: Vec<&str> = head
        .split("\r\n")
        .filter(|line| {
            let lower = line.to_ascii_lowercase();
            !["date:", "connection:", "keep-alive:", "transfer-encoding:", "content-length:"].iter().any(|name| lower.starts_with(name))
        })
        .collect();
    let status = lines.remove(0);
    lines.sort();
    let mut text = vec![status];
    text.extend(lines);
    format!("{} || {:?}", text.join(" | "), String::from_utf8_lossy(&decoded))
}

/// A static file server (`createServer` + `writeHead(n).end()` + `res.end(cond ? undefined : await readFile())`
/// + the `process.argv[1] === fileURLToPath(import.meta.url)` entry guard) answers like Node 22.
#[test]
fn conformance_static_file_server_matches_node() {
    let (dir, binary) = build_static_server("fc-static-server");
    let mut child = Command::new(&binary)
        .env("PORT", "0")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdout = std::io::BufReader::new(child.stdout.take().unwrap());
    let mut banner = String::new();
    std::io::BufRead::read_line(&mut stdout, &mut banner).unwrap();
    let port: u16 = banner
        .strip_prefix("Listening: http://0.0.0.0:")
        .and_then(|rest| rest.split(' ').next())
        .and_then(|port| port.parse().ok())
        .unwrap_or_else(|| panic!("unexpected banner {banner:?}"));
    assert_eq!(banner.trim_end(), format!("Listening: http://0.0.0.0:{port} (local: http://localhost:{port})"));
    let mut transcript = String::new();
    for (method, path) in [
        ("GET", "/"),
        ("GET", "/src/a.js"),
        ("HEAD", "/"),
        ("GET", "/missing"),
        ("GET", "/../server.mjs"),
        ("GET", "/%2e%2e/server.mjs"),
        ("GET", "/.hidden"),
        ("GET", "/src/.x"),
        ("POST", "/"),
        ("GET", "/%00"),
        ("GET", "/%zz"),
        ("GET", "/src"),
    ] {
        transcript.push_str(&format!("{method} {path} => {}\n", static_server_exchange(port, method, path)));
    }
    let _ = child.kill();
    let _ = child.wait();
    let _ = std::fs::remove_dir_all(dir);
    assert_eq!(transcript, "GET / => HTTP/1.1 200 OK | Cache-Control: no-store | Content-Type: text/html; charset=utf-8 | X-Content-Type-Options: nosniff || \"<html>ok</html>\\n\"
GET /src/a.js => HTTP/1.1 200 OK | Cache-Control: no-store | Content-Type: text/javascript; charset=utf-8 | X-Content-Type-Options: nosniff || \"console.log(1)\\n\"
HEAD / => HTTP/1.1 200 OK | Cache-Control: no-store | Content-Type: text/html; charset=utf-8 | X-Content-Type-Options: nosniff || \"\"
GET /missing => HTTP/1.1 404 Not Found || \"Not found\"
GET /../server.mjs => HTTP/1.1 404 Not Found || \"Not found\"
GET /%2e%2e/server.mjs => HTTP/1.1 404 Not Found || \"Not found\"
GET /.hidden => HTTP/1.1 404 Not Found || \"\"
GET /src/.x => HTTP/1.1 404 Not Found || \"\"
POST / => HTTP/1.1 405 Method Not Allowed || \"\"
GET /%00 => HTTP/1.1 404 Not Found || \"\"
GET /%zz => HTTP/1.1 404 Not Found || \"Not found\"
GET /src => HTTP/1.1 404 Not Found || \"\"\n");
}

/// The `server.on('error')` handler reports `EADDRINUSE` and sets `process.exitCode = 1`, like Node 22.
#[test]
fn conformance_static_file_server_listen_error_exits_with_one() {
    let (dir, binary) = build_static_server("fc-static-server-inuse");
    let occupied = TcpListener::bind("0.0.0.0:0").unwrap();
    let port = occupied.local_addr().unwrap().port();
    let result = Command::new(&binary).env("PORT", port.to_string()).output().unwrap();
    let _ = std::fs::remove_dir_all(dir);
    assert_eq!(String::from_utf8_lossy(&result.stderr), format!("listen EADDRINUSE: address already in use 0.0.0.0:{port}\n"));
    assert_eq!(result.status.code(), Some(1));
}
