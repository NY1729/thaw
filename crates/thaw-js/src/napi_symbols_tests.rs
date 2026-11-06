//! Unit tests for the Symbol identity natives in `napi_symbols.rs`, the `nsy` wire token and the
//! Symbol key-text path. Each test drives the `extern "C"` functions directly, as the host tests do.

use crate::host::*;
use crate::napi_ops::{handle_value, thaw_tjs_strict_equal_handles_result};
use crate::napi_symbols::*;
use std::ffi::CString;

fn q(text: &str) -> CString {
    CString::new(text).unwrap()
}

/// Run a script in the default realm.
fn load(source: &str) {
    assert_eq!(thaw_tjs_load_script(q(source).as_ptr()), 1, "{source}");
}

/// A handle to the global `path`.
fn global(path: &str) -> u64 {
    let handle = thaw_tjs_get_global(q(path).as_ptr());
    assert!(handle > 0, "no global {path}");
    handle as u64
}

/// The value of a successful handle result.
fn ok(result: HandleResult) -> u64 {
    assert!(result.error.is_null(), "unexpected error");
    result.value
}

/// Whether two handles hold the same value (`===`).
fn same(left: u64, right: u64) -> bool {
    ok(thaw_tjs_strict_equal_handles_result(left, right)) == 1
}

/// The graph wire of the value behind `handle`, encoded as a live-handle graph.
fn wire_of(owner: usize, handle: u64) -> String {
    with_package(owner, |p| {
        let it = &mut p.interp;
        let v = handle_value(handle).unwrap();
        let mut out = WireOut {
            owner,
            plain: false,
            live_functions: true,
            copy_root: false,
            receiver: None,
            nodes: Vec::new(),
            seen: Vec::new(),
            leases: Vec::new(),
        };
        let root = out.token(it, &v)?;
        Ok(out.finish(root))
    })
    .map_err(|e| e.to_string())
    .unwrap()
}

#[test]
fn register_is_stable_and_pin_controls_liveness() {
    load("globalThis.napiSym = Symbol('napi');");
    let owner = default_realm() as u64;
    let h = global("napiSym");
    assert_eq!(thaw_tjs_napi_handle_is_symbol(h), 1);
    assert_eq!(thaw_tjs_napi_handle_is_symbol(global("Object")), 0);
    let id = thaw_tjs_napi_graph_symbol_register(owner, h);
    assert_ne!(id, 0);
    assert_eq!(thaw_tjs_napi_graph_symbol_register(owner, h), id);
    assert_eq!(
        thaw_tjs_napi_graph_symbol_register(owner, global("Object")),
        0
    );
    assert_eq!(thaw_tjs_napi_symbol_weak_live(owner, id), 1);
    assert_eq!(thaw_tjs_napi_symbol_set_pin(owner, id, 1), 1);
    assert_eq!(thaw_tjs_napi_symbol_weak_live(owner, id), 1);
    // ponytail: rt.symbols keeps every Symbol alive, so unpinning cannot make it dead yet.
    assert_eq!(thaw_tjs_napi_symbol_set_pin(owner, id, 0), 1);
    assert_eq!(thaw_tjs_napi_symbol_weak_live(owner, id), 1);
}

#[test]
fn unknown_ids_and_owners_are_unknown_not_dead() {
    let owner = default_realm() as u64;
    assert_eq!(thaw_tjs_napi_symbol_weak_live(999, 1), 2);
    assert_eq!(thaw_tjs_napi_symbol_weak_live(owner, 999_999), 0);
    assert_eq!(thaw_tjs_napi_symbol_set_pin(owner, 999_999, 1), 0);
    assert_eq!(thaw_tjs_napi_handle_is_symbol(0), 2);
    assert_eq!(
        thaw_tjs_napi_graph_symbol_register(999, global("Object")),
        0
    );
}

#[test]
fn native_symbol_handle_is_created_once_per_native_id() {
    let a = ok(thaw_tjs_native_symbol_handle_result(4242));
    let b = ok(thaw_tjs_native_symbol_handle_result(4242));
    assert_eq!(thaw_tjs_napi_handle_is_symbol(a), 1);
    assert!(same(a, b));
    let owner = default_realm() as u64;
    assert_eq!(thaw_tjs_napi_symbol_weak_live(owner, 4242), 1);
    assert_eq!(thaw_tjs_napi_symbol_set_pin(owner, 4242, 1), 1);
}

#[test]
fn key_text_resolves_unique_and_well_known_symbols() {
    load("globalThis.keySym = Symbol('k');");
    let h = global("keySym");
    let key = symbol_key(&handle_value(h).unwrap()).unwrap();
    let id = key.trim_start_matches("@@sym:");
    let unique = format!("[\"\\u0003{id}\"]");
    assert!(same(
        ok(thaw_tjs_event_key_handle_result(q(&unique).as_ptr(), 1)),
        h
    ));
    let well_known = q("[\"\\u001f@@iterator\"]");
    let found = ok(thaw_tjs_event_key_handle_result(well_known.as_ptr(), 1));
    assert_eq!(thaw_tjs_napi_handle_is_symbol(found), 1);
    let bad = q("[\"plain\"]");
    let failed = thaw_tjs_event_key_handle_result(bad.as_ptr(), 1);
    assert!(!failed.error.is_null());
}

#[test]
fn registered_symbol_round_trips_through_the_wire_by_identity() {
    load("globalThis.wireSym = Symbol('wire');");
    let owner = default_realm();
    let h = global("wireSym");
    let id = thaw_tjs_napi_graph_symbol_register(owner as u64, h);
    let wire = wire_of(owner, h);
    assert!(wire.contains(&format!("\"nsy\":\"{id}\"")), "{wire}");
    let (_, decoded) = decode_wire(&wire).unwrap();
    assert_eq!(symbol_key(&decoded), symbol_key(&handle_value(h).unwrap()));
}

#[test]
fn unregistered_symbol_keeps_the_plain_handle_token() {
    load("globalThis.plainSym = Symbol('plain');");
    let owner = default_realm();
    let wire = wire_of(owner, global("plainSym"));
    assert!(!wire.contains("nsy"), "{wire}");
    assert!(wire.contains("\"hdl\""), "{wire}");
}

#[test]
fn graph_object_symbol_key_decodes_to_the_symbol_property() {
    load("globalThis.keyedSym = Symbol('key');");
    let owner = default_realm();
    let h = global("keyedSym");
    let id = thaw_tjs_napi_graph_symbol_register(owner as u64, h);
    let key = symbol_key(&handle_value(h).unwrap()).unwrap();
    // the decoder releases one reference per `hdl`, so retain it for the wire first
    thaw_tjs_retain_handle(h as i64);
    let wire = format!(
        r#"{{"root":{{"r":0}},"nodes":[{{"o":[[{{"nsy":"{id}","hdl":{h}}},{{"v":3}},7]]}}],"leases":[],"napiLeases":[]}}"#
    );
    let (_, object) = decode_wire(&wire).unwrap();
    let got = with_package(owner, |p| {
        let read = p.interp.get(&object, &key);
        read.map_err(|c| classify(&mut p.interp, c))
    })
    .unwrap();
    let crate::Value::Number(n) = got else {
        panic!("the symbol-keyed property is not a number");
    };
    assert_eq!(n, 3.0);
}

#[test]
fn symbol_string_coercion_is_a_type_error_exception() {
    ok(thaw_tjs_native_symbol_handle_result(5150));
    let owner = default_realm() as u64;
    let r = thaw_tjs_napi_symbol_abstract_to_string_units_result(owner, 5150);
    assert!(r.error.is_null());
    assert!(r.units.is_null());
    assert_ne!(r.exception_handle, 0);
    let unknown = thaw_tjs_napi_symbol_abstract_to_string_units_result(owner, 6161);
    assert!(!unknown.error.is_null());
}
