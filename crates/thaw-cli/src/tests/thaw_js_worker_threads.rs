// node:worker_threads on thaw-js: every program that uses it routes to thaw-js (no blocker is left) and the thaw-js form.

/// Programs that used to be refused by the worker_threads blocker (`transferList`, `moveMessagePortToContext`, main-thread
/// `process.stdin`) or by an earlier option list, plus the plain shapes. The blocker and its constant are deleted: each one
/// must now be accepted by the router.
#[test]
fn thaw_js_worker_threads_programs_all_route_to_thaw_js() {
    let import = "import { Worker } from \"node:worker_threads\";\n";
    // The router judges the thaw-js form of the module (the prelude export), not the thaw-js bundle.
    let bundle = thaw_js::scan::script_as_bundle(THAW_JS_WORKER_THREADS_FORM).into_owned();
    for program in [
        "const w = new Worker(`parentPort.postMessage(1)`, { eval: true, workerData: 1, env: {}, argv: [] });",
        // stdin: a Worker with `stdin: true`, and main-thread process.stdin (served by process_stdin.js)
        "new Worker(src, { eval: true, stdin: true });",
        "const w = new Worker(src, { eval: true, stdin: true });\nw.stdin!.write('abc'); w.stdin!.end('def');",
        "const w = new Worker(src, { eval: true, stdout: true, stderr: true });\nw.stdout.setEncoding('utf8'); w.stderr.on('data', f);",
        "const w = new Worker(`process.stdin.on('data', f);`, { eval: true, stdin: true });",
        "process.stdin.setEncoding('utf8'); process.stdin.on('data', f);",
        // SharedArrayBuffer and Atomics are real shared blocks on thaw-js
        "const shared = new SharedArrayBuffer(8);",
        "Atomics.store(view, 0, 42);",
        // resourceLimits is enforced by heap_limit
        "new Worker(src, { eval: true, resourceLimits: { maxOldGenerationSizeMb: 16 } });",
        "new Worker(src, { eval: true, resourceLimits: {} });",
        // transferList moves ArrayBuffers and MessagePorts across threads
        "const transferList = [buffer]; w.postMessage(x, transferList);",
        "new Worker(src, { eval: true, workerData: { port }, transferList: [port] });",
        // moveMessagePortToContext
        "const moved = moveMessagePortToContext(port, context);",
        "import { moveMessagePortToContext } from \"node:worker_threads\";",
        // postMessageToThread, isInternalThread, SHARE_ENV, the clone marks, message channels and environment data
        "import { postMessageToThread, isInternalThread } from \"node:worker_threads\";",
        "const { SHARE_ENV, Worker: W } = require(\"node:worker_threads\"); new W(src, { eval: true, env: SHARE_ENV });",
        "import { markAsUntransferable, markAsUncloneable, isMarkedAsUntransferable } from \"node:worker_threads\";",
        "import { MessageChannel, setEnvironmentData, getEnvironmentData } from \"node:worker_threads\";",
    ] {
        let user = format!("{import}{program}");
        assert!(
            thaw_js_package_eligible(&builtin_routing_package("node:worker_threads"), &bundle, &user, &Default::default(), ""),
            "{program} must route to thaw-js"
        );
    }
}

#[test]
fn thaw_js_worker_threads_form_is_allowlisted_scan_clean_and_present() {
    assert!(THAW_JS_BUILTIN_MODULES.contains(&"node:worker_threads"));
    let form = thaw_js::scan::script_as_bundle(THAW_JS_WORKER_THREADS_FORM).into_owned();
    let report = thaw_js::scan::scan_bundle(&form).unwrap();
    assert!(report.is_clean(), "{:?}", report.blockers);
    assert_eq!(report.entry, "thaw-js:script");
}
