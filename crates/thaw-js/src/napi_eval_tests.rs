//! Unit tests for the eval and reference natives in `napi_eval.rs`, and the `get_global` and
//! `load_script` parity they rely on. Each test drives the `extern "C"` functions directly.

use crate::host::*;
use crate::napi_eval::*;
use crate::napi_ops::{
    thaw_tjs_handle_is_promise_result, thaw_tjs_strict_equal_handle_scalar_result,
};
use std::ffi::{c_char, c_void, CStr, CString};
use std::io::Write;

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

/// Takes an owned string from a native result (`None` for a null pointer).
fn take(pointer: *const c_char) -> Option<String> {
    if pointer.is_null() {
        return None;
    }
    // SAFETY: the native side hands back an owned, NUL-terminated string, freed here once.
    let text = unsafe { CStr::from_ptr(pointer) }
        .to_string_lossy()
        .into_owned();
    unsafe { thaw_arena::destroy_string(pointer) };
    Some(text)
}

/// `thaw_tjs_eval_json` as `Ok(Some(json))`, `Ok(None)` for undefined, or `Err(error text)`.
fn eval(source: &str) -> Result<Option<String>, String> {
    let result = thaw_tjs_eval_json(q(source).as_ptr());
    match take(result.error) {
        Some(error) => Err(error),
        None => Ok(take(result.value)),
    }
}

/// `thaw_tjs_strict_equal_handle_scalar_result` against a number, as a bool.
fn is_number(handle: u64, number: f64) -> bool {
    let result = thaw_tjs_strict_equal_handle_scalar_result(handle, 3, number, std::ptr::null());
    assert!(result.error.is_null());
    result.value == 1
}

/// A wire item: a live handle (retained for the wire), or a JSON value token.
enum Item {
    Handle(u64),
    Json(&'static str),
}

/// A graph wire whose root is the array of `items`, in order.
fn wire(items: &[Item]) -> CString {
    // nodes[0] is the array; each handle is its own `hdl` node after it, JSON items are inline tokens
    let mut nodes = vec![String::new()];
    let mut refs = Vec::new();
    for item in items {
        match item {
            Item::Handle(handle) => {
                thaw_tjs_retain_handle(*handle as i64);
                nodes.push(format!(r#"{{"hdl":{handle}}}"#));
                refs.push(format!(r#"{{"r":{}}}"#, nodes.len() - 1));
            }
            Item::Json(json) => refs.push(format!(r#"{{"v":{json}}}"#)),
        }
    }
    nodes[0] = format!(r#"{{"a":[{}]}}"#, refs.join(","));
    q(&format!(
        r#"{{"root":{{"r":0}},"nodes":[{}]}}"#,
        nodes.join(",")
    ))
}

/// The `[thisArg, ...arguments]` call of `thaw_tjs_call_handle_with_this_graph_result`.
fn call_with_this(function: u64, items: &[Item]) -> ThawCallWithExceptionResult {
    let graph = wire(items);
    thaw_tjs_call_handle_with_this_graph_result(function, graph.as_ptr())
}

#[test]
fn eval_json_returns_the_completion_value_as_json() {
    assert_eq!(eval("1 + 2"), Ok(Some("3".into())));
    assert_eq!(
        eval("({ a: [1, 'x'] })"),
        Ok(Some(r#"{"a":[1,"x"]}"#.into()))
    );
}

#[test]
fn eval_json_gives_none_for_undefined_and_reports_failures() {
    assert_eq!(eval("undefined"), Ok(None));
    assert_eq!(eval("(() => 1)"), Ok(None));
    assert!(eval("throw new TypeError('bad')")
        .unwrap_err()
        .contains("bad"));
    assert!(eval("let = ;").is_err());
}

#[test]
fn call_reference_returns_json_and_marks_throws() {
    // The thaw-napi tests.rs:20 flow: an ordinary object result, then a throw.
    load("globalThis.__thaw_napi_reference_746291401 = () => ({ __thaw_error__: 'ordinary' }); globalThis.__thaw_napi_reference_746291402 = () => { throw new TypeError('callback \\u0002boom'); };");
    let text = take(unsafe { thaw_tjs_call_reference(746291401 as *mut c_void, q("[]").as_ptr()) })
        .unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(parsed["__thaw_error__"], "ordinary", "{text}");

    let text = take(unsafe { thaw_tjs_call_reference(746291402 as *mut c_void, q("[]").as_ptr()) })
        .unwrap();
    let body = text
        .strip_prefix('\u{2}')
        .expect("reference throw is marked");
    let framed: String = serde_json::from_str(body).unwrap();
    // thaw-js frames a thrown Error as `\u{1}name\u{1}message`
    assert_eq!(framed, "\u{1}TypeError\u{1}callback \u{2}boom");
}

#[test]
fn call_reference_passes_arguments_and_reports_invalid_calls() {
    load("globalThis.__thaw_napi_reference_746291403 = (a, b) => a + b; globalThis.__thaw_napi_reference_746291404 = () => undefined;");
    let text =
        take(unsafe { thaw_tjs_call_reference(746291403 as *mut c_void, q("[2,3]").as_ptr()) });
    assert_eq!(text.as_deref(), Some("5"));
    // undefined result: the marker, not null
    let text = take(unsafe { thaw_tjs_call_reference(746291404 as *mut c_void, q("[]").as_ptr()) });
    assert_eq!(text.as_deref(), Some(r#"{"$__thaw_napi_undefined$":true}"#));
    // a missing reference and a non-array argument list are marked errors
    let missing =
        take(unsafe { thaw_tjs_call_reference(746291499 as *mut c_void, q("[]").as_ptr()) })
            .unwrap();
    assert!(
        missing.starts_with('\u{2}') && missing.contains("no such function"),
        "{missing}"
    );
    let malformed =
        take(unsafe { thaw_tjs_call_reference(746291403 as *mut c_void, q("{}").as_ptr()) })
            .unwrap();
    assert!(
        malformed.starts_with('\u{2}') && malformed.contains("JSON array"),
        "{malformed}"
    );
}

#[test]
fn call_reference_graph_delivers_the_receiver_and_live_argument() {
    // The thaw-napi tests.rs:2770 flow: a live argument and receiver cross as handles.
    load("globalThis.napiLiveArgument = [42]; globalThis.napiLiveReceiver = { tag: 'receiver' }; globalThis.__thaw_napi_reference_746291405 = function(meta, receiver, argument) { return (meta === true && receiver.tag === 'receiver' && argument === globalThis.napiLiveArgument) ? 'graph-ok' : 'graph-bad'; };");
    let argument = global("napiLiveArgument");
    let receiver = global("napiLiveReceiver");
    let graph = wire(&[
        Item::Json("true"),
        Item::Handle(receiver),
        Item::Handle(argument),
    ]);
    let text =
        take(unsafe { thaw_tjs_call_reference_graph(746291405 as *mut c_void, graph.as_ptr()) })
            .unwrap();
    assert!(text.contains("graph-ok"), "{text}");
    assert!(!text.starts_with('\u{2}'));

    // invalid: a missing reference is a marked error
    let graph = wire(&[Item::Json("true")]);
    let missing =
        take(unsafe { thaw_tjs_call_reference_graph(746291499 as *mut c_void, graph.as_ptr()) })
            .unwrap();
    assert!(missing.starts_with('\u{2}'), "{missing}");
}

#[test]
fn call_handle_with_this_keeps_the_result_as_a_live_handle() {
    load("globalThis.napiThisFn = function(x) { return this.tag + x; }; globalThis.napiThisRecv = { tag: 'r' };");
    let function = global("napiThisFn");
    let receiver = global("napiThisRecv");
    let result = call_with_this(function, &[Item::Handle(receiver), Item::Json("5")]);
    assert!(result.error.is_null());
    assert_eq!(result.exception_handle, 0);
    assert_ne!(result.value, 0);
    let scalar =
        thaw_tjs_strict_equal_handle_scalar_result(result.value, 4, 0.0, q("\"r5\"").as_ptr());
    assert_eq!(scalar.value, 1);
}

#[test]
fn call_handle_with_this_does_not_assimilate_a_returned_promise() {
    load("globalThis.napiPromiseFn = function() { return Promise.resolve(7); }; globalThis.napiRecvForPromise = {};");
    let function = global("napiPromiseFn");
    let receiver = global("napiRecvForPromise");
    let result = call_with_this(function, &[Item::Handle(receiver)]);
    assert!(result.error.is_null());
    assert_ne!(result.value, 0);
    assert_eq!(thaw_tjs_handle_is_promise_result(result.value).value, 1);
}

#[test]
fn call_handle_with_this_captures_a_throw_as_an_exception_handle() {
    load("globalThis.napiThrowFn = function() { throw new TypeError('thrown here'); }; globalThis.napiRecvForThrow = {};");
    let function = global("napiThrowFn");
    let receiver = global("napiRecvForThrow");
    let result = call_with_this(function, &[Item::Handle(receiver)]);
    assert!(result.error.is_null());
    assert_eq!(result.value, 0);
    assert_ne!(result.exception_handle, 0);
    assert_eq!(result.exception_object_like, 1);
}

#[test]
fn call_handle_with_this_reports_invalid_and_undefined_calls() {
    load("globalThis.napiNotAFunction = 5; globalThis.napiRecvForInvalid = {};");
    load("globalThis.napiEmptyFn = function() {};");
    let receiver = global("napiRecvForInvalid");
    // no such handle
    let result = call_with_this(0, &[Item::Handle(receiver)]);
    assert!(!result.error.is_null());
    assert_eq!(
        take(result.error).as_deref(),
        Some("invalid JavaScript handle")
    );
    // not callable
    let number = global("napiNotAFunction");
    let result = call_with_this(number, &[Item::Handle(receiver)]);
    assert!(!result.error.is_null());
    take(result.error);
    // an empty payload has no receiver
    let graph = q(r#"{"root":{"r":0},"nodes":[{"a":[]}]}"#);
    let empty_fn = global("napiEmptyFn");
    let result = thaw_tjs_call_handle_with_this_graph_result(empty_fn, graph.as_ptr());
    assert_eq!(
        take(result.error).as_deref(),
        Some("bad bundle: JS callback receiver payload is empty")
    );
}

#[test]
fn graph_native_pair_matches_asks_the_prelude_check() {
    load("globalThis.napiPairValue = {}; globalThis.__thaw_json_graph_native_pair_matches = (value, id, kind) => value === globalThis.napiPairValue && id === '42' && kind === 1;");
    let value = global("napiPairValue");
    assert_eq!(
        thaw_tjs_graph_native_pair_matches_result(value, 42, 1).value,
        1
    );
    assert_eq!(
        thaw_tjs_graph_native_pair_matches_result(value, 43, 1).value,
        0
    );
    assert_eq!(
        thaw_tjs_graph_native_pair_matches_result(value, 42, 2).value,
        0
    );
    // invalid: no such handle
    let result = thaw_tjs_graph_native_pair_matches_result(0, 42, 1);
    assert!(!result.error.is_null());
    take(result.error);
}

#[test]
fn graph_native_pair_without_the_prelude_check_is_an_error() {
    load("globalThis.napiPairNoCheck = {};");
    let value = global("napiPairNoCheck");
    let result = thaw_tjs_graph_native_pair_matches_result(value, 42, 1);
    assert!(!result.error.is_null());
    take(result.error);
}

#[test]
fn get_global_prefers_a_literal_dotted_global_then_walks_the_path() {
    load("globalThis['napiShadow.x'] = 1; globalThis.napiShadow = { x: 2 }; globalThis.napiNs = { k: 9 };");
    assert!(is_number(global("napiShadow.x"), 1.0));
    assert!(is_number(global("napiNs.k"), 9.0));
    // invalid: nothing by either spelling
    assert_eq!(thaw_tjs_get_global(q("napiMissing.none").as_ptr()), 0);
    assert_eq!(thaw_tjs_get_global(q("no.such.thing").as_ptr()), 0);
}

#[test]
fn load_script_accepts_a_gz_base64_payload() {
    use base64::Engine;
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    encoder.write_all(b"globalThis.napiGzValue = 13").unwrap();
    let packed = base64::engine::general_purpose::STANDARD.encode(encoder.finish().unwrap());
    load(&format!("gz:{packed}"));
    assert!(is_number(global("napiGzValue"), 13.0));
    // invalid: not base64 of gzip, so the text itself is loaded and fails to parse
    assert_eq!(thaw_tjs_load_script(q("gz:!!!").as_ptr()), 0);
}
