use super::*;

/// Run `src` and join its console.log lines with `|` (or the uncaught error text).
fn one(src: &str) -> String {
    let mut it = Interp::new();
    match it.run(src) {
        Ok(_) => it.out.join("|"),
        Err(Ctl::Throw(v)) => format!("Uncaught {}", it.to_str(&v)),
        Err(_) => "stray completion".into(),
    }
}

// Guards for the reported `configurable` attribute (writable, enumerable, configurable). Each
// expectation matches Node.

#[test]
fn configurable_defaults_for_assignment_literal_and_class_members() {
    assert_eq!(
        one("const o = {}; o.x = 1; const d = Object.getOwnPropertyDescriptor(o, 'x'); console.log(d.writable, d.enumerable, d.configurable);"),
        "true true true"
    );
    assert_eq!(
        one("const o = { a: 1 }; console.log(Object.getOwnPropertyDescriptor(o, 'a').configurable);"),
        "true"
    );
    assert_eq!(
        one("class C { m() {} } const d = Object.getOwnPropertyDescriptor(C.prototype, 'm'); console.log(d.writable, d.enumerable, d.configurable);"),
        "true false true"
    );
    assert_eq!(
        one("const d = Object.getOwnPropertyDescriptor(Array.prototype, 'map'); console.log(d.writable, d.enumerable, d.configurable);"),
        "true false true"
    );
}

#[test]
fn configurable_defaults_for_define_property() {
    // a new property defined without `configurable` is non-configurable
    assert_eq!(
        one("const o = {}; Object.defineProperty(o, 'x', { value: 1 }); const d = Object.getOwnPropertyDescriptor(o, 'x'); console.log(d.writable, d.enumerable, d.configurable);"),
        "false false false"
    );
    assert_eq!(
        one("const o = {}; Object.defineProperty(o, 'x', { value: 1, writable: true, enumerable: true, configurable: true }); const d = Object.getOwnPropertyDescriptor(o, 'x'); console.log(d.writable, d.enumerable, d.configurable);"),
        "true true true"
    );
    assert_eq!(
        one("const o = {}; Object.defineProperty(o, 'g', { get() { return 1; } }); const d = Object.getOwnPropertyDescriptor(o, 'g'); console.log(d.enumerable, d.configurable);"),
        "false false"
    );
    // redefining keeps the existing flag when the descriptor does not mention it
    assert_eq!(
        one("const o = {}; Object.defineProperty(o, 'x', { value: 1, configurable: true }); Object.defineProperty(o, 'x', { value: 2 }); console.log(Object.getOwnPropertyDescriptor(o, 'x').configurable);"),
        "true"
    );
    assert_eq!(
        one("const o = {}; Object.defineProperty(o, 'x', { value: 1, configurable: true }); Object.defineProperty(o, 'x', { configurable: false }); console.log(Object.getOwnPropertyDescriptor(o, 'x').configurable);"),
        "false"
    );
}

#[test]
fn configurable_for_arrays_functions_and_constants() {
    assert_eq!(
        one("const d = Object.getOwnPropertyDescriptor([1, 2], 'length'); console.log(d.writable, d.enumerable, d.configurable);"),
        "true false false"
    );
    assert_eq!(
        one("console.log(Object.getOwnPropertyDescriptor([1], '0').configurable);"),
        "true"
    );
    assert_eq!(
        one("function f() {} const d = Object.getOwnPropertyDescriptor(f, 'prototype'); console.log(d.writable, d.enumerable, d.configurable);"),
        "true false false"
    );
    assert_eq!(
        one("const d = Object.getOwnPropertyDescriptor(Math, 'PI'); console.log(d.writable, d.enumerable, d.configurable);"),
        "false false false"
    );
}

#[test]
fn configurable_follows_freeze_and_seal() {
    assert_eq!(
        one("const d = Object.getOwnPropertyDescriptor(Object.freeze({ a: 1 }), 'a'); console.log(d.writable, d.enumerable, d.configurable);"),
        "false true false"
    );
    assert_eq!(
        one("const d = Object.getOwnPropertyDescriptor(Object.seal({ a: 1 }), 'a'); console.log(d.writable, d.enumerable, d.configurable);"),
        "true true false"
    );
    assert_eq!(
        one("const d = Object.getOwnPropertyDescriptor(Object.freeze([1]), 'length'); console.log(d.writable, d.enumerable, d.configurable);"),
        "false false false"
    );
    assert_eq!(
        one("const o = Object.freeze({ a: 1 }); o.a = 2; console.log(o.a, Object.isFrozen(o), Object.isSealed(o));"),
        "1 true true"
    );
    assert_eq!(
        one("const o = Object.seal({ a: 1 }); o.a = 2; console.log(o.a, Object.isFrozen(o), Object.isSealed(o));"),
        "2 false true"
    );
}

// `delete` (operator, Reflect.deleteProperty) refuses non-configurable props, array `length`, and
// anything on a frozen or sealed object. Sloppy semantics: a refusal is `false`, never a throw.

#[test]
fn delete_removes_configurable_props() {
    assert_eq!(
        one("const o = { a: 1 }; console.log(delete o.a, 'a' in o);"),
        "true false"
    );
    assert_eq!(
        one("const o = {}; Object.defineProperty(o, 'g', { get() { return 1; }, configurable: true }); console.log(delete o.g, o.g === undefined);"),
        "true true"
    );
    assert_eq!(
        one("const o = Object.preventExtensions({ x: 1 }); console.log(delete o.x, 'x' in o);"),
        "true false"
    );
}

#[test]
fn delete_refuses_non_configurable_props() {
    assert_eq!(
        one("const o = {}; Object.defineProperty(o, 'x', { value: 1 }); console.log(delete o.x, o.x);"),
        "false 1"
    );
    assert_eq!(
        one("const o = {}; Object.defineProperty(o, 'x', { value: 1 }); console.log(Reflect.deleteProperty(o, 'x'), o.x);"),
        "false 1"
    );
}

#[test]
fn delete_refuses_array_length_and_keeps_elements_of_plain_arrays() {
    assert_eq!(
        one("const a = [1, 2]; console.log(delete a.length, a.length);"),
        "false 2"
    );
    assert_eq!(
        one("const a = [1, 2]; console.log(delete a[0], 0 in a, a.length);"),
        "true false 2"
    );
}

#[test]
fn delete_refuses_on_frozen_and_sealed_objects() {
    assert_eq!(
        one("const o = Object.freeze({ x: 1 }); console.log(delete o.x, o.x);"),
        "false 1"
    );
    assert_eq!(
        one("const o = Object.seal({ x: 1 }); console.log(delete o.x, o.x);"),
        "false 1"
    );
    assert_eq!(
        one("const a = Object.freeze([1, 2]); console.log(delete a[0], a[0], a.length);"),
        "false 1 2"
    );
    assert_eq!(
        one("console.log(Reflect.deleteProperty(Object.seal({ s: 1 }), 's'), Reflect.deleteProperty(Object.freeze({ f: 1 }), 'f'));"),
        "false false"
    );
}

#[test]
fn reflect_delete_missing_and_configurable_return_true() {
    assert_eq!(
        one("const o = { a: 1 }; console.log(delete o.missing, Reflect.deleteProperty({}, 'm'), Reflect.deleteProperty(o, 'a'), 'a' in o);"),
        "true true true false"
    );
}
