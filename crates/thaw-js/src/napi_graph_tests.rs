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
