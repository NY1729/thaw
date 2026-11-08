//! Unit tests of the native addon proxy layer's scaffolding (PR 6b-1): the opt-in enable and its immutable
//! globals, the export listing, the native pair global and the invoker factory. The bridge is the fake of
//! napi_graph_tests: it answers the handle operations these tests make and records the graph calls.

use std::cell::{Cell, RefCell};
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
fn proxy_layer_globals_exist_only_after_enable() {
    let _bridge = fake_bridge();
    assert_eq!(eval(PROBE), "undefined|undefined|undefined|undefined");
    thaw_tjs_napi_enable_proxy_layer();
    assert_eq!(eval(PROBE), "function|function|function|function");
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

// ---- Object and array proxies (PR 6b-2): a table-driven fake bridge. ----

thread_local! {
    /// The fake's replies: `(operation, needle, reply)`. The first entry whose operation matches and whose
    /// packet text contains `needle` answers (an empty needle matches any packet); an unlisted call gets `{}`.
    static TABLE: RefCell<Vec<(String, String, String)>> = const { RefCell::new(Vec::new()) };
    /// Every handle operation the table fake got on this thread: `(operation, target, packet)`.
    static OPS: RefCell<Vec<(String, String, String)>> = const { RefCell::new(Vec::new()) };
    /// How many `promise_state` polls of the table fake still answer `pending` before the next one resolves.
    static PENDING_POLLS: Cell<usize> = const { Cell::new(0) };
}

unsafe extern "C" fn table_handle(
    operation: *const c_char,
    target: *const c_char,
    _: *const c_char,
    graph: *const c_char,
) -> *const c_char {
    let (operation, target, graph) = (read(operation), read(target), read(graph));
    OPS.with(|o| {
        o.borrow_mut()
            .push((operation.clone(), target, graph.clone()))
    });
    if operation == "promise_state" {
        let left = PENDING_POLLS.with(|p| {
            let n = p.get();
            p.set(n.saturating_sub(1));
            n
        });
        let text = if left > 0 {
            r#"{"kind":"pending"}"#
        } else {
            r#"{"kind":"resolved","value":"done","origins":[]}"#
        };
        return owned(text);
    }
    let reply = TABLE.with(|t| {
        t.borrow()
            .iter()
            .find(|(op, needle, _)| *op == operation && graph.contains(needle.as_str()))
            .map(|(_, _, reply)| reply.clone())
    });
    owned(&reply.unwrap_or_else(|| "{}".to_string()))
}

/// The owner of every handle is live Symbol owner 7 (the proxy's owner must be a positive decimal).
unsafe extern "C" fn owner_seven(_: u64) -> *mut c_char {
    CString::new("7").unwrap().into_raw()
}

/// The wire text of a graph with `root` and `nodes` (the JSON text of each).
fn wire(root: &str, nodes: &str) -> String {
    format!(r#"{{"root":{root},"nodes":[{nodes}],"leases":[],"napiLeases":[]}}"#)
}

/// A `graph` reply carrying the wire.
fn graph(root: &str, nodes: &str) -> String {
    serde_json::json!({ "kind": "graph", "value": wire(root, nodes) }).to_string()
}

fn reply(operation: &str, needle: &str, text: &str) {
    TABLE.with(|t| {
        t.borrow_mut()
            .push((operation.into(), needle.into(), text.into()))
    });
}

/// Holds the bridge test lock with the table fake registered, owner 7; dropping it clears the registration.
fn table_bridge() -> FakeBridge {
    let lock = crate::napi_bridge_tests::LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    clear_for_tests();
    TABLE.with(|t| t.borrow_mut().clear());
    OPS.with(|o| o.borrow_mut().clear());
    PENDING_POLLS.with(|p| p.set(0));
    reply("retain_graph_handle", "", r#"{"value":"73"}"#);
    thaw_tjs_register_napi_bridge(
        fake_exports,
        fake_call,
        table_handle,
        fake_poll,
        fake_pending,
        owner_seven,
        fake_release,
    );
    FakeBridge { _lock: lock }
}

/// How many times the table fake got `operation`.
fn op_count(operation: &str) -> usize {
    OPS.with(|o| {
        o.borrow()
            .iter()
            .filter(|(op, _, _)| op == operation)
            .count()
    })
}

/// The operation names the table fake got, in order.
fn op_names() -> Vec<String> {
    OPS.with(|o| o.borrow().iter().map(|(op, _, _)| op.clone()).collect())
}

#[test]
fn proxy_for_handle_is_an_immutable_global_and_each_handle_has_one_proxy() {
    let _bridge = table_bridge();
    reply("array_kind_graph", "", r#"{"value":false}"#);
    thaw_tjs_napi_enable_proxy_layer();
    assert_eq!(
        eval(
            "[typeof __thaw_json_graph_proxy_for_handle, (() => { let r; \
             try { __thaw_json_graph_proxy_for_handle = 1; r = 'written'; } catch (e) { r = e.name; } \
             return r; })()].join('|')"
        ),
        "function|TypeError"
    );
    assert_eq!(
        eval("__thaw_json_graph_proxy_for_handle('9') === __thaw_json_graph_proxy_for_handle('9')"),
        "true"
    );
    assert_eq!(op_count("renew_handle"), 1);
    assert_eq!(
        eval("(() => { try { __thaw_json_graph_proxy_for_handle('0'); return 'made'; } catch (e) { return e.name; } })()"),
        "TypeError"
    );
}

#[test]
fn proxy_of_a_native_handle_encodes_as_its_handle() {
    let _bridge = table_bridge();
    reply("array_kind_graph", "", r#"{"value":false}"#);
    thaw_tjs_napi_enable_proxy_layer();
    let encoded = eval(
        "__thaw_json_graph_encode_js(__thaw_json_graph_proxy_for_handle('9'), 0, true, false, undefined)",
    );
    assert!(encoded.contains(r#""nh":"9""#), "{encoded}");
}

#[test]
fn object_proxy_reads_has_keys_and_descriptors_through_the_bridge() {
    let _bridge = table_bridge();
    reply("array_kind_graph", "", r#"{"value":false}"#);
    reply("has_graph", r#"{"v":"x"}"#, r#"{"value":true}"#);
    reply("get_graph", r#"{"v":"x"}"#, &graph(r#"{"v":42}"#, ""));
    reply(
        "own_keys_graph",
        "",
        &graph(r#"{"r":0}"#, r#"{"a":[{"v":"x"},{"v":"y"}]}"#),
    );
    reply(
        "descriptor_graph",
        r#"{"v":"x"}"#,
        r#"{"value":{"accessor":false,"configurable":true,"enumerable":true,"writable":true}}"#,
    );
    thaw_tjs_napi_enable_proxy_layer();
    assert_eq!(
        eval(
            "(() => { const p = __thaw_json_graph_proxy_for_handle('9'); return [p.x, 'x' in p, \
             Reflect.ownKeys(p).join(','), Object.getOwnPropertyDescriptor(p, 'x').enumerable, \
             Object.getPrototypeOf(p) === Object.prototype, Array.isArray(p)].join('|'); })()"
        ),
        "42|true|x,y|true|true|false"
    );
}

#[test]
fn array_proxy_syncs_length_and_hides_holes() {
    let _bridge = table_bridge();
    reply("array_kind_graph", "", r#"{"value":true}"#);
    reply(
        "descriptor_graph",
        r#"{"v":"length"}"#,
        r#"{"value":{"accessor":false,"configurable":false,"enumerable":false,"writable":true}}"#,
    );
    reply("get_graph", r#"{"v":"length"}"#, &graph(r#"{"v":3}"#, ""));
    reply("has_graph", r#"{"v":"1"}"#, r#"{"value":false}"#);
    reply("descriptor_graph", r#"{"v":"1"}"#, r#"{"value":null}"#);
    thaw_tjs_napi_enable_proxy_layer();
    assert_eq!(
        eval(
            "(() => { const p = __thaw_json_graph_proxy_for_handle('9'); return [Array.isArray(p), \
             p.length, 1 in p, Object.getOwnPropertyDescriptor(p, 'length').writable].join('|'); })()"
        ),
        "true|3|false|true"
    );
}

#[test]
fn accessor_descriptor_keeps_one_getter_identity_across_reads() {
    let _bridge = table_bridge();
    reply("array_kind_graph", "", r#"{"value":false}"#);
    reply(
        "descriptor_graph",
        r#"{"v":"computed"}"#,
        r#"{"value":{"accessor":true,"configurable":true,"enumerable":true,"getter":true,"setter":false}}"#,
    );
    reply("has_graph", r#"{"v":"computed"}"#, r#"{"value":true}"#);
    reply("get_graph", r#"{"v":"computed"}"#, &graph(r#"{"v":5}"#, ""));
    thaw_tjs_napi_enable_proxy_layer();
    assert_eq!(
        eval(
            "(() => { const p = __thaw_json_graph_proxy_for_handle('9'); \
             return [Reflect.getOwnPropertyDescriptor(p, 'computed').get === \
             Reflect.getOwnPropertyDescriptor(p, 'computed').get, \
             typeof Reflect.getOwnPropertyDescriptor(p, 'computed').get, p.computed].join('|'); })()"
        ),
        "true|function|5"
    );
}

#[test]
fn defining_an_accessor_sends_define_accessor_graph_with_the_key() {
    let _bridge = table_bridge();
    reply("array_kind_graph", "", r#"{"value":false}"#);
    reply("descriptor_graph", r#"{"v":"acc"}"#, r#"{"value":null}"#);
    reply("define_accessor_graph", "", r#"{"value":true}"#);
    thaw_tjs_napi_enable_proxy_layer();
    assert_eq!(
        eval(
            "(() => { const p = __thaw_json_graph_proxy_for_handle('9'); \
             Object.defineProperty(p, 'acc', { get() { return 1; }, configurable: true }); return 'done'; })()"
        ),
        "done"
    );
    OPS.with(|o| {
        let o = o.borrow();
        let defined: Vec<&(String, String, String)> = o
            .iter()
            .filter(|(op, _, _)| op == "define_accessor_graph")
            .collect();
        assert_eq!(defined.len(), 1);
        assert_eq!(defined[0].1, "9");
        assert!(defined[0].2.contains(r#"{"v":"acc"}"#), "{}", defined[0].2);
    });
}

#[test]
fn delete_and_failed_set_are_the_native_answers() {
    let _bridge = table_bridge();
    reply("array_kind_graph", "", r#"{"value":false}"#);
    reply("delete_graph", r#"{"v":"x"}"#, r#"{"value":true}"#);
    reply("set_graph", r#"{"v":"x"}"#, r#"{"value":false}"#);
    thaw_tjs_napi_enable_proxy_layer();
    assert_eq!(
        eval(
            "(() => { const p = __thaw_json_graph_proxy_for_handle('9'); \
             return [Reflect.deleteProperty(p, 'x'), Reflect.set(p, 'x', 1)].join('|'); })()"
        ),
        "true|false"
    );
    assert_eq!(op_count("delete_graph"), 1);
    assert_eq!(op_count("set_graph"), 1);
}

#[test]
fn prevent_extensions_materializes_native_keys_before_the_native_call() {
    let _bridge = table_bridge();
    reply("array_kind_graph", "", r#"{"value":false}"#);
    reply(
        "own_keys_graph",
        "",
        &graph(r#"{"r":0}"#, r#"{"a":[{"v":"x"}]}"#),
    );
    reply(
        "descriptor_graph",
        r#"{"v":"x"}"#,
        r#"{"value":{"accessor":false,"configurable":true,"enumerable":true,"writable":true}}"#,
    );
    reply("has_graph", r#"{"v":"x"}"#, r#"{"value":true}"#);
    reply("get_graph", r#"{"v":"x"}"#, &graph(r#"{"v":42}"#, ""));
    reply("prevent_extensions_graph", "", r#"{"value":true}"#);
    thaw_tjs_napi_enable_proxy_layer();
    assert_eq!(
        eval(
            "(() => { const p = __thaw_json_graph_proxy_for_handle('9'); \
             return [Object.preventExtensions(p) === p, Object.isExtensible(p), p.x].join('|'); })()"
        ),
        "true|false|42"
    );
    let names = op_names();
    let keys_at = names.iter().position(|op| op == "own_keys_graph").unwrap();
    let prevent_at = names
        .iter()
        .position(|op| op == "prevent_extensions_graph")
        .unwrap();
    assert!(keys_at < prevent_at, "{names:?}");
}

#[test]
fn native_exception_reply_is_thrown_as_its_decoded_value() {
    let _bridge = table_bridge();
    reply("array_kind_graph", "", r#"{"value":false}"#);
    reply("has_graph", r#"{"v":"bad"}"#, r#"{"value":true}"#);
    let thrown =
        serde_json::json!({ "__thaw_exception_graph__": wire(r#"{"v":"boom"}"#, "") }).to_string();
    reply("get_graph", r#"{"v":"bad"}"#, &thrown);
    thaw_tjs_napi_enable_proxy_layer();
    assert_eq!(
        eval(
            "(() => { const p = __thaw_json_graph_proxy_for_handle('9'); let caught; \
             try { p.bad; } catch (e) { caught = e; } return caught === 'boom'; })()"
        ),
        "true"
    );
}

// ---- Function proxies (PR 6b-3). ----

/// Evaluates `expr` over `d`, the decoded graph packet of the last `operation` call the table fake got.
fn decoded_packet(operation: &str, expr: &str) -> String {
    let packet = OPS
        .with(|o| {
            o.borrow()
                .iter()
                .rev()
                .find(|(op, _, _)| op == operation)
                .map(|(_, _, graph)| graph.clone())
        })
        .unwrap_or_else(|| panic!("no {operation} call"));
    eval(&format!(
        "(() => {{ const d = __thaw_json_graph_decode_owned({}); return {expr}; }})()",
        serde_json::to_string(&packet).unwrap()
    ))
}

/// The graph packet of the last `operation` call the table fake got, parsed.
fn packet_of(operation: &str) -> serde_json::Value {
    let packet = OPS
        .with(|o| {
            o.borrow()
                .iter()
                .rev()
                .find(|(op, _, _)| op == operation)
                .map(|(_, _, graph)| graph.clone())
        })
        .unwrap_or_else(|| panic!("no {operation} call"));
    serde_json::from_str(&packet).unwrap()
}

#[test]
fn function_for_handle_is_an_immutable_global_declared_only_when_enabled() {
    let _bridge = table_bridge();
    assert_eq!(
        eval("typeof __thaw_napi_graph_function_for_handle"),
        "undefined"
    );
    thaw_tjs_napi_enable_proxy_layer();
    assert_eq!(
        eval(
            "[typeof __thaw_napi_graph_function_for_handle, (() => { let r; \
             try { __thaw_napi_graph_function_for_handle = 1; r = 'written'; } catch (e) { r = e.name; } \
             return r; })(), __thaw_napi_graph_function_for_handle('9') === __thaw_napi_graph_function_for_handle('9'), \
             typeof __thaw_napi_graph_function_for_handle('9')].join('|')"
        ),
        "function|TypeError|true|function"
    );
    assert_eq!(
        eval("(() => { try { __thaw_napi_graph_function_for_handle('0'); return 'made'; } catch (e) { return e.name; } })()"),
        "TypeError"
    );
}

#[test]
fn calling_a_function_proxy_sends_the_receiver_first_and_returns_the_decoded_reply() {
    let _bridge = table_bridge();
    reply("call_captured_graph", "", &graph(r#"{"v":7}"#, ""));
    thaw_tjs_napi_enable_proxy_layer();
    assert_eq!(
        eval("__thaw_napi_graph_function_for_handle('9').call('recv', 2)"),
        "7"
    );
    assert_eq!(
        decoded_packet("call_captured_graph", "[d[0], d[1], d.length].join('|')"),
        "recv|2|2"
    );
}

#[test]
fn constructing_a_function_proxy_sends_new_target_fallback_and_the_arguments() {
    let _bridge = table_bridge();
    reply(
        "construct_captured_graph",
        "",
        &graph(r#"{"r":0}"#, r#"{"o":[]}"#),
    );
    thaw_tjs_napi_enable_proxy_layer();
    assert_eq!(
        eval("(() => { const f = __thaw_napi_graph_function_for_handle('9'); globalThis.__f = f; return typeof new f(5); })()"),
        "object"
    );
    // The values are [new.target, fallback, 5]: new.target is the tagged function's nfn node (node 1), the
    // fallback prototype travels as a forced handle, the argument by value.
    let packet = packet_of("construct_captured_graph");
    let root = &packet["nodes"][0]["a"];
    assert_eq!(root.as_array().map(Vec::len), Some(3), "{packet}");
    assert_eq!(root[0], serde_json::json!({ "r": 1 }), "{packet}");
    assert_eq!(packet["nodes"][1]["nfn"], "9", "{packet}");
    assert!(root[1].get("hdl").is_some(), "{packet}");
    assert_eq!(root[2], serde_json::json!({ "v": 5 }), "{packet}");
}

#[test]
fn a_derived_class_reaches_the_native_constructor_with_its_own_new_target() {
    let _bridge = table_bridge();
    reply(
        "construct_captured_graph",
        "",
        &graph(r#"{"r":0}"#, r#"{"o":[]}"#),
    );
    thaw_tjs_napi_enable_proxy_layer();
    assert_eq!(
        eval("(() => { const f = __thaw_napi_graph_function_for_handle('9'); class Derived extends f {} globalThis.__D = Derived; return typeof new Derived(1); })()"),
        "object"
    );
    // new.target is the class itself: a live function, so a plain handle rather than an nfn node.
    let packet = packet_of("construct_captured_graph");
    let root = &packet["nodes"][0]["a"];
    assert_eq!(root.as_array().map(Vec::len), Some(3), "{packet}");
    let target = root[0]["r"].as_u64().map(|k| &packet["nodes"][k as usize]);
    assert!(target.is_some_and(|n| n.get("hdl").is_some()), "{packet}");
    assert_eq!(root[2], serde_json::json!({ "v": 1 }), "{packet}");
}

#[test]
fn function_proxy_hides_prototype_and_reports_only_native_keys() {
    let _bridge = table_bridge();
    reply(
        "own_keys_graph",
        "",
        &graph(r#"{"r":0}"#, r#"{"a":[{"v":"x"}]}"#),
    );
    thaw_tjs_napi_enable_proxy_layer();
    assert_eq!(
        eval("(() => { const f = __thaw_napi_graph_function_for_handle('9'); return [Reflect.ownKeys(f).join(','), 'prototype' in f, typeof f].join('|'); })()"),
        "x,length,name|false|function"
    );
}

#[test]
fn a_nfn_token_decodes_to_the_function_proxy_and_encodes_back_as_its_handle() {
    let _bridge = table_bridge();
    thaw_tjs_napi_enable_proxy_layer();
    let out = eval(
        r#"(() => { const g = __thaw_json_graph_decode({"root":{"r":0},"nodes":[{"nfn":"9"}],"leases":[],"napiLeases":[]}); const back = JSON.parse(__thaw_json_graph_encode_js(g, 0, true)); return [g === __thaw_napi_graph_function_for_handle('9'), typeof g, back.nodes[0].nfn, back.nodes[0].hdl !== undefined].join('|'); })()"#,
    );
    assert_eq!(out, "true|function|9|true");
}

// ---- Plain typed ABI, structured part (PR 6b-4a): thaw-napi's handles.rs wire cases as pure JS in and out. ----

/// A fresh realm with the layer's API (a separate factory instance with its own caches) declared as `api`.
fn plain_realm() -> Interp {
    let mut it = Interp::new();
    let api = crate::web_napi_proxy::factory_api(&mut it);
    it.global.declare("api", api, false);
    it
}

/// Runs `source` in `it` and returns its value as text.
fn run_in(it: &mut Interp, source: &str) -> String {
    let value = it
        .run(source)
        .map_err(|c| classify(it, c))
        .map_err(|e| e.to_string())
        .unwrap();
    it.to_str(&value).to_string()
}

#[test]
fn plain_argument_snapshots_objects_and_arrays_and_their_cycles_are_references() {
    let mut it = plain_realm();
    assert_eq!(
        run_in(
            &mut it,
            "(() => { const o = { a: 1 }; o.self = o; return JSON.stringify(api.argument(o)); })()"
        ),
        r#"{"__thaw_napi_object__":1,"value":{"a":1,"self":{"__thaw_napi_ref__":1}}}"#
    );
    assert_eq!(
        run_in(
            &mut it,
            "(() => { const a = []; a.push(a); return JSON.stringify(api.argument(a)); })()"
        ),
        r#"{"__thaw_napi_array__":2,"value":[{"__thaw_napi_ref__":2}]}"#
    );
    assert_eq!(
        run_in(&mut it, "JSON.stringify(api.arguments([1, 'x', null]))"),
        r#"[1,"x",null]"#
    );
    assert_eq!(
        run_in(
            &mut it,
            "[api.argument(undefined) === undefined, api.argument(3), api.argument('s')].join(',')"
        ),
        "true,3,s"
    );
    assert_eq!(
        run_in(&mut it, "(() => { try { api.argument(new Uint8Array(1)); return 'passed'; } catch (e) { return e.name; } })()"),
        "TypeError"
    );
}

#[test]
fn plain_function_argument_is_a_marker_whose_callback_global_the_reference_lookup_finds() {
    let mut it = plain_realm();
    assert_eq!(
        run_in(&mut it, "JSON.stringify(api.argument((a, b) => a + b))"),
        r#"{"__thaw_napi_function__":1}"#
    );
    // The callback global is installed by `defineCallback` with a bare `Object.defineProperty`, so it is
    // both non-writable and non-configurable.
    assert_eq!(
        run_in(&mut it, "Object.getOwnPropertyDescriptor(globalThis, '__thaw_napi_reference_1').writable === false && Object.getOwnPropertyDescriptor(globalThis, '__thaw_napi_reference_1').configurable === false"),
        "true"
    );
    // The lookup that napi_eval's call_reference makes on the realm global.
    let global = it.global.clone();
    let callback = it
        .lookup(&global, "__thaw_napi_reference_1")
        .expect("the callback global is found by name");
    assert!(callback.is_callable());
    let result = it.call(
        &callback,
        &Value::Undefined,
        &[Value::Null, Value::Number(2.0), Value::Number(3.0)],
    );
    assert!(matches!(result, Ok(Value::Number(n)) if n == 5.0));
}

#[test]
fn plain_decode_maps_error_undefined_reference_symbol_and_origin_markers() {
    let mut it = plain_realm();
    assert_eq!(
        run_in(&mut it, "(() => { const e = api.decode({ __thaw_napi_error__: 'boom', name: 'TypeError' }); return [e instanceof TypeError, e.message, e.name].join(','); })()"),
        "true,boom,TypeError"
    );
    assert_eq!(
        run_in(
            &mut it,
            "api.decode({ '$__thaw_napi_undefined$': true }) === undefined"
        ),
        "true"
    );
    assert_eq!(
        run_in(&mut it, "(() => { const o = { x: 1 }; api.argument(o); return api.decode({ __thaw_napi_ref__: 1 }) === o; })()"),
        "true"
    );
    assert_eq!(
        run_in(&mut it, "api.decode({ __thaw_napi_symbol__: '5', description: 'k', global: true }) === Symbol.for('k')"),
        "true"
    );
    assert_eq!(
        run_in(
            &mut it,
            "api.decode({ d: 0 }, [[['d'], 'date', 0]]).d.getTime()"
        ),
        "0"
    );
    assert_eq!(
        run_in(&mut it, "String(api.decode(0, [[[], 'nonfinite', 'NaN']]))"),
        "NaN"
    );
    assert_eq!(
        run_in(
            &mut it,
            "JSON.stringify(api.decode({ __thaw_napi_ref__: 9 }, [[[], 'plain']]))"
        ),
        r#"{"__thaw_napi_ref__":9}"#
    );
}

#[test]
fn plain_handle_marker_of_an_unowned_native_object_is_refused() {
    let _bridge = table_bridge();
    reply("array_kind_graph", "", r#"{"value":false}"#);
    let mut it = plain_realm();
    assert_eq!(
        run_in(&mut it, "(() => { const p = api.decode({ __thaw_napi_handle__: '9' }); try { api.argument(p); return 'passed'; } catch (e) { return e.name + ':' + e.message; } })()"),
        "TypeError:native object belongs to another addon"
    );
}

#[test]
fn plain_handle_wrapper_encodes_the_arguments_and_throws_the_error_reply() {
    let _bridge = table_bridge();
    reply("op_fail", "", r#"{"__thaw_error__":"nope"}"#);
    reply("op_ok", "", r#"{"value":[1,{"a":2}]}"#);
    let mut it = plain_realm();
    assert_eq!(
        run_in(&mut it, "(() => { try { api.handle('op_fail', 1, '', []); return 'no'; } catch (e) { return e.message; } })()"),
        "nope"
    );
    assert_eq!(
        run_in(
            &mut it,
            "JSON.stringify(api.handle('op_ok', 1, '', [{ k: 1 }]).value)"
        ),
        r#"[1,{"a":2}]"#
    );
    let last = OPS.with(|o| o.borrow().last().map(|(_, _, packet)| packet.clone()));
    assert_eq!(
        last.as_deref(),
        Some(r#"[{"__thaw_napi_object__":1,"value":{"k":1}}]"#)
    );
}

#[test]
fn plain_sync_arguments_writes_the_native_values_back_into_the_js_objects() {
    let _bridge = table_bridge();
    reply(
        "sync_reference",
        "",
        r#"{"value":{"a":2,"b":{"c":3}},"origins":[]}"#,
    );
    let mut it = plain_realm();
    assert_eq!(
        run_in(&mut it, "(() => { const o = { a: 1, gone: 1 }; api.argument(o); api.syncArguments([o]); return JSON.stringify(o); })()"),
        r#"{"a":2,"b":{"c":3}}"#
    );
}

// ---- Binary values and promises (PR 6b-4b). ----

#[test]
fn plain_buffer_and_array_buffer_arguments_cross_by_value_and_a_plain_view_is_still_refused() {
    let mut it = plain_realm();
    assert_eq!(
        run_in(&mut it, "JSON.stringify(api.argument(Buffer.from([1, 2])))"),
        r#"{"__thaw_napi_buffer__":1,"value":[1,2]}"#
    );
    assert_eq!(
        run_in(
            &mut it,
            "JSON.stringify(api.argument(new Uint8Array([3]).buffer))"
        ),
        r#"{"__thaw_napi_arraybuffer__":2,"shared":false,"value":[3]}"#
    );
    assert_eq!(
        run_in(
            &mut it,
            "(() => { const ab = new ArrayBuffer(1); return [api.argument(ab).__thaw_napi_arraybuffer__ === api.argument(ab).__thaw_napi_arraybuffer__, api.argument(ab).__thaw_napi_arraybuffer__].join('|'); })()"
        ),
        "true|3"
    );
}

#[test]
fn native_binaries_decode_to_one_value_per_id_and_renew_each_handle_once() {
    let _bridge = table_bridge();
    let mut it = plain_realm();
    assert_eq!(
        run_in(
            &mut it,
            "(() => { const ab = api.decode({ __thaw_napi_binary__: '93', kind: 'ArrayBuffer', data: [1, 2, 3] }); \
             const view = api.decode({ __thaw_napi_binary__: '94', buffer: { __thaw_napi_binary__: '93', \
             kind: 'ArrayBuffer', data: [1, 2, 3] }, array_type: 1, byte_offset: 1, length: 2 }); \
             return [ab instanceof ArrayBuffer, view instanceof Uint8Array, view.buffer === ab, view.join(','), \
             new Uint8Array(ab).join(','), api.decode({ __thaw_napi_binary__: '93', kind: 'ArrayBuffer', data: [9] }) === ab, \
             api.decode({ __thaw_napi_binary__: '94', buffer: { __thaw_napi_binary__: '93' }, array_type: 1, byte_offset: 0, length: 1 }) === view].join('|'); })()"
        ),
        "true|true|true|2,3|1,2,3|true|true"
    );
    assert_eq!(op_count("renew_handle"), 2);
    assert_eq!(
        run_in(
            &mut it,
            "(() => { try { api.decode({ __thaw_napi_binary__: '95', kind: 'DataView', buffer: { __thaw_napi_binary__: '96', kind: 'ArrayBuffer', data: [] }, array_type: 1, byte_offset: 0, length: 0 }); return 'made'; } catch (e) { return e.name + ':' + e.message; } })()"
        ),
        "TypeError:unsupported native binary kind"
    );
    assert_eq!(
        run_in(
            &mut it,
            "(() => { try { api.decode({ __thaw_napi_binary__: '97', buffer: { __thaw_napi_binary__: '98', kind: 'ArrayBuffer', data: [] }, array_type: 5, byte_offset: 0, length: 0 }); return 'made'; } catch (e) { return e.name + ':' + e.message; } })()"
        ),
        "TypeError:unsupported native typed array kind"
    );
}

#[test]
fn native_array_buffer_bytes_are_written_back_by_sync_arguments() {
    let _bridge = table_bridge();
    reply("sync_reference", "", r#"{"value":[7,7,7],"origins":[]}"#);
    let mut it = plain_realm();
    assert_eq!(
        run_in(
            &mut it,
            "(() => { const ab = new Uint8Array([1, 2, 3]).buffer; api.argument(ab); api.syncArguments([ab]); return new Uint8Array(ab).join(','); })()"
        ),
        "7,7,7"
    );
}

#[test]
fn native_promise_resolves_once_promise_state_stops_reporting_pending() {
    let _bridge = table_bridge();
    PENDING_POLLS.with(|p| p.set(3));
    let mut it = plain_realm();
    run_in(
        &mut it,
        "globalThis.settled = []; globalThis.same = api.decode({ __thaw_napi_promise__: '5' }) === api.decode({ __thaw_napi_promise__: '5' }); \
         api.decode({ __thaw_napi_promise__: '5' }).then(v => settled.push(v));",
    );
    assert_eq!(run_in(&mut it, "settled.length + '|' + same"), "0|true");
    it.run_event_loop().unwrap_or_else(|_| panic!("event loop"));
    assert_eq!(run_in(&mut it, "settled.join(',')"), "done");
    assert_eq!(op_count("promise_state"), 4);
}

thread_local! {
    /// The `(name, payload)` of every `bridge_call` the shim-style fake got on this thread.
    static E2E_CALLS: RefCell<Vec<(String, String)>> = const { RefCell::new(Vec::new()) };
    /// The exports listing and the `bridge_call` reply of the shim-style fake.
    static E2E_EXPORTS: RefCell<String> = const { RefCell::new(String::new()) };
    static E2E_CALL_REPLY: RefCell<String> = const { RefCell::new(String::new()) };
}

unsafe extern "C" fn e2e_exports() -> *const c_char {
    owned(&E2E_EXPORTS.with(|e| e.borrow().clone()))
}

unsafe extern "C" fn e2e_call(name: *const c_char, payload: *const c_char) -> *const c_char {
    let (name, payload) = (read(name), read(payload));
    E2E_CALLS.with(|c| c.borrow_mut().push((name, payload)));
    owned(&E2E_CALL_REPLY.with(|r| r.borrow().clone()))
}

/// The table bridge (owner 7, handle replies from `reply`) with the shim-style exports and call fake.
fn shim_bridge(exports: &str, call_reply: &str) -> FakeBridge {
    let bridge = table_bridge();
    E2E_EXPORTS.with(|e| *e.borrow_mut() = exports.to_string());
    E2E_CALL_REPLY.with(|r| *r.borrow_mut() = call_reply.to_string());
    E2E_CALLS.with(|c| c.borrow_mut().clear());
    clear_for_tests();
    thaw_tjs_register_napi_bridge(
        e2e_exports,
        e2e_call,
        table_handle,
        fake_poll,
        fake_pending,
        owner_seven,
        fake_release,
    );
    bridge
}

#[test]
fn shim_trusted_path_calls_an_export_through_the_bridge_call() {
    let _bridge = shim_bridge(
        r#"{"names":["addon::fn"],"qualifiedPackages":[],"ownerByExportName":{"addon::fn":"7"}}"#,
        r#"{"value":3}"#,
    );
    thaw_tjs_napi_enable_proxy_layer();
    // The shim's own wrapper shape: the trusted globals are read as `globalThis` properties.
    assert_eq!(
        eval(
            "(() => { const addon = {}; addon.fn = function () { return globalThis.__thaw_json_graph_napi_call_export('addon::fn', arguments); }; \
             return [addon.fn(1, {a: 1}), typeof globalThis.__thaw_json_graph_napi_bridge_available, \
             globalThis.__thaw_json_graph_napi_bridge_available()].join('|'); })()"
        ),
        "3|function|true"
    );
    assert_eq!(
        eval(
            "(() => { try { globalThis.__thaw_json_graph_napi_call_export('addon::missing', []); return 'no error'; } \
             catch (e) { return e.name + '|' + e.message; } })()"
        ),
        "TypeError|Native addon export has no live owner"
    );
    E2E_CALLS.with(|c| {
        let calls = c.borrow();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].0, "addon::fn");
        assert!(calls[0].1.starts_with("[1,{"), "{}", calls[0].1);
        assert!(calls[0].1.contains(r#""value":{"a":1}"#), "{}", calls[0].1);
    });
}

#[test]
fn shim_trusted_path_constructs_a_proxy_that_reads_through_the_bridge() {
    let _bridge = shim_bridge(
        r#"{"names":["addon::Cls"],"qualifiedPackages":[],"ownerByExportName":{"addon::Cls":"7"}}"#,
        r#"{"value":0}"#,
    );
    reply("construct", "", r#"{"value":"81"}"#);
    reply("array_kind_graph", "", r#"{"value":false}"#);
    reply("has_graph", r#"{"v":"x"}"#, r#"{"value":true}"#);
    reply("get_graph", r#"{"v":"x"}"#, &graph(r#"{"v":42}"#, ""));
    thaw_tjs_napi_enable_proxy_layer();
    assert_eq!(
        eval(
            "(() => { const addon = {}; addon.Cls = function () { if (new.target) return \
             globalThis.__thaw_json_graph_napi_construct('addon::Cls', arguments, new.target.prototype); }; \
             const p = new addon.Cls(5, 'b'); return p.x; })()"
        ),
        "42"
    );
    OPS.with(|o| {
        let ops = o.borrow();
        assert!(
            ops.iter().any(|(op, target, graph)| op == "construct"
                && target == "addon::Cls"
                && graph == r#"[5,"b"]"#),
            "{ops:?}"
        );
    });
    assert_eq!(op_count("get_graph"), 1);
}
