use super::*;

/// Run `src` as a global script (`vm.runInThisContext` semantics) and join its console.log lines with `|`.
fn script(src: &str) -> String {
    let mut it = Interp::new();
    match it.run_parsed(src, thaw_parser::parse_javascript_script(src), true) {
        Ok(_) => it.out.join("|"),
        Err(Ctl::Throw(v)) => format!("Uncaught {}", it.to_str(&v)),
        Err(_) => "stray completion".into(),
    }
}

// The `globalThis` object view: every global binding is an own property with Node 22's attributes
// (verified with `vm.runInThisContext` on Node v22.23.1 and against tools/parity surface.txt).
// Lexical (`let`/`const`/`class`) bindings are not on the object at all.

#[test]
fn builtin_constructor_is_own_writable_configurable_non_enumerable_data() {
    // Node: data;function;-wc;length=1;name="Object"
    assert_eq!(
        script("const d = Object.getOwnPropertyDescriptor(globalThis, 'Object'); console.log(typeof d.value, d.writable, d.enumerable, d.configurable, d.value === Object, d.value.name);"),
        "function true false true true Object"
    );
    // Node: data;object;-wc;tag="Math"
    assert_eq!(
        script("const d = Object.getOwnPropertyDescriptor(globalThis, 'Math'); console.log(d.writable, d.enumerable, d.configurable);"),
        "true false true"
    );
    // Node: data;object;-wc;tag="global"
    assert_eq!(
        script("const d = Object.getOwnPropertyDescriptor(globalThis, 'globalThis'); console.log(d.writable, d.enumerable, d.configurable, d.value === globalThis);"),
        "true false true true"
    );
}

#[test]
fn value_globals_are_read_only_non_enumerable_non_configurable() {
    // Node: data;number;--- for NaN, Infinity; data;undefined;--- for undefined
    assert_eq!(
        script("const d = Object.getOwnPropertyDescriptor(globalThis, 'NaN'); console.log(d.value, d.writable, d.enumerable, d.configurable);"),
        "NaN false false false"
    );
    assert_eq!(
        script("const u = Object.getOwnPropertyDescriptor(globalThis, 'undefined'); const i = Object.getOwnPropertyDescriptor(globalThis, 'Infinity'); console.log(u.writable, u.configurable, i.writable, i.configurable);"),
        "false false false false"
    );
}

#[test]
fn node_enumerable_globals_are_listed_by_object_keys() {
    // Node: data;function;ewc for these (setTimeout, queueMicrotask, structuredClone, atob) and data;object;ewc for global
    assert_eq!(
        script("const k = Object.keys(globalThis); console.log(k.includes('setTimeout'), k.includes('global'), k.includes('queueMicrotask'), k.includes('structuredClone'), k.includes('atob'));"),
        "true true true true true"
    );
    assert_eq!(
        script("const d = Object.getOwnPropertyDescriptor(globalThis, 'setTimeout'); console.log(d.writable, d.enumerable, d.configurable);"),
        "true true true"
    );
}

#[test]
fn non_enumerable_builtins_are_not_listed_by_object_keys() {
    assert_eq!(
        script("const k = Object.keys(globalThis); console.log(k.includes('Object'), k.includes('Math'), k.includes('NaN'));"),
        "false false false"
    );
}

#[test]
fn builtins_are_in_own_property_names_and_the_in_operator() {
    // Node: `in` and hasOwnProperty are true for every builtin global; `nope` is absent
    assert_eq!(
        script("const n = Object.getOwnPropertyNames(globalThis); console.log(n.includes('Object'), 'Object' in globalThis, 'setTimeout' in globalThis, 'nope' in globalThis, globalThis.hasOwnProperty('Object'), Reflect.ownKeys(globalThis).includes('Math'));"),
        "true true true false true true"
    );
}

#[test]
fn thaw_internal_names_are_hidden_from_the_view_but_still_resolve() {
    // `__thaw_*` is not a Node global: hidden from `in`, keys, own names and descriptors, while
    // scope lookup and property reads keep resolving it (compiled code and preludes depend on that).
    assert_eq!(
        script("globalThis.__thaw_probe = 7; const n = Object.getOwnPropertyNames(globalThis); console.log('__thaw_probe' in globalThis, Object.keys(globalThis).includes('__thaw_probe'), n.includes('__thaw_probe'), globalThis.hasOwnProperty('__thaw_probe'), Object.getOwnPropertyDescriptor(globalThis, '__thaw_probe'), n.some(k => k.startsWith('__thaw_')), __thaw_probe, globalThis.__thaw_probe);"),
        "false false false false undefined false 7 7"
    );
}

#[test]
fn var_and_implicit_globals_keep_script_flags() {
    // Node: var -> writable, enumerable, non-configurable; implicit global -> writable, enumerable, configurable
    assert_eq!(
        script("var v = 1; w2 = 2; const a = Object.getOwnPropertyDescriptor(globalThis, 'v'); const b = Object.getOwnPropertyDescriptor(globalThis, 'w2'); console.log(a.writable, a.enumerable, a.configurable, b.writable, b.enumerable, b.configurable, Object.keys(globalThis).includes('v'), Object.keys(globalThis).includes('w2'));"),
        "true true false true true true true true"
    );
}

#[test]
fn lexical_bindings_are_not_global_object_properties() {
    // Node: let/const/class are scope bindings only
    assert_eq!(
        script("let l = 1; const c = 2; class K {} console.log('l' in globalThis, 'c' in globalThis, 'K' in globalThis, Object.getOwnPropertyDescriptor(globalThis, 'l'), typeof l, c, K.name, Object.keys(globalThis).includes('c'));"),
        "false false false undefined number 2 K false"
    );
}

#[test]
fn prelude_installed_builtins_keep_node_flags() {
    // Installed by a JS prelude (an implicit global to thaw-js), but a non-enumerable builtin in Node.
    assert_eq!(
        script("const d = Object.getOwnPropertyDescriptor(globalThis, 'Event'); const w = Object.getOwnPropertyDescriptor(globalThis, 'WeakMap'); console.log(d.writable, d.enumerable, d.configurable, w.writable, w.enumerable, w.configurable);"),
        "true false true true false true"
    );
    assert_eq!(
        script("console.log(Object.keys(globalThis).includes('Event'), Object.keys(globalThis).includes('WeakMap'));"),
        "false false"
    );
}

#[test]
fn value_globals_refuse_delete() {
    // Node: NaN, undefined and Infinity are non-configurable, so `delete` returns false and they stay
    assert_eq!(
        script("console.log(delete globalThis.NaN, delete globalThis.undefined, delete globalThis.Infinity, NaN, Reflect.deleteProperty(globalThis, 'NaN'));"),
        "false false false NaN false"
    );
}

#[test]
fn timeout_handle_exposes_to_primitive_as_a_function() {
    // Node 22: `typeof setTimeout(...)[Symbol.toPrimitive]` is "function" (Timeout.prototype has it).
    assert_eq!(
        script("console.log(typeof setTimeout(() => {}, 1)[Symbol.toPrimitive]);"),
        "function"
    );
}

#[test]
fn a_configurable_property_defined_on_globalthis_is_removed_by_delete() {
    // Node: true undefined false true / false number / true undefined / true undefined / false
    assert_eq!(
        script("Object.defineProperty(globalThis, 'q', { value: 1, configurable: true }); console.log(delete globalThis.q, typeof q, 'q' in globalThis, Object.getOwnPropertyDescriptor(globalThis, 'q') === undefined);"),
        "true undefined false true"
    );
    assert_eq!(
        script("Object.defineProperty(globalThis, 'r', { value: 1 }); console.log(delete globalThis.r, typeof r);"),
        "false number"
    );
    assert_eq!(
        script("Object.defineProperty(globalThis, 's', { value: function () {}, writable: false, configurable: true }); console.log(Reflect.deleteProperty(globalThis, 's'), typeof s);"),
        "true undefined"
    );
    assert_eq!(
        script("Object.defineProperty(globalThis, 't', { value: 1, configurable: true }); Object.defineProperty(globalThis, 't', { value: 2, configurable: true }); console.log(delete globalThis.t, typeof t);"),
        "true undefined"
    );
    assert_eq!(
        script("Object.defineProperty(globalThis, 'u', { value: 1, configurable: true }); console.log(Object.keys(globalThis).includes('u'));"),
        "false"
    );
}

#[test]
fn a_top_level_let_shadows_a_builtin_without_replacing_the_global_property() {
    // test262 language/global-code/decl-lex-configurable-global
    assert_eq!(
        script("let Array = 5; console.log(Array, typeof globalThis.Array, Object.getOwnPropertyDescriptor(globalThis, 'Array').configurable);"),
        "5 function true"
    );
    // a plain top-level let is not a property of the global object
    assert_eq!(
        script("let foo = 1; console.log(globalThis.foo, 'foo' in globalThis);"),
        "undefined false"
    );
}

#[test]
fn builtin_methods_have_no_construct() {
    assert_eq!(
        script("var r = []; for (var f of [Math.max, Array.prototype.map, Object.keys]) { try { new f(); r.push('ctor'); } catch (e) { r.push(e.constructor.name); } } console.log(r.join(), Reflect.construct(function () {}, [], Array) instanceof Array);"),
        "TypeError,TypeError,TypeError true"
    );
}
