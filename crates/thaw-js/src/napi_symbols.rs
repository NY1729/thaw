//! Symbol identity of the N-API engine seam (PR 3 of the stage 2 plan): a registry from the native
//! symbol id to the thaw-js Symbol it names, per graph owner (the napi environment's owner id, an opaque
//! key), the `{"nsy","hdl"}` wire token that carries the id with a handle to the Symbol, and the pin that
//! keeps a native-created Symbol alive. Follows the bridge protocol of thaw-quickjs's runtime.js: the
//! native id comes from the bridge (`symbol_graph`), a native handle is made through `symbol_metadata`.
//! ponytail: `rt.symbols` (collections.rs) holds every Symbol strongly, so the `Weak` never dies and
//! `thaw_tjs_napi_symbol_weak_live` answers live until that table is weak. Add when WeakRef gets a real weakness.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::{Rc, Weak};

use crate::host::{
    classify, default_realm, error_text, handle_result, store_handle, thaw_tjs_release_handle,
    with_package, HandleResult, HostError,
};
use crate::napi_bridge::{handle_op, keep_until_shutdown, owner_of_handle, registered as bridged};
use crate::napi_graph::{is_decimal, symbol_from_metadata, symbol_metadata, SymbolMeta};
use crate::napi_ops::handle_value;
use crate::napi_props::{thaw_tjs_abstract_to_string_units_result, ThawStringCoercionResult};
use crate::{Interp, Kind, Obj, ObjData, Value};

const OWNER_UNAVAILABLE: &str = "Native Symbol owner is unavailable";

struct Entry {
    weak: Weak<ObjData>,
    /// Set while pinned: a strong reference that keeps a native-created Symbol alive.
    pinned: Option<Obj>,
}

#[derive(Default)]
struct Registry {
    /// Last id allocated locally, for the standalone registry (no bridge registered).
    last: u64,
    /// (owner, native id) -> the Symbol it names.
    by_native: HashMap<(u64, u64), Entry>,
    /// (owner, thaw-js Symbol id) -> native id.
    by_symbol: HashMap<(u64, u64), u64>,
}

thread_local! {
    static REGISTRY: RefCell<Registry> = RefCell::new(Registry::default());
}

/// Whether `owner` names a graph owner this registry serves. With a bridge, any owner is an opaque key. Without one
/// (standalone), `owner` is a package index of this thread.
fn valid_owner(owner: u64) -> bool {
    bridged() || with_package(owner as usize, |_| Ok(())).is_ok()
}

/// The thaw-js Symbol id of `value`, `None` for anything but a Symbol.
fn symbol_identity(value: &Value) -> Option<(u64, Obj)> {
    let Value::Object(o) = value else {
        return None;
    };
    let id = symbol_id(o)?;
    Some((id, o.clone()))
}

/// The thaw-js Symbol id of a Symbol object.
fn symbol_id(symbol: &Obj) -> Option<u64> {
    match &*symbol.kind.borrow() {
        Kind::Symbol { id, .. } => Some(*id),
        _ => None,
    }
}

/// Whether `value` is a Symbol.
pub(crate) fn is_symbol(value: &Value) -> bool {
    symbol_identity(value).is_some()
}

/// Name `symbol` (a Symbol object) by `id` in `owner`, replacing any earlier name of it.
pub(crate) fn bind(owner: u64, id: u64, symbol: &Obj) {
    let Some(symbol_id) = symbol_id(symbol) else {
        return;
    };
    let entry = Entry {
        weak: Rc::downgrade(symbol),
        pinned: None,
    };
    REGISTRY.with(|r| {
        let mut r = r.borrow_mut();
        r.by_symbol.insert((owner, symbol_id), id);
        r.by_native.insert((owner, id), entry);
    });
}

/// Forget the name `id` of `symbol` in `owner` (a rollback of `bind`).
fn unbind(owner: u64, id: u64, symbol: &Obj) {
    let Some(symbol_id) = symbol_id(symbol) else {
        return;
    };
    REGISTRY.with(|r| {
        let mut r = r.borrow_mut();
        r.by_symbol.remove(&(owner, symbol_id));
        r.by_native.remove(&(owner, id));
    });
}

/// The native id `owner` has bound to the thaw-js Symbol `symbol_id`, if any.
fn bound_id(owner: u64, symbol_id: u64) -> Option<u64> {
    REGISTRY.with(|r| r.borrow().by_symbol.get(&(owner, symbol_id)).copied())
}

/// The native id a Symbol is known by: `owner`'s own name first, else the name the lowest owner gave it.
pub(crate) fn native_id_of_value(owner: u64, value: &Value) -> Option<u64> {
    let (symbol_id, _) = symbol_identity(value)?;
    let bridge = bridged();
    REGISTRY.with(|r| {
        let r = r.borrow();
        if let Some(id) = r.by_symbol.get(&(owner, symbol_id)) {
            return Some(*id);
        }
        if !bridge {
            return None;
        }
        // ponytail: two addons sharing one JS Symbol resolve to the lowest owner's name; upgrade = thread the
        // graph owner id into `DecodeCx`/`WireOut`.
        r.by_symbol
            .iter()
            .filter(|((_, s), _)| *s == symbol_id)
            .min_by_key(|((o, _), _)| *o)
            .map(|(_, id)| *id)
    })
}

/// The Symbol that native id `id` names in `owner`, while it is alive (pinned or still referenced).
pub(crate) fn symbol_of(owner: u64, id: u64) -> Option<Value> {
    REGISTRY.with(|r| {
        let r = r.borrow();
        let entry = r.by_native.get(&(owner, id))?;
        let symbol = entry.pinned.clone().or_else(|| entry.weak.upgrade())?;
        Some(Value::Object(symbol))
    })
}

/// The property key a Symbol takes in the object model (`@@sym:<id>`), for a Symbol value.
pub(crate) fn symbol_key(value: &Value) -> Option<String> {
    symbol_identity(value).map(|(id, _)| format!("@@sym:{id}"))
}

/// The native id (a decimal text) of a `symbol_graph`, `symbol_handle` or `retain_graph_handle` reply's `value`.
fn reply_id(reply: &serde_json::Value) -> Option<u64> {
    reply
        .get("value")
        .and_then(|v| v.as_str())
        .filter(|v| is_decimal(v))
        .and_then(|v| v.parse::<u64>().ok())
}

/// `graph_symbol_register`: the native id of the Symbol behind `handle` in `owner` (0 if not a Symbol, or the
/// bridge cannot name it). A Symbol `owner` has not named yet gets its id from the bridge (`symbol_graph`),
/// is bound to it, and is pinned through `symbol_pin` unless it is permanent (`Symbol.for` or well-known).
#[no_mangle]
pub extern "C" fn thaw_tjs_napi_graph_symbol_register(owner: u64, handle: u64) -> u64 {
    if !valid_owner(owner) || handle == 0 {
        return 0;
    }
    let Ok(value) = handle_value(handle) else {
        return 0;
    };
    let Some((symbol_id, symbol)) = symbol_identity(&value) else {
        return 0;
    };
    if let Some(id) = bound_id(owner, symbol_id) {
        return id;
    }
    if !bridged() {
        // ponytail: standalone (no bridge): native ids are invented here; the napi bridge supplies them otherwise.
        let id = REGISTRY.with(|r| {
            let mut r = r.borrow_mut();
            r.last += 1;
            r.last
        });
        bind(owner, id, &symbol);
        return id;
    }
    // ponytail: the registries of the default realm answer for every Symbol; a Symbol of another realm
    // is taken as non-registered and non-well-known.
    let (key, well_known) = match with_package(default_realm(), |p| {
        Ok((
            p.interp.symbol_key_for(&symbol),
            p.interp.is_well_known(&symbol),
        ))
    }) {
        Ok(found) => found,
        Err(_) => return 0,
    };
    let permanent = key.is_some() || well_known;
    let name = match &key {
        Some(key) => key.to_string(),
        None => match &*symbol.kind.borrow() {
            Kind::Symbol { desc, .. } => desc.as_deref().unwrap_or("").to_string(),
            _ => String::new(),
        },
    };
    let args = format!("[{},null,{permanent}]", key.is_some());
    let Some(id) = handle_op("symbol_graph", &owner.to_string(), &name, &args)
        .ok()
        .and_then(|reply| reply_id(&reply))
        .filter(|id| *id != 0)
    else {
        return 0;
    };
    bind(owner, id, &symbol);
    if !permanent && handle_op("symbol_pin", &owner.to_string(), &id.to_string(), "[]").is_err() {
        let _ = handle_op("symbol_rollback", &owner.to_string(), &id.to_string(), "[]");
        unbind(owner, id, &symbol);
        return 0;
    }
    id
}

/// 1 if the handle holds a Symbol, 0 if it holds anything else, 2 if the handle is invalid.
#[no_mangle]
pub extern "C" fn thaw_tjs_napi_handle_is_symbol(handle: u64) -> u8 {
    if handle == 0 {
        return 2;
    }
    match handle_value(handle) {
        Ok(value) => u8::from(is_symbol(&value)),
        Err(_) => 2,
    }
}

/// 1 while the native id's Symbol is pinned or still referenced, 0 for an unknown id, 2 for an invalid owner or id.
#[no_mangle]
pub extern "C" fn thaw_tjs_napi_symbol_weak_live(owner: u64, id: u64) -> u8 {
    if !valid_owner(owner) || id == 0 {
        return 2;
    }
    REGISTRY.with(|r| match r.borrow().by_native.get(&(owner, id)) {
        Some(entry) => u8::from(entry.pinned.is_some() || entry.weak.strong_count() > 0),
        None => 0,
    })
}

/// Pin (`pin != 0`) or unpin the native id's Symbol. Returns 1 when it took effect, 0 for an unknown or collected Symbol.
#[no_mangle]
pub extern "C" fn thaw_tjs_napi_symbol_set_pin(owner: u64, id: u64, pin: u8) -> u8 {
    if !valid_owner(owner) || id == 0 {
        return 0;
    }
    REGISTRY.with(|r| {
        let mut r = r.borrow_mut();
        let Some(entry) = r.by_native.get_mut(&(owner, id)) else {
            return 0;
        };
        if pin == 0 {
            entry.pinned = None;
            return 1;
        }
        match entry.weak.upgrade() {
            Some(symbol) => {
                entry.pinned = Some(symbol);
                1
            }
            None => 0,
        }
    })
}

/// The Symbol of native handle `native` in `owner` (runtime.js `thawHostNativeSymbolMake`): the bridge's metadata
/// decides it. A Symbol `owner` has not seen yet is retained once for the rest of the run, unless it has a
/// JavaScript origin or is registered.
fn make_from_native_handle(owner: u64, native: u64) -> Result<Value, String> {
    let text = native.to_string();
    let meta: SymbolMeta = symbol_metadata(&text)?;
    let id: u64 = meta
        .id
        .parse()
        .map_err(|_| "Invalid native Symbol metadata".to_string())?;
    if symbol_of(owner, id).is_none() && !meta.js_origin && !meta.registered {
        let token = handle_op("retain_graph_handle", &text, "", "[]")
            .ok()
            .and_then(|reply| reply_id(&reply))
            .filter(|token| *token != 0)
            .ok_or("Native Symbol cannot be retained")?;
        keep_until_shutdown(token);
    }
    with_package(default_realm(), |p| {
        Ok(symbol_from_metadata(&mut p.interp, owner, &meta))
    })
    .map_err(error_text)?
}

/// The Symbol native id `id` names in the bridge (`symbol_handle`), as a native handle that `owner` owns.
fn native_handle_of_id(owner: u64, id: u64) -> Result<u64, String> {
    let reply = handle_op("symbol_handle", &id.to_string(), "", "[]")
        .map_err(|_| OWNER_UNAVAILABLE.to_string())?;
    let native = reply_id(&reply).ok_or(OWNER_UNAVAILABLE)?;
    if owner_of_handle(native) != Some(owner) {
        return Err(OWNER_UNAVAILABLE.into());
    }
    Ok(native)
}

/// The Symbol native handle `native_handle` names in its owner, created (with its description) and named on first use.
#[no_mangle]
pub extern "C" fn thaw_tjs_native_symbol_handle_result(native_handle: u64) -> HandleResult {
    handle_result((|| {
        if native_handle == 0 {
            return Err("invalid native Symbol id".to_string());
        }
        if !bridged() {
            return standalone_native_symbol(native_handle);
        }
        let owner = owner_of_handle(native_handle).ok_or(OWNER_UNAVAILABLE)?;
        let symbol = make_from_native_handle(owner, native_handle)?;
        Ok(store_handle(default_realm(), symbol))
    })())
}

/// Without a bridge: the Symbol native id `native_handle` names in the default realm, created (with no
/// description) and named by that id on first use.
fn standalone_native_symbol(native_handle: u64) -> Result<i64, String> {
    let owner = default_realm();
    if let Some(symbol) = symbol_of(owner as u64, native_handle) {
        return Ok(store_handle(owner, symbol));
    }
    with_package(owner, |p| {
        let symbol = p
            .interp
            .new_symbol(None)
            .map_err(|c| classify(&mut p.interp, c))?;
        if let Value::Object(o) = &symbol {
            bind(owner as u64, native_handle, o);
        }
        Ok(store_handle(owner, symbol))
    })
    .map_err(error_text)
}

/// ToString of the native id's Symbol: the engine's TypeError, as an exception handle.
#[no_mangle]
pub extern "C" fn thaw_tjs_napi_symbol_abstract_to_string_units_result(
    owner: u64,
    id: u64,
) -> ThawStringCoercionResult {
    let fail = |message: &str| ThawStringCoercionResult {
        units: std::ptr::null_mut(),
        exception_handle: 0,
        exception_object_like: 0,
        error: thaw_arena::owned_string(message),
    };
    if !valid_owner(owner) || id == 0 {
        return fail("invalid native Symbol owner");
    }
    let bridge = bridged();
    let symbol = match symbol_of(owner, id) {
        Some(symbol) => symbol,
        None if !bridge => return fail("native Symbol bridge did not return a Symbol"),
        None => {
            match native_handle_of_id(owner, id).and_then(|n| make_from_native_handle(owner, n)) {
                Ok(symbol) => symbol,
                Err(e) => return fail(&e),
            }
        }
    };
    let realm = if bridge {
        default_realm()
    } else {
        owner as usize
    };
    let handle = store_handle(realm, symbol);
    let out = thaw_tjs_abstract_to_string_units_result(handle as u64, 0);
    thaw_tjs_release_handle(handle);
    out
}

/// The intrinsic `Symbol` constructor of `it` (not the global binding, which user code may replace).
fn intrinsic_symbol(it: &Interp) -> Result<Value, HostError> {
    it.symbol_ctor
        .clone()
        .map(Value::Object)
        .ok_or_else(|| HostError::Thrown("Symbol is not defined".into()))
}

/// The Symbol a wire key text names in `owner`: `\u{3}<id>` / `\u{3}R<id>:...` (a unique or registered
/// Symbol of this runtime) or `\u{1f}@@<name>` (a well-known Symbol). A new handle to it.
pub(crate) fn key_text_symbol(owner: usize, text: &str) -> Result<i64, String> {
    with_package(owner, |p| {
        let it = &mut p.interp;
        let symbol = if let Some(name) = text.strip_prefix("\u{1f}@@") {
            let ctor = intrinsic_symbol(it)?;
            it.get(&ctor, name).map_err(|c| classify(it, c))?
        } else {
            let rest = text.strip_prefix('\u{3}').ok_or_else(|| {
                HostError::Thrown("native Symbol token did not decode to a Symbol".into())
            })?;
            let rest = rest.strip_prefix('R').unwrap_or(rest);
            let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
            let id = digits.parse::<u64>().map_err(|_| {
                HostError::Thrown("native Symbol token did not decode to a Symbol".into())
            })?;
            it.key_value(&format!("@@sym:{id}"))
        };
        if !is_symbol(&symbol) {
            return Err(HostError::Thrown(
                "native Symbol token did not decode to a Symbol".into(),
            ));
        }
        Ok(store_handle(owner, symbol))
    })
    .map_err(error_text)
}
