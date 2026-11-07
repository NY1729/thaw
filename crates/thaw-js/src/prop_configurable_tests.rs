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
