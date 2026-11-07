//! Unit tests for the exception-capturing property operations in `napi_props.rs`. Each test drives
//! the `extern "C"` functions directly, the way the existing host tests do.

use crate::host::*;
use crate::napi_ops::*;
use crate::napi_props::*;
use std::ffi::{c_char, CStr, CString};

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

fn error_of(error: *mut c_char) -> String {
    // SAFETY: the native side hands back an owned, NUL-terminated string
    let text = unsafe { CStr::from_ptr(error) }
        .to_string_lossy()
        .into_owned();
    unsafe { thaw_arena::destroy_string(error) };
    text
}

/// A successful exception-capturing read or write: its value.
fn ok_value(result: ThawCallWithExceptionResult) -> u64 {
    assert!(
        result.error.is_null(),
        "unexpected error: {}",
        error_of(result.error)
    );
    assert_eq!(result.exception_handle, 0, "unexpected exception");
    result.value
}

/// The `(handle, object_like)` of a thrown value.
fn thrown(result: ThawCallWithExceptionResult) -> (u64, bool) {
    assert!(
        result.error.is_null(),
        "unexpected error: {}",
        error_of(result.error)
    );
    assert_ne!(result.exception_handle, 0, "expected an exception");
    (result.exception_handle, result.exception_object_like != 0)
}

/// The error text of a failed exception-capturing call.
fn call_failure(result: ThawCallWithExceptionResult) -> String {
    assert!(
        !result.error.is_null(),
        "expected an error, got {}",
        result.value
    );
    error_of(result.error)
}

/// Whether two handles hold the same value (`===`).
fn same(left: u64, right: u64) -> bool {
    value(thaw_tjs_strict_equal_handles_result(left, right)) == 1
}

/// Whether the handle holds the number `n`.
fn is_number(handle: u64, n: f64) -> bool {
    value(thaw_tjs_strict_equal_handle_scalar_result(
        handle,
        3,
        n,
        std::ptr::null(),
    )) == 1
}

/// Whether the handle holds the string `text` (compared as its JSON text).
fn is_string(handle: u64, text: &str) -> bool {
    let json = q(&serde_json::to_string(text).unwrap());
    value(thaw_tjs_strict_equal_handle_scalar_result(
        handle,
        4,
        0.0,
        json.as_ptr(),
    )) == 1
}

/// A handle to the string `text`, made by a script.
fn string_handle(text: &str) -> u64 {
    let name = format!("__strkey_{}", text.len());
    load(&format!(
        "globalThis.{name} = {};",
        serde_json::to_string(text).unwrap()
    ));
    global(&name)
}

/// A graph wire of the number `n`, as the napi side builds it for a write.
fn number_wire(n: f64) -> CString {
    q(&format!(r#"{{"root":{{"v":{n}}},"nodes":[]}}"#))
}

/// A JSON key text for `text`.
fn key(text: &str) -> CString {
    q(&serde_json::to_string(text).unwrap())
}

/// The text of a host-query operation on a handle.
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

/// The error text of a failed key-list call.
fn names_failure(result: ThawResult) -> String {
    assert!(!result.error.is_null(), "expected an error");
    error_of(result.error)
}

/// The own-key list of `object` as a JSON array (`query` operation 8), via a graph wire.
fn names(object: u64, mode: i32, filter: u32, conversion: i32) -> Result<String, String> {
    let result = thaw_tjs_host_property_names_result(object, mode, filter, conversion);
    if !result.error.is_null() {
        return Err(error_of(result.error));
    }
    let list = value(thaw_tjs_retain_graph_result(result.value));
    query(list, 8)
}

#[test]
fn read_keeps_the_identity_of_a_thrown_value() {
    load(
        "globalThis.__wtag = { tag: 'thrown' };\
         globalThis.__t = { get ok() { return 42; }, get thrower() { throw globalThis.__wtag; },\
           get prim() { throw 'plain'; } };\
         globalThis.__tp = new Proxy({}, { get() { throw globalThis.__wtag; } });",
    );
    let object = global("__t");
    assert!(is_number(
        ok_value(thaw_tjs_get_property_json_key_with_exception_result(
            object,
            key("ok").as_ptr()
        )),
        42.0
    ));
    let (exception, like) = thrown(thaw_tjs_get_property_json_key_with_exception_result(
        object,
        key("thrower").as_ptr(),
    ));
    assert!(like);
    assert!(same(exception, global("__wtag")));

    let (primitive, like) = thrown(thaw_tjs_get_property_json_key_with_exception_result(
        object,
        key("prim").as_ptr(),
    ));
    assert!(!like);
    assert!(is_string(primitive, "plain"));

    // a Proxy get trap that throws keeps its thrown value too
    let (proxied, like) = thrown(thaw_tjs_get_property_json_key_with_exception_result(
        global("__tp"),
        key("x").as_ptr(),
    ));
    assert!(like);
    assert!(same(proxied, global("__wtag")));

    // a stale object handle and a key that is neither a string nor a number are text errors
    assert!(
        call_failure(thaw_tjs_get_property_json_key_with_exception_result(
            999_999,
            key("ok").as_ptr()
        ))
        .contains("invalid JavaScript handle")
    );
    assert!(
        call_failure(thaw_tjs_get_property_json_key_with_exception_result(
            object,
            q("[1]").as_ptr()
        ))
        .contains("property keys")
    );
}

#[test]
fn key_handles_hold_a_string_or_a_symbol() {
    load("globalThis.__k = { s: 'x' }; globalThis.__ks = Symbol('k'); __k[__ks] = 'sym-value';");
    let object = global("__k");
    let symbol = global("__ks");
    assert!(is_string(
        ok_value(thaw_tjs_get_property_key_handle_with_exception_result(
            object, symbol
        )),
        "sym-value"
    ));
    assert!(is_string(
        ok_value(thaw_tjs_get_property_key_handle_with_exception_result(
            object,
            string_handle("s")
        )),
        "x"
    ));
    assert!(
        call_failure(thaw_tjs_get_property_key_handle_with_exception_result(
            object, 999_999
        ))
        .contains("invalid JavaScript handle")
    );
    assert!(
        call_failure(thaw_tjs_get_property_key_handle_with_exception_result(
            999_999, symbol
        ))
        .contains("invalid JavaScript handle")
    );
}

#[test]
fn predicate_by_key_handle_covers_has_own_and_delete() {
    load(
        "globalThis.__pr = Object.create({ inherited: 1 }); __pr.own = 2;\
         globalThis.__psym = Symbol('p'); __pr[__psym] = 3;",
    );
    let object = global("__pr");
    let inherited = string_handle("inherited");
    let own = string_handle("own");
    let symbol = global("__psym");
    assert_eq!(
        value(thaw_tjs_property_predicate_key_handle_result(
            object, inherited, 0
        )),
        1
    );
    assert_eq!(
        value(thaw_tjs_property_predicate_key_handle_result(
            object, inherited, 1
        )),
        0
    );
    assert_eq!(
        value(thaw_tjs_property_predicate_key_handle_result(
            object, own, 1
        )),
        1
    );
    assert_eq!(
        value(thaw_tjs_property_predicate_key_handle_result(
            object, symbol, 0
        )),
        1
    );
    assert_eq!(
        value(thaw_tjs_property_predicate_key_handle_result(
            object, symbol, 2
        )),
        1
    );
    assert_eq!(
        value(thaw_tjs_property_predicate_key_handle_result(
            object, symbol, 0
        )),
        0
    );
    assert!(failure(thaw_tjs_property_predicate_key_handle_result(
        999_999, own, 0
    ))
    .contains("invalid JavaScript handle"));
    assert!(failure(thaw_tjs_property_predicate_key_handle_result(
        string_handle("s"),
        own,
        0
    ))
    .contains("Reflect.has called on non-object"));
}

#[test]
fn set_writes_and_reports_refused_writes() {
    load(
        "globalThis.__wtag = { tag: 'set' };\
         globalThis.__w = { plain: 0, get acc() { return 1; } };\
         globalThis.__wf = Object.freeze({ a: 1 });\
         globalThis.__wro = {}; Object.defineProperty(__wro, 'ro', { value: 1, enumerable: true });\
         globalThis.__wthrow = { set bad(v) { throw globalThis.__wtag; } };",
    );
    let object = global("__w");
    assert_eq!(
        ok_value(thaw_tjs_set_property_graph_with_exception_result(
            object,
            key("plain").as_ptr(),
            number_wire(5.0).as_ptr()
        )),
        1
    );
    assert!(is_number(
        value(thaw_tjs_get_property_result(object, q("plain").as_ptr())),
        5.0
    ));

    // a frozen object and a read-only property refuse the write as a TypeError text
    let refused = call_failure(thaw_tjs_set_property_graph_with_exception_result(
        global("__wf"),
        key("a").as_ptr(),
        number_wire(2.0).as_ptr(),
    ));
    assert!(refused.contains("read only property 'a'"), "{refused}");
    assert!(
        call_failure(thaw_tjs_set_property_graph_with_exception_result(
            global("__wro"),
            key("ro").as_ptr(),
            number_wire(2.0).as_ptr()
        ))
        .contains("read only property 'ro'")
    );

    // a setter that throws keeps the thrown value's identity
    let (exception, like) = thrown(thaw_tjs_set_property_graph_with_exception_result(
        global("__wthrow"),
        key("bad").as_ptr(),
        number_wire(1.0).as_ptr(),
    ));
    assert!(like);
    assert!(same(exception, global("__wtag")));

    // a getter-only accessor refuses the write
    assert!(
        call_failure(thaw_tjs_set_property_graph_with_exception_result(
            object,
            key("acc").as_ptr(),
            number_wire(3.0).as_ptr()
        ))
        .contains("read only property 'acc'")
    );
    assert!(
        call_failure(thaw_tjs_set_property_graph_with_exception_result(
            999_999,
            key("plain").as_ptr(),
            number_wire(1.0).as_ptr()
        ))
        .contains("invalid JavaScript handle")
    );
}

#[test]
fn receiver_data_writes_only_own_writable_data() {
    load(
        "globalThis.__rd = { own: 1 };\
         Object.defineProperty(__rd, 'acc', { get() { return 1; }, set(v) { globalThis.__rdHit = v; }, enumerable: true });\
         globalThis.__rdInh = Object.create({ set inh(v) { globalThis.__rdHit = 'inherited'; } });\
         globalThis.__rdHit = 'none';\
         globalThis.__rdFrozen = Object.freeze({ f: 1 });",
    );
    let object = global("__rd");
    assert_eq!(
        ok_value(thaw_tjs_set_receiver_data_graph_with_exception_result(
            object,
            key("own").as_ptr(),
            number_wire(9.0).as_ptr()
        )),
        1
    );
    assert!(is_number(
        value(thaw_tjs_get_property_result(object, q("own").as_ptr())),
        9.0
    ));
    // an own accessor is not invoked and the write is refused without an error
    assert_eq!(
        ok_value(thaw_tjs_set_receiver_data_graph_with_exception_result(
            object,
            key("acc").as_ptr(),
            number_wire(5.0).as_ptr()
        )),
        0
    );
    // an inherited setter does not run: the receiver gets its own property instead
    assert_eq!(
        ok_value(thaw_tjs_set_receiver_data_graph_with_exception_result(
            global("__rdInh"),
            key("inh").as_ptr(),
            number_wire(5.0).as_ptr()
        )),
        1
    );
    assert!(is_string(global("__rdHit"), "none"));
    assert_eq!(
        ok_value(thaw_tjs_set_receiver_data_graph_with_exception_result(
            global("__rdFrozen"),
            key("f").as_ptr(),
            number_wire(5.0).as_ptr()
        )),
        0
    );
}

#[test]
fn set_by_key_handle_and_through_a_proxy_target() {
    load(
        "globalThis.__sk = {}; globalThis.__skSym = Symbol('s');\
         globalThis.__px = new Proxy({}, { set(t, k, v) { t.trapped = v; return true; } });\
         globalThis.__pxNo = new Proxy({}, { set() { return false; } });\
         globalThis.__wtag = { tag: 'proxy' };\
         globalThis.__pxThrow = new Proxy({}, { set() { throw globalThis.__wtag; } });",
    );
    let object = global("__sk");
    let symbol = global("__skSym");
    assert_eq!(
        ok_value(
            thaw_tjs_set_property_key_handle_graph_with_exception_result(
                object,
                symbol,
                number_wire(11.0).as_ptr(),
                0
            )
        ),
        1
    );
    assert!(is_number(
        ok_value(thaw_tjs_get_property_key_handle_with_exception_result(
            object, symbol
        )),
        11.0
    ));

    // a set trap that returns true writes through; one that returns false refuses
    assert_eq!(
        ok_value(thaw_tjs_set_property_graph_with_exception_result(
            global("__px"),
            key("k").as_ptr(),
            number_wire(4.0).as_ptr()
        )),
        1
    );
    assert!(is_number(
        value(thaw_tjs_get_property_result(
            global("__px"),
            q("trapped").as_ptr()
        )),
        4.0
    ));
    assert!(
        call_failure(thaw_tjs_set_property_graph_with_exception_result(
            global("__pxNo"),
            key("k").as_ptr(),
            number_wire(4.0).as_ptr()
        ))
        .contains("read only property")
    );
    let (exception, like) = thrown(thaw_tjs_set_property_graph_with_exception_result(
        global("__pxThrow"),
        key("k").as_ptr(),
        number_wire(4.0).as_ptr(),
    ));
    assert!(like);
    assert!(same(exception, global("__wtag")));
}

#[test]
fn define_data_property_by_key_handle_and_refusals() {
    load(
        "globalThis.__dd = {}; globalThis.__dk = Symbol('d'); globalThis.__dhi = 'hi';\
         globalThis.__dfz = Object.freeze({}); globalThis.__dpe = Object.preventExtensions({});",
    );
    let object = global("__dd");
    let symbol = global("__dk");
    let wire = number_wire(6.0);
    assert_eq!(
        value(thaw_tjs_define_data_property_key_handle_graph_result(
            object,
            symbol,
            wire.as_ptr(),
            7
        )),
        1
    );
    assert!(is_number(
        ok_value(thaw_tjs_get_property_key_handle_with_exception_result(
            object, symbol
        )),
        6.0
    ));
    assert!(
        failure(thaw_tjs_define_data_property_key_handle_graph_result(
            global("__dfz"),
            global("__dhi"),
            wire.as_ptr(),
            7
        ))
        .contains("not extensible")
    );
    assert!(
        failure(thaw_tjs_define_data_property_key_handle_graph_result(
            global("__dpe"),
            global("__dhi"),
            wire.as_ptr(),
            7
        ))
        .contains("not extensible")
    );
    assert!(
        failure(thaw_tjs_define_data_property_key_handle_graph_result(
            999_999,
            symbol,
            wire.as_ptr(),
            7
        ))
        .contains("invalid JavaScript handle")
    );
}

/// A native callback bridge that answers id 7 with the graph number 42 and any other id with an
/// error, the answer shape the napi side gives.
unsafe extern "C" fn fake_bridge(
    _operation: *const c_char,
    target: *const c_char,
    _empty: *const c_char,
    _graph: *const c_char,
) -> *const c_char {
    if CStr::from_ptr(target).to_bytes() == b"7" {
        c"{\"kind\":\"graph\",\"value\":\"{\\\"root\\\":{\\\"v\\\":42},\\\"nodes\\\":[]}\"}"
            .as_ptr()
    } else {
        c"{\"__thaw_error__\":\"bad id\"}".as_ptr()
    }
}

#[test]
fn callback_accessor_is_defined_and_read_through_the_bridge() {
    load("globalThis.__cb = {}; globalThis.__cbk = 'prop'; globalThis.__cbk2 = 'other';");
    let object = global("__cb");
    let prop = global("__cbk");
    // no bridge on this thread: reading the accessor throws the unavailable-bridge TypeError
    let define = |key: u64, getter: u64| {
        value(thaw_tjs_define_callback_property_key_handle_result(
            object,
            key,
            getter,
            0,
            0,
            2 | 4,
            0,
            0,
            0,
        ))
    };
    assert_eq!(define(prop, 7), 1);
    let (_, like) = thrown(thaw_tjs_get_property_key_handle_with_exception_result(
        object, prop,
    ));
    assert!(like);

    set_callback_bridge(Some(fake_bridge));
    assert!(is_number(
        ok_value(thaw_tjs_get_property_key_handle_with_exception_result(
            object, prop
        )),
        42.0
    ));
    // a bridge error reads as a thrown TypeError
    assert_eq!(define(global("__cbk2"), 9), 1);
    let (error, like) = thrown(thaw_tjs_get_property_key_handle_with_exception_result(
        object,
        global("__cbk2"),
    ));
    assert!(like);
    assert!(query(error, 2).unwrap().contains("bad id"));
    set_callback_bridge(None);

    // a method is a data property holding the function: its call goes through the bridge too
    assert_eq!(
        value(thaw_tjs_define_callback_property_key_handle_result(
            object,
            string_handle("method"),
            0,
            0,
            7,
            1 | 2,
            0,
            0,
            0,
        )),
        1
    );
    assert_eq!(
        query(
            ok_value(thaw_tjs_get_property_key_handle_with_exception_result(
                object,
                string_handle("method")
            )),
            0
        )
        .unwrap(),
        "function"
    );
}

#[test]
fn host_property_names_orders_and_filters_keys() {
    load(
        "globalThis.__pn = Object.create({ inherited: 1 }); __pn.a = 1; __pn[2] = 'two';\
         globalThis.__pnSym = Symbol('n'); __pn[__pnSym] = 3;\
         Object.defineProperty(__pn, 'hidden', { value: 4, enumerable: false, writable: true, configurable: true });\
         globalThis.__pnStr = 's';",
    );
    let object = global("__pn");
    // own keys: indices first (as numbers when converted), then insertion order, then Symbols
    assert_eq!(names(object, 1, 16, 0).unwrap(), r#"[2,"a","hidden"]"#);
    assert_eq!(names(object, 1, 16 | 2, 0).unwrap(), r#"[2,"a"]"#);
    assert_eq!(names(object, 1, 16, 1).unwrap(), r#"["2","a","hidden"]"#);
    // walking the chain adds inherited enumerable keys; a string-skipping filter keeps Symbols only
    assert_eq!(
        names(object, 0, 16 | 2, 0).unwrap(),
        r#"[2,"a","inherited"]"#
    );
    // the Symbol key of a filter that skips strings is an element that reads as a symbol
    let list = thaw_tjs_host_property_names_result(object, 1, 8, 0);
    let retained = value(thaw_tjs_retain_graph_result(list.value));
    assert_eq!(
        query(
            value(thaw_tjs_get_property_result(retained, q("0").as_ptr())),
            0
        )
        .unwrap(),
        "symbol"
    );
    assert!(
        names_failure(thaw_tjs_host_property_names_result(999_999, 1, 0, 0))
            .contains("invalid JavaScript handle")
    );
    assert!(names_failure(thaw_tjs_host_property_names_result(
        global("__pnStr"),
        1,
        0,
        0
    ))
    .contains("Reflect.ownKeys called on non-object"));
}

/// The handle of an `undefined` value (a global that is undefined has no handle, a property does).
fn undefined_of_holder() -> u64 {
    value(thaw_tjs_get_property_result(
        global("__uh"),
        q("u").as_ptr(),
    ))
}

#[test]
fn abstract_to_string_units_runs_effects_and_keeps_throws() {
    load(
        "globalThis.__wtag = { tag: 'str' };\
         globalThis.__as = { toString() { return 'ab'; } };\
         globalThis.__asBad = { toString() { throw globalThis.__wtag; } };\
         globalThis.__asSym = Symbol('q'); globalThis.__n5 = 5; globalThis.__uh = { u: undefined };",
    );
    let units = |result: ThawStringCoercionResult| -> String {
        assert!(result.error.is_null(), "unexpected error");
        assert_eq!(result.exception_handle, 0);
        let text = unsafe { CStr::from_ptr(result.units) }
            .to_string_lossy()
            .into_owned();
        unsafe { thaw_arena::destroy_string(result.units) };
        text
    };
    assert_eq!(
        units(thaw_tjs_abstract_to_string_units_result(global("__n5"), 0)),
        "[53]"
    );
    assert_eq!(
        units(thaw_tjs_abstract_to_string_units_result(global("__as"), 0)),
        "[97,98]"
    );
    assert_eq!(
        units(thaw_tjs_abstract_to_string_units_result(
            undefined_of_holder(),
            1
        )),
        "[]"
    );
    assert_eq!(
        units(thaw_tjs_abstract_to_string_units_result(
            undefined_of_holder(),
            0
        )),
        "[117,110,100,101,102,105,110,101,100]"
    );

    let coerced = thaw_tjs_abstract_to_string_units_result(global("__asBad"), 0);
    assert!(coerced.units.is_null());
    assert_eq!(coerced.exception_object_like, 1);
    assert!(same(coerced.exception_handle, global("__wtag")));

    let symbol = thaw_tjs_abstract_to_string_units_result(global("__asSym"), 0);
    assert_eq!(symbol.exception_object_like, 1);
    assert!(query(symbol.exception_handle, 2)
        .unwrap()
        .contains("Cannot convert a Symbol value to a string"));

    let stale = thaw_tjs_abstract_to_string_units_result(999_999, 0);
    assert!(!stale.error.is_null());
    assert_eq!(error_of(stale.error), "invalid JavaScript handle");
}

/// The graph wire of `thaw_tjs_host_property_names_result` for `object` (keys as napi graph nodes).
fn names_wire(object: u64, filter: u32) -> String {
    let result = thaw_tjs_napi_host_property_names_result(object, 1, filter, 0);
    assert!(result.error.is_null(), "{}", error_of(result.error));
    unsafe { CStr::from_ptr(result.value) }
        .to_string_lossy()
        .into_owned()
}

#[test]
fn property_names_wire_keeps_a_lone_surrogate_key_as_utf16_units() {
    load("globalThis.__pnSu = { [String.fromCharCode(0xD800)]: 1 };");
    let wire = names_wire(global("__pnSu"), 0);
    assert!(wire.contains(r#"{"su":[55296]}"#), "{wire}");
}

#[test]
fn property_names_wire_encodes_a_symbol_key_as_a_handle_node() {
    load("globalThis.__pnSy = { [Symbol('pnSym')]: 1 };");
    let wire = names_wire(global("__pnSy"), 8);
    assert!(wire.contains(r#""hdl""#), "{wire}");
}

/// The napi attribute bits of own key `key` of `object` (`1` writable, `2` enumerable, `4`
/// configurable), probed one bit at a time through the key-list filter.
fn attr_bits(object: u64, key: &str) -> u32 {
    [1u32, 2, 4]
        .into_iter()
        .filter(|bit| {
            names(object, 1, *bit, 0)
                .unwrap()
                .contains(&format!("\"{key}\""))
        })
        .sum()
}

#[test]
fn own_entries_keep_each_property_writable_and_configurable_bits() {
    load("globalThis.__c3aDp = {}; Object.defineProperty(__c3aDp, 'x', { value: 1, writable: false, enumerable: true, configurable: true });");
    assert_eq!(attr_bits(global("__c3aDp"), "x"), 6);
}

#[test]
fn own_entries_default_property_has_no_attribute_bits() {
    load("globalThis.__c3aDef = {}; Object.defineProperty(__c3aDef, 'x', { value: 1 });");
    assert_eq!(attr_bits(global("__c3aDef"), "x"), 0);
}

#[test]
fn own_entries_accessor_bit_is_configurable_only() {
    load("globalThis.__c3aAcc = {}; Object.defineProperty(__c3aAcc, 'x', { get() { return 1; }, configurable: true });");
    assert_eq!(attr_bits(global("__c3aAcc"), "x"), 4);
}

#[test]
fn own_entries_sealed_object_drops_only_configurable() {
    load("globalThis.__c3aSealed = Object.seal({ a: 1 });");
    assert_eq!(attr_bits(global("__c3aSealed"), "a"), 3);
}

#[test]
fn own_entries_frozen_object_drops_writable_and_configurable() {
    load("globalThis.__c3aFrozen = Object.freeze({ a: 1 });");
    assert_eq!(attr_bits(global("__c3aFrozen"), "a"), 2);
}
