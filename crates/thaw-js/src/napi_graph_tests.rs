//! Unit tests for the N-API graph wire decoder in `napi_graph.rs`. Probes are JS functions run in the
//! default realm; a decoded value is passed to the probe and its string result compared.

use crate::host::*;
use crate::napi_graph::decode_napi_wire;
use crate::Value;
use std::ffi::CString;

fn q(text: &str) -> CString {
    CString::new(text).unwrap()
}

/// Run a script in the default realm.
fn load(source: &str) {
    assert_eq!(thaw_tjs_load_script(q(source).as_ptr()), 1, "{source}");
}

/// Decode `wire` with the N-API rules and pass the value to `probe` (a JS function declaration named
/// `__probe`); the probe's result as text.
fn probe(wire: &str, source: &str) -> String {
    load(source);
    let (owner, value) = decode_napi_wire(wire).unwrap();
    with_package(owner, |p| {
        let it = &mut p.interp;
        let global = it.global.clone();
        let f = it.lookup(&global, "__probe").unwrap_or(Value::Undefined);
        let result = it
            .call(&f, &Value::Undefined, &[value])
            .map_err(|c| classify(it, c))?;
        Ok(it.to_str(&result).to_string())
    })
    .map_err(|e| e.to_string())
    .unwrap()
}

#[test]
fn napi_graph_array_props_keep_order_flags_and_cycle() {
    let wire = r#"{"root":{"r":0},"nodes":[{"a":[{"v":7},{"h":1}],"p":[["0",{"v":7},6],["length",{"v":2},1],["self",{"r":0},0],["named",{"v":"saved"},7]]}],"leases":[],"napiLeases":[]}"#;
    assert_eq!(
        probe(
            wire,
            "function __probe(a) { const ref = [7, ,]; Object.defineProperty(ref, 'self', { value: ref, enumerable: false, writable: false }); ref.named = 'saved'; const s = Object.getOwnPropertyDescriptor(a, 'self'); return [a.length, 1 in a, Reflect.ownKeys(a).join(',') === Reflect.ownKeys(ref).join(','), s.value === a, s.enumerable, s.writable, a.named].join('|'); }"
        ),
        "2|false|true|true|false|false|saved"
    );
}

#[test]
fn napi_graph_object_attributes_and_cycle() {
    let wire = r#"{"root":{"r":0},"nodes":[{"o":[["self",{"r":0},7],["ro",{"v":1},0],["hid",{"v":2},2]]}],"leases":[],"napiLeases":[]}"#;
    assert_eq!(
        probe(
            wire,
            "function __probe(o) { return [o.self === o, o.ro, o.hid, Object.keys(o).join(','), Object.getOwnPropertyDescriptor(o, 'ro').writable, Object.getOwnPropertyDescriptor(o, 'hid').enumerable].join('|'); }"
        ),
        "true|1|2|self,hid|false|true"
    );
}

#[test]
fn napi_graph_lone_surrogate_token_and_plain_wire_rejects_it() {
    let wire = r#"{"root":{"su":[97,55296]},"nodes":[],"leases":[],"napiLeases":[]}"#;
    assert_eq!(
        probe(
            wire,
            "function __probe(s) { return [s.length, s.charCodeAt(1)].join('|'); }"
        ),
        "2|55296"
    );
    // The plain wire keeps rejecting `su`, as before.
    assert!(decode_wire(wire).is_err());
}

#[test]
fn napi_graph_array_cycle_resolves_and_plain_wire_still_rejects_it() {
    let wire = r#"{"root":{"r":0},"nodes":[{"a":[{"r":0},{"v":1}]}],"leases":[],"napiLeases":[]}"#;
    assert_eq!(
        probe(
            wire,
            "function __probe(a) { return [a[0] === a, a[1]].join('|'); }"
        ),
        "true|1"
    );
    let Err(error) = decode_wire(wire) else {
        panic!("cycle accepted by the plain wire")
    };
    assert!(error.contains("cyclic graph is not supported"), "{error}");
}

/// Two references are retained for the wire (the store's and the lease's); after the decode exactly the
/// lease is gone, so the handle's remaining reference releases to zero on the second release.
fn assert_one_reference_left(handle: i64) {
    assert_eq!(thaw_tjs_release_handle(handle), 1);
    assert_eq!(thaw_tjs_release_handle(handle), 0);
}

#[test]
fn napi_graph_releases_leases_on_success_and_on_error() {
    let owner = default_realm();
    load("function __probe(v) { return 'ok'; }");

    let ok = store_handle(owner, Value::Number(1.0));
    assert_eq!(thaw_tjs_retain_handle(ok), 1);
    let wire = format!(
        r#"{{"root":{{"r":0}},"nodes":[{{"hdl":{ok}}}],"leases":[{ok}],"napiLeases":["7"]}}"#
    );
    assert!(decode_napi_wire(&wire).is_ok());
    assert_one_reference_left(ok);

    let failed = store_handle(owner, Value::Number(2.0));
    assert_eq!(thaw_tjs_retain_handle(failed), 1);
    // The second node is malformed, after the first (which holds the lease) has decoded.
    let wire = format!(
        r#"{{"root":{{"a":[{{"r":0}},{{"r":1}}]}},"nodes":[{{"hdl":{failed}}},{{"x":1}}],"leases":[{failed}],"napiLeases":[]}}"#
    );
    assert!(decode_napi_wire(&wire).is_err());
    assert_one_reference_left(failed);
}

#[test]
fn napi_graph_wire_without_leases_releases_handles_in_node() {
    let owner = default_realm();
    let h = store_handle(owner, Value::Number(3.0));
    assert_eq!(thaw_tjs_retain_handle(h), 1);
    let wire = format!(r#"{{"root":{{"r":0}},"nodes":[{{"hdl":{h}}}]}}"#);
    assert!(decode_napi_wire(&wire).is_ok());
    assert_one_reference_left(h);
}

// ---- Native tags, pair proofs and lone tokens (PR 6a-2) ----

use crate::napi_bridge::{clear_for_tests, thaw_tjs_register_napi_bridge};
use crate::napi_graph::{encode_napi_graph, native_pair_matches, tag, Tag};
use std::ffi::{c_char, CStr};

fn read(ptr: *const c_char) -> String {
    unsafe { CStr::from_ptr(ptr) }
        .to_string_lossy()
        .into_owned()
}

fn owned(text: &str) -> *const c_char {
    CString::new(text).unwrap().into_raw() as *const c_char
}

unsafe extern "C" fn fake_exports() -> *const c_char {
    owned("{\"names\":[]}")
}

unsafe extern "C" fn fake_call(_: *const c_char, _: *const c_char) -> *const c_char {
    owned("")
}

/// `symbol_metadata` names the Symbol id 5 for target 5 and id 99 otherwise; target 7 fails.
unsafe extern "C" fn fake_handle(
    operation: *const c_char,
    target: *const c_char,
    _: *const c_char,
    _: *const c_char,
) -> *const c_char {
    let (operation, target) = (read(operation), read(target));
    let reply = match (operation.as_str(), target.as_str()) {
        (_, "7") => r#"{"__thaw_error__":"boom"}"#.to_string(),
        ("retain_graph_handle", _) => r#"{"value":"73"}"#.to_string(),
        ("symbol_handle", t) => format!(r#"{{"value":"{t}"}}"#),
        ("symbol_metadata", t) => {
            let id = if t == "5" { "5" } else { "99" };
            format!(
                r#"{{"value":{{"id":"{id}","handle":"{t}","units":[104,105],"registered":false,"jsOrigin":false}}}}"#
            )
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

extern "C" fn fake_release(_: u64) -> u8 {
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

/// A global of the default realm, by name.
fn global_value(owner: usize, name: &str) -> Value {
    with_package(owner, |p| {
        let it = &mut p.interp;
        let global = it.global.clone();
        Ok(it.lookup(&global, name).unwrap_or(Value::Undefined))
    })
    .unwrap()
}

#[test]
fn native_pair_matches_proves_tagged_functions_and_bridge_confirmed_symbols() {
    let _bridge = fake_bridge();
    let owner = default_realm();
    load("globalThis.tagged = function() {}; globalThis.named = Symbol('hi');");
    let function = global_value(owner, "tagged");
    let Value::Object(object) = &function else {
        panic!("tagged is not an object")
    };
    tag(object, Tag::Function, "9");
    assert!(native_pair_matches(Some(owner as u64), &function, "9", 1));
    assert!(!native_pair_matches(Some(owner as u64), &function, "8", 1));
    assert!(!native_pair_matches(None, &function, "9", 2));

    let symbol = global_value(owner, "named");
    let Value::Object(symbol_object) = &symbol else {
        panic!("named is not an object")
    };
    crate::napi_symbols::bind(owner as u64, 5, symbol_object);
    assert!(native_pair_matches(Some(owner as u64), &symbol, "5", 2));
    // The reply names another id for target 6, and fails for target 7.
    assert!(!native_pair_matches(Some(owner as u64), &symbol, "6", 2));
    assert!(!native_pair_matches(Some(owner as u64), &symbol, "7", 2));
    assert!(!native_pair_matches(Some(owner as u64), &function, "9", 2));
}

#[test]
fn handle_op_returns_the_reply_or_the_bridge_error() {
    let _bridge = fake_bridge();
    let reply = crate::napi_bridge::handle_op("retain_graph_handle", "9", "", "[]").unwrap();
    assert_eq!(reply["value"], "73");
    assert_eq!(
        crate::napi_bridge::handle_op("symbol_metadata", "7", "", "[]").unwrap_err(),
        "boom"
    );
}

#[test]
fn napi_function_pair_releases_its_lease_and_decodes_only_when_proven() {
    let _bridge = fake_bridge();
    let owner = default_realm();
    load("globalThis.plain = function() {}; globalThis.tagged2 = function() {};");
    let plain = global_value(owner, "plain");
    let h = store_handle(owner, plain);
    assert_eq!(thaw_tjs_retain_handle(h), 1);
    let wire = format!(
        r#"{{"root":{{"r":0}},"nodes":[{{"nfn":"9","hdl":{h}}}],"leases":[{h}],"napiLeases":[]}}"#
    );
    let err = decode_napi_wire(&wire).err().expect("wire rejected");
    assert!(
        err.contains("Mismatched native Function graph node"),
        "{err}"
    );
    assert_one_reference_left(h);

    let tagged = global_value(owner, "tagged2");
    let Value::Object(object) = &tagged else {
        panic!("tagged2 is not an object")
    };
    tag(object, Tag::Function, "9");
    let h = store_handle(owner, tagged.clone());
    assert_eq!(thaw_tjs_retain_handle(h), 1);
    let wire = format!(
        r#"{{"root":{{"r":0}},"nodes":[{{"nfn":"9","hdl":{h}}}],"leases":[{h}],"napiLeases":[]}}"#
    );
    let (_, decoded) = decode_napi_wire(&wire).unwrap();
    let Value::Object(decoded) = decoded else {
        panic!("decoded value is not a function")
    };
    let Value::Object(original) = &tagged else {
        unreachable!()
    };
    assert!(std::rc::Rc::ptr_eq(&decoded, original));
    assert_one_reference_left(h);
}

#[test]
fn napi_symbol_tokens_create_pin_and_prove_their_native_id() {
    let _bridge = fake_bridge();
    let owner = default_realm();
    // A lone `{"nsy"}` creates the Symbol the bridge describes ("hi") and pins it.
    let wire = r#"{"root":{"nsy":"5"},"nodes":[],"leases":[],"napiLeases":[]}"#;
    assert_eq!(
        probe(
            wire,
            "function __probe(s) { return [typeof s, s.description].join('|'); }"
        ),
        "symbol|hi"
    );
    assert_eq!(
        crate::napi_symbols::thaw_tjs_napi_symbol_weak_live(owner as u64, 5),
        1
    );
    let (_, first) = decode_napi_wire(wire).unwrap();

    // The pair proves the id against the handle's Symbol, and releases the handle's lease.
    let h = store_handle(owner, first.clone());
    assert_eq!(thaw_tjs_retain_handle(h), 1);
    let pair =
        format!(r#"{{"root":{{"nsy":"5","hdl":{h}}},"nodes":[],"leases":[{h}],"napiLeases":[]}}"#);
    let (_, again) = decode_napi_wire(&pair).unwrap();
    let (Value::Object(a), Value::Object(b)) = (&first, &again) else {
        panic!("symbols expected")
    };
    assert!(std::rc::Rc::ptr_eq(a, b));
    assert_one_reference_left(h);

    // Another id names no Symbol of this handle: rejected, and the lease still released.
    let h = store_handle(owner, first);
    assert_eq!(thaw_tjs_retain_handle(h), 1);
    let forged =
        format!(r#"{{"root":{{"nsy":"6","hdl":{h}}},"nodes":[],"leases":[{h}],"napiLeases":[]}}"#);
    let err = decode_napi_wire(&forged).err().expect("wire rejected");
    assert!(
        err.contains("Mismatched native Symbol graph token"),
        "{err}"
    );
    assert_one_reference_left(h);
}

#[test]
fn lone_native_handle_and_function_tokens_need_the_proxy_layer() {
    let _bridge = fake_bridge();
    let wire = r#"{"root":{"r":0},"nodes":[{"nfn":"9"}],"leases":[],"napiLeases":[]}"#;
    let err = decode_napi_wire(wire).err().expect("wire rejected");
    assert!(
        err.contains("Native addon proxy layer is not installed"),
        "{err}"
    );
    load("globalThis.__thaw_json_graph_proxy_for_handle = n => ({ n });");
    let wire = r#"{"root":{"r":0},"nodes":[{"nh":"9"}],"leases":[],"napiLeases":[]}"#;
    assert_eq!(probe(wire, "function __probe(o) { return o.n; }"), "9");
}

#[test]
fn napi_graph_tag_global_refuses_bad_kinds_and_owner_alias_answers() {
    let _bridge = fake_bridge();
    let wire = r#"{"root":{"v":1},"nodes":[],"leases":[],"napiLeases":[]}"#;
    assert_eq!(
        probe(
            wire,
            "function __probe(x) { const f = function() {}; \
             let refused = ''; try { __thaw_napi_graph_tag(f, 3, '9'); } catch (e) { refused = e.name; } \
             return [refused, typeof __thaw_json_graph_owner_of_handle].join('|'); }"
        ),
        "TypeError|function"
    );
}

// ---- Encoder native tags and napi leases (PR 6a-4) ----

/// Encode the default-realm global `name` as a live root, with the fake bridge already registered.
fn encode_named(name: &str) -> Result<String, String> {
    let owner = default_realm();
    with_package(owner, |p| {
        let it = &mut p.interp;
        let global = it.global.clone();
        let root = it.lookup(&global, name).unwrap_or(Value::Undefined);
        encode_napi_graph(it, owner, &root, 0, true, None)
    })
    .map_err(|e| e.to_string())
}

/// Tag the default-realm global `name` as the native `kind` of `native`.
fn tag_global(name: &str, kind: Tag, native: &str) {
    let value = global_value(default_realm(), name);
    let Value::Object(object) = &value else {
        panic!("{name} is not an object")
    };
    tag(object, kind, native);
}

#[test]
fn napi_graph_encode_tagged_function_root_is_nfn_with_a_napi_lease() {
    let _bridge = fake_bridge();
    load("globalThis.native_fn = function() {};");
    tag_global("native_fn", Tag::Function, "9");
    let wire = encode_named("native_fn").unwrap();
    let v: serde_json::Value = serde_json::from_str(&wire).unwrap();
    let h = v["leases"][0].as_u64().unwrap();
    assert_eq!(v["root"], serde_json::json!({"r": 0}));
    assert_eq!(v["nodes"], serde_json::json!([{"nfn": "9", "hdl": h}]));
    assert_eq!(v["leases"], serde_json::json!([h]));
    assert_eq!(v["napiLeases"], serde_json::json!(["73"]));
    // The wire decodes back to the same function: the pair is proven by the tag.
    let (_, back) = decode_napi_wire(&wire).unwrap();
    let original = global_value(default_realm(), "native_fn");
    let (Value::Object(a), Value::Object(b)) = (&original, &back) else {
        panic!("functions expected")
    };
    assert!(std::rc::Rc::ptr_eq(a, b));
}

#[test]
fn napi_graph_encode_tagged_handle_is_nh_and_takes_no_host_lease() {
    let _bridge = fake_bridge();
    load("globalThis.proxy = {};");
    tag_global("proxy", Tag::Handle, "9");
    assert_eq!(
        encode_named("proxy").unwrap(),
        r#"{"root":{"r":0},"nodes":[{"nh":"9"}],"leases":[],"napiLeases":["73"]}"#
    );
    // Inside a copied object the tagged handle is its own node.
    load("globalThis.holder = { p: proxy };");
    assert_eq!(
        encode_named("holder").unwrap(),
        r#"{"root":{"r":0},"nodes":[{"o":[["p",{"r":1},7]]},{"nh":"9"}],"leases":[],"napiLeases":["73"]}"#
    );
}

#[test]
fn napi_graph_encode_native_symbol_key_is_nsy_with_handle_and_napi_lease() {
    let _bridge = fake_bridge();
    load("globalThis.hi = Symbol('hi'); globalThis.keyed = { [hi]: 1 };");
    let symbol = global_value(default_realm(), "hi");
    let Value::Object(object) = &symbol else {
        panic!("hi is not an object")
    };
    crate::napi_symbols::bind(default_realm() as u64, 5, object);
    let wire = encode_named("keyed").unwrap();
    let v: serde_json::Value = serde_json::from_str(&wire).unwrap();
    let h = v["leases"][0].as_u64().unwrap();
    assert_eq!(v["leases"], serde_json::json!([h]));
    assert_eq!(v["napiLeases"], serde_json::json!(["73"]));
    assert_eq!(
        v["nodes"][0]["o"][0][0],
        serde_json::json!({"nsy": "5", "hdl": h})
    );
    assert_eq!(v["nodes"][0]["o"][0][1], serde_json::json!({"v": 1}));
}

#[test]
fn napi_graph_encode_failed_retain_is_an_error_and_names_no_tag() {
    let _bridge = fake_bridge();
    // Target 7 fails in the fake bridge: the second tagged function cannot be retained.
    load("globalThis.good_fn = function() {}; globalThis.bad_fn = function() {}; globalThis.pair = { f: good_fn, g: bad_fn };");
    tag_global("good_fn", Tag::Function, "9");
    tag_global("bad_fn", Tag::Function, "7");
    let err = encode_named("pair").unwrap_err();
    assert!(err.contains("Cannot retain native graph handle"), "{err}");
}
