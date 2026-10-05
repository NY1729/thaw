# QuickJS ↔ N-API typed Function/Symbol graph source plan (source-only, 2026-10-03)

Status: design and mutable `/tmp` prototype; no product execution or managed edits. Atomic graph v2 is the required baseline. Existing `nh` decodes through `thawGraphNapiProxyForHandle` to an object Proxy and is valid for native instances only. `NapiResultGraph::token` rejects native Function/Symbol, so relabeling either as `nh` is semantically false.

## Function lane

A native Function graph node needs `{"nfn":"<NapiValue ID>"}` plus one positive NAPI Reference in the wire's decimal-string `napiLeases`. `NapiGraphInput` must accept an incoming `nfn` only when that ID belongs to the pinned Env and is `Value::Function`; the wire's reference remains in the dispatch guard until result encoding. QuickJS graph decode creates a per-owner callable Proxy, cached by ID with weak reachability, and acquires a *second* positive Reference owned by that JS wrapper. Its collector/drop path releases the second Reference. The transfer Reference is released by `thawGraphDecodeOwned` as today. The reciprocal graph encoder must recognize that private wrapper before generic live `hdl` and emit `nfn`, with a fresh transfer Reference.

Invocation must send `[this,...args]` as a graph, once and in order, to the existing guarded `thaw_napi_handle_bridge` with a distinct `call_captured_graph` operation. The Rust callback currently exists in the carrier draft; it must use atomic `NapiGraphInput` so prelookup failures release both lease kinds. Construct must use a distinct operation and invoke existing `napi_new_instance`, which sets `CallbackInfo.new_target` and its native instance prototype; it cannot reuse call mode. A callable Proxy target can be a bound ordinary function (no nonconfigurable own `prototype`), with `apply`/`construct` traps and the existing owner-aware property/descriptor operations. Proxy invariants, `prototype`, `name`, `length`, and identity need dedicated controls. A Rust QuickJS Function closure owning the second positive Reference is preferable to a JS FinalizationRegistry-only pin because Drop releases on context teardown; the JS wrapper can capture a Rust invoker closure. The private factory can be installed with `Object::prop` read-only/nonconfigurable during bridge setup, then cached in runtime, avoiding a mutable global callback endpoint.

`NapiResultGraph::encode` currently emits `nh` for `is_native_instance` Object only; place `nfn` before Object and before generic Function rejection. Do not snapshot Function property values or read getters during identity checks. Tests must cover return→call, return→new/new_target, own method/properties, identity repeated returns, roundtrip back to original native Function, invalid callback target with transferred hdl/napiLeases, owner unload while wrapper alive, release on GC/context teardown, and no double release on constructor throw.

## Symbol lane

Use a distinct primitive token, e.g. `{"nsy":"<NapiValue ID>"}`, with one transferred positive Reference. QuickJS decode must reconstruct one JS Symbol per native ID and preserve local distinctness and registered identity. `Value::Symbol.description: String` is lossy for lone surrogates: add an exact UTF-16 sidecar on native Symbol creation and use it for description, `Symbol.for` key, property-key conversion, equality, and graph wire. Existing `utf16_strings` sidecar handles string values, but `GLOBAL_SYMBOLS` currently keys Rust String; that registry must be keyed by exact units or an equivalent injective representation. Roundtrip encoder must recognize private native-origin Symbols before the generic QuickJS Symbol/h-d-l path. Symbols cannot be `WeakRef` targets in the pinned QuickJS 0.12.2 source contract; an ID→JS Symbol cache may retain positive native roots for the context lifetime unless a shorter proven lifecycle is available. Do not claim full teardown/GC semantics from an unbounded strong map. Tests need high-vs-low surrogate-vs-replacement, same-description local distinctness, registered Symbol.for equivalence, property-key identity, and recipient Env retirement.

## Existing source anchors

- `crates/thaw-napi/src/napi/module_host.rs`: `napi_graph_value`, `NapiGraphInput`, `NapiResultGraph`, `thaw_napi_handle_bridge`.
- `crates/thaw-napi/src/napi/classes.rs`: `napi_new_instance` and `napi_strict_equals`.
- `crates/thaw-quickjs/src/quickjs/platform_globals/runtime.js`: `thawGraphEncode`, `thawGraphDecode`, owner-aware NAPI proxy state.
- `crates/thaw-quickjs/src/quickjs/api.rs`: `NativeGraphCallbackRoots`, graph prelookup lease cleanup, `install_graph_handle_functions`.
- `rquickjs-core-0.12.2/src/value/object/property.rs`: `Object::prop` supports read-only property publication.

This plan is a dependency gate for the full carrier; it is not a finished fix.
