// node:worker_threads Worker on thaw-js (W3): which Worker programs route to thaw-js and which stay on QuickJS.

#[test]
fn thaw_js_worker_threads_blocker_routes_plain_worker_programs() {
    let source = "import { Worker } from \"node:worker_threads\";\nconst w = new Worker(`parentPort.postMessage(1)`, { eval: true, workerData: 1, env: {}, argv: [] });\nw.on('exit', () => {}); w.postMessage(2); w.unref(); w.ref(); w.terminate();";
    assert!(thaw_js_worker_threads_blocker("node:worker_threads", source).is_none());
}

#[test]
fn thaw_js_worker_threads_blocker_keeps_unserved_worker_options_on_quickjs() {
    let import = "import { Worker } from \"node:worker_threads\";\n";
    // a Worker that uses stdin is served on thaw-js (W5)
    for routed in [
        "new Worker(src, { eval: true, stdin: true });",
        "const w = new Worker(src, { eval: true, stdin: true });\nw.stdin!.write('abc');",
        // SharedArrayBuffer crosses to a worker as a shared block (W-SAB), so it routes to thaw-js
        "const shared = new SharedArrayBuffer(8);",
        // Atomics are real atomics on a shared block, with wait/notify parking (W-ATOMICS), so they route to thaw-js
        "Atomics.store(view, 0, 42);",
        // resourceLimits.maxOldGenerationSizeMb is enforced by heap_limit (B2), so a limited Worker routes to thaw-js
        "new Worker(src, { eval: true, resourceLimits: { maxOldGenerationSizeMb: 16 } });",
        // transferList moves ArrayBuffers and MessagePorts across threads (PR3), so it routes to thaw-js
        "const transferList = [buffer]; w.postMessage(x, transferList);",
        "new Worker(src, { eval: true, workerData: { port }, transferList: [port] });",
    ] {
        assert!(
            thaw_js_worker_threads_blocker("node:worker_threads", &format!("{import}{routed}"))
                .is_none(),
            "{routed} routes to thaw-js"
        );
    }
    let blocked = "const moved = moveMessagePortToContext(port, context);";
    assert!(
        thaw_js_worker_threads_blocker("node:worker_threads", &format!("{import}{blocked}"))
            .is_some(),
        "{blocked} must stay on QuickJS"
    );
}
