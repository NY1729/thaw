//! Unit tests for the pure handle operations in `napi_ops.rs` and the host-query 15/21 arms.
//! Each test drives the `extern "C"` functions directly, the way the existing host tests do.

use crate::host::*;
use crate::napi_ops::*;
use std::ffi::{c_void, CStr, CString};

fn q(text: &str) -> CString {
    CString::new(text).unwrap()
}

/// Run a script in the default realm.
fn load(source: &str) {
    assert_eq!(thaw_tjs_load_script(q(source).as_ptr()), 1, "{source}");
}

/// A handle to the global `path` (dotted paths allowed).
fn global(path: &str) -> u64 {
    let handle = thaw_tjs_get_global(q(path).as_ptr());
    assert!(handle > 0, "no global {path}");
    handle as u64
}

/// The handle of property `name` of the object behind `object` (undefined values included).
fn prop(object: u64, name: &str) -> u64 {
    value(thaw_tjs_get_property_result(object, q(name).as_ptr()))
}

/// The handle of a successful result.
fn value(result: HandleResult) -> u64 {
    assert!(
        result.error.is_null(),
        "unexpected error: {}",
        error_of(result.error)
    );
    result.value
}

/// The error text of a failed result.
fn failure(result: HandleResult) -> String {
    assert!(
        !result.error.is_null(),
        "expected an error, got {}",
        result.value
    );
    error_of(result.error)
}

fn error_of(error: *mut std::ffi::c_char) -> String {
    // SAFETY: the native side hands back an owned, NUL-terminated string
    let text = unsafe { CStr::from_ptr(error) }
        .to_string_lossy()
        .into_owned();
    unsafe { thaw_arena::destroy_string(error) };
    text
}

/// A wire of one array whose elements are the live handles, in order. Decoding an `hdl` node
/// releases one reference, so each handle is retained for the wire first.
fn handle_array(handles: &[u64]) -> CString {
    for handle in handles {
        // an unknown id retains nothing and decodes as undefined
        thaw_tjs_retain_handle(*handle as i64);
    }
    let items: Vec<String> = handles
        .iter()
        .enumerate()
        .map(|(i, _)| format!(r#"{{"r":{}}}"#, i + 1))
        .collect();
    let nodes: Vec<String> = handles
        .iter()
        .map(|h| format!(r#"{{"hdl":{h}}}"#))
        .collect();
    let mut all = vec![format!(r#"{{"a":[{}]}}"#, items.join(","))];
    all.extend(nodes);
    q(&format!(
        r#"{{"root":{{"r":0}},"nodes":[{}]}}"#,
        all.join(",")
    ))
}

/// Fixtures shared by the tests: native scalars, objects, a Proxy, a Promise and a lone surrogate.
const FIXTURES: &str = "class K {}\n\
globalThis.__napi = {\
  nan: NaN, zero: 0, negZero: -0, one: 1, yes: true, hi: 'hi', s: '\u{e9}', big: 5n,\
  nil: null, undef: undefined, lone: '\\uD800', pair: '\\uD83D\\uDE00',\
  obj: {}, plain: {}, K: K, kp: K.prototype, inst: new K(), fn: function () {},\
  px: new Proxy({}, { getPrototypeOf() { return K.prototype; } }),\
  promise: Promise.resolve(1), frozen: {}, sealed: {}, np: {}, proto: { hello: 1 },\
  nullProto: Object.create(null)\
};";

#[test]
fn strict_equal_handles_follow_js_identity() {
    load(FIXTURES);
    let obj = global("__napi.obj");
    assert_eq!(
        value(thaw_tjs_strict_equal_handles_result(
            obj,
            global("__napi.obj")
        )),
        1
    );
    assert_eq!(
        value(thaw_tjs_strict_equal_handles_result(
            obj,
            global("__napi.plain")
        )),
        0
    );
    let nan = global("__napi.nan");
    assert_eq!(
        value(thaw_tjs_strict_equal_handles_result(
            nan,
            global("__napi.nan")
        )),
        0
    );
    let (zero, negative) = (global("__napi.zero"), global("__napi.negZero"));
    assert_eq!(
        value(thaw_tjs_strict_equal_handles_result(zero, negative)),
        1
    );
    assert!(failure(thaw_tjs_strict_equal_handles_result(obj, 999_999))
        .contains("invalid JavaScript handle"));
    assert!(
        failure(thaw_tjs_strict_equal_handles_result(0, obj)).contains("invalid JavaScript handle")
    );
}

#[test]
fn strict_equal_handle_scalar_compares_native_scalars() {
    load(FIXTURES);
    let scalar = |handle: u64, kind: u8, number: f64, text: &str| {
        let text = q(text);
        thaw_tjs_strict_equal_handle_scalar_result(handle, kind, number, text.as_ptr())
    };
    assert_eq!(value(scalar(global("__napi.one"), 3, 1.0, "")), 1);
    assert_eq!(value(scalar(global("__napi.one"), 3, 2.0, "")), 0);
    assert_eq!(value(scalar(global("__napi.nil"), 1, 0.0, "")), 1);
    assert_eq!(
        value(scalar(prop(global("__napi"), "undef"), 0, 0.0, "")),
        1
    );
    assert_eq!(value(scalar(global("__napi.nil"), 0, 0.0, "")), 0);
    assert_eq!(value(scalar(global("__napi.yes"), 2, 1.0, "")), 1);
    assert_eq!(value(scalar(global("__napi.s"), 4, 0.0, "\"\u{e9}\"")), 1);
    assert_eq!(value(scalar(global("__napi.big"), 5, 0.0, "5")), 1);
    assert_eq!(value(scalar(global("__napi.big"), 5, 0.0, "6")), 0);
    assert_eq!(value(scalar(global("__napi.hi"), 6, 0.0, "[104,105]")), 1);
    assert_eq!(value(scalar(global("__napi.lone"), 6, 0.0, "[55296]")), 1);
    assert!(
        failure(scalar(global("__napi.one"), 9, 0.0, "")).contains("Invalid native scalar kind")
    );
    assert!(failure(scalar(999_999, 3, 1.0, "")).contains("invalid JavaScript handle"));
}

#[test]
fn instanceof_handles_follow_the_chain_and_proxy_traps() {
    load(FIXTURES);
    let k = global("__napi.K");
    assert_eq!(
        value(thaw_tjs_instanceof_handles_result(global("__napi.inst"), k)),
        1
    );
    assert_eq!(
        value(thaw_tjs_instanceof_handles_result(global("__napi.obj"), k)),
        0
    );
    assert_eq!(
        value(thaw_tjs_instanceof_handles_result(global("__napi.px"), k)),
        1
    );
    assert_eq!(
        value(thaw_tjs_instanceof_handles_result(global("__napi.one"), k)),
        0
    );
    let object = global("__napi.inst");
    assert!(failure(thaw_tjs_instanceof_handles_result(
        object,
        global("__napi.one")
    ))
    .contains("not an object"));
    assert!(failure(thaw_tjs_instanceof_handles_result(
        object,
        global("__napi.obj")
    ))
    .contains("not callable"));
    assert!(failure(thaw_tjs_instanceof_handles_result(999_999, k))
        .contains("invalid JavaScript handle"));
}

#[test]
fn instanceof_graph_reads_an_object_and_constructor_pair() {
    load(FIXTURES);
    let (inst, obj, k) = (
        global("__napi.inst"),
        global("__napi.obj"),
        global("__napi.K"),
    );
    let pair = |a: u64, b: u64| handle_array(&[a, b]);
    assert_eq!(
        value(thaw_tjs_instanceof_graph_result(pair(inst, k).as_ptr())),
        1
    );
    assert_eq!(
        value(thaw_tjs_instanceof_graph_result(pair(obj, k).as_ptr())),
        0
    );
    assert!(failure(thaw_tjs_instanceof_graph_result(
        pair(obj, 999_999).as_ptr()
    ))
    .contains("not an object"));
}

#[test]
fn get_prototype_returns_a_handle_or_null() {
    load(FIXTURES);
    let proto = value(thaw_tjs_get_prototype_handle_result(global("__napi.inst")));
    assert_eq!(
        value(thaw_tjs_strict_equal_handles_result(
            proto,
            global("__napi.kp")
        )),
        1
    );
    let none = value(thaw_tjs_get_prototype_handle_result(global(
        "__napi.nullProto",
    )));
    assert_eq!(
        value(thaw_tjs_strict_equal_handle_scalar_result(
            none,
            1,
            0.0,
            q("").as_ptr()
        )),
        1
    );
    assert!(
        failure(thaw_tjs_get_prototype_handle_result(global("__napi.one"))).contains("non-object")
    );
    assert!(failure(thaw_tjs_get_prototype_handle_result(999_999))
        .contains("invalid JavaScript handle"));
}

#[test]
fn set_prototype_graph_changes_the_chain_once() {
    load(FIXTURES);
    let (np, proto) = (global("__napi.np"), global("__napi.proto"));
    assert_eq!(
        value(thaw_tjs_set_prototype_graph_result(
            np,
            handle_array(&[proto]).as_ptr()
        )),
        1
    );
    let hello = global("__napi.np.hello");
    assert_eq!(
        value(thaw_tjs_strict_equal_handle_scalar_result(
            hello,
            3,
            1.0,
            q("").as_ptr()
        )),
        1
    );
    // a cycle (proto -> np -> proto) is refused
    assert_eq!(
        value(thaw_tjs_set_prototype_graph_result(
            proto,
            handle_array(&[np]).as_ptr()
        )),
        0
    );
    // a sealed object is not extensible, so its prototype does not change
    let sealed = global("__napi.sealed");
    assert_eq!(value(thaw_tjs_host_object_integrity_result(sealed, 0)), 1);
    assert_eq!(
        value(thaw_tjs_set_prototype_graph_result(
            sealed,
            handle_array(&[proto]).as_ptr()
        )),
        0
    );
    assert!(failure(thaw_tjs_set_prototype_graph_result(
        999_999,
        handle_array(&[proto]).as_ptr()
    ))
    .contains("invalid JavaScript handle"));
}

#[test]
fn is_promise_names_native_promises_only() {
    load(FIXTURES);
    assert_eq!(
        value(thaw_tjs_handle_is_promise_result(global("__napi.promise"))),
        1
    );
    assert_eq!(
        value(thaw_tjs_handle_is_promise_result(global("__napi.obj"))),
        0
    );
    assert!(
        failure(thaw_tjs_handle_is_promise_result(999_999)).contains("invalid JavaScript handle")
    );
}

#[test]
fn integrity_freezes_and_seals_the_object() {
    load(FIXTURES);
    let frozen = global("__napi.frozen");
    assert_eq!(value(thaw_tjs_host_object_integrity_result(frozen, 1)), 1);
    load("__napi.frozen.x = 1; __napi.frozenX = __napi.frozen.x;");
    let frozen_x = prop(global("__napi"), "frozenX");
    assert_eq!(
        value(thaw_tjs_strict_equal_handle_scalar_result(
            frozen_x,
            0,
            0.0,
            q("").as_ptr()
        )),
        1
    );
    let sealed = global("__napi.sealed");
    assert_eq!(value(thaw_tjs_host_object_integrity_result(sealed, 0)), 1);
    load("__napi.sealed.y = 1; __napi.sealedY = __napi.sealed.y;");
    let sealed_y = prop(global("__napi"), "sealedY");
    assert_eq!(
        value(thaw_tjs_strict_equal_handle_scalar_result(
            sealed_y,
            0,
            0.0,
            q("").as_ptr()
        )),
        1
    );
    assert!(failure(thaw_tjs_host_object_integrity_result(
        global("__napi.one"),
        1
    ))
    .contains("not an object"));
    assert!(failure(thaw_tjs_host_object_integrity_result(999_999, 1))
        .contains("invalid JavaScript handle"));
}

#[test]
fn utf16_units_build_strings_and_keep_lone_surrogates() {
    load(FIXTURES);
    let hi = value(thaw_tjs_string_from_utf16_units_result(
        q("[104,105]").as_ptr(),
    ));
    assert_eq!(
        value(thaw_tjs_strict_equal_handles_result(
            hi,
            global("__napi.hi")
        )),
        1
    );
    let lone = value(thaw_tjs_string_from_utf16_units_result(
        q("[55296]").as_ptr(),
    ));
    assert_eq!(
        value(thaw_tjs_strict_equal_handles_result(
            lone,
            global("__napi.lone")
        )),
        1
    );
    assert_eq!(
        value(thaw_tjs_strict_equal_handles_result(
            lone,
            global("__napi.pair")
        )),
        0
    );
    let pair = value(thaw_tjs_string_from_utf16_units_result(
        q("[55357,56832]").as_ptr(),
    ));
    assert_eq!(
        value(thaw_tjs_strict_equal_handles_result(
            pair,
            global("__napi.pair")
        )),
        1
    );
    assert!(failure(thaw_tjs_string_from_utf16_units_result(
        q("[65536]").as_ptr()
    ))
    .contains("UTF-16"));
    assert!(failure(thaw_tjs_string_from_utf16_units_result(
        q("not json").as_ptr()
    ))
    .contains("UTF-16"));
}

/// Host query text (or the error) for `handle` and `operation`.
fn query(handle: u64, operation: u8) -> Result<String, String> {
    let result = thaw_tjs_host_query_result(handle, operation);
    if result.error.is_null() {
        let text = unsafe { CStr::from_ptr(result.value) }
            .to_string_lossy()
            .into_owned();
        unsafe { thaw_arena::destroy_string(result.value) };
        Ok(text)
    } else {
        Err(error_of(result.error))
    }
}

#[test]
fn host_query_15_gives_string_units_and_21_the_native_owner() {
    load(FIXTURES);
    assert_eq!(query(global("__napi.s"), 15).unwrap(), "[233]");
    assert_eq!(query(global("__napi.pair"), 15).unwrap(), "[55357,56832]");
    assert!(query(global("__napi.obj"), 15)
        .unwrap_err()
        .contains("Expected string"));
    assert!(query(999_999, 15)
        .unwrap_err()
        .contains("invalid JavaScript handle"));

    // an interned native wrapper answers its owner pointer in hex; a plain object has none
    let candidate = global("__napi.plain");
    // the layout of a one-field record `a` (hex segments, as the host tests build them)
    let layout = q("1:611:54");
    let wrapper =
        thaw_tjs_intern_native_object(0x1000 as *const c_void, layout.as_ptr(), candidate);
    assert!(wrapper.error.is_null());
    assert_eq!(query(wrapper.value, 21).unwrap(), "1000");
    assert_eq!(query(global("__napi.obj"), 21).unwrap(), "");
    assert!(query(999_999, 21)
        .unwrap_err()
        .contains("invalid JavaScript handle"));
}
