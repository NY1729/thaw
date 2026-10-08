//! `JSON.parse`/`JSON.stringify`/`.field`/`[i]`/`Number`/`String`/`Boolean`
//! on the `HirType::Json` dynamic value (see `thaw_hir::HirExpr::JsonGet`
//! et al. and `thaw-llvm::hir_codegen`).
//!
//! A `Json` value at the LLVM level is an opaque pointer to a
//! `Box<Value>` -- this file's own shared representation (see `Value`'s
//! doc comment), not `serde_json::Value`. Values that cross the dynamic
//! runtime bridge are explicitly destroyed once their ownership ends;
//! longer-lived user values remain live for the process lifetime like
//! other Thaw heap values.
//!
//! The compiled user-facing calls check runtime status after operations
//! that can fail, then raise through the existing HIR exception channel.
//! Internal marshaling callers retain the pointer/scalar FFI ABI.
//!
//! A genuinely *missing* key/index (`thaw_json_get`/`thaw_json_index`)
//! degrades to a distinct `$__thaw_napi_undefined$`-tagged sentinel
//! object (`napi_undefined_value`/`is_napi_undefined`), not plain JSON
//! `null` -- keeping "the key is absent" and "the key is present with
//! an explicit `null`" distinguishable the way real JS's `undefined`
//! and `null` are, through `typeof`/truthiness/`String()`/`===`/`==`.
//! `Number()`/`thaw_json_as_number` maps that sentinel to `NaN`, matching
//! real `Number(undefined)`, so a missing key is neither `== 0` nor usable
//! in arithmetic (real JS behavior).
//!
//! User-facing `JSON.stringify(...)` *does* omit/null a nested sentinel
//! value the way real `JSON.stringify` treats a real `undefined`
//! (`ordered_json_omitting_undefined`/`filtered_json_omitting_undefined`,
//! reached via `thaw_json_stringify_public` and its `_number_space`/
//! `_string_space`/`_keys*` siblings) -- but the *plain* `thaw_json_
//! stringify` (no `_public` suffix) deliberately stays unaware: it's also
//! load-bearing for unrelated internal argument/result marshaling that
//! needs the sentinel's exact shape preserved verbatim, so giving it the
//! same omitting behavior would silently corrupt that mechanism instead
//! (see `ordered_json`'s own doc comment for the full story, including
//! the earlier attempt and regression that led to this split). The
//! *top-level* sentinel case (`JSON.stringify(x)` where `x` itself, not
//! a nested field/element, is `undefined`) is handled too, in the three
//! public entry points themselves rather than the nested-value helpers
//! above -- see `top_level_undefined_string`'s own doc comment for why
//! that's a string approximation rather than a real `undefined` return.

use std::cell::{Cell, RefCell, UnsafeCell};
use std::collections::{HashMap, HashSet};
use std::ffi::CString;
use std::os::raw::c_char;
use std::rc::Rc;
use thaw_arena::NativeStr as CStr;

/// A live-shared native array -- `Rc` gives every read of the same
/// logical array (via `Value::clone`) a handle to the *same* underlying
/// `Vec`, so a mutation through one alias (`thaw_json_index_set`, or a
/// `Vec::push` from the Map/Set helpers below) is observable through
/// every other alias, matching real JS's own array/object reference
/// semantics. `UnsafeCell` (not `RefCell`) because `thaw_json_index_get_
/// mut`'s own contract -- a raw pointer to a slot *inside* the
/// container, valid only for the caller's own short, single-expression
/// use (see that function's doc comment) -- needs a plain `*mut` it can
/// return across the FFI boundary; a `RefCell` guard's lifetime can't
/// cross that boundary at all.
type SharedArray = Rc<SharedValue<Vec<Value>>>;
/// See `SharedArray`'s own doc comment -- identical reasoning, for
/// objects. `IndexMap` preserves insertion order for the consumers of
/// `ordered_object_fields`; WTF-8 keys retain UTF-16 property identity.
type SharedObject = Rc<SharedValue<indexmap::IndexMap<Vec<u8>, Value>>>;

/// A native lease on one QuickJS value. The bridge installs the release
/// operation before decoding a callback graph; cloning a Json value shares
/// this lease, so only its final owner releases the handle. The handle ID is
/// canonical in the QuickJS registry (Object.is) and is stable for aliases.
struct HostLease {
    handle: u64,
    release: extern "C" fn(u64) -> u8,
}

// A decoded N-API instance graph node owns one positive native reference.
// The object Rc shares it across native Json aliases; the last owner releases
// only after graph edges have been detached (see release_isolated_json_cycles).
struct NapiLease {
    handle: u64,
    reference: u64,
    release: extern "C" fn(u64) -> u8,
}

impl Drop for NapiLease {
    fn drop(&mut self) {
        without_typed_decode_scope(|| (self.release)(self.reference));
    }
}

#[derive(Clone, Copy)]
struct NapiHandleOperations {
    retain: extern "C" fn(u64) -> u64,
    release: extern "C" fn(u64) -> u8,
}

thread_local! {
    static NAPI_HANDLE_OPERATIONS: Cell<Option<NapiHandleOperations>> = const { Cell::new(None) };
}

#[no_mangle]
pub extern "C" fn thaw_json_register_napi_handle_operations(
    retain: extern "C" fn(u64) -> u64,
    release: extern "C" fn(u64) -> u8,
) {
    NAPI_HANDLE_OPERATIONS.with(|slot| slot.set(Some(NapiHandleOperations { retain, release })));
}

#[repr(C)]
struct HostHandleResult {
    value: u64,
    error: *const c_char,
}

#[repr(C)]
struct HostTextResult {
    value: *const c_char,
    error: *const c_char,
}

#[derive(Clone, Copy)]
struct HostOperations {
    retain: extern "C" fn(u64) -> u8,
    release: extern "C" fn(u64) -> u8,
    get: extern "C" fn(u64, *const c_char) -> HostHandleResult,
    query: extern "C" fn(u64, u8) -> HostTextResult,
    date_set: extern "C" fn(u64, f64) -> HostTextResult,
    set: extern "C" fn(u64, *const c_char, *const c_char) -> HostHandleResult,
    predicate: extern "C" fn(u64, *const c_char, u8) -> HostHandleResult,
    enumerate: extern "C" fn(u64, u8) -> HostTextResult,
}

impl Drop for HostLease {
    fn drop(&mut self) {
        without_typed_decode_scope(|| (self.release)(self.handle));
    }
}

thread_local! {
    static HOST_OPERATIONS: Cell<Option<HostOperations>> = const { Cell::new(None) };
    static HOST_ERROR: RefCell<Option<String>> = const { RefCell::new(None) };
    // `None` is a boundary while a trusted Host/NAPI callback executes.
    // User reentry without its own decode scope must not join the caller's ledger.
    static TYPED_DECODE_SCOPES: RefCell<Vec<Option<TypedDecodeScope>>> = const { RefCell::new(Vec::new()) };
}

#[derive(Default)]
struct TypedDecodeScope {
    children: Vec<usize>,
    handles: Vec<u64>,
    origins: Vec<(usize, u64, u64)>,
}

fn track_typed_decode_child(value: *mut Value) -> *mut Value {
    TYPED_DECODE_SCOPES.with(|scopes| {
        let mut scopes = scopes.borrow_mut();
        if scopes.iter().any(|scope| scope.as_ref().is_some_and(|scope|
            scope.children.contains(&(value as usize)))) {
            return;
        }
        if let Some(scope) = scopes.last_mut().and_then(Option::as_mut) {
            scope.children.push(value as usize);
        }
    });
    value
}

#[no_mangle]
pub extern "C" fn thaw_json_typed_decode_scope_own(value: *mut Value) {
    if !value.is_null() { track_typed_decode_child(value); }
}

fn untrack_typed_decode_child(value: *mut Value) {
    TYPED_DECODE_SCOPES.with(|scopes| {
        for scope in scopes.borrow_mut().iter_mut().rev().filter_map(Option::as_mut) {
            if let Some(index) = scope.children.iter().position(|child| *child == value as usize) {
                scope.children.swap_remove(index);
                break;
            }
        }
    });
}

#[no_mangle]
pub extern "C" fn thaw_json_typed_decode_scope_begin() {
    TYPED_DECODE_SCOPES.with(|scopes| scopes.borrow_mut().push(Some(TypedDecodeScope::default())));
}

struct SuspendedTypedDecodeScope;

impl Drop for SuspendedTypedDecodeScope {
    fn drop(&mut self) {
        TYPED_DECODE_SCOPES.with(|scopes| {
            let boundary = scopes.borrow_mut().pop();
            debug_assert!(matches!(boundary, Some(None)));
        });
    }
}

fn without_typed_decode_scope<T>(callback: impl FnOnce() -> T) -> T {
    TYPED_DECODE_SCOPES.with(|scopes| scopes.borrow_mut().push(None));
    let _boundary = SuspendedTypedDecodeScope;
    callback()
}

/// Mode 0 disarms a completed top-level conversion, 1 rolls it back, and 2
/// merges a completed nested conversion into its parent's failure ownership.
#[no_mangle]
pub extern "C" fn thaw_json_typed_decode_scope_end(mode: u8) {
    let Some(scope) = TYPED_DECODE_SCOPES.with(|scopes| scopes.borrow_mut().pop()).flatten() else { return };
    if mode == 2 {
        TYPED_DECODE_SCOPES.with(|scopes| {
            if let Some(parent) = scopes.borrow_mut().last_mut().and_then(Option::as_mut) {
                parent.children.extend(scope.children);
                parent.handles.extend(scope.handles);
                parent.origins.extend(scope.origins);
            }
        });
        return;
    }
    if mode == 0 { return; }
    // Cleanup callbacks may report their own Host error. Keep the conversion's
    // original state and do not let a cleanup-only error poison the next call.
    let original_host_error = HOST_ERROR.with(|slot| slot.borrow_mut().take());
    // Remove sidecar entries before any release callback can re-enter and
    // observe or reuse the arena closure address. A successful conversion
    // disarms this list; a later sibling failure rolls it back.
    for (closure, handle, serial) in scope.origins.into_iter().rev() {
        let removed = CALLBACK_ORIGINS.with(|origins| {
            let mut origins = origins.borrow_mut();
            if origins.get(&closure) == Some(&(handle, serial)) {
                origins.remove(&closure)
            } else { None }
        });
        if removed.is_some() {
            if let Some(ops) = HOST_OPERATIONS.with(|slot| slot.get()) {
                without_typed_decode_scope(|| (ops.release)(handle));
            }
        }
    }
    // Remove the scope before dropping any child or handle: release callbacks
    // can re-enter a separate typed decoder on this thread.
    for child in scope.children.into_iter().rev() {
        unsafe { thaw_json_destroy(child as *mut Value) };
    }
    for handle in scope.handles.into_iter().rev() {
        if let Some(ops) = HOST_OPERATIONS.with(|slot| slot.get()) {
            without_typed_decode_scope(|| (ops.release)(handle));
        }
    }
    HOST_ERROR.with(|slot| *slot.borrow_mut() = original_host_error);
}

// A decoded JS Function closure owns exactly one independently retained
// original handle. Its arena reachability controls that handle's lifetime.
thread_local! {
    static CALLBACK_ORIGINS: RefCell<HashMap<usize, (u64, u64)>> = RefCell::new(HashMap::new());
    static NEXT_CALLBACK_ORIGIN: Cell<u64> = const { Cell::new(0) };
}

fn reset_callback_origins(tracing: bool) {
    let dead = CALLBACK_ORIGINS.with(|origins| origins.borrow().iter()
        .filter_map(|(&closure, &(handle, serial))|
            (!tracing || thaw_arena::was_reclaimed(closure))
                .then_some((closure, handle, serial)))
        .collect::<Vec<_>>());
    for (closure, handle, serial) in dead {
        let removed = CALLBACK_ORIGINS.with(|origins| {
            let mut origins = origins.borrow_mut();
            if origins.get(&closure) == Some(&(handle, serial)) {
                origins.remove(&closure)
            } else { None }
        });
        if removed.is_some() {
            // Release callbacks may report a cleanup-only Host error; keep
            // the error that preceded arena reset authoritative.
            let prior = HOST_ERROR.with(|slot| slot.borrow_mut().take());
            if let Some(ops) = HOST_OPERATIONS.with(|slot| slot.get()) {
                without_typed_decode_scope(|| (ops.release)(handle));
            }
            HOST_ERROR.with(|slot| *slot.borrow_mut() = prior);
        }
    }
}

/// Acquires one callback-origin reference for either a live Host value or a
/// branded handle placeholder. The active typed scope keeps failure ownership
/// through registration and transfers lifetime to the sidecar only on success.
#[no_mangle]
pub unsafe extern "C" fn thaw_json_callback_origin_acquire(value: *const Value) -> u64 {
    let Some(value) = (unsafe { value.as_ref() }) else { return 0; };
    let handle = unsafe { thaw_json_borrowed_handle_id(value as *const Value) };
    if handle == 0 { return 0; }
    // A branded marker is only a handle carrier. The typed Function lane
    // must verify the live value, including for an already-decoded Host.
    if host_query_handle(handle, 14).as_deref() != Some("1") {
        set_host_error("Callback origin is not a function".into());
        return 0;
    }
    let retained = HOST_OPERATIONS.with(|slot| slot.get())
        .is_some_and(|ops| without_typed_decode_scope(|| (ops.retain)(handle)) != 0);
    if !retained { return 0; }
    TYPED_DECODE_SCOPES.with(|scopes| {
        if let Some(scope) = scopes.borrow_mut().last_mut().and_then(Option::as_mut) {
            scope.handles.push(handle);
        }
    });
    handle
}

/// Associates the acquired reference with its initialized arena closure.
/// The active conversion scope still owns rollback until all sibling fields
/// complete; on success the sidecar owns the reference until arena reset.
#[no_mangle]
pub extern "C" fn thaw_json_register_callback_origin(closure: *const u8, handle: u64) -> u8 {
    if closure.is_null() || handle == 0 {
        set_host_error("Invalid callback origin".into());
        return 0;
    }
    let Some(serial) = NEXT_CALLBACK_ORIGIN.with(|next| next.get().checked_add(1)
        .inspect(|serial| next.set(*serial))) else {
        set_host_error("Callback origin serial exhausted".into());
        return 0;
    };
    let inserted = CALLBACK_ORIGINS.with(|origins| {
        let mut origins = origins.borrow_mut();
        if origins.contains_key(&(closure as usize)) { false }
        else { origins.insert(closure as usize, (handle, serial)); true }
    });
    if !inserted {
        set_host_error("Callback origin already registered".into());
        return 0;
    }
    thaw_arena::register_reset_hook(reset_callback_origins);
    TYPED_DECODE_SCOPES.with(|scopes| {
        // A reentrant callback may sit behind a None boundary; never steal
        // an identically numbered retain from its suspended parent scope.
        if let Some(scope) = scopes.borrow_mut().last_mut().and_then(Option::as_mut) {
            if let Some(index) = scope.handles.iter().rposition(|id| *id == handle) {
                scope.handles.swap_remove(index);
                scope.origins.push((closure as usize, handle, serial));
            }
        }
    });
    1
}

#[no_mangle]
pub extern "C" fn thaw_json_callback_origin_handle(closure: *const u8) -> u64 {
    CALLBACK_ORIGINS.with(|origins| origins.borrow()
        .get(&(closure as usize)).map_or(0, |(handle, _)| *handle))
}

fn set_host_error(message: String) {
    HOST_ERROR.with(|slot| {
        let mut pending = slot.borrow_mut();
        if pending.is_none() { *pending = Some(message); }
    });
}

/// Registration is scoped to the thread running the reentrant QuickJS
/// callback. `thaw-std` has no link dependency on the optional JS runtime.
#[no_mangle]
pub extern "C" fn thaw_json_register_host_operations(
    retain: extern "C" fn(u64) -> u8,
    release: extern "C" fn(u64) -> u8,
    get: extern "C" fn(u64, *const c_char) -> HostHandleResult,
    query: extern "C" fn(u64, u8) -> HostTextResult,
    date_set: extern "C" fn(u64, f64) -> HostTextResult,
    set: extern "C" fn(u64, *const c_char, *const c_char) -> HostHandleResult,
    predicate: extern "C" fn(u64, *const c_char, u8) -> HostHandleResult,
    enumerate: extern "C" fn(u64, u8) -> HostTextResult,
) {
    HOST_OPERATIONS.with(|slot| slot.set(Some(HostOperations {
        retain, release, get, query, date_set, set, predicate, enumerate,
    })));
}

#[no_mangle]
pub extern "C" fn thaw_json_take_host_error() -> *const c_char {
    HOST_ERROR.with(|error| error.borrow_mut().take())
        .map(|message| thaw_arena::owned_string(message).cast_const())
        .unwrap_or(std::ptr::null())
}

fn canonical_bigint_decimal(value: &str) -> bool {
    let digits = value.strip_prefix('-').unwrap_or(value);
    !digits.is_empty() && (digits == "0" || !digits.starts_with('0'))
        && !(value.starts_with('-') && digits == "0")
        && digits.bytes().all(|byte| byte.is_ascii_digit())
}

fn host_value_from_owned_handle(handle: u64) -> Option<Value> {
    if handle == 0 { return None; }
    let ops = HOST_OPERATIONS.with(|slot| slot.get())?;
    let lease = HostLease { handle, release: ops.release };
    // A property read may return a primitive. Materialize those as native
    // scalars so strict equality, Object.is and SameValueZero retain their
    // established number/string/undefined semantics (including -0/NaN).
    // Objects, functions and Symbols retain live JS identity; BigInt is a value.
    let kind = host_query(&lease, 0)?;
    let value = match kind.as_str() {
        "undefined" => napi_undefined_value(),
        "boolean" => Value::Bool(host_query(&lease, 1)?.as_str() == "1"),
        "number" => {
            let text = host_query(&lease, 10)?;
            let number = match text.as_str() {
                "NaN" => f64::NAN,
                "Infinity" => f64::INFINITY,
                "-Infinity" => f64::NEG_INFINITY,
                _ => match text.parse::<f64>() {
                    Ok(number) => number,
                    Err(_) => { set_host_error("Invalid host number value".into()); return None; }
                },
            };
            number_value(number)
        }
        "bigint" => {
            let text = host_query(&lease, 10)?;
            if !canonical_bigint_decimal(&text) {
                set_host_error("Invalid host BigInt value".into());
                return None;
            }
            Value::BigInt(text)
        }
        "string" => {
            let text = host_query(&lease, 3)?;
            let Some(parsed) = JsonParser::new(text.as_bytes()).parse() else {
                set_host_error("Invalid host string value".into());
                return None;
            };
            match parsed {
                value @ (Value::String(_) | Value::Wtf8(_)) => value,
                _ => { set_host_error("Invalid host string value".into()); return None; }
            }
        }
        "object" if host_query(&lease, 7)?.as_str() == "1" => Value::Null,
        _ => return Some(Value::Host(Rc::new(lease))),
    };
    Some(value)
}

fn host_value_from_borrowed_handle(handle: u64) -> Option<Value> {
    let ops = HOST_OPERATIONS.with(|slot| slot.get())?;
    if handle == 0 || without_typed_decode_scope(|| (ops.retain)(handle)) == 0 { return None; }
    let lease = HostLease { handle, release: ops.release };
    if host_query(&lease, 0)?.as_str() == "bigint" {
        let decimal = host_query(&lease, 10)?;
        if !canonical_bigint_decimal(&decimal) {
            set_host_error("Invalid host BigInt value".into());
            return None;
        }
        return Some(Value::BigInt(decimal));
    }
    Some(Value::Host(Rc::new(lease)))
}

/// Convert a retained JS object into the existing live Json Host lease.
/// The caller keeps ownership of `handle` and releases it after this call;
/// `host_value_from_borrowed_handle` retains an independent reference.
#[no_mangle]
pub extern "C" fn thaw_json_host_from_borrowed_handle(handle: u64) -> *mut Value {
    match host_value_from_borrowed_handle(handle) {
        Some(Value::Host(lease)) => {
            let kind = host_query(&lease, 0);
            let is_live_object = match kind.as_deref() {
                Some("function") => true,
                Some("object") => host_query(&lease, 7).as_deref() == Some("0"),
                _ => false,
            };
            if is_live_object {
                thaw_json_track_arena_owned_root(leak(Value::Host(lease)))
            } else {
                set_host_error("Live JSON object builder returned a nonobject".into());
                std::ptr::null_mut()
            }
        }
        Some(_) => {
            set_host_error("Live JSON object builder returned a nonobject".into());
            std::ptr::null_mut()
        }
        None => {
            set_host_error("Unable to retain live JSON object".into());
            std::ptr::null_mut()
        }
    }
}

/// Convert a borrowed retained JS value of any kind into a Json value: primitives
/// materialize as native scalars, objects/functions as a live Host lease. The
/// lease retains its own reference; the caller keeps its handle.
#[no_mangle]
pub extern "C" fn thaw_json_from_borrowed_handle(handle: u64) -> *mut Value {
    let Some(ops) = HOST_OPERATIONS.with(|slot| slot.get()) else {
        set_host_error("Host operations unavailable".into());
        return std::ptr::null_mut();
    };
    if handle == 0 || without_typed_decode_scope(|| (ops.retain)(handle)) == 0 {
        set_host_error("Unable to retain host JSON value".into());
        return std::ptr::null_mut();
    }
    match host_value_from_owned_handle(handle) {
        Some(value) => thaw_json_track_arena_owned_root(leak(materialize_native_wrapper(value))),
        None => std::ptr::null_mut(),
    }
}

fn host_key_json(key: &[u8]) -> CString {
    let mut encoded = Vec::new();
    write_json_string(key, &mut encoded);
    CString::new(encoded).expect("escaped property key contains no NUL")
}

fn host_get_property(lease: &HostLease, key: &[u8]) -> Value {
    let Some(ops) = HOST_OPERATIONS.with(|slot| slot.get()) else {
        set_host_error("Host operations unavailable".into());
        return Value::Null;
    };
    // JSON-string escaping preserves NUL and lone UTF-16 surrogates across
    // the C boundary; the host parses this as one exact property key.
    let key_json = host_key_json(key);
    let result = without_typed_decode_scope(|| (ops.get)(lease.handle, key_json.as_ptr()));
    if !result.error.is_null() {
        let error = to_str(result.error);
        unsafe { thaw_arena::destroy_string(result.error.cast_mut()) };
        set_host_error(error);
        return Value::Null;
    }
    host_value_from_owned_handle(result.value).unwrap_or_else(|| {
        set_host_error("Invalid host value handle".into());
        Value::Null
    })
}

fn host_set_property(lease: &HostLease, key: &[u8], value: &Value) {
    let Some(ops) = HOST_OPERATIONS.with(|slot| slot.get()) else {
        set_host_error("Host operations unavailable".into());
        return;
    };
    let key_json = host_key_json(key);
    let graph = thaw_json_graph_encode(value as *const Value);
    if graph.is_null() {
        set_host_error("Unable to encode host assignment".into());
        return;
    }
    if unsafe { *graph.cast::<u8>() } == 2 {
        set_host_error(to_str(graph));
        unsafe { thaw_arena::destroy_string(graph.cast_mut()) };
        return;
    }
    let result = without_typed_decode_scope(|| (ops.set)(lease.handle, key_json.as_ptr(), graph));
    unsafe { thaw_arena::destroy_string(graph.cast_mut()) };
    if !result.error.is_null() {
        let error = to_str(result.error);
        unsafe { thaw_arena::destroy_string(result.error.cast_mut()) };
        set_host_error(error);
    }
}

fn host_property_predicate(lease: &HostLease, key: &[u8], operation: u8) -> bool {
    let Some(ops) = HOST_OPERATIONS.with(|slot| slot.get()) else {
        set_host_error("Host operations unavailable".into());
        return false;
    };
    let key_json = host_key_json(key);
    let result = without_typed_decode_scope(|| (ops.predicate)(lease.handle, key_json.as_ptr(), operation));
    if !result.error.is_null() {
        let error = to_str(result.error);
        unsafe { thaw_arena::destroy_string(result.error.cast_mut()) };
        set_host_error(error);
        return false;
    }
    result.value != 0
}

fn host_enumerate_values(lease: &HostLease, operation: u8) -> Vec<Value> {
    let Some(ops) = HOST_OPERATIONS.with(|slot| slot.get()) else {
        set_host_error("Host operations unavailable".into());
        return Vec::new();
    };
    let result = without_typed_decode_scope(|| (ops.enumerate)(lease.handle, operation));
    if !result.error.is_null() {
        let error = to_str(result.error);
        unsafe { thaw_arena::destroy_string(result.error.cast_mut()) };
        set_host_error(error);
        return Vec::new();
    }
    if result.value.is_null() {
        set_host_error("Host enumeration returned no value".into());
        return Vec::new();
    }
    let decoded = thaw_json_graph_decode(result.value);
    unsafe { thaw_arena::destroy_string(result.value.cast_mut()) };
    let invalid = thaw_json_take_graph_error() != 0;
    let decoded = unsafe { Box::from_raw(decoded) };
    if invalid {
        set_host_error("Invalid host enumeration graph".into());
        return Vec::new();
    }
    decoded.as_array().cloned().unwrap_or_else(|| {
        set_host_error("Invalid host enumeration result".into());
        Vec::new()
    })
}

fn host_enumerable_entries(lease: &HostLease) -> Vec<(Vec<u8>, Value)> {
    let mut entries = Vec::new();
    for pair in host_enumerate_values(lease, 0) {
        if HOST_ERROR.with(|error| error.borrow().is_some()) { break; }
        let Value::Host(pair) = pair else {
            set_host_error("Invalid host entry pair".into());
            break;
        };
        let key = host_get_property(&pair, b"0");
        if HOST_ERROR.with(|error| error.borrow().is_some()) { break; }
        let key_text = thaw_json_as_string(&key as *const Value as *mut Value);
        if HOST_ERROR.with(|error| error.borrow().is_some()) {
            unsafe { thaw_arena::destroy_string(key_text.cast_mut()) };
            break;
        }
        let key = canonical_key(unsafe { CStr::from_ptr(key_text) }.to_bytes());
        unsafe { thaw_arena::destroy_string(key_text.cast_mut()) };
        let value = host_get_property(&pair, b"1");
        if HOST_ERROR.with(|error| error.borrow().is_some()) { break; }
        entries.push((key, value));
    }
    if HOST_ERROR.with(|error| error.borrow().is_some()) {
        entries.clear();
    }
    entries
}

fn host_query(lease: &HostLease, operation: u8) -> Option<String> {
    host_query_handle(lease.handle, operation)
}

fn host_query_handle(handle: u64, operation: u8) -> Option<String> {
    let Some(ops) = HOST_OPERATIONS.with(|slot| slot.get()) else {
        set_host_error("Host operations unavailable".into());
        return None;
    };
    let result = without_typed_decode_scope(|| (ops.query)(handle, operation));
    if !result.error.is_null() {
        let error = to_str(result.error);
        unsafe { thaw_arena::destroy_string(result.error.cast_mut()) };
        set_host_error(error);
        return None;
    }
    if result.value.is_null() {
        set_host_error("Host query returned no value".into());
        return None;
    }
    let value = to_str(result.value);
    unsafe { thaw_arena::destroy_string(result.value.cast_mut()) };
    Some(value)
}

struct SharedValue<T> {
    data: UnsafeCell<T>,
    napi_lease: RefCell<Option<Rc<NapiLease>>>,
}

impl<T> SharedValue<T> {
    fn get(&self) -> *mut T {
        self.data.get()
    }
}

impl<T> Drop for SharedValue<T> {
    fn drop(&mut self) {
        let key = self as *const Self as usize;
        let _ = PROTOTYPES.try_with(|table| {
            let previous = { table.borrow_mut().remove(&key) };
            drop(previous);
        });
        let _ = ARRAY_HOLES.try_with(|holes| {
            holes.borrow_mut().remove(&key);
        });
        let _ = WRAPPER_BRANDS.try_with(|brands| {
            brands.borrow_mut().remove(&key);
        });
        let _ = GRAPH_PLAIN_OBJECTS.try_with(|objects| {
            objects.borrow_mut().remove(&key);
        });
        let _ = DETACHED_HANDLE_MARKERS.try_with(|markers| {
            markers.borrow_mut().remove(&key);
        });
        unsafe { thaw_object_clear_state(key as *const u8) };
    }
}

unsafe extern "C" {
    fn thaw_object_clear_state(object: *const u8);
    fn thaw_object_state(object: *const u8, query: u8) -> bool;
    fn thaw_object_set_state(object: *const u8, operation: u8) -> bool;
}

/// Thaw's own dynamic (`Json`/`any`-typed) value representation --
/// deliberately *not* `serde_json::Value` (an earlier version of this
/// file used that directly, see git history): a plain owned tree gives
/// every nested read a fresh deep copy (`Value::clone`), which silently
/// diverges from real JS reference semantics the moment a nested object/
/// array is read into a local and mutated (`const inner = obj.c; inner.d
/// = false;` never reached back into `obj.c.d`, a long-standing tracked
/// gap). Only `Array`/`Object` carry a `Rc<UnsafeCell<...>>` -- a
/// scalar (`Null`/`Bool`/`Number`/`BigInt`/`String`) stays a plain value (real JS
/// primitives are values, not references, so this isn't a divergence to
/// fix). `#[derive(Clone)]` on this enum is exactly the "shallow, shares
/// the same container" clone `Rc::clone` already gives `Array`/`Object`,
/// so every existing `.clone()` call site that reads a nested value
/// keeps compiling unchanged and now means "share", not "deep-copy" --
/// see `thaw_json_clone` for the one place that still needs a real,
/// independent recursive copy (`structuredClone`'s own contract).
#[derive(Clone)]
pub(crate) enum Value {
    Null,
    Bool(bool),
    Number(serde_json::Number),
    /// Canonical signed decimal integer; unlike a JSON number, never passes through f64.
    BigInt(String),
    String(String),
    /// A string containing at least one lone UTF-16 surrogate, held as
    /// WTF-8 bytes (a Rust `String` cannot represent it). Only ever
    /// produced from a native string crossing into JSON; serde_json is
    /// bypassed when serializing it (see `write_json_value`).
    Wtf8(Vec<u8>),
    Array(SharedArray),
    Object(SharedObject),
    Host(Rc<HostLease>),
}

impl Value {
    fn shared_array(items: Vec<Value>) -> Value {
        Value::Array(Rc::new(SharedValue {
            data: UnsafeCell::new(items), napi_lease: RefCell::new(None),
        }))
    }

    fn shared_object(fields: indexmap::IndexMap<Vec<u8>, Value>) -> Value {
        Self::shared_object_with_lease(fields, None)
    }

    fn shared_object_with_lease(
        fields: indexmap::IndexMap<Vec<u8>, Value>, lease: Option<Rc<NapiLease>>,
    ) -> Value {
        Value::Object(Rc::new(SharedValue {
            data: UnsafeCell::new(fields), napi_lease: RefCell::new(lease),
        }))
    }

    fn as_array(&self) -> Option<&Vec<Value>> {
        match self {
            Value::Array(items) => Some(unsafe { &*items.get() }),
            _ => None,
        }
    }

    #[allow(clippy::mut_from_ref)]
    fn as_array_mut(&self) -> Option<&mut Vec<Value>> {
        match self {
            Value::Array(items) => Some(unsafe { &mut *items.get() }),
            _ => None,
        }
    }

    fn as_object(&self) -> Option<&indexmap::IndexMap<Vec<u8>, Value>> {
        match self {
            Value::Object(fields) => Some(unsafe { &*fields.get() }),
            _ => None,
        }
    }

    #[allow(clippy::mut_from_ref)]
    fn as_object_mut(&self) -> Option<&mut indexmap::IndexMap<Vec<u8>, Value>> {
        match self {
            Value::Object(fields) => Some(unsafe { &mut *fields.get() }),
            _ => None,
        }
    }

    fn as_str(&self) -> Option<&str> {
        match self {
            Value::String(value) => Some(value.as_str()),
            _ => None,
        }
    }

    fn as_bool(&self) -> Option<bool> {
        match self {
            Value::Bool(value) => Some(*value),
            _ => None,
        }
    }

    fn as_f64(&self) -> Option<f64> {
        match self {
            Value::Number(value) => value.as_f64(),
            _ => None,
        }
    }

    fn is_null(&self) -> bool {
        matches!(self, Value::Null)
    }

    fn is_array(&self) -> bool {
        matches!(self, Value::Array(_))
    }

    /// A genuine, independent recursive deep copy -- unlike
    /// `#[derive(Clone)]` (which shares the same `Rc`-backed container),
    /// every `Object`/`Array` node gets its own fresh `Rc<UnsafeCell<...>>`,
    /// so mutating the copy never observably affects the original.
    /// `structuredClone`'s own contract (`thaw_json_clone`).
    fn deep_clone(&self) -> Value { self.deep_clone_mode(false) }

    /// `live`: materialize host objects into independent native copies (`structuredClone`).
    fn deep_clone_mode(&self, live: bool) -> Value {
        match self {
            Value::Null => Value::Null,
            Value::Bool(value) => Value::Bool(*value),
            Value::Number(value) => Value::Number(value.clone()),
            Value::BigInt(value) => Value::BigInt(value.clone()),
            Value::String(value) => Value::String(value.clone()),
            Value::Wtf8(bytes) => Value::Wtf8(bytes.clone()),
            Value::Array(items) => {
                let copy = Value::shared_array(
                    unsafe { &*items.get() }.iter().map(|item| item.deep_clone_mode(live)).collect(),
                );
                if let Value::Array(target) = &copy {
                    copy_array_holes(items, target, 0);
                }
                copy
            }
            Value::Object(fields) => {
                let copy = Value::shared_object_with_lease(
                    unsafe { &*fields.get() }
                        .iter()
                        .map(|(key, value)| (key.clone(), value.deep_clone_mode(live)))
                        .collect(),
                    fields.napi_lease.borrow().clone(),
                );
                if is_branded_wrapper(self) { brand_internal_value(copy) }
                else { if is_graph_plain_object(self) { mark_graph_plain_object(&copy); } copy }
            },
            // A host object cannot be cloned by walking native fields. Its
            // clone operation must be delegated to the registered host.
            Value::Host(lease) if live => host_deep_clone(lease),
            Value::Host(lease) => Value::Host(Rc::clone(lease)),
        }
    }
}

/// `structuredClone` of a live host object: plain objects and arrays are
/// copied into independent native containers (nested live values recurse);
/// anything else (functions, Dates, Maps, ...) keeps sharing the host lease.
fn host_deep_clone(lease: &Rc<HostLease>) -> Value {
    let plain = host_query(lease, 0).as_deref() == Some("object")
        && host_query(lease, 7).as_deref() == Some("0")
        && host_query(lease, 12).as_deref() == Some("0");
    if !plain { return Value::Host(Rc::clone(lease)); }
    if host_query(lease, 4).as_deref() == Some("1") {
        return Value::shared_array(host_enumerate_values(lease, 3).iter().map(|item| item.deep_clone_mode(true)).collect());
    }
    Value::shared_object(host_enumerable_entries(lease).into_iter()
        .map(|(key, value)| (key, value.deep_clone_mode(true))).collect())
}

/// Display uses the same serializer as `JSON.stringify`, including UTF-16 keys.
impl std::fmt::Display for Value {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        begin_stringify();
        let mut bytes = Vec::new();
        write_json_value(self, &mut bytes, None, 0);
        if thaw_json_take_stringify_error() != 0 {
            return Err(std::fmt::Error);
        }
        f.write_str(std::str::from_utf8(&bytes).map_err(|_| std::fmt::Error)?)
    }
}

thread_local! {
    // `Object.create(proto)`/`Object.setPrototypeOf` on a `Json`-typed
    // value record its prototype by shared identity. Same
    // pointer-identity side-table pattern `OBJECT_STATES` (thaw-runtime's
    // `objects.rs`) already uses for `Object.freeze`/`seal` state on
    // fixed-layout objects. Keyed by the *inner* shared container's own
    // address (`Rc::as_ptr`, stable across every alias/clone). Values
    // are owned clones so a prototype stays alive while the child does;
    // SharedValue::drop clears its entry before address reuse.
    static PROTOTYPES: RefCell<HashMap<usize, Value>> = RefCell::new(HashMap::new());
    static ARRAY_HOLES: RefCell<HashMap<usize, HashSet<usize>>> = RefCell::new(HashMap::new());
    static ASSIGN_ERROR: Cell<bool> = const { Cell::new(false) };
    static PROTOTYPE_ERROR: Cell<bool> = const { Cell::new(false) };
    static FROM_ENTRIES_ERROR: Cell<bool> = const { Cell::new(false) };
    static PARSE_ERROR: Cell<bool> = const { Cell::new(false) };
    static STRINGIFY_ERROR: Cell<bool> = const { Cell::new(false) };
    static STRINGIFY_ANCESTORS: RefCell<Vec<usize>> = const { RefCell::new(Vec::new()) };
    static WRAPPER_BRANDS: RefCell<HashSet<usize>> = RefCell::new(HashSet::new());
    static GRAPH_PLAIN_OBJECTS: RefCell<HashSet<usize>> = RefCell::new(HashSet::new());
    // `hdl` markers decoded without HostOperations: branded so they re-encode
    // as `hdl`, but not a projection of the original handle.
    static DETACHED_HANDLE_MARKERS: RefCell<HashSet<usize>> = RefCell::new(HashSet::new());
    static GRAPH_ERROR: Cell<bool> = const { Cell::new(false) };
}

struct StringifyAncestor(Option<usize>);

impl Drop for StringifyAncestor {
    fn drop(&mut self) {
        if let Some(identity) = self.0 {
            STRINGIFY_ANCESTORS.with(|ancestors| {
                let popped = ancestors.borrow_mut().pop();
                debug_assert_eq!(popped, Some(identity));
            });
        }
    }
}

fn stringify_ancestor(value: &Value) -> Option<StringifyAncestor> {
    let Some(identity) = object_identity_key(value) else {
        return Some(StringifyAncestor(None));
    };
    let cycle = STRINGIFY_ANCESTORS.with(|ancestors| ancestors.borrow().contains(&identity));
    if cycle {
        STRINGIFY_ERROR.with(|error| error.set(true));
        return None;
    }
    STRINGIFY_ANCESTORS.with(|ancestors| ancestors.borrow_mut().push(identity));
    Some(StringifyAncestor(Some(identity)))
}

fn begin_stringify() {
    STRINGIFY_ERROR.with(|error| error.set(false));
}

fn stringify_cycle_error() -> *const c_char {
    CString::new("\u{1}TypeError\u{1}Converting circular structure to JSON")
        .expect("static error message").into_raw()
}

#[no_mangle]
pub extern "C" fn thaw_json_take_stringify_error() -> u8 {
    STRINGIFY_ERROR.with(|error| u8::from(error.replace(false)))
}

fn object_writable(object: *const Value, key: &[u8]) -> bool {
    let state_key = unsafe { thaw_json_state_key(object) };
    if unsafe { thaw_object_state(state_key, 2) } {
        return false;
    }
    (unsafe { thaw_object_state(state_key, 0) })
        || unsafe { object.as_ref() }.is_some_and(|object| json_has_own_value(object, key))
}

#[no_mangle]
pub extern "C" fn thaw_json_take_assign_error() -> u8 {
    ASSIGN_ERROR.with(|error| u8::from(error.replace(false)))
}

#[no_mangle]
pub extern "C" fn thaw_json_take_prototype_error() -> u8 {
    PROTOTYPE_ERROR.with(|error| u8::from(error.replace(false)))
}

#[no_mangle]
pub extern "C" fn thaw_json_take_from_entries_error() -> u8 {
    FROM_ENTRIES_ERROR.with(|error| u8::from(error.replace(false)))
}

#[no_mangle]
pub extern "C" fn thaw_json_take_parse_error() -> u8 {
    PARSE_ERROR.with(|error| u8::from(error.replace(false)))
}

fn array_has_index(items: &SharedArray, index: usize) -> bool {
    index < shared_array_ref(items).len()
        && !ARRAY_HOLES.with(|holes| {
            holes.borrow().get(&(Rc::as_ptr(items) as usize))
                .is_some_and(|missing| missing.contains(&index))
        })
}

fn mark_array_holes(items: &SharedArray, indices: impl IntoIterator<Item = usize>) {
    ARRAY_HOLES.with(|holes| {
        holes.borrow_mut().entry(Rc::as_ptr(items) as usize)
            .or_default().extend(indices);
    });
}

fn mark_array_present(items: &SharedArray, index: usize) {
    ARRAY_HOLES.with(|holes| {
        if let Some(missing) = holes.borrow_mut().get_mut(&(Rc::as_ptr(items) as usize)) {
            missing.remove(&index);
        }
    });
}

fn copy_array_holes(source: &SharedArray, target: &SharedArray, skip: usize) {
    ARRAY_HOLES.with(|holes| {
        let mut holes = holes.borrow_mut();
        if let Some(missing) = holes.get(&(Rc::as_ptr(source) as usize)).cloned() {
            holes.insert(
                Rc::as_ptr(target) as usize,
                missing.into_iter().filter_map(|index| index.checked_sub(skip)).collect(),
            );
        }
    });
}

/// True when `value` is a live Host wrapper of a native owner (a projection that must receive
/// descriptor operations itself instead of a detached copy).
#[no_mangle]
pub unsafe extern "C" fn thaw_native_wrapper_is_live(value: *const Value) -> bool {
    let Some(Value::Host(lease)) = (unsafe { value.as_ref() }) else { return false };
    host_query(lease, 21).is_some_and(|text| usize::from_str_radix(text.trim(), 16).is_ok_and(|owner| owner != 0))
}

/// Shared identity for Object.freeze/seal/preventExtensions. Drop removes
/// runtime state before the Rc allocation can be reused.
#[no_mangle]
pub unsafe extern "C" fn thaw_json_state_key(value: *const Value) -> *const u8 {
    let Some(value) = (unsafe { value.as_ref() }) else {
        return std::ptr::null();
    };
    // A live wrapper of a native owner shares the owner's pointer-keyed integrity state.
    if let Value::Host(lease) = value {
        if let Some(owner) = host_query(lease, 21)
            .and_then(|text| usize::from_str_radix(text.trim(), 16).ok())
            .filter(|owner| *owner != 0)
        {
            return owner as *const u8;
        }
    }
    let Some(identity) = object_identity_key(value) else {
        return value as *const Value as *const u8;
    };
    identity as *const u8
}

/// The stable identity key for `object`'s own shared container (see
/// `PROTOTYPES`'s own doc comment) -- `None` for a scalar, which has no
/// shared identity to key on at all (matches the existing behavior:
/// `Object.getPrototypeOf`/`setPrototypeOf` on a primitive is a rare,
/// low-value case this compiler doesn't specially guard against
/// elsewhere either).
/// Short-hand for reading through a shared container's `UnsafeCell` --
/// used at every match arm that already destructured a `Value::Array`/
/// `Value::Object` down to its own `&SharedArray`/`&SharedObject` (so
/// `Value::as_array`/`as_object`, which need the whole `&Value`, don't
/// apply). See `SharedArray`'s own doc comment for why this is a raw
/// `UnsafeCell` read (not a `RefCell` borrow) in the first place.
fn shared_array_ref(items: &SharedArray) -> &Vec<Value> {
    unsafe { &*items.get() }
}

#[allow(clippy::mut_from_ref)]
fn shared_array_ref_mut(items: &SharedArray) -> &mut Vec<Value> {
    unsafe { &mut *items.get() }
}

fn shared_object_ref(fields: &SharedObject) -> &indexmap::IndexMap<Vec<u8>, Value> {
    unsafe { &*fields.get() }
}

#[allow(clippy::mut_from_ref)]
fn shared_object_ref_mut(fields: &SharedObject) -> &mut indexmap::IndexMap<Vec<u8>, Value> {
    unsafe { &mut *fields.get() }
}

fn object_identity_key(object: &Value) -> Option<usize> {
    match object {
        Value::Array(items) => Some(Rc::as_ptr(items) as usize),
        Value::Object(fields) => Some(Rc::as_ptr(fields) as usize),
        // Native allocations are aligned; the low-bit tag keeps canonical
        // host handle IDs disjoint from all native allocation identities.
        Value::Host(lease) => Some(((lease.handle as usize) << 1) | 1),
        _ => None,
    }
}

/// Decodes native WTF-8 bytes to a `String`, replacing each lone
/// surrogate (a 3-byte WTF-8 sequence) with a single U+FFFD -- where
/// `from_utf8_lossy` would emit one per invalid byte (3). A `String`
/// cannot hold a lone surrogate, so this is the lossy projection used
/// wherever a value must become a `serde_json::String` (the JS boundary)
/// or a Rust `&str`.
fn wtf8_to_string(bytes: &[u8]) -> String {
    match std::str::from_utf8(bytes) {
        Ok(text) => text.to_string(),
        Err(_) => {
            let mut fixed = Vec::with_capacity(bytes.len());
            let mut index = 0;
            while index < bytes.len() {
                if bytes[index] == 0xED
                    && index + 2 < bytes.len()
                    && (0xA0..=0xBF).contains(&bytes[index + 1])
                    && bytes[index + 2] & 0xC0 == 0x80
                {
                    fixed.extend_from_slice("\u{FFFD}".as_bytes());
                    index += 3;
                } else {
                    fixed.push(bytes[index]);
                    index += 1;
                }
            }
            String::from_utf8_lossy(&fixed).into_owned()
        }
    }
}

fn to_str(ptr: *const c_char) -> String {
    wtf8_to_string(unsafe { CStr::from_ptr(ptr) }.to_bytes())
}

fn to_key(ptr: *const c_char) -> Vec<u8> {
    canonical_key(unsafe { CStr::from_ptr(ptr) }.to_bytes())
}

fn canonical_key(bytes: &[u8]) -> Vec<u8> {
    wtf8_encode_utf16(&wtf8_decode_utf16(bytes))
}

fn utf8_or_wtf8(bytes: Vec<u8>) -> Value {
    match String::from_utf8(bytes) {
        Ok(text) => Value::String(text),
        Err(error) => Value::Wtf8(error.into_bytes()),
    }
}

/// A native string *value* being marshaled into JSON. Thaw's `HirType::Str`
/// is a bare pointer with no "undefined" representation, and a class
/// instance field declared without an initializer (`name!: string`) is left
/// null -- `console.log` already renders that as `(null)`. Marshaling must
/// therefore emit a JSON `null` for it rather than dereferencing the null
/// pointer: `CStr::from_ptr(null)` is undefined behavior, and the `strlen`
/// it runs is a hard segfault. Real trigger: a native class whose `string`
/// field has no initializer, constructed from JavaScript through a callback
/// (`new cls()` inside class-transformer's own `plainToInstance`).
fn string_value(value: *const c_char) -> Value {
    if value.is_null() {
        Value::Null
    } else {
        // Keep the raw WTF-8 bytes so a lone surrogate survives to the
        // serializer; a valid-UTF-8 string stays the common `Value::String`.
        let bytes = unsafe { CStr::from_ptr(value) }.to_bytes();
        utf8_or_wtf8(bytes.to_vec())
    }
}

// A Box<Value> is outside thaw-arena, but generated globals/captured arena
// cells can hold its raw pointer. Keep one arena token reachable from that
// pointer and dispose the box when the token becomes unreachable at reset.
thread_local! {
    // The serial distinguishes an old dead root from a new Box reusing its
    // address while a Host release callback reenters this thread.
    static NEXT_ARENA_OWNED_JSON_ROOT: Cell<u64> = const { Cell::new(0) };
    static ARENA_OWNED_JSON_ROOTS: RefCell<HashMap<usize, (usize, bool, u64)>> = RefCell::new(HashMap::new());
}

fn untrack_arena_owned_json_root(value: *mut Value) {
    let removed = ARENA_OWNED_JSON_ROOTS.with(|roots| roots.borrow_mut().remove(&(value as usize)));
    if removed.is_some() { thaw_arena::forget_references(value as usize); }
}

fn reset_arena_owned_json_roots(tracing: bool) {
    let dead = ARENA_OWNED_JSON_ROOTS.with(|roots| {
        roots.borrow().iter().filter_map(|(&value, &(token, was_traced, serial))| {
            (!tracing || !was_traced || thaw_arena::was_reclaimed(token))
                .then_some((value, serial))
        }).collect::<Vec<_>>()
    });
    for (value, serial) in dead {
        // A previous release callback can destroy this root and even create
        // a new Box at the same address. Remove only the exact old owner,
        // and drop it with no side-table borrow held across reentry.
        let removed = ARENA_OWNED_JSON_ROOTS.with(|roots| {
            let mut roots = roots.borrow_mut();
            if roots.get(&value).is_some_and(|entry| entry.2 == serial) {
                roots.remove(&value)
            } else { None }
        });
        if removed.is_none() { continue; }
        thaw_arena::forget_references(value);
        unsafe { thaw_json_destroy(value as *mut Value) };
    }
}

/// Tie an owned Json box to the existing arena reachability graph.
/// Returning or capturing its pointer preserves the token; an unescaped
/// temporary is released when the invocation arena resets.
#[no_mangle]
pub extern "C" fn thaw_json_track_arena_owned_root(value: *mut Value) -> *mut Value {
    if value.is_null() { return value; }
    if ARENA_OWNED_JSON_ROOTS.with(|roots| roots.borrow().contains_key(&(value as usize))) {
        return value;
    }
    let traced = thaw_arena::is_tracing();
    let token = thaw_arena::thaw_arena_alloc(1, 1);
    if token.is_null() {
        set_host_error("Unable to retain JSON value".into());
        unsafe { thaw_json_destroy(value) };
        return std::ptr::null_mut();
    }
    thaw_arena::register_reset_hook(reset_arena_owned_json_roots);
    let serial = NEXT_ARENA_OWNED_JSON_ROOT.with(|next| {
        let serial = next.get().checked_add(1);
        if let Some(serial) = serial { next.set(serial); }
        serial
    });
    let Some(serial) = serial else {
        set_host_error("Unable to retain JSON value".into());
        unsafe { thaw_json_destroy(value) };
        return std::ptr::null_mut();
    };
    ARENA_OWNED_JSON_ROOTS.with(|roots| {
        roots.borrow_mut().insert(value as usize, (token as usize, traced, serial));
    });
    if traced { thaw_arena::replace_reference(value as usize, 0, token as usize); }
    value
}

fn leak(value: Value) -> *mut Value {
    Box::into_raw(Box::new(value))
}

/// Records `prototype` as `object`'s prototype (`Object.create(proto)`,
/// `Object.setPrototypeOf(object, prototype)`). Returns `object` back
/// unchanged, so a caller can use this as one leg of a single expression
/// (`Object.setPrototypeOf` itself returns its first argument) instead of
/// needing a separate void-call statement. See `PROTOTYPES`'s own doc
/// comment for what this does and doesn't model.
///
/// # Safety
/// `object` must be null or point to a valid, mutable JSON `Value`;
/// `prototype` must be null or point to a valid JSON `Value` that
/// outlives this call.
#[no_mangle]
pub unsafe extern "C" fn thaw_json_set_prototype(
    object: *mut Value,
    prototype: *mut Value,
) -> *mut Value {
    PROTOTYPE_ERROR.with(|error| error.set(false));
    if object.is_null() {
        PROTOTYPE_ERROR.with(|error| error.set(true));
        return object;
    }
    let Some(key) = object_identity_key(unsafe { &*object }) else {
        PROTOTYPE_ERROR.with(|error| error.set(true));
        return object;
    };
    let valid_prototype = match unsafe { prototype.as_ref() } {
        Some(Value::Null | Value::Array(_) | Value::Host(_)) => true,
        Some(value @ Value::Object(_)) => {
            !is_napi_undefined(value) && non_finite_number(value).is_none()
        }
        _ => false,
    };
    if !valid_prototype {
        PROTOTYPE_ERROR.with(|error| error.set(true));
        return object;
    }
    let previous = PROTOTYPES.with(|table| table.borrow().get(&key).cloned());
    let previous_key = previous.as_ref().and_then(object_identity_key);
    let new_key = unsafe { prototype.as_ref() }.and_then(object_identity_key);
    if !unsafe { thaw_object_state(thaw_json_state_key(object), 0) }
        && previous_key != new_key
    {
        PROTOTYPE_ERROR.with(|error| error.set(true));
        return object;
    }
    let mut current = unsafe { &*prototype }.clone();
    let mut seen = HashSet::new();
    while let Some(identity) = object_identity_key(&current) {
        if identity == key || !seen.insert(identity) {
            PROTOTYPE_ERROR.with(|error| error.set(true));
            return object;
        }
        let Some(next) = PROTOTYPES.with(|table| table.borrow().get(&identity).cloned()) else {
            break;
        };
        current = next;
    }
    PROTOTYPES.with(|table| {
        let previous = { table.borrow_mut().insert(key, unsafe { &*prototype }.clone()) };
        drop(previous);
    });
    object
}

/// Returns the prototype `thaw_json_set_prototype` recorded for `object`,
/// or a fresh JSON `null` if none was ever recorded (`Object.create(null)`,
/// or a plain object literal that was never passed through
/// `Object.create`/`Object.setPrototypeOf`).
#[no_mangle]
pub extern "C" fn thaw_json_get_prototype(object: *const Value) -> *mut Value {
    let Some(key) = (unsafe { object.as_ref() }).and_then(object_identity_key) else {
        return leak(Value::Null);
    };
    leak(PROTOTYPES
        .with(|table| table.borrow().get(&key).cloned())
        .unwrap_or(Value::Null))
}

/// Gives a borrowed Json value (e.g. the arena-owned result of a caught
/// exception projection) a new owned handle that shares its containers
/// (`Value::clone` is `Rc::clone` for arrays/objects), so the caller can
/// destroy or arena-root the result without invalidating the borrowed one.
/// Null stays null. Unlike `thaw_json_clone` this is not a deep copy.
///
/// # Safety
/// `value` must be null or point to a valid JSON `Value`.
#[no_mangle]
pub unsafe extern "C" fn thaw_json_share(value: *const Value) -> *mut Value {
    let Some(value) = (unsafe { value.as_ref() }) else {
        return std::ptr::null_mut();
    };
    leak(value.clone())
}

#[no_mangle]
/// `structuredClone` on a dynamic (`Json`-typed) value -- unlike this
/// file's derived `Clone` (which shares the same `Rc`-backed `Array`/
/// `Object` container, see `Value`'s own doc comment), this needs a
/// genuinely independent recursive copy so mutating the clone never
/// observably affects the original, matching the specification.
///
/// # Safety
/// `value` must be null or point to a valid JSON `Value`.
pub unsafe extern "C" fn thaw_json_clone(value: *const Value) -> *mut Value {
    let Some(value) = (unsafe { value.as_ref() }) else {
        return leak(Value::Null);
    };
    leak(value.deep_clone())
}

/// `structuredClone` of a dynamic value: like `thaw_json_clone`, but live host
/// objects become independent native copies instead of sharing the lease.
///
/// # Safety
/// `value` must be null or point to a valid JSON `Value`.
#[no_mangle]
pub unsafe extern "C" fn thaw_json_structured_clone(value: *const Value) -> *mut Value {
    let Some(value) = (unsafe { value.as_ref() }) else {
        return leak(Value::Null);
    };
    leak(value.deep_clone_mode(true))
}

fn number_value(value: f64) -> Value {
    if value.is_finite() && value.fract() == 0.0 && value.to_bits() != (-0.0f64).to_bits() {
        if value >= i64::MIN as f64 && value <= i64::MAX as f64 {
            return Value::Number((value as i64).into());
        }
        if value >= 0.0 && value <= u64::MAX as f64 {
            return Value::Number((value as u64).into());
        }
    }
    if let Some(number) = serde_json::Number::from_f64(value) {
        return Value::Number(number);
    }
    // `NaN`/`Infinity`/`-Infinity` have no JSON representation --
    // `serde_json::Number` structurally cannot hold one, so this used to
    // silently collapse to `Value::Null`, indistinguishable from a real
    // `null`. That broke every consumer that needs to tell them apart
    // (`typeof`, `===`, `Number.isNaN`, arithmetic coercion), even
    // though those consumers were already built to recognize the
    // sentinel below (`non_finite_number`, `json_to_number`,
    // `thaw_json_typeof`, `thaw_json_strict_equal`/`json_same_value_
    // zero` in thaw-runtime) -- it just wasn't being produced by this,
    // the main native f64-to-`Json` conversion path. Only
    // `platform_globals/dates.js`'s `__thaw_json_safe_stringify`
    // replacer (a JS-side `JSON.stringify` round-trip) tagged it,
    // covering values crossing the QuickJS boundary but not an
    // ordinary `any`-typed `NaN`/`Infinity` constructed in compiled
    // code. Tag it here too, the exact same shape, so both paths agree.
    napi_non_finite_value(value)
}

fn napi_non_finite_value(value: f64) -> Value {
    let mut fields = indexmap::IndexMap::new();
    let tag = if value.is_nan() {
        "NaN"
    } else if value > 0.0 {
        "Infinity"
    } else {
        "-Infinity"
    };
    fields.insert(
        b"$__thaw_non_finite$".to_vec(),
        Value::String(tag.to_string()),
    );
    brand_internal_value(Value::shared_object(fields))
}

fn array_index_key(key: &[u8]) -> Option<u32> {
    let text = std::str::from_utf8(key).ok()?;
    let index = text.parse::<u32>().ok()?;
    (index != u32::MAX && index.to_string() == text).then_some(index)
}

fn ordered_object_fields(fields: &indexmap::IndexMap<Vec<u8>, Value>) -> Vec<(&Vec<u8>, &Value)> {
    let mut indices = Vec::new();
    let mut names = Vec::new();
    for (key, value) in fields {
        if let Some(index) = array_index_key(key) {
            indices.push((index, key, value));
        } else {
            names.push((key, value));
        }
    }
    indices.sort_by_key(|(index, _, _)| *index);
    indices
        .into_iter()
        .map(|(_, key, value)| (key, value))
        .chain(names)
        .collect()
}

fn ordered_object_fields_shared(fields: &SharedObject) -> Vec<(&Vec<u8>, &Value)> {
    ordered_object_fields(unsafe { &*fields.get() })
}

// NOTE: `ordered_json` deliberately does NOT special-case the
// napi-undefined sentinel (e.g. to omit a sentinel-valued object field
// or null a sentinel-valued array element the way real `JSON.stringify`
// treats a real `undefined`) -- an earlier version of this fix did, but
// `thaw_json_stringify` (which this function backs) is not only reached
// by user-facing `JSON.stringify(...)` calls: it's *also* the mechanism
// several dynamic-call argument/result marshaling paths use internally
// to serialize a value (often nested inside a fresh single-element
// array, e.g. `compile_set_dynamic_property_json`,
// `crates/thaw-llvm/src/hir_codegen/invocations/dynamic_calls.rs`)
// before handing it to the live QuickJS engine, whose own deserializer
// specifically looks for the sentinel's exact shape to revive a real
// `undefined` argument. Omitting/nulling it here would silently corrupt
// that unrelated, pervasive mechanism instead -- confirmed by a real
// regression in existing tests (`a_bare_undefined_literal_can_be_
// passed_as_a_dynamic_call_argument` and its `Optional`-typed sibling,
// `crates/thaw-cli/src/tests/registry_fallback/classes_and_values.rs`)
// when this was tried. `thaw_json_stringify_public` (below) is the
// sentinel-aware sibling that's actually safe to use for a real
// `JSON.stringify(...)` call, precisely because it's reached *only*
// from there -- see its own doc comment.
fn ordered_json(value: &Value) -> Value {
    let Some(_ancestor) = stringify_ancestor(value) else { return Value::Null };
    match value {
        Value::Object(fields) => Value::shared_object(
            ordered_object_fields_shared(fields)
                .into_iter()
                .map(|(key, value)| (key.clone(), ordered_json(value)))
                .collect(),
        ),
        Value::Array(items) => {
            Value::shared_array(unsafe { &*items.get() }.iter().map(ordered_json).collect())
        }
        other => other.clone(),
    }
}

/// Like `ordered_json`, but matches real `JSON.stringify`'s own treatment
/// of a genuinely `undefined` *nested* value: an object field whose value
/// is the napi-undefined sentinel is omitted entirely (`JSON.stringify({a:
/// 1, b: undefined})` -> `{"a":1}`), and an array element that's the
/// sentinel is replaced with `null` (`JSON.stringify([1, undefined, 3])`
/// -> `[1,null,3]`) -- JS's own real object-vs-array asymmetry for
/// `undefined`. Only ever called from `thaw_json_stringify_public` and
/// the other user-facing `JSON.stringify` variants
/// (`stringify_with_indent`/`filtered_json_omitting_undefined`), never
/// from the internal marshaling paths that need the sentinel preserved
/// verbatim -- see `ordered_json`'s own doc comment for why those two
/// audiences can't share one implementation. The bare top-level case
/// (the `JSON.stringify` argument itself, not nested, *is* the
/// sentinel) is handled by this function's three callers themselves,
/// before they ever reach here -- see `top_level_undefined_string`'s
/// own doc comment.
fn ordered_json_omitting_undefined(value: &Value) -> Value {
    if non_finite_number(value).is_some() {
        return Value::Null;
    }
    match date_iso_string(value) {
        Some(Some(iso)) => return Value::String(iso),
        Some(None) => return Value::Null,
        None => {}
    }
    let Some(_ancestor) = stringify_ancestor(value) else { return Value::Null };
    match value {
        Value::Object(_) if is_thaw_internal_wrapper(value) => {
            Value::shared_object(indexmap::IndexMap::new())
        }
        Value::Object(fields) => Value::shared_object(
            ordered_object_fields_shared(fields)
                .into_iter()
                .filter(|(_, value)| !is_napi_undefined(value))
                .map(|(key, value)| {
                    let value = if non_finite_number(value).is_some() {
                        Value::Null
                    } else {
                        ordered_json_omitting_undefined(value)
                    };
                    (key.clone(), value)
                })
                .collect(),
        ),
        Value::Array(items) => Value::shared_array(
            shared_array_ref(items)
                .iter()
                .map(|item| {
                    if is_napi_undefined(item) || non_finite_number(item).is_some() {
                        Value::Null
                    } else {
                        ordered_json_omitting_undefined(item)
                    }
                })
                .collect(),
        ),
        other => other.clone(),
    }
}

/// Sentinel-aware sibling of `filtered_json`, for the same reason
/// `ordered_json_omitting_undefined` exists alongside `ordered_json` --
/// backs `JSON.stringify(value, [keys])`/`JSON.stringify(value, [keys],
/// space)`, both exclusively user-facing call forms.
fn filtered_json_omitting_undefined(value: &Value, keys: &[Vec<u8>]) -> Value {
    if non_finite_number(value).is_some() {
        return Value::Null;
    }
    // `Date.prototype.toJSON` runs before the replacer-keys `PropertyList`
    // filter even applies (the filter only ever narrows an *object*'s own
    // enumerable keys) -- a `Date` argument (or nested field) stringifies
    // to its ISO string exactly like the no-replacer form, confirmed
    // against real Node (`JSON.stringify(date, ["timestamp"])` still
    // gives the plain ISO string, not `{"timestamp":...}`).
    match date_iso_string(value) {
        Some(Some(iso)) => return Value::String(iso),
        Some(None) => return Value::Null,
        None => {}
    }
    let Some(_ancestor) = stringify_ancestor(value) else { return Value::Null };
    match value {
        // The `PropertyList` filter's own `[[Get]]` walks the prototype
        // chain for each requested key -- unlike the no-replacer form
        // (which only ever asks "does this wrapper have any *own*
        // enumerable properties" -- none, so always `{}`), a replacer-
        // keys array can name a *prototype accessor* (`RegExp.prototype.
        // source`/`Map.prototype.size`/...), which real `[[Get]]` still
        // invokes and includes if present. `regexp_wrapper_property`/
        // `map_or_set_wrapper_size` are the same accessor-emulation
        // helpers `thaw_json_get` already uses for a direct `re.source`/
        // `m.size` read on a bare `any`-typed value -- reused here
        // instead of the previous unconditional empty object, matching
        // real Node exactly for the common requested keys (`source`/
        // `flags`/`lastIndex`/`global`/etc. for a RegExp, `size` for a
        // Map/Set). A key that resolves to nothing (no such accessor,
        // e.g. `["a"]` against a Map) is omitted, matching a real
        // `undefined` `[[Get]]` result being skipped by `JSON.stringify`.
        Value::Object(_) if is_thaw_internal_wrapper(value) => {
            Value::shared_object(
                keys.iter()
                    .filter_map(|key| {
                        let resolved = regexp_wrapper_property(value, &wtf8_to_string(key)).or_else(|| {
                            (key.as_slice() == b"size")
                                .then(|| map_or_set_wrapper_size(value))
                                .flatten()
                        })?;
                        if is_napi_undefined(&resolved) {
                            return None;
                        }
                        let resolved = if non_finite_number(&resolved).is_some() {
                            Value::Null
                        } else {
                            resolved
                        };
                        Some((key.clone(), resolved))
                    })
                    .collect(),
            )
        }
        Value::Object(fields) => Value::shared_object(
            keys.iter()
                .filter_map(|key| {
                    let value = shared_object_ref(fields).get(key.as_slice())?;
                    if is_napi_undefined(value) {
                        return None;
                    }
                    let value = if non_finite_number(value).is_some() {
                        Value::Null
                    } else {
                        filtered_json_omitting_undefined(value, keys)
                    };
                    Some((key.clone(), value))
                })
                .collect(),
        ),
        Value::Array(items) => Value::shared_array(
            shared_array_ref(items)
                .iter()
                .map(|item| {
                    if is_napi_undefined(item) || non_finite_number(item).is_some() {
                        Value::Null
                    } else {
                        filtered_json_omitting_undefined(item, keys)
                    }
                })
                .collect(),
        ),
        other => other.clone(),
    }
}

#[no_mangle]
/// Encodes UTF-16 code units to WTF-8, combining a valid surrogate pair
/// into one astral code point and leaving a lone surrogate as a 3-byte
/// sequence.
fn wtf8_encode_utf16(units: &[u16]) -> Vec<u8> {
    fn push(bytes: &mut Vec<u8>, code: u32) {
        if code < 0x80 {
            bytes.push(code as u8);
        } else if code < 0x800 {
            bytes.push(0xC0 | (code >> 6) as u8);
            bytes.push(0x80 | (code & 0x3F) as u8);
        } else if code < 0x10000 {
            bytes.push(0xE0 | (code >> 12) as u8);
            bytes.push(0x80 | ((code >> 6) & 0x3F) as u8);
            bytes.push(0x80 | (code & 0x3F) as u8);
        } else {
            bytes.push(0xF0 | (code >> 18) as u8);
            bytes.push(0x80 | ((code >> 12) & 0x3F) as u8);
            bytes.push(0x80 | ((code >> 6) & 0x3F) as u8);
            bytes.push(0x80 | (code & 0x3F) as u8);
        }
    }
    let mut bytes = Vec::with_capacity(units.len());
    let mut index = 0;
    while index < units.len() {
        let unit = units[index];
        if (0xD800..=0xDBFF).contains(&unit)
            && index + 1 < units.len()
            && (0xDC00..=0xDFFF).contains(&units[index + 1])
        {
            let code = 0x10000
                + ((u32::from(unit) - 0xD800) << 10)
                + (u32::from(units[index + 1]) - 0xDC00);
            push(&mut bytes, code);
            index += 2;
        } else {
            push(&mut bytes, u32::from(unit));
            index += 1;
        }
    }
    bytes
}

/// A minimal JSON parser producing this module's own `Value` directly.
/// `serde_json` is unusable for `JSON.parse` here because it rejects a
/// lone-surrogate `\uXXXX` escape (turning `JSON.parse('"\\ud800"')` into
/// `null`); this parser keeps it as a `Value::Wtf8`.
struct JsonParser<'a> {
    bytes: &'a [u8],
    position: usize,
}

impl<'a> JsonParser<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, position: 0 }
    }

    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.position).copied()
    }

    fn skip_whitespace(&mut self) {
        while let Some(byte) = self.peek() {
            if matches!(byte, b' ' | b'\t' | b'\n' | b'\r') {
                self.position += 1;
            } else {
                break;
            }
        }
    }

    fn parse(&mut self) -> Option<Value> {
        self.skip_whitespace();
        let value = self.parse_value()?;
        self.skip_whitespace();
        (self.position == self.bytes.len()).then_some(value)
    }

    fn parse_value(&mut self) -> Option<Value> {
        self.skip_whitespace();
        match self.peek()? {
            b'{' => self.parse_object(),
            b'[' => self.parse_array(),
            b'"' => self.parse_string(),
            b't' => self.consume(b"true").map(|_| Value::Bool(true)),
            b'f' => self.consume(b"false").map(|_| Value::Bool(false)),
            b'n' => self.consume(b"null").map(|_| Value::Null),
            _ => self.parse_number(),
        }
    }

    fn consume(&mut self, literal: &[u8]) -> Option<()> {
        self.bytes[self.position..]
            .starts_with(literal)
            .then(|| self.position += literal.len())
    }

    fn parse_number(&mut self) -> Option<Value> {
        let start = self.position;
        while let Some(byte) = self.peek() {
            if matches!(byte, b'-' | b'+' | b'.' | b'e' | b'E' | b'0'..=b'9') {
                self.position += 1;
            } else {
                break;
            }
        }
        let text = std::str::from_utf8(&self.bytes[start..self.position]).ok()?;
        if !valid_json_number(text.as_bytes()) {
            return None;
        }
        if text == "-0" {
            return Some(number_value(-0.0));
        }
        match serde_json::from_str::<serde_json::Number>(text) {
            Ok(number) => Some(Value::Number(number)),
            Err(_) => text.parse::<f64>().ok().map(number_value),
        }
    }

    fn parse_string(&mut self) -> Option<Value> {
        // Current byte is the opening quote.
        self.position += 1;
        let mut units: Vec<u16> = Vec::new();
        loop {
            match self.peek()? {
                b'"' => {
                    self.position += 1;
                    break;
                }
                b'\\' => {
                    self.position += 1;
                    let escape = self.peek()?;
                    self.position += 1;
                    match escape {
                        b'"' => units.push(0x22),
                        b'\\' => units.push(0x5C),
                        b'/' => units.push(0x2F),
                        b'b' => units.push(0x08),
                        b'f' => units.push(0x0C),
                        b'n' => units.push(0x0A),
                        b'r' => units.push(0x0D),
                        b't' => units.push(0x09),
                        b'u' => units.push(self.parse_hex4()?),
                        _ => return None,
                    }
                }
                0x00..=0x1F => return None,
                _ => {
                    let (width, code) = self.decode_utf8()?;
                    if code >= 0x10000 {
                        let code = code - 0x10000;
                        units.push(0xD800 + (code >> 10) as u16);
                        units.push(0xDC00 + (code & 0x3FF) as u16);
                    } else {
                        units.push(code as u16);
                    }
                    self.position += width;
                }
            }
        }
        // A lone surrogate survives as `Value::Wtf8`; otherwise a plain
        // `String`.
        let lone = units.iter().enumerate().any(|(index, &unit)| {
            if !(0xD800..=0xDFFF).contains(&unit) {
                return false;
            }
            if (0xD800..=0xDBFF).contains(&unit) {
                // High surrogate: paired when the next unit is a low one.
                !units
                    .get(index + 1)
                    .is_some_and(|next| (0xDC00..=0xDFFF).contains(next))
            } else {
                // Low surrogate: paired when the previous unit is a high one.
                !(index > 0 && (0xD800..=0xDBFF).contains(&units[index - 1]))
            }
        });
        if lone {
            Some(Value::Wtf8(wtf8_encode_utf16(&units)))
        } else {
            String::from_utf16(&units).ok().map(Value::String)
        }
    }

    fn parse_hex4(&mut self) -> Option<u16> {
        let mut value = 0u16;
        for _ in 0..4 {
            let digit = (self.peek()? as char).to_digit(16)? as u16;
            value = value * 16 + digit;
            self.position += 1;
        }
        Some(value)
    }

    fn decode_utf8(&self) -> Option<(usize, u32)> {
        let bytes = &self.bytes[self.position..];
        let first = *bytes.first()?;
        let continuation = |byte: u8| byte & 0xC0 == 0x80;
        if first < 0x80 {
            Some((1, u32::from(first)))
        } else if (0xC2..=0xDF).contains(&first) && bytes.len() >= 2 && continuation(bytes[1]) {
            Some((
                2,
                ((u32::from(first) & 0x1F) << 6) | (u32::from(bytes[1]) & 0x3F),
            ))
        } else if (0xE0..=0xEF).contains(&first)
            && bytes.len() >= 3
            && continuation(bytes[1])
            && continuation(bytes[2])
        {
            Some((
                3,
                ((u32::from(first) & 0x0F) << 12)
                    | ((u32::from(bytes[1]) & 0x3F) << 6)
                    | (u32::from(bytes[2]) & 0x3F),
            ))
        } else if (0xF0..=0xF4).contains(&first)
            && bytes.len() >= 4
            && continuation(bytes[1])
            && continuation(bytes[2])
            && continuation(bytes[3])
        {
            Some((
                4,
                ((u32::from(first) & 0x07) << 18)
                    | ((u32::from(bytes[1]) & 0x3F) << 12)
                    | ((u32::from(bytes[2]) & 0x3F) << 6)
                    | (u32::from(bytes[3]) & 0x3F),
            ))
        } else {
            Some((1, 0xFFFD))
        }
    }

    fn parse_array(&mut self) -> Option<Value> {
        self.position += 1; // '['
        let mut items = Vec::new();
        self.skip_whitespace();
        if self.peek()? == b']' {
            self.position += 1;
            return Some(Value::shared_array(items));
        }
        loop {
            items.push(self.parse_value()?);
            self.skip_whitespace();
            match self.peek()? {
                b',' => self.position += 1,
                b']' => {
                    self.position += 1;
                    break;
                }
                _ => return None,
            }
        }
        Some(Value::shared_array(items))
    }

    fn parse_object(&mut self) -> Option<Value> {
        self.position += 1; // '{'
        let mut fields = indexmap::IndexMap::new();
        self.skip_whitespace();
        if self.peek()? == b'}' {
            self.position += 1;
            return Some(Value::shared_object(fields));
        }
        loop {
            self.skip_whitespace();
            if self.peek()? != b'"' {
                return None;
            }
            let key = match self.parse_string()? {
                Value::String(text) => canonical_key(text.as_bytes()),
                Value::Wtf8(bytes) => canonical_key(&bytes),
                _ => return None,
            };
            self.skip_whitespace();
            if self.peek()? != b':' {
                return None;
            }
            self.position += 1;
            let value = self.parse_value()?;
            fields.insert(key, value);
            self.skip_whitespace();
            match self.peek()? {
                b',' => self.position += 1,
                b'}' => {
                    self.position += 1;
                    break;
                }
                _ => return None,
            }
        }
        Some(Value::shared_object(fields))
    }
}

fn valid_json_number(bytes: &[u8]) -> bool {
    let mut index = usize::from(bytes.first() == Some(&b'-'));
    if bytes.get(index) == Some(&b'0') {
        index += 1;
    } else {
        let start = index;
        while bytes.get(index).is_some_and(|byte| byte.is_ascii_digit()) {
            index += 1;
        }
        if index == start || bytes[start] == b'0' {
            return false;
        }
    }
    if bytes.get(index) == Some(&b'.') {
        index += 1;
        let start = index;
        while bytes.get(index).is_some_and(|byte| byte.is_ascii_digit()) {
            index += 1;
        }
        if index == start {
            return false;
        }
    }
    if matches!(bytes.get(index), Some(&b'e' | &b'E')) {
        index += 1;
        if matches!(bytes.get(index), Some(&b'+' | &b'-')) {
            index += 1;
        }
        let start = index;
        while bytes.get(index).is_some_and(|byte| byte.is_ascii_digit()) {
            index += 1;
        }
        if index == start {
            return false;
        }
    }
    index == bytes.len()
}

#[no_mangle]
pub extern "C" fn thaw_json_parse(text: *const c_char) -> *mut Value {
    let bytes = unsafe { CStr::from_ptr(text) }.to_bytes();
    let value = JsonParser::new(bytes).parse();
    PARSE_ERROR.with(|error| error.set(value.is_none()));
    let value = value.unwrap_or(Value::Null);
    leak(value)
}

enum DestroyNode {
    Array(SharedArray),
    Object(SharedObject),
}

impl DestroyNode {
    fn strong_count(&self) -> usize {
        match self {
            Self::Array(items) => Rc::strong_count(items),
            Self::Object(fields) => Rc::strong_count(fields),
        }
    }

    fn values(&self) -> Vec<Value> {
        match self {
            Self::Array(items) => shared_array_ref(items).clone(),
            Self::Object(fields) => shared_object_ref(fields).values().cloned().collect(),
        }
    }

    fn take_values(&self) -> Vec<Value> {
        match self {
            Self::Array(items) => std::mem::take(shared_array_ref_mut(items)),
            Self::Object(fields) => std::mem::take(shared_object_ref_mut(fields))
                .into_iter().map(|(_, value)| value).collect(),
        }
    }

    fn take_napi_lease(&self) -> Option<Rc<NapiLease>> {
        match self {
            Self::Array(_) => None,
            Self::Object(fields) => fields.napi_lease.borrow_mut().take(),
        }
    }
}

// A graph result may contain native Rc cycles, including a HostLease inside
// one of the cycle's fields. Dropping the Box alone cannot release them. Count
// edges within the reachable component, then keep every node reachable from
// another live Box/alias. Only the isolated remainder may have edges severed.
// The prototype side table owns real Value clones, so it participates as an
// outgoing edge from its key's object rather than looking like an outside root.
fn release_isolated_json_cycles(root: &Value) {
    let root_identity = object_identity_key(root);
    let mut nodes = HashMap::<usize, DestroyNode>::new();
    let mut edges = HashMap::<usize, Vec<usize>>::new();
    let mut pending = vec![root.clone()];
    while let Some(value) = pending.pop() {
        let Some(identity) = object_identity_key(&value) else { continue };
        if nodes.contains_key(&identity) { continue; }
        let node = match value {
            Value::Array(items) => DestroyNode::Array(items),
            Value::Object(fields) => DestroyNode::Object(fields),
            Value::Host(_) => continue,
            _ => continue,
        };
        let mut children = node.values();
        if let Some(prototype) = PROTOTYPES.with(|table| table.borrow().get(&identity).cloned()) {
            children.push(prototype);
        }
        edges.insert(identity, children.iter().filter_map(object_identity_key).collect());
        nodes.insert(identity, node);
        pending.extend(children);
    }

    let mut internal = HashMap::<usize, usize>::new();
    for targets in edges.values() {
        for target in targets {
            if nodes.contains_key(target) {
                *internal.entry(*target).or_default() += 1;
            }
        }
    }
    let mut retained = HashSet::new();
    let mut pending = nodes.iter().filter_map(|(identity, node)| {
        // Each entry in `nodes` itself owns one temporary Rc reference.
        // The root Box owns one more reference, which this destroy consumes.
        let accounted = 1 + internal.get(identity).copied().unwrap_or_default()
            + usize::from(Some(*identity) == root_identity);
        (node.strong_count() != accounted).then_some(*identity)
    }).collect::<Vec<_>>();
    while let Some(identity) = pending.pop() {
        if !retained.insert(identity) { continue; }
        if let Some(targets) = edges.get(&identity) {
            pending.extend(targets.iter().filter(|target| nodes.contains_key(*target)).copied());
        }
    }
    let mut detached = Vec::new();
    let mut pending_napi_release = Vec::new();
    for (identity, node) in &nodes {
        if retained.contains(identity) { continue; }
        // Detach every edge before dropping a HostLease: its release callback
        // enters QuickJS and may run user-observable code. No side-table borrow
        // or partly-cleared graph remains when the callbacks run.
        detached.extend(PROTOTYPES.with(|table| table.borrow_mut().remove(identity)));
        detached.extend(node.take_values());
        pending_napi_release.extend(node.take_napi_lease());
    }
    // A HostLease release can re-enter thaw_json_destroy for another Box.
    // Drop all temporary Rc graph owners before calling any such release:
    // otherwise a nested traversal mistakes these guards for external aliases
    // and can leave a newly isolated cycle behind.
    let mut pending_host_release = Vec::new();
    let mut detached_non_host = Vec::new();
    for value in detached {
        match value {
            Value::Host(lease) => pending_host_release.push(lease),
            other => detached_non_host.push(other),
        }
    }
    drop(nodes);
    drop(detached_non_host);
    drop(pending_host_release);
    drop(pending_napi_release);
}

/// # Safety
///
/// `value` must be null or a pointer returned by this module's JSON constructors,
/// and it must not be used or destroyed again after this call.
#[no_mangle]
pub unsafe extern "C" fn thaw_json_destroy(value: *mut Value) {
    if !value.is_null() {
        // Dropping the last Host/NAPI lease may invoke a reentrant release
        // callback. Destruction is cleanup: preserve the operation's prior
        // Host error and discard any error reported only by that callback.
        let original_host_error = HOST_ERROR.with(|slot| slot.borrow_mut().take());
        untrack_typed_decode_child(value);
        untrack_arena_owned_json_root(value);
        release_isolated_json_cycles(unsafe { &*value });
        drop(unsafe { Box::from_raw(value) });
        HOST_ERROR.with(|slot| *slot.borrow_mut() = original_host_error);
    }
}

/// # Safety
///
/// `value` must be null or an owned pointer returned by `CString::into_raw`
/// or `thaw_arena::owned_string`. Use `NativeStr` to read embedded NUL bytes
/// before destroying the pointer.
#[no_mangle]
pub unsafe extern "C" fn thaw_cstring_destroy(value: *mut c_char) {
    if !value.is_null() {
        unsafe { thaw_arena::destroy_string(value) };
    }
}

/// Borrows an already-owned live handle for one synchronous comparison.
/// Unlike `thaw_json_handle_id`, this must not retain or query a Host lease:
/// compiled strict equality runs outside the callback-only HOST_OPERATIONS scope.
/// A quoted user property is not a trusted handle without WRAPPER_BRANDS.
///
/// # Safety
/// `value` must point to a live JSON value and remain live through the call.
#[no_mangle]
pub unsafe extern "C" fn thaw_json_borrowed_handle_id(value: *const Value) -> u64 {
    let Some(value) = (unsafe { value.as_ref() }) else { return 0; };
    if let Value::Host(lease) = value { return lease.handle; }
    if !is_branded_wrapper(value) { return 0; }
    if object_identity_key(value).is_some_and(|identity| {
        DETACHED_HANDLE_MARKERS.with(|markers| markers.borrow().contains(&identity))
    }) { return 0; }
    let Some(fields) = value.as_object() else { return 0; };
    if fields.len() != 1 { return 0; }
    fields.get(b"__thaw_js_handle_id__".as_slice())
        .and_then(Value::as_f64)
        .filter(|handle| handle.is_finite() && *handle > 0.0
            && handle.fract() == 0.0 && *handle <= u64::MAX as f64)
        .map(|handle| handle as u64)
        .unwrap_or(0)
}

/// Reads the internal dynamic-value handle marker without allocating a child JSON value.
///
/// # Safety
///
/// `value` must point to a valid JSON value.
#[no_mangle]
pub unsafe extern "C" fn thaw_json_handle_id(value: *const Value) -> u64 {
    if let Some(Value::Host(lease)) = unsafe { value.as_ref() } {
        // The compiled JsValue callback parameter has its own end-of-call
        // release; give it an independent reference from the Json lease.
        let handle = HOST_OPERATIONS.with(|slot| slot.get())
            .filter(|ops| without_typed_decode_scope(|| (ops.retain)(lease.handle)) != 0)
            .map(|_| lease.handle).unwrap_or(0);
        if handle != 0 {
            TYPED_DECODE_SCOPES.with(|scopes| {
                if let Some(scope) = scopes.borrow_mut().last_mut().and_then(Option::as_mut) {
                    scope.handles.push(handle);
                }
            });
        }
        return handle;
    }
    unsafe { value.as_ref() }
        .filter(|value| is_branded_wrapper(value))
        .and_then(Value::as_object)
        .filter(|fields| fields.len() == 1)
        .and_then(|fields| fields.get(b"__thaw_js_handle_id__".as_slice()))
        .and_then(Value::as_f64)
        .filter(|handle| handle.is_finite() && *handle > 0.0 && handle.fract() == 0.0)
        .map(|handle| handle as u64)
        .unwrap_or(0)
}

// Cycle-preserving transport for the function-replacer path. The envelope is
// private to the bridge: user property names are kept as pairs, never treated
// as control fields, and node indices refer only to this envelope's table.
#[no_mangle]
pub extern "C" fn thaw_json_brand_wrapper(value: *mut Value) -> *mut Value {
    if let Some(identity) = unsafe { value.as_ref() }.and_then(object_identity_key) {
        WRAPPER_BRANDS.with(|brands| { brands.borrow_mut().insert(identity); });
    }
    value
}

/// Brands a callback's top-level `Json` argument only when it is exactly the handle placeholder
/// the callback wrapper substitutes for a JS function, so a function stays callable through a
/// `Json` parameter. Any other value is returned unbranded.
#[no_mangle]
pub extern "C" fn thaw_json_brand_handle_placeholder(value: *mut Value) -> *mut Value {
    let is_placeholder = unsafe { value.as_ref() }
        .and_then(Value::as_object)
        .filter(|fields| fields.len() == 1)
        .and_then(|fields| fields.get(b"__thaw_js_handle_id__".as_slice()))
        .and_then(Value::as_f64)
        .is_some_and(|handle| handle.is_finite() && handle > 0.0 && handle.fract() == 0.0);
    if is_placeholder { thaw_json_brand_wrapper(value) } else { value }
}

#[no_mangle]
pub extern "C" fn thaw_json_brand_wrapper_if(value: *mut Value, trusted_origin: bool) -> *mut Value {
    if trusted_origin { thaw_json_brand_wrapper(value) } else { value }
}

fn is_branded_wrapper(value: &Value) -> bool {
    object_identity_key(value).is_some_and(|identity| {
        WRAPPER_BRANDS.with(|brands| brands.borrow().contains(&identity))
    })
}

fn is_graph_plain_object(value: &Value) -> bool {
    object_identity_key(value).is_some_and(|identity| {
        GRAPH_PLAIN_OBJECTS.with(|objects| objects.borrow().contains(&identity))
    })
}

fn mark_graph_plain_object(value: &Value) {
    if let Some(identity) = object_identity_key(value) {
        GRAPH_PLAIN_OBJECTS.with(|objects| { objects.borrow_mut().insert(identity); });
    }
}

fn brand_internal_value(value: Value) -> Value {
    if let Some(identity) = object_identity_key(&value) {
        WRAPPER_BRANDS.with(|brands| { brands.borrow_mut().insert(identity); });
    }
    value
}

fn write_graph_token(
    value: &Value,
    out: &mut Vec<u8>,
    ids: &mut HashMap<usize, usize>,
    nodes: &mut Vec<Value>,
) {
    if value.as_f64().is_some_and(|number| number.to_bits() == (-0.0f64).to_bits()) {
        out.extend_from_slice(b"{\"nf\":\"-0\"}");
    } else if is_branded_wrapper(value) && is_napi_undefined(value) {
        out.extend_from_slice(b"{\"u\":1}");
    } else if let Some(number) = is_branded_wrapper(value).then(|| non_finite_number(value)).flatten() {
        out.extend_from_slice(b"{\"nf\":");
        write_json_string(non_finite_display(number).as_bytes(), out);
        out.push(b'}');
    } else if let Value::BigInt(decimal) = value {
        out.extend_from_slice(b"{\"bi\":");
        write_json_string(decimal.as_bytes(), out);
        out.push(b'}');
    } else if let Some(identity) = object_identity_key(value) {
        let index = *ids.entry(identity).or_insert_with(|| {
            let index = nodes.len();
            nodes.push(value.clone());
            index
        });
        out.extend_from_slice(format!("{{\"r\":{index}}}").as_bytes());
    } else {
        out.extend_from_slice(b"{\"v\":");
        write_json_value(value, out, None, 0);
        out.push(b'}');
    }
}

#[no_mangle]
pub extern "C" fn thaw_json_graph_encode(value: *const Value) -> *const c_char {
    begin_stringify();
    let Some(value) = (unsafe { value.as_ref() }) else {
        let wire = CString::new("{\"root\":{\"v\":null},\"nodes\":[]}").unwrap().into_raw();
        if !unsafe { thaw_arena::register_owned_graph_wire(wire) } {
            unsafe { thaw_arena::destroy_string(wire) };
            return std::ptr::null();
        }
        return wire;
    };
    let mut ids = HashMap::new();
    let mut nodes = Vec::new();
    let mut output_leases = Vec::new();
    let mut napi_output_leases = Vec::new();
    let mut lease_error = false;
    let mut out = b"{\"root\":".to_vec();
    write_graph_token(value, &mut out, &mut ids, &mut nodes);
    out.extend_from_slice(b",\"nodes\":[");
    let mut index = 0;
    while index < nodes.len() {
        if index != 0 { out.push(b','); }
        let node = nodes[index].clone();
        if is_branded_wrapper(&node) {
            if let Some(handle) = node.as_object()
                .filter(|fields| fields.len() == 1)
                .and_then(|fields| fields.get(b"__thaw_js_handle_id__".as_slice()))
                .and_then(Value::as_f64)
                .filter(|handle| handle.is_finite() && *handle > 0.0 && handle.fract() == 0.0) {
                let handle = handle as u64;
                if HOST_OPERATIONS.with(|slot| slot.get())
                    .is_some_and(|ops| without_typed_decode_scope(|| (ops.retain)(handle)) != 0) {
                    output_leases.push(handle);
                    out.extend_from_slice(format!("{{\"hdl\":{handle}}}").as_bytes());
                } else {
                    lease_error = true;
                }
                index += 1;
                continue;
            }
        }
        if let Value::Object(fields) = &node {
            // The retain callback can re-enter graph destruction. Drop the
            // RefCell borrow before crossing that boundary.
            let napi_lease = fields.napi_lease.borrow().clone();
            if let Some(lease) = napi_lease.as_ref() {
                if let Some(reference) = NAPI_HANDLE_OPERATIONS.with(|slot| slot.get())
                    .map(|ops| without_typed_decode_scope(|| (ops.retain)(lease.handle))).filter(|reference| *reference != 0) {
                    napi_output_leases.push(reference);
                    out.extend_from_slice(format!("{{\"nh\":\"{}\"}}", lease.handle).as_bytes());
                } else {
                    lease_error = true;
                }
                index += 1;
                continue;
            }
        }
        if is_branded_wrapper(&node) {
            if let Some(fields) = node.as_object().filter(|fields| fields.len() == 2) {
                if fields.get(b"type".as_slice()).and_then(Value::as_str) == Some("Buffer") {
                    if let Some(bytes) = fields.get(b"data".as_slice()).and_then(Value::as_array)
                        .filter(|bytes| bytes.iter().all(|byte| byte.as_f64().is_some_and(
                            |number| number >= 0.0 && number <= 255.0 && number.fract() == 0.0))) {
                        out.extend_from_slice(b"{\"b\":[");
                        for (slot, byte) in bytes.iter().enumerate() {
                            if slot != 0 { out.push(b','); }
                            out.extend_from_slice(byte.as_f64().unwrap().to_string().as_bytes());
                        }
                        out.extend_from_slice(b"]}");
                        index += 1;
                        continue;
                    }
                }
            }
        }
        if is_branded_wrapper(&node) && date_iso_string(&node).is_some() {
            out.extend_from_slice(b"{\"d\":");
            let timestamp = thaw_json_date_timestamp(&node as *const Value);
            if timestamp.is_finite() {
                out.extend_from_slice(timestamp.to_string().as_bytes());
            } else {
                out.extend_from_slice(b"null");
            }
            out.push(b'}');
            index += 1;
            continue;
        }
        match &node {
            Value::Host(lease) => {
                if HOST_OPERATIONS.with(|slot| slot.get())
                    .is_some_and(|ops| without_typed_decode_scope(|| (ops.retain)(lease.handle)) != 0) {
                    output_leases.push(lease.handle);
                    out.extend_from_slice(format!("{{\"hdl\":{}}}", lease.handle).as_bytes());
                } else {
                    lease_error = true;
                }
            }
            Value::Array(items) => {
                out.extend_from_slice(b"{\"a\":[");
                let items_ref = shared_array_ref(items);
                for (slot, item) in items_ref.iter().enumerate() {
                    if slot != 0 { out.push(b','); }
                    if array_has_index(items, slot) {
                        write_graph_token(item, &mut out, &mut ids, &mut nodes);
                    } else {
                        out.extend_from_slice(b"{\"h\":1}");
                    }
                }
                out.extend_from_slice(b"]}");
            }
            Value::Object(fields) => {
                let fields_ref = shared_object_ref(fields);
                if is_branded_wrapper(&node) && fields_ref.len() == 1 {
                    if let Some(Value::Array(_)) = fields_ref.get(b"__thaw_map_entries__".as_slice()) {
                        out.extend_from_slice(b"{\"m\":");
                        write_graph_token(fields_ref.get(b"__thaw_map_entries__".as_slice()).unwrap(), &mut out, &mut ids, &mut nodes);
                        out.push(b'}');
                        index += 1;
                        continue;
                    }
                    if let Some(Value::Array(_)) = fields_ref.get(b"__thaw_set_values__".as_slice()) {
                        out.extend_from_slice(b"{\"s\":");
                        write_graph_token(fields_ref.get(b"__thaw_set_values__".as_slice()).unwrap(), &mut out, &mut ids, &mut nodes);
                        out.push(b'}');
                        index += 1;
                        continue;
                    }
                    if let Some(Value::Object(pattern)) = fields_ref.get(b"__thaw_regexp__".as_slice()) {
                        let pattern = shared_object_ref(pattern);
                        if let Some(source @ (Value::String(_) | Value::Wtf8(_))) = pattern.get(b"source".as_slice()) {
                            out.extend_from_slice(b"{\"re\":[");
                            write_json_value(source, &mut out, None, 0);
                            out.push(b',');
                            if let Some(flags @ (Value::String(_) | Value::Wtf8(_))) = pattern.get(b"flags".as_slice()) {
                                write_json_value(flags, &mut out, None, 0);
                            } else {
                                out.extend_from_slice(b"\"\"");
                            }
                            out.push(b',');
                            match pattern.get(b"lastIndex".as_slice()) {
                                Some(number @ Value::Number(_)) => write_json_value(number, &mut out, None, 0),
                                Some(value) if non_finite_number(value).is_some() => {
                                    write_json_string(non_finite_display(non_finite_number(value).unwrap()).as_bytes(), &mut out);
                                }
                                _ => out.push(b'0'),
                            }
                            out.extend_from_slice(b"]}");
                            index += 1;
                            continue;
                        }
                    }
                }
                out.extend_from_slice(b"{\"o\":[");
                for (slot, (key, item)) in ordered_object_fields_shared(fields).into_iter().enumerate() {
                    if slot != 0 { out.push(b','); }
                    out.push(b'[');
                    write_json_string(key, &mut out);
                    out.push(b',');
                    write_graph_token(item, &mut out, &mut ids, &mut nodes);
                    out.push(b']');
                }
                out.extend_from_slice(b"]}");
            }
            _ => unreachable!("graph node is not an object or array"),
        }
        index += 1;
    }
    // A host property/metadata query can fail after earlier live nodes were
    // retained. No decoder receives this wire on that path, so release those
    // provisional transfers here; the caller still consumes HOST_ERROR.
    let host_failed = HOST_ERROR.with(|slot| slot.borrow().is_some());
    if lease_error || host_failed {
        if let Some(ops) = HOST_OPERATIONS.with(|slot| slot.get()) {
            for handle in output_leases { without_typed_decode_scope(|| (ops.release)(handle)); }
        }
        if let Some(ops) = NAPI_HANDLE_OPERATIONS.with(|slot| slot.get()) {
            for reference in napi_output_leases { without_typed_decode_scope(|| (ops.release)(reference)); }
        }
        if lease_error {
            set_host_error("\u{1}TypeError\u{1}Unable to retain host JSON value".into());
            return CString::new("\u{2}TypeError:Unable to retain host JSON value")
                .unwrap().into_raw();
        }
        return CString::new("").unwrap().into_raw();
    }
    out.extend_from_slice(b"],\"leases\":[");
    for (index, handle) in output_leases.iter().enumerate() {
        if index != 0 { out.push(b','); }
        out.extend_from_slice(handle.to_string().as_bytes());
    }
    out.extend_from_slice(b"],\"napiLeases\":[");
    for (index, reference) in napi_output_leases.iter().enumerate() {
        if index != 0 { out.push(b','); }
        write_json_string(reference.to_string().as_bytes(), &mut out);
    }
    out.extend_from_slice(b"]}");
    let wire = CString::new(out).expect("JSON graph envelope contains no raw NUL").into_raw();
    if !unsafe { thaw_arena::register_owned_graph_wire(wire) } {
        if let Some(ops) = HOST_OPERATIONS.with(|slot| slot.get()) {
            for handle in output_leases { without_typed_decode_scope(|| (ops.release)(handle)); }
        }
        if let Some(ops) = NAPI_HANDLE_OPERATIONS.with(|slot| slot.get()) {
            for reference in napi_output_leases { without_typed_decode_scope(|| (ops.release)(reference)); }
        }
        unsafe { thaw_arena::destroy_string(wire) };
        set_host_error("\u{1}TypeError\u{1}Unable to transfer JSON graph".into());
        return std::ptr::null();
    }
    wire
}

fn graph_field<'a>(value: &'a Value, key: &[u8]) -> Option<&'a Value> {
    value.as_object()?.get(key)
}

fn napi_wire_lease_tokens(graph: &Value) -> Vec<u64> {
    graph_field(graph, b"napiLeases").and_then(Value::as_array)
        .map(|tokens| tokens.iter().filter_map(Value::as_str)
            .filter_map(|token| token.parse::<u64>().ok())
            .filter(|token| *token != 0).collect())
        .unwrap_or_default()
}

fn parse_graph_grant_snapshot(bytes: &[u8]) -> *mut Value {
    let source = thaw_arena::owned_string(bytes);
    let graph = thaw_json_parse(source);
    unsafe { thaw_arena::destroy_string(source) };
    graph
}

fn graph_token_value(token: &Value, nodes: &[Value]) -> Option<Value> {
    let fields = token.as_object()?;
    // `{ u: 1, fn: name }`: a function in a compiled result, which is JSON data (undefined).
    if fields.len() == 2 && fields.get(b"u".as_slice()).and_then(Value::as_f64) == Some(1.0) {
        return Some(function_sentinel_value(fields.get(b"fn".as_slice())?.as_str()?));
    }
    if fields.len() != 1 { return None; }
    if let Some(index) = fields.get(b"r".as_slice()).and_then(Value::as_f64) {
        if index < 0.0 || index.fract() != 0.0 { return None; }
        return nodes.get(index as usize).cloned();
    }
    if let Some(decimal) = fields.get(b"bi".as_slice()).and_then(Value::as_str) {
        return canonical_bigint_decimal(decimal).then(|| Value::BigInt(decimal.to_string()));
    }
    if let Some(value) = fields.get(b"v".as_slice()) {
        return matches!(value, Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) | Value::Wtf8(_))
            .then(|| value.clone());
    }
    if fields.get(b"u".as_slice()).and_then(Value::as_f64) == Some(1.0) {
        return Some(napi_undefined_value());
    }
    let non_finite = fields.get(b"nf".as_slice())?.as_str()?;
    let number = match non_finite {
        "-0" => return Some(number_value(-0.0)),
        "NaN" => f64::NAN,
        "Infinity" => f64::INFINITY,
        "-Infinity" => f64::NEG_INFINITY,
        _ => return None,
    };
    Some(napi_non_finite_value(number))
}

fn clear_unreachable_graph_nodes(nodes: &[Value], root: Option<&Value>) {
    let indices = nodes.iter().enumerate().filter_map(|(index, node)|
        object_identity_key(node).map(|identity| (identity, index)))
        .collect::<HashMap<_, _>>();
    let mut reachable = HashSet::new();
    let mut visited = HashSet::new();
    let mut pending = root.cloned().into_iter().collect::<Vec<_>>();
    while let Some(value) = pending.pop() {
        let Some(identity) = object_identity_key(&value) else { continue };
        if !visited.insert(identity) { continue; }
        if let Some(index) = indices.get(&identity) { reachable.insert(*index); }
        match &value {
            Value::Array(items) => pending.extend(shared_array_ref(items).iter().cloned()),
            Value::Object(fields) => pending.extend(shared_object_ref(fields).values().cloned()),
            _ => {}
        }
    }
    // These nodes were allocated only for this decode. On failure none
    // escaped; on success only nodes unreachable from the returned root
    // are cleared, so a returned alias keeps every edge it can observe.
    for (index, node) in nodes.iter().enumerate() {
        if reachable.contains(&index) { continue; }
        match node {
            Value::Array(items) => shared_array_ref_mut(items).clear(),
            Value::Object(fields) => shared_object_ref_mut(fields).clear(),
            _ => {}
        }
    }
}

fn decode_graph_value(graph: &Value) -> Option<Value> {
    let descriptions = graph_field(graph, b"nodes")?.as_array()?;
    let mut nodes = Vec::with_capacity(descriptions.len());
    for description in descriptions {
        let fields = description.as_object()?;
        // An array node may carry its own property descriptors (`p`): the JS encoder
        // lists every index and `length`. A native array stores only the elements, so
        // those standard entries are redundant; any other own property cannot be kept.
        let array_with_properties = fields.len() == 2 && fields.get(b"a".as_slice()).is_some_and(Value::is_array)
            && fields.get(b"p".as_slice()).and_then(Value::as_array).is_some_and(|properties| properties.iter().all(|entry| {
                let Some([key, _, _]) = entry.as_array().map(Vec::as_slice) else { return false };
                match key {
                    Value::String(text) => text == "length" || text.parse::<usize>().is_ok_and(|index| index.to_string() == *text),
                    _ => false,
                }
            }));
        if fields.len() != 1 && !array_with_properties { return None; }
        let node = if let Some(items) = fields.get(b"a".as_slice()).and_then(Value::as_array) {
            Value::shared_array(Vec::with_capacity(items.len()))
        } else if let Some(entries) = fields.get(b"o".as_slice()).and_then(Value::as_array) {
            let plain = Value::shared_object(indexmap::IndexMap::with_capacity(entries.len()));
            mark_graph_plain_object(&plain);
            plain
        } else if let Some(timestamp) = fields.get(b"d".as_slice()) {
            if !timestamp.is_null() && timestamp.as_f64().is_none() { return None; }
            let timestamp = if timestamp.is_null() { napi_non_finite_value(f64::NAN) }
                else { timestamp.clone() };
            brand_internal_value(Value::shared_object(indexmap::IndexMap::from([(b"timestamp".to_vec(), timestamp)])))
        } else if let Some(bytes) = fields.get(b"b".as_slice()).and_then(Value::as_array) {
            if !bytes.iter().all(|byte| byte.as_f64().is_some_and(|number| number >= 0.0 && number <= 255.0 && number.fract() == 0.0)) { return None; }
            brand_internal_value(Value::shared_object(indexmap::IndexMap::from([
                (b"type".to_vec(), Value::String("Buffer".to_string())),
                (b"data".to_vec(), Value::shared_array(bytes.to_vec())),
            ])))
        } else if let Some(handle) = fields.get(b"nh".as_slice()).and_then(Value::as_str) {
            let handle = handle.parse::<u64>().ok()?;
            if handle == 0 { return None; }
            let ops = NAPI_HANDLE_OPERATIONS.with(|slot| slot.get())?;
            let reference = without_typed_decode_scope(|| (ops.retain)(handle));
            if reference == 0 { return None; }
            let lease = Rc::new(NapiLease { handle, reference, release: ops.release });
            brand_internal_value(Value::shared_object_with_lease(indexmap::IndexMap::from([
                (b"__thaw_napi_handle__".to_vec(), Value::String(handle.to_string())),
            ]), Some(lease)))
        } else if let Some(handle) = fields.get(b"hdl".as_slice()).and_then(Value::as_f64) {
            if !handle.is_finite() || handle <= 0.0 || handle.fract() != 0.0 { return None; }
            if HOST_OPERATIONS.with(|slot| slot.get()).is_some() {
                host_value_from_borrowed_handle(handle as u64)?
            } else {
                let marker = Value::shared_object(indexmap::IndexMap::from([
                    (b"__thaw_js_handle_id__".to_vec(), Value::Number(serde_json::Number::from_f64(handle)?)),
                ]));
                if let Some(identity) = object_identity_key(&marker) {
                    DETACHED_HANDLE_MARKERS.with(|markers| { markers.borrow_mut().insert(identity); });
                }
                marker
            }
        } else if fields.contains_key(b"m".as_slice())
            || fields.contains_key(b"s".as_slice())
            || fields.contains_key(b"re".as_slice()) {
            Value::shared_object(indexmap::IndexMap::new())
        } else { return None };
        nodes.push(node);
    }
    let decoded = (|| -> Option<Value> {
    for (description, node) in descriptions.iter().zip(&nodes) {
        if let Some(items) = graph_field(description, b"a").and_then(Value::as_array) {
            let Value::Array(array) = node else { return None };
            let mut holes = Vec::new();
            for (slot, item) in items.iter().enumerate() {
                if item.as_object().is_some_and(|fields| fields.len() == 1)
                    && graph_field(item, b"h").and_then(Value::as_f64) == Some(1.0) {
                    shared_array_ref_mut(array).push(Value::Null);
                    holes.push(slot);
                } else {
                    shared_array_ref_mut(array).push(graph_token_value(item, &nodes)?);
                }
            }
            mark_array_holes(array, holes);
        } else if let Some(entries) = graph_field(description, b"o").and_then(Value::as_array) {
            let Value::Object(object) = node else { return None };
            for entry in entries {
                // The JS encoder appends the property's attribute flags as a third element.
                let (key, token) = match entry.as_array()?.as_slice() {
                    [key, token] | [key, token, _] => (key, token),
                    _ => return None,
                };
                let bytes = match key { Value::String(text) => text.as_bytes(), Value::Wtf8(bytes) => bytes, _ => return None };
                shared_object_ref_mut(object).insert(canonical_key(bytes), graph_token_value(token, &nodes)?);
            }
        }
    }
    for (description, node) in descriptions.iter().zip(&nodes) {
        let Value::Object(object) = node else { continue };
        if let Some(token) = graph_field(description, b"m") {
            let entries = graph_token_value(token, &nodes)?;
            if !entries.is_array() { return None; }
            shared_object_ref_mut(object).insert(b"__thaw_map_entries__".to_vec(), entries);
        } else if let Some(token) = graph_field(description, b"s") {
            let values = graph_token_value(token, &nodes)?;
            if !values.is_array() { return None; }
            shared_object_ref_mut(object).insert(b"__thaw_set_values__".to_vec(), values);
        } else if let Some(parts) = graph_field(description, b"re").and_then(Value::as_array) {
            let [source, flags, last_index] = parts.as_slice() else { return None };
            if !matches!(source, Value::String(_) | Value::Wtf8(_))
                || !matches!(flags, Value::String(_) | Value::Wtf8(_)) {
                return None;
            }
            let last_index = match last_index {
                Value::Number(_) => last_index.clone(),
                Value::String(text) => match text.as_str() {
                    "NaN" => number_value(f64::NAN),
                    "Infinity" => number_value(f64::INFINITY),
                    "-Infinity" => number_value(f64::NEG_INFINITY),
                    _ => return None,
                },
                _ => return None,
            };
            let mut inner = indexmap::IndexMap::new();
            inner.insert(b"source".to_vec(), source.clone());
            inner.insert(b"flags".to_vec(), flags.clone());
            inner.insert(b"lastIndex".to_vec(), last_index);
            shared_object_ref_mut(object).insert(b"__thaw_regexp__".to_vec(), Value::shared_object(inner));
        }
        if graph_field(description, b"m").is_some()
            || graph_field(description, b"s").is_some()
            || graph_field(description, b"re").is_some()
            || graph_field(description, b"hdl").is_some()
            || graph_field(description, b"b").is_some() {
            if let Some(identity) = object_identity_key(node) {
                WRAPPER_BRANDS.with(|brands| { brands.borrow_mut().insert(identity); });
            }
        }
    }
    graph_token_value(graph_field(graph, b"root")?, &nodes)
    })();
    clear_unreachable_graph_nodes(&nodes, decoded.as_ref());
    decoded
}

#[no_mangle]
pub extern "C" fn thaw_json_take_graph_error() -> u8 {
    GRAPH_ERROR.with(|error| u8::from(error.replace(false)))
}

/// Returns the transferred host leases in a graph wire that was never sent
/// to a decoder. This is used only on producer-side failures after encoding.
/// The decoder remains the sole owner once dispatch begins.
#[no_mangle]
pub extern "C" fn thaw_json_discard_graph_wire(source: *const c_char) {
    if source.is_null() { return; }
    // Consume before parsing or releasing: callbacks may re-enter with the
    // same pointer, but only this invocation owns the one-shot grant. Parse
    // the producer snapshot so a mutated live buffer cannot redirect cleanup
    // to attacker-selected lease IDs or strand the original transfer.
    let Some((bytes, _bytes_match)) = (unsafe {
        thaw_arena::take_owned_graph_wire_snapshot(source)
    }) else { return; };
    // A producer already failed before dispatch. Release callbacks for its
    // unconsumed wire must not replace that error or poison a later call.
    let original_host_error = HOST_ERROR.with(|slot| slot.borrow_mut().take());
    let graph = parse_graph_grant_snapshot(&bytes);
    let parse_error = thaw_json_take_parse_error() != 0;
    if !parse_error {
        if let Some(ops) = HOST_OPERATIONS.with(|slot| slot.get()) {
            if let Some(handles) = graph_field(unsafe { &*graph }, b"leases").and_then(Value::as_array) {
                for handle in handles.iter().filter_map(Value::as_f64)
                    .filter(|handle| handle.is_finite() && *handle > 0.0
                        && *handle <= 9_007_199_254_740_991.0 && handle.fract() == 0.0) {
                    without_typed_decode_scope(|| (ops.release)(handle as u64));
                }
            }
        }
        if let Some(ops) = NAPI_HANDLE_OPERATIONS.with(|slot| slot.get()) {
            for reference in napi_wire_lease_tokens(unsafe { &*graph }) {
                without_typed_decode_scope(|| (ops.release)(reference));
            }
        }
    }
    unsafe { thaw_json_destroy(graph) };
    HOST_ERROR.with(|slot| *slot.borrow_mut() = original_host_error);
}

#[no_mangle]
pub extern "C" fn thaw_json_graph_decode(source: *const c_char) -> *mut Value {
    GRAPH_ERROR.with(|error| error.set(false));
    if source.is_null() {
        GRAPH_ERROR.with(|error| error.set(true));
        return leak(Value::Null);
    }
    let grant = unsafe { thaw_arena::take_owned_graph_wire_snapshot(source) };
    let bytes_match = grant.as_ref().is_some_and(|(_, bytes_match)| *bytes_match);
    let graph = match grant.as_ref() {
        Some((bytes, _)) => parse_graph_grant_snapshot(bytes),
        None => thaw_json_parse(source),
    };
    let parse_error = thaw_json_take_parse_error() != 0;
    let leases = if parse_error { Vec::new() } else {
        graph_field(unsafe { &*graph }, b"leases")
            .and_then(Value::as_array)
            .map(|handles| handles.iter().filter_map(Value::as_f64)
                .filter(|handle| handle.is_finite() && *handle > 0.0
                    && *handle <= 9_007_199_254_740_991.0 && handle.fract() == 0.0)
                .map(|handle| handle as u64).collect::<Vec<_>>())
            .unwrap_or_default()
    };
    let napi_leases = if parse_error { Vec::new() }
        else { napi_wire_lease_tokens(unsafe { &*graph }) };
    let decoded = if parse_error
        || (grant.is_some() && !bytes_match)
        || (grant.is_none() && (!leases.is_empty() || !napi_leases.is_empty())) {
        None
    } else { decode_graph_value(unsafe { &*graph }) };
    // The JS encoder owns these initial references. A decoded Host node
    // independently retains its handle, including when it escapes the call.
    if grant.is_some() {
      if let Some(ops) = HOST_OPERATIONS.with(|slot| slot.get()) {
        for handle in leases { without_typed_decode_scope(|| (ops.release)(handle)); }
      }
      if let Some(ops) = NAPI_HANDLE_OPERATIONS.with(|slot| slot.get()) {
        for reference in napi_leases { without_typed_decode_scope(|| (ops.release)(reference)); }
      }
    }
    unsafe { thaw_json_destroy(graph) };
    match decoded {
        Some(value) => leak(value),
        None => {
            GRAPH_ERROR.with(|error| error.set(true));
            leak(Value::Null)
        }
    }
}

#[cfg(test)]
#[test]
fn unregistered_graph_wire_cannot_transfer_napi_lease_tokens() {
    std::thread::spawn(|| {
        thaw_json_register_napi_handle_operations(
            mutated_graph_retain, mutated_graph_release,
        );
        let wire = CString::new(
            r#"{"root":{"v":null},"nodes":[],"leases":[],"napiLeases":["73"]}"#,
        ).unwrap();
        let decoded = thaw_json_graph_decode(wire.as_ptr());
        assert_eq!(thaw_json_take_graph_error(), 1);
        MUTATED_GRAPH_RELEASES_73.with(|count| assert_eq!(count.get(), 0));
        MUTATED_GRAPH_RELEASES_79.with(|count| assert_eq!(count.get(), 0));
        unsafe { thaw_json_destroy(decoded) };
    }).join().unwrap();
}

#[cfg(test)]
thread_local! {
    static MUTATED_GRAPH_RELEASES_73: std::cell::Cell<u8> = const {
        std::cell::Cell::new(0)
    };
    static MUTATED_GRAPH_RELEASES_79: std::cell::Cell<u8> = const {
        std::cell::Cell::new(0)
    };
}

#[cfg(test)]
extern "C" fn mutated_graph_retain(reference: u64) -> u64 { reference }

#[cfg(test)]
extern "C" fn mutated_graph_release(reference: u64) -> u8 {
    match reference {
        73 => MUTATED_GRAPH_RELEASES_73.with(|count| count.set(count.get() + 1)),
        79 => MUTATED_GRAPH_RELEASES_79.with(|count| count.set(count.get() + 1)),
        _ => {}
    }
    u8::from(matches!(reference, 73 | 79))
}

#[cfg(test)]
#[test]
fn mutated_graph_wire_retires_only_registered_snapshot_leases() {
    std::thread::spawn(|| {
        thaw_json_register_napi_handle_operations(
            mutated_graph_retain, mutated_graph_release,
        );
        let wire = thaw_arena::owned_string(
            br#"{"root":{"v":null},"nodes":[],"leases":[],"napiLeases":["73"]}"#,
        );
        assert!(unsafe { thaw_arena::register_owned_graph_wire(wire) });
        let offset = unsafe { CStr::from_ptr(wire) }.to_bytes()
            .windows(2).position(|pair| pair == b"73").unwrap();
        unsafe { wire.cast::<u8>().add(offset + 1).write(b'9') };

        thaw_json_discard_graph_wire(wire);
        MUTATED_GRAPH_RELEASES_73.with(|count| assert_eq!(count.get(), 1));
        MUTATED_GRAPH_RELEASES_79.with(|count| assert_eq!(count.get(), 0));
        thaw_json_discard_graph_wire(wire);
        MUTATED_GRAPH_RELEASES_73.with(|count| assert_eq!(count.get(), 1));
        MUTATED_GRAPH_RELEASES_79.with(|count| assert_eq!(count.get(), 0));
        unsafe { thaw_arena::destroy_string(wire) };

        let wire = thaw_arena::owned_string(
            br#"{"root":{"v":null},"nodes":[],"leases":[],"napiLeases":["73"]}"#,
        );
        assert!(unsafe { thaw_arena::register_owned_graph_wire(wire) });
        let offset = unsafe { CStr::from_ptr(wire) }.to_bytes()
            .windows(2).position(|pair| pair == b"73").unwrap();
        unsafe { wire.cast::<u8>().add(offset + 1).write(b'9') };
        let decoded = thaw_json_graph_decode(wire);
        assert_eq!(thaw_json_take_graph_error(), 1);
        unsafe { thaw_json_destroy(decoded) };
        MUTATED_GRAPH_RELEASES_73.with(|count| assert_eq!(count.get(), 2));
        MUTATED_GRAPH_RELEASES_79.with(|count| assert_eq!(count.get(), 0));
        unsafe { thaw_arena::destroy_string(wire) };
    }).join().unwrap();
}

#[no_mangle]
pub extern "C" fn thaw_json_stringify(value: *mut Value) -> *const c_char {
    begin_stringify();
    let value = unsafe { &*value };
    stringify_value(&ordered_json(value), &[])
}

/// Real `JSON.stringify(x)` returns the actual JS value `undefined`
/// (not a string) when `x` itself -- the top-level argument, not a
/// nested field/element -- is `undefined`; `typeof JSON.stringify(x)
/// === 'undefined'`, and `JSON.stringify(x) === undefined`. Thaw's own
/// compiled calling convention always returns a real `Str` here (the
/// same practical-subset choice TypeScript's own `lib.d.ts` already
/// makes for `JSON.stringify`'s declared return type -- `string`, never
/// `string | undefined`, even though this exact case makes that
/// signature unsound in real Node too), so there's no way to return a
/// genuine non-string value through this ABI. Returning the *string*
/// `"undefined"` is the closest honest approximation: it matches real
/// JS's own `String(undefined)` coercion, so the overwhelmingly common
/// usage patterns (`` `${JSON.stringify(x)}` ``, string concatenation,
/// `console.log`) read identically to real Node; a narrower,
/// defensive-only check like `JSON.stringify(x) === undefined` or
/// `typeof JSON.stringify(x) === 'undefined'` is the one thing this
/// doesn't reproduce -- a documented, narrow, honest simplification,
/// not a silent miscalculation.
fn top_level_undefined_string() -> *const c_char {
    CString::new("undefined").unwrap_or_default().into_raw()
}

/// The user-facing `JSON.stringify(value)` (no replacer/space) entry
/// point -- distinct from `thaw_json_stringify` precisely because that
/// one is *also* used internally for argument/result marshaling and
/// can't safely gain sentinel-omitting behavior (see its doc comment).
/// This one omits/nulls a nested napi-undefined sentinel the way real
/// `JSON.stringify` treats a real `undefined`, and special-cases the
/// top-level sentinel itself the same way (see `top_level_undefined_
/// string`'s own doc comment).
#[no_mangle]
pub extern "C" fn thaw_json_stringify_public(value: *mut Value) -> *const c_char {
    begin_stringify();
    let value = unsafe { &*value };
    if is_napi_undefined(value) {
        return top_level_undefined_string();
    }
    stringify_value(&ordered_json_omitting_undefined(value), &[])
}

#[no_mangle]
/// Prefixes a native callback exception so the JavaScript callback wrapper can
/// distinguish it from a successful JSON result.
///
/// # Safety
///
/// `message` must point to a valid NUL-terminated string.
pub unsafe extern "C" fn thaw_json_callback_error(message: *const c_char) -> *const c_char {
    CString::new(format!("\u{2}{}", to_str(message)))
        .unwrap_or_default()
        .into_raw()
}

/// `String(NaN)`/`String(Infinity)`/`String(-Infinity)`, and the same
/// text real Node's `console.log`/template-literal coercion uses for
/// them -- shared by `thaw_json_console_string` and `thaw_json_as_string`
/// so a `NaN`/`Infinity` `any`-typed value reads the same in either.
fn non_finite_display(value: f64) -> &'static str {
    if value.is_nan() {
        "NaN"
    } else if value > 0.0 {
        "Infinity"
    } else {
        "-Infinity"
    }
}

/// Converts a `Json` value into the tree `crate::inspect` formats, recognising the same
/// wrapper shapes (`undefined`, non-finite numbers, RegExp, Date, Map, Set, Buffer) the
/// old console formatter did. `stack` holds the containers being converted, so a cycle
/// becomes a `Circular` back reference instead of an infinite walk.
thread_local! { static HOST_INSPECT_DEPTH: Cell<usize> = const { Cell::new(0) }; }

/// A live host value for `util.inspect`: arrays, Dates and plain objects are read live
/// (children recurse); everything else (functions, Errors, Map/Set, ...) keeps the host's
/// own `String(value)` text.
// ponytail: a class instance prints as a plain object (no class name query op exists).
fn host_inspect_node(lease: &Rc<HostLease>, stack: &mut Vec<usize>) -> crate::inspect::Node {
    use crate::inspect::{Ctor, Node};
    let raw = || Node::Raw(host_query(lease, 3).unwrap_or_default());
    if host_query(lease, 0).as_deref() != Some("object") { return raw(); }
    if host_query(lease, 12).as_deref() == Some("1") {
        let text = host_query(lease, 13).and_then(|time| time.parse::<f64>().ok())
            .map(|time| unsafe { thaw_date_to_iso_string(time) });
        return Node::Date(text.filter(|text| !text.is_null()).map(to_str));
    }
    let array = host_query(lease, 4).as_deref() == Some("1");
    // Materialization precedes the formatter's own depth cut-off, so cap it (also ends cycles).
    if HOST_INSPECT_DEPTH.with(Cell::get) >= 4 {
        return Node::Raw(if array { "[Array]" } else { "[Object]" }.to_string());
    }
    if !array && host_query(lease, 3).as_deref() != Some("\"[object Object]\"") { return raw(); }
    HOST_INSPECT_DEPTH.with(|depth| depth.set(depth.get() + 1));
    let node = if array {
        let items = host_enumerate_values(lease, 3).iter().map(|item| Some(inspect_node(item, stack))).collect();
        Node::Array { id: 0, items, extra: Vec::new() }
    } else {
        let entries = host_enumerable_entries(lease).into_iter()
            .map(|(key, value)| (wtf8_decode_utf16(&key), inspect_node(&value, stack))).collect();
        Node::Object { id: 0, ctor: Ctor::Plain, entries }
    };
    HOST_INSPECT_DEPTH.with(|depth| depth.set(depth.get() - 1));
    node
}

fn inspect_node(value: &Value, stack: &mut Vec<usize>) -> crate::inspect::Node {
    use crate::inspect::{Ctor, Node};
    if is_napi_undefined(value) {
        return match sentinel_function_name(value) {
            Some(name) => Node::Function { name, kind: crate::inspect::FnKind::Function },
            None => Node::Undefined,
        };
    }
    if let Some(number) = non_finite_number(value) {
        return Node::Number(number);
    }
    if let Some(source) = regexp_wrapper_property(value, "source") {
        let flags = regexp_wrapper_property(value, "flags").and_then(|flags| flags.as_str().map(str::to_string)).unwrap_or_default();
        return Node::RegExp(format!("/{}/{}", source.as_str().unwrap_or_default(), flags));
    }
    if let Some(iso) = date_iso_string(value) {
        return Node::Date(iso);
    }
    if let Value::Host(lease) = value {
        return host_inspect_node(lease, stack);
    }
    if thaw_json_is_buffer_shape(value as *const Value) == 1 {
        let bytes = value.as_object()
            .and_then(|fields| fields.get(b"data".as_slice()))
            .and_then(Value::as_array)
            .map(|items| items.iter().map(|item| item.as_f64().unwrap_or(0.0) as u8).collect())
            .unwrap_or_default();
        return Node::Buffer(bytes);
    }
    let id = object_identity_key(value).unwrap_or(0);
    if id != 0 && matches!(value, Value::Array(_) | Value::Object(_)) {
        if stack.contains(&id) {
            return Node::Circular(id);
        }
        stack.push(id);
    }
    let node = match value {
        Value::Null => Node::Null,
        Value::Bool(value) => Node::Bool(*value),
        Value::Number(number) => Node::Number(number.as_f64().unwrap_or(f64::NAN)),
        Value::BigInt(text) => Node::BigInt(text.clone()),
        Value::String(text) => tagged_error_node(text.as_bytes())
            .unwrap_or_else(|| Node::Str(text.encode_utf16().collect())),
        Value::Wtf8(bytes) => tagged_error_node(bytes)
            .unwrap_or_else(|| Node::Str(wtf8_decode_utf16(bytes))),
        Value::Array(items) => {
            let items = shared_array_ref(items);
            let array = match value { Value::Array(shared) => shared, _ => unreachable!() };
            let converted = items
                .iter()
                .enumerate()
                .map(|(index, item)| array_has_index(array, index).then(|| inspect_node(item, stack)))
                .collect();
            Node::Array { id, items: converted, extra: Vec::new() }
        }
        Value::Object(fields) => {
            let fields = shared_object_ref(fields);
            if let Some(entries) = fields.get(b"__thaw_map_entries__".as_slice()).and_then(Value::as_array) {
                let entries = entries
                    .iter()
                    .filter_map(|entry| {
                        let [key, value] = entry.as_array()?.as_slice() else { return None };
                        Some((inspect_node(key, stack), inspect_node(value, stack)))
                    })
                    .collect();
                Node::Map { id, entries }
            } else if let Some(values) = fields.get(b"__thaw_set_values__".as_slice()).and_then(Value::as_array) {
                Node::Set { id, items: values.iter().map(|item| inspect_node(item, stack)).collect() }
            } else {
                // console.log of a typed class instance carries its most-derived class name
                // under a private key (see `compile_native_object_to_json_with_undefined`).
                let class = fields.get(b"__thaw_class__".as_slice()).and_then(Value::as_str).map(str::to_string);
                let field = |name: &str| fields.get(name.as_bytes()).and_then(Value::as_str).map(str::to_string);
                // A caught `AggregateError` crosses as `{ message, name, errors, .. }`
                // (see `lower_new_aggregate_error`); print it as Node prints an error with no
                // stack frames: `[AggregateError: msg] { [errors]: [ .. ] }`.
                if field("name").as_deref() == Some("AggregateError") && field("message").is_some()
                    && fields.get(b"errors".as_slice()).is_some_and(|errors| errors.as_array().is_some())
                {
                    let header = error_header("AggregateError", &field("message").unwrap_or_default());
                    let entries: Vec<(Vec<u16>, Node)> = ordered_object_fields(fields)
                        .into_iter()
                        .filter(|(key, _)| ![b"message".as_slice(), b"name", b"__thaw_class__"].contains(&&key[..])
                            && !key.starts_with(b"__thaw_class_identity_"))
                        .map(|(key, value)| {
                            // `errors`/`cause` are non-enumerable: Node shows them as `[errors]`.
                            let label: Vec<u16> = if key == b"errors".as_slice() || key == b"cause".as_slice() {
                                format!("\u{1}[{}]", String::from_utf8_lossy(key)).encode_utf16().collect()
                            } else { wtf8_decode_utf16(key) };
                            (label, inspect_node(value, stack))
                        })
                        .collect();
                    if id != 0 { stack.pop(); }
                    return Node::Object { id, ctor: Ctor::Named(header), entries };
                }
                if class.as_deref() == Some("RegExp") {
                    if let (Some(source), Some(flags)) = (field("source"), field("flags")) {
                        if id != 0 { stack.pop(); }
                        return Node::RegExp(format!("/{source}/{flags}"));
                    }
                }
                if class.as_deref() == Some("Date") {
                    if let Some(timestamp) = fields.get(b"timestamp".as_slice()).and_then(Value::as_f64) {
                        if id != 0 { stack.pop(); }
                        let text = unsafe { thaw_date_to_iso_string(timestamp) };
                        return Node::Date((!text.is_null()).then(|| to_str(text)));
                    }
                }
                let entries = ordered_object_fields(fields)
                    .into_iter()
                    .filter(|(key, _)| *key != b"__thaw_class__".as_slice())
                    .map(|(key, value)| (wtf8_decode_utf16(key), inspect_node(value, stack)))
                    .collect();
                let ctor = match class {
                    // module-scoped classes are mangled `__thawmod<N>_Name`
                    Some(name) if name != "Object" => Ctor::Named(match name.strip_prefix("__thawmod") {
                        Some(rest) => rest.split_once('_').map_or(name.clone(), |(_, plain)| plain.to_string()),
                        None => name,
                    }),
                    _ => Ctor::Plain,
                };
                Node::Object { id, ctor, entries }
            }
        }
        Value::Host(_) => unreachable!("handled above"),
    };
    if id != 0 && matches!(value, Value::Array(_) | Value::Object(_)) {
        stack.pop();
    }
    node
}

/// Node's rendering of an error with no stack frames: `[Name: message]` (`[Name]` when empty).
fn error_header(name: &str, message: &str) -> String {
    if message.is_empty() { format!("[{name}]") } else { format!("[{name}: {message}]") }
}

/// A thrown `Error` carried as a tagged string (`\u{1}Name\u{1}..` frame) nested inside a value.
fn tagged_error_node(bytes: &[u8]) -> Option<crate::inspect::Node> {
    let frame = thaw_arena::error_wire::parse_tagged(bytes)?;
    let name = frame.chain.split(|byte| *byte == 0x1f).next().unwrap_or(b"Error");
    Some(crate::inspect::Node::Raw(error_header(&String::from_utf8_lossy(name), &String::from_utf8_lossy(frame.display))))
}

/// `util.inspect(value)` with Node's default options.
fn inspect_json(value: &Value) -> String {
    crate::inspect::inspect(&inspect_node(value, &mut Vec::new()), &crate::inspect::InspectOptions::default())
}

/// `util.inspect(value)`: strings are quoted, unlike `console.log`'s bare top-level string.
///
/// # Safety
/// `value` must point to a valid JSON `Value`.
#[no_mangle]
pub unsafe extern "C" fn thaw_json_inspect(value: *const Value) -> *const c_char {
    thaw_arena::owned_string(inspect_json(unsafe { &*value }))
}

/// `util.inspect` of a byte buffer given as a JSON number array (`console.log(Buffer)`).
#[no_mangle]
pub extern "C" fn thaw_json_buffer_inspect(value: *mut Value) -> *const c_char {
    let bytes = unsafe { &*value }.as_array()
        .map(|items| items.iter().map(|item| item.as_f64().unwrap_or(0.0) as u8).collect())
        .unwrap_or_default();
    thaw_arena::owned_string(crate::inspect::inspect(
        &crate::inspect::Node::Buffer(bytes), &crate::inspect::InspectOptions::default()))
}

/// `console.log` of a live JS value: the typed-array probe's JSON when it is one, else
/// the already-computed fallback text.
#[no_mangle]
pub extern "C" fn thaw_console_typed_or(probe: *const c_char, fallback: *const c_char) -> *const c_char {
    if probe.is_null() {
        return fallback;
    }
    let parsed: Option<(String, Vec<crate::inspect::Node>)> = serde_json::from_str::<serde_json::Value>(&to_str(probe)).ok().and_then(|json| {
        let name = json.get("n")?.as_str()?.to_string();
        let items = json.get("v")?.as_array()?.iter().map(|item| {
            let text = item.as_str()?;
            Some(match text.strip_suffix('n') {
                Some(digits) => crate::inspect::Node::BigInt(digits.to_string()),
                None => crate::inspect::Node::Number(text.parse().ok()?),
            })
        }).collect::<Option<Vec<_>>>()?;
        Some((name, items))
    });
    let Some((name, items)) = parsed else { return fallback };
    thaw_arena::owned_string(crate::inspect::inspect(
        &crate::inspect::Node::Typed { name, items }, &crate::inspect::InspectOptions::default()))
}

#[no_mangle]
pub extern "C" fn thaw_json_console_string(value: *mut Value) -> *const c_char {
    let value = unsafe { &*value };
    let text = match value {
        // A bare top-level string prints unquoted; everything else (including a nested
        // string) goes through `util.inspect`.
        Value::String(value) => value.clone(),
        Value::Wtf8(bytes) => wtf8_to_string(bytes),
        other => inspect_json(other),
    };
    thaw_arena::owned_string(text)
}

/// `util.format` over an args array (`console.log("%s is %d", a, b, ...)`): the first string
/// consumes `%s %d %i %f %j %o %O %c %%` against the following args; the rest are appended
/// space-separated (strings bare, everything else `util.inspect`ed).
#[no_mangle]
pub extern "C" fn thaw_console_format(args: *mut Value) -> *const c_char {
    let args = unsafe { &*args }.as_array().cloned().unwrap_or_default();
    thaw_arena::owned_string(console_format(&args))
}

fn console_format(args: &[Value]) -> String {
    let bare = |value: &Value| match value {
        Value::String(text) => text.clone(),
        Value::Wtf8(bytes) => wtf8_to_string(bytes),
        other => inspect_json(other),
    };
    let Some(first) = args.first() else { return String::new() };
    let mut out = String::new();
    let mut next = 1;
    if let (Value::String(_) | Value::Wtf8(_), true) = (first, args.len() > 1) {
        let template = bare(first);
        let mut chars = template.chars().peekable();
        while let Some(c) = chars.next() {
            if c != '%' { out.push(c); continue; }
            let Some(&spec) = chars.peek() else { out.push('%'); break };
            if spec == '%' { chars.next(); out.push('%'); continue; }
            if next >= args.len() || !"sdifjoOc".contains(spec) { out.push('%'); continue; }
            chars.next();
            let arg = &args[next];
            next += 1;
            let number = |arg: &Value| match arg {
                Value::Number(n) => n.as_f64().unwrap_or(f64::NAN),
                Value::String(s) => s.trim().parse().unwrap_or(f64::NAN),
                Value::Bool(b) => *b as u8 as f64,
                Value::Null => 0.0,
                _ => f64::NAN,
            };
            match spec {
                's' => out.push_str(&match arg {
                    Value::Array(_) | Value::Object(_) => crate::inspect::inspect(
                        &inspect_node(arg, &mut Vec::new()),
                        &crate::inspect::InspectOptions { depth: Some(0), ..Default::default() },
                    ),
                    Value::BigInt(_) => format!("{}n", bare(arg)),
                    other => bare(other),
                }),
                'd' if matches!(arg, Value::BigInt(_)) => out.push_str(&format!("{}n", bare(arg))),
                'd' => out.push_str(&inspect_json(&number_value(number(arg)))),
                'i' | 'f' => {
                    let text = bare(arg);
                    let parsed = if spec == 'i' {
                        let t = text.trim_start();
                        let end = t.char_indices().find(|&(i, c)| !(c.is_ascii_digit() || (i == 0 && (c == '-' || c == '+')))).map_or(t.len(), |(i, _)| i);
                        t[..end].parse::<f64>().unwrap_or(f64::NAN)
                    } else {
                        let t = text.trim_start();
                        (0..=t.len()).rev().filter(|&i| t.is_char_boundary(i)).find_map(|i| t[..i].parse::<f64>().ok()).unwrap_or(f64::NAN)
                    };
                    out.push_str(&inspect_json(&number_value(parsed)));
                }
                'j' => {
                    let mut bytes = Vec::new();
                    write_json_value(arg, &mut bytes, None, 0);
                    out.push_str(&String::from_utf8_lossy(&bytes));
                }
                'O' => out.push_str(&inspect_json(arg)),
                // `%o`: `{ showHidden: true, showProxy: true, depth: 4 }`.
                'o' => out.push_str(&crate::inspect::inspect(
                    &inspect_node(arg, &mut Vec::new()),
                    &crate::inspect::InspectOptions { depth: Some(4), show_hidden: true, ..Default::default() },
                )),
                _ => {}
            }
        }
    } else {
        out = bare(first);
    }
    for arg in &args[next..] {
        out.push(' ');
        out.push_str(&bare(arg));
    }
    out
}

/// Decodes WTF-8 bytes to UTF-16 code units, preserving lone surrogates
/// (a compact copy of the runtime codec -- `thaw-std` cannot reach
/// `thaw-runtime`'s private helpers).
fn wtf8_decode_utf16(bytes: &[u8]) -> Vec<u16> {
    let continuation = |byte: u8| byte & 0xC0 == 0x80;
    let mut units = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        let first = bytes[index];
        let (width, code) = if first < 0x80 {
            (1, u32::from(first))
        } else if (0xC2..=0xDF).contains(&first)
            && index + 1 < bytes.len()
            && continuation(bytes[index + 1])
        {
            (
                2,
                ((u32::from(first) & 0x1F) << 6) | (u32::from(bytes[index + 1]) & 0x3F),
            )
        } else if (0xE0..=0xEF).contains(&first)
            && index + 2 < bytes.len()
            && continuation(bytes[index + 1])
            && continuation(bytes[index + 2])
        {
            (
                3,
                ((u32::from(first) & 0x0F) << 12)
                    | ((u32::from(bytes[index + 1]) & 0x3F) << 6)
                    | (u32::from(bytes[index + 2]) & 0x3F),
            )
        } else if (0xF0..=0xF4).contains(&first)
            && index + 3 < bytes.len()
            && continuation(bytes[index + 1])
            && continuation(bytes[index + 2])
            && continuation(bytes[index + 3])
        {
            (
                4,
                ((u32::from(first) & 0x07) << 18)
                    | ((u32::from(bytes[index + 1]) & 0x3F) << 12)
                    | ((u32::from(bytes[index + 2]) & 0x3F) << 6)
                    | (u32::from(bytes[index + 3]) & 0x3F),
            )
        } else {
            units.push(0xFFFD);
            index += 1;
            continue;
        };
        if width == 4 && (0x10000..=0x10FFFF).contains(&code) {
            let code = code - 0x10000;
            units.push(0xD800 + (code >> 10) as u16);
            units.push(0xDC00 + (code & 0x3FF) as u16);
        } else {
            units.push(code as u16);
        }
        index += width;
    }
    units
}

/// Writes `bytes` (WTF-8) as a JSON string literal, escaping a lone
/// surrogate as `\uXXXX` -- which `serde_json` cannot, since it only
/// handles valid UTF-8.
fn write_json_string(bytes: &[u8], out: &mut Vec<u8>) {
    out.push(b'"');
    let units = wtf8_decode_utf16(bytes);
    let mut index = 0;
    while index < units.len() {
        let unit = units[index];
        match unit {
            0x22 => out.extend_from_slice(b"\\\""),
            0x5C => out.extend_from_slice(b"\\\\"),
            0x08 => out.extend_from_slice(b"\\b"),
            0x0C => out.extend_from_slice(b"\\f"),
            0x0A => out.extend_from_slice(b"\\n"),
            0x0D => out.extend_from_slice(b"\\r"),
            0x09 => out.extend_from_slice(b"\\t"),
            0x00..=0x1F => {
                out.extend_from_slice(format!("\\u{unit:04x}").as_bytes());
            }
            0xD800..=0xDBFF
                if index + 1 < units.len() && (0xDC00..=0xDFFF).contains(&units[index + 1]) =>
            {
                let code = 0x10000
                    + ((u32::from(unit) - 0xD800) << 10)
                    + (u32::from(units[index + 1]) - 0xDC00);
                let mut buffer = [0u8; 4];
                let text = char::from_u32(code)
                    .expect("astral code point")
                    .encode_utf8(&mut buffer);
                out.extend_from_slice(text.as_bytes());
                index += 2;
                continue;
            }
            0xD800..=0xDFFF => {
                out.extend_from_slice(format!("\\u{unit:04x}").as_bytes());
            }
            _ => {
                let mut buffer = [0u8; 4];
                let text = char::from_u32(u32::from(unit))
                    .expect("BMP code unit")
                    .encode_utf8(&mut buffer);
                out.extend_from_slice(text.as_bytes());
            }
        }
        index += 1;
    }
    out.push(b'"');
}

fn write_json_indent(out: &mut Vec<u8>, indent: &[u8], depth: usize) {
    out.push(b'\n');
    for _ in 0..depth {
        out.extend_from_slice(indent);
    }
}

/// Serializes a `Value` directly (bypassing `serde_json`, which cannot
/// represent a lone surrogate) to JSON text.
fn write_json_value(value: &Value, out: &mut Vec<u8>, indent: Option<&[u8]>, depth: usize) {
    let Some(_ancestor) = stringify_ancestor(value) else {
        out.extend_from_slice(b"null");
        return;
    };
    match value {
        Value::Null => out.extend_from_slice(b"null"),
        Value::Bool(true) => out.extend_from_slice(b"true"),
        Value::Bool(false) => out.extend_from_slice(b"false"),
        Value::BigInt(_) => {
            // The public stringify caller reads this through the existing
            // host-error branch, which preserves the distinct TypeError
            // message rather than reporting a circular structure.
            set_host_error("\u{1}TypeError\u{1}Do not know how to serialize a BigInt".into());
            out.extend_from_slice(b"null");
        }
        Value::Number(number) => {
            // A JSON number with an exponent (`-2e3`) parses to an `f64`;
            // Node's `JSON.stringify` writes an integral value without a
            // trailing `.0`, so normalize within the exact-integer range.
            if let Some(value) = number.as_f64() {
                if value.is_finite() && value.fract() == 0.0 && value.abs() < 1e15 {
                    out.extend_from_slice(format!("{}", value as i64).as_bytes());
                    return;
                }
            }
            out.extend_from_slice(number.to_string().as_bytes());
        }
        Value::String(text) => write_json_string(text.as_bytes(), out),
        Value::Wtf8(bytes) => write_json_string(bytes, out),
        Value::Array(items) => {
            let items = unsafe { &*items.get() };
            if items.is_empty() {
                out.extend_from_slice(b"[]");
                return;
            }
            out.push(b'[');
            for (index, item) in items.iter().enumerate() {
                if index > 0 {
                    out.push(b',');
                }
                if let Some(indent) = indent {
                    write_json_indent(out, indent, depth + 1);
                }
                write_json_value(item, out, indent, depth + 1);
            }
            if let Some(indent) = indent {
                write_json_indent(out, indent, depth);
            }
            out.push(b']');
        }
        Value::Object(fields) => {
            let fields = unsafe { &*fields.get() };
            if fields.is_empty() {
                out.extend_from_slice(b"{}");
                return;
            }
            out.push(b'{');
            for (index, (key, value)) in fields.iter().enumerate() {
                if index > 0 {
                    out.push(b',');
                }
                if let Some(indent) = indent {
                    write_json_indent(out, indent, depth + 1);
                }
                write_json_string(key, out);
                out.push(b':');
                if indent.is_some() {
                    out.push(b' ');
                }
                write_json_value(value, out, indent, depth + 1);
            }
            if let Some(indent) = indent {
                write_json_indent(out, indent, depth);
            }
            out.push(b'}');
        }
        Value::Host(lease) => {
            if let Some(serialized) = host_query(lease, 8) {
                // The host serializes compactly; re-indent through the native writer.
                match JsonParser::new(serialized.as_bytes()).parse().filter(|_| indent.is_some()) {
                    Some(parsed) => write_json_value(&parsed, out, indent, depth),
                    None => out.extend_from_slice(serialized.as_bytes()),
                }
            } else {
                out.extend_from_slice(b"null");
            }
        }
    }
}

fn stringify_value(value: &Value, indent: &[u8]) -> *const c_char {
    // Serialized directly rather than through `serde_json`, so a
    // `Value::Wtf8` string keeps its lone surrogate as a `\uXXXX` escape.
    let indent = if indent.is_empty() {
        None
    } else {
        Some(indent)
    };
    if STRINGIFY_ERROR.with(Cell::get) {
        return stringify_cycle_error();
    }
    let mut output = Vec::new();
    write_json_value(value, &mut output, indent, 0);
    if STRINGIFY_ERROR.with(Cell::get) {
        return stringify_cycle_error();
    }
    CString::new(output).unwrap_or_default().into_raw()
}

fn stringify_with_indent(value: *mut Value, indent: &[u8]) -> *const c_char {
    begin_stringify();
    let value = unsafe { &*value };
    if is_napi_undefined(value) {
        return top_level_undefined_string();
    }
    stringify_value(&ordered_json_omitting_undefined(value), indent)
}

fn string_array(array: *const u8, presence: *const u8) -> Vec<Vec<u8>> {
    if array.is_null() {
        return Vec::new();
    }
    let length = unsafe { (array as *const i64).read() }.max(0) as usize;
    let mut keys = Vec::new();
    for index in 0..length {
        if !native_array_slot_present(presence, index) {
            continue;
        }
        let key = unsafe { (array.add(8 + index * 8) as *const *const c_char).read() };
        let key = to_key(key);
        if !keys.contains(&key) {
            keys.push(key);
        }
    }
    keys
}

fn stringify_with_keys(value: *mut Value, keys: *const u8, presence: *const u8, indent: &[u8]) -> *const c_char {
    begin_stringify();
    let value = unsafe { &*value };
    if is_napi_undefined(value) {
        return top_level_undefined_string();
    }
    let keys = string_array(keys, presence);
    stringify_value(&filtered_json_omitting_undefined(value, &keys), indent)
}

#[no_mangle]
pub extern "C" fn thaw_json_stringify_number_space(value: *mut Value, space: f64) -> *const c_char {
    let width = space.trunc().clamp(0.0, 10.0) as usize;
    stringify_with_indent(value, &vec![b' '; width])
}

#[no_mangle]
pub extern "C" fn thaw_json_stringify_string_space(
    value: *mut Value,
    space: *const c_char,
) -> *const c_char {
    let indent = wtf8_encode_utf16(&wtf8_decode_utf16(unsafe { CStr::from_ptr(space) }.to_bytes())
        .into_iter().take(10).collect::<Vec<_>>());
    stringify_with_indent(value, &indent)
}

#[no_mangle]
pub extern "C" fn thaw_json_stringify_keys(value: *mut Value, keys: *const u8, presence: *const u8) -> *const c_char {
    stringify_with_keys(value, keys, presence, &[])
}

#[no_mangle]
pub extern "C" fn thaw_json_stringify_keys_number_space(
    value: *mut Value,
    keys: *const u8,
    presence: *const u8,
    space: f64,
) -> *const c_char {
    let width = space.trunc().clamp(0.0, 10.0) as usize;
    stringify_with_keys(value, keys, presence, &vec![b' '; width])
}

#[no_mangle]
pub extern "C" fn thaw_json_stringify_keys_string_space(
    value: *mut Value,
    keys: *const u8,
    presence: *const u8,
    space: *const c_char,
) -> *const c_char {
    let indent = wtf8_encode_utf16(&wtf8_decode_utf16(unsafe { CStr::from_ptr(space) }.to_bytes())
        .into_iter().take(10).collect::<Vec<_>>());
    stringify_with_keys(value, keys, presence, &indent)
}

#[no_mangle]
pub extern "C" fn thaw_json_get(value: *mut Value, key: *const c_char) -> *mut Value {
    let value = unsafe { &*value };
    let key = to_key(key);
    if let Value::Host(lease) = value {
        return track_typed_decode_child(leak(host_get_property(lease, &key)));
    }
    if let Some(result) = regexp_wrapper_property(value, &wtf8_to_string(&key)) {
        return track_typed_decode_child(leak(result));
    }
    if key.as_slice() == b"size" {
        if let Some(result) = map_or_set_wrapper_size(value) {
            return track_typed_decode_child(leak(result));
        }
    }
    // `.length` on a JSON array (or a Buffer-shaped object, `{"type":
    // "Buffer","data":[...]}` -- see `json_array_or_buffer_data`'s own
    // doc comment for why a real Buffer crossing into a native callback
    // takes this shape) is a built-in, not a data field -- handled here
    // rather than via a dedicated HIR node/lowering rule, since a
    // dynamically-typed `Json` value's runtime kind isn't known until
    // now. `serde_json::Value::get` only matches string keys against
    // objects, so without this a Buffer's `.length` would silently
    // resolve to `Null` (-> `0`)/`undefined` instead of its real byte
    // count (found via a real `fs.createReadStream(...).on('data', ...)`
    // callback with an untyped/`any` chunk parameter).
    let result = match json_array_or_buffer_data(value) {
        Some(items) if key.as_slice() == b"length" => Value::Number((items.len() as u64).into()),
        Some(_) if matches!(value, Value::Array(items) if array_index_key(&key)
            .is_some_and(|index| !array_has_index(items, index as usize))) =>
            json_get_with_prototype(value, &key),
        Some(items) if array_index_key(&key).is_some() => items
            .get(array_index_key(&key).unwrap() as usize)
            .cloned()
            .unwrap_or_else(napi_undefined_value),
        _ => json_get_with_prototype(value, &key),
    };
    track_typed_decode_child(leak(result))
}

/// `object[key]` with prototype-chain lookup: own field, else walk the
/// prototypes recorded by `Object.create`/`Object.setPrototypeOf`
/// (`PROTOTYPES`), stopping at a missing/null prototype or a depth cap
/// (cycle guard). Returns `undefined` when not found. An inherited *getter*
/// isn't modelled (the prototype stores plain values).
fn json_get_with_prototype(value: &Value, key: &[u8]) -> Value {
    let mut current = value.clone();
    let mut seen = HashSet::new();
    loop {
        if let Some(found) = current.as_object().and_then(|fields| fields.get(key)) {
            return found.clone();
        }
        if let (Value::Array(items), Some(index)) = (&current, array_index_key(key)) {
            if array_has_index(items, index as usize) {
                return shared_array_ref(items)[index as usize].clone();
            }
        }
        let Some(identity) = object_identity_key(&current) else {
            break;
        };
        if !seen.insert(identity) {
            break;
        }
        let Some(prototype) = PROTOTYPES.with(|table| table.borrow().get(&identity).cloned())
        else {
            break;
        };
        current = prototype;
        if matches!(current, Value::Null) {
            break;
        }
    }
    napi_undefined_value()
}

/// `key in object` with prototype-chain lookup (the `HasProperty` used by
/// `in`), unlike `thaw_json_has_own` (`hasOwnProperty`). Own-ness of a key
/// on one link of the chain is `json_has_own_value` (so an array's
/// `length`/index keys count, matching `thaw_json_has_own`).
#[no_mangle]
pub extern "C" fn thaw_json_has(value: *mut Value, key: *const c_char) -> u8 {
    let key = to_key(key);
    let value = unsafe { &*value };
    if let Value::Host(lease) = value {
        return u8::from(host_property_predicate(lease, &key, 0));
    }
    let mut current = value.clone();
    let mut seen = HashSet::new();
    loop {
        if json_has_own_value(&current, &key) {
            return 1;
        }
        let Some(identity) = object_identity_key(&current) else {
            return 0;
        };
        if !seen.insert(identity) {
            return 0;
        }
        let Some(prototype) = PROTOTYPES.with(|table| table.borrow().get(&identity).cloned())
        else {
            return 0;
        };
        current = prototype;
        if matches!(current, Value::Null) {
            return 0;
        }
    }
    0
}

/// Own-key check for one link of a prototype chain: object fields, plus an
/// array's `length`/in-range index keys (the same set `thaw_json_has_own`
/// reports).
fn json_has_own_value(value: &Value, key: &[u8]) -> bool {
    match value {
        Value::Object(fields) => shared_object_ref(fields).contains_key(key),
        Value::Array(_) if key == b"length".as_slice() => true,
        Value::Array(items) => array_index_key(key)
            .is_some_and(|index| array_has_index(items, index as usize)),
        _ => false,
    }
}

/// Takes one field while consuming a temporary JSON object.
///
/// # Safety
///
/// `value` must be a pointer returned by this module and must not be used again.
#[no_mangle]
pub unsafe extern "C" fn thaw_json_take(value: *mut Value, key: *const c_char) -> *mut Value {
    untrack_typed_decode_child(value);
    untrack_arena_owned_json_root(value);
    let value = unsafe { Box::from_raw(value) };
    let result = value
        .as_object_mut()
        .and_then(|fields| fields.shift_remove(to_key(key).as_slice()))
        .unwrap_or_else(napi_undefined_value);
    leak(result)
}

#[no_mangle]
pub extern "C" fn thaw_json_index(value: *mut Value, index: f64, key: *const c_char) -> *mut Value {
    let value = unsafe { &*value };
    if let Value::Host(lease) = value {
        let key = if key.is_null() { index.to_string().into_bytes() } else { to_key(key) };
        return track_typed_decode_child(leak(host_get_property(lease, &key)));
    }
    let valid_index =
        index.is_finite() && index >= 0.0 && index <= (u32::MAX - 1) as f64 && index.fract() == 0.0;
    let result = match value {
        Value::Array(items) if valid_index && array_has_index(items, index as usize) =>
            shared_array_ref(items).get(index as usize).cloned(),
        // A Buffer-shaped object (`{"type":"Buffer","data":[...]}`)
        // indexed numerically -- see `json_array_or_buffer_data`'s own
        // doc comment. Only wins when the object is genuinely
        // Buffer-shaped: an ordinary object with an unrelated
        // numeric-string key (`{"0": "one", ...}`, a real pattern this
        // same function already serves) falls through to the plain key
        // lookup below untouched.
        Value::Object(_) if valid_index && json_array_or_buffer_data(value).is_some() => {
            json_array_or_buffer_data(value)
                .and_then(|items| items.get(index as usize))
                .cloned()
        }
        Value::Object(fields) => shared_object_ref(fields).get(to_key(key).as_slice()).cloned(),
        _ => None,
    }
    .unwrap_or_else(napi_undefined_value);
    track_typed_decode_child(leak(result))
}

#[no_mangle]
/// # Safety
///
/// `array` and `value` must be null or point to valid JSON values.
pub unsafe extern "C" fn thaw_json_index_set(
    array: *mut Value,
    index: f64,
    key: *const c_char,
    value: *mut Value,
) -> *mut Value {
    if let Some(Value::Host(lease)) = unsafe { array.as_ref() } {
        let key = if key.is_null() { index.to_string().into_bytes() } else { to_key(key) };
        if let Some(value_ref) = unsafe { value.as_ref() } {
            host_set_property(lease, &key, value_ref);
        }
        return value;
    }
    if !object_writable(array, &to_key(key)) {
        return value;
    }
    if let (Some(container), Some(value)) = (unsafe { array.as_mut() }, unsafe { value.as_ref() }) {
        match container {
            Value::Array(items)
                if index.is_finite()
                    && index >= 0.0
                    && index <= (u32::MAX - 1) as f64
                    && index.fract() == 0.0 =>
            {
                let index = index as usize;
                let old_len = shared_array_ref(items).len();
                if old_len < index {
                    mark_array_holes(items, old_len..index);
                }
                mark_array_present(items, index);
                let items = shared_array_ref_mut(items);
                if items.len() <= index {
                    // A write past the end leaves the intermediate slots as
                    // real JS array *holes*, which read back as `undefined`
                    // (not `null`); `JSON.stringify` still emits them as
                    // `null` via `ordered_json_omitting_undefined`.
                    items.resize(index + 1, napi_undefined_value());
                }
                items[index] = value.clone();
            }
            Value::Object(fields) => {
                shared_object_ref_mut(fields).insert(to_key(key), value.clone());
            }
            _ => {}
        }
    }
    value
}

/// Like `thaw_json_get`, but returns a pointer *into* `value`'s own
/// storage instead of a clone -- for an assignment target's intermediate
/// container (`a.b.c = x` needs `a.b` to still be `a`'s own nested
/// object, not a disconnected copy, or the write never reaches `a`).
/// Auto-vivifies a missing/non-object field to a fresh empty object,
/// matching the permissive "degrade instead of abort" policy the rest of
/// this file follows -- real JS would throw assigning through `undefined`
/// here, but there's no exception channel wired to this dynamic-value
/// bridge yet (see this file's own module doc comment).
///
/// A newly-created field defaults to an empty *object*, not `Null` --
/// `.field` access always implies the caller is about to treat it as an
/// object (either another `.field`/computed-string-key step, which
/// re-vivifies through this same function, or the final property write,
/// whose native setter (`thaw_json_object_set_*`/`object_insert`) only
/// inserts into an already-`Value::Object` target and silently no-ops
/// otherwise -- by design, matching real JS assigning through a
/// primitive receiver). Only a chain ending in a *computed numeric*
/// index (`.a[0] = x` where `.a` doesn't exist yet) still needs `.a` to
/// become an array instead; `thaw_json_index_get_mut` re-vivifies an
/// object placeholder like this one into an array on its own, so this
/// only stays wrong for `.a[0] = x` written as the assignment's own
/// final step (no further chained access) -- left as a narrower,
/// unfixed edge case.
///
/// # Safety
/// `value` must be null or point to a valid, mutable JSON `Value`.
#[no_mangle]
pub unsafe extern "C" fn thaw_json_get_mut(value: *mut Value, key: *const c_char) -> *mut Value {
    let Some(value) = (unsafe { value.as_mut() }) else {
        return std::ptr::null_mut();
    };
    if !matches!(value, Value::Object(_)) || is_napi_undefined(value) {
        *value = Value::shared_object(indexmap::IndexMap::new());
    }
    let Value::Object(fields) = value else {
        unreachable!()
    };
    shared_object_ref_mut(fields)
        .entry(to_key(key))
        .or_insert_with(|| Value::shared_object(indexmap::IndexMap::new()))
}

/// The array/computed-index counterpart to `thaw_json_get_mut`.
///
/// # Safety
/// `value` must be null or point to a valid, mutable JSON `Value`.
#[no_mangle]
pub unsafe extern "C" fn thaw_json_index_get_mut(value: *mut Value, index: f64) -> *mut Value {
    let Some(value) = (unsafe { value.as_mut() }) else {
        return std::ptr::null_mut();
    };
    if !index.is_finite() || index < 0.0 || index > (u32::MAX - 1) as f64 || index.fract() != 0.0 {
        return std::ptr::null_mut();
    }
    if !matches!(value, Value::Array(_)) {
        *value = Value::shared_array(Vec::new());
    }
    let Value::Array(items) = value else {
        unreachable!()
    };
    let index = index as usize;
    let old_len = shared_array_ref(items).len();
    if old_len < index {
        mark_array_holes(items, old_len..index);
    }
    mark_array_present(items, index);
    let items = shared_array_ref_mut(items);
    if items.len() <= index {
        // Intermediate slots are real JS array holes reading as
        // `undefined` (see `thaw_json_index_set`'s own comment).
        items.resize(index + 1, napi_undefined_value());
    }
    &mut items[index]
}

#[no_mangle]
pub extern "C" fn thaw_json_as_number(value: *mut Value) -> f64 {
    let value = unsafe { &*value };
    json_to_number(value)
}

/// Read a Json-backed BigInt into Thaw's native signed-i64 representation.
/// Values outside that representation fail through the existing host-error
/// channel; this must never round through f64 or wrap on overflow.
///
/// # Safety
/// `value` must point to a live Json value allocated by this runtime.
#[no_mangle]
pub unsafe extern "C" fn thaw_json_as_bigint_i64(value: *const Value) -> i64 {
    let Some(value) = (unsafe { value.as_ref() }) else {
        set_host_error("Missing Json BigInt value".into());
        return 0;
    };
    let decimal = match value {
        Value::BigInt(decimal) => decimal.clone(),
        Value::Host(lease) => {
            match host_query(lease, 0).as_deref() {
                Some("bigint") => {}
                Some(_) => {
                    set_host_error("Json value is not a BigInt".into());
                    return 0;
                }
                None => return 0,
            }
            let Some(decimal) = host_query(lease, 10) else {
                // `host_query` recorded the callback error. Return immediately
                // so the LLVM caller consumes it before any further host call.
                return 0;
            };
            decimal
        }
        _ => {
            set_host_error("Json value is not a BigInt".into());
            return 0;
        }
    };
    if !canonical_bigint_decimal(&decimal) {
        set_host_error("Invalid Json BigInt decimal".into());
        return 0;
    }
    match decimal.parse::<i64>() {
        Ok(value) => value,
        Err(_) => {
            set_host_error("Json BigInt is outside native i64 range".into());
            0
        }
    }
}

/// Real ECMAScript `ToNumber`, for every `Value` shape this file can
/// actually produce. A missing-key/index sentinel (`is_napi_undefined`)
/// reads as `NaN`, matching real `Number(undefined)` -- so a missing key
/// is neither `== 0` nor usable in arithmetic, exactly like real JS.
fn json_to_number(value: &Value) -> f64 {
    if let Some(number) = non_finite_number(value) {
        return number;
    }
    if is_napi_undefined(value) {
        return f64::NAN;
    }
    match value {
        Value::Number(number) => number.as_f64().unwrap_or(0.0),
        Value::BigInt(decimal) => decimal.parse::<f64>().unwrap_or(f64::NAN),
        Value::Null => 0.0,
        Value::Bool(flag) => f64::from(*flag),
        Value::String(text) => javascript_string_to_number(text),
        Value::Wtf8(bytes) => javascript_string_to_number(&wtf8_to_string(bytes)),
        Value::Array(_) => {
            let joined = unsafe { thaw_json_array_join(value, c",".as_ptr()) };
            let number = javascript_string_to_number(&to_str(joined));
            unsafe { thaw_cstring_destroy(joined.cast_mut()) };
            number
        }
        Value::Object(_) if thaw_json_is_date_shape(value) != 0 => thaw_json_date_timestamp(value),
        Value::Object(_) => f64::NAN,
        Value::Host(lease) => host_query(lease, 2)
            .map(|text| javascript_string_to_number(&text)).unwrap_or(f64::NAN),
    }
}

unsafe extern "C" {
    fn thaw_string_to_number(value: *const c_char) -> f64;
    fn thaw_number_to_string(value: f64) -> *const c_char;
    /// Defined in thaw-runtime's `native_values/date.rs` -- resolved at
    /// link time the same way `thaw_string_to_number` above already is
    /// (thaw-std has no direct crate dependency on thaw-runtime; both
    /// land in the same final linked binary).
    fn thaw_date_to_iso_string(timestamp: f64) -> *const c_char;
}

/// A `console.log`/`JSON.stringify`-ready rendering of a `Date`-shaped
/// `Json` value (`thaw_json_is_date_shape`'s exact `{"timestamp": N}`
/// convention, `platform_globals/dates.js`'s own wire format) as real
/// JS's `Date.prototype.toISOString()` text. `None` when `value` isn't
/// Date-shaped at all -- the caller falls through to its own generic
/// handling. `Some(None)` when it *is* Date-shaped but the timestamp is
/// non-finite/out-of-range (an invalid Date, `new Date(NaN)`) -- distinct
/// from `None` because real `Date.prototype.toJSON` still special-cases
/// this (returns `null` rather than throwing, and `String(date)`/
/// `console.log` gives `"Invalid Date"`), so a caller needs to tell
/// "not a Date" apart from "a Date, but an invalid one" rather than
/// silently falling through to generic object serialization for the
/// latter (which previously leaked the raw `{"timestamp":null}` wire
/// shape instead of either of those).
fn date_iso_string(value: &Value) -> Option<Option<String>> {
    if thaw_json_is_date_shape(value as *const Value) == 0 {
        return None;
    }
    let timestamp = thaw_json_date_timestamp(value as *const Value);
    let text = unsafe { thaw_date_to_iso_string(timestamp) };
    Some(if text.is_null() {
        None
    } else {
        Some(to_str(text))
    })
}

fn javascript_string_to_number(text: &str) -> f64 {
    let Ok(text) = CString::new(text) else {
        return f64::NAN;
    };
    unsafe { thaw_string_to_number(text.as_ptr()) }
}

/// `Array.prototype.join` on a dynamic (`Json`) array value -- e.g. a
/// `Json`-typed value returned by a dynamic call
/// (`Array.prototype.map.call(...).join(", ")`, real test262 harness). Joins
/// each element's JavaScript `String()` form with `separator`, treating
/// `null`/`undefined` as an empty string (unlike `thaw_json_as_string`, which
/// renders them "null"/"undefined" for a bare `String(value)`).
///
/// # Safety
/// `value` must be null or point to a valid JSON `Value`; `separator` must
/// point to a valid NUL-terminated string.
#[no_mangle]
pub unsafe extern "C" fn thaw_json_array_join(
    value: *const Value,
    separator: *const c_char,
) -> *const c_char {
    let separator = to_str(separator);
    let Some(Value::Array(items)) = (unsafe { value.as_ref() }) else {
        return CString::new("").unwrap_or_default().into_raw();
    };
    let mut joined = String::new();
    for (index, item) in shared_array_ref(items).iter().enumerate() {
        if index > 0 {
            joined.push_str(&separator);
        }
        match item {
            Value::Null => {}
            other if is_napi_undefined(other) => {}
            other => {
                let text = thaw_json_as_string(other as *const Value as *mut Value);
                joined.push_str(&to_str(text));
                thaw_cstring_destroy(text as *mut c_char);
            }
        }
    }
    thaw_arena::owned_string(joined)
}

/// The string-valued `key` of a caught (live) Json value, if it has one.
fn caught_json_text(value: *mut Value, key: &str) -> Option<String> {
    let key = std::ffi::CString::new(key).ok()?;
    let found = thaw_json_get(value, key.as_ptr());
    match unsafe { &*found } {
        Value::String(text) => Some(text.clone()),
        _ => None,
    }
}

/// `String(error)` / `error.toString()` for a caught value that arrived as live Json:
/// `Name: message` for an Error-like object (a `name` or `message` string), `[object Object]`
/// otherwise. Mirrors `thaw_error_to_string` for the framed-string carrier member.
#[no_mangle]
pub extern "C" fn thaw_json_error_to_string(value: *mut Value) -> *const c_char {
    let text = match unsafe { &*value } {
        Value::String(text) => text.clone(),
        Value::Object(_) | Value::Host(_) => {
            let name = caught_json_text(value, "name");
            let message = caught_json_text(value, "message");
            if name.is_none() && message.is_none() {
                "[object Object]".to_string()
            } else {
                let name = name.unwrap_or_else(|| "Error".to_string());
                let message = message.unwrap_or_default();
                if message.is_empty() { name } else if name.is_empty() { message } else { format!("{name}: {message}") }
            }
        }
        _ => return thaw_json_as_string(value),
    };
    thaw_arena::owned_string(text.as_bytes())
}

/// `error.stack` for a caught live Json value: its own `stack` string if it has one, else the
/// first line a real stack always starts with (`Name: message`; a native binary has no frames
/// to append), else `undefined`.
#[no_mangle]
pub extern "C" fn thaw_json_error_stack(value: *mut Value) -> *mut Value {
    if let Some(stack) = caught_json_text(value, "stack").filter(|stack| !stack.is_empty()) {
        return leak(Value::String(stack));
    }
    if caught_json_text(value, "name").is_none() && caught_json_text(value, "message").is_none() {
        return leak(napi_undefined_value());
    }
    let text = thaw_json_error_to_string(value);
    leak(Value::String(unsafe { CStr::from_ptr(text) }.to_string_lossy().into_owned()))
}

#[no_mangle]
pub extern "C" fn thaw_json_as_string(value: *mut Value) -> *const c_char {
    let value = unsafe { &*value };
    let text = match value {
        Value::Bool(flag) => {
            if *flag { "true".to_string() } else { "false".to_string() }
        }
        Value::Number(_) => {
            let formatted = unsafe { thaw_number_to_string(json_to_number(value)) };
            if formatted.is_null() {
                return std::ptr::null();
            }
            let bytes = unsafe { CStr::from_ptr(formatted) }.to_bytes();
            return thaw_arena::owned_string(bytes);
        }
        Value::String(s) => s.clone(),
        Value::BigInt(decimal) => decimal.clone(),
        // A lone-surrogate string keeps its raw WTF-8 bytes (a `String`
        // couldn't hold them).
        Value::Wtf8(bytes) => return thaw_arena::owned_string(bytes),
        Value::Null => "null".to_string(),
        // Matches real JS `String(undefined)` -- without this, `other`
        // below would stringify the sentinel's own raw JSON shape
        // (`{"$__thaw_napi_undefined$":true}`) instead. Also fixes
        // `thaw_jit_dictionary_get`'s string-kind path, which calls this
        // function.
        other if is_napi_undefined(other) => "undefined".to_string(),
        // Matches real JS `String(NaN)`/`String(Infinity)` -- without
        // this, `other` below would stringify the `$__thaw_non_finite$`
        // sentinel's own raw JSON shape instead.
        other if non_finite_number(other).is_some() => {
            non_finite_display(non_finite_number(other).unwrap()).to_string()
        }
        Value::Array(_) => {
            return unsafe { thaw_json_array_join(value, c",".as_ptr()) };
        }
        Value::Object(_) => "[object Object]".to_string(),
        Value::Host(lease) => {
            let encoded = host_query(lease, 3).unwrap_or_else(|| "\"\"".into());
            match JsonParser::new(encoded.as_bytes()).parse() {
                Some(Value::String(text)) => text,
                Some(Value::Wtf8(bytes)) => return thaw_arena::owned_string(bytes),
                _ => String::new(),
            }
        }
    };
    thaw_arena::owned_string(text)
}

#[no_mangle]
/// # Safety
/// `value` must point to a live JSON value allocated by this runtime.
pub unsafe extern "C" fn thaw_json_typeof(value: *const Value) -> *const c_char {
    let value = unsafe { &*value };
    if is_napi_undefined(value) {
        return c"undefined".as_ptr();
    }
    if non_finite_number(value).is_some() {
        return c"number".as_ptr();
    }
    match value {
        Value::Null | Value::Array(_) | Value::Object(_) => c"object".as_ptr(),
        Value::Host(lease) => match host_query(lease, 0).as_deref() {
            Some("undefined") => c"undefined".as_ptr(),
            Some("boolean") => c"boolean".as_ptr(),
            Some("number") => c"number".as_ptr(),
            Some("string") => c"string".as_ptr(),
            Some("function") => c"function".as_ptr(),
            Some("symbol") => c"symbol".as_ptr(),
            Some("bigint") => c"bigint".as_ptr(),
            _ => c"object".as_ptr(),
        },
        Value::Bool(_) => c"boolean".as_ptr(),
        Value::Number(_) => c"number".as_ptr(),
        Value::BigInt(_) => c"bigint".as_ptr(),
        Value::String(_) | Value::Wtf8(_) => c"string".as_ptr(),
    }
}

/// Returns `0` or `1` rather than a Rust `bool` -- deliberately avoids
/// relying on `bool`'s C ABI representation across the FFI boundary; the
/// LLVM caller compares this against zero itself (see `compile_expr`'s
/// `JsonAsBool` case in hir_codegen.rs).
#[no_mangle]
pub extern "C" fn thaw_json_as_bool(value: *mut Value) -> u8 {
    let value = unsafe { &*value };
    if is_napi_undefined(value) {
        return 0;
    }
    if let Some(number) = non_finite_number(value) {
        return u8::from(!number.is_nan());
    }
    match value {
        Value::Null => 0,
        Value::Bool(value) => u8::from(*value),
        Value::Number(value) => u8::from(value.as_f64().is_some_and(|value| value != 0.0)),
        Value::BigInt(value) => u8::from(value != "0"),
        Value::String(value) => u8::from(!value.is_empty()),
        Value::Wtf8(bytes) => u8::from(!bytes.is_empty()),
        Value::Array(_) | Value::Object(_) => 1,
        Value::Host(lease) => u8::from(host_query(lease, 1).as_deref() == Some("1")),
    }
}

fn is_napi_undefined(value: &Value) -> bool {
    if is_graph_plain_object(value) { return false; }
    matches!(
        value,
        Value::Object(object)
            if shared_object_ref(object).len() == 1 && matches!(
                shared_object_ref(object).get(b"$__thaw_napi_undefined$".as_slice()),
                Some(Value::Bool(true))
            )
    )
}

/// True for the write-only sentinel wrapper `coerce_to_declared`'s
/// `Json`-target branch produces for a `RegExp`/`Map`/`Set` value
/// crossing into `any` (`inference/coercions.rs`): `{"__thaw_regexp__":
/// {...}}` / `{"__thaw_map_entries__": [...]}` / `{"__thaw_set_values__":
/// [...]}`, always the object's one and only field. Real JS gives `{}`
/// for `JSON.stringify(/re/)` / `JSON.stringify(new Map())` /
/// `JSON.stringify(new Set())` (none of the three have their own
/// enumerable properties), so the wrapper should stringify the same way
/// instead of leaking its internal representation.
fn is_thaw_internal_wrapper(value: &Value) -> bool {
    let Some(fields) = value.as_object().filter(|_| is_branded_wrapper(value)) else {
        return false;
    };
    fields.len() == 1 && fields.keys().next().is_some_and(|key| {
        [b"__thaw_regexp__".as_slice(), b"__thaw_map_entries__", b"__thaw_set_values__"]
            .contains(&key.as_slice())
    })
}

/// General form of `is_thaw_internal_wrapper`'s own key check -- true
/// when `value` is an object whose one and only field is named `key`.
/// Backs `RegExp`/`Map`/`Set`'s own `instanceof` check against a
/// `Json`-typed (`any`) value (`lower_bin_expr`'s `InstanceOf` case,
/// `thaw-hir`'s `expressions/lowering.rs`): a value tagged with e.g.
/// `__thaw_regexp__` when it crossed into `any` is recognized as one
/// again the same way `regexp_wrapper_property`/`is_thaw_internal_
/// wrapper` already do for property reads and `JSON.stringify`, just
/// generalized to take the key as a runtime argument instead of a
/// fixed set.
#[no_mangle]
pub extern "C" fn thaw_json_has_wrapper_key(value: *const Value, key: *const c_char) -> u8 {
    let Some(value) = (unsafe { value.as_ref() }) else {
        return 0;
    };
    // A live Host lease is the handle marker's native form.
    if matches!(value, Value::Host(_)) {
        return u8::from(to_key(key).as_slice() == b"__thaw_js_handle_id__");
    }
    let Some(fields) = value.as_object().filter(|_| is_branded_wrapper(value)) else {
        return 0;
    };
    let key = to_key(key);
    u8::from(fields.len() == 1 && fields.keys().next().is_some_and(|field| *field == key))
}

/// `NaN`/`Infinity`/`-Infinity` have no JSON representation at all --
/// real `JSON.stringify` collapses them to `null`, and even if it
/// didn't, `serde_json::Number` structurally cannot hold a non-finite
/// value (`Number::from_f64` returns `None` for one). `platform_globals/
/// dates.js`'s `__thaw_json_safe_stringify` replacer tags one of these
/// as `{"$__thaw_non_finite$": "NaN" | "Infinity" | "-Infinity"}`
/// instead of letting it collapse silently -- this recognizes that
/// shape and recovers the real `f64` it stands for, mirroring
/// `is_napi_undefined`'s exact pattern for the sibling `undefined`
/// sentinel.
fn non_finite_number(value: &Value) -> Option<f64> {
    if is_graph_plain_object(value) { return None; }
    let Value::Object(object) = value else {
        return None;
    };
    let fields = shared_object_ref(object);
    if fields.len() != 1 { return None; }
    match fields
        .get(b"$__thaw_non_finite$".as_slice())?
        .as_str()?
    {
        "NaN" => Some(f64::NAN),
        "Infinity" => Some(f64::INFINITY),
        "-Infinity" => Some(f64::NEG_INFINITY),
        _ => None,
    }
}

/// A `Buffer`/`Uint8Array` crossing into `any`/`Json` is wrapped as
/// `{"type":"Buffer","data":[...]}` (`coerce_to_declared`'s `Json`-
/// target branch, `inference/coercions.rs`), matching real Node's own
/// `Buffer.prototype.toJSON`. Used by `instanceof Uint8Array` on a
/// `Json`-typed value -- deliberately *not* the same check `json_array_
/// or_buffer_data` (used for indexing/`.length`) makes, since that one
/// also accepts a bare JSON array for read purposes; `instanceof
/// Uint8Array` must stay `false` for a plain `number[]` the way real
/// JS's `[1,2,3] instanceof Uint8Array` does, only `true` for the
/// tagged wrapper shape specifically.
#[no_mangle]
pub extern "C" fn thaw_json_is_buffer_shape(value: *const Value) -> u8 {
    let Some(value) = (unsafe { value.as_ref() }) else {
        return 0;
    };
    if is_graph_plain_object(value) { return 0; }
    let Value::Object(fields) = value else {
        return 0;
    };
    let fields = shared_object_ref(fields);
    u8::from(
        fields.get(b"type".as_slice()).and_then(Value::as_str) == Some("Buffer")
            && fields.get(b"data".as_slice()).is_some_and(Value::is_array),
    )
}

/// A live JS object that is not an array (a candidate for the iteration
/// protocol). Plain native Json arrays and scalars answer `0`, so `for...of`
/// keeps its direct array loop for them.
#[no_mangle]
pub extern "C" fn thaw_json_is_live_iterable(value: *const Value) -> u8 {
    let Some(Value::Host(lease)) = (unsafe { value.as_ref() }) else {
        return 0;
    };
    u8::from(
        host_query(lease, 9).as_deref() == Some("1") && host_query(lease, 4).as_deref() == Some("0"),
    )
}

/// Recognize a trusted native or graph Date wrapper, or a live Host value
/// with a genuine Date internal slot. A plain user `{timestamp: ...}` object
/// is not enough to establish Date origin.
#[no_mangle]
pub extern "C" fn thaw_json_is_date_shape(value: *const Value) -> u8 {
    let Some(value) = (unsafe { value.as_ref() }) else {
        return 0;
    };
    if let Value::Host(lease) = value {
        return u8::from(host_query(lease, 12).as_deref() == Some("1"));
    }
    if !is_branded_wrapper(value) || is_graph_plain_object(value) { return 0; }
    let Value::Object(object) = value else {
        return 0;
    };
    // A NaN/out-of-range `timestamp` (`new Date(NaN)`, an *invalid* but
    // still real Date -- `real Date.prototype.toJSON`/`instanceof Date`
    // both still recognize it as a Date, just one whose every field
    // getter returns `NaN`) can't be stored as a plain JSON `Number`
    // (`serde_json::Number` structurally cannot hold a non-finite `f64`)
    // -- it's wrapped in the same `$__thaw_non_finite$` sentinel every
    // other non-finite `f64` written into a `Json` value gets
    // (`non_finite_number`, this same file). Without also matching that
    // shape here, an invalid Date silently stopped being recognized as
    // Date-shaped at all -- confirmed via a real probe: `(new Date(NaN)
    // as any) instanceof Date` gave `false` instead of real JS's `true`.
    let object = shared_object_ref(object);
    let is_timestamp = |field: Option<&Value>| {
        matches!(field, Some(Value::Number(_)))
            || field.is_some_and(|field| non_finite_number(field).is_some())
    };
    u8::from(object.len() == 1 && is_timestamp(object.get(b"timestamp".as_slice())))
}

/// Apply a Date setter's already-computed clipped time to the original
/// receiver. A `Json::Host` Date must mutate its JS internal slot; assigning
/// an ordinary `timestamp` property would leave aliases unchanged.
#[no_mangle]
pub extern "C" fn thaw_json_date_set_timestamp(value: *mut Value, timestamp: f64) -> f64 {
    if thaw_json_is_date_shape(value.cast_const()) == 0 {
        if HOST_ERROR.with(|error| error.borrow().is_none()) {
            set_host_error("\u{1}TypeError\u{1}Date method called on a non-Date value".into());
        }
        return f64::NAN;
    }
    let Some(value) = (unsafe { value.as_mut() }) else { return f64::NAN; };
    if let Value::Host(lease) = value {
        let Some(ops) = HOST_OPERATIONS.with(|slot| slot.get()) else {
            set_host_error("Host operations unavailable".into());
            return f64::NAN;
        };
        let result = without_typed_decode_scope(|| (ops.date_set)(lease.handle, timestamp));
        if !result.error.is_null() {
            let error = to_str(result.error);
            unsafe { thaw_arena::destroy_string(result.error.cast_mut()) };
            set_host_error(error);
            return f64::NAN;
        }
        if result.value.is_null() {
            set_host_error("Host Date setter returned no value".into());
            return f64::NAN;
        }
        let actual = javascript_string_to_number(&to_str(result.value));
        unsafe { thaw_arena::destroy_string(result.value.cast_mut()) };
        return actual;
    }
    if let Some(fields) = value.as_object_mut() {
        fields.insert(b"timestamp".to_vec(), number_value(timestamp));
        return timestamp;
    }
    set_host_error("\u{1}TypeError\u{1}Date method called on a non-Date value".into());
    f64::NAN
}

/// Extracts the millisecond timestamp from the `{"timestamp": N}` shape
/// `thaw_json_is_date_shape` recognizes -- including a non-finite one
/// wrapped in the `$__thaw_non_finite$` sentinel (see that function's
/// own doc comment). Any other shape (a caller that didn't check first,
/// or a genuinely malformed value): `NaN`, matching how this file
/// already degrades other malformed/absent numeric reads.
#[no_mangle]
pub extern "C" fn thaw_json_date_timestamp(value: *const Value) -> f64 {
    let Some(value) = (unsafe { value.as_ref() }) else {
        return f64::NAN;
    };
    if thaw_json_is_date_shape(value as *const Value) == 0 {
        if HOST_ERROR.with(|error| error.borrow().is_none()) {
            set_host_error("\u{1}TypeError\u{1}Date method called on a non-Date value".into());
        }
        return f64::NAN;
    }
    if let Value::Host(lease) = value {
        return host_query(lease, 13)
            .map(|text| javascript_string_to_number(&text)).unwrap_or(f64::NAN);
    }
    let Some(field) = value.as_object().and_then(|fields| fields.get(b"timestamp".as_slice())) else {
        return f64::NAN;
    };
    if let Some(non_finite) = non_finite_number(field) {
        return non_finite;
    }
    field.as_f64().unwrap_or(f64::NAN)
}

/// The same `$__thaw_napi_undefined$`-tagged sentinel object
/// `is_napi_undefined` recognizes, freshly constructed. Historically only
/// produced by native-callback-argument marshaling
/// (`compile_napi_undefined_json`, thaw-llvm); `thaw_json_get`/
/// `thaw_json_index` also reach for it now for a genuinely *missing*
/// key/index -- distinguishing that case from an explicit `null`, which
/// stays real `Value::Null` (untouched by this function).
thread_local! {
    /// Names of functions that crossed a compiled-result boundary as the undefined sentinel
    /// (JSON data has no function values); only `console.log` consults it to print
    /// `[Function: name]`. Keyed by sentinel identity, cleared when an identity is reused.
    static SENTINEL_FUNCTION_NAMES: RefCell<std::collections::HashMap<usize, String>> =
        RefCell::new(std::collections::HashMap::new());
}

fn napi_undefined_value() -> Value {
    let mut fields = indexmap::IndexMap::new();
    fields.insert(b"$__thaw_napi_undefined$".to_vec(), Value::Bool(true));
    let value = brand_internal_value(Value::shared_object(fields));
    if let Some(identity) = object_identity_key(&value) {
        SENTINEL_FUNCTION_NAMES.with(|names| { names.borrow_mut().remove(&identity); });
    }
    value
}

fn function_sentinel_value(name: &str) -> Value {
    let value = napi_undefined_value();
    if let Some(identity) = object_identity_key(&value) {
        SENTINEL_FUNCTION_NAMES.with(|names| { names.borrow_mut().insert(identity, name.to_string()); });
    }
    value
}

fn sentinel_function_name(value: &Value) -> Option<String> {
    let identity = object_identity_key(value)?;
    SENTINEL_FUNCTION_NAMES.with(|names| names.borrow().get(&identity).cloned())
}

const JIT_DICTIONARY_TYPE_ERROR: &[u8] = b"\x01TypeError\x01Cannot convert undefined or null to object\0";
const JIT_DICTIONARY_DELETE_ERROR: &[u8] = b"\x01TypeError\x01Cannot delete property\0";
const JIT_DICTIONARY_ENTRIES_ERROR: &[u8] = b"\x01TypeError\x01Iterator value is not an entry object\0";
const JIT_DICTIONARY_ASSIGN_ERROR: &[u8] = b"\x01TypeError\x01Cannot assign to read only property\0";

fn jit_dictionary_error(error: *mut *const c_char, message: &'static [u8]) {
    if !error.is_null() {
        unsafe { error.write(message.as_ptr().cast()) };
    }
}

#[no_mangle]
pub extern "C" fn thaw_jit_dictionary_get(
    kind: u8,
    object: *mut Value,
    key: *const c_char,
    present: *mut u8,
    error: *mut *const c_char,
) -> f64 {
    if object.is_null() || unsafe { thaw_json_is_nullish(object) } != 0 {
        jit_dictionary_error(error, JIT_DICTIONARY_TYPE_ERROR);
        return 0.0;
    }
    let value = thaw_json_get(object, key);
    if !present.is_null() {
        let state = if is_napi_undefined(unsafe { &*value }) {
            0
        } else if matches!(unsafe { &*value }, Value::Null) {
            2
        } else {
            1
        };
        unsafe { present.write(state) };
    }
    let result = match kind % 3 {
        0 => thaw_json_as_number(value),
        1 => f64::from(thaw_json_as_bool(value)),
        2 => f64::from_bits(thaw_json_as_string(value) as usize as u64),
        _ => 0.0,
    };
    unsafe { thaw_json_destroy(value) };
    result
}

#[no_mangle]
pub extern "C" fn thaw_jit_dictionary_mutate(
    kind: u8,
    object: *mut Value,
    key: *const c_char,
    value: f64,
    error: *mut *const c_char,
) -> f64 {
    if object.is_null() || unsafe { thaw_json_is_nullish(object) } != 0 {
        jit_dictionary_error(error, JIT_DICTIONARY_TYPE_ERROR);
        return 0.0;
    }
    if kind <= 2 && !object_writable(object, &to_key(key)) {
        jit_dictionary_error(error, JIT_DICTIONARY_ASSIGN_ERROR);
        return 0.0;
    }
    match kind {
        0 => thaw_json_object_set_number(object, key, value),
        1 => thaw_json_object_set_bool(object, key, (value != 0.0).into()),
        2 => thaw_json_object_set_string(object, key, value.to_bits() as usize as *const c_char),
        3 => return f64::from(thaw_json_object_delete(object, key)),
        4 => {
            if thaw_json_object_delete(object, key) == 0 {
                jit_dictionary_error(error, JIT_DICTIONARY_DELETE_ERROR);
            }
            return 1.0;
        }
        _ => return 0.0,
    }
    value
}

#[no_mangle]
/// # Safety
///
/// For operations `0` through `7`, `object` must point to a valid JSON value;
/// operation `0` also requires `key` to point to a valid NUL-terminated
/// string. Operations `1` through `7` return an arena-backed array handle.
/// For operations `8` through `10`, `object` must be such an array handle.
/// For operation `11`, `object` and `key` must point to valid JSON values.
/// Operation `12` ignores both pointers and creates an empty object. Operation
/// `13` returns an object's enumerable-key count. Operation `14` interprets
/// `key` as an encoded numeric index and returns that ordered key as a string.
/// Operation `23` implements the prototype-aware `in` predicate. Errors are
/// returned as a tagged TypeError pointer through `error`.
pub unsafe extern "C" fn thaw_jit_dictionary_query(
    operation: u8,
    object: *mut Value,
    key: *const c_char,
    error: *mut *const c_char,
) -> f64 {
    if (operation <= 7 || operation == 23)
        && (object.is_null() || unsafe { thaw_json_is_nullish(object) } != 0)
    {
        jit_dictionary_error(error, JIT_DICTIONARY_TYPE_ERROR);
        return 0.0;
    }
    if matches!(operation, 8..=11) && object.is_null() {
        jit_dictionary_error(error, JIT_DICTIONARY_TYPE_ERROR);
        return 0.0;
    }
    if operation == 11 && unsafe { thaw_json_is_nullish(object) } != 0 {
        jit_dictionary_error(error, JIT_DICTIONARY_TYPE_ERROR);
        return 0.0;
    }
    match operation {
        0 => f64::from(unsafe { thaw_json_has_own(object, key) }),
        23 => {
            if unsafe { thaw_json_is_object_like(object) } == 0 {
                jit_dictionary_error(error, JIT_DICTIONARY_TYPE_ERROR);
                return 0.0;
            }
            f64::from(thaw_json_has(object, key))
        },
        1 => f64::from_bits(wrap_array_handle(unsafe { thaw_json_keys(object) }) as usize as u64),
        2 => f64::from_bits(
            wrap_array_handle(unsafe { thaw_json_number_values(object) }) as usize as u64,
        ),
        3 => f64::from_bits(
            wrap_array_handle(unsafe { thaw_json_bool_values(object) }) as usize as u64,
        ),
        4 => f64::from_bits(
            wrap_array_handle(unsafe { thaw_json_string_values(object) }) as usize as u64,
        ),
        5 => f64::from_bits(
            wrap_array_handle(unsafe { thaw_json_number_entries(object) }) as usize as u64,
        ),
        6 => f64::from_bits(
            wrap_array_handle(unsafe { thaw_json_bool_entries(object) }) as usize as u64,
        ),
        7 => f64::from_bits(
            wrap_array_handle(unsafe { thaw_json_string_entries(object) }) as usize as u64,
        ),
        8 => {
            let result = unsafe {
            thaw_json_object_from_number_entries(
                unwrap_array_handle(object.cast()),
                unwrap_array_presence(object.cast()),
            )
            };
            if thaw_json_take_from_entries_error() != 0 {
                jit_dictionary_error(error, JIT_DICTIONARY_ENTRIES_ERROR);
                unsafe { thaw_json_destroy(result) };
                return 0.0;
            }
            f64::from_bits(result as usize as u64)
        },
        9 => {
            let result = unsafe {
            thaw_json_object_from_bool_entries(
                unwrap_array_handle(object.cast()),
                unwrap_array_presence(object.cast()),
            )
            };
            if thaw_json_take_from_entries_error() != 0 {
                jit_dictionary_error(error, JIT_DICTIONARY_ENTRIES_ERROR);
                unsafe { thaw_json_destroy(result) };
                return 0.0;
            }
            f64::from_bits(result as usize as u64)
        },
        10 => {
            let result = unsafe {
            thaw_json_object_from_string_entries(
                unwrap_array_handle(object.cast()),
                unwrap_array_presence(object.cast()),
            )
            };
            if thaw_json_take_from_entries_error() != 0 {
                jit_dictionary_error(error, JIT_DICTIONARY_ENTRIES_ERROR);
                unsafe { thaw_json_destroy(result) };
                return 0.0;
            }
            f64::from_bits(result as usize as u64)
        },
        11 => {
            let result = unsafe { thaw_json_object_assign(object, key.cast()) };
            if thaw_json_take_assign_error() != 0 {
                jit_dictionary_error(error, JIT_DICTIONARY_ASSIGN_ERROR);
            }
            f64::from_bits(result as usize as u64)
        }
        12 => f64::from_bits(thaw_json_object_new() as usize as u64),
        13 => unsafe { object.as_ref() }.map_or(0.0, |value| match value {
            Value::Object(fields) => shared_object_ref(fields).len() as f64,
            _ => 0.0,
        }),
        14 => {
            let index = f64::from_bits(key as usize as u64);
            let Some(index) = (index.is_finite() && index >= 0.0 && index.fract() == 0.0)
                .then_some(index as usize)
            else {
                return 0.0;
            };
            let Some(Value::Object(fields)) = (unsafe { object.as_ref() }) else {
                return 0.0;
            };
            let Some((key, _)) = ordered_object_fields_shared(fields).into_iter().nth(index) else {
                return 0.0;
            };
            f64::from_bits(thaw_arena::owned_string(key) as usize as u64)
        }
        // Set composition/predicates (15-21): `object` is the receiver and
        // `key` carries the *other* object pointer. Only the key sets
        // matter (a JIT `Set` is a string-keyed dictionary).
        15..=18 => {
            let receiver = unsafe { object.as_ref() };
            let other = unsafe { key.cast::<Value>().as_ref() };
            let (Some(Value::Object(receiver)), Some(Value::Object(other))) = (receiver, other)
            else {
                return 0.0;
            };
            let receiver = shared_object_ref(receiver);
            let other = shared_object_ref(other);
            let mut fields = indexmap::IndexMap::new();
            match operation {
                15 => {
                    for (key, value) in receiver {
                        fields.insert(key.clone(), value.clone());
                    }
                    for (key, value) in other {
                        fields.insert(key.clone(), value.clone());
                    }
                }
                16 => {
                    for (key, value) in receiver {
                        if other.contains_key(key) {
                            fields.insert(key.clone(), value.clone());
                        }
                    }
                }
                17 => {
                    for (key, value) in receiver {
                        if !other.contains_key(key) {
                            fields.insert(key.clone(), value.clone());
                        }
                    }
                }
                _ => {
                    for (key, value) in receiver {
                        if !other.contains_key(key) {
                            fields.insert(key.clone(), value.clone());
                        }
                    }
                    for (key, value) in other {
                        if !receiver.contains_key(key) {
                            fields.insert(key.clone(), value.clone());
                        }
                    }
                }
            }
            f64::from_bits(leak(Value::shared_object(fields)) as usize as u64)
        }
        19..=21 => {
            let receiver = unsafe { object.as_ref() };
            let other = unsafe { key.cast::<Value>().as_ref() };
            let (Some(Value::Object(receiver)), Some(Value::Object(other))) = (receiver, other)
            else {
                return 0.0;
            };
            let receiver = shared_object_ref(receiver);
            let other = shared_object_ref(other);
            let result = match operation {
                19 => receiver.keys().all(|key| other.contains_key(key)),
                20 => other.keys().all(|key| receiver.contains_key(key)),
                _ => receiver.keys().all(|key| !other.contains_key(key)),
            };
            f64::from(result)
        }
        // `new Set(stringIterable)` (22): `object` is a JIT array value.
        // Its bits are either a tagged raw buffer (`buffer | 1`) or a
        // handle cell holding the buffer pointer (see thaw-jit's
        // `array_data`). Build a string-keyed dictionary from its elements.
        22 => {
            const ARRAY_RESULT_TAG: usize = 1;
            let bits = object as usize;
            let buffer = if bits & ARRAY_RESULT_TAG != 0 {
                (bits & !ARRAY_RESULT_TAG) as *const u8
            } else {
                let handle = bits as *const *const u8;
                if handle.is_null() {
                    return 0.0;
                }
                unsafe { handle.read() }
            };
            if buffer.is_null() {
                return 0.0;
            }
            let length = unsafe { buffer.cast::<i64>().read() }.max(0) as usize;
            let slots = unsafe { buffer.add(8) }.cast::<*const std::ffi::c_char>();
            let mut fields = indexmap::IndexMap::new();
            for index in 0..length {
                let pointer = unsafe { slots.add(index).read() };
                let key = if pointer.is_null() { Vec::new() } else { to_key(pointer) };
                fields.insert(key.clone(), utf8_or_wtf8(key));
            }
            f64::from_bits(leak(Value::shared_object(fields)) as usize as u64)
        }
        _ => 0.0,
    }
}

#[no_mangle]
pub extern "C" fn thaw_json_array_new() -> *mut Value {
    leak(Value::shared_array(Vec::new()))
}

#[no_mangle]
/// # Safety
///
/// `value` must be null or point to a valid JSON `Value`.
pub unsafe extern "C" fn thaw_json_is_array(value: *const Value) -> u8 {
    match unsafe { value.as_ref() } {
        Some(Value::Host(lease)) => u8::from(host_query(lease, 4).as_deref() == Some("1")),
        Some(value) => u8::from(matches!(value, Value::Array(_))),
        None => 0,
    }
}

/// Native array presence uses null for dense arrays and a `[length][state...]`
/// mask otherwise. State zero is a hole; states one and two are present.
fn native_array_slot_present(presence: *const u8, index: usize) -> bool {
    if presence.is_null() {
        return true;
    }
    let mask_len = unsafe { presence.cast::<u64>().read() as usize };
    index >= mask_len || unsafe { presence.add(8 + index).read() } != 0
}

fn flatten_json_value(value: Value, depth: usize, out: &mut Vec<Value>) {
    match value {
        Value::Array(items) if depth > 0 => {
            let elements = shared_array_ref(&items);
            for (index, item) in elements.iter().enumerate() {
                if array_has_index(&items, index) {
                    flatten_json_value(item.clone(), depth - 1, out);
                }
            }
        }
        other => out.push(other),
    }
}

#[no_mangle]
/// `any[].flat(depth)` -- whether a `Json`-typed array element is itself
/// an array is only knowable at runtime, unlike a statically nested
/// `T[][]` (see `lower_array_flat_one`, thaw-hir), so this can't be
/// expressed as a compile-time unwrap-and-copy loop the way the concrete
/// case is. Only the array's own top level is the native `[len][Json
/// ptr...]` buffer -- everything nested is already a plain
/// `Value::Array` inside the boxed JSON tree, so flattening past the
/// first level is really just walking `serde_json::Value`.
///
/// # Safety
/// `array` must be null or point to a valid native `[length][Json
/// ptr...]` buffer (every slot a `*const Value`, `HirType::Json`'s
/// native array element width). `presence` must be null or point to the
/// matching native `[length][state...]` mask.
pub unsafe extern "C" fn thaw_any_array_flat(array: *const u8, presence: *const u8, depth: f64) -> *mut u8 {
    let depth = if depth.is_nan() || depth <= 0.0 {
        0
    } else if depth == f64::INFINITY {
        usize::MAX
    } else {
        depth as usize
    };
    let length = if array.is_null() {
        0
    } else {
        unsafe { array.cast::<u64>().read() as usize }
    };
    let mut out = Vec::with_capacity(length);
    for index in 0..length {
        if !native_array_slot_present(presence, index) {
            continue;
        }
        let element = unsafe {
            array
                .add(8 + index * 8)
                .cast::<*const Value>()
                .read_unaligned()
        };
        let value = if element.is_null() {
            Value::Null
        } else {
            unsafe { (*element).clone() }
        };
        flatten_json_value(value, depth, &mut out);
    }
    let buffer = thaw_arena::thaw_arena_alloc(8 + out.len() * 8, 8);
    unsafe {
        buffer.cast::<u64>().write(out.len() as u64);
        for (index, value) in out.into_iter().enumerate() {
            buffer
                .add(8 + index * 8)
                .cast::<*const Value>()
                .write_unaligned(leak(value));
        }
    }
    buffer
}

#[no_mangle]
/// `Buffer.isBuffer(value)` for a `Json`-typed operand -- recognizes
/// the same `{"type":"Buffer","data":[...]}` shape `__thaw_json_
/// binary_replacer` (thaw-quickjs) produces for a live Buffer/
/// TypedArray crossing into JSON, mirroring `json_array_or_buffer_
/// data`'s own check below (kept separate rather than shared, since
/// that one also needs to unwrap a plain array, which this doesn't).
///
/// # Safety
///
/// `value` must be null or point to a valid JSON `Value`.
pub unsafe extern "C" fn thaw_json_is_buffer(value: *const Value) -> u8 {
    let Some(value) = (unsafe { value.as_ref() }) else {
        return 0;
    };
    if let Value::Host(lease) = value {
        return u8::from(host_query(lease, 11).as_deref() == Some("1"));
    }
    let Some(fields) = value.as_object() else {
        return 0;
    };
    u8::from(
        fields.get(b"type".as_slice()).and_then(Value::as_str) == Some("Buffer")
            && fields.get(b"data".as_slice()).is_some_and(Value::is_array),
    )
}

#[no_mangle]
/// # Safety
///
/// `value` must be null or point to a valid JSON `Value`.
pub unsafe extern "C" fn thaw_json_is_null(value: *const Value) -> u8 {
    match unsafe { value.as_ref() } {
        Some(Value::Host(lease)) => u8::from(host_query(lease, 7).as_deref() == Some("1")),
        Some(value) => u8::from(value.is_null()),
        None => 0,
    }
}

#[no_mangle]
/// Backs `typeof json === "undefined"`/strict `=== undefined` for a
/// `Json` value -- the napi-undefined sentinel
/// (`napi_undefined_value`), checked by `is_napi_undefined`'s exact
/// value test (the sentinel key must map to `true`, not merely be
/// present), matching the Rust-side predicate every other consumer uses.
///
/// # Safety
///
/// `value` must be null or point to a valid JSON `Value`.
pub unsafe extern "C" fn thaw_json_is_undefined(value: *const Value) -> u8 {
    if value.is_null() {
        return 0;
    }
    if let Value::Host(lease) = unsafe { &*value } {
        return u8::from(host_query(lease, 5).as_deref() == Some("1"));
    }
    u8::from(is_napi_undefined(unsafe { &*value }))
}

#[no_mangle]
/// Backs `json == null`/`json == undefined` (loose equality, which in
/// real JS means "is either `null` or `undefined`, full stop" -- no
/// other coercion applies). Real `null` and a genuinely-missing key/
/// index (now the napi-undefined sentinel, see `napi_undefined_value`)
/// both count.
///
/// # Safety
///
/// `value` must be null or point to a valid JSON `Value`.
pub unsafe extern "C" fn thaw_json_is_nullish(value: *const Value) -> u8 {
    if value.is_null() {
        return 0;
    }
    let value = unsafe { &*value };
    if let Value::Host(lease) = value {
        return u8::from(host_query(lease, 6).as_deref() == Some("1"));
    }
    (matches!(value, Value::Null) || is_napi_undefined(value)).into()
}

#[no_mangle]
/// # Safety
/// `value` must be null or point to a valid JSON `Value`.
pub unsafe extern "C" fn thaw_json_is_object_like(value: *const Value) -> u8 {
    match unsafe { value.as_ref() } {
        Some(Value::Host(lease)) => u8::from(host_query(lease, 9).as_deref() == Some("1")),
        Some(Value::Array(_)) => 1,
        Some(value @ Value::Object(_)) => {
            u8::from(!is_napi_undefined(value) && non_finite_number(value).is_none())
        }
        _ => 0,
    }
}

fn alloc_pointer_array(values: Vec<*mut u8>) -> *mut u8 {
    let output = thaw_arena::thaw_arena_alloc(8 + values.len() * 8, 8);
    if output.is_null() {
        return output;
    }
    unsafe { (output as *mut i64).write(values.len() as i64) };
    for (index, value) in values.into_iter().enumerate() {
        unsafe { (output.add(8 + index * 8) as *mut *mut u8).write(value) };
    }
    output
}

fn enumerable_entries(value: &Value) -> Vec<(Vec<u8>, Value)> {
    match value {
        Value::Host(lease) => host_enumerable_entries(lease),
        Value::Object(fields) => ordered_object_fields_shared(fields)
            .into_iter().map(|(key, value)| (key.clone(), value.clone())).collect(),
        Value::Array(items) => shared_array_ref(items).iter().enumerate()
            .filter(|(index, _)| array_has_index(items, *index))
            .map(|(index, value)| (index.to_string().into_bytes(), value.clone())).collect(),
        Value::String(text) => text.encode_utf16().enumerate()
            .map(|(index, unit)| (index.to_string().into_bytes(), utf16_unit_value(unit))).collect(),
        Value::Wtf8(bytes) => wtf8_decode_utf16(bytes).into_iter().enumerate()
            .map(|(index, unit)| (index.to_string().into_bytes(), utf16_unit_value(unit))).collect(),
        _ => Vec::new(),
    }
}

/// `Object.keys`/`values`/`entries` see string keys only; symbol keys are stored
/// as `\u{1f}@@name` sentinel keys.
fn enumerable_string_entries(value: &Value) -> Vec<(Vec<u8>, Value)> {
    let mut entries = enumerable_entries(value);
    entries.retain(|(key, _)| !key.starts_with(b"\x1f@@"));
    entries
}

fn utf16_unit_value(unit: u16) -> Value {
    let bytes = wtf8_encode_utf16(&[unit]);
    match String::from_utf8(bytes) {
        Ok(text) => Value::String(text),
        Err(error) => Value::Wtf8(error.into_bytes()),
    }
}

/// Wraps a freshly built native `[length][elem...]` array/tuple `buffer`
/// (a `[string, T]` entry pair, here) in a one-word "handle" cell, matching
/// thaw-llvm's `compile_array_wrap` -- every `Array`/`Tuple` value is a
/// handle now, including one built entirely in Rust like an
/// `Object.entries` pair, since it becomes an *element* of the outer
/// entries array and gets indexed back out expecting a handle. Returns
/// null if `buffer` is null (propagating an earlier allocation failure)
/// or if the handle's own allocation fails.
fn wrap_array_handle(buffer: *mut u8) -> *mut u8 {
    if buffer.is_null() {
        return std::ptr::null_mut();
    }
    let handle = thaw_arena::thaw_arena_alloc(16, 8);
    if handle.is_null() {
        return std::ptr::null_mut();
    }
    unsafe {
        (handle as *mut *mut u8).write(buffer);
        (handle.add(8) as *mut *mut u8).write(std::ptr::null_mut());
    }
    handle
}

/// Loads the current raw buffer pointer out of an array/tuple handle. See
/// `wrap_array_handle`. Returns null if `handle` itself is null.
///
/// # Safety
/// `handle` must be null or a pointer written by `wrap_array_handle` (or
/// thaw-llvm's `compile_array_wrap`).
unsafe fn unwrap_array_handle(handle: *const u8) -> *const u8 {
    if handle.is_null() {
        return std::ptr::null();
    }
    unsafe { (handle as *const *const u8).read() }
}

unsafe fn unwrap_array_presence(handle: *const u8) -> *const u8 {
    if handle.is_null() {
        return std::ptr::null();
    }
    unsafe { (handle.add(8) as *const *const u8).read() }
}

#[no_mangle]
/// # Safety
///
/// `value` must be null or point to a valid JSON `Value`.
pub unsafe extern "C" fn thaw_json_keys(value: *const Value) -> *mut u8 {
    if let Some(Value::Host(lease)) = unsafe { value.as_ref() } {
        return alloc_pointer_array(host_enumerate_values(lease, 2).iter()
            .map(|key| native_string_value(key).cast()).collect());
    }
    let keys = unsafe { value.as_ref() }.map(enumerable_string_entries).unwrap_or_default()
        .into_iter().map(|(key, _)| key).collect::<Vec<_>>();
    alloc_pointer_array(
        keys.into_iter()
            .map(|key| thaw_arena::owned_string(key).cast())
            .collect(),
    )
}

#[no_mangle]
/// # Safety
/// `value` must be null or point to a valid JSON `Value`.
pub unsafe extern "C" fn thaw_json_own_keys(value: *const Value) -> *mut u8 {
    if let Some(Value::Host(lease)) = unsafe { value.as_ref() } {
        return alloc_pointer_array(host_enumerate_values(lease, 1).into_iter()
            .map(|key| Box::into_raw(Box::new(key)).cast()).collect());
    }
    let keys: Vec<Value> = match unsafe { value.as_ref() } {
        Some(Value::Array(items)) => (0..shared_array_ref(items).len())
            .filter(|index| array_has_index(items, *index))
            .map(|index| Value::String(index.to_string()))
            .chain(std::iter::once(Value::String("length".to_string())))
            .collect(),
        Some(value) => enumerable_entries(value).into_iter()
            .map(|(key, _)| Value::Wtf8(key)).collect(),
        None => Vec::new(),
    };
    alloc_pointer_array(keys.into_iter()
        .map(|key| Box::into_raw(Box::new(key)).cast()).collect())
}

#[no_mangle]
/// # Safety
///
/// `value` must be null or point to a valid JSON `Value`.
pub unsafe extern "C" fn thaw_json_values(value: *const Value) -> *mut u8 {
    if let Some(Value::Host(lease)) = unsafe { value.as_ref() } {
        return alloc_pointer_array(host_enumerate_values(lease, 3).into_iter()
            .map(|value| Box::into_raw(Box::new(value)).cast()).collect());
    }
    let values = unsafe { value.as_ref() }.map(enumerable_string_entries).unwrap_or_default()
        .into_iter().map(|(_, value)| value).collect::<Vec<_>>();
    alloc_pointer_array(
        values
            .into_iter()
            .map(|value| Box::into_raw(Box::new(value)).cast())
            .collect(),
    )
}

fn object_values(value: *const Value) -> Vec<Value> {
    match unsafe { value.as_ref() } {
        Some(Value::Host(lease)) => host_enumerate_values(lease, 3),
        Some(Value::Object(fields)) => ordered_object_fields_shared(fields)
            .into_iter()
            .map(|(_, value)| value.clone())
            .collect(),
        _ => Vec::new(),
    }
}

fn alloc_scalar_array(
    values: &[Value],
    element_bytes: usize,
    write: impl Fn(*mut u8, &Value),
) -> *mut u8 {
    let output =
        thaw_arena::thaw_arena_alloc(8 + values.len() * element_bytes, element_bytes.min(8));
    if output.is_null() {
        return output;
    }
    unsafe { (output as *mut i64).write(values.len() as i64) };
    for (index, value) in values.iter().enumerate() {
        write(unsafe { output.add(8 + index * element_bytes) }, value);
    }
    output
}

#[no_mangle]
/// # Safety
///
/// `value` must be null or point to a valid JSON object containing numbers.
pub unsafe extern "C" fn thaw_json_number_values(value: *const Value) -> *mut u8 {
    alloc_scalar_array(&object_values(value), 8, |slot, value| unsafe {
        (slot as *mut f64).write(if matches!(value, Value::Host(_) | Value::BigInt(_)) {
            json_to_number(value)
        } else {
            non_finite_number(value).or_else(|| value.as_f64()).unwrap_or(0.0)
        });
    })
}

fn native_string_value(value: &Value) -> *mut c_char {
    match value {
        Value::String(text) => thaw_arena::owned_string(text),
        Value::Wtf8(bytes) => thaw_arena::owned_string(bytes),
        Value::BigInt(decimal) => thaw_arena::owned_string(decimal),
        Value::Host(_) => thaw_json_as_string(value as *const Value as *mut Value).cast_mut(),
        _ => thaw_arena::owned_string(b""),
    }
}

#[no_mangle]
/// # Safety
///
/// `value` must be null or point to a valid JSON object containing strings.
pub unsafe extern "C" fn thaw_json_string_values(value: *const Value) -> *mut u8 {
    alloc_scalar_array(&object_values(value), 8, |slot, value| unsafe {
        (slot as *mut *mut u8).write(native_string_value(value).cast());
    })
}

#[no_mangle]
/// # Safety
///
/// `value` must be null or point to a valid JSON object containing booleans.
pub unsafe extern "C" fn thaw_json_bool_values(value: *const Value) -> *mut u8 {
    alloc_scalar_array(&object_values(value), 1, |slot, value| unsafe {
        slot.write(if matches!(value, Value::Host(_) | Value::BigInt(_)) {
            thaw_json_as_bool(value as *const Value as *mut Value)
        } else {
            value.as_bool().unwrap_or(false).into()
        });
    })
}

#[no_mangle]
/// # Safety
///
/// `value` must be null or point to a valid JSON `Value`.
pub unsafe extern "C" fn thaw_json_entries(value: *const Value) -> *mut u8 {
    let entries = unsafe { value.as_ref() }.map(enumerable_string_entries).unwrap_or_default();
    alloc_pointer_array(
        entries
            .into_iter()
            .map(|(key, value)| {
                wrap_array_handle(alloc_pointer_array(vec![
                    thaw_arena::owned_string(key).cast(),
                    Box::into_raw(Box::new(value)).cast(),
                ]))
            })
            .collect(),
    )
}

fn alloc_typed_entries(value: *const Value, write: impl Fn(*mut u8, &Value)) -> *mut u8 {
    let entries = match unsafe { value.as_ref() } {
        Some(Value::Object(fields)) => ordered_object_fields_shared(fields),
        _ => Vec::new(),
    };
    alloc_pointer_array(
        entries
            .into_iter()
            .map(|(key, value)| {
                let entry = thaw_arena::thaw_arena_alloc(24, 8);
                if entry.is_null() {
                    return entry;
                }
                unsafe {
                    (entry as *mut i64).write(2);
                    (entry.add(8) as *mut *mut u8)
                        .write(thaw_arena::owned_string(key).cast());
                }
                write(unsafe { entry.add(16) }, value);
                wrap_array_handle(entry)
            })
            .collect(),
    )
}

#[no_mangle]
/// # Safety
///
/// `value` must be null or point to a valid JSON object containing numbers.
pub unsafe extern "C" fn thaw_json_number_entries(value: *const Value) -> *mut u8 {
    alloc_typed_entries(value, |slot, value| unsafe {
        (slot as *mut f64).write(non_finite_number(value).or_else(|| value.as_f64()).unwrap_or(0.0));
    })
}

#[no_mangle]
/// # Safety
///
/// `value` must be null or point to a valid JSON object containing strings.
pub unsafe extern "C" fn thaw_json_string_entries(value: *const Value) -> *mut u8 {
    alloc_typed_entries(value, |slot, value| unsafe {
        (slot as *mut *mut u8).write(native_string_value(value).cast());
    })
}

#[no_mangle]
/// # Safety
///
/// `value` must be null or point to a valid JSON object containing booleans.
pub unsafe extern "C" fn thaw_json_bool_entries(value: *const Value) -> *mut u8 {
    alloc_typed_entries(value, |slot, value| unsafe {
        slot.write(value.as_bool().unwrap_or(false).into());
    })
}

fn object_from_typed_entries(
    entries: *const u8,
    presence: *const u8,
    read: impl Fn(*const u8) -> Value,
) -> *mut Value {
    FROM_ENTRIES_ERROR.with(|error| error.set(false));
    let mut object = indexmap::IndexMap::new();
    if entries.is_null() {
        return leak(Value::shared_object(object));
    }
    let length = unsafe { (entries as *const i64).read() }.max(0) as usize;
    for index in 0..length {
        if !presence.is_null() {
            let mask_len = unsafe { (presence as *const u64).read() as usize };
            if index < mask_len && unsafe { presence.add(8 + index).read() } == 0 {
                FROM_ENTRIES_ERROR.with(|error| error.set(true));
                break;
            }
        }
        let handle = unsafe { (entries.add(8 + index * 8) as *const *const u8).read() };
        let entry = unsafe { unwrap_array_handle(handle) };
        if entry.is_null() {
            FROM_ENTRIES_ERROR.with(|error| error.set(true));
            break;
        }
        if unsafe { (entry as *const i64).read() } < 2 {
            FROM_ENTRIES_ERROR.with(|error| error.set(true));
            break;
        }
        let key = unsafe { (entry.add(8) as *const *const c_char).read() };
        if key.is_null() {
            FROM_ENTRIES_ERROR.with(|error| error.set(true));
            break;
        }
        object.insert(to_key(key), read(unsafe { entry.add(16) }));
    }
    leak(Value::shared_object(object))
}

#[no_mangle]
/// # Safety
///
/// `entries` must be null or point to a native `[string, number][]` array.
pub unsafe extern "C" fn thaw_json_object_from_number_entries(entries: *const u8, presence: *const u8) -> *mut Value {
    object_from_typed_entries(entries, presence, |slot| {
        let value = unsafe { (slot as *const f64).read() };
        number_value(value)
    })
}

#[no_mangle]
/// # Safety
///
/// `entries` must be null or point to a native `[string, string][]` array.
pub unsafe extern "C" fn thaw_json_object_from_string_entries(entries: *const u8, presence: *const u8) -> *mut Value {
    object_from_typed_entries(entries, presence, |slot| {
        let value = unsafe { (slot as *const *const c_char).read() };
        string_value(value)
    })
}

#[no_mangle]
/// # Safety
///
/// `entries` must be null or point to a native `[string, boolean][]` array.
pub unsafe extern "C" fn thaw_json_object_from_bool_entries(entries: *const u8, presence: *const u8) -> *mut Value {
    object_from_typed_entries(entries, presence, |slot| Value::Bool(unsafe { slot.read() } != 0))
}

#[no_mangle]
/// # Safety
///
/// `entries` must be null or point to a native `[string, Json][]` array.
pub unsafe extern "C" fn thaw_json_object_from_json_entries(entries: *const u8, presence: *const u8) -> *mut Value {
    object_from_typed_entries(entries, presence, |slot| {
        let value = unsafe { (slot as *const *const Value).read() };
        unsafe { value.as_ref() }.cloned().unwrap_or(Value::Null)
    })
}

#[no_mangle]
/// # Safety
///
/// `value` must be null or point to a valid JSON `Value`; `key` must point to
/// a valid NUL-terminated string.
pub unsafe extern "C" fn thaw_json_has_own(value: *const Value, key: *const c_char) -> u8 {
    let key = to_key(key);
    match unsafe { value.as_ref() } {
        Some(Value::Host(lease)) => u8::from(host_property_predicate(lease, &key, 1)),
        Some(value) => json_has_own_value(value, &key).into(),
        None => 0,
    }
}

#[no_mangle]
/// # Safety
///
/// `value` must be null or point to a valid JSON `Value`; `key` must point to
/// a valid NUL-terminated string.
pub unsafe extern "C" fn thaw_json_property_is_enumerable(value: *const Value, key: *const c_char) -> u8 {
    let key = to_key(key);
    match unsafe { value.as_ref() } {
        Some(Value::Host(lease)) => u8::from(host_property_predicate(lease, &key, 3)),
        Some(Value::Array(_)) if key == b"length" => 0,
        Some(Value::String(text)) => u8::from(array_index_key(&key)
            .is_some_and(|index| (index as usize) < text.encode_utf16().count())),
        Some(Value::Wtf8(bytes)) => u8::from(array_index_key(&key)
            .is_some_and(|index| (index as usize) < wtf8_decode_utf16(bytes).len())),
        Some(value) => json_has_own_value(value, &key).into(),
        None => 0,
    }
}

fn json_number_is(left: f64, right: f64) -> bool {
    (left.is_nan() && right.is_nan()) || (left == right && left.to_bits() == right.to_bits())
}

fn string_bytes(value: &Value) -> Option<&[u8]> {
    match value {
        Value::String(text) => Some(text.as_bytes()),
        Value::Wtf8(bytes) => Some(bytes.as_slice()),
        _ => None,
    }
}

fn same_string(left: &Value, right: &Value) -> Option<bool> {
    Some(string_bytes(left)? == string_bytes(right)?)
}

#[no_mangle]
/// # Safety
///
/// Both operands must be null or point to valid JSON `Value`s.
pub unsafe extern "C" fn thaw_json_object_is(left: *const Value, right: *const Value) -> u8 {
    if let (Some(left), Some(right)) = (unsafe { left.as_ref() }, unsafe { right.as_ref() }) {
        if is_napi_undefined(left) || is_napi_undefined(right) {
            return u8::from(is_napi_undefined(left) && is_napi_undefined(right));
        }
        let left_number = non_finite_number(left);
        let right_number = non_finite_number(right);
        if left_number.is_some() || right_number.is_some() {
            return u8::from(left_number.zip(right_number)
                .is_some_and(|(left, right)| json_number_is(left, right)));
        }
    }
    let result = match (unsafe { left.as_ref() }, unsafe { right.as_ref() }) {
        (Some(Value::Null), Some(Value::Null)) => true,
        (Some(Value::Bool(left)), Some(Value::Bool(right))) => left == right,
        (Some(Value::String(left)), Some(Value::String(right))) => left == right,
        (Some(Value::Wtf8(_)), Some(Value::Wtf8(_)))
        | (Some(Value::String(_)), Some(Value::Wtf8(_)))
        | (Some(Value::Wtf8(_)), Some(Value::String(_))) => {
            same_string(unsafe { &*left }, unsafe { &*right }).unwrap_or(false)
        }
        (Some(Value::BigInt(left)), Some(Value::BigInt(right))) => left == right,
        (Some(Value::Number(left)), Some(Value::Number(right))) => left
            .as_f64()
            .zip(right.as_f64())
            .is_some_and(|(left, right)| json_number_is(left, right)),
        (Some(Value::Array(left)), Some(Value::Array(right))) => Rc::ptr_eq(left, right),
        (Some(Value::Object(left)), Some(Value::Object(right))) => Rc::ptr_eq(left, right),
        (Some(Value::Host(left)), Some(Value::Host(right))) => left.handle == right.handle,
        _ => false,
    };
    result.into()
}

/// `===` between two dynamic (`Json`-typed) values -- real ECMAScript
/// Strict Equality Comparison: `undefined`/`null` compare by their own
/// kind, a number/string/boolean by value (a real `NaN` is unequal to
/// itself, matching spec, since it's excluded before the `Number` arm
/// below), and an array/object by real reference identity (`Rc::ptr_eq`
/// on the shared container `Value`'s own doc comment describes) --
/// *not* by structurally comparing their fields, so two separately-
/// allocated `Json` values that happen to hold identical array/object
/// content stay unequal, matching real `{} === {}` being `false`.
/// Exported from here (not thaw-runtime, `.indexOf()`/`.includes()`'s own
/// crate) because only this crate's `Value` has real access to the
/// shared container's identity -- thaw-runtime has no Cargo dependency
/// on this crate (each of thaw-arena/thaw-runtime/thaw-std is built as
/// its own independent static archive and only combined at the final
/// system-link step, so a real Rust-level dependency between them would
/// duplicate every `#[no_mangle]` symbol they share), and calls this the
/// same "resolved at link time" way `thaw_string_to_number`/
/// `thaw_date_to_iso_string` already are.
///
/// # Safety
/// `a` and `b` must each be null or point to a valid JSON `Value`.
#[no_mangle]
pub unsafe extern "C" fn thaw_json_strict_equal(a: *const Value, b: *const Value) -> u8 {
    let (Some(a_val), Some(b_val)) = (unsafe { a.as_ref() }, unsafe { b.as_ref() }) else {
        return u8::from(std::ptr::eq(a, b));
    };
    let a_undefined = is_napi_undefined(a_val);
    let b_undefined = is_napi_undefined(b_val);
    if a_undefined || b_undefined {
        return u8::from(a_undefined && b_undefined);
    }
    let a_non_finite = non_finite_number(a_val);
    let b_non_finite = non_finite_number(b_val);
    if a_non_finite.is_some() || b_non_finite.is_some() {
        return u8::from(a_non_finite.is_some() && a_non_finite == b_non_finite);
    }
    u8::from(match (a_val, b_val) {
        (Value::Null, Value::Null) => true,
        (Value::Bool(x), Value::Bool(y)) => x == y,
        (Value::BigInt(x), Value::BigInt(y)) => x == y,
        (Value::Number(x), Value::Number(y)) => x.as_f64() == y.as_f64(),
        (Value::String(x), Value::String(y)) => x == y,
        (Value::Wtf8(_), Value::Wtf8(_))
        | (Value::String(_), Value::Wtf8(_))
        | (Value::Wtf8(_), Value::String(_)) => same_string(a_val, b_val).unwrap_or(false),
        (Value::Array(x), Value::Array(y)) => Rc::ptr_eq(x, y),
        (Value::Object(x), Value::Object(y)) => Rc::ptr_eq(x, y),
        (Value::Host(x), Value::Host(y)) => x.handle == y.handle,
        _ => false,
    })
}

/// `SameValueZero` between two dynamic (`Json`-typed) values -- used for
/// `Map<any, V>`/`Set<any>` key equality (thaw-runtime's `maps.rs`'s
/// `AnyKey`), unlike `thaw_json_strict_equal` (`===`): the one real
/// difference is `NaN`, which `SameValueZero` treats as equal to itself.
///
/// # Safety
/// `a` and `b` must each be null or point to a valid JSON `Value`.
#[no_mangle]
pub unsafe extern "C" fn thaw_json_same_value_zero(a: *const Value, b: *const Value) -> u8 {
    let (Some(a_val), Some(b_val)) = (unsafe { a.as_ref() }, unsafe { b.as_ref() }) else {
        return u8::from(std::ptr::eq(a, b));
    };
    let a_undefined = is_napi_undefined(a_val);
    let b_undefined = is_napi_undefined(b_val);
    if a_undefined || b_undefined {
        return u8::from(a_undefined && b_undefined);
    }
    let a_non_finite = non_finite_number(a_val);
    let b_non_finite = non_finite_number(b_val);
    if a_non_finite.is_some() || b_non_finite.is_some() {
        return u8::from(match (a_non_finite, b_non_finite) {
            (Some(x), Some(y)) if x.is_nan() && y.is_nan() => true,
            (Some(x), Some(y)) => x == y,
            _ => false,
        });
    }
    u8::from(match (a_val, b_val) {
        (Value::Null, Value::Null) => true,
        (Value::Bool(x), Value::Bool(y)) => x == y,
        (Value::BigInt(x), Value::BigInt(y)) => x == y,
        (Value::Number(x), Value::Number(y)) => x.as_f64() == y.as_f64(),
        (Value::String(x), Value::String(y)) => x == y,
        (Value::Wtf8(_), Value::Wtf8(_))
        | (Value::String(_), Value::Wtf8(_))
        | (Value::Wtf8(_), Value::String(_)) => same_string(a_val, b_val).unwrap_or(false),
        (Value::Array(x), Value::Array(y)) => Rc::ptr_eq(x, y),
        (Value::Object(x), Value::Object(y)) => Rc::ptr_eq(x, y),
        (Value::Host(x), Value::Host(y)) => x.handle == y.handle,
        _ => false,
    })
}

/// The stable identity key for `value`'s own shared container (see
/// `object_identity_key`), or `0` for a scalar/null pointer. Used by
/// thaw-runtime's `AnyKey` (`maps.rs`) as the hash for a `Map<any, V>`/
/// `Set<any>` key that's itself an Array/Object -- it has to track
/// `thaw_json_same_value_zero`'s own real-reference-identity notion, not
/// the raw outer pointer (a fresh `leak`ed wrapper on every read, see
/// `Value`'s own doc comment), or two aliases of the same shared value
/// would hash unequally despite comparing equal, breaking that hash
/// table's own equal-keys-hash-equally invariant.
///
/// # Safety
/// `value` must be null or point to a valid JSON `Value`.
#[no_mangle]
pub unsafe extern "C" fn thaw_json_identity_key(value: *const Value) -> u64 {
    (unsafe { value.as_ref() })
        .and_then(object_identity_key)
        .map(|key| key as u64)
        .unwrap_or(0)
}

#[no_mangle]
/// # Safety
///
/// `value` must be null or point to a valid JSON `Value`.
pub unsafe extern "C" fn thaw_json_object_is_number(value: *const Value, other: f64) -> u8 {
    unsafe { value.as_ref() }
        .and_then(|value| non_finite_number(value).or_else(|| value.as_f64()))
        .is_some_and(|value| json_number_is(value, other))
        .into()
}

#[no_mangle]
/// # Safety
///
/// `value` must be null or point to a valid JSON `Value`; `other` must point
/// to a valid NUL-terminated string.
pub unsafe extern "C" fn thaw_json_object_is_string(
    value: *const Value,
    other: *const c_char,
) -> u8 {
    unsafe { value.as_ref() }
        .and_then(string_bytes)
        .is_some_and(|value| value == unsafe { CStr::from_ptr(other) }.to_bytes())
        .into()
}

#[no_mangle]
/// # Safety
///
/// `value` must be null or point to a valid JSON `Value`.
pub unsafe extern "C" fn thaw_json_object_is_bool(value: *const Value, other: bool) -> u8 {
    unsafe { value.as_ref() }
        .and_then(Value::as_bool)
        .is_some_and(|value| value == other)
        .into()
}

#[no_mangle]
pub extern "C" fn thaw_json_array_push_number(array: *mut Value, value: f64) {
    if let Some(items) = (unsafe { array.as_ref() }).and_then(Value::as_array_mut) {
        items.push(number_value(value));
    }
}

#[no_mangle]
pub extern "C" fn thaw_json_array_push_string(array: *mut Value, value: *const c_char) {
    if let Some(items) = (unsafe { array.as_ref() }).and_then(Value::as_array_mut) {
        items.push(string_value(value));
    }
}

#[no_mangle]
pub extern "C" fn thaw_json_array_push_bool(array: *mut Value, value: u8) {
    if let Some(items) = (unsafe { array.as_ref() }).and_then(Value::as_array_mut) {
        items.push(Value::Bool(value != 0));
    }
}

#[no_mangle]
pub extern "C" fn thaw_json_array_push_hole(array: *mut Value) {
    if let Some(Value::Array(items)) = unsafe { array.as_ref() } {
        let index = shared_array_ref(items).len();
        shared_array_ref_mut(items).push(napi_undefined_value());
        mark_array_holes(items, std::iter::once(index));
    }
}

#[no_mangle]
pub extern "C" fn thaw_json_array_push_json(array: *mut Value, value: *mut Value) {
    let Some(value) = (unsafe { value.as_ref() }).cloned() else {
        return;
    };
    if let Some(items) = (unsafe { array.as_ref() }).and_then(Value::as_array_mut) {
        items.push(value);
    }
}

#[no_mangle]
pub extern "C" fn thaw_json_receiver_bigint(value: i64) -> *mut Value {
    leak(Value::BigInt(value.to_string()))
}

#[no_mangle]
pub extern "C" fn thaw_json_receiver_bool(value: u8) -> *mut Value {
    leak(Value::Bool(value != 0))
}

#[no_mangle]
pub extern "C" fn thaw_json_receiver_number(value: f64) -> *mut Value {
    leak(number_value(value))
}

#[no_mangle]
pub extern "C" fn thaw_json_receiver_string(value: *const c_char) -> *mut Value {
    leak(string_value(value))
}

#[no_mangle]
pub extern "C" fn thaw_json_null() -> *mut Value {
    leak(Value::Null)
}

/// A genuine, safe-to-dereference `Json` value representing JS
/// `undefined` (the same `$__thaw_napi_undefined$`-tagged sentinel
/// object `typeof`/`JSON.stringify`/etc. already recognize throughout
/// this module) -- distinct from `thaw_json_null()`. Used for a
/// `Json`-typed array slot whose presence state says "undefined" (see
/// `__thaw_array_set_undefined`, thaw-llvm's `invocations/calls.rs`):
/// that codegen only updates the presence bitmap, so a plain-`Json`-
/// element array previously left the slot's own bytes at whatever the
/// arena allocation zero-initialized them to -- a null pointer a raw
/// (non-state-aware) reader like `.map()`'s fast element-read path, or
/// ordinary `arr[i]` indexing on this element shape, then dereferenced
/// directly, crashing.
#[no_mangle]
pub extern "C" fn thaw_json_undefined() -> *mut Value {
    leak(napi_undefined_value())
}

#[no_mangle]
pub extern "C" fn thaw_json_from_number_array(array: *const u8) -> *mut Value {
    if array.is_null() {
        return leak(Value::shared_array(Vec::new()));
    }
    let length = unsafe { (array as *const i64).read() }.max(0) as usize;
    let values = (0..length)
        .map(|index| {
            let value = unsafe { (array.add(8 + index * 8) as *const f64).read() };
            number_value(value)
        })
        .collect();
    leak(Value::shared_array(values))
}

/// `value` as a plain JSON array, *or* -- if it's shaped
/// `{"type":"Buffer","data":[...]}` (`Buffer.prototype.toJSON()`'s own
/// real shape, and what `__thaw_json_binary_replacer`, thaw-quickjs,
/// produces whenever a live Buffer/TypedArray crosses into a native
/// callback's JSON-encoded arguments) -- its `data` field instead. A
/// native callback parameter declared `Buffer`/`Uint8Array` has already
/// been erased to a plain `Array(F64)` by lowering time
/// (`HirType::Bytes` is a lowering-only distinction, gone by the time
/// any of this runs), so nothing here can tell from the *declared* type
/// alone that the real value is a Buffer -- only the JSON shape itself
/// says so, the same way `__thaw_json_date_reviver` (thaw-quickjs, the
/// JS-side counterpart for a value flowing the *other* direction)
/// recognizes it purely structurally too. Without this, a real Buffer
/// argument reaching a native callback silently decoded as an empty
/// array (`thaw_json_array_length`/`thaw_json_index` both treat a plain
/// JSON object as "not an array" and fall back to their own empty/
/// key-lookup defaults) -- no error, just missing data (found via a real
/// `req.pipe(nativeCallbackDestination)`, busboy's own `defaultStreamHandler`).
/// `.source`/`.flags`/`.lastIndex`/`.global`/etc. read directly on a
/// `RegExp` value stored in an `any`-typed slot -- the wrapper
/// `is_thaw_internal_wrapper` recognizes for `JSON.stringify` also
/// makes every other read of the value opaque, since nothing else in
/// the codebase unwraps `__thaw_regexp__` outside the one place that
/// crosses back into real QuickJS (`__thaw_json_date_reviver`,
/// `platform_globals/dates.js`) -- direct native-compiled property
/// access on an `any`-typed regex never reaches QuickJS at all, so it
/// previously read straight off the wrapper object and got `undefined`
/// for every real regex property. `global`/`ignoreCase`/etc. aren't
/// stored fields on the inner `{source, flags, lastIndex}` object --
/// each is "does `flags` contain this one character", mirroring the
/// identical match table for the statically-typed case
/// (`lower_member_read`, `thaw-hir/src/lower/objects.rs`).
fn regexp_wrapper_property(value: &Value, key: &str) -> Option<Value> {
    if !is_branded_wrapper(value) { return None; }
    let inner = value.as_object()?.get(b"__thaw_regexp__".as_slice())?.as_object()?;
    if matches!(key, "source" | "flags" | "lastIndex") {
        return Some(inner.get(key.as_bytes()).cloned().unwrap_or(Value::Null));
    }
    let flag_char = match key {
        "global" => "g",
        "ignoreCase" => "i",
        "multiline" => "m",
        "dotAll" => "s",
        "sticky" => "y",
        "unicode" => "u",
        "unicodeSets" => "v",
        "hasIndices" => "d",
        _ => return None,
    };
    let flags = inner.get(b"flags".as_slice()).and_then(Value::as_str).unwrap_or("");
    Some(Value::Bool(flags.contains(flag_char)))
}

/// `.size` read directly on a `Map`/`Set` value stored in an `any`-typed
/// slot -- the same "sentinel wrapper is write-only" gap
/// `regexp_wrapper_property` fixes for `RegExp`, just for the one
/// property `Map`/`Set` share that doesn't need decoding a whole
/// element/entry back into a native `K`/`V` (which the wrapper alone
/// can't do -- `Map<K, V>`/`Set<T>` are generic, and a bare `Json`-typed
/// receiver carries no record of what `K`/`V` originally were, unlike
/// `RegExp`'s fixed `{source, flags, lastIndex}` shape -- so full,
/// generic method dispatch (`new HirType::Map`/`Set` reconstruction)
/// stays out of reach; `.size` alone just needs the wrapped array's
/// length. (Read-only methods that only need the *array itself*, not
/// a real reconstructed `Map`/`Set`, are a separate story -- see
/// `thaw_json_map_or_set_entries`/`thaw_json_map_or_set_get`/
/// `thaw_json_map_or_set_has`, below.)
fn map_or_set_wrapper_size(value: &Value) -> Option<Value> {
    if !is_branded_wrapper(value) { return None; }
    let fields = value.as_object()?;
    let entries = fields
        .get(b"__thaw_map_entries__".as_slice())
        .or_else(|| fields.get(b"__thaw_set_values__".as_slice()))?
        .as_array()?;
    Some(Value::Number(entries.len().into()))
}

/// True when `value` is `Map`-shaped (holds `__thaw_map_entries__`, an
/// array of `[key, value]` pairs) rather than `Set`-shaped -- most
/// Stage A read-only methods below (`.get`/`.keys`/`.values`/
/// `.entries`) need to know which of the two conventions applies,
/// since the *shape* of the wrapped array is the same JSON array either
/// way but its *meaning* differs (a flat list of values for a `Set`, a
/// list of pairs for a `Map`).
fn is_map_wrapper(value: &Value) -> bool {
    is_branded_wrapper(value) && value
        .as_object()
        .is_some_and(|fields| fields.contains_key(b"__thaw_map_entries__".as_slice()))
}

/// The plain JSON array underlying a `Map`/`Set` value stored in an
/// `any`-typed slot -- `for (const x of anyMapOrSet)` and every Stage A
/// read-only method below build on this single unwrap. Returns a clone
/// of `value` itself, unchanged, for anything that isn't one of the two
/// sentinel shapes (matching this file's established "degrade instead
/// of crash" policy) -- a `for...of` over an ordinary `Json` array is
/// already supported on its own, so this transparently extends that
/// same path to a wrapped one without needing a separate dispatch.
#[no_mangle]
pub extern "C" fn thaw_json_map_or_set_entries(value: *mut Value) -> *mut Value {
    let value = unsafe { &*value };
    let Some(fields) = value.as_object().filter(|_| is_branded_wrapper(value)) else {
        return leak(value.clone());
    };
    let Some(entries) = fields
        .get(b"__thaw_map_entries__".as_slice())
        .or_else(|| fields.get(b"__thaw_set_values__".as_slice()))
    else {
        return leak(value.clone());
    };
    leak(entries.clone())
}

/// `Map.prototype.get`/`.has` (and `Set.prototype.has`) on an `any`-
/// typed receiver -- a linear scan over the unwrapped entries/values
/// array, comparing keys with `thaw_json_object_is`'s own SameValue
/// semantics (the closest existing primitive; real `Map`/`Set` key
/// comparison is SameValueZero, differing from SameValue only for
/// `+0`/`-0`, an edge case not worth a bespoke comparator here). `.get`
/// returns the matched value or the napi-undefined sentinel every
/// other missing-key read in this file already uses; `.has` a plain
/// bool. Both silently answer "not found" for a value that isn't
/// actually Map/Set-shaped (or, for `.get`, one that's Set- rather than
/// Map-shaped, since a real Set has no `.get`), matching this file's
/// established degrade-instead-of-crash policy.
#[no_mangle]
pub extern "C" fn thaw_json_map_or_set_get(value: *const Value, key: *const Value) -> *mut Value {
    let value = unsafe { &*value };
    let key = unsafe { &*key };
    if !is_map_wrapper(value) {
        return leak(napi_undefined_value());
    }
    let Some(entries) = value
        .as_object()
        .and_then(|fields| fields.get(b"__thaw_map_entries__".as_slice()))
        .and_then(Value::as_array)
    else {
        return leak(napi_undefined_value());
    };
    for entry in entries {
        let Some([entry_key, entry_value]) = entry
            .as_array()
            .and_then(|pair| <&[Value; 2]>::try_from(pair.as_slice()).ok())
        else {
            continue;
        };
        if unsafe { thaw_json_same_value_zero(entry_key, key) } != 0 {
            return leak(entry_value.clone());
        }
    }
    leak(napi_undefined_value())
}

#[no_mangle]
pub extern "C" fn thaw_json_map_or_set_has(value: *const Value, key: *const Value) -> u8 {
    let value = unsafe { &*value };
    let key = unsafe { &*key };
    let Some(fields) = value.as_object().filter(|_| is_branded_wrapper(value)) else {
        return 0;
    };
    let entries = fields
        .get(b"__thaw_map_entries__".as_slice())
        .or_else(|| fields.get(b"__thaw_set_values__".as_slice()))
        .and_then(Value::as_array);
    let Some(entries) = entries else {
        return 0;
    };
    let is_map = fields.contains_key(b"__thaw_map_entries__".as_slice());
    u8::from(entries.iter().any(|entry| {
        let candidate = if is_map {
            let Some(pair) = entry.as_array() else {
                return false;
            };
            let Some(entry_key) = pair.first() else {
                return false;
            };
            entry_key
        } else {
            entry
        };
        (unsafe { thaw_json_same_value_zero(candidate, key) }) != 0
    }))
}

/// Stage B: `.set()`/`.add()`/`.delete()`/`.clear()` on a `Map`/`Set`
/// value stored in `any`. Each one computes and returns a *whole new*
/// sentinel object -- `Json`/`any` values in this compiler are
/// snapshots, not references, so the caller (`map_set_methods.rs`)
/// is the one that writes this back onto the original lvalue; these
/// helpers are pure functions with no mutation of their own. Key/
/// element comparison is `thaw_json_object_is`'s SameValue, same
/// approximation Stage A's `.get`/`.has` already made (real Map/Set
/// use SameValueZero, differing only for `+0`/`-0`).
#[no_mangle]
pub extern "C" fn thaw_json_map_or_set_set(
    value: *const Value,
    key: *const Value,
    new_value: *const Value,
) -> *mut Value {
    let value = unsafe { &*value };
    let key = unsafe { &*key };
    let new_value = unsafe { &*new_value };
    if !is_branded_wrapper(value) { return leak(value.clone()); }
    let Value::Object(fields_rc) = value else {
        return leak(value.clone());
    };
    let fields = shared_object_ref_mut(fields_rc);
    let entries_rc = fields
        .entry(b"__thaw_map_entries__".to_vec())
        .or_insert_with(|| Value::shared_array(Vec::new()));
    let Value::Array(entries_rc) = entries_rc else {
        unreachable!()
    };
    let entries = shared_array_ref_mut(entries_rc);
    let existing = entries.iter_mut().find(|entry| {
        entry
            .as_array()
            .and_then(|pair| pair.first())
            .is_some_and(|entry_key| unsafe { thaw_json_same_value_zero(entry_key, key) } != 0)
    });
    match existing {
        Some(entry) => *entry = Value::shared_array(vec![key.clone(), new_value.clone()]),
        None => entries.push(Value::shared_array(vec![key.clone(), new_value.clone()])),
    }
    leak(value.clone())
}

#[no_mangle]
pub extern "C" fn thaw_json_map_or_set_add(
    value: *const Value,
    element: *const Value,
) -> *mut Value {
    let value = unsafe { &*value };
    let element = unsafe { &*element };
    if !is_branded_wrapper(value) { return leak(value.clone()); }
    let Value::Object(fields_rc) = value else {
        return leak(value.clone());
    };
    let fields = shared_object_ref_mut(fields_rc);
    let values_rc = fields
        .entry(b"__thaw_set_values__".to_vec())
        .or_insert_with(|| Value::shared_array(Vec::new()));
    let Value::Array(values_rc) = values_rc else {
        unreachable!()
    };
    let values = shared_array_ref_mut(values_rc);
    let already_present = values
        .iter()
        .any(|candidate| unsafe { thaw_json_same_value_zero(candidate, element) } != 0);
    if !already_present {
        values.push(element.clone());
    }
    leak(value.clone())
}

#[no_mangle]
pub extern "C" fn thaw_json_map_or_set_delete(
    value: *const Value,
    key: *const Value,
) -> *mut Value {
    let value = unsafe { &*value };
    let key = unsafe { &*key };
    if !is_branded_wrapper(value) { return leak(value.clone()); }
    let Value::Object(fields_rc) = value else {
        return leak(value.clone());
    };
    let fields = shared_object_ref(fields_rc);
    if let Some(Value::Array(entries_rc)) = fields.get(b"__thaw_map_entries__".as_slice()) {
        let entries = shared_array_ref_mut(entries_rc);
        entries.retain(|entry| {
            let entry_key = entry.as_array().and_then(|pair| pair.first());
            entry_key.is_none_or(|entry_key| unsafe { thaw_json_same_value_zero(entry_key, key) } == 0)
        });
        return leak(value.clone());
    }
    if let Some(Value::Array(values_rc)) = fields.get(b"__thaw_set_values__".as_slice()) {
        let values = shared_array_ref_mut(values_rc);
        values.retain(|candidate| unsafe { thaw_json_same_value_zero(candidate, key) } == 0);
        return leak(value.clone());
    }
    leak(value.clone())
}

#[no_mangle]
pub extern "C" fn thaw_json_map_or_set_clear(value: *const Value) -> *mut Value {
    let value = unsafe { &*value };
    if !is_branded_wrapper(value) { return leak(value.clone()); }
    if let Value::Object(fields_rc) = value {
        let fields = shared_object_ref(fields_rc);
        if let Some(Value::Array(entries_rc)) = fields.get(b"__thaw_map_entries__".as_slice()) {
            shared_array_ref_mut(entries_rc).clear();
            return leak(value.clone());
        }
        if let Some(Value::Array(values_rc)) = fields.get(b"__thaw_set_values__".as_slice()) {
            shared_array_ref_mut(values_rc).clear();
            return leak(value.clone());
        }
    }
    leak(value.clone())
}

/// `.keys()`/`.values()`/`.entries()` on a `Map`/`Set` value stored in
/// `any`. Unlike the real, lazy iterator objects the statically-typed
/// case builds (`lower_map_iterator`, thaw-hir's `map_set_methods.rs`),
/// these three return a plain JSON array *eagerly*.
///
/// ponytail: a real lazy iterator (supporting early termination via
/// `.next()`/`.return()`) isn't built for the `any`-typed case -- the
/// upgrade path is the same generator-producer machinery `lower_map_
/// iterator` already has, built from this same unwrapped array instead
/// of typed native fields. This is enough for the overwhelmingly
/// common `for (const k of m.keys())`/`[...m.values()]` usage, which
/// only needs *something* iterable, and an eager array is one.
fn map_or_set_keys(value: &Value) -> Vec<Value> {
    let Some(fields) = value.as_object().filter(|_| is_branded_wrapper(value)) else {
        return Vec::new();
    };
    if let Some(entries) = fields.get(b"__thaw_map_entries__".as_slice()).and_then(Value::as_array) {
        return entries
            .iter()
            .filter_map(|entry| entry.as_array()?.first().cloned())
            .collect();
    }
    fields
        .get(b"__thaw_set_values__".as_slice())
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
}

fn map_or_set_values(value: &Value) -> Vec<Value> {
    let Some(fields) = value.as_object().filter(|_| is_branded_wrapper(value)) else {
        return Vec::new();
    };
    if let Some(entries) = fields.get(b"__thaw_map_entries__".as_slice()).and_then(Value::as_array) {
        return entries
            .iter()
            .filter_map(|entry| entry.as_array()?.get(1).cloned())
            .collect();
    }
    fields
        .get(b"__thaw_set_values__".as_slice())
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
}

fn map_or_set_entries_view(value: &Value) -> Vec<Value> {
    let Some(fields) = value.as_object().filter(|_| is_branded_wrapper(value)) else {
        return Vec::new();
    };
    if let Some(entries) = fields.get(b"__thaw_map_entries__".as_slice()).and_then(Value::as_array) {
        return entries.clone();
    }
    fields
        .get(b"__thaw_set_values__".as_slice())
        .and_then(Value::as_array)
        .map(|values| {
            values
                .iter()
                .map(|value| Value::shared_array(vec![value.clone(), value.clone()]))
                .collect()
        })
        .unwrap_or_default()
}

#[no_mangle]
pub extern "C" fn thaw_json_map_or_set_keys(value: *const Value) -> *mut u8 {
    alloc_pointer_array(
        map_or_set_keys(unsafe { &*value })
            .into_iter()
            .map(|value| Box::into_raw(Box::new(value)).cast())
            .collect(),
    )
}

#[no_mangle]
pub extern "C" fn thaw_json_map_or_set_values(value: *const Value) -> *mut u8 {
    alloc_pointer_array(
        map_or_set_values(unsafe { &*value })
            .into_iter()
            .map(|value| Box::into_raw(Box::new(value)).cast())
            .collect(),
    )
}

#[no_mangle]
pub extern "C" fn thaw_json_map_or_set_entries_view(value: *const Value) -> *mut u8 {
    alloc_pointer_array(
        map_or_set_entries_view(unsafe { &*value })
            .into_iter()
            .map(|value| Box::into_raw(Box::new(value)).cast())
            .collect(),
    )
}

fn json_array_or_buffer_data(value: &Value) -> Option<&Vec<Value>> {
    if is_graph_plain_object(value) { return value.as_array(); }
    if let Some(array) = value.as_array() {
        return Some(array);
    }
    let fields = value.as_object()?;
    if fields.get(b"type".as_slice()).and_then(Value::as_str) != Some("Buffer") {
        return None;
    }
    fields.get(b"data".as_slice())?.as_array()
}

#[no_mangle]
pub extern "C" fn thaw_json_to_number_array(value: *mut Value) -> *mut u8 {
    let values = unsafe { value.as_ref() }
        .and_then(json_array_or_buffer_data)
        .cloned()
        .unwrap_or_default();
    let output = thaw_arena::thaw_arena_alloc(8 + values.len() * 8, 8);
    if output.is_null() {
        return output;
    }
    unsafe { (output as *mut i64).write(values.len() as i64) };
    for (index, value) in values.iter().enumerate() {
        unsafe {
            (output.add(8 + index * 8) as *mut f64).write(value.as_f64().unwrap_or(0.0));
        }
    }
    output
}

#[no_mangle]
pub extern "C" fn thaw_json_array_length(value: *mut Value) -> i64 {
    unsafe { value.as_ref() }
        .and_then(json_array_or_buffer_data)
        .map_or(0, |values| values.len() as i64)
}

/// Appends every element of `other` (a Json array) to `array`: the spread of a
/// native rest array into a JS callback's argument list.
#[no_mangle]
pub extern "C" fn thaw_json_array_extend(array: *mut Value, other: *mut Value) {
    let Some(extra) = (unsafe { other.as_ref() }).and_then(Value::as_array).map(|items| items.to_vec()) else {
        return;
    };
    if let Some(items) = (unsafe { array.as_ref() }).and_then(Value::as_array_mut) {
        items.extend(extra);
    }
}

#[no_mangle]
pub extern "C" fn thaw_json_array_slice(value: *mut Value, start: i64) -> *mut Value {
    // A live JS array (a nested value of a native `any` wrapper) is read through its host.
    if let Some(Value::Host(lease)) = unsafe { value.as_ref() } {
        let items = host_enumerate_values(lease, 3).into_iter().skip(start.max(0) as usize).collect();
        return track_typed_decode_child(leak(Value::shared_array(items)));
    }
    let copy = Value::shared_array(
        unsafe { value.as_ref() }
            .and_then(Value::as_array)
            .map(|values| values.iter().skip(start.max(0) as usize).cloned().collect())
            .unwrap_or_default(),
    );
    if let (Some(Value::Array(source)), Value::Array(target)) = (unsafe { value.as_ref() }, &copy) {
        copy_array_holes(source, target, start.max(0) as usize);
    }
    track_typed_decode_child(leak(copy))
}

#[no_mangle]
pub extern "C" fn thaw_json_object_new() -> *mut Value {
    leak(Value::shared_object(indexmap::IndexMap::new()))
}

/// A native fixed-layout owner wrapper without accessors (`any` object literal) cannot grow,
/// lose a key, or hold `undefined` in a typed field. A dynamic (`any`) value built from one is
/// therefore its own mutable native copy (the old snapshot representation, now with real
/// contents); nested objects stay live unless they are wrappers themselves.
fn materialize_native_wrapper(value: Value) -> Value {
    let Value::Host(lease) = &value else { return value };
    if host_query(lease, 17).as_deref() != Some("1") { return value; }
    let entries = host_enumerable_entries(lease);
    if HOST_ERROR.with(|error| error.borrow().is_some()) { return value; }
    // The owner's integrity state (freeze/seal/preventExtensions) is keyed by pointer identity in
    // thaw-runtime; carry it onto the copy so it survives crossing into `any`.
    let operation = if host_query(lease, 18).as_deref() == Some("1") { 3 }
        else if host_query(lease, 19).as_deref() == Some("1") { 2 }
        else if host_query(lease, 20).as_deref() == Some("1") { 1 } else { 0 };
    let copy = Value::shared_object(entries.into_iter().map(|(key, item)| (key, materialize_native_wrapper(item))).collect());
    if operation != 0 {
        // SAFETY: `copy` is a live Value; the key is its shared-container identity.
        unsafe { thaw_object_set_state(thaw_json_state_key(&copy), operation) };
    }
    copy
}

fn object_insert(object: *mut Value, key: *const c_char, value: Value) {
    let key = to_key(key);
    if let Some(Value::Host(lease)) = unsafe { object.as_ref() } {
        host_set_property(lease, &key, &value);
        return;
    }
    if !object_writable(object, &key) {
        return;
    }
    if let Some(fields) = (unsafe { object.as_ref() }).and_then(Value::as_object_mut) {
        fields.insert(key, value);
    }
}

#[no_mangle]
pub extern "C" fn thaw_json_object_set_number(object: *mut Value, key: *const c_char, value: f64) {
    object_insert(object, key, number_value(value));
}

#[no_mangle]
pub extern "C" fn thaw_json_object_set_string(
    object: *mut Value,
    key: *const c_char,
    value: *const c_char,
) {
    object_insert(object, key, string_value(value));
}

#[no_mangle]
pub extern "C" fn thaw_json_object_set_bool(object: *mut Value, key: *const c_char, value: u8) {
    object_insert(object, key, Value::Bool(value != 0));
}

#[no_mangle]
pub extern "C" fn thaw_json_object_set_json(
    object: *mut Value,
    key: *const c_char,
    value: *mut Value,
) {
    object_insert(
        object,
        key,
        unsafe { value.as_ref() }.cloned().unwrap_or(Value::Null),
    );
}

/// # Safety
///
/// `value` must be a pointer returned by this module and must not be used again.
#[no_mangle]
pub unsafe extern "C" fn thaw_json_object_set_json_owned(
    object: *mut Value,
    key: *const c_char,
    value: *mut Value,
) {
    untrack_typed_decode_child(value);
    untrack_arena_owned_json_root(value);
    object_insert(object, key, *unsafe { Box::from_raw(value) });
}

#[no_mangle]
pub extern "C" fn thaw_json_object_delete(object: *mut Value, key: *const c_char) -> u8 {
    let Some(value) = (unsafe { object.as_ref() }) else {
        return 0;
    };
    if let Value::Host(lease) = value {
        return u8::from(host_property_predicate(lease, &to_key(key), 2));
    }
    if matches!(value, Value::Null) || is_napi_undefined(value) {
        return 0;
    }
    let key = to_key(key);
    if json_has_own_value(value, &key)
        && unsafe { thaw_object_state(thaw_json_state_key(object), 1) }
    {
        return 0;
    }
    if let Value::Array(items) = value {
        if key.as_slice() == b"length" {
            return 0;
        }
        if let Some(index) = array_index_key(&key) {
            let index = index as usize;
            if array_has_index(items, index) {
                mark_array_holes(items, std::iter::once(index));
                shared_array_ref_mut(items)[index] = napi_undefined_value();
            }
        }
        return 1;
    }
    if let Some(fields) = (unsafe { object.as_ref() }).and_then(Value::as_object_mut) {
        // `serde_json::Map::remove` (with the `preserve_order` feature
        // this crate builds with) is `swap_remove` -- it moves the map's
        // last entry into the removed slot instead of shifting the
        // rest down, silently reordering every remaining key. Real JS
        // objects preserve string-key insertion order (`delete obj.a`
        // never reorders `b`/`c`/`d`), and this matters to every
        // consumer that walks the object's own key order: `Object.
        // keys`/`JSON.stringify`/for-in/the destructuring `...rest`
        // pattern (`crates/thaw-hir/src/lower/destructuring.rs`'s
        // `lower_dictionary_object_rest`, which builds its rest object
        // by deleting the destructured keys from a full copy). Use the
        // order-preserving removal instead.
        fields.shift_remove(&key);
    }
    1
}

#[no_mangle]
/// # Safety
///
/// `target` and `source` must be null or point to valid JSON values.
pub unsafe extern "C" fn thaw_json_object_assign(
    target: *mut Value,
    source: *const Value,
) -> *mut Value {
    ASSIGN_ERROR.with(|error| error.set(false));
    if (unsafe { target.as_ref() }).and_then(Value::as_object).is_none() {
        return target;
    }
    let Some(source) = (unsafe { source.as_ref() }) else {
        return target;
    };
    // Real `Object.assign`/object-literal-spread semantics: `null`/
    // `undefined` sources are silently skipped (never an error, matching
    // `Object.assign({}, null, undefined, {a: 1})` -> `{a: 1}`) -- this
    // wasn't just a `.assign()` gap either: copying the napi-undefined
    // sentinel (`is_napi_undefined`) object's own `$__thaw_napi_undefined
    // $` tracking key straight into `target_fields` (previously, since
    // it *is* structurally a `Value::Object`) corrupted the whole target
    // into something every other `is_napi_undefined` check then saw as
    // itself being `undefined` -- observed via `{...maybeUndefined,
    // b: 2}` silently losing every field, not just the nullish source's
    // absence of any.
    if matches!(source, Value::Null) || is_napi_undefined(source) {
        return target;
    }
    if let Some(Value::Host(lease)) = unsafe { target.as_ref() } {
        for (key, value) in enumerable_entries(source) {
            host_set_property(lease, &key, &value);
        }
        return target;
    }
    for (key, value) in enumerable_entries(source) {
        if !object_writable(target, &key) {
            ASSIGN_ERROR.with(|error| error.set(true));
            break;
        }
        if let Some(fields) = (unsafe { target.as_ref() }).and_then(Value::as_object_mut) {
            fields.insert(key, value);
        }
    }
    target
}

#[cfg(test)]
mod tests {
    use super::*;

    thread_local! {
        static HOST_RELEASES: Cell<usize> = const { Cell::new(0) };
        static CLEANUP_HOST_RELEASES: Cell<usize> = const { Cell::new(0) };
        static TYPED_SCOPE_RETAINS: Cell<usize> = const { Cell::new(0) };
        static REENTRANT_SOURCE: Cell<*mut Value> = const { Cell::new(std::ptr::null_mut()) };
        static REENTRANT_CHILD: Cell<*mut Value> = const { Cell::new(std::ptr::null_mut()) };
        static NESTED_DESTROY: Cell<*mut Value> = const { Cell::new(std::ptr::null_mut()) };
        static NAPI_RETAINS: Cell<usize> = const { Cell::new(0) };
        static NAPI_RELEASES: Cell<usize> = const { Cell::new(0) };
        static TRACKED_REENTRANT_ROOTS: Cell<(usize, usize)> = const { Cell::new((0, 0)) };
        static TRACKED_RELEASE_REENTERED: Cell<bool> = const { Cell::new(false) };
    }

    extern "C" fn retain_test_napi_handle(handle: u64) -> u64 {
        if handle != 17 { return 0; }
        NAPI_RETAINS.with(|count| {
            let next = count.get() + 1;
            count.set(next);
            117 + next as u64
        })
    }

    extern "C" fn release_test_napi_reference(reference: u64) -> u8 {
        assert!((118..=120).contains(&reference));
        NAPI_RELEASES.with(|count| count.set(count.get() + 1));
        1
    }

    extern "C" fn count_host_release(_handle: u64) -> u8 {
        HOST_RELEASES.with(|count| count.set(count.get() + 1));
        1
    }

    extern "C" fn release_and_destroy_other_tracked_root(handle: u64) -> u8 {
        HOST_RELEASES.with(|count| count.set(count.get() + 1));
        if !TRACKED_RELEASE_REENTERED.with(|active| active.replace(true)) {
            let (first, second) = TRACKED_REENTRANT_ROOTS.with(Cell::get);
            let other = if handle == 41 { second } else { first };
            assert_ne!(other, 0);
            unsafe { thaw_json_destroy(other as *mut Value) };
        }
        1
    }

    extern "C" fn retain_typed_scope_host(_handle: u64) -> u8 {
        TYPED_SCOPE_RETAINS.with(|count| count.set(count.get() + 1));
        1
    }

    extern "C" fn release_with_cleanup_host_error(_handle: u64) -> u8 {
        CLEANUP_HOST_RELEASES.with(|count| count.set(count.get() + 1));
        set_host_error("cleanup callback error".into());
        1
    }

    extern "C" fn reentrant_typed_scope_get(_: u64, _: *const c_char) -> HostHandleResult {
        let source = REENTRANT_SOURCE.with(Cell::get);
        let child = thaw_json_get(source, c"x".as_ptr());
        REENTRANT_CHILD.with(|slot| slot.set(child));
        HostHandleResult { value: 0, error: std::ptr::null() }
    }

    extern "C" fn unused_typed_scope_set(_: u64, _: *const c_char, _: *const c_char) -> HostHandleResult {
        HostHandleResult { value: 1, error: std::ptr::null() }
    }

    extern "C" fn unused_typed_scope_predicate(_: u64, _: *const c_char, _: u8) -> HostHandleResult {
        HostHandleResult { value: 0, error: std::ptr::null() }
    }

    extern "C" fn unused_typed_scope_text(_: u64, _: u8) -> HostTextResult {
        HostTextResult { value: std::ptr::null(), error: std::ptr::null() }
    }

    extern "C" fn callback_origin_test_query(handle: u64, operation: u8) -> HostTextResult {
        assert_eq!(operation, 14);
        let kind = if handle == 42 { b"1".as_slice() } else { b"0".as_slice() };
        HostTextResult { value: thaw_arena::owned_string(kind), error: std::ptr::null() }
    }

    extern "C" fn cleanup_test_host_query(_: u64, _: u8) -> HostTextResult {
        HostTextResult { value: thaw_arena::owned_string(b"object"), error: std::ptr::null() }
    }

    extern "C" fn owned_receiver_test_host_query(_: u64, operation: u8) -> HostTextResult {
        let value = if operation == 7 { b"0".as_slice() } else { b"object".as_slice() };
        HostTextResult { value: thaw_arena::owned_string(value), error: std::ptr::null() }
    }

    extern "C" fn unused_typed_scope_date(_: u64, _: f64) -> HostTextResult {
        HostTextResult { value: std::ptr::null(), error: std::ptr::null() }
    }

    extern "C" fn release_and_destroy_nested(_handle: u64) -> u8 {
        NESTED_DESTROY.with(|slot| {
            let nested = slot.replace(std::ptr::null_mut());
            assert!(!nested.is_null());
            unsafe { thaw_json_destroy(nested) };
        });
        count_host_release(0)
    }

    fn parse(s: &str) -> *mut Value {
        let c = CString::new(s).unwrap();
        thaw_json_parse(c.as_ptr())
    }

    #[test]
    fn object_keys_preserve_utf16_identity_through_native_operations() {
        let object = parse(r#"{"\ud800":1,"\ud801":2,"x\u0000y":3}"#);
        let first = thaw_arena::arena_string(&wtf8_encode_utf16(&[0xD800]));
        let second = thaw_arena::arena_string(&wtf8_encode_utf16(&[0xD801]));
        let nul = thaw_arena::arena_string(b"x\0y");
        assert_eq!(thaw_json_as_number(thaw_json_get(object, first)), 1.0);
        assert_eq!(thaw_json_as_number(thaw_json_get(object, second)), 2.0);
        assert_eq!(thaw_json_as_number(thaw_json_get(object, nul)), 3.0);
        assert_eq!(unsafe { thaw_json_has_own(object, second) }, 1);
        let keys = unsafe { thaw_json_keys(object) };
        assert_eq!(unsafe { keys.cast::<u64>().read() }, 3);
        for (index, expected) in [wtf8_encode_utf16(&[0xD800]), wtf8_encode_utf16(&[0xD801]), b"x\0y".to_vec()].iter().enumerate() {
            let key = unsafe { keys.add(8 + index * 8).cast::<*const c_char>().read() };
            assert_eq!(unsafe { CStr::from_ptr(key) }.to_bytes(), expected);
        }
        assert_eq!(read_c_string(thaw_json_stringify(object)), r#"{"\ud800":1,"\ud801":2,"x\u0000y":3}"#);
        let selected = alloc_pointer_array(vec![second.cast(), first.cast()]);
        assert_eq!(read_c_string(thaw_json_stringify_keys(object, selected, std::ptr::null())), r#"{"\ud801":2,"\ud800":1}"#);
        let child = parse("{}");
        unsafe { thaw_json_set_prototype(child, object) };
        assert_eq!(thaw_json_as_number(thaw_json_get(child, second)), 2.0);
        assert_eq!(thaw_json_has(child, second), 1);
        thaw_json_object_set_number(object, first, 5.0);
        assert_eq!(thaw_json_as_number(thaw_json_get(object, first)), 5.0);
        assert_eq!(thaw_json_as_number(thaw_json_get(object, second)), 2.0);
        assert_eq!(thaw_json_object_delete(object, first), 1);
        assert_eq!(unsafe { thaw_json_has_own(object, first) }, 0);
        assert_eq!(thaw_json_as_number(thaw_json_get(object, second)), 2.0);
    }

    #[test]
    fn object_keys_normalize_equivalent_astral_spellings() {
        let object = parse("{\"😀\":1,\"\\ud83d\\ude00\":2}");
        let pair = thaw_arena::arena_string(&[
            wtf8_encode_utf16(&[0xD83D]), wtf8_encode_utf16(&[0xDE00]),
        ].concat());
        assert_eq!(thaw_json_as_number(thaw_json_get(object, pair)), 2.0);
        assert_eq!(unsafe { object.as_ref() }.and_then(Value::as_object).unwrap().len(), 1);
    }

    #[test]
    fn share_gives_an_independent_owner_of_the_same_containers() {
        assert!(unsafe { thaw_json_share(std::ptr::null()) }.is_null());
        let object = parse(r#"{"answer":42}"#);
        let shared = unsafe { thaw_json_share(object) };
        assert!(!shared.is_null() && shared != object);
        // Same Rc-backed container: a write through one handle is visible in the other.
        let key = CString::new("extra").unwrap();
        thaw_json_object_set_number(object, key.as_ptr(), 7.0);
        assert_eq!(thaw_json_as_number(thaw_json_get(shared, key.as_ptr())), 7.0);
        // Destroying the shared owner must not invalidate the borrowed original.
        unsafe { thaw_json_destroy(shared) };
        assert_eq!(thaw_json_as_number(thaw_json_get(object, key.as_ptr())), 7.0);
        unsafe { thaw_json_destroy(object) };
    }

    #[test]
    fn deletes_object_properties_and_succeeds_for_missing_keys() {
        let object = parse(r#"{"answer":42}"#);
        let answer = CString::new("answer").unwrap();
        let missing = CString::new("missing").unwrap();
        assert_eq!(thaw_json_object_delete(object, answer.as_ptr()), 1);
        assert_eq!(thaw_json_object_delete(object, missing.as_ptr()), 1);
        assert!(unsafe { object.as_ref() }
            .and_then(Value::as_object)
            .is_some_and(|fields| fields.is_empty()));
    }

    fn read_c_string(ptr: *const c_char) -> String {
        unsafe { CStr::from_ptr(ptr) }
            .to_string_lossy()
            .into_owned()
    }

    #[test]
    fn checked_json_bigint_i64_preserves_range_and_rejects_overflow() {
        assert!(thaw_json_take_host_error().is_null());
        let minimum = leak(Value::BigInt(i64::MIN.to_string()));
        let maximum = leak(Value::BigInt(i64::MAX.to_string()));
        assert_eq!(unsafe { thaw_json_as_bigint_i64(minimum) }, i64::MIN);
        assert_eq!(unsafe { thaw_json_as_bigint_i64(maximum) }, i64::MAX);
        unsafe {
            thaw_json_destroy(minimum);
            thaw_json_destroy(maximum);
        }

        let overflow = leak(Value::BigInt("9223372036854775808".into()));
        assert_eq!(unsafe { thaw_json_as_bigint_i64(overflow) }, 0);
        let error = thaw_json_take_host_error();
        assert_eq!(read_c_string(error), "Json BigInt is outside native i64 range");
        unsafe {
            thaw_cstring_destroy(error.cast_mut());
            thaw_json_destroy(overflow);
        }
        assert!(thaw_json_take_host_error().is_null());
    }

    #[test]
    fn reports_javascript_typeof_categories_for_json_values() {
        for (source, expected) in [
            ("null", "object"),
            ("[]", "object"),
            ("true", "boolean"),
            ("42", "number"),
            (r#""text""#, "string"),
            (r#"{"$__thaw_napi_undefined$":true}"#, "undefined"),
        ] {
            assert_eq!(
                read_c_string(unsafe { thaw_json_typeof(parse(source)) }),
                expected
            );
        }
    }

    #[test]
    fn round_trips_object_field_access() {
        let value = parse(r#"{"name": "thaw", "version": 2, "stable": false}"#);

        let name_key = CString::new("name").unwrap();
        let name = thaw_json_get(value, name_key.as_ptr());
        assert_eq!(read_c_string(thaw_json_as_string(name)), "thaw");

        let version_key = CString::new("version").unwrap();
        let version = thaw_json_get(value, version_key.as_ptr());
        assert_eq!(thaw_json_as_number(version), 2.0);

        let stable_key = CString::new("stable").unwrap();
        let stable = thaw_json_get(value, stable_key.as_ptr());
        assert_eq!(thaw_json_as_bool(stable), 0);
    }

    #[test]
    fn walks_the_prototype_chain_for_dynamic_objects() {
        let proto = parse(r#"{"inherited": 7}"#);
        let object = parse("{}");
        unsafe { thaw_json_set_prototype(object, proto) };

        let inherited = CString::new("inherited").unwrap();
        assert_eq!(
            thaw_json_as_number(thaw_json_get(object, inherited.as_ptr())),
            7.0
        );
        assert_eq!(thaw_json_has(object, inherited.as_ptr()), 1);
        assert_eq!(unsafe { thaw_json_has_own(object, inherited.as_ptr()) }, 0);

        let missing = CString::new("missing").unwrap();
        assert_eq!(
            read_c_string(unsafe { thaw_json_typeof(thaw_json_get(object, missing.as_ptr())) }),
            "undefined"
        );
        assert_eq!(thaw_json_has(object, missing.as_ptr()), 0);
    }

    #[test]
    fn takes_a_field_from_a_consumed_temporary_object() {
        let value = thaw_json_object_new();
        let key = CString::new("answer").unwrap();
        let child = parse("42");
        unsafe { thaw_json_object_set_json_owned(value, key.as_ptr(), child) };
        let answer = unsafe { thaw_json_take(value, key.as_ptr()) };
        assert_eq!(thaw_json_as_number(answer), 42.0);
        unsafe { thaw_json_destroy(answer) };
    }

    #[test]
    fn formats_dynamic_json_like_console_log() {
        assert_eq!(
            read_c_string(thaw_json_console_string(parse(r#""text""#))),
            "text"
        );
        assert_eq!(
            read_c_string(thaw_json_console_string(parse("null"))),
            "null"
        );
        assert_eq!(
            read_c_string(thaw_json_console_string(parse(r#"{"value":1}"#))),
            "{ value: 1 }"
        );
    }

    #[test]
    fn applies_javascript_truthiness_to_json_values() {
        for (source, expected) in [
            ("null", 0),
            ("false", 0),
            ("true", 1),
            ("0", 0),
            ("1", 1),
            (r#"""#, 0),
            (r#""text""#, 1),
            ("[]", 1),
            ("{}", 1),
            (r#"{"$__thaw_napi_undefined$":true}"#, 0),
        ] {
            assert_eq!(thaw_json_as_bool(parse(source)), expected, "{source}");
        }
    }

    #[test]
    fn orders_integer_object_keys_before_string_keys() {
        let value = parse(r#"{"tail":0,"10":"ten","2":"two","01":"leading"}"#);
        assert_eq!(
            read_c_string(thaw_json_stringify(value)),
            r#"{"2":"two","10":"ten","tail":0,"01":"leading"}"#
        );
        let keys = unsafe { thaw_json_keys(value) };
        let names = (0..4)
            .map(|index| {
                read_c_string(unsafe { (keys.add(8 + index * 8) as *const *const c_char).read() })
            })
            .collect::<Vec<_>>();
        assert_eq!(names, ["2", "10", "tail", "01"]);
    }

    #[test]
    fn indexes_into_arrays() {
        let value = parse("[10, 20, 30]");
        assert_eq!(
            thaw_json_as_number(thaw_json_index(value, 0.0, std::ptr::null())),
            10.0
        );
        assert_eq!(
            thaw_json_as_number(thaw_json_index(value, 2.0, std::ptr::null())),
            30.0
        );
    }

    #[test]
    fn array_length_is_a_builtin_not_a_data_field() {
        let value = parse("[1, 2, 3]");
        let length_key = CString::new("length").unwrap();
        assert_eq!(
            thaw_json_as_number(thaw_json_get(value, length_key.as_ptr())),
            3.0
        );

        // An object with an actual field named "length" still works via
        // the normal lookup path.
        let obj = parse(r#"{"length": 42}"#);
        assert_eq!(
            thaw_json_as_number(thaw_json_get(obj, length_key.as_ptr())),
            42.0
        );
    }

    #[test]
    fn buffer_shaped_object_length_is_the_data_array_length() {
        // `{"type":"Buffer","data":[...]}` -- the shape `__thaw_json_
        // binary_replacer` (thaw-quickjs) produces for a real Buffer/
        // TypedArray crossing into a `Json`-typed native callback
        // parameter (an untyped/`any` chunk, e.g. `fs.createReadStream(
        // ...).on('data', chunk => ...)`). `.length` must resolve to the
        // real byte count, not fall through to the plain key-lookup
        // default (which would silently read `Null` -> `0`).
        let value = parse(r#"{"type":"Buffer","data":[1,2,3,4,5]}"#);
        let length_key = CString::new("length").unwrap();
        assert_eq!(
            thaw_json_as_number(thaw_json_get(value, length_key.as_ptr())),
            5.0
        );

        // A plain object that merely happens to have a "type"/"data"
        // pair not shaped like a Buffer keeps the ordinary key lookup.
        let not_a_buffer = parse(r#"{"type":"Widget","data":[1,2,3],"length":99}"#);
        assert_eq!(
            thaw_json_as_number(thaw_json_get(not_a_buffer, length_key.as_ptr())),
            99.0
        );
    }

    #[test]
    fn returns_object_and_array_keys_in_javascript_order() {
        let object = parse(r#"{"second": 2, "first": 1}"#);
        let keys = unsafe { thaw_json_keys(object) };
        assert_eq!(unsafe { (keys as *const i64).read() }, 2);
        assert_eq!(
            read_c_string(unsafe { (keys.add(8) as *const *const c_char).read() }),
            "second"
        );
        assert_eq!(
            read_c_string(unsafe { (keys.add(16) as *const *const c_char).read() }),
            "first"
        );

        let array = parse("[10, 20]");
        let keys = unsafe { thaw_json_keys(array) };
        assert_eq!(unsafe { (keys as *const i64).read() }, 2);
        assert_eq!(
            read_c_string(unsafe { (keys.add(8) as *const *const c_char).read() }),
            "0"
        );
        assert_eq!(
            read_c_string(unsafe { (keys.add(16) as *const *const c_char).read() }),
            "1"
        );
    }

    #[test]
    fn returns_json_values_and_entries_in_key_order() {
        let object = parse(r#"{"second": 2, "first": "one"}"#);
        let values = unsafe { thaw_json_values(object) };
        assert_eq!(unsafe { (values as *const i64).read() }, 2);
        let second = unsafe { (values.add(8) as *const *mut Value).read() };
        let first = unsafe { (values.add(16) as *const *mut Value).read() };
        assert_eq!(thaw_json_as_number(second), 2.0);
        assert_eq!(read_c_string(thaw_json_as_string(first)), "one");

        let entries = unsafe { thaw_json_entries(object) };
        assert_eq!(unsafe { (entries as *const i64).read() }, 2);
        let first_entry =
            unsafe { unwrap_array_handle((entries.add(8) as *const *mut u8).read()).cast_mut() };
        assert_eq!(unsafe { (first_entry as *const i64).read() }, 2);
        assert_eq!(
            read_c_string(unsafe { (first_entry.add(8) as *const *const c_char).read() }),
            "second"
        );
        let value = unsafe { (first_entry.add(16) as *const *mut Value).read() };
        assert_eq!(thaw_json_as_number(value), 2.0);
    }

    #[test]
    fn detects_owned_json_object_and_array_properties() {
        let object = parse(r#"{"value": 1}"#);
        let value = CString::new("value").unwrap();
        let missing = CString::new("missing").unwrap();
        assert_eq!(unsafe { thaw_json_has_own(object, value.as_ptr()) }, 1);
        assert_eq!(unsafe { thaw_json_has_own(object, missing.as_ptr()) }, 0);

        let array = parse("[10, 20]");
        let zero = CString::new("0").unwrap();
        let leading_zero = CString::new("00").unwrap();
        let length = CString::new("length").unwrap();
        assert_eq!(unsafe { thaw_json_has_own(array, zero.as_ptr()) }, 1);
        assert_eq!(
            unsafe { thaw_json_has_own(array, leading_zero.as_ptr()) },
            0
        );
        assert_eq!(unsafe { thaw_json_has_own(array, length.as_ptr()) }, 1);
    }

    #[test]
    fn compares_json_values_with_same_value_semantics() {
        let number = parse("1");
        let same_number = parse("1");
        let text = parse(r#""thaw""#);
        let object = parse(r#"{"value": 1}"#);
        let equal_object = parse(r#"{"value": 1}"#);
        assert_eq!(unsafe { thaw_json_object_is(number, same_number) }, 1);
        assert_eq!(unsafe { thaw_json_object_is(object, object) }, 1);
        assert_eq!(unsafe { thaw_json_object_is(object, equal_object) }, 0);
        assert_eq!(unsafe { thaw_json_object_is_number(number, 1.0) }, 1);
        assert_eq!(unsafe { thaw_json_object_is_number(number, -0.0) }, 0);
        let thaw = CString::new("thaw").unwrap();
        assert_eq!(
            unsafe { thaw_json_object_is_string(text, thaw.as_ptr()) },
            1
        );
        assert_eq!(unsafe { thaw_json_object_is_bool(parse("true"), true) }, 1);
    }

    #[test]
    fn missing_field_and_out_of_range_index_degrade_to_defaults_not_crashes() {
        let value = parse(r#"{"a": 1}"#);
        let missing_key = CString::new("nope").unwrap();
        let missing = thaw_json_get(value, missing_key.as_ptr());
        // Real `Number(undefined)` is `NaN` (so a missing key is neither
        // `== 0` nor usable in arithmetic), while an explicit `null`
        // reads as `0`.
        assert!(thaw_json_as_number(missing).is_nan());
        let null_key = CString::new("n").unwrap();
        let present = parse(r#"{"n": null}"#);
        assert_eq!(
            thaw_json_as_number(thaw_json_get(present, null_key.as_ptr())),
            0.0
        );
        // A genuinely missing key now degrades to the same
        // napi-undefined sentinel a real `undefined` argument does, not
        // bare `Value::Null` -- `String(undefined)` is `"undefined"` in
        // real JS, not `""` (which is what this assertion used to check,
        // back when a missing key and an explicit `null` were
        // indistinguishable).
        assert_eq!(read_c_string(thaw_json_as_string(missing)), "undefined");

        let arr = parse("[1, 2]");
        let oob = thaw_json_index(arr, 99.0, std::ptr::null());
        assert!(thaw_json_as_number(oob).is_nan());
    }

    #[test]
    fn a_missing_key_or_index_is_distinguishable_from_an_explicit_null() {
        let value = parse(r#"{"a": 1, "n": null}"#);
        let present_null_key = CString::new("n").unwrap();
        let present_null = thaw_json_get(value, present_null_key.as_ptr());
        let missing_key = CString::new("nope").unwrap();
        let missing = thaw_json_get(value, missing_key.as_ptr());

        // An explicit `null` field stays real `null` throughout.
        assert_eq!(unsafe { thaw_json_is_null(present_null) }, 1);
        assert_eq!(unsafe { thaw_json_is_nullish(present_null) }, 1);
        assert_eq!(
            read_c_string(unsafe { thaw_json_typeof(present_null) }),
            "object"
        );

        // A genuinely-missing key must NOT read as null...
        assert_eq!(unsafe { thaw_json_is_null(missing) }, 0);
        // ...but must read as nullish (real JS: `missing == null` is
        // `true`) and as `undefined` throughout `typeof`/`String()`.
        assert_eq!(unsafe { thaw_json_is_nullish(missing) }, 1);
        assert_eq!(
            read_c_string(unsafe { thaw_json_typeof(missing) }),
            "undefined"
        );
        assert_eq!(thaw_json_as_bool(missing), 0);
        assert_eq!(read_c_string(thaw_json_as_string(missing)), "undefined");
        assert_eq!(
            read_c_string(thaw_json_console_string(missing)),
            "undefined"
        );

        // Same story for an out-of-range array index.
        let arr = parse("[1, 2]");
        let oob = thaw_json_index(arr, 99.0, std::ptr::null());
        assert_eq!(unsafe { thaw_json_is_null(oob) }, 0);
        assert_eq!(unsafe { thaw_json_is_nullish(oob) }, 1);
        assert_eq!(read_c_string(unsafe { thaw_json_typeof(oob) }), "undefined");
    }

    #[test]
    fn console_format_follows_util_format() {
        let fmt = |args: &str| read_c_string(thaw_console_format(parse(args)));
        assert_eq!(fmt(r#"["%s is %d", "a", 5]"#), "a is 5");
        assert_eq!(fmt(r#"["%j %%", {"a":1}, 2]"#), "{\"a\":1} % 2");
        assert_eq!(fmt(r#"["%s"]"#), "%s");
        assert_eq!(fmt(r#"["x", "y", [1]]"#), "x y [ 1 ]");
        assert_eq!(fmt(r#"["%i %f", "3.9px", "1.5e1x"]"#), "3 15");
    }

    #[test]
    fn stringify_round_trips() {
        let value = parse(r#"{"a": 1, "b": [true, false]}"#);
        let text = read_c_string(thaw_json_stringify(value));
        let reparsed: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(reparsed, serde_json::json!({"a": 1, "b": [true, false]}));
    }

    /// Builds the native `[len: i64][ptr: *const c_char; len]` layout
    /// `string_array` (backing `thaw_json_stringify_keys*`) expects,
    /// leaking each key's `CString` for the test's duration.
    fn encode_string_array(keys: &[&str]) -> *const u8 {
        let pointers: Vec<*const c_char> = keys
            .iter()
            .map(|key| CString::new(*key).unwrap().into_raw() as *const c_char)
            .collect();
        let mut bytes = (keys.len() as i64).to_ne_bytes().to_vec();
        for pointer in pointers {
            bytes.extend_from_slice(&(pointer as usize as u64).to_ne_bytes());
        }
        Box::leak(bytes.into_boxed_slice()).as_ptr()
    }

    #[test]
    fn graph_decode_accepts_an_array_node_with_its_standard_property_descriptors() {
        let wire = CString::new(r#"{"root":{"r":0},"nodes":[{"a":[{"v":1},{"v":2}],"p":[["0",{"v":1},7],["1",{"v":2},7],["length",{"v":2},1]]}],"leases":[],"napiLeases":[]}"#).unwrap();
        let decoded = thaw_json_graph_decode(wire.as_ptr());
        assert_eq!(thaw_json_take_graph_error(), 0);
        assert_eq!(unsafe { &*decoded }.as_array().map(Vec::len), Some(2));
        // An own property a native array cannot hold is still rejected, not dropped.
        let extra = CString::new(r#"{"root":{"r":0},"nodes":[{"a":[],"p":[["extra",{"v":1},7]]}],"leases":[],"napiLeases":[]}"#).unwrap();
        thaw_json_graph_decode(extra.as_ptr());
        assert_eq!(thaw_json_take_graph_error(), 1);
    }

    #[test]
    fn graph_transport_preserves_negative_zero_without_changing_public_json() {
        let value = leak(number_value(-0.0));
        let wire = thaw_json_graph_encode(value);
        assert!(read_c_string(wire).contains("\"nf\":\"-0\""));
        let decoded = thaw_json_graph_decode(wire);
        assert_eq!(thaw_json_take_graph_error(), 0);
        assert_eq!(unsafe { &*decoded }.as_f64().unwrap().to_bits(), (-0.0f64).to_bits());
        let mut ordinary = Vec::new();
        write_json_value(unsafe { &*value }, &mut ordinary, None, 0);
        assert_eq!(ordinary, b"0");
        unsafe { thaw_json_destroy(value); thaw_json_destroy(decoded); thaw_cstring_destroy(wire.cast_mut()); }
    }

    #[test]
    fn graph_transport_keeps_wrapper_identity_and_map_back_edges() {
        let map = thaw_json_brand_wrapper(thaw_json_object_new());
        let entries = thaw_json_array_new();
        let pair = thaw_json_array_new();
        thaw_json_array_push_string(pair, CString::new("self").unwrap().as_ptr());
        thaw_json_array_push_json(pair, map);
        thaw_json_array_push_json(entries, pair);
        thaw_json_object_set_json(map, CString::new("__thaw_map_entries__").unwrap().as_ptr(), entries);
        let root = thaw_json_object_new();
        thaw_json_object_set_json(root, CString::new("left").unwrap().as_ptr(), map);
        thaw_json_object_set_json(root, CString::new("right").unwrap().as_ptr(), map);
        let graph: serde_json::Value = serde_json::from_str(
            &read_c_string(thaw_json_graph_encode(root))
        ).unwrap();
        assert_eq!(graph["nodes"][0]["o"][0][1]["r"], 1);
        assert_eq!(graph["nodes"][0]["o"][1][1]["r"], 1);
        assert_eq!(graph["nodes"][1]["m"]["r"], 2);
        assert_eq!(graph["nodes"][3]["a"][1]["r"], 1);

        let ordinary = parse(r#"{"__thaw_map_entries__":[["a",1]]}"#);
        let ordinary_graph: serde_json::Value = serde_json::from_str(
            &read_c_string(thaw_json_graph_encode(ordinary))
        ).unwrap();
        assert!(ordinary_graph["nodes"][0].get("o").is_some());
        assert!(ordinary_graph["nodes"][0].get("m").is_none());
    }

    #[test]
    fn parsed_wrapper_shaped_user_objects_remain_plain_objects() {
        let parsed = parse(r#"{"__thaw_map_entries__":[["k",1]]}"#);
        assert_eq!(thaw_json_has_wrapper_key(parsed, c"__thaw_map_entries__".as_ptr()), 0);
        let key = parse(r#""k""#);
        let missing = thaw_json_map_or_set_get(parsed, key);
        assert_eq!(unsafe { thaw_json_is_undefined(missing) }, 1);
        assert_eq!(thaw_json_map_or_set_has(parsed, key), 0);
        let branded = thaw_json_brand_wrapper(parse(r#"{"__thaw_map_entries__":[["k",1]]}"#));
        assert_eq!(thaw_json_has_wrapper_key(branded, c"__thaw_map_entries__".as_ptr()), 1);
        assert_eq!(thaw_json_map_or_set_has(branded, key), 1);
    }

    #[test]
    fn only_branded_dynamic_handle_placeholders_expose_handle_ids() {
        let forged = parse(r#"{"__thaw_js_handle_id__":7}"#);
        assert_eq!(unsafe { thaw_json_handle_id(forged) }, 0);
        let extra = parse(r#"{"__thaw_js_handle_id__":7,"user":true}"#);
        thaw_json_brand_wrapper(extra);
        assert_eq!(unsafe { thaw_json_handle_id(extra) }, 0);
        let placeholder = parse(r#"{"__thaw_js_handle_id__":7}"#);
        thaw_json_brand_wrapper(placeholder);
        assert_eq!(unsafe { thaw_json_handle_id(placeholder) }, 7);
    }

    #[test]
    fn graph_decode_preserves_callback_cycles_binary_and_handles() {
        let encoded = CString::new(r#"{"root":{"r":0},"nodes":[{"a":[{"r":0},{"r":1},{"r":2}]},{"b":[0,127,255]},{"hdl":7}]}"#).unwrap();
        let decoded = thaw_json_graph_decode(encoded.as_ptr());
        assert_eq!(thaw_json_take_graph_error(), 0);
        // Without HostOperations the `hdl` node decodes to a detached marker
        // that cannot be re-leased, so re-encoding the whole graph must fail
        // rather than emit a handle nobody retained.
        assert!(read_c_string(thaw_json_graph_encode(decoded)).starts_with("\u{2}TypeError:"));
        assert!(!thaw_json_take_host_error().is_null());
        let plain = CString::new(r#"{"root":{"r":0},"nodes":[{"a":[{"r":0},{"r":1}]},{"b":[0,127,255]}]}"#).unwrap();
        let graph: serde_json::Value = serde_json::from_str(
            &read_c_string(thaw_json_graph_encode(thaw_json_graph_decode(plain.as_ptr())))
        ).unwrap();
        assert_eq!(graph["nodes"][0]["a"][0]["r"], 0);
        assert_eq!(graph["nodes"][1]["b"], serde_json::json!([0, 127, 255]));
        let malformed = CString::new(r#"{"root":{"r":3},"nodes":[]}"#).unwrap();
        let invalid = thaw_json_graph_decode(malformed.as_ptr());
        assert_eq!(thaw_json_take_graph_error(), 1);
        unsafe { thaw_json_destroy(invalid) };
    }

    #[test]
    fn graph_transport_preserves_cycle_alias_and_date_node_identity() {
        let root = thaw_json_object_new();
        thaw_json_object_set_json(root, CString::new("self").unwrap().as_ptr(), root);
        let date = thaw_json_brand_wrapper(parse(r#"{"timestamp":0}"#));
        thaw_json_object_set_json(root, CString::new("left").unwrap().as_ptr(), date);
        thaw_json_object_set_json(root, CString::new("right").unwrap().as_ptr(), date);
        let graph: serde_json::Value = serde_json::from_str(
            &read_c_string(thaw_json_graph_encode(root))
        ).unwrap();
        assert_eq!(graph["root"]["r"], 0);
        assert_eq!(graph["nodes"][0]["o"][0][1]["r"], 0);
        assert_eq!(graph["nodes"][0]["o"][1][1]["r"], 1);
        assert_eq!(graph["nodes"][0]["o"][2][1]["r"], 1);
        assert_eq!(graph["nodes"][1]["d"], 0);
        assert_eq!(thaw_json_take_stringify_error(), 0);
    }

    #[test]
    fn graph_tokens_distinguish_napi_and_user_reserved_shapes() {
        NAPI_RETAINS.with(|count| count.set(0));
        NAPI_RELEASES.with(|count| count.set(0));
        thaw_json_register_napi_handle_operations(retain_test_napi_handle, release_test_napi_reference);
        let source = CString::new(r#"{"root":{"r":0},"nodes":[{"a":[{"r":1},{"r":2},{"u":1},{"r":3},{"r":4},{"r":5}]},{"d":0},{"o":[["timestamp",{"v":0}]]},{"o":[["$__thaw_napi_undefined$",{"v":true}]]},{"nh":"17"},{"o":[["__thaw_napi_handle__",{"v":"17"}]]}]}"#).unwrap();
        let value = thaw_json_graph_decode(source.as_ptr());
        assert_eq!(thaw_json_take_graph_error(), 0);
        let items = unsafe { &*value }.as_array().unwrap();
        assert_eq!(thaw_json_is_date_shape(&items[0]), 1);
        assert_eq!(thaw_json_is_date_shape(&items[1]), 0);
        assert!(is_napi_undefined(&items[2]));
        assert!(!is_napi_undefined(&items[3]));
        let wire = thaw_json_graph_encode(value);
        let encoded: serde_json::Value = serde_json::from_str(&read_c_string(wire)).unwrap();
        assert_eq!(encoded["nodes"][1]["d"], 0);
        assert!(encoded["nodes"][2].get("o").is_some());
        assert!(encoded["nodes"][3].get("o").is_some());
        assert_eq!(encoded["nodes"][4]["nh"], "17");
        assert_eq!(encoded["napiLeases"], serde_json::json!(["119"]));
        assert_eq!(encoded["nodes"][5]["o"][0][0], "__thaw_napi_handle__");
        NAPI_RETAINS.with(|count| assert_eq!(count.get(), 2));
        thaw_json_discard_graph_wire(wire);
        unsafe { thaw_cstring_destroy(wire.cast_mut()) };
        NAPI_RELEASES.with(|count| assert_eq!(count.get(), 1));
        let cloned = unsafe { thaw_json_clone(value) };
        unsafe { thaw_json_destroy(value) };
        NAPI_RELEASES.with(|count| assert_eq!(count.get(), 1));
        unsafe { thaw_json_destroy(cloned) };
        NAPI_RELEASES.with(|count| assert_eq!(count.get(), 2));

        // A later malformed node must roll back the positive reference
        // acquired for an earlier nh node, even though no root escapes.
        let invalid = CString::new(r#"{"root":{"r":0},"nodes":[{"nh":"17"},{"unknown":1}]}"#).unwrap();
        let failed = thaw_json_graph_decode(invalid.as_ptr());
        assert_eq!(thaw_json_take_graph_error(), 1);
        unsafe { thaw_json_destroy(failed) };
        NAPI_RETAINS.with(|count| assert_eq!(count.get(), 3));
        NAPI_RELEASES.with(|count| assert_eq!(count.get(), 3));
    }

    #[test]
    fn parsed_timestamp_object_is_not_a_date() {
        let ordinary = parse(r#"{"timestamp":0}"#);
        assert_eq!(thaw_json_is_date_shape(ordinary), 0);
        let trusted = thaw_json_brand_wrapper(parse(r#"{"timestamp":0}"#));
        assert_eq!(thaw_json_is_date_shape(trusted), 1);
    }

    #[test]
    fn graph_regexp_last_index_keeps_nonfinite_number_kind() {
        for name in ["NaN", "Infinity", "-Infinity"] {
            let source = CString::new(format!(r#"{{"root":{{"r":0}},"nodes":[{{"re":["x","g","{name}"]}}]}}"#)).unwrap();
            let decoded = thaw_json_graph_decode(source.as_ptr());
            assert_eq!(thaw_json_take_graph_error(), 0);
            let last_index = regexp_wrapper_property(unsafe { &*decoded }, "lastIndex").unwrap();
            assert_eq!(unsafe { CStr::from_ptr(thaw_json_typeof(&last_index)) }.to_str().unwrap(), "number");
            let encoded: serde_json::Value = serde_json::from_str(
                &read_c_string(thaw_json_graph_encode(decoded))
            ).unwrap();
            assert_eq!(encoded["nodes"][0]["re"][2], name);
        }
    }

    #[test]
    fn stringify_rejects_only_ancestor_cycles_and_resets_error() {
        let object = thaw_json_object_new();
        thaw_json_object_set_number(object, CString::new("value").unwrap().as_ptr(), 1.0);
        thaw_json_object_set_json(object, CString::new("self").unwrap().as_ptr(), object);
        assert_eq!(
            read_c_string(thaw_json_stringify_public(object)),
            "\u{1}TypeError\u{1}Converting circular structure to JSON"
        );
        assert_eq!(thaw_json_take_stringify_error(), 1);
        assert_eq!(thaw_json_take_stringify_error(), 0);

        // The selected property list excludes the back edge.
        let keys = encode_string_array(&["value"]);
        assert_eq!(read_c_string(thaw_json_stringify_keys(object, keys, std::ptr::null())), r#"{"value":1}"#);
        assert_eq!(thaw_json_take_stringify_error(), 0);

        let child = thaw_json_object_new();
        thaw_json_object_set_number(child, CString::new("n").unwrap().as_ptr(), 2.0);
        let shared = thaw_json_object_new();
        thaw_json_object_set_json(shared, CString::new("left").unwrap().as_ptr(), child);
        thaw_json_object_set_json(shared, CString::new("right").unwrap().as_ptr(), child);
        assert_eq!(
            read_c_string(thaw_json_stringify_public(shared)),
            r#"{"left":{"n":2},"right":{"n":2}}"#
        );
        assert_eq!(thaw_json_take_stringify_error(), 0);
    }

    #[test]
    fn stringify_cyclic_array_and_spaced_replacer_report_type_error() {
        let array = thaw_json_array_new();
        thaw_json_array_push_json(array, array);
        assert_eq!(
            read_c_string(thaw_json_stringify_number_space(array, 2.0)),
            "\u{1}TypeError\u{1}Converting circular structure to JSON"
        );
        assert_eq!(thaw_json_take_stringify_error(), 1);

        let object = thaw_json_object_new();
        thaw_json_object_set_json(object, CString::new("self").unwrap().as_ptr(), object);
        let keys = encode_string_array(&["self"]);
        assert_eq!(
            read_c_string(thaw_json_stringify_keys_string_space(
                object, keys, std::ptr::null(), CString::new("  ").unwrap().as_ptr()
            )),
            "\u{1}TypeError\u{1}Converting circular structure to JSON"
        );
        assert_eq!(thaw_json_take_stringify_error(), 1);
    }

    /// User-facing `JSON.stringify` (the `_public` entry points) matches
    /// real JS's own object-vs-array asymmetry for a nested genuinely
    /// `undefined` value (here, a real napi-undefined sentinel obtained
    /// exactly the way a missing key produces one): an object field
    /// holding it is omitted entirely, an array element holding it comes
    /// back as `null` -- at any nesting depth, and unaffected by whether
    /// a `keys`/`indent` filter is also in play. The *plain*
    /// `thaw_json_stringify` (no `_public` suffix, still used internally
    /// for argument/result marshaling) must keep leaking the sentinel's
    /// raw JSON shape completely unchanged -- that's the whole reason
    /// the two functions are split apart in the first place.
    #[test]
    fn public_stringify_omits_or_nulls_a_nested_undefined_sentinel() {
        let empty = parse("{}");
        let missing_key = CString::new("nope").unwrap();
        let sentinel = thaw_json_get(empty, missing_key.as_ptr());

        let object = thaw_json_object_new();
        thaw_json_object_set_number(object, CString::new("a").unwrap().as_ptr(), 1.0);
        thaw_json_object_set_json(object, CString::new("b").unwrap().as_ptr(), sentinel);
        thaw_json_object_set_number(object, CString::new("c").unwrap().as_ptr(), 3.0);

        assert_eq!(
            read_c_string(thaw_json_stringify_public(object)),
            r#"{"a":1,"c":3}"#
        );
        assert_eq!(
            read_c_string(thaw_json_stringify_number_space(object, 2.0)),
            "{\n  \"a\": 1,\n  \"c\": 3\n}"
        );
        let key_array = encode_string_array(&["a", "b", "c"]);
        assert_eq!(
            read_c_string(thaw_json_stringify_keys(object, key_array, std::ptr::null())),
            r#"{"a":1,"c":3}"#
        );

        // The plain (non-`_public`) function is completely unaffected --
        // it still leaks the sentinel's own raw shape, exactly as
        // before this fix, since internal marshaling relies on that.
        assert_eq!(
            read_c_string(thaw_json_stringify(object)),
            r#"{"a":1,"b":{"$__thaw_napi_undefined$":true},"c":3}"#
        );

        let array = thaw_json_array_new();
        thaw_json_array_push_number(array, 1.0);
        thaw_json_array_push_json(array, sentinel);
        thaw_json_array_push_number(array, 3.0);
        assert_eq!(
            read_c_string(thaw_json_stringify_public(array)),
            "[1,null,3]"
        );
        assert_eq!(
            read_c_string(thaw_json_stringify(array)),
            r#"[1,{"$__thaw_napi_undefined$":true},3]"#
        );

        // Nested two levels deep, still correctly omitted.
        let outer = thaw_json_object_new();
        let inner = thaw_json_object_new();
        thaw_json_object_set_number(inner, CString::new("y").unwrap().as_ptr(), 2.0);
        thaw_json_object_set_json(inner, CString::new("x").unwrap().as_ptr(), sentinel);
        thaw_json_object_set_json(outer, CString::new("nested").unwrap().as_ptr(), inner);
        assert_eq!(
            read_c_string(thaw_json_stringify_public(outer)),
            r#"{"nested":{"y":2}}"#
        );
    }

    /// `JSON.stringify(x)` where `x` *itself* -- the top-level argument,
    /// not a nested field/element -- is the napi-undefined sentinel:
    /// every `_public` entry point now returns the string `"undefined"`
    /// (a documented approximation of real JS's actual `undefined`
    /// return value, which doesn't fit this always-a-string ABI --
    /// see `top_level_undefined_string`'s own doc comment) instead of
    /// leaking the sentinel's raw JSON shape (`{"$__thaw_napi_undefined
    /// $":true}"`, confirmed as the pre-fix behavior below via the
    /// still-unaffected plain `thaw_json_stringify`).
    #[test]
    fn public_stringify_of_a_bare_top_level_undefined_sentinel_returns_the_string_undefined() {
        let empty = parse("{}");
        let missing_key = CString::new("nope").unwrap();
        let sentinel = thaw_json_get(empty, missing_key.as_ptr());

        assert_eq!(
            read_c_string(thaw_json_stringify_public(sentinel)),
            "undefined"
        );
        assert_eq!(
            read_c_string(thaw_json_stringify_number_space(sentinel, 2.0)),
            "undefined"
        );
        assert_eq!(
            read_c_string(thaw_json_stringify_string_space(
                sentinel,
                CString::new("  ").unwrap().as_ptr()
            )),
            "undefined"
        );
        let key_array = encode_string_array(&["a"]);
        assert_eq!(
            read_c_string(thaw_json_stringify_keys(sentinel, key_array, std::ptr::null())),
            "undefined"
        );
        assert_eq!(
            read_c_string(thaw_json_stringify_keys_number_space(
                sentinel, key_array, std::ptr::null(), 2.0
            )),
            "undefined"
        );

        // The plain (non-`_public`) function stays unaffected -- this
        // confirms the sentinel's raw shape is exactly what would
        // otherwise leak through here.
        assert_eq!(
            read_c_string(thaw_json_stringify(sentinel)),
            r#"{"$__thaw_napi_undefined$":true}"#
        );
    }

    #[test]
    fn invalid_json_parses_as_null_instead_of_crashing() {
        let value = parse("{not valid json");
        assert_eq!(read_c_string(thaw_json_stringify(value)), "null");
    }

    #[test]
    fn constructs_dynamic_call_arrays_and_objects() {
        let arguments = thaw_json_array_new();
        thaw_json_array_push_number(arguments, 42.0);
        let text = CString::new("thaw").unwrap();
        thaw_json_array_push_string(arguments, text.as_ptr());
        thaw_json_array_push_bool(arguments, 1);
        assert_eq!(
            read_c_string(thaw_json_stringify(arguments)),
            r#"[42,"thaw",true]"#
        );

        let object = thaw_json_object_new();
        let answer = CString::new("answer").unwrap();
        thaw_json_object_set_number(object, answer.as_ptr(), 42.0);
        let nested = CString::new("arguments").unwrap();
        thaw_json_object_set_json(object, nested.as_ptr(), arguments);
        let decoded: serde_json::Value =
            serde_json::from_str(&read_c_string(thaw_json_stringify(object))).unwrap();
        assert_eq!(
            decoded,
            serde_json::json!({"answer": 42, "arguments": [42, "thaw", true]})
        );
    }

    #[test]
    fn converts_number_arrays_between_json_and_native_layout() {
        let value = parse("[1.5, 2, false]");
        let native = thaw_json_to_number_array(value);
        assert!(!native.is_null());
        unsafe {
            assert_eq!((native as *const i64).read(), 3);
            assert_eq!((native.add(8) as *const f64).read(), 1.5);
            assert_eq!((native.add(16) as *const f64).read(), 2.0);
            assert_eq!((native.add(24) as *const f64).read(), 0.0);
        }

        let round_trip = thaw_json_from_number_array(native);
        assert_eq!(read_c_string(thaw_json_stringify(round_trip)), "[1.5,2,0]");
    }

    #[test]
    fn dynamic_number_and_key_semantics() {
        let negative_zero = leak(number_value(-0.0));
        assert_eq!(unsafe { thaw_json_is_object_like(negative_zero) }, 0);
        assert_eq!(unsafe { thaw_json_is_object_like(parse("[]")) }, 1);
        assert_eq!(unsafe { thaw_json_is_object_like(parse("{}")) }, 1);
        assert_eq!(thaw_json_as_number(negative_zero).to_bits(), (-0.0f64).to_bits());
        assert_eq!(unsafe { thaw_json_object_is_number(negative_zero, -0.0) }, 1);
        assert!(thaw_json_as_number(parse("[true]")).is_nan());
        assert!(thaw_json_as_number(parse("{}")).is_nan());
        assert_eq!(thaw_json_as_bool(leak(number_value(f64::NAN))), 0);
        assert_eq!(unsafe {
            thaw_json_same_value_zero(
                leak(number_value(f64::NAN)),
                leak(number_value(f64::NAN)),
            )
        }, 1);
        let surrogate = parse(r#""\ud800""#);
        assert_eq!(unsafe { thaw_json_strict_equal(surrogate, surrogate) }, 1);
    }

    #[test]
    fn dynamic_array_holes_and_string_sources() {
        let array = parse("[1,2]");
        let zero = c"0".as_ptr();
        assert_eq!(thaw_json_object_delete(array, zero), 1);
        assert_eq!(unsafe { thaw_json_has_own(array, zero) }, 0);
        assert_eq!(unsafe { thaw_json_is_undefined(thaw_json_get(array, zero)) }, 1);
        assert_eq!(thaw_json_array_length(array), 2);
        assert_eq!(unsafe { (thaw_json_keys(array) as *const i64).read() }, 1);

        let target = thaw_json_object_new();
        let source = parse(r#""ab""#);
        unsafe { thaw_json_object_assign(target, source) };
        assert_eq!(read_c_string(thaw_json_stringify(target)), r#"{"0":"a","1":"b"}"#);
    }

    #[test]
    fn json_parse_rejects_trailing_text_and_entries_have_full_handles() {
        let trailing = parse("1 2");
        assert_eq!(thaw_json_take_parse_error(), 1);
        assert_eq!(read_c_string(thaw_json_stringify(trailing)), "null");

        let entries = unsafe { thaw_json_entries(parse(r#"{"a":1}"#)) };
        let handle = unsafe { (entries.add(8) as *const *mut u8).read() };
        assert!(!handle.is_null());
        assert!(unsafe { (handle.add(8) as *const *const u8).read() }.is_null());
    }

    #[test]
    fn from_entries_rejects_absent_outer_slot() {
        // Both ABI headers are read as aligned 64-bit integers. The first
        // presence byte after the header is zero, so slot 0 is absent.
        let entries = [1_u64, 0];
        let presence = [1_u64, 0];
        let result = unsafe {
            thaw_json_object_from_number_entries(
                entries.as_ptr().cast::<u8>(),
                presence.as_ptr().cast::<u8>(),
            )
        };
        assert_eq!(thaw_json_take_from_entries_error(), 1);
        assert_eq!(thaw_json_take_from_entries_error(), 0);
        assert_eq!(read_c_string(thaw_json_stringify(result)), "{}");
        unsafe { thaw_json_destroy(result) };
    }

    #[test]
    fn prototype_cycle_is_rejected_and_metadata_lives_until_last_alias() {
        let object = parse("{}");
        let alias = leak(unsafe { &*object }.clone());
        let prototype = parse(r#"{"inherited":1}"#);
        let key = object_identity_key(unsafe { &*object }).unwrap();
        unsafe { thaw_json_set_prototype(object, prototype) };
        assert_eq!(thaw_json_take_prototype_error(), 0);
        unsafe { thaw_json_set_prototype(prototype, object) };
        assert_eq!(thaw_json_take_prototype_error(), 1);
        assert!(PROTOTYPES.with(|table| table.borrow().contains_key(&key)));
        unsafe { thaw_json_destroy(object) };
        assert!(PROTOTYPES.with(|table| table.borrow().contains_key(&key)));
        unsafe { thaw_json_destroy(alias) };
        assert!(!PROTOTYPES.with(|table| table.borrow().contains_key(&key)));
        unsafe { thaw_json_destroy(prototype) };
    }

    #[test]
    fn destroy_releases_isolated_cycle_but_preserves_external_alias() {
        let object = Value::shared_object(indexmap::IndexMap::new());
        let Value::Object(fields) = &object else { unreachable!() };
        let weak = Rc::downgrade(fields);
        shared_object_ref_mut(fields).insert(b"self".to_vec(), object.clone());
        let first = leak(object.clone());
        let second = leak(object);
        unsafe { thaw_json_destroy(first) };
        assert!(weak.upgrade().is_some());
        let self_value = thaw_json_get(second, c"self".as_ptr());
        assert_eq!(unsafe { thaw_json_strict_equal(second, self_value) }, 1);
        unsafe { thaw_json_destroy(self_value); thaw_json_destroy(second) };
        assert!(weak.upgrade().is_none());
    }

    #[test]
    fn destroy_retains_prototype_child_alias_then_releases_host_once() {
        HOST_RELEASES.with(|count| count.set(0));
        let parent = Value::shared_object(indexmap::IndexMap::new());
        let child = Value::shared_object(indexmap::IndexMap::new());
        let Value::Object(parent_fields) = &parent else { unreachable!() };
        let Value::Object(child_fields) = &child else { unreachable!() };
        let parent_weak = Rc::downgrade(parent_fields);
        let child_weak = Rc::downgrade(child_fields);
        shared_object_ref_mut(child_fields).insert(b"parent".to_vec(), parent.clone());
        shared_object_ref_mut(child_fields).insert(b"host".to_vec(),
            Value::Host(Rc::new(HostLease { handle: 47, release: count_host_release })));
        let parent = leak(parent);
        let child = leak(child);
        unsafe { thaw_json_set_prototype(parent, child) };
        assert_eq!(thaw_json_take_prototype_error(), 0);
        unsafe { thaw_json_destroy(parent) };
        HOST_RELEASES.with(|count| assert_eq!(count.get(), 0));
        assert!(parent_weak.upgrade().is_some());
        assert!(child_weak.upgrade().is_some());
        unsafe { thaw_json_destroy(child) };
        HOST_RELEASES.with(|count| assert_eq!(count.get(), 1));
        assert!(parent_weak.upgrade().is_none());
        assert!(child_weak.upgrade().is_none());
    }

    #[test]
    fn host_release_reenters_destroy_without_temporary_graph_aliases() {
        HOST_RELEASES.with(|count| count.set(0));
        let outer = Value::shared_object(indexmap::IndexMap::new());
        let nested = Value::shared_object(indexmap::IndexMap::new());
        let Value::Object(outer_fields) = &outer else { unreachable!() };
        let Value::Object(nested_fields) = &nested else { unreachable!() };
        let outer_weak = Rc::downgrade(outer_fields);
        let nested_weak = Rc::downgrade(nested_fields);
        shared_object_ref_mut(nested_fields).insert(b"self".to_vec(), nested.clone());
        shared_object_ref_mut(outer_fields).insert(b"self".to_vec(), outer.clone());
        shared_object_ref_mut(outer_fields).insert(b"nested".to_vec(), nested.clone());
        shared_object_ref_mut(outer_fields).insert(b"host".to_vec(),
            Value::Host(Rc::new(HostLease { handle: 48, release: release_and_destroy_nested })));
        let outer = leak(outer);
        let nested = leak(nested);
        NESTED_DESTROY.with(|slot| slot.set(nested));
        unsafe { thaw_json_destroy(outer) };
        HOST_RELEASES.with(|count| assert_eq!(count.get(), 1));
        NESTED_DESTROY.with(|slot| assert!(slot.get().is_null()));
        assert!(outer_weak.upgrade().is_none());
        assert!(nested_weak.upgrade().is_none());
    }

    #[test]
    fn host_error_text_keeps_embedded_nul() {
        set_host_error("before\0after".to_string());
        let text = thaw_json_take_host_error();
        assert_eq!(unsafe { CStr::from_ptr(text) }.to_bytes(), b"before\0after");
        unsafe { thaw_cstring_destroy(text.cast_mut()) };
        assert!(thaw_json_take_host_error().is_null());
    }

    #[test]
    fn jit_dictionary_callbacks_preserve_presence_and_typed_failures() {
        unsafe extern "C" {
            fn thaw_object_set_state(object: *const u8, operation: u8) -> bool;
        }
        let object = parse(r#"{"first":1}"#);
        let mut present = 1;
        let mut error = std::ptr::null();
        let missing = thaw_jit_dictionary_get(
            0, object, c"missing".as_ptr(), &mut present, &mut error,
        );
        assert!(missing.is_nan());
        assert_eq!(present, 0);
        assert!(error.is_null());
        present = 1;
        let null_value = parse(r#"{"x":null}"#);
        thaw_jit_dictionary_get(0, null_value, c"x".as_ptr(), &mut present, &mut error);
        assert_eq!(present, 2);

        let sealed = parse(r#"{"first":1}"#);
        assert!(unsafe { thaw_object_set_state(thaw_json_state_key(sealed), 2) });
        let source = parse(r#"{"first":9,"new":10}"#);
        unsafe { thaw_jit_dictionary_query(11, sealed, source.cast(), &mut error) };
        assert!(read_c_string(error).starts_with("\u{1}TypeError\u{1}"));
        assert_eq!(thaw_json_as_number(thaw_json_get(sealed, c"first".as_ptr())), 9.0);
        assert_eq!(unsafe { thaw_json_has_own(sealed, c"new".as_ptr()) }, 0);

        error = std::ptr::null();
        assert_eq!(thaw_jit_dictionary_mutate(3, sealed, c"first".as_ptr(), 0.0, &mut error), 0.0);
        assert!(error.is_null());
        assert_eq!(thaw_jit_dictionary_mutate(4, sealed, c"first".as_ptr(), 0.0, &mut error), 1.0);
        assert!(read_c_string(error).starts_with("\u{1}TypeError\u{1}"));

        error = std::ptr::null();
        let null_receiver = parse("null");
        unsafe { thaw_jit_dictionary_query(1, null_receiver, std::ptr::null(), &mut error) };
        assert!(read_c_string(error).starts_with("\u{1}TypeError\u{1}"));
        for operation in [0, 23] {
            error = std::ptr::null();
            unsafe { thaw_jit_dictionary_query(operation, null_receiver, c"x".as_ptr(), &mut error) };
            assert!(read_c_string(error).starts_with("\u{1}TypeError\u{1}"));
        }
        error = std::ptr::null();
        thaw_jit_dictionary_mutate(4, null_receiver, c"x".as_ptr(), 0.0, &mut error);
        assert!(read_c_string(error).starts_with("\u{1}TypeError\u{1}"));

        let entries = [1_u64, 0];
        let presence = [1_u64, 0];
        let handle = [entries.as_ptr() as u64, presence.as_ptr() as u64];
        error = std::ptr::null();
        let result = unsafe {
            thaw_jit_dictionary_query(8, handle.as_ptr().cast_mut().cast(), std::ptr::null(), &mut error)
        };
        assert!(read_c_string(error).starts_with("\u{1}TypeError\u{1}"));
        unsafe { thaw_json_destroy(result.to_bits() as usize as *mut Value) };
        unsafe { thaw_json_destroy(object) };
        unsafe { thaw_json_destroy(null_value) };
        unsafe { thaw_json_destroy(null_receiver) };
        unsafe { thaw_json_destroy(sealed) };
        unsafe { thaw_json_destroy(source) };
    }

    #[test]
    fn bigint_receiver_graph_uses_exact_decimal_and_public_stringify_errors() {
        let value = thaw_json_receiver_bigint(9_007_199_254_740_993);
        assert_eq!(read_c_string(thaw_json_graph_encode(value)),
            r#"{"root":{"bi":"9007199254740993"},"nodes":[],"leases":[],"napiLeases":[]}"#);
        let graph = CString::new(r#"{"root":{"bi":"-18446744073709551617"},"nodes":[]}"#).unwrap();
        let decoded = thaw_json_graph_decode(graph.as_ptr());
        assert_eq!(read_c_string(thaw_json_as_string(decoded)), "-18446744073709551617");
        assert_eq!(unsafe { CStr::from_ptr(thaw_json_typeof(decoded)) }.to_bytes(), b"bigint");
        let _ = thaw_json_stringify_public(decoded);
        assert_eq!(read_c_string(thaw_json_take_host_error()),
            "\u{1}TypeError\u{1}Do not know how to serialize a BigInt");
        unsafe { thaw_json_destroy(value); thaw_json_destroy(decoded); }
    }

    #[test]
    fn destroying_last_host_root_does_not_leave_cleanup_error() {
        std::thread::spawn(|| {
            CLEANUP_HOST_RELEASES.with(|count| count.set(0));
            HOST_OPERATIONS.with(|slot| slot.set(Some(HostOperations {
                retain: retain_typed_scope_host,
                release: release_with_cleanup_host_error,
                get: reentrant_typed_scope_get,
                query: cleanup_test_host_query,
                date_set: unused_typed_scope_date,
                set: unused_typed_scope_set,
                predicate: unused_typed_scope_predicate,
                enumerate: unused_typed_scope_text,
            })));
            let first = leak(Value::Host(Rc::new(HostLease {
                handle: 11,
                release: release_with_cleanup_host_error,
            })));
            unsafe { thaw_json_destroy(first) };
            CLEANUP_HOST_RELEASES.with(|count| assert_eq!(count.get(), 1));
            assert!(thaw_json_take_host_error().is_null());

            let second = leak(Value::Host(Rc::new(HostLease {
                handle: 12,
                release: release_with_cleanup_host_error,
            })));
            assert_eq!(unsafe { CStr::from_ptr(thaw_json_typeof(second)) }.to_bytes(), b"object");
            unsafe { thaw_json_destroy(second) };
            CLEANUP_HOST_RELEASES.with(|count| assert_eq!(count.get(), 2));
            assert!(thaw_json_take_host_error().is_null());

            set_host_error("original operation error".into());
            let third = leak(Value::Host(Rc::new(HostLease {
                handle: 13,
                release: release_with_cleanup_host_error,
            })));
            unsafe { thaw_json_destroy(third) };
            CLEANUP_HOST_RELEASES.with(|count| assert_eq!(count.get(), 3));
            let original = thaw_json_take_host_error();
            assert_eq!(read_c_string(original), "original operation error");
            unsafe { thaw_arena::destroy_string(original.cast_mut()) };
        }).join().unwrap();
    }

    #[test]
    fn typed_decode_scope_rollback_preserves_original_host_error() {
        std::thread::spawn(|| {
            HOST_OPERATIONS.with(|slot| slot.set(Some(HostOperations {
                retain: retain_typed_scope_host,
                release: release_with_cleanup_host_error,
                get: reentrant_typed_scope_get,
                query: unused_typed_scope_text,
                date_set: unused_typed_scope_date,
                set: unused_typed_scope_set,
                predicate: unused_typed_scope_predicate,
                enumerate: unused_typed_scope_text,
            })));
            let host = Value::Host(Rc::new(HostLease {
                handle: 12, release: release_with_cleanup_host_error,
            }));
            let array = leak(Value::shared_array(vec![host]));
            set_host_error("original conversion error".into());
            thaw_json_typed_decode_scope_begin();
            let child = thaw_json_index(array, 0.0, std::ptr::null());
            assert_eq!(unsafe { thaw_json_handle_id(child) }, 12);
            thaw_json_typed_decode_scope_end(1);
            let original = thaw_json_take_host_error();
            assert_eq!(read_c_string(original), "original conversion error");
            unsafe { thaw_arena::destroy_string(original.cast_mut()) };

            thaw_json_typed_decode_scope_begin();
            let child = thaw_json_index(array, 0.0, std::ptr::null());
            assert_eq!(unsafe { thaw_json_handle_id(child) }, 12);
            thaw_json_typed_decode_scope_end(1);
            assert!(thaw_json_take_host_error().is_null());
            let discarded = CString::new(r#"{"root":{"v":null},"nodes":[],"leases":[12]}"#).unwrap();
            thaw_json_discard_graph_wire(discarded.as_ptr());
            assert!(thaw_json_take_host_error().is_null());
            let value = parse(r#"{"ok":1}"#);
            let ok = thaw_json_get(value, c"ok".as_ptr());
            assert_eq!(thaw_json_as_number(ok), 1.0);
            unsafe { thaw_json_destroy(ok); thaw_json_destroy(value); thaw_json_destroy(array) };
            // The source's final Host lease may itself set an error; it is
            // independent of the completed conversion.
            let final_error = thaw_json_take_host_error();
            if !final_error.is_null() {
                unsafe { thaw_arena::destroy_string(final_error.cast_mut()) };
            }
        }).join().unwrap();
    }

    #[test]
    fn typed_decode_scope_tracks_dynamic_retains_merges_and_excludes_reentry() {
        std::thread::spawn(|| {
            HOST_RELEASES.with(|count| count.set(0));
            TYPED_SCOPE_RETAINS.with(|count| count.set(0));
            HOST_OPERATIONS.with(|slot| slot.set(Some(HostOperations {
                retain: retain_typed_scope_host,
                release: count_host_release,
                get: reentrant_typed_scope_get,
                query: unused_typed_scope_text,
                date_set: unused_typed_scope_date,
                set: unused_typed_scope_set,
                predicate: unused_typed_scope_predicate,
                enumerate: unused_typed_scope_text,
            })));

            let host = Value::Host(Rc::new(HostLease {
                handle: 7, release: count_host_release,
            }));
            let array = leak(Value::shared_array(vec![host.clone(), host]));
            thaw_json_typed_decode_scope_begin();
            for index in 0..2 {
                let child = thaw_json_index(array, index as f64, std::ptr::null());
                assert_eq!(unsafe { thaw_json_handle_id(child) }, 7);
                unsafe { thaw_json_destroy(child) };
            }
            thaw_json_typed_decode_scope_end(1);
            TYPED_SCOPE_RETAINS.with(|count| assert_eq!(count.get(), 2));
            HOST_RELEASES.with(|count| assert_eq!(count.get(), 2));

            let nested = leak(Value::shared_array(vec![Value::shared_object(
                indexmap::IndexMap::from([(b"ok".to_vec(), number_value(1.0))]),
            )]));
            let Value::Array(items) = (unsafe { &*nested }) else { unreachable!() };
            let Value::Object(fields) = &shared_array_ref(items)[0] else { unreachable!() };
            let before = Rc::strong_count(fields);
            thaw_json_typed_decode_scope_begin();
            thaw_json_typed_decode_scope_begin();
            let _child = thaw_json_index(nested, 0.0, std::ptr::null());
            assert_eq!(Rc::strong_count(fields), before + 1);
            thaw_json_typed_decode_scope_end(2);
            thaw_json_typed_decode_scope_end(1);
            assert_eq!(Rc::strong_count(fields), before);

            let source = parse(r#"{"x":{"ok":1}}"#);
            REENTRANT_SOURCE.with(|slot| slot.set(source));
            let host_root = leak(Value::Host(Rc::new(HostLease {
                handle: 9, release: count_host_release,
            })));
            thaw_json_typed_decode_scope_begin();
            let _outer = thaw_json_get(host_root, c"reenter".as_ptr());
            thaw_json_typed_decode_scope_end(1);
            let escaped = REENTRANT_CHILD.with(|slot| slot.replace(std::ptr::null_mut()));
            assert!(!escaped.is_null());
            let ok = thaw_json_get(escaped, c"ok".as_ptr());
            assert_eq!(thaw_json_as_number(ok), 1.0);
            unsafe {
                thaw_json_destroy(ok);
                thaw_json_destroy(escaped);
                thaw_json_destroy(source);
                thaw_json_destroy(host_root);
                thaw_json_destroy(array);
                thaw_json_destroy(nested);
            }
        }).join().unwrap();
    }


    #[test]
    fn callback_origin_transfer_owns_one_handle_until_closure_reclaim() {
        std::thread::spawn(|| {
            HOST_RELEASES.with(|count| count.set(0));
            TYPED_SCOPE_RETAINS.with(|count| count.set(0));
            HOST_OPERATIONS.with(|slot| slot.set(Some(HostOperations {
                retain: retain_typed_scope_host,
                release: count_host_release,
                get: reentrant_typed_scope_get,
                query: callback_origin_test_query,
                date_set: unused_typed_scope_date,
                set: unused_typed_scope_set,
                predicate: unused_typed_scope_predicate,
                enumerate: unused_typed_scope_text,
            })));
            thaw_arena::thaw_arena_enable_tracing();
            let host = leak(Value::Host(Rc::new(HostLease {
                handle: 42, release: count_host_release,
            })));
            thaw_json_typed_decode_scope_begin();
            assert_eq!(unsafe { thaw_json_callback_origin_acquire(host) }, 42);
            let closure = thaw_arena::thaw_arena_alloc(24, 8);
            assert!(!closure.is_null());
            assert_eq!(thaw_json_register_callback_origin(closure, 42), 1);
            assert_eq!(thaw_json_callback_origin_handle(closure), 42);
            thaw_json_typed_decode_scope_end(0);
            HOST_RELEASES.with(|count| assert_eq!(count.get(), 0));
            let root = thaw_arena::ArenaRoot::new(closure as usize);
            thaw_arena::thaw_arena_reset();
            HOST_RELEASES.with(|count| assert_eq!(count.get(), 0));
            drop(root);
            thaw_arena::thaw_arena_reset();
            HOST_RELEASES.with(|count| assert_eq!(count.get(), 1));
            assert_eq!(thaw_json_callback_origin_handle(closure), 0);
            thaw_json_typed_decode_scope_begin();
            assert_eq!(unsafe { thaw_json_callback_origin_acquire(host) }, 42);
            thaw_json_typed_decode_scope_end(1);
            HOST_RELEASES.with(|count| assert_eq!(count.get(), 2));
            let forged = parse(r#"{"__thaw_js_handle_id__":42}"#);
            assert_eq!(unsafe { thaw_json_callback_origin_acquire(forged) }, 0);
            let branded = thaw_json_brand_wrapper(parse(r#"{"__thaw_js_handle_id__":42}"#));
            thaw_json_typed_decode_scope_begin();
            assert_eq!(unsafe { thaw_json_callback_origin_acquire(branded) }, 42);
            thaw_json_typed_decode_scope_end(1);
            TYPED_SCOPE_RETAINS.with(|count| assert_eq!(count.get(), 3));
            HOST_RELEASES.with(|count| assert_eq!(count.get(), 3));
            unsafe { thaw_json_destroy(forged); thaw_json_destroy(branded); thaw_json_destroy(host) };
            HOST_RELEASES.with(|count| assert_eq!(count.get(), 4));
        }).join().unwrap();
    }

    #[test]
    fn callback_origin_nested_failure_rolls_back_and_rejects_non_functions() {
        std::thread::spawn(|| {
            HOST_RELEASES.with(|count| count.set(0));
            TYPED_SCOPE_RETAINS.with(|count| count.set(0));
            HOST_OPERATIONS.with(|slot| slot.set(Some(HostOperations {
                retain: retain_typed_scope_host,
                release: count_host_release,
                get: reentrant_typed_scope_get,
                query: callback_origin_test_query,
                date_set: unused_typed_scope_date,
                set: unused_typed_scope_set,
                predicate: unused_typed_scope_predicate,
                enumerate: unused_typed_scope_text,
            })));
            thaw_arena::thaw_arena_enable_tracing();
            let invalid = leak(Value::Host(Rc::new(HostLease {
                handle: 43, release: count_host_release,
            })));
            thaw_json_typed_decode_scope_begin();
            assert_eq!(unsafe { thaw_json_callback_origin_acquire(invalid) }, 0);
            TYPED_SCOPE_RETAINS.with(|count| assert_eq!(count.get(), 0));
            let error = thaw_json_take_host_error();
            assert_eq!(to_str(error), "Callback origin is not a function");
            unsafe { thaw_arena::destroy_string(error.cast_mut()) };
            thaw_json_typed_decode_scope_end(1);
            let valid = leak(Value::Host(Rc::new(HostLease {
                handle: 42, release: count_host_release,
            })));
            thaw_json_typed_decode_scope_begin();
            thaw_json_typed_decode_scope_begin();
            assert_eq!(unsafe { thaw_json_callback_origin_acquire(valid) }, 42);
            let closure = thaw_arena::thaw_arena_alloc(24, 8);
            assert_eq!(thaw_json_register_callback_origin(closure, 42), 1);
            thaw_json_typed_decode_scope_end(2);
            assert_eq!(thaw_json_callback_origin_handle(closure), 42);
            thaw_json_typed_decode_scope_end(1);
            assert_eq!(thaw_json_callback_origin_handle(closure), 0);
            TYPED_SCOPE_RETAINS.with(|count| assert_eq!(count.get(), 1));
            HOST_RELEASES.with(|count| assert_eq!(count.get(), 1));
            thaw_arena::thaw_arena_reset();
            HOST_RELEASES.with(|count| assert_eq!(count.get(), 1));
            unsafe { thaw_json_destroy(valid); thaw_json_destroy(invalid); }
        }).join().unwrap();
    }

    #[test]
    fn arena_owned_host_receiver_survives_root_then_releases() {
        std::thread::spawn(|| {
            HOST_RELEASES.with(|count| count.set(0));
            thaw_arena::thaw_arena_enable_tracing();
            let receiver = leak(Value::Host(Rc::new(HostLease {
                handle: 29,
                release: count_host_release,
            })));
            assert_eq!(thaw_json_track_arena_owned_root(receiver), receiver);
            let cell = thaw_arena::thaw_arena_alloc(
                std::mem::size_of::<usize>(), std::mem::align_of::<usize>());
            assert!(!cell.is_null());
            unsafe { (cell as *mut usize).write(receiver as usize) };
            let root = thaw_arena::ArenaRoot::new(cell as usize);
            thaw_arena::thaw_arena_reset();
            HOST_RELEASES.with(|count| assert_eq!(count.get(), 0));
            assert!(ARENA_OWNED_JSON_ROOTS.with(|roots| roots.borrow().contains_key(&(receiver as usize))));
            drop(root);
            thaw_arena::thaw_arena_reset();
            HOST_RELEASES.with(|count| assert_eq!(count.get(), 1));
            assert!(!ARENA_OWNED_JSON_ROOTS.with(|roots| roots.borrow().contains_key(&(receiver as usize))));
        }).join().unwrap();
    }

    #[test]
    fn moving_tracked_receiver_into_object_does_not_release_twice() {
        std::thread::spawn(|| {
            HOST_RELEASES.with(|count| count.set(0));
            thaw_arena::thaw_arena_enable_tracing();
            let receiver = leak(Value::Host(Rc::new(HostLease {
                handle: 30,
                release: count_host_release,
            })));
            assert_eq!(thaw_json_track_arena_owned_root(receiver), receiver);
            let object = thaw_json_object_new();
            unsafe { thaw_json_object_set_json_owned(object, c"receiver".as_ptr(), receiver) };
            assert!(!ARENA_OWNED_JSON_ROOTS.with(|roots| roots.borrow().contains_key(&(receiver as usize))));
            HOST_RELEASES.with(|count| assert_eq!(count.get(), 0));
            unsafe { thaw_json_destroy(object) };
            HOST_RELEASES.with(|count| assert_eq!(count.get(), 1));
            thaw_arena::thaw_arena_reset();
            HOST_RELEASES.with(|count| assert_eq!(count.get(), 1));
        }).join().unwrap();
    }

    #[test]
    fn nonarena_owner_edge_keeps_escaped_receiver_until_owner_released() {
        std::thread::spawn(|| {
            HOST_RELEASES.with(|count| count.set(0));
            thaw_arena::thaw_arena_enable_tracing();
            let receiver = leak(Value::Host(Rc::new(HostLease {
                handle: 31,
                release: count_host_release,
            })));
            assert_eq!(thaw_json_track_arena_owned_root(receiver), receiver);
            // A settled Promise is also a non-arena Box with a registered
            // owner→result edge; its caller keeps the Promise handle rooted.
            let owner = Box::into_raw(Box::new(0_usize));
            thaw_arena::replace_reference(owner as usize, 0, receiver as usize);
            let root = thaw_arena::ArenaRoot::new(owner as usize);
            thaw_arena::thaw_arena_reset();
            HOST_RELEASES.with(|count| assert_eq!(count.get(), 0));
            drop(root);
            thaw_arena::forget_references(owner as usize);
            unsafe { drop(Box::from_raw(owner)) };
            thaw_arena::thaw_arena_reset();
            HOST_RELEASES.with(|count| assert_eq!(count.get(), 1));
        }).join().unwrap();
    }

    #[test]
    fn live_host_helper_enrolls_its_fresh_owned_box() {
        std::thread::spawn(|| {
            HOST_RELEASES.with(|count| count.set(0));
            TYPED_SCOPE_RETAINS.with(|count| count.set(0));
            HOST_OPERATIONS.with(|slot| slot.set(Some(HostOperations {
                retain: retain_typed_scope_host,
                release: count_host_release,
                get: reentrant_typed_scope_get,
                query: owned_receiver_test_host_query,
                date_set: unused_typed_scope_date,
                set: unused_typed_scope_set,
                predicate: unused_typed_scope_predicate,
                enumerate: unused_typed_scope_text,
            })));
            thaw_arena::thaw_arena_enable_tracing();
            let value = thaw_json_host_from_borrowed_handle(32);
            assert!(!value.is_null());
            assert!(ARENA_OWNED_JSON_ROOTS.with(|roots| roots.borrow().contains_key(&(value as usize))));
            TYPED_SCOPE_RETAINS.with(|count| assert_eq!(count.get(), 1));
            thaw_arena::thaw_arena_reset();
            HOST_RELEASES.with(|count| assert_eq!(count.get(), 1));
            assert!(!ARENA_OWNED_JSON_ROOTS.with(|roots| roots.borrow().contains_key(&(value as usize))));
        }).join().unwrap();
    }

    #[test]
    fn arena_reset_release_reentry_does_not_destroy_second_root_twice() {
        std::thread::spawn(|| {
            HOST_RELEASES.with(|count| count.set(0));
            TRACKED_RELEASE_REENTERED.with(|active| active.set(false));
            thaw_arena::thaw_arena_enable_tracing();
            let first = leak(Value::Host(Rc::new(HostLease {
                handle: 41, release: release_and_destroy_other_tracked_root,
            })));
            let second = leak(Value::Host(Rc::new(HostLease {
                handle: 42, release: release_and_destroy_other_tracked_root,
            })));
            TRACKED_REENTRANT_ROOTS.with(|roots| roots.set((first as usize, second as usize)));
            assert_eq!(thaw_json_track_arena_owned_root(first), first);
            assert_eq!(thaw_json_track_arena_owned_root(second), second);
            thaw_arena::thaw_arena_reset();
            HOST_RELEASES.with(|count| assert_eq!(count.get(), 2));
            assert!(ARENA_OWNED_JSON_ROOTS.with(|roots| roots.borrow().is_empty()));
        }).join().unwrap();
    }

    #[test]
    fn dependency_compile_writable_expression() {
        unsafe extern "C" {
            fn thaw_object_set_state(object: *const u8, operation: u8) -> bool;
        }

        let assert_number = |object: *mut Value, key: *const c_char, expected: f64| {
            let value = thaw_json_get(object, key);
            assert_eq!(thaw_json_as_number(value), expected);
            unsafe { thaw_json_destroy(value) };
        };

        let extensible = parse(r#"{"own":1}"#);
        let extensible_alias = leak(unsafe { (&*extensible).clone() });
        assert_eq!(unsafe { thaw_json_strict_equal(extensible, extensible_alias) }, 1);
        thaw_json_object_set_number(extensible, c"own".as_ptr(), 2.0);
        thaw_json_object_set_number(extensible_alias, c"new".as_ptr(), 3.0);
        assert_number(extensible, c"own".as_ptr(), 2.0);
        assert_number(extensible_alias, c"new".as_ptr(), 3.0);
        let extensible_source = parse(r#"{"own":4,"assigned":5}"#);
        unsafe { thaw_json_object_assign(extensible_alias, extensible_source) };
        assert_eq!(thaw_json_take_assign_error(), 0);
        assert_number(extensible, c"own".as_ptr(), 4.0);
        assert_number(extensible, c"assigned".as_ptr(), 5.0);

        let prevented = parse(r#"{"own":1}"#);
        let prevented_alias = leak(unsafe { (&*prevented).clone() });
        assert_eq!(unsafe { thaw_json_strict_equal(prevented, prevented_alias) }, 1);
        assert!(unsafe { thaw_object_set_state(thaw_json_state_key(prevented), 1) });
        thaw_json_object_set_number(prevented, c"own".as_ptr(), 2.0);
        thaw_json_object_set_number(prevented_alias, c"new".as_ptr(), 3.0);
        assert_number(prevented_alias, c"own".as_ptr(), 2.0);
        assert_eq!(unsafe { thaw_json_has_own(prevented, c"new".as_ptr()) }, 0);
        let prevented_source = parse(r#"{"own":4,"new":5}"#);
        unsafe { thaw_json_object_assign(prevented_alias, prevented_source) };
        assert_eq!(thaw_json_take_assign_error(), 1);
        assert_number(prevented, c"own".as_ptr(), 4.0);
        assert_eq!(unsafe { thaw_json_has_own(prevented_alias, c"new".as_ptr()) }, 0);

        let sealed = parse(r#"{"own":1}"#);
        let sealed_alias = leak(unsafe { (&*sealed).clone() });
        assert_eq!(unsafe { thaw_json_strict_equal(sealed, sealed_alias) }, 1);
        assert!(unsafe { thaw_object_set_state(thaw_json_state_key(sealed), 2) });
        thaw_json_object_set_number(sealed, c"own".as_ptr(), 2.0);
        thaw_json_object_set_number(sealed_alias, c"new".as_ptr(), 3.0);
        assert_number(sealed_alias, c"own".as_ptr(), 2.0);
        assert_eq!(unsafe { thaw_json_has_own(sealed, c"new".as_ptr()) }, 0);
        let sealed_source = parse(r#"{"own":4,"new":5}"#);
        unsafe { thaw_json_object_assign(sealed, sealed_source) };
        assert_eq!(thaw_json_take_assign_error(), 1);
        assert_number(sealed_alias, c"own".as_ptr(), 4.0);
        assert_eq!(unsafe { thaw_json_has_own(sealed_alias, c"new".as_ptr()) }, 0);

        let frozen = parse(r#"{"own":1}"#);
        let frozen_alias = leak(unsafe { (&*frozen).clone() });
        assert_eq!(unsafe { thaw_json_strict_equal(frozen, frozen_alias) }, 1);
        assert!(unsafe { thaw_object_set_state(thaw_json_state_key(frozen), 3) });
        thaw_json_object_set_number(frozen, c"own".as_ptr(), 2.0);
        thaw_json_object_set_number(frozen_alias, c"new".as_ptr(), 3.0);
        assert_number(frozen, c"own".as_ptr(), 1.0);
        assert_eq!(unsafe { thaw_json_has_own(frozen_alias, c"new".as_ptr()) }, 0);
        let frozen_source = parse(r#"{"own":4,"new":5}"#);
        unsafe { thaw_json_object_assign(frozen_alias, frozen_source) };
        assert_eq!(thaw_json_take_assign_error(), 1);
        assert_number(frozen, c"own".as_ptr(), 1.0);
        assert_eq!(unsafe { thaw_json_has_own(frozen, c"new".as_ptr()) }, 0);

        unsafe {
            thaw_json_destroy(extensible_source);
            thaw_json_destroy(extensible_alias);
            thaw_json_destroy(extensible);
            thaw_json_destroy(prevented_source);
            thaw_json_destroy(prevented_alias);
            thaw_json_destroy(prevented);
            thaw_json_destroy(sealed_source);
            thaw_json_destroy(sealed_alias);
            thaw_json_destroy(sealed);
            thaw_json_destroy(frozen_source);
            thaw_json_destroy(frozen_alias);
            thaw_json_destroy(frozen);
        }
    }

    #[test]
    fn dependency_compile_as_string_variants() {
        let assert_string_after_destroy = |value: *mut Value, expected: &[u8]| {
            let output = thaw_json_as_string(value);
            assert!(!output.is_null());
            unsafe { thaw_json_destroy(value) };
            assert_eq!(unsafe { thaw_arena::NativeStr::from_ptr(output) }.to_bytes(), expected);
            unsafe { thaw_cstring_destroy(output.cast_mut()) };
        };

        assert_string_after_destroy(leak(Value::Bool(true)), b"true");
        assert_string_after_destroy(leak(Value::Bool(false)), b"false");

        for (number, source, expected) in [
            (serde_json::Number::from(-7_i64), None, "-7"),
            (serde_json::Number::from(42_u64), None, "42"),
            (
                serde_json::Number::from(9_007_199_254_740_993_u64),
                Some("9007199254740993"),
                "9007199254740992",
            ),
            (
                serde_json::Number::from(i64::MAX),
                Some("9223372036854775807"),
                "9223372036854776000",
            ),
            (
                serde_json::Number::from(u64::MAX),
                Some("18446744073709551615"),
                "18446744073709552000",
            ),
        ] {
            let constructed = leak(Value::Number(number));
            if let Some(source) = source {
                let parsed = parse(source);
                assert_eq!(
                    thaw_json_as_number(constructed).to_bits(),
                    thaw_json_as_number(parsed).to_bits(),
                );
                assert_string_after_destroy(parsed, expected.as_bytes());
            }
            assert_string_after_destroy(constructed, expected.as_bytes());
        }

        for (number, expected) in [
            (1.0, "1"),
            (-0.0, "0"),
            (1.5, "1.5"),
            (1e20, "100000000000000000000"),
            (1e21, "1e+21"),
            (1e-6, "0.000001"),
            (1e-7, "1e-7"),
        ] {
            let value = leak(number_value(number));
            assert_eq!(thaw_json_as_number(value).to_bits(), number.to_bits());
            assert_string_after_destroy(value, expected.as_bytes());
        }

        for (number, expected) in [
            (f64::NAN, "NaN"),
            (f64::INFINITY, "Infinity"),
            (f64::NEG_INFINITY, "-Infinity"),
        ] {
            assert_string_after_destroy(leak(number_value(number)), expected.as_bytes());
        }

        assert_string_after_destroy(parse(r#""text""#), b"text");
        assert_string_after_destroy(
            leak(Value::BigInt("-18446744073709551617".to_string())),
            b"-18446744073709551617",
        );
        assert_string_after_destroy(thaw_json_null(), b"null");
        assert_string_after_destroy(
            parse(r#"{"$__thaw_napi_undefined$":true}"#),
            b"undefined",
        );
        assert_string_after_destroy(parse(r#"[1,null,"x"]"#), b"1,,x");
        assert_string_after_destroy(parse("{}"), b"[object Object]");

        let wtf8 = wtf8_encode_utf16(&[0xD800, 0]);
        assert_string_after_destroy(leak(Value::Wtf8(wtf8.clone())), &wtf8);
    }
}
