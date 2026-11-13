//! Every thaw-js extern that takes a JSON argument, and how its argument is serialised on the compiler side.
//!
//! A JSON argument can carry a live JsValue, which compiled code sends as a Host lease. The rule is one
//! serializer per wire family, so no lease is ever copied where it should stay live:
//!
//! - `PlainTjs`: a compiled thaw-js call argument, serialised by `thaw_json_stringify_tjs`. Each lease
//!   becomes `{"__thaw_tjs_handle__": id}`, which `to_value` maps back to the live value (host.rs).
//! - `GraphWire`: a graph wire, serialised by `thaw_json_graph_encode`. A lease becomes an `hdl` token
//!   that `graph_token` resolves.
//! - `NativeScalar`: a key or scalar wire that compiled code builds from native strings, so no lease can
//!   appear in it.
//! - `NoCompiledProducer`: called only by this crate's tests and by embedders, never by compiled code.
//! - `NapiWire`: an N-API argument wire, which is not a thaw-js JSON argument.
//!
//! The table is the contract. The test scans the extern source text for every `extern "C" fn thaw_tjs_*`
//! with a JSON parameter and fails when one is not classified, so a new JSON-argument extern cannot ship
//! without a serializer decision. The compiler-side mapping from a backend symbol to its stringifier is
//! `json_argument_stringifier` in thaw-llvm, and its test pins the `PlainTjs` names to `thaw_json_stringify_tjs`.

use std::collections::BTreeSet;

/// Which serializer and decoder a JSON-argument extern uses.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Wire {
    PlainTjs,
    GraphWire,
    NativeScalar,
    NoCompiledProducer,
    NapiWire,
}

/// Every thaw-js extern with a JSON parameter, with its wire family.
const TABLE: &[(&str, Wire)] = &[
    ("thaw_tjs_call_typed_result", Wire::PlainTjs),
    ("thaw_tjs_class_call", Wire::PlainTjs),
    ("thaw_tjs_new_function", Wire::PlainTjs),
    ("thaw_tjs_call", Wire::NoCompiledProducer),
    ("thaw_tjs_call_graph_result", Wire::GraphWire),
    ("thaw_tjs_retain_graph_result", Wire::GraphWire),
    ("thaw_tjs_call_method_graph_result", Wire::GraphWire),
    (
        "thaw_tjs_call_selected_method_graph_result",
        Wire::GraphWire,
    ),
    (
        "thaw_tjs_call_selected_method_handle_graph_args_result",
        Wire::GraphWire,
    ),
    (
        "thaw_tjs_construct_handle_graph_args_result",
        Wire::GraphWire,
    ),
    (
        "thaw_tjs_call_handle_handle_graph_args_result",
        Wire::GraphWire,
    ),
    ("thaw_tjs_call_handle_graph_result", Wire::GraphWire),
    (
        "thaw_tjs_call_handle_with_this_graph_wire_result",
        Wire::GraphWire,
    ),
    ("thaw_tjs_call_handle_mixed_graph_result", Wire::GraphWire),
    (
        "thaw_tjs_call_handle_mixed_exact_consuming_result",
        Wire::GraphWire,
    ),
    (
        "thaw_tjs_call_handle_mixed_exact_resolved_consuming_result",
        Wire::GraphWire,
    ),
    (
        "thaw_tjs_call_handle_mixed_handle_graph_args_result",
        Wire::GraphWire,
    ),
    (
        "thaw_tjs_call_handle_mixed_handle_graph_args_consuming_result",
        Wire::GraphWire,
    ),
    (
        "thaw_tjs_call_method_handle_graph_args_result",
        Wire::GraphWire,
    ),
    ("thaw_tjs_set_property_json_graph_result", Wire::GraphWire),
    ("thaw_tjs_set_property_graph_result", Wire::GraphWire),
    ("thaw_tjs_get_property_json_key_result", Wire::NativeScalar),
    (
        "thaw_tjs_property_predicate_json_key_result",
        Wire::NativeScalar,
    ),
    ("thaw_tjs_event_key_handle_result", Wire::NativeScalar),
    (
        "thaw_tjs_string_from_utf16_units_result",
        Wire::NativeScalar,
    ),
    ("thaw_tjs_call_reference", Wire::NapiWire),
    ("thaw_tjs_call_reference_graph", Wire::NapiWire),
    ("thaw_tjs_napi_call_reference_graph", Wire::NapiWire),
    (
        "thaw_tjs_call_handle_with_this_graph_result",
        Wire::NapiWire,
    ),
    (
        "thaw_tjs_napi_construct_handle_graph_args_result",
        Wire::NapiWire,
    ),
    (
        "thaw_tjs_get_property_json_key_with_exception_result",
        Wire::NapiWire,
    ),
    (
        "thaw_tjs_set_property_graph_with_exception_result",
        Wire::NapiWire,
    ),
    (
        "thaw_tjs_set_receiver_data_graph_with_exception_result",
        Wire::NapiWire,
    ),
    (
        "thaw_tjs_set_property_key_handle_graph_with_exception_result",
        Wire::NapiWire,
    ),
    (
        "thaw_tjs_define_data_property_key_handle_graph_result",
        Wire::NapiWire,
    ),
];

/// The source files the scan reads (the same set as `handle_zero_tests`).
const SOURCES: &[&str] = &[
    include_str!("host.rs"),
    include_str!("napi_eval.rs"),
    include_str!("napi_ops.rs"),
    include_str!("napi_props.rs"),
    include_str!("napi_symbols.rs"),
];

/// Whether a parameter (`name: type`) carries JSON text.
fn is_json_param(param: &str) -> bool {
    let Some((name, ty)) = param.split_once(':') else {
        return false;
    };
    let name = name.trim().trim_start_matches('_');
    ty.trim() == "*const c_char" && (name.contains("json") || name == "wire")
}

/// Every `extern "C" fn thaw_tjs_*` in `SOURCES` with a JSON parameter, by name.
fn scanned_json_externs() -> BTreeSet<String> {
    let mut names = BTreeSet::new();
    for text in SOURCES {
        let mut rest: &str = text;
        while let Some(at) = rest.find("extern \"C\" fn thaw_tjs_") {
            let after = &rest[at + "extern \"C\" fn ".len()..];
            let open = after.find('(').expect("extern parameter list");
            let close = after.find(')').expect("extern parameter list end");
            let name = &after[..open];
            if after[open + 1..close].split(',').any(is_json_param) {
                names.insert(name.to_string());
            }
            rest = &after[close..];
        }
    }
    names
}

#[test]
fn every_json_argument_extern_is_classified() {
    let scanned = scanned_json_externs();
    let listed: BTreeSet<String> = TABLE.iter().map(|(name, _)| name.to_string()).collect();
    assert_eq!(listed.len(), TABLE.len(), "duplicate table entry");
    let unclassified: Vec<_> = scanned.difference(&listed).collect();
    assert!(
        unclassified.is_empty(),
        "JSON-argument externs with no serializer classification: {unclassified:?}"
    );
    let stale: Vec<_> = listed.difference(&scanned).collect();
    assert!(
        stale.is_empty(),
        "table entries that are not JSON-argument externs: {stale:?}"
    );
}

#[test]
fn only_compiled_plain_json_arguments_take_the_marker_serializer() {
    // These three are the names `json_argument_stringifier` (thaw-llvm) maps to `thaw_json_stringify_tjs`.
    let plain: BTreeSet<&str> = TABLE
        .iter()
        .filter(|(_, wire)| *wire == Wire::PlainTjs)
        .map(|(name, _)| *name)
        .collect();
    assert_eq!(
        plain,
        BTreeSet::from([
            "thaw_tjs_call_typed_result",
            "thaw_tjs_class_call",
            "thaw_tjs_new_function"
        ])
    );
}
