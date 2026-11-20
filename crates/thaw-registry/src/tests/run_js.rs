/// Runs the entry export of a registry `bundle` (`bundle_commonjs_package` output) on thaw-js with no
/// arguments and returns the result as the string the QuickJS tests compared against: a string result
/// verbatim, anything else as its plain-data JSON. Errors come back as `ERROR: {error}`.
/// The 256 MB stack matches the QuickJS tests' native stack budget.
fn run_js(bundle: &str) -> String {
    run_js_with_args(bundle, Vec::new())
}

/// `run_js` with `args` passed to the entry export, for tests whose QuickJS version passed a JSON argument array.
fn run_js_with_args(bundle: &str, args: Vec<thaw_js::host::Native>) -> String {
    let bundle = bundle.to_string();
    std::thread::Builder::new()
        .stack_size(256 << 20)
        .spawn(move || match thaw_js::host::Package::load(&bundle) {
            Ok(mut package) => match package.call("", &args) {
                Ok(thaw_js::host::Native::Str(json)) => json,
                Ok(other) => other.to_json(),
                Err(error) => format!("ERROR: {error}"),
            },
            Err(error) => format!("ERROR: {error}"),
        })
        .unwrap()
        .join()
        .unwrap()
}
