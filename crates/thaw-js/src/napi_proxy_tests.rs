//! Unit tests of the native addon proxy layer's scaffolding (PR 6b-1): the opt-in enable and its immutable
//! globals, the export listing, the native pair global and the invoker factory. The bridge is the fake of
//! napi_graph_tests: it answers the handle operations these tests make and records the graph calls.

use std::cell::RefCell;
use std::ffi::{c_char, CStr, CString};

use crate::host::{
    classify, default_realm, store_handle, thaw_tjs_release_handle, thaw_tjs_retain_handle,
    with_package, HostError,
};
use crate::napi_bridge::{clear_for_tests, thaw_tjs_register_napi_bridge};
use crate::napi_graph::{tag, Tag};
use crate::web_napi_proxy::thaw_tjs_napi_enable_proxy_layer;
use crate::{Interp, Value};

thread_local! {
    /// The `(operation, target, graph)` of every graph call the fake bridge got on this thread.
    static CALLS: RefCell<Vec<(String, String, String)>> = const { RefCell::new(Vec::new()) };
    /// The napi references the fake bridge was asked to release on this thread.
    static RELEASED: RefCell<Vec<u64>> = const { RefCell::new(Vec::new()) };
}

fn read(ptr: *const c_char) -> String {
    unsafe { CStr::from_ptr(ptr) }
        .to_string_lossy()
        .into_owned()
}

fn owned(text: &str) -> *const c_char {
    CString::new(text).unwrap().into_raw() as *const c_char
}

unsafe extern "C" fn fake_exports() -> *const c_char {
    owned(r#"{"names":["a::x","a::y","b::z","plain"],"qualifiedPackages":["a"]}"#)
}

unsafe extern "C" fn fake_call(_: *const c_char, _: *const c_char) -> *const c_char {
    owned("")
}

/// `symbol_metadata` names Symbol id 5 for target 5 and id 99 otherwise.
unsafe extern "C" fn fake_handle(
    operation: *const c_char,
    target: *const c_char,
    _: *const c_char,
    graph: *const c_char,
) -> *const c_char {
    let (operation, target, graph) = (read(operation), read(target), read(graph));
    let reply = match operation.as_str() {
        "retain_graph_handle" => r#"{"value":"73"}"#.to_string(),
        "symbol_metadata" => {
            let id = if target == "5" { "5" } else { "99" };
            format!(
                r#"{{"value":{{"id":"{id}","handle":"{target}","units":[104,105],"registered":false,"jsOrigin":false}}}}"#
            )
        }
        "call_captured_graph" | "construct_captured_graph" => {
            CALLS.with(|c| c.borrow_mut().push((operation, target, graph)));
            r#"{"kind":"graph","value":"done"}"#.to_string()
        }
        _ => "{}".to_string(),
    };
    owned(&reply)
}

extern "C" fn fake_poll() -> usize {
    0
}

extern "C" fn fake_pending() -> u8 {
    0
}

unsafe extern "C" fn fake_owner(_: u64) -> *mut c_char {
    CString::new("").unwrap().into_raw()
}

extern "C" fn fake_release(token: u64) -> u8 {
    RELEASED.with(|r| r.borrow_mut().push(token));
    1
}

/// Holds the bridge test lock with the fake bridge registered; dropping it clears the registration.
struct FakeBridge {
    _lock: std::sync::MutexGuard<'static, ()>,
}

impl Drop for FakeBridge {
    fn drop(&mut self) {
        clear_for_tests();
    }
}

fn fake_bridge() -> FakeBridge {
    let lock = crate::napi_bridge_tests::LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    clear_for_tests();
    CALLS.with(|c| c.borrow_mut().clear());
    RELEASED.with(|r| r.borrow_mut().clear());
    thaw_tjs_register_napi_bridge(
        fake_exports,
        fake_call,
        fake_handle,
        fake_poll,
        fake_pending,
        fake_owner,
        fake_release,
    );
    FakeBridge { _lock: lock }
}

/// Runs `source` in the default realm and returns its value as text.
fn eval(source: &str) -> String {
    with_package(default_realm(), |p| {
        let it = &mut p.interp;
        let value = it.run(source).map_err(|c| classify(it, c))?;
        Ok(it.to_str(&value).to_string())
    })
    .map_err(|e| e.to_string())
    .unwrap()
}

/// A global of the default realm, by name.
fn global_value(name: &str) -> Value {
    with_package(default_realm(), |p| {
        let it = &mut p.interp;
        let global = it.global.clone();
        Ok(it.lookup(&global, name).unwrap_or(Value::Undefined))
    })
    .unwrap()
}

const PROBE: &str = "[typeof __thaw_json_graph_napi_exports, typeof __thaw_json_graph_napi_each_export, \
     typeof __thaw_json_graph_native_pair_matches, typeof __thaw_json_graph_napi_bridge_available].join('|')";

#[test]
fn proxy_layer_globals_exist_only_after_enable_and_bridge_available_never_does() {
    let _bridge = fake_bridge();
    assert_eq!(eval(PROBE), "undefined|undefined|undefined|undefined");
    thaw_tjs_napi_enable_proxy_layer();
    assert_eq!(eval(PROBE), "function|function|function|undefined");
}

#[test]
fn enabling_twice_keeps_one_set_of_immutable_globals() {
    let _bridge = fake_bridge();
    thaw_tjs_napi_enable_proxy_layer();
    eval("globalThis.__first = [__thaw_json_graph_napi_exports, __thaw_json_graph_napi_each_export];");
    thaw_tjs_napi_enable_proxy_layer();
    assert_eq!(
        eval("__first[0] === __thaw_json_graph_napi_exports && __first[1] === __thaw_json_graph_napi_each_export"),
        "true"
    );
    assert_eq!(
        eval("(() => { let r; try { __thaw_json_graph_napi_exports = 1; r = 'written'; } catch (e) { r = e.name; } return r + '|' + typeof __thaw_json_graph_napi_exports; })()"),
        "TypeError|function"
    );
}

#[test]
fn realms_built_after_enable_get_the_layer() {
    thaw_tjs_napi_enable_proxy_layer();
    let mut it = Interp::new();
    let value = it
        .run("typeof __thaw_json_graph_napi_exports + '|' + typeof __thaw_json_graph_native_pair_matches")
        .unwrap_or_else(|_| panic!("probe must run"));
    assert_eq!(it.to_str(&value).to_string(), "function|function");
}

#[test]
fn exports_and_each_export_follow_the_qualified_package_rules() {
    let _bridge = fake_bridge();
    thaw_tjs_napi_enable_proxy_layer();
    assert_eq!(
        eval("__thaw_json_graph_napi_exports().names.join(',')"),
        "a::x,a::y,b::z,plain"
    );
    assert_eq!(
        eval(
            "(() => { const out = []; const info = __thaw_json_graph_napi_exports(); \
             __thaw_json_graph_napi_each_export(info, 'a::', 'a', (n, b) => out.push(n + '=' + b)); \
             __thaw_json_graph_napi_each_export(info, '', 'c', (n, b) => out.push(n + '=' + b)); \
             return out.join(','); })()"
        ),
        "a::x=x,a::y=y,plain=plain"
    );
}

#[test]
fn native_pair_global_proves_tagged_functions_and_bridge_confirmed_symbols() {
    let _bridge = fake_bridge();
    thaw_tjs_napi_enable_proxy_layer();
    let owner = default_realm();
    eval("globalThis.tagged = function () {}; globalThis.named = Symbol('hi');");
    let Value::Object(function) = global_value("tagged") else {
        panic!("tagged is not an object")
    };
    tag(&function, Tag::Function, "9");
    let Value::Object(symbol) = global_value("named") else {
        panic!("named is not an object")
    };
    crate::napi_symbols::bind(owner as u64, 5, &symbol);
    assert_eq!(
        eval(
            "[__thaw_json_graph_native_pair_matches(tagged, '9', 1), \
             __thaw_json_graph_native_pair_matches(tagged, '8', 1), \
             __thaw_json_graph_native_pair_matches(tagged, 'x', 1), \
             __thaw_json_graph_native_pair_matches(tagged, '9', 2), \
             __thaw_json_graph_native_pair_matches(named, '5', 2), \
             __thaw_json_graph_native_pair_matches(named, '6', 2), \
             __thaw_json_graph_native_pair_matches(named, '5', 3)].join(',')"
        ),
        "true,false,false,false,true,false,false"
    );
}

#[test]
fn invoker_sends_the_packet_as_call_or_construct_and_returns_the_reply() {
    let _bridge = fake_bridge();
    let owner = default_realm();
    let packet = r#"{"root":{"r":0},"nodes":[],"leases":[],"napiLeases":[]}"#;
    let reply = with_package(owner, |p| {
        let it = &mut p.interp;
        let factory = crate::napi_bridge::graph_invoker_factory(it);
        let closure = it
            .call(&factory, &Value::Undefined, &[Value::str("9")])
            .map_err(|c| classify(it, c))?;
        let call = it
            .call(
                &closure,
                &Value::Undefined,
                &[Value::str(packet), Value::Bool(false)],
            )
            .map_err(|c| classify(it, c))?;
        let construct = it
            .call(
                &closure,
                &Value::Undefined,
                &[Value::str(packet), Value::Bool(true)],
            )
            .map_err(|c| classify(it, c))?;
        Ok(format!("{}|{}", it.to_str(&call), it.to_str(&construct)))
    })
    .map_err(|e: HostError| e.to_string())
    .unwrap();
    assert_eq!(
        reply,
        r#"{"kind":"graph","value":"done"}|{"kind":"graph","value":"done"}"#
    );
    CALLS.with(|c| {
        assert_eq!(
            *c.borrow(),
            vec![
                (
                    "call_captured_graph".to_string(),
                    "9".to_string(),
                    packet.to_string()
                ),
                (
                    "construct_captured_graph".to_string(),
                    "9".to_string(),
                    packet.to_string()
                ),
            ]
        )
    });
}

#[test]
fn invoker_factory_needs_a_bridge_and_an_unsent_packet_releases_its_leases() {
    let _bridge = fake_bridge();
    let owner = default_realm();
    let (factory, closure) = with_package(owner, |p| {
        let it = &mut p.interp;
        let factory = crate::napi_bridge::graph_invoker_factory(it);
        let closure = it
            .call(&factory, &Value::Undefined, &[Value::str("9")])
            .map_err(|c| classify(it, c))?;
        Ok((factory, closure))
    })
    .unwrap();

    // The packet's handle lease: the store's reference and the lease's remain; the lease goes unsent.
    let h = store_handle(owner, Value::Number(1.0));
    assert_eq!(thaw_tjs_retain_handle(h), 1);
    let packet = format!(r#"{{"root":{{"r":0}},"nodes":[],"leases":[{h}],"napiLeases":[]}}"#);
    clear_for_tests();
    let reply = with_package(owner, |p| {
        let it = &mut p.interp;
        let reply = it
            .call(
                &closure,
                &Value::Undefined,
                &[Value::str(&packet), Value::Bool(false)],
            )
            .map_err(|c| classify(it, c))?;
        Ok(it.to_str(&reply).to_string())
    })
    .unwrap();
    assert_eq!(
        reply,
        r#"{"__thaw_error__":"Native addon bridge is unavailable"}"#
    );
    assert_eq!(thaw_tjs_release_handle(h), 1);
    assert_eq!(thaw_tjs_release_handle(h), 0);

    // Without a bridge the factory cannot retain its handle.
    let err = with_package(owner, |p| {
        let it = &mut p.interp;
        it.call(&factory, &Value::Undefined, &[Value::str("9")])
            .map_err(|c| classify(it, c))
    })
    .map_err(|e| e.to_string())
    .err()
    .expect("factory without a bridge");
    assert!(err.contains("Cannot retain native Function"), "{err}");
}
