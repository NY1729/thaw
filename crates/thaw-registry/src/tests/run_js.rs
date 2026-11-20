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
