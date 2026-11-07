//! Engine seam: every JS-engine symbol thaw-napi uses goes through this module.
//! Stage 1: pure forwarding to thaw-quickjs. No behaviour change.

#[cfg(all(feature = "engine-quickjs", feature = "engine-thaw-js"))]
compile_error!("features `engine-quickjs` and `engine-thaw-js` are mutually exclusive");

#[cfg(all(feature = "engine-thaw-js", not(feature = "engine-quickjs")))]
mod tjs;
#[allow(unused_imports)]
#[cfg(all(feature = "engine-thaw-js", not(feature = "engine-quickjs")))]
pub use tjs::*;

#[allow(unused_imports)]
#[cfg(feature = "engine-quickjs")]
pub use thaw_quickjs::{
    eval_json, register_napi_bridge, register_napi_shutdown_bridge,
    thaw_js_abstract_to_string_units_result, thaw_js_call_handle_with_this_graph_result,
    thaw_js_call_reference, thaw_js_call_reference_graph,
    thaw_js_construct_handle_graph_args_result, thaw_js_define_callback_property_key_handle_result,
    thaw_js_define_data_property_key_handle_graph_result, thaw_js_get_global,
    thaw_js_get_property_json_key_result, thaw_js_get_property_json_key_with_exception_result,
    thaw_js_get_property_key_handle_with_exception_result, thaw_js_get_property_result,
    thaw_js_get_prototype_handle_result, thaw_js_graph_native_pair_matches_result,
    thaw_js_handle_is_promise_result, thaw_js_host_object_integrity_result,
    thaw_js_host_property_names_result, thaw_js_host_query_result, thaw_js_instanceof_graph_result,
    thaw_js_instanceof_handles_result, thaw_js_load, thaw_js_napi_graph_symbol_register,
    thaw_js_napi_handle_is_symbol, thaw_js_napi_symbol_abstract_to_string_units_result,
    thaw_js_napi_symbol_set_pin, thaw_js_napi_symbol_weak_live,
    thaw_js_native_symbol_handle_result, thaw_js_property_predicate_json_key_result,
    thaw_js_property_predicate_key_handle_result, thaw_js_release_handle, thaw_js_retain_handle,
    thaw_js_set_property_graph_with_exception_result,
    thaw_js_set_property_key_handle_graph_with_exception_result,
    thaw_js_set_prototype_graph_result, thaw_js_set_receiver_data_graph_with_exception_result,
    thaw_js_strict_equal_handle_scalar_result, thaw_js_strict_equal_handles_result,
    thaw_js_string_from_utf16_units_result, ThawCallWithExceptionResult, ThawHandleResult,
    ThawResult, ThawStringCoercionResult,
};
