//! The N-API side of the graph wire (PR 6a of the N-API seam). `decode_napi_wire` reads the wire thaw-napi
//! writes: the shared token and node decoder of host.rs with `GraphCx::napi` set, plus the lease lists.
//! The native tags (`TAGS`), the native pair proofs and the lone `nh`/`nfn`/`nsy` tokens are in the second half.
//! ponytail: no encoder yet (PR 6a-3, 6a-4).

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use crate::host::{
    classify, graph_field, graph_token, handle_is_live, thaw_tjs_release_handle, wire_owner,
    with_package, GraphCx, HostError, Native,
};
use crate::{Interp, Obj, Value};

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
        let mut cx = GraphCx::new(nodes.len(), true, leases.is_none(), owner);
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

// ---- Native tags, native pair proofs and lone tokens (PR 6a-2) ----

/// What a tagged object is: a native Function wrapper (`nfn`) or a native-handle proxy (`nh`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Tag {
    Function,
    Handle,
}

thread_local! {
    /// The native identity of the JS objects the proxy layer made, keyed by address. The entry holds its
    /// `Obj`, so the address is not reused while the entry lives.
    /// ponytail: entries leak until the thread ends; a weak guard replaces the strong `Obj` when WeakRef gets real weakness (tj90 Q3).
    static TAGS: RefCell<HashMap<usize, (Obj, Tag, String)>> = RefCell::new(HashMap::new());
}

/// Record `object` as the native `kind` named by the decimal handle `native`.
pub(crate) fn tag(object: &Obj, kind: Tag, native: &str) {
    TAGS.with(|t| {
        t.borrow_mut().insert(
            Rc::as_ptr(object) as usize,
            (object.clone(), kind, native.to_string()),
        )
    });
}

/// Whether `text` is a positive decimal (`^[1-9][0-9]*$`), the form the wire spells native handles in.
pub(crate) fn is_decimal(text: &str) -> bool {
    !text.is_empty()
        && !text.starts_with('0')
        && text.bytes().all(|b| b.is_ascii_digit())
        && text.parse::<u64>().is_ok()
}

/// Whether `value` is proven to be the native `native` of `kind`: kind 1 a tagged native Function
/// (`thawGraphNapiFunctionHandles`), kind 2 a Symbol whose native id the bridge's metadata for `native`
/// confirms (runtime.js:979-998). `owner` names the Symbol registry for kind 2.
pub(crate) fn native_pair_matches(
    owner: Option<u64>,
    value: &Value,
    native: &str,
    kind: u8,
) -> bool {
    if !is_decimal(native) {
        return false;
    }
    match (kind, value) {
        (1, Value::Object(o)) => TAGS.with(|t| {
            t.borrow()
                .get(&(Rc::as_ptr(o) as usize))
                .is_some_and(|(_, tag, dec)| *tag == Tag::Function && dec == native)
        }),
        (2, _) => {
            let Some(id) =
                owner.and_then(|owner| crate::napi_symbols::native_id_of_value(owner, value))
            else {
                return false;
            };
            let Ok(reply) = crate::napi_bridge::handle_op("symbol_metadata", native, "", "[]")
            else {
                return false;
            };
            let Some(v) = reply.get("value") else {
                return false;
            };
            v.get("id").and_then(|i| i.as_str()) == Some(id.to_string().as_str())
                && v.get("handle").and_then(|h| h.as_str()) == Some(native)
        }
        _ => false,
    }
}

/// The reply's `value` of a `symbol_metadata` operation, checked as runtime.js:372-384 checks it.
struct SymbolMeta {
    id: String,
    units: Vec<u16>,
    registered: bool,
    js_origin: bool,
}

fn symbol_metadata(native: &str) -> Result<SymbolMeta, String> {
    const INVALID: &str = "Invalid native Symbol metadata";
    let reply = crate::napi_bridge::handle_op("symbol_metadata", native, "", "[]")?;
    let v = reply.get("value").ok_or(INVALID)?;
    let id = v
        .get("id")
        .and_then(|i| i.as_str())
        .filter(|i| is_decimal(i));
    let units = v.get("units").and_then(|u| u.as_array()).and_then(|units| {
        units
            .iter()
            .map(|u| u.as_u64().and_then(|n| u16::try_from(n).ok()))
            .collect::<Option<Vec<u16>>>()
    });
    match (
        id,
        units,
        v.get("handle").and_then(|h| h.as_str()),
        v.get("registered").and_then(|r| r.as_bool()),
        v.get("jsOrigin").and_then(|j| j.as_bool()),
    ) {
        (Some(id), Some(units), Some(handle), Some(registered), Some(js_origin))
            if handle == native =>
        {
            Ok(SymbolMeta {
                id: id.to_string(),
                units,
                registered,
                js_origin,
            })
        }
        _ => Err(INVALID.into()),
    }
}

/// The value of a live handle `handle` (a host handle id the wire names), with its reference still held.
fn live_value(handle: i64) -> Option<Value> {
    if !handle_is_live(handle) {
        return None;
    }
    crate::napi_ops::handle_value(handle as u64).ok()
}

/// A `{"nfn","hdl"}` node (`kind` 1) or a `{"nsy","hdl"}` token (`kind` 2): the value the handle names,
/// once `native_pair_matches` proves the pair (runtime.js:1964-1976, 2036-2044). The handle's reference is
/// released when `release` is set, as the in-node `hdl` arm does.
pub(crate) fn native_pair(
    owner: usize,
    fields: &[(String, Native)],
    kind: u8,
    release: bool,
) -> Result<Value, String> {
    let (field, invalid, mismatch) = if kind == 1 {
        (
            "nfn",
            "Invalid paired native Function graph node",
            "Mismatched native Function graph node",
        )
    } else {
        (
            "nsy",
            "Invalid paired native Symbol graph token",
            "Mismatched native Symbol graph token",
        )
    };
    let native = match graph_field(fields, field) {
        Some(Native::Str(s)) if is_decimal(s) => s.clone(),
        _ => return Err(invalid.into()),
    };
    let handle = match graph_field(fields, "hdl") {
        Some(Native::Number(n)) if n.fract() == 0.0 && *n >= 1.0 => *n as i64,
        _ => return Err(invalid.into()),
    };
    let value = live_value(handle).ok_or(invalid)?;
    if !native_pair_matches(Some(owner as u64), &value, &native, kind) {
        return Err(mismatch.into());
    }
    if release {
        thaw_tjs_release_handle(handle);
    }
    Ok(value)
}

/// A lone `{"nh"}` or `{"nfn"}` node: what the realm's proxy layer makes for the native handle `native`
/// (`__thaw_json_graph_proxy_for_handle` or `__thaw_napi_graph_function_for_handle`, installed by PR 6b).
pub(crate) fn lone_handle(it: &mut Interp, field: &str, native: &str) -> Result<Value, String> {
    let global_name = if field == "nh" {
        "__thaw_json_graph_proxy_for_handle"
    } else {
        "__thaw_napi_graph_function_for_handle"
    };
    let global = it.global.clone();
    let make = it
        .lookup(&global, global_name)
        .filter(Value::is_callable)
        .ok_or("Native addon proxy layer is not installed")?;
    it.call(&make, &Value::Undefined, &[Value::str(native)])
        .map_err(|c| classify(it, c).to_string())
}

/// A lone `{"nsy"}` token: the Symbol native id `native` names in `owner`, created from the bridge's metadata
/// when it is not known yet (`Symbol.for` for a registered one). Pinned unless it has a JavaScript origin or
/// is registered (runtime.js:2042-2050, 1244-1262).
pub(crate) fn lone_symbol(it: &mut Interp, owner: usize, native: &str) -> Result<Value, String> {
    let id = match native.parse::<u64>() {
        Ok(id) if is_decimal(native) => id,
        _ => return Err("Invalid native JSON graph token".into()),
    };
    let meta = symbol_metadata(native)?;
    if meta.id != native {
        return Err("Invalid native Symbol metadata".into());
    }
    let symbol = match crate::napi_symbols::symbol_of(owner as u64, id) {
        Some(symbol) => symbol,
        None => {
            // ponytail: a JavaScript-origin Symbol needs the proxy layer (PR 6b).
            if meta.js_origin {
                return Err(
                    "native Symbol with a JavaScript origin needs the N-API proxy layer".into(),
                );
            }
            let global = it.global.clone();
            let ctor = it
                .lookup(&global, "Symbol")
                .ok_or("Symbol is not defined")?;
            let description = Value::str(&crate::builtins::units_str(&meta.units));
            let symbol = if meta.registered {
                let symbol_for = it
                    .get(&ctor, "for")
                    .map_err(|c| classify(it, c).to_string())?;
                it.call(&symbol_for, &Value::Undefined, &[description])
            } else {
                it.call(&ctor, &Value::Undefined, &[description])
            }
            .map_err(|c| classify(it, c).to_string())?;
            let Value::Object(object) = &symbol else {
                return Err("native Symbol did not create a Symbol".into());
            };
            crate::napi_symbols::bind(owner as u64, id, object);
            symbol
        }
    };
    if !meta.js_origin && !meta.registered {
        crate::napi_symbols::thaw_tjs_napi_symbol_set_pin(owner as u64, id, 1);
    }
    Ok(symbol)
}
