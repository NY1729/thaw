//! The `__thaw_*` helper globals the compiler calls by name (`getDynamicValue("__thaw_...")`) must exist
//! in the thaw-js default realm, and the ported helpers must match Node 22 (values recorded with `node`).
//!
//! The contract iterates `thaw_hir::compiler_helpers::COMPILER_EMITTED_JS_HELPERS`, the names every
//! compiler emitter reads by name, so a new emitted helper fails this test until thaw-js defines it.

use super::*;
use std::path::Path;

/// The default realm as `thaw_tjs_get_global` sees it: the dynamic prelude is loaded on first use.
fn default_realm() -> Interp {
    let mut it = Interp::new();
    if it
        .lookup(&it.global.clone(), "__thaw_json_host_enumerate")
        .is_none()
    {
        assert!(it.run_sloppy(include_str!("dynamic_prelude.js")).is_ok());
    }
    it
}

/// Run `src` in a fresh default realm and return its console.log lines.
fn run_realm(src: &str) -> Vec<String> {
    let mut it = default_realm();
    if let Err(Ctl::Throw(v)) = it.run(src) {
        let message = it.get(&v, "message").unwrap_or(Value::Undefined);
        panic!("helper program threw: {}", it.to_str(&message));
    }
    it.out
}

/// Typeof of `name` in the default realm, looked up exactly as `thaw_tjs_get_global` does.
fn realm_typeofs(names: &[&str]) -> Vec<String> {
    let mut it = default_realm();
    let global = it.global.clone();
    let mut kinds = Vec::new();
    for name in names {
        let defined = it
            .lookup(&global, name)
            .filter(|v| !matches!(v, Value::Undefined));
        let kind = match defined {
            None => "undefined".to_string(),
            Some(_) => {
                it.out.clear();
                it.run(&format!("console.log(typeof {name})")).ok();
                it.out.last().cloned().unwrap_or_default()
            }
        };
        kinds.push(kind);
    }
    kinds
}

/// Every name the compiler looks up by name is either a function in the thaw-js default realm, a defined
/// JS global, or a host-provided name (absent from the realm, with a provider file that defines it).
#[test]
fn compiler_emitted_names_resolve_in_default_realm_or_host() {
    use thaw_hir::compiler_helpers::{
        COMPILER_EMITTED_JS_HELPERS, HOST_PROVIDED_HELPERS, JS_GLOBALS,
    };
    let helper_kinds = realm_typeofs(COMPILER_EMITTED_JS_HELPERS);
    let missing_helpers: Vec<&str> = COMPILER_EMITTED_JS_HELPERS
        .iter()
        .zip(&helper_kinds)
        .filter(|(_, kind)| kind.as_str() != "function")
        .map(|(name, _)| *name)
        .collect();
    let global_kinds = realm_typeofs(JS_GLOBALS);
    let missing_globals: Vec<&str> = JS_GLOBALS
        .iter()
        .zip(&global_kinds)
        .filter(|(_, kind)| kind.as_str() == "undefined")
        .map(|(name, _)| *name)
        .collect();
    let host_names: Vec<&str> = HOST_PROVIDED_HELPERS
        .iter()
        .map(|(name, _)| *name)
        .collect();
    let host_kinds = realm_typeofs(&host_names);
    let mut stale_host = Vec::new();
    let mut unprovided = Vec::new();
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    for ((name, provider), kind) in HOST_PROVIDED_HELPERS.iter().zip(&host_kinds) {
        if kind != "undefined" {
            stale_host.push(*name);
        }
        let text = std::fs::read_to_string(manifest.join("../..").join(provider).as_path())
            .unwrap_or_default();
        if !text.contains(name) {
            unprovided.push(*name);
        }
    }
    assert!(
        missing_helpers.is_empty() && missing_globals.is_empty(),
        "compiler-emitted names not resolvable in the thaw-js default realm: \
         helpers not functions {missing_helpers:?}, globals undefined {missing_globals:?}"
    );
    assert!(
        stale_host.is_empty() && unprovided.is_empty(),
        "host-provided list stale: defined in realm {stale_host:?}, provider file lacks {unprovided:?}"
    );
}

#[test]
fn json_stringify_helpers_match_node_and_drop_symbol_keys() {
    // Node 22: JSON.stringify with a function replacer, an indent, and a real Symbol key (dropped).
    let out = run_realm(
        r#"
        const sym = Symbol('secret');
        const o = { a: 1, [sym]: 2, b: [1, { c: 3 }] };
        const double = function (k, v) { return typeof v === 'number' ? v * 10 : v; };
        console.log(__thaw_json_stringify_live_replacer(null, o, double));
        console.log(__thaw_json_stringify_live_replacer(2, { x: 5, y: [1, 2] }, function (k, v) { return k === 'x' ? undefined : v; }));
        console.log(__thaw_json_stringify_native_accessors(null, null, o));
        console.log(__thaw_json_stringify_native_accessors(['b'], null, o));
        console.log(__thaw_json_stringify_native_accessors(null, '--', { g: 7 }));
        console.log(__thaw_json_stringify_live_dynamic_replacer(null, { a: 1, b: 2 }, ['b']));
        console.log(__thaw_json_stringify_live_dynamic_replacer(null, { a: 1, b: 2 }, function (k, v) { return k === 'a' ? undefined : v; }));
        "#,
    );
    assert_eq!(
        out,
        // console.log lines: a multi-line JSON text is one line per output line
        vec![
            r#"{"a":10,"b":[10,{"c":30}]}"#,
            "{",
            "  \"y\": [",
            "    1,",
            "    2",
            "  ]",
            "}",
            r#"{"a":1,"b":[1,{"c":3}]}"#,
            r#"{"b":[1,{}]}"#,
            "{",
            "--\"g\": 7",
            "}",
            r#"{"b":2}"#,
            r#"{"b":2}"#,
        ]
    );
}

#[test]
fn json_stringify_drops_symbol_wire_keys_like_quickjs() {
    // A `\u0003<id>:desc` key is symbol wire text, never data: every stringify variant drops it.
    let out = run_realm(
        r#"
        const wire = { a: 1, '\u00031:secret': 2 };
        const fn = function (k, v) { return v; };
        console.log(__thaw_json_stringify_direct(wire));
        console.log(__thaw_json_stringify_live_replacer(null, wire, fn));
        console.log(__thaw_json_stringify_native_accessors(null, null, wire));
        console.log(__thaw_json_stringify_live_dynamic_replacer(null, wire, fn));
        "#,
    );
    assert_eq!(out, vec![r#"{"a":1}"#; 4]);
}

#[test]
fn descriptor_helpers_match_node_shapes() {
    let out = run_realm(
        r#"
        const g = () => 1, st = () => 2;
        const a = __thaw_accessor_descriptor(true, true, 6, g, st);
        const b = __thaw_accessor_descriptor(true, false, 0, g, st);
        const d = __thaw_data_descriptor(9);
        const n = __thaw_data_descriptor(9, 0);
        console.log([Object.keys(a).join(), a.get === g, a.set === st, a.enumerable, a.configurable].join(' '));
        console.log([Object.keys(b).join(), b.get === g, String(b.set), b.enumerable, b.configurable].join(' '));
        console.log([Object.keys(d).join(), d.value, d.writable, d.enumerable, d.configurable].join(' '));
        console.log([n.value, n.writable, n.enumerable, n.configurable].join(' '));
        "#,
    );
    assert_eq!(
        out,
        vec![
            "get,set,enumerable,configurable true true true true",
            "get,set,enumerable,configurable true undefined false false",
            "value,writable,enumerable,configurable 9 true true true",
            "9 false false false",
        ]
    );
}

#[test]
fn number_and_date_helpers_match_node() {
    let out = run_realm(
        r#"
        console.log([__thaw_number_constructor('3'), __thaw_number_constructor(true),
          __thaw_number_constructor(null), __thaw_number_constructor(undefined)].join(' '));
        console.log(__thaw_date_from_live_value(1700000000000));
        console.log(__thaw_date_from_live_value(new Date(1700000000000)));
        console.log(__thaw_date_from_live_value(NaN));
        "#,
    );
    assert_eq!(
        out,
        vec!["3 1 0 NaN", "1700000000000", "1700000000000", "NaN"]
    );
}

#[test]
fn property_key_maps_wire_text_to_cached_symbols() {
    let out = run_realm(
        r#"
        const key = __thaw_property_key('\u00031:secret');
        console.log(typeof key, key.description, key === __thaw_property_key('\u00031:secret'));
        console.log(__thaw_property_key('\u001f@@toPrimitive') === Symbol.toPrimitive);
        console.log(__thaw_property_key('\u0003R7:reg') === Symbol.for('reg'));
        console.log(__thaw_property_key('plain'));
        console.log(__thaw_property_key('\u0003Rx:bad'));
        "#,
    );
    assert_eq!(
        out,
        vec!["symbol secret true", "true", "true", "plain", "\u{3}Rx:bad",]
    );
}

#[test]
fn object_literal_symbol_keys_and_methods_match_node() {
    // Node 22: { [sym]: 2, [Symbol.toPrimitive]: () => 42 } and method names after their keys.
    let out = run_realm(
        r#"
        const o = __thaw_object_from_operations(['data', 'data'], '["\\u00031:secret","\\u001f@@toPrimitive"]',
          [true, true], [2, () => 42]);
        console.log([Object.keys(o).length, Object.getOwnPropertySymbols(o).length, o + 1, +o, `${o}`].join(' '));
        const m = __thaw_object_from_operations(['method', 'method'], '["f","\\u001f@@toPrimitive"]',
          [false, true], [0, 0], () => 1, () => 2);
        console.log([m.f.name, Object.getOwnPropertySymbols(m).map(s => m[s].name).join()].join(' '));
        "#,
    );
    assert_eq!(out, vec!["0 2 43 42 42", "f [Symbol.toPrimitive]"]);
}

/// A class projection's symbol-keyed method arrives as wire text (`\u001f@@toPrimitive`); the
/// projection maps it to `Symbol.toPrimitive`, so `in` and ToPrimitive (through the Proxy `get` trap) see it.
#[test]
fn dynamic_projection_maps_symbol_wire_keys() {
    // callbacks: [method getter, method setter, extensible, sealed, frozen, setState, propertyFlags, setFlags, canFlags]
    let src = r#"
      var cb = [() => h => 'Q:' + h, () => undefined, () => true, () => false, () => false, () => true, () => 7, () => true, () => true];
      var w = __thaw_object_with_native_getters(['\u001f@@toPrimitive'], [false], [false], [false], [true], '__thawmod0_Q', ...cb);
      console.log(Symbol.toPrimitive in w, w + '', `${w}`, w * 1);
    "#;
    // One console.log line; Node 22 prints `true Q:default Q:string NaN`.
    assert_eq!(run_realm(src), vec!["true Q:default Q:string NaN"]);
}
