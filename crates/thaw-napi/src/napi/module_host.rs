#[no_mangle]
pub unsafe extern "C" fn napi_module_register(module: *mut NapiModule) {
    if let Some(module) = module.as_ref() {
        PENDING_MODULE.with(|slot| *slot.borrow_mut() = Some(*module));
    }
}

type RegisterV1 = unsafe extern "C" fn(NapiEnv, NapiValue) -> NapiValue;

unsafe fn load_impl(path: &str, root_name: Option<&str>) -> Result<(), String> {
    let path_text = path;
    let path = CString::new(path).map_err(|_| "addon path contains NUL".to_string())?;
    PENDING_MODULE.with(|slot| slot.borrow_mut().take());
    // Node exports libuv from its executable. Some otherwise portable N-API
    // addons (notably serialport) call that API directly, so expose the system
    // libuv with global symbol visibility before resolving the addon.
    #[cfg(target_os = "linux")]
    let uv_handle = libc::dlopen(c"libuv.so.1".as_ptr(), libc::RTLD_NOW | libc::RTLD_GLOBAL);
    libc::dlerror();
    let handle = libc::dlopen(path.as_ptr(), libc::RTLD_NOW | libc::RTLD_LOCAL);
    if handle.is_null() {
        let error = libc::dlerror();
        return Err(if error.is_null() {
            "dlopen failed".into()
        } else {
            CStr::from_ptr(error).to_string_lossy().into_owned()
        });
    }
    let direct = libc::dlsym(handle, c"napi_register_module_v1".as_ptr());
    let registered = PENDING_MODULE.with(|slot| slot.borrow_mut().take());
    let init: RegisterV1 = if !direct.is_null() {
        std::mem::transmute::<*mut c_void, RegisterV1>(direct)
    } else if let Some(module) = registered {
        module
            .nm_register_func
            .ok_or("registered N-API module has no init function")?
    } else {
        libc::dlclose(handle);
        return Err(
            "addon exports neither napi_register_module_v1 nor a registered N-API module".into(),
        );
    };

    let mut env = Box::new(Env::new());
    let absolute_path =
        std::fs::canonicalize(path_text).unwrap_or_else(|_| std::path::PathBuf::from(path_text));
    env.module_file_name = CString::new(format!("file://{}", absolute_path.to_string_lossy()))
        .unwrap_or_else(|_| CString::new("").unwrap());
    let env_ptr = &mut *env as NapiEnv;
    let exports = env.alloc(Value::Object(HashMap::new()));
    let returned = init(env_ptr, exports);
    let exports = if returned.is_null() {
        exports
    } else {
        returned
    };
    if let Some(exception) = env.exception {
        let message = json_from_value(exception)?.to_string();
        libc::dlclose(handle);
        return Err(format!("addon initialization threw: {message}"));
    }
    let mut functions = Vec::new();
    let mut exported_values = Vec::new();
    match value_ref(exports).map_err(|_| "invalid exports value")? {
        Value::Object(object) => {
            for (name, value) in object {
                if let Value::Function(function) =
                    value_ref(*value).map_err(|_| "invalid export value")?
                {
                    if let PropertyKey::String(name) = name {
                        functions.push((name.clone(), function.clone()));
                        exported_values.push((name.clone(), *value));
                    }
                }
            }
        }
        Value::Function(function) => {
            let name = root_name.unwrap_or("default").to_string();
            functions.push((name.clone(), function.clone()));
            exported_values.push((name, exports));
        }
        _ => return Err("addon initialization returned neither an object nor a function".into()),
    }
    HOST.with(|host| {
        let mut host = host.borrow_mut();
        host.functions.extend(functions);
        host.exports.extend(
            exported_values
                .into_iter()
                .map(|(name, value)| (name, (env_ptr as usize, value))),
        );
        #[cfg(target_os = "linux")]
        if !uv_handle.is_null() {
            host.libraries.push(uv_handle);
        }
        host.libraries.push(handle);
        host.module_envs.push(env);
    });
    #[cfg(feature = "quickjs")]
    thaw_quickjs::register_napi_bridge(
        thaw_napi_export_names,
        thaw_napi_call,
        thaw_napi_handle_bridge,
        thaw_napi_poll_async_work,
    );
    Ok(())
}

#[cfg(feature = "quickjs")]
unsafe extern "C" fn thaw_napi_export_names() -> *const c_char {
    let names = HOST.with(|host| host.borrow().functions.keys().cloned().collect::<Vec<_>>());
    CString::new(serde_json::to_string(&names).unwrap_or_else(|_| "[]".into()))
        .unwrap_or_default()
        .into_raw()
}

#[cfg(feature = "quickjs")]
unsafe extern "C" fn thaw_napi_handle_bridge(
    operation: *const c_char,
    target: *const c_char,
    name: *const c_char,
    args: *const c_char,
) -> *const c_char {
    let result = (|| -> Result<serde_json::Value, String> {
        let operation = text(operation)?;
        let target = text(target)?;
        let name = text(name)?;
        if operation == "release" {
            let reference = target
                .parse::<u64>()
                .map_err(|_| "invalid QuickJS reference")?;
            release_quickjs_reference(reference);
            return Ok(serde_json::json!({ "kind": "value", "value": true }));
        }
        if operation == "release_handle" {
            let handle = target
                .parse::<u64>()
                .map_err(|_| "invalid native addon handle")?;
            release_napi_handle(handle)?;
            return Ok(serde_json::json!({ "kind": "value", "value": true }));
        }
        let handle = if operation == "construct" {
            let target = CString::new(target).map_err(|_| "export contains NUL")?;
            thaw_napi_get_export(target.as_ptr())
        } else {
            target
                .parse::<u64>()
                .map_err(|_| "invalid native addon handle")?
        };
        if handle == 0 {
            return Err("unknown native addon export".into());
        }
        match operation.as_str() {
            "construct" => {
                let value = construct_handle_impl(handle, args, true);
                if value.error.is_null() {
                    Ok(serde_json::json!({ "kind": "handle", "value": value.value.to_string() }))
                } else {
                    Err(CStr::from_ptr(value.error).to_string_lossy().into_owned())
                }
            }
            "get" => {
                let env = module_env_for_handle(handle)?;
                let property = CString::new(name).map_err(|_| "property contains NUL")?;
                let mut value = ptr::null_mut();
                let status = napi_get_named_property(
                    env,
                    handle as NapiValue,
                    property.as_ptr(),
                    &mut value,
                );
                take_env_exception(env)?;
                if status != NAPI_OK || value.is_null() {
                    return Err(format!("failed to get native property: status {status}"));
                }
                if matches!(value_ref(value), Ok(Value::Function(_))) {
                    Ok(serde_json::json!({ "kind": "method" }))
                } else {
                    Ok(serde_json::json!({
                        "kind": "value",
                        "value": json_from_value_with_undefined(wait_for_promise(value)?, true)?
                    }))
                }
            }
            "call" => {
                let name = CString::new(name).map_err(|_| "method contains NUL")?;
                let result = call_method_impl(handle, name.as_ptr(), args, true);
                if result.error.is_null() {
                    let value = CStr::from_ptr(result.value).to_string_lossy();
                    Ok(serde_json::json!({
                        "kind": "value",
                        "value": serde_json::from_str::<serde_json::Value>(&value)
                            .map_err(|error| error.to_string())?
                    }))
                } else {
                    Err(CStr::from_ptr(result.error).to_string_lossy().into_owned())
                }
            }
            "set" => {
                let name = CString::new(name).map_err(|_| "property contains NUL")?;
                let result = set_property_impl(handle, name.as_ptr(), args, true, true);
                if result.error.is_null() {
                    Ok(serde_json::json!({ "kind": "value", "value": true }))
                } else {
                    Err(CStr::from_ptr(result.error).to_string_lossy().into_owned())
                }
            }
            _ => Err(format!("unknown native addon handle operation `{operation}`")),
        }
    })();
    let value = match result {
        Ok(value) => value,
        Err(error) => serde_json::json!({ "__thaw_error__": error }),
    };
    CString::new(value.to_string())
        .unwrap_or_default()
        .into_raw()
}

#[cfg(feature = "quickjs")]
fn release_quickjs_reference(reference: u64) {
    let released = HOST.with(|host| {
        let mut host = host.borrow_mut();
        host.module_envs
            .iter_mut()
            .filter_map(|env| {
                let value = env.quickjs_references.remove(&reference)?;
                for candidate in &mut env.references {
                    if candidate.count == 0 && candidate.value == value {
                        candidate.value = ptr::null_mut();
                    }
                }
                let finalizers = env
                    .object_finalizers
                    .remove(&(value as usize))
                    .unwrap_or_default();
                Some((&mut **env as NapiEnv, finalizers))
            })
            .collect::<Vec<_>>()
    });
    for (env, finalizers) in released {
        for record in finalizers {
            if let Some(finalize) = record.finalize {
                unsafe { finalize(env, record.data, record.hint) };
            }
        }
    }
}

#[cfg(feature = "quickjs")]
fn release_napi_handle(handle: u64) -> Result<(), String> {
    let released = HOST.with(|host| {
        let mut host = host.borrow_mut();
        let env = host
            .module_envs
            .iter_mut()
            .find(|env| env.values.contains(&(handle as NapiValue)))
            .ok_or_else(|| "unknown native addon handle".to_string())?;
        let value = handle as NapiValue;
        if env
            .references
            .iter()
            .any(|reference| !reference.deleted && reference.value == value && reference.count > 0)
        {
            env.released_handles.insert(handle as usize);
            return Ok::<_, String>(None);
        }
        for reference in &mut env.references {
            if reference.count == 0 && reference.value == value {
                reference.value = ptr::null_mut();
            }
        }
        let wrap = env.wraps.remove(&(value as usize));
        let finalizers = env
            .object_finalizers
            .remove(&(value as usize))
            .unwrap_or_default();
        Ok::<_, String>(Some((&mut **env as NapiEnv, wrap, finalizers)))
    })?;
    let Some((env, wrap, finalizers)) = released else {
        return Ok(());
    };
    if let Some(wrap) = wrap {
        if let Some(finalize) = wrap.finalize {
            unsafe { finalize(env, wrap.data, wrap.hint) };
        }
    }
    for record in finalizers {
        if let Some(finalize) = record.finalize {
            unsafe { finalize(env, record.data, record.hint) };
        }
    }
    Ok(())
}

#[no_mangle]
pub unsafe extern "C" fn thaw_napi_load(path: *const c_char) -> u8 {
    match text(path).and_then(|path| load_impl(&path, None)) {
        Ok(()) => 1,
        Err(error) => {
            HOST.with(|host| host.borrow_mut().last_error = error.clone());
            eprintln!("thaw-napi: {error}");
            0
        }
    }
}

#[no_mangle]
pub unsafe extern "C" fn thaw_napi_load_named(path: *const c_char, root_name: *const c_char) -> u8 {
    let result = text(path)
        .and_then(|path| text(root_name).and_then(|root_name| load_impl(&path, Some(&root_name))));
    match result {
        Ok(()) => 1,
        Err(error) => {
            HOST.with(|host| host.borrow_mut().last_error = error.clone());
            eprintln!("thaw-napi: {error}");
            0
        }
    }
}

#[no_mangle]
pub extern "C" fn thaw_napi_unload_all() -> u8 {
    if ACTIVE_ASYNC_WORK.load(Ordering::Acquire) != 0
        || LIVE_THREADSAFE_FUNCTIONS.load(Ordering::Acquire) != 0
        || ACTIVE_ASYNC_CLEANUP_HOOKS.load(Ordering::Acquire) != 0
    {
        HOST.with(|host| {
            host.borrow_mut().last_error =
                "cannot unload N-API addons while asynchronous work or cleanup is active".into();
        });
        return 0;
    }
    HOST.with(|host| {
        let mut host = host.borrow_mut();
        host.functions.clear();
        host.exports.clear();
        host.compiled_callbacks.clear();
        host.pending_call_envs.clear();
        // Cleanup hooks and native finalizers must run while their addon code
        // is still mapped.
        host.module_envs.clear();
        if ACTIVE_ASYNC_CLEANUP_HOOKS.load(Ordering::Acquire) != 0 {
            host.last_error =
                "cannot unload N-API addons while asynchronous cleanup is active".into();
            return 0;
        }
        for handle in host.libraries.drain(..).rev() {
            unsafe {
                libc::dlclose(handle);
            }
        }
        1
    })
}

fn decode_hex(input: &str) -> Result<Vec<u8>, String> {
    if !input.len().is_multiple_of(2) {
        return Err("embedded addon hex has an odd length".into());
    }
    input
        .as_bytes()
        .chunks_exact(2)
        .map(|pair| {
            let text = std::str::from_utf8(pair).map_err(|error| error.to_string())?;
            u8::from_str_radix(text, 16).map_err(|error| error.to_string())
        })
        .collect()
}

#[cfg(target_os = "linux")]
fn load_embedded_impl(bytes: &[u8], root_name: Option<&str>) -> Result<(), String> {
    let name = CString::new("thaw-native-addon").unwrap();
    let fd = unsafe { libc::memfd_create(name.as_ptr(), libc::MFD_CLOEXEC) };
    if fd < 0 {
        return Err(format!(
            "memfd_create failed: {}",
            std::io::Error::last_os_error()
        ));
    }
    let mut file = unsafe { std::fs::File::from_raw_fd(fd) };
    file.write_all(bytes)
        .map_err(|error| format!("failed to write embedded addon: {error}"))?;
    unsafe { load_impl(&format!("/proc/self/fd/{fd}"), root_name) }
}

#[cfg(not(target_os = "linux"))]
fn load_embedded_impl(bytes: &[u8], root_name: Option<&str>) -> Result<(), String> {
    let mut file = tempfile::Builder::new()
        .prefix("thaw-native-addon-")
        .suffix(".node")
        .tempfile()
        .map_err(|error| format!("failed to create embedded addon file: {error}"))?;
    file.write_all(bytes)
        .map_err(|error| format!("failed to write embedded addon: {error}"))?;
    file.flush()
        .map_err(|error| format!("failed to flush embedded addon: {error}"))?;
    unsafe { load_impl(&file.path().to_string_lossy(), root_name) }
}

#[no_mangle]
pub unsafe extern "C" fn thaw_napi_load_embedded_hex(
    hex: *const c_char,
    root_name: *const c_char,
) -> u8 {
    let result = text(hex).and_then(|hex| {
        let bytes = decode_hex(&hex)?;
        let root_name = text(root_name)?;
        let root_name = (!root_name.is_empty()).then_some(root_name.as_str());
        load_embedded_impl(&bytes, root_name)
    });
    match result {
        Ok(()) => 1,
        Err(error) => {
            HOST.with(|host| host.borrow_mut().last_error = error.clone());
            eprintln!("thaw-napi: {error}");
            0
        }
    }
}

const TYPED_UNDEFINED_KEY: &str = "$__thaw_napi_undefined$";

fn value_from_json_with_undefined(
    env: &mut Env,
    json: &JsonValue,
    preserve_undefined: bool,
) -> NapiValue {
    match json {
        JsonValue::Null => env.alloc(Value::Null),
        JsonValue::Bool(value) => env.alloc(Value::Bool(*value)),
        JsonValue::Number(value) => env.alloc(Value::Number(value.as_f64().unwrap_or(0.0))),
        JsonValue::String(value) => env.alloc(Value::String(value.clone())),
        JsonValue::Array(values) => {
            let values = values
                .iter()
                .map(|value| {
                    Some(value_from_json_with_undefined(
                        env,
                        value,
                        preserve_undefined,
                    ))
                })
                .collect();
            env.alloc(Value::Array(values))
        }
        JsonValue::Object(values) => {
            if preserve_undefined
                && values.len() == 1
                && values.get(TYPED_UNDEFINED_KEY).and_then(JsonValue::as_bool) == Some(true)
            {
                return env.alloc(Value::Undefined);
            }
            if values.get("type").and_then(JsonValue::as_str) == Some("Buffer") {
                if let Some(bytes) = values.get("data").and_then(JsonValue::as_array) {
                    return env.alloc(Value::Buffer(
                        bytes
                            .iter()
                            .map(|value| value.as_u64().unwrap_or(0) as u8)
                            .collect(),
                    ));
                }
            }
            if let Some(handle) = values
                .get("__thaw_napi_handle__")
                .and_then(JsonValue::as_str)
                .and_then(|handle| handle.parse::<u64>().ok())
            {
                let value = handle as NapiValue;
                if env.values.contains(&value) {
                    return value;
                }
                return env.alloc(Value::Undefined);
            }
            if let Some(reference) = values
                .get("__thaw_napi_function__")
                .and_then(JsonValue::as_u64)
            {
                #[cfg(not(feature = "quickjs"))]
                {
                    let _ = reference;
                    return env.alloc(Value::Undefined);
                }
                #[cfg(feature = "quickjs")]
                {
                if let Some(value) = env.quickjs_references.get(&reference) {
                    return *value;
                }
                let bridge = Arc::new(ThawCallbackBridge {
                    callback: ThawCallback::QuickJs(thaw_quickjs::thaw_js_call_reference),
                    context: reference as usize,
                });
                let function = env.alloc(Value::Function(Function {
                    callback: thaw_compiled_callback,
                    data: Arc::as_ptr(&bridge) as *mut c_void,
                    properties: HashMap::new(),
                    _thaw_bridge: Some(bridge),
                }));
                env.quickjs_references.insert(reference, function);
                return function;
                }
            }
            if let Some(reference) = values
                .get("__thaw_napi_object__")
                .and_then(JsonValue::as_u64)
            {
                if let Some(value) = env.quickjs_references.get(&reference) {
                    return *value;
                }
                let object = env.alloc(Value::Object(HashMap::new()));
                env.quickjs_references.insert(reference, object);
                let properties = values
                    .get("value")
                    .and_then(JsonValue::as_object)
                    .map(|properties| {
                        properties
                            .iter()
                            .map(|(key, value)| {
                                (
                                    key.clone().into(),
                                    value_from_json_with_undefined(
                                        env,
                                        value,
                                        preserve_undefined,
                                    ),
                                )
                            })
                            .collect::<HashMap<_, _>>()
                    })
                    .unwrap_or_default();
                if let Some(Value::Object(target)) = unsafe { object.as_mut() } {
                    *target = properties;
                }
                return object;
            }
            let values = values
                .iter()
                .map(|(key, value)| {
                    (
                        key.clone().into(),
                        value_from_json_with_undefined(env, value, preserve_undefined),
                    )
                })
                .collect();
            env.alloc(Value::Object(values))
        }
    }
}

fn value_from_json(env: &mut Env, json: &JsonValue) -> NapiValue {
    value_from_json_with_undefined(env, json, false)
}

unsafe fn json_from_value_with_undefined(
    value: NapiValue,
    preserve_undefined: bool,
) -> Result<JsonValue, String> {
    Ok(match value_ref(value).map_err(|_| "invalid napi_value")? {
        Value::Undefined if preserve_undefined => {
            serde_json::json!({ (TYPED_UNDEFINED_KEY): true })
        }
        Value::Undefined => JsonValue::Null,
        Value::Null => JsonValue::Null,
        Value::Bool(value) => JsonValue::Bool(*value),
        Value::Number(value) | Value::Date(value) => serde_json::Number::from_f64(*value)
            .map(JsonValue::Number)
            .unwrap_or(JsonValue::Null),
        Value::String(value) | Value::Error(value) => JsonValue::String(value.clone()),
        Value::Symbol { .. } => return Err("cannot JSON-encode a Symbol".into()),
        Value::Array(values) => JsonValue::Array(
            values
                .iter()
                .map(|value| match value {
                    Some(value) => json_from_value_with_undefined(*value, preserve_undefined),
                    None => Ok(JsonValue::Null),
                })
                .collect::<Result<_, _>>()?,
        ),
        Value::Object(_) if is_native_instance(value as usize) => {
            serde_json::json!({ "__thaw_napi_handle__": (value as u64).to_string() })
        }
        Value::Object(values) => JsonValue::Object(
            values
                .iter()
                .filter_map(|(key, value)| match key {
                    PropertyKey::String(key) => {
                        Some(
                            json_from_value_with_undefined(*value, preserve_undefined)
                                .map(|value| (key.clone(), value)),
                        )
                    }
                    PropertyKey::Symbol(_) => None,
                })
                .collect::<Result<_, String>>()?,
        ),
        Value::Buffer(values) => {
            JsonValue::Array(values.iter().map(|value| JsonValue::from(*value)).collect())
        }
        Value::ExternalBuffer { data, length } => {
            let bytes = if *length == 0 {
                &[]
            } else {
                std::slice::from_raw_parts(*data, *length)
            };
            JsonValue::Array(bytes.iter().map(|value| JsonValue::from(*value)).collect())
        }
        Value::BufferView {
            array_buffer,
            byte_offset,
            length,
        } => {
            let (data, _, detached) = arraybuffer_parts(*array_buffer)
                .map_err(|_| "invalid Buffer backing ArrayBuffer")?;
            let bytes = if detached || *length == 0 {
                &[]
            } else {
                std::slice::from_raw_parts(data.add(*byte_offset), *length)
            };
            JsonValue::Array(bytes.iter().map(|value| JsonValue::from(*value)).collect())
        }
        Value::ArrayBuffer { .. }
        | Value::SharedArrayBuffer(_)
        | Value::ExternalSharedArrayBuffer { .. }
        | Value::ExternalArrayBuffer { .. }
        | Value::TypedArray { .. }
        | Value::DataView { .. } => {
            return Err("cannot JSON-encode an ArrayBuffer view".into());
        }
        Value::Function(_) => return Err("cannot JSON-encode a function".into()),
        Value::External(_) => return Err("cannot JSON-encode an external value".into()),
        Value::BigInt { .. } => return Err("cannot JSON-encode a BigInt".into()),
        Value::Promise(state) => match &*state.borrow() {
            PromiseState::Pending => return Err("native addon returned a pending Promise".into()),
            PromiseState::Resolved(value) => json_from_value(*value)?,
            PromiseState::Rejected(value) => {
                let message = match value_ref(*value).map_err(|_| "invalid Promise rejection")? {
                    Value::Error(message) | Value::String(message) => message.clone(),
                    _ => json_from_value(*value)?.to_string(),
                };
                return Err(message);
            }
        },
    })
}

fn is_native_instance(value: usize) -> bool {
    HOST.with(|host| {
        host.borrow()
            .module_envs
            .iter()
            .any(|env| env.instances.contains_key(&value))
    })
}

unsafe fn json_from_value(value: NapiValue) -> Result<JsonValue, String> {
    json_from_value_with_undefined(value, false)
}

fn wait_for_promise(value: NapiValue) -> Result<NapiValue, String> {
    let state = match unsafe { value_ref(value) } {
        Ok(Value::Promise(state)) => Rc::clone(state),
        _ => return Ok(value),
    };
    loop {
        let settled = {
            let state = state.borrow();
            match *state {
                PromiseState::Pending => None,
                PromiseState::Resolved(value) => return Ok(value),
                PromiseState::Rejected(value) => Some(value),
            }
        };
        if let Some(value) = settled {
            let message = unsafe {
                match value_ref(value).map_err(|_| "invalid Promise rejection")? {
                    Value::Error(message) | Value::String(message) => message.clone(),
                    _ => json_from_value(value)?.to_string(),
                }
            };
            return Err(message);
        }
        if ACTIVE_ASYNC_WORK.load(Ordering::Acquire) == 0 {
            return Err("native addon returned a Promise with no pending work".into());
        }
        thaw_napi_poll_async_work();
        std::thread::sleep(Duration::from_millis(1));
    }
}

fn module_file_name_for_export(name: &str) -> Option<CString> {
    HOST.with(|host| {
        let host = host.borrow();
        let (env, _) = host.exports.get(name)?;
        unsafe { Some((*(*env as NapiEnv)).module_file_name.clone()) }
    })
}

unsafe fn call_impl(
    name: &str,
    args_json: &str,
    preserve_undefined: bool,
) -> Result<String, String> {
    let args: Vec<JsonValue> = serde_json::from_str(args_json)
        .map_err(|error| format!("invalid argument JSON: {error}"))?;
    let function = HOST
        .with(|host| host.borrow().functions.get(name).cloned())
        .ok_or_else(|| format!("no such native addon function `{name}`"))?;
    let mut env = Box::new(Env::new());
    if let Some(module_file_name) = module_file_name_for_export(name) {
        env.module_file_name = module_file_name;
    }
    let args = args
        .iter()
        .map(|value| value_from_json_with_undefined(&mut env, value, preserve_undefined))
        .collect();
    let this_arg = env.alloc(Value::Undefined);
    let mut info = CallbackInfo {
        args,
        this_arg,
        new_target: ptr::null_mut(),
        data: function.data,
    };
    let result = (function.callback)(&mut *env, &mut info);
    if let Some(exception) = env.exception {
        return Err(describe_env_exception(&mut *env as *mut Env, exception)?);
    }
    let result = wait_for_promise(result)?;
    serde_json::to_string(&json_from_value_with_undefined(result, preserve_undefined)?)
        .map_err(|error| error.to_string())
}

fn text_result(result: Result<String, String>) -> ThawResult {
    match result {
        Ok(value) => ThawResult {
            value: CString::new(value).unwrap_or_default().into_raw(),
            error: ptr::null_mut(),
        },
        Err(error) => ThawResult {
            value: ptr::null_mut(),
            error: CString::new(error).unwrap_or_default().into_raw(),
        },
    }
}

#[no_mangle]
pub unsafe extern "C" fn thaw_napi_call_result(
    name: *const c_char,
    args: *const c_char,
) -> ThawResult {
    text_result(text(name).and_then(|name| {
        text(args).and_then(|args| call_impl(&name, &args, false))
    }))
}

#[no_mangle]
pub unsafe extern "C" fn thaw_napi_call_typed_result(
    name: *const c_char,
    args: *const c_char,
) -> ThawResult {
    text_result(text(name).and_then(|name| {
        text(args).and_then(|args| call_impl(&name, &args, true))
    }))
}

#[no_mangle]
pub unsafe extern "C" fn thaw_napi_get_export(name: *const c_char) -> u64 {
    let Ok(name) = text(name) else {
        return 0;
    };
    HOST.with(|host| {
        host.borrow()
            .exports
            .get(&name)
            .map(|(_, value)| *value as u64)
            .unwrap_or(0)
    })
}

fn handle_error(error: impl Into<String>) -> ThawNapiHandleResult {
    ThawNapiHandleResult {
        value: 0,
        error: CString::new(error.into()).unwrap_or_default().into_raw(),
    }
}

unsafe fn module_env_for_handle(handle: u64) -> Result<NapiEnv, String> {
    if handle == 0 {
        return Err("invalid native addon handle 0".into());
    }
    HOST.with(|host| {
        host.borrow()
            .module_envs
            .iter()
            .find(|env| env.values.contains(&(handle as NapiValue)))
            .map(|env| (&**env as *const Env).cast_mut())
            .ok_or_else(|| format!("unknown native addon handle {handle}"))
    })
}

unsafe fn module_arguments(
    env: NapiEnv,
    args: *const c_char,
    preserve_undefined: bool,
) -> Result<Vec<NapiValue>, String> {
    let args: Vec<JsonValue> = serde_json::from_str(&text(args)?)
        .map_err(|error| format!("invalid argument JSON: {error}"))?;
    let env = env_mut(env).map_err(|_| "invalid native addon environment")?;
    Ok(args
        .iter()
        .map(|value| value_from_json_with_undefined(env, value, preserve_undefined))
        .collect())
}

unsafe fn call_function_handle(
    callable: u64,
    args: *const c_char,
    preserve_undefined: bool,
) -> Result<NapiValue, String> {
    let env = module_env_for_handle(callable)?;
    let values = module_arguments(env, args, preserve_undefined)?;
    let function = match value_ref(callable as NapiValue).map_err(|_| "invalid function handle")? {
        Value::Function(function) => function.clone(),
        _ => return Err(format!("native addon handle {callable} is not callable")),
    };
    let this_arg = env_mut(env)
        .map_err(|_| "invalid native addon environment")?
        .alloc(Value::Undefined);
    let mut info = CallbackInfo {
        args: values,
        this_arg,
        new_target: ptr::null_mut(),
        data: function.data,
    };
    let value = (function.callback)(env, &mut info);
    take_env_exception(env)?;
    wait_for_promise(value)
}

unsafe fn call_export_handle_impl(
    name: *const c_char,
    args: *const c_char,
    preserve_undefined: bool,
) -> ThawNapiHandleResult {
    let result = (|| -> Result<u64, String> {
        let name = text(name)?;
        let callable = thaw_napi_get_export(CString::new(name.as_str()).unwrap().as_ptr());
        if callable == 0 {
            return Err("unknown native addon export".into());
        }
        Ok(call_function_handle(callable, args, preserve_undefined)? as u64)
    })();
    match result {
        Ok(value) => ThawNapiHandleResult {
            value,
            error: ptr::null_mut(),
        },
        Err(error) => handle_error(error),
    }
}

#[no_mangle]
pub unsafe extern "C" fn thaw_napi_call_export_handle_typed_result(
    name: *const c_char,
    args: *const c_char,
) -> ThawNapiHandleResult {
    call_export_handle_impl(name, args, true)
}

#[no_mangle]
pub unsafe extern "C" fn thaw_napi_call_export_handle_with_function_typed_result(
    name: *const c_char,
    args: *const c_char,
    function_index: usize,
    callback: Option<ThawNativeValueCallback>,
    context: *mut c_void,
) -> ThawNapiHandleResult {
    let result = (|| -> Result<u64, String> {
        let name = text(name)?;
        let callable = thaw_napi_get_export(CString::new(name.as_str()).unwrap().as_ptr());
        let callback = callback.ok_or("native addon function argument is null")?;
        let env = module_env_for_handle(callable)?;
        let mut values = module_arguments(env, args, true)?;
        if function_index > values.len() {
            return Err(format!(
                "function argument index {function_index} exceeds argument count {}",
                values.len()
            ));
        }
        let bridge = Arc::new(ThawCallbackBridge {
            callback: ThawCallback::Value(callback),
            context: context as usize,
        });
        let function = env_mut(env)
            .map_err(|_| "invalid native addon environment")?
            .alloc(Value::Function(Function {
                callback: thaw_compiled_callback,
                data: Arc::as_ptr(&bridge) as *mut c_void,
                properties: HashMap::new(),
                _thaw_bridge: Some(bridge),
            }));
        values.insert(function_index, function);
        let exported = match value_ref(callable as NapiValue)
            .map_err(|_| "invalid function handle")?
        {
            Value::Function(exported) => exported.clone(),
            _ => return Err(format!("native addon export `{name}` is not callable")),
        };
        let this_arg = env_mut(env)
            .map_err(|_| "invalid native addon environment")?
            .alloc(Value::Undefined);
        let mut info = CallbackInfo {
            args: values,
            this_arg,
            new_target: ptr::null_mut(),
            data: exported.data,
        };
        let value = (exported.callback)(env, &mut info);
        take_env_exception(env)?;
        Ok(wait_for_promise(value)? as u64)
    })();
    match result {
        Ok(value) => ThawNapiHandleResult {
            value,
            error: ptr::null_mut(),
        },
        Err(error) => handle_error(error),
    }
}

#[no_mangle]
pub unsafe extern "C" fn thaw_napi_call_handle_typed_result(
    callable: u64,
    args: *const c_char,
) -> ThawResult {
    let result = call_function_handle(callable, args, true).and_then(|value| {
        serde_json::to_string(&json_from_value_with_undefined(value, true)?)
            .map_err(|error| error.to_string())
    });
    text_result(result)
}

// `napi_create_error`/`napi_create_type_error`/etc. record the error's class
// name in `error_names` alongside its message (see `alloc_error`); tag it
// onto the message the same way `new Error(...)`/etc. do in thaw-hir
// (`\u{1}<Name>\u{1}<message>`, see `thaw_runtime::split_error_tag`) so a
// native addon's typed error still exposes `.name`/`instanceof` at the
// catching Thaw code's catch site instead of collapsing to an untagged
// (default-`Error`) string.
unsafe fn describe_env_exception(env: NapiEnv, exception: NapiValue) -> Result<String, String> {
    let message = match value_ref(exception).map_err(|_| "invalid exception")? {
        Value::Error(message) | Value::String(message) => message.clone(),
        _ => json_from_value(exception)?.to_string(),
    };
    Ok(match error_name_for_owner(env, exception as usize) {
        Some(name) => format!("\u{1}{name}\u{1}{message}"),
        None => message,
    })
}

unsafe fn take_env_exception(env: NapiEnv) -> Result<(), String> {
    let Some(exception) = env_mut(env)
        .map_err(|_| "invalid native addon environment")?
        .exception
        .take()
    else {
        return Ok(());
    };
    Err(describe_env_exception(env, exception)?)
}

unsafe fn construct_handle_impl(
    constructor: u64,
    args: *const c_char,
    preserve_undefined: bool,
) -> ThawNapiHandleResult {
    let result = (|| -> Result<u64, String> {
        let env = module_env_for_handle(constructor)?;
        let values = module_arguments(env, args, preserve_undefined)?;
        let mut instance = ptr::null_mut();
        let status = napi_new_instance(
            env,
            constructor as NapiValue,
            values.len(),
            values.as_ptr(),
            &mut instance,
        );
        take_env_exception(env)?;
        if status != NAPI_OK || instance.is_null() {
            return Err(format!(
                "native addon constructor failed with status {status}"
            ));
        }
        Ok(instance as u64)
    })();
    match result {
        Ok(value) => ThawNapiHandleResult {
            value,
            error: ptr::null_mut(),
        },
        Err(error) => handle_error(error),
    }
}

#[no_mangle]
pub unsafe extern "C" fn thaw_napi_construct_handle_result(
    constructor: u64,
    args: *const c_char,
) -> ThawNapiHandleResult {
    construct_handle_impl(constructor, args, false)
}

#[no_mangle]
pub unsafe extern "C" fn thaw_napi_construct_handle_typed_result(
    constructor: u64,
    args: *const c_char,
) -> ThawNapiHandleResult {
    construct_handle_impl(constructor, args, true)
}

unsafe fn call_method_impl(
    receiver: u64,
    method: *const c_char,
    args: *const c_char,
    preserve_undefined: bool,
) -> ThawResult {
    let result = (|| -> Result<String, String> {
        let env = module_env_for_handle(receiver)?;
        let method_name = text(method)?;
        let values = module_arguments(env, args, preserve_undefined)?;
        let mut callable = ptr::null_mut();
        let method_name_c = CString::new(method_name.clone()).map_err(|_| "method contains NUL")?;
        let status = napi_get_named_property(
            env,
            receiver as NapiValue,
            method_name_c.as_ptr(),
            &mut callable,
        );
        if status != NAPI_OK {
            take_env_exception(env)?;
            return Err(format!(
                "failed to get native method `{method_name}`: status {status}"
            ));
        }
        let function = match value_ref(callable).map_err(|_| "invalid native method")? {
            Value::Function(function) => function.clone(),
            _ => return Err(format!("native property `{method_name}` is not callable")),
        };
        let mut info = CallbackInfo {
            args: values,
            this_arg: receiver as NapiValue,
            new_target: ptr::null_mut(),
            data: function.data,
        };
        let value = (function.callback)(env, &mut info);
        take_env_exception(env)?;
        let value = wait_for_promise(value)?;
        serde_json::to_string(&json_from_value_with_undefined(value, preserve_undefined)?)
            .map_err(|error| error.to_string())
    })();
    text_result(result)
}

#[no_mangle]
pub unsafe extern "C" fn thaw_napi_call_method_result(
    receiver: u64,
    method: *const c_char,
    args: *const c_char,
) -> ThawResult {
    call_method_impl(receiver, method, args, false)
}

#[no_mangle]
pub unsafe extern "C" fn thaw_napi_call_method_typed_result(
    receiver: u64,
    method: *const c_char,
    args: *const c_char,
) -> ThawResult {
    call_method_impl(receiver, method, args, true)
}

unsafe fn get_property_impl(
    receiver: u64,
    property: *const c_char,
    preserve_undefined: bool,
) -> ThawResult {
    let result = (|| -> Result<String, String> {
        let env = module_env_for_handle(receiver)?;
        let property_name = text(property)?;
        let property_name_c =
            CString::new(property_name.clone()).map_err(|_| "property contains NUL")?;
        let mut value = ptr::null_mut();
        let status = napi_get_named_property(
            env,
            receiver as NapiValue,
            property_name_c.as_ptr(),
            &mut value,
        );
        take_env_exception(env)?;
        if status != NAPI_OK || value.is_null() {
            return Err(format!(
                "failed to get native property `{property_name}`: status {status}"
            ));
        }
        let value = wait_for_promise(value)?;
        serde_json::to_string(&json_from_value_with_undefined(value, preserve_undefined)?)
            .map_err(|error| error.to_string())
    })();
    text_result(result)
}

#[no_mangle]
pub unsafe extern "C" fn thaw_napi_get_property_result(
    receiver: u64,
    property: *const c_char,
) -> ThawResult {
    get_property_impl(receiver, property, false)
}

#[no_mangle]
pub unsafe extern "C" fn thaw_napi_get_property_typed_result(
    receiver: u64,
    property: *const c_char,
) -> ThawResult {
    get_property_impl(receiver, property, true)
}

unsafe fn set_property_impl(
    receiver: u64,
    property: *const c_char,
    args: *const c_char,
    preserve_undefined: bool,
    discard_result: bool,
) -> ThawResult {
    let result = (|| -> Result<String, String> {
        let env = module_env_for_handle(receiver)?;
        let property_name = text(property)?;
        let property_name_c =
            CString::new(property_name.clone()).map_err(|_| "property contains NUL")?;
        let values = module_arguments(env, args, preserve_undefined)?;
        let [value] = values.as_slice() else {
            return Err("native property setter expects exactly one value".into());
        };
        let status =
            napi_set_named_property(env, receiver as NapiValue, property_name_c.as_ptr(), *value);
        take_env_exception(env)?;
        if status != NAPI_OK {
            return Err(format!(
                "failed to set native property `{property_name}`: status {status}"
            ));
        }
        if discard_result {
            Ok("true".into())
        } else {
            serde_json::to_string(&json_from_value_with_undefined(*value, preserve_undefined)?)
                .map_err(|error| error.to_string())
        }
    })();
    text_result(result)
}

#[no_mangle]
pub unsafe extern "C" fn thaw_napi_set_property_result(
    receiver: u64,
    property: *const c_char,
    args: *const c_char,
) -> ThawResult {
    set_property_impl(receiver, property, args, false, false)
}

#[no_mangle]
pub unsafe extern "C" fn thaw_napi_set_property_typed_result(
    receiver: u64,
    property: *const c_char,
    args: *const c_char,
) -> ThawResult {
    set_property_impl(receiver, property, args, true, false)
}

unsafe extern "C" fn thaw_compiled_callback(_env: NapiEnv, info: NapiCallbackInfo) -> NapiValue {
    let Some(info) = info.as_ref() else {
        return ptr::null_mut();
    };
    let Some(bridge) = (info.data as *const ThawCallbackBridge).as_ref() else {
        return ptr::null_mut();
    };
    let callback = match bridge.callback {
        ThawCallback::Value(callback) => Some((callback, false)),
        #[cfg(feature = "quickjs")]
        ThawCallback::QuickJs(callback) => Some((callback, true)),
        ThawCallback::Event(_) => None,
    };
    if let Some((callback, _is_quickjs)) = callback {
        let args = info
            .args
            .iter()
            .map(|value| json_from_value_with_undefined(*value, true))
            .collect::<Result<Vec<_>, _>>();
        #[cfg(feature = "quickjs")]
        let args = if _is_quickjs {
            args.map(|mut args| {
                let receiver = value_ref(info.this_arg)
                    .ok()
                    .filter(|value| is_object_value(value))
                    .map(|_| serde_json::json!({
                        "__thaw_napi_this_handle__": (info.this_arg as u64).to_string()
                    }))
                    .unwrap_or(JsonValue::Null);
                args.insert(0, receiver);
                args
            })
        } else {
            args
        };
        let args = args.and_then(|args| {
            serde_json::to_string(&args).map_err(|error| error.to_string())
        });
        let Ok(args) = args.and_then(|args| CString::new(args).map_err(|error| error.to_string()))
        else {
            return ptr::null_mut();
        };
        let result = callback(bridge.context as *mut c_void, args.as_ptr());
        let Ok(result) = text(result)
            .and_then(|result| serde_json::from_str(&result).map_err(|error| error.to_string()))
        else {
            return ptr::null_mut();
        };
        return value_from_json_with_undefined(env_mut(_env).unwrap(), &result, true);
    }
    let ThawCallback::Event(callback) = bridge.callback else {
        unreachable!()
    };
    let error = info
        .args
        .first()
        .copied()
        .map(|value| json_from_value(value).unwrap_or(JsonValue::Null))
        .unwrap_or(JsonValue::Null);
    let result = info
        .args
        .get(1)
        .copied()
        .map(|value| json_from_value(value).unwrap_or(JsonValue::Null))
        .unwrap_or(JsonValue::Null);
    let error = CString::new(serde_json::to_string(&error).unwrap()).unwrap();
    let result = CString::new(serde_json::to_string(&result).unwrap()).unwrap();
    callback(
        bridge.context as *mut c_void,
        error.as_ptr(),
        result.as_ptr(),
    );
    ptr::null_mut()
}

#[no_mangle]
pub unsafe extern "C" fn thaw_napi_call_with_callback_result(
    name: *const c_char,
    args: *const c_char,
    callback: Option<ThawNativeCallback>,
    context: *mut c_void,
) -> ThawResult {
    let result = (|| -> Result<String, String> {
        let name = text(name)?;
        let args: Vec<JsonValue> = serde_json::from_str(&text(args)?)
            .map_err(|error| format!("invalid argument JSON: {error}"))?;
        let function = HOST
            .with(|host| host.borrow().functions.get(&name).cloned())
            .ok_or_else(|| format!("no such native addon function `{name}`"))?;
        let callback = callback.ok_or("native addon callback is null")?;
        // Generated call sites create a small ABI adapter per invocation,
        // while the closure allocation itself remains stable. Use that
        // closure context as the identity so subscribe/unsubscribe calls made
        // at different source locations still receive the same napi_value.
        let callback_key = (
            context as usize,
            if context.is_null() {
                callback as usize
            } else {
                0
            },
        );
        let mut env = Box::new(Env::new());
        if let Some(module_file_name) = module_file_name_for_export(&name) {
            env.module_file_name = module_file_name;
        }
        let mut values: Vec<NapiValue> = args
            .iter()
            .map(|value| value_from_json(&mut env, value))
            .collect();
        let cached_callback =
            HOST.with(|host| host.borrow().compiled_callbacks.get(&callback_key).copied());
        let mut created_callback = None;
        let callback_value = if let Some(callback) = cached_callback {
            callback
        } else {
            let bridge = Arc::new(ThawCallbackBridge {
                callback: ThawCallback::Event(callback),
                context: context as usize,
            });
            let bridge_data = Arc::as_ptr(&bridge) as *mut c_void;
            let value = env.alloc(Value::Function(Function {
                callback: thaw_compiled_callback,
                data: bridge_data,
                properties: HashMap::new(),
                _thaw_bridge: Some(bridge),
            }));
            created_callback = Some(value);
            value
        };
        values.push(callback_value);
        let this_arg = env.alloc(Value::Undefined);
        let mut info = CallbackInfo {
            args: values,
            this_arg,
            new_target: ptr::null_mut(),
            data: function.data,
        };
        let value = (function.callback)(&mut *env, &mut info);
        if let Some(exception) = env.exception {
            return Err(describe_env_exception(&mut *env as *mut Env, exception)?);
        }
        let value = wait_for_promise(value)?;
        let value = if value.is_null() {
            "null".to_string()
        } else {
            serde_json::to_string(&json_from_value(value)?).map_err(|error| error.to_string())?
        };
        if ACTIVE_ASYNC_WORK.load(Ordering::Acquire) != 0
            || LIVE_THREADSAFE_FUNCTIONS.load(Ordering::Acquire) != 0
        {
            if let Some(callback) = created_callback {
                HOST.with(|host| {
                    host.borrow_mut()
                        .compiled_callbacks
                        .insert(callback_key, callback);
                });
            }
            HOST.with(|host| host.borrow_mut().pending_call_envs.push(env));
        } else {
            HOST.with(|host| {
                let mut host = host.borrow_mut();
                host.compiled_callbacks.remove(&callback_key);
                host.pending_call_envs.clear();
            });
        }
        Ok(value)
    })();
    match result {
        Ok(value) => ThawResult {
            value: CString::new(value).unwrap().into_raw(),
            error: ptr::null_mut(),
        },
        Err(error) => ThawResult {
            value: ptr::null_mut(),
            error: CString::new(error).unwrap_or_default().into_raw(),
        },
    }
}

#[no_mangle]
pub unsafe extern "C" fn thaw_napi_call_method_with_callback_result(
    receiver: u64,
    method: *const c_char,
    args: *const c_char,
    callback: Option<ThawNativeCallback>,
    context: *mut c_void,
    discard_result: u8,
) -> ThawResult {
    let result = (|| -> Result<String, String> {
        thaw_napi_run_async_work();
        let env = module_env_for_handle(receiver)?;
        let method_name = text(method)?;
        let args: Vec<JsonValue> = serde_json::from_str(&text(args)?)
            .map_err(|error| format!("invalid argument JSON: {error}"))?;
        let mut callable = ptr::null_mut();
        let method_name_c = CString::new(method_name.clone()).map_err(|_| "method contains NUL")?;
        let status = napi_get_named_property(
            env,
            receiver as NapiValue,
            method_name_c.as_ptr(),
            &mut callable,
        );
        if status != NAPI_OK {
            take_env_exception(env)?;
            return Err(format!(
                "failed to get native method `{method_name}`: status {status}"
            ));
        }
        let function = match value_ref(callable).map_err(|_| "invalid native method")? {
            Value::Function(function) => function.clone(),
            _ => return Err(format!("native property `{method_name}` is not callable")),
        };
        let callback = callback.ok_or("native addon callback is null")?;
        let callback_key = (
            context as usize,
            if context.is_null() {
                callback as usize
            } else {
                0
            },
        );
        let mut values: Vec<NapiValue> = args
            .iter()
            .map(|value| value_from_json_with_undefined(&mut *env, value, true))
            .collect();
        let cached_callback =
            HOST.with(|host| host.borrow().compiled_callbacks.get(&callback_key).copied());
        let callback_value = if let Some(callback) = cached_callback {
            callback
        } else {
            let bridge = Arc::new(ThawCallbackBridge {
                callback: ThawCallback::Event(callback),
                context: context as usize,
            });
            let bridge_data = Arc::as_ptr(&bridge) as *mut c_void;
            let value = (*env).alloc(Value::Function(Function {
                callback: thaw_compiled_callback,
                data: bridge_data,
                properties: HashMap::new(),
                _thaw_bridge: Some(bridge),
            }));
            HOST.with(|host| {
                host.borrow_mut()
                    .compiled_callbacks
                    .insert(callback_key, value);
            });
            value
        };
        values.push(callback_value);
        let mut info = CallbackInfo {
            args: values,
            this_arg: receiver as NapiValue,
            new_target: ptr::null_mut(),
            data: function.data,
        };
        let value = (function.callback)(env, &mut info);
        take_env_exception(env)?;
        let value = wait_for_promise(value)?;
        if discard_result != 0 || value.is_null() {
            Ok("null".to_string())
        } else {
            serde_json::to_string(&json_from_value_with_undefined(value, true)?)
                .map_err(|error| error.to_string())
        }
    })();
    text_result(result)
}

#[no_mangle]
pub unsafe extern "C" fn thaw_napi_call(name: *const c_char, args: *const c_char) -> *const c_char {
    let result = thaw_napi_call_result(name, args);
    if result.error.is_null() {
        result.value
    } else {
        let error = CStr::from_ptr(result.error).to_string_lossy();
        CString::new(format!(
            "{{\"__thaw_error__\":{}}}",
            serde_json::to_string(error.as_ref()).unwrap()
        ))
        .unwrap()
        .into_raw()
    }
}
