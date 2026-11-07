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

// `Object.defineProperty` / `Reflect.defineProperty` against non-configurable props, frozen and sealed
// objects, non-extensible objects and array `length`/elements. Expected values match Node 22.

#[test]
fn redefine_value_change_on_writable_non_configurable_is_allowed() {
    assert_eq!(
        one("const o = {}; Object.defineProperty(o, 'x', { value: 1, writable: true }); Object.defineProperty(o, 'x', { value: 2 }); console.log(o.x);"),
        "2"
    );
}

#[test]
fn redefine_writable_true_to_false_is_allowed() {
    assert_eq!(
        one("const o = {}; Object.defineProperty(o, 'x', { value: 1, writable: true }); Object.defineProperty(o, 'x', { writable: false }); console.log(o.x, Object.getOwnPropertyDescriptor(o, 'x').writable);"),
        "1 false"
    );
}

#[test]
fn redefine_writable_false_to_true_throws() {
    assert_eq!(
        one("const o = {}; Object.defineProperty(o, 'x', { value: 1 }); try { Object.defineProperty(o, 'x', { writable: true }); console.log('no'); } catch (e) { console.log(e.name, e.message); }"),
        "TypeError Cannot redefine property: x"
    );
}

#[test]
fn redefine_enumerable_change_throws() {
    assert_eq!(
        one("const o = {}; Object.defineProperty(o, 'x', { value: 1, enumerable: true }); try { Object.defineProperty(o, 'x', { enumerable: false }); console.log('no'); } catch (e) { console.log(e.name, e.message); }"),
        "TypeError Cannot redefine property: x"
    );
}

#[test]
fn redefine_data_to_accessor_throws() {
    assert_eq!(
        one("const o = {}; Object.defineProperty(o, 'x', { value: 1 }); try { Object.defineProperty(o, 'x', { get() { return 1; } }); console.log('no'); } catch (e) { console.log(e.name, e.message); }"),
        "TypeError Cannot redefine property: x"
    );
}

#[test]
fn redefine_accessor_to_data_throws() {
    assert_eq!(
        one("const o = {}; Object.defineProperty(o, 'x', { get() { return 1; } }); try { Object.defineProperty(o, 'x', { value: 1 }); console.log('no'); } catch (e) { console.log(e.name, e.message); }"),
        "TypeError Cannot redefine property: x"
    );
}

#[test]
fn redefine_same_value_is_allowed() {
    assert_eq!(
        one("const o = {}; Object.defineProperty(o, 'x', { value: 1 }); Object.defineProperty(o, 'x', { value: 1 }); Object.defineProperty(o, 'n', { value: NaN }); Object.defineProperty(o, 'n', { value: NaN }); console.log(o.x, o.n);"),
        "1 NaN"
    );
}

#[test]
fn redefine_negative_zero_to_zero_throws() {
    assert_eq!(
        one("const o = {}; Object.defineProperty(o, 'z', { value: -0 }); try { Object.defineProperty(o, 'z', { value: 0 }); console.log('no'); } catch (e) { console.log(e.name, e.message); }"),
        "TypeError Cannot redefine property: z"
    );
}

#[test]
fn redefine_accessor_get_change_throws() {
    assert_eq!(
        one("const g = function () {}; const o = {}; Object.defineProperty(o, 'a', { get: g }); Object.defineProperty(o, 'a', { get: g }); try { Object.defineProperty(o, 'a', { get: function () {} }); console.log('no'); } catch (e) { console.log(e.name, e.message); }"),
        "TypeError Cannot redefine property: a"
    );
}

#[test]
fn redefine_same_accessor_getter_is_allowed() {
    assert_eq!(
        one("const g = function () {}; const o = {}; Object.defineProperty(o, 'a', { get: g }); Object.defineProperty(o, 'a', { get: g }); console.log(typeof Object.getOwnPropertyDescriptor(o, 'a').get);"),
        "function"
    );
}

#[test]
fn redefine_configurable_true_on_non_configurable_throws() {
    assert_eq!(
        one("const o = {}; Object.defineProperty(o, 'x', { value: 1 }); try { Object.defineProperty(o, 'x', { configurable: true }); console.log('no'); } catch (e) { console.log(e.name, e.message); }"),
        "TypeError Cannot redefine property: x"
    );
}

#[test]
fn reflect_define_property_returns_false_instead_of_throwing() {
    assert_eq!(
        one("const o = {}; Object.defineProperty(o, 'x', { value: 1 }); console.log(Reflect.defineProperty(o, 'x', { value: 2 }), Reflect.defineProperty(o, 'x', { value: 1 }), o.x);"),
        "false true 1"
    );
}

#[test]
fn reflect_define_property_on_frozen_compares_values() {
    assert_eq!(
        one("const f = Object.freeze({ a: 1 }); console.log(Reflect.defineProperty(f, 'a', { value: 1 }), Reflect.defineProperty(f, 'a', { value: 2 }), f.a);"),
        "true false 1"
    );
}

#[test]
fn reflect_define_new_property_on_non_extensible_is_false() {
    assert_eq!(
        one("console.log(Reflect.defineProperty(Object.preventExtensions({}), 'n', { value: 1 }), Reflect.defineProperty(Object.freeze({}), 'n', { value: 1 }));"),
        "false false"
    );
}

#[test]
fn define_new_property_on_non_extensible_throws() {
    assert_eq!(
        one("try { Object.defineProperty(Object.preventExtensions({}), 'n', { value: 1 }); console.log('no'); } catch (e) { console.log(e.name, e.message); }"),
        "TypeError Cannot define property n, object is not extensible"
    );
}

#[test]
fn define_configurable_property_on_non_extensible_is_allowed() {
    assert_eq!(
        one("const o = Object.preventExtensions({ a: 1 }); Object.defineProperty(o, 'a', { value: 2 }); console.log(o.a);"),
        "2"
    );
}

#[test]
fn array_length_configurable_true_throws() {
    assert_eq!(
        one("try { Object.defineProperty([1, 2], 'length', { configurable: true }); console.log('no'); } catch (e) { console.log(e.name, e.message); }"),
        "TypeError Cannot redefine property: length"
    );
}

#[test]
fn array_length_value_truncates() {
    assert_eq!(
        one("const a = [1, 2, 3]; Object.defineProperty(a, 'length', { value: 1 }); console.log(a.length, a[1]);"),
        "1 undefined"
    );
}

#[test]
fn array_length_enumerable_true_throws() {
    assert_eq!(
        one("try { Object.defineProperty([1], 'length', { enumerable: true }); console.log('no'); } catch (e) { console.log(e.name, e.message); }"),
        "TypeError Cannot redefine property: length"
    );
}

#[test]
fn frozen_array_length_compares_value() {
    assert_eq!(
        one("const f = Object.freeze([1, 2]); console.log(Reflect.defineProperty(f, 'length', { value: 2 }), Reflect.defineProperty(f, 'length', { value: 5 }), f.length);"),
        "true false 2"
    );
}

#[test]
fn frozen_array_element_compares_value() {
    assert_eq!(
        one("const f = Object.freeze([1]); console.log(Reflect.defineProperty(f, 0, { value: 1 }), Reflect.defineProperty(f, 0, { value: 2 }), f[0]);"),
        "true false 1"
    );
}

#[test]
fn sealed_array_element_value_change_is_allowed() {
    assert_eq!(
        one("const s = Object.seal([1]); Object.defineProperty(s, 0, { value: 9 }); console.log(s[0]);"),
        "9"
    );
}

#[test]
fn array_index_define_on_non_extensible_throws() {
    assert_eq!(
        one("try { Object.defineProperty(Object.preventExtensions([1]), 1, { value: 2 }); console.log('no'); } catch (e) { console.log(e.name, e.message); }"),
        "TypeError Cannot define property 1, object is not extensible"
    );
}
