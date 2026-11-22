//! The script-level `__thaw_register_native_bundle(json)` global: the QuickJS-private native-bundle
//! evaluator re-implemented on thaw-js. One Loader per registered bundle (ids `thaw-bundle:N:key`), an
//! evaluator whose `readNative(id, name)` is owner-scoped, foreign-bundle isolation, locked descriptor
//! flags, and the same ready promise on re-entry.

use crate::host::Package;

/// A package whose entry is the CommonJS script `body`; it sets `exports.probe` (async allowed).
fn probe(body: &str) -> String {
    let bundle = "module.exports = (function(require) {\n\
        var __thaw_bundle_factories = { \"s.js\": function (module, exports, require) {\nBODY\n} };\n\
        var __thaw_bundle_require_maps = { \"s.js\": {} };\n\
        var __thaw_bundle_entry_key = \"s.js\";\n})(globalThis.require);"
        .replace("BODY", body);
    let mut package = Package::load(&bundle).expect("loads");
    package.call("probe", &[]).expect("probe").to_json()
}

#[test]
fn registrar_is_a_hidden_host_global_returning_a_callable_evaluator() {
    let out = probe(
        r#"exports.probe = function () {
          var e = __thaw_register_native_bundle(JSON.stringify({ main: 'm', modules: [{ key: 'm', imports: {}, source: 'export const v = 1;' }] }));
          return [typeof __thaw_register_native_bundle, Object.keys(globalThis).indexOf('__thaw_register_native_bundle'), typeof e, typeof e.readNative];
        };"#,
    );
    assert_eq!(out, r#"["function",-1,"function","function"]"#);
}

#[test]
fn reentry_during_evaluation_gets_the_same_ready_promise_and_runs_once() {
    let out = probe(
        r#"globalThis.nativeRuns = 0;
        globalThis.nativeSelfEval = __thaw_register_native_bundle(JSON.stringify({
          main: 'self/index.mjs', modules: [{ key: 'self/index.mjs', imports: {},
            source: 'globalThis.nativeRuns++; globalThis.nativeSelfReady = globalThis.nativeSelfEval(); export const value = 7;' }]
        }));
        exports.probe = async function () {
          var ready = nativeSelfEval();
          var ns = await ready;
          return [nativeRuns, nativeSelfReady === ready, nativeSelfEval() === ready, ns.value,
            typeof globalThis.__thaw_eval_native_entry, typeof globalThis.__thaw_native_esm_sources];
        };"#,
    );
    assert_eq!(out, r#"[1,true,true,7,"undefined","undefined"]"#);
}

// thaw-only: a mutated Promise.prototype.then (Node itself throws there) must not break the evaluator,
// because the ready promise is settled by the host, not through the JS-visible `then`.
#[test]
fn mutated_promise_then_does_not_break_the_evaluator() {
    let out = probe(
        r#"globalThis.nativeRuns = 0;
        globalThis.nativeSelfEval = __thaw_register_native_bundle(JSON.stringify({
          main: 'self/index.mjs', modules: [{ key: 'self/index.mjs', imports: {},
            source: 'globalThis.nativeRuns++; globalThis.nativeSelfReady = globalThis.nativeSelfEval(); export const value = 7;' }]
        }));
        Promise.prototype.then = function () { throw Error('mutated then'); };
        var ready = nativeSelfEval();
        exports.probe = function () {
          return [nativeRuns, nativeSelfReady === ready, nativeSelfEval() === ready, ready instanceof Promise];
        };"#,
    );
    assert_eq!(out, "[1,true,true,true]");
}

const LINKED: &str = r#"
  globalThis.entryA = __thaw_register_native_bundle(JSON.stringify({
    main: 'a', modules: [
      { key: 'a', imports: { './b': 'b', './unused': 'unused' }, source:
        'import "./b"; export function available() { return 7; } export const late = 9; export function loadUnused() { return import("./unused"); }' },
      { key: 'b', imports: { './a': 'a' }, source:
        'import "./a"; globalThis.nativeHoisted = globalThis.entryA.readNative("thaw-bundle:1:a", "available")(); try { globalThis.entryA.readNative("thaw-bundle:1:a", "late"); } catch (e) { globalThis.nativeTdz = e instanceof ReferenceError; } export const peer = 3;' },
      { key: 'unused', imports: {}, source: 'export const hidden = 4;' }
    ]
  }));
  globalThis.beforeNative = (() => {
    try { entryA.readNative('thaw-bundle:1:a', 'available'); return false; }
    catch (_) { return true; }
  })();
  globalThis.beforeNamespace = (() => {
    try { entryA.readNative('thaw-bundle:1:a'); return false; }
    catch (_) { return true; }
  })();
  globalThis.entryB = __thaw_register_native_bundle(JSON.stringify({
    main: 'other', modules: [{ key: 'other', imports: {}, source: 'export const value = 8;' }]
  }));
  globalThis.readyA = entryA();
  globalThis.readyB = entryB();
"#;

#[test]
fn linked_read_is_owner_scoped_and_live_with_locked_flags() {
    let body = format!(
        "{LINKED}
  exports.probe = async function () {{
    await Promise.all([readyA, readyB]);
    let unused, unusedNamespace, foreign, foreignNamespace;
    try {{ entryA.readNative('thaw-bundle:1:unused', 'hidden'); unused = false; }} catch (_) {{ unused = true; }}
    try {{ entryA.readNative('thaw-bundle:1:unused'); unusedNamespace = false; }} catch (_) {{ unusedNamespace = true; }}
    try {{ entryA.readNative('thaw-bundle:2:other', 'value'); foreign = false; }} catch (_) {{ foreign = true; }}
    try {{ entryA.readNative('thaw-bundle:2:other'); foreignNamespace = false; }} catch (_) {{ foreignNamespace = true; }}
    const descriptor = Object.getOwnPropertyDescriptor(entryA, 'readNative');
    return [beforeNative, beforeNamespace, nativeHoisted, nativeTdz, entryA.readNative('thaw-bundle:1:a', 'late'),
      entryA.readNative('thaw-bundle:1:b', 'peer'), unused, unusedNamespace,
      entryB.readNative('thaw-bundle:2:other', 'value') === 8,
      entryA.readNative('thaw-bundle:1:a').late === 9, foreign, foreignNamespace,
      descriptor.enumerable === false && descriptor.writable === false,
      entryA() === readyA];
  }};"
    );
    assert_eq!(
        probe(&body),
        "[true,true,7,true,9,3,true,true,true,true,true,true,true,true]"
    );
}

#[test]
fn a_dynamically_imported_module_becomes_readable_and_flags_are_locked() {
    let body = format!(
        "{LINKED}
  exports.probe = async function () {{
    await readyA;
    await entryA.readNative('thaw-bundle:1:a', 'loadUnused')();
    const d = Object.getOwnPropertyDescriptor(entryA, 'readNative');
    return [entryA.readNative('thaw-bundle:1:unused', 'hidden'), d.enumerable, d.writable, d.configurable,
      Object.keys(entryA).length];
  }};"
    );
    assert_eq!(probe(&body), "[4,false,false,false,0]");
}
