use super::*;

/// Run `src` and return the console.log lines (or the uncaught error text).
fn run(src: &str) -> Vec<String> {
    let mut it = Interp::new();
    match it.run(src) {
        Ok(_) => it.out,
        Err(Ctl::Throw(v)) => {
            let mut out = it.out.clone();
            out.push(format!("Uncaught {}", it.to_str(&v)));
            out
        }
        Err(Ctl::Unsupported(f)) => vec![format!("unsupported: {f}")],
        Err(_) => vec!["stray completion".into()],
    }
}

fn one(src: &str) -> String {
    run(src).join("|")
}

// Guards: behaviour that must not change with the new traps.

#[test]
fn proxy_trap_guards_unchanged_paths() {
    assert_eq!(
        one("console.log(Array.isArray([]), Array.isArray({}), Array.isArray(null), Array.isArray(new Proxy({}, {})));"),
        "true false false false"
    );
    assert_eq!(
        one("var o = {}; Object.preventExtensions(o); console.log(Object.isExtensible(o));"),
        "false"
    );
    assert_eq!(
        one("var log = []; var p = new Proxy({}, { getPrototypeOf(t) { log.push('gpo'); return Object.prototype; } }); console.log(Object.getPrototypeOf(p) === Object.prototype, log.join());"),
        "true gpo"
    );
}

// New behaviour: proxy traps and target forwarding for the integrity/extensibility builtins.

#[test]
fn proxy_prevent_extensions_trap_and_forwarding() {
    assert_eq!(
        one("var log = []; var t = {}; var p = new Proxy(t, { preventExtensions(tg) { log.push('pe'); Object.preventExtensions(tg); return true; } }); Object.preventExtensions(p); console.log(log.join(), Object.isExtensible(p));"),
        "pe false"
    );
    assert_eq!(
        one("var t = {}; var p = new Proxy(t, {}); Object.preventExtensions(p); console.log(Object.isExtensible(t));"),
        "false"
    );
}

#[test]
fn proxy_is_extensible_trap_and_forwarding() {
    assert_eq!(
        one("var log = []; var p = new Proxy({}, { isExtensible(tg) { log.push('ie'); return true; } }); console.log(Object.isExtensible(p), log.join());"),
        "true ie"
    );
    assert_eq!(
        one("var t = {}; Object.preventExtensions(t); var p = new Proxy(t, {}); console.log(Object.isExtensible(p));"),
        "false"
    );
}

#[test]
fn proxy_set_prototype_of_trap_and_forwarding() {
    assert_eq!(
        one("var log = []; var p = new Proxy({}, { setPrototypeOf(tg, pr) { log.push('spo'); Object.setPrototypeOf(tg, pr); return true; } }); Object.setPrototypeOf(p, Array.prototype); console.log(log.join(), Object.getPrototypeOf(p) === Array.prototype);"),
        "spo true"
    );
    assert_eq!(
        one("var t = {}; var p = new Proxy(t, {}); Object.setPrototypeOf(p, Array.prototype); console.log(Object.getPrototypeOf(t) === Array.prototype);"),
        "true"
    );
}

#[test]
fn proxy_array_is_array_looks_through() {
    assert_eq!(
        one("console.log(Array.isArray(new Proxy([], {})), Array.isArray(new Proxy(new Proxy([], {}), {})));"),
        "true true"
    );
    assert_eq!(
        one("var r = Proxy.revocable([], {}); r.revoke(); try { Array.isArray(r.proxy); console.log('no'); } catch (e) { console.log(e.name); }"),
        "TypeError"
    );
}

#[test]
fn proxy_is_frozen_is_sealed_forward_to_target() {
    assert_eq!(
        one("console.log(Object.isFrozen(new Proxy(Object.freeze({ a: 1 }), {})), Object.isFrozen(new Proxy({ a: 1 }, {})));"),
        "true false"
    );
    assert_eq!(
        one("console.log(Object.isSealed(new Proxy(Object.seal({ a: 1 }), {})));"),
        "true"
    );
}

/// Like `one`, after the dynamic prelude (which defines `__thaw_array_with_native_view`) is installed.
fn one_after_prelude(src: &str) -> String {
    let mut it = Interp::new();
    assert!(it.run_sloppy(include_str!("dynamic_prelude.js")).is_ok());
    match it.run(src) {
        Ok(_) => it.out.join("|"),
        Err(Ctl::Throw(v)) => format!("Uncaught {}", it.to_str(&v)),
        Err(_) => "stray completion".into(),
    }
}

#[test]
fn json_stringify_proxy_over_array_is_an_array() {
    assert_eq!(
        one("var p = new Proxy([], { get(t, k) { if (k === 'length') return 3; var n = +k; return n >= 0 && n < 3 ? n + 1 : undefined; }, has(t, k) { return k === 'length' || (+k >= 0 && +k < 3); } }); console.log(JSON.stringify(p), Array.isArray(p), JSON.stringify({ a: p }));"),
        "[1,2,3] true {\"a\":[1,2,3]}"
    );
}

#[test]
fn json_stringify_prelude_array_view_is_an_array() {
    assert_eq!(
        one_after_prelude("var view = __thaw_array_with_native_view(false, () => 3, i => i * 10, () => {}, () => {}, () => {}, () => {}, () => true, () => undefined); console.log(JSON.stringify(view), Array.isArray(view), JSON.stringify({ a: view }));"),
        "[0,10,20] true {\"a\":[0,10,20]}"
    );
}

#[test]
fn json_stringify_object_proxy_lists_keys_by_gopd_enumerable() {
    assert_eq!(
        one("console.log(JSON.stringify(new Proxy({}, { ownKeys: () => ['a', 'b'], getOwnPropertyDescriptor: (t, k) => ({ value: 1, enumerable: k === 'a', configurable: true }), get: () => 1 })));"),
        "{\"a\":1}"
    );
}

#[test]
fn json_stringify_revoked_proxy_over_array_throws() {
    assert_eq!(
        one("var r = Proxy.revocable([], {}); r.revoke(); try { JSON.stringify(r.proxy); console.log('no'); } catch (e) { console.log(e.name); }"),
        "TypeError"
    );
}
