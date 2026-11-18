//! `THAW_JS_PROFILE=1` end to end: typed calls made through the C ABI (as AOT code makes them) are
//! counted and reported on stderr at process exit. The switch is read once per process, so the
//! counted calls run in a child process (this same test binary) that has it set.
use std::ffi::{CStr, CString};
use thaw_js::host::{thaw_tjs_call_typed_result, thaw_tjs_free, thaw_tjs_register};

const BUNDLE: &str = r#"module.exports = (function(require){ var __thaw_bundle_factories = {"a/index.js": function(module, exports, require) { exports.f = function () { return 42; }; }}; var __thaw_bundle_require_maps = {"a/index.js": {}}; var __thaw_bundle_entry_key = "a/index.js"; })(globalThis.require);"#;

#[test]
fn profile_counts_typed_calls() {
    if std::env::var_os("THAW_JS_PROFILE_CHILD").is_none() {
        let exe = std::env::current_exe().unwrap();
        let output = std::process::Command::new(exe)
            .args(["--exact", "profile_counts_typed_calls", "--test-threads=1"])
            .env("THAW_JS_PROFILE", "1")
            .env("THAW_JS_PROFILE_CHILD", "1")
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
        let stderr = String::from_utf8_lossy(&output.stderr);
        let row = stderr
            .lines()
            .find(|line| line.starts_with("thaw-js-profile\tprofile_pkg::f\t"))
            .unwrap_or_else(|| panic!("no row for profile_pkg::f in stderr: {stderr}"));
        let fields: Vec<&str> = row.split('\t').collect();
        assert_eq!(fields.len(), 4, "{row}");
        assert_eq!(fields[2], "2", "two outermost typed calls: {row}");
        assert!(fields[3].parse::<f64>().is_ok(), "{row}");
        let total = stderr
            .lines()
            .find(|line| line.starts_with("thaw-js-profile-total\t"))
            .unwrap_or_else(|| panic!("no total line: {stderr}"));
        assert_eq!(total.split('\t').count(), 3, "{total}");
        return;
    }

    let name = CString::new("profile_pkg").unwrap();
    let source = CString::new(BUNDLE).unwrap();
    assert_eq!(thaw_tjs_register(name.as_ptr(), source.as_ptr()), 1);
    let symbol = CString::new("profile_pkg::f").unwrap();
    let args = CString::new("[]").unwrap();
    for _ in 0..2 {
        let result = thaw_tjs_call_typed_result(symbol.as_ptr(), args.as_ptr());
        assert!(result.error.is_null());
        let value = unsafe { CStr::from_ptr(result.value) }
            .to_str()
            .unwrap()
            .to_owned();
        assert_eq!(value, "42");
        unsafe { thaw_tjs_free(result.value) };
    }
}
