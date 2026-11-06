//! Symbol identity of the N-API engine seam (PR 3 of the stage 2 plan): a registry from the native
//! symbol id to the thaw-js Symbol it names, per owner (package), the `{"nsy","hdl"}` wire token that
//! carries the id with a handle to the Symbol, and the pin that keeps a native-created Symbol alive.
//! Mirrors the `thaw_js_napi_*symbol*` and `thaw_js_native_symbol_handle_result` functions of
//! thaw-quickjs, which answer through JS helpers; here the registry is plain Rust.
//! ponytail: `rt.symbols` (collections.rs) holds every Symbol strongly, so the `Weak` never dies and
//! `thaw_tjs_napi_symbol_weak_live` answers live until that table is weak. Add when WeakRef gets a real weakness.
//! ponytail: native ids are allocated here for symbols the napi side never named; the napi bridge (PR 5)
//! should supply them, otherwise the two id spaces could collide.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::{Rc, Weak};

use crate::host::{
    classify, default_realm, error_text, handle_result, store_handle, thaw_tjs_release_handle,
    with_package, HandleResult, HostError,
};
use crate::napi_ops::handle_value;
use crate::napi_props::{thaw_tjs_abstract_to_string_units_result, ThawStringCoercionResult};
use crate::{Kind, Obj, ObjData, Value};

struct Entry {
    weak: Weak<ObjData>,
    /// Set while pinned: a strong reference that keeps a native-created Symbol alive.
    pinned: Option<Obj>,
}

#[derive(Default)]
struct Registry {
    last: u64,
    /// (owner, native id) -> the Symbol it names.
    by_native: HashMap<(u64, u64), Entry>,
    /// (owner, thaw-js Symbol id) -> native id.
    by_symbol: HashMap<(u64, u64), u64>,
}

thread_local! {
    static REGISTRY: RefCell<Registry> = RefCell::new(Registry::default());
}

/// Whether `owner` (a package index, 0 included) names a package of this thread.
fn known_owner(owner: u64) -> bool {
    with_package(owner as usize, |_| Ok(())).is_ok()
}

/// The thaw-js Symbol id of `value`, `None` for anything but a Symbol.
fn symbol_identity(value: &Value) -> Option<(u64, Obj)> {
    let Value::Object(o) = value else {
        return None;
    };
    let id = match &*o.kind.borrow() {
        Kind::Symbol { id, .. } => *id,
        _ => return None,
    };
    Some((id, o.clone()))
}

/// Whether `value` is a Symbol.
pub(crate) fn is_symbol(value: &Value) -> bool {
    symbol_identity(value).is_some()
}

/// Name `symbol` (a Symbol object) by `id` in `owner`, replacing any earlier name of it.
pub(crate) fn bind(owner: u64, id: u64, symbol: &Obj) {
    let Some(symbol_id) = (match &*symbol.kind.borrow() {
        Kind::Symbol { id, .. } => Some(*id),
        _ => None,
    }) else {
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

/// The native id a Symbol of `owner` is known by, if any.
pub(crate) fn native_id_of_value(owner: u64, value: &Value) -> Option<u64> {
    let (symbol_id, _) = symbol_identity(value)?;
    REGISTRY.with(|r| r.borrow().by_symbol.get(&(owner, symbol_id)).copied())
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

/// `graph_symbol_register`: the native id of the Symbol behind `handle` in `owner`, allocated on first use (0 if not a Symbol).
#[no_mangle]
pub extern "C" fn thaw_tjs_napi_graph_symbol_register(owner: u64, handle: u64) -> u64 {
    if !known_owner(owner) || handle == 0 {
        return 0;
    }
    let Ok(value) = handle_value(handle) else {
        return 0;
    };
    let Some((_, symbol)) = symbol_identity(&value) else {
        return 0;
    };
    if let Some(id) = native_id_of_value(owner, &value) {
        return id;
    }
    let id = REGISTRY.with(|r| {
        let mut r = r.borrow_mut();
        r.last += 1;
        r.last
    });
    bind(owner, id, &symbol);
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

/// 1 while the native id's Symbol is pinned or still referenced, 0 for an unknown id, 2 for an unknown owner (never dead).
#[no_mangle]
pub extern "C" fn thaw_tjs_napi_symbol_weak_live(owner: u64, id: u64) -> u8 {
    if !known_owner(owner) || id == 0 {
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
    if !known_owner(owner) || id == 0 {
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

/// The Symbol native id `native_handle` names in the default realm, created (with no description) and named on first use.
#[no_mangle]
pub extern "C" fn thaw_tjs_native_symbol_handle_result(native_handle: u64) -> HandleResult {
    handle_result((|| {
        if native_handle == 0 {
            return Err("invalid native Symbol id".to_string());
        }
        let owner = default_realm();
        if let Some(symbol) = symbol_of(owner as u64, native_handle) {
            return Ok(store_handle(owner, symbol));
        }
        with_package(owner, |p| {
            let it = &mut p.interp;
            let ctor = it
                .lookup(&it.global.clone(), "Symbol")
                .unwrap_or(Value::Undefined);
            let symbol = it
                .call(&ctor, &Value::Undefined, &[])
                .map_err(|c| classify(it, c))?;
            if let Value::Object(o) = &symbol {
                bind(owner as u64, native_handle, o);
            }
            Ok(store_handle(owner, symbol))
        })
        .map_err(error_text)
    })())
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
    if !known_owner(owner) || id == 0 {
        return fail("invalid native Symbol owner");
    }
    let Some(symbol) = symbol_of(owner, id) else {
        return fail("native Symbol bridge did not return a Symbol");
    };
    let handle = store_handle(owner as usize, symbol);
    let out = thaw_tjs_abstract_to_string_units_result(handle as u64, 0);
    thaw_tjs_release_handle(handle);
    out
}

/// The Symbol a wire key text names in `owner`: `\u{3}<id>` / `\u{3}R<id>:...` (a unique or registered
/// Symbol of this runtime) or `\u{1f}@@<name>` (a well-known Symbol). A new handle to it.
pub(crate) fn key_text_symbol(owner: usize, text: &str) -> Result<i64, String> {
    with_package(owner, |p| {
        let it = &mut p.interp;
        let symbol = if let Some(name) = text.strip_prefix("\u{1f}@@") {
            let ctor = it
                .lookup(&it.global.clone(), "Symbol")
                .unwrap_or(Value::Undefined);
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
