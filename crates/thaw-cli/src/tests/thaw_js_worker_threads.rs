// node:worker_threads on thaw-js (W1, W3): routing blocker and the thaw-js form.

#[test]
fn thaw_js_worker_threads_blocker_keeps_unsupported_worker_use_on_quickjs() {
    let worker = "import { Worker } from \"node:worker_threads\";";
    // Worker runs on thaw-js (W3): plain Worker programs route to it, Worker options it does not serve do not
    assert!(thaw_js_worker_threads_blocker("node:worker_threads", worker).is_none());
    // stdin is served on thaw-js (W5): a Worker program that uses stdin routes to thaw-js too
    let stdin = "import { Worker } from \"node:worker_threads\";\nconst w = new Worker(src, { eval: true, stdin: true });";
    assert!(thaw_js_worker_threads_blocker("node:worker_threads", stdin).is_none());
    let limits = "import { Worker } from \"node:worker_threads\";\nnew Worker(src, { eval: true, resourceLimits: {} });";
    assert!(thaw_js_worker_threads_blocker("node:worker_threads", limits).is_some());
    assert!(thaw_js_worker_threads_blocker(
        "node:worker_threads",
        "import { postMessageToThread } from \"node:worker_threads\";"
    )
    .is_some());
    assert!(thaw_js_worker_threads_blocker(
        "node:worker_threads",
        "const { SHARE_ENV } = require(\"node:worker_threads\");"
    )
    .is_some());
    // the W1 subset (message channels and environment data) routes to thaw-js
    let subset = "import { MessageChannel, setEnvironmentData, getEnvironmentData } from \"node:worker_threads\";";
    assert!(thaw_js_worker_threads_blocker("node:worker_threads", subset).is_none());
    // other packages are not judged by this blocker
    assert!(thaw_js_worker_threads_blocker("node:path", worker).is_none());
}

#[test]
fn thaw_js_worker_threads_form_is_allowlisted_scan_clean_and_present() {
    assert!(THAW_JS_BUILTIN_MODULES.contains(&"node:worker_threads"));
    let form = thaw_js::scan::script_as_bundle(THAW_JS_WORKER_THREADS_FORM).into_owned();
    let report = thaw_js::scan::scan_bundle(&form).unwrap();
    assert!(report.is_clean(), "{:?}", report.blockers);
    assert_eq!(report.entry, "thaw-js:script");
}
