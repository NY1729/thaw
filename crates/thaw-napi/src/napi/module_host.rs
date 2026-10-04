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
    // An export replaced by a non-function must also retire the older
    // callable entry; shutdown uses the exports owner to unpublish callbacks.
    for (name, _) in &exported_values {
        host.functions.remove(name);
        if let Some(package) = package_name {
            host.functions.remove(&format!("{package}::{name}"));
        }
    }
    if let Some(package) = package_name {
        host.qualified_packages.insert(package.into());
        host.functions.extend(functions.iter().map(|(name, function)|
            (format!("{package}::{name}"), function.clone())));
        host.exports.extend(exported_values.iter().map(|(name, value)|
            (format!("{package}::{name}"), (env_ptr, *value))));
    } else {
        // Legacy unqualified loaders keep their public names. A qualified
        // load must not overwrite another package's bare export.
        host.functions.extend(functions);
        host.exports.extend(exported_values.into_iter().map(|(name, value)|
            (name, (env_ptr, value))));
    }
}

thread_local! {
    static NEXT_GRAPH_OWNER_ID: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

fn next_graph_owner_id() -> Result<u64, String> {
    NEXT_GRAPH_OWNER_ID.with(|next| {
        let id = next.get().checked_add(1)
            .ok_or_else(|| "native addon owner ID limit exceeded".to_string())?;
        next.set(id);
        Ok(id)
    })
}

unsafe fn load_impl(path: &str, root_name: Option<&str>, package_name: Option<&str>) -> Result<(), String> {
    // A failed initialization can leave async cleanup hooks and a Host-owned
    // Env. Register the Worker retirement seam before any fallible load step.
    #[cfg(feature = "quickjs")]
    thaw_quickjs::register_napi_shutdown_bridge(
        thaw_napi_begin_shutdown,
        thaw_napi_poll_shutdown,
        thaw_napi_take_shutdown_error,
        thaw_napi_finish_shutdown,
    );
    if HOST.with(|host| host.borrow().unloading) {
        return Err("cannot load N-API addons while unloading".into());
    }
    unsafe {
        thaw_json_register_napi_handle_operations(
            retain_napi_graph_handle, release_napi_graph_reference,
        );
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
    // A failed addon load or init can still have exposed libuv callbacks.
    // Keep this reference with Host from the moment symbols become visible.
    #[cfg(target_os = "linux")]
    if !uv_handle.is_null() {
        HOST.with(|host| host.borrow_mut().libraries.push(uv_handle));
        record_process_default_uv_loop();
    }
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
    env.graph_owner_id = next_graph_owner_id()?;
    // Both success and failed-init paths publish this Box into Host before
    // cleanup. Graph pins begin only after publication to the Host owner list.
    env.host_managed = true;
    let absolute_path = std::fs::canonicalize(path_text.as_ref())
        .unwrap_or_else(|_| std::path::PathBuf::from(path_text.as_ref()));
    env.module_file_name = CString::new(format!("file://{}", absolute_path.to_string_lossy()))
        .unwrap_or_else(|_| CString::new("").unwrap());
    let env_ptr = &mut *env as NapiEnv;
    let exports = env.alloc(Value::Object(HashMap::new()));
    let returned = {
        let _dispatch = ForeignCallbackGuard::new();
        init(env_ptr, exports)
    };
    let exports = if returned.is_null() {
        exports
    } else {
        returned
    };
    let initialized = (|| -> Result<_, String> {
        if let Some(exception) = env.exception.take() {
            let message = describe_env_exception(env_ptr, exception)?;
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
            // Publish stable Env and library ownership before cleanup callbacks
            // run; they may remove hooks synchronously or reenter the host.
            env.shutdown_requested = true;
            HOST.with(|host| {
                let mut host = host.borrow_mut();
                host.module_envs.push(env);
                host.libraries.push(handle);
            });
            retire_owned_envs();
            return Err(error);
        }
    };
    HOST.with(|host| {
        let mut host = host.borrow_mut();
        register_loaded_exports(&mut host, env_ptr as usize, package_name, functions, exported_values);
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
        thaw_napi_graph_owner,
        release_napi_graph_reference,
    );
    Ok(())
}

#[cfg(feature = "quickjs")]
unsafe extern "C" fn thaw_napi_call_typed_bridge(
    name: *const c_char,
    args: *const c_char,
) -> *const c_char {
    let _dispatch = ForeignCallbackGuard::new();
    let result = text(name).and_then(|name| text(args).and_then(|args| {
        let value = call_value_impl(&name, &args, true, false, None)?;
        quickjs_bridge_value(value).and_then(|wire| serde_json::to_string(&quickjs_wire_result("value", wire)).map_err(|error| error.to_string()))
    }));
    let result = result.unwrap_or_else(|error| serde_json::json!({ "__thaw_error__": error }).to_string());
    CString::new(result).unwrap_or_default().into_raw()
}

#[cfg(feature = "quickjs")]
unsafe extern "C" fn thaw_napi_export_names() -> *const c_char {
    let exports = HOST.with(|host| {
        let host = host.borrow();
        let owners = host.exports.iter().filter_map(|(name, (owner, _))| {
            host.module_envs.iter()
                .find(|env| (&***env as *const Env as usize) == *owner
                    && !env.finalizing && !env.finalized && !env.shutdown_requested)
                .map(|env| (name.clone(), env.graph_owner_id.to_string()))
        }).collect::<HashMap<_, _>>();
        serde_json::json!({
            "names": host.functions.keys().collect::<Vec<_>>(),
            "qualifiedPackages": host.qualified_packages.iter().collect::<Vec<_>>(),
            "ownerByExportName": owners,
        })
    });
    CString::new(exports.to_string())
        .unwrap_or_default()
        .into_raw()
}

#[cfg(feature = "quickjs")]
unsafe fn symbol_from_js_owner(
    env: NapiEnv, name: String, args: &str,
) -> Result<serde_json::Value, String> {
    let options = serde_json::from_str::<Vec<JsonValue>>(args)
        .map_err(|error| format!("invalid symbol options: {error}"))?;
    let global = options.first().and_then(JsonValue::as_bool).unwrap_or(false);
    let permanent = global || options.get(2).and_then(JsonValue::as_bool).unwrap_or(false);
    if let Some(id) = options.get(1).and_then(JsonValue::as_str)
        .and_then(|id| id.parse::<u64>().ok()) {
        let owner = env_mut(env).map_err(|_| "invalid native addon environment")?;
        if !owner.symbols.contains_key(&id) {
            let symbol = owner.alloc(Value::Symbol { id, description: name });
            owner.symbols.insert(id, symbol);
            if owner.active_handle_scopes.is_empty() {
                if let Some(generation) = owner.value_generations.get(&(symbol as usize)).copied() {
                    owner.pending_scope_values.push((symbol, generation));
                }
            }
        }
        owner.js_original_symbol_ids.insert(id);
        if permanent { owner.strong_symbol_ids.insert(id); }
        else {
            owner.js_origin_symbol_ids.insert(id);
            if !owner.js_live_symbol_ids.contains(&id) {
                owner.js_registering_symbol_ids.insert(id);
            }
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
    let id = *id;
    let owner = env_mut(env).map_err(|_| "invalid native addon environment")?;
    owner.js_original_symbol_ids.insert(id);
    if permanent { owner.strong_symbol_ids.insert(id); }
    else {
        owner.js_origin_symbol_ids.insert(id);
        owner.js_registering_symbol_ids.insert(id);
    }
    if owner.active_handle_scopes.is_empty() {
        if let Some(generation) = owner.value_generations.get(&(symbol as usize)).copied() {
            owner.pending_scope_values.push((symbol, generation));
        }
    }
    Ok(serde_json::json!({ "kind": "value", "value": id.to_string() }))
}

#[cfg(feature = "quickjs")]
unsafe extern "C" fn thaw_napi_handle_bridge(
    operation: *const c_char,
    target: *const c_char,
    name: *const c_char,
    args: *const c_char,
) -> *const c_char {
    let _dispatch = ForeignCallbackGuard::new();
    let result = (|| -> Result<serde_json::Value, String> {
        let operation = text(operation)?;
        let target = text(target)?;
        let name = text(name)?;
        if operation == "symbol_graph" {
            let owner_id = target.parse::<u64>()
                .map_err(|_| "invalid JavaScript graph Symbol owner")?;
            let env = HOST.try_with(|host| {
                let host = host.try_borrow().ok()?;
                host.module_envs.iter().chain(host.pending_call_envs.iter())
                    .find(|env| env.graph_owner_id == owner_id
                        && !env.finalizing && !env.finalized && !env.shutdown_requested)
                    .map(|env| (&**env as *const Env).cast_mut())
            }).ok().flatten().ok_or("JavaScript graph Symbol owner is unavailable")?;
            return unsafe { symbol_from_js_owner(env, name, &text(args)?) };
        }
        if operation == "symbol_collected" || operation == "symbol_pin"
            || operation == "symbol_rollback" {
            let owner_id = target.parse::<u64>()
                .map_err(|_| "invalid JavaScript Symbol owner")?;
            let symbol_id = name.parse::<u64>()
                .map_err(|_| "invalid JavaScript Symbol ID")?;
            let weak_state = thaw_quickjs::thaw_js_napi_symbol_weak_live(owner_id, symbol_id);
            if (operation == "symbol_collected" && weak_state != 0)
                || (operation == "symbol_pin" && weak_state != 1) {
                return Err("JavaScript Symbol weak state is not confirmed".into());
            }
            let env = HOST.try_with(|host| {
                let host = host.try_borrow().ok()?;
                host.module_envs.iter().chain(host.pending_call_envs.iter())
                    .find(|env| env.graph_owner_id == owner_id
                        && !env.finalizing && !env.finalized)
                    .map(|env| (&**env as *const Env).cast_mut())
            }).ok().flatten().ok_or("JavaScript Symbol owner is unavailable")?;
            let symbol = unsafe { &*env }.symbols.get(&symbol_id).copied()
                .ok_or("JavaScript Symbol ID is unavailable")?;
            if !unsafe { &*env }.js_origin_symbol_ids.contains(&symbol_id)
                || !unsafe { &*env }.values.contains(&symbol) {
                return Err("JavaScript Symbol owner no longer matches".into());
            }
            if operation == "symbol_rollback"
                && !unsafe { &*env }.js_registering_symbol_ids.contains(&symbol_id) {
                return Err("JavaScript Symbol registration is not pending".into());
            }
            // IDs are monotonic and never reused. A finalization notice is
            // the only proof that a JS-local Symbol lost its weak target;
            // query failures never clear this conservative live record.
            if operation == "symbol_collected" {
                unsafe { (*env).js_live_symbol_ids.remove(&symbol_id); }
            } else if operation == "symbol_pin" {
                unsafe { (*env).js_live_symbol_ids.insert(symbol_id); }
            }
            unsafe { (*env).js_registering_symbol_ids.remove(&symbol_id); }
            unsafe { sync_js_origin_symbol_pins(env); }
            if operation != "symbol_pin" {
                unsafe { sweep_pending_scope_values(env); }
            }
            return Ok(serde_json::json!({ "kind": "value", "value": true }));
        }
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
            let env = live_graph_owner_env(handle)
                .ok_or("native addon handle is no longer live")?;
            let count = unsafe { (*env).proxy_live_counts.get(&(handle as usize)).copied().unwrap_or(0) };
            if count > 1 {
                unsafe { (*env).proxy_live_counts.insert(handle as usize, count - 1); }
            } else {
                unsafe { (*env).proxy_live_counts.remove(&(handle as usize)); }
                release_napi_handle(handle)?;
                unsafe { sweep_pending_scope_values(env); }
            }
            return Ok(serde_json::json!({ "kind": "value", "value": true }));
        }
        if operation == "renew_handle" {
            let handle = target.parse::<u64>()
                .map_err(|_| "invalid native addon handle")?;
            let env = live_graph_owner_env(handle)
                .ok_or("native addon handle is no longer live")?;
            let owner = unsafe { &mut *env };
            let count = owner.proxy_live_counts.get(&(handle as usize)).copied().unwrap_or(0);
            owner.proxy_live_counts.insert(handle as usize, count.checked_add(1)
                .ok_or("native addon proxy owner count exceeded")?);
            owner.released_handles.remove(&(handle as usize));
            return Ok(serde_json::json!({ "kind": "value", "value": true }));
        }
        if operation == "retain_graph_handle" {
            let handle = target.parse::<u64>()
                .map_err(|_| "invalid native graph handle")?;
            let reference = retain_napi_graph_handle(handle);
            if reference == 0 { return Err("native graph handle is no longer live".into()); }
            return Ok(serde_json::json!({ "kind": "value", "value": reference.to_string() }));
        }
        if operation == "symbol_metadata" {
            let handle = target.parse::<u64>()
                .map_err(|_| "invalid native Symbol handle")?;
            let env = live_graph_owner_env(handle)
                .ok_or("native Symbol graph handle is no longer live")?;
            let symbol = handle as NapiValue;
            let (id, description) = match value_ref(symbol) {
                Ok(Value::Symbol { id, description }) => (*id, description.clone()),
                _ => return Err("native graph handle is not a Symbol".into()),
            };
            let units = env.as_ref().and_then(|owner|
                owner.utf16_symbols.get(&(symbol as usize))).cloned()
                .unwrap_or_else(|| description.encode_utf16().collect());
            let registered = GLOBAL_SYMBOLS.get().is_some_and(|symbols| {
                symbols.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
                    .get(&units) == Some(&id)
            });
            let js_origin = env.as_ref().is_some_and(|owner|
                owner.symbols.get(&id) == Some(&symbol)
                    && owner.js_original_symbol_ids.contains(&id));
            return Ok(serde_json::json!({ "kind": "value", "value": {
                "id": id.to_string(), "handle": handle.to_string(),
                "units": units, "registered": registered, "jsOrigin": js_origin
            } }));
        }
        if operation == "symbol_handle" {
            let id = target.parse::<u64>()
                .map_err(|_| "invalid native Symbol identity")?;
            let handle = HOST.with(|host| {
                let host = host.borrow();
                host.module_envs.iter().chain(host.pending_call_envs.iter())
                    .filter(|env| !env.finalizing && !env.finalized
                        && !env.shutdown_requested && env.graph_owner_id != 0)
                    .filter_map(|env| env.symbols.get(&id)
                        .copied().filter(|value|
                            !env.finalized_handles.contains(&(*value as usize))
                                && matches!(unsafe { (*value).as_ref() },
                                    Some(Value::Symbol { id: actual, .. }) if *actual == id))
                        .map(|value| value as u64))
                    .next()
            }).ok_or("native Symbol identity has no live owner")?;
            return Ok(serde_json::json!({ "kind": "value", "value": handle.to_string() }));
        }
        if operation == "sync_reference" {
            let reference = target
                .parse::<u64>()
                .map_err(|_| "invalid QuickJS reference")?;
            let (env, value) = HOST.with(|host| {
                let host = host.borrow();
                let env = host.module_envs.iter()
                    .find(|env| !env.finalized && env.quickjs_references.contains_key(&reference))
                    .ok_or_else(|| "unknown QuickJS reference".to_string())?;
                Ok::<_, String>((&**env as *const Env, env.quickjs_references[&reference]))
            })?;
            // The outer dispatch guard pins the owner through this encode;
            // no HOST borrow survives a future accessor callback.
            return Ok(quickjs_wire_result("value", quickjs_reference_wire(env as NapiEnv, value, true)?));
        }
        if operation == "sync_handle" {
            let handle = target.parse::<u64>().map_err(|_| "invalid native binary handle")?;
            let env = module_env_for_handle(handle)?;
            let wire = quickjs_reference_wire(env, handle as NapiValue, true)?;
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
            // A settled value may contain an accessor that reenters deferred settlement.
            // Copy the handle before bridging so no RefCell borrow spans that callback.
            let settled = {
                let state = state.borrow();
                match *state {
                    PromiseState::Pending => None,
                    PromiseState::Resolved(value) => Some(("resolved", value)),
                    PromiseState::Rejected(value) => Some(("rejected", value)),
                }
            };
            return match settled {
                Some((kind, value)) => Ok(quickjs_wire_result(kind, quickjs_bridge_value(value)?)),
                None => Ok(serde_json::json!({ "kind": "pending" })),
            };
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
        if operation == "array_kind_graph" {
            let _ = module_env_for_handle(handle)?;
            return Ok(serde_json::json!({ "kind": "value", "value":
                matches!(value_ref(handle as NapiValue), Ok(Value::Array(_))) }));
        }
        if matches!(operation.as_str(), "get_graph" | "set_graph" | "has_graph"
            | "own_keys_graph" | "descriptor_graph" | "delete_graph"
            | "get_prototype_graph" | "set_prototype_graph"
            | "prevent_extensions_graph" | "is_extensible_graph"
            | "define_accessor_graph" | "define_data_graph"
            | "reconfigure_graph") {
            // A property key may be an exact UTF-16 string or Symbol. Decode
            // it through the same graph/lease owner as callback arguments,
            // before any user getter or setter can reenter addon retirement.
            let graph = text(args)?;
            let input = NapiGraphInput::parse(&graph);
            let env = module_env_for_handle(handle)?;
            let cached_before = if operation == "define_accessor_graph" {
                env.as_ref().ok_or("invalid native addon environment")?
                    .quickjs_live_values.keys().copied().collect::<HashSet<_>>()
            } else { HashSet::new() };
            let values = parse_napi_arguments(env, &graph, true, true, Some(&input))?;
            let object = handle as NapiValue;
            if operation == "is_extensible_graph" || operation == "prevent_extensions_graph" {
                if !values.is_empty() { return Err("native integrity query expects no arguments".into()); }
                let owner = env_mut(env).map_err(|_| "invalid native addon environment")?;
                let identity = object as usize;
                if operation == "prevent_extensions_graph" {
                    owner.nonextensible_objects.insert(identity);
                }
                return Ok(serde_json::json!({"kind": "value",
                    "value": operation == "prevent_extensions_graph"
                        || !owner.nonextensible_objects.contains(&identity)}));
            }
            if operation == "reconfigure_graph" {
                let [key, flags] = values.as_slice() else {
                    return Err("native descriptor reconfiguration expects key and flags".into());
                };
                let key = property_key(env, *key)
                    .map_err(|status| format!("invalid native descriptor key: status {status}"))?;
                let flags = match value_ref(*flags).map_err(|_| "invalid native descriptor flags")? {
                    Value::Number(number) if number.is_finite() && *number >= 0.0
                        && number.fract() == 0.0 && *number <= 7.0 => *number as u32,
                    _ => return Err("invalid native descriptor attributes".into()),
                };
                let identity = object as usize;
                let old = property_attributes_for(env, identity, &key);
                let has_data = own_property_value(env, object, &key).is_some();
                let has_accessor = accessor_for_owner(env, identity, &key).is_some();
                let owner = env_mut(env).map_err(|_| "invalid native addon environment")?;
                let exists = has_data || has_accessor;
                let readonly_transition = old & NAPI_CONFIGURABLE == 0
                    && old & NAPI_WRITABLE != 0 && flags & NAPI_WRITABLE == 0
                    && (old ^ flags) & (NAPI_ENUMERABLE | NAPI_CONFIGURABLE) == 0
                    && !has_accessor;
                let changed = exists && (old & NAPI_CONFIGURABLE != 0
                    || old == flags || readonly_transition);
                if changed { owner.property_attributes.insert((identity, key), flags); }
                return Ok(serde_json::json!({"kind": "value", "value": changed}));
            }
            if operation == "define_data_graph" {
                let [key, value, flags, presence] = values.as_slice() else {
                    return Err("native data definition expects key, value, flags and presence".into());
                };
                let key = property_key(env, *key)
                    .map_err(|status| format!("invalid native data key: status {status}"))?;
                let flags = match value_ref(*flags).map_err(|_| "invalid native data flags")? {
                    Value::Number(number) if number.is_finite() && *number >= 0.0
                        && number.fract() == 0.0 && *number <= 7.0 => *number as u32,
                    _ => return Err("invalid native data attributes".into()),
                };
                let presence = match value_ref(*presence).map_err(|_| "invalid native data presence")? {
                    Value::Number(number) if *number == 0.0 || *number == 1.0 => *number != 0.0,
                    _ => return Err("invalid native data presence".into()),
                };
                let mut scope_sweep = ScopeMutationSweep::new(env);
                // ArraySetLength can publish a partial truncation even when
                // a later nonconfigurable index makes the definition fail.
                if is_array_length_property(object, &key) { scope_sweep.changed(); }
                let status = qjs_install_native_data_property(env, object, key,
                    *value, presence, flags);
                if status == NAPI_OK { scope_sweep.changed(); }
                if let Some(exception) = take_env_exception_graph(env)? { return Ok(exception); }
                return Ok(serde_json::json!({"kind": "value", "value": status == NAPI_OK}));
            }
            if operation == "define_accessor_graph" {
                let [key, getter, setter, flags, presence] = values.as_slice() else {
                    return Err("native accessor definition expects key, get, set, flags and presence".into());
                };
                let key = property_key(env, *key)
                    .map_err(|status| format!("invalid native accessor key: status {status}"))?;
                let callback = |value: NapiValue| -> Result<(u64, NapiValue), String> {
                    match value_ref(value).map_err(|_| "invalid native accessor callback")? {
                        Value::Undefined => Ok((0, ptr::null_mut())),
                        Value::QuickJsHandle { handle, .. } => Ok((*handle, ptr::null_mut())),
                        Value::Function(_) => Ok((0, value)),
                        _ => Err("native graph accessor is not a JavaScript Function".into()),
                    }
                };
                let (getter, getter_native) = callback(*getter)?;
                let (setter, setter_native) = callback(*setter)?;
                let flags = match value_ref(*flags).map_err(|_| "invalid native accessor flags")? {
                    Value::Number(value) if value.is_finite() && *value >= 0.0
                        && value.fract() == 0.0 && *value <= 7.0 => *value as u32,
                    _ => return Err("invalid native accessor attributes".into()),
                };
                let presence = match value_ref(*presence).map_err(|_| "invalid native accessor presence")? {
                    Value::Number(value) if *value >= 1.0 && *value <= 3.0
                        && value.fract() == 0.0 => *value as u8,
                    _ => return Err("invalid native accessor presence".into()),
                };
                // Own each retain immediately. Any later validation, Env
                // lookup, graph-carrier cleanup, or descriptor install can
                // fail and will then release every acquired root.
                // The graph decoder returns a raw native Function only when
                // it belongs to this Env. Foreign Functions arrive as
                // retained QuickJsHandle values instead. A same-Env positive
                // Reference here would cycle with the descriptor table and
                // prevent Env retirement.
                let mut roots = Arc::new(QuickJsAccessorRoots {
                    getter: 0, setter: 0, getter_native, setter_native,
                });
                for (index, handle) in [getter, setter].into_iter().enumerate() {
                    if handle == 0 { continue; }
                    if thaw_quickjs::thaw_js_retain_handle(handle) == 0 {
                        return Err("released JavaScript accessor callback".into());
                    }
                    let owner = Arc::get_mut(&mut roots).unwrap();
                    if index == 0 { owner.getter = handle; }
                    else { owner.setter = handle; }
                }
                // The graph decoder needed carriers to validate this packet,
                // but descriptor callbacks have their own Arc roots. Remove
                // only newly-created callback-only carriers; preexisting Env
                // values may be used elsewhere. Release registry handles after
                // the Env borrow and before any accessor installation reentry.
                let temporary = {
                    let owner = env_mut(env).map_err(|_| "invalid native addon environment")?;
                    [getter, setter].into_iter().filter(|id| *id != 0)
                        .filter(|id| !cached_before.contains(id))
                        .collect::<HashSet<_>>().into_iter().filter_map(|id| {
                            let value = owner.quickjs_live_values.remove(&id)?;
                            *value = Value::Undefined;
                            Some(id)
                        }).collect::<Vec<_>>()
                };
                for id in temporary { thaw_quickjs::thaw_js_release_handle(id); }
                let mut scope_sweep = ScopeMutationSweep::new(env);
                let status = qjs_install_native_accessor(env, object, key,
                    roots,
                    presence & 1 != 0, presence & 2 != 0, flags);
                if status == NAPI_OK { scope_sweep.changed(); }
                if let Some(exception) = take_env_exception_graph(env)? { return Ok(exception); }
                return Ok(serde_json::json!({"kind": "value", "value": status == NAPI_OK}));
            }
            if operation == "get_prototype_graph" {
                if !values.is_empty() { return Err("native prototype read expects no arguments".into()); }
                let mut prototype = ptr::null_mut();
                let status = napi_get_prototype(env, object, &mut prototype);
                if let Some(exception) = take_env_exception_graph(env)? { return Ok(exception); }
                if status != NAPI_OK || prototype.is_null() {
                    return Err(format!("native prototype read failed: status {status}"));
                }
                return Ok(serde_json::json!({"kind": "graph",
                    "value": napi_result_graph_for_env(env, prototype, true)? }));
            }
            if operation == "set_prototype_graph" {
                let [prototype] = values.as_slice() else {
                    return Err("native prototype write expects one value".into());
                };
                let status = node_api_set_prototype(env, object, *prototype);
                if let Some(exception) = take_env_exception_graph(env)? { return Ok(exception); }
                return Ok(serde_json::json!({"kind": "value", "value": status == NAPI_OK}));
            }
            if operation == "own_keys_graph" {
                if !values.is_empty() { return Err("native ownKeys graph expects no arguments".into()); }
                let mut keys = ptr::null_mut();
                let status = napi_get_all_property_names(env, object, NAPI_KEY_OWN_ONLY,
                    NAPI_KEY_ALL_PROPERTIES, NAPI_KEY_NUMBERS_TO_STRINGS, &mut keys);
                if let Some(exception) = take_env_exception_graph(env)? { return Ok(exception); }
                if status != NAPI_OK || keys.is_null() {
                    return Err(format!("native ownKeys graph failed: status {status}"));
                }
                return Ok(serde_json::json!({"kind": "graph",
                    "value": napi_result_graph_for_env(env, keys, true)? }));
            }
            let Some(key) = values.first().copied() else {
                return Err("native property graph has no key".into());
            };
            if operation == "get_graph" {
                if values.is_empty() || values.len() > 2 {
                    return Err("native get graph expects a key and optional receiver".into());
                }
                let native_key = property_key(env, key)
                    .map_err(|status| format!("invalid native get key: status {status}"))?;
                let receiver = values.get(1).copied().unwrap_or(object);
                let mut value = ptr::null_mut();
                let status = get_property_key(env, object, &native_key, &mut value, receiver);
                if let Some(exception) = take_env_exception_graph(env)? { return Ok(exception); }
                if status != NAPI_OK || value.is_null() {
                    return Err(format!("native get graph failed: status {status}"));
                }
                return Ok(serde_json::json!({"kind": "graph",
                    "value": napi_result_graph_for_env(env, value, true)? }));
            }
            if operation == "set_graph" {
                if values.len() < 2 || values.len() > 3 {
                    return Err("native set graph expects key, value and optional receiver".into());
                }
                let native_key = property_key(env, key)
                    .map_err(|status| format!("invalid native set key: status {status}"))?;
                let receiver = values.get(2).copied().unwrap_or(object);
                let status = set_property_key(env, object, native_key, values[1], receiver);
                if let Some(exception) = take_env_exception_graph(env)? { return Ok(exception); }
                // A readonly or nonextensible ordinary assignment reports
                // failure to the Proxy trap. A pending native exception was
                // already returned by take_env_exception_graph above.
                return Ok(serde_json::json!({"kind": "value", "value": status == NAPI_OK}));
            }
            if values.len() != 1 { return Err("native property graph expects one key".into()); }
            if operation == "has_graph" || operation == "delete_graph" {
                let mut result = false;
                let status = if operation == "has_graph" {
                    napi_has_property(env, object, key, &mut result)
                } else {
                    napi_delete_property(env, object, key, &mut result)
                };
                if let Some(exception) = take_env_exception_graph(env)? { return Ok(exception); }
                if status != NAPI_OK {
                    return Err(format!("native property graph failed: status {status}"));
                }
                return Ok(serde_json::json!({"kind": "value", "value": result}));
            }
            let native_key = property_key(env, key)
                .map_err(|status| format!("invalid native descriptor key: status {status}"))?;
            let accessor = accessor_for_owner(env, object as usize, &native_key);
            let own = own_property_value(env, object, &native_key).is_some()
                || accessor.is_some();
            if !own { return Ok(serde_json::json!({"kind": "value", "value": null})); }
            let flags = property_attributes_for(env, object as usize, &native_key);
            let getter_source = match accessor.as_ref() {
                Some(accessor) => qjs_reflected_native_accessor_source(
                    env, object, &native_key, accessor, true)?,
                None => serde_json::Value::Null,
            };
            let setter_source = match accessor.as_ref() {
                Some(accessor) => qjs_reflected_native_accessor_source(
                    env, object, &native_key, accessor, false)?,
                None => serde_json::Value::Null,
            };
            return Ok(serde_json::json!({"kind": "value", "value": {
                "configurable": flags & NAPI_CONFIGURABLE != 0,
                "enumerable": flags & NAPI_ENUMERABLE != 0,
                "writable": flags & NAPI_WRITABLE != 0,
                "accessor": accessor.is_some(),
                "getter": accessor.as_ref().is_some_and(|accessor| accessor.getter.is_some()),
                "setter": accessor.as_ref().is_some_and(|accessor| accessor.setter.is_some()),
                "getter_source": getter_source,
                "setter_source": setter_source,
            }}));
        }
        match operation.as_str() {
            "construct" => {
                let value = construct_handle_impl(handle, args, true, false);
                if value.error.is_null() {
                    Ok(serde_json::json!({ "kind": "handle", "value": value.value.to_string() }))
                } else {
                    let error = thaw_arena::NativeStr::from_ptr(value.error)
                        .to_string_lossy().into_owned();
                    thaw_arena::destroy_string(value.error);
                    Err(error)
                }
            }
            "symbol" => {
                let env = module_env_for_handle(handle)?;
                symbol_from_js_owner(env, name, &text(args)?)
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
                if let Some(exception) = take_env_exception_graph(env)? { return Ok(exception); }
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
                if let Some(exception) = take_env_exception_graph(env)? { return Ok(exception); }
                if status != NAPI_OK { return Err(format!("failed to check native property: status {status}")); }
                Ok(serde_json::json!({ "kind": "value", "value": present }))
            }
            "own_keys" => {
                let env = module_env_for_handle(handle)?;
                let mut keys = ptr::null_mut();
                let status = napi_get_all_property_names(env, handle as NapiValue, NAPI_KEY_OWN_ONLY,
                    NAPI_KEY_ALL_PROPERTIES, NAPI_KEY_NUMBERS_TO_STRINGS, &mut keys);
                if let Some(exception) = take_env_exception_graph(env)? { return Ok(exception); }
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
                let values = module_arguments(env, args, true, false, None)?;
                let function = match value_ref(handle as NapiValue).map_err(|_| "invalid native method")? {
                    Value::Function(function) => function.clone(),
                    _ => return Err("captured native property is not callable".into()),
                };
                let mut info = CallbackInfo { args: values, this_arg: receiver as NapiValue,
                    new_target: ptr::null_mut(), data: function.data };
                let value = callback_result(env, invoke_napi_callback(env, function.callback, &mut info));
                take_env_exception(env)?;
                Ok(quickjs_wire_result("value", quickjs_bridge_value(value)?))
            }
            "call_captured_graph" => {
                let graph = text(args)?;
                let env = match module_env_for_handle(handle) {
                    Ok(env) => env,
                    Err(error) => {
                        release_unconsumed_napi_callback_graph(&graph);
                        return Err(error);
                    }
                };
                // The graph decoder consumes both QuickJS and NAPI transfer
                // leases before user callback code can reenter addon unload.
                let graph_input = NapiGraphInput::parse(&graph);
                let mut values = parse_napi_arguments(
                    env, &graph, true, true, Some(&graph_input))?;
                if values.is_empty() { return Err("captured callback graph lacks this".into()); }
                let receiver = values.remove(0);
                let function = match value_ref(handle as NapiValue)
                    .map_err(|_| "invalid native callback")? {
                    Value::Function(function) => function.clone(),
                    _ => return Err("captured native property is not callable".into()),
                };
                let mut info = CallbackInfo { args: values, this_arg: receiver,
                    new_target: ptr::null_mut(), data: function.data };
                let value = callback_result(env, invoke_napi_callback(env, function.callback, &mut info));
                if let Some(exception) = take_env_exception_graph(env)? { return Ok(exception); }
                Ok(serde_json::json!({ "kind": "graph",
                    "value": napi_result_graph_for_env(env, value, true)? }))
            }
            "construct_captured_graph" => {
                let graph = text(args)?;
                let env = match module_env_for_handle(handle) {
                    Ok(env) => env,
                    Err(error) => {
                        release_unconsumed_napi_callback_graph(&graph);
                        return Err(error);
                    }
                };
                let graph_input = NapiGraphInput::parse(&graph);
                let values = parse_napi_arguments(
                    env, &graph, true, true, Some(&graph_input))?;
                if !matches!(value_ref(handle as NapiValue), Ok(Value::Function(_))) {
                    return Err("captured native constructor is not callable".into());
                }
                let Some((new_target, tail)) = values.split_first() else {
                    return Err("captured constructor graph lacks new.target".into());
                };
                let Some((fallback_prototype, arguments)) = tail.split_first() else {
                    return Err("captured constructor graph lacks realm fallback".into());
                };
                let mut result = ptr::null_mut();
                let status = napi_new_instance_with_target(env, handle as NapiValue,
                    *new_target, *fallback_prototype, arguments.len(), arguments.as_ptr(), &mut result);
                if let Some(exception) = take_env_exception_graph(env)? { return Ok(exception); }
                if status != NAPI_OK {
                    return Err(format!("native constructor failed: status {status}"));
                }
                Ok(serde_json::json!({ "kind": "graph",
                    "value": napi_result_graph_for_env(env, result, true)? }))
            }
            "call" => {
                let name = CString::new(name).map_err(|_| "method contains NUL")?;
                let value = call_method_value_impl(handle, name.as_ptr(), args, true, false)?;
                Ok(quickjs_wire_result("value", quickjs_bridge_value(value)?))
            }
            "set_symbol" => {
                let env = module_env_for_handle(handle)?;
                let id = name.parse::<u64>().map_err(|_| "invalid native Symbol ID")?;
                let symbol = env_mut(env).map_err(|_| "invalid native addon environment")?
                    .symbols.get(&id).copied().ok_or("unknown native Symbol")?;
                let values = module_arguments(env, args, true, false, None)?;
                let [value] = values.as_slice() else { return Err("native property setter expects exactly one value".into()); };
                let status = napi_set_property(env, handle as NapiValue, symbol, *value);
                take_env_exception(env)?;
                if status != NAPI_OK { return Err(format!("failed to set native Symbol property: status {status}")); }
                Ok(serde_json::json!({ "kind": "value", "value": true }))
            }
            "set" => {
                let name = CString::new(name).map_err(|_| "property contains NUL")?;
                let result = set_property_impl(handle, name.as_ptr(), args, true, false, true);
                if result.error.is_null() {
                    Ok(serde_json::json!({ "kind": "value", "value": true }))
                } else {
                    let error = thaw_arena::NativeStr::from_ptr(result.error)
                        .to_string_lossy().into_owned();
                    thaw_arena::destroy_string(result.error);
                    Err(error)
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
                if env.finalized { return None; }
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
        let _dispatch = ForeignCallbackGuard::new();
        for record in finalizers {
            if let Some(finalize) = record.finalize {
                unsafe { finalize(env, record.data, record.hint) };
            }
        }
    }
}

#[cfg(feature = "quickjs")]
fn release_napi_handle(handle: u64) -> Result<(), String> {
    // A live JS proxy is only one owner. A native parent field, active scope,
    // or escaped handle may still reach this Value after the proxy is gone.
    // Let the same graph collector used by scope close decide before the
    // legacy proxy finalizer marks the handle dead.
    let scoped_env = HOST.with(|host| {
        let host = host.borrow();
        host.module_envs.iter().chain(host.pending_call_envs.iter())
            .find(|env| !env.finalized && env.values.contains(&(handle as NapiValue)))
            .and_then(|env| {
                let key = handle as usize;
                let tracked = env.pending_scope_values.iter().any(|(value, generation)|
                    *value as usize == key && env.value_generations.get(&key) == Some(generation))
                    || env.active_handle_scopes.iter().any(|scope| unsafe { &**scope }.values.iter()
                        .any(|(value, generation)| *value as usize == key
                            && env.value_generations.get(&key) == Some(generation)))
                    || env.rooted_scope_escapes.contains(&key);
                tracked.then_some((&**env as *const Env).cast_mut())
            })
    });
    if let Some(env) = scoped_env {
        unsafe { sweep_pending_scope_values(env); }
        if unsafe { !(*env).values.contains(&(handle as NapiValue)) } { return Ok(()); }
        // The candidate remained reachable, so its existing owner will
        // eventually trigger a sweep or Env teardown. Do not finalize now.
        unsafe { (*env).released_handles.insert(handle as usize); }
        return Ok(());
    }
    let released = HOST.with(|host| {
        let mut host = host.borrow_mut();
        let env = host
            .module_envs
            .iter_mut()
            .find(|env| !env.finalized && env.values.contains(&(handle as NapiValue)))
            .ok_or_else(|| "unknown native addon handle".to_string())?;
        let value = handle as NapiValue;
        if env.proxy_live_counts.get(&(handle as usize)).copied().unwrap_or(0) != 0
            || env.references.iter().any(|reference|
                !reference.deleted && reference.value == value && reference.count > 0)
        {
            env.released_handles.insert(handle as usize);
            return Ok::<_, String>(None);
        }
        if !env.finalized_handles.insert(handle as usize) {
            return Ok(None);
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
    let _dispatch = ForeignCallbackGuard::new();
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
pub extern "C" fn thaw_napi_begin_shutdown() -> u8 {
    if ForeignCallbackGuard::active() { return 0; }
    HOST.with(|host| {
        let mut host = host.borrow_mut();
        if !host.unloading {
            host.unloading = true;
            for env in &mut host.module_envs { env.shutdown_requested = true; }
            for env in &mut host.pending_call_envs { env.shutdown_requested = true; }
        }
    });
    // A prior native callback can leave an Env exception pending. Preserve it
    // in FIFO order before cleanup callbacks set their own exceptions; keep
    // every Box in HOST and release the borrow before describing a value.
    let envs = HOST.with(|host| {
        let host = host.borrow();
        host.module_envs.iter().chain(host.pending_call_envs.iter())
            .map(|env| (&**env as *const Env).cast_mut()).collect::<Vec<_>>()
    });
    for env in envs { unsafe { capture_shutdown_exception(env) }; }
    // Hooks and finalizers may reenter HOST; do not keep its borrow here.
    retire_owned_envs();
    1
}

/// One bounded progress step. A zero result means the owner must continue
/// polling and reporting errors while every Env and library stays mapped.
#[no_mangle]
pub extern "C" fn thaw_napi_poll_shutdown() -> u8 {
    if ForeignCallbackGuard::active() || !HOST.with(|host| host.borrow().unloading) {
        return 0;
    }
    thaw_napi_poll_async_work();
    retire_owned_envs();
    let owners_ready = HOST.with(|host| {
        let host = host.borrow();
        !host.has_active_async_work()
            && !host.has_active_cleanup()
            && host.module_envs.iter().chain(host.pending_call_envs.iter()).all(|env| env.finalized)
    });
    u8::from(owners_ready && !host_threadsafe_state(false) && registered_uv_loops().is_empty() && unsafe { !uv_handles_open() })
}

#[cfg(target_os = "linux")]
unsafe fn close_owned_uv_loop() -> bool {
    type UvLoopClose = unsafe extern "C" fn(*mut c_void) -> i32;
    let Some(address) = HOST.with(|host| host.borrow().owned_uv_loop) else { return true };
    let symbol = libc::dlsym(libc::RTLD_DEFAULT, c"uv_loop_close".as_ptr());
    if symbol.is_null() { return false; }
    let close = std::mem::transmute::<*mut c_void, UvLoopClose>(symbol);
    if close(address as *mut c_void) != 0 { return false; }
    HOST.with(|host| host.borrow_mut().owned_uv_loop = None);
    libc::free(address as *mut c_void);
    true
}

#[cfg(not(target_os = "linux"))]
unsafe fn close_owned_uv_loop() -> bool { true }

/// Native library destructors may reenter N-API during dlclose. A failed
/// barrier retains Env Boxes and libraries for another poll/close attempt.
#[no_mangle]
pub extern "C" fn thaw_napi_finish_shutdown() -> u8 {
    if ForeignCallbackGuard::active() { return 0; }
    if !HOST.with(|host| host.borrow().unloading) { return 0; }
    if HOST.with(|host| {
        let host = host.borrow();
        host.has_active_async_work() || host.has_active_cleanup()
    }) || host_threadsafe_state(false)
        || !registered_uv_loops().is_empty()
        || unsafe { uv_handles_open() }
    {
        HOST.with(|host| host.borrow_mut().last_error =
            "cannot release N-API addons while native callbacks or libuv handles remain".into());
        return 0;
    }
    if !HOST.with(|host| {
        let host = host.borrow();
        host.shutdown_errors.is_empty()
            && host.module_envs.iter().chain(host.pending_call_envs.iter()).all(|env| env.finalized)
    }) {
        HOST.with(|host| host.borrow_mut().last_error =
            "N-API shutdown still has owners or unreported errors".into());
        return 0;
    }
    if !unsafe { close_owned_uv_loop() } {
        HOST.with(|host| host.borrow_mut().last_error =
            "N-API host-owned libuv loop could not close".into());
        return 0;
    }
    // dlclose may run addon destructors. Pin the entire release epoch so a
    // nested shutdown cannot clear `unloading` or free the outer Env batch.
    let _dispatch = ForeignCallbackGuard::new();
    let Some((envs, libraries, embedded_files)) = HOST.with(|host| {
        let mut host = host.borrow_mut();
        if !host.shutdown_errors.is_empty()
            || host.module_envs.iter().chain(host.pending_call_envs.iter()).any(|env| !env.finalized)
        {
            host.last_error = "N-API shutdown still has owners or unreported errors".into();
            return None;
        }
        host.functions.clear();
        host.exports.clear();
        host.qualified_packages.clear();
        host.compiled_callbacks.clear();
        let mut envs = std::mem::take(&mut host.pending_call_envs);
        envs.extend(std::mem::take(&mut host.module_envs));
        host.main_default_uv_loop = None;
        let libraries = std::mem::take(&mut host.libraries);
        let embedded_files = std::mem::take(&mut host.embedded_files);
        Some((envs, libraries, embedded_files))
    }) else { return 0; };
    // Library destructors can still hold raw napi_env pointers. Keep the
    // finalized Boxes allocated through dlclose so those calls fail with a
    // stable, closed Env instead of dereferencing freed memory.
    for handle in libraries.into_iter().rev() { unsafe { libc::dlclose(handle) }; }
    drop(envs);
    drop(embedded_files);
    HOST.with(|host| host.borrow_mut().unloading = false);
    1
}

/// Compatibility entry. Generated main and Workers should use the phased
/// API so they can report errors while addon code remains mapped.
#[no_mangle]
pub extern "C" fn thaw_napi_unload_all() -> u8 {
    if ForeignCallbackGuard::active() { return 0; }
    // A first call may stop at an unreferenced live uv handle or TSFN.
    // Begin is idempotent; a later caller must resume the same pinned Host.
    if thaw_napi_begin_shutdown() == 0 { return 0; }
    loop {
        if thaw_napi_poll_shutdown() != 0 { break; }
        // A live TSFN may need an external owner to release it, even while
        // an async cleanup hook is pending. One poll has already delivered all
        // ready callbacks; return a retryable 0 instead of blocking that owner.
        if host_threadsafe_state(false) { return 0; }
        if HOST.with(|host| {
            let host = host.borrow();
            !host.has_active_async_work() && !host.has_active_cleanup()
        }) {
            return 0;
        }
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    let errors = thaw_napi_report_shutdown_errors();
    let released = thaw_napi_finish_shutdown();
    u8::from(released != 0 && errors == 0)
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
                    _thaw_bridge: Some(bridge), _accessor_owner: None,
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

// The private argument envelope carries kind and identity outside user object
// fields. Only graph sibling exports call this decoder; the public JSON ABI
// continues to treat its input as ordinary JSON.
fn decimal_bigint_words(decimal: &str) -> Option<(bool, Vec<u64>)> {
    let negative = decimal.starts_with('-');
    let digits = decimal.strip_prefix('-').unwrap_or(decimal);
    if digits.is_empty() || (digits != "0" && digits.starts_with('0'))
        || (negative && digits == "0") || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let mut words = vec![0_u64];
    for digit in digits.bytes() {
        let mut carry = u128::from(digit - b'0');
        for word in &mut words {
            let value = u128::from(*word) * 10 + carry;
            *word = value as u64;
            carry = value >> 64;
        }
        if carry != 0 { words.push(carry as u64); }
    }
    Some((negative, words))
}

fn napi_graph_utf16_units(value: &JsonValue) -> Result<Vec<u16>, String> {
    let units = value.as_array().ok_or("invalid native UTF-16 graph units")?;
    units.iter().map(|unit| unit.as_u64().and_then(|unit| u16::try_from(unit).ok())
        .ok_or_else(|| "invalid native UTF-16 graph unit".into())).collect()
}

// A graph token's decimal native handle is untrusted until it resolves to a
// live owner. In particular, never dereference a foreign Symbol key merely
// because its paired QuickJS wrapper passed the independent JS-side check.
fn napi_graph_symbol_identity(handle: NapiValue) -> Result<u64, String> {
    let _owner = live_graph_owner_env(handle as u64)
        .ok_or("native graph Symbol owner is no longer live")?;
    match unsafe { value_ref(handle) } {
        Ok(Value::Symbol { id, .. }) => Ok(*id),
        _ => Err("native graph handle is not a Symbol".into()),
    }
}

fn napi_graph_token(env: &mut Env, token: &JsonValue, nodes: &[NapiValue]) -> Result<NapiValue, String> {
    let fields = token.as_object().ok_or("invalid native argument graph token")?;
    if fields.len() != 1 && !(fields.len() == 2
        && fields.contains_key("nsy") && fields.contains_key("hdl")) {
        return Err("invalid native argument graph token".into());
    }
    if let Some(index) = fields.get("r").and_then(JsonValue::as_u64) {
        return nodes.get(index as usize).copied().ok_or("native argument graph reference is out of range".into());
    }
    if fields.get("u") == Some(&JsonValue::from(1)) {
        return Ok(env.alloc(Value::Undefined));
    }
    if let Some(units) = fields.get("su") {
        let units = napi_graph_utf16_units(units)?;
        return Ok(match String::from_utf16(&units) {
            Ok(text) => env.alloc(Value::String(text)),
            Err(_) => {
                let value = env.alloc(Value::String(String::from_utf16_lossy(&units)));
                env.utf16_strings.insert(value as usize, units);
                value
            }
        });
    }
    if let Some(handle) = fields.get("nsy").and_then(JsonValue::as_str) {
        let handle = handle.parse::<u64>()
            .map_err(|_| "invalid native Symbol graph handle")? as NapiValue;
        if env.values.contains(&handle)
            && !env.finalized_handles.contains(&(handle as usize))
            && matches!(unsafe { handle.as_ref() }, Some(Value::Symbol { .. })) {
            return Ok(handle);
        }
        let source = fields.get("hdl").and_then(JsonValue::as_u64)
            .ok_or("foreign native Symbol has no JavaScript wrapper")?;
        return env.quickjs_live_values.get(&source).copied()
            .ok_or("unretained native Symbol wrapper");
    }
    if let Some(decimal) = fields.get("bi").and_then(JsonValue::as_str) {
        let (negative, words) = decimal_bigint_words(decimal)
            .ok_or("invalid native BigInt graph decimal")?;
        return Ok(env.alloc(Value::BigInt { negative, words }));
    }
    if let Some(kind) = fields.get("nf").and_then(JsonValue::as_str) {
        let number = match kind {
            "NaN" => f64::NAN,
            "Infinity" => f64::INFINITY,
            "-Infinity" => f64::NEG_INFINITY,
            _ => return Err("invalid nonfinite native argument".into()),
        };
        return Ok(env.alloc(Value::Number(number)));
    }
    if let Some(value) = fields.get("v") {
        return match value {
            JsonValue::Null => Ok(env.alloc(Value::Null)),
            JsonValue::Bool(value) => Ok(env.alloc(Value::Bool(*value))),
            JsonValue::Number(value) => Ok(env.alloc(Value::Number(value.as_f64().ok_or("invalid native argument number")?))),
            JsonValue::String(value) => Ok(env.alloc(Value::String(value.clone()))),
            _ => Err("invalid native argument scalar".into()),
        };
    }
    Err("unknown native argument graph token".into())
}

fn napi_graph_property_key(
    env: &mut Env, key: &JsonValue, nodes: &[NapiValue],
) -> Result<PropertyKey, String> {
    if let Some(key) = key.as_str() {
        return Ok(PropertyKey::String(key.to_string()));
    }
    if key.as_object().is_some_and(|fields| fields.len() == 1 && fields.contains_key("su")) {
        let units = napi_graph_utf16_units(&key["su"])?;
        return Ok(match String::from_utf16(&units) {
            Ok(text) => PropertyKey::String(text),
            Err(_) => PropertyKey::Utf16(units),
        });
    }
    let symbol = napi_graph_token(env, key, nodes)?;
    match unsafe { value_ref(symbol) }.map_err(|_| "invalid native graph Symbol key")? {
        Value::Symbol { id, .. } => Ok(PropertyKey::Symbol(*id)),
        #[cfg(feature = "quickjs")]
        Value::QuickJsHandle { handle, .. } => Ok(PropertyKey::Symbol(
            *env.quickjs_symbol_ids.get(handle)
                .ok_or("unverified JavaScript graph Symbol key")?)),
        #[cfg(not(feature = "quickjs"))]
        Value::QuickJsHandle { .. } => Err("JavaScript graph Symbol keys require QuickJS".into()),
        _ => Err("invalid native graph property name".into()),
    }
}

fn napi_graph_value(env: &mut Env, graph: &JsonValue) -> Result<NapiValue, String> {
    let descriptions = graph.get("nodes").and_then(JsonValue::as_array)
        .ok_or("native argument graph has no nodes")?;
    let mut nodes = Vec::with_capacity(descriptions.len());
    for description in descriptions {
        let fields = description.as_object().ok_or("invalid native argument graph node")?;
        if fields.len() != 1 && !(fields.len() == 2
            && ((fields.contains_key("a") && fields.contains_key("p"))
                || (fields.contains_key("nfn") && fields.contains_key("hdl"))
                || (fields.contains_key("nsy") && fields.contains_key("hdl")))) {
            return Err("invalid native argument graph node".into());
        }
        let node = if fields.get("a").and_then(JsonValue::as_array).is_some() {
            env.alloc(Value::Array(Vec::new()))
        } else if fields.get("o").and_then(JsonValue::as_array).is_some() {
            env.alloc(Value::Object(HashMap::new()))
        } else if let Some(timestamp) = fields.get("d") {
            let time = if timestamp.is_null() { f64::NAN }
                else { timestamp.as_f64().ok_or("invalid native Date timestamp")? };
            env.alloc(Value::Date(time))
        } else if let Some(bytes) = fields.get("b").and_then(JsonValue::as_array) {
            let bytes = bytes.iter().map(|byte| byte.as_u64().and_then(|byte| u8::try_from(byte).ok())
                .ok_or("invalid native Buffer byte")).collect::<Result<Vec<_>, _>>()?;
            env.alloc(Value::Buffer(bytes))
        } else if let Some(handle) = fields.get("nfn").and_then(JsonValue::as_str) {
            let handle = handle.parse::<u64>().map_err(|_| "invalid native Function ID")? as NapiValue;
            if env.values.contains(&handle) && !env.finalized_handles.contains(&(handle as usize))
                && matches!(unsafe { handle.as_ref() }, Some(Value::Function(_))) {
                handle
            } else {
                let source = fields.get("hdl").and_then(JsonValue::as_u64)
                    .ok_or("foreign native Function has no JavaScript wrapper")?;
                env.quickjs_live_values.get(&source).copied()
                    .ok_or("unretained native Function wrapper")?
            }
        } else if let Some(handle) = fields.get("hdl").and_then(JsonValue::as_u64) {
            env.quickjs_live_values.get(&handle).copied()
                .ok_or("unretained JavaScript graph handle")?
        } else if let Some(handle) = fields.get("nh").and_then(JsonValue::as_str) {
            let handle = handle.parse::<u64>().map_err(|_| "invalid native handle ID")? as NapiValue;
            if !env.values.contains(&handle) || env.finalized_handles.contains(&(handle as usize)) {
                return Err("unknown native handle ID".into());
            }
            handle
        } else if fields.contains_key("m") || fields.contains_key("s") || fields.contains_key("re") {
            let kind = if fields.contains_key("m") { NapiGraphWrapperKind::Map }
                else if fields.contains_key("s") { NapiGraphWrapperKind::Set }
                else { NapiGraphWrapperKind::RegExp };
            let value = env.alloc(Value::Object(HashMap::new()));
            env.graph_wrappers.insert(value as usize, kind);
            value
        } else {
            return Err("unsupported native argument graph node".into());
        };
        nodes.push(node);
    }
    for (description, node) in descriptions.iter().zip(nodes.iter().copied()) {
        if let Some(items) = description.get("a").and_then(JsonValue::as_array) {
            let mut decoded = Vec::with_capacity(items.len());
            for item in items {
                if item.as_object().is_some_and(|fields| fields.len() == 1 && fields.get("h") == Some(&JsonValue::from(1))) {
                    decoded.push(None);
                } else {
                    decoded.push(Some(napi_graph_token(env, item, &nodes)?));
                }
            }
            let Some(Value::Array(target)) = (unsafe { node.as_mut() }) else { return Err("native graph array changed type".into()); };
            *target = decoded;
            if let Some(entries) = description.get("p") {
                let entries = entries.as_array().ok_or("invalid native graph array properties")?;
                let mut order = Vec::with_capacity(entries.len());
                let mut attributes = Vec::with_capacity(entries.len());
                let mut extras = HashMap::new();
                for entry in entries {
                    let Some([key, token, flags]) = entry.as_array().map(Vec::as_slice)
                        else { return Err("invalid native graph array property".into()); };
                    let flags = flags.as_u64().and_then(|flags| u32::try_from(flags).ok())
                        .filter(|flags| flags & !NAPI_DEFAULT_PROPERTY_ATTRIBUTES == 0)
                        .ok_or("invalid native graph array property attributes")?;
                    let key = napi_graph_property_key(env, key, &nodes)?;
                    if order.contains(&key) { return Err("duplicate native graph array property".into()); }
                    if matches!(&key, PropertyKey::String(name) if name == "length") {
                        if token.get("v").and_then(JsonValue::as_f64) != Some(items.len() as f64) {
                            return Err("native graph array length changed".into());
                        }
                    } else if let Some(index) = property_array_index(&key) {
                        if index >= items.len() || items[index].get("h").is_some() {
                            return Err("invalid native graph array index property".into());
                        }
                        // The indexed value was installed from `a` above;
                        // `p` carries its descriptor. Compare wire tokens so
                        // scalar values are not decoded into a second Box.
                        if token != &items[index] {
                            return Err("conflicting native graph array index".into());
                        }
                    } else {
                        let child = napi_graph_token(env, token, &nodes)?;
                        extras.insert(key.clone(), child);
                    }
                    order.push(key.clone());
                    attributes.push((key, flags));
                }
                env.host_properties.insert(node as usize, extras);
                env.property_order.insert(node as usize, order);
                for (key, flags) in attributes {
                    env.property_attributes.insert((node as usize, key), flags);
                }
            }
        } else if let Some(entries) = description.get("o").and_then(JsonValue::as_array) {
            let mut decoded = HashMap::new();
            let mut order = Vec::with_capacity(entries.len());
            let mut attributes = Vec::with_capacity(entries.len());
            for entry in entries {
                let parts = entry.as_array().ok_or("invalid native graph entry")?;
                let (key, token, flags) = match parts.as_slice() {
                    [key, token] => (key, token, NAPI_DEFAULT_PROPERTY_ATTRIBUTES),
                    [key, token, flags] => {
                        let flags = flags.as_u64().and_then(|flags| u32::try_from(flags).ok())
                            .filter(|flags| flags & !NAPI_DEFAULT_PROPERTY_ATTRIBUTES == 0)
                            .ok_or("invalid native graph property attributes")?;
                        (key, token, flags)
                    }
                    _ => return Err("invalid native graph entry".into()),
                };
                let key = napi_graph_property_key(env, key, &nodes)?;
                if decoded.contains_key(&key) {
                    return Err("duplicate native graph property key".into());
                }
                let child = napi_graph_token(env, token, &nodes)?;
                order.push(key.clone());
                attributes.push((key.clone(), flags));
                decoded.insert(key, child);
            }
            let Some(Value::Object(target)) = (unsafe { node.as_mut() }) else { return Err("native graph object changed type".into()); };
            *target = decoded;
            env.property_order.insert(node as usize, order);
            for (key, flags) in attributes {
                env.property_attributes.insert((node as usize, key), flags);
            }
        } else if let Some(token) = description.get("m").or_else(|| description.get("s")) {
            let key = if description.get("m").is_some() { "__thaw_map_entries__" }
                else { "__thaw_set_values__" };
            let value = napi_graph_token(env, token, &nodes)?;
            let Some(Value::Object(target)) = (unsafe { node.as_mut() }) else { return Err("native graph wrapper changed type".into()); };
            target.insert(PropertyKey::String(key.into()), value);
        } else if let Some(parts) = description.get("re").and_then(JsonValue::as_array) {
            let [source, flags, last_index] = parts.as_slice() else { return Err("invalid native RegExp graph node".into()); };
            let source = source.as_str().ok_or("invalid native RegExp source")?;
            let flags = flags.as_str().ok_or("invalid native RegExp flags")?;
            let source = env.alloc(Value::String(source.into()));
            let flags = env.alloc(Value::String(flags.into()));
            let last_index = match last_index {
                JsonValue::Number(number) => env.alloc(Value::Number(number.as_f64().ok_or("invalid native RegExp lastIndex")?)),
                JsonValue::String(text) => env.alloc(Value::Number(match text.as_str() {
                    "NaN" => f64::NAN,
                    "Infinity" => f64::INFINITY,
                    "-Infinity" => f64::NEG_INFINITY,
                    _ => return Err("invalid native RegExp lastIndex".into()),
                })),
                _ => return Err("invalid native RegExp lastIndex".into()),
            };
            let pattern = env.alloc(Value::Object(HashMap::from([
                (PropertyKey::String("source".into()), source),
                (PropertyKey::String("flags".into()), flags),
                (PropertyKey::String("lastIndex".into()), last_index),
            ])));
            let Some(Value::Object(target)) = (unsafe { node.as_mut() }) else { return Err("native graph wrapper changed type".into()); };
            target.insert(PropertyKey::String("__thaw_regexp__".into()), pattern);
        }
    }
    napi_graph_token(env, graph.get("root").ok_or("native argument graph has no root")?, &nodes)
}

// An argument graph transfers its QuickJS leases before any N-API export or
// handle lookup. Keep the parsed wire and its leases together across those
// fallible lookups; successful decode releases them before invoking the addon.
struct GraphLeases {
    host: Vec<u64>,
    napi: Vec<u64>,
}

unsafe extern "C" {
    fn thaw_json_register_napi_handle_operations(
        retain: extern "C" fn(u64) -> u64,
        release: extern "C" fn(u64) -> u8,
    );
    fn thaw_json_discard_graph_wire(source: *const c_char);
}

fn live_graph_owner_env(handle: u64) -> Option<NapiEnv> {
    if handle == 0 { return None; }
    HOST.try_with(|host| {
        let host = host.borrow();
        host.module_envs.iter().chain(host.pending_call_envs.iter())
            .find(|env| env.values.contains(&(handle as NapiValue))
                && !env.finalized_handles.contains(&(handle as usize))
                && !env.finalizing && !env.finalized && !env.shutdown_requested
                && env.graph_owner_id != 0)
            .map(|env| (&**env as *const Env).cast_mut())
    }).ok().flatten()
}

extern "C" fn retain_napi_graph_handle(handle: u64) -> u64 {
    let _dispatch = ForeignCallbackGuard::new();
    let Some(env) = live_graph_owner_env(handle) else { return 0; };
    let mut reference = ptr::null_mut();
    let status = unsafe { napi_create_reference(env, handle as NapiValue, 1, &mut reference) };
    if status != NAPI_OK || reference.is_null() { return 0; }
    let Some(next) = (unsafe { &*env }).native_graph_pins.checked_add(1) else {
        unsafe { napi_delete_reference(env, reference) };
        return 0;
    };
    unsafe {
        (*env).graph_reference_tokens.insert(reference as usize);
        (*env).native_graph_pins = next;
    }
    reference as u64
}

unsafe extern "C" fn thaw_napi_graph_owner(handle: u64) -> *mut c_char {
    let _dispatch = ForeignCallbackGuard::new();
    let owner = live_graph_owner_env(handle)
        .map(|env| unsafe { (*env).graph_owner_id.to_string() })
        .unwrap_or_default();
    thaw_arena::owned_string(owner)
}

extern "C" fn release_napi_graph_reference(token: u64) -> u8 {
    if token == 0 { return 0; }
    let _dispatch = ForeignCallbackGuard::new();
    let reference = token as *mut Reference;
    // Find the stable Box in the still-live Env before dereferencing a token.
    let env = HOST.try_with(|host| {
        let host = host.borrow();
        host.module_envs.iter()
            .chain(host.pending_call_envs.iter())
            .find(|env| !env.finalized
                && env.graph_reference_tokens.contains(&(reference as usize)))
            .map(|env| (&**env as *const Env).cast_mut())
    }).ok().flatten();
    let Some(env) = env else { return 0; };
    if unsafe { (*env).native_graph_pins } == 0 { return 0; }
    // Consume provenance before invoking the N-API deletion path, which may
    // run a user finalizer that reenters graph release. Restore it if N-API
    // rejects the reference without deleting it.
    unsafe { (*env).graph_reference_tokens.remove(&(reference as usize)); }
    if unsafe { napi_delete_reference(env, reference) } != NAPI_OK {
        unsafe { (*env).graph_reference_tokens.insert(reference as usize); }
        return 0;
    }
    unsafe { (*env).native_graph_pins -= 1; }
    1
}

impl Drop for GraphLeases {
    fn drop(&mut self) {
        #[cfg(feature = "quickjs")]
        for handle in self.host.drain(..) {
            thaw_quickjs::thaw_js_release_handle(handle);
        }
        for reference in self.napi.drain(..) {
            release_napi_graph_reference(reference);
        }
    }
}

struct NapiGraphInput {
    parsed: Result<JsonValue, String>,
    leases: std::cell::RefCell<Option<GraphLeases>>,
    malformed_lease: bool,
    // Keep the owning Env pinned through a failed lookup's lease release.
    // Fields drop in declaration order, so this guard outlives `leases`.
    _dispatch: ForeignCallbackGuard,
}

// An inbound live native Function/Symbol carries both its native pointer and
// the exact JS wrapper handle. The latter is the safe recipient carrier when
// a different addon Env owns the native pointer. Collect from tokens and
// nodes alike: Symbols can occur directly as keys or scalar children.
fn inbound_graph_handles(
    value: &JsonValue, handles: &mut Vec<u64>, standalone: &mut Vec<u64>,
    pairs: &mut Vec<(u64, u64, u8)>,
) -> Result<(), String> {
    match value {
        JsonValue::Array(items) => {
            for item in items { inbound_graph_handles(item, handles, standalone, pairs)?; }
        }
        JsonValue::Object(fields) => {
            if let Some(raw) = fields.get("hdl") {
                let handle = raw.as_u64().filter(|handle| *handle != 0)
                    .ok_or("invalid JavaScript graph handle")?;
                if fields.len() != 1 && !(fields.len() == 2
                    && (fields.contains_key("nfn") ^ fields.contains_key("nsy"))) {
                    return Err("invalid paired native graph handle".into());
                }
                if let Some((native, kind)) = fields.get("nfn").map(|native| (native, 1))
                    .or_else(|| fields.get("nsy").map(|native| (native, 2))) {
                    let native = native.as_str().and_then(|native| native.parse::<u64>().ok())
                        .filter(|native| *native != 0)
                        .ok_or("invalid paired native graph identity")?;
                    pairs.push((handle, native, kind));
                } else {
                    standalone.push(handle);
                }
                handles.push(handle);
            } else if fields.contains_key("nfn") || fields.contains_key("nsy") {
                return Err("native graph identity has no JavaScript wrapper lease".into());
            }
            for child in fields.values() {
                inbound_graph_handles(child, handles, standalone, pairs)?;
            }
        }
        _ => {}
    }
    Ok(())
}

impl NapiGraphInput {
    fn parse(text: &str) -> Self {
        let parsed: Result<JsonValue, String> = serde_json::from_str(text)
            .map_err(|error| format!("invalid native graph JSON: {error}"));
        let mut leases = GraphLeases { host: Vec::new(), napi: Vec::new() };
        let mut malformed_lease = false;
        if let Ok(parsed) = &parsed {
            if let Some(items) = parsed.get("leases").and_then(JsonValue::as_array) {
                for lease in items {
                    if let Some(handle) = lease.as_u64().filter(|handle| *handle != 0) {
                        leases.host.push(handle);
                    } else {
                        malformed_lease = true;
                    }
                }
            }
            if let Some(items) = parsed.get("napiLeases") {
                if let Some(items) = items.as_array() {
                    for lease in items {
                        if let Some(reference) = lease.as_str()
                            .and_then(|reference| reference.parse::<u64>().ok())
                            .filter(|reference| *reference != 0) {
                            leases.napi.push(reference);
                        } else {
                            malformed_lease = true;
                        }
                    }
                } else {
                    malformed_lease = true;
                }
            }
        }
        Self {
            parsed,
            leases: std::cell::RefCell::new(Some(leases)),
            malformed_lease,
            _dispatch: ForeignCallbackGuard::new(),
        }
    }

    unsafe fn decode(&self, env: NapiEnv) -> Result<NapiValue, String> {
        let parsed = self.parsed.as_ref().map_err(Clone::clone)?;
        if parsed.get("leases").and_then(JsonValue::as_array).is_none() {
            return Err("native argument graph has no lease list".into());
        }
        if self.malformed_lease {
            return Err("invalid native argument graph lease".into());
        }
        // Preflight every hdl without holding an Env borrow across QuickJS.
        // The wire leases remain owned by this input until a decoded carrier
        // acquires its own independent registry retain.
        #[cfg(not(feature = "quickjs"))]
        if parsed.get("nodes").and_then(JsonValue::as_array).is_some_and(|nodes|
            nodes.iter().any(|node| node.get("hdl").is_some())) {
            return Err("JavaScript graph handles require QuickJS".into());
        }
        #[cfg(feature = "quickjs")]
        let mut wire_handles = Vec::new();
        #[cfg(feature = "quickjs")]
        let mut standalone_handles = Vec::new();
        #[cfg(feature = "quickjs")]
        let mut native_pairs = Vec::new();
        #[cfg(feature = "quickjs")]
        inbound_graph_handles(parsed, &mut wire_handles, &mut standalone_handles,
            &mut native_pairs)?;
        #[cfg(feature = "quickjs")]
        let mut new_handles = {
            let owner = env_mut(env).map_err(|_| "invalid native addon environment")?;
            let leases = self.leases.borrow();
            let host_leases = &leases.as_ref().ok_or("native graph leases already consumed")?.host;
            let mut handles = Vec::new();
            for handle in wire_handles {
                if !host_leases.contains(&handle) {
                    return Err("JavaScript graph handle has no transfer lease".into());
                }
                if !owner.quickjs_live_values.contains_key(&handle)
                    && !handles.contains(&handle) { handles.push(handle); }
            }
            handles
        };
        #[cfg(feature = "quickjs")]
        struct EnvRetains(Vec<u64>);
        #[cfg(feature = "quickjs")]
        impl Drop for EnvRetains {
            fn drop(&mut self) {
                for handle in self.0.drain(..) {
                    thaw_quickjs::thaw_js_release_handle(handle);
                }
            }
        }
        #[cfg(feature = "quickjs")]
        let mut owned = EnvRetains(Vec::new());
        #[cfg(feature = "quickjs")]
        for handle in &new_handles {
            if thaw_quickjs::thaw_js_retain_handle(*handle) == 0 {
                return Err("released JavaScript graph handle".into());
            }
            owned.0.push(*handle);
        }
        #[cfg(feature = "quickjs")]
        let mut object_likes = HashMap::new();
        #[cfg(feature = "quickjs")]
        for handle in &new_handles {
            object_likes.insert(*handle, qjs_query(*handle, 9)? == "1");
        }
        // A forged packet must not pair a live native pointer with some other
        // retained JS handle. The private wrapper maps validate the exact
        // Function or Symbol provenance before an Env borrow begins.
        #[cfg(feature = "quickjs")]
        for (handle, native, kind) in &native_pairs {
            let checked = thaw_quickjs::thaw_js_graph_native_pair_matches_result(
                *handle, *native, *kind);
            if !checked.error.is_null() {
                let error = thaw_arena::NativeStr::from_ptr(checked.error)
                    .to_string_lossy().into_owned();
                thaw_arena::destroy_string(checked.error);
                return Err(error);
            }
            if checked.value != 1 {
                return Err("native graph wrapper identity does not match its handle".into());
            }
        }
        // A same-Env native value already owns its Function/Symbol identity.
        // Retaining its JS wrapper in that Env would form a cycle:
        // Env -> hdl wrapper -> native positive Reference -> Env.
        // Keep an hdl only if the graph also uses it standalone, or some
        // paired occurrence belongs to a different Env.
        #[cfg(feature = "quickjs")]
        {
            let owner = env.as_ref().ok_or("invalid native addon environment")?;
            new_handles.retain(|handle| {
                if standalone_handles.contains(handle) { return true; }
                let has_pair = native_pairs.iter().any(|pair| pair.0 == *handle);
                !has_pair || native_pairs.iter().filter(|pair| pair.0 == *handle)
                    .any(|pair| {
                    let value = pair.1 as NapiValue;
                    !owner.values.contains(&value)
                        || owner.finalized_handles.contains(&(value as usize))
                        || !matches!((pair.2, unsafe { value.as_ref() }),
                            (1, Some(Value::Function(_))) | (2, Some(Value::Symbol { .. })))
                })
            });
        }
        // A Symbol's native recipient owns an ID and a weak JS cache entry,
        // not an independently strong QuickJS handle. Keep the wire retain
        // only until decode finishes, and use a transient map for hdl tokens.
        #[cfg(feature = "quickjs")]
        let mut transient_symbols = Vec::new();
        #[cfg(feature = "quickjs")]
        {
            let owner_id = env.as_ref().ok_or("invalid native addon environment")?.graph_owner_id;
            for handle in &new_handles {
                if !qjs_is_symbol(*handle)? { continue; }
                let id = thaw_quickjs::thaw_js_napi_graph_symbol_register(owner_id, *handle);
                if id == 0 { return Err("cannot register weak JavaScript graph Symbol".into()); }
                let value = env.as_ref().and_then(|owner| owner.symbols.get(&id).copied())
                    .ok_or("registered JavaScript graph Symbol has no native owner")?;
                transient_symbols.push((*handle, value));
            }
            new_handles.retain(|handle| !transient_symbols.iter().any(|(symbol, _)| symbol == handle));
        }
        // A graph key may refer to an hdl node. Verify its actual Symbol
        // type before entering the mutable Env decoder; QuickJS may reenter
        // addon code during a query, while the decoder's Env borrow may not.
        #[cfg(feature = "quickjs")]
        let mut symbol_key_handles = Vec::new();
        #[cfg(feature = "quickjs")]
        for (handle, native, kind) in &native_pairs {
            if *kind != 2 { continue; }
            if transient_symbols.iter().any(|(symbol, _)| symbol == handle) { continue; }
            let native = *native as NapiValue;
            if env.as_ref().is_some_and(|owner|
                owner.values.contains(&native)
                    && !owner.finalized_handles.contains(&(native as usize))) {
                continue;
            }
            // Pair validation above proved this is a live native Symbol and
            // that its hdl denotes the exact same JS Symbol. Assign its
            // stable native ID even when this transfer uses it only as a
            // value; it may become a property key later in the recipient.
            let id = napi_graph_symbol_identity(native)?;
            if !symbol_key_handles.iter().any(|(seen, _)| seen == handle) {
                if !qjs_is_symbol(*handle)? {
                    return Err("native graph Symbol wrapper changed type".into());
                }
                symbol_key_handles.push((*handle, Some(id)));
            }
        }
        #[cfg(feature = "quickjs")]
        {
            let nodes = parsed.get("nodes").and_then(JsonValue::as_array)
                .ok_or("native argument graph has no nodes")?;
            for node in nodes {
                let Some(entries) = node.get("o").or_else(|| node.get("p"))
                    .and_then(JsonValue::as_array) else { continue };
                for entry in entries {
                    let Some(key) = entry.as_array().and_then(|entry| entry.first())
                        else { continue };
                    let (handle, stable_id) = if let Some(index) = key.get("r").and_then(JsonValue::as_u64) {
                        (nodes.get(index as usize)
                            .and_then(|node| node.get("hdl")).and_then(JsonValue::as_u64)
                            .ok_or("native graph Symbol key has no live handle")?, None)
                    } else if key.get("nsy").is_some() {
                        let native = key.get("nsy").and_then(JsonValue::as_str)
                            .and_then(|native| native.parse::<u64>().ok())
                            .ok_or("invalid native graph Symbol key")? as NapiValue;
                        let stable_id = napi_graph_symbol_identity(native)?;
                        if env.as_ref().is_some_and(|owner|
                            owner.values.contains(&native)
                                && !owner.finalized_handles.contains(&(native as usize))
                                && matches!(unsafe { native.as_ref() },
                                    Some(Value::Symbol { .. }))) {
                            continue;
                        }
                        (key.get("hdl").and_then(JsonValue::as_u64)
                            .ok_or("native graph Symbol key has no wrapper handle")?,
                            Some(stable_id))
                    } else { continue };
                    if transient_symbols.iter().any(|(symbol, _)| *symbol == handle) { continue; }
                    if !symbol_key_handles.iter().any(|(seen, _)| *seen == handle) {
                        if !qjs_is_symbol(handle)? {
                            return Err("native graph property key is not a Symbol".into());
                        }
                        symbol_key_handles.push((handle, stable_id));
                    }
                }
            }
        }
        let (decoded, inserted, new_symbol_ids) = {
            let owner = env_mut(env).map_err(|_| "invalid native addon environment")?;
            #[cfg(feature = "quickjs")]
            let mut inserted = Vec::new();
            #[cfg(feature = "quickjs")]
            for (handle, value) in &transient_symbols {
                owner.quickjs_live_values.insert(*handle, *value);
            }
            #[cfg(feature = "quickjs")]
            for handle in new_handles.drain(..) {
                if owner.quickjs_live_values.contains_key(&handle) {
                    continue;
                }
                let value = owner.alloc(Value::QuickJsHandle {
                    handle, object_like: object_likes[&handle],
                });
                owner.quickjs_live_values.insert(handle, value);
                inserted.push((handle, value));
            }
            #[cfg(feature = "quickjs")]
            let mut new_symbol_ids = Vec::new();
            #[cfg(feature = "quickjs")]
            let mut preparation_error = None;
            #[cfg(feature = "quickjs")]
            for (handle, stable_id) in symbol_key_handles {
                if owner.quickjs_symbol_ids.contains_key(&handle) { continue; }
                let Some(value) = owner.quickjs_live_values.get(&handle).copied() else {
                    preparation_error = Some("unretained JavaScript graph Symbol key".to_string());
                    break;
                };
                let id = stable_id.unwrap_or_else(|| NEXT_SYMBOL_ID.fetch_add(1, Ordering::Relaxed));
                owner.quickjs_symbol_ids.insert(handle, id);
                let inserted_owner = if owner.symbols.contains_key(&id) { false }
                    else { owner.symbols.insert(id, value); true };
                new_symbol_ids.push((handle, id, inserted_owner));
            }
            #[cfg(not(feature = "quickjs"))]
            let inserted: Vec<(u64, NapiValue)> = Vec::new();
            #[cfg(not(feature = "quickjs"))]
            let new_symbol_ids: Vec<(u64, u64, bool)> = Vec::new();
            #[cfg(feature = "quickjs")]
            let decoded = match preparation_error {
                Some(error) => Err(error),
                None => napi_graph_value(owner, parsed),
            };
            #[cfg(not(feature = "quickjs"))]
            let decoded = napi_graph_value(owner, parsed);
            (decoded, inserted, new_symbol_ids)
        };
        #[cfg(feature = "quickjs")]
        {
            let owner = env_mut(env).map_err(|_| "invalid native addon environment")?;
            for (handle, value) in &transient_symbols {
                if owner.quickjs_live_values.get(handle) == Some(value) {
                    owner.quickjs_live_values.remove(handle);
                }
            }
            if decoded.is_err() {
                for (handle, id, inserted_owner) in &new_symbol_ids {
                    owner.quickjs_symbol_ids.remove(handle);
                    if *inserted_owner { owner.symbols.remove(id); }
                }
                for (handle, value) in &inserted {
                    owner.quickjs_live_values.remove(handle);
                    **value = Value::Undefined;
                }
            } else {
                // A nested callback may have installed one of these handles
                // during preflight. Retain ownership only for inserted values.
                owned.0.retain(|handle| !inserted.iter().any(|(entry, _)| entry == handle));
            }
        }
        // The decoded N-API value may be an nh instance whose old JS proxy
        // finalized while this wire was in flight. Move its positive native
        // reference into the enclosing dispatch guard, which outlives even
        // helper-local graph inputs and result encoding. The hdl leases may
        // be released now: their decoded carriers own new QuickJS handles.
        let leases = self.leases.borrow_mut().take();
        if let Some(mut leases) = leases {
            if decoded.is_ok() {
                let napi = std::mem::take(&mut leases.napi);
                NAPI_RECIPIENT_GRAPH_PINS.with(|pins| pins.borrow_mut().extend(napi));
            }
            drop(leases);
        }
        decoded
    }
}

unsafe fn parse_napi_graph_value(env: NapiEnv, text: &str) -> Result<NapiValue, String> {
    NapiGraphInput::parse(text).decode(env)
}

unsafe fn graph_input_for_args(args: *const c_char, graph_args: bool) -> Option<NapiGraphInput> {
    graph_args.then(|| text(args).ok().map(|text| NapiGraphInput::parse(&text))).flatten()
}

unsafe fn release_unconsumed_napi_callback_graph(text: &str) {
    let _input = NapiGraphInput::parse(text);
}

// The public N-API property/call adapters use this same live carrier. No
// Env reference or HOST borrow may survive a call into the QuickJS engine.
#[cfg(feature = "quickjs")]
unsafe fn qjs_handle(value: NapiValue) -> Option<u64> {
    match value_ref(value).ok()? {
        Value::QuickJsHandle { handle, .. } => Some(*handle),
        _ => None,
    }
}

#[cfg(feature = "quickjs")]
unsafe fn qjs_take_result_text(result: thaw_quickjs::ThawResult) -> Result<String, String> {
    let (pointer, error) = if result.error.is_null() {
        (result.value, false)
    } else {
        (result.error, true)
    };
    if pointer.is_null() { return Err("empty JavaScript bridge result".into()); }
    let text = thaw_arena::NativeStr::from_ptr(pointer).to_string_lossy().into_owned();
    thaw_arena::destroy_string(pointer);
    if error { Err(text) } else { Ok(text) }
}

#[cfg(feature = "quickjs")]
unsafe fn qjs_error_status(env: NapiEnv, error: String) -> NapiStatus {
    let Ok(owner) = env_mut(env) else { return NAPI_INVALID_ARG };
    let exception = owner.alloc(Value::Error(error));
    owner.exception = Some(exception);
    record_status(env, NAPI_PENDING_EXCEPTION)
}

// A retained engine Symbol is only a transfer lease. The recipient owns its
// stable native Symbol ID and the engine keeps the original identity weakly;
// storing this registry handle in Env would keep a local Symbol alive forever.
#[cfg(feature = "quickjs")]
fn adopted_symbol_root_proven(
    owner: &Env, id: u64, symbol: NapiValue, generation: u64,
) -> bool {
    (owner.strong_symbol_ids.contains(&id)
        || owner.js_pinned_symbol_ids.contains(&id))
        && owner.symbols.get(&id) == Some(&symbol)
        && owner.value_generations.get(&(symbol as usize)) == Some(&generation)
}

#[cfg(feature = "quickjs")]
unsafe fn qjs_adopt_symbol(env: NapiEnv, handle: u64) -> Result<Option<NapiValue>, String> {
    if !qjs_is_symbol(handle)? { return Ok(None); }
    let owner_id = env.as_ref().ok_or("invalid native addon environment")?.graph_owner_id;
    if owner_id == 0 { return Ok(None); } // Standalone test Env has no owner state.
    let id = thaw_quickjs::thaw_js_napi_graph_symbol_register(owner_id, handle);
    if id == 0 { return Err("cannot register weak JavaScript Symbol".into()); }
    let (symbol, generation) = {
        let owner = env.as_ref().ok_or("invalid native addon environment")?;
        let symbol = owner.symbols.get(&id).copied()
            .ok_or("registered JavaScript Symbol has no native owner")?;
        let generation = owner.value_generations.get(&(symbol as usize)).copied()
            .ok_or("registered JavaScript Symbol has no live generation")?;
        (symbol, generation)
    };
    let recipient_scope = env.as_ref().and_then(|owner|
        owner.active_handle_scopes.last().copied());
    let root = root_existing_value(env, symbol)
        .map_err(|_| "registered JavaScript Symbol is no longer live")?;
    // Establish the native-rooted JS pin before dropping the transferred
    // QuickJS retain, including for a reused canonical Symbol ID.
    sync_js_origin_symbol_pins(env);
    if !env.as_ref().is_some_and(|owner|
        adopted_symbol_root_proven(owner, id, symbol, generation))
        || !prepared_scoped_values_still_rooted(env, recipient_scope,
            &[(symbol, generation)]) {
        rollback_existing_value_root(env, root);
        return Err("cannot pin adopted JavaScript Symbol".into());
    }
    Ok(Some(symbol))
}

// A handle-valued QuickJS API result already owns one registry reference.
// Adopt it for a new Env carrier or release it when the Env cache already has
// the same JS identity. Release only after leaving the mutable Env borrow.
#[cfg(feature = "quickjs")]
unsafe fn qjs_adopt_handle_result(
    env: NapiEnv, result: thaw_quickjs::ThawHandleResult, out: *mut NapiValue,
) -> NapiStatus {
    let _foreign_dispatch = ForeignCallbackGuard::new();
    if !result.error.is_null() {
        let error = thaw_arena::NativeStr::from_ptr(result.error)
            .to_string_lossy().into_owned();
        thaw_arena::destroy_string(result.error);
        return qjs_error_status(env, error);
    }
    if result.value == 0 { return record_status(env, NAPI_INVALID_ARG); }
    if out.is_null() {
        thaw_quickjs::thaw_js_release_handle(result.value);
        return NAPI_OK;
    }
    let symbol = match qjs_adopt_symbol(env, result.value) {
        Ok(symbol) => symbol,
        Err(error) => {
            thaw_quickjs::thaw_js_release_handle(result.value);
            return qjs_error_status(env, error);
        }
    };
    if let Some(symbol) = symbol {
        let recipient_scope = env.as_ref().and_then(|owner|
            owner.active_handle_scopes.last().copied());
        let generation = env.as_ref().and_then(|owner|
            owner.value_generations.get(&(symbol as usize)).copied());
        thaw_quickjs::thaw_js_release_handle(result.value);
        let Some(generation) = generation else { return record_status(env, NAPI_INVALID_ARG); };
        if !prepared_scoped_values_still_rooted(env, recipient_scope,
            &[(symbol, generation)]) {
            return record_status(env, NAPI_INVALID_ARG);
        }
        return write_value(out, symbol);
    }
    let object_like = match qjs_query(result.value, 9) {
        Ok(value) => value == "1",
        Err(error) => {
            thaw_quickjs::thaw_js_release_handle(result.value);
            return qjs_error_status(env, error);
        }
    };
    let (value, duplicate) = {
        let Ok(owner) = env_mut(env) else {
            thaw_quickjs::thaw_js_release_handle(result.value);
            return NAPI_INVALID_ARG;
        };
        if let Some(value) = owner.quickjs_live_values.get(&result.value).copied() {
            (value, true)
        } else {
            let value = owner.alloc(Value::QuickJsHandle {
                handle: result.value, object_like,
            });
            owner.quickjs_live_values.insert(result.value, value);
            (value, false)
        }
    };
    if duplicate {
        let recipient_scope = env.as_ref().and_then(|owner|
            owner.active_handle_scopes.last().copied());
        let root = match prepare_scoped_value(env, value) {
            Ok(root) => root,
            Err(status) => {
                thaw_quickjs::thaw_js_release_handle(result.value);
                return record_status(env, status);
            }
        };
        let generation = env.as_ref().and_then(|owner|
            owner.value_generations.get(&(value as usize)).copied());
        thaw_quickjs::thaw_js_release_handle(result.value);
        if !generation.is_some_and(|generation|
            prepared_scoped_values_still_rooted(env, recipient_scope,
                &[(value, generation)])) {
            rollback_existing_value_root(env, root);
            return record_status(env, NAPI_INVALID_ARG);
        }
        return write_value(out, value);
    }
    write_scoped_value(env, out, value)
}

#[cfg(feature = "quickjs")]
unsafe fn qjs_adopt_thrown_handle(
    env: NapiEnv, handle: u64, object_like: bool,
) -> NapiStatus {
    let _foreign_dispatch = ForeignCallbackGuard::new();
    let symbol = match qjs_adopt_symbol(env, handle) {
        Ok(symbol) => symbol,
        Err(error) => {
            thaw_quickjs::thaw_js_release_handle(handle);
            return qjs_error_status(env, error);
        }
    };
    if let Some(symbol) = symbol {
        // The original thrown value becomes an Env root before the last
        // transferred engine retain can invoke a reentrant finalizer.
        let Ok(owner) = env_mut(env) else {
            thaw_quickjs::thaw_js_release_handle(handle);
            return NAPI_INVALID_ARG;
        };
        owner.exception = Some(symbol);
        thaw_quickjs::thaw_js_release_handle(handle);
        let Ok(owner) = env_mut(env) else { return NAPI_INVALID_ARG; };
        if !owner.values.contains(&symbol)
            || owner.finalized_handles.contains(&(symbol as usize)) {
            return NAPI_INVALID_ARG;
        }
        owner.exception = Some(symbol);
        return record_status(env, NAPI_PENDING_EXCEPTION);
    }
    let (exception, generation, duplicate) = {
        let Ok(owner) = env_mut(env) else {
            thaw_quickjs::thaw_js_release_handle(handle);
            return NAPI_INVALID_ARG;
        };
        let (value, duplicate) = if let Some(value) = owner.quickjs_live_values.get(&handle).copied() {
            (value, true)
        } else {
            let value = owner.alloc(Value::QuickJsHandle { handle, object_like });
            owner.quickjs_live_values.insert(handle, value);
            (value, false)
        };
        let generation = owner.value_generations.get(&(value as usize)).copied();
        if generation.is_some() { owner.exception = Some(value); }
        (value, generation, duplicate)
    };
    let Some(generation) = generation else {
        if duplicate { thaw_quickjs::thaw_js_release_handle(handle); }
        return NAPI_INVALID_ARG;
    };
    if duplicate { thaw_quickjs::thaw_js_release_handle(handle); }
    let Ok(owner) = env_mut(env) else { return NAPI_INVALID_ARG; };
    if !owner.values.contains(&exception)
        || owner.finalized_handles.contains(&(exception as usize))
        || owner.value_generations.get(&(exception as usize)) != Some(&generation) {
        return NAPI_INVALID_ARG;
    }
    owner.exception = Some(exception);
    record_status(env, NAPI_PENDING_EXCEPTION)
}

#[cfg(feature = "quickjs")]
unsafe fn qjs_adopt_property_read_result(
    env: NapiEnv, result: thaw_quickjs::ThawCallWithExceptionResult,
    out: *mut NapiValue,
) -> NapiStatus {
    if result.exception_handle != 0 {
        return qjs_adopt_thrown_handle(env, result.exception_handle,
            result.exception_object_like != 0);
    }
    qjs_adopt_handle_result(env, thaw_quickjs::ThawHandleResult {
        value: result.value, error: result.error,
    }, out)
}

#[cfg(feature = "quickjs")]
unsafe fn qjs_finish_property_write_result(
    env: NapiEnv, result: thaw_quickjs::ThawCallWithExceptionResult,
) -> NapiStatus {
    if result.exception_handle != 0 {
        return qjs_adopt_thrown_handle(env, result.exception_handle,
            result.exception_object_like != 0);
    }
    if !result.error.is_null() {
        let error = thaw_arena::NativeStr::from_ptr(result.error)
            .to_string_lossy().into_owned();
        thaw_arena::destroy_string(result.error);
        return qjs_error_status(env, error);
    }
    if result.value == 0 { return record_status(env, NAPI_GENERIC_FAILURE); }
    NAPI_OK
}

#[cfg(feature = "quickjs")]
unsafe fn qjs_utf16_key_handle(env: NapiEnv, units: &[u16]) -> Result<u64, NapiStatus> {
    let encoded = serde_json::to_string(units)
        .map_err(|_| record_status(env, NAPI_GENERIC_FAILURE))?;
    let encoded = CString::new(encoded)
        .map_err(|_| record_status(env, NAPI_INVALID_ARG))?;
    let result = thaw_quickjs::thaw_js_string_from_utf16_units_result(encoded.as_ptr());
    if !result.error.is_null() {
        let error = thaw_arena::NativeStr::from_ptr(result.error).to_string_lossy().into_owned();
        thaw_arena::destroy_string(result.error);
        return Err(qjs_error_status(env, error));
    }
    if result.value == 0 { return Err(record_status(env, NAPI_GENERIC_FAILURE)); }
    Ok(result.value)
}

#[cfg(feature = "quickjs")]
unsafe fn qjs_native_symbol_key_handle(
    env: NapiEnv, symbol: NapiValue,
) -> Result<u64, NapiStatus> {
    match value_ref(symbol) {
        Ok(Value::Symbol { .. }) => {},
        _ => return Err(record_status(env, NAPI_INVALID_ARG)),
    }
    // The QuickJS helper requests owner-validated metadata for this exact
    // native handle. Callers cannot seed its identity cache with an arbitrary
    // id/description pair.
    let result = thaw_quickjs::thaw_js_native_symbol_handle_result(symbol as u64);
    if !result.error.is_null() {
        let error = thaw_arena::NativeStr::from_ptr(result.error)
            .to_string_lossy().into_owned();
        thaw_arena::destroy_string(result.error);
        return Err(qjs_error_status(env, error));
    }
    if result.value == 0 { return Err(record_status(env, NAPI_GENERIC_FAILURE)); }
    Ok(result.value)
}

#[cfg(feature = "quickjs")]
unsafe fn qjs_nonstring_key_handle(
    env: NapiEnv, key: &PropertyKey,
) -> Option<Result<u64, NapiStatus>> {
    match key {
        PropertyKey::String(_) => None,
        PropertyKey::Utf16(units) => Some(qjs_utf16_key_handle(env, units)),
        PropertyKey::Symbol(id) => {
            let symbol = match env.as_ref().and_then(|owner| owner.symbols.get(id).copied()) {
                Some(value) => value,
                None => return Some(Err(record_status(env, NAPI_INVALID_ARG))),
            };
            if let Some(source) = qjs_handle(symbol) {
                return Some(if thaw_quickjs::thaw_js_retain_handle(source) != 0 {
                    Ok(source)
                } else {
                    Err(record_status(env, NAPI_INVALID_ARG))
                });
            }
            Some(qjs_native_symbol_key_handle(env, symbol))
        }
    }
}

#[cfg(feature = "quickjs")]
unsafe fn qjs_get_property(
    env: NapiEnv, handle: u64, key: &PropertyKey, out: *mut NapiValue,
) -> NapiStatus {
    let _foreign_dispatch = ForeignCallbackGuard::new();
    if let Some(key_handle) = qjs_nonstring_key_handle(env, key) {
        let key_handle = match key_handle {
            Ok(handle) => handle, Err(status) => return status,
        };
        let status = qjs_get_property_live_key(env, handle, key_handle, out);
        thaw_quickjs::thaw_js_release_handle(key_handle);
        return status;
    }
    let PropertyKey::String(key) = key else {
        return record_status(env, NAPI_GENERIC_FAILURE);
    };
    let Ok(key_json) = serde_json::to_string(key) else {
        return record_status(env, NAPI_GENERIC_FAILURE);
    };
    let Ok(key_json) = CString::new(key_json) else {
        return record_status(env, NAPI_INVALID_ARG);
    };
    let result = thaw_quickjs::thaw_js_get_property_json_key_with_exception_result(
        handle, key_json.as_ptr());
    qjs_adopt_property_read_result(env, result, out)
}

#[cfg(feature = "quickjs")]
unsafe fn qjs_get_property_live_key(
    env: NapiEnv, handle: u64, key_handle: u64, out: *mut NapiValue,
) -> NapiStatus {
    let _foreign_dispatch = ForeignCallbackGuard::new();
    let result = thaw_quickjs::thaw_js_get_property_key_handle_with_exception_result(
        handle, key_handle);
    qjs_adopt_property_read_result(env, result, out)
}

#[cfg(feature = "quickjs")]
unsafe fn qjs_property_predicate_live_key(
    env: NapiEnv, handle: u64, key_handle: u64, operation: u8, out: *mut bool,
) -> NapiStatus {
    let _foreign_dispatch = ForeignCallbackGuard::new();
    let result = thaw_quickjs::thaw_js_property_predicate_key_handle_result(
        handle, key_handle, operation);
    if !result.error.is_null() {
        let error = thaw_arena::NativeStr::from_ptr(result.error)
            .to_string_lossy().into_owned();
        thaw_arena::destroy_string(result.error);
        return qjs_error_status(env, error);
    }
    if let Some(out) = out.as_mut() { *out = result.value != 0; }
    NAPI_OK
}

#[cfg(feature = "quickjs")]
unsafe fn qjs_set_property_live_key(
    env: NapiEnv, handle: u64, key_handle: u64, value: NapiValue,
    receiver_data: bool,
) -> NapiStatus {
    let _foreign_dispatch = ForeignCallbackGuard::new();
    let Ok(encoded) = napi_result_graph_for_env(env, value, true) else {
        return record_status(env, NAPI_GENERIC_FAILURE);
    };
    let Ok(encoded) = CString::new(encoded) else {
        return record_status(env, NAPI_INVALID_ARG);
    };
    let result = thaw_quickjs::thaw_js_set_property_key_handle_graph_with_exception_result(
        handle, key_handle, encoded.as_ptr(), receiver_data as u8);
    qjs_finish_property_write_result(env, result)
}

#[cfg(feature = "quickjs")]
unsafe fn qjs_descriptor_key_handle(env: NapiEnv, key: NapiValue)
    -> Result<(u64, bool), NapiStatus> {
    if let Some(handle) = qjs_handle(key) { return Ok((handle, false)); }
    if matches!(value_ref(key), Ok(Value::Symbol { .. })) {
        return qjs_native_symbol_key_handle(env, key).map(|handle| (handle, true));
    }
    let key = property_key(env, key)?;
    let units = match key {
        PropertyKey::String(text) => text.encode_utf16().collect::<Vec<_>>(),
        PropertyKey::Utf16(units) => units,
        PropertyKey::Symbol(_) => return Err(record_status(env, NAPI_INVALID_ARG)),
    };
    qjs_utf16_key_handle(env, &units).map(|handle| (handle, true))
}

#[cfg(feature = "quickjs")]
unsafe fn qjs_define_data_property(
    env: NapiEnv, handle: u64, key: NapiValue, value: NapiValue, attributes: u32,
) -> NapiStatus {
    let _foreign_dispatch = ForeignCallbackGuard::new();
    let (key_handle, owned_key) = match qjs_descriptor_key_handle(env, key) {
        Ok(key) => key, Err(status) => return status,
    };
    let status = (|| {
        let encoded = napi_result_graph_for_env(env, value, true)
            .map_err(|_| record_status(env, NAPI_GENERIC_FAILURE))?;
        let encoded = CString::new(encoded)
            .map_err(|_| record_status(env, NAPI_INVALID_ARG))?;
        let result = thaw_quickjs::thaw_js_define_data_property_key_handle_graph_result(
            handle, key_handle, encoded.as_ptr(), attributes);
        if !result.error.is_null() {
            let error = thaw_arena::NativeStr::from_ptr(result.error)
                .to_string_lossy().into_owned();
            thaw_arena::destroy_string(result.error);
            return Err(qjs_error_status(env, error));
        }
        Ok(())
    })();
    if owned_key { thaw_quickjs::thaw_js_release_handle(key_handle); }
    match status { Ok(()) => NAPI_OK, Err(status) => status }
}

#[cfg(feature = "quickjs")]
unsafe fn qjs_define_callback_property(
    env: NapiEnv, handle: u64, key: NapiValue,
    getter: Option<NapiCallback>, setter: Option<NapiCallback>,
    method: Option<NapiCallback>, data: *mut c_void, attributes: u32,
) -> NapiStatus {
    let _foreign_dispatch = ForeignCallbackGuard::new();
    let (key_handle, owned_key) = match qjs_descriptor_key_handle(env, key) {
        Ok(key) => key, Err(status) => return status,
    };
    let status = (|| {
        let mut create = |callback| -> Result<u64, NapiStatus> {
            let mut value = ptr::null_mut();
            let status = napi_create_function(env, ptr::null(), NAPI_AUTO_LENGTH,
                Some(callback), data, &mut value);
            if status != NAPI_OK { return Err(status); }
            Ok(value as u64)
        };
        let getter_id = getter.map(&mut create).transpose()?.unwrap_or(0);
        let setter_id = setter.map(&mut create).transpose()?.unwrap_or(0);
        let method_id = method.map(&mut create).transpose()?.unwrap_or(0);
        // A JS descriptor can outlive the addon call which installed it.
        // Retain each native Function before handing its identity to QuickJS;
        // the QuickJS closure owns these references after the FFI call.
        let mut roots = [0; 3];
        for (slot, id) in roots.iter_mut().zip([getter_id, setter_id, method_id]) {
            if id == 0 { continue; }
            *slot = retain_napi_graph_handle(id);
            if *slot == 0 {
                for root in roots.iter().copied().filter(|root| *root != 0) {
                    release_napi_graph_reference(root);
                }
                return Err(record_status(env, NAPI_GENERIC_FAILURE));
            }
        }
        let result = thaw_quickjs::thaw_js_define_callback_property_key_handle_result(
            handle, key_handle, getter_id, setter_id, method_id, attributes,
            roots[0], roots[1], roots[2]);
        if !result.error.is_null() {
            let error = thaw_arena::NativeStr::from_ptr(result.error)
                .to_string_lossy().into_owned();
            thaw_arena::destroy_string(result.error);
            return Err(qjs_error_status(env, error));
        }
        Ok(())
    })();
    if owned_key { thaw_quickjs::thaw_js_release_handle(key_handle); }
    match status { Ok(()) => NAPI_OK, Err(status) => status }
}

#[cfg(feature = "quickjs")]
unsafe fn qjs_object_integrity(env: NapiEnv, handle: u64, freeze: bool) -> NapiStatus {
    let _foreign_dispatch = ForeignCallbackGuard::new();
    let result = thaw_quickjs::thaw_js_host_object_integrity_result(handle, u8::from(freeze));
    if !result.error.is_null() {
        let error = thaw_arena::NativeStr::from_ptr(result.error)
            .to_string_lossy().into_owned();
        thaw_arena::destroy_string(result.error);
        return qjs_error_status(env, error);
    }
    if result.value == 0 { return record_status(env, NAPI_GENERIC_FAILURE); }
    NAPI_OK
}

#[cfg(feature = "quickjs")]
unsafe fn qjs_set_property(
    env: NapiEnv, handle: u64, key: &PropertyKey, value: NapiValue,
    receiver_data: bool,
) -> NapiStatus {
    let _foreign_dispatch = ForeignCallbackGuard::new();
    if let Some(key_handle) = qjs_nonstring_key_handle(env, key) {
        let key_handle = match key_handle {
            Ok(handle) => handle, Err(status) => return status,
        };
        let status = qjs_set_property_live_key(env, handle, key_handle, value, receiver_data);
        thaw_quickjs::thaw_js_release_handle(key_handle);
        return status;
    }
    let PropertyKey::String(key) = key else {
        return record_status(env, NAPI_GENERIC_FAILURE);
    };
    let Ok(key_json) = serde_json::to_string(key) else {
        return record_status(env, NAPI_GENERIC_FAILURE);
    };
    let Ok(key_json) = CString::new(key_json) else {
        return record_status(env, NAPI_INVALID_ARG);
    };
    let Ok(encoded) = napi_result_graph_for_env(env, value, true) else {
        return record_status(env, NAPI_GENERIC_FAILURE);
    };
    let Ok(encoded) = CString::new(encoded) else {
        return record_status(env, NAPI_INVALID_ARG);
    };
    let result = if receiver_data {
        thaw_quickjs::thaw_js_set_receiver_data_graph_with_exception_result(
            handle, key_json.as_ptr(), encoded.as_ptr())
    } else {
        thaw_quickjs::thaw_js_set_property_graph_with_exception_result(
            handle, key_json.as_ptr(), encoded.as_ptr())
    };
    qjs_finish_property_write_result(env, result)
}

#[cfg(feature = "quickjs")]
unsafe fn qjs_property_predicate(
    env: NapiEnv, handle: u64, key: &PropertyKey,
    operation: u8, out: *mut bool,
) -> NapiStatus {
    let _foreign_dispatch = ForeignCallbackGuard::new();
    if let Some(key_handle) = qjs_nonstring_key_handle(env, key) {
        let key_handle = match key_handle {
            Ok(handle) => handle, Err(status) => return status,
        };
        let status = qjs_property_predicate_live_key(env, handle, key_handle, operation, out);
        thaw_quickjs::thaw_js_release_handle(key_handle);
        return status;
    }
    let PropertyKey::String(key) = key else {
        return record_status(env, NAPI_GENERIC_FAILURE);
    };
    let Ok(key_json) = serde_json::to_string(key) else {
        return record_status(env, NAPI_GENERIC_FAILURE);
    };
    let Ok(key_json) = CString::new(key_json) else {
        return record_status(env, NAPI_INVALID_ARG);
    };
    let result = thaw_quickjs::thaw_js_property_predicate_json_key_result(
        handle, key_json.as_ptr(), operation);
    if !result.error.is_null() {
        let error = thaw_arena::NativeStr::from_ptr(result.error)
            .to_string_lossy().into_owned();
        thaw_arena::destroy_string(result.error);
        return qjs_error_status(env, error);
    }
    if let Some(out) = out.as_mut() { *out = result.value != 0; }
    NAPI_OK
}

#[cfg(feature = "quickjs")]
unsafe fn qjs_query(handle: u64, operation: u8) -> Result<String, String> {
    let _foreign_dispatch = ForeignCallbackGuard::new();
    qjs_take_result_text(thaw_quickjs::thaw_js_host_query_result(handle, operation))
}

#[cfg(feature = "quickjs")]
fn qjs_is_symbol(handle: u64) -> Result<bool, String> {
    match thaw_quickjs::thaw_js_napi_handle_is_symbol(handle) {
        0 => Ok(false),
        1 => Ok(true),
        _ => Err("released JavaScript Symbol handle".into()),
    }
}

#[cfg(feature = "quickjs")]
unsafe fn qjs_number_value(handle: u64) -> Result<f64, String> {
    // Operation 14 is the captured engine-level IsCallable query. Numeric
    // coercion uses its own host-query slot and may run ToPrimitive.
    let text = qjs_query(handle, 16)?;
    match text.as_str() {
        "NaN" => Ok(f64::NAN),
        "Infinity" => Ok(f64::INFINITY),
        "-Infinity" => Ok(f64::NEG_INFINITY),
        "-0" => Ok(-0.0),
        _ => text.parse::<f64>().map_err(|_| "invalid JavaScript numeric result".into()),
    }
}

#[cfg(feature = "quickjs")]
unsafe fn qjs_actual_number(env: NapiEnv, handle: u64) -> Result<f64, NapiStatus> {
    match qjs_query(handle, 0) {
        Ok(kind) if kind == "number" => {},
        Ok(_) => return Err(record_status(env, NAPI_NUMBER_EXPECTED)),
        Err(error) => return Err(qjs_error_status(env, error)),
    }
    qjs_number_value(handle).map_err(|error| qjs_error_status(env, error))
}

#[cfg(feature = "quickjs")]
unsafe fn qjs_actual_string_units(env: NapiEnv, handle: u64) -> Result<Vec<u16>, NapiStatus> {
    match qjs_query(handle, 0) {
        Ok(kind) if kind == "string" => {},
        Ok(_) => return Err(record_status(env, NAPI_STRING_EXPECTED)),
        Err(error) => return Err(qjs_error_status(env, error)),
    }
    let text = qjs_query(handle, 15).map_err(|error| qjs_error_status(env, error))?;
    serde_json::from_str::<Vec<u16>>(&text)
        .map_err(|_| record_status(env, NAPI_GENERIC_FAILURE))
}

#[cfg(feature = "quickjs")]
unsafe fn qjs_strict_equals(
    env: NapiEnv, left: u64, right: u64, out: *mut bool,
) -> NapiStatus {
    let _foreign_dispatch = ForeignCallbackGuard::new();
    let compared = thaw_quickjs::thaw_js_strict_equal_handles_result(left, right);
    if !compared.error.is_null() {
        let error = thaw_arena::NativeStr::from_ptr(compared.error)
            .to_string_lossy().into_owned();
        thaw_arena::destroy_string(compared.error);
        return qjs_error_status(env, error);
    }
    *out = compared.value != 0;
    NAPI_OK
}

#[cfg(feature = "quickjs")]
unsafe fn qjs_strict_equals_native_symbol(
    env: NapiEnv, handle: u64, symbol: NapiValue, out: *mut bool,
) -> NapiStatus {
    let _foreign_dispatch = ForeignCallbackGuard::new();
    let symbol_handle = match qjs_native_symbol_key_handle(env, symbol) {
        Ok(handle) => handle,
        Err(status) => return status,
    };
    let status = qjs_strict_equals(env, handle, symbol_handle, out);
    thaw_quickjs::thaw_js_release_handle(symbol_handle);
    status
}

#[cfg(feature = "quickjs")]
unsafe fn qjs_is_promise(env: NapiEnv, handle: u64, out: *mut bool) -> NapiStatus {
    let _foreign_dispatch = ForeignCallbackGuard::new();
    let result = thaw_quickjs::thaw_js_handle_is_promise_result(handle);
    if !result.error.is_null() {
        let error = thaw_arena::NativeStr::from_ptr(result.error)
            .to_string_lossy().into_owned();
        thaw_arena::destroy_string(result.error);
        return qjs_error_status(env, error);
    }
    *out = result.value != 0;
    NAPI_OK
}

#[cfg(feature = "quickjs")]
unsafe fn qjs_array_length(env: NapiEnv, handle: u64, out: *mut u32) -> NapiStatus {
    let _foreign_dispatch = ForeignCallbackGuard::new();
    match qjs_query(handle, 4) {
        Ok(answer) if answer == "1" => {},
        Ok(_) => return record_status(env, NAPI_ARRAY_EXPECTED),
        Err(error) => return qjs_error_status(env, error),
    }
    let result = thaw_quickjs::thaw_js_get_property_json_key_result(
        handle, c"\"length\"".as_ptr());
    if !result.error.is_null() {
        let error = thaw_arena::NativeStr::from_ptr(result.error)
            .to_string_lossy().into_owned();
        thaw_arena::destroy_string(result.error);
        return qjs_error_status(env, error);
    }
    if result.value == 0 { return record_status(env, NAPI_GENERIC_FAILURE); }
    let length = qjs_query(result.value, 16);
    thaw_quickjs::thaw_js_release_handle(result.value);
    match length {
        Ok(length) => match length.parse::<u32>() {
            Ok(length) => { *out = length; NAPI_OK },
            Err(_) => record_status(env, NAPI_GENERIC_FAILURE),
        },
        Err(error) => qjs_error_status(env, error),
    }
}

#[cfg(feature = "quickjs")]
unsafe fn qjs_call_function(
    env: NapiEnv, handle: u64, this_arg: NapiValue,
    args: &[NapiValue], out: *mut NapiValue,
) -> NapiStatus {
    let _foreign_dispatch = ForeignCallbackGuard::new();
    let arguments = {
        let Ok(owner) = env_mut(env) else { return NAPI_INVALID_ARG };
        let mut items = Vec::with_capacity(args.len() + 1);
        items.push(Some(this_arg));
        items.extend(args.iter().copied().map(Some));
        owner.alloc(Value::Array(items))
    };
    let guard = CallbackGraphArguments { env, value: arguments };
    let encoded = napi_result_graph_for_env(env, arguments, true);
    drop(guard);
    let Ok(encoded) = encoded else {
        return record_status(env, NAPI_GENERIC_FAILURE);
    };
    let Ok(encoded) = CString::new(encoded) else {
        return record_status(env, NAPI_INVALID_ARG);
    };
    let result = thaw_quickjs::thaw_js_call_handle_with_this_graph_result(handle, encoded.as_ptr());
    if result.exception_handle != 0 {
        return qjs_adopt_thrown_handle(env, result.exception_handle,
            result.exception_object_like != 0);
    }
    qjs_adopt_handle_result(env,
        thaw_quickjs::ThawHandleResult { value: result.value, error: result.error }, out)
}

// An accessor descriptor owns its callback handles independently of the
// graph packet which installed it. `find_accessor` returns an Accessor clone,
// keeping these roots alive through the entire invocation even if a getter
// replaces or deletes its own descriptor. No Env borrow crosses QuickJS.
#[cfg(feature = "quickjs")]
unsafe extern "C" fn reflected_native_getter(
    env: NapiEnv, info: NapiCallbackInfo,
) -> NapiValue {
    let Some(info) = info.as_ref() else { return ptr::null_mut() };
    let Some(source) = (info.data as *const Accessor).as_ref() else {
        return ptr::null_mut();
    };
    let Some(getter) = source.getter else { return ptr::null_mut() };
    let mut forwarded = CallbackInfo {
        args: info.args.clone(), this_arg: info.this_arg,
        new_target: ptr::null_mut(), data: source.getter_data,
    };
    invoke_napi_callback(env, getter, &mut forwarded)
}

#[cfg(feature = "quickjs")]
unsafe extern "C" fn reflected_native_setter(
    env: NapiEnv, info: NapiCallbackInfo,
) -> NapiValue {
    let Some(info) = info.as_ref() else { return ptr::null_mut() };
    let Some(source) = (info.data as *const Accessor).as_ref() else {
        return ptr::null_mut();
    };
    let Some(setter) = source.setter else { return ptr::null_mut() };
    let mut forwarded = CallbackInfo {
        args: info.args.clone(), this_arg: info.this_arg,
        new_target: ptr::null_mut(), data: source.setter_data,
    };
    invoke_napi_callback(env, setter, &mut forwarded)
}

// A reflected accessor is the original callback, not a property-name
// forwarding shim. Keep its descriptor snapshot with the Function value so
// `.call(other)` and calls after replacement use the original callback and
// call-time receiver. The descriptor caches only the value's address; the
// snapshot deliberately omits reflection addresses to avoid a retention cycle.
#[cfg(feature = "quickjs")]
unsafe fn qjs_reflected_native_accessor_source(
    env: NapiEnv, object: NapiValue, key: &PropertyKey,
    accessor: &Accessor, getter: bool,
) -> Result<serde_json::Value, String> {
    let callback = if getter { accessor.getter } else { accessor.setter };
    if callback.is_none() { return Ok(serde_json::Value::Null); }
    if let Some(roots) = accessor.js_owner.as_ref() {
        let (handle, native) = if getter {
            (roots.getter, roots.getter_native)
        } else { (roots.setter, roots.setter_native) };
        if handle != 0 { return Ok(serde_json::json!({"hdl": handle})); }
        if !native.is_null() {
            return Ok(serde_json::json!({"nfn": (native as u64).to_string()}));
        }
    }
    let identity = (object as usize, key.clone());
    let owner = env_mut(env).map_err(|_| "invalid native accessor owner")?;
    let previous = owner.accessors.get(&identity)
        .ok_or("native accessor was replaced during reflection")?;
    let cached = if getter { previous.getter_reflection }
        else { previous.setter_reflection };
    if let Some(value) = cached {
        return Ok(serde_json::json!({"nfn": (value as u64).to_string()}));
    }
    let mut snapshot = accessor.clone();
    snapshot.getter_reflection = None;
    snapshot.setter_reflection = None;
    let snapshot = Arc::new(snapshot);
    let callback = if getter { reflected_native_getter }
        else { reflected_native_setter };
    let value = owner.alloc(Value::Function(Function {
        callback, data: Arc::as_ptr(&snapshot) as *mut c_void,
        properties: HashMap::new(), _thaw_bridge: None,
        _accessor_owner: Some(snapshot),
    }));
    let entry = owner.accessors.get_mut(&identity)
        .ok_or("native accessor was replaced during reflection")?;
    if getter { entry.getter_reflection = Some(value); }
    else { entry.setter_reflection = Some(value); }
    Ok(serde_json::json!({"nfn": (value as u64).to_string()}))
}

#[cfg(feature = "quickjs")]
unsafe extern "C" fn quickjs_accessor_getter(
    env: NapiEnv, info: NapiCallbackInfo,
) -> NapiValue {
    let Some(info) = info.as_ref() else { return ptr::null_mut() };
    let Some(roots) = (info.data as *const QuickJsAccessorRoots).as_ref() else {
        return ptr::null_mut();
    };
    if roots.getter == 0 && roots.getter_native.is_null() { return ptr::null_mut(); }
    let mut result = ptr::null_mut();
    let status = if roots.getter_native.is_null() {
        qjs_call_function(env, roots.getter, info.this_arg, &[], &mut result)
    } else {
        napi_call_function(env, info.this_arg, roots.getter_native,
            0, ptr::null(), &mut result)
    };
    if status == NAPI_OK {
        result
    } else {
        ptr::null_mut()
    }
}

#[cfg(feature = "quickjs")]
unsafe extern "C" fn quickjs_accessor_setter(
    env: NapiEnv, info: NapiCallbackInfo,
) -> NapiValue {
    let Some(info) = info.as_ref() else { return ptr::null_mut() };
    let Some(roots) = (info.data as *const QuickJsAccessorRoots).as_ref() else {
        return ptr::null_mut();
    };
    if (roots.setter == 0 && roots.setter_native.is_null()) || info.args.len() != 1 {
        return ptr::null_mut();
    }
    // A setter's returned value is discarded by property assignment. The
    // QuickJS result handle is released rather than stored in Env's carrier
    // cache, while the callback error still becomes the pending exception.
    if roots.setter_native.is_null() {
        qjs_call_function(env, roots.setter, info.this_arg, &info.args, ptr::null_mut());
    } else {
        napi_call_function(env, info.this_arg, roots.setter_native,
            1, info.args.as_ptr(), ptr::null_mut());
    }
    ptr::null_mut()
}

// `roots` already owns independent retained QuickJS callback handles.
// It owns them across every validation failure. Its final drop is deliberately
// outside the Env mutation borrow, because releasing a handle may reenter JS.
#[cfg(feature = "quickjs")]
unsafe fn qjs_install_native_accessor(
    env: NapiEnv, object: NapiValue, key: PropertyKey,
    mut roots: Arc<QuickJsAccessorRoots>,
    getter_present: bool, setter_present: bool,
    attributes: u32,
) -> NapiStatus {
    let previous = accessor_for_owner(env, object as usize, &key);
    let old_attributes = property_attributes_for(env, object as usize, &key);
    let _dispatch = ForeignCallbackGuard::new();
    if !getter_present && !setter_present { return record_status(env, NAPI_INVALID_ARG); }
    if !value_belongs_to_environment(env, object)
        || qjs_handle(object).is_some()
        || !matches!(value_ref(object), Ok(value) if is_object_value(value)) {
        return record_status(env, NAPI_OBJECT_EXPECTED);
    }
    // A partial accessor descriptor preserves its existing opposite side.
    // Transfer a fresh registry retain into the new Arc if that side was a
    // QuickJS callback; a native callback keeps its original data pointer.
    if !getter_present {
        if let Some(native) = previous.as_ref().and_then(|old| old.js_owner.as_ref())
            .map(|owner| owner.getter_native).filter(|native| !native.is_null()) {
            Arc::get_mut(&mut roots).unwrap().getter_native = native;
        }
        if let Some(handle) = previous.as_ref().and_then(|old| old.js_owner.as_ref())
            .map(|owner| owner.getter).filter(|handle| *handle != 0) {
            if thaw_quickjs::thaw_js_retain_handle(handle) == 0 {
                return record_status(env, NAPI_INVALID_ARG);
            }
            Arc::get_mut(&mut roots).unwrap().getter = handle;
        }
    }
    if !setter_present {
        if let Some(native) = previous.as_ref().and_then(|old| old.js_owner.as_ref())
            .map(|owner| owner.setter_native).filter(|native| !native.is_null()) {
            Arc::get_mut(&mut roots).unwrap().setter_native = native;
        }
        if let Some(handle) = previous.as_ref().and_then(|old| old.js_owner.as_ref())
            .map(|owner| owner.setter).filter(|handle| *handle != 0) {
            if thaw_quickjs::thaw_js_retain_handle(handle) == 0 {
                return record_status(env, NAPI_INVALID_ARG);
            }
            Arc::get_mut(&mut roots).unwrap().setter = handle;
        }
    }
    for handle in [roots.getter, roots.setter].into_iter().filter(|handle| *handle != 0) {
        // The ordinary typeof query is a writable bootstrap global. Use the
        // engine-level IsCallable slot so user code cannot bless a data value
        // as a native accessor callback.
        match qjs_query(handle, 14) {
            Ok(kind) if kind == "1" => {},
            Ok(_) => return record_status(env, NAPI_FUNCTION_EXPECTED),
            Err(error) => return qjs_error_status(env, error),
        }
    }
    if old_attributes & NAPI_CONFIGURABLE == 0 {
        let matches_side = |present: bool, handle: u64, native: NapiValue,
                            old_handle: u64, old_native: NapiValue,
                            old_callback: Option<NapiCallback>| {
            !present || (handle == old_handle && native == old_native
                && (handle != 0 || !native.is_null()) == old_callback.is_some())
        };
        let Some(old) = previous.as_ref() else {
            return record_status(env, NAPI_GENERIC_FAILURE);
        };
        let old_owner = old.js_owner.as_ref();
        if attributes & (NAPI_ENUMERABLE | NAPI_CONFIGURABLE)
                != old_attributes & (NAPI_ENUMERABLE | NAPI_CONFIGURABLE)
            || !matches_side(getter_present, roots.getter, roots.getter_native,
                old_owner.map_or(0, |owner| owner.getter),
                old_owner.map_or(ptr::null_mut(), |owner| owner.getter_native), old.getter)
            || !matches_side(setter_present, roots.setter, roots.setter_native,
                old_owner.map_or(0, |owner| owner.setter),
                old_owner.map_or(ptr::null_mut(), |owner| owner.setter_native), old.setter) {
            return record_status(env, NAPI_GENERIC_FAILURE);
        }
    }
    // The slot lookup can allocate for an indexed native view. Resolve both
    // answers before taking the Env's exclusive mutation borrow.
    let had_data = own_property_value(env, object, &key).is_some();
    let beyond_readonly_length = array_index_exceeds_readonly_length(env, object, &key);
    let old = {
        let Ok(owner) = env_mut(env) else { return NAPI_INVALID_ARG; };
        let identity = object as usize;
        if owner.nonextensible_objects.contains(&identity)
            && previous.is_none() && !had_data {
            return record_status(env, NAPI_GENERIC_FAILURE);
        }
        if beyond_readonly_length {
            return record_status(env, NAPI_GENERIC_FAILURE);
        }
        if had_data {
            let status = remove_own_property(owner, object, &key);
            if status != NAPI_OK { return record_status(env, status); }
        }
        extend_array_for_own_index(object, &key);
        let descriptor = Accessor {
            getter: if getter_present {
                (roots.getter != 0 || !roots.getter_native.is_null())
                    .then_some(quickjs_accessor_getter as NapiCallback)
            } else { previous.as_ref().and_then(|old| old.getter) },
            setter: if setter_present {
                (roots.setter != 0 || !roots.setter_native.is_null())
                    .then_some(quickjs_accessor_setter as NapiCallback)
            } else { previous.as_ref().and_then(|old| old.setter) },
            getter_data: if roots.getter != 0 || !roots.getter_native.is_null() {
                Arc::as_ptr(&roots) as *mut c_void
            } else { previous.as_ref().map(|old| old.getter_data).unwrap_or(ptr::null_mut()) },
            setter_data: if roots.setter != 0 || !roots.setter_native.is_null() {
                Arc::as_ptr(&roots) as *mut c_void
            } else { previous.as_ref().map(|old| old.setter_data).unwrap_or(ptr::null_mut()) },
            getter_reflection: if getter_present { None }
                else { previous.as_ref().and_then(|old| old.getter_reflection) },
            setter_reflection: if setter_present { None }
                else { previous.as_ref().and_then(|old| old.setter_reflection) },
            js_owner: Some(roots.clone()),
        };
        let old = owner.accessors.insert((identity, key.clone()), descriptor);
        record_property_order(owner, identity, &key);
        owner.property_attributes.insert((identity, key), attributes & (NAPI_ENUMERABLE | NAPI_CONFIGURABLE));
        old
    };
    drop(old);
    NAPI_OK
}

// Define a native data descriptor without invoking a replaced setter. The
// old accessor owns callback handles, so release it only after the mutable
// Env scope ends; a QuickJS finalizer may reenter this addon.
unsafe fn qjs_install_native_data_property(
    env: NapiEnv, object: NapiValue, key: PropertyKey,
    value: NapiValue, value_present: bool, attributes: u32,
) -> NapiStatus {
    #[cfg(feature = "quickjs")]
    let is_quickjs_object = qjs_handle(object).is_some();
    #[cfg(not(feature = "quickjs"))]
    let is_quickjs_object = false;
    if !value_belongs_to_environment(env, object)
        || (value_present && !value_belongs_to_environment(env, value))
        || is_quickjs_object
        || !matches!(value_ref(object), Ok(value) if is_object_value(value)) {
        return record_status(env, NAPI_OBJECT_EXPECTED);
    }
    let value = if value_present { value } else {
        match own_property_value(env, object, &key) {
            Some(value) => value,
            None => match env_mut(env) {
                Ok(owner) => owner.alloc(Value::Undefined),
                Err(status) => return record_status(env, status),
            },
        }
    };
    let old_value = own_property_value(env, object, &key);
    let old_accessor = accessor_for_owner(env, object as usize, &key);
    let old_attributes = property_attributes_for(env, object as usize, &key);
    let same_value = if let Some(previous) = old_value {
        match (value_ref(previous), value_ref(value)) {
            (Ok(Value::Number(left)), Ok(Value::Number(right))) =>
                left.to_bits() == right.to_bits() || (left.is_nan() && right.is_nan()),
            _ => {
                let mut equal = false;
                let status = napi_strict_equals(env, previous, value, &mut equal);
                if status != NAPI_OK { return record_status(env, status); }
                equal
            }
        }
    } else { false };
    let nonconfigurable = old_value.is_some() || old_accessor.is_some();
    let nonconfigurable = nonconfigurable && old_attributes & NAPI_CONFIGURABLE == 0;
    if nonconfigurable {
        // JS permits a writable nonconfigurable data property to change its
        // value or become readonly. Its enumerable/configurable flags cannot
        // change, and an accessor cannot become data at this point.
        if old_accessor.is_some()
            || (attributes ^ old_attributes) & (NAPI_ENUMERABLE | NAPI_CONFIGURABLE) != 0
            || (old_attributes & NAPI_WRITABLE == 0 && attributes & NAPI_WRITABLE != 0) {
            return record_status(env, NAPI_GENERIC_FAILURE);
        }
        if old_attributes & NAPI_WRITABLE == 0 && value_present && !same_value {
            return record_status(env, NAPI_GENERIC_FAILURE);
        }
    }
    let array_length = is_array_length_property(object, &key);
    if array_length && value_present && !same_value
        && env.as_ref().is_some_and(|owner| owner.frozen_objects.contains(&(object as usize))) {
        return record_status(env, NAPI_GENERIC_FAILURE);
    }
    let length_status = if array_length && !(old_attributes & NAPI_WRITABLE == 0 && !value_present) {
        set_array_length(env, object, value,
            attributes & NAPI_WRITABLE == 0)
    } else { NAPI_OK };
    if length_status != NAPI_OK {
        return record_status(env, length_status);
    }
    if array_length {
        // ArraySetLength already published the intrinsic descriptor state
        // before releasing removed accessor roots. A release callback may
        // redefine or freeze the Array; writing the pre-call `attributes`
        // below would revive a stale writable bit after that reentry.
        return NAPI_OK;
    }
    let (status, old) = {
        let Ok(owner) = env_mut(env) else { return NAPI_INVALID_ARG; };
        let identity = object as usize;
        if (!array_length && owner.nonextensible_objects.contains(&identity)
                && old_value.is_none() && old_accessor.is_none())
            || (!array_length && owner.frozen_objects.contains(&identity)
                && value_present && !same_value) {
            return record_status(env, NAPI_GENERIC_FAILURE);
        }
        // Omitting a value preserves an existing data slot, but converting a
        // configurable accessor to data must still install an own Undefined
        // slot. Accessors have no writable bit, so the old shortcut would
        // remove the accessor without creating its replacement.
        let status = if array_length || (old_accessor.is_none()
            && old_attributes & NAPI_WRITABLE == 0
            && (!value_present || same_value)) {
            NAPI_OK
        } else { set_own_property(owner, object, &key, value) };
        if status != NAPI_OK { return record_status(env, status); }
        let old = owner.accessors.remove(&(identity, key.clone()));
        record_property_order(owner, identity, &key);
        owner.property_attributes.insert((identity, key),
            attributes & NAPI_DEFAULT_PROPERTY_ATTRIBUTES);
        (status, old)
    };
    drop(old);
    record_status(env, status)
}

#[cfg(feature = "quickjs")]
unsafe fn qjs_construct_function(
    env: NapiEnv, handle: u64, args: &[NapiValue], out: *mut NapiValue,
) -> NapiStatus {
    let _foreign_dispatch = ForeignCallbackGuard::new();
    let arguments = {
        let Ok(owner) = env_mut(env) else { return NAPI_INVALID_ARG };
        owner.alloc(Value::Array(args.iter().copied().map(Some).collect()))
    };
    let guard = CallbackGraphArguments { env, value: arguments };
    let encoded = napi_result_graph_for_env(env, arguments, true);
    drop(guard);
    let Ok(encoded) = encoded else { return record_status(env, NAPI_GENERIC_FAILURE) };
    let Ok(encoded) = CString::new(encoded) else { return record_status(env, NAPI_INVALID_ARG) };
    let result = thaw_quickjs::thaw_js_construct_handle_graph_args_result(handle, encoded.as_ptr());
    qjs_adopt_handle_result(env, result, out)
}

#[cfg(feature = "quickjs")]
unsafe fn qjs_get_prototype(
    env: NapiEnv, handle: u64, out: *mut NapiValue,
) -> NapiStatus {
    let _foreign_dispatch = ForeignCallbackGuard::new();
    let result = thaw_quickjs::thaw_js_get_prototype_handle_result(handle);
    if result.error.is_null() && result.value != 0 {
        match qjs_query(result.value, 7) {
            Ok(kind) if kind == "1" => {
                thaw_quickjs::thaw_js_release_handle(result.value);
                let Ok(owner) = env_mut(env) else { return NAPI_INVALID_ARG };
                let null = owner.alloc(Value::Null);
                return write_value(out, null);
            }
            Ok(_) => {},
            Err(error) => {
                thaw_quickjs::thaw_js_release_handle(result.value);
                return qjs_error_status(env, error);
            }
        }
    }
    qjs_adopt_handle_result(env, result, out)
}

#[cfg(feature = "quickjs")]
unsafe fn qjs_property_names(
    env: NapiEnv, handle: u64, key_mode: i32, key_filter: u32,
    key_conversion: i32, out: *mut NapiValue,
) -> NapiStatus {
    let _foreign_dispatch = ForeignCallbackGuard::new();
    let result = thaw_quickjs::thaw_js_host_property_names_result(
        handle, key_mode, key_filter, key_conversion);
    let text = match qjs_take_result_text(result) {
        Ok(text) => text,
        Err(error) => return qjs_error_status(env, error),
    };
    let value = match parse_napi_graph_value(env, &text) {
        Ok(value) => value,
        Err(error) => return qjs_error_status(env, error),
    };
    if !matches!(value_ref(value), Ok(Value::Array(_))) {
        return record_status(env, NAPI_GENERIC_FAILURE);
    }
    write_value(out, value)
}

#[cfg(feature = "quickjs")]
unsafe fn qjs_set_prototype(env: NapiEnv, handle: u64, prototype: NapiValue) -> NapiStatus {
    let _foreign_dispatch = ForeignCallbackGuard::new();
    let arguments = {
        let Ok(owner) = env_mut(env) else { return NAPI_INVALID_ARG };
        owner.alloc(Value::Array(vec![Some(prototype)]))
    };
    let guard = CallbackGraphArguments { env, value: arguments };
    let encoded = napi_result_graph_for_env(env, arguments, true);
    drop(guard);
    let Ok(encoded) = encoded else { return record_status(env, NAPI_GENERIC_FAILURE) };
    let Ok(encoded) = CString::new(encoded) else { return record_status(env, NAPI_INVALID_ARG) };
    let result = thaw_quickjs::thaw_js_set_prototype_graph_result(handle, encoded.as_ptr());
    if !result.error.is_null() {
        let error = thaw_arena::NativeStr::from_ptr(result.error).to_string_lossy().into_owned();
        thaw_arena::destroy_string(result.error);
        return qjs_error_status(env, error);
    }
    if result.value == 0 { return record_status(env, NAPI_GENERIC_FAILURE); }
    NAPI_OK
}

#[cfg(feature = "quickjs")]
unsafe fn qjs_instanceof(env: NapiEnv, object: NapiValue, constructor: NapiValue, out: *mut bool) -> NapiStatus {
    let _foreign_dispatch = ForeignCallbackGuard::new();
    let arguments = {
        let Ok(owner) = env_mut(env) else { return NAPI_INVALID_ARG };
        owner.alloc(Value::Array(vec![Some(object), Some(constructor)]))
    };
    let guard = CallbackGraphArguments { env, value: arguments };
    let encoded = napi_result_graph_for_env(env, arguments, true);
    drop(guard);
    let Ok(encoded) = encoded else { return record_status(env, NAPI_GENERIC_FAILURE) };
    let Ok(encoded) = CString::new(encoded) else { return record_status(env, NAPI_INVALID_ARG) };
    let result = thaw_quickjs::thaw_js_instanceof_graph_result(encoded.as_ptr());
    if !result.error.is_null() {
        let error = thaw_arena::NativeStr::from_ptr(result.error).to_string_lossy().into_owned();
        thaw_arena::destroy_string(result.error);
        return qjs_error_status(env, error);
    }
    *out = result.value != 0;
    NAPI_OK
}

#[cfg(feature = "quickjs")]
unsafe fn qjs_instanceof_handles(
    env: NapiEnv, object: u64, constructor: u64, out: *mut bool,
) -> NapiStatus {
    let _foreign_dispatch = ForeignCallbackGuard::new();
    let result = thaw_quickjs::thaw_js_instanceof_handles_result(object, constructor);
    if !result.error.is_null() {
        let error = thaw_arena::NativeStr::from_ptr(result.error).to_string_lossy().into_owned();
        thaw_arena::destroy_string(result.error);
        return qjs_error_status(env, error);
    }
    *out = result.value != 0;
    NAPI_OK
}

#[cfg(feature = "quickjs")]
unsafe fn qjs_strict_equals_scalar(
    env: NapiEnv, handle: u64, scalar: NapiValue, out: *mut bool,
) -> NapiStatus {
    let _foreign_dispatch = ForeignCallbackGuard::new();
    let (kind, number, text) = match value_ref(scalar) {
        Ok(Value::Undefined) => (0, 0.0, String::new()),
        Ok(Value::Null) => (1, 0.0, String::new()),
        Ok(Value::Bool(value)) => (2, f64::from(u8::from(*value)), String::new()),
        Ok(Value::Number(value)) => (3, *value, String::new()),
        Ok(Value::String(value)) => {
            let units = env.as_ref().and_then(|owner| owner.utf16_strings.get(&(scalar as usize)));
            let encoded = match units {
                Some(units) => serde_json::to_string(units),
                None => serde_json::to_string(value),
            };
            let encoded = match encoded {
                Ok(text) => text, Err(_) => return record_status(env, NAPI_GENERIC_FAILURE),
            };
            (if units.is_some() { 6 } else { 4 }, 0.0, encoded)
        },
        Ok(Value::BigInt { negative, words }) => (5, 0.0, bigint_to_decimal(*negative, words)),
        _ => return record_status(env, NAPI_INVALID_ARG),
    };
    let Ok(text) = CString::new(text) else { return record_status(env, NAPI_INVALID_ARG) };
    let result = thaw_quickjs::thaw_js_strict_equal_handle_scalar_result(
        handle, kind, number, text.as_ptr());
    if !result.error.is_null() {
        let error = thaw_arena::NativeStr::from_ptr(result.error).to_string_lossy().into_owned();
        thaw_arena::destroy_string(result.error);
        return qjs_error_status(env, error);
    }
    *out = result.value != 0;
    NAPI_OK
}

unsafe fn parse_napi_arguments(
    env: NapiEnv, text: &str, preserve_undefined: bool, graph_args: bool,
    graph_input: Option<&NapiGraphInput>,
)
    -> Result<Vec<NapiValue>, String> {
    if graph_args {
        let root = match graph_input {
            Some(input) => input.decode(env)?,
            None => parse_napi_graph_value(env, text)?,
        };
        let Value::Array(arguments) = (unsafe { value_ref(root) })
            .map_err(|_| "native argument graph root is invalid")? else {
            return Err("native argument graph root is not an array".into());
        };
        return arguments.iter().map(|value|
            value.ok_or("native argument graph contains a hole".into())).collect();
    }
    let parsed: JsonValue = serde_json::from_str(text)
        .map_err(|error| format!("invalid argument JSON: {error}"))?;
    let arguments = parsed.as_array().ok_or("native arguments must be an array")?;
    arguments.iter().map(|value| {
        let env = env_mut(env).map_err(|_| "invalid native addon environment")?;
        value_from_json_with_undefined(env, value, preserve_undefined)
    }).collect()
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

// The N-API enumerator allocates a result array and one key Value per name.
// Keep those handles through property dispatch, then retire only these private
// enumeration temporaries on every success or error path.
struct OwnKeySnapshot {
    env: NapiEnv,
    items: Vec<(String, NapiValue)>,
    graph_keys: Vec<NapiValue>,
    allocated: Vec<NapiValue>,
}

impl Drop for OwnKeySnapshot {
    fn drop(&mut self) {
        let Ok(env) = (unsafe { env_mut(self.env) }) else { return };
        for value in self.allocated.drain(..) {
            if let Some(index) = env.values.iter().position(|item| *item == value) {
                env.utf16_strings.remove(&(value as usize));
                env.values.swap_remove(index);
                unsafe { drop(Box::from_raw(value)); }
            }
        }
    }
}

// Copy the own string key set before any accessor can reenter the bridge.
unsafe fn snapshot_own_string_keys(env: NapiEnv, object: NapiValue) -> Result<OwnKeySnapshot, String> {
    snapshot_own_keys(env, object, false)
}

// The graph codec preserves all own keys in N-API's enumeration order. The
// older plain JSON snapshot intentionally requests only string keys.
unsafe fn snapshot_own_graph_keys(env: NapiEnv, object: NapiValue) -> Result<OwnKeySnapshot, String> {
    snapshot_own_keys(env, object, true)
}

unsafe fn snapshot_own_keys(
    env: NapiEnv, object: NapiValue, include_symbols: bool,
) -> Result<OwnKeySnapshot, String> {
    let mut snapshot = OwnKeySnapshot {
        env, items: Vec::new(), graph_keys: Vec::new(), allocated: Vec::new(),
    };
    let mut names = ptr::null_mut();
    let status = napi_get_all_property_names(env, object, NAPI_KEY_OWN_ONLY,
        NAPI_KEY_ALL_PROPERTIES | (if include_symbols { 0 } else { NAPI_KEY_SKIP_SYMBOLS }),
        NAPI_KEY_NUMBERS_TO_STRINGS, &mut names);
    if !names.is_null() {
        snapshot.allocated.push(names);
        let Value::Array(keys) = value_ref(names).map_err(|_| "invalid native property names")? else {
            return Err("native property names were not an array".into());
        };
        let keys = keys.clone();
        for key in keys {
            let key = key.ok_or("invalid native property name")?;
            snapshot.graph_keys.push(key);
            match value_ref(key).map_err(|_| "invalid native property name")? {
                Value::String(name) => {
                    snapshot.allocated.push(key);
                    snapshot.items.push((name.clone(), key));
                }
                Value::Symbol { .. } | Value::QuickJsHandle { .. } if include_symbols => {},
                _ => return Err("native property name was not a string or Symbol".into()),
            }
        }
    }
    take_env_exception(env)?;
    if status != NAPI_OK { return Err(format!("failed to enumerate native properties: status {status}")); }
    if names.is_null() { return Err("native property names were not an array".into()); }
    Ok(snapshot)
}

// Use the N-API dispatcher so descriptor callbacks receive the original object as `this`.
unsafe fn snapshot_property(env: NapiEnv, object: NapiValue, name: &str, key: NapiValue) -> Result<NapiValue, String> {
    let mut value = ptr::null_mut();
    let status = napi_get_property(env, object, key, &mut value);
    take_env_exception(env)?;
    if status != NAPI_OK || value.is_null() {
        return Err(format!("failed to read native property `{name}`: status {status}"));
    }
    Ok(value)
}

// JSON arrays preserve their entry count and holes, but an own numeric
// accessor at an existing index must be read through the same dispatcher.
unsafe fn snapshot_array_keys(env: NapiEnv, array: NapiValue) -> Result<(OwnKeySnapshot, Vec<Option<NapiValue>>), String> {
    let length = match value_ref(array).map_err(|_| "invalid native array")? {
        Value::Array(values) => values.len(),
        _ => return Err("native value was not an array".into()),
    };
    let keys = snapshot_own_string_keys(env, array)?;
    let lookup = keys.items.iter().map(|(name, value)| (name.as_str(), *value))
        .collect::<HashMap<_, _>>();
    let mut items = Vec::with_capacity(length);
    for index in 0..length {
        items.push(lookup.get(index.to_string().as_str()).copied());
    }
    Ok((keys, items))
}

unsafe fn owner_env_for_value(value: NapiValue) -> Result<NapiEnv, String> {
    HOST.with(|host| {
        let host = host.borrow();
        host.module_envs.iter().chain(host.pending_call_envs.iter())
            .find(|env| !env.finalized && env.values.contains(&value))
            .map(|env| &**env as *const Env as NapiEnv)
            .ok_or_else(|| "native addon result belongs to an unknown environment".into())
    })
}

// A callback may place another loaded Env's value into an object or Promise.
// Prefer the explicit pending owner for its own values, otherwise use the
// actual registered owner for accessor metadata and temporary key storage.
unsafe fn snapshot_owner_env(preferred: NapiEnv, value: NapiValue) -> Result<NapiEnv, String> {
    if preferred.as_ref().is_some_and(|env| !env.finalized && env.values.contains(&value)) {
        Ok(preferred)
    } else {
        owner_env_for_value(value)
    }
}

unsafe fn quickjs_reference_wire(env: NapiEnv, value: NapiValue, root: bool) -> Result<QuickJsWireValue, String> {
    let _dispatch = ForeignCallbackGuard::new();
    let mut origins = Vec::new();
    let value = quickjs_reference_json(env, value, root, &mut HashSet::new(), &[], &mut origins)?;
    Ok(QuickJsWireValue { value, origins })
}

/// Snapshot one reference while keeping child identities on the existing wire ID.
unsafe fn quickjs_reference_json(
    env: NapiEnv,
    value: NapiValue,
    root: bool,
    seen: &mut HashSet<usize>,
    path: &[JsonValue],
    origins: &mut Vec<JsonValue>,
) -> Result<JsonValue, String> {
    let env = snapshot_owner_env(env, value)?;
    if !root {
        if let Some(reference) = env.as_ref().and_then(|env| env.quickjs_references.iter()
            .find(|(_, item)| **item == value).map(|(reference, _)| *reference)) {
            return Ok(serde_json::json!({ "__thaw_napi_ref__": reference }));
        }
    }
    if !seen.insert(value as usize) {
        return Ok(serde_json::json!({ "__thaw_napi_handle__": (value as u64).to_string() }));
    }
    let plain_object = matches!(value_ref(value), Ok(Value::Object(_)))
        && !env.as_ref().is_some_and(|env| env.instances.contains_key(&(value as usize)));
    let array = matches!(value_ref(value), Ok(Value::Array(_)));
    let array = if array { Some(snapshot_array_keys(env, value)?) } else { None };
    let result = if plain_object {
        origins.push(serde_json::json!([path, "plain"]));
        let keys = snapshot_own_string_keys(env, value)?;
        let mut out = serde_json::Map::new();
        for (key, key_value) in keys.items.iter().cloned() {
            let child = snapshot_property(env, value, &key, key_value)?;
            let child_owner = snapshot_owner_env(env, child)?;
            let is_reference = child_owner.as_ref().is_some_and(|env|
                env.quickjs_references.values().any(|item| *item == child));
            if !matches!(value_ref(child), Ok(Value::Symbol { .. }))
                && (!matches!(value_ref(child), Ok(Value::Function(_))) || is_reference) {
                let child_path = quickjs_child_path(path, JsonValue::String(key.clone()));
                out.insert(key, quickjs_reference_json(env, child, false, seen, &child_path, origins)?);
            }
        }
        JsonValue::Object(out)
    } else if let Some((_array_keys, children)) = array {
        JsonValue::Array(children.into_iter().enumerate().map(|(index, child)| match child {
            Some(key) => {
                let child = snapshot_property(env, value, &index.to_string(), key)?;
                let child_path = quickjs_child_path(path, serde_json::json!(index));
                quickjs_reference_json(env, child, false, seen, &child_path, origins)
            }
            None => Ok(JsonValue::Null),
        }).collect::<Result<_, _>>()?)
    } else {
        match value_ref(value).map_err(|_| "invalid napi_value")? {
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
        }
    };
    seen.remove(&(value as usize));
    Ok(result)
}

unsafe fn quickjs_bridge_value(value: NapiValue) -> Result<QuickJsWireValue, String> {
    let _dispatch = ForeignCallbackGuard::new();
    let marker = |value| Ok(QuickJsWireValue { value, origins: Vec::new() });
    let symbol_owner = owner_env_for_value(value)?;
    match value_ref(value) {
        Ok(Value::Promise(_)) => return marker(serde_json::json!({ "__thaw_napi_promise__": (value as u64).to_string() })),
        Ok(Value::Symbol { id, description }) => {
            let global = GLOBAL_SYMBOLS.get().is_some_and(|symbols| {
                symbols.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
                    .get(&symbol_owner.as_ref().and_then(|owner| owner.utf16_symbols.get(&(value as usize)))
                        .cloned().unwrap_or_else(|| description.encode_utf16().collect())) == Some(id)
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
    let env = HOST.with(|host| {
        let host = host.borrow();
        host.module_envs.iter().chain(host.pending_call_envs.iter())
            .find(|env| !env.finalized && env.values.contains(&value))
            .map(|env| &**env as *const Env)
            .ok_or_else(|| "native addon result belongs to an unknown environment".to_string())
    })?;
    quickjs_reference_wire(env as NapiEnv, value, false)
}

unsafe fn json_from_value_with_undefined(
    value: NapiValue,
    preserve_undefined: bool,
) -> Result<JsonValue, String> {
    json_from_value_with_undefined_for_env(ptr::null_mut(), value, preserve_undefined)
}

unsafe fn json_from_value_with_undefined_for_env(
    env: NapiEnv,
    value: NapiValue,
    preserve_undefined: bool,
) -> Result<JsonValue, String> {
    json_from_value_with_undefined_for_env_mode(env, value, preserve_undefined, true)
}

// Exception reporting walks stored data only. Calling an accessor while
// describing an already-thrown value can throw again and replace that failure.
unsafe fn json_from_value_with_undefined_for_env_mode(
    env: NapiEnv,
    value: NapiValue,
    preserve_undefined: bool,
    read_accessors: bool,
) -> Result<JsonValue, String> {
    json_from_value_with_undefined_for_env_mode_inner(
        env, value, preserve_undefined, read_accessors, &mut HashSet::new(),
    )
}

unsafe fn json_from_value_with_undefined_for_env_mode_inner(
    env: NapiEnv,
    value: NapiValue,
    preserve_undefined: bool,
    read_accessors: bool,
    seen: &mut HashSet<usize>,
) -> Result<JsonValue, String> {
    let _dispatch = ForeignCallbackGuard::new();
    // Path-local tracking applies only to data-only exception reports.
    // A repeated child reached through a different branch is not a cycle.
    let tracked = !read_accessors && matches!(value_ref(value),
        Ok(Value::Object(_) | Value::Array(_) | Value::Promise(_)));
    if tracked && !seen.insert(value as usize) {
        return Err("cyclic native exception value".into());
    }
    let result = (|| -> Result<JsonValue, String> {
    if matches!(value_ref(value), Ok(Value::Object(_))) {
        if is_native_instance(value as usize) {
            return Ok(serde_json::json!({ "__thaw_napi_handle__": (value as u64).to_string() }));
        }
        if !read_accessors {
            let Value::Object(fields) = value_ref(value).map_err(|_| "invalid napi_value")? else { unreachable!() };
            let fields = fields.iter().filter_map(|(key, child)| match key {
                PropertyKey::String(name) => Some((name.clone(), *child)),
                PropertyKey::Symbol(_) | PropertyKey::Utf16(_) => None,
            }).collect::<Vec<_>>();
            let mut out = serde_json::Map::new();
            for (key, child) in fields {
                if !matches!(value_ref(child), Ok(Value::Function(_) | Value::Symbol { .. })) {
                    out.insert(key, json_from_value_with_undefined_for_env_mode_inner(env, child, preserve_undefined, false, seen)?);
                }
            }
            return Ok(JsonValue::Object(out));
        }
        let env = snapshot_owner_env(env, value)?;
        let keys = snapshot_own_string_keys(env, value)?;
        let mut out = serde_json::Map::new();
        for (key, key_value) in keys.items.iter().cloned() {
            let child = snapshot_property(env, value, &key, key_value)?;
            if !matches!(value_ref(child), Ok(Value::Function(_) | Value::Symbol { .. })) {
                out.insert(key, json_from_value_with_undefined_for_env_mode_inner(env, child, preserve_undefined, read_accessors, seen)?);
            }
        }
        return Ok(JsonValue::Object(out));
    }
    let array = matches!(value_ref(value), Ok(Value::Array(_)));
    let array = if array && read_accessors {
        let env = snapshot_owner_env(env, value)?;
        Some(snapshot_array_keys(env, value)?)
    } else { None };
    if !read_accessors && matches!(value_ref(value), Ok(Value::Array(_))) {
        let Value::Array(values) = value_ref(value).map_err(|_| "invalid napi_value")? else { unreachable!() };
        let values = values.clone();
        return Ok(JsonValue::Array(values.into_iter().map(|child| match child {
            Some(child) if matches!(value_ref(child), Ok(Value::Function(_) | Value::Symbol { .. })) => Ok(JsonValue::Null),
            Some(child) => json_from_value_with_undefined_for_env_mode_inner(env, child, preserve_undefined, false, seen),
            None => Ok(JsonValue::Null),
        }).collect::<Result<_, _>>()?));
    }
    if let Some((_array_keys, values)) = array {
        let env = snapshot_owner_env(env, value)?;
        return Ok(JsonValue::Array(values.into_iter().enumerate().map(|(index, key)| match key {
            Some(key) => {
                let child = snapshot_property(env, value, &index.to_string(), key)?;
                if matches!(value_ref(child), Ok(Value::Function(_) | Value::Symbol { .. })) {
                    Ok(JsonValue::Null)
                } else {
                    json_from_value_with_undefined_for_env_mode_inner(env, child, preserve_undefined, read_accessors, seen)
                }
            }
            None => Ok(JsonValue::Null),
        }).collect::<Result<_, _>>()?));
    }
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
        Value::Array(_) | Value::Object(_) => unreachable!("containers handled before scalar match"),
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
        Value::QuickJsHandle { .. } => return Err("live JavaScript value requires graph transport".into()),
        Value::External(_) => return Err("cannot JSON-encode an external value".into()),
        Value::BigInt { .. } => return Err("cannot JSON-encode a BigInt".into()),
        Value::Promise(state) => {
            // Getter dispatch during recursive JSON encoding can settle the same
            // deferred again. Keep only the settled tag and handle across it.
            let settled = {
                let state = state.borrow();
                match *state {
                    PromiseState::Pending => None,
                    PromiseState::Resolved(value) => Some((true, value)),
                    PromiseState::Rejected(value) => Some((false, value)),
                }
            };
            let Some((resolved, settled_value)) = settled else {
                return Err("native addon returned a pending Promise".into());
            };
            if resolved {
                json_from_value_with_undefined_for_env_mode_inner(env, settled_value, preserve_undefined, read_accessors, seen)?
            } else {
                let message = match value_ref(settled_value).map_err(|_| "invalid Promise rejection")? {
                    Value::Error(message) | Value::String(message) => message.clone(),
                    _ => json_from_value_with_undefined_for_env_mode_inner(env, settled_value, preserve_undefined, read_accessors, seen)?.to_string(),
                };
                return Err(message);
            }
        },
    })
    })();
    if tracked { seen.remove(&(value as usize)); }
    result
}

// Private result wire: metadata lives in graph tokens rather than user object
// keys, so a native Date/undefined/non-finite value cannot be confused with
// an addon's ordinary object carrying the same fields.
struct NapiResultGraph {
    env: NapiEnv,
    wrapper_kinds: HashMap<usize, NapiGraphWrapperKind>,
    ids: HashMap<usize, usize>,
    pending: Vec<NapiValue>,
    nodes: Vec<JsonValue>,
    leases: GraphLeases,
    preserve_undefined: bool,
}

impl NapiResultGraph {
    unsafe fn token(&mut self, value: NapiValue) -> Result<JsonValue, String> {
        if matches!(value_ref(value), Ok(Value::Promise(_))) {
            return self.token(wait_for_promise(value)?);
        }
        match value_ref(value).map_err(|_| "invalid napi_value")? {
            Value::Undefined => Ok(if self.preserve_undefined { serde_json::json!({"u": 1}) }
                else { serde_json::json!({"v": null}) }),
            Value::Null => Ok(serde_json::json!({"v": null})),
            Value::Bool(value) => Ok(serde_json::json!({"v": value})),
            Value::String(text) => {
                let units = snapshot_owner_env(self.env, value)?.as_ref()
                    .and_then(|owner| owner.utf16_strings.get(&(value as usize))).cloned();
                Ok(match units {
                    Some(units) => serde_json::json!({"su": units}),
                    None => serde_json::json!({"v": text}),
                })
            },
            Value::Error(value) => Ok(serde_json::json!({"v": value})),
            Value::Number(number) if !number.is_finite() => Ok(serde_json::json!({
                "nf": if number.is_nan() { "NaN" } else if *number > 0.0 { "Infinity" } else { "-Infinity" }
            })),
            Value::Number(number) => Ok(serde_json::json!({"v": number})),
            Value::QuickJsHandle { .. } => {
                let identity = value as usize;
                let index = if let Some(index) = self.ids.get(&identity) { *index } else {
                    let index = self.pending.len();
                    self.ids.insert(identity, index);
                    self.pending.push(value);
                    index
                };
                Ok(serde_json::json!({"r": index}))
            },
            Value::Function(_) => {
                let identity = value as usize;
                let index = if let Some(index) = self.ids.get(&identity) { *index } else {
                    let index = self.pending.len();
                    self.ids.insert(identity, index);
                    self.pending.push(value);
                    index
                };
                Ok(serde_json::json!({"r": index}))
            },
            Value::Symbol { .. } => {
                let reference = retain_napi_graph_handle(value as u64);
                if reference == 0 { return Err("cannot retain native Symbol graph value".into()); }
                self.leases.napi.push(reference);
                Ok(serde_json::json!({"nsy": (value as u64).to_string()}))
            },
            Value::BigInt { negative, words } => Ok(serde_json::json!({
                "bi": bigint_to_decimal(*negative, words)
            })),
            Value::External(_) => Err("cannot JSON-encode an external value".into()),
            _ => {
                let identity = value as usize;
                let index = if let Some(index) = self.ids.get(&identity) { *index } else {
                    let index = self.pending.len();
                    self.ids.insert(identity, index);
                    self.pending.push(value);
                    index
                };
                Ok(serde_json::json!({"r": index}))
            }
        }
    }

    unsafe fn encode(mut self, value: NapiValue) -> Result<String, String> {
        let root = self.token(wait_for_promise(value)?)?;
        while self.nodes.len() < self.pending.len() {
            let value = self.pending[self.nodes.len()];
            let owner_env = snapshot_owner_env(self.env, value)?;
            let wrapper_kind = owner_env.as_ref()
                .and_then(|env| env.graph_wrappers.get(&(value as usize))).copied()
                .or_else(|| self.wrapper_kinds.get(&(value as usize)).copied());
            let array = matches!(value_ref(value), Ok(Value::Array(_)));
            let array = if array { Some(snapshot_array_keys(owner_env, value)?) } else { None };
            let node = if let Ok(Value::QuickJsHandle { handle, .. }) = value_ref(value) {
                #[cfg(not(feature = "quickjs"))]
                return Err("JavaScript graph handles require QuickJS".into());
                #[cfg(feature = "quickjs")]
                if thaw_quickjs::thaw_js_retain_handle(*handle) == 0 {
                    return Err("released JavaScript graph handle".into());
                }
                #[cfg(feature = "quickjs")]
                self.leases.host.push(*handle);
                serde_json::json!({"hdl": handle})
            } else if matches!(value_ref(value), Ok(Value::Function(_))) {
                let reference = retain_napi_graph_handle(value as u64);
                if reference == 0 { return Err("cannot retain native Function graph value".into()); }
                self.leases.napi.push(reference);
                serde_json::json!({"nfn": (value as u64).to_string()})
            } else if let Some((_array_keys, values)) = array {
                if owner_env.as_ref().is_some_and(|owner|
                    owner.live_constructor_returns.contains(&(value as usize))
                    || owner.accessors.keys().any(|(object, _)| *object == value as usize)) {
                    let reference = retain_napi_graph_handle(value as u64);
                    if reference == 0 {
                        return Err("cannot retain live native addon graph array".into());
                    }
                    self.leases.napi.push(reference);
                    self.nodes.push(serde_json::json!({"nh": (value as u64).to_string()}));
                    continue;
                }
                let mut items = Vec::with_capacity(values.len());
                for (index, key) in values.into_iter().enumerate() {
                    items.push(match key {
                        Some(key) => {
                            let child = snapshot_property(owner_env, value, &index.to_string(), key)?;
                            self.token(child)?
                        }
                        None => serde_json::json!({"h": 1}),
                    });
                }
                let keys = snapshot_own_graph_keys(owner_env, value)?;
                let mut properties = Vec::with_capacity(keys.graph_keys.len());
                for key_value in keys.graph_keys.iter().copied() {
                    let native_key = property_key(owner_env, key_value)
                        .map_err(|status| format!("invalid native array graph key: status {status}"))?;
                    let wire_key = match value_ref(key_value).map_err(|_| "invalid native array graph key")? {
                        Value::String(key) => owner_env.as_ref()
                            .and_then(|owner| owner.utf16_strings.get(&(key_value as usize)))
                            .map(|units| serde_json::json!({"su": units}))
                            .unwrap_or_else(|| serde_json::json!(key)),
                        Value::Symbol { .. } | Value::QuickJsHandle { .. } => self.token(key_value)?,
                        _ => return Err("invalid native array graph key type".into()),
                    };
                    let child = snapshot_property(owner_env, value, "array graph key", key_value)?;
                    let attributes = property_attributes_for(owner_env, value as usize, &native_key);
                    properties.push(serde_json::json!([wire_key, self.token(child)?, attributes]));
                }
                serde_json::json!({"a": items, "p": properties})
            } else if matches!(value_ref(value), Ok(Value::Object(_))) {
                let has_accessor = owner_env.as_ref().is_some_and(|owner|
                    owner.accessors.keys().any(|(object, _)| *object == value as usize));
                let explicit_live = owner_env.as_ref().is_some_and(|owner|
                    owner.live_constructor_returns.contains(&(value as usize)));
                if is_native_instance(value as usize) || has_accessor || explicit_live {
                    let reference = retain_napi_graph_handle(value as u64);
                    if reference == 0 { return Err("cannot retain live native addon graph object".into()); }
                    self.leases.napi.push(reference);
                    serde_json::json!({"nh": (value as u64).to_string()})
                } else if matches!(wrapper_kind, Some(NapiGraphWrapperKind::Map | NapiGraphWrapperKind::Set)) {
                    let map = wrapper_kind == Some(NapiGraphWrapperKind::Map);
                    let key = if map { "__thaw_map_entries__" } else { "__thaw_set_values__" };
                    let keys = snapshot_own_string_keys(owner_env, value)?;
                    let key_value = keys.items.iter()
                        .find(|(name, _)| name == key).map(|(_, key_value)| *key_value)
                        .ok_or("native graph wrapper lost its entries")?;
                    let child = snapshot_property(owner_env, value, key, key_value)?;
                    if map { serde_json::json!({"m": self.token(child)?}) }
                    else { serde_json::json!({"s": self.token(child)?}) }
                } else if wrapper_kind == Some(NapiGraphWrapperKind::RegExp) {
                    let keys = snapshot_own_string_keys(owner_env, value)?;
                    let key_value = keys.items.iter()
                        .find(|(name, _)| name == "__thaw_regexp__").map(|(_, key_value)| *key_value)
                        .ok_or("native RegExp graph wrapper lost its pattern")?;
                    let pattern = snapshot_property(owner_env, value, "__thaw_regexp__", key_value)?;
                    let pattern_env = snapshot_owner_env(owner_env, pattern)?;
                    let own_keys = snapshot_own_string_keys(pattern_env, pattern)?;
                    let mut encoded = Vec::with_capacity(3);
                    for (index, name) in ["source", "flags", "lastIndex"].into_iter().enumerate() {
                        let key_value = own_keys.items.iter().find(|(key, _)| key == name)
                            .map(|(_, key_value)| *key_value)
                            .ok_or_else(|| format!("native RegExp pattern lost `{name}`"))?;
                        let child = snapshot_property(pattern_env, pattern, name, key_value)?;
                        if index == 2 {
                            if let Value::Number(number) = value_ref(child).map_err(|_| "invalid native RegExp lastIndex")? {
                                if !number.is_finite() {
                                    encoded.push(JsonValue::String(if number.is_nan() { "NaN" }
                                        else if *number > 0.0 { "Infinity" } else { "-Infinity" }.into()));
                                    continue;
                                }
                            }
                        }
                        encoded.push(json_from_value_with_undefined_for_env(owner_env, child, true)?);
                    }
                    serde_json::json!({"re": encoded})
                } else {
                    let keys = snapshot_own_graph_keys(owner_env, value)?;
                    let mut entries = Vec::new();
                    for key_value in keys.graph_keys.iter().copied() {
                        let native_key = property_key(owner_env, key_value)
                            .map_err(|status| format!("invalid native graph property key: status {status}"))?;
                        let wire_key = match value_ref(key_value).map_err(|_| "invalid native graph key")? {
                            Value::String(key) => owner_env.as_ref()
                                .and_then(|owner| owner.utf16_strings.get(&(key_value as usize)))
                                .map(|units| serde_json::json!({"su": units}))
                                .unwrap_or_else(|| serde_json::json!(key)),
                            Value::Symbol { .. } | Value::QuickJsHandle { .. } => self.token(key_value)?,
                            _ => return Err("invalid native graph key type".into()),
                        };
                        let child = snapshot_property(owner_env, value, "graph key", key_value)?;
                        let attributes = property_attributes_for(owner_env, value as usize, &native_key);
                        entries.push(serde_json::json!([wire_key, self.token(child)?, attributes]));
                    }
                    serde_json::json!({"o": entries})
                }
            } else {
                match value_ref(value).map_err(|_| "invalid napi_value")? {
                    Value::Date(time) => serde_json::json!({"d": if time.is_finite() { Some(*time) } else { None }}),
                Value::Buffer(bytes) => serde_json::json!({"b": bytes}),
                Value::ExternalBuffer { data, length } => {
                    let bytes = if *length == 0 { &[][..] } else { std::slice::from_raw_parts(*data, *length) };
                    serde_json::json!({"b": bytes})
                }
                Value::BufferView { array_buffer, byte_offset, length } => {
                    let (data, _, detached) = arraybuffer_parts(*array_buffer)
                        .map_err(|_| "invalid Buffer backing ArrayBuffer")?;
                    let bytes = if detached || *length == 0 { &[][..] }
                        else { std::slice::from_raw_parts(data.add(*byte_offset), *length) };
                    serde_json::json!({"b": bytes})
                }
                _ => return Err("cannot JSON-encode an ArrayBuffer view".into()),
                }
            };
            self.nodes.push(node);
        }
        let wire = serde_json::to_string(&serde_json::json!({
            "root": root, "nodes": self.nodes,
            "leases": &self.leases.host,
            "napiLeases": self.leases.napi.iter().map(u64::to_string).collect::<Vec<_>>(),
        })).map_err(|error| error.to_string())?;
        // The wire, not this temporary encoder, now owns these references.
        self.leases.host.clear();
        self.leases.napi.clear();
        Ok(wire)
    }
}

unsafe fn napi_result_graph_for_env(env: NapiEnv, value: NapiValue, preserve_undefined: bool) -> Result<String, String> {
    let _dispatch = ForeignCallbackGuard::new();
    let wrapper_kinds = env_mut(env).map_err(|_| "invalid native addon environment")?.graph_wrappers.clone();
    // No Env borrow is held by the walker; a property read may call addon JS.
    NapiResultGraph { env, wrapper_kinds, ids: HashMap::new(),
        pending: Vec::new(), nodes: Vec::new(),
        leases: GraphLeases { host: Vec::new(), napi: Vec::new() }, preserve_undefined }.encode(value)
}

unsafe fn napi_result_graph(value: NapiValue, preserve_undefined: bool) -> Result<String, String> {
    let _dispatch = ForeignCallbackGuard::new();
    let wrapper_kinds = HOST.with(|host| {
        let host = host.borrow();
        host.module_envs.iter().chain(host.pending_call_envs.iter())
            .find(|env| env.values.contains(&value))
            .map(|env| env.graph_wrappers.clone()).unwrap_or_default()
    });
    let env = owner_env_for_value(value)?;
    NapiResultGraph { env, wrapper_kinds, ids: HashMap::new(), pending: Vec::new(),
        nodes: Vec::new(), leases: GraphLeases { host: Vec::new(), napi: Vec::new() },
        preserve_undefined }.encode(value)
}

unsafe fn encode_napi_result(value: NapiValue, preserve_undefined: bool, graph_result: bool) -> Result<String, String> {
    let _dispatch = ForeignCallbackGuard::new();
    if graph_result { napi_result_graph(value, preserve_undefined) }
    else { serde_json::to_string(&json_from_value_with_undefined(value, preserve_undefined)?)
        .map_err(|error| error.to_string()) }
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
            .any(|env| !env.finalized && env.instances.contains_key(&value))
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
        if !HOST.with(|host| host.borrow().has_active_async_work())
            && !host_has_unfinalized_threadsafe()
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
    graph_result: bool,
    graph_input: Option<&NapiGraphInput>,
) -> Result<String, String> {
    let _dispatch = ForeignCallbackGuard::new();
    let result = wait_for_promise(call_value_impl(
        name, args_json, preserve_undefined, graph_result, graph_input,
    )?)?;
    encode_napi_result(result, preserve_undefined, graph_result)
}

unsafe fn call_value_impl(
    name: &str,
    args_json: &str,
    preserve_undefined: bool,
    graph_args: bool,
    graph_input: Option<&NapiGraphInput>,
) -> Result<NapiValue, String> {
    let _dispatch = ForeignCallbackGuard::new();
    let (function, env) = HOST
        .with(|host| {
            let host = host.borrow();
            Some((
                host.functions.get(name)?.clone(),
                host.exports.get(name)?.0 as NapiEnv,
            ))
        })
        .ok_or_else(|| format!("no such native addon function `{name}`"))?;
    let args = parse_napi_arguments(
        env, args_json, preserve_undefined, graph_args, graph_input,
    )?;
    let this_arg = env_mut(env).map_err(|_| "invalid native addon environment")?
        .alloc(Value::Undefined);
    let mut info = CallbackInfo {
        args,
        this_arg,
        new_target: ptr::null_mut(),
        data: function.data,
    };
    let result = callback_result(env, invoke_napi_callback(env, function.callback, &mut info));
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
            error: thaw_arena::owned_string(error),
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
        text(args).and_then(|args| call_impl(&name, &args, false, false, None))
    }))
}

#[no_mangle]
pub unsafe extern "C" fn thaw_napi_call_graph_result(
    name: *const c_char, args: *const c_char,
) -> ThawResult {
    let args_text = text(args);
    let graph_input = args_text.as_ref().ok().map(|text| NapiGraphInput::parse(text));
    text_result(text(name).and_then(|name| {
        args_text.and_then(|args| call_impl(&name, &args, false, true, graph_input.as_ref()))
    }))
}

#[no_mangle]
pub unsafe extern "C" fn thaw_napi_call_typed_result(
    name: *const c_char,
    args: *const c_char,
) -> ThawResult {
    text_result(text(name).and_then(|name| {
        text(args).and_then(|args| call_impl(&name, &args, true, false, None))
    }))
}

#[no_mangle]
pub unsafe extern "C" fn thaw_napi_get_export(name: *const c_char) -> u64 {
    get_export_result(name).unwrap_or(0)
}

#[no_mangle]
pub unsafe extern "C" fn thaw_napi_call_typed_graph_result(
    name: *const c_char, args: *const c_char,
) -> ThawResult {
    let args_text = text(args);
    let graph_input = args_text.as_ref().ok().map(|text| NapiGraphInput::parse(text));
    text_result(text(name).and_then(|name| {
        args_text.and_then(|args| call_impl(&name, &args, true, true, graph_input.as_ref()))
    }))
}

#[no_mangle]
pub unsafe extern "C" fn thaw_napi_get_export_typed_result(name: *const c_char) -> ThawNapiHandleResult {
    match get_export_result(name) {
        Ok(value) => returned_handle(value),
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
        error: thaw_arena::owned_string(error.into()),
    }
}

fn returned_handle(value: u64) -> ThawNapiHandleResult {
    if is_native_instance(value as usize) {
        let Ok(env) = (unsafe { module_env_for_handle(value) }) else {
            return handle_error("native addon instance is no longer live");
        };
        if unsafe { (*env).finalizing || (*env).shutdown_requested } {
            return handle_error("native addon instance is closing");
        }
        if unsafe { !(*env).escaped_native_handles.contains(&(value as usize)) } {
            let mut reference = ptr::null_mut();
            let status = unsafe { napi_create_reference(env, value as NapiValue, 1, &mut reference) };
            if status != NAPI_OK || reference.is_null() {
                return handle_error("cannot root returned native addon instance");
            }
            unsafe { (*env).escaped_native_handles.insert(value as usize) };
        }
        // A previous JS proxy may have queued finalization while a graph
        // transfer pinned this same instance. The compiled raw handle is now
        // an independent Env-owned root, with no graph pin at shutdown.
        unsafe { (*env).released_handles.remove(&(value as usize)) };
    }
    ThawNapiHandleResult { value, error: ptr::null_mut() }
}

unsafe fn module_env_for_handle(handle: u64) -> Result<NapiEnv, String> {
    if handle == 0 {
        return Err("invalid native addon handle 0".into());
    }
    HOST.with(|host| {
        host.borrow()
            .module_envs
            .iter()
            .find(|env| !env.finalized && env.values.contains(&(handle as NapiValue))
                && !env.finalized_handles.contains(&(handle as usize)))
            .map(|env| (&**env as *const Env).cast_mut())
            .ok_or_else(|| format!("unknown native addon handle {handle}"))
    })
}

unsafe fn module_arguments(
    env: NapiEnv, args: *const c_char,
    preserve_undefined: bool, graph_args: bool,
    graph_input: Option<&NapiGraphInput>,
) -> Result<Vec<NapiValue>, String> {
    let args = text(args)?;
    parse_napi_arguments(env, &args, preserve_undefined, graph_args, graph_input)
}

unsafe fn call_function_handle(
    callable: u64,
    args: *const c_char,
    preserve_undefined: bool, graph_args: bool,
    graph_input: Option<&NapiGraphInput>,
) -> Result<NapiValue, String> {
    let _dispatch = ForeignCallbackGuard::new();
    let env = module_env_for_handle(callable)?;
    let values = module_arguments(env, args, preserve_undefined, graph_args, graph_input)?;
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
    let value = callback_result(env, invoke_napi_callback(env, function.callback, &mut info));
    take_env_exception(env)?;
    wait_for_promise(value)
}

unsafe fn call_export_handle_impl(
    name: *const c_char,
    args: *const c_char,
    preserve_undefined: bool, graph_args: bool,
) -> ThawNapiHandleResult {
    let _dispatch = ForeignCallbackGuard::new();
    let graph_input = graph_input_for_args(args, graph_args);
    let result = (|| -> Result<u64, String> {
        let name = text(name)?;
        let callable = get_export_result(CString::new(name.as_str()).unwrap().as_ptr())?;
        if callable == 0 {
            return Err("unknown native addon export".into());
        }
        Ok(call_function_handle(callable, args, preserve_undefined, graph_args, graph_input.as_ref())? as u64)
    })();
    match result {
        Ok(value) => returned_handle(value),
        Err(error) => handle_error(error),
    }
}

#[no_mangle]
pub unsafe extern "C" fn thaw_napi_call_export_handle_typed_result(
    name: *const c_char,
    args: *const c_char,
) -> ThawNapiHandleResult {
    call_export_handle_impl(name, args, true, false)
}

#[no_mangle]
pub unsafe extern "C" fn thaw_napi_call_export_handle_typed_graph_args_result(
    name: *const c_char, args: *const c_char,
) -> ThawNapiHandleResult {
    call_export_handle_impl(name, args, true, true)
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
    function_count: usize, graph_args: bool,
) -> Result<NapiValue, String> {
    let _dispatch = ForeignCallbackGuard::new();
    let graph_input = graph_input_for_args(args, graph_args);
    if function_count != 0 && functions.is_null() {
        return Err("native addon function argument list is null".into());
    }
    let name = text(name)?;
    let callable = get_export_result(CString::new(name.as_str()).unwrap().as_ptr())?;
    let env = module_env_for_handle(callable)?;
    let mut values = module_arguments(env, args, true, graph_args, graph_input.as_ref())?;
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
                graph_args,
            );
            if let Some(value) =
                HOST.with(|host| host.borrow().compiled_callbacks.get(&callback_key).copied())
            {
                value
            } else {
                let bridge = Arc::new(ThawCallbackBridge {
                    callback: if graph_args { ThawCallback::ValueGraph(callback) }
                        else { ThawCallback::Value(callback) },
                    context: function.context as usize,
                });
                let value = env_mut(env)
                    .map_err(|_| "invalid native addon environment")?
                    .alloc(Value::Function(Function {
                        callback: thaw_compiled_callback,
                        data: Arc::as_ptr(&bridge) as *mut c_void,
                        properties: HashMap::new(),
                        _thaw_bridge: Some(bridge), _accessor_owner: None,
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
    let value = callback_result(env, invoke_napi_callback(env, exported.callback, &mut info));
    take_env_exception(env)?;
    wait_for_promise(value)
}

#[no_mangle]
pub unsafe extern "C" fn thaw_napi_call_export_handle_with_functions_typed_result(
    name: *const c_char, args: *const c_char,
    functions: *const ThawNativeFunctionArgument, function_count: usize,
) -> ThawNapiHandleResult {
    call_export_handle_with_functions_impl(name, args, functions, function_count, false)
}

#[no_mangle]
pub unsafe extern "C" fn thaw_napi_call_export_handle_with_functions_typed_graph_args_result(
    name: *const c_char, args: *const c_char,
    functions: *const ThawNativeFunctionArgument, function_count: usize,
) -> ThawNapiHandleResult {
    call_export_handle_with_functions_impl(name, args, functions, function_count, true)
}

unsafe fn call_export_handle_with_functions_impl(
    name: *const c_char, args: *const c_char,
    functions: *const ThawNativeFunctionArgument, function_count: usize,
    graph_args: bool,
) -> ThawNapiHandleResult {
    let _dispatch = ForeignCallbackGuard::new();
    let result = (|| -> Result<u64, String> {
        Ok(call_export_with_functions(name, args, functions, function_count, graph_args)? as u64)
    })();
    match result {
        Ok(value) => returned_handle(value),
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
    let _dispatch = ForeignCallbackGuard::new();
    text_result(
        call_export_with_functions(name, args, functions, function_count, false).and_then(|value| {
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
    let _dispatch = ForeignCallbackGuard::new();
    let result = call_function_handle(callable, args, true, false, None).and_then(|value| {
        serde_json::to_string(&json_from_value_with_undefined(value, true)?)
            .map_err(|error| error.to_string())
    });
    text_result(result)
}

#[no_mangle]
pub unsafe extern "C" fn thaw_napi_call_with_functions_typed_graph_result(
    name: *const c_char, args: *const c_char,
    functions: *const ThawNativeFunctionArgument, function_count: usize,
) -> ThawResult {
    let _dispatch = ForeignCallbackGuard::new();
    text_result(call_export_with_functions(name, args, functions, function_count, true)
        .and_then(|value| encode_napi_result(value, true, true)))
}

#[no_mangle]
pub unsafe extern "C" fn thaw_napi_call_handle_typed_graph_result(
    callable: u64, args: *const c_char,
) -> ThawResult {
    let _dispatch = ForeignCallbackGuard::new();
    let graph_input = graph_input_for_args(args, true);
    text_result(call_function_handle(callable, args, true, true, graph_input.as_ref())
        .and_then(|value| encode_napi_result(value, true, true)))
}

// `napi_create_error`/`napi_create_type_error`/etc. record the error's class
// name in `error_names` alongside its message (see `alloc_error`); tag it
// onto the message the same way `new Error(...)`/etc. do in thaw-hir
// (`\u{1}<Name>\u{1}<message>`, see `thaw_runtime::split_error_tag`) so a
// native addon's typed error still exposes `.name`/`instanceof` at the
// catching Thaw code's catch site instead of collapsing to an untagged
// (default-`Error`) string.
unsafe fn describe_env_exception(env: NapiEnv, exception: NapiValue) -> Result<String, String> {
    let _dispatch = ForeignCallbackGuard::new();
    let name = error_name_for_owner(env, exception as usize);
    let message = match value_ref(exception).map_err(|_| "invalid exception")? {
        Value::Error(message) | Value::String(message) => message.clone(),
        _ => json_from_value_with_undefined_for_env_mode(env, exception, false, false)
            .map(|value| value.to_string())
            .unwrap_or_else(|_| "native addon threw an unserializable exception value".into()),
    };
    // A report must leave no newly thrown exception in the Env. The data-only
    // walk above does not invoke accessors; this also clears any nested failure.
    if let Ok(env_ref) = env_mut(env) { env_ref.exception.take(); }
    Ok(match name {
        Some(name) => String::from_utf8(
            thaw_arena::error_wire::encode_tagged(name.as_bytes(), message.as_bytes(), None)
                .ok_or("invalid native error text")?,
        ).map_err(|_| "invalid native error text")?,
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

// Graph bridge callers can return a thrown QuickJS value itself, retaining
// its lease until the receiving realm decodes it. Native-only exceptions keep
// the established tagged-text fallback.
unsafe fn take_env_exception_graph(env: NapiEnv) -> Result<Option<JsonValue>, String> {
    let Some(exception) = env_mut(env)
        .map_err(|_| "invalid native addon environment")?
        .exception.take()
    else { return Ok(None); };
    #[cfg(feature = "quickjs")]
    if matches!(value_ref(exception), Ok(Value::QuickJsHandle { .. } | Value::Symbol { .. })) {
        return match napi_result_graph_for_env(env, exception, true) {
            Ok(wire) => Ok(Some(serde_json::json!({ "__thaw_exception_graph__": wire }))),
            Err(_) => Err(describe_env_exception(env, exception)?),
        };
    }
    Err(describe_env_exception(env, exception)?)
}

unsafe fn construct_handle_impl(
    constructor: u64,
    args: *const c_char,
    preserve_undefined: bool, graph_args: bool,
) -> ThawNapiHandleResult {
    let _dispatch = ForeignCallbackGuard::new();
    let graph_input = graph_input_for_args(args, graph_args);
    let result = (|| -> Result<u64, String> {
        let env = module_env_for_handle(constructor)?;
        let values = module_arguments(env, args, preserve_undefined, graph_args, graph_input.as_ref())?;
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
        Ok(value) => returned_handle(value),
        Err(error) => handle_error(error),
    }
}

#[no_mangle]
pub unsafe extern "C" fn thaw_napi_construct_handle_result(
    constructor: u64,
    args: *const c_char,
) -> ThawNapiHandleResult {
    construct_handle_impl(constructor, args, false, false)
}

#[no_mangle]
pub unsafe extern "C" fn thaw_napi_construct_handle_typed_result(
    constructor: u64,
    args: *const c_char,
) -> ThawNapiHandleResult {
    construct_handle_impl(constructor, args, true, false)
}

#[no_mangle]
pub unsafe extern "C" fn thaw_napi_construct_handle_typed_graph_args_result(
    constructor: u64, args: *const c_char,
) -> ThawNapiHandleResult {
    construct_handle_impl(constructor, args, true, true)
}

unsafe fn call_method_value_impl(
    receiver: u64,
    method: *const c_char,
    args: *const c_char,
    preserve_undefined: bool,
    graph_args: bool,
) -> Result<NapiValue, String> {
    let _dispatch = ForeignCallbackGuard::new();
    let graph_input = graph_input_for_args(args, graph_args);
    (|| -> Result<NapiValue, String> {
        let env = module_env_for_handle(receiver)?;
        let method_name = text(method)?;
        let values = module_arguments(env, args, preserve_undefined, graph_args, graph_input.as_ref())?;
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
        let value = callback_result(env, invoke_napi_callback(env, function.callback, &mut info));
        take_env_exception(env)?;
        wait_for_promise(value)
    })()
}

unsafe fn call_method_impl(
    receiver: u64,
    method: *const c_char,
    args: *const c_char,
    preserve_undefined: bool,
    graph_result: bool,
) -> ThawResult {
    let _dispatch = ForeignCallbackGuard::new();
    text_result(call_method_value_impl(receiver, method, args, preserve_undefined, graph_result)
        .and_then(|value| encode_napi_result(value, preserve_undefined, graph_result)))
}

#[no_mangle]
pub unsafe extern "C" fn thaw_napi_call_method_result(
    receiver: u64,
    method: *const c_char,
    args: *const c_char,
) -> ThawResult {
    call_method_impl(receiver, method, args, false, false)
}

#[no_mangle]
pub unsafe extern "C" fn thaw_napi_call_method_typed_result(
    receiver: u64,
    method: *const c_char,
    args: *const c_char,
) -> ThawResult {
    call_method_impl(receiver, method, args, true, false)
}

#[no_mangle]
pub unsafe extern "C" fn thaw_napi_call_method_typed_graph_result(
    receiver: u64, method: *const c_char, args: *const c_char,
) -> ThawResult {
    call_method_impl(receiver, method, args, true, true)
}

unsafe fn get_property_impl(
    receiver: u64,
    property: *const c_char,
    preserve_undefined: bool, graph_result: bool,
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
        encode_napi_result(value, preserve_undefined, graph_result)
    })();
    text_result(result)
}

#[no_mangle]
pub unsafe extern "C" fn thaw_napi_get_property_result(
    receiver: u64,
    property: *const c_char,
) -> ThawResult {
    get_property_impl(receiver, property, false, false)
}

#[no_mangle]
pub unsafe extern "C" fn thaw_napi_get_property_typed_result(
    receiver: u64,
    property: *const c_char,
) -> ThawResult {
    get_property_impl(receiver, property, true, false)
}

#[no_mangle]
pub unsafe extern "C" fn thaw_napi_get_property_typed_graph_result(
    receiver: u64, property: *const c_char,
) -> ThawResult {
    get_property_impl(receiver, property, true, true)
}

unsafe fn set_property_impl(
    receiver: u64,
    property: *const c_char,
    args: *const c_char,
    preserve_undefined: bool, graph_result: bool,
    discard_result: bool,
) -> ThawResult {
    let _dispatch = ForeignCallbackGuard::new();
    let graph_input = graph_input_for_args(args, graph_result);
    let result = (|| -> Result<String, String> {
        let env = module_env_for_handle(receiver)?;
        let property_name = text(property)?;
        let property_name_c =
            CString::new(property_name.clone()).map_err(|_| "property contains NUL")?;
        let values = module_arguments(env, args, preserve_undefined, graph_result, graph_input.as_ref())?;
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
            encode_napi_result(*value, preserve_undefined, graph_result)
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
    set_property_impl(receiver, property, args, false, false, false)
}

#[no_mangle]
pub unsafe extern "C" fn thaw_napi_set_property_typed_result(
    receiver: u64,
    property: *const c_char,
    args: *const c_char,
) -> ThawResult {
    set_property_impl(receiver, property, args, true, false, false)
}

#[no_mangle]
pub unsafe extern "C" fn thaw_napi_set_property_typed_graph_result(
    receiver: u64, property: *const c_char, args: *const c_char,
) -> ThawResult {
    set_property_impl(receiver, property, args, true, true, false)
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


// The synthetic argument array is only a synchronous graph-encode root.
// Remove it on every error or reentrant return before invoking JS.
struct CallbackGraphArguments { env: NapiEnv, value: NapiValue }
impl Drop for CallbackGraphArguments {
    fn drop(&mut self) {
        let Ok(env) = (unsafe { env_mut(self.env) }) else { return };
        if let Some(index) = env.values.iter().position(|value| *value == self.value) {
            env.values.swap_remove(index);
            unsafe { drop(Box::from_raw(self.value)); }
        }
    }
}

unsafe extern "C" fn thaw_compiled_callback(_env: NapiEnv, info: NapiCallbackInfo) -> NapiValue {
    let _dispatch = ForeignCallbackGuard::new();
    let Some(info) = info.as_ref() else {
        return ptr::null_mut();
    };
    let Some(bridge) = (info.data as *const ThawCallbackBridge).as_ref() else {
        return ptr::null_mut();
    };
    let context = bridge.context as *mut c_void;
    let callback = match bridge.callback {
        ThawCallback::Value(callback) => Some((callback, false, false)),
        ThawCallback::ValueGraph(callback) => Some((callback, false, true)),
        #[cfg(feature = "quickjs")]
        ThawCallback::QuickJs(callback) => Some((callback, true, false)),
        ThawCallback::Event(_) | ThawCallback::EventGraph(_) => None,
    };
    if let Some((callback, _is_quickjs, graph_wire)) = callback {
        if graph_wire {
            let arguments = {
                let env = env_mut(_env).unwrap();
                env.alloc(Value::Array(info.args.iter().copied().map(Some).collect()))
            };
            let arguments_guard = CallbackGraphArguments { env: _env, value: arguments };
            let encoded = napi_result_graph_for_env(_env, arguments, true);
            drop(arguments_guard);
            let Ok(encoded) = encoded else { return ptr::null_mut(); };
            let Ok(encoded) = CString::new(encoded) else { return ptr::null_mut(); };
            // No Env reference or HOST borrow survives into external JS.
            let result = callback(context, encoded.as_ptr());
            let Ok(result) = text(result) else { return ptr::null_mut(); };
            if let Some(message) = result.strip_prefix('\u{2}') {
                let env = env_mut(_env).unwrap();
                env.exception = Some(env.alloc(Value::Error(message.into())));
                return ptr::null_mut();
            }
            return match parse_napi_graph_value(_env, &result) {
                Ok(value) => value,
                Err(message) => {
                    let env = env_mut(_env).unwrap();
                    env.exception = Some(env.alloc(Value::Error(message)));
                    ptr::null_mut()
                }
            };
        }
        let args = info
            .args
            .iter()
            .map(|value| json_from_value_with_undefined_for_env(_env, *value, true))
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
        let result = callback(context, args.as_ptr());
        let Ok(result) = text(result) else { return ptr::null_mut(); };
        if let Some(message) = result.strip_prefix('\u{2}') {
            let env = env_mut(_env).unwrap();
            let error = if _is_quickjs {
                let message = serde_json::from_str::<String>(message)
                    .unwrap_or_else(|_| "invalid QuickJS callback error response".into());
                let framed = thaw_arena::error_wire::parse_tagged(message.as_bytes())
                    .and_then(|frame| {
                        let name = std::str::from_utf8(frame.chain).ok()?;
                        let body = std::str::from_utf8(frame.original.unwrap_or(frame.display)).ok()?;
                        Some((name, body))
                    });
                let (name, body) = framed.unwrap_or_else(|| message.strip_prefix('\u{1}')
                    .and_then(|tagged| tagged.split_once('\u{1}'))
                    .filter(|(name, _)| !name.is_empty())
                    .unwrap_or(("Error", message.as_str())));
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
    let (callback, graph_result) = match bridge.callback {
        ThawCallback::Event(callback) => (callback, false),
        ThawCallback::EventGraph(callback) => (callback, true),
        _ => unreachable!(),
    };
    let encode = |value: Option<NapiValue>| -> Result<String, String> {
        match value {
            Some(value) if graph_result => napi_result_graph(value, true),
            Some(value) => serde_json::to_string(&json_from_value(value)?)
                .map_err(|error| error.to_string()),
            None if graph_result => Ok(r#"{"root":{"v":null},"nodes":[],"leases":[]}"#.into()),
            None => Ok("null".into()),
        }
    };
    let record_encode_error = |message: String| {
        if let Ok(env) = unsafe { env_mut(_env) } {
            // A native getter may have installed the original exception.
            if env.exception.is_none() {
                let error = env.alloc(Value::Error(message));
                env.exception = Some(error);
            }
        }
    };
    let error = match encode(info.args.first().copied()) {
        Ok(wire) => wire,
        Err(message) => { record_encode_error(message); return ptr::null_mut(); }
    };
    // Both encoders produce JSON text, which escapes embedded NULs.
    let error = CString::new(error).unwrap();
    let result = match encode(info.args.get(1).copied()) {
        Ok(wire) => wire,
        Err(message) => {
            // The first graph wire now owns its NAPI reference tokens, but
            // no adapter was called to consume them.
            if graph_result { unsafe { thaw_json_discard_graph_wire(error.as_ptr()) }; }
            record_encode_error(message);
            return ptr::null_mut();
        }
    };
    let result = CString::new(result).unwrap();
    callback(
        context,
        error.as_ptr(),
        result.as_ptr(),
    );
    ptr::null_mut()
}

#[no_mangle]
pub unsafe extern "C" fn thaw_napi_call_with_callback_result(
    name: *const c_char, args: *const c_char,
    callback: Option<ThawNativeCallback>, context: *mut c_void,
) -> ThawResult {
    call_with_callback_impl(name, args, callback, context, false)
}

#[no_mangle]
pub unsafe extern "C" fn thaw_napi_call_with_callback_graph_result(
    name: *const c_char, args: *const c_char,
    callback: Option<ThawNativeCallback>, context: *mut c_void,
) -> ThawResult {
    call_with_callback_impl(name, args, callback, context, true)
}

unsafe fn call_with_callback_impl(
    name: *const c_char, args: *const c_char,
    callback: Option<ThawNativeCallback>, context: *mut c_void,
    graph_result: bool,
) -> ThawResult {
    let _dispatch = ForeignCallbackGuard::new();
    let graph_input = graph_input_for_args(args, graph_result);
    let result = (|| -> Result<String, String> {
        let name = text(name)?;
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
            graph_result,
        );
        let mut values = module_arguments(env_ptr, args, false, graph_result, graph_input.as_ref())?;
        let cached_callback =
            HOST.with(|host| host.borrow().compiled_callbacks.get(&callback_key).copied());
        let mut created_callback = None;
        let callback_value = if let Some(callback) = cached_callback {
            callback
        } else {
            let bridge = Arc::new(ThawCallbackBridge {
                callback: if graph_result { ThawCallback::EventGraph(callback) } else { ThawCallback::Event(callback) },
                context: context as usize,
            });
            let bridge_data = Arc::as_ptr(&bridge) as *mut c_void;
            let value = env_mut(env_ptr)
                .map_err(|_| "invalid native addon environment")?
                .alloc(Value::Function(Function {
                    callback: thaw_compiled_callback,
                    data: bridge_data,
                    properties: HashMap::new(),
                    _thaw_bridge: Some(bridge), _accessor_owner: None,
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
        let value = callback_result(env_ptr, invoke_napi_callback(env_ptr, function.callback, &mut info));
        if let Some(exception) = (*env_ptr).exception.take() {
            return Err(describe_env_exception(env_ptr, exception)?);
        }
        let value = wait_for_promise(value)?;
        let value = if value.is_null() {
            if graph_result { r#"{"root":{"v":null},"nodes":[],"leases":[]}"#.to_string() }
            else { "null".to_string() }
        } else {
            if graph_result { napi_result_graph(value, true)? }
            else { serde_json::to_string(&json_from_value(value)?)
                .map_err(|error| error.to_string())? }
        };
        if HOST.with(|host| host.borrow().has_active_async_work())
            || host_threadsafe_state(false)
        {
            if let Some(callback) = created_callback {
                HOST.with(|host| {
                    host.borrow_mut()
                        .compiled_callbacks
                        .insert(callback_key, callback);
                });
            }
        } else {
            HOST.with(|host| { host.borrow_mut().compiled_callbacks.remove(&callback_key); });
            retire_owned_envs();
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
            error: thaw_arena::owned_string(error),
        },
    }
}

#[no_mangle]
pub unsafe extern "C" fn thaw_napi_call_method_with_callback_result(
    receiver: u64, method: *const c_char, args: *const c_char,
    callback: Option<ThawNativeCallback>, context: *mut c_void,
    discard_result: u8,
) -> ThawResult {
    call_method_with_callback_impl(receiver, method, args, callback, context, discard_result, false)
}

#[no_mangle]
pub unsafe extern "C" fn thaw_napi_call_method_with_callback_graph_result(
    receiver: u64, method: *const c_char, args: *const c_char,
    callback: Option<ThawNativeCallback>, context: *mut c_void,
    discard_result: u8,
) -> ThawResult {
    call_method_with_callback_impl(receiver, method, args, callback, context, discard_result, true)
}

unsafe fn call_method_with_callback_impl(
    receiver: u64, method: *const c_char, args: *const c_char,
    callback: Option<ThawNativeCallback>, context: *mut c_void,
    discard_result: u8, graph_result: bool,
) -> ThawResult {
    let _dispatch = ForeignCallbackGuard::new();
    let graph_input = graph_input_for_args(args, graph_result);
    let result = (|| -> Result<String, String> {
        thaw_napi_poll_async_work();
        let env = module_env_for_handle(receiver)?;
        let method_name = text(method)?;
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
            graph_result,
        );
        let mut values = module_arguments(env, args, true, graph_result, graph_input.as_ref())?;
        let cached_callback =
            HOST.with(|host| host.borrow().compiled_callbacks.get(&callback_key).copied());
        let callback_value = if let Some(callback) = cached_callback {
            callback
        } else {
            let bridge = Arc::new(ThawCallbackBridge {
                callback: if graph_result { ThawCallback::EventGraph(callback) } else { ThawCallback::Event(callback) },
                context: context as usize,
            });
            let bridge_data = Arc::as_ptr(&bridge) as *mut c_void;
            let value = (*env).alloc(Value::Function(Function {
                callback: thaw_compiled_callback,
                data: bridge_data,
                properties: HashMap::new(),
                _thaw_bridge: Some(bridge), _accessor_owner: None,
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
        let value = callback_result(env, invoke_napi_callback(env, function.callback, &mut info));
        take_env_exception(env)?;
        let value = wait_for_promise(value)?;
        if discard_result != 0 || value.is_null() {
            if graph_result { Ok(r#"{"root":{"v":null},"nodes":[],"leases":[]}"#.into()) }
            else { Ok("null".to_string()) }
        } else {
            encode_napi_result(value, true, graph_result)
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
        let error = thaw_arena::NativeStr::from_ptr(result.error)
            .to_string_lossy().into_owned();
        thaw_arena::destroy_string(result.error);
        CString::new(format!(
            "{{\"__thaw_error__\":{}}}",
            serde_json::to_string(&error).unwrap()
        ))
        .unwrap()
        .into_raw()
    }
}


#[cfg(test)]
mod bigint_graph_token_tests {
    use super::{bigint_to_decimal, decimal_bigint_words};

    #[test]
    fn arbitrary_words_round_trip_through_canonical_decimal() {
        for value in ["0", "-1", "9007199254740993", "-18446744073709551617"] {
            let (negative, words) = decimal_bigint_words(value).unwrap();
            assert_eq!(bigint_to_decimal(negative, &words), value);
        }
        for invalid in ["", "-0", "00", "+1", "01", "1.0", " 1"] {
            assert!(decimal_bigint_words(invalid).is_none());
        }
    }
}

#[cfg(all(test, feature = "quickjs"))]
mod native_graph_pin_tests {
    use super::*;

    thread_local! {
        static FINALIZATIONS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    }

    #[test]
    fn graph_release_rejects_ordinary_reference_and_duplicate_token() {
        let mut env = Box::new(Env::new());
        env.graph_owner_id = next_graph_owner_id().unwrap();
        let value = env.alloc(Value::Object(HashMap::new()));
        let env_ptr = &mut *env as NapiEnv;
        HOST.with(|host| host.borrow_mut().module_envs.push(env));

        let mut ordinary = ptr::null_mut();
        assert_eq!(unsafe { napi_create_reference(env_ptr, value, 1, &mut ordinary) }, NAPI_OK);
        let graph = retain_napi_graph_handle(value as u64);
        assert_ne!(graph, 0);
        assert_eq!(unsafe { (*env_ptr).native_graph_pins }, 1);
        assert_eq!(release_napi_graph_reference(ordinary as u64), 0);
        assert_eq!(unsafe { (*env_ptr).native_graph_pins }, 1);
        assert!(!unsafe { (*ordinary).deleted });
        assert_eq!(release_napi_graph_reference(graph), 1);
        assert_eq!(release_napi_graph_reference(graph), 0);
        assert_eq!(unsafe { (*env_ptr).native_graph_pins }, 0);
        assert!(!unsafe { (*ordinary).deleted });
        assert_eq!(unsafe { napi_delete_reference(env_ptr, ordinary) }, NAPI_OK);

        let env = HOST.with(|host| host.borrow_mut().module_envs.pop().unwrap());
        drop(env);
    }

    unsafe extern "C" fn finalize_graph_value(_: NapiEnv, data: *mut c_void, _: *mut c_void) {
        FINALIZATIONS.with(|count| count.set(count.get() + 1));
        // The handle must already be marked finalized before user callbacks.
        assert_eq!(retain_napi_graph_handle(data as u64), 0);
    }

    #[test]
    fn event_second_graph_encode_failure_discards_first_wire_pin() {
        unsafe { thaw_json_register_napi_handle_operations(
            retain_napi_graph_handle, release_napi_graph_reference,
        ); }
        unsafe extern "C" fn unexpected_callback(
            context: *mut c_void, _: *const c_char, _: *const c_char,
        ) {
            unsafe { *(context as *mut usize) += 1; }
        }
        let mut env = Box::new(Env::new());
        env.graph_owner_id = next_graph_owner_id().unwrap();
        let instance = env.alloc(Value::Object(HashMap::new()));
        env.instances.insert(instance as usize, 0);
        let symbol = env.alloc(Value::Symbol { id: 1, description: "second".into() });
        let env_ptr = &mut *env as NapiEnv;
        HOST.with(|host| host.borrow_mut().module_envs.push(env));
        let mut called = 0usize;
        let bridge = Arc::new(ThawCallbackBridge {
            callback: ThawCallback::EventGraph(unexpected_callback),
            context: (&mut called as *mut usize) as usize,
        });
        let mut info = CallbackInfo {
            args: vec![instance, symbol],
            this_arg: ptr::null_mut(),
            new_target: ptr::null_mut(),
            data: Arc::as_ptr(&bridge) as *mut c_void,
        };
        assert!(unsafe { thaw_compiled_callback(env_ptr, &mut info) }.is_null());
        assert_eq!(called, 0);
        assert_eq!(unsafe { (*env_ptr).native_graph_pins }, 0);
        assert!(unsafe { (*env_ptr).exception.is_some() });
        let env = HOST.with(|host| host.borrow_mut().module_envs.pop().unwrap());
        drop(env);
    }

    #[test]
    fn graph_pin_defers_proxy_release_and_rejects_finalized_handle() {
        FINALIZATIONS.with(|count| count.set(0));
        let mut env = Box::new(Env::new());
        env.graph_owner_id = next_graph_owner_id().unwrap();
        let value = env.alloc(Value::Object(HashMap::new()));
        env.instances.insert(value as usize, 0);
        env.object_finalizers.entry(value as usize).or_default().push(FinalizeRecord {
            data: value.cast(), finalize: Some(finalize_graph_value),
            hint: ptr::null_mut(), backing: ptr::null_mut(),
        });
        let env_ptr = &mut *env as NapiEnv;
        HOST.with(|host| host.borrow_mut().module_envs.push(env));

        let wire = unsafe { napi_result_graph_for_env(env_ptr, value, true) }.unwrap();
        let parsed: JsonValue = serde_json::from_str(&wire).unwrap();
        assert_eq!(parsed["nodes"][0]["nh"], (value as u64).to_string());
        assert_eq!(parsed["napiLeases"].as_array().unwrap().len(), 1);
        assert_eq!(unsafe { (*env_ptr).native_graph_pins }, 1);
        assert_eq!(unsafe { NapiGraphInput::parse(&wire).decode(env_ptr) }.unwrap(), value);
        assert_eq!(unsafe { (*env_ptr).native_graph_pins }, 0);

        let pin = retain_napi_graph_handle(value as u64);
        assert_ne!(pin, 0);
        assert_eq!(unsafe { (*env_ptr).native_graph_pins }, 1);
        let incoming = serde_json::json!({
            "root": {"r": 0}, "nodes": [{"nh": (value as u64).to_string()}],
            "leases": [], "napiLeases": [pin.to_string()],
        }).to_string();
        let outer = ForeignCallbackGuard::new();
        let input = NapiGraphInput::parse(&incoming);
        release_napi_handle(value as u64).unwrap();
        assert!(unsafe { (*env_ptr).released_handles.contains(&(value as usize)) });
        assert!(!unsafe { (*env_ptr).finalized_handles.contains(&(value as usize)) });
        assert_eq!(unsafe { input.decode(env_ptr) }.unwrap(), value);
        // A callback can still use and return the original instance after
        // decode; both recipient pins remain with the outer dispatch guard
        // through result encoding, even after the inner input is dropped.
        let returned = unsafe { napi_result_graph_for_env(env_ptr, value, true) }.unwrap();
        let returned_input = NapiGraphInput::parse(&returned);
        assert_eq!(unsafe { returned_input.decode(env_ptr) }.unwrap(), value);
        drop(returned_input);
        drop(input);
        assert_eq!(unsafe { (*env_ptr).native_graph_pins }, 2);
        FINALIZATIONS.with(|count| assert_eq!(count.get(), 0));
        unsafe { (*env_ptr).shutdown_requested = true };
        assert_eq!(retain_napi_graph_handle(value as u64), 0);
        retire_owned_envs();
        assert!(!unsafe { (*env_ptr).finalized });

        drop(outer);
        assert_eq!(unsafe { (*env_ptr).native_graph_pins }, 0);
        assert!(unsafe { (*env_ptr).finalized_handles.contains(&(value as usize)) });
        FINALIZATIONS.with(|count| assert_eq!(count.get(), 1));
        assert_eq!(retain_napi_graph_handle(value as u64), 0);
        let escaped = returned_handle(value as u64);
        assert!(!escaped.error.is_null());
        unsafe { thaw_arena::destroy_string(escaped.error) };

        let env = HOST.with(|host| host.borrow_mut().module_envs.pop().unwrap());
        drop(env);
    }

    #[test]
    fn reverse_graph_callback_keeps_instance_live_until_outer_dispatch_returns() {
        FINALIZATIONS.with(|count| count.set(0));
        let mut env = Box::new(Env::new());
        env.graph_owner_id = next_graph_owner_id().unwrap();
        let value = env.alloc(Value::Object(HashMap::new()));
        env.instances.insert(value as usize, 0);
        env.object_finalizers.entry(value as usize).or_default().push(FinalizeRecord {
            data: value.cast(), finalize: Some(finalize_graph_value),
            hint: ptr::null_mut(), backing: ptr::null_mut(),
        });
        let env_ptr = &mut *env as NapiEnv;
        HOST.with(|host| host.borrow_mut().module_envs.push(env));
        let pin = retain_napi_graph_handle(value as u64);
        assert_ne!(pin, 0);
        let wire = serde_json::json!({
            "root": {"r": 0}, "nodes": [{"nh": (value as u64).to_string()}],
            "leases": [], "napiLeases": [pin.to_string()],
        }).to_string();
        let outer = ForeignCallbackGuard::new();
        assert_eq!(unsafe { parse_napi_graph_value(env_ptr, &wire) }.unwrap(), value);
        release_napi_handle(value as u64).unwrap();
        assert_eq!(unsafe { (*env_ptr).native_graph_pins }, 1);
        assert!(!unsafe { (*env_ptr).finalized_handles.contains(&(value as usize)) });
        drop(outer);
        assert_eq!(unsafe { (*env_ptr).native_graph_pins }, 0);
        assert!(unsafe { (*env_ptr).finalized_handles.contains(&(value as usize)) });
        FINALIZATIONS.with(|count| assert_eq!(count.get(), 1));
        let env = HOST.with(|host| host.borrow_mut().module_envs.pop().unwrap());
        drop(env);
    }

    #[test]
    fn returned_raw_instance_has_one_env_root_after_proxy_release() {
        FINALIZATIONS.with(|count| count.set(0));
        let mut env = Box::new(Env::new());
        env.graph_owner_id = next_graph_owner_id().unwrap();
        let value = env.alloc(Value::Object(HashMap::new()));
        env.instances.insert(value as usize, 0);
        env.object_finalizers.entry(value as usize).or_default().push(FinalizeRecord {
            data: value.cast(), finalize: Some(finalize_graph_value),
            hint: ptr::null_mut(), backing: ptr::null_mut(),
        });
        let env_ptr = &mut *env as NapiEnv;
        HOST.with(|host| host.borrow_mut().module_envs.push(env));

        // The old proxy can queue release while a graph transfer holds the
        // instance. Acquire the raw result's independent Env root before
        // dropping that temporary wire pin.
        let pin = retain_napi_graph_handle(value as u64);
        assert_ne!(pin, 0);
        release_napi_handle(value as u64).unwrap();
        assert!(unsafe { (*env_ptr).released_handles.contains(&(value as usize)) });
        assert!(returned_handle(value as u64).error.is_null());
        assert!(!unsafe { (*env_ptr).released_handles.contains(&(value as usize)) });
        assert_eq!(unsafe { (*env_ptr).references.iter()
            .filter(|reference| !reference.deleted && reference.count > 0).count() }, 2);
        assert_eq!(unsafe { (*env_ptr).native_graph_pins }, 1);
        assert_eq!(release_napi_graph_reference(pin), 1);
        assert_eq!(unsafe { (*env_ptr).native_graph_pins }, 0);
        FINALIZATIONS.with(|count| assert_eq!(count.get(), 0));
        release_napi_handle(value as u64).unwrap();
        assert!(!unsafe { (*env_ptr).finalized_handles.contains(&(value as usize)) });
        assert!(returned_handle(value as u64).error.is_null());
        assert_eq!(unsafe { (*env_ptr).references.iter()
            .filter(|reference| !reference.deleted && reference.count > 0).count() }, 1);
        assert!(!unsafe { (*env_ptr).released_handles.contains(&(value as usize)) });
        release_napi_handle(value as u64).unwrap();
        assert!(unsafe { (*env_ptr).released_handles.contains(&(value as usize)) });
        FINALIZATIONS.with(|count| assert_eq!(count.get(), 0));

        unsafe { (*env_ptr).shutdown_requested = true };
        retire_owned_envs();
        assert!(unsafe { (*env_ptr).finalized });
        FINALIZATIONS.with(|count| assert_eq!(count.get(), 1));
        let env = HOST.with(|host| host.borrow_mut().module_envs.pop().unwrap());
        drop(env);
    }
}

#[cfg(all(test, feature = "quickjs"))]
mod typed_native_graph_value_tests {
    use super::*;

    // Unrun: duplicate retained QuickJS objects are independently returned
    // or thrown after their creator scope has closed. The recipient scope or
    // exception slot must be the owner before its transfer retain is dropped.
    #[test]
    fn duplicate_live_handle_result_and_throw_transfer_their_native_root() {
        unsafe {
            assert_eq!(thaw_quickjs::eval_json(
                "globalThis.__thaw_duplicate_handle_control = {}; true"
            ).unwrap().as_deref(), Some("true"));
            for thrown in [false, true] {
                let mut env = Env::new();
                let env_ptr: NapiEnv = &mut env;
                let parent = env.alloc(Value::Object(HashMap::new()));
                let mut creator = ptr::null_mut();
                assert_eq!(napi_open_handle_scope(env_ptr, &mut creator), NAPI_OK);
                let handle = thaw_quickjs::thaw_js_get_global(
                    c"__thaw_duplicate_handle_control".as_ptr());
                assert_ne!(handle, 0);
                let mut first = ptr::null_mut();
                assert_eq!(qjs_adopt_handle_result(env_ptr,
                    thaw_quickjs::ThawHandleResult { value: handle, error: ptr::null() },
                    &mut first), NAPI_OK);
                assert_eq!(napi_set_named_property(env_ptr, parent,
                    c"child".as_ptr(), first), NAPI_OK);
                assert_eq!(napi_close_handle_scope(env_ptr, creator), NAPI_OK);
                let mut recipient = ptr::null_mut();
                assert_eq!(napi_open_handle_scope(env_ptr, &mut recipient), NAPI_OK);
                assert_eq!(thaw_quickjs::thaw_js_retain_handle(handle), 1);
                if thrown {
                    assert_eq!(qjs_adopt_thrown_handle(env_ptr, handle, true),
                        NAPI_PENDING_EXCEPTION);
                    assert_eq!(env.exception, Some(first));
                } else {
                    let mut second = ptr::null_mut();
                    assert_eq!(qjs_adopt_handle_result(env_ptr,
                        thaw_quickjs::ThawHandleResult { value: handle, error: ptr::null() },
                        &mut second), NAPI_OK);
                    assert_eq!(second, first);
                }
                let key = env.alloc(Value::String("child".into()));
                let mut deleted = false;
                assert_eq!(napi_delete_property(env_ptr, parent, key, &mut deleted), NAPI_OK);
                assert!(deleted);
                sweep_pending_scope_values(env_ptr);
                assert!(matches!(value_ref(first), Ok(Value::QuickJsHandle { .. })));
                if thrown {
                    let mut cleared = ptr::null_mut();
                    assert_eq!(napi_get_and_clear_last_exception(env_ptr, &mut cleared), NAPI_OK);
                    assert_eq!(cleared, first);
                }
                assert_eq!(napi_close_handle_scope(env_ptr, recipient), NAPI_OK);
            }
        }
    }

    // Unrun integration control: a JS-created permanent Symbol must come
    // back from native graph transport as the original realm Symbol, both
    // as a normal result and as a thrown value. Symbol.iterator cannot be
    // reconstructed by Symbol.for(description).
    #[test]
    fn js_permanent_symbols_roundtrip_original_identity_and_throw() {
        unsafe {
            thaw_quickjs::register_napi_bridge(
                thaw_napi_export_names, thaw_napi_call_typed_bridge,
                thaw_napi_handle_bridge, thaw_napi_poll_async_work,
                thaw_napi_async_work_pending, thaw_napi_graph_owner,
                release_napi_graph_reference,
            );
            let mut env = Box::new(Env::new());
            env.graph_owner_id = next_graph_owner_id().unwrap();
            env.host_managed = true;
            let object = env.alloc(Value::Object(HashMap::new()));
            let env_ptr = &mut *env as NapiEnv;
            HOST.with(|host| host.borrow_mut().module_envs.push(env));
            let bootstrap = format!(
                "__thaw_json_graph_proxy_for_handle('{}'); true", object as u64);
            assert_eq!(thaw_quickjs::eval_json(&bootstrap).unwrap().as_deref(), Some("true"));
            for source in ["Symbol.iterator", "Symbol.for('thaw-weak-permanent')"] {
                let install = format!("globalThis.__thaw_weak_symbol_control = {source}; true");
                assert_eq!(thaw_quickjs::eval_json(&install).unwrap().as_deref(), Some("true"));
                let original = thaw_quickjs::thaw_js_get_global(
                    c"__thaw_weak_symbol_control".as_ptr());
                assert_ne!(original, 0);
                let native = qjs_adopt_symbol(env_ptr, original).unwrap().unwrap();
                assert_eq!(thaw_quickjs::thaw_js_release_handle(original), 1);
                let id = match value_ref(native).unwrap() {
                    Value::Symbol { id, .. } => *id,
                    _ => panic!("native Symbol expected"),
                };
                assert!((*env_ptr).js_original_symbol_ids.contains(&id));
                assert!((*env_ptr).strong_symbol_ids.contains(&id));
                assert!(!(*env_ptr).js_pinned_symbol_ids.contains(&id));
                let revived = thaw_quickjs::thaw_js_native_symbol_handle_result(native as u64);
                assert!(revived.error.is_null());
                assert_ne!(revived.value, 0);
                let original = thaw_quickjs::thaw_js_get_global(
                    c"__thaw_weak_symbol_control".as_ptr());
                let same = thaw_quickjs::thaw_js_strict_equal_handles_result(
                    original, revived.value);
                assert!(same.error.is_null());
                assert_eq!(same.value, 1);
                assert_eq!(thaw_quickjs::thaw_js_release_handle(original), 1);
                assert_eq!(thaw_quickjs::thaw_js_release_handle(revived.value), 1);
                for thrown in [false, true] {
                    let wire = if thrown {
                        (*env_ptr).exception = Some(native);
                        take_env_exception_graph(env_ptr).unwrap().unwrap()
                            ["__thaw_exception_graph__"].as_str().unwrap().to_owned()
                    } else {
                        napi_result_graph_for_env(env_ptr, native, true).unwrap()
                    };
                    let compare = format!(
                        "__thaw_json_graph_decode_owned({}) === globalThis.__thaw_weak_symbol_control",
                        serde_json::to_string(&wire).unwrap());
                    assert_eq!(thaw_quickjs::eval_json(&compare).unwrap().as_deref(), Some("true"));
                }
            }
            thaw_quickjs::eval_json("delete globalThis.__thaw_weak_symbol_control; true").unwrap();
            let env = HOST.with(|host| host.borrow_mut().module_envs.pop().unwrap());
            drop(env);
        }
    }

    #[test]
    fn adopted_symbol_root_accepts_registered_and_well_known_without_weak_pin() {
        let mut env = Env::new();
        for description in ["registered", "well-known"] {
            let id = NEXT_SYMBOL_ID.fetch_add(1, Ordering::Relaxed);
            let symbol = env.alloc(Value::Symbol { id, description: description.into() });
            env.symbols.insert(id, symbol);
            env.strong_symbol_ids.insert(id);
            let generation = env.value_generations[&(symbol as usize)];
            assert!(adopted_symbol_root_proven(&env, id, symbol, generation));
            assert!(!env.js_pinned_symbol_ids.contains(&id));
            // A stale/reused raw address must not pass the direct result or
            // thrown-handle transfer check, even for a permanent Symbol.
            assert!(!adopted_symbol_root_proven(&env, id, symbol, generation + 1));
        }
    }

    #[test]
    fn adopted_local_symbol_requires_native_pin_before_transfer_release() {
        let mut env = Env::new();
        let id = NEXT_SYMBOL_ID.fetch_add(1, Ordering::Relaxed);
        let symbol = env.alloc(Value::Symbol { id, description: "local".into() });
        env.symbols.insert(id, symbol);
        env.js_origin_symbol_ids.insert(id);
        let generation = env.value_generations[&(symbol as usize)];
        assert!(!adopted_symbol_root_proven(&env, id, symbol, generation));
        env.js_pinned_symbol_ids.insert(id);
        assert!(adopted_symbol_root_proven(&env, id, symbol, generation));
    }

    #[test]
    fn symbol_metadata_distinguishes_js_origin_from_native_pin_owner() {
        let mut env = Box::new(Env::new());
        env.graph_owner_id = next_graph_owner_id().unwrap();
        let js_id = NEXT_SYMBOL_ID.fetch_add(1, Ordering::Relaxed);
        let js = env.alloc(Value::Symbol { id: js_id, description: "JS".into() });
        env.symbols.insert(js_id, js);
        env.js_origin_symbol_ids.insert(js_id);
        env.js_original_symbol_ids.insert(js_id);
        let permanent_id = NEXT_SYMBOL_ID.fetch_add(1, Ordering::Relaxed);
        let permanent = env.alloc(Value::Symbol { id: permanent_id,
            description: "well-known".into() });
        env.symbols.insert(permanent_id, permanent);
        env.strong_symbol_ids.insert(permanent_id);
        env.js_original_symbol_ids.insert(permanent_id);
        let native_id = NEXT_SYMBOL_ID.fetch_add(1, Ordering::Relaxed);
        let native = env.alloc(Value::Symbol { id: native_id, description: "native".into() });
        env.symbols.insert(native_id, native);
        HOST.with(|host| host.borrow_mut().module_envs.push(env));
        for (value, js_origin) in [(js, true), (permanent, true), (native, false)] {
            let target = CString::new((value as u64).to_string()).unwrap();
            let raw = unsafe { thaw_napi_handle_bridge(c"symbol_metadata".as_ptr(),
                target.as_ptr(), c"".as_ptr(), c"[]".as_ptr()) };
            assert!(!raw.is_null());
            let response: JsonValue = serde_json::from_str(
                unsafe { std::ffi::CStr::from_ptr(raw) }.to_str().unwrap()).unwrap();
            unsafe { drop(CString::from_raw(raw.cast_mut())); }
            assert_eq!(response["value"]["jsOrigin"], js_origin);
        }
        let env = HOST.with(|host| host.borrow_mut().module_envs.pop().unwrap());
        drop(env);
    }

    #[test]
    fn transferred_symbol_graph_reference_pins_original_until_wire_consumed() {
        let mut env = Env::new();
        let id = NEXT_SYMBOL_ID.fetch_add(1, Ordering::Relaxed);
        let symbol = env.alloc(Value::Symbol { id, description: "wire".into() });
        env.symbols.insert(id, symbol);
        env.js_origin_symbol_ids.insert(id);
        let reference = alloc_reference(&mut env, symbol, 1);
        env.graph_reference_tokens.insert(reference as usize);
        let candidates = HashSet::from([symbol as usize]);
        // The JS weak-live notice is not itself a native root. The wire's
        // independent positive reference is, until the decoder releases it.
        assert!(!env.scope_reachable_values(&candidates, None, &[], false, false)
            .contains(&(symbol as usize)));
        assert!(env.scope_reachable_values(&candidates, None, &[], false, true)
            .contains(&(symbol as usize)));
        env.js_live_symbol_ids.insert(id);
        env.references.clear();
        env.graph_reference_tokens.clear();
        assert!(!env.scope_reachable_values(&candidates, None, &[], false, true)
            .contains(&(symbol as usize)));
    }

    #[test]
    fn thrown_native_symbol_uses_exact_graph_exception_lane() {
        let mut env = Box::new(Env::new());
        env.graph_owner_id = next_graph_owner_id().unwrap();
        let id = NEXT_SYMBOL_ID.fetch_add(1, Ordering::Relaxed);
        let symbol = env.alloc(Value::Symbol { id, description: "thrown".into() });
        env.symbols.insert(id, symbol);
        env.exception = Some(symbol);
        let env_ptr = &mut *env as NapiEnv;
        HOST.with(|host| host.borrow_mut().module_envs.push(env));
        let result = unsafe { take_env_exception_graph(env_ptr) }.unwrap().unwrap();
        let wire = result["__thaw_exception_graph__"].as_str().unwrap();
        let graph: JsonValue = serde_json::from_str(wire).unwrap();
        assert_eq!(graph["root"]["nsy"], (symbol as u64).to_string());
        drop(NapiGraphInput::parse(wire));
        assert!(unsafe { (*env_ptr).exception.is_none() });
        let env = HOST.with(|host| host.borrow_mut().module_envs.pop().unwrap());
        drop(env);
    }

    unsafe extern "C" fn echo_native(_: NapiEnv, _: NapiCallbackInfo) -> NapiValue {
        ptr::null_mut()
    }

    #[test]
    fn function_and_surrogate_symbol_have_distinct_graph_tokens_and_owned_pins() {
        let mut env = Box::new(Env::new());
        env.graph_owner_id = next_graph_owner_id().unwrap();
        let function = env.alloc(Value::Function(Function {
            callback: echo_native, data: ptr::null_mut(), properties: HashMap::new(),
            _thaw_bridge: None, _accessor_owner: None,
        }));
        let symbol = env.alloc(Value::Symbol {
            id: NEXT_SYMBOL_ID.fetch_add(1, Ordering::Relaxed),
            description: String::from_utf16_lossy(&[0xD800]),
        });
        env.utf16_symbols.insert(symbol as usize, vec![0xD800]);
        let env_ptr = &mut *env as NapiEnv;
        HOST.with(|host| host.borrow_mut().module_envs.push(env));

        let function_wire = unsafe { napi_result_graph_for_env(env_ptr, function, true) }.unwrap();
        let symbol_wire = unsafe { napi_result_graph_for_env(env_ptr, symbol, true) }.unwrap();
        let function_json: JsonValue = serde_json::from_str(&function_wire).unwrap();
        let symbol_json: JsonValue = serde_json::from_str(&symbol_wire).unwrap();
        assert_eq!(function_json["nodes"][0]["nfn"], (function as u64).to_string());
        assert_eq!(symbol_json["root"]["nsy"], (symbol as u64).to_string());
        assert_eq!(unsafe { (*env_ptr).native_graph_pins }, 2);
        assert_eq!(unsafe { NapiGraphInput::parse(&function_wire).decode(env_ptr) }.unwrap(), function);
        assert_eq!(unsafe { NapiGraphInput::parse(&symbol_wire).decode(env_ptr) }.unwrap(), symbol);
        assert_eq!(unsafe { (*env_ptr).native_graph_pins }, 0);
        assert_eq!(unsafe { (*env_ptr).utf16_symbols.get(&(symbol as usize)) }, Some(&vec![0xD800]));

        let env = HOST.with(|host| host.borrow_mut().module_envs.pop().unwrap());
        drop(env);
    }

    #[test]
    fn native_graph_preserves_symbol_key_order_flags_and_nested_value() {
        let mut env = Box::new(Env::new());
        env.graph_owner_id = next_graph_owner_id().unwrap();
        let symbol_id = NEXT_SYMBOL_ID.fetch_add(1, Ordering::Relaxed);
        let symbol = env.alloc(Value::Symbol { id: symbol_id, description: "key".into() });
        env.symbols.insert(symbol_id, symbol);
        let nested = env.alloc(Value::Object(HashMap::new()));
        let mut fields = HashMap::new();
        fields.insert(PropertyKey::Symbol(symbol_id), nested);
        let object = env.alloc(Value::Object(fields));
        env.property_order.insert(object as usize, vec![PropertyKey::Symbol(symbol_id)]);
        env.property_attributes.insert(
            (object as usize, PropertyKey::Symbol(symbol_id)), NAPI_WRITABLE,
        );
        let env_ptr = &mut *env as NapiEnv;
        HOST.with(|host| host.borrow_mut().module_envs.push(env));

        let wire = unsafe { napi_result_graph_for_env(env_ptr, object, true) }.unwrap();
        let graph: JsonValue = serde_json::from_str(&wire).unwrap();
        assert_eq!(graph["nodes"][0]["o"][0][0]["nsy"], (symbol as u64).to_string());
        assert_eq!(graph["nodes"][0]["o"][0][2], NAPI_WRITABLE);
        let decoded = unsafe { NapiGraphInput::parse(&wire).decode(env_ptr) }.unwrap();
        let Value::Object(decoded_fields) = unsafe { value_ref(decoded) }.unwrap() else {
            panic!("native graph result is not an object");
        };
        let child = decoded_fields.get(&PropertyKey::Symbol(symbol_id)).copied().unwrap();
        assert!(matches!(unsafe { value_ref(child) }, Ok(Value::Object(fields)) if fields.is_empty()));
        assert_eq!(unsafe { (*env_ptr).property_order.get(&(decoded as usize)) },
            Some(&vec![PropertyKey::Symbol(symbol_id)]));
        assert_eq!(unsafe { (*env_ptr).property_attributes.get(
            &(decoded as usize, PropertyKey::Symbol(symbol_id))) }, Some(&NAPI_WRITABLE));
        assert_eq!(unsafe { (*env_ptr).native_graph_pins }, 0);

        let env = HOST.with(|host| host.borrow_mut().module_envs.pop().unwrap());
        drop(env);
    }

    #[test]
    fn native_graph_array_retains_symbol_extra_property_and_index_attributes() {
        let mut env = Box::new(Env::new());
        env.graph_owner_id = next_graph_owner_id().unwrap();
        let symbol_id = NEXT_SYMBOL_ID.fetch_add(1, Ordering::Relaxed);
        let symbol = env.alloc(Value::Symbol { id: symbol_id, description: "extra".into() });
        env.symbols.insert(symbol_id, symbol);
        let indexed = env.alloc(Value::Number(7.0));
        let extra = env.alloc(Value::Object(HashMap::new()));
        let array = env.alloc(Value::Array(vec![Some(indexed), None]));
        env.host_properties.insert(array as usize,
            HashMap::from([(PropertyKey::Symbol(symbol_id), extra)]));
        env.property_order.insert(array as usize,
            vec![PropertyKey::Symbol(symbol_id)]);
        env.property_attributes.insert((array as usize, PropertyKey::String("0".into())),
            NAPI_ENUMERABLE);
        env.property_attributes.insert((array as usize, PropertyKey::Symbol(symbol_id)),
            NAPI_WRITABLE);
        let env_ptr = &mut *env as NapiEnv;
        HOST.with(|host| host.borrow_mut().module_envs.push(env));

        let wire = unsafe { napi_result_graph_for_env(env_ptr, array, true) }.unwrap();
        let graph: JsonValue = serde_json::from_str(&wire).unwrap();
        let props = graph["nodes"][0]["p"].as_array().unwrap();
        assert!(props.iter().any(|part| part[0]["nsy"] == (symbol as u64).to_string()
            && part[2] == NAPI_WRITABLE));
        assert!(props.iter().any(|part| part[0] == "0" && part[2] == NAPI_ENUMERABLE));
        let decoded = unsafe { NapiGraphInput::parse(&wire).decode(env_ptr) }.unwrap();
        assert!(matches!(unsafe { value_ref(decoded) }, Ok(Value::Array(values))
            if values.len() == 2 && values[0].is_some() && values[1].is_none()));
        assert_eq!(unsafe { (*env_ptr).host_properties.get(&(decoded as usize))
            .and_then(|properties| properties.get(&PropertyKey::Symbol(symbol_id))) },
            Some(&extra));
        assert_eq!(unsafe { (*env_ptr).property_attributes.get(
            &(decoded as usize, PropertyKey::String("0".into()))) }, Some(&NAPI_ENUMERABLE));
        assert_eq!(unsafe { (*env_ptr).native_graph_pins }, 0);

        let env = HOST.with(|host| host.borrow_mut().module_envs.pop().unwrap());
        drop(env);
    }
    #[test]
    fn counted_proxy_owner_keeps_scoped_instance_until_last_release() {
        let mut env = Box::new(Env::new());
        env.graph_owner_id = next_graph_owner_id().unwrap();
        let env_ptr = &mut *env as NapiEnv;
        HOST.with(|host| host.borrow_mut().module_envs.push(env));
        let mut scope = ptr::null_mut();
        assert_eq!(unsafe { napi_open_handle_scope(env_ptr, &mut scope) }, NAPI_OK);
        let value = unsafe { (*env_ptr).alloc(Value::Object(HashMap::new())) };
        unsafe { (*env_ptr).instances.insert(value as usize, 0); }
        let target = CString::new((value as u64).to_string()).unwrap();
        let dispatch = |operation: &std::ffi::CStr| {
            let raw = unsafe { thaw_napi_handle_bridge(operation.as_ptr(), target.as_ptr(),
                c"".as_ptr(), c"[]".as_ptr()) };
            assert!(!raw.is_null());
            let text = unsafe { std::ffi::CStr::from_ptr(raw) }.to_string_lossy().into_owned();
            unsafe { drop(CString::from_raw(raw.cast_mut())); }
            assert!(!text.contains("__thaw_error__"), "{text}");
        };
        dispatch(c"renew_handle");
        dispatch(c"renew_handle");
        assert_eq!(unsafe { (*env_ptr).proxy_live_counts.get(&(value as usize)) }, Some(&2));
        assert_eq!(unsafe { napi_close_handle_scope(env_ptr, scope) }, NAPI_OK);
        assert!(unsafe { (*env_ptr).values.contains(&value) });
        dispatch(c"release_handle");
        assert_eq!(unsafe { (*env_ptr).proxy_live_counts.get(&(value as usize)) }, Some(&1));
        assert!(unsafe { (*env_ptr).values.contains(&value) });
        dispatch(c"release_handle");
        assert!(!unsafe { (*env_ptr).values.contains(&value) });
        assert!(unsafe { value_ref(value) }.is_err());
        let env = HOST.with(|host| host.borrow_mut().module_envs.pop().unwrap());
        drop(env);
    }

    unsafe extern "C" fn finalize_scoped_proxy_with_reentry(
        env: NapiEnv, data: *mut c_void, _: *mut c_void,
    ) {
        FINALIZATIONS.with(|count| count.set(count.get() + 1));
        assert_eq!(retain_napi_graph_handle(data as u64), 0);
        let mut created = ptr::null_mut();
        assert_eq!(napi_create_double(env, 5.0, &mut created), NAPI_OK);
        assert!(matches!(value_ref(created), Ok(Value::Number(5.0))));
    }

    #[test]
    fn last_proxy_release_finalizes_once_before_scoped_slot_retirement() {
        FINALIZATIONS.with(|count| count.set(0));
        let mut env = Box::new(Env::new());
        env.graph_owner_id = next_graph_owner_id().unwrap();
        let env_ptr = &mut *env as NapiEnv;
        HOST.with(|host| host.borrow_mut().module_envs.push(env));
        let mut scope = ptr::null_mut();
        assert_eq!(unsafe { napi_open_handle_scope(env_ptr, &mut scope) }, NAPI_OK);
        let value = unsafe { (*env_ptr).alloc(Value::Object(HashMap::new())) };
        unsafe {
            (*env_ptr).instances.insert(value as usize, 0);
            (*env_ptr).object_finalizers.entry(value as usize).or_default().push(
                FinalizeRecord { data: value.cast(), finalize: Some(finalize_scoped_proxy_with_reentry),
                    hint: ptr::null_mut(), backing: ptr::null_mut() });
        }
        let target = CString::new((value as u64).to_string()).unwrap();
        for operation in [c"renew_handle".as_ref(), c"release_handle".as_ref()] {
            let raw = unsafe { thaw_napi_handle_bridge(operation.as_ptr(), target.as_ptr(),
                c"".as_ptr(), c"[]".as_ptr()) };
            assert!(!raw.is_null());
            let text = unsafe { std::ffi::CStr::from_ptr(raw) }.to_string_lossy().into_owned();
            unsafe { drop(CString::from_raw(raw.cast_mut())); }
            assert!(!text.contains("__thaw_error__"), "{text}");
            if operation == c"renew_handle".as_ref() {
                assert_eq!(unsafe { napi_close_handle_scope(env_ptr, scope) }, NAPI_OK);
            }
        }
        FINALIZATIONS.with(|count| assert_eq!(count.get(), 1));
        assert!(!unsafe { (*env_ptr).values.contains(&value) });
        assert!(unsafe { value_ref(value) }.is_err());
        let env = HOST.with(|host| host.borrow_mut().module_envs.pop().unwrap());
        drop(env);
    }

    #[test]
    fn last_proxy_release_defers_finalization_while_native_parent_reaches_child() {
        FINALIZATIONS.with(|count| count.set(0));
        let mut env = Box::new(Env::new());
        env.graph_owner_id = next_graph_owner_id().unwrap();
        let env_ptr = &mut *env as NapiEnv;
        HOST.with(|host| host.borrow_mut().module_envs.push(env));
        let parent = unsafe { (*env_ptr).alloc(Value::Object(HashMap::new())) };
        let mut scope = ptr::null_mut();
        assert_eq!(unsafe { napi_open_handle_scope(env_ptr, &mut scope) }, NAPI_OK);
        let child = unsafe { (*env_ptr).alloc(Value::Object(HashMap::new())) };
        unsafe {
            if let Value::Object(fields) = &mut *parent {
                fields.insert(PropertyKey::String("child".into()), child);
            }
            (*env_ptr).object_finalizers.entry(child as usize).or_default().push(
                FinalizeRecord { data: child.cast(), finalize: Some(finalize_scoped_proxy_with_reentry),
                    hint: ptr::null_mut(), backing: ptr::null_mut() });
        }
        let target = CString::new((child as u64).to_string()).unwrap();
        for operation in [c"renew_handle".as_ref(), c"release_handle".as_ref()] {
            let raw = unsafe { thaw_napi_handle_bridge(operation.as_ptr(), target.as_ptr(),
                c"".as_ptr(), c"[]".as_ptr()) };
            assert!(!raw.is_null());
            let text = unsafe { std::ffi::CStr::from_ptr(raw) }.to_string_lossy().into_owned();
            unsafe { drop(CString::from_raw(raw.cast_mut())); }
            assert!(!text.contains("__thaw_error__"), "{text}");
            if operation == c"renew_handle".as_ref() {
                assert_eq!(unsafe { napi_close_handle_scope(env_ptr, scope) }, NAPI_OK);
            }
        }
        FINALIZATIONS.with(|count| assert_eq!(count.get(), 0));
        assert!(unsafe { (*env_ptr).values.contains(&child) });
        assert!(matches!(unsafe { value_ref(child) }, Ok(Value::Object(_))));
        unsafe { if let Value::Object(fields) = &mut *parent {
            fields.remove(&PropertyKey::String("child".into()));
        } }
        unsafe { sweep_pending_scope_values(env_ptr); }
        FINALIZATIONS.with(|count| assert_eq!(count.get(), 1));
        assert!(!unsafe { (*env_ptr).values.contains(&child) });
        let env = HOST.with(|host| host.borrow_mut().module_envs.pop().unwrap());
        drop(env);
    }

    #[test]
    fn counted_binary_owner_retains_scoped_arraybuffer_until_release() {
        let mut env = Box::new(Env::new());
        env.graph_owner_id = next_graph_owner_id().unwrap();
        let env_ptr = &mut *env as NapiEnv;
        HOST.with(|host| host.borrow_mut().module_envs.push(env));
        let mut scope = ptr::null_mut();
        assert_eq!(unsafe { napi_open_handle_scope(env_ptr, &mut scope) }, NAPI_OK);
        let value = unsafe { (*env_ptr).alloc(Value::ArrayBuffer {
            bytes: vec![1, 2, 3], detached: false,
        }) };
        let target = CString::new((value as u64).to_string()).unwrap();
        for operation in [c"renew_handle".as_ref(), c"release_handle".as_ref()] {
            let raw = unsafe { thaw_napi_handle_bridge(operation.as_ptr(), target.as_ptr(),
                c"".as_ptr(), c"[]".as_ptr()) };
            assert!(!raw.is_null());
            let text = unsafe { std::ffi::CStr::from_ptr(raw) }.to_string_lossy().into_owned();
            unsafe { drop(CString::from_raw(raw.cast_mut())); }
            assert!(!text.contains("__thaw_error__"), "{text}");
            if operation == c"renew_handle".as_ref() {
                assert_eq!(unsafe { napi_close_handle_scope(env_ptr, scope) }, NAPI_OK);
                assert!(unsafe { (*env_ptr).values.contains(&value) });
            }
        }
        assert!(!unsafe { (*env_ptr).values.contains(&value) });
        let env = HOST.with(|host| host.borrow_mut().module_envs.pop().unwrap());
        drop(env);
    }

}
