/// Test-only route for the bridge tests: load generated bridge JS as sloppy scripts in thaw-js's default realm,
/// call a global by name (awaited), `eval_json` for the completion value. Not a product API.
mod thaw_js_shim {
    use std::ffi::{c_char, CStr, CString};

    #[repr(C)]
    pub struct ThawResult {
        value: *mut c_char,
        error: *mut c_char,
    }

    extern "C" {
        fn thaw_tjs_napi_enable_proxy_layer();
        fn thaw_tjs_eval_json(source: *const c_char) -> ThawResult;
        fn thaw_tjs_call_reference(context: *mut std::ffi::c_void, args: *const c_char) -> *const c_char;
    }

    /// Opts the thread's default realm into the native addon proxy layer (defines the graph helpers).
    pub fn enable_napi_proxy_layer() {
        unsafe { thaw_tjs_napi_enable_proxy_layer() }
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
