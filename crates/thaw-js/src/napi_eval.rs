//! Eval and reference calls of the N-API engine seam: `eval_json`, the calls through the
//! `__thaw_napi_reference_<id>` globals, the exception-capturing `this`-call and the graph pair check.
//! Each mirrors the same-named `thaw_js_*` function of thaw-quickjs.

use std::ffi::{c_char, c_void, CStr, CString};
use std::io::Read;
use std::ptr;

use base64::Engine;

use crate::host::{
    awaited, check_handle_callable, classify, cstr, decode_wire, default_realm, error_text,
    from_value, handle_result, on_handle, store_handle, thaw_tjs_call_graph_result, to_value,
    with_package, HandleResult, HostError, Native, ThawResult,
};
use crate::napi_props::ThawCallWithExceptionResult;
use crate::{Ctl, Interp, Value};

/// The JSON text an `undefined` result takes (the marker the N-API side decodes as `undefined`).
const UNDEFINED_MARKER: &str = r#"{"$__thaw_napi_undefined$":true}"#;

enum Outcome {
    Value(u64),
    Thrown(u64, bool),
}

/// The script text of a `gz:` payload (base64 of gzip). Anything else, or a payload that does not
/// decode, is used as it is, as `thaw_js_load` does.
pub(crate) fn unpack_script(source: String) -> String {
    let Some(encoded) = source.strip_prefix("gz:") else {
        return source;
    };
    base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .ok()
        .and_then(|compressed| {
            let mut decoded = String::new();
            flate2::read::GzDecoder::new(compressed.as_slice())
                .read_to_string(&mut decoded)
                .ok()
                .map(|_| decoded)
        })
        .unwrap_or(source)
}

/// `JSON.stringify(value)`: `None` when the value has no JSON form (`undefined`, functions).
fn json_stringify(it: &mut Interp, value: &Value) -> Result<Option<String>, HostError> {
    let global = Value::Object(it.global_object.clone());
    let json = it.get(&global, "JSON").map_err(|c| classify(it, c))?;
    let stringify = it.get(&json, "stringify").map_err(|c| classify(it, c))?;
    match it
        .call(&stringify, &json, std::slice::from_ref(value))
        .map_err(|c| classify(it, c))?
    {
        Value::Str(text) => Ok(Some(text.to_string())),
        _ => Ok(None),
    }
}

/// The JSON text of a call result. An `undefined` top level and an `undefined` own property both
/// keep the marker, so the N-API side can tell them from `null`.
fn result_json(it: &mut Interp, value: &Value) -> Result<String, HostError> {
    if matches!(value, Value::Undefined) {
        return Ok(UNDEFINED_MARKER.into());
    }
    Ok(from_value(it, value, 0)?.to_json_keeping_undefined())
}

/// The error text of a failed reference call: the `\u{2}` marker, then the error as a JSON string.
fn reference_error(text: &str) -> String {
    format!("\u{2}{}", serde_json::to_string(text).unwrap_or_default())
}

fn reference_name(context: *mut c_void) -> String {
    format!("__thaw_napi_reference_{}", context as usize)
}

/// Calls the global `name` with the JSON array `args_json`; the result is its JSON text.
fn call_reference_json(name: &str, args_json: &str) -> Result<String, String> {
    let Native::Array(args) = Native::from_json(args_json).map_err(|e| e.to_string())? else {
        return Err("arguments must be a JSON array".into());
    };
    with_package(default_realm(), |p| {
        let it = &mut p.interp;
        let target = it
            .lookup(&it.global.clone(), name)
            .unwrap_or(Value::Undefined);
        if !target.is_callable() {
            return Err(HostError::Bundle(format!(
                "no such function `{name}` (was it loaded via loadScript?)"
            )));
        }
        let args: Vec<Value> = args.iter().map(|a| to_value(it, a)).collect();
        let result = it
            .call(&target, &Value::Undefined, &args)
            .map_err(|c| classify(it, c))?;
        let result = awaited(it, result)?;
        result_json(it, &result)
    })
    .map_err(error_text)
}

/// Evaluates `source` in the default realm; the completion value's JSON text, or `None` for `undefined`.
/// Failures (syntax errors, throws) are the error text.
#[no_mangle]
pub extern "C" fn thaw_tjs_eval_json(source: *const c_char) -> ThawResult {
    let result = (|| {
        let source = cstr(source).ok_or("invalid source")?;
        with_package(default_realm(), |p| {
            let it = &mut p.interp;
            let value = it.run_sloppy(&source).map_err(|c| classify(it, c))?;
            json_stringify(it, &value)
        })
        .map_err(error_text)
    })();
    match result {
        Ok(Some(text)) => ThawResult {
            value: thaw_arena::owned_string(text),
            error: ptr::null_mut(),
        },
        Ok(None) => ThawResult {
            value: ptr::null_mut(),
            error: ptr::null_mut(),
        },
        Err(error) => ThawResult {
            value: ptr::null_mut(),
            error: thaw_arena::owned_string(error),
        },
    }
}

/// Calls the retained reference `__thaw_napi_reference_<context>` with the JSON array `args_json`.
/// The result is its JSON text; a failure is the `\u{2}` marker and the error as a JSON string.
///
/// # Safety
///
/// `args_json` must point to a live NUL-terminated string.
#[no_mangle]
pub unsafe extern "C" fn thaw_tjs_call_reference(
    context: *mut c_void,
    args_json: *const c_char,
) -> *const c_char {
    let args_json = cstr(args_json).unwrap_or_default();
    let text = match call_reference_json(&reference_name(context), &args_json) {
        Ok(text) => text,
        Err(reason) => reference_error(&reason),
    };
    thaw_arena::owned_string(text) as *const c_char
}

/// Graph-transport companion of [`thaw_tjs_call_reference`]: `args_json` is a graph wire of
/// `[true, receiver, ...arguments]`, the result a graph wire.
///
/// # Safety
///
/// Same contract as [`thaw_tjs_call_reference`].
#[no_mangle]
pub unsafe extern "C" fn thaw_tjs_call_reference_graph(
    context: *mut c_void,
    args_json: *const c_char,
) -> *const c_char {
    let name = CString::new(reference_name(context)).unwrap_or_default();
    let result = thaw_tjs_call_graph_result(name.as_ptr(), args_json);
    if result.error.is_null() {
        return result.value;
    }
    let message = CStr::from_ptr(result.error).to_string_lossy().into_owned();
    thaw_arena::destroy_string(result.error);
    thaw_arena::owned_string(reference_error(&message)) as *const c_char
}

/// Calls `handle` with `args_json` a graph wire of `[thisArg, ...arguments]`. The result stays a live
/// handle (a returned Promise is not assimilated); a throw is returned as the thrown value's handle.
#[no_mangle]
pub extern "C" fn thaw_tjs_call_handle_with_this_graph_result(
    handle: u64,
    args_json: *const c_char,
) -> ThawCallWithExceptionResult {
    match call_with_this(handle, args_json) {
        Ok(Outcome::Value(value)) => ThawCallWithExceptionResult {
            value,
            exception_handle: 0,
            exception_object_like: 0,
            error: ptr::null_mut(),
        },
        Ok(Outcome::Thrown(exception_handle, object_like)) => ThawCallWithExceptionResult {
            value: 0,
            exception_handle,
            exception_object_like: u8::from(object_like),
            error: ptr::null_mut(),
        },
        Err(error) => ThawCallWithExceptionResult {
            value: 0,
            exception_handle: 0,
            exception_object_like: 0,
            error: thaw_arena::owned_string(error),
        },
    }
}

fn call_with_this(handle: u64, args_json: *const c_char) -> Result<Outcome, String> {
    let args_json = cstr(args_json).ok_or("invalid arguments")?;
    let (_, args) = decode_wire(&args_json)?;
    on_handle(handle, |p, owner, f| {
        let it = &mut p.interp;
        check_handle_callable(it, &f)?;
        let mut args = it.array_items(&args);
        if args.is_empty() {
            return Err(HostError::Bundle(
                "JS callback receiver payload is empty".into(),
            ));
        }
        let this = args.remove(0);
        match it.call(&f, &this, &args) {
            Ok(value) => Ok(Outcome::Value(store_handle(owner, value) as u64)),
            Err(Ctl::Throw(thrown)) => {
                let object_like = matches!(thrown, Value::Object(_));
                Ok(Outcome::Thrown(
                    store_handle(owner, thrown) as u64,
                    object_like,
                ))
            }
            Err(c) => Err(classify(it, c)),
        }
    })
}

/// Whether the native handle `native_handle` of kind `kind` is the one the JS graph token `handle`
/// names, as checked by the prelude's `__thaw_json_graph_native_pair_matches`. `value` is 1 or 0.
#[no_mangle]
pub extern "C" fn thaw_tjs_graph_native_pair_matches_result(
    handle: u64,
    native_handle: u64,
    kind: u8,
) -> HandleResult {
    handle_result(on_handle(handle, |p, _, value| {
        let it = &mut p.interp;
        let global = Value::Object(it.global_object.clone());
        let verify = it
            .get(&global, "__thaw_json_graph_native_pair_matches")
            .map_err(|c| classify(it, c))?;
        if !verify.is_callable() {
            return Err(HostError::Bundle(
                "__thaw_json_graph_native_pair_matches is not defined".into(),
            ));
        }
        let args = [
            value,
            Value::Str(native_handle.to_string().into()),
            Value::Number(f64::from(kind)),
        ];
        let matches = it
            .call(&verify, &global, &args)
            .map_err(|c| classify(it, c))?;
        Ok(i64::from(matches.truthy()))
    }))
}
