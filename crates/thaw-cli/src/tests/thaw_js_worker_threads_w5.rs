// node:worker_threads stdio on thaw-js (W5): Worker programs that use stdin, stdout or stderr route to thaw-js.

#[test]
fn thaw_js_worker_threads_w5_routes_worker_stdio_programs() {
    let import = "import { Worker } from \"node:worker_threads\";\n";
    for routed in [
        "const w = new Worker(src, { eval: true, stdin: true }); w.stdin!.write('abc'); w.stdin!.end('def');",
        "const w = new Worker(src, { eval: true, stdout: true, stderr: true });\nw.stdout.setEncoding('utf8'); w.stderr.on('data', f);",
        "const w = new Worker(`process.stdin.on('data', f);`, { eval: true, stdin: true });",
    ] {
        assert!(
            thaw_js_worker_threads_blocker("node:worker_threads", &format!("{import}{routed}"))
                .is_none(),
            "{routed} routes to thaw-js"
        );
    }
}

#[test]
fn thaw_js_worker_threads_w5_keeps_main_thread_stdin_on_quickjs() {
    let import = "import { Worker } from \"node:worker_threads\";\n";
    assert!(
        thaw_js_worker_threads_blocker(
            "node:worker_threads",
            "process.stdin.setEncoding('utf8'); process.stdin.on('data', f);"
        )
        .is_some()
    );
    assert!(thaw_js_worker_threads_blocker(
        "node:worker_threads",
        &format!("{import}process.stdin.on('data', f);")
    )
    .is_some());
}
