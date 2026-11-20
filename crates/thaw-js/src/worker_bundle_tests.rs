//! A `Worker` whose filename is a bundled module's virtual path (`/thaw_modules/<key>`, what that module's
//! `__filename` reads as, as esbuild's child worker does) runs that factory as the worker's main module, from the
//! bundle the parent loaded. Any other filename keeps the disk loader (`main_file.js`).

use crate::host::Package;

/// A one-module registry bundle (key `a/index.js`). Its factory posts its own `__filename` from a worker and, in
/// the parent, spawns `spawn` (a JS expression for the worker's filename) and records the worker's first event.
fn parent_bundle(spawn: &str) -> String {
    format!(
        r#"module.exports = (function(require){{ var __thaw_bundle_factories = {{"a/index.js": function(module, exports, require, _u, __filename, __dirname) {{
  if (typeof __thaw_worker_side_exports !== 'undefined') {{
    __thaw_worker_side_exports.parentPort.postMessage(__filename);
  }} else {{
    const w = new globalThis.__thaw_worker_threads_exports.Worker({spawn}, {{ eval: false }});
    w.on('message', (v) => {{ globalThis.got = 'message ' + v; }});
    w.on('error', (e) => {{ globalThis.got = 'error ' + e.message; }});
  }}
}}}}; var __thaw_bundle_require_maps = {{"a/index.js": {{}}}}; var __thaw_bundle_entry_key = "a/index.js"; }})(globalThis.require);"#
    )
}

/// Loads the parent bundle, runs its event loop (the worker's events arrive there); returns `globalThis.got`.
fn run_parent(bundle: &str) -> String {
    let mut package = Package::load_named("a", bundle, 0).expect("bundle loads");
    let it = package.interp();
    it.run_event_loop().expect("event loop runs");
    let v = it.run("String(globalThis.got)").expect("read result");
    it.to_str(&v).to_string()
}

#[test]
fn worker_file_at_a_bundled_key_runs_that_factory_as_its_main_module() {
    // RED before the change: the disk loader cannot find `/thaw_modules/a/index.js`
    assert_eq!(
        run_parent(&parent_bundle("__filename")),
        "message /thaw_modules/a/index.js"
    );
}

#[test]
fn worker_file_outside_every_bundle_keeps_the_disk_loader() {
    assert_eq!(
        run_parent(&parent_bundle("'/nonexistent/x.js'")),
        "error Cannot find module '/nonexistent/x.js'"
    );
}

#[test]
fn worker_file_at_a_key_no_loaded_bundle_has_keeps_the_disk_loader() {
    assert_eq!(
        run_parent(&parent_bundle("'/thaw_modules/missing/y.js'")),
        "error Cannot find module '/thaw_modules/missing/y.js'"
    );
}

#[test]
fn worker_file_on_disk_still_runs_as_a_main_file() {
    let path = std::env::temp_dir().join(format!("thaw_worker_bundle_{}.js", std::process::id()));
    std::fs::write(
        &path,
        "require('node:worker_threads').parentPort.postMessage('disk ran');",
    )
    .expect("write worker file");
    let spawn = format!("{:?}", path.to_string_lossy());
    let got = run_parent(&parent_bundle(&spawn));
    let _ = std::fs::remove_file(&path);
    assert_eq!(got, "message disk ran");
}

#[test]
fn marked_classic_worker_url_is_rewritten_to_the_bundled_key_and_runs_that_module() {
    // RED before the rewrite: the classic data URL is not a bundled key, so the disk loader cannot find it
    let spawn = "/*thaw-worker-url:a/index.js*/('data:text/javascript;thaw-bundle,unused')/*thaw-worker-url-end*/";
    assert_eq!(
        run_parent(&parent_bundle(spawn)),
        "message /thaw_modules/a/index.js"
    );
}
