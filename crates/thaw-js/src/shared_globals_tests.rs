//! Process-wide user writes to the global object (shared_globals.rs): registry-package realms share their
//! global writes, while bare, vm and worker interpreters stay isolated. Realms are made with `load_named`
//! from small bundles, as the registry does, and run through `with_package`.

use super::*;
use crate::host::{register_test_package, with_package, Package};

/// A registry bundle whose entry factory runs `body` when the package loads.
fn bundle(body: &str) -> String {
    format!(
        r#"module.exports = (function(require){{ var __thaw_bundle_factories = {{"a/index.js": function(module, exports, require) {{ {body} }}}}; var __thaw_bundle_require_maps = {{"a/index.js": {{}}}}; var __thaw_bundle_entry_key = "a/index.js"; }})(globalThis.require);"#
    )
}

/// Loads a package realm running `body` at load; returns its handle.
fn load(body: &str) -> usize {
    let package = Package::load_named("a", &bundle(body), 0).expect("bundle loads");
    register_test_package(package)
}

/// Evaluates `src` in `it` and returns its string value.
fn text(it: &mut Interp, src: &str) -> String {
    let v = it
        .run(src)
        .unwrap_or_else(|e| panic!("script failed: {src}: {e:?}"));
    it.to_str(&v).to_string()
}

/// Evaluates `src` in the realm `handle` and returns its string value.
fn js(handle: usize, src: &str) -> String {
    with_package(handle, |p| Ok(text(p.interp(), src))).expect("package call")
}

#[test]
fn define_property_on_globalthis_is_seen_by_another_realm() {
    let a = load(
        "class X {} Object.defineProperty(globalThis, 'Response', {value: X, writable: true, configurable: true}); globalThis.handle = new X();",
    );
    let b = load("");
    assert_eq!(js(b, "Response.name"), "X");
    // one class, not two: an instance made in A is an instance of B's `Response`
    assert_eq!(js(b, "String(handle instanceof Response)"), "true");
    assert_eq!(js(a, "String(Response === globalThis.Response)"), "true");
}

#[test]
fn a_realm_loaded_later_sees_the_published_global() {
    let _a = load("Object.defineProperty(globalThis, 'Response', {value: class Late {}});");
    let c = load("");
    assert_eq!(js(c, "Response.name"), "Late");
}

#[test]
fn implicit_and_property_globals_are_shared_both_ways() {
    let a = load("zz = 1; globalThis.yy = 2;");
    let b = load("");
    assert_eq!(js(b, "String(zz + yy)"), "3");
    assert_eq!(js(b, "globalThis.zz = 5; 'ok'"), "ok");
    assert_eq!(js(a, "String(zz)"), "5");
    assert_eq!(js(a, "String(globalThis.yy)"), "2");
}

#[test]
fn delete_propagates_to_other_realms() {
    let a = load("globalThis.gone = 1;");
    let b = load("");
    assert_eq!(js(b, "typeof gone"), "number");
    assert_eq!(js(a, "String(delete globalThis.gone)"), "true");
    assert_eq!(js(b, "typeof gone"), "undefined");
}

#[test]
fn internal_thaw_names_are_not_published() {
    let a = load("globalThis.__thaw_x = 1;");
    let b = load("");
    assert_eq!(js(b, "typeof __thaw_x"), "undefined");
    assert_eq!(js(a, "String(__thaw_x)"), "1");
}

#[test]
fn reading_a_builtin_does_not_overwrite_the_other_realm() {
    let b = load("globalThis.mine = Response;");
    // A only reads its own `Response`: nothing is published, so B keeps its own builtin
    let a = load("globalThis.readback = String(Response.name);");
    assert_eq!(js(a, "readback"), "Response");
    assert_eq!(js(b, "String(Response === mine)"), "true");
}

#[test]
fn bare_interpreters_do_not_share_or_receive_globals() {
    let _a = load("Object.defineProperty(globalThis, 'Response', {value: class Only {}}); zz = 1;");
    let mut bare = Interp::runtime();
    assert_eq!(text(&mut bare, "typeof zz"), "undefined");
    assert_eq!(text(&mut bare, "Response.name"), "Response");
    text(&mut bare, "globalThis.bareleak = 1");
    let mut later = Interp::runtime();
    assert_eq!(text(&mut later, "typeof bareleak"), "undefined");
}

#[test]
fn vm_contexts_do_not_publish_or_receive() {
    let a = load("zz = 1; Object.defineProperty(globalThis, 'Response', {value: class Vm {}});");
    let mut vm = Interp::new_vm_realm();
    text(&mut vm, "globalThis.vmleak = 1");
    assert_eq!(
        text(&mut vm, "typeof zz + ' ' + typeof Response"),
        "undefined undefined"
    );
    let b = load("");
    assert_eq!(js(b, "typeof vmleak"), "undefined");
    assert_eq!(js(a, "typeof vmleak"), "undefined");
}

#[test]
fn worker_thread_realms_stay_isolated() {
    let _a = load("zz = 1;");
    let seen = std::thread::Builder::new()
        .stack_size(64 << 20)
        .spawn(|| {
            let mut it = Interp::runtime();
            text(&mut it, "globalThis.wkleak = 1; typeof zz")
        })
        .expect("spawn")
        .join()
        .expect("worker ran");
    assert_eq!(seen, "undefined");
    let b = load("");
    assert_eq!(js(b, "typeof wkleak"), "undefined");
}
