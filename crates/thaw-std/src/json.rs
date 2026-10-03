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

struct SharedValue<T> {
    data: UnsafeCell<T>,
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
        unsafe { thaw_object_clear_state(key as *const u8) };
    }
}

unsafe extern "C" {
    fn thaw_object_clear_state(object: *const u8);
    fn thaw_object_state(object: *const u8, query: u8) -> bool;
}

/// Thaw's own dynamic (`Json`/`any`-typed) value representation --
/// deliberately *not* `serde_json::Value` (an earlier version of this
/// file used that directly, see git history): a plain owned tree gives
/// every nested read a fresh deep copy (`Value::clone`), which silently
/// diverges from real JS reference semantics the moment a nested object/
/// array is read into a local and mutated (`const inner = obj.c; inner.d
/// = false;` never reached back into `obj.c.d`, a long-standing tracked
/// gap). Only `Array`/`Object` carry a `Rc<UnsafeCell<...>>` -- a
/// scalar (`Null`/`Bool`/`Number`/`String`) stays a plain value (real JS
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
    String(String),
    /// A string containing at least one lone UTF-16 surrogate, held as
    /// WTF-8 bytes (a Rust `String` cannot represent it). Only ever
    /// produced from a native string crossing into JSON; serde_json is
    /// bypassed when serializing it (see `write_json_value`).
    Wtf8(Vec<u8>),
    Array(SharedArray),
    Object(SharedObject),
}

impl Value {
    fn shared_array(items: Vec<Value>) -> Value {
        Value::Array(Rc::new(SharedValue { data: UnsafeCell::new(items) }))
    }

    fn shared_object(fields: indexmap::IndexMap<Vec<u8>, Value>) -> Value {
        Value::Object(Rc::new(SharedValue { data: UnsafeCell::new(fields) }))
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
    fn deep_clone(&self) -> Value {
        match self {
            Value::Null => Value::Null,
            Value::Bool(value) => Value::Bool(*value),
            Value::Number(value) => Value::Number(value.clone()),
            Value::String(value) => Value::String(value.clone()),
            Value::Wtf8(bytes) => Value::Wtf8(bytes.clone()),
            Value::Array(items) => {
                let copy = Value::shared_array(
                    unsafe { &*items.get() }.iter().map(Value::deep_clone).collect(),
                );
                if let Value::Array(target) = &copy {
                    copy_array_holes(items, target, 0);
                }
                copy
            }
            Value::Object(fields) => Value::shared_object(
                unsafe { &*fields.get() }
                    .iter()
                    .map(|(key, value)| (key.clone(), value.deep_clone()))
                    .collect(),
            ),
        }
    }
}

/// Display uses the same serializer as `JSON.stringify`, including UTF-16 keys.
impl std::fmt::Display for Value {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut bytes = Vec::new();
        write_json_value(self, &mut bytes, None, 0);
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
}

fn object_writable(object: *const Value, key: &[u8]) -> bool {
    let state_key = unsafe { thaw_json_state_key(object) };
    if unsafe { thaw_object_state(state_key, 2) } {
        return false;
    }
    unsafe { thaw_object_state(state_key, 0) }
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

/// Shared identity for Object.freeze/seal/preventExtensions. Drop removes
/// runtime state before the Rc allocation can be reused.
#[no_mangle]
pub unsafe extern "C" fn thaw_json_state_key(value: *const Value) -> *const u8 {
    let Some(value) = (unsafe { value.as_ref() }) else {
        return std::ptr::null();
    };
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
        Some(Value::Null | Value::Array(_)) => true,
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
    Value::shared_object(fields)
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
    match value {
        Value::Object(fields) if is_thaw_internal_wrapper(shared_object_ref(fields)) => {
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
        Value::Object(fields) if is_thaw_internal_wrapper(shared_object_ref(fields)) => {
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

/// # Safety
///
/// `value` must be null or a pointer returned by this module's JSON constructors,
/// and it must not be used or destroyed again after this call.
#[no_mangle]
pub unsafe extern "C" fn thaw_json_destroy(value: *mut Value) {
    if !value.is_null() {
        drop(unsafe { Box::from_raw(value) });
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
    unsafe { value.as_ref() }
        .and_then(Value::as_object)
        .and_then(|fields| fields.get(b"__thaw_js_handle_id__".as_slice()))
        .and_then(Value::as_f64)
        .map(|value| value as u64)
        .unwrap_or(0)
}

#[no_mangle]
pub extern "C" fn thaw_json_stringify(value: *mut Value) -> *const c_char {
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

#[no_mangle]
pub extern "C" fn thaw_json_console_string(value: *mut Value) -> *const c_char {
    let value = unsafe { &*value };
    let text = match value {
        Value::String(value) => value.clone(),
        // Matches real Node's `console.log(undefined)` -- without this,
        // a value read back from a missing `Json` key/index (or a real
        // napi-undefined-marshaled one) would print its raw sentinel
        // JSON shape (`{"$__thaw_napi_undefined$":true}`) instead.
        other if is_napi_undefined(other) => "undefined".to_string(),
        // Same story for `NaN`/`Infinity`/`-Infinity` -- without this,
        // `console.log(0 / 0)` would print the `$__thaw_non_finite$`
        // sentinel's raw JSON shape instead of `NaN`.
        other if non_finite_number(other).is_some() => {
            non_finite_display(non_finite_number(other).unwrap()).to_string()
        }
        // A `RegExp` stored in `any` -- without this, `console.log(re)`
        // would print the raw `{"__thaw_regexp__":{...}}` wrapper
        // instead of real Node's `/source/flags` rendering. `Map`/`Set`
        // wrapped the same way still fall through to the generic branch
        // below -- matching their *own* `Map(n) { ... }`/`Set(n) { ... }`
        // `util.inspect` rendering exactly would need real inspect-style
        // formatting this compiler's `console.log` doesn't have for any
        // object (a plain `any`-typed object already prints JSON-style,
        // `{"a":1}`, not Node's `{ a: 1 }` -- a separate, pre-existing,
        // much larger gap), so it's left as the same accepted limitation.
        other if regexp_wrapper_property(other, "source").is_some() => format!(
            "/{}/{}",
            regexp_wrapper_property(other, "source")
                .and_then(|value| value.as_str().map(str::to_string))
                .unwrap_or_default(),
            regexp_wrapper_property(other, "flags")
                .and_then(|value| value.as_str().map(str::to_string))
                .unwrap_or_default(),
        ),
        // A `Date` stored in `any` -- without this, `console.log(d)`
        // would print the raw `{"timestamp":N}` wire shape
        // (`thaw_json_is_date_shape`, `platform_globals/dates.js`)
        // instead of real Node's bare (unquoted) ISO string, or, for an
        // invalid Date, real `String(date)`'s `"Invalid Date"`.
        other if date_iso_string(other).is_some() => date_iso_string(other)
            .unwrap()
            .unwrap_or_else(|| "Invalid Date".to_string()),
        other => inspect_json_value(other),
    };
    thaw_arena::owned_string(text)
}

/// Real Node's `console.log`/`util.inspect` rendering of a `Json` value,
/// used recursively for anything nested inside an object/array (an
/// object/array itself is never the bare top-level case `thaw_json_
/// console_string` already special-cases, so this never needs the
/// "leave a bare top-level string unquoted" rule -- every string this
/// function itself prints is a *nested* one, always quoted). Unlike
/// `JSON.stringify`, real `util.inspect` prints `undefined` as the bare
/// word `undefined` wherever it appears (an object field, an array
/// slot) rather than omitting/nulling it -- this checks the napi-
/// undefined sentinel itself, never delegating to `ordered_json_
/// omitting_undefined`'s own (JSON.stringify-shaped) omission rule.
fn inspect_json_value(value: &Value) -> String {
    if is_napi_undefined(value) {
        return "undefined".to_string();
    }
    if let Some(number) = non_finite_number(value) {
        return non_finite_display(number).to_string();
    }
    if regexp_wrapper_property(value, "source").is_some() {
        return format!(
            "/{}/{}",
            regexp_wrapper_property(value, "source")
                .and_then(|value| value.as_str().map(str::to_string))
                .unwrap_or_default(),
            regexp_wrapper_property(value, "flags")
                .and_then(|value| value.as_str().map(str::to_string))
                .unwrap_or_default(),
        );
    }
    if let Some(iso) = date_iso_string(value) {
        return iso.unwrap_or_else(|| "Invalid Date".to_string());
    }
    if let Value::Object(fields) = value {
        let fields = shared_object_ref(fields);
        if let Some(entries) = fields.get(b"__thaw_map_entries__".as_slice()).and_then(Value::as_array) {
            let items = entries
                .iter()
                .filter_map(|entry| {
                    let [key, value] = entry.as_array()?.as_slice() else {
                        return None;
                    };
                    Some(format!(
                        "{} => {}",
                        inspect_json_value(key),
                        inspect_json_value(value)
                    ))
                })
                .collect::<Vec<_>>();
            return if items.is_empty() {
                format!("Map({}) {{}}", entries.len())
            } else {
                format!("Map({}) {{ {} }}", entries.len(), items.join(", "))
            };
        }
        if let Some(values) = fields.get(b"__thaw_set_values__".as_slice()).and_then(Value::as_array) {
            let items = values.iter().map(inspect_json_value).collect::<Vec<_>>();
            return if items.is_empty() {
                format!("Set({}) {{}}", values.len())
            } else {
                format!("Set({}) {{ {} }}", values.len(), items.join(", "))
            };
        }
    }
    match value {
        Value::Null => "null".to_string(),
        Value::Bool(value) => value.to_string(),
        Value::Number(value) => value.to_string(),
        Value::String(value) => inspect_string_literal(value),
        Value::Wtf8(bytes) => inspect_string_literal(&wtf8_to_string(bytes)),
        Value::Array(items) => {
            let items = shared_array_ref(items);
            if items.is_empty() {
                "[]".to_string()
            } else {
                format!(
                    "[ {} ]",
                    items
                        .iter()
                        .map(inspect_json_value)
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            }
        }
        Value::Object(fields) => {
            let fields = shared_object_ref(fields);
            if fields.is_empty() {
                "{}".to_string()
            } else {
                let items = ordered_object_fields(fields)
                    .into_iter()
                    .map(|(key, value)| {
                        let key = match std::str::from_utf8(key) {
                            Ok(text) if is_valid_identifier_key(text) => text.to_string(),
                            Ok(text) => inspect_string_literal(text),
                            Err(_) => {
                                let mut quoted = Vec::new();
                                write_json_string(key, &mut quoted);
                                String::from_utf8(quoted).expect("JSON string is UTF-8")
                            }
                        };
                        format!("{key}: {}", inspect_json_value(value))
                    })
                    .collect::<Vec<_>>();
                format!("{{ {} }}", items.join(", "))
            }
        }
    }
}

/// A nested string's own `util.inspect` quoting: single-quoted by
/// default, switching to double quotes when the string itself contains
/// a `'` but no `"`, and to backticks (a template literal) when it
/// contains both -- real Node's own precedence, confirmed directly
/// (`{ s: "it's a test" }` / `{ s: 'has "double" quotes' }` /
/// `` { s: `has both 'single' and "double"` } ``).
fn inspect_string_literal(value: &str) -> String {
    let quote = if value.contains('\'') && !value.contains('"') {
        '"'
    } else if value.contains('\'') && value.contains('"') {
        '`'
    } else {
        '\''
    };
    let mut text = String::with_capacity(value.len() + 2);
    text.push(quote);
    for ch in value.chars() {
        match ch {
            '\\' => text.push_str("\\\\"),
            '\n' => text.push_str("\\n"),
            '\r' => text.push_str("\\r"),
            '\t' => text.push_str("\\t"),
            ch if ch == quote => {
                text.push('\\');
                text.push(ch);
            }
            ch => text.push(ch),
        }
    }
    text.push(quote);
    text
}

/// Whether an object key can print bare (`{ key: ... }`) or needs
/// quoting (`{ 'weird-key': ... }`) -- real Node's own rule, an
/// identifier-shaped key prints unquoted. A practical ASCII subset
/// (real Node also allows Unicode identifier characters) covers every
/// real-world key this compiler's own object/dictionary literals can
/// produce.
fn is_valid_identifier_key(key: &str) -> bool {
    let mut chars = key.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    (first.is_ascii_alphabetic() || first == '_' || first == '$')
        && chars.all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '$')
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
    match value {
        Value::Null => out.extend_from_slice(b"null"),
        Value::Bool(true) => out.extend_from_slice(b"true"),
        Value::Bool(false) => out.extend_from_slice(b"false"),
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
    let mut output = Vec::new();
    write_json_value(value, &mut output, indent, 0);
    CString::new(output).unwrap_or_default().into_raw()
}

fn stringify_with_indent(value: *mut Value, indent: &[u8]) -> *const c_char {
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
    if let Some(result) = regexp_wrapper_property(value, &wtf8_to_string(&key)) {
        return leak(result);
    }
    if key.as_slice() == b"size" {
        if let Some(result) = map_or_set_wrapper_size(value) {
            return leak(result);
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
    leak(result)
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
    leak(result)
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
    }
}

unsafe extern "C" {
    fn thaw_string_to_number(value: *const c_char) -> f64;
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

#[no_mangle]
pub extern "C" fn thaw_json_as_string(value: *mut Value) -> *const c_char {
    let value = unsafe { &*value };
    // A lone-surrogate string keeps its raw WTF-8 bytes (a `String`
    // couldn't hold them).
    if let Value::Wtf8(bytes) = value {
        return thaw_arena::owned_string(bytes);
    }
    let text = match value {
        Value::String(s) => s.clone(),
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
        Value::Bool(_) => c"boolean".as_ptr(),
        Value::Number(_) => c"number".as_ptr(),
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
        Value::String(value) => u8::from(!value.is_empty()),
        Value::Wtf8(bytes) => u8::from(!bytes.is_empty()),
        Value::Array(_) | Value::Object(_) => 1,
    }
}

fn is_napi_undefined(value: &Value) -> bool {
    matches!(
        value,
        Value::Object(object)
            if matches!(
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
fn is_thaw_internal_wrapper(fields: &indexmap::IndexMap<Vec<u8>, Value>) -> bool {
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
    let Value::Object(fields) = value else {
        return 0;
    };
    let key = to_key(key);
    let fields = shared_object_ref(fields);
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
    let Value::Object(object) = value else {
        return None;
    };
    match shared_object_ref(object)
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
    let Value::Object(fields) = value else {
        return 0;
    };
    let fields = shared_object_ref(fields);
    u8::from(
        fields.get(b"type".as_slice()).and_then(Value::as_str) == Some("Buffer")
            && fields.get(b"data".as_slice()).is_some_and(Value::is_array),
    )
}

/// `Date.prototype.toJSON` is overridden globally (`platform_globals/
/// dates.js`) to `{ timestamp: this.getTime() }`, so a `Date` returned
/// from a Fallback call already survives the QuickJS boundary as this
/// exact, structurally-recognizable shape -- an `Object` with exactly
/// one key, `"timestamp"`, holding a `Number`. Used by `instanceof
/// Date`/Date-prototype-method dispatch on a `Json`-typed value (see
/// `crates/thaw-hir/src/lower/expressions/lowering.rs` and `crates/
/// thaw-hir/src/lower/invocations/calls.rs`).
#[no_mangle]
pub extern "C" fn thaw_json_is_date_shape(value: *const Value) -> u8 {
    let Some(value) = (unsafe { value.as_ref() }) else {
        return 0;
    };
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
fn napi_undefined_value() -> Value {
    let mut fields = indexmap::IndexMap::new();
    fields.insert(b"$__thaw_napi_undefined$".to_vec(), Value::Bool(true));
    Value::shared_object(fields)
}

#[no_mangle]
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
    (!value.is_null() && matches!(unsafe { &*value }, Value::Array(_))).into()
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
    (!value.is_null() && unsafe { &*value }.is_null()).into()
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
    (matches!(value, Value::Null) || is_napi_undefined(value)).into()
}

#[no_mangle]
/// # Safety
/// `value` must be null or point to a valid JSON `Value`.
pub unsafe extern "C" fn thaw_json_is_object_like(value: *const Value) -> u8 {
    match unsafe { value.as_ref() } {
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
    let keys = unsafe { value.as_ref() }.map(enumerable_entries).unwrap_or_default()
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
    let Some(Value::Array(items)) = (unsafe { value.as_ref() }) else {
        return unsafe { thaw_json_keys(value) };
    };
    alloc_pointer_array(
        (0..shared_array_ref(items).len()).filter(|index| array_has_index(items, *index))
            .map(|index| index.to_string())
            .chain(std::iter::once("length".to_string()))
            .map(|key| thaw_arena::owned_string(key).cast())
            .collect(),
    )
}

#[no_mangle]
/// # Safety
///
/// `value` must be null or point to a valid JSON `Value`.
pub unsafe extern "C" fn thaw_json_values(value: *const Value) -> *mut u8 {
    let values = unsafe { value.as_ref() }.map(enumerable_entries).unwrap_or_default()
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
        (slot as *mut f64).write(non_finite_number(value).or_else(|| value.as_f64()).unwrap_or(0.0));
    })
}

fn native_string_value(value: &Value) -> *mut c_char {
    match value {
        Value::String(text) => thaw_arena::owned_string(text),
        Value::Wtf8(bytes) => thaw_arena::owned_string(bytes),
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
        slot.write(value.as_bool().unwrap_or(false).into());
    })
}

#[no_mangle]
/// # Safety
///
/// `value` must be null or point to a valid JSON `Value`.
pub unsafe extern "C" fn thaw_json_entries(value: *const Value) -> *mut u8 {
    let entries = unsafe { value.as_ref() }.map(enumerable_entries).unwrap_or_default();
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
        (Some(Value::Number(left)), Some(Value::Number(right))) => left
            .as_f64()
            .zip(right.as_f64())
            .is_some_and(|(left, right)| json_number_is(left, right)),
        (Some(Value::Array(left)), Some(Value::Array(right))) => Rc::ptr_eq(left, right),
        (Some(Value::Object(left)), Some(Value::Object(right))) => Rc::ptr_eq(left, right),
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
        (Value::Number(x), Value::Number(y)) => x.as_f64() == y.as_f64(),
        (Value::String(x), Value::String(y)) => x == y,
        (Value::Wtf8(_), Value::Wtf8(_))
        | (Value::String(_), Value::Wtf8(_))
        | (Value::Wtf8(_), Value::String(_)) => same_string(a_val, b_val).unwrap_or(false),
        (Value::Array(x), Value::Array(y)) => Rc::ptr_eq(x, y),
        (Value::Object(x), Value::Object(y)) => Rc::ptr_eq(x, y),
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
        (Value::Number(x), Value::Number(y)) => x.as_f64() == y.as_f64(),
        (Value::String(x), Value::String(y)) => x == y,
        (Value::Wtf8(_), Value::Wtf8(_))
        | (Value::String(_), Value::Wtf8(_))
        | (Value::Wtf8(_), Value::String(_)) => same_string(a_val, b_val).unwrap_or(false),
        (Value::Array(x), Value::Array(y)) => Rc::ptr_eq(x, y),
        (Value::Object(x), Value::Object(y)) => Rc::ptr_eq(x, y),
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
    value
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
    let Some(fields) = value.as_object() else {
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
    let Some(fields) = value.as_object() else {
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
    let Some(fields) = value.as_object() else {
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
    let Some(fields) = value.as_object() else {
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
    let Some(fields) = value.as_object() else {
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

#[no_mangle]
pub extern "C" fn thaw_json_array_slice(value: *mut Value, start: i64) -> *mut Value {
    let copy = Value::shared_array(
        unsafe { value.as_ref() }
            .and_then(Value::as_array)
            .map(|values| values.iter().skip(start.max(0) as usize).cloned().collect())
            .unwrap_or_default(),
    );
    if let (Some(Value::Array(source)), Value::Array(target)) = (unsafe { value.as_ref() }, &copy) {
        copy_array_holes(source, target, start.max(0) as usize);
    }
    leak(copy)
}

#[no_mangle]
pub extern "C" fn thaw_json_object_new() -> *mut Value {
    leak(Value::shared_object(indexmap::IndexMap::new()))
}

fn object_insert(object: *mut Value, key: *const c_char, value: Value) {
    let key = to_key(key);
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
    object_insert(object, key, *unsafe { Box::from_raw(value) });
}

#[no_mangle]
pub extern "C" fn thaw_json_object_delete(object: *mut Value, key: *const c_char) -> u8 {
    let Some(value) = (unsafe { object.as_ref() }) else {
        return 0;
    };
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
}
