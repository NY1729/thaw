//! Exception-capturing property operations of the N-API engine seam (PR 2 of the stage 2 plan). A JS
//! throw from a getter, setter, proxy trap or coercion is retained as a handle, the thrown value
//! itself, so its identity survives; a key may be a string handle or a Symbol handle. Each mirrors
//! the same-named `thaw_js_*` function of thaw-quickjs.

use std::cell::Cell;
use std::collections::{HashMap, HashSet};
use std::ffi::{c_char, CStr, CString};
use std::rc::Rc;

use crate::extras::Hint;
use crate::host::{
    classify, cstr, decode_wire, graph_result, handle_result, json_key, on_handle, store_handle,
    HandleResult, HostError, Native, ThawResult, WireOut,
};
use crate::napi_graph::{decode_napi_wire, encode_napi_graph};
use crate::napi_ops::{handle_value, prototype_of};
use crate::{
    array_index, builtins, find_own, order_keys, push_prop, Ctl, Interp, Kind, Obj, Prop, Slot,
    Value, R,
};

/// A property read or write result: a value, or a thrown value kept as a handle (`exception_handle`).
#[repr(C)]
pub struct ThawCallWithExceptionResult {
    pub value: u64,
    pub exception_handle: u64,
    pub exception_object_like: u8,
    pub error: *mut c_char,
}

/// A ToString result: the UTF-16 units as JSON text, or a thrown value kept as a handle.
#[repr(C)]
pub struct ThawStringCoercionResult {
    pub units: *mut c_char,
    pub exception_handle: u64,
    pub exception_object_like: u8,
    pub error: *mut c_char,
}

/// The napi side's callback bridge: `(operation, target, empty, graph)` answers the JSON text of
/// `{"kind":"graph","value":wire}` or `{"__thaw_error__":message}`. It is per thread, like the packages.
pub(crate) type CallbackBridge = unsafe extern "C" fn(
    *const c_char,
    *const c_char,
    *const c_char,
    *const c_char,
) -> *const c_char;

thread_local! {
    static CALLBACK_BRIDGE: Cell<Option<CallbackBridge>> = const { Cell::new(None) };
}

/// Install (or clear) the callback bridge of this thread. The napi bridge host calls this.
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn set_callback_bridge(bridge: Option<CallbackBridge>) {
    CALLBACK_BRIDGE.with(|b| b.set(bridge));
}

/// Outcome of an operation that may throw.
enum Exact<T> {
    Done(T),
    /// The thrown value's handle, and whether it is an object (or function).
    Thrown(u64, bool),
}

/// Run `op` on the value behind `handle`, in its package. A JS throw is retained; other failures
/// come back as text.
fn exact<T>(
    handle: u64,
    op: impl FnOnce(&mut Interp, usize, Value) -> R<T>,
) -> Result<Exact<T>, String> {
    on_handle(handle, |p, owner, target| {
        let it = &mut p.interp;
        match op(it, owner, target) {
            Ok(value) => Ok(Exact::Done(value)),
            Err(Ctl::Throw(thrown)) => {
                let object_like = matches!(thrown, Value::Object(_));
                Ok(Exact::Thrown(
                    store_handle(owner, thrown) as u64,
                    object_like,
                ))
            }
            Err(c) => Err(classify(it, c)),
        }
    })
}

fn failure(error: String) -> ThawCallWithExceptionResult {
    ThawCallWithExceptionResult {
        value: 0,
        exception_handle: 0,
        exception_object_like: 0,
        error: thaw_arena::owned_string(error),
    }
}

fn exception(handle: u64, object_like: bool) -> ThawCallWithExceptionResult {
    ThawCallWithExceptionResult {
        value: 0,
        exception_handle: handle,
        exception_object_like: u8::from(object_like),
        error: std::ptr::null_mut(),
    }
}

fn value_of(value: u64) -> ThawCallWithExceptionResult {
    ThawCallWithExceptionResult {
        value,
        exception_handle: 0,
        exception_object_like: 0,
        error: std::ptr::null_mut(),
    }
}

fn is_symbol(v: &Value) -> bool {
    matches!(v, Value::Object(o) if matches!(&*o.kind.borrow(), Kind::Symbol { .. }))
}

/// `Reflect.set(target, key, value, receiver)` as a bool: false where the write is refused (a
/// frozen object, a read-only or setter-less property, a non-extensible receiver). `receiver_data`
/// is the napi form: only an own writable data property of the receiver is written, and no setter runs.
// ponytail: a proxy receiver is written directly rather than through its defineProperty trap.
fn reflect_set(
    it: &mut Interp,
    target: &Obj,
    key: &str,
    value: Value,
    receiver: &Value,
    receiver_data: bool,
) -> R<bool> {
    if matches!(&*target.kind.borrow(), Kind::Proxy(_)) {
        let Some((inner, handler, trap)) = it.proxy_trap(target, "set")? else {
            return Ok(false);
        };
        if trap.is_callable() {
            let k = it.key_value(key);
            return Ok(it
                .call(&trap, &handler, &[inner, k, value, receiver.clone()])?
                .truthy());
        }
        return match &inner {
            Value::Object(inner) => reflect_set(it, inner, key, value, receiver, receiver_data),
            _ => Ok(false),
        };
    }
    let indexed = matches!(&*target.kind.borrow(), Kind::Array(_) | Kind::Typed(_));
    if indexed && !receiver_data {
        if target.frozen.get() {
            return Ok(false);
        }
        it.set(&Value::Object(target.clone()), key, value)?;
        return Ok(true);
    }
    if !receiver_data {
        if let Some((holder, slot, writable)) = find_with_holder(target, key) {
            match slot {
                Slot::Accessor { set, .. } => {
                    return match set {
                        Some(setter) => {
                            it.call(&setter, receiver, &[value])?;
                            Ok(true)
                        }
                        None => Ok(false),
                    };
                }
                Slot::Data(_) if !writable || holder.frozen.get() => return Ok(false),
                Slot::Data(_) => {}
            }
        }
    }
    let Value::Object(receiver) = receiver else {
        return Ok(false);
    };
    match receiver.position(key) {
        Some(i) => {
            let mut props = receiver.props.borrow_mut();
            if matches!(props[i].slot, Slot::Accessor { .. })
                || !props[i].writable
                || receiver.frozen.get()
            {
                return Ok(false);
            }
            props[i].slot = Slot::Data(value);
            Ok(true)
        }
        None => {
            if receiver.ext.get() != 0 || receiver.frozen.get() {
                return Ok(false);
            }
            let mut props = receiver.props.borrow_mut();
            push_prop(
                receiver,
                &mut props,
                Prop {
                    key: key.into(),
                    slot: Slot::Data(value),
                    enumerable: true,
                    writable: true,
                },
            );
            Ok(true)
        }
    }
}

/// The first own-or-inherited property of `key`: its holder, slot and writable flag.
fn find_with_holder(o: &Obj, key: &str) -> Option<(Obj, Slot, bool)> {
    let mut current = Some(o.clone());
    while let Some(node) = current {
        if let Some(i) = node.position(key) {
            let (slot, writable) = {
                let props = node.props.borrow();
                (props[i].slot.clone(), props[i].writable)
            };
            return Some((node, slot, writable));
        }
        current = node.proto.borrow().clone();
    }
    None
}

/// `Reflect.has` / own-has / `Reflect.deleteProperty` for `key`: the predicate operations of the
/// key-handle path (0 has, 1 own, 2 delete).
fn predicate(it: &mut Interp, target: &Value, key: &str, operation: u8) -> R<bool> {
    match operation {
        0 => {
            if !matches!(target, Value::Object(_)) {
                return it.throw_error("TypeError", "Reflect.has called on non-object");
            }
            it.has_property(target, key)
        }
        1 => Ok(matches!(target, Value::Object(o) if find_own(o, key).is_some())),
        2 => {
            let Value::Object(o) = target else {
                return it.throw_error("TypeError", "Reflect.deleteProperty called on non-object");
            };
            match it.proxy_delete(o, key)? {
                Some(done) => Ok(done),
                None if o.frozen.get() => Ok(false),
                None => {
                    o.remove_prop(key);
                    Ok(true)
                }
            }
        }
        _ => it.throw_error("TypeError", "Invalid native property operation"),
    }
}

/// Own properties of a non-proxy object as `(key, attributes)` in `Reflect.ownKeys` order. Attributes
/// are `1` writable, `2` enumerable, `4` configurable. Per-property configurability is not kept: a
/// frozen or sealed object's properties are non-configurable, the rest configurable.
// ponytail: configurability is per object, not per property.
pub(crate) fn own_entries(o: &Obj) -> Vec<(Rc<str>, u32)> {
    let frozen = o.frozen.get();
    let configurable = if frozen || o.ext.get() == 2 { 0 } else { 4 };
    let writable = if frozen { 0 } else { 1 };
    let mut out: Vec<(Rc<str>, u32)> = Vec::new();
    match &*o.kind.borrow() {
        Kind::Array(items) => {
            for (i, item) in items.iter().enumerate() {
                if !matches!(item, Value::Hole) {
                    out.push((i.to_string().into(), 2 | writable | configurable));
                }
            }
            out.push(("length".into(), writable));
        }
        Kind::Typed(t) => {
            for i in 0..t.len {
                out.push((i.to_string().into(), 2 | writable | configurable));
            }
        }
        Kind::Wrapper(Value::Str(s)) => {
            for i in 0..builtins::str_len(s) {
                out.push((i.to_string().into(), 2));
            }
        }
        _ => {}
    }
    let props = o.props.borrow();
    let mut strings: Vec<(Rc<str>, u32)> = Vec::new();
    let mut symbols: Vec<(Rc<str>, u32)> = Vec::new();
    for p in props.iter() {
        let accessor = matches!(p.slot, Slot::Accessor { .. });
        let attributes =
            (if accessor { 0 } else { writable }) | (u32::from(p.enumerable) << 1) | configurable;
        if p.key.starts_with("@@sym:") {
            symbols.push((p.key.clone(), attributes));
        } else {
            strings.push((p.key.clone(), attributes));
        }
    }
    let by_key: HashMap<Rc<str>, u32> = strings.iter().cloned().collect();
    for key in order_keys(strings.iter().map(|(k, _)| k.clone()).collect()) {
        let attributes = by_key[&key];
        out.push((key, attributes));
    }
    out.extend(symbols);
    out
}

/// The attributes of own property `key` of a proxy: its `getOwnPropertyDescriptor` trap when it has
/// one, else the target's. `None` when the key does not exist.
fn proxy_attributes(it: &mut Interp, o: &Obj, key: &str) -> R<Option<u32>> {
    let Some((inner, handler, trap)) = it.proxy_trap(o, "getOwnPropertyDescriptor")? else {
        return Ok(None);
    };
    let Value::Object(inner) = inner else {
        return Ok(None);
    };
    if !trap.is_callable() {
        return Ok(own_entries(&inner)
            .into_iter()
            .find(|(k, _)| **k == *key)
            .map(|(_, a)| a));
    }
    let k = it.key_value(key);
    let desc = it.call(&trap, &handler, &[Value::Object(inner), k])?;
    if !matches!(desc, Value::Object(_)) {
        return Ok(None);
    }
    let flag = |it: &mut Interp, name: &str| -> R<bool> { Ok(it.get(&desc, name)?.truthy()) };
    let writable = flag(it, "writable")?;
    let enumerable = flag(it, "enumerable")?;
    let configurable = flag(it, "configurable")?;
    Ok(Some(
        (u32::from(writable)) | (u32::from(enumerable) << 1) | (u32::from(configurable) << 2),
    ))
}

/// `Reflect.ownKeys` with attributes for `o`, a proxy included.
fn own_keys_with_attributes(it: &mut Interp, o: &Obj) -> R<Vec<(Rc<str>, u32)>> {
    if !matches!(&*o.kind.borrow(), Kind::Proxy(_)) {
        return Ok(own_entries(o));
    }
    let keys = it.proxy_own_keys(o, false)?.unwrap_or_default();
    let mut out = Vec::new();
    for key in keys {
        let key = it.key_of(&key);
        if let Some(a) = proxy_attributes(it, o, &key)? {
            out.push((key, a));
        }
    }
    Ok(out)
}

/// `handle[key]` as a result that keeps a thrown value's identity.
fn read_with_exception(handle: u64, key: Value) -> ThawCallWithExceptionResult {
    match exact(handle, |it, owner, target| {
        let key = it.key_of(&key);
        it.get(&target, &key).map(|v| store_handle(owner, v) as u64)
    }) {
        Ok(Exact::Done(value)) => value_of(value),
        Ok(Exact::Thrown(handle, like)) => exception(handle, like),
        Err(e) => failure(e),
    }
}

/// The write a set performs: decodes the graph and stores it under `key`. A refused write is a
/// TypeError (text) unless `receiver_data`, where it is value 0 with no error.
fn write_with_exception(
    handle: u64,
    key: Value,
    graph: *const c_char,
    receiver_data: bool,
) -> ThawCallWithExceptionResult {
    let value = match cstr(graph)
        .ok_or_else(|| "invalid graph".to_string())
        .and_then(|wire| decode_napi_wire(&wire))
    {
        Ok((_, value)) => value,
        Err(e) => return failure(e),
    };
    let key_name = match &key {
        Value::Str(s) => Some(s.to_string()),
        _ => None,
    };
    match exact(handle, |it, _, target| {
        let Value::Object(object) = &target else {
            return it.throw_error("TypeError", "Reflect.set called on non-object");
        };
        let key = it.key_of(&key);
        reflect_set(it, object, &key, value, &target, receiver_data)
    }) {
        Ok(Exact::Done(true)) => value_of(1),
        Ok(Exact::Done(false)) if receiver_data => value_of(0),
        Ok(Exact::Done(false)) => failure(match key_name {
            Some(key) => format!("\u{1}TypeError\u{1}Cannot assign to read only property '{key}' of object '#<Object>'"),
            None => "\u{1}TypeError\u{1}Cannot assign to read only property".to_string(),
        }),
        Ok(Exact::Thrown(handle, like)) => exception(handle, like),
        Err(e) => failure(e),
    }
}

/// `handle[key]` for a JSON key: a property read that keeps a thrown value as a handle.
#[no_mangle]
pub extern "C" fn thaw_tjs_get_property_json_key_with_exception_result(
    handle: u64,
    key_json: *const c_char,
) -> ThawCallWithExceptionResult {
    match json_key(key_json) {
        Ok(key) => read_with_exception(handle, Value::Str(key.into())),
        Err(e) => failure(e),
    }
}

/// `handle[key]` with the key a string or Symbol handle.
#[no_mangle]
pub extern "C" fn thaw_tjs_get_property_key_handle_with_exception_result(
    handle: u64,
    key_handle: u64,
) -> ThawCallWithExceptionResult {
    match handle_value(key_handle) {
        Ok(key) => read_with_exception(handle, key),
        Err(e) => failure(e),
    }
}

/// `key in handle` (0), own-property test (1) or `delete handle[key]` (2) with the key a string or
/// Symbol handle. Returns 0 or 1.
#[no_mangle]
pub extern "C" fn thaw_tjs_property_predicate_key_handle_result(
    handle: u64,
    key_handle: u64,
    operation: u8,
) -> HandleResult {
    handle_result((|| {
        let key = handle_value(key_handle)?;
        on_handle(handle, |p, _, target| {
            let it = &mut p.interp;
            let key = it.key_of(&key);
            let answer = predicate(it, &target, &key, operation).map_err(|c| classify(it, c))?;
            Ok(i64::from(answer))
        })
    })())
}

/// `handle[key] = value` (the value a graph wire), keeping a setter's thrown value as a handle.
/// Returns 1 when written.
#[no_mangle]
pub extern "C" fn thaw_tjs_set_property_graph_with_exception_result(
    handle: u64,
    key_json: *const c_char,
    graph_json: *const c_char,
) -> ThawCallWithExceptionResult {
    match json_key(key_json) {
        Ok(key) => write_with_exception(handle, Value::Str(key.into()), graph_json, false),
        Err(e) => failure(e),
    }
}

/// As [`thaw_tjs_set_property_graph_with_exception_result`] through a receiver: an own writable data
/// property of the receiver is written, no setter runs, and a refused write is value 0.
#[no_mangle]
pub extern "C" fn thaw_tjs_set_receiver_data_graph_with_exception_result(
    handle: u64,
    key_json: *const c_char,
    graph_json: *const c_char,
) -> ThawCallWithExceptionResult {
    match json_key(key_json) {
        Ok(key) => write_with_exception(handle, Value::Str(key.into()), graph_json, true),
        Err(e) => failure(e),
    }
}

/// As [`thaw_tjs_set_property_graph_with_exception_result`], the key a string or Symbol handle;
/// `receiver_data` != 0 selects the receiver form.
#[no_mangle]
pub extern "C" fn thaw_tjs_set_property_key_handle_graph_with_exception_result(
    handle: u64,
    key_handle: u64,
    graph_json: *const c_char,
    receiver_data: u8,
) -> ThawCallWithExceptionResult {
    match handle_value(key_handle) {
        Ok(key) => write_with_exception(handle, key, graph_json, receiver_data != 0),
        Err(e) => failure(e),
    }
}

/// `Object.defineProperty`-style data property with the key a string or Symbol handle and the value a
/// graph wire. `attributes`: 1 writable, 2 enumerable, 4 configurable. Returns 1.
#[no_mangle]
pub extern "C" fn thaw_tjs_define_data_property_key_handle_graph_result(
    handle: u64,
    key_handle: u64,
    graph_json: *const c_char,
    attributes: u32,
) -> HandleResult {
    handle_result((|| {
        let key = handle_value(key_handle)?;
        let (_, value) = decode_wire(&cstr(graph_json).ok_or("invalid graph")?)?;
        on_handle(handle, |p, _, target| {
            let it = &mut p.interp;
            let key = it.key_of(&key);
            define_slot(
                it,
                &target,
                &key,
                Slot::Data(value),
                attributes & 2 != 0,
                attributes & 1 != 0,
            )
            .map_err(|c| classify(it, c))?;
            Ok(1)
        })
    })())
}

/// A native function that calls the napi callback `id` through the callback bridge, with `this` and
/// the arguments as its graph wire.
fn callback_function(it: &Interp, owner: usize, id: u64) -> Value {
    it.native(move |it, this, args| invoke_callback(it, owner, id, this, args))
}

fn invoke_callback(
    it: &mut Interp,
    owner: usize,
    id: u64,
    this: &Value,
    args: &[Value],
) -> R<Value> {
    let Some(bridge) = CALLBACK_BRIDGE.with(Cell::get) else {
        return it.throw_error("TypeError", "Native addon bridge is unavailable");
    };
    let mut items = vec![this.clone()];
    items.extend(args.iter().cloned());
    let packet = it.array(items);
    let wire = match encode_napi_graph(it, owner, &packet, 0, true, None) {
        Ok(wire) => wire,
        Err(e) => {
            let text = e.to_string();
            return it.throw_error("TypeError", text.trim_start_matches("TypeError: "));
        }
    };
    let (Ok(target), Ok(graph)) = (CString::new(id.to_string()), CString::new(wire)) else {
        return it.throw_error("TypeError", "Invalid callback graph");
    };
    let operation = c"call_captured_graph";
    let empty = c"";
    // SAFETY: the bridge reads the four C strings during the call only, and they outlive it.
    let answer = unsafe {
        bridge(
            operation.as_ptr(),
            target.as_ptr(),
            empty.as_ptr(),
            graph.as_ptr(),
        )
    };
    if answer.is_null() {
        return it.throw_error("TypeError", "Invalid native callback result");
    }
    // SAFETY: the bridge answers a NUL-terminated string that stays valid until the next call.
    let text = unsafe { CStr::from_ptr(answer) }
        .to_string_lossy()
        .into_owned();
    let Ok(Native::Object(fields)) = Native::from_json(&text) else {
        return it.throw_error("TypeError", "Invalid native callback result");
    };
    let field = |name: &str| fields.iter().find(|(k, _)| k == name).map(|(_, v)| v);
    if let Some(Native::Str(message)) = field("__thaw_error__") {
        return it.throw_error("TypeError", message);
    }
    match (field("kind"), field("value")) {
        (Some(Native::Str(kind)), Some(Native::Str(wire))) if kind == "graph" => {
            match decode_wire(wire) {
                Ok((_, value)) => Ok(value),
                Err(e) => it.throw_error("TypeError", &e),
            }
        }
        _ => it.throw_error("TypeError", "Invalid native callback result"),
    }
}

/// `Object.defineProperty(handle, key, ...)` for a native callback: `getter_id` / `setter_id` make an
/// accessor, `method_id` a data property holding the function. The `*_root` arguments are the napi
/// side's retained references to those functions; thaw-js has no GC, so they are never released.
#[no_mangle]
pub extern "C" fn thaw_tjs_define_callback_property_key_handle_result(
    handle: u64,
    key_handle: u64,
    getter_id: u64,
    setter_id: u64,
    method_id: u64,
    attributes: u32,
    _getter_root: u64,
    _setter_root: u64,
    _method_root: u64,
) -> HandleResult {
    handle_result((|| {
        let key = handle_value(key_handle)?;
        on_handle(handle, |p, owner, target| {
            let it = &mut p.interp;
            let key = it.key_of(&key);
            let enumerable = attributes & 2 != 0;
            let result = if method_id != 0 {
                let method = callback_function(it, owner, method_id);
                define_slot(
                    it,
                    &target,
                    &key,
                    Slot::Data(method),
                    enumerable,
                    attributes & 1 != 0,
                )
            } else {
                let get = (getter_id != 0).then(|| callback_function(it, owner, getter_id));
                let set = (setter_id != 0).then(|| callback_function(it, owner, setter_id));
                define_slot(
                    it,
                    &target,
                    &key,
                    Slot::Accessor { get, set },
                    enumerable,
                    false,
                )
            };
            result.map_err(|c| classify(it, c))?;
            Ok(1)
        })
    })())
}

/// Define `key` on `target` with `slot`, as `Object.defineProperty` does for the descriptor these
/// arguments describe. A frozen object refuses any change; a frozen or non-extensible one refuses a
/// new property. An accessor merges with an existing accessor's other half.
pub(crate) fn define_slot(
    it: &mut Interp,
    target: &Value,
    key: &str,
    slot: Slot,
    enumerable: bool,
    writable: bool,
) -> R<()> {
    let Value::Object(o) = target else {
        return it.throw_error("TypeError", "Object.defineProperty called on non-object");
    };
    if matches!(&*o.kind.borrow(), Kind::Proxy(_)) {
        let Some((inner, handler, trap)) = it.proxy_trap(o, "defineProperty")? else {
            return Ok(());
        };
        if trap.is_callable() {
            let desc = it.plain();
            match &slot {
                Slot::Data(v) => {
                    crate::put(&desc, "value", v.clone());
                    crate::put(&desc, "writable", Value::Bool(writable));
                }
                Slot::Accessor { get, set } => {
                    crate::put(&desc, "get", get.clone().unwrap_or(Value::Undefined));
                    crate::put(&desc, "set", set.clone().unwrap_or(Value::Undefined));
                }
            }
            crate::put(&desc, "enumerable", Value::Bool(enumerable));
            let k = it.key_value(key);
            it.call(&trap, &handler, &[inner, k, Value::Object(desc)])?;
            return Ok(());
        }
        return define_slot(it, &inner, key, slot, enumerable, writable);
    }
    let frozen = o.frozen.get();
    // array elements and length live in the element storage, written through `set`
    let element = match &*o.kind.borrow() {
        Kind::Array(items) => {
            let index = array_index(key);
            if index.is_some() || key == "length" {
                let exists = index.is_some_and(|i| i < items.len()) || key == "length";
                Some(exists)
            } else {
                None
            }
        }
        _ => None,
    };
    if let Some(exists) = element {
        let Slot::Data(v) = slot else {
            return it.throw_error("TypeError", "Accessor on an array index is not supported");
        };
        if frozen {
            return it.throw_error("TypeError", &format!("Cannot redefine property: {key}"));
        }
        if !exists && o.ext.get() != 0 {
            return it.throw_error(
                "TypeError",
                &format!("Cannot define property {key}, object is not extensible"),
            );
        }
        it.set(target, key, v)?;
        return Ok(());
    }
    let existing = o.position(key);
    if frozen && existing.is_some() {
        return it.throw_error("TypeError", &format!("Cannot redefine property: {key}"));
    }
    if existing.is_none() && (frozen || o.ext.get() != 0) {
        return it.throw_error(
            "TypeError",
            &format!("Cannot define property {key}, object is not extensible"),
        );
    }
    let mut props = o.props.borrow_mut();
    match existing {
        Some(i) => {
            let old = props[i].slot.clone();
            let merged = match (slot, old) {
                (
                    Slot::Accessor { get, set },
                    Slot::Accessor {
                        get: old_get,
                        set: old_set,
                    },
                ) => Slot::Accessor {
                    get: get.or(old_get),
                    set: set.or(old_set),
                },
                (slot, _) => slot,
            };
            props[i].slot = merged;
            props[i].enumerable = enumerable;
            props[i].writable = writable;
        }
        None => push_prop(
            o,
            &mut props,
            Prop {
                key: key.into(),
                slot,
                enumerable,
                writable,
            },
        ),
    }
    Ok(())
}

/// The own keys of the value behind `handle` as a graph wire of the array of key strings (numbers
/// for canonical integer keys when `key_conversion` is 0; Symbol keys stay Symbol handles).
/// `key_mode` 1 stops at the object itself, 0 walks the prototype chain. `key_filter`: 8 skips string
/// keys, 16 skips Symbol keys, and the bits 1, 2, 4 are the attributes each key must have.
#[no_mangle]
pub extern "C" fn thaw_tjs_host_property_names_result(
    handle: u64,
    key_mode: i32,
    key_filter: u32,
    key_conversion: i32,
) -> ThawResult {
    graph_result(on_handle(handle, |p, owner, target| {
        let it = &mut p.interp;
        let Value::Object(first) = &target else {
            return Err(HostError::Thrown(
                "TypeError: Reflect.ownKeys called on non-object".into(),
            ));
        };
        let mut out: Vec<Value> = Vec::new();
        let mut seen: HashSet<Rc<str>> = HashSet::new();
        let mut current = Some(first.clone());
        while let Some(node) = current {
            for (key, attributes) in
                own_keys_with_attributes(it, &node).map_err(|c| classify(it, c))?
            {
                let symbol = key.starts_with("@@sym:");
                if (symbol && key_filter & 16 != 0) || (!symbol && key_filter & 8 != 0) {
                    continue;
                }
                if !seen.insert(key.clone()) {
                    continue;
                }
                let required = key_filter & 7;
                if attributes & required != required {
                    continue;
                }
                let item = match (symbol, key_conversion, array_index(&key)) {
                    (false, 0, Some(n)) => Value::Number(n as f64),
                    _ => it.key_value(&key),
                };
                out.push(item);
            }
            if key_mode == 1 {
                break;
            }
            current = match prototype_of(it, &node).map_err(|c| classify(it, c))? {
                Value::Object(parent) => Some(parent),
                _ => None,
            };
        }
        let array = it.array(out);
        let mut wire = WireOut {
            owner,
            plain: false,
            live_functions: false,
            copy_root: true,
            receiver: None,
            nodes: Vec::new(),
            seen: Vec::new(),
            leases: Vec::new(),
        };
        let root = wire.token(it, &array)?;
        Ok(wire.finish(root))
    }))
}

/// The UTF-16 units of `text` as a JSON array of numbers.
fn units_json(text: &str) -> String {
    let units: Vec<String> = builtins::str_units(text)
        .iter()
        .map(u16::to_string)
        .collect();
    format!("[{}]", units.join(","))
}

/// ToString of the value behind `handle` with its effects (`toString` / `valueOf`), as UTF-16 units
/// JSON. `array_element` != 0 maps an undefined or null element to `[]`. A Symbol is a TypeError.
#[no_mangle]
pub extern "C" fn thaw_tjs_abstract_to_string_units_result(
    handle: u64,
    array_element: u8,
) -> ThawStringCoercionResult {
    let outcome = exact(handle, |it, _, v| {
        if array_element != 0 && matches!(v, Value::Undefined | Value::Null | Value::Hole) {
            return Ok("[]".to_string());
        }
        if is_symbol(&v) {
            return it.throw_error("TypeError", "Cannot convert a Symbol value to a string");
        }
        let primitive = it.primitive_of(&v, Hint::String)?;
        if is_symbol(&primitive) {
            return it.throw_error("TypeError", "Cannot convert a Symbol value to a string");
        }
        let text = it.to_str(&primitive);
        Ok(units_json(&text))
    });
    let none = ThawStringCoercionResult {
        units: std::ptr::null_mut(),
        exception_handle: 0,
        exception_object_like: 0,
        error: std::ptr::null_mut(),
    };
    match outcome {
        Ok(Exact::Done(units)) => ThawStringCoercionResult {
            units: thaw_arena::owned_string(units),
            ..none
        },
        Ok(Exact::Thrown(handle, like)) => ThawStringCoercionResult {
            exception_handle: handle,
            exception_object_like: u8::from(like),
            ..none
        },
        Err(e) => ThawStringCoercionResult {
            error: thaw_arena::owned_string(e),
            ..none
        },
    }
}
