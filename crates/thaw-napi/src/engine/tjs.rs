//! `engine-thaw-js` backend: the thaw-js archive's `thaw_tjs_*` natives behind the
//! QuickJS-era `thaw_js_*` names and signatures that `crate::engine::` callers use, so the
//! call sites stay unchanged. Lib code is `extern "C"` only; there is no Cargo edge to thaw-js
//! (see the `[dev-dependencies]` comment in Cargo.toml).

use libc::{c_char, c_void};

// Result structs keep the field types the call sites read (`*const c_char`). Layout matches
// thaw-js's `#[repr(C)]` mirrors field for field (same order, pointer-sized fields).
#[repr(C)]
pub struct ThawResult {
    pub value: *const c_char,
    pub error: *const c_char,
}

#[repr(C)]
pub struct ThawHandleResult {
    pub value: u64,
    pub error: *const c_char,
}

#[repr(C)]
pub struct ThawCallWithExceptionResult {
    pub value: u64,
    pub exception_handle: u64,
    pub exception_object_like: u8,
    pub error: *const c_char,
}

#[repr(C)]
pub struct ThawStringCoercionResult {
    pub units: *const c_char,
    pub exception_handle: u64,
    pub exception_object_like: u8,
    pub error: *const c_char,
}

// Bridge callback types, identical to thaw-js's `napi_bridge` aliases (ABI only).
type BridgeCallback = unsafe extern "C" fn(*const c_char, *const c_char) -> *const c_char;
type BridgeExports = unsafe extern "C" fn() -> *const c_char;
type BridgePoll = extern "C" fn() -> usize;
type BridgePending = extern "C" fn() -> u8;
type BridgeOwner = unsafe extern "C" fn(u64) -> *mut c_char;
type BridgeRelease = extern "C" fn(u64) -> u8;
type BridgeHandle = unsafe extern "C" fn(
    *const c_char,
    *const c_char,
    *const c_char,
    *const c_char,
) -> *const c_char;

extern "C" {
    fn thaw_tjs_eval_json(source: *const c_char) -> ThawResult;
    fn thaw_tjs_register_napi_bridge(
        exports: BridgeExports,
        call: BridgeCallback,
        handle: BridgeHandle,
        poll: BridgePoll,
        pending: BridgePending,
        owner: BridgeOwner,
        release: BridgeRelease,
    );
    fn thaw_tjs_register_napi_shutdown_bridge(
        begin: extern "C" fn() -> u8,
        poll: extern "C" fn() -> u8,
        take_error: extern "C" fn() -> *mut c_char,
        finish: extern "C" fn() -> u8,
    );
    fn thaw_tjs_napi_enable_proxy_layer();
    fn thaw_tjs_abstract_to_string_units_result(
        handle: u64,
        array_element: u8,
    ) -> ThawStringCoercionResult;
    fn thaw_tjs_call_handle_with_this_graph_result(
        handle: u64,
        args_json: *const c_char,
    ) -> ThawCallWithExceptionResult;
    fn thaw_tjs_call_reference(context: *mut c_void, args_json: *const c_char) -> *const c_char;
    fn thaw_tjs_napi_call_reference_graph(
        context: *mut c_void,
        args_json: *const c_char,
    ) -> *const c_char;
    fn thaw_tjs_construct_handle_graph_args_result(
        handle: u64,
        args_json: *const c_char,
    ) -> ThawHandleResult;
    fn thaw_tjs_define_callback_property_key_handle_result(
        handle: u64,
        key_handle: u64,
        getter_id: u64,
        setter_id: u64,
        method_id: u64,
        attributes: u32,
        getter_root: u64,
        setter_root: u64,
        method_root: u64,
    ) -> ThawHandleResult;
    fn thaw_tjs_define_data_property_key_handle_graph_result(
        handle: u64,
        key_handle: u64,
        graph_json: *const c_char,
        attributes: u32,
    ) -> ThawHandleResult;
    fn thaw_tjs_get_global(name: *const c_char) -> i64;
    fn thaw_tjs_get_property_json_key_result(
        handle: u64,
        key_json: *const c_char,
    ) -> ThawHandleResult;
    fn thaw_tjs_get_property_json_key_with_exception_result(
        handle: u64,
        key_json: *const c_char,
    ) -> ThawCallWithExceptionResult;
    fn thaw_tjs_get_property_key_handle_with_exception_result(
        handle: u64,
        key_handle: u64,
    ) -> ThawCallWithExceptionResult;
    fn thaw_tjs_get_property_result(handle: u64, name: *const c_char) -> ThawHandleResult;
    fn thaw_tjs_get_prototype_handle_result(handle: u64) -> ThawHandleResult;
    fn thaw_tjs_graph_native_pair_matches_result(
        handle: u64,
        native_handle: u64,
        kind: u8,
    ) -> ThawHandleResult;
    fn thaw_tjs_handle_is_promise_result(handle: u64) -> ThawHandleResult;
    fn thaw_tjs_host_object_integrity_result(handle: u64, freeze: u8) -> ThawHandleResult;
    fn thaw_tjs_napi_host_property_names_result(
        handle: u64,
        key_mode: i32,
        key_filter: u32,
        key_conversion: i32,
    ) -> ThawResult;
    fn thaw_tjs_host_query_result(handle: u64, operation: u8) -> ThawResult;
    fn thaw_tjs_instanceof_graph_result(arguments_graph: *const c_char) -> ThawHandleResult;
    fn thaw_tjs_instanceof_handles_result(
        object_handle: u64,
        constructor_handle: u64,
    ) -> ThawHandleResult;
    fn thaw_tjs_load_script(source: *const c_char) -> u8;
    fn thaw_tjs_napi_graph_symbol_register(owner: u64, handle: u64) -> u64;
    fn thaw_tjs_napi_handle_is_symbol(handle: u64) -> u8;
    fn thaw_tjs_napi_symbol_abstract_to_string_units_result(
        owner: u64,
        id: u64,
    ) -> ThawStringCoercionResult;
    fn thaw_tjs_napi_symbol_set_pin(owner: u64, id: u64, pin: u8) -> u8;
    fn thaw_tjs_napi_symbol_weak_live(owner: u64, id: u64) -> u8;
    fn thaw_tjs_native_symbol_handle_result(native_handle: u64) -> ThawHandleResult;
    fn thaw_tjs_property_predicate_json_key_result(
        handle: u64,
        key_json: *const c_char,
        operation: u8,
    ) -> ThawHandleResult;
    fn thaw_tjs_property_predicate_key_handle_result(
        handle: u64,
        key_handle: u64,
        operation: u8,
    ) -> ThawHandleResult;
    fn thaw_tjs_release_handle(handle: i64) -> u8;
    fn thaw_tjs_retain_handle(handle: i64) -> u8;
    fn thaw_tjs_set_property_graph_with_exception_result(
        handle: u64,
        key_json: *const c_char,
        graph_json: *const c_char,
    ) -> ThawCallWithExceptionResult;
    fn thaw_tjs_set_property_key_handle_graph_with_exception_result(
        handle: u64,
        key_handle: u64,
        graph_json: *const c_char,
        receiver_data: u8,
    ) -> ThawCallWithExceptionResult;
    fn thaw_tjs_set_prototype_graph_result(
        handle: u64,
        prototype_graph: *const c_char,
    ) -> ThawHandleResult;
    fn thaw_tjs_set_receiver_data_graph_with_exception_result(
        handle: u64,
        key_json: *const c_char,
        graph_json: *const c_char,
    ) -> ThawCallWithExceptionResult;
    fn thaw_tjs_strict_equal_handle_scalar_result(
        handle: u64,
        kind: u8,
        number: f64,
        text: *const c_char,
    ) -> ThawHandleResult;
    fn thaw_tjs_strict_equal_handles_result(
        left_handle: u64,
        right_handle: u64,
    ) -> ThawHandleResult;
    fn thaw_tjs_string_from_utf16_units_result(units_json: *const c_char) -> ThawHandleResult;
}

/// Evaluates `source` in the default realm and returns its JSON text (`None` for `undefined`),
/// or the error text.
pub fn eval_json(source: &str) -> Result<Option<String>, String> {
    let Ok(source) = std::ffi::CString::new(source) else {
        return Err("source contains a NUL byte".to_string());
    };
    // SAFETY: `source` is a valid NUL-terminated string for the call; both returned strings are
    // owned and released with `thaw_arena::destroy_string` below.
    unsafe {
        let result = thaw_tjs_eval_json(source.as_ptr());
        if !result.error.is_null() {
            let error = std::ffi::CStr::from_ptr(result.error)
                .to_string_lossy()
                .into_owned();
            thaw_arena::destroy_string(result.error);
            return Err(error);
        }
        if result.value.is_null() {
            return Ok(None);
        }
        let value = std::ffi::CStr::from_ptr(result.value)
            .to_string_lossy()
            .into_owned();
        thaw_arena::destroy_string(result.value);
        Ok(Some(value))
    }
}

/// Registers the napi side's seven bridge callbacks, then enables the thaw-js proxy layer (it is
/// opt-in; the QuickJS engine has no equivalent step).
#[allow(clippy::too_many_arguments)]
pub fn register_napi_bridge(
    exports: BridgeExports,
    call: BridgeCallback,
    handle: BridgeHandle,
    poll: BridgePoll,
    pending: BridgePending,
    owner: BridgeOwner,
    release: BridgeRelease,
) {
    // SAFETY: the callbacks are plain fn pointers stored by thaw-js for the process lifetime.
    unsafe {
        thaw_tjs_register_napi_bridge(exports, call, handle, poll, pending, owner, release);
        thaw_tjs_napi_enable_proxy_layer();
    }
}

pub fn register_napi_shutdown_bridge(
    begin: extern "C" fn() -> u8,
    poll: extern "C" fn() -> u8,
    take_error: extern "C" fn() -> *mut c_char,
    finish: extern "C" fn() -> u8,
) {
    // SAFETY: plain fn pointers, stored by thaw-js.
    unsafe { thaw_tjs_register_napi_shutdown_bridge(begin, poll, take_error, finish) }
}

// Wrappers keep the QuickJS-era names and signatures used by `crate::engine::` callers.
pub extern "C" fn thaw_js_abstract_to_string_units_result(
    handle: u64,
    array_element: u8,
) -> ThawStringCoercionResult {
    unsafe { thaw_tjs_abstract_to_string_units_result(handle, array_element) }
}
pub extern "C" fn thaw_js_call_handle_with_this_graph_result(
    handle: u64,
    args_json: *const c_char,
) -> ThawCallWithExceptionResult {
    unsafe { thaw_tjs_call_handle_with_this_graph_result(handle, args_json) }
}
pub unsafe extern "C" fn thaw_js_call_reference(
    context: *mut std::ffi::c_void,
    args_json: *const c_char,
) -> *const c_char {
    unsafe { thaw_tjs_call_reference(context, args_json) }
}
pub unsafe extern "C" fn thaw_js_call_reference_graph(
    context: *mut std::ffi::c_void,
    args_json: *const c_char,
) -> *const c_char {
    unsafe { thaw_tjs_napi_call_reference_graph(context, args_json) }
}
pub extern "C" fn thaw_js_construct_handle_graph_args_result(
    handle: u64,
    args_json: *const c_char,
) -> ThawHandleResult {
    unsafe { thaw_tjs_construct_handle_graph_args_result(handle, args_json) }
}
pub extern "C" fn thaw_js_define_callback_property_key_handle_result(
    handle: u64,
    key_handle: u64,
    getter_id: u64,
    setter_id: u64,
    method_id: u64,
    attributes: u32,
    getter_root: u64,
    setter_root: u64,
    method_root: u64,
) -> ThawHandleResult {
    unsafe {
        thaw_tjs_define_callback_property_key_handle_result(
            handle,
            key_handle,
            getter_id,
            setter_id,
            method_id,
            attributes,
            getter_root,
            setter_root,
            method_root,
        )
    }
}
pub extern "C" fn thaw_js_define_data_property_key_handle_graph_result(
    handle: u64,
    key_handle: u64,
    graph_json: *const c_char,
    attributes: u32,
) -> ThawHandleResult {
    unsafe {
        thaw_tjs_define_data_property_key_handle_graph_result(
            handle, key_handle, graph_json, attributes,
        )
    }
}
pub extern "C" fn thaw_js_get_global(name: *const c_char) -> u64 {
    unsafe { thaw_tjs_get_global(name) as u64 }
}
pub extern "C" fn thaw_js_get_property_json_key_result(
    handle: u64,
    key_json: *const c_char,
) -> ThawHandleResult {
    unsafe { thaw_tjs_get_property_json_key_result(handle, key_json) }
}
pub extern "C" fn thaw_js_get_property_json_key_with_exception_result(
    handle: u64,
    key_json: *const c_char,
) -> ThawCallWithExceptionResult {
    unsafe { thaw_tjs_get_property_json_key_with_exception_result(handle, key_json) }
}
pub extern "C" fn thaw_js_get_property_key_handle_with_exception_result(
    handle: u64,
    key_handle: u64,
) -> ThawCallWithExceptionResult {
    unsafe { thaw_tjs_get_property_key_handle_with_exception_result(handle, key_handle) }
}
pub extern "C" fn thaw_js_get_property_result(
    handle: u64,
    name: *const c_char,
) -> ThawHandleResult {
    unsafe { thaw_tjs_get_property_result(handle, name) }
}
pub extern "C" fn thaw_js_get_prototype_handle_result(handle: u64) -> ThawHandleResult {
    unsafe { thaw_tjs_get_prototype_handle_result(handle) }
}
pub extern "C" fn thaw_js_graph_native_pair_matches_result(
    handle: u64,
    native_handle: u64,
    kind: u8,
) -> ThawHandleResult {
    unsafe { thaw_tjs_graph_native_pair_matches_result(handle, native_handle, kind) }
}
pub extern "C" fn thaw_js_handle_is_promise_result(handle: u64) -> ThawHandleResult {
    unsafe { thaw_tjs_handle_is_promise_result(handle) }
}
pub extern "C" fn thaw_js_host_object_integrity_result(
    handle: u64,
    freeze: u8,
) -> ThawHandleResult {
    unsafe { thaw_tjs_host_object_integrity_result(handle, freeze) }
}
pub extern "C" fn thaw_js_host_property_names_result(
    handle: u64,
    key_mode: i32,
    key_filter: u32,
    key_conversion: i32,
) -> ThawResult {
    unsafe {
        thaw_tjs_napi_host_property_names_result(handle, key_mode, key_filter, key_conversion)
    }
}
pub extern "C" fn thaw_js_host_query_result(handle: u64, operation: u8) -> ThawResult {
    unsafe { thaw_tjs_host_query_result(handle, operation) }
}
pub extern "C" fn thaw_js_instanceof_graph_result(
    arguments_graph: *const c_char,
) -> ThawHandleResult {
    unsafe { thaw_tjs_instanceof_graph_result(arguments_graph) }
}
pub extern "C" fn thaw_js_instanceof_handles_result(
    object_handle: u64,
    constructor_handle: u64,
) -> ThawHandleResult {
    unsafe { thaw_tjs_instanceof_handles_result(object_handle, constructor_handle) }
}
pub extern "C" fn thaw_js_load(source: *const c_char) -> u8 {
    unsafe { thaw_tjs_load_script(source) }
}
pub extern "C" fn thaw_js_napi_graph_symbol_register(owner: u64, handle: u64) -> u64 {
    unsafe { thaw_tjs_napi_graph_symbol_register(owner, handle) }
}
pub extern "C" fn thaw_js_napi_handle_is_symbol(handle: u64) -> u8 {
    unsafe { thaw_tjs_napi_handle_is_symbol(handle) }
}
pub extern "C" fn thaw_js_napi_symbol_abstract_to_string_units_result(
    owner: u64,
    id: u64,
) -> ThawStringCoercionResult {
    unsafe { thaw_tjs_napi_symbol_abstract_to_string_units_result(owner, id) }
}
pub extern "C" fn thaw_js_napi_symbol_set_pin(owner: u64, id: u64, pin: u8) -> u8 {
    unsafe { thaw_tjs_napi_symbol_set_pin(owner, id, pin) }
}
pub extern "C" fn thaw_js_napi_symbol_weak_live(owner: u64, id: u64) -> u8 {
    unsafe { thaw_tjs_napi_symbol_weak_live(owner, id) }
}
pub extern "C" fn thaw_js_native_symbol_handle_result(native_handle: u64) -> ThawHandleResult {
    unsafe { thaw_tjs_native_symbol_handle_result(native_handle) }
}
pub extern "C" fn thaw_js_property_predicate_json_key_result(
    handle: u64,
    key_json: *const c_char,
    operation: u8,
) -> ThawHandleResult {
    unsafe { thaw_tjs_property_predicate_json_key_result(handle, key_json, operation) }
}
pub extern "C" fn thaw_js_property_predicate_key_handle_result(
    handle: u64,
    key_handle: u64,
    operation: u8,
) -> ThawHandleResult {
    unsafe { thaw_tjs_property_predicate_key_handle_result(handle, key_handle, operation) }
}
pub extern "C" fn thaw_js_release_handle(handle: u64) -> u8 {
    unsafe { thaw_tjs_release_handle(handle as i64) }
}
pub extern "C" fn thaw_js_retain_handle(handle: u64) -> u8 {
    unsafe { thaw_tjs_retain_handle(handle as i64) }
}
pub extern "C" fn thaw_js_set_property_graph_with_exception_result(
    handle: u64,
    key_json: *const c_char,
    graph_json: *const c_char,
) -> ThawCallWithExceptionResult {
    unsafe { thaw_tjs_set_property_graph_with_exception_result(handle, key_json, graph_json) }
}
pub extern "C" fn thaw_js_set_property_key_handle_graph_with_exception_result(
    handle: u64,
    key_handle: u64,
    graph_json: *const c_char,
    receiver_data: u8,
) -> ThawCallWithExceptionResult {
    unsafe {
        thaw_tjs_set_property_key_handle_graph_with_exception_result(
            handle,
            key_handle,
            graph_json,
            receiver_data,
        )
    }
}
pub extern "C" fn thaw_js_set_prototype_graph_result(
    handle: u64,
    prototype_graph: *const c_char,
) -> ThawHandleResult {
    unsafe { thaw_tjs_set_prototype_graph_result(handle, prototype_graph) }
}
pub extern "C" fn thaw_js_set_receiver_data_graph_with_exception_result(
    handle: u64,
    key_json: *const c_char,
    graph_json: *const c_char,
) -> ThawCallWithExceptionResult {
    unsafe { thaw_tjs_set_receiver_data_graph_with_exception_result(handle, key_json, graph_json) }
}
pub extern "C" fn thaw_js_strict_equal_handle_scalar_result(
    handle: u64,
    kind: u8,
    number: f64,
    text: *const c_char,
) -> ThawHandleResult {
    unsafe { thaw_tjs_strict_equal_handle_scalar_result(handle, kind, number, text) }
}
pub extern "C" fn thaw_js_strict_equal_handles_result(
    left_handle: u64,
    right_handle: u64,
) -> ThawHandleResult {
    unsafe { thaw_tjs_strict_equal_handles_result(left_handle, right_handle) }
}
pub extern "C" fn thaw_js_string_from_utf16_units_result(
    units_json: *const c_char,
) -> ThawHandleResult {
    unsafe { thaw_tjs_string_from_utf16_units_result(units_json) }
}
