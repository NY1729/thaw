//! Unit tests for the N-API graph encoder in `napi_graph.rs` (PR 6a-3). Goldens are the QuickJS
//! `thawGraphEncode` bytes (thaw-quickjs api.rs `forced_root_guard_preserves_other_graph_modes`); the
//! round trip goes through `decode_napi_wire`.

use crate::host::*;
use crate::napi_graph::{decode_napi_wire, encode_napi_graph};
use crate::Value;
use std::ffi::CString;

fn q(text: &str) -> CString {
    CString::new(text).unwrap()
}

/// Run `source` in the default realm, then encode global `name` (live or not, mask, forced global).
fn encode_global(
    source: &str,
    name: &str,
    live: bool,
    mask: u32,
    force: Option<&str>,
) -> Result<String, String> {
    assert_eq!(thaw_tjs_load_script(q(source).as_ptr()), 1, "{source}");
    let owner = default_realm();
    with_package(owner, |p| {
        let it = &mut p.interp;
        let global = it.global.clone();
        let root = it.lookup(&global, name).unwrap_or(Value::Undefined);
        let force = force.map(|f| it.lookup(&global, f).unwrap_or(Value::Undefined));
        encode_napi_graph(it, owner, &root, mask, live, force)
    })
    .map_err(|e| e.to_string())
}

fn encode_ok(source: &str, name: &str, live: bool, mask: u32, force: Option<&str>) -> String {
    encode_global(source, name, live, mask, force).unwrap()
}

/// The `leases` list of an encoded wire.
fn leases_of(wire: &str) -> Vec<u64> {
    let v: serde_json::Value = serde_json::from_str(wire).unwrap();
    v["leases"]
        .as_array()
        .unwrap()
        .iter()
        .map(|x| x.as_u64().unwrap())
        .collect()
}

#[test]
fn napi_graph_encode_ordinary_object_matches_quickjs_bytes() {
    let source = "globalThis.ordinary = {x:1};";
    let golden =
        r#"{"root":{"r":0},"nodes":[{"o":[["x",{"v":1},7]]}],"leases":[],"napiLeases":[]}"#;
    assert_eq!(encode_ok(source, "ordinary", true, 0, None), golden);
    assert_eq!(encode_ok(source, "ordinary", false, 0, None), golden);
}

#[test]
fn napi_graph_encode_scalar_root_is_inline() {
    assert_eq!(
        encode_ok("globalThis.big = 12n;", "big", true, 0, None),
        r#"{"root":{"bi":"12"},"nodes":[],"leases":[],"napiLeases":[]}"#
    );
    assert_eq!(
        encode_ok(
            "globalThis.lone = String.fromCharCode(0xd800, 97);",
            "lone",
            true,
            0,
            None
        ),
        r#"{"root":{"su":[55296,97]},"nodes":[],"leases":[],"napiLeases":[]}"#
    );
}

#[test]
fn napi_graph_encode_nested_force_leases_each_reference() {
    let wire = encode_ok(
        "globalThis.nested_force = {}; globalThis.nested = [nested_force];",
        "nested",
        true,
        0,
        Some("nested_force"),
    );
    let leases = leases_of(&wire);
    assert_eq!(
        leases.len(),
        2,
        "entry and own property each transfer a lease"
    );
    let h = leases[0];
    assert_eq!(
        leases[1], h,
        "both nested tokens reuse the canonical handle"
    );
    assert_eq!(
        wire,
        format!(
            r#"{{"root":{{"r":0}},"nodes":[{{"a":[{{"hdl":{h}}}],"p":[["0",{{"hdl":{h}}},7],["length",{{"v":1}},1]]}}],"leases":[{h},{h}],"napiLeases":[]}}"#
        )
    );
}

#[test]
fn napi_graph_encode_mask_carries_root_item_by_handle() {
    let wire = encode_ok("globalThis.masked = [{}];", "masked", false, 1, None);
    let m = leases_of(&wire)[0];
    assert_eq!(
        wire,
        format!(
            r#"{{"root":{{"r":0}},"nodes":[{{"a":[{{"r":1}}],"p":[["0",{{"r":1}},7],["length",{{"v":1}},1]]}},{{"hdl":{m}}}],"leases":[{m}],"napiLeases":[]}}"#
        )
    );
}

#[test]
fn napi_graph_encode_root_equal_to_force_is_a_direct_handle() {
    let direct = encode_ok("globalThis.same = {a:1};", "same", true, 1, Some("same"));
    let h = leases_of(&direct)[0];
    assert_eq!(
        direct,
        format!(r#"{{"root":{{"hdl":{h}}},"nodes":[],"leases":[{h}],"napiLeases":[]}}"#)
    );
    // Without a mask, a live root equal to the force keeps the forced-root fast path.
    let fast = encode_ok("globalThis.same2 = {a:1};", "same2", true, 0, Some("same2"));
    let h2 = leases_of(&fast)[0];
    assert_eq!(
        fast,
        format!(r#"{{"root":{{"r":0}},"nodes":[{{"hdl":{h2}}}],"leases":[{h2}],"napiLeases":[]}}"#)
    );
}

#[test]
fn napi_graph_encode_accessor_root_stays_a_handle() {
    let wire = encode_ok(
        "globalThis.acc = { get g() { return 1; } };",
        "acc",
        true,
        0,
        None,
    );
    let h = leases_of(&wire)[0];
    assert_eq!(
        wire,
        format!(r#"{{"root":{{"r":0}},"nodes":[{{"hdl":{h}}}],"leases":[{h}],"napiLeases":[]}}"#)
    );
}

#[test]
fn napi_graph_encode_symbol_key_is_a_handle_node_and_cycle_reuses_root() {
    let wire = encode_ok(
        "globalThis.sym_root = { self: null, [Symbol('k')]: 2 }; sym_root.self = sym_root;",
        "sym_root",
        true,
        0,
        None,
    );
    let v: serde_json::Value = serde_json::from_str(&wire).unwrap();
    let nodes = v["nodes"].as_array().unwrap();
    assert_eq!(nodes.len(), 2);
    assert_eq!(nodes[0]["o"][0], serde_json::json!(["self", {"r": 0}, 7]));
    assert_eq!(nodes[0]["o"][1][0], serde_json::json!({"r": 1}));
    assert_eq!(nodes[0]["o"][1][1], serde_json::json!({"v": 2}));
    assert!(nodes[1]["hdl"].as_u64().is_some());
    assert_eq!(leases_of(&wire).len(), 1);
}

#[test]
fn napi_graph_encode_non_live_symbol_is_unsupported() {
    let err = encode_global(
        "globalThis.bad = {[Symbol('x')]: 1};",
        "bad",
        false,
        0,
        None,
    )
    .unwrap_err();
    assert!(
        err.contains("Unsupported native callback graph value"),
        "{err}"
    );
}

#[test]
fn napi_graph_encode_root_bytes_and_date_are_copied() {
    assert_eq!(
        encode_ok(
            "globalThis.u8 = new Uint8Array([1, 2]);",
            "u8",
            true,
            0,
            None
        ),
        r#"{"root":{"r":0},"nodes":[{"b":[1,2]}],"leases":[],"napiLeases":[]}"#
    );
    assert_eq!(
        encode_ok("globalThis.when = new Date(1000);", "when", true, 0, None),
        r#"{"root":{"r":0},"nodes":[{"d":1000}],"leases":[],"napiLeases":[]}"#
    );
}

#[test]
fn napi_graph_encode_round_trips_through_the_decoder() {
    let wire = encode_ok(
        "globalThis.cyc = {x:1}; cyc.self = cyc;",
        "cyc",
        true,
        0,
        None,
    );
    assert_eq!(
        wire,
        r#"{"root":{"r":0},"nodes":[{"o":[["x",{"v":1},7],["self",{"r":0},7]]}],"leases":[],"napiLeases":[]}"#
    );
    assert_eq!(
        thaw_tjs_load_script(
            q("function __probe(o) { return [o.self === o, o.x].join('|'); }").as_ptr()
        ),
        1
    );
    let (owner, value) = decode_napi_wire(&wire).unwrap();
    let text = with_package(owner, |p| {
        let it = &mut p.interp;
        let global = it.global.clone();
        let f = it.lookup(&global, "__probe").unwrap_or(Value::Undefined);
        let result = it
            .call(&f, &Value::Undefined, &[value])
            .map_err(|c| classify(it, c))?;
        Ok(it.to_str(&result).to_string())
    })
    .map_err(|e| e.to_string())
    .unwrap();
    assert_eq!(text, "true|1");
}
