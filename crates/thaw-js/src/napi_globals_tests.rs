//! Tests of the JS-visible N-API graph globals (`__thaw_json_graph_encode_js`, `__thaw_json_graph_decode`,
//! `__thaw_json_graph_decode_owned`) installed by `napi_bridge::install`, and of the natives switched to
//! the N-API wire rules (PR 6a-5). Probes run in the default realm.

use std::ffi::{c_char, c_void, CStr, CString};

use crate::host::*;
use crate::napi_eval::{
    thaw_tjs_call_handle_with_this_graph_result, thaw_tjs_eval_json,
    thaw_tjs_napi_call_reference_graph,
};
use crate::napi_graph::decode_napi_wire;
use crate::napi_ops::{
    handle_value, thaw_tjs_instanceof_graph_result,
    thaw_tjs_napi_construct_handle_graph_args_result,
};
use crate::napi_props::thaw_tjs_set_property_graph_with_exception_result;
use crate::Value;

fn q(text: &str) -> CString {
    CString::new(text).unwrap()
}

fn load(source: &str) {
    assert_eq!(thaw_tjs_load_script(q(source).as_ptr()), 1, "{source}");
}

/// Takes ownership of a native string (null is the empty string).
///
/// # Safety
/// `pointer` must be null or a string the thaw-js arena owns.
unsafe fn take(pointer: *const c_char) -> String {
    if pointer.is_null() {
        return String::new();
    }
    let text = CStr::from_ptr(pointer).to_string_lossy().into_owned();
    thaw_arena::destroy_string(pointer);
    text
}

/// The JSON text of `source`'s completion value.
fn eval(source: &str) -> String {
    let result = thaw_tjs_eval_json(q(source).as_ptr());
    assert!(result.error.is_null(), "{source}");
    // SAFETY: the value is an arena string this call owns.
    unsafe { take(result.value) }
}

/// `[this, {"su"}]`-style wire: one array node holding `items` (JSON tokens), no leases.
fn array_wire(items: &str) -> CString {
    q(&format!(
        r#"{{"root":{{"r":0}},"nodes":[{{"a":[{items}]}}],"leases":[],"napiLeases":[]}}"#
    ))
}

#[test]
fn encode_global_live_keeps_objects_behind_leases() {
    assert_eq!(
        eval(
            "(() => { const o = { x: 1 }; const g = JSON.parse(__thaw_json_graph_encode_js([o], 0, true)); return g.leases.length === 1 && g.nodes.some(n => n.hdl !== undefined) && g.napiLeases.length === 0; })()"
        ),
        "true"
    );
}

#[test]
fn encode_global_plain_copies_without_leases() {
    assert_eq!(
        eval(
            "(() => { const g = JSON.parse(__thaw_json_graph_encode_js({ x: 1 }, 0, false, true)); return g.leases.length === 0 && JSON.stringify(g.nodes) === '[{\"o\":[[\"x\",{\"v\":1},7]]}]'; })()"
        ),
        "true"
    );
}

#[test]
fn decode_owned_global_restores_a_live_object_by_identity() {
    assert_eq!(
        eval(
            "(() => { const o = { x: 1 }; const back = __thaw_json_graph_decode_owned(__thaw_json_graph_encode_js([o], 0, true)); return back[0] === o; })()"
        ),
        "true"
    );
}

#[test]
fn decode_global_restores_a_copied_cycle() {
    assert_eq!(
        eval(
            "(() => { const o = { x: 1 }; o.self = o; const r = __thaw_json_graph_decode(JSON.parse(__thaw_json_graph_encode_js([o], 0, false)))[0]; return r !== o && r.self === r && r.x === 1; })()"
        ),
        "true"
    );
}

#[test]
fn decode_global_keeps_non_enumerable_flags() {
    assert_eq!(
        eval(
            "(() => { const o = {}; Object.defineProperty(o, 'h', { value: 2, enumerable: false, writable: true, configurable: true }); const r = __thaw_json_graph_decode(JSON.parse(__thaw_json_graph_encode_js(o, 0, false))); const d = Object.getOwnPropertyDescriptor(r, 'h'); return d.value === 2 && d.enumerable === false && Object.keys(r).length === 0; })()"
        ),
        "true"
    );
}

#[test]
fn decode_owned_global_rejects_a_malformed_wire_with_a_type_error() {
    assert_eq!(
        eval(
            "(() => { try { __thaw_json_graph_decode_owned('{\"nodes\":[]}'); return false; } catch (e) { return e instanceof TypeError; } })()"
        ),
        "true"
    );
}

#[test]
fn graph_globals_are_immutable_to_user_code() {
    assert_eq!(
        eval(
            "(() => { const before = __thaw_json_graph_decode; try { __thaw_json_graph_decode = () => 1; } catch (e) { /* refused */ } return __thaw_json_graph_decode === before; })()"
        ),
        "true"
    );
}

#[test]
fn owner_of_handle_alias_is_still_installed() {
    assert_eq!(
        eval("typeof __thaw_json_graph_owner_of_handle === 'function'"),
        "true"
    );
}

#[test]
fn call_with_this_decodes_the_napi_wire_lone_surrogate() {
    load("globalThis.napiSurrogateFn = function(x) { return x.length; }; globalThis.napiSurrogateRecv = { tag: 'r' };");
    let function = thaw_tjs_get_global(q("napiSurrogateFn").as_ptr()) as u64;
    let wire = array_wire(r#"{"v":null},{"su":[55296]}"#);
    assert!(
        decode_wire(wire.to_str().unwrap()).is_err(),
        "the plain wire rejects the surrogate token"
    );
    let result = thaw_tjs_call_handle_with_this_graph_result(function, wire.as_ptr());
    assert!(result.error.is_null());
    assert_eq!(result.exception_handle, 0);
    assert!(matches!(handle_value(result.value), Ok(Value::Number(n)) if n == 1.0));
}

#[test]
fn napi_call_reference_graph_answers_a_live_napi_wire() {
    load("globalThis.__thaw_napi_reference_746291777 = function(meta, s) { return s.length === 1 ? { ok: meta } : 'bad'; };");
    let wire = array_wire(r#"{"v":true},{"su":[55296]}"#);
    // SAFETY: the context names a reference that is installed above; the wire is a live C string.
    let text = unsafe {
        take(thaw_tjs_napi_call_reference_graph(
            746291777 as *mut c_void,
            wire.as_ptr(),
        ))
    };
    assert!(!text.starts_with('\u{2}'), "{text}");
    // The root array is copied (its `ok` object is a copy too), so the wire holds no handle.
    let parsed: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(parsed["napiLeases"], serde_json::json!([]));
    assert_eq!(parsed["leases"], serde_json::json!([]), "{text}");
    assert!(parsed["nodes"][0].get("o").is_some(), "{text}");
    let (_, value) = decode_napi_wire(&text).unwrap();
    assert!(matches!(value, Value::Object(_)));
}

#[test]
fn napi_call_reference_graph_marks_a_missing_reference() {
    let wire = array_wire(r#"{"v":true}"#);
    // SAFETY: as above; no reference with this context exists.
    let text = unsafe {
        take(thaw_tjs_napi_call_reference_graph(
            746291791 as *mut c_void,
            wire.as_ptr(),
        ))
    };
    assert!(text.starts_with('\u{2}'), "{text}");
}

#[test]
fn napi_construct_decodes_the_napi_wire_lone_surrogate() {
    load("globalThis.NapiBoxGlobal = function Box(x) { this.len = x.length; };");
    let ctor = thaw_tjs_get_global(q("NapiBoxGlobal").as_ptr()) as u64;
    let wire = array_wire(r#"{"su":[55296]}"#);
    assert!(
        !thaw_tjs_construct_handle_graph_args_result(ctor, wire.as_ptr())
            .error
            .is_null(),
        "the plain wire rejects the surrogate token"
    );
    let result = thaw_tjs_napi_construct_handle_graph_args_result(ctor, wire.as_ptr());
    assert!(result.error.is_null());
    let boxed = handle_value(result.value).unwrap();
    let Value::Object(_) = &boxed else {
        panic!("not an object")
    };
    let len = with_package(default_realm(), |p| {
        let it = &mut p.interp;
        it.get(&boxed, "len").map_err(|c| classify(it, c))
    })
    .unwrap();
    assert!(matches!(len, Value::Number(n) if n == 1.0));
}

#[test]
fn instanceof_decodes_the_napi_wire_lone_surrogate() {
    let object_ctor = thaw_tjs_get_global(q("Object").as_ptr()) as u64;
    assert_eq!(thaw_tjs_retain_handle(object_ctor as i64), 1);
    let wire = q(&format!(
        r#"{{"root":{{"r":0}},"nodes":[{{"a":[{{"su":[55296]}},{{"r":1}}]}},{{"hdl":{object_ctor}}}],"leases":[{object_ctor}],"napiLeases":[]}}"#
    ));
    let result = thaw_tjs_instanceof_graph_result(wire.as_ptr());
    assert!(result.error.is_null());
    assert_eq!(result.value, 0, "a string is not an Object");
}

#[test]
fn set_property_graph_writes_the_napi_wire_lone_surrogate() {
    load("globalThis.NapiSetTarget = {};");
    let target = thaw_tjs_get_global(q("NapiSetTarget").as_ptr()) as u64;
    let wire = q(r#"{"root":{"su":[55296]},"nodes":[],"leases":[],"napiLeases":[]}"#);
    let result = thaw_tjs_set_property_graph_with_exception_result(
        target,
        q("\"k\"").as_ptr(),
        wire.as_ptr(),
    );
    assert!(result.error.is_null());
    assert_eq!(eval("NapiSetTarget.k.length === 1"), "true");
}
