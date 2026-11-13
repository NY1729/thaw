//! Pure handle operations of the N-API engine seam: strict equality, `instanceof`, prototype reads
//! and writes, Promise tests, object integrity and UTF-16 string building. Each mirrors the
//! same-named `thaw_js_*` function of thaw-quickjs. None captures an exception object (PR 2 does):
//! a JS throw comes back as the error text.

use std::ffi::c_char;
use std::rc::Rc;

use crate::host::{
    classify, cstr, default_realm, error_text, handle_result, on_handle_strict, slot_strict,
    store_handle, with_package, HandleResult, HostError,
};
use crate::napi_graph::decode_napi_wire;
use crate::{builtins, strict_equals, Interp, Kind, Obj, Value, R};

/// The value behind a handle id, for a read that is not an operation on it.
pub(crate) fn handle_value(handle: u64) -> Result<Value, String> {
    slot_strict(handle).map(|(_, v)| v)
}

/// `[[GetPrototypeOf]]`, honouring a proxy's trap.
pub(crate) fn prototype_of(it: &mut Interp, object: &Obj) -> R<Value> {
    Ok(match it.proxy_get_prototype_of(object)? {
        Some(proto) => proto,
        None => object
            .proto
            .borrow()
            .clone()
            .map_or(Value::Null, Value::Object),
    })
}

/// `object instanceof constructor`: `Symbol.hasInstance` first, else the prototype chain walked with
/// proxy `getPrototypeOf` traps honoured (the interpreter's own operator reads the raw chain).
fn instance_of(it: &mut Interp, object: &Value, constructor: &Value) -> R<bool> {
    if !matches!(constructor, Value::Object(_)) {
        return it.throw_error(
            "TypeError",
            "Right-hand side of 'instanceof' is not an object",
        );
    }
    let key = it.well_known_key("hasInstance");
    let custom = it.get(constructor, &key)?;
    if custom.is_callable() {
        return Ok(it
            .call(&custom, constructor, std::slice::from_ref(object))?
            .truthy());
    }
    if !constructor.is_callable() {
        return it.throw_error(
            "TypeError",
            "Right-hand side of 'instanceof' is not callable",
        );
    }
    let Value::Object(proto) = it.get(constructor, "prototype")? else {
        return it.throw_error(
            "TypeError",
            "Function has non-object prototype in instanceof check",
        );
    };
    let Value::Object(object) = object else {
        return Ok(false);
    };
    let mut current = prototype_of(it, object)?;
    loop {
        let node = match &current {
            Value::Object(node) => node.clone(),
            _ => return Ok(false),
        };
        if Rc::ptr_eq(&node, &proto) {
            return Ok(true);
        }
        current = prototype_of(it, &node)?;
    }
}

/// `Reflect.setPrototypeOf(object, prototype)`: false when the object is not extensible, or the
/// change would make a cycle. A proxy's `setPrototypeOf` trap is not modelled, so a proxy is refused.
fn set_prototype(it: &mut Interp, object: &Value, prototype: &Value) -> R<bool> {
    let Value::Object(target) = object else {
        return it.throw_error("TypeError", "Reflect.setPrototypeOf called on non-object");
    };
    let proto: Option<Obj> = match prototype {
        Value::Object(p) => Some(p.clone()),
        Value::Null => None,
        _ => {
            return it.throw_error(
                "TypeError",
                "Object prototype may only be an Object or null",
            )
        }
    };
    if matches!(&*target.kind.borrow(), Kind::Proxy(_)) {
        return it.throw_error("TypeError", "setPrototypeOf on a Proxy is not supported");
    }
    let current = target.proto.borrow().clone();
    let unchanged = match (&current, &proto) {
        (None, None) => true,
        (Some(a), Some(b)) => Rc::ptr_eq(a, b),
        _ => false,
    };
    if unchanged {
        return Ok(true);
    }
    if target.ext.get() != 0 || target.frozen.get() {
        return Ok(false);
    }
    let mut walk = proto.clone();
    while let Some(node) = walk {
        if Rc::ptr_eq(&node, target) {
            return Ok(false);
        }
        walk = node.proto.borrow().clone();
    }
    *target.proto.borrow_mut() = proto;
    Ok(true)
}

/// Strict equality of the values behind two handles (`===`; NaN is unequal, -0 equals 0).
#[no_mangle]
pub extern "C" fn thaw_tjs_strict_equal_handles_result(left: u64, right: u64) -> HandleResult {
    handle_result((|| {
        let right = handle_value(right)?;
        on_handle_strict(left, |_, _, left| {
            Ok(i64::from(strict_equals(&left, &right)))
        })
    })())
}

/// Strict equality of the value behind `handle` and a native scalar. `kind`: 0 undefined, 1 null,
/// 2 boolean (`number` != 0), 3 number, 4 string (`text` is its JSON), 5 BigInt (`text` decimal),
/// 6 string (`text` is its UTF-16 units as JSON).
#[no_mangle]
pub extern "C" fn thaw_tjs_strict_equal_handle_scalar_result(
    handle: u64,
    kind: u8,
    number: f64,
    text: *const c_char,
) -> HandleResult {
    handle_result((|| {
        let text = cstr(text).unwrap_or_default();
        let scalar = match kind {
            0 => Value::Undefined,
            1 => Value::Null,
            2 => Value::Bool(number != 0.0),
            3 => Value::Number(number),
            4 => Value::Str(
                serde_json::from_str::<String>(&text)
                    .map_err(|e| e.to_string())?
                    .into(),
            ),
            5 => Value::BigInt(Rc::new(
                text.parse::<num_bigint::BigInt>()
                    .map_err(|_| "invalid BigInt text")?,
            )),
            6 => {
                let units: Vec<u16> = serde_json::from_str(&text).map_err(|e| e.to_string())?;
                Value::Str(builtins::units_str(&units).into())
            }
            _ => return Err("TypeError: Invalid native scalar kind".to_string()),
        };
        on_handle_strict(handle, |_, _, left| {
            Ok(i64::from(strict_equals(&left, &scalar)))
        })
    })())
}

/// `object instanceof constructor` for the values behind two handles.
#[no_mangle]
pub extern "C" fn thaw_tjs_instanceof_handles_result(
    object: u64,
    constructor: u64,
) -> HandleResult {
    handle_result((|| {
        let constructor = handle_value(constructor)?;
        on_handle_strict(object, |p, _, object| {
            let it = &mut p.interp;
            let answer = instance_of(it, &object, &constructor).map_err(|c| classify(it, c))?;
            Ok(i64::from(answer))
        })
    })())
}

/// `instanceof` for a graph wire of `[object, constructor]`.
#[no_mangle]
pub extern "C" fn thaw_tjs_instanceof_graph_result(arguments_graph: *const c_char) -> HandleResult {
    handle_result((|| {
        let wire = cstr(arguments_graph).ok_or("invalid graph")?;
        let (owner, arguments) = decode_napi_wire(&wire)?;
        with_package(owner, |p| {
            let it = &mut p.interp;
            let object = it.get(&arguments, "0").map_err(|c| classify(it, c))?;
            let constructor = it.get(&arguments, "1").map_err(|c| classify(it, c))?;
            let answer = instance_of(it, &object, &constructor).map_err(|c| classify(it, c))?;
            Ok(i64::from(answer))
        })
        .map_err(error_text)
    })())
}

/// Napi-mode counterpart of `thaw_tjs_construct_handle_graph_args_result`: the arguments decode with the
/// N-API rules; the instance is kept as a new handle.
#[no_mangle]
pub extern "C" fn thaw_tjs_napi_construct_handle_graph_args_result(
    handle: u64,
    args_json: *const c_char,
) -> HandleResult {
    handle_result((|| {
        let args_json = cstr(args_json).ok_or("invalid arguments")?;
        let (_, args) = decode_napi_wire(&args_json)?;
        on_handle_strict(handle, |p, owner, ctor| {
            let it = &mut p.interp;
            let args = it.array_items(&args);
            let value = it.construct(&ctor, &args).map_err(|c| classify(it, c))?;
            Ok(store_handle(owner, value))
        })
    })())
}

/// Whether the value behind `handle` is a native Promise (no `instanceof` or user property is read).
#[no_mangle]
pub extern "C" fn thaw_tjs_handle_is_promise_result(handle: u64) -> HandleResult {
    handle_result(on_handle_strict(handle, |_, _, v| {
        let promise =
            matches!(&v, Value::Object(o) if matches!(&*o.kind.borrow(), Kind::Promise(_)));
        Ok(i64::from(promise))
    }))
}

/// `Reflect.getPrototypeOf` of the object behind `handle`: a new handle (null for no prototype).
#[no_mangle]
pub extern "C" fn thaw_tjs_get_prototype_handle_result(handle: u64) -> HandleResult {
    handle_result(on_handle_strict(handle, |p, owner, object| {
        let Value::Object(object) = &object else {
            return Err(HostError::Thrown(
                "TypeError: Reflect.getPrototypeOf called on non-object".into(),
            ));
        };
        let it = &mut p.interp;
        let proto = prototype_of(it, object).map_err(|c| classify(it, c))?;
        Ok(store_handle(owner, proto))
    }))
}

/// `Reflect.setPrototypeOf` of the object behind `handle`; the prototype is the single element of
/// the graph wire's argument array. Returns 1 when the change took effect (or was already in place).
#[no_mangle]
pub extern "C" fn thaw_tjs_set_prototype_graph_result(
    handle: u64,
    prototype_graph: *const c_char,
) -> HandleResult {
    handle_result((|| {
        let wire = cstr(prototype_graph).ok_or("invalid graph")?;
        let (_, arguments) = decode_napi_wire(&wire)?;
        on_handle_strict(handle, |p, _, object| {
            let it = &mut p.interp;
            let prototype = it.get(&arguments, "0").map_err(|c| classify(it, c))?;
            let changed = set_prototype(it, &object, &prototype).map_err(|c| classify(it, c))?;
            Ok(i64::from(changed))
        })
    })())
}

/// Freeze (`freeze` != 0) or seal the object behind `handle`, as `Object.freeze` / `Object.seal`.
/// Returns 1 on success.
#[no_mangle]
pub extern "C" fn thaw_tjs_host_object_integrity_result(handle: u64, freeze: u8) -> HandleResult {
    handle_result(on_handle_strict(handle, |_, _, v| {
        let Value::Object(object) = &v else {
            return Err(HostError::Thrown(format!(
                "TypeError: JavaScript value handle {handle} is not an object"
            )));
        };
        if freeze != 0 {
            builtins::freeze_obj(object);
        } else {
            builtins::seal_obj(object);
        }
        Ok(1)
    }))
}

/// A new string from its UTF-16 code units (a JSON array of numbers). Lone surrogates survive.
#[no_mangle]
pub extern "C" fn thaw_tjs_string_from_utf16_units_result(
    units_json: *const c_char,
) -> HandleResult {
    handle_result((|| {
        let units = cstr(units_json).ok_or("invalid UTF-16 units")?;
        let units: Vec<u16> =
            serde_json::from_str(&units).map_err(|_| "Expected UTF-16 units".to_string())?;
        let value = Value::Str(builtins::units_str(&units).into());
        Ok(store_handle(default_realm(), value))
    })())
}
