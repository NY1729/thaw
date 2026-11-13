//! Handle 0 means `undefined` at every thaw-js boundary (user decision D1, 2026-11-13). The N-API
//! externs are the one exception: Node-API's NULL `napi_value` is invalid there, so they keep 0 invalid.
//!
//! The table below is the contract. The test scans the extern source text for every
//! `extern "C" fn thaw_tjs_*` that takes a handle-like `u64`/`i64` parameter and fails when one is not
//! classified, so a new handle operation cannot ship without a decision about 0.

use std::ffi::{c_char, CStr, CString};

/// How an extern treats handle 0.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Policy {
    /// Id 0 resolves to `undefined` (the convention).
    Lenient,
    /// N-API seam (decision D1): id 0 is an invalid handle, the bare "invalid JavaScript handle".
    StrictNapi,
    /// Retain/release: id 0 holds no reference, so these are no-ops that report 0.
    NoReference,
    /// The argument is a package handle (or another index), not a value handle.
    NotValueHandle,
}

/// Every thaw-js extern that takes a handle-like parameter, with its policy for 0.
const TABLE: &[(&str, Policy)] = &[
    ("thaw_tjs_call_handle_graph_result", Policy::Lenient),
    (
        "thaw_tjs_call_handle_handle_graph_args_result",
        Policy::Lenient,
    ),
    (
        "thaw_tjs_call_handle_mixed_exact_consuming_result",
        Policy::Lenient,
    ),
    (
        "thaw_tjs_call_handle_mixed_exact_resolved_consuming_result",
        Policy::Lenient,
    ),
    ("thaw_tjs_call_handle_mixed_graph_result", Policy::Lenient),
    (
        "thaw_tjs_call_handle_mixed_handle_graph_args_consuming_result",
        Policy::Lenient,
    ),
    (
        "thaw_tjs_call_handle_mixed_handle_graph_args_result",
        Policy::Lenient,
    ),
    ("thaw_tjs_call_handle_value_graph_result", Policy::Lenient),
    (
        "thaw_tjs_call_handle_with_this_graph_wire_result",
        Policy::Lenient,
    ),
    ("thaw_tjs_call_method_graph_result", Policy::Lenient),
    (
        "thaw_tjs_call_method_handle_graph_args_result",
        Policy::Lenient,
    ),
    (
        "thaw_tjs_call_selected_method_graph_result",
        Policy::Lenient,
    ),
    (
        "thaw_tjs_call_selected_method_handle_graph_args_result",
        Policy::Lenient,
    ),
    ("thaw_tjs_capture_handle_graph_result", Policy::Lenient),
    ("thaw_tjs_class_call", Policy::Lenient),
    (
        "thaw_tjs_construct_handle_graph_args_result",
        Policy::Lenient,
    ),
    ("thaw_tjs_delete_property_result", Policy::Lenient),
    ("thaw_tjs_exception_native_token", Policy::Lenient),
    ("thaw_tjs_get_property_exact_result", Policy::Lenient),
    ("thaw_tjs_get_property_json_key_result", Policy::Lenient),
    ("thaw_tjs_get_property_result", Policy::Lenient),
    ("thaw_tjs_handle_is_plain_data", Policy::Lenient),
    ("thaw_tjs_handle_to_console_string", Policy::Lenient),
    ("thaw_tjs_handle_to_string", Policy::Lenient),
    ("thaw_tjs_handle_typed_array_probe", Policy::Lenient),
    ("thaw_tjs_has_property_result", Policy::Lenient),
    ("thaw_tjs_host_date_set_result", Policy::Lenient),
    ("thaw_tjs_host_enumerate_result", Policy::Lenient),
    ("thaw_tjs_host_query_result", Policy::Lenient),
    ("thaw_tjs_intern_native_object", Policy::Lenient),
    ("thaw_tjs_iterator_step_exact_result", Policy::Lenient),
    ("thaw_tjs_native_object_pointer", Policy::Lenient),
    (
        "thaw_tjs_property_predicate_json_key_result",
        Policy::Lenient,
    ),
    ("thaw_tjs_resolve_handle_graph_result", Policy::Lenient),
    ("thaw_tjs_resolve_handle_handle_result", Policy::Lenient),
    ("thaw_tjs_set_property_graph_result", Policy::Lenient),
    ("thaw_tjs_set_property_json_graph_result", Policy::Lenient),
    ("thaw_tjs_set_property_result", Policy::Lenient),
    ("thaw_tjs_to_async_iterator_exact_result", Policy::Lenient),
    ("thaw_tjs_to_iterator_exact_result", Policy::Lenient),
    ("thaw_tjs_watch_promise", Policy::Lenient),
    ("thaw_tjs_release_handle", Policy::NoReference),
    ("thaw_tjs_retain_handle", Policy::NoReference),
    ("thaw_tjs_call", Policy::NotValueHandle),
    ("thaw_tjs_unload", Policy::NotValueHandle),
    (
        "thaw_tjs_call_handle_with_this_graph_result",
        Policy::StrictNapi,
    ),
    (
        "thaw_tjs_graph_native_pair_matches_result",
        Policy::StrictNapi,
    ),
    ("thaw_tjs_get_prototype_handle_result", Policy::StrictNapi),
    ("thaw_tjs_handle_is_promise_result", Policy::StrictNapi),
    ("thaw_tjs_host_object_integrity_result", Policy::StrictNapi),
    ("thaw_tjs_instanceof_handles_result", Policy::StrictNapi),
    (
        "thaw_tjs_napi_construct_handle_graph_args_result",
        Policy::StrictNapi,
    ),
    ("thaw_tjs_set_prototype_graph_result", Policy::StrictNapi),
    (
        "thaw_tjs_strict_equal_handle_scalar_result",
        Policy::StrictNapi,
    ),
    ("thaw_tjs_strict_equal_handles_result", Policy::StrictNapi),
    (
        "thaw_tjs_abstract_to_string_units_result",
        Policy::StrictNapi,
    ),
    (
        "thaw_tjs_define_callback_property_key_handle_result",
        Policy::StrictNapi,
    ),
    (
        "thaw_tjs_define_data_property_key_handle_graph_result",
        Policy::StrictNapi,
    ),
    (
        "thaw_tjs_get_property_json_key_with_exception_result",
        Policy::StrictNapi,
    ),
    (
        "thaw_tjs_get_property_key_handle_with_exception_result",
        Policy::StrictNapi,
    ),
    ("thaw_tjs_host_property_names_result", Policy::StrictNapi),
    (
        "thaw_tjs_napi_host_property_names_result",
        Policy::StrictNapi,
    ),
    (
        "thaw_tjs_property_predicate_key_handle_result",
        Policy::StrictNapi,
    ),
    (
        "thaw_tjs_set_property_graph_with_exception_result",
        Policy::StrictNapi,
    ),
    (
        "thaw_tjs_set_property_key_handle_graph_with_exception_result",
        Policy::StrictNapi,
    ),
    (
        "thaw_tjs_set_receiver_data_graph_with_exception_result",
        Policy::StrictNapi,
    ),
    ("thaw_tjs_napi_graph_symbol_register", Policy::StrictNapi),
    ("thaw_tjs_napi_handle_is_symbol", Policy::StrictNapi),
    ("thaw_tjs_native_symbol_handle_result", Policy::StrictNapi),
];

/// The source files the extern scan reads (the thaw-js crate's own handle operations).
const SOURCES: &[(&str, &str)] = &[
    ("host.rs", include_str!("host.rs")),
    ("napi_eval.rs", include_str!("napi_eval.rs")),
    ("napi_ops.rs", include_str!("napi_ops.rs")),
    ("napi_props.rs", include_str!("napi_props.rs")),
    ("napi_symbols.rs", include_str!("napi_symbols.rs")),
];

/// Whether a parameter (`name: type`) is a handle-like `u64`/`i64`.
fn is_handle_param(param: &str) -> bool {
    let Some((name, ty)) = param.split_once(':') else {
        return false;
    };
    if !matches!(ty.trim(), "u64" | "i64") {
        return false;
    }
    let name = name.trim().trim_start_matches('_');
    name.contains("handle")
        || matches!(
            name,
            "receiver"
                | "selected"
                | "source"
                | "iterator"
                | "argument"
                | "object"
                | "constructor"
                | "left"
                | "right"
                | "prototype"
                | "candidate"
        )
}

/// Every handle-taking `extern "C" fn thaw_tjs_*` in `SOURCES`, by name.
fn scanned_handle_externs() -> Vec<String> {
    let mut names = Vec::new();
    for (_, text) in SOURCES {
        let mut rest: &str = text;
        while let Some(at) = rest.find("extern \"C\" fn thaw_tjs_") {
            let after = &rest[at + "extern \"C\" fn ".len()..];
            let open = after.find('(').expect("extern parameter list");
            let close = after.find(')').expect("extern parameter list end");
            let name = &after[..open];
            if after[open + 1..close].split(',').any(is_handle_param) {
                names.push(name.to_string());
            }
            rest = &after[close..];
        }
    }
    names.sort();
    names.dedup();
    names
}

fn policy_of(name: &str) -> Option<Policy> {
    TABLE.iter().find(|(n, _)| *n == name).map(|(_, p)| *p)
}

fn text_of(pointer: *const c_char) -> String {
    if pointer.is_null() {
        return String::new();
    }
    unsafe { CStr::from_ptr(pointer) }
        .to_string_lossy()
        .into_owned()
}

/// Calls the Lenient extern `name` with handle 0 where a call needs no live fixture. Returns the error
/// text it reports (empty when none), or None when the extern is not called here.
fn call_with_zero(name: &str) -> Option<String> {
    use crate::host::{
        thaw_tjs_call_handle_graph_result, thaw_tjs_get_property_result,
        thaw_tjs_handle_is_plain_data, thaw_tjs_handle_to_console_string,
        thaw_tjs_handle_to_string, thaw_tjs_handle_typed_array_probe, thaw_tjs_has_property_result,
        thaw_tjs_native_object_pointer, thaw_tjs_resolve_handle_graph_result,
    };
    let layout = CString::new("probe").unwrap();
    let property = CString::new("x").unwrap();
    let args = CString::new("[]").unwrap();
    let error = match name {
        "thaw_tjs_get_property_result" => {
            text_of(thaw_tjs_get_property_result(0, property.as_ptr()).error)
        }
        "thaw_tjs_has_property_result" => {
            text_of(thaw_tjs_has_property_result(0, property.as_ptr()).error)
        }
        "thaw_tjs_handle_to_string" => {
            assert_eq!(text_of(thaw_tjs_handle_to_string(0)), "undefined");
            String::new()
        }
        "thaw_tjs_handle_to_console_string" => {
            assert_eq!(text_of(thaw_tjs_handle_to_console_string(0)), "undefined");
            String::new()
        }
        "thaw_tjs_handle_is_plain_data" => {
            assert_eq!(thaw_tjs_handle_is_plain_data(0), 0);
            String::new()
        }
        "thaw_tjs_handle_typed_array_probe" => {
            assert!(thaw_tjs_handle_typed_array_probe(0).is_null());
            String::new()
        }
        "thaw_tjs_resolve_handle_graph_result" => {
            text_of(thaw_tjs_resolve_handle_graph_result(0).error)
        }
        "thaw_tjs_call_handle_graph_result" => {
            text_of(thaw_tjs_call_handle_graph_result(0, args.as_ptr()).error)
        }
        "thaw_tjs_native_object_pointer" => {
            let r = thaw_tjs_native_object_pointer(0, layout.as_ptr());
            assert_eq!(r.value, 0);
            text_of(r.error)
        }
        _ => return None,
    };
    Some(error)
}

#[test]
fn table_covers_every_handle_extern() {
    let scanned = scanned_handle_externs();
    let unclassified: Vec<&String> = scanned.iter().filter(|n| policy_of(n).is_none()).collect();
    assert!(
        unclassified.is_empty(),
        "handle-taking externs without a 0 policy: {unclassified:?}"
    );
    let stale: Vec<&str> = TABLE
        .iter()
        .map(|(n, _)| *n)
        .filter(|n| !scanned.iter().any(|s| s == n))
        .collect();
    assert!(
        stale.is_empty(),
        "table names that are not handle externs: {stale:?}"
    );
}

#[test]
fn lenient_externs_treat_handle_zero_as_undefined() {
    let mut called = 0;
    for name in scanned_handle_externs() {
        if policy_of(&name) != Some(Policy::Lenient) {
            continue;
        }
        if let Some(error) = call_with_zero(&name) {
            called += 1;
            assert!(
                !error.contains("invalid JavaScript handle"),
                "{name} rejects handle 0 as invalid: {error}"
            );
        }
    }
    assert!(
        called >= 8,
        "only {called} lenient externs were called with 0"
    );
}

#[test]
fn strict_napi_externs_keep_handle_zero_invalid() {
    use crate::napi_ops::thaw_tjs_strict_equal_handles_result;
    let r = thaw_tjs_strict_equal_handles_result(0, 0);
    assert!(text_of(r.error).contains("invalid JavaScript handle"));
}
