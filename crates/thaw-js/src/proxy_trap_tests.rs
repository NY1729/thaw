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
