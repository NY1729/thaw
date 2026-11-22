/// Runs the entry export of a registry `bundle` (`bundle_commonjs_package` output) on thaw-js with no
/// arguments and returns the result as the string the QuickJS tests compared against: a string result
/// verbatim, anything else as its plain-data JSON. Errors come back as `ERROR: {error}`.
/// The 256 MB stack matches the QuickJS tests' native stack budget.
fn run_js(bundle: &str) -> String {
    run_js_with_args(bundle, Vec::new())
}

/// `run_js` with `args` passed to the entry export, for tests whose QuickJS version passed a JSON argument array.
fn run_js_with_args(bundle: &str, args: Vec<thaw_js::host::Native>) -> String {
    run_js_export(bundle, "", args)
}

/// `run_js_with_args` for a named export of `bundle` (`export` is a `Package::call` path, e.g. `"default"`).
fn run_js_export(bundle: &str, export: &str, args: Vec<thaw_js::host::Native>) -> String {
    match run_js_outcome(bundle, export, args) {
        Ok(thaw_js::host::Native::Str(text)) => text,
        Ok(other) => other.to_json(),
        Err(error) => error,
    }
}

/// `run_js` that always returns the JSON of the result, so a string result stays JSON-quoted (`"\"symbol\""`),
/// as the QuickJS helper returned it. Errors come back as `ERROR: {error}`.
fn run_js_json(bundle: &str) -> String {
    match run_js_outcome(bundle, "", Vec::new()) {
        Ok(result) => result.to_json(),
        Err(error) => error,
    }
}

/// `run_js_json` with `args` passed to the entry export (a string result stays JSON-quoted).
fn run_js_json_with_args(bundle: &str, args: Vec<thaw_js::host::Native>) -> String {
    match run_js_outcome(bundle, "", args) {
        Ok(result) => result.to_json(),
        Err(error) => error,
    }
}

/// The shared thaw-js machinery: load `bundle`, call `export` with `args` on a 256 MB-stack thread.
/// A load or call failure comes back as `Err("ERROR: {error}")`.
fn run_js_outcome(
    bundle: &str,
    export: &str,
    args: Vec<thaw_js::host::Native>,
) -> Result<thaw_js::host::Native, String> {
    let bundle = bundle.to_string();
    let export = export.to_string();
    std::thread::Builder::new()
        .stack_size(256 << 20)
        .spawn(move || match thaw_js::host::Package::load(&bundle) {
            Ok(mut package) => package
                .call(&export, &args)
                .map_err(|error| format!("ERROR: {error}")),
            Err(error) => Err(format!("ERROR: {error}")),
        })
        .unwrap()
        .join()
        .unwrap()
}

// Stateful tests: one package loaded on a 256 MB-stack thread through the routed `thaw_tjs_*` entry points,
// the thread-local package table that `thaw_js_load` / `thaw_js_run_event_loop` used. Calls and event-loop
// turns share that package's state, and the loop services native watchers.

/// Loads `bundle` on a 256 MB-stack thread and runs `body` against it; `body` may call several exports and run the loop.
fn run_js_session<T: Send + 'static>(bundle: &str, body: impl FnOnce(&Session) -> T + Send + 'static) -> T {
    let bundle = bundle.to_string();
    std::thread::Builder::new()
        .stack_size(256 << 20)
        .spawn(move || body(&Session::load(&bundle)))
        .unwrap()
        .join()
        .unwrap()
}

/// A package loaded by `Session::load`, addressed by its handle.
struct Session {
    handle: i64,
}

impl Session {
    fn load(bundle: &str) -> Session {
        let source = std::ffi::CString::new(bundle).unwrap();
        let handle = thaw_js::host::thaw_tjs_load(source.as_ptr());
        assert!(
            handle >= 0,
            "bundle failed to load: {}",
            take_c_string(thaw_js::host::thaw_tjs_last_error())
        );
        Session { handle }
    }

    /// Calls `path` on the package exports with `args`; returns the result as JSON, or `ERROR: {error}`.
    fn call(&self, path: &str, args: &[thaw_js::host::Native]) -> String {
        let path = std::ffi::CString::new(path).unwrap();
        let args = std::ffi::CString::new(thaw_js::host::Native::Array(args.to_vec()).to_json()).unwrap();
        let result = thaw_js::host::thaw_tjs_call(self.handle, path.as_ptr(), args.as_ptr());
        if result.is_null() {
            format!("ERROR: {}", take_c_string(thaw_js::host::thaw_tjs_last_error()))
        } else {
            take_c_string(result)
        }
    }

    /// One run of the event loop over every loaded package; returns 1 if a package ended with an uncaught error.
    fn run_event_loop(&self) -> i32 {
        thaw_js::host::thaw_tjs_run_event_loop()
    }
}

/// Reads and frees a C string returned by a `thaw_tjs_*` entry point.
fn take_c_string(pointer: *mut std::ffi::c_char) -> String {
    let text = unsafe { std::ffi::CStr::from_ptr(pointer) }
        .to_string_lossy()
        .into_owned();
    unsafe { thaw_js::host::thaw_tjs_free(pointer) };
    text
}


/// The JSON of the member at `path` of a registry `bundle`'s exports (`""` = the entry export itself), not called.
/// For the native-ESM bundles, which only `Package::load` registers (`__thaw_register_native_bundle`).
/// Errors come back as `ERROR: {error}`. 256 MB stack, like `run_js_outcome`.
fn run_js_get(bundle: &str, path: &str) -> String {
    let (bundle, path) = (bundle.to_string(), path.to_string());
    std::thread::Builder::new()
        .stack_size(256 << 20)
        .spawn(move || match thaw_js::host::Package::load(&bundle) {
            Ok(mut package) => package
                .get(&path)
                .map(|value| value.to_json())
                .unwrap_or_else(|error| format!("ERROR: {error}")),
            Err(error) => format!("ERROR: {error}"),
        })
        .unwrap()
        .join()
        .unwrap()
}

/// The JSON of the JS expression `expr` evaluated after `bundle` loads, awaited like `Package::call`
/// (`run_js_get` does not await). `expr` sees the package exports as `exports` and can read globals and
/// promise-valued exports. It runs as a closure set on `exports` (`__thaw_probe`). Errors come back as
/// `ERROR: {error}`. 256 MB stack, like `run_js_outcome`.
fn run_js_probe(bundle: &str, expr: &str) -> String {
    let (bundle, expr) = (bundle.to_string(), expr.to_string());
    std::thread::Builder::new()
        .stack_size(256 << 20)
        .spawn(move || match thaw_js::host::Package::load(&bundle) {
            Ok(mut package) => {
                let exports = package.exports();
                let installed = {
                    let it = package.interp();
                    it.eval_function(&format!(
                        "function () {{ return (function (exports) {{ return ({expr}); }})(this); }}"
                    ))
                        .and_then(|probe| it.set(&exports, "__thaw_probe", probe))
                        .is_ok()
                };
                if !installed {
                    return "ERROR: probe setup failed".to_string();
                }
                package
                    .call("__thaw_probe", &[])
                    .map(|value| value.to_json())
                    .unwrap_or_else(|error| format!("ERROR: {error}"))
            }
            Err(error) => format!("ERROR: {error}"),
        })
        .unwrap()
        .join()
        .unwrap()
}

/// Writes `files` (path relative to a fresh fixture dir named `test_name` -> exact contents) and returns the dir.
/// The worker-file tests put real files on disk so `new Worker(<absolute path>)` loads them through the same
/// loader a product worker uses.
fn worker_fixture(test_name: &str, files: &[(&str, &str)]) -> PathBuf {
    let dir = temp_registry(test_name);
    for (relative, contents) in files {
        let path = dir.join(relative);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, contents).unwrap();
    }
    dir
}

/// Parent-side helper for `run_worker_fixture` bodies: `runWorker(file, onFix)` resolves `{messages, errors}` once the
/// worker exits (a `'fix'` message goes to `onFix` instead of `messages`), or `{syncCode}` when `new Worker(file)` throws.
const WORKER_RUNNER_JS: &str = "var Worker = require('node:worker_threads').Worker; \
    function runWorker(file, onFix) { return new Promise(function (resolve) { \
        var out = { messages: [], errors: [] }, worker; \
        try { worker = new Worker(file); } catch (error) { resolve({ syncCode: error.code }); return; } \
        worker.on('message', function (value) { if (value === 'fix' && onFix) onFix(worker); else out.messages.push(value); }); \
        worker.on('error', function (error) { out.errors.push(error.code || error.name); }); \
        worker.on('exit', function () { resolve(out); }); \
    }); }";

/// Bundles `body` as the async entry of a package whose `index.js` is `WORKER_RUNNER_JS` plus `body` (`runWorker`
/// is in scope), runs it on thaw-js, and returns the result JSON. `dir` holds the worker files and must exist.
fn run_worker_fixture(dir: &Path, body: &str) -> String {
    fs::write(
        dir.join("index.js"),
        format!("{WORKER_RUNNER_JS} module.exports = async function () {{ {body} }};"),
    )
    .unwrap();
    let node_modules = temp_registry(&format!(
        "{}_node_modules",
        dir.file_name().unwrap().to_string_lossy()
    ));
    let (bundle, _, _, _) = bundle_commonjs_package(&node_modules, "pkg", dir, "index.js").unwrap();
    let result = run_js(&bundle);
    let _ = fs::remove_dir_all(node_modules);
    result
}
