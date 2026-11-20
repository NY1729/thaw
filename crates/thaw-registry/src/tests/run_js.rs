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

/// Test-only route for the QuickJS-era tests (`thaw_quickjs::thaw_js_load` / `thaw_js_call` / `eval_json` /
/// `thaw_js_run_event_loop`). It shadows the `thaw_quickjs` extern crate name inside the tests module
/// (the glob import from `use super::*` wins over the extern prelude), so those tests run on thaw-js with no
/// body edits. Script route: `thaw_js_load` runs the source as a sloppy script in thaw-js's default realm,
/// `thaw_js_call` calls a global function by name (awaited), `eval_json` returns the completion value as JSON.
/// Not a product API; the shipped code path is `thaw_js::host::Package`.
mod thaw_quickjs {
    use std::ffi::{c_char, CStr, CString};

    #[repr(C)]
    pub struct ThawResult {
        value: *mut c_char,
        error: *mut c_char,
    }

    extern "C" {
        fn thaw_tjs_eval_json(source: *const c_char) -> ThawResult;
        fn thaw_tjs_call_reference(context: *mut std::ffi::c_void, args: *const c_char) -> *const c_char;
    }

    pub fn thaw_js_run_event_loop() -> i32 {
        thaw_js::host::thaw_tjs_run_event_loop()
    }

    pub extern "C" fn thaw_js_load(source: *const c_char) -> u8 {
        thaw_js::host::thaw_tjs_load_script(source)
    }

    /// Completion value of `source` as JSON (`None` for undefined), or the thrown error text.
    pub fn eval_json(source: &str) -> Result<Option<String>, String> {
        let source = CString::new(source).unwrap();
        let result = unsafe { thaw_tjs_eval_json(source.as_ptr()) };
        // Leaked on purpose: test-only shim, the strings live for the test process.
        let take = |p: *mut c_char| unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned();
        if !result.error.is_null() {
            return Err(take(result.error));
        }
        Ok((!result.value.is_null()).then(|| take(result.value)))
    }

    pub extern "C" fn thaw_js_call(name: *const c_char, args: *const c_char) -> *const c_char {
        let name = unsafe { CStr::from_ptr(name) }.to_string_lossy().into_owned();
        let _ = eval_json(&format!("globalThis.__thaw_napi_reference_1 = globalThis[{name:?}]; 0"));
        let text = unsafe { thaw_tjs_call_reference(std::ptr::without_provenance_mut(1), args) };
        let text = unsafe { CStr::from_ptr(text) }.to_string_lossy().into_owned();
        let text = match text.strip_prefix('\u{2}') {
            Some(error) => format!("{{\"__thaw_error__\":{error}}}"),
            None => text,
        };
        CString::new(text).unwrap().into_raw()
    }
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
