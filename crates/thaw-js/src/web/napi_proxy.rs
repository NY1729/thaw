//! The native addon proxy layer (PR 6b of the N-API seam): `web/napi_proxy.js`, the JS half of the graph
//! lane that turns native handles into JS proxies. Opt-in per thread: `thaw_tjs_napi_enable_proxy_layer`
//! enables the default realm now, and every host realm `Interp::new` builds afterwards gets it
//! (`napi_bridge::install` calls `enable_if_requested`). Other realms keep the PR 6a
//! "Native addon proxy layer is not installed" error.

use std::cell::Cell;

use crate::host::{default_realm, with_package};
use crate::{arg, napi_bridge, napi_graph, new_obj, Interp, Kind, Value, R};

const SOURCE: &str = include_str!("napi_proxy.js");

thread_local! {
    /// Whether this thread's new realms get the layer. Thread-local, like the bridge's per-realm state, so
    /// one thread's opt-in never reaches another thread's realms.
    static ENABLED: Cell<bool> = const { Cell::new(false) };
}

/// Opts this thread into the proxy layer. Idempotent. Nothing calls it yet (PR 7 wires it into the bridge
/// registration).
#[no_mangle]
pub extern "C" fn thaw_tjs_napi_enable_proxy_layer() {
    ENABLED.with(|e| e.set(true));
    let _ = with_package(default_realm(), |p| {
        enable(&mut p.interp);
        Ok(())
    });
}

/// Enables the layer in a freshly built host realm when this thread opted in.
pub(crate) fn enable_if_requested(it: &mut Interp) {
    if ENABLED.with(Cell::get) {
        enable(it);
    }
}

/// Installs the layer in `it`: runs the script, calls its factory with the private natives and declares the
/// globals the decoder and the shim read (immutable bindings, as the bridge globals are). A realm that already
/// has them is left alone.
pub(crate) fn enable(it: &mut Interp) {
    let global = it.global.clone();
    if it
        .lookup(&global, "__thaw_json_graph_napi_exports")
        .is_some()
    {
        return;
    }
    let api = factory_api(it);
    for (name, field) in [
        ("__thaw_json_graph_napi_exports", "exports"),
        ("__thaw_json_graph_napi_each_export", "eachExport"),
        ("__thaw_json_graph_proxy_for_handle", "proxyForHandle"),
        ("__thaw_napi_graph_function_for_handle", "functionForHandle"),
        ("__thaw_json_graph_napi_call_export", "callExport"),
        ("__thaw_json_graph_napi_construct", "construct"),
    ] {
        let value = it
            .get(&api, field)
            .unwrap_or_else(|_| panic!("native addon proxy layer exports {field}"));
        it.global.declare(name, value, false);
    }
    // Last: the shim takes the trusted path as soon as this reports the bridge, so every other global is in place.
    let available = it
        .get(&api, "bridgeAvailable")
        .unwrap_or_else(|_| panic!("native addon proxy layer exports bridgeAvailable"));
    it.global
        .declare("__thaw_json_graph_napi_bridge_available", available, false);
    let pair = native_pair_matches_global(it);
    it.global
        .declare("__thaw_json_graph_native_pair_matches", pair, false);
}

/// Runs the layer's script in `it` and calls its factory with the private natives: the factory's object, which
/// holds the layer's functions. Each call makes a separate instance with its own caches.
pub(crate) fn factory_api(it: &mut Interp) -> Value {
    let factory = it
        .run_sloppy(SOURCE)
        .unwrap_or_else(|_| panic!("native addon proxy layer must evaluate"));
    let cap = cap_object(it);
    it.call(&factory, &Value::Undefined, &[cap])
        .unwrap_or_else(|_| panic!("native addon proxy layer factory must run"))
}

/// The private natives the layer's factory receives: the bridge's exports and owner lookups, and the invoker
/// factory. Not globals: only the layer's own script sees them.
fn cap_object(it: &mut Interp) -> Value {
    let global = it.global.clone();
    let exports = it
        .lookup(&global, "__thaw_napi_bridge_exports")
        .unwrap_or(Value::Undefined);
    let owner = it
        .lookup(&global, "__thaw_napi_graph_owner")
        .unwrap_or(Value::Undefined);
    let invoker = napi_bridge::graph_invoker_factory(it);
    let cap = Value::Object(new_obj(Some(it.object_proto.clone()), Kind::Plain));
    for (name, value) in [("exports", exports), ("owner", owner), ("invoker", invoker)] {
        let _ = it.set(&cap, name, value);
    }
    cap
}

/// `__thaw_json_graph_native_pair_matches(value, nativeHandle, kind)`: the Rust `native_pair_matches` of the
/// realm's package (kind 1 a tagged native Function, kind 2 a bridge-confirmed Symbol), false otherwise.
fn native_pair_matches_global(it: &mut Interp) -> Value {
    it.native(|_, _, args| -> R<Value> {
        let kind = match arg(args, 2) {
            Value::Number(1.0) => 1,
            Value::Number(2.0) => 2,
            _ => return Ok(Value::Bool(false)),
        };
        let native = match arg(args, 1) {
            Value::Str(s) => s.to_string(),
            _ => return Ok(Value::Bool(false)),
        };
        let owner = default_realm() as u64;
        Ok(Value::Bool(napi_graph::native_pair_matches(
            Some(owner),
            &arg(args, 0),
            &native,
            kind,
        )))
    })
}
