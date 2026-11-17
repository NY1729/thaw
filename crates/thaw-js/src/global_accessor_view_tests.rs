//! The `globalThis` accessor view of Node 22's five accessor globals: `process`, `Buffer`, `performance`,
//! `navigator`, `crypto` (W8-c of `designs/plan-q9-waivers-w7-w8.md`).
//!
//! Oracle: Node v22.23.1. Each case below was run as a file under `require('vm').runInThisContext(src)`, one
//! process per case, and the expected text is that output joined with `|`. `node -e` is not an oracle: it
//! installs builtin modules (`crypto`, ...) as globals and adds `require`/`module` to the global object.
//! The probe is the `CASES` table; every case must print the same text in the AST interpreter, the closure
//! compiler (`on`) and `check`.
//!
//! Node's shape: `process`/`Buffer` get+set non-enumerable; `performance` get+set enumerable; `navigator`/
//! `crypto` getter only, enumerable. Getters are named `get` (process, Buffer) or `get <name>`, setters `set`
//! or `set <name>`, lengths 0 and 1. Assignment through the setter keeps the accessor; `defineProperty` with
//! a `value` (or `writable`) turns it into a data property.

use crate::compile::CompileMode;
use crate::strict::StrictMode;
use crate::Interp;

const ENGINES: [CompileMode; 3] = [CompileMode::Off, CompileMode::On, CompileMode::Check];

/// `(name, source)`: the Node 22 probe cases.
const CASES: &[(&str, &str)] = &[
    (
        "shapes",
        "const out=[]; for (const k of ['process','Buffer','performance','navigator','crypto']) { const d=Object.getOwnPropertyDescriptor(globalThis,k); out.push([k, Object.keys(d).join(','), typeof d.get, d.get.name, d.get.length, d.set===undefined?'noset':d.set.name+'/'+d.set.length, d.enumerable, d.configurable].join(' ')); } console.log(out.join(';'));",
    ),
    (
        "identity_and_set",
        "const a=Object.getOwnPropertyDescriptor(globalThis,'performance'); const b=Object.getOwnPropertyDescriptor(globalThis,'performance'); globalThis.performance = 5; const c=Object.getOwnPropertyDescriptor(globalThis,'performance'); console.log(a.get===b.get, a.set===b.set, typeof performance, c.get===a.get, typeof c.set, 'value' in c);",
    ),
    (
        "crypto_getter_this",
        "const d=Object.getOwnPropertyDescriptor(globalThis,'crypto'); console.log(typeof d.get.call(undefined), typeof d.get.call(globalThis)); try { d.get.call({}); console.log('no throw') } catch(e) { console.log(e.name+': '+e.message) }",
    ),
    (
        "define_value_navigator",
        "Object.defineProperty(globalThis,'navigator',{value:3,writable:true,enumerable:true,configurable:true}); const d=Object.getOwnPropertyDescriptor(globalThis,'navigator'); console.log(Object.keys(d).join(','), d.value, d.writable, d.enumerable, d.configurable, navigator);",
    ),
    (
        "define_value_process",
        "Object.defineProperty(globalThis,'process',{value:1}); const d=Object.getOwnPropertyDescriptor(globalThis,'process'); console.log(Object.keys(d).join(','), d.value, d.writable, d.enumerable, d.configurable, process);",
    ),
    (
        "assign_keeps_accessor",
        "globalThis.process = 1; const d=Object.getOwnPropertyDescriptor(globalThis,'process'); console.log(typeof process, 'get' in d, 'value' in d, d.enumerable, d.configurable);",
    ),
    (
        "getter_only_writes",
        "crypto = 1; console.log(typeof crypto); (function(){'use strict'; try { navigator = 2; console.log('no throw') } catch(e){ console.log(e.name+': '+e.message) } })(); (function(){'use strict'; try { globalThis.crypto = 1; console.log('no throw') } catch(e){ console.log(e.name+': '+e.message) } })(); console.log(typeof crypto, typeof navigator);",
    ),
    (
        "setter_writes_strict",
        "(function(){'use strict'; performance = 2; Buffer = 3; process = 4})(); console.log(typeof performance, typeof Buffer, typeof process);",
    ),
    (
        "delete_accessors",
        "console.log(delete globalThis.process, typeof process, delete globalThis.navigator, typeof navigator);",
    ),
    (
        "free_identifiers",
        "console.log(typeof process.env, Buffer.from('a').length, typeof performance.now(), typeof navigator.hardwareConcurrency, typeof crypto.randomUUID(), globalThis.Buffer.from('ab').length, typeof globalThis.process.env);",
    ),
    (
        "keys_membership",
        "const k=Object.keys(globalThis); console.log(k.includes('performance'), k.includes('navigator'), k.includes('crypto'), k.includes('process'), k.includes('Buffer'), Object.getOwnPropertyNames(globalThis).includes('process'), 'process' in globalThis);",
    ),
];

/// Runs `src` as a sloppy global script in each engine with the strict switch on; every engine must print
/// the same lines, which are returned joined with `|`.
fn run_all(src: &str) -> String {
    let mut want: Option<Vec<String>> = None;
    for mode in ENGINES {
        let mut it = Interp::with_compile_mode(mode);
        it.compile_after = 1;
        it.set_strict_mode(StrictMode::On);
        it.run_sloppy(src)
            .unwrap_or_else(|_| panic!("{mode:?}: script must run: {src}"));
        let got = std::mem::take(&mut it.out);
        match &want {
            None => want = Some(got),
            Some(w) => assert_eq!(&got, w, "{mode:?} differs: {src}"),
        }
    }
    want.unwrap().join("|")
}

fn case(name: &str) -> String {
    let src = CASES
        .iter()
        .find(|(n, _)| *n == name)
        .unwrap_or_else(|| panic!("no case {name}"))
        .1;
    run_all(src)
}

#[test]
fn accessor_globals_have_node_descriptor_shape_and_function_names() {
    assert_eq!(
        case("shapes"),
        "process get,set,enumerable,configurable function get 0 set/1 false true;Buffer get,set,enumerable,configurable function get 0 set/1 false true;performance get,set,enumerable,configurable function get performance 0 set performance/1 true true;navigator get,set,enumerable,configurable function get navigator 0 noset true true;crypto get,set,enumerable,configurable function get crypto 0 noset true true"
    );
}

#[test]
fn accessor_pair_is_shared_per_key_and_set_keeps_the_accessor() {
    assert_eq!(
        case("identity_and_set"),
        "true true number true function false"
    );
    assert_eq!(
        case("assign_keeps_accessor"),
        "number true false false true"
    );
}

#[test]
fn crypto_getter_checks_its_receiver_and_the_others_do_not() {
    assert_eq!(
        case("crypto_getter_this"),
        "object object|TypeError: Value of \"this\" must be of type nullish or must be the global object"
    );
}

#[test]
fn define_property_with_a_value_turns_the_accessor_into_data() {
    assert_eq!(
        case("define_value_navigator"),
        "value,writable,enumerable,configurable 3 true true true 3"
    );
    assert_eq!(
        case("define_value_process"),
        "value,writable,enumerable,configurable 1 false false true 1"
    );
}

#[test]
fn getter_only_globals_ignore_sloppy_writes_and_throw_in_strict_code() {
    assert_eq!(
        case("getter_only_writes"),
        "object|TypeError: Cannot set property navigator of #<Object> which has only a getter|TypeError: Cannot set property crypto of #<Object> which has only a getter|object object"
    );
}

#[test]
fn setter_globals_accept_strict_writes_and_keep_the_value() {
    assert_eq!(case("setter_writes_strict"), "number number number");
}

#[test]
fn delete_removes_accessor_globals_as_free_identifiers() {
    assert_eq!(case("delete_accessors"), "true undefined true undefined");
}

#[test]
fn accessor_globals_still_resolve_as_free_identifiers_and_through_globalthis() {
    assert_eq!(
        case("free_identifiers"),
        "object 1 number number string 2 object"
    );
}

#[test]
fn object_keys_lists_only_the_enumerable_accessor_globals() {
    assert_eq!(
        case("keys_membership"),
        "true true true false false true true"
    );
}
