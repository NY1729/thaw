#[no_mangle]
pub unsafe extern "C" fn napi_get_value_double(
    env: NapiEnv,
    value: NapiValue,
    out: *mut f64,
) -> NapiStatus {
    if out.is_null() || !value_belongs_to_environment(env, value) {
        return record_status(env, NAPI_INVALID_ARG);
    }
    let status = match value_ref(value) {
        Ok(Value::Number(number)) => {
            *out = *number;
            NAPI_OK
        }
        _ => NAPI_NUMBER_EXPECTED,
    };
    record_status(env, status)
}
#[no_mangle]
pub unsafe extern "C" fn napi_get_value_int32(
    env: NapiEnv,
    value: NapiValue,
    out: *mut i32,
) -> NapiStatus {
    if out.is_null() || !value_belongs_to_environment(env, value) {
        return record_status(env, NAPI_INVALID_ARG);
    }
    let status = match value_ref(value) {
        Ok(Value::Number(number)) => {
            *out = *number as i32;
            NAPI_OK
        }
        _ => NAPI_NUMBER_EXPECTED,
    };
    record_status(env, status)
}

#[no_mangle]
pub unsafe extern "C" fn napi_get_value_uint32(
    env: NapiEnv,
    value: NapiValue,
    out: *mut u32,
) -> NapiStatus {
    if out.is_null() || !value_belongs_to_environment(env, value) {
        return record_status(env, NAPI_INVALID_ARG);
    }
    let status = match value_ref(value) {
        Ok(Value::Number(number)) => {
            *out = if !number.is_finite() || *number == 0.0 {
                0
            } else {
                number.trunc().rem_euclid(4_294_967_296.0) as u32
            };
            NAPI_OK
        }
        _ => NAPI_NUMBER_EXPECTED,
    };
    record_status(env, status)
}

#[no_mangle]
pub unsafe extern "C" fn napi_get_value_int64(
    env: NapiEnv,
    value: NapiValue,
    out: *mut i64,
) -> NapiStatus {
    if out.is_null() || !value_belongs_to_environment(env, value) {
        return record_status(env, NAPI_INVALID_ARG);
    }
    let status = match value_ref(value) {
        Ok(Value::Number(number)) => {
            *out = *number as i64;
            NAPI_OK
        }
        _ => NAPI_NUMBER_EXPECTED,
    };
    record_status(env, status)
}

#[no_mangle]
pub unsafe extern "C" fn napi_coerce_to_bool(
    env: NapiEnv,
    value: NapiValue,
    out: *mut NapiValue,
) -> NapiStatus {
    if out.is_null() || !value_belongs_to_environment(env, value) {
        return record_status(env, NAPI_INVALID_ARG);
    }
    let boolean = match value_ref(value) {
        Ok(Value::Undefined | Value::Null) => false,
        Ok(Value::Bool(value)) => *value,
        Ok(Value::Number(value)) => *value != 0.0 && !value.is_nan(),
        Ok(Value::String(value)) => !value.is_empty(),
        Ok(_) => true,
        Err(status) => return record_status(env, status),
    };
    let status = napi_get_boolean(env, boolean, out);
    record_status(env, status)
}

fn javascript_number_from_string(value: &str) -> f64 {
    let value = value.trim();
    if value.is_empty() {
        return 0.0;
    }
    match value {
        "Infinity" | "+Infinity" => return f64::INFINITY,
        "-Infinity" => return f64::NEG_INFINITY,
        _ => {}
    }
    let radix = [
        ("0x", 16),
        ("0X", 16),
        ("0b", 2),
        ("0B", 2),
        ("0o", 8),
        ("0O", 8),
    ]
    .into_iter()
    .find_map(|(prefix, radix)| value.strip_prefix(prefix).map(|digits| (digits, radix)));
    if let Some((digits, radix)) = radix {
        return u128::from_str_radix(digits, radix)
            .map(|number| number as f64)
            .unwrap_or(f64::NAN);
    }
    value.parse().unwrap_or(f64::NAN)
}

fn bigint_to_decimal(negative: bool, words: &[u64]) -> String {
    const BASE: u128 = 1_000_000_000;
    let mut decimal = vec![0_u32];
    for word in words.iter().rev() {
        let mut carry = *word as u128;
        for limb in &mut decimal {
            let value = (*limb as u128) * (1_u128 << 64) + carry;
            *limb = (value % BASE) as u32;
            carry = value / BASE;
        }
        while carry != 0 {
            decimal.push((carry % BASE) as u32);
            carry /= BASE;
        }
    }
    while decimal.len() > 1 && decimal.last() == Some(&0) {
        decimal.pop();
    }
    let mut result = decimal.last().unwrap_or(&0).to_string();
    for limb in decimal.iter().rev().skip(1) {
        result.push_str(&format!("{limb:09}"));
    }
    if negative && result != "0" {
        result.insert(0, '-');
    }
    result
}

unsafe fn javascript_string(
    env: NapiEnv,
    value: NapiValue,
    arrays: &mut HashSet<usize>,
) -> Result<String, ()> {
    Ok(match value_ref(value).map_err(|_| ())? {
        Value::Undefined => "undefined".into(),
        Value::Null => "null".into(),
        Value::Bool(value) => value.to_string(),
        Value::Number(value) if *value == 0.0 => "0".into(),
        Value::Number(value) if value.is_infinite() => {
            if value.is_sign_negative() {
                "-Infinity".into()
            } else {
                "Infinity".into()
            }
        }
        Value::Number(value) => value.to_string(),
        Value::BigInt { negative, words } => bigint_to_decimal(*negative, words),
        Value::String(value) => value.clone(),
        Value::Error(message) => {
            let name_key = PropertyKey::String("name".into());
            let name = find_property_value(env, value, &name_key)
                .and_then(|name| match value_ref(name) {
                    Ok(Value::String(name)) => Some(name.clone()),
                    _ => None,
                })
                .unwrap_or_else(|| "Error".into());
            match (name.is_empty(), message.is_empty()) {
                (true, _) => message.clone(),
                (_, true) => name,
                _ => format!("{name}: {message}"),
            }
        }
        Value::Symbol { .. } => return Err(()),
        Value::Array(values) => {
            if !arrays.insert(value as usize) {
                return Ok(String::new());
            }
            let result = values
                .iter()
                .map(|item| {
                    match item.and_then(|item| value_ref(item).ok().map(|value| (item, value))) {
                        None | Some((_, Value::Undefined | Value::Null)) => Ok(String::new()),
                        Some((item, _)) => javascript_string(env, item, arrays),
                    }
                })
                .collect::<Result<Vec<_>, _>>()?
                .join(",");
            arrays.remove(&(value as usize));
            result
        }
        _ => "[object Object]".into(),
    })
}

unsafe fn coercion_type_error(env: NapiEnv, message: &str) -> NapiStatus {
    let Ok(env) = env_mut(env) else {
        return NAPI_INVALID_ARG;
    };
    let error = env.alloc(Value::Error(message.into()));
    env.exception = Some(error);
    record_status(env, NAPI_PENDING_EXCEPTION)
}

#[no_mangle]
pub unsafe extern "C" fn napi_coerce_to_number(
    env: NapiEnv,
    value: NapiValue,
    out: *mut NapiValue,
) -> NapiStatus {
    if out.is_null() || !value_belongs_to_environment(env, value) {
        return record_status(env, NAPI_INVALID_ARG);
    }
    let number = match value_ref(value) {
        Ok(Value::Undefined) => f64::NAN,
        Ok(Value::Null) => 0.0,
        Ok(Value::Bool(value)) => u8::from(*value) as f64,
        Ok(Value::Number(value)) => *value,
        Ok(Value::Date(value)) => *value,
        Ok(Value::String(value)) => javascript_number_from_string(value),
        Ok(Value::BigInt { .. } | Value::Symbol { .. }) => {
            return coercion_type_error(env, "value cannot be converted to a number");
        }
        Ok(Value::Array(_)) => match javascript_string(env, value, &mut HashSet::new()) {
            Ok(value) => javascript_number_from_string(&value),
            Err(()) => return coercion_type_error(env, "value cannot be converted to a number"),
        },
        Ok(_) => f64::NAN,
        Err(status) => return record_status(env, status),
    };
    let status = napi_create_double(env, number, out);
    record_status(env, status)
}

#[no_mangle]
pub unsafe extern "C" fn napi_coerce_to_string(
    env: NapiEnv,
    value: NapiValue,
    out: *mut NapiValue,
) -> NapiStatus {
    if out.is_null() || !value_belongs_to_environment(env, value) {
        return record_status(env, NAPI_INVALID_ARG);
    }
    let string = match javascript_string(env, value, &mut HashSet::new()) {
        Ok(string) => string,
        Err(()) => return coercion_type_error(env, "a Symbol cannot be converted to a string"),
    };
    let Ok(env) = env_mut(env) else {
        return NAPI_INVALID_ARG;
    };
    let value = env.alloc(Value::String(string));
    write_value(out, value)
}

#[no_mangle]
pub unsafe extern "C" fn napi_coerce_to_object(
    env: NapiEnv,
    value: NapiValue,
    out: *mut NapiValue,
) -> NapiStatus {
    if out.is_null() || !value_belongs_to_environment(env, value) {
        return record_status(env, NAPI_INVALID_ARG);
    }
    if matches!(value_ref(value), Ok(value) if is_object_value(value)) {
        return write_value(out, value);
    }
    if matches!(value_ref(value), Ok(Value::Undefined | Value::Null)) {
        return record_status(env, NAPI_INVALID_ARG);
    }
    let Ok(env) = env_mut(env) else {
        return NAPI_INVALID_ARG;
    };
    let object = env.alloc(Value::Object(HashMap::from([("value".into(), value)])));
    write_value(out, object)
}
#[no_mangle]
pub unsafe extern "C" fn napi_get_value_bool(
    env: NapiEnv,
    value: NapiValue,
    out: *mut bool,
) -> NapiStatus {
    if out.is_null() || !value_belongs_to_environment(env, value) {
        return record_status(env, NAPI_INVALID_ARG);
    }
    let status = match value_ref(value) {
        Ok(Value::Bool(boolean)) => {
            *out = *boolean;
            NAPI_OK
        }
        _ => NAPI_BOOLEAN_EXPECTED,
    };
    record_status(env, status)
}

#[no_mangle]
pub unsafe extern "C" fn napi_get_value_string_utf8(
    env: NapiEnv,
    value: NapiValue,
    buffer: *mut c_char,
    size: usize,
    written: *mut usize,
) -> NapiStatus {
    if !value_belongs_to_environment(env, value) {
        return record_status(env, NAPI_INVALID_ARG);
    }
    let string = match value_ref(value) {
        Ok(Value::String(string)) => string,
        _ => return record_status(env, NAPI_STRING_EXPECTED),
    };
    let mut count = string.len();
    if !buffer.is_null() && size > 0 {
        count = string.len().min(size - 1);
        while !string.is_char_boundary(count) {
            count -= 1;
        }
        ptr::copy_nonoverlapping(string.as_ptr(), buffer.cast(), count);
        *buffer.add(count) = 0;
    }
    if !written.is_null() {
        *written = count;
    }
    NAPI_OK
}

#[no_mangle]
pub unsafe extern "C" fn napi_get_value_string_latin1(
    env: NapiEnv,
    value: NapiValue,
    buffer: *mut c_char,
    size: usize,
    written: *mut usize,
) -> NapiStatus {
    if !value_belongs_to_environment(env, value) {
        return record_status(env, NAPI_INVALID_ARG);
    }
    let string = match value_ref(value) {
        Ok(Value::String(string)) => string,
        _ => return record_status(env, NAPI_STRING_EXPECTED),
    };
    let encoded: Vec<u8> = string.encode_utf16().map(|unit| unit as u8).collect();
    let count = if buffer.is_null() || size == 0 {
        encoded.len()
    } else {
        let count = encoded.len().min(size - 1);
        ptr::copy_nonoverlapping(encoded.as_ptr(), buffer.cast(), count);
        *buffer.add(count) = 0;
        count
    };
    if !written.is_null() {
        *written = count;
    }
    NAPI_OK
}

#[no_mangle]
pub unsafe extern "C" fn napi_get_value_string_utf16(
    env: NapiEnv,
    value: NapiValue,
    buffer: *mut u16,
    size: usize,
    written: *mut usize,
) -> NapiStatus {
    if !value_belongs_to_environment(env, value) {
        return record_status(env, NAPI_INVALID_ARG);
    }
    let string = match value_ref(value) {
        Ok(Value::String(string)) => string,
        _ => return record_status(env, NAPI_STRING_EXPECTED),
    };
    let encoded: Vec<u16> = string.encode_utf16().collect();
    let count = if buffer.is_null() || size == 0 {
        encoded.len()
    } else {
        let count = encoded.len().min(size - 1);
        ptr::copy_nonoverlapping(encoded.as_ptr(), buffer, count);
        *buffer.add(count) = 0;
        count
    };
    if !written.is_null() {
        *written = count;
    }
    NAPI_OK
}

#[no_mangle]
pub unsafe extern "C" fn napi_throw_error(
    env: NapiEnv,
    code: *const c_char,
    message: *const c_char,
) -> NapiStatus {
    let Ok(message) = text(message) else {
        return record_status(env, NAPI_INVALID_ARG);
    };
    let Ok(env) = env_mut(env) else {
        return NAPI_INVALID_ARG;
    };
    let code = (!code.is_null()).then(|| env.alloc(Value::String(text(code).unwrap_or_default())));
    let error = alloc_error(env, "Error", message, code);
    env.exception = Some(error);
    NAPI_OK
}
#[no_mangle]
pub unsafe extern "C" fn napi_throw_type_error(
    env: NapiEnv,
    code: *const c_char,
    message: *const c_char,
) -> NapiStatus {
    throw_error_kind(env, "TypeError", code, message)
}
#[no_mangle]
pub unsafe extern "C" fn napi_throw_range_error(
    env: NapiEnv,
    code: *const c_char,
    message: *const c_char,
) -> NapiStatus {
    throw_error_kind(env, "RangeError", code, message)
}

#[no_mangle]
pub unsafe extern "C" fn node_api_throw_syntax_error(
    env: NapiEnv,
    code: *const c_char,
    message: *const c_char,
) -> NapiStatus {
    throw_error_kind(env, "SyntaxError", code, message)
}

unsafe fn throw_error_kind(
    env: NapiEnv,
    name: &str,
    code: *const c_char,
    message: *const c_char,
) -> NapiStatus {
    let Ok(message) = text(message) else {
        return record_status(env, NAPI_INVALID_ARG);
    };
    let Ok(env) = env_mut(env) else {
        return NAPI_INVALID_ARG;
    };
    let code = (!code.is_null()).then(|| env.alloc(Value::String(text(code).unwrap_or_default())));
    let error = alloc_error(env, name, message, code);
    env.exception = Some(error);
    NAPI_OK
}
#[no_mangle]
pub unsafe extern "C" fn napi_is_exception_pending(env: NapiEnv, out: *mut bool) -> NapiStatus {
    if out.is_null() {
        return record_status(env, NAPI_INVALID_ARG);
    }
    let Ok(env) = env_mut(env) else {
        return NAPI_INVALID_ARG;
    };
    *out = env.exception.is_some();
    NAPI_OK
}
#[no_mangle]
pub unsafe extern "C" fn napi_get_and_clear_last_exception(
    env: NapiEnv,
    out: *mut NapiValue,
) -> NapiStatus {
    if out.is_null() {
        return record_status(env, NAPI_INVALID_ARG);
    }
    let Ok(env) = env_mut(env) else {
        return NAPI_INVALID_ARG;
    };
    let value = env
        .exception
        .take()
        .unwrap_or_else(|| env.alloc(Value::Undefined));
    write_value(out, value)
}
#[no_mangle]
pub unsafe extern "C" fn napi_get_version(env: NapiEnv, out: *mut u32) -> NapiStatus {
    if env.is_null() || out.is_null() {
        record_status(env, NAPI_INVALID_ARG)
    } else {
        *out = 10;
        NAPI_OK
    }
}

#[no_mangle]
pub unsafe extern "C" fn napi_get_global(env: NapiEnv, out: *mut NapiValue) -> NapiStatus {
    let Ok(env_ref) = env_mut(env) else {
        return NAPI_INVALID_ARG;
    };
    if out.is_null() {
        return record_status(env, NAPI_INVALID_ARG);
    }
    if env_ref.global.is_null() {
        env_ref.global = env_ref.alloc(Value::Object(HashMap::new()));
        let global = env_ref.global;
        for name in [c"Date", c"RegExp"] {
            let mut constructor = ptr::null_mut();
            let mut prototype = ptr::null_mut();
            if napi_create_function(
                env,
                name.as_ptr(),
                NAPI_AUTO_LENGTH,
                Some(napi_builtin_constructor),
                ptr::null_mut(),
                &mut constructor,
            ) != NAPI_OK
                || napi_create_object(env, &mut prototype) != NAPI_OK
                || napi_set_named_property(env, constructor, c"prototype".as_ptr(), prototype)
                    != NAPI_OK
                || napi_set_named_property(env, global, name.as_ptr(), constructor) != NAPI_OK
            {
                return record_status(env, NAPI_GENERIC_FAILURE);
            }
        }
        let mut symbol = ptr::null_mut();
        if napi_create_function(
            env,
            c"Symbol".as_ptr(),
            NAPI_AUTO_LENGTH,
            Some(napi_builtin_constructor),
            ptr::null_mut(),
            &mut symbol,
        ) != NAPI_OK
        {
            return record_status(env, NAPI_GENERIC_FAILURE);
        }
        for name in [
            c"asyncDispose",
            c"asyncIterator",
            c"dispose",
            c"hasInstance",
            c"isConcatSpreadable",
            c"iterator",
            c"match",
            c"matchAll",
            c"replace",
            c"search",
            c"species",
            c"split",
            c"toPrimitive",
            c"toStringTag",
            c"unscopables",
        ] {
            let mut description = ptr::null_mut();
            let mut value = ptr::null_mut();
            if napi_create_string_utf8(
                env,
                name.as_ptr(),
                NAPI_AUTO_LENGTH,
                &mut description,
            ) != NAPI_OK
                || napi_create_symbol(env, description, &mut value) != NAPI_OK
                || napi_set_named_property(env, symbol, name.as_ptr(), value) != NAPI_OK
            {
                return record_status(env, NAPI_GENERIC_FAILURE);
            }
        }
        if napi_set_named_property(env, global, c"Symbol".as_ptr(), symbol) != NAPI_OK {
            return record_status(env, NAPI_GENERIC_FAILURE);
        }
    }
    *out = env_mut(env).unwrap().global;
    NAPI_OK
}

unsafe extern "C" fn napi_builtin_constructor(
    _env: NapiEnv,
    _info: NapiCallbackInfo,
) -> NapiValue {
    ptr::null_mut()
}

#[no_mangle]
pub unsafe extern "C" fn napi_run_script(
    env: NapiEnv,
    script: NapiValue,
    result: *mut NapiValue,
) -> NapiStatus {
    if result.is_null() || !value_belongs_to_environment(env, script) {
        return record_status(env, NAPI_INVALID_ARG);
    }
    let source = match value_ref(script) {
        Ok(Value::String(source)) => source.clone(),
        _ => return record_status(env, NAPI_STRING_EXPECTED),
    };
    // A JSON literal is also a valid JavaScript script, but evaluating it does
    // not require a JavaScript engine. Keep this fast path deliberately strict:
    // assignments, expressions, and other syntax continue through QuickJS so
    // `napi_run_script` retains its existing semantics for non-JSON scripts.
    let evaluated = match serde_json::from_str::<JsonValue>(&source) {
        Ok(json) => Ok(Some(json.to_string())),
        #[cfg(feature = "quickjs")]
        Err(_) => thaw_quickjs::eval_json(&source),
        #[cfg(not(feature = "quickjs"))]
        Err(_) => Err("napi_run_script requires the QuickJS feature for JavaScript syntax".into()),
    };
    let env_ptr = env;
    let Ok(env) = env_mut(env) else {
        return NAPI_INVALID_ARG;
    };
    let status = match evaluated {
        Ok(Some(json)) => match serde_json::from_str::<JsonValue>(&json) {
            Ok(json) => write_value(result, value_from_json(env, &json)),
            Err(error) => {
                env.exception = Some(env.alloc(Value::Error(format!(
                    "failed to decode script result: {error}"
                ))));
                NAPI_PENDING_EXCEPTION
            }
        },
        Ok(None) => {
            let undefined = env.alloc(Value::Undefined);
            write_value(result, undefined)
        }
        Err(error) => {
            env.exception = Some(env.alloc(Value::Error(error)));
            NAPI_PENDING_EXCEPTION
        }
    };
    record_status(env_ptr, status)
}
