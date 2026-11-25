//! Strict-mode write and delete failures (`strict.rs`). Every probe runs with the switch `On` under the AST
//! interpreter, the closure compiler (`on`) and `check`, and must print the same text. The expected texts are
//! Node 22 output: `/home/fedora/work/thaw-tmp/sa1-scratch/oracle.js` and `oracle2.js` run each body inside a
//! `'use strict'` function and print `e.name + ': ' + e.message`.

use crate::compile::CompileMode;
use crate::strict::StrictMode;
use crate::{Interp, Value};

const ENGINES: [CompileMode; 3] = [CompileMode::Off, CompileMode::On, CompileMode::Check];

/// `body` in a probe function, strict or sloppy, that prints the thrown `name: message` or `no throw`.
fn probe(body: &str, strict_fn: bool) -> String {
    let directive = if strict_fn { "'use strict';" } else { "" };
    format!(
        "function probe() {{ {directive} try {{ {body}\n console.log('no throw'); }} catch (e) {{ console.log(e.name + ': ' + e.message); }} }}\nprobe();"
    )
}

/// Runs `src` in each engine with the switch `sw`; every engine must agree. Returns the common output.
fn outcome(src: &str, sw: StrictMode) -> Vec<String> {
    let mut want: Option<Vec<String>> = None;
    for mode in ENGINES {
        let mut it = Interp::with_compile_mode(mode);
        it.compile_after = 1;
        it.set_strict_mode(sw);
        it.run_sloppy(src)
            .unwrap_or_else(|_| panic!("{mode:?}: script must run: {src}"));
        let got = std::mem::take(&mut it.out);
        match &want {
            None => want = Some(got),
            Some(w) => assert_eq!(&got, w, "{mode:?} differs: {src}"),
        }
    }
    want.unwrap()
}

/// The probe output with the switch on, inside a strict function.
fn strict_on(body: &str) -> String {
    outcome(&probe(body, true), StrictMode::On).join("|")
}

/// Node 22: each entry is `body` and the exact message the strict-mode run prints.
const STRICT_ONLY: &[(&str, &str)] = &[
    (
        "const o = Object.freeze({a:1}); o.a = 2;",
        "TypeError: Cannot assign to read only property 'a' of object '#<Object>'",
    ),
    (
        "const o = Object.freeze({a:1}); o.b = 2;",
        "TypeError: Cannot add property b, object is not extensible",
    ),
    (
        "const o = Object.freeze([1]); o[0] = 2;",
        "TypeError: Cannot assign to read only property '0' of object '[object Array]'",
    ),
    (
        "const o = Object.seal({a:1}); delete o.a;",
        "TypeError: Cannot delete property 'a' of #<Object>",
    ),
    (
        "const o = {}; Object.defineProperty(o,'x',{value:1}); o.x = 2;",
        "TypeError: Cannot assign to read only property 'x' of object '#<Object>'",
    ),
    (
        "const o = {get x(){return 1}}; o.x = 2;",
        "TypeError: Cannot set property x of #<Object> which has only a getter",
    ),
    (
        "const o = Object.preventExtensions({}); o.y = 2;",
        "TypeError: Cannot add property y, object is not extensible",
    ),
    (
        "const o = {}; Object.defineProperty(o,'k',{value:1}); delete o.k;",
        "TypeError: Cannot delete property 'k' of #<Object>",
    ),
    (
        "'abc'[0] = 'x';",
        "TypeError: Cannot assign to read only property '0' of string 'abc'",
    ),
    ("(5).x = 1;", "TypeError: Cannot create property 'x' on number '5'"),
    (
        "undeclaredVarXyz = 1;",
        "ReferenceError: undeclaredVarXyz is not defined",
    ),
    (
        "const a = Object.freeze([1,2]); a.length = 0;",
        "TypeError: Cannot assign to read only property 'length' of object '[object Array]'",
    ),
    (
        "class A { constructor(){ this.v = 1; Object.freeze(this); } set(){ this.v = 2; } } new A().set();",
        "TypeError: Cannot assign to read only property 'v' of object '#<A>'",
    ),
    (
        "const f = Object.freeze(function q(){}); f.x = 1;",
        "TypeError: Cannot add property x, object is not extensible",
    ),
];

#[test]
fn strict_probes_throw_the_node_message_in_every_engine() {
    for (body, want) in STRICT_ONLY {
        assert_eq!(strict_on(body), want.to_string(), "{body}");
    }
}

#[test]
fn strict_sloppy_counterparts_stay_silent_with_the_switch_on() {
    // a sloppy function is never strict, whatever the switch says. A class body is always strict (its
    // method throws in Node too), so the class probe has no sloppy counterpart.
    for (body, _) in STRICT_ONLY.iter().filter(|(b, _)| !b.starts_with("class ")) {
        let src = probe(body, false);
        assert_eq!(
            outcome(&src, StrictMode::On).join("|"),
            "no throw",
            "{body}"
        );
    }
}

#[test]
fn strict_switch_off_keeps_silent_failure_in_strict_code() {
    for (body, _) in STRICT_ONLY {
        let src = probe(body, true);
        assert_eq!(
            outcome(&src, StrictMode::Off).join("|"),
            "no throw",
            "{body}"
        );
    }
}

#[test]
fn strict_audit_logs_but_does_not_throw() {
    for (body, _) in STRICT_ONLY {
        let src = probe(body, true);
        assert_eq!(
            outcome(&src, StrictMode::Audit).join("|"),
            "no throw",
            "{body}"
        );
    }
}

#[test]
fn strict_setter_on_frozen_object_still_runs() {
    // Node: `setter 5`, then no throw. A freeze keeps accessors callable.
    let src = probe(
        "const o = Object.freeze({ set x(v){ console.log('setter', v) } }); o.x = 5;",
        true,
    );
    assert_eq!(outcome(&src, StrictMode::On), ["setter 5", "no throw"]);
}

#[test]
fn strict_nested_arrow_inherits_strictness_from_its_scope() {
    // the arrow has no directive: its strictness comes from the scope it closes over
    assert_eq!(
        strict_on("const f = () => { const o = Object.freeze({a:1}); o.a = 2; }; f();"),
        "TypeError: Cannot assign to read only property 'a' of object '#<Object>'"
    );
    assert_eq!(
        strict_on("const f = () => { zz9 = 1; }; f();"),
        "ReferenceError: zz9 is not defined"
    );
}

#[test]
fn strict_class_method_is_strict() {
    assert_eq!(
        strict_on("class K { m(){ const o = Object.freeze({a:1}); o.a = 2; } } new K().m();"),
        "TypeError: Cannot assign to read only property 'a' of object '#<Object>'"
    );
}

#[test]
fn strict_frozen_array_delete_and_length_texts() {
    // Node: delete of an element of a frozen array
    assert_eq!(
        strict_on("const a = Object.freeze([1]); delete a[0];"),
        "TypeError: Cannot delete property '0' of [object Array]"
    );
    // Node: a non-writable length refuses an index past it under the `length` text
    assert_eq!(
        strict_on("const a = [1]; Object.defineProperty(a,'length',{writable:false}); a[5] = 2;"),
        "TypeError: Cannot assign to read only property 'length' of object '[object Array]'"
    );
}

#[test]
fn strict_string_primitive_length_and_non_index_keys() {
    // Node: 'abc'.length = 2 is the read-only text; a new key is a create failure
    assert_eq!(
        strict_on("'abc'.length = 2;"),
        "TypeError: Cannot assign to read only property 'length' of string 'abc'"
    );
    assert_eq!(
        strict_on("'abc'.x = 'x';"),
        "TypeError: Cannot create property 'x' on string 'abc'"
    );
    assert_eq!(
        strict_on("(true).x = 1;"),
        "TypeError: Cannot create property 'x' on boolean 'true'"
    );
}

#[test]
fn strict_sealed_array_element_stays_writable() {
    // Node: sealing keeps existing elements writable; only new keys fail
    assert_eq!(
        strict_on("const o = Object.seal({0:1}); o[0] = 5; console.log('sealed ok', o[0]);"),
        "sealed ok 5|no throw"
    );
}

#[test]
fn strict_inherited_frozen_property_refuses_the_own_write() {
    assert_eq!(
        strict_on("const p = Object.freeze({q:1}); const o = Object.create(p); o.q = 2;"),
        "TypeError: Cannot assign to read only property 'q' of object '#<Object>'"
    );
}

#[test]
fn strict_inherited_getter_only_refuses() {
    assert_eq!(
        strict_on("const p = { get q(){return 1} }; const o = Object.create(p); o.q = 3;"),
        "TypeError: Cannot set property q of #<Object> which has only a getter"
    );
}

#[test]
fn strict_delete_of_missing_key_is_true() {
    assert_eq!(
        strict_on("const o = Object.freeze({a:1}); console.log('del missing', delete o.zz);"),
        "del missing true|no throw"
    );
}

#[test]
fn strict_rust_set_stays_sloppy_under_the_switch() {
    // the public `set` (natives, host, preludes) never throws, even with the switch on
    let mut it = Interp::new();
    it.set_strict_mode(StrictMode::On);
    let o = it.run("Object.freeze({a: 1})").unwrap();
    assert!(it.set(&o, "a", Value::Number(2.0)).is_ok());
    assert!(it.set(&o, "b", Value::Number(2.0)).is_ok());
    let a = it.get(&o, "a").unwrap();
    assert_eq!(it.to_str(&a).to_string(), "1");
}

#[test]
fn strict_builtin_prelude_code_never_throws_with_the_switch_on() {
    // URL, Buffer and TextEncoder run preludes (`run_builtin`); they must not throw under On
    let src = "const u = new URL('http://a.b/c?x=1'); const b = Buffer.from('xy'); \
               const e = new TextEncoder().encode('a'); console.log(u.pathname, b.length, e.length);";
    assert_eq!(outcome(src, StrictMode::On), ["/c 2 1"]);
}

#[test]
fn strict_env_default_is_on_and_off_or_audit_select_the_other_modes() {
    assert_eq!(StrictMode::parse(None), StrictMode::On);
    assert_eq!(StrictMode::parse(Some("bogus")), StrictMode::On);
    assert_eq!(StrictMode::parse(Some(" on ")), StrictMode::On);
    assert_eq!(StrictMode::parse(Some("off")), StrictMode::Off);
    assert_eq!(StrictMode::parse(Some("audit")), StrictMode::Audit);
}

#[test]
fn strict_script_toplevel_lexicals_are_visible_to_hoisted_functions() {
    let cases: &[(&str, &str)] = &[
        ("'use strict'; const a = 1; function f() { return a } console.log(f());", "1"),
        ("\"use strict\"; let a = 2; function f() { return a } a = 3; console.log(f());", "3"),
        ("'use strict'; class C { m() { return 4 } } function f() { return new C().m() } console.log(f());", "4"),
        ("'use strict'; const a = 5; console.log((function () { return a })(), (() => a)());", "5 5"),
        (
            "'use strict'; const mk = () => () => k; const before = mk(); const k = 6; const after = mk(); console.log(before(), after());",
            "6 6",
        ),
        ("'use strict'; var v = 7; function f() { return v } console.log(f(), globalThis.f === f);", "7 true"),
        ("function f() { 'use strict'; const a = 8; function g() { return a } return g() } console.log(f());", "8"),
        ("const a = 9; function f() { return a } console.log(f());", "9"),
    ];
    for (src, want) in cases {
        assert_eq!(outcome(src, StrictMode::On), [*want], "{src}");
    }
}
