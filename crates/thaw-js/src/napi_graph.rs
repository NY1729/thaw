//! The N-API side of the graph wire (PR 6a of the N-API seam). `decode_napi_wire` reads the wire thaw-napi
//! writes: the shared token and node decoder of host.rs with `GraphCx::napi` set, plus the lease lists.
//! ponytail: no `nfn`/`nh`/lone `nsy` tokens yet (PR 6a-2), no encoder yet (PR 6a-3, 6a-4).

use crate::host::{
    graph_field, graph_token, thaw_tjs_release_handle, wire_owner, with_package, GraphCx,
    HostError, Native,
};
use crate::Value;

/// Decode an N-API graph wire into `(owning package, value)`. With a `leases` list, exactly those handle
/// references are released once decoding ends, on success and on error; a wire without the list (the
/// plain wire's shape) releases its `hdl` references in-node, as `decode_wire` does. `napiLeases`
/// references are released through the bridge in both cases.
#[allow(dead_code)] // wired up by the JS-visible globals in PR 6a-5
pub(crate) fn decode_napi_wire(wire: &str) -> Result<(usize, Value), String> {
    let Native::Object(top) = Native::from_json(wire).map_err(|e| e.to_string())? else {
        return Err("malformed graph wire".into());
    };
    let root = graph_field(&top, "root").ok_or("graph wire has no root")?;
    let nodes = match graph_field(&top, "nodes") {
        Some(Native::Array(nodes)) => nodes.as_slice(),
        _ => &[],
    };
    // ponytail: a malformed lease list returns before any release; the encoder's references then leak.
    let leases = match graph_field(&top, "leases") {
        None => None,
        Some(Native::Array(ids)) => Some(
            ids.iter()
                .map(|id| match id {
                    Native::Number(n) if n.fract() == 0.0 && *n >= 1.0 => Ok(*n as i64),
                    _ => Err("malformed graph lease".to_string()),
                })
                .collect::<Result<Vec<i64>, String>>()?,
        ),
        Some(_) => return Err("malformed graph leases".into()),
    };
    let napi_leases = match graph_field(&top, "napiLeases") {
        None => Vec::new(),
        Some(Native::Array(refs)) => refs
            .iter()
            .map(|r| match r {
                Native::Str(s) => s
                    .parse::<u64>()
                    .map_err(|_| "malformed graph lease".to_string()),
                _ => Err("malformed graph lease".to_string()),
            })
            .collect::<Result<Vec<u64>, String>>()?,
        Some(_) => return Err("malformed graph napi leases".into()),
    };
    let owner = wire_owner(nodes);
    let decoded = with_package(owner, |p| {
        let mut cx = GraphCx::new(nodes.len(), true, leases.is_none());
        graph_token(&mut p.interp, root, nodes, &mut cx).map_err(HostError::Bundle)
    });
    for id in leases.iter().flatten() {
        thaw_tjs_release_handle(*id);
    }
    for reference in napi_leases {
        crate::napi_bridge::release_reference(reference);
    }
    decoded.map(|v| (owner, v)).map_err(|e| e.to_string())
}
