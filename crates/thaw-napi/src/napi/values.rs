#[no_mangle]
pub unsafe extern "C" fn napi_get_undefined(env: NapiEnv, out: *mut NapiValue) -> NapiStatus {
    let Ok(env) = env_for_value_output(env, out) else {
        return NAPI_INVALID_ARG;
    };
    let value = env.alloc(Value::Undefined);
    write_value(out, value)
}
#[no_mangle]
pub unsafe extern "C" fn napi_get_null(env: NapiEnv, out: *mut NapiValue) -> NapiStatus {
    let Ok(env) = env_for_value_output(env, out) else {
        return NAPI_INVALID_ARG;
    };
    let value = env.alloc(Value::Null);
    write_value(out, value)
}
#[no_mangle]
pub unsafe extern "C" fn napi_get_boolean(
    env: NapiEnv,
    value: bool,
    out: *mut NapiValue,
) -> NapiStatus {
    let Ok(env) = env_for_value_output(env, out) else {
        return NAPI_INVALID_ARG;
    };
    let value = env.alloc(Value::Bool(value));
    write_value(out, value)
}
#[no_mangle]
pub unsafe extern "C" fn napi_create_double(
    env: NapiEnv,
    value: f64,
    out: *mut NapiValue,
) -> NapiStatus {
    let Ok(env) = env_for_value_output(env, out) else {
        return NAPI_INVALID_ARG;
    };
    let value = env.alloc(Value::Number(value));
    write_value(out, value)
}

#[no_mangle]
pub unsafe extern "C" fn napi_create_int32(
    env: NapiEnv,
    value: i32,
    out: *mut NapiValue,
) -> NapiStatus {
    napi_create_double(env, f64::from(value), out)
}

#[no_mangle]
pub unsafe extern "C" fn napi_create_uint32(
    env: NapiEnv,
    value: u32,
    out: *mut NapiValue,
) -> NapiStatus {
    napi_create_double(env, f64::from(value), out)
}

#[no_mangle]
pub unsafe extern "C" fn napi_create_int64(
    env: NapiEnv,
    value: i64,
    out: *mut NapiValue,
) -> NapiStatus {
    napi_create_double(env, value as f64, out)
}

#[no_mangle]
pub unsafe extern "C" fn napi_create_date(
    env: NapiEnv,
    value: f64,
    out: *mut NapiValue,
) -> NapiStatus {
    let Ok(env) = env_for_value_output(env, out) else {
        return NAPI_INVALID_ARG;
    };
    let value = env.alloc(Value::Date(value));
    write_value(out, value)
}

#[no_mangle]
pub unsafe extern "C" fn napi_is_date(
    env: NapiEnv,
    value: NapiValue,
    out: *mut bool,
) -> NapiStatus {
    if out.is_null() || !value_belongs_to_environment(env, value) {
        return record_status(env, NAPI_INVALID_ARG);
    }
    *out = matches!(value_ref(value), Ok(Value::Date(_)));
    NAPI_OK
}

#[no_mangle]
pub unsafe extern "C" fn napi_get_date_value(
    env: NapiEnv,
    value: NapiValue,
    out: *mut f64,
) -> NapiStatus {
    if out.is_null() || !value_belongs_to_environment(env, value) {
        return record_status(env, NAPI_INVALID_ARG);
    }
    let status = match value_ref(value) {
        Ok(Value::Date(milliseconds)) => {
            *out = *milliseconds;
            NAPI_OK
        }
        _ => NAPI_DATE_EXPECTED,
    };
    record_status(env, status)
}

#[no_mangle]
pub unsafe extern "C" fn napi_create_bigint_int64(
    env: NapiEnv,
    value: i64,
    out: *mut NapiValue,
) -> NapiStatus {
    let Ok(env) = env_for_value_output(env, out) else {
        return NAPI_INVALID_ARG;
    };
    let magnitude = value.unsigned_abs();
    let value = env.alloc(Value::BigInt {
        negative: value.is_negative(),
        words: vec![magnitude],
    });
    write_value(out, value)
}

#[no_mangle]
pub unsafe extern "C" fn napi_create_bigint_uint64(
    env: NapiEnv,
    value: u64,
    out: *mut NapiValue,
) -> NapiStatus {
    let Ok(env) = env_for_value_output(env, out) else {
        return NAPI_INVALID_ARG;
    };
    let value = env.alloc(Value::BigInt {
        negative: false,
        words: vec![value],
    });
    write_value(out, value)
}

#[no_mangle]
pub unsafe extern "C" fn napi_create_bigint_words(
    env: NapiEnv,
    sign_bit: i32,
    word_count: usize,
    words: *const u64,
    out: *mut NapiValue,
) -> NapiStatus {
    if out.is_null() || sign_bit != 0 && sign_bit != 1 || (word_count != 0 && words.is_null()) {
        return record_status(env, NAPI_INVALID_ARG);
    }
    let Ok(env) = env_for_value_output(env, out) else {
        return NAPI_INVALID_ARG;
    };
    let mut magnitude = if word_count == 0 {
        Vec::new()
    } else {
        std::slice::from_raw_parts(words, word_count).to_vec()
    };
    while magnitude.last() == Some(&0) {
        magnitude.pop();
    }
    if magnitude.is_empty() {
        magnitude.push(0);
    }
    let value = env.alloc(Value::BigInt {
        negative: sign_bit == 1 && magnitude != [0],
        words: magnitude,
    });
    write_value(out, value)
}

#[no_mangle]
pub unsafe extern "C" fn napi_get_value_bigint_int64(
    env: NapiEnv,
    value: NapiValue,
    out: *mut i64,
    lossless: *mut bool,
) -> NapiStatus {
    if out.is_null() || lossless.is_null() || !value_belongs_to_environment(env, value) {
        return record_status(env, NAPI_INVALID_ARG);
    }
    let Value::BigInt { negative, words } = (match value_ref(value) {
        Ok(value) => value,
        Err(_) => return record_status(env, NAPI_INVALID_ARG),
    }) else {
        return record_status(env, NAPI_BIGINT_EXPECTED);
    };
    let low = words.first().copied().unwrap_or(0);
    *out = if *negative {
        low.wrapping_neg() as i64
    } else {
        low as i64
    };
    *lossless = words.len() <= 1
        && if *negative {
            low <= (i64::MAX as u64) + 1
        } else {
            low <= i64::MAX as u64
        };
    NAPI_OK
}

#[no_mangle]
pub unsafe extern "C" fn napi_get_value_bigint_uint64(
    env: NapiEnv,
    value: NapiValue,
    out: *mut u64,
    lossless: *mut bool,
) -> NapiStatus {
    if out.is_null() || lossless.is_null() || !value_belongs_to_environment(env, value) {
        return record_status(env, NAPI_INVALID_ARG);
    }
    let Value::BigInt { negative, words } = (match value_ref(value) {
        Ok(value) => value,
        Err(_) => return record_status(env, NAPI_INVALID_ARG),
    }) else {
        return record_status(env, NAPI_BIGINT_EXPECTED);
    };
    let low = words.first().copied().unwrap_or(0);
    *out = if *negative { low.wrapping_neg() } else { low };
    *lossless = !*negative && words.len() <= 1;
    NAPI_OK
}

#[no_mangle]
pub unsafe extern "C" fn napi_get_value_bigint_words(
    env: NapiEnv,
    value: NapiValue,
    sign_bit: *mut i32,
    word_count: *mut usize,
    words: *mut u64,
) -> NapiStatus {
    if sign_bit.is_null() || word_count.is_null() || !value_belongs_to_environment(env, value) {
        return record_status(env, NAPI_INVALID_ARG);
    }
    let Value::BigInt {
        negative,
        words: magnitude,
    } = (match value_ref(value) {
        Ok(value) => value,
        Err(_) => return record_status(env, NAPI_INVALID_ARG),
    })
    else {
        return record_status(env, NAPI_BIGINT_EXPECTED);
    };
    *sign_bit = i32::from(*negative);
    let capacity = *word_count;
    *word_count = magnitude.len();
    if !words.is_null() && capacity != 0 {
        ptr::copy_nonoverlapping(magnitude.as_ptr(), words, capacity.min(magnitude.len()));
    }
    NAPI_OK
}

#[no_mangle]
pub unsafe extern "C" fn napi_create_string_utf8(
    env: NapiEnv,
    value: *const c_char,
    length: usize,
    out: *mut NapiValue,
) -> NapiStatus {
    if value.is_null() {
        return record_status(env, NAPI_INVALID_ARG);
    }
    let Ok(env) = env_for_value_output(env, out) else {
        return NAPI_INVALID_ARG;
    };
    let bytes = if length == NAPI_AUTO_LENGTH {
        CStr::from_ptr(value).to_bytes()
    } else {
        std::slice::from_raw_parts(value.cast(), length)
    };
    let string = String::from_utf8_lossy(bytes).into_owned();
    let value = env.alloc(Value::String(string));
    write_value(out, value)
}

#[no_mangle]
pub unsafe extern "C" fn napi_create_string_latin1(
    env: NapiEnv,
    value: *const c_char,
    length: usize,
    out: *mut NapiValue,
) -> NapiStatus {
    if value.is_null() {
        return record_status(env, NAPI_INVALID_ARG);
    }
    let Ok(env) = env_for_value_output(env, out) else {
        return NAPI_INVALID_ARG;
    };
    let bytes = if length == NAPI_AUTO_LENGTH {
        CStr::from_ptr(value).to_bytes()
    } else {
        std::slice::from_raw_parts(value.cast::<u8>(), length)
    };
    let string: String = bytes.iter().map(|byte| char::from(*byte)).collect();
    let value = env.alloc(Value::String(string));
    write_value(out, value)
}

#[no_mangle]
pub unsafe extern "C" fn napi_create_string_utf16(
    env: NapiEnv,
    value: *const u16,
    length: usize,
    out: *mut NapiValue,
) -> NapiStatus {
    if value.is_null() {
        return record_status(env, NAPI_INVALID_ARG);
    }
    let Ok(env) = env_for_value_output(env, out) else {
        return NAPI_INVALID_ARG;
    };
    let length = if length == NAPI_AUTO_LENGTH {
        let mut length = 0;
        while *value.add(length) != 0 {
            length += 1;
        }
        length
    } else {
        length
    };
    let string = String::from_utf16_lossy(std::slice::from_raw_parts(value, length));
    let value = env.alloc(Value::String(string));
    write_value(out, value)
}

fn intern_property_key(env: &mut Env, key: String) -> NapiValue {
    if let Some(value) = env.property_keys.get(&key).copied() {
        return value;
    }
    let value = env.alloc(Value::String(key.clone()));
    env.property_keys.insert(key, value);
    value
}

#[no_mangle]
pub unsafe extern "C" fn node_api_create_property_key_utf8(
    env: NapiEnv,
    value: *const c_char,
    length: usize,
    out: *mut NapiValue,
) -> NapiStatus {
    if value.is_null() {
        return record_status(env, NAPI_INVALID_ARG);
    }
    let Ok(env) = env_for_value_output(env, out) else {
        return NAPI_INVALID_ARG;
    };
    let bytes = if length == NAPI_AUTO_LENGTH {
        CStr::from_ptr(value).to_bytes()
    } else {
        std::slice::from_raw_parts(value.cast::<u8>(), length)
    };
    let key = String::from_utf8_lossy(bytes).into_owned();
    write_value(out, intern_property_key(env, key))
}

#[no_mangle]
pub unsafe extern "C" fn node_api_create_property_key_latin1(
    env: NapiEnv,
    value: *const c_char,
    length: usize,
    out: *mut NapiValue,
) -> NapiStatus {
    if value.is_null() {
        return record_status(env, NAPI_INVALID_ARG);
    }
    let Ok(env) = env_for_value_output(env, out) else {
        return NAPI_INVALID_ARG;
    };
    let bytes = if length == NAPI_AUTO_LENGTH {
        CStr::from_ptr(value).to_bytes()
    } else {
        std::slice::from_raw_parts(value.cast::<u8>(), length)
    };
    let key = bytes.iter().map(|byte| char::from(*byte)).collect();
    write_value(out, intern_property_key(env, key))
}

#[no_mangle]
pub unsafe extern "C" fn node_api_create_property_key_utf16(
    env: NapiEnv,
    value: *const u16,
    length: usize,
    out: *mut NapiValue,
) -> NapiStatus {
    if value.is_null() {
        return record_status(env, NAPI_INVALID_ARG);
    }
    let Ok(env) = env_for_value_output(env, out) else {
        return NAPI_INVALID_ARG;
    };
    let length = if length == NAPI_AUTO_LENGTH {
        let mut length = 0;
        while *value.add(length) != 0 {
            length += 1;
        }
        length
    } else {
        length
    };
    let key = String::from_utf16_lossy(std::slice::from_raw_parts(value, length));
    write_value(out, intern_property_key(env, key))
}

#[no_mangle]
pub unsafe extern "C" fn node_api_create_external_string_latin1(
    env: NapiEnv,
    value: *mut c_char,
    length: usize,
    finalize: Option<NapiFinalize>,
    hint: *mut c_void,
    out: *mut NapiValue,
    copied: *mut bool,
) -> NapiStatus {
    if value.is_null() || out.is_null() || copied.is_null() {
        return record_status(env, NAPI_INVALID_ARG);
    }
    let status = napi_create_string_latin1(env, value, length, out);
    if status != NAPI_OK {
        return record_status(env, status);
    }
    *copied = true;
    if let Some(finalize) = finalize {
        finalize(env, value.cast(), hint);
    }
    NAPI_OK
}

#[no_mangle]
pub unsafe extern "C" fn node_api_create_external_string_utf16(
    env: NapiEnv,
    value: *mut u16,
    length: usize,
    finalize: Option<NapiFinalize>,
    hint: *mut c_void,
    out: *mut NapiValue,
    copied: *mut bool,
) -> NapiStatus {
    if value.is_null() || out.is_null() || copied.is_null() {
        return record_status(env, NAPI_INVALID_ARG);
    }
    let status = napi_create_string_utf16(env, value, length, out);
    if status != NAPI_OK {
        return record_status(env, status);
    }
    *copied = true;
    if let Some(finalize) = finalize {
        finalize(env, value.cast(), hint);
    }
    NAPI_OK
}

#[no_mangle]
pub unsafe extern "C" fn napi_create_symbol(
    env: NapiEnv,
    description: NapiValue,
    out: *mut NapiValue,
) -> NapiStatus {
    if out.is_null() || (!description.is_null() && !value_belongs_to_environment(env, description))
    {
        return record_status(env, NAPI_INVALID_ARG);
    }
    let description = if description.is_null() {
        String::new()
    } else {
        match value_ref(description) {
            Ok(Value::String(value)) => value.clone(),
            _ => return record_status(env, NAPI_STRING_EXPECTED),
        }
    };
    let Ok(env) = env_mut(env) else {
        return NAPI_INVALID_ARG;
    };
    let id = NEXT_SYMBOL_ID.fetch_add(1, Ordering::Relaxed);
    let value = env.alloc(Value::Symbol { id, description });
    env.symbols.insert(id, value);
    write_value(out, value)
}

#[no_mangle]
pub unsafe extern "C" fn node_api_symbol_for(
    env: NapiEnv,
    description: *const c_char,
    length: usize,
    out: *mut NapiValue,
) -> NapiStatus {
    if description.is_null() {
        return record_status(env, NAPI_INVALID_ARG);
    }
    let Ok(env) = env_for_value_output(env, out) else {
        return NAPI_INVALID_ARG;
    };
    let bytes = if length == NAPI_AUTO_LENGTH {
        CStr::from_ptr(description).to_bytes()
    } else {
        std::slice::from_raw_parts(description.cast::<u8>(), length)
    };
    let description = String::from_utf8_lossy(bytes).into_owned();
    let id = *GLOBAL_SYMBOLS
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .entry(description.clone())
        .or_insert_with(|| NEXT_SYMBOL_ID.fetch_add(1, Ordering::Relaxed));
    let value = if let Some(value) = env.symbols.get(&id).copied() {
        value
    } else {
        let value = env.alloc(Value::Symbol { id, description });
        env.symbols.insert(id, value);
        value
    };
    write_value(out, value)
}

#[no_mangle]
pub unsafe extern "C" fn napi_create_external(
    env: NapiEnv,
    data: *mut c_void,
    finalize: Option<unsafe extern "C" fn(NapiEnv, *mut c_void, *mut c_void)>,
    hint: *mut c_void,
    out: *mut NapiValue,
) -> NapiStatus {
    if out.is_null() {
        return record_status(env, NAPI_INVALID_ARG);
    }
    let Ok(env) = env_mut(env) else {
        return NAPI_INVALID_ARG;
    };
    let value = env.alloc(Value::External(data));
    if finalize.is_some() {
        env.finalizers.push(FinalizeRecord {
            data,
            finalize,
            hint,
        });
    }
    *out = value;
    NAPI_OK
}

#[no_mangle]
pub unsafe extern "C" fn napi_get_value_external(
    env: NapiEnv,
    value: NapiValue,
    out: *mut *mut c_void,
) -> NapiStatus {
    if out.is_null() || !value_belongs_to_environment(env, value) {
        return record_status(env, NAPI_INVALID_ARG);
    }
    let status = match value_ref(value) {
        Ok(Value::External(data)) => {
            *out = *data;
            NAPI_OK
        }
        _ => NAPI_INVALID_ARG,
    };
    record_status(env, status)
}

#[no_mangle]
pub unsafe extern "C" fn napi_create_object(env: NapiEnv, out: *mut NapiValue) -> NapiStatus {
    let Ok(env) = env_for_value_output(env, out) else {
        return NAPI_INVALID_ARG;
    };
    let value = env.alloc(Value::Object(HashMap::new()));
    write_value(out, value)
}

#[no_mangle]
pub unsafe extern "C" fn node_api_create_object_with_properties(
    env: NapiEnv,
    prototype_or_null: NapiValue,
    property_names: *mut NapiValue,
    property_values: *mut NapiValue,
    property_count: usize,
    out: *mut NapiValue,
) -> NapiStatus {
    if out.is_null() || !value_belongs_to_environment(env, prototype_or_null) {
        return record_status(env, NAPI_INVALID_ARG);
    }
    if !matches!(value_ref(prototype_or_null), Ok(value) if is_object_value(value) || matches!(value, Value::Null))
    {
        return record_status(env, NAPI_OBJECT_EXPECTED);
    }
    if property_count != 0 && (property_names.is_null() || property_values.is_null()) {
        return record_status(env, NAPI_INVALID_ARG);
    }

    let (names, values) = if property_count == 0 {
        (&[][..], &[][..])
    } else {
        (
            std::slice::from_raw_parts(property_names, property_count),
            std::slice::from_raw_parts(property_values, property_count),
        )
    };
    for (&name, &value) in names.iter().zip(values) {
        if !value_belongs_to_environment(env, name)
            || !value_belongs_to_environment(env, value)
            || property_key(name).is_err()
        {
            return record_status(env, NAPI_INVALID_ARG);
        }
    }

    let mut object = ptr::null_mut();
    let status = napi_create_object(env, &mut object);
    if status != NAPI_OK {
        return record_status(env, status);
    }
    let status = node_api_set_prototype(env, object, prototype_or_null);
    if status != NAPI_OK {
        return record_status(env, status);
    }
    for (&name, &value) in names.iter().zip(values) {
        let status = napi_set_property(env, object, name, value);
        if status != NAPI_OK {
            return record_status(env, status);
        }
    }
    write_value(out, object)
}
#[no_mangle]
pub unsafe extern "C" fn napi_create_array(env: NapiEnv, out: *mut NapiValue) -> NapiStatus {
    napi_create_array_with_length(env, 0, out)
}
#[no_mangle]
pub unsafe extern "C" fn napi_create_array_with_length(
    env: NapiEnv,
    length: usize,
    out: *mut NapiValue,
) -> NapiStatus {
    let Ok(env) = env_for_value_output(env, out) else {
        return NAPI_INVALID_ARG;
    };
    let value = env.alloc(Value::Array(vec![None; length]));
    write_value(out, value)
}

#[no_mangle]
pub unsafe extern "C" fn napi_create_function(
    env: NapiEnv,
    name: *const c_char,
    length: usize,
    callback: Option<NapiCallback>,
    data: *mut c_void,
    out: *mut NapiValue,
) -> NapiStatus {
    let Some(callback) = callback else {
        return record_status(env, NAPI_INVALID_ARG);
    };
    if out.is_null() {
        return record_status(env, NAPI_INVALID_ARG);
    }
    let Ok(env) = env_for_value_output(env, out) else {
        return NAPI_INVALID_ARG;
    };
    let function_name = if name.is_null() {
        String::new()
    } else {
        let bytes = if length == NAPI_AUTO_LENGTH {
            CStr::from_ptr(name).to_bytes()
        } else {
            std::slice::from_raw_parts(name.cast::<u8>(), length)
        };
        String::from_utf8_lossy(bytes).into_owned()
    };
    let value = env.alloc(Value::Function(Function {
        callback,
        data,
        properties: HashMap::new(),
        _thaw_bridge: None,
    }));
    let name_value = env.alloc(Value::String(function_name));
    let length_value = env.alloc(Value::Number(0.0));
    let Some(Value::Function(function)) = value.as_mut() else {
        unreachable!();
    };
    function
        .properties
        .insert(PropertyKey::String("name".into()), name_value);
    function
        .properties
        .insert(PropertyKey::String("length".into()), length_value);
    for key in [
        PropertyKey::String("name".into()),
        PropertyKey::String("length".into()),
    ] {
        record_property_order(env, value as usize, &key);
        env.property_attributes
            .insert((value as usize, key), NAPI_CONFIGURABLE);
    }
    write_value(out, value)
}

#[no_mangle]
pub unsafe extern "C" fn napi_get_cb_info(
    env: NapiEnv,
    info: NapiCallbackInfo,
    argc: *mut usize,
    argv: *mut NapiValue,
    this_arg: *mut NapiValue,
    data: *mut *mut c_void,
) -> NapiStatus {
    if env.is_null() {
        return NAPI_INVALID_ARG;
    }
    let Some(info) = info.as_ref() else {
        return record_status(env, NAPI_INVALID_ARG);
    };
    if !argc.is_null() {
        let capacity = *argc;
        *argc = info.args.len();
        if !argv.is_null() {
            let copied = capacity.min(info.args.len());
            ptr::copy_nonoverlapping(info.args.as_ptr(), argv, copied);
            if copied < capacity {
                let mut undefined = ptr::null_mut();
                let status = napi_get_undefined(env, &mut undefined);
                if status != NAPI_OK {
                    return record_status(env, status);
                }
                for index in copied..capacity {
                    *argv.add(index) = undefined;
                }
            }
        }
    }
    if !this_arg.is_null() {
        *this_arg = info.this_arg;
    }
    if !data.is_null() {
        *data = info.data;
    }
    NAPI_OK
}

#[no_mangle]
pub unsafe extern "C" fn napi_get_new_target(
    env: NapiEnv,
    info: NapiCallbackInfo,
    result: *mut NapiValue,
) -> NapiStatus {
    if env.is_null() || result.is_null() {
        return record_status(env, NAPI_INVALID_ARG);
    }
    let Some(info) = info.as_ref() else {
        return record_status(env, NAPI_INVALID_ARG);
    };
    *result = info.new_target;
    NAPI_OK
}

