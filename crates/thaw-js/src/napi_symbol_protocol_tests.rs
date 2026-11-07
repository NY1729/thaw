//! Unit tests for the bridge protocol of the Symbol registry in `napi_symbols.rs`: the id the bridge
//! supplies for a graph owner, the pin and its rollback, the native-handle metadata path, and the
//! retained references released at shutdown. The bridge is a fake whose state is per thread; the
//! registration is process-wide, so every test holds the bridge test lock.

use std::cell::RefCell;
use std::collections::HashMap;
use std::ffi::{c_char, CStr, CString};
use std::sync::MutexGuard;

use crate::host::{
    classify, default_realm, thaw_tjs_get_global, thaw_tjs_load_script, with_package, HandleResult,
};
use crate::napi_bridge::{
    clear_for_tests, shutdown, thaw_tjs_register_napi_bridge,
    thaw_tjs_register_napi_shutdown_bridge,
};
use crate::napi_ops::{handle_value, thaw_tjs_strict_equal_handles_result};
use crate::napi_props::ThawStringCoercionResult;
use crate::napi_symbols::*;
use crate::{Interp, Value};

/// A graph owner id of the napi side (not a package index).
const OWNER: u64 = 7042;

#[derive(Default)]
struct Fake {
    /// (operation, target, name, args) of every `handle` call, in order.
    calls: Vec<(String, String, String, String)>,
    /// The id a `symbol_graph` reply names; 0 makes it fail.
    graph_id: u64,
    pin_fails: bool,
    /// native handle -> owner
    owners: HashMap<u64, u64>,
    /// native handle -> the `symbol_metadata` reply value
    meta: HashMap<u64, String>,
    /// symbol id -> native handle (`symbol_handle`)
    handles: HashMap<u64, u64>,
    /// tokens released by `release`
    released: Vec<u64>,
}

thread_local! {
    static FAKE: RefCell<Fake> = RefCell::new(Fake::default());
}

fn fake<T>(f: impl FnOnce(&mut Fake) -> T) -> T {
    FAKE.with(|s| f(&mut s.borrow_mut()))
}

/// Holds the bridge test lock; dropping it clears the registrations.
struct Guard {
    _lock: MutexGuard<'static, ()>,
}

impl Drop for Guard {
    fn drop(&mut self) {
        clear_for_tests();
    }
}

fn setup() -> Guard {
    let lock = crate::napi_bridge_tests::LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    clear_for_tests();
    FAKE.with(|s| *s.borrow_mut() = Fake::default());
    thaw_tjs_register_napi_bridge(
        fake_exports,
        fake_call,
        fake_handle,
        fake_poll,
        fake_pending,
        fake_owner,
        fake_release,
    );
    Guard { _lock: lock }
}

fn q(text: &str) -> CString {
    CString::new(text).unwrap()
}

fn owned(text: &str) -> *mut c_char {
    q(text).into_raw()
}

unsafe fn read(ptr: *const c_char) -> String {
    CStr::from_ptr(ptr).to_string_lossy().into_owned()
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

/// The `symbol_metadata` reply value of a Symbol.
fn meta(id: u64, handle: u64, units: &[u16], registered: bool, js_origin: bool) -> String {
    let units: Vec<String> = units.iter().map(u16::to_string).collect();
    format!(
        r#"{{"id":"{id}","handle":"{handle}","units":[{}],"registered":{registered},"jsOrigin":{js_origin}}}"#,
        units.join(",")
    )
}

fn error_reply(text: &str) -> String {
    format!(r#"{{"__thaw_error__":"{text}"}}"#)
}

unsafe extern "C" fn fake_exports() -> *const c_char {
    owned("{\"names\":[]}")
}

unsafe extern "C" fn fake_call(_: *const c_char, _: *const c_char) -> *const c_char {
    owned("")
}

unsafe extern "C" fn fake_handle(
    operation: *const c_char,
    target: *const c_char,
    name: *const c_char,
    args: *const c_char,
) -> *const c_char {
    let (operation, target, name, args) = (read(operation), read(target), read(name), read(args));
    let reply = fake(|f| {
        let t: u64 = target.parse().unwrap_or(0);
        f.calls.push((operation.clone(), target, name, args));
        match operation.as_str() {
            "symbol_graph" if f.graph_id != 0 => format!(r#"{{"value":"{}"}}"#, f.graph_id),
            "symbol_pin" if f.pin_fails => error_reply("pin refused"),
            "symbol_handle" => match f.handles.get(&t) {
                Some(native) => format!(r#"{{"value":"{native}"}}"#),
                None => error_reply("no symbol"),
            },
            "symbol_metadata" => match f.meta.get(&t) {
                Some(value) => format!(r#"{{"value":{value}}}"#),
                None => error_reply("no metadata"),
            },
            "retain_graph_handle" => r#"{"value":"901"}"#.to_string(),
            _ => r#"{"value":true}"#.to_string(),
        }
    });
    owned(&reply) as *const c_char
}

extern "C" fn fake_poll() -> usize {
    0
}

extern "C" fn fake_pending() -> u8 {
    0
}

unsafe extern "C" fn fake_owner(handle: u64) -> *mut c_char {
    let text = fake(|f| f.owners.get(&handle).map(u64::to_string)).unwrap_or_default();
    owned(&text)
}

extern "C" fn fake_release(token: u64) -> u8 {
    fake(|f| f.released.push(token));
    1
}

extern "C" fn shutdown_begin() -> u8 {
    1
}

extern "C" fn shutdown_poll() -> u8 {
    1
}

extern "C" fn shutdown_take_error() -> *mut c_char {
    std::ptr::null_mut()
}

/// Returns 0, which ends the shutdown loop after one finish.
extern "C" fn shutdown_finish() -> u8 {
    0
}

/// The graph owner's Symbol id the bridge reports for the next `symbol_graph`.
fn arm_graph_id(id: u64) {
    fake(|f| f.graph_id = id);
}

/// Register the Symbol the global `path` names for `OWNER`.
fn register(path: &str) -> u64 {
    thaw_tjs_napi_graph_symbol_register(OWNER, global(path))
}

#[test]
fn register_takes_the_bridge_id_for_a_graph_owner() {
    let _guard = setup();
    load("globalThis.gsym = Symbol('a');");
    arm_graph_id(41);
    let value = handle_value(global("gsym")).unwrap();
    assert_eq!(register("gsym"), 41);
    assert_eq!(native_id_of_value(OWNER, &value), Some(41));
    // another owner that has no name of its own gets the single name the lowest owner gave it
    assert_eq!(native_id_of_value(99, &value), Some(41));
    assert_eq!(thaw_tjs_napi_symbol_weak_live(OWNER, 41), 1);
    fake(|f| {
        let (op, target, name, args) = &f.calls[0];
        assert_eq!((op.as_str(), target.as_str()), ("symbol_graph", "7042"));
        assert_eq!((name.as_str(), args.as_str()), ("a", "[false,null,false]"));
        let (op, target, name, _) = &f.calls[1];
        assert_eq!(
            (op.as_str(), target.as_str(), name.as_str()),
            ("symbol_pin", "7042", "41")
        );
    });
    // a second registration is answered from the binding, without asking the bridge again
    assert_eq!(register("gsym"), 41);
    fake(|f| assert_eq!(f.calls.len(), 2));
}

#[test]
fn permanent_symbol_is_named_without_a_pin() {
    let _guard = setup();
    load("globalThis.regSym = Symbol.for('k');");
    arm_graph_id(42);
    assert_eq!(register("regSym"), 42);
    fake(|f| {
        assert_eq!(f.calls.len(), 1, "no symbol_pin for a permanent Symbol");
        assert_eq!(f.calls[0].2, "k");
        assert_eq!(f.calls[0].3, "[true,null,true]");
    });
    assert_eq!(thaw_tjs_napi_symbol_weak_live(OWNER, 42), 1);
}

#[test]
fn refused_pin_rolls_the_naming_back() {
    let _guard = setup();
    load("globalThis.pinSym = Symbol('p');");
    arm_graph_id(43);
    fake(|f| f.pin_fails = true);
    let value = handle_value(global("pinSym")).unwrap();
    assert_eq!(register("pinSym"), 0);
    fake(|f| {
        let rollback = f.calls.iter().any(|(op, target, name, _)| {
            op == "symbol_rollback" && target == "7042" && name == "43"
        });
        assert!(rollback, "no symbol_rollback in {:?}", f.calls);
    });
    assert_eq!(thaw_tjs_napi_symbol_weak_live(OWNER, 43), 0);
    assert_eq!(native_id_of_value(OWNER, &value), None);
}

#[test]
fn coercion_of_a_native_symbol_is_a_type_error_after_global_symbol_is_replaced() {
    let _guard = setup();
    fake(|f| {
        f.handles.insert(42, 5151);
        f.owners.insert(5151, OWNER);
        f.meta
            .insert(5151, meta(42, 5151, &[104, 105], false, false));
    });
    load("globalThis.Symbol = () => 17;");
    let r: ThawStringCoercionResult =
        thaw_tjs_napi_symbol_abstract_to_string_units_result(OWNER, 42);
    assert!(r.error.is_null(), "unexpected error");
    assert!(r.units.is_null());
    assert_ne!(r.exception_handle, 0);
    let thrown = handle_value(r.exception_handle).unwrap();
    let name = with_package(default_realm(), |p| {
        let it = &mut p.interp;
        let name = it.get(&thrown, "name").map_err(|c| classify(it, c))?;
        Ok(it.to_str(&name).to_string())
    })
    .unwrap();
    assert_eq!(name, "TypeError");
}

#[test]
fn js_origin_native_handle_is_the_bound_original() {
    let _guard = setup();
    load("globalThis.origSym = Symbol('o');");
    let original = handle_value(global("origSym")).unwrap();
    let Value::Object(object) = &original else {
        panic!("origSym is not a Symbol object")
    };
    bind(OWNER, 77, object);
    fake(|f| {
        f.owners.insert(5160, OWNER);
        f.meta.insert(5160, meta(77, 5160, &[111], false, true));
    });
    let got = ok(thaw_tjs_native_symbol_handle_result(5160));
    assert!(same(got, global("origSym")));
}

#[test]
fn registered_native_handle_is_symbol_for_its_key() {
    let _guard = setup();
    load("globalThis.keyedSym = Symbol.for('k');");
    fake(|f| {
        f.owners.insert(5161, OWNER);
        f.meta.insert(5161, meta(78, 5161, &[107], true, false));
    });
    let got = ok(thaw_tjs_native_symbol_handle_result(5161));
    assert!(same(got, global("keyedSym")));
}

#[test]
fn native_created_symbol_reference_is_released_at_shutdown() {
    let _guard = setup();
    fake(|f| {
        f.owners.insert(5170, OWNER);
        f.meta
            .insert(5170, meta(88, 5170, &[104, 105], false, false));
    });
    thaw_tjs_register_napi_shutdown_bridge(
        shutdown_begin,
        shutdown_poll,
        shutdown_take_error,
        shutdown_finish,
    );
    let _ = ok(thaw_tjs_native_symbol_handle_result(5170));
    fake(|f| assert!(f.released.is_empty()));
    let mut it = Interp::new();
    assert_eq!(shutdown(&mut it), Ok(()));
    fake(|f| assert_eq!(f.released, vec![901]));
}
