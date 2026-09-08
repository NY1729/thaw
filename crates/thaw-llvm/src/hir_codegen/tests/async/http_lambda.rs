#[test]
fn frame_split_await_fetch_uses_nonblocking_http_promise() {
    let source = r#"
        async function main(): Promise<void> {
            const body = await fetch("http://127.0.0.1:8080/data");
            console.log(body);
        }
    "#;
    let module = thaw_parser::parse_typescript(source).unwrap();
    let program = thaw_hir::lower_module(&module).unwrap();
    let context = Context::create();
    let mut compiler = HirCompiler::new(&context, "async_fetch");
    compiler.compile_program(&program).unwrap();
    let ir = compiler.print_to_string();
    assert!(ir.contains("call ptr @thaw_http_get_async"));
    assert!(ir.contains("define internal void @thaw_user_main.resume"));
    assert!(!ir.contains("call ptr @thaw_fetch_get"));
}

/// Full pipeline: `fetch` a JSON body from a real (mock) HTTP server,
/// `JSON.parse` it, read fields (both `.field` and `[i]`), and convert
/// them to concrete types with `Number`/`String`/`Boolean`.
#[test]
fn compiles_fetch_and_json_parsing() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();

    let server = std::thread::spawn(move || {
        let (mut conn, _) = listener.accept().unwrap();
        let mut buf = [0u8; 4096];
        let _ = conn.read(&mut buf).unwrap();
        let body = r#"{"name": "thaw", "active": true, "tags": ["fast", "native"]}"#;
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        );
        conn.write_all(response.as_bytes()).unwrap();
    });

    let source = format!(
        r#"
        async function main(): Promise<void> {{
            const text: string = await fetch("http://{addr}/");
            const data = JSON.parse(text);
            console.log(String(data.name));
            console.log(Boolean(data.active));
            console.log(Number(data.tags.length));
            console.log(String(data.tags[0]));
            console.log(JSON.stringify(data));
        }}
    "#
    );

    let output = compile_and_run(&source, "fetch_json");
    server.join().unwrap();

    let mut lines = output.lines();
    assert_eq!(lines.next(), Some("thaw"));
    assert_eq!(lines.next(), Some("true"));
    assert_eq!(lines.next(), Some("2"));
    assert_eq!(lines.next(), Some("fast"));
    let reparsed: serde_json::Value = serde_json::from_str(lines.next().unwrap()).unwrap();
    assert_eq!(
        reparsed,
        serde_json::json!({"name": "thaw", "active": true, "tags": ["fast", "native"]})
    );
}

/// Full pipeline test for the Lambda entry point: a `handler(event:
/// Json): Json` program is compiled, linked against both
/// thaw-arena and thaw-runtime, run as a real subprocess against a
/// mock Lambda Runtime API server (the same protocol thaw-runtime
/// itself is tested against), and its actual HTTP interaction is
/// verified end to end.
#[test]
fn compiles_and_runs_a_json_lambda_handler_against_a_mock_runtime_api() {
    let source = r#"
        function handler(event: Json): Json {
            console.log(String(event.message));
            return event;
        }
    "#;
    let (post_request, stdout) = compile_and_invoke_lambda(
        source,
        "json_lambda_handler",
        "{\"message\":\"ping\",\"ok\":true}",
    );

    assert!(post_request.starts_with("POST /2018-06-01/runtime/invocation/test-req-1/response"));
    assert!(post_request.ends_with("{\"message\":\"ping\",\"ok\":true}"));
    // Also confirms the fflush-after-console.log fix: stdout is a pipe
    // here (fully buffered by default in libc), and the process is
    // killed rather than exited normally, so without an explicit flush
    // this assertion would flake/fail.
    assert!(stdout.contains("ping"));
}

#[test]
fn awaits_an_async_json_lambda_handler() {
    let source = r#"
        async function handler(event: Json): Promise<Json> {
            await sleep(1);
            return event;
        }
    "#;
    let (post_request, _) = compile_and_invoke_lambda(
        source,
        "async_json_lambda_handler",
        "{\"message\":\"after await\"}",
    );

    assert!(post_request.starts_with("POST /2018-06-01/runtime/invocation/test-req-1/response"));
    assert!(post_request.ends_with("{\"message\":\"after await\"}"));
}

#[test]
fn posts_json_lambda_handler_failures_to_the_error_endpoint() {
    let source = r#"
        function handler(event: Json): Json {
            throw "json handler failed";
        }
    "#;
    let (post_request, _) =
        compile_and_invoke_lambda(source, "failing_json_lambda_handler", "{\"ok\":false}");

    assert!(post_request.starts_with("POST /2018-06-01/runtime/invocation/test-req-1/error"));
    assert!(post_request.contains("json handler failed"));
}

#[test]
fn posts_async_json_lambda_rejections_to_the_error_endpoint() {
    let source = r#"
        async function handler(event: Json): Promise<Json> {
            await sleep(1);
            throw "async json handler failed";
        }
    "#;
    let (post_request, _) = compile_and_invoke_lambda(
        source,
        "rejecting_async_json_lambda_handler",
        "{\"ok\":false}",
    );

    assert!(post_request.starts_with("POST /2018-06-01/runtime/invocation/test-req-1/error"));
    assert!(post_request.contains("async json handler failed"));
}
