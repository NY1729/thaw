#[test]
fn import_meta_resolution_prefers_the_selected_registry_backend() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-import-meta-backend-{}",
        std::process::id()
    ));
    let package = dir.join("pkg");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(package.join("bundle.js"), "module.exports = {};").unwrap();
    std::fs::write(package.join("native.node"), []).unwrap();
    let resolutions =
        registry_import_meta_resolutions(&dir, &["pkg".to_string(), "node:fs".to_string()]);
    assert_eq!(
        resolutions.get("pkg"),
        Some(&module_graph::file_url(&package.join("native.node")))
    );
    assert_eq!(
        resolutions.get("node:fs").map(String::as_str),
        Some("node:fs")
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn registry_bundle_uses_timers_microtasks_and_text_encoding() {
    let dir =
        std::env::temp_dir().join(format!("thaw-cli-platform-globals-{}", std::process::id()));
    let registry = dir.join("modules");
    let package = registry.join("platform-work");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
            package.join("package.d.ts"),
            "export interface PendingValue { (): void; }\nexport declare function exercise(seed: number): PendingValue;\n",
        )
        .unwrap();
    std::fs::write(
            package.join("bundle.js"),
            r#"module.exports = { exercise: function() {
                return new Promise(resolve => {
                    const events = [];
                    const cancelled = setTimeout(() => events.push('cancelled'), 0);
                    clearTimeout(cancelled);
                    const cancelledImmediate = setImmediate(() => events.push('cancelled-immediate'));
                    clearImmediate(cancelledImmediate);
                    process.nextTick(value => events.push(value), 'nextTick');
                    queueMicrotask(() => events.push('microtask'));
                    setImmediate(value => events.push(value), 'immediate');
                    let ticks = 0;
                    const interval = setInterval(() => {
                        ticks++;
                        if (ticks === 2) {
                            clearInterval(interval);
                            const bytes = new TextEncoder().encode('雪');
                            const text = new TextDecoder().decode(bytes);
                            const original = { nested: { value: 1 } };
                            const copied = structuredClone(original);
                            copied.nested.value = 2;
                            const controller = new AbortController();
                            controller.abort('stopped');
                            const combined = AbortSignal.any([controller.signal]);
                            setTimeout(() => resolve(events.join(',') + ':' + ticks + ':' + text + ':' + btoa('hi') + ':' + atob('aGk=') + ':' + (performance.now() >= 0) + ':' + original.nested.value + ':' + copied.nested.value + ':' + combined.aborted + ':' + combined.reason), 0);
                        }
                    }, 1);
                });
            } };"#,
        )
        .unwrap();
    let source = dir.join("main.ts");
    std::fs::write(
        &source,
        r#"import { exercise } from "platform-work";
                function main(): void {
                    const pending: JsValue = exercise(0);
                    console.log(String(readDynamicValue(pending)));
                    releaseDynamicValue(pending);
                }"#,
    )
    .unwrap();
    let output = dir.join("app");
    build(&source, &output, &[], &[], &[], &registry, &[]).unwrap();
    let result = Command::new(output).output().unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&result.stdout),
        "nextTick,microtask,immediate:2:雪:aGk=:hi:true:1:2:true:stopped\n"
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn imports_supported_node_builtin_modules() {
    let dir = std::env::temp_dir().join(format!("thaw-cli-node-imports-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
            &entry,
            r#"
                import * as path from "node:path";
                import { inspect, format } from "node:util";
                import { cwd } from "node:process";
                import { byteLength } from "node:buffer";
                import * as os from "node:os";
                import * as querystring from "node:querystring";
                import { EventEmitter } from "node:events";
                import { strictEqual } from "node:assert/strict";
                import { StringDecoder } from "node:string_decoder";
                import { isatty } from "node:tty";
                import { pathToFileURL, fileURLToPath, urlToHttpOptions } from "node:url";
                function main(): void {
                    console.log(path.join("a", "b"));
                    console.log(path.extname("archive.tar.gz"));
                    console.log(path.relative("/a/b", "/a/c/d"));
                    console.log(String(inspect(42)));
                    console.log(String(format("%s:%d", "value", 4)));
                    console.log(cwd().length > 0);
                    console.log(Number(byteLength("thaw")));
                    console.log(os.arch() + ":" + os.platform() + ":" + os.type() + ":" + os.tmpdir());
                    console.log(Boolean(isatty(1)));
                    console.log(String(querystring.stringify({ a: [1, 2], space: "two words" })));
                    console.log(String(querystring.parse("a=1&a=2&space=two+words")));
                    const emitter = new EventEmitter();
                    let emitted: number = 0;
                    emitter.once("value", (value): void => { emitted = Number(value); });
                    emitter.emit("value", 42);
                    emitter.emit("value", 7);
                    console.log(emitted);
                    console.log(String(readDynamicValue(pathToFileURL("/tmp/a b"))));
                    console.log(fileURLToPath(pathToFileURL("/tmp/a b")));
                    console.log(String(urlToHttpOptions(pathToFileURL("/tmp/a b"))));
                }
            "#,
        )
        .unwrap();
    let output = dir.join("app");
    build(&entry, &output, &[], &[], &[], &dir.join("registry"), &[]).unwrap();
    let result = Command::new(output).output().unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let expected_tmpdir = std::env::var("TMPDIR").unwrap_or_else(|_| "/tmp".to_string());
    assert_eq!(
            String::from_utf8_lossy(&result.stdout),
            format!("a/b\n.gz\n../c/d\n42\nvalue:4\ntrue\n4\nx64:linux:Linux:{expected_tmpdir}\nfalse\na=1&a=2&space=two%20words\n{{\"a\":[\"1\",\"2\"],\"space\":\"two words\"}}\n42\nfile:///tmp/a%20b\n/tmp/a b\n{{\"protocol\":\"file:\",\"hostname\":\"\",\"hash\":\"\",\"search\":\"\",\"pathname\":\"/tmp/a%20b\",\"path\":\"/tmp/a%20b\",\"href\":\"file:///tmp/a%20b\"}}\n")
        );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn node_fs_reads_and_writes_real_files_in_a_static_binary() {
    if ensure_static_system_libraries().is_err() {
        return;
    }
    let dir = std::env::temp_dir().join(format!("thaw-cli-node-fs-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let data_dir = dir.join("data");
    let data_file = data_dir.join("message.txt");
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        format!(
            r#"
                    import {{ existsSync, readFileSync, writeFileSync, mkdirSync }} from "node:fs";
                    function main(): void {{
                        mkdirSync("{}");
                        writeFileSync("{}", "hello from thaw");
                        console.log(existsSync("{}"));
                        console.log(readFileSync("{}", "utf8"));
                    }}
                "#,
            data_dir.display(),
            data_file.display(),
            data_file.display(),
            data_file.display()
        ),
    )
    .unwrap();
    let output = dir.join("app");
    build_with_link_mode(
        &entry,
        &output,
        &[],
        &[],
        &[],
        &dir.join("registry"),
        &[],
        true,
    )
    .unwrap();
    let result = Command::new(&output).output().unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&result.stdout),
        "true\nhello from thaw\n"
    );
    assert_eq!(
        std::fs::read_to_string(data_file).unwrap(),
        "hello from thaw"
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn node_http_serves_a_real_request_from_a_static_binary() {
    if ensure_static_system_libraries().is_err() {
        return;
    }
    let probe = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = probe.local_addr().unwrap().port();
    drop(probe);
    let occupied_listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let occupied_port = occupied_listener.local_addr().unwrap().port();
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-node-http-{}-{}",
        std::process::id(),
        port
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
            &entry,
            format!(
                r#"
                    import {{ createServer }} from "node:http";
                    function main(): void {{
                        const prefix: string = "hello";
                        let requests: number = 0;
                        const server = createServer(
                            (
                                request: {{ method: string; url: string; statusCode: number; body: string }},
                                response: {{
                                    statusCode: number;
                                    setHeader: (name: string, value: string) => boolean;
                                    end: (chunk: string) => boolean;
                                    write: (chunk: string) => boolean;
                                    endEncoded: (content: string, encoding: string) => boolean;
                                }}
                            ): void => {{
                                requests = requests + 1;
                                response.statusCode = 201;
                                response.setHeader("X-Thaw", request.method);
                                response.write(prefix);
                                response.end(request.url);
                            }}
                        );
                        const target: string = server.listenMany({}, 2);
                        console.log(target);
                        console.log(requests);
                        console.log(server.close());
                        console.log(server.close());
                        server.on("listening", (): void => {{
                            console.log("event:listening");
                        }});
                        server.on("close", (): void => {{
                            console.log("event:close");
                        }});
                        server.on("error", (error: {{ message: string; code: string; syscall: string; address: string; port: number }}): void => {{
                            console.log(error.code);
                        }});
                        server.listen({}, (): void => {{
                            console.log("listening");
                        }});
                        console.log(server.close((): void => {{
                            console.log("closed");
                        }}));
                        server.listen(70000);
                        server.listen({});
                    }}
                "#,
                port, port, occupied_port
            ),
        )
        .unwrap();
    let executable = dir.join("app");
    build_with_link_mode(
        &entry,
        &executable,
        &[],
        &[],
        &[],
        &dir.join("registry"),
        &[],
        true,
    )
    .unwrap();
    assert!(!elf_has_program_interpreter(&executable).unwrap());

    let child = Command::new(&executable)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    fn request(port: u16, target: &str) -> String {
        let mut stream = (0..200)
            .find_map(|_| match TcpStream::connect(("127.0.0.1", port)) {
                Ok(stream) => Some(stream),
                Err(_) => {
                    std::thread::sleep(Duration::from_millis(5));
                    None
                }
            })
            .expect("compiled HTTP server did not start listening");
        stream
            .write_all(format!("GET {target} HTTP/1.1\r\nHost: localhost\r\n\r\n").as_bytes())
            .unwrap();
        let mut response = String::new();
        stream.read_to_string(&mut response).unwrap();
        response
    }
    let first_request = std::thread::spawn(move || request(port, "/health"));
    let second_response = request(port, "/ready");
    let response = first_request.join().unwrap();
    let result = child.wait_with_output().unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(response.starts_with("HTTP/1.1 201 Created\r\n"));
    assert!(response.contains("X-Thaw: GET\r\n"));
    assert!(response.ends_with("hello/health"));
    assert!(second_response.ends_with("hello/ready"));
    let stdout = String::from_utf8_lossy(&result.stdout);
    let (last_target, events) = stdout.split_once('\n').unwrap();
    assert!(matches!(last_target, "/health" | "/ready"));
    assert_eq!(
        events,
        "2\ntrue\nfalse\ntrue\nevent:listening\nlistening\nERR_SOCKET_BAD_PORT\nEADDRINUSE\nevent:close\nclosed\n"
    );

    std::fs::write(
            &entry,
            r#"import { createServer } from "node:http";
            function main(): void {
                const server = createServer((
                    request: { method: string; url: string; statusCode: number; body: string },
                    response: { statusCode: number; setHeader: (name: string, value: string) => boolean; end: (chunk: string) => boolean; write: (chunk: string) => boolean; endEncoded: (content: string, encoding: string) => boolean }
                ): boolean => true);
                server.listen(70000);
            }"#,
        )
        .unwrap();
    let unhandled = dir.join("unhandled-error");
    build_with_link_mode(
        &entry,
        &unhandled,
        &[],
        &[],
        &[],
        &dir.join("registry-unhandled"),
        &[],
        true,
    )
    .unwrap();
    let failed = Command::new(&unhandled).output().unwrap();
    assert!(!failed.status.success());
    assert!(String::from_utf8_lossy(&failed.stderr).contains("Unhandled 'error' event"));
    drop(occupied_listener);
    let _ = std::fs::remove_dir_all(dir);
}

/// Reads exactly one HTTP response (status line + headers + a body
/// exactly `Content-Length` bytes long) off `stream`, returning just the
/// body -- correct on a kept-alive connection, unlike reading to EOF.
fn read_one_response_body(stream: &mut TcpStream) -> String {
    let mut buffer = Vec::new();
    let mut chunk = [0_u8; 512];
    let header_end = loop {
        if let Some(index) = buffer.windows(4).position(|bytes| bytes == b"\r\n\r\n") {
            break index + 4;
        }
        let length = stream.read(&mut chunk).unwrap();
        assert!(length > 0, "connection closed before headers completed");
        buffer.extend_from_slice(&chunk[..length]);
    };
    let head = String::from_utf8_lossy(&buffer[..header_end]).into_owned();
    let content_length: usize = head
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.trim()
                .eq_ignore_ascii_case("Content-Length")
                .then(|| value.trim().parse().ok())?
        })
        .expect("response had no Content-Length");
    while buffer.len() < header_end + content_length {
        let length = stream.read(&mut chunk).unwrap();
        assert!(length > 0, "connection closed before body completed");
        buffer.extend_from_slice(&chunk[..length]);
    }
    String::from_utf8_lossy(&buffer[header_end..header_end + content_length]).into_owned()
}

/// Reads exactly one HTTP response off `stream` and returns its decoded
/// body, handling both `Content-Length` and `Transfer-Encoding: chunked`
/// framing (so it works whether the handler buffered its response or
/// streamed it).
fn read_one_response_body_any(stream: &mut TcpStream) -> String {
    let mut buffer = Vec::new();
    let mut chunk = [0_u8; 512];
    let header_end = loop {
        if let Some(index) = buffer.windows(4).position(|bytes| bytes == b"\r\n\r\n") {
            break index + 4;
        }
        let length = stream.read(&mut chunk).unwrap();
        assert!(length > 0, "connection closed before headers completed");
        buffer.extend_from_slice(&chunk[..length]);
    };
    let head = String::from_utf8_lossy(&buffer[..header_end]).into_owned();
    let chunked = head
        .lines()
        .any(|line| {
            let Some((name, value)) = line.split_once(':') else {
                return false;
            };
            name.trim().eq_ignore_ascii_case("Transfer-Encoding")
                && value.to_ascii_lowercase().contains("chunked")
        });
    if !chunked {
        let content_length: usize = head
            .lines()
            .find_map(|line| {
                let (name, value) = line.split_once(':')?;
                name.trim()
                    .eq_ignore_ascii_case("Content-Length")
                    .then(|| value.trim().parse().ok())?
            })
            .expect("response had neither Content-Length nor chunked framing");
        while buffer.len() < header_end + content_length {
            let length = stream.read(&mut chunk).unwrap();
            assert!(length > 0, "connection closed before body completed");
            buffer.extend_from_slice(&chunk[..length]);
        }
        return String::from_utf8_lossy(&buffer[header_end..header_end + content_length])
            .into_owned();
    }
    // Decode chunks: `<hex len>\r\n<bytes>\r\n` ..., ending at a 0-length chunk.
    let mut rest = buffer[header_end..].to_vec();
    let mut body = Vec::new();
    loop {
        let line_end = loop {
            if let Some(index) = rest.windows(2).position(|bytes| bytes == b"\r\n") {
                break index;
            }
            let length = stream.read(&mut chunk).unwrap();
            assert!(length > 0, "connection closed mid-chunk-size");
            rest.extend_from_slice(&chunk[..length]);
        };
        let size = usize::from_str_radix(
            String::from_utf8_lossy(&rest[..line_end]).trim(),
            16,
        )
        .expect("invalid chunk size");
        rest.drain(..line_end + 2);
        while rest.len() < size + 2 {
            let length = stream.read(&mut chunk).unwrap();
            assert!(length > 0, "connection closed mid-chunk-body");
            rest.extend_from_slice(&chunk[..length]);
        }
        body.extend_from_slice(&rest[..size]);
        rest.drain(..size + 2);
        if size == 0 {
            break;
        }
    }
    String::from_utf8_lossy(&body).into_owned()
}

/// The originally-requested comparison: 50 concurrent connections, each
/// making several keep-alive requests on the *same* socket (never
/// reconnecting -- if the server incorrectly closed after one response,
/// the next write/read on that same `TcpStream` would fail or hang),
/// against both a real compiled thaw `createServer` program and a real
/// `node` reference server with equivalent logic, asserting identical
/// behavior. Streaming/delayed-chunk responses aren't implemented yet
/// (a separate, later stage) -- this covers keep-alive/concurrency
/// parity only.
#[test]
fn node_http_matches_real_node_under_fifty_concurrent_keep_alive_connections_when_enabled() {
    if std::env::var("THAW_RUN_NPM_INTEGRATION").as_deref() != Ok("1") {
        return;
    }
    if Command::new("node").arg("--version").output().is_err() {
        return;
    }
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-keepalive-vs-node-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();

    let source = dir.join("main.ts");
    std::fs::write(
        &source,
        r#"import { createServer } from "node:http";
function main(): void {
    const server = createServer((
        request: { method: string; url: string; statusCode: number; body: string },
        response: { statusCode: number; setHeader: (name: string, value: string) => boolean; end: (chunk: string) => boolean; write: (chunk: string) => boolean; endEncoded: (content: string, encoding: string) => boolean }
    ): void => {
        response.setHeader("Content-Type", "text/plain");
        response.end("thaw:" + request.url);
    });
    server.listen(Number(process.env.PORT));
}"#,
    )
    .unwrap();
    let output = dir.join("app");
    build(&source, &output, &[], &[], &[], &dir.join("registry"), &[]).unwrap();

    let reference = dir.join("reference.js");
    std::fs::write(
        &reference,
        r#"const http = require("http");
const server = http.createServer((request, response) => {
    response.setHeader("Content-Type", "text/plain");
    response.end("node:" + request.url);
});
server.listen(Number(process.env.PORT));
"#,
    )
    .unwrap();

    let free_port = || {
        let probe = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = probe.local_addr().unwrap().port();
        drop(probe);
        port
    };
    let thaw_port = free_port();
    let node_port = free_port();

    let mut thaw_child = Command::new(&output)
        .env("PORT", thaw_port.to_string())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut node_child = Command::new("node")
        .arg(&reference)
        .env("PORT", node_port.to_string())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();

    let wait_for_port = |port: u16| {
        (0..500)
            .find_map(|_| match TcpStream::connect(("127.0.0.1", port)) {
                Ok(stream) => Some(stream),
                Err(_) => {
                    std::thread::sleep(Duration::from_millis(10));
                    None
                }
            })
            .unwrap_or_else(|| panic!("server on port {port} did not start"))
    };
    drop(wait_for_port(thaw_port));
    drop(wait_for_port(node_port));

    for (label, port) in [("thaw", thaw_port), ("node", node_port)] {
        std::thread::scope(|scope| {
            for connection_id in 0..50 {
                scope.spawn(move || {
                    let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
                    stream
                        .set_read_timeout(Some(Duration::from_secs(10)))
                        .unwrap();
                    for request_id in 0..3 {
                        let path = format!("/conn{connection_id}/req{request_id}");
                        stream
                            .write_all(
                                format!("GET {path} HTTP/1.1\r\nHost: localhost\r\n\r\n")
                                    .as_bytes(),
                            )
                            .unwrap();
                        let body = read_one_response_body(&mut stream);
                        assert_eq!(body, format!("{label}:{path}"));
                    }
                });
            }
        });
    }

    thaw_child.kill().unwrap();
    thaw_child.wait().unwrap();
    node_child.kill().unwrap();
    node_child.wait().unwrap();
    let _ = std::fs::remove_dir_all(dir);
}

/// A genuinely `async` `createServer` handler -- one that `await`s
/// (suspending the whole handler body, `response.end(...)` included,
/// until a timer fires) before it produces any output -- must still get
/// its response written, and must do so per-connection under concurrent
/// keep-alive load without one request's buffered body leaking into
/// another's. `run_server_callback` returning `Pending` and the
/// event-loop resuming the handler later (its `response.end(...)` then
/// driving the write) is what this exercises; with that path disabled
/// every request comes back with an empty body. Also covers a handler
/// that interleaves `response.write(...)` calls around the `await`: the
/// pieces are buffered and flushed together on `end()` (true incremental
/// streaming is a later stage).
#[test]
fn node_http_runs_a_genuinely_async_handler_under_concurrent_keep_alive_load() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-async-http-handler-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let source = dir.join("main.ts");
    std::fs::write(
        &source,
        r#"import { createServer } from "node:http";
async function delay(ms: number): Promise<void> {
    await sleep(ms);
}
function main(): void {
    const server = createServer(async (
        request: { method: string; url: string; statusCode: number; body: string },
        response: { statusCode: number; setHeader: (name: string, value: string) => boolean; end: (chunk: string) => boolean; write: (chunk: string) => boolean; endEncoded: (content: string, encoding: string) => boolean }
    ): Promise<void> => {
        if (request.url === "/pieces") {
            response.write("a");
            await delay(5);
            response.write("b");
            await delay(5);
            response.end("c");
            return;
        }
        await delay(5);
        response.end("thaw:" + request.url);
    });
    server.listen(Number(process.env.PORT));
}"#,
    )
    .unwrap();
    let output = dir.join("app");
    build(&source, &output, &[], &[], &[], &dir.join("registry"), &[]).unwrap();

    let probe = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = probe.local_addr().unwrap().port();
    drop(probe);

    let mut child = Command::new(&output)
        .env("PORT", port.to_string())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    (0..500)
        .find_map(|_| match TcpStream::connect(("127.0.0.1", port)) {
            Ok(stream) => Some(stream),
            Err(_) => {
                std::thread::sleep(Duration::from_millis(10));
                None
            }
        })
        .expect("server did not start");

    let failures = std::sync::Mutex::new(Vec::<String>::new());
    std::thread::scope(|scope| {
        for connection_id in 0..50 {
            let failures = &failures;
            scope.spawn(move || {
                let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(20)))
                    .unwrap();
                for request_id in 0..4 {
                    let path = format!("/conn{connection_id}/req{request_id}");
                    stream
                        .write_all(
                            format!("GET {path} HTTP/1.1\r\nHost: localhost\r\n\r\n").as_bytes(),
                        )
                        .unwrap();
                    let body = read_one_response_body(&mut stream);
                    if body != format!("thaw:{path}") {
                        failures.lock().unwrap().push(format!("{path}: got [{body}]"));
                    }
                }
                // A handler that writes around its awaits: streamed as
                // chunks, they arrive concatenated and in order.
                stream
                    .write_all(b"GET /pieces HTTP/1.1\r\nHost: localhost\r\n\r\n")
                    .unwrap();
                let body = read_one_response_body_any(&mut stream);
                if body != "abc" {
                    failures
                        .lock()
                        .unwrap()
                        .push(format!("/pieces on conn{connection_id}: got [{body}]"));
                }
            });
        }
    });

    child.kill().unwrap();
    child.wait().unwrap();
    let failures = failures.into_inner().unwrap();
    let _ = std::fs::remove_dir_all(&dir);
    assert!(
        failures.is_empty(),
        "{} failures:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

/// Reads a chunked response's decoded pieces off `stream`, each tagged
/// with how long after the read started it arrived. Stops at the
/// terminating zero-length chunk.
fn read_chunks_timed(stream: &mut TcpStream) -> Vec<(String, Duration)> {
    let started = std::time::Instant::now();
    let mut buffer = Vec::new();
    let mut scratch = [0_u8; 512];
    let header_end = loop {
        if let Some(index) = buffer.windows(4).position(|bytes| bytes == b"\r\n\r\n") {
            break index + 4;
        }
        let length = stream.read(&mut scratch).unwrap();
        assert!(length > 0, "closed before headers");
        buffer.extend_from_slice(&scratch[..length]);
    };
    let head = String::from_utf8_lossy(&buffer[..header_end]).into_owned();
    assert!(
        head.to_ascii_lowercase().contains("transfer-encoding: chunked"),
        "expected a chunked response, got head:\n{head}"
    );
    let mut rest = buffer[header_end..].to_vec();
    let mut pieces = Vec::new();
    loop {
        let line_end = loop {
            if let Some(index) = rest.windows(2).position(|bytes| bytes == b"\r\n") {
                break index;
            }
            let length = stream.read(&mut scratch).unwrap();
            assert!(length > 0, "closed mid-chunk-size");
            rest.extend_from_slice(&scratch[..length]);
        };
        let size =
            usize::from_str_radix(String::from_utf8_lossy(&rest[..line_end]).trim(), 16).unwrap();
        rest.drain(..line_end + 2);
        while rest.len() < size + 2 {
            let length = stream.read(&mut scratch).unwrap();
            assert!(length > 0, "closed mid-chunk-body");
            rest.extend_from_slice(&scratch[..length]);
        }
        if size == 0 {
            break;
        }
        pieces.push((
            String::from_utf8_lossy(&rest[..size]).into_owned(),
            started.elapsed(),
        ));
        rest.drain(..size + 2);
    }
    pieces
}

/// A streamed response must reach the client incrementally: each
/// `response.write(...)` after an `await` arrives on its own, spaced by
/// the handler's real delay -- not all buffered up and flushed together
/// at `end()`. Runs several such streams concurrently to confirm one
/// connection's chunks never bleed into another's.
#[test]
fn node_http_streams_response_chunks_incrementally_as_the_handler_produces_them() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-streaming-http-handler-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let source = dir.join("main.ts");
    std::fs::write(
        &source,
        r#"import { createServer } from "node:http";
async function delay(ms: number): Promise<void> {
    await sleep(ms);
}
function main(): void {
    const server = createServer(async (
        request: { method: string; url: string; statusCode: number; body: string },
        response: { statusCode: number; setHeader: (name: string, value: string) => boolean; end: (chunk: string) => boolean; write: (chunk: string) => boolean; endEncoded: (content: string, encoding: string) => boolean }
    ): Promise<void> => {
        response.write("A" + request.url + ";");
        await delay(250);
        response.write("B;");
        await delay(250);
        response.end("C;");
    });
    server.listen(Number(process.env.PORT));
}"#,
    )
    .unwrap();
    let output = dir.join("app");
    build(&source, &output, &[], &[], &[], &dir.join("registry"), &[]).unwrap();

    let probe = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = probe.local_addr().unwrap().port();
    drop(probe);
    let mut child = Command::new(&output)
        .env("PORT", port.to_string())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    (0..500)
        .find_map(|_| match TcpStream::connect(("127.0.0.1", port)) {
            Ok(stream) => Some(stream),
            Err(_) => {
                std::thread::sleep(Duration::from_millis(10));
                None
            }
        })
        .expect("server did not start");

    let failures = std::sync::Mutex::new(Vec::<String>::new());
    std::thread::scope(|scope| {
        for connection_id in 0..15 {
            let failures = &failures;
            scope.spawn(move || {
                let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(20)))
                    .unwrap();
                let path = format!("/c{connection_id}");
                stream
                    .write_all(
                        format!("GET {path} HTTP/1.1\r\nHost: localhost\r\n\r\n").as_bytes(),
                    )
                    .unwrap();
                let pieces = read_chunks_timed(&mut stream);
                let joined: String = pieces.iter().map(|(text, _)| text.as_str()).collect();
                if joined != format!("A{path};B;C;") {
                    failures
                        .lock()
                        .unwrap()
                        .push(format!("{path}: reassembled [{joined}]"));
                    return;
                }
                // Three separate chunks, each meaningfully later than the
                // last -- not one buffered flush.
                if pieces.len() < 3 {
                    failures.lock().unwrap().push(format!(
                        "{path}: got {} chunk(s), expected the writes to arrive separately",
                        pieces.len()
                    ));
                    return;
                }
                let gap_ab = pieces[1].1.saturating_sub(pieces[0].1);
                let gap_bc = pieces[2].1.saturating_sub(pieces[1].1);
                if gap_ab < Duration::from_millis(150) || gap_bc < Duration::from_millis(150) {
                    failures.lock().unwrap().push(format!(
                        "{path}: chunk gaps {gap_ab:?}/{gap_bc:?} -- arrived bunched, not streamed"
                    ));
                }
            });
        }
    });

    child.kill().unwrap();
    child.wait().unwrap();
    let failures = failures.into_inner().unwrap();
    let _ = std::fs::remove_dir_all(&dir);
    assert!(
        failures.is_empty(),
        "{} failures:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

/// The native server must survive hostile / broken clients: ones that
/// open a connection and never finish the request head, ones that
/// connect and vanish by the hundred, and ones that hang up while an
/// `async` handler is mid-flight. None of that may leak descriptors or
/// take the server down for the well-behaved clients sharing it.
#[test]
fn node_http_survives_disconnects_timeouts_and_aborts_without_leaking() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-http-resilience-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let source = dir.join("main.ts");
    std::fs::write(
        &source,
        r#"import { createServer } from "node:http";
async function delay(ms: number): Promise<void> {
    await sleep(ms);
}
function main(): void {
    const server = createServer(async (
        request: { method: string; url: string; statusCode: number; body: string },
        response: { statusCode: number; setHeader: (name: string, value: string) => boolean; end: (chunk: string) => boolean; write: (chunk: string) => boolean; endEncoded: (content: string, encoding: string) => boolean }
    ): Promise<void> => {
        if (request.url === "/slow") {
            await delay(400);
            response.end("slow-done");
            return;
        }
        if (request.url === "/stream") {
            response.write("chunk1;");
            await delay(200);
            response.write("chunk2;");
            await delay(200);
            response.end("chunk3");
            return;
        }
        response.end("ok:" + request.url);
    });
    server.listen(Number(process.env.PORT));
}"#,
    )
    .unwrap();
    let output = dir.join("app");
    build(&source, &output, &[], &[], &[], &dir.join("registry"), &[]).unwrap();

    let probe = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = probe.local_addr().unwrap().port();
    drop(probe);
    let mut child = Command::new(&output)
        .env("PORT", port.to_string())
        // A short header-read timeout so the stalled-client case is quick.
        .env("THAW_HTTP_HEADER_TIMEOUT_MS", "700")
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let pid = child.id();
    drop(
        (0..500)
            .find_map(|_| match TcpStream::connect(("127.0.0.1", port)) {
                Ok(stream) => Some(stream),
                Err(_) => {
                    std::thread::sleep(Duration::from_millis(10));
                    None
                }
            })
            .expect("server did not start"),
    );

    let fd_count = || {
        std::fs::read_dir(format!("/proc/{pid}/fd"))
            .map(|entries| entries.count())
            .unwrap_or(0)
    };
    let ok_request = |path: &str| {
        let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        stream
            .write_all(format!("GET {path} HTTP/1.1\r\nHost: localhost\r\n\r\n").as_bytes())
            .unwrap();
        read_one_response_body(&mut stream)
    };

    assert_eq!(ok_request("/warmup"), "ok:/warmup");
    let baseline_fds = fd_count();

    // 1. A client that sends a partial head and then stalls forever is
    //    closed by the server's read-side timeout (700ms here), not left
    //    to occupy a descriptor indefinitely.
    let mut stalled = TcpStream::connect(("127.0.0.1", port)).unwrap();
    stalled.write_all(b"GET /never HTTP/1.1\r\n").unwrap();
    let closed_within = {
        let started = std::time::Instant::now();
        stalled
            .set_read_timeout(Some(Duration::from_secs(4)))
            .unwrap();
        let mut sink = [0_u8; 16];
        // Read returns 0 (clean EOF) once the server drops the connection.
        loop {
            match stalled.read(&mut sink) {
                Ok(0) => break started.elapsed(),
                Ok(_) => {}
                Err(error) => panic!("unexpected read error on stalled conn: {error}"),
            }
        }
    };
    assert!(
        closed_within >= Duration::from_millis(500)
            && closed_within < Duration::from_secs(3),
        "stalled connection closed after {closed_within:?}, expected ~700ms"
    );
    assert_eq!(ok_request("/after-timeout"), "ok:/after-timeout");

    // 2. Hundreds of connect-write-a-byte-then-vanish clients must not
    //    accumulate descriptors on the server.
    for index in 0..300 {
        let mut fleeting = TcpStream::connect(("127.0.0.1", port)).unwrap();
        let _ = fleeting.write_all(b"G");
        drop(fleeting);
        if index % 50 == 0 {
            // Let the server process the churn.
            std::thread::sleep(Duration::from_millis(20));
        }
    }
    // Give the server a beat to reap them (its own timeout plus slack).
    std::thread::sleep(Duration::from_millis(1200));
    assert_eq!(ok_request("/after-churn"), "ok:/after-churn");
    let after_churn_fds = fd_count();
    assert!(
        after_churn_fds <= baseline_fds + 8,
        "fd count grew from {baseline_fds} to {after_churn_fds} after 300 disconnects"
    );

    // 3. A client that hangs up while a genuinely `async` handler is
    //    still awaiting must not crash the server or wedge the event
    //    loop -- and its descriptor is released *right away*, not only
    //    once the (still-running) handler finishes 400ms later.
    for _ in 0..20 {
        let mut aborter = TcpStream::connect(("127.0.0.1", port)).unwrap();
        aborter
            .write_all(b"GET /slow HTTP/1.1\r\nHost: localhost\r\n\r\n")
            .unwrap();
        // Vanish well before the handler's 400ms delay elapses.
        std::thread::sleep(Duration::from_millis(5));
        drop(aborter);
    }
    // Still ~300ms before any of those handlers can call response.end():
    // the server has already noticed the hangups and dropped the sockets.
    std::thread::sleep(Duration::from_millis(120));
    assert_eq!(ok_request("/mid-abort-check"), "ok:/mid-abort-check");
    let mid_abort_fds = fd_count();
    assert!(
        mid_abort_fds <= baseline_fds + 8,
        "fd count is {mid_abort_fds} (baseline {baseline_fds}) while 20 aborted \
         handlers are still running -- their sockets weren't released promptly"
    );
    // And it stays flat after the handlers actually complete.
    std::thread::sleep(Duration::from_millis(600));
    assert_eq!(ok_request("/after-aborts"), "ok:/after-aborts");
    let after_aborts_fds = fd_count();
    assert!(
        after_aborts_fds <= baseline_fds + 8,
        "fd count grew from {baseline_fds} to {after_aborts_fds} after 20 mid-handler aborts"
    );

    // 4. A client that hangs up after the first chunk of a *streaming*
    //    response: the handler keeps producing chunks for a while, but
    //    the server must notice the hangup (not spin), stay responsive
    //    to other clients throughout, and recover the descriptor.
    for _ in 0..15 {
        let mut aborter = TcpStream::connect(("127.0.0.1", port)).unwrap();
        aborter
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        aborter
            .write_all(b"GET /stream HTTP/1.1\r\nHost: localhost\r\n\r\n")
            .unwrap();
        let mut sink = [0_u8; 64];
        let _ = aborter.read(&mut sink); // first chunk (or headers)
        drop(aborter);
        // The server stays responsive while the abandoned handler is
        // still notionally running its delays.
        assert_eq!(ok_request("/mid-stream-check"), "ok:/mid-stream-check");
    }
    std::thread::sleep(Duration::from_millis(700));
    assert_eq!(ok_request("/after-stream-aborts"), "ok:/after-stream-aborts");
    let after_stream_fds = fd_count();
    assert!(
        after_stream_fds <= baseline_fds + 8,
        "fd count grew from {baseline_fds} to {after_stream_fds} after 15 mid-stream aborts"
    );

    child.kill().unwrap();
    child.wait().unwrap();
    let _ = std::fs::remove_dir_all(&dir);
}

/// `request.on("data", ...)` / `request.on("end", ...)` -- the streaming
/// body-reading shape most Node handlers actually use -- replays the
/// already-buffered body: `data` fires once with the whole body (never
/// for an empty one), `end` once after, regardless of the order the two
/// listeners were registered in. `request.method`/`.url` still resolve
/// correctly with the `on` member present.
#[test]
fn node_http_replays_the_request_body_through_on_data_and_on_end() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-http-req-on-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let source = dir.join("main.ts");
    std::fs::write(
        &source,
        r#"import { createServer } from "node:http";
function main(): void {
    const server = createServer((request, response): void => {
        let chunks = "";
        let dataCalls = 0;
        // `end` registered before `data` on purpose: delivery must still
        // fire `data` first, so `chunks` is populated when `end` runs.
        request.on("end", (): void => {
            response.setHeader("Content-Type", "text/plain");
            response.end(
                request.method + " " + request.url
                    + " data=" + String(dataCalls)
                    + " [" + chunks + "]"
            );
        });
        request.on("data", (chunk: string): void => {
            dataCalls = dataCalls + 1;
            chunks = chunks + chunk;
        });
    });
    server.listen(Number(process.env.PORT));
}"#,
    )
    .unwrap();
    let output = dir.join("app");
    build(&source, &output, &[], &[], &[], &dir.join("registry"), &[]).unwrap();

    let probe = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = probe.local_addr().unwrap().port();
    drop(probe);
    let mut child = Command::new(&output)
        .env("PORT", port.to_string())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    drop(
        (0..500)
            .find_map(|_| TcpStream::connect(("127.0.0.1", port)).ok().or_else(|| {
                std::thread::sleep(Duration::from_millis(10));
                None
            }))
            .expect("server did not start"),
    );

    let mut keep = TcpStream::connect(("127.0.0.1", port)).unwrap();
    keep.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    let send = |stream: &mut TcpStream, raw: &[u8]| {
        stream.write_all(raw).unwrap();
        read_one_response_body_any(stream)
    };

    // Fixed-length body: one `data` chunk, then `end`.
    assert_eq!(
        send(
            &mut keep,
            b"POST /a HTTP/1.1\r\nHost: x\r\nContent-Length: 5\r\n\r\nalpha"
        ),
        "POST /a data=1 [alpha]"
    );
    // Chunked body: still replayed as a single `data` chunk.
    assert_eq!(
        send(
            &mut keep,
            b"POST /b HTTP/1.1\r\nHost: x\r\nTransfer-Encoding: chunked\r\n\r\n\
              4\r\nbeta\r\n5\r\n-body\r\n0\r\n\r\n"
        ),
        "POST /b data=1 [beta-body]"
    );
    // Empty body: `data` never fires, `end` still does.
    assert_eq!(
        send(&mut keep, b"GET /c HTTP/1.1\r\nHost: x\r\n\r\n"),
        "GET /c data=0 []"
    );

    child.kill().unwrap();
    child.wait().unwrap();
    let _ = std::fs::remove_dir_all(&dir);
}

/// The same `request.on(...)` replay from inside an `async` handler that
/// registers its listeners before its first `await`. This also exercises
/// `hir_type_as_ts_type` over `IncomingMessage`'s function-typed `on`
/// member -- an async handler round-trips the request object's type
/// through it, which previously hit "cannot express inferred generic
/// class type".
#[test]
fn node_http_replays_the_request_body_to_an_async_handler() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-http-req-on-async-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let source = dir.join("main.ts");
    std::fs::write(
        &source,
        r#"import { createServer } from "node:http";
async function main(): Promise<void> {
    const server = createServer(async (request, response): Promise<void> => {
        let body = "";
        request.on("data", (chunk: string): void => { body = body + chunk; });
        request.on("end", (): void => {
            response.end("async " + request.method + " " + request.url + " [" + body + "]");
        });
        await new Promise<void>((resolve): void => { setTimeout(resolve, 5); });
    });
    server.listen(Number(process.env.PORT));
}"#,
    )
    .unwrap();
    let output = dir.join("app");
    build(&source, &output, &[], &[], &[], &dir.join("registry"), &[]).unwrap();

    let probe = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = probe.local_addr().unwrap().port();
    drop(probe);
    let mut child = Command::new(&output)
        .env("PORT", port.to_string())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut keep = (0..500)
        .find_map(|_| TcpStream::connect(("127.0.0.1", port)).ok().or_else(|| {
            std::thread::sleep(Duration::from_millis(10));
            None
        }))
        .expect("server did not start");
    keep.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    keep.write_all(b"POST /async HTTP/1.1\r\nHost: x\r\nContent-Length: 7\r\n\r\npayload")
        .unwrap();
    assert_eq!(read_one_response_body_any(&mut keep), "async POST /async [payload]");

    child.kill().unwrap();
    child.wait().unwrap();
    let _ = std::fs::remove_dir_all(&dir);
}

/// `request.bodyHex()` returns the request body's raw bytes as a hex
/// string -- the escape hatch for a body that `request.body` /
/// `on("data")` would mangle, since a thaw string is NUL-terminated. A
/// four-byte body `00 ff 41 80` (NUL, non-UTF-8) round-trips exactly.
#[test]
fn node_http_exposes_the_raw_request_body_as_hex() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-http-body-hex-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let source = dir.join("main.ts");
    std::fs::write(
        &source,
        r#"import { createServer } from "node:http";
function main(): void {
    const server = createServer((request, response): void => {
        const hex: string = request.bodyHex();
        response.setHeader("Content-Type", "text/plain");
        response.end(
            request.method + " " + request.url
                + " hex=" + hex
                + " body=[" + request.body + "]"
        );
    });
    server.listen(Number(process.env.PORT));
}"#,
    )
    .unwrap();
    let output = dir.join("app");
    build(&source, &output, &[], &[], &[], &dir.join("registry"), &[]).unwrap();

    let probe = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = probe.local_addr().unwrap().port();
    drop(probe);
    let mut child = Command::new(&output)
        .env("PORT", port.to_string())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut keep = (0..500)
        .find_map(|_| TcpStream::connect(("127.0.0.1", port)).ok().or_else(|| {
            std::thread::sleep(Duration::from_millis(10));
            None
        }))
        .expect("server did not start");
    keep.set_read_timeout(Some(Duration::from_secs(5))).unwrap();

    // 00 ff 41 80: NUL, a non-UTF-8 lead byte, 'A', a bare continuation
    // byte. `request.body` sees a truncated / U+FFFD-mangled version;
    // `bodyHex()` sees all four bytes.
    keep.write_all(b"POST /raw HTTP/1.1\r\nHost: x\r\nContent-Length: 4\r\n\r\n\x00\xff\x41\x80")
        .unwrap();
    // `body`: NUL and each invalid byte become U+FFFD.
    assert_eq!(
        read_one_response_body_any(&mut keep),
        "POST /raw hex=00ff4180 body=[\u{fffd}\u{fffd}A\u{fffd}]"
    );
    // A bodyless request: empty hex, empty body.
    assert_eq!(
        {
            keep.write_all(b"GET /none HTTP/1.1\r\nHost: x\r\n\r\n").unwrap();
            read_one_response_body_any(&mut keep)
        },
        "GET /none hex= body=[]"
    );

    child.kill().unwrap();
    child.wait().unwrap();
    let _ = std::fs::remove_dir_all(&dir);
}

/// The native server reads a request body (`Content-Length` or
/// `Transfer-Encoding: chunked`) before invoking the handler and exposes
/// it as `request.body`. Consuming it also means a body-carrying request
/// no longer forces the connection closed -- several with distinct
/// bodies ride one keep-alive connection without desyncing. Oversized
/// and stalled bodies are refused / timed out rather than buffered
/// forever.
#[test]
fn node_http_reads_request_bodies_and_keeps_the_connection_in_sync() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-http-request-body-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let source = dir.join("main.ts");
    std::fs::write(
        &source,
        r#"import { createServer } from "node:http";
function main(): void {
    const server = createServer((
        request: { method: string; url: string; statusCode: number; body: string },
        response: { statusCode: number; setHeader: (name: string, value: string) => boolean; end: (chunk: string) => boolean; write: (chunk: string) => boolean; endEncoded: (content: string, encoding: string) => boolean }
    ): void => {
        response.end(request.method + " " + request.url + " [" + request.body + "]");
    });
    server.listen(Number(process.env.PORT));
}"#,
    )
    .unwrap();
    let output = dir.join("app");
    build(&source, &output, &[], &[], &[], &dir.join("registry"), &[]).unwrap();

    let probe = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = probe.local_addr().unwrap().port();
    drop(probe);
    let mut child = Command::new(&output)
        .env("PORT", port.to_string())
        .env("THAW_HTTP_HEADER_TIMEOUT_MS", "700")
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    drop(
        (0..500)
            .find_map(|_| match TcpStream::connect(("127.0.0.1", port)) {
                Ok(stream) => Some(stream),
                Err(_) => {
                    std::thread::sleep(Duration::from_millis(10));
                    None
                }
            })
            .expect("server did not start"),
    );

    let send = |stream: &mut TcpStream, raw: &[u8]| {
        stream.write_all(raw).unwrap();
        read_one_response_body_any(stream)
    };

    // Three distinct bodies -- fixed, chunked, and empty -- back to back
    // on one connection: each handler sees exactly its own body and the
    // stream stays framed.
    let mut keep = TcpStream::connect(("127.0.0.1", port)).unwrap();
    keep.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    assert_eq!(
        send(
            &mut keep,
            b"POST /a HTTP/1.1\r\nHost: x\r\nContent-Length: 5\r\n\r\nalpha"
        ),
        "POST /a [alpha]"
    );
    assert_eq!(
        send(
            &mut keep,
            b"POST /b HTTP/1.1\r\nHost: x\r\nTransfer-Encoding: chunked\r\n\r\n\
              4\r\nbeta\r\n5\r\n-body\r\n0\r\n\r\n"
        ),
        "POST /b [beta-body]"
    );
    assert_eq!(
        send(&mut keep, b"GET /c HTTP/1.1\r\nHost: x\r\n\r\n"),
        "GET /c []"
    );
    drop(keep);

    // A ~2 MiB body round-trips intact (well under the 8 MiB cap).
    let big = "x".repeat(2 * 1024 * 1024);
    let mut large = TcpStream::connect(("127.0.0.1", port)).unwrap();
    large.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
    large
        .write_all(
            format!(
                "POST /big HTTP/1.1\r\nHost: x\r\nContent-Length: {}\r\n\r\n{big}",
                big.len()
            )
            .as_bytes(),
        )
        .unwrap();
    assert_eq!(
        read_one_response_body_any(&mut large),
        format!("POST /big [{big}]")
    );
    drop(large);

    // A body that stops arriving mid-stream is closed by the read-side
    // timeout, not buffered indefinitely.
    let mut stalled = TcpStream::connect(("127.0.0.1", port)).unwrap();
    stalled
        .write_all(b"POST /slow HTTP/1.1\r\nHost: x\r\nContent-Length: 100\r\n\r\nonly-a-few-bytes")
        .unwrap();
    stalled
        .set_read_timeout(Some(Duration::from_secs(4)))
        .unwrap();
    let started = std::time::Instant::now();
    let mut sink = [0_u8; 16];
    loop {
        match stalled.read(&mut sink) {
            Ok(0) => break,
            Ok(_) => {}
            Err(error) => panic!("stalled body conn: unexpected {error}"),
        }
    }
    assert!(
        started.elapsed() < Duration::from_secs(3),
        "stalled body connection not closed within the timeout"
    );

    // The server is still healthy for the next well-behaved client.
    let mut after = TcpStream::connect(("127.0.0.1", port)).unwrap();
    after.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    assert_eq!(
        send(&mut after, b"GET /done HTTP/1.1\r\nHost: x\r\n\r\n"),
        "GET /done []"
    );

    child.kill().unwrap();
    child.wait().unwrap();
    let _ = std::fs::remove_dir_all(&dir);
}

/// Reads `count` pipelined HTTP/1.1 responses (each `Content-Length`-
/// framed) off one stream and returns their bodies in order. Unlike
/// `read_one_response_body_any` this keeps a single buffer across
/// responses, so bytes of response N+1 that arrive coalesced with
/// response N aren't dropped.
fn read_pipelined_bodies(stream: &mut TcpStream, count: usize) -> Vec<String> {
    let mut buffer = Vec::new();
    let mut scratch = [0_u8; 1024];
    let mut bodies = Vec::new();
    while bodies.len() < count {
        let header_end = loop {
            if let Some(index) = buffer.windows(4).position(|bytes| bytes == b"\r\n\r\n") {
                break index + 4;
            }
            let length = stream.read(&mut scratch).unwrap();
            assert!(length > 0, "connection closed before response {}", bodies.len() + 1);
            buffer.extend_from_slice(&scratch[..length]);
        };
        let head = String::from_utf8_lossy(&buffer[..header_end]).into_owned();
        let content_length: usize = head
            .lines()
            .find_map(|line| {
                let (name, value) = line.split_once(':')?;
                name.trim()
                    .eq_ignore_ascii_case("Content-Length")
                    .then(|| value.trim().parse().ok())?
            })
            .expect("pipelined response had no Content-Length");
        while buffer.len() < header_end + content_length {
            let length = stream.read(&mut scratch).unwrap();
            assert!(length > 0, "connection closed mid-body of response {}", bodies.len() + 1);
            buffer.extend_from_slice(&scratch[..length]);
        }
        bodies.push(
            String::from_utf8_lossy(&buffer[header_end..header_end + content_length]).into_owned(),
        );
        buffer.drain(..header_end + content_length);
    }
    bodies
}

/// A client that sends several requests back to back without waiting for
/// each response (HTTP/1.1 pipelining) gets every response, in request
/// order -- including when a handler in the middle of the pipeline is
/// `async` and suspends. Nothing sent ahead is dropped.
#[test]
fn node_http_answers_pipelined_requests_in_order() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-http-pipelining-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let source = dir.join("main.ts");
    std::fs::write(
        &source,
        r#"import { createServer } from "node:http";
async function delay(ms: number): Promise<void> {
    await sleep(ms);
}
function main(): void {
    const server = createServer(async (
        request: { method: string; url: string; statusCode: number; body: string },
        response: { statusCode: number; setHeader: (name: string, value: string) => boolean; end: (chunk: string) => boolean; write: (chunk: string) => boolean; endEncoded: (content: string, encoding: string) => boolean }
    ): Promise<void> => {
        // A couple of paths make the handler genuinely suspend, so a
        // pipelined request behind them has to wait its turn.
        if (request.url === "/slow") {
            await delay(120);
            response.end("slow[" + request.body + "]");
            return;
        }
        response.end(request.method + " " + request.url + "[" + request.body + "]");
    });
    server.listen(Number(process.env.PORT));
}"#,
    )
    .unwrap();
    let output = dir.join("app");
    build(&source, &output, &[], &[], &[], &dir.join("registry"), &[]).unwrap();

    let probe = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = probe.local_addr().unwrap().port();
    drop(probe);
    let mut child = Command::new(&output)
        .env("PORT", port.to_string())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    drop(
        (0..500)
            .find_map(|_| match TcpStream::connect(("127.0.0.1", port)) {
                Ok(stream) => Some(stream),
                Err(_) => {
                    std::thread::sleep(Duration::from_millis(10));
                    None
                }
            })
            .expect("server did not start"),
    );

    let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
    stream.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
    // One write, six requests -- fixed body, chunked body, a slow async
    // one with a pipelined fast one right behind it, an empty GET, and a
    // final `Connection: close`.
    stream
        .write_all(
            b"POST /a HTTP/1.1\r\nHost: x\r\nContent-Length: 3\r\n\r\nAAA\
              POST /b HTTP/1.1\r\nHost: x\r\nTransfer-Encoding: chunked\r\n\r\n2\r\nBB\r\n0\r\n\r\n\
              POST /slow HTTP/1.1\r\nHost: x\r\nContent-Length: 4\r\n\r\nSLOW\
              GET /c HTTP/1.1\r\nHost: x\r\n\r\n\
              GET /d HTTP/1.1\r\nHost: x\r\n\r\n\
              GET /e HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n",
        )
        .unwrap();
    let bodies = read_pipelined_bodies(&mut stream, 6);
    assert_eq!(
        bodies,
        vec![
            "POST /a[AAA]".to_string(),
            "POST /b[BB]".to_string(),
            "slow[SLOW]".to_string(),
            "GET /c[]".to_string(),
            "GET /d[]".to_string(),
            "GET /e[]".to_string(),
        ]
    );
    // After the last (`Connection: close`) response the server closes.
    let mut trailing = Vec::new();
    stream.read_to_end(&mut trailing).unwrap();
    assert!(trailing.is_empty());

    child.kill().unwrap();
    child.wait().unwrap();
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn multifile_cli_includes_transitive_node_builtin_dependencies() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-transitive-node-builtins-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let data = dir.join("message.txt");
    std::fs::write(&data, "hello").unwrap();
    let data = serde_json::to_string(&data.to_string_lossy()).unwrap();
    std::fs::write(
        dir.join("files.ts"),
        format!(
            r#"import {{ readFileSync }} from "node:fs";
                import "node:path";
                export function describe(): string {{
                    return "message.txt:" + readFileSync({data}, "utf8");
                }}"#
        ),
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        r#"import { describe } from "./files";
            function main(): void { console.log(describe()); }"#,
    )
    .unwrap();
    let output = dir.join("app");
    build(&entry, &output, &[], &[], &[], &dir.join("modules"), &[]).unwrap();
    let result = Command::new(output).output().unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&result.stdout),
        "message.txt:hello\n"
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn bare_import_resolution_errors_include_source_location() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-missing-bare-import-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        "\n\nimport { missing } from \"not-installed\";\nfunction main(): void {}\n",
    )
    .unwrap();
    let error = build(
        &entry,
        &dir.join("app"),
        &[],
        &[],
        &[],
        &dir.join("registry"),
        &[],
    )
    .unwrap_err();
    assert!(error.contains("main.ts:3:"), "{error}");
    assert!(error.contains("not-installed"), "{error}");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn builds_and_runs_embedded_wasi_preview1_module() {
    let dir = std::env::temp_dir().join(format!("thaw-cli-wasi-preview1-{}", std::process::id()));
    let registry = dir.join("modules");
    let package = registry.join("wasi-fixture");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export declare function run(): number;\n",
    )
    .unwrap();
    std::fs::write(
            package.join("bundle.js"),
            r#"module.exports = { run: function () {
                 var options = { args: ['embedded'], env: { MODE: 'standalone' }, preopens: {}, returnOnExit: true, version: 'preview1' };
                 var imports = { wasi_snapshot_preview1: Object.freeze({ __thawWasiOptions: JSON.stringify(options) }) };
                 var source = new TextEncoder().encode(`(module
                   (import "wasi_snapshot_preview1" "proc_exit" (func $exit (param i32)))
                   (memory (export "memory") 1)
                   (func (export "_start") i32.const 6 call $exit))`);
                 var instance = new WebAssembly.Instance(new WebAssembly.Module(source), imports);
                 try { instance.exports._start(); } catch (error) { if (error && error.__thawWasiExit !== undefined) return Number(error.__thawWasiExit); throw error; }
                 return 0;
               } };
"#,
        )
        .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        "import { run } from \"wasi-fixture\"; function main(): void { console.log(run()); }\n",
    )
    .unwrap();
    let output = dir.join("app");
    build(&entry, &output, &[], &[], &[], &registry, &[]).unwrap();
    std::fs::remove_dir_all(&registry).unwrap();
    let result = Command::new(&output).output().unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&result.stdout), "6\n");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn imports_a_scoped_package_subpath_with_default_and_namespace_forms() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-scoped-subpath-import-{}",
        std::process::id()
    ));
    let registry = dir.join("registry");
    let package = registry.join("@scope/tools/subpaths/feature");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export default function double(value: number): number;\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "module.exports = function(value) { return value * 2; };\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        r#"
                import double from "@scope/tools/feature";
                import * as feature from "@scope/tools/feature";
                function main(): void {
                    console.log(double(20));
                    console.log(feature.double(21));
                }
            "#,
    )
    .unwrap();
    let output = dir.join("app");
    build(&entry, &output, &[], &[], &[], &registry, &[]).unwrap();
    let result = Command::new(output).output().unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&result.stdout), "40\n42\n");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn default_callable_import_exposes_export_assignment_namespace_methods() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-export-assignment-namespace-{}",
        std::process::id()
    ));
    let registry = dir.join("registry");
    let package = registry.join("callable-tools");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "declare function tools(value: number): number;\ndeclare namespace tools {\n    function answer(value: number): number;\n}\nexport = tools;\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "function tools(value) { return value * 2; } tools.answer = value => value; module.exports = tools;\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        "import tools from \"callable-tools\"; function main(): void { console.log(tools(21)); console.log(tools.answer(42)); }\n",
    )
    .unwrap();
    let output = dir.join("app");
    build(&entry, &output, &[], &[], &[], &registry, &[]).unwrap();
    let result = Command::new(output).output().unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&result.stdout), "42\n42\n");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn default_callable_import_exposes_called_typeof_namespace_properties() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-export-assignment-typeof-property-{}",
        std::process::id()
    ));
    let registry = dir.join("registry");
    let package = registry.join("callable-tools");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "declare function tools(): unknown;\ndeclare namespace tools { var json: typeof imported.json; }\nexport = tools;\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "function tools() {} tools.json = (...values) => values.length; module.exports = tools;\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        "import tools from \"callable-tools\"; function main(): void { console.log(tools.json()); console.log(tools.json(1)); }\n",
    )
    .unwrap();
    let output = dir.join("app");
    build(&entry, &output, &[], &[], &[], &registry, &[]).unwrap();
    let result = Command::new(output).output().unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&result.stdout), "0\n1\n");
    let _ = std::fs::remove_dir_all(dir);
}

/// The `export = X;` shape the test above covers, but where `X` is bound
/// to a whole *interface-typed const* with several methods (real
/// example: lodash's `declare const _: LoDashStatic;`, ~300 methods) --
/// unlike `tools` above, `_` is never itself one of its own flattened
/// method names, so `package_exports.get("_")` always misses. `import {
/// chunk } from "..."` (a named import of one flattened method) already
/// worked before this fix; `import _ from "..."` (the default-import
/// form, as common in real lodash code as the named form) failed
/// outright ("no export named `default`") since nothing ever inserted a
/// `"default"` key for this shape.
#[test]
fn default_import_exposes_an_export_assignment_interfaces_methods() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-export-assignment-interface-default-{}",
        std::process::id()
    ));
    let registry = dir.join("registry");
    let package = registry.join("lodash-kit");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export = _;\n\
         export as namespace _;\n\
         declare const _: LoDashStatic;\n\
         interface LoDashStatic {\n\
         \x20\x20\x20\x20chunk(value: number): number;\n\
         \x20\x20\x20\x20capitalize(value: number): number;\n\
         }\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "module.exports = { \
         chunk: function(value) { return value * 2; }, \
         capitalize: function(value) { return value + 1; } \
         };\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        "import _ from \"lodash-kit\"; function main(): void { console.log(_.chunk(20)); console.log(_.capitalize(41)); }\n",
    )
    .unwrap();
    let output = dir.join("app");
    build(&entry, &output, &[], &[], &[], &registry, &[]).unwrap();
    let result = Command::new(output).output().unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&result.stdout), "40\n42\n");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn commonjs_default_import_exposes_named_only_exports() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-commonjs-named-class-default-{}",
        std::process::id()
    ));
    let registry = dir.join("registry");
    let package = registry.join("store-kit");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export declare function answer(): number;\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "module.exports = { answer: function() { return 42; } };\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        "import store from \"store-kit\"; function main(): void { console.log(store.answer()); }\n",
    )
    .unwrap();
    let output = dir.join("app");
    build(&entry, &output, &[], &[], &[], &registry, &[]).unwrap();
    let result = Command::new(&output).output().unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&result.stdout), "42\n");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn dynamic_collection_results_coerce_to_declared_native_types() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-dynamic-collection-results-{}",
        std::process::id()
    ));
    let registry = dir.join("registry");
    let package = registry.join("collection-kit");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export = toolkit;\n\
         declare const toolkit: Toolkit;\n\
         interface Toolkit {\n\
         \x20\x20transform(values: number[], callback: (value: number) => number): number[];\n\
         \x20\x20select(values: string, callback: (value: string) => boolean): string[];\n\
         \x20\x20select(values: number[], callback: (value: number) => boolean): number[];\n\
         \x20\x20fold(values: number[], callback: (total: number, value: number) => number, initial: number): number;\n\
         }\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "module.exports = { \
         transform: function(values, callback) { return values.map(callback); }, \
         select: function(values, callback) { return values.filter(callback); }, \
         fold: function(values, callback, initial) { return values.reduce(callback, initial); } \
         };\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        "import toolkit from \"collection-kit\"; function main(): void { const doubled: number[] = toolkit.transform([1, 2, 3, 4], (value: number): number => value * 2); const selected: number[] = toolkit.select(doubled, (value: number): boolean => value >= 6); const total: number = toolkit.fold(selected, (sum: number, value: number): number => sum + value, 0); console.log(doubled.join(',')); console.log(selected.join(',')); console.log(total); }\n",
    )
    .unwrap();
    let output = dir.join("app");
    build(&entry, &output, &[], &[], &[], &registry, &[]).unwrap();
    let result = Command::new(output).output().unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&result.stdout), "2,4,6,8\n6,8\n14\n");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn registers_builds_and_runs_an_installed_npm_wildcard_subpath() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-installed-wildcard-{}",
        std::process::id()
    ));
    let node_modules = dir.join("node_modules");
    let package = node_modules.join("feature-kit");
    std::fs::create_dir_all(package.join("dist/features")).unwrap();
    std::fs::write(
        package.join("package.json"),
        r#"{
                "name":"feature-kit",
                "version":"1.2.3",
                "types":"./index.d.ts",
                "main":"./index.js",
                "exports":{
                    ".":{"types":"./index.d.ts","require":"./index.js"},
                    "./features/*":{
                        "types":"./dist/features/*.d.ts",
                        "require":"./dist/features/*.js"
                    }
                }
            }"#,
    )
    .unwrap();
    std::fs::write(
        package.join("index.d.ts"),
        "export declare function root(): number;\n",
    )
    .unwrap();
    std::fs::write(
        package.join("index.js"),
        "module.exports = { root: function() { return 1; } };\n",
    )
    .unwrap();
    std::fs::write(
        package.join("dist/features/triple.d.ts"),
        "export default function triple(value: number): number;\n",
    )
    .unwrap();
    std::fs::write(
        package.join("dist/features/triple.js"),
        "module.exports = function(value) { return value * 3; };\n",
    )
    .unwrap();
    let registry = dir.join("registry");

    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        r#"
                import triple from "feature-kit/features/triple";
                function main(): void { console.log(triple(14)); }
            "#,
    )
    .unwrap();
    let output = dir.join("app");
    build(&entry, &output, &[], &[], &[], &registry, &[]).unwrap();
    let result = Command::new(output).output().unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&result.stdout), "42\n");
    assert!(registry.join("feature-kit/package.d.ts").is_file());
    assert!(registry
        .join("feature-kit/subpaths/features/triple/package.d.ts")
        .is_file());
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn missing_package_subpath_reports_the_import_location() {
    let dir = std::env::temp_dir().join(format!("thaw-cli-missing-subpath-{}", std::process::id()));
    let registry = dir.join("registry");
    std::fs::create_dir_all(registry.join("math-kit")).unwrap();
    std::fs::write(
        registry.join("math-kit/package.d.ts"),
        "export declare function add(a: number, b: number): number;\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        "\n\nimport { square } from \"math-kit/missing\";\nfunction main(): void {}\n",
    )
    .unwrap();
    let error = build(&entry, &dir.join("app"), &[], &[], &[], &registry, &[]).unwrap_err();
    assert!(error.contains("main.ts:3:"), "{error}");
    assert!(error.contains("math-kit/missing"), "{error}");
    assert!(error.contains("package.d.ts"), "{error}");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn runs_a_multifile_lambda_with_two_bare_import_packages() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-bare-import-lambda-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    for (package, value) in [("left-mark", 20), ("right-mark", 22)] {
        std::fs::create_dir_all(registry.join(package)).unwrap();
        std::fs::write(
            registry.join(package).join("package.d.ts"),
            "export declare function mark(): number;\n",
        )
        .unwrap();
        std::fs::write(
            registry.join(package).join("bundle.js"),
            format!("module.exports = {{ mark: function() {{ return {value}; }} }};\n"),
        )
        .unwrap();
    }
    std::fs::write(
        dir.join("work.ts"),
        r#"
                import { mark as leftMark } from "left-mark";
                import { mark as rightMark } from "right-mark";
                export async function work(): Promise<void> {
                    await sleep(1);
                    console.log(leftMark() + rightMark());
                }
            "#,
    )
    .unwrap();
    let entry = dir.join("handler.ts");
    std::fs::write(
        &entry,
        r#"
                import { work } from "./work";
                async function handler(event: Json): Promise<Json> {
                    await work();
                    return event;
                }
            "#,
    )
    .unwrap();
    let output = dir.join("bootstrap");
    build(&entry, &output, &[], &[], &[], &registry, &[]).unwrap();

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap().to_string();
    let (tx, rx) = mpsc::channel();
    let server = std::thread::spawn(move || {
        let (mut conn, _) = listener.accept().unwrap();
        let mut request = [0u8; 4096];
        let _ = conn.read(&mut request).unwrap();
        let event = "{\"packages\":2}";
        conn.write_all(
                format!(
                    "HTTP/1.1 200 OK\r\nLambda-Runtime-Aws-Request-Id: packages-request\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    event.len(), event
                )
                .as_bytes(),
            )
            .unwrap();
        drop(conn);
        let (mut conn, _) = listener.accept().unwrap();
        let mut request = Vec::new();
        conn.read_to_end(&mut request).unwrap();
        tx.send(String::from_utf8_lossy(&request).into_owned())
            .unwrap();
        conn.write_all(b"HTTP/1.1 202 Accepted\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
            .unwrap();
    });
    let mut child = Command::new(&output)
        .env("AWS_LAMBDA_RUNTIME_API", addr)
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let request = rx
        .recv_timeout(Duration::from_secs(10))
        .expect("package Lambda handler did not post a response");
    server.join().unwrap();
    let _ = child.kill();
    let result = child.wait_with_output().unwrap();
    assert!(request.starts_with("POST /2018-06-01/runtime/invocation/packages-request/response"));
    assert!(request.ends_with("{\"packages\":2}"));
    assert!(String::from_utf8_lossy(&result.stdout).contains("42"));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn numeric_comparison_edge_cases_use_jit_without_quickjs() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-jit-compare-edge-cases-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("compare-kit");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export declare function compare(a: number, b: number): boolean[];\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "module.exports.compare = (a, b) => [a < b, a <= b, a > b, a >= b, a === b, a !== b];\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        "import { compare } from 'compare-kit';\nfunction main(): void { const nan = 0 / 0; const zero = 0; const negZero = -zero; console.log(compare(nan, 5).join(',')); console.log(compare(5, nan).join(',')); console.log(compare(nan, nan).join(',')); console.log(compare(zero, negZero).join(',')); }\n",
    )
    .unwrap();
    let output = dir.join("app");
    build(&entry, &output, &[], &[], &[], &registry, &[]).unwrap();
    let manifest = artifact_manifest_from_bytes(&std::fs::read(&output).unwrap()).unwrap();
    assert_eq!(manifest["quickjs"], false);
    std::fs::remove_dir_all(&registry).unwrap();
    let result = Command::new(&output).output().unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&result.stdout),
        "false,false,false,false,false,true\nfalse,false,false,false,false,true\nfalse,false,false,false,false,true\nfalse,true,false,true,true,false\n"
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn bitwise_int32_boundary_edge_cases_use_jit_without_quickjs() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-jit-bitwise-edge-cases-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("bits-kit");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export declare function bits(a: number): number[];\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "module.exports.bits = (a) => [a | 0, a >>> 0, ~a, a << 1, a >>> 1];\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        "import { bits } from 'bits-kit';\nfunction main(): void { const nan = 0 / 0; console.log(bits(-1).join(',')); console.log(bits(nan).join(',')); console.log(bits(4294967295).join(',')); console.log(bits(2147483648).join(',')); console.log(bits(5.9).join(',')); console.log(bits(-5.9).join(',')); }\n",
    )
    .unwrap();
    let output = dir.join("app");
    build(&entry, &output, &[], &[], &[], &registry, &[]).unwrap();
    let manifest = artifact_manifest_from_bytes(&std::fs::read(&output).unwrap()).unwrap();
    assert_eq!(manifest["quickjs"], false);
    std::fs::remove_dir_all(&registry).unwrap();
    let result = Command::new(&output).output().unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&result.stdout),
        "-1,4294967295,0,-2,2147483647\n\
         0,0,-1,0,0\n\
         -1,4294967295,0,-2,2147483647\n\
         -2147483648,2147483648,2147483647,0,1073741824\n\
         5,5,-6,10,2\n\
         -5,4294967291,4,-10,2147483645\n"
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn string_surrogate_pair_and_empty_edge_cases_use_jit_without_quickjs() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-jit-surrogate-edge-cases-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("strings-kit");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export declare function codePoints(s: string): number[];\nexport declare function charCodes(s: string): number[];\nexport declare function fromStringInfo(s: string): number[];\nexport declare function emptyProbe(s: string): number;\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "module.exports.codePoints = (s) => [s.codePointAt(0), s.codePointAt(1), s.codePointAt(2)]; module.exports.charCodes = (s) => [s.charCodeAt(0), s.charCodeAt(1), s.charCodeAt(2), s.charCodeAt(99)]; module.exports.fromStringInfo = (s) => [Array.from(s).length, s.length]; module.exports.emptyProbe = (s) => s.charCodeAt(0);\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        "import { codePoints, charCodes, fromStringInfo, emptyProbe } from 'strings-kit';\nfunction main(): void { const emoji = \"\\uD83D\\uDE00x\"; console.log(codePoints(emoji).join(',')); console.log(charCodes(emoji).join(',')); console.log(fromStringInfo(emoji).join(',')); console.log(emptyProbe(\"\")); }\n",
    )
    .unwrap();
    let output = dir.join("app");
    build(&entry, &output, &[], &[], &[], &registry, &[]).unwrap();
    let manifest = artifact_manifest_from_bytes(&std::fs::read(&output).unwrap()).unwrap();
    assert_eq!(manifest["quickjs"], false);
    std::fs::remove_dir_all(&registry).unwrap();
    let result = Command::new(&output).output().unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&result.stdout),
        "128512,56832,120\n55357,56832,120,NaN\n2,3\nNaN\n"
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn recursion_and_dictionary_aliasing_edge_cases_use_jit_without_quickjs() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-jit-recursion-edge-cases-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("rec-kit");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export declare function fib(n: number): number;\nexport declare function isEven(n: number): boolean;\nexport declare function isOdd(n: number): boolean;\nexport declare function dictAlias(seed: Record<string, number>): number;\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "function fib(n) { return n < 2 ? n : fib(n - 1) + fib(n - 2); }\nfunction isEven(n) { return n === 0 ? true : isOdd(n - 1); }\nfunction isOdd(n) { return n === 0 ? false : isEven(n - 1); }\nfunction dictAlias(seed) { const alias = seed; seed[\"a\"] = 1; alias[\"b\"] = 2; seed[\"a\"] = (seed[\"a\"] ?? 0) + 10; return alias[\"a\"] + alias[\"b\"]; }\nmodule.exports = { fib, isEven, isOdd, dictAlias };\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        "import { fib, isEven, isOdd, dictAlias } from 'rec-kit';\nfunction main(): void { console.log(fib(20)); console.log(isEven(11)); console.log(isOdd(11)); console.log(dictAlias({})); }\n",
    )
    .unwrap();
    let output = dir.join("app");
    build(&entry, &output, &[], &[], &[], &registry, &[]).unwrap();
    let manifest = artifact_manifest_from_bytes(&std::fs::read(&output).unwrap()).unwrap();
    assert_eq!(manifest["quickjs"], false);
    std::fs::remove_dir_all(&registry).unwrap();
    let result = Command::new(&output).output().unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&result.stdout),
        "6765\nfalse\ntrue\n13\n"
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn array_methods_chained_directly_on_array_from_use_jit_without_quickjs() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-jit-array-from-chain-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("from-chain-kit");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export declare function codeUnitLengths(s: string): number[];\nexport declare function evens(values: number[]): number[];\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "module.exports.codeUnitLengths = (s) => Array.from(s).map((c) => c.length); module.exports.evens = (values) => Array.from(values).filter((v) => v % 2 === 0);\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        "import { codeUnitLengths, evens } from 'from-chain-kit';\nfunction main(): void { const emoji = \"\\uD83D\\uDE00x\"; console.log(codeUnitLengths(emoji).join(',')); console.log(evens([1, 2, 3, 4, 5, 6]).join(',')); }\n",
    )
    .unwrap();
    let output = dir.join("app");
    build(&entry, &output, &[], &[], &[], &registry, &[]).unwrap();
    let manifest = artifact_manifest_from_bytes(&std::fs::read(&output).unwrap()).unwrap();
    assert_eq!(manifest["quickjs"], false);
    std::fs::remove_dir_all(&registry).unwrap();
    let result = Command::new(&output).output().unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&result.stdout),
        "2,1\n2,4,6\n"
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn array_from_and_of_chains_generalize_across_methods_without_quickjs() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-jit-array-from-of-chains-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("chain-kit");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export declare function reduceFrom(values: number[]): number;\nexport declare function doubleChain(values: number[]): number[];\nexport declare function ofChain(): number[];\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "module.exports.reduceFrom = (values) => Array.from(values).reduce((a, b) => a + b, 0); module.exports.doubleChain = (values) => Array.from(values).map((v) => v * 2).filter((v) => v > 4); module.exports.ofChain = () => Array.of(1, 2, 3).map((v) => v * 10);\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        "import { reduceFrom, doubleChain, ofChain } from 'chain-kit';\nfunction main(): void { console.log(reduceFrom([1, 2, 3, 4])); console.log(doubleChain([1, 2, 3, 4]).join(',')); console.log(ofChain().join(',')); }\n",
    )
    .unwrap();
    let output = dir.join("app");
    build(&entry, &output, &[], &[], &[], &registry, &[]).unwrap();
    let manifest = artifact_manifest_from_bytes(&std::fs::read(&output).unwrap()).unwrap();
    assert_eq!(manifest["quickjs"], false);
    std::fs::remove_dir_all(&registry).unwrap();
    let result = Command::new(&output).output().unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&result.stdout),
        "10\n6,8\n10,20,30\n"
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn array_methods_chained_directly_on_object_keys_values_use_jit_without_quickjs() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-jit-object-keys-chain-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("obj-kit");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export declare function keyLengths(o: Record<string, number>): number[];\nexport declare function doubledValues(o: Record<string, number>): number[];\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "module.exports.keyLengths = (o) => Object.keys(o).map((k) => k.length); module.exports.doubledValues = (o) => Object.values(o).map((v) => v * 2);\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        "import { keyLengths, doubledValues } from 'obj-kit';\nfunction main(): void { console.log(keyLengths({ ab: 1, cde: 2 }).join(',')); console.log(doubledValues({ ab: 1, cde: 2 }).join(',')); }\n",
    )
    .unwrap();
    let output = dir.join("app");
    build(&entry, &output, &[], &[], &[], &registry, &[]).unwrap();
    let manifest = artifact_manifest_from_bytes(&std::fs::read(&output).unwrap()).unwrap();
    assert_eq!(manifest["quickjs"], false);
    std::fs::remove_dir_all(&registry).unwrap();
    let result = Command::new(&output).output().unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&result.stdout), "2,3\n2,4\n");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn string_methods_chained_directly_on_from_char_code_use_jit_without_quickjs() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-jit-fromcharcode-chain-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("str-kit");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export declare function upper(code: number): string;\nexport declare function fromPoint(point: number): string;\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "module.exports.upper = (code) => String.fromCharCode(code).toUpperCase(); module.exports.fromPoint = (point) => String.fromCodePoint(point).trim();\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        "import { upper, fromPoint } from 'str-kit';\nfunction main(): void { console.log(upper(97)); console.log(fromPoint(98)); }\n",
    )
    .unwrap();
    let output = dir.join("app");
    build(&entry, &output, &[], &[], &[], &registry, &[]).unwrap();
    let manifest = artifact_manifest_from_bytes(&std::fs::read(&output).unwrap()).unwrap();
    assert_eq!(manifest["quickjs"], false);
    std::fs::remove_dir_all(&registry).unwrap();
    let result = Command::new(&output).output().unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&result.stdout), "A\nb\n");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn array_methods_chained_on_split_and_spread_literals_use_jit_without_quickjs() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-jit-split-spread-chain-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("mix-kit");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export declare function wordLens(s: string): number[];\nexport declare function spreadDouble(values: number[]): number[];\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "module.exports.wordLens = (s) => s.split(' ').map((w) => w.length); module.exports.spreadDouble = (values) => [...values].map((v) => v * 2);\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        "import { wordLens, spreadDouble } from 'mix-kit';\nfunction main(): void { console.log(wordLens(\"ab cde f\").join(',')); console.log(spreadDouble([1, 2, 3]).join(',')); }\n",
    )
    .unwrap();
    let output = dir.join("app");
    build(&entry, &output, &[], &[], &[], &registry, &[]).unwrap();
    let manifest = artifact_manifest_from_bytes(&std::fs::read(&output).unwrap()).unwrap();
    assert_eq!(manifest["quickjs"], false);
    std::fs::remove_dir_all(&registry).unwrap();
    let result = Command::new(&output).output().unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&result.stdout), "2,3,1\n2,4,6\n");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn array_methods_chained_on_a_parenthesized_ternary_use_jit_without_quickjs() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-jit-ternary-receiver-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("ternary-kit");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export declare function ternaryChain(flag: boolean, values: number[]): number[];\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "module.exports.ternaryChain = (flag, values) => (flag ? Array.from(values) : values).map((v) => v * 2);\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        "import { ternaryChain } from 'ternary-kit';\nfunction main(): void { console.log(ternaryChain(true, [1, 2, 3]).join(',')); console.log(ternaryChain(false, [1, 2, 3]).join(',')); }\n",
    )
    .unwrap();
    let output = dir.join("app");
    build(&entry, &output, &[], &[], &[], &registry, &[]).unwrap();
    let manifest = artifact_manifest_from_bytes(&std::fs::read(&output).unwrap()).unwrap();
    assert_eq!(manifest["quickjs"], false);
    std::fs::remove_dir_all(&registry).unwrap();
    let result = Command::new(&output).output().unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&result.stdout), "2,4,6\n2,4,6\n");
    let _ = std::fs::remove_dir_all(dir);
}

// Object.fromEntries only accepts the output of Object.entries(...) (a round trip),
// not an arbitrary array-of-pairs literal; Object.assign requires every argument to
// already be dictionary-typed (a plain `{}` object literal defaults to a fixed-shape
// object type instead). Both are intentional scope limits, not receiver-gate bugs -
// this locks in the shapes that already work correctly through the JIT today.
#[test]
fn object_from_entries_and_assign_round_trips_use_jit_without_quickjs() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-jit-fromentries-assign-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("obj-roundtrip-kit");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export declare function roundtripKeyCount(o: Record<string, number>): number;\nexport declare function assignedValues(a: Record<string, number>, b: Record<string, number>): number[];\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "module.exports.roundtripKeyCount = (o) => Object.keys(Object.fromEntries(Object.entries(o))).length; module.exports.assignedValues = (a, b) => Object.values(Object.assign(a, b));\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        "import { roundtripKeyCount, assignedValues } from 'obj-roundtrip-kit';\nfunction main(): void { console.log(roundtripKeyCount({ a: 1, b: 2 })); console.log(assignedValues({ x: 1 }, { y: 2 }).join(',')); }\n",
    )
    .unwrap();
    let output = dir.join("app");
    build(&entry, &output, &[], &[], &[], &registry, &[]).unwrap();
    let manifest = artifact_manifest_from_bytes(&std::fs::read(&output).unwrap()).unwrap();
    assert_eq!(manifest["quickjs"], false);
    std::fs::remove_dir_all(&registry).unwrap();
    let result = Command::new(&output).output().unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&result.stdout), "2\n1,2\n");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn array_valued_short_circuit_operators_use_jit_without_quickjs() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-jit-short-circuit-array-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("short-circuit-kit");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export declare function orValue(a: number[], b: number[]): number[];\nexport declare function andValue(a: number[], b: number[]): number[];\nexport declare function nullishValue(a: number[], b: number[]): number[];\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "module.exports.orValue = (a, b) => a || b; module.exports.andValue = (a, b) => a && b; module.exports.nullishValue = (a, b) => a ?? b;\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        "import { orValue, andValue, nullishValue } from 'short-circuit-kit';\nfunction main(): void { console.log(orValue([], [3, 4]).join(',')); console.log(orValue([1, 2], [3, 4]).join(',')); console.log(andValue([1, 2], [3, 4]).join(',')); console.log(nullishValue([1, 2], [3, 4]).join(',')); }\n",
    )
    .unwrap();
    let output = dir.join("app");
    build(&entry, &output, &[], &[], &[], &registry, &[]).unwrap();
    let manifest = artifact_manifest_from_bytes(&std::fs::read(&output).unwrap()).unwrap();
    assert_eq!(manifest["quickjs"], false);
    std::fs::remove_dir_all(&registry).unwrap();
    let result = Command::new(&output).output().unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&result.stdout),
        "\n1,2\n3,4\n1,2\n"
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn array_methods_chained_on_short_circuit_operators_use_jit_without_quickjs() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-jit-short-circuit-chain-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("short-circuit-chain-kit");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export declare function orChain(a: number[], b: number[]): number[];\nexport declare function andChain(a: number[], b: number[]): number[];\nexport declare function nullishChain(a: number[], b: number[]): number[];\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "module.exports.orChain = (a, b) => (a || b).map((v) => v * 2); module.exports.andChain = (a, b) => (a && b).map((v) => v * 2); module.exports.nullishChain = (a, b) => (a ?? b).map((v) => v * 2);\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        "import { orChain, andChain, nullishChain } from 'short-circuit-chain-kit';\nfunction main(): void { console.log(orChain([1, 2], [3, 4]).join(',')); console.log(andChain([1, 2], [3, 4]).join(',')); console.log(nullishChain([1, 2], [3, 4]).join(',')); }\n",
    )
    .unwrap();
    let output = dir.join("app");
    build(&entry, &output, &[], &[], &[], &registry, &[]).unwrap();
    let manifest = artifact_manifest_from_bytes(&std::fs::read(&output).unwrap()).unwrap();
    assert_eq!(manifest["quickjs"], false);
    std::fs::remove_dir_all(&registry).unwrap();
    let result = Command::new(&output).output().unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&result.stdout), "2,4\n6,8\n2,4\n");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn object_assign_with_an_empty_literal_target_uses_jit_without_quickjs() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-jit-assign-empty-literal-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("assign-empty-kit");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export declare function assignChain(a: Record<string, number>, b: Record<string, number>): number[];\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "module.exports.assignChain = (a, b) => Object.values(Object.assign({}, a, b));\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        "import { assignChain } from 'assign-empty-kit';\nfunction main(): void { console.log(assignChain({ x: 1 }, { y: 2 }).join(',')); }\n",
    )
    .unwrap();
    let output = dir.join("app");
    build(&entry, &output, &[], &[], &[], &registry, &[]).unwrap();
    let manifest = artifact_manifest_from_bytes(&std::fs::read(&output).unwrap()).unwrap();
    assert_eq!(manifest["quickjs"], false);
    std::fs::remove_dir_all(&registry).unwrap();
    let result = Command::new(&output).output().unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&result.stdout), "1,2\n");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn object_from_entries_with_a_literal_array_of_pairs_uses_jit_without_quickjs() {
    let dir = std::env::temp_dir().join(format!(
        "thaw-cli-registry-jit-fromentries-literal-{}",
        std::process::id()
    ));
    let registry = dir.join("modules");
    let package = registry.join("fromentries-literal-kit");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("package.d.ts"),
        "export declare function fromLiteralPairs(pairs: number[]): number[];\n",
    )
    .unwrap();
    std::fs::write(
        package.join("bundle.js"),
        "module.exports.fromLiteralPairs = (pairs) => { const built = Object.fromEntries([[\"a\", pairs[0]], [\"b\", pairs[1]]]); return [built.a, built.b, Object.keys(built).length]; };\n",
    )
    .unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        "import { fromLiteralPairs } from 'fromentries-literal-kit';\nfunction main(): void { console.log(fromLiteralPairs([10, 20]).join(',')); }\n",
    )
    .unwrap();
    let output = dir.join("app");
    build(&entry, &output, &[], &[], &[], &registry, &[]).unwrap();
    let manifest = artifact_manifest_from_bytes(&std::fs::read(&output).unwrap()).unwrap();
    assert_eq!(manifest["quickjs"], false);
    std::fs::remove_dir_all(&registry).unwrap();
    let result = Command::new(&output).output().unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&result.stdout), "10,20,2\n");
    let _ = std::fs::remove_dir_all(dir);
}
