#[no_mangle]
pub unsafe extern "C" fn napi_module_register(module: *mut NapiModule) {
    if let Some(module) = module.as_ref() {
        PENDING_MODULE.with(|slot| *slot.borrow_mut() = Some(*module));
    }
}

type RegisterV1 = unsafe extern "C" fn(NapiEnv, NapiValue) -> NapiValue;

unsafe fn dl_error() -> String {
    let error = libc::dlerror();
    if error.is_null() {
        "dlopen failed".into()
    } else {
        CStr::from_ptr(error).to_string_lossy().into_owned()
    }
}

fn executable_relative_path(path: &str) -> Result<std::path::PathBuf, String> {
    let Some(relative) = path.strip_prefix("@executable/") else {
        return Ok(path.into());
    };
    let executable = std::env::current_exe()
        .map_err(|error| format!("failed to locate the current executable: {error}"))?;
    Ok(executable
        .parent()
        .ok_or("the current executable has no parent directory")?
        .join(relative))
}

fn register_loaded_exports(
    host: &mut Host,
    env_ptr: usize,
    package_name: Option<&str>,
    functions: Vec<(String, Function)>,
    exported_values: Vec<(String, NapiValue)>,
) {
    if let Some(package) = package_name {
        host.functions.extend(functions.iter().map(|(name, function)|
            (format!("{package}::{name}"), function.clone())));
        host.exports.extend(exported_values.iter().map(|(name, value)|
            (format!("{package}::{name}"), (env_ptr, *value))));
    }
    host.functions.extend(functions);
    host.exports.extend(exported_values.into_iter().map(|(name, value)|
        (name, (env_ptr, value))));
}

unsafe fn load_impl(path: &str, root_name: Option<&str>, package_name: Option<&str>) -> Result<(), String> {
    if HOST.with(|host| host.borrow().unloading) {
        return Err("cannot load N-API addons while unloading".into());
    }
    let path = executable_relative_path(path)?;
    let path_text = path.to_string_lossy();
    let path = CString::new(path_text.as_bytes())
        .map_err(|_| "addon path contains NUL".to_string())?;
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
        match module.nm_register_func {
            Some(init) => init,
            None => {
                libc::dlclose(handle);
                return Err("registered N-API module has no init function".into());
            }
        }
    } else {
        libc::dlclose(handle);
        return Err(
            "addon exports neither napi_register_module_v1 nor a registered N-API module".into(),
        );
    };

    let mut env = Box::new(Env::new());
    let absolute_path = std::fs::canonicalize(path_text.as_ref())
        .unwrap_or_else(|_| std::path::PathBuf::from(path_text.as_ref()));
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
    let initialized = (|| -> Result<_, String> {
        if let Some(exception) = env.exception {
            let message = json_from_value(exception)?.to_string();
            return Err(format!("addon initialization threw: {message}"));
        }
        let mut functions = Vec::new();
        let mut exported_values = Vec::new();
        match value_ref(exports).map_err(|_| "invalid exports value")? {
            Value::Object(object) => {
                for (name, value) in object {
                    if let PropertyKey::String(name) = name {
                        if let Value::Function(function) =
                            value_ref(*value).map_err(|_| "invalid export value")?
                        {
                            functions.push((name.clone(), function.clone()));
                        }
                        exported_values.push((name.clone(), *value));
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
        Ok((functions, exported_values))
    })();
    let (functions, exported_values) = match initialized {
        Ok(values) => values,
        Err(error) => {
            if ACTIVE_ASYNC_WORK.load(Ordering::Acquire) != 0
                || LIVE_THREADSAFE_FUNCTIONS.load(Ordering::Acquire) != 0
            {
                // Async work and threadsafe functions retain both this Env
                // and addon callbacks. The async poller drops pending Envs
                // after those callbacks finish; unload_all closes libraries.
                HOST.with(|host| {
                    let mut host = host.borrow_mut();
                    host.pending_call_envs.push(env);
                    host.libraries.push(handle);
                });
            } else {
                drop(env); // Finalizers and cleanup hooks may call into the addon.
                if ACTIVE_ASYNC_CLEANUP_HOOKS.load(Ordering::Acquire) == 0 {
                    libc::dlclose(handle);
                } else {
                    // An asynchronous cleanup hook can still execute addon code.
                    HOST.with(|host| host.borrow_mut().libraries.push(handle));
                }
            }
            return Err(error);
        }
    };
    HOST.with(|host| {
        let mut host = host.borrow_mut();
        register_loaded_exports(&mut host, env_ptr as usize, package_name, functions, exported_values);
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
        thaw_napi_call_typed_bridge,
        thaw_napi_handle_bridge,
        thaw_napi_poll_async_work,
        thaw_napi_async_work_pending,
    );
    Ok(())
}

#[cfg(feature = "quickjs")]
unsafe extern "C" fn thaw_napi_call_typed_bridge(
    name: *const c_char,
    args: *const c_char,
) -> *const c_char {
    let result = text(name).and_then(|name| text(args).and_then(|args| {
        let value = call_value_impl(&name, &args, true)?;
        quickjs_bridge_value(value).and_then(|wire| serde_json::to_string(&quickjs_wire_result("value", wire)).map_err(|error| error.to_string()))
    }));
    let result = result.unwrap_or_else(|error| serde_json::json!({ "__thaw_error__": error }).to_string());
    CString::new(result).unwrap_or_default().into_raw()
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
        if operation == "sync_reference" {
            let reference = target
                .parse::<u64>()
                .map_err(|_| "invalid QuickJS reference")?;
            return HOST.with(|host| {
                let host = host.borrow();
                let env = host
                    .module_envs
                    .iter()
                    .find(|env| env.quickjs_references.contains_key(&reference))
                    .ok_or_else(|| "unknown QuickJS reference".to_string())?;
                let value = env.quickjs_references[&reference];
                Ok(quickjs_wire_result("value", quickjs_reference_wire(env, value, true)?))
            });
        }
        if operation == "sync_handle" {
            let handle = target.parse::<u64>().map_err(|_| "invalid native binary handle")?;
            let env = module_env_for_handle(handle)?;
            let wire = quickjs_reference_wire(&*env, handle as NapiValue, true)?;
            return Ok(quickjs_wire_result("value", wire));
        }
        if operation == "promise_state" {
            let handle = target.parse::<u64>().map_err(|_| "invalid native Promise handle")?;
            let _ = module_env_for_handle(handle)?;
            thaw_napi_poll_async_work();
            let state = match value_ref(handle as NapiValue).map_err(|_| "invalid native Promise")? {
                Value::Promise(state) => Rc::clone(state),
                _ => return Err("native handle is not a Promise".into()),
            };
            let settled = match *state.borrow() {
                PromiseState::Pending => Ok(serde_json::json!({ "kind": "pending" })),
                PromiseState::Resolved(value) => Ok(quickjs_wire_result("resolved", quickjs_bridge_value(value)?)),
                PromiseState::Rejected(value) => Ok(quickjs_wire_result("rejected", quickjs_bridge_value(value)?)),
            };
            return settled;
        }
        let handle = if operation == "construct" {
            let target = CString::new(target).map_err(|_| "export contains NUL")?;
            get_export_result(target.as_ptr())?
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
            "symbol" => {
                let env = module_env_for_handle(handle)?;
                let options = serde_json::from_str::<Vec<JsonValue>>(&text(args)?)
                    .map_err(|error| format!("invalid symbol options: {error}"))?;
                let global = options.first().and_then(JsonValue::as_bool).unwrap_or(false);
                if let Some(id) = options.get(1).and_then(JsonValue::as_str)
                    .and_then(|id| id.parse::<u64>().ok())
                {
                    let env = env_mut(env).map_err(|_| "invalid native addon environment")?;
                    if !env.symbols.contains_key(&id) {
                        let symbol = env.alloc(Value::Symbol { id, description: name });
                        env.symbols.insert(id, symbol);
                    }
                    return Ok(serde_json::json!({ "kind": "value", "value": id.to_string() }));
                }
                let mut symbol = ptr::null_mut();
                let status = if global {
                    node_api_symbol_for(env, name.as_ptr().cast(), name.len(), &mut symbol)
                } else {
                    let description = env_mut(env).map_err(|_| "invalid native addon environment")?
                        .alloc(Value::String(name));
                    napi_create_symbol(env, description, &mut symbol)
                };
                if status != NAPI_OK { return Err(format!("failed to create native Symbol: status {status}")); }
                let Value::Symbol { id, .. } = value_ref(symbol).map_err(|_| "invalid native Symbol")? else {
                    return Err("native Symbol creation returned another value".into());
                };
                Ok(serde_json::json!({ "kind": "value", "value": id.to_string() }))
            }
            "get" | "get_symbol" => {
                let env = module_env_for_handle(handle)?;
                let mut value = ptr::null_mut();
                let status = if operation == "get_symbol" {
                    let id = name.parse::<u64>().map_err(|_| "invalid native Symbol ID")?;
                    let symbol = env_mut(env).map_err(|_| "invalid native addon environment")?
                        .symbols.get(&id).copied().ok_or("unknown native Symbol")?;
                    napi_get_property(env, handle as NapiValue, symbol, &mut value)
                } else {
                    let property = CString::new(name).map_err(|_| "property contains NUL")?;
                    napi_get_named_property(env, handle as NapiValue, property.as_ptr(), &mut value)
                };
                take_env_exception(env)?;
                if status != NAPI_OK || value.is_null() {
                    return Err(format!("failed to get native property: status {status}"));
                }
                if matches!(value_ref(value), Ok(Value::Function(_))) {
                    Ok(serde_json::json!({ "kind": "method", "value": (value as u64).to_string() }))
                } else {
                    Ok(quickjs_wire_result("value", quickjs_bridge_value(value)?))
                }
            }
            "has" | "has_symbol" => {
                let env = module_env_for_handle(handle)?;
                let mut present = false;
                let status = if operation == "has_symbol" {
                    let id = name.parse::<u64>().map_err(|_| "invalid native Symbol ID")?;
                    let symbol = env_mut(env).map_err(|_| "invalid native addon environment")?
                        .symbols.get(&id).copied().ok_or("unknown native Symbol")?;
                    napi_has_property(env, handle as NapiValue, symbol, &mut present)
                } else {
                    let property = CString::new(name).map_err(|_| "property contains NUL")?;
                    napi_has_named_property(env, handle as NapiValue, property.as_ptr(), &mut present)
                };
                take_env_exception(env)?;
                if status != NAPI_OK { return Err(format!("failed to check native property: status {status}")); }
                Ok(serde_json::json!({ "kind": "value", "value": present }))
            }
            "own_keys" => {
                let env = module_env_for_handle(handle)?;
                let mut keys = ptr::null_mut();
                let status = napi_get_all_property_names(env, handle as NapiValue, NAPI_KEY_OWN_ONLY,
                    NAPI_KEY_ALL_PROPERTIES, NAPI_KEY_NUMBERS_TO_STRINGS, &mut keys);
                take_env_exception(env)?;
                if status != NAPI_OK { return Err(format!("failed to enumerate native properties: status {status}")); }
                let Value::Array(keys) = value_ref(keys).map_err(|_| "invalid native keys")? else {
                    return Err("native property names were not an array".into());
                };
                let keys = keys.iter().filter_map(|key| *key)
                    .map(|value| quickjs_bridge_value(value).map(|wire| wire.value)).collect::<Result<Vec<_>, _>>()?;
                Ok(serde_json::json!({ "kind": "value", "value": keys }))
            }
            "descriptor" | "descriptor_symbol" => {
                let env = module_env_for_handle(handle)?;
                let key = if operation == "descriptor_symbol" {
                    let id = name.parse::<u64>().map_err(|_| "invalid native Symbol ID")?;
                    if !env_mut(env).map_err(|_| "invalid native addon environment")?.symbols.contains_key(&id) {
                        return Err("unknown native Symbol".into());
                    }
                    PropertyKey::Symbol(id)
                } else { PropertyKey::String(name) };
                let accessor = accessor_for_owner(env, handle as usize, &key);
                let own = own_property_value(env, handle as NapiValue, &key).is_some()
                    || accessor.is_some();
                if !own { return Ok(serde_json::json!({ "kind": "value", "value": null })); }
                let attributes = property_attributes_for(env, handle as usize, &key);
                Ok(serde_json::json!({ "kind": "value", "value": {
                    "configurable": attributes & NAPI_CONFIGURABLE != 0,
                    "enumerable": attributes & NAPI_ENUMERABLE != 0,
                    "writable": attributes & NAPI_WRITABLE != 0,
                    "accessor": accessor.is_some(),
                    "setter": accessor.is_some_and(|accessor| accessor.setter.is_some())
                } }))
            }
            "call_captured" => {
                let env = module_env_for_handle(handle)?;
                let receiver = name.parse::<u64>().map_err(|_| "invalid native receiver")?;
                if module_env_for_handle(receiver)? != env { return Err("native method receiver belongs to another environment".into()); }
                let values = module_arguments(env, args, true)?;
                let function = match value_ref(handle as NapiValue).map_err(|_| "invalid native method")? {
                    Value::Function(function) => function.clone(),
                    _ => return Err("captured native property is not callable".into()),
                };
                let mut info = CallbackInfo { args: values, this_arg: receiver as NapiValue,
                    new_target: ptr::null_mut(), data: function.data };
                let value = callback_result(env, (function.callback)(env, &mut info));
                take_env_exception(env)?;
                Ok(quickjs_wire_result("value", quickjs_bridge_value(value)?))
            }
            "call" => {
                let name = CString::new(name).map_err(|_| "method contains NUL")?;
                let value = call_method_value_impl(handle, name.as_ptr(), args, true)?;
                Ok(quickjs_wire_result("value", quickjs_bridge_value(value)?))
            }
            "set_symbol" => {
                let env = module_env_for_handle(handle)?;
                let id = name.parse::<u64>().map_err(|_| "invalid native Symbol ID")?;
                let symbol = env_mut(env).map_err(|_| "invalid native addon environment")?
                    .symbols.get(&id).copied().ok_or("unknown native Symbol")?;
                let values = module_arguments(env, args, true)?;
                let [value] = values.as_slice() else { return Err("native property setter expects exactly one value".into()); };
                let status = napi_set_property(env, handle as NapiValue, symbol, *value);
                take_env_exception(env)?;
                if status != NAPI_OK { return Err(format!("failed to set native Symbol property: status {status}")); }
                Ok(serde_json::json!({ "kind": "value", "value": true }))
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
    match text(path).and_then(|path| load_impl(&path, None, None)) {
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
        .and_then(|path| text(root_name).and_then(|root_name| load_impl(&path, Some(&root_name), None)));
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
pub unsafe extern "C" fn thaw_napi_load_named_qualified(
    path: *const c_char, root_name: *const c_char, package_name: *const c_char,
) -> u8 {
    let result = text(path).and_then(|path| text(root_name).and_then(|root_name|
        text(package_name).and_then(|package_name|
            load_impl(&path, (!root_name.is_empty()).then_some(root_name.as_str()), Some(&package_name)))));
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
    struct UnloadGuard;
    impl Drop for UnloadGuard {
        fn drop(&mut self) {
            HOST.with(|host| host.borrow_mut().unloading = false);
        }
    }

    let Some((pending_envs, module_envs)) = HOST.with(|host| {
        let mut host = host.borrow_mut();
        if host.unloading {
            host.last_error = "N-API addon unload is already in progress".into();
            return None;
        }
        if ACTIVE_ASYNC_WORK.load(Ordering::Acquire) != 0
            || LIVE_THREADSAFE_FUNCTIONS.load(Ordering::Acquire) != 0
            || ACTIVE_ASYNC_CLEANUP_HOOKS.load(Ordering::Acquire) != 0
        {
            host.last_error =
                "cannot unload N-API addons while asynchronous work or cleanup is active".into();
            return None;
        }
        host.unloading = true;
        host.functions.clear();
        host.exports.clear();
        host.compiled_callbacks.clear();
        Some((std::mem::take(&mut host.pending_call_envs), std::mem::take(&mut host.module_envs)))
    }) else {
        return 0;
    };
    let _unloading = UnloadGuard;
    // Cleanup hooks and native finalizers may reenter HOST and addon code.
    drop(pending_envs);
    drop(module_envs);
    let Some((libraries, embedded_files)) = HOST.with(|host| {
        let mut host = host.borrow_mut();
        if ACTIVE_ASYNC_WORK.load(Ordering::Acquire) != 0
            || LIVE_THREADSAFE_FUNCTIONS.load(Ordering::Acquire) != 0
            || ACTIVE_ASYNC_CLEANUP_HOOKS.load(Ordering::Acquire) != 0
        {
            host.last_error =
                "cannot unload N-API addons while asynchronous work or cleanup is active".into();
            return None;
        }
        Some((std::mem::take(&mut host.libraries), std::mem::take(&mut host.embedded_files)))
    }) else {
        return 0;
    };
    for handle in libraries.into_iter().rev() {
        unsafe { libc::dlclose(handle) };
    }
    drop(embedded_files);
    1
}

fn decode_hex(input: &str) -> Result<Vec<u8>, String> {
    if let Some(encoded) = input.strip_prefix("gz:") {
        let compressed = base64::engine::general_purpose::STANDARD
            .decode(encoded)
            .map_err(|error| format!("invalid embedded addon base64: {error}"))?;
        let mut bytes = Vec::new();
        GzDecoder::new(compressed.as_slice())
            .read_to_end(&mut bytes)
            .map_err(|error| format!("invalid embedded addon gzip: {error}"))?;
        return Ok(bytes);
    }
    if !input.len().is_multiple_of(2) {
        return Err("embedded addon hex has an odd length".into());
    }
    input
        .as_bytes()
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| {
            let text = std::str::from_utf8(pair).map_err(|error| error.to_string())?;
            u8::from_str_radix(text, 16).map_err(|error| error.to_string())
        })
        .collect()
}

#[cfg(target_os = "linux")]
fn load_embedded_shared_library(bytes: &[u8]) -> Result<(), String> {
    let name = CString::new("thaw-native-dependency").unwrap();
    let fd = unsafe { libc::memfd_create(name.as_ptr(), libc::MFD_CLOEXEC) };
    if fd < 0 {
        return Err(format!("memfd_create failed: {}", std::io::Error::last_os_error()));
    }
    let mut file = unsafe { std::fs::File::from_raw_fd(fd) };
    file.write_all(bytes)
        .map_err(|error| format!("failed to write embedded native dependency: {error}"))?;
    let path = CString::new(format!("/proc/self/fd/{fd}")).unwrap();
    let handle = unsafe { libc::dlopen(path.as_ptr(), libc::RTLD_NOW | libc::RTLD_GLOBAL) };
    if handle.is_null() {
        return Err(unsafe { dl_error() });
    }
    HOST.with(|host| {
        let mut host = host.borrow_mut();
        host.libraries.push(handle);
        host.embedded_files.push(file);
    });
    Ok(())
}

#[cfg(not(target_os = "linux"))]
fn load_embedded_shared_library(bytes: &[u8]) -> Result<(), String> {
    let mut file = tempfile::Builder::new()
        .prefix("thaw-native-dependency-")
        .tempfile()
        .map_err(|error| error.to_string())?;
    file.write_all(bytes).map_err(|error| error.to_string())?;
    file.flush().map_err(|error| error.to_string())?;
    let path = CString::new(file.path().to_string_lossy().as_bytes()).unwrap();
    let handle = unsafe { libc::dlopen(path.as_ptr(), libc::RTLD_NOW | libc::RTLD_GLOBAL) };
    if handle.is_null() {
        return Err(unsafe { dl_error() });
    }
    HOST.with(|host| host.borrow_mut().libraries.push(handle));
    Ok(())
}

#[no_mangle]
pub unsafe extern "C" fn thaw_napi_load_embedded_shared_hex(hex: *const c_char) -> u8 {
    let result = text(hex).and_then(|hex| decode_hex(&hex)).and_then(|bytes| load_embedded_shared_library(&bytes));
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
pub unsafe extern "C" fn thaw_napi_load_shared(path: *const c_char) -> u8 {
    let result = text(path).and_then(|path| {
        let path = executable_relative_path(&path)?;
        let path = CString::new(path.to_string_lossy().as_bytes())
            .map_err(|_| "shared library path contains NUL".to_string())?;
        let handle = libc::dlopen(path.as_ptr(), libc::RTLD_NOW | libc::RTLD_GLOBAL);
        if handle.is_null() {
            return Err(dl_error());
        }
        HOST.with(|host| host.borrow_mut().libraries.push(handle));
        Ok(())
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

#[cfg(target_os = "linux")]
fn load_embedded_impl(bytes: &[u8], root_name: Option<&str>, package_name: Option<&str>) -> Result<(), String> {
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
    unsafe { load_impl(&format!("/proc/self/fd/{fd}"), root_name, package_name)? };
    HOST.with(|host| host.borrow_mut().embedded_files.push(file));
    Ok(())
}

#[cfg(not(target_os = "linux"))]
fn load_embedded_impl(bytes: &[u8], root_name: Option<&str>, package_name: Option<&str>) -> Result<(), String> {
    let mut file = tempfile::Builder::new()
        .prefix("thaw-native-addon-")
        .suffix(".node")
        .tempfile()
        .map_err(|error| format!("failed to create embedded addon file: {error}"))?;
    file.write_all(bytes)
        .map_err(|error| format!("failed to write embedded addon: {error}"))?;
    file.flush()
        .map_err(|error| format!("failed to flush embedded addon: {error}"))?;
    unsafe { load_impl(&file.path().to_string_lossy(), root_name, package_name) }
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
        load_embedded_impl(&bytes, root_name, None)
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

#[no_mangle]
pub unsafe extern "C" fn thaw_napi_load_embedded_hex_qualified(
    hex: *const c_char, root_name: *const c_char, package_name: *const c_char,
) -> u8 {
    let result = text(hex).and_then(|hex| {
        let bytes = decode_hex(&hex)?;
        let root_name = text(root_name)?;
        let package_name = text(package_name)?;
        load_embedded_impl(&bytes, (!root_name.is_empty()).then_some(root_name.as_str()), Some(&package_name))
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

#[no_mangle]
pub unsafe extern "C" fn thaw_napi_embed_executable_hex(hex: *const c_char) -> *const c_char {
    let result = text(hex).and_then(|hex| {
        let bytes = decode_hex(&hex)?;
        #[cfg(target_os = "linux")]
        {
            let name = CString::new("thaw-embedded-executable").unwrap();
            let fd = unsafe { libc::memfd_create(name.as_ptr(), 0) };
            if fd < 0 {
                return Err(format!(
                    "memfd_create failed: {}",
                    std::io::Error::last_os_error()
                ));
            }
            let mut file = unsafe { std::fs::File::from_raw_fd(fd) };
            file.write_all(&bytes)
                .map_err(|error| format!("failed to write embedded executable: {error}"))?;
            if unsafe { libc::fchmod(fd, 0o700) } != 0 {
                return Err(format!(
                    "failed to mark embedded executable executable: {}",
                    std::io::Error::last_os_error()
                ));
            }
            HOST.with(|host| host.borrow_mut().embedded_files.push(file));
            Ok(format!("/proc/self/fd/{fd}"))
        }
        #[cfg(not(target_os = "linux"))]
        {
            let _ = bytes;
            Err("embedded executables are currently supported only on Linux".into())
        }
    });
    match result {
        Ok(path) => CString::new(path).unwrap_or_default().into_raw(),
        Err(error) => {
            HOST.with(|host| host.borrow_mut().last_error = error.clone());
            eprintln!("thaw-napi: {error}");
            std::ptr::null()
        }
    }
}

const TYPED_UNDEFINED_KEY: &str = "$__thaw_napi_undefined$";

fn value_from_json_with_undefined(
    env: &mut Env,
    json: &JsonValue,
    preserve_undefined: bool,
) -> Result<NapiValue, String> {
    match json {
        JsonValue::Null => Ok(env.alloc(Value::Null)),
        JsonValue::Bool(value) => Ok(env.alloc(Value::Bool(*value))),
        JsonValue::Number(value) => Ok(env.alloc(Value::Number(value.as_f64().unwrap_or(0.0)))),
        JsonValue::String(value) => Ok(env.alloc(Value::String(value.clone()))),
        JsonValue::Array(values) => {
            let values = values
                .iter()
                .map(|value| {
                    value_from_json_with_undefined(
                        env,
                        value,
                        preserve_undefined,
                    ).map(Some)
                })
                .collect::<Result<Vec<_>, _>>()?;
            Ok(env.alloc(Value::Array(values)))
        }
        JsonValue::Object(values) => {
            if preserve_undefined
                && values.len() == 1
                && values.get(TYPED_UNDEFINED_KEY).and_then(JsonValue::as_bool) == Some(true)
            {
                return Ok(env.alloc(Value::Undefined));
            }
            if values.get("type").and_then(JsonValue::as_str) == Some("Buffer") {
                if let Some(bytes) = values.get("data").and_then(JsonValue::as_array) {
                    return Ok(env.alloc(Value::Buffer(
                        bytes
                            .iter()
                            .map(|value| value.as_u64().unwrap_or(0) as u8)
                            .collect(),
                    )));
                }
            }
            if let Some(handle) = values
                .get("__thaw_napi_handle__")
                .and_then(JsonValue::as_str)
                .and_then(|handle| handle.parse::<u64>().ok())
            {
                let value = handle as NapiValue;
                if env.values.contains(&value) {
                    return Ok(value);
                }
                return Ok(env.alloc(Value::Undefined));
            }
            if let Some(reference) = values
                .get("__thaw_napi_function__")
                .and_then(JsonValue::as_u64)
            {
                #[cfg(not(feature = "quickjs"))]
                {
                    let _ = reference;
                    return Ok(env.alloc(Value::Undefined));
                }
                #[cfg(feature = "quickjs")]
                {
                if let Some(value) = env.quickjs_references.get(&reference) {
                    return Ok(*value);
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
                return Ok(function);
                }
            }
            if let Some(reference) = values
                .get("__thaw_napi_object__")
                .and_then(JsonValue::as_u64)
            {
                let object = env.quickjs_references.get(&reference).copied().unwrap_or_else(|| {
                    let object = env.alloc(Value::Object(HashMap::new()));
                    env.quickjs_references.insert(reference, object);
                    object
                });
                let properties = values
                    .get("value")
                    .and_then(JsonValue::as_object)
                    .map(|properties| {
                        properties
                            .iter()
                            .map(|(key, value)| {
                                Ok((
                                    key.clone().into(),
                                    value_from_json_with_undefined(
                                        env,
                                        value,
                                        preserve_undefined,
                                    )?,
                                ))
                            })
                            .collect::<Result<HashMap<_, _>, String>>()
                    })
                    .transpose()?.unwrap_or_default();
                if let Some(Value::Object(target)) = unsafe { object.as_mut() } {
                    *target = properties;
                }
                return Ok(object);
            }
            if let Some(reference) = values
                .get("__thaw_napi_array__")
                .and_then(JsonValue::as_u64)
            {
                let array = env.quickjs_references.get(&reference).copied().unwrap_or_else(|| {
                    let array = env.alloc(Value::Array(Vec::new()));
                    env.quickjs_references.insert(reference, array);
                    array
                });
                let elements = values.get("value").and_then(JsonValue::as_array);
                if let (Some(elements), Some(Value::Array(target))) =
                    (elements, unsafe { array.as_mut() })
                {
                    // Decode through the registered placeholder so self-references resolve.
                    let decoded = elements
                        .iter()
                        .map(|value| value_from_json_with_undefined(env, value, preserve_undefined).map(Some))
                        .collect::<Result<Vec<_>, _>>()?;
                    *target = decoded;
                }
                return Ok(array);
            }
            if let Some(reference) = values
                .get("__thaw_napi_buffer__")
                .and_then(JsonValue::as_u64)
            {
                let bytes = values.get("value").and_then(JsonValue::as_array)
                    .ok_or("native Buffer is missing its bytes")?;
                let decoded: Vec<u8> = bytes.iter()
                    .map(|value| value.as_u64().unwrap_or(0) as u8).collect();
                if let Some(buffer) = env.quickjs_references.get(&reference).copied() {
                    let Some(Value::Buffer(target)) = (unsafe { buffer.as_mut() }) else {
                        return Err("native Buffer reference changed its type".into());
                    };
                    if target.len() != decoded.len() {
                        return Err("native Buffer reference changed its fixed byte length".into());
                    }
                    target.as_mut_slice().copy_from_slice(&decoded);
                    return Ok(buffer);
                }
                let buffer = env.alloc(Value::Buffer(decoded));
                env.quickjs_references.insert(reference, buffer);
                return Ok(buffer);
            }
            if let Some(reference) = values
                .get("__thaw_napi_arraybuffer__")
                .and_then(JsonValue::as_u64)
            {
                let bytes = values.get("value").and_then(JsonValue::as_array)
                    .ok_or("native ArrayBuffer is missing its bytes")?;
                let shared = values.get("shared").and_then(JsonValue::as_bool).unwrap_or(false);
                let decoded: Vec<u8> = bytes.iter()
                    .map(|byte| byte.as_u64().unwrap_or(0) as u8).collect();
                if let Some(buffer) = env.quickjs_references.get(&reference).copied() {
                    let target = match unsafe { buffer.as_mut() } {
                        Some(Value::ArrayBuffer { bytes, detached: false }) if !shared => bytes,
                        Some(Value::SharedArrayBuffer(bytes)) if shared => bytes,
                        _ => return Err("native ArrayBuffer reference changed its type or was detached".into()),
                    };
                    if target.len() != decoded.len() {
                        return Err("native ArrayBuffer reference changed its fixed byte length".into());
                    }
                    target.as_mut_slice().copy_from_slice(&decoded);
                    return Ok(buffer);
                }
                let buffer = env.alloc(if shared {
                    Value::SharedArrayBuffer(decoded)
                } else {
                    Value::ArrayBuffer { bytes: decoded, detached: false }
                });
                env.quickjs_references.insert(reference, buffer);
                return Ok(buffer);
            }
            if values.contains_key("__thaw_napi_view__") {
                let reference = values.get("__thaw_napi_view__").and_then(JsonValue::as_u64)
                    .ok_or("native view has an invalid reference ID")?;
                let backing = values.get("buffer")
                    .map(|value| value_from_json_with_undefined(env, value, preserve_undefined))
                    .ok_or("native view is missing its backing buffer")??;
                let kind = values.get("kind").and_then(JsonValue::as_i64)
                    .ok_or("native view has an invalid kind")?;
                let length = values.get("length").and_then(JsonValue::as_u64)
                    .and_then(|length| usize::try_from(length).ok())
                    .ok_or("native view has an invalid length")?;
                let byte_offset = values.get("byte_offset").and_then(JsonValue::as_u64)
                    .and_then(|offset| usize::try_from(offset).ok())
                    .ok_or("native view has an invalid byte offset")?;
                let mut view = ptr::null_mut();
                let status = if kind == -1 {
                    unsafe { napi_create_dataview(env as NapiEnv, length, backing, byte_offset, &mut view) }
                } else {
                    let kind = i32::try_from(kind).map_err(|_| "native view kind is out of range")?;
                    unsafe { napi_create_typedarray(env as NapiEnv, kind, length, backing, byte_offset, &mut view) }
                };
                if status != NAPI_OK {
                    return Err(format!("invalid native view metadata: N-API status {status}"));
                }
                if let Some(existing) = env.quickjs_references.get(&reference).copied() {
                    // A repeated ID must describe the same view, including its backing store.
                    let same = match (unsafe { value_ref(existing) }, unsafe { value_ref(view) }) {
                        (Ok(Value::TypedArray { array_type: a, length: al, array_buffer: ab, byte_offset: ao }),
                         Ok(Value::TypedArray { array_type: b, length: bl, array_buffer: bb, byte_offset: bo })) =>
                            a == b && al == bl && ab == bb && ao == bo,
                        (Ok(Value::DataView { length: al, array_buffer: ab, byte_offset: ao }),
                         Ok(Value::DataView { length: bl, array_buffer: bb, byte_offset: bo })) =>
                            al == bl && ab == bb && ao == bo,
                        _ => false,
                    };
                    // The constructor created this value only to validate untrusted metadata.
                    if env.values.last().copied() == Some(view) {
                        env.values.pop();
                        unsafe { drop(Box::from_raw(view)); }
                    }
                    if !same {
                        return Err("native view reference changed its metadata".into());
                    }
                    return Ok(existing);
                }
                env.quickjs_references.insert(reference, view);
                return Ok(view);
            }
            if let Some(reference) = values.get("__thaw_napi_ref__").and_then(JsonValue::as_u64) {
                return Ok(env.quickjs_references.get(&reference).copied()
                    .unwrap_or_else(|| env.alloc(Value::Undefined)));
            }
            let values = values
                .iter()
                .map(|(key, value)| {
                    Ok((
                        key.clone().into(),
                        value_from_json_with_undefined(env, value, preserve_undefined)?,
                    ))
                })
                .collect::<Result<HashMap<_, _>, String>>()?;
            Ok(env.alloc(Value::Object(values)))
        }
    }
}

fn value_from_json(env: &mut Env, json: &JsonValue) -> Result<NapiValue, String> {
    value_from_json_with_undefined(env, json, false)
}

struct QuickJsWireValue {
    value: JsonValue,
    // [path, kind, payload]. Paths contain only own string keys and array indices.
    origins: Vec<JsonValue>,
}

fn quickjs_wire_result(kind: &str, wire: QuickJsWireValue) -> JsonValue {
    serde_json::json!({ "kind": kind, "value": wire.value, "origins": wire.origins })
}

fn quickjs_child_path(path: &[JsonValue], segment: JsonValue) -> Vec<JsonValue> {
    let mut child = path.to_vec();
    child.push(segment);
    child
}

unsafe fn quickjs_reference_wire(env: &Env, value: NapiValue, root: bool) -> Result<QuickJsWireValue, String> {
    let mut origins = Vec::new();
    let value = quickjs_reference_json(env, value, root, &mut HashSet::new(), &[], &mut origins)?;
    Ok(QuickJsWireValue { value, origins })
}

/// Snapshot one reference while keeping child identities on the existing wire ID.
unsafe fn quickjs_reference_json(
    env: &Env,
    value: NapiValue,
    root: bool,
    seen: &mut HashSet<usize>,
    path: &[JsonValue],
    origins: &mut Vec<JsonValue>,
) -> Result<JsonValue, String> {
    if !root {
        if let Some((reference, _)) = env.quickjs_references.iter().find(|(_, item)| **item == value) {
            return Ok(serde_json::json!({ "__thaw_napi_ref__": reference }));
        }
    }
    if !seen.insert(value as usize) {
        return Ok(serde_json::json!({ "__thaw_napi_handle__": (value as u64).to_string() }));
    }
    let result = match value_ref(value).map_err(|_| "invalid napi_value")? {
        Value::Object(properties) if !env.instances.contains_key(&(value as usize)) => {
            // The normal object is data, even if its own keys resemble native markers.
            origins.push(serde_json::json!([path, "plain"]));
            let mut out = serde_json::Map::new();
            for (key, child) in properties {
                if let PropertyKey::String(key) = key {
                    if !matches!(value_ref(*child), Ok(Value::Symbol { .. }))
                        && (!matches!(value_ref(*child), Ok(Value::Function(_)))
                            || env.quickjs_references.values().any(|reference| *reference == *child))
                    {
                        let child_path = quickjs_child_path(path, JsonValue::String(key.clone()));
                        out.insert(key.clone(), quickjs_reference_json(env, *child, false, seen, &child_path, origins)?);
                    }
                }
            }
            JsonValue::Object(out)
        }
        Value::Array(values) => JsonValue::Array(values.iter().enumerate().map(|(index, child)| match child {
            Some(child) => {
                let child_path = quickjs_child_path(path, serde_json::json!(index));
                quickjs_reference_json(env, *child, false, seen, &child_path, origins)
            }
            None => Ok(JsonValue::Null),
        }).collect::<Result<_, _>>()?),
        Value::Buffer(bytes) if root => JsonValue::Array(bytes.iter().map(|byte| serde_json::json!(byte)).collect()),
        Value::Buffer(bytes) => buffer_json(bytes, true),
        Value::ArrayBuffer { .. } | Value::SharedArrayBuffer(_) |
        Value::ExternalArrayBuffer { .. } | Value::ExternalSharedArrayBuffer { .. } => {
            let (data, length, detached) = arraybuffer_parts(value)
                .map_err(|_| "invalid ArrayBuffer backing store")?;
            if !detached && length != 0 && data.is_null() {
                return Err("invalid ArrayBuffer data pointer".into());
            }
            let bytes = if detached || length == 0 { &[] } else { std::slice::from_raw_parts(data, length) };
            let bytes = bytes.iter().copied().map(JsonValue::from).collect::<Vec<_>>();
            if root { JsonValue::Array(bytes) } else {
                let shared = matches!(value_ref(value), Ok(Value::SharedArrayBuffer(_) | Value::ExternalSharedArrayBuffer { .. }));
                serde_json::json!({ "__thaw_napi_binary__": (value as u64).to_string(),
                    "kind": if shared { "SharedArrayBuffer" } else { "ArrayBuffer" }, "data": bytes })
            }
        }
        Value::TypedArray { array_type, length, array_buffer, byte_offset } => {
            serde_json::json!({ "__thaw_napi_binary__": (value as u64).to_string(),
                "kind": "TypedArray", "array_type": array_type, "length": length,
                "byte_offset": byte_offset,
                "buffer": quickjs_reference_json(env, *array_buffer, false, seen, &quickjs_child_path(path, serde_json::json!("buffer")), origins)? })
        }
        Value::DataView { length, array_buffer, byte_offset } => {
            serde_json::json!({ "__thaw_napi_binary__": (value as u64).to_string(),
                "kind": "DataView", "length": length, "byte_offset": byte_offset,
                "buffer": quickjs_reference_json(env, *array_buffer, false, seen, &quickjs_child_path(path, serde_json::json!("buffer")), origins)? })
        }
        Value::Object(_) => serde_json::json!({ "__thaw_napi_handle__": (value as u64).to_string() }),
        Value::Promise(_) => serde_json::json!({ "__thaw_napi_promise__": (value as u64).to_string() }),
        Value::Date(time) => {
            origins.push(serde_json::json!([path, "date", time.is_finite().then_some(*time)]));
            json_from_value_with_undefined(value, true)?
        }
        Value::Number(number) if !number.is_finite() => {
            let token = if number.is_nan() { "NaN" } else if number.is_sign_positive() { "Infinity" } else { "-Infinity" };
            origins.push(serde_json::json!([path, "nonfinite", token]));
            JsonValue::Null
        }
        _ => json_from_value_with_undefined(value, true)?,
    };
    seen.remove(&(value as usize));
    Ok(result)
}

unsafe fn quickjs_bridge_value(value: NapiValue) -> Result<QuickJsWireValue, String> {
    let marker = |value| Ok(QuickJsWireValue { value, origins: Vec::new() });
    match value_ref(value) {
        Ok(Value::Promise(_)) => return marker(serde_json::json!({ "__thaw_napi_promise__": (value as u64).to_string() })),
        Ok(Value::Symbol { id, description }) => {
            let global = GLOBAL_SYMBOLS.get().is_some_and(|symbols| {
                symbols.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
                    .get(description) == Some(id)
            });
            return marker(serde_json::json!({ "__thaw_napi_symbol__": id.to_string(), "description": description, "global": global }));
        }
        Ok(Value::Error(message)) => {
            let name = error_name_for_owner(ptr::null_mut(), value as usize)
                .unwrap_or_else(|| "Error".into());
            return marker(serde_json::json!({ "__thaw_napi_error__": message, "name": name }));
        }
        _ => {}
    }
    HOST.with(|host| {
        let host = host.borrow();
        if let Some(env) = host.module_envs.iter().find(|env| env.values.contains(&value)) {
            quickjs_reference_wire(env, value, false)
        } else {
            Err("native addon result belongs to an unknown environment".into())
        }
    })
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
                    Some(value)
                        if matches!(value_ref(*value), Ok(Value::Function(_) | Value::Symbol { .. })) =>
                    {
                        Ok(JsonValue::Null)
                    }
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
                    PropertyKey::String(_)
                        if matches!(value_ref(*value), Ok(Value::Function(_) | Value::Symbol { .. })) =>
                    {
                        None
                    }
                    PropertyKey::String(key) => Some(
                        json_from_value_with_undefined(*value, preserve_undefined)
                            .map(|value| (key.clone(), value)),
                    ),
                    PropertyKey::Symbol(_) => None,
                })
                .collect::<Result<_, String>>()?,
        ),
        Value::Buffer(values) => buffer_json(values, preserve_undefined),
        Value::ExternalBuffer { data, length } => {
            let bytes = if *length == 0 {
                &[]
            } else {
                std::slice::from_raw_parts(*data, *length)
            };
            buffer_json(bytes, preserve_undefined)
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
            buffer_json(bytes, preserve_undefined)
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

fn buffer_json(bytes: &[u8], typed: bool) -> JsonValue {
    let data = bytes
        .iter()
        .copied()
        .map(JsonValue::from)
        .collect::<Vec<_>>();
    if typed {
        serde_json::json!({ "type": "Buffer", "data": data })
    } else {
        JsonValue::Array(data)
    }
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
        thaw_napi_poll_async_work();
        if !matches!(*state.borrow(), PromiseState::Pending) {
            continue;
        }
        if ACTIVE_ASYNC_WORK.load(Ordering::Acquire) == 0
            && UNFINALIZED_THREADSAFE_FUNCTIONS.load(Ordering::Acquire) == 0
            && !unsafe { poll_uv_loop() }
        {
            return Err("native addon returned a Promise with no pending work".into());
        }
        std::thread::sleep(Duration::from_millis(1));
    }
}

unsafe fn call_impl(
    name: &str,
    args_json: &str,
    preserve_undefined: bool,
) -> Result<String, String> {
    let result = wait_for_promise(call_value_impl(name, args_json, preserve_undefined)?)?;
    serde_json::to_string(&json_from_value_with_undefined(result, preserve_undefined)?)
        .map_err(|error| error.to_string())
}

unsafe fn call_value_impl(
    name: &str,
    args_json: &str,
    preserve_undefined: bool,
) -> Result<NapiValue, String> {
    let args: Vec<JsonValue> = serde_json::from_str(args_json)
        .map_err(|error| format!("invalid argument JSON: {error}"))?;
    let (function, env) = HOST
        .with(|host| {
            let host = host.borrow();
            Some((
                host.functions.get(name)?.clone(),
                host.exports.get(name)?.0 as NapiEnv,
            ))
        })
        .ok_or_else(|| format!("no such native addon function `{name}`"))?;
    let env = env_mut(env).map_err(|_| "invalid native addon environment")?;
    let args = args
        .iter()
        .map(|value| value_from_json_with_undefined(env, value, preserve_undefined))
        .collect::<Result<Vec<_>, _>>()?;
    let this_arg = env.alloc(Value::Undefined);
    let mut info = CallbackInfo {
        args,
        this_arg,
        new_target: ptr::null_mut(),
        data: function.data,
    };
    let result = callback_result(env, (function.callback)(env, &mut info));
    take_env_exception(env)?;
    Ok(result)
}

fn text_result(result: Result<String, String>) -> ThawResult {
    match result {
        Ok(value) => ThawResult {
            value: thaw_arena::owned_string(value),
            error: ptr::null_mut(),
        },
        Err(error) => ThawResult {
            value: ptr::null_mut(),
            error: CString::new(error).unwrap_or_default().into_raw(),
        },
    }
}

unsafe fn callback_result(env: NapiEnv, value: NapiValue) -> NapiValue {
    if value.is_null() {
        env_mut(env).unwrap().alloc(Value::Undefined)
    } else {
        value
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
    get_export_result(name).unwrap_or(0)
}

#[no_mangle]
pub unsafe extern "C" fn thaw_napi_get_export_typed_result(name: *const c_char) -> ThawNapiHandleResult {
    match get_export_result(name) {
        Ok(value) => ThawNapiHandleResult { value, error: ptr::null_mut() },
        Err(error) => handle_error(error),
    }
}

unsafe fn get_export_result(name: *const c_char) -> Result<u64, String> {
    let name = text(name)?;
    if let Some(value) = HOST.with(|host| host.borrow().exports.get(&name).map(|(_, value)| *value as u64)) {
        return Ok(value);
    }
    let (first, parts) = if let Some((package, path)) = name.split_once("::") {
        let mut parts = path.split('.');
        let Some(root) = parts.next() else { return Ok(0); };
        (format!("{package}::{root}"), parts.collect::<Vec<_>>())
    } else {
        let mut parts = name.split('.');
        let Some(root) = parts.next() else { return Ok(0); };
        (root.to_string(), parts.collect::<Vec<_>>())
    };
    let Some(mut value) = HOST.with(|host| host.borrow().exports.get(&first).map(|(_, value)| *value as u64)) else {
        return Ok(0);
    };
    for part in parts {
        let env = module_env_for_handle(value)?;
        let property = CString::new(part).map_err(|_| "native addon export contains NUL")?;
        let mut next = ptr::null_mut();
        let status = napi_get_named_property(env, value as NapiValue, property.as_ptr(), &mut next);
        if status != NAPI_OK {
            take_env_exception(env)?;
            return Err(format!("failed to read native addon export `{name}`: N-API status {status}"));
        }
        if next.is_null() || matches!(value_ref(next), Ok(Value::Undefined)) {
            return Ok(0);
        }
        value = next as u64;
    }
    Ok(value)
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
    args
        .iter()
        .map(|value| value_from_json_with_undefined(env, value, preserve_undefined))
        .collect()
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
    let value = callback_result(env, (function.callback)(env, &mut info));
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
        let callable = get_export_result(CString::new(name.as_str()).unwrap().as_ptr())?;
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
    let argument = ThawNativeFunctionArgument {
        index: function_index,
        callback,
        context,
    };
    thaw_napi_call_export_handle_with_functions_typed_result(name, args, &argument, 1)
}

#[repr(C)]
pub struct ThawNativeFunctionArgument {
    index: usize,
    callback: Option<ThawNativeValueCallback>,
    context: *mut c_void,
}

unsafe fn call_export_with_functions(
    name: *const c_char,
    args: *const c_char,
    functions: *const ThawNativeFunctionArgument,
    function_count: usize,
) -> Result<NapiValue, String> {
    if function_count != 0 && functions.is_null() {
        return Err("native addon function argument list is null".into());
    }
    let name = text(name)?;
    let callable = get_export_result(CString::new(name.as_str()).unwrap().as_ptr())?;
    let env = module_env_for_handle(callable)?;
    let mut values = module_arguments(env, args, true)?;
    let functions = if function_count == 0 {
        &[]
    } else {
        std::slice::from_raw_parts(functions, function_count)
    };
    for function in functions {
        if function.index > values.len() {
            return Err(format!(
                "function argument index {} exceeds argument count {}",
                function.index,
                values.len()
            ));
        }
        let value = if let Some(callback) = function.callback {
            let callback_key = (
                env as usize,
                function.context as usize,
                if function.context.is_null() {
                    callback as usize
                } else {
                    0
                },
            );
            if let Some(value) =
                HOST.with(|host| host.borrow().compiled_callbacks.get(&callback_key).copied())
            {
                value
            } else {
                let bridge = Arc::new(ThawCallbackBridge {
                    callback: ThawCallback::Value(callback),
                    context: function.context as usize,
                });
                let value = env_mut(env)
                    .map_err(|_| "invalid native addon environment")?
                    .alloc(Value::Function(Function {
                        callback: thaw_compiled_callback,
                        data: Arc::as_ptr(&bridge) as *mut c_void,
                        properties: HashMap::new(),
                        _thaw_bridge: Some(bridge),
                    }));
                HOST.with(|host| {
                    host.borrow_mut()
                        .compiled_callbacks
                        .insert(callback_key, value);
                });
                value
            }
        } else {
            env_mut(env)
                .map_err(|_| "invalid native addon environment")?
                .alloc(Value::Undefined)
        };
        values.insert(function.index, value);
    }
    let exported = match value_ref(callable as NapiValue).map_err(|_| "invalid function handle")? {
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
    let value = callback_result(env, (exported.callback)(env, &mut info));
    take_env_exception(env)?;
    wait_for_promise(value)
}

#[no_mangle]
pub unsafe extern "C" fn thaw_napi_call_export_handle_with_functions_typed_result(
    name: *const c_char,
    args: *const c_char,
    functions: *const ThawNativeFunctionArgument,
    function_count: usize,
) -> ThawNapiHandleResult {
    let result = (|| -> Result<u64, String> {
        Ok(call_export_with_functions(name, args, functions, function_count)? as u64)
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
pub unsafe extern "C" fn thaw_napi_call_with_functions_typed_result(
    name: *const c_char,
    args: *const c_char,
    functions: *const ThawNativeFunctionArgument,
    function_count: usize,
) -> ThawResult {
    text_result(
        call_export_with_functions(name, args, functions, function_count).and_then(|value| {
            serde_json::to_string(&json_from_value_with_undefined(value, true)?)
                .map_err(|error| error.to_string())
        }),
    )
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

unsafe fn call_method_value_impl(
    receiver: u64,
    method: *const c_char,
    args: *const c_char,
    preserve_undefined: bool,
) -> Result<NapiValue, String> {
    (|| -> Result<NapiValue, String> {
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
        let value = callback_result(env, (function.callback)(env, &mut info));
        take_env_exception(env)?;
        wait_for_promise(value)
    })()
}

unsafe fn call_method_impl(
    receiver: u64,
    method: *const c_char,
    args: *const c_char,
    preserve_undefined: bool,
) -> ThawResult {
    text_result(call_method_value_impl(receiver, method, args, preserve_undefined)
        .and_then(|value| json_from_value_with_undefined(value, preserve_undefined))
        .and_then(|value| serde_json::to_string(&value).map_err(|error| error.to_string())))
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

#[cfg(feature = "quickjs")]
unsafe fn callback_native_handle_paths(args: &[NapiValue]) -> Result<Vec<JsonValue>, String> {
    unsafe fn collect(value: NapiValue, path: &mut Vec<JsonValue>, paths: &mut Vec<JsonValue>) -> Result<(), String> {
        match value_ref(value).map_err(|_| "invalid napi_value")? {
            Value::Object(_) if is_native_instance(value as usize) => {
                paths.push(serde_json::json!([path, (value as u64).to_string()]));
            }
            Value::Array(values) => {
                for (index, value) in values.iter().enumerate() {
                    if let Some(value) = value {
                        path.push(JsonValue::from(index));
                        collect(*value, path, paths)?;
                        path.pop();
                    }
                }
            }
            Value::Object(fields) => {
                for (key, value) in fields {
                    if let PropertyKey::String(key) = key {
                        path.push(JsonValue::String(key.clone()));
                        collect(*value, path, paths)?;
                        path.pop();
                    }
                }
            }
            _ => {}
        }
        Ok(())
    }
    let mut paths = Vec::new();
    for (index, value) in args.iter().enumerate() {
        collect(*value, &mut vec![JsonValue::from(index)], &mut paths)?;
    }
    Ok(paths)
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
            args.and_then(|mut args| {
                let paths = callback_native_handle_paths(&info.args)?;
                let mut meta = value_ref(info.this_arg)
                    .ok()
                    .filter(|value| is_object_value(value))
                    .map(|_| serde_json::json!({
                        "__thaw_napi_this_handle__": (info.this_arg as u64).to_string()
                    }))
                    .unwrap_or(JsonValue::Null);
                if !paths.is_empty() {
                    if meta.is_null() { meta = serde_json::json!({}); }
                    meta.as_object_mut().unwrap().insert(
                        "__thaw_napi_argument_handles__".into(), JsonValue::Array(paths));
                }
                args.insert(0, meta);
                Ok(args)
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
        let Ok(result) = text(result) else { return ptr::null_mut(); };
        if let Some(message) = result.strip_prefix('\u{2}') {
            let env = env_mut(_env).unwrap();
            let error = if _is_quickjs {
                let message = serde_json::from_str::<String>(message)
                    .unwrap_or_else(|_| "invalid QuickJS callback error response".into());
                let (name, body) = message.strip_prefix('\u{1}')
                    .and_then(|tagged| tagged.split_once('\u{1}'))
                    .filter(|(name, _)| !name.is_empty())
                    .unwrap_or(("Error", message.as_str()));
                alloc_error(env, name, body.into(), None)
            } else {
                env.alloc(Value::Error(message.into()))
            };
            env.exception = Some(error);
            return ptr::null_mut();
        }
        let Ok(result) = serde_json::from_str(&result) else { return ptr::null_mut(); };
        return match value_from_json_with_undefined(env_mut(_env).unwrap(), &result, true) {
            Ok(value) => value,
            Err(message) => {
                let env = env_mut(_env).unwrap();
                env.exception = Some(env.alloc(Value::Error(message)));
                ptr::null_mut()
            }
        };
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
        let (function, env_ptr) = HOST
            .with(|host| {
                let host = host.borrow();
                Some((host.functions.get(&name)?.clone(), host.exports.get(&name)?.0 as NapiEnv))
            })
            .ok_or_else(|| format!("no such native addon function `{name}`"))?;
        let callback = callback.ok_or("native addon callback is null")?;
        // Generated call sites create a small ABI adapter per invocation,
        // while the closure allocation itself remains stable. Use that
        // closure context as the identity so subscribe/unsubscribe calls made
        // at different source locations still receive the same napi_value.
        let callback_key = (
            env_ptr as usize,
            context as usize,
            if context.is_null() {
                callback as usize
            } else {
                0
            },
        );
        let mut values: Vec<NapiValue> = {
            let env = env_mut(env_ptr).map_err(|_| "invalid native addon environment")?;
            args.iter().map(|value| value_from_json(&mut *env, value)).collect::<Result<Vec<_>, _>>()?
        };
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
            let value = env_mut(env_ptr)
                .map_err(|_| "invalid native addon environment")?
                .alloc(Value::Function(Function {
                    callback: thaw_compiled_callback,
                    data: bridge_data,
                    properties: HashMap::new(),
                    _thaw_bridge: Some(bridge),
                }));
            created_callback = Some(value);
            value
        };
        values.push(callback_value);
        let this_arg = env_mut(env_ptr)
            .map_err(|_| "invalid native addon environment")?
            .alloc(Value::Undefined);
        let mut info = CallbackInfo {
            args: values,
            this_arg,
            new_target: ptr::null_mut(),
            data: function.data,
        };
        let value = callback_result(env_ptr, (function.callback)(env_ptr, &mut info));
        if let Some(exception) = (*env_ptr).exception.take() {
            return Err(describe_env_exception(env_ptr, exception)?);
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
        } else {
            let pending_envs = HOST.with(|host| {
                let mut host = host.borrow_mut();
                host.compiled_callbacks.remove(&callback_key);
                std::mem::take(&mut host.pending_call_envs)
            });
            drop(pending_envs);
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
        thaw_napi_poll_async_work();
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
            env as usize,
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
            .collect::<Result<Vec<_>, _>>()?;
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
        let value = callback_result(env, (function.callback)(env, &mut info));
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
