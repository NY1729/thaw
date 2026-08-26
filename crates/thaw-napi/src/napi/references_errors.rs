#[no_mangle]
pub unsafe extern "C" fn napi_create_reference(
    env: NapiEnv,
    value: NapiValue,
    initial_count: u32,
    out: *mut *mut Reference,
) -> NapiStatus {
    let Ok(env_ref) = env_mut(env) else {
        return NAPI_INVALID_ARG;
    };
    if value.is_null() || out.is_null() || !value_belongs_to_environment(env, value) {
        return record_status(env, NAPI_INVALID_ARG);
    }
    *out = alloc_reference(env_ref, value, initial_count);
    NAPI_OK
}

#[no_mangle]
pub unsafe extern "C" fn napi_delete_reference(
    env: NapiEnv,
    reference: *mut Reference,
) -> NapiStatus {
    let Ok(reference) = reference_mut(env, reference) else {
        return record_status(env, NAPI_INVALID_ARG);
    };
    reference.deleted = true;
    NAPI_OK
}

#[no_mangle]
pub unsafe extern "C" fn napi_get_reference_value(
    env: NapiEnv,
    reference: *mut Reference,
    out: *mut NapiValue,
) -> NapiStatus {
    let Ok(reference) = reference_mut(env, reference) else {
        return record_status(env, NAPI_INVALID_ARG);
    };
    let _ = reference.count;
    let status = write_value(out, reference.value);
    record_status(env, status)
}

#[no_mangle]
pub unsafe extern "C" fn napi_reference_ref(
    env: NapiEnv,
    reference: *mut Reference,
    result: *mut u32,
) -> NapiStatus {
    let Ok(reference) = reference_mut(env, reference) else {
        return record_status(env, NAPI_INVALID_ARG);
    };
    let Some(count) = reference.count.checked_add(1) else {
        return record_status(env, NAPI_GENERIC_FAILURE);
    };
    reference.count = count;
    if !result.is_null() {
        *result = reference.count;
    }
    NAPI_OK
}

#[no_mangle]
pub unsafe extern "C" fn napi_reference_unref(
    env: NapiEnv,
    reference: *mut Reference,
    result: *mut u32,
) -> NapiStatus {
    let Ok(reference) = reference_mut(env, reference) else {
        return record_status(env, NAPI_INVALID_ARG);
    };
    if reference.count == 0 {
        return record_status(env, NAPI_GENERIC_FAILURE);
    }
    reference.count -= 1;
    if !result.is_null() {
        *result = reference.count;
    }
    NAPI_OK
}

#[no_mangle]
pub unsafe extern "C" fn napi_call_function(
    env: NapiEnv,
    this_arg: NapiValue,
    function: NapiValue,
    argc: usize,
    argv: *const NapiValue,
    out: *mut NapiValue,
) -> NapiStatus {
    let Ok(env_ref) = env_mut(env) else {
        return NAPI_INVALID_ARG;
    };
    if env_ref.exception.is_some() {
        return record_status(env, NAPI_PENDING_EXCEPTION);
    }
    if this_arg.is_null()
        || !value_belongs_to_environment(env, this_arg)
        || !value_belongs_to_environment(env, function)
        || (argc != 0 && argv.is_null())
    {
        return record_status(env, NAPI_INVALID_ARG);
    }
    if argc != 0
        && std::slice::from_raw_parts(argv, argc)
            .iter()
            .any(|value| !value_belongs_to_environment(env, *value))
    {
        return record_status(env, NAPI_INVALID_ARG);
    }
    let function = match value_ref(function) {
        Ok(Value::Function(function)) => function.clone(),
        _ => return record_status(env, NAPI_FUNCTION_EXPECTED),
    };
    let args = if argc == 0 {
        Vec::new()
    } else {
        std::slice::from_raw_parts(argv, argc).to_vec()
    };
    let mut info = CallbackInfo {
        args,
        this_arg,
        new_target: ptr::null_mut(),
        data: function.data,
    };
    let result = (function.callback)(env, &mut info);
    if env_mut(env)
        .map(|env| env.exception.is_some())
        .unwrap_or(false)
    {
        return record_status(env, NAPI_PENDING_EXCEPTION);
    }
    if !result.is_null() && !value_belongs_to_environment(env, result) {
        return NAPI_INVALID_ARG;
    }
    let status = if out.is_null() {
        NAPI_OK
    } else if result.is_null() {
        napi_get_undefined(env, out)
    } else {
        write_value(out, result)
    };
    record_status(env, status)
}

#[no_mangle]
pub unsafe extern "C" fn napi_make_callback(
    env: NapiEnv,
    async_context: *mut c_void,
    this_arg: NapiValue,
    function: NapiValue,
    argc: usize,
    argv: *const NapiValue,
    result: *mut NapiValue,
) -> NapiStatus {
    if !async_context.is_null() && async_context_mut(env, async_context).is_err() {
        return record_status(env, NAPI_INVALID_ARG);
    }
    let status = napi_call_function(env, this_arg, function, argc, argv, result);
    record_status(env, status)
}

const NAPI_PENDING_EXCEPTION: NapiStatus = 10;

fn alloc_error(env: &mut Env, name: &str, message: String, code: Option<NapiValue>) -> NapiValue {
    let value = env.alloc(Value::Error(message.clone()));
    let message_value = env.alloc(Value::String(message));
    let owner = value as usize;
    env.error_names.insert(owner, name.into());
    let properties = env.host_properties.entry(owner).or_default();
    properties.insert(PropertyKey::String("message".into()), message_value);
    if let Some(code) = code {
        properties.insert(PropertyKey::String("code".into()), code);
    }
    let message_key = PropertyKey::String("message".into());
    record_property_order(env, owner, &message_key);
    env.property_attributes
        .insert((owner, message_key), NAPI_WRITABLE | NAPI_CONFIGURABLE);
    if code.is_some() {
        let code_key = PropertyKey::String("code".into());
        record_property_order(env, owner, &code_key);
        env.property_attributes
            .insert((owner, code_key), NAPI_DEFAULT_PROPERTY_ATTRIBUTES);
    }
    value
}

unsafe fn create_error_kind(
    env: NapiEnv,
    name: &str,
    code: NapiValue,
    message: NapiValue,
    out: *mut NapiValue,
) -> NapiStatus {
    if out.is_null()
        || !value_belongs_to_environment(env, message)
        || (!code.is_null() && !value_belongs_to_environment(env, code))
    {
        return record_status(env, NAPI_INVALID_ARG);
    }
    let message = match value_ref(message) {
        Ok(Value::String(message)) => message.clone(),
        _ => return record_status(env, NAPI_STRING_EXPECTED),
    };
    if !code.is_null() && !matches!(value_ref(code), Ok(Value::String(_))) {
        return record_status(env, NAPI_STRING_EXPECTED);
    }
    let Ok(env) = env_mut(env) else {
        return NAPI_INVALID_ARG;
    };
    write_value(
        out,
        alloc_error(env, name, message, (!code.is_null()).then_some(code)),
    )
}

#[no_mangle]
pub unsafe extern "C" fn napi_create_error(
    env: NapiEnv,
    code: NapiValue,
    message: NapiValue,
    out: *mut NapiValue,
) -> NapiStatus {
    create_error_kind(env, "Error", code, message, out)
}

#[no_mangle]
pub unsafe extern "C" fn napi_create_type_error(
    env: NapiEnv,
    code: NapiValue,
    message: NapiValue,
    out: *mut NapiValue,
) -> NapiStatus {
    create_error_kind(env, "TypeError", code, message, out)
}

#[no_mangle]
pub unsafe extern "C" fn napi_create_range_error(
    env: NapiEnv,
    code: NapiValue,
    message: NapiValue,
    out: *mut NapiValue,
) -> NapiStatus {
    create_error_kind(env, "RangeError", code, message, out)
}

#[no_mangle]
pub unsafe extern "C" fn node_api_create_syntax_error(
    env: NapiEnv,
    code: NapiValue,
    message: NapiValue,
    out: *mut NapiValue,
) -> NapiStatus {
    create_error_kind(env, "SyntaxError", code, message, out)
}

#[no_mangle]
pub unsafe extern "C" fn napi_is_error(
    env: NapiEnv,
    value: NapiValue,
    out: *mut bool,
) -> NapiStatus {
    if out.is_null() || !value_belongs_to_environment(env, value) {
        return record_status(env, NAPI_INVALID_ARG);
    }
    *out = matches!(value_ref(value), Ok(Value::Error(_)));
    NAPI_OK
}

#[no_mangle]
pub unsafe extern "C" fn napi_throw(env: NapiEnv, error: NapiValue) -> NapiStatus {
    if !value_belongs_to_environment(env, error) {
        return NAPI_INVALID_ARG;
    }
    let Ok(env) = env_mut(env) else {
        return NAPI_INVALID_ARG;
    };
    env.exception = Some(error);
    NAPI_OK
}

#[no_mangle]
pub unsafe extern "C" fn napi_get_last_error_info(
    env: NapiEnv,
    out: *mut *const NapiExtendedErrorInfo,
) -> NapiStatus {
    let Some(env) = env.as_mut() else {
        return NAPI_INVALID_ARG;
    };
    if out.is_null() {
        env.last_error_info.error_code = NAPI_INVALID_ARG;
        env.last_error_info.error_message = status_message(NAPI_INVALID_ARG);
        return NAPI_INVALID_ARG;
    }
    *out = &env.last_error_info;
    NAPI_OK
}

#[no_mangle]
pub unsafe extern "C" fn napi_fatal_error(
    _location: *const c_char,
    _location_len: usize,
    message: *const c_char,
    message_len: usize,
) -> ! {
    let message = if message.is_null() {
        "N-API fatal error".into()
    } else if message_len == NAPI_AUTO_LENGTH {
        CStr::from_ptr(message).to_string_lossy().into_owned()
    } else {
        String::from_utf8_lossy(std::slice::from_raw_parts(message.cast(), message_len))
            .into_owned()
    };
    eprintln!("thaw-napi: {message}");
    std::process::abort()
}

#[no_mangle]
pub unsafe extern "C" fn napi_fatal_exception(env: NapiEnv, error: NapiValue) -> NapiStatus {
    if !value_belongs_to_environment(env, error) {
        return NAPI_INVALID_ARG;
    }
    let Ok(env) = env_mut(env) else {
        return NAPI_INVALID_ARG;
    };
    let message = match value_ref(error) {
        Ok(Value::Error(message) | Value::String(message)) => message.clone(),
        _ => "unhandled N-API callback exception".into(),
    };
    eprintln!("thaw-napi: fatal exception: {message}");
    env.exception = Some(error);
    FATAL_EXCEPTION_PENDING.store(true, Ordering::Release);
    NAPI_OK
}

#[no_mangle]
pub extern "C" fn thaw_napi_take_fatal_exception() -> u8 {
    u8::from(FATAL_EXCEPTION_PENDING.swap(false, Ordering::AcqRel))
}

#[repr(C)]
pub struct NapiNodeVersion {
    major: u32,
    minor: u32,
    patch: u32,
    release: *const c_char,
}
static NODE_RELEASE: &[u8] = b"thaw\0";
static NODE_VERSION: NapiNodeVersion = NapiNodeVersion {
    major: 20,
    minor: 0,
    patch: 0,
    release: NODE_RELEASE.as_ptr().cast(),
};
unsafe impl Sync for NapiNodeVersion {}
#[no_mangle]
pub unsafe extern "C" fn napi_get_node_version(
    env: NapiEnv,
    out: *mut *const NapiNodeVersion,
) -> NapiStatus {
    if env.is_null() || out.is_null() {
        NAPI_INVALID_ARG
    } else {
        *out = &NODE_VERSION;
        NAPI_OK
    }
}

#[no_mangle]
pub unsafe extern "C" fn node_api_get_module_file_name(
    env: NapiEnv,
    result: *mut *const c_char,
) -> NapiStatus {
    let Some(env_ref) = env.as_ref() else {
        return NAPI_INVALID_ARG;
    };
    let Some(result) = result.as_mut() else {
        return record_status(env, NAPI_INVALID_ARG);
    };
    *result = env_ref.module_file_name.as_ptr();
    NAPI_OK
}

