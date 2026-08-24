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

use serde_json::Value;

fn to_str(ptr: *const c_char) -> String {
    unsafe { CStr::from_ptr(ptr) }
        .to_string_lossy()
        .into_owned()
}

fn leak(value: Value) -> *mut Value {
    Box::into_raw(Box::new(value))
}

#[no_mangle]
pub extern "C" fn thaw_json_parse(text: *const c_char) -> *mut Value {
    let text = to_str(text);
    leak(serde_json::from_str(&text).unwrap_or(Value::Null))
}

#[no_mangle]
pub extern "C" fn thaw_json_stringify(value: *mut Value) -> *const c_char {
    let value = unsafe { &*value };
    let text = serde_json::to_string(value).unwrap_or_else(|_| "null".to_string());
    CString::new(text).unwrap_or_default().into_raw() as *const c_char
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
pub extern "C" fn thaw_json_index(value: *mut Value, index: i64) -> *mut Value {
    let value = unsafe { &*value };
    let result = usize::try_from(index)
        .ok()
        .and_then(|i| value.get(i))
        .cloned()
        .unwrap_or(Value::Null);
    leak(result)
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

#[no_mangle]
pub extern "C" fn thaw_json_array_push_number(array: *mut Value, value: f64) {
    if let Some(items) = (unsafe { array.as_mut() }).and_then(Value::as_array_mut) {
        items.push(serde_json::Number::from_f64(value).map_or(Value::Null, Value::Number));
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
pub extern "C" fn thaw_json_from_number_array(array: *const u8) -> *mut Value {
    if array.is_null() {
        return leak(Value::Array(Vec::new()));
    }
    let length = unsafe { (array as *const i64).read() }.max(0) as usize;
    let values = (0..length)
        .map(|index| {
            let value = unsafe { (array.add(8 + index * 8) as *const f64).read() };
            serde_json::Number::from_f64(value).map_or(Value::Null, Value::Number)
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
    object_insert(
        object,
        key,
        serde_json::Number::from_f64(value).map_or(Value::Null, Value::Number),
    );
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

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(s: &str) -> *mut Value {
        let c = CString::new(s).unwrap();
        thaw_json_parse(c.as_ptr())
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
    fn indexes_into_arrays() {
        let value = parse("[10, 20, 30]");
        assert_eq!(thaw_json_as_number(thaw_json_index(value, 0)), 10.0);
        assert_eq!(thaw_json_as_number(thaw_json_index(value, 2)), 30.0);
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
    fn missing_field_and_out_of_range_index_degrade_to_defaults_not_crashes() {
        let value = parse(r#"{"a": 1}"#);
        let missing_key = CString::new("nope").unwrap();
        let missing = thaw_json_get(value, missing_key.as_ptr());
        assert_eq!(thaw_json_as_number(missing), 0.0);
        assert_eq!(read_c_string(thaw_json_as_string(missing)), "");

        let arr = parse("[1, 2]");
        let oob = thaw_json_index(arr, 99);
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
            r#"[42.0,"thaw",true]"#
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
            serde_json::json!({"answer": 42.0, "arguments": [42.0, "thaw", true]})
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
        assert_eq!(
            read_c_string(thaw_json_stringify(round_trip)),
            "[1.5,2.0,0.0]"
        );
    }
}
