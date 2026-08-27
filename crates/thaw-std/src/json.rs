//! `JSON.parse`/`JSON.stringify`/`.field`/`[i]`/`Number`/`String`/`Boolean`
//! on the `HirType::Json` dynamic value (see `thaw_hir::HirExpr::JsonGet`
//! et al. and `thaw-llvm::hir_codegen`).
//!
//! A `Json` value at the LLVM level is an opaque pointer to a
//! `Box<serde_json::Value>`, leaked on creation -- consistent with every
//! other heap value in Thaw today (strings, arrays, objects): nothing is
//! freed yet, since the arena-reset lifecycle isn't wired to a request
//! boundary until Phase 2's Lambda loop actually needs it to be.
//!
//! There is no exception channel wired to any of this yet (`throw` only
//! unwinds within a single HIR function -- see `HirStmt::Try`), so every
//! failure mode here (parse errors, missing fields, wrong-type access)
//! degrades to a default value (`Value::Null`, `0.0`, `""`, `false`)
//! instead of aborting or panicking across the FFI boundary.

use std::ffi::{CStr, CString};
use std::os::raw::c_char;

use serde::Serialize;
use serde_json::Value;

fn to_str(ptr: *const c_char) -> String {
    unsafe { CStr::from_ptr(ptr) }
        .to_string_lossy()
        .into_owned()
}

fn leak(value: Value) -> *mut Value {
    Box::into_raw(Box::new(value))
}

fn number_value(value: f64) -> Value {
    if value.is_finite() && value.fract() == 0.0 {
        if value >= i64::MIN as f64 && value <= i64::MAX as f64 {
            return Value::Number((value as i64).into());
        }
        if value >= 0.0 && value <= u64::MAX as f64 {
            return Value::Number((value as u64).into());
        }
    }
    serde_json::Number::from_f64(value).map_or(Value::Null, Value::Number)
}

fn array_index_key(key: &str) -> Option<u32> {
    let index = key.parse::<u32>().ok()?;
    (index != u32::MAX && index.to_string() == key).then_some(index)
}

fn ordered_object_fields(fields: &serde_json::Map<String, Value>) -> Vec<(&String, &Value)> {
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

fn ordered_json(value: &Value) -> Value {
    match value {
        Value::Object(fields) => Value::Object(
            ordered_object_fields(fields)
                .into_iter()
                .map(|(key, value)| (key.clone(), ordered_json(value)))
                .collect(),
        ),
        Value::Array(items) => Value::Array(items.iter().map(ordered_json).collect()),
        other => other.clone(),
    }
}

#[no_mangle]
pub extern "C" fn thaw_json_parse(text: *const c_char) -> *mut Value {
    let text = to_str(text);
    leak(serde_json::from_str(&text).unwrap_or(Value::Null))
}

#[no_mangle]
pub extern "C" fn thaw_json_stringify(value: *mut Value) -> *const c_char {
    let value = unsafe { &*value };
    stringify_value(&ordered_json(value), &[])
}

fn stringify_value(value: &Value, indent: &[u8]) -> *const c_char {
    if indent.is_empty() {
        let text = serde_json::to_string(value).unwrap_or_else(|_| "null".to_string());
        return CString::new(text).unwrap_or_default().into_raw();
    }
    let mut output = Vec::new();
    let formatter = serde_json::ser::PrettyFormatter::with_indent(indent);
    let mut serializer = serde_json::Serializer::with_formatter(&mut output, formatter);
    let text = if value.serialize(&mut serializer).is_ok() {
        String::from_utf8(output).unwrap_or_else(|_| "null".to_string())
    } else {
        "null".to_string()
    };
    CString::new(text).unwrap_or_default().into_raw()
}

fn stringify_with_indent(value: *mut Value, indent: &[u8]) -> *const c_char {
    let value = unsafe { &*value };
    stringify_value(&ordered_json(value), indent)
}

fn string_array(array: *const u8) -> Vec<String> {
    if array.is_null() {
        return Vec::new();
    }
    let length = unsafe { (array as *const i64).read() }.max(0) as usize;
    let mut keys = Vec::new();
    for index in 0..length {
        let key = unsafe { (array.add(8 + index * 8) as *const *const c_char).read() };
        let key = to_str(key);
        if !keys.contains(&key) {
            keys.push(key);
        }
    }
    keys
}

fn filtered_json(value: &Value, keys: &[String]) -> Value {
    match value {
        Value::Object(fields) => Value::Object(
            keys.iter()
                .filter_map(|key| {
                    fields
                        .get(key)
                        .map(|value| (key.clone(), filtered_json(value, keys)))
                })
                .collect(),
        ),
        Value::Array(items) => Value::Array(
            items
                .iter()
                .map(|value| filtered_json(value, keys))
                .collect(),
        ),
        other => other.clone(),
    }
}

fn stringify_with_keys(value: *mut Value, keys: *const u8, indent: &[u8]) -> *const c_char {
    let value = unsafe { &*value };
    let keys = string_array(keys);
    stringify_value(&filtered_json(value, &keys), indent)
}

#[no_mangle]
pub extern "C" fn thaw_json_stringify_number_space(value: *mut Value, space: f64) -> *const c_char {
    let width = if space.is_finite() {
        space.trunc().clamp(0.0, 10.0) as usize
    } else {
        0
    };
    stringify_with_indent(value, &vec![b' '; width])
}

#[no_mangle]
pub extern "C" fn thaw_json_stringify_string_space(
    value: *mut Value,
    space: *const c_char,
) -> *const c_char {
    let indent = to_str(space).chars().take(10).collect::<String>();
    stringify_with_indent(value, indent.as_bytes())
}

#[no_mangle]
pub extern "C" fn thaw_json_stringify_keys(value: *mut Value, keys: *const u8) -> *const c_char {
    stringify_with_keys(value, keys, &[])
}

#[no_mangle]
pub extern "C" fn thaw_json_stringify_keys_number_space(
    value: *mut Value,
    keys: *const u8,
    space: f64,
) -> *const c_char {
    let width = if space.is_finite() {
        space.trunc().clamp(0.0, 10.0) as usize
    } else {
        0
    };
    stringify_with_keys(value, keys, &vec![b' '; width])
}

#[no_mangle]
pub extern "C" fn thaw_json_stringify_keys_string_space(
    value: *mut Value,
    keys: *const u8,
    space: *const c_char,
) -> *const c_char {
    let indent = to_str(space).chars().take(10).collect::<String>();
    stringify_with_keys(value, keys, indent.as_bytes())
}

#[no_mangle]
pub extern "C" fn thaw_json_get(value: *mut Value, key: *const c_char) -> *mut Value {
    let value = unsafe { &*value };
    let key = to_str(key);
    // `.length` on a JSON array is a built-in, not a data field -- handled
    // here rather than via a dedicated HIR node/lowering rule, since a
    // dynamically-typed `Json` value's runtime kind (array vs. object)
    // isn't known until now. `serde_json::Value::get` only matches string
    // keys against objects, so without this an array's `.length` would
    // silently resolve to `Null` (-> `0`) instead of the actual length.
    let result = match value {
        Value::Array(items) if key == "length" => Value::Number((items.len() as u64).into()),
        _ => value.get(&key).cloned().unwrap_or(Value::Null),
    };
    leak(result)
}

#[no_mangle]
pub extern "C" fn thaw_json_index(value: *mut Value, index: f64, key: *const c_char) -> *mut Value {
    let value = unsafe { &*value };
    let result = match value {
        Value::Array(items)
            if index.is_finite()
                && index >= 0.0
                && index <= (u32::MAX - 1) as f64
                && index.fract() == 0.0 =>
        {
            items.get(index as usize).cloned()
        }
        Value::Object(fields) => fields.get(&to_str(key)).cloned(),
        _ => None,
    }
    .unwrap_or(Value::Null);
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
    if let (Some(container), Some(value)) = (unsafe { array.as_mut() }, unsafe { value.as_ref() }) {
        match container {
            Value::Array(items)
                if index.is_finite()
                    && index >= 0.0
                    && index <= (u32::MAX - 1) as f64
                    && index.fract() == 0.0 =>
            {
                let index = index as usize;
                if items.len() <= index {
                    items.resize(index + 1, Value::Null);
                }
                items[index] = value.clone();
            }
            Value::Object(fields) => {
                fields.insert(to_str(key), value.clone());
            }
            _ => {}
        }
    }
    value
}

#[no_mangle]
pub extern "C" fn thaw_json_as_number(value: *mut Value) -> f64 {
    let value = unsafe { &*value };
    value.as_f64().unwrap_or(0.0)
}

#[no_mangle]
pub extern "C" fn thaw_json_as_string(value: *mut Value) -> *const c_char {
    let value = unsafe { &*value };
    let text = match value {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        other => other.to_string(),
    };
    CString::new(text).unwrap_or_default().into_raw() as *const c_char
}

/// Returns `0` or `1` rather than a Rust `bool` -- deliberately avoids
/// relying on `bool`'s C ABI representation across the FFI boundary; the
/// LLVM caller compares this against zero itself (see `compile_expr`'s
/// `JsonAsBool` case in hir_codegen.rs).
#[no_mangle]
pub extern "C" fn thaw_json_as_bool(value: *mut Value) -> u8 {
    let value = unsafe { &*value };
    value.as_bool().unwrap_or(false) as u8
}

#[no_mangle]
pub extern "C" fn thaw_json_array_new() -> *mut Value {
    leak(Value::Array(Vec::new()))
}

#[no_mangle]
/// # Safety
///
/// `value` must be null or point to a valid JSON `Value`.
pub unsafe extern "C" fn thaw_json_is_array(value: *const Value) -> u8 {
    (!value.is_null() && matches!(unsafe { &*value }, Value::Array(_))).into()
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

#[no_mangle]
/// # Safety
///
/// `value` must be null or point to a valid JSON `Value`.
pub unsafe extern "C" fn thaw_json_keys(value: *const Value) -> *mut u8 {
    let keys = match unsafe { value.as_ref() } {
        Some(Value::Object(fields)) => ordered_object_fields(fields)
            .into_iter()
            .map(|(key, _)| key.clone())
            .collect::<Vec<_>>(),
        Some(Value::Array(items)) => (0..items.len()).map(|index| index.to_string()).collect(),
        _ => Vec::new(),
    };
    alloc_pointer_array(
        keys.into_iter()
            .map(|key| CString::new(key).unwrap_or_default().into_raw().cast())
            .collect(),
    )
}

#[no_mangle]
/// # Safety
///
/// `value` must be null or point to a valid JSON `Value`.
pub unsafe extern "C" fn thaw_json_values(value: *const Value) -> *mut u8 {
    let values = match unsafe { value.as_ref() } {
        Some(Value::Object(fields)) => ordered_object_fields(fields)
            .into_iter()
            .map(|(_, value)| value.clone())
            .collect::<Vec<_>>(),
        Some(Value::Array(items)) => items.clone(),
        _ => Vec::new(),
    };
    alloc_pointer_array(
        values
            .into_iter()
            .map(|value| Box::into_raw(Box::new(value)).cast())
            .collect(),
    )
}

fn object_values(value: *const Value) -> Vec<Value> {
    match unsafe { value.as_ref() } {
        Some(Value::Object(fields)) => ordered_object_fields(fields)
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
        (slot as *mut f64).write(value.as_f64().unwrap_or(0.0));
    })
}

#[no_mangle]
/// # Safety
///
/// `value` must be null or point to a valid JSON object containing strings.
pub unsafe extern "C" fn thaw_json_string_values(value: *const Value) -> *mut u8 {
    alloc_scalar_array(&object_values(value), 8, |slot, value| unsafe {
        (slot as *mut *mut u8).write(
            CString::new(value.as_str().unwrap_or_default())
                .unwrap_or_default()
                .into_raw()
                .cast(),
        );
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
    let entries = match unsafe { value.as_ref() } {
        Some(Value::Object(fields)) => ordered_object_fields(fields)
            .into_iter()
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect::<Vec<_>>(),
        Some(Value::Array(items)) => items
            .iter()
            .enumerate()
            .map(|(index, value)| (index.to_string(), value.clone()))
            .collect(),
        _ => Vec::new(),
    };
    alloc_pointer_array(
        entries
            .into_iter()
            .map(|(key, value)| {
                alloc_pointer_array(vec![
                    CString::new(key).unwrap_or_default().into_raw().cast(),
                    Box::into_raw(Box::new(value)).cast(),
                ])
            })
            .collect(),
    )
}

fn alloc_typed_entries(value: *const Value, write: impl Fn(*mut u8, &Value)) -> *mut u8 {
    let entries = match unsafe { value.as_ref() } {
        Some(Value::Object(fields)) => ordered_object_fields(fields),
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
                    (entry.add(8) as *mut *mut u8).write(
                        CString::new(key.as_str())
                            .unwrap_or_default()
                            .into_raw()
                            .cast(),
                    );
                }
                write(unsafe { entry.add(16) }, value);
                entry
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
        (slot as *mut f64).write(value.as_f64().unwrap_or(0.0));
    })
}

#[no_mangle]
/// # Safety
///
/// `value` must be null or point to a valid JSON object containing strings.
pub unsafe extern "C" fn thaw_json_string_entries(value: *const Value) -> *mut u8 {
    alloc_typed_entries(value, |slot, value| unsafe {
        (slot as *mut *mut u8).write(
            CString::new(value.as_str().unwrap_or_default())
                .unwrap_or_default()
                .into_raw()
                .cast(),
        );
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

fn object_from_typed_entries(entries: *const u8, read: impl Fn(*const u8) -> Value) -> *mut Value {
    let mut object = serde_json::Map::new();
    if entries.is_null() {
        return leak(Value::Object(object));
    }
    let length = unsafe { (entries as *const i64).read() }.max(0) as usize;
    for index in 0..length {
        let entry = unsafe { (entries.add(8 + index * 8) as *const *const u8).read() };
        if entry.is_null() {
            continue;
        }
        let key = unsafe { (entry.add(8) as *const *const c_char).read() };
        object.insert(to_str(key), read(unsafe { entry.add(16) }));
    }
    leak(Value::Object(object))
}

#[no_mangle]
/// # Safety
///
/// `entries` must be null or point to a native `[string, number][]` array.
pub unsafe extern "C" fn thaw_json_object_from_number_entries(entries: *const u8) -> *mut Value {
    object_from_typed_entries(entries, |slot| {
        let value = unsafe { (slot as *const f64).read() };
        number_value(value)
    })
}

#[no_mangle]
/// # Safety
///
/// `entries` must be null or point to a native `[string, string][]` array.
pub unsafe extern "C" fn thaw_json_object_from_string_entries(entries: *const u8) -> *mut Value {
    object_from_typed_entries(entries, |slot| {
        let value = unsafe { (slot as *const *const c_char).read() };
        Value::String(to_str(value))
    })
}

#[no_mangle]
/// # Safety
///
/// `entries` must be null or point to a native `[string, boolean][]` array.
pub unsafe extern "C" fn thaw_json_object_from_bool_entries(entries: *const u8) -> *mut Value {
    object_from_typed_entries(entries, |slot| Value::Bool(unsafe { slot.read() } != 0))
}

#[no_mangle]
/// # Safety
///
/// `entries` must be null or point to a native `[string, Json][]` array.
pub unsafe extern "C" fn thaw_json_object_from_json_entries(entries: *const u8) -> *mut Value {
    object_from_typed_entries(entries, |slot| {
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
    let key = to_str(key);
    match unsafe { value.as_ref() } {
        Some(Value::Object(fields)) => fields.contains_key(&key),
        Some(Value::Array(_)) if key == "length" => true,
        Some(Value::Array(items)) => key
            .parse::<usize>()
            .ok()
            .is_some_and(|index| index.to_string() == key && index < items.len()),
        _ => false,
    }
    .into()
}

fn json_number_is(left: f64, right: f64) -> bool {
    (left.is_nan() && right.is_nan()) || (left == right && left.to_bits() == right.to_bits())
}

#[no_mangle]
/// # Safety
///
/// Both operands must be null or point to valid JSON `Value`s.
pub unsafe extern "C" fn thaw_json_object_is(left: *const Value, right: *const Value) -> u8 {
    let result = match (unsafe { left.as_ref() }, unsafe { right.as_ref() }) {
        (Some(Value::Null), Some(Value::Null)) => true,
        (Some(Value::Bool(left)), Some(Value::Bool(right))) => left == right,
        (Some(Value::String(left)), Some(Value::String(right))) => left == right,
        (Some(Value::Number(left)), Some(Value::Number(right))) => left
            .as_f64()
            .zip(right.as_f64())
            .is_some_and(|(left, right)| json_number_is(left, right)),
        (Some(Value::Array(_)), Some(Value::Array(_)))
        | (Some(Value::Object(_)), Some(Value::Object(_))) => left == right,
        _ => false,
    };
    result.into()
}

#[no_mangle]
/// # Safety
///
/// `value` must be null or point to a valid JSON `Value`.
pub unsafe extern "C" fn thaw_json_object_is_number(value: *const Value, other: f64) -> u8 {
    unsafe { value.as_ref() }
        .and_then(Value::as_f64)
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
        .and_then(Value::as_str)
        .is_some_and(|value| value == to_str(other))
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
    if let Some(items) = (unsafe { array.as_mut() }).and_then(Value::as_array_mut) {
        items.push(number_value(value));
    }
}

#[no_mangle]
pub extern "C" fn thaw_json_array_push_string(array: *mut Value, value: *const c_char) {
    if let Some(items) = (unsafe { array.as_mut() }).and_then(Value::as_array_mut) {
        items.push(Value::String(to_str(value)));
    }
}

#[no_mangle]
pub extern "C" fn thaw_json_array_push_bool(array: *mut Value, value: u8) {
    if let Some(items) = (unsafe { array.as_mut() }).and_then(Value::as_array_mut) {
        items.push(Value::Bool(value != 0));
    }
}

#[no_mangle]
pub extern "C" fn thaw_json_array_push_json(array: *mut Value, value: *mut Value) {
    let Some(value) = (unsafe { value.as_ref() }).cloned() else {
        return;
    };
    if let Some(items) = (unsafe { array.as_mut() }).and_then(Value::as_array_mut) {
        items.push(value);
    }
}

#[no_mangle]
pub extern "C" fn thaw_json_null() -> *mut Value {
    leak(Value::Null)
}

#[no_mangle]
pub extern "C" fn thaw_json_from_number_array(array: *const u8) -> *mut Value {
    if array.is_null() {
        return leak(Value::Array(Vec::new()));
    }
    let length = unsafe { (array as *const i64).read() }.max(0) as usize;
    let values = (0..length)
        .map(|index| {
            let value = unsafe { (array.add(8 + index * 8) as *const f64).read() };
            number_value(value)
        })
        .collect();
    leak(Value::Array(values))
}

#[no_mangle]
pub extern "C" fn thaw_json_to_number_array(value: *mut Value) -> *mut u8 {
    let values = unsafe { value.as_ref() }
        .and_then(Value::as_array)
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
pub extern "C" fn thaw_json_to_string_array(value: *mut Value) -> *mut u8 {
    let values = unsafe { value.as_ref() }
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let output = thaw_arena::thaw_arena_alloc(8 + values.len() * 8, 8);
    if output.is_null() {
        return output;
    }
    unsafe { (output as *mut i64).write(values.len() as i64) };
    for (index, value) in values.iter().enumerate() {
        let text = match value {
            Value::String(text) => text.clone(),
            Value::Null => String::new(),
            other => other.to_string(),
        };
        let text = CString::new(text).unwrap_or_default().into_raw();
        unsafe { (output.add(8 + index * 8) as *mut *mut c_char).write(text) };
    }
    output
}

#[no_mangle]
pub extern "C" fn thaw_json_to_bool_array(value: *mut Value) -> *mut u8 {
    let values = unsafe { value.as_ref() }
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let output = thaw_arena::thaw_arena_alloc(8 + values.len() * 8, 8);
    if output.is_null() {
        return output;
    }
    unsafe { (output as *mut i64).write(values.len() as i64) };
    for (index, value) in values.iter().enumerate() {
        unsafe {
            output
                .add(8 + index * 8)
                .write(value.as_bool().unwrap_or(false) as u8)
        };
    }
    output
}

#[no_mangle]
pub extern "C" fn thaw_json_object_new() -> *mut Value {
    leak(Value::Object(serde_json::Map::new()))
}

fn object_insert(object: *mut Value, key: *const c_char, value: Value) {
    if let Some(fields) = (unsafe { object.as_mut() }).and_then(Value::as_object_mut) {
        fields.insert(to_str(key), value);
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
    object_insert(object, key, Value::String(to_str(value)));
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

#[no_mangle]
pub extern "C" fn thaw_json_object_delete(object: *mut Value, key: *const c_char) -> u8 {
    if let Some(fields) = (unsafe { object.as_mut() }).and_then(Value::as_object_mut) {
        fields.remove(&to_str(key));
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
    let Some(target_fields) = (unsafe { target.as_mut() }).and_then(Value::as_object_mut) else {
        return target;
    };
    if let Some(source_fields) = (unsafe { source.as_ref() }).and_then(Value::as_object) {
        for (key, value) in source_fields {
            target_fields.insert(key.clone(), value.clone());
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
        let first_entry = unsafe { (entries.add(8) as *const *mut u8).read() };
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
        assert_eq!(thaw_json_as_number(missing), 0.0);
        assert_eq!(read_c_string(thaw_json_as_string(missing)), "");

        let arr = parse("[1, 2]");
        let oob = thaw_json_index(arr, 99.0, std::ptr::null());
        assert_eq!(thaw_json_as_number(oob), 0.0);
    }

    #[test]
    fn stringify_round_trips() {
        let value = parse(r#"{"a": 1, "b": [true, false]}"#);
        let text = read_c_string(thaw_json_stringify(value));
        let reparsed: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(reparsed, serde_json::json!({"a": 1, "b": [true, false]}));
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
        let decoded: Value =
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
}
