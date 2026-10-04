#[no_mangle]
pub unsafe extern "C" fn napi_set_named_property(
    env: NapiEnv,
    object: NapiValue,
    name: *const c_char,
    value: NapiValue,
) -> NapiStatus {
    if !value_belongs_to_environment(env, object) || !value_belongs_to_environment(env, value) {
        return NAPI_INVALID_ARG;
    }
    let Ok(name) = text(name) else {
        return record_status(env, NAPI_INVALID_ARG);
    };
    let name = PropertyKey::String(name);
    #[cfg(feature = "quickjs")]
    if let Some(handle) = qjs_handle(object) {
        return qjs_set_property(env, handle, &name, value, false);
    }
    let _dispatch = ForeignCallbackGuard::new();
    let mut scope_sweep = ScopeMutationSweep::new(env);
    if let Some(accessor) = find_accessor(env, object, &name) {
        let Some(setter) = accessor.setter else {
            return record_status(env, NAPI_GENERIC_FAILURE);
        };
        let mut info = CallbackInfo {
            args: vec![value],
            this_arg: object,
            new_target: ptr::null_mut(),
            data: accessor.setter_data,
        };
        invoke_napi_callback(env, setter, &mut info);
        let status = if env_mut(env)
            .map(|env| env.exception.is_some())
            .unwrap_or(false)
        {
            NAPI_PENDING_EXCEPTION
        } else {
            NAPI_OK
        };
        return record_status(env, status);
    }
    let inherited_owner = find_data_property_owner(env, object, &name);
    let (frozen, nonextensible) = env
        .as_ref()
        .map(|env| {
            (
                env.frozen_objects.contains(&(object as usize)),
                env.nonextensible_objects.contains(&(object as usize)),
            )
        })
        .unwrap_or((false, false));
    let writable = inherited_owner
        .map(|owner| property_attributes_for(env, owner, &name) & NAPI_WRITABLE != 0)
        .unwrap_or(true);
    if !matches!(value_ref(object), Ok(value) if is_object_value(value)) {
        return record_status(env, NAPI_OBJECT_EXPECTED);
    }
    let exists = own_property_value(env, object, &name).is_some();
    if frozen || (inherited_owner.is_some() && !writable) || (nonextensible && !exists) {
        return record_status(env, NAPI_GENERIC_FAILURE);
    }
    let status = if is_array_length_property(object, &name) {
        // ArraySetLength can delete high indices before a lower readonly
        // index stops the operation with a failure status.
        scope_sweep.changed();
        set_array_length(env, object, value, false)
    } else {
        match env_mut(env) {
            Ok(env) => set_own_property(env, object, &name, value),
            Err(status) => status,
        }
    };
    if status == NAPI_OK && !exists {
        if let Ok(env) = env_mut(env) {
            record_property_order(env, object as usize, &name);
            env.property_attributes
                .insert((object as usize, name), NAPI_DEFAULT_PROPERTY_ATTRIBUTES);
        }
    }
    if status == NAPI_OK { scope_sweep.changed(); }
    record_status(env, status)
}

#[no_mangle]
pub unsafe extern "C" fn napi_get_named_property(
    env: NapiEnv,
    object: NapiValue,
    name: *const c_char,
    out: *mut NapiValue,
) -> NapiStatus {
    if out.is_null() || !value_belongs_to_environment(env, object) {
        return record_status(env, NAPI_INVALID_ARG);
    }
    let Ok(name) = text(name) else {
        return record_status(env, NAPI_INVALID_ARG);
    };
    let name = PropertyKey::String(name);
    #[cfg(feature = "quickjs")]
    if let Some(handle) = qjs_handle(object) {
        return qjs_get_property(env, handle, &name, out);
    }
    let value = find_property_value(env, object, &name);
    if value.is_none() {
        let accessor = find_accessor(env, object, &name);
        if let Some(getter) = accessor.as_ref().and_then(|accessor| accessor.getter) {
            let mut info = CallbackInfo {
                args: Vec::new(),
                this_arg: object,
                new_target: ptr::null_mut(),
                data: accessor.as_ref().unwrap().getter_data,
            };
            let value = invoke_napi_callback(env, getter, &mut info);
            if env_mut(env)
                .map(|env| env.exception.is_some())
                .unwrap_or(false)
            {
                return record_status(env, NAPI_PENDING_EXCEPTION);
            }
            let status = write_callback_value(env, out, value);
            return record_status(env, status);
        }
    }
    let status = match value {
        Some(value) => write_scoped_value(env, out, value),
        None => napi_get_undefined(env, out),
    };
    record_status(env, status)
}

unsafe fn property_key(env: NapiEnv, handle: NapiValue) -> Result<PropertyKey, NapiStatus> {
    match value_ref(handle) {
        Ok(Value::String(value)) => Ok(env.as_ref()
            .and_then(|owner| owner.utf16_strings.get(&(handle as usize)))
            .map(|units| PropertyKey::Utf16(units.clone()))
            .unwrap_or_else(|| PropertyKey::String(value.clone()))),
        Ok(Value::Symbol { id, .. }) => Ok(PropertyKey::Symbol(*id)),
        #[cfg(feature = "quickjs")]
        Ok(Value::QuickJsHandle { handle: source, .. }) => {
            // The engine owns the exact UTF-16 units and Symbol identity.
            // Probe before allocating native metadata, with the owner pinned
            // and without an Env borrow across the engine call.
            let source = *source;
            let _dispatch = ForeignCallbackGuard::new();
            let symbol = match qjs_is_symbol(source) {
                Ok(symbol) => symbol,
                Err(error) => return Err(qjs_error_status(env, error)),
            };
            if !symbol {
                match qjs_query(source, 0) {
                Ok(kind) if kind == "string" => {
                    let units = qjs_actual_string_units(env, source)?;
                    return Ok(match String::from_utf16(&units) {
                        Ok(text) => PropertyKey::String(text),
                        Err(_) => PropertyKey::Utf16(units),
                    });
                }
                Ok(_) => return Err(record_status(env, NAPI_STRING_EXPECTED)),
                Err(error) => return Err(qjs_error_status(env, error)),
                }
            }
            // A property key owns the Symbol through the native object graph.
            // Register its exact JS identity in the per-owner weak cache; do
            // not add an independent strong QuickJS registry retain merely
            // because the key was materialized through a live carrier.
            let owner_id = env.as_ref().map(|owner| owner.graph_owner_id)
                .ok_or(NAPI_INVALID_ARG)?;
            if owner_id == 0 {
                // A non-bridge test Env has no owner-specific JS weak cache.
                let needs_retain = env.as_ref().is_some_and(|owner|
                    !owner.quickjs_live_values.contains_key(&source));
                // A key may arrive from another live addon Env. Own a separate
                // QuickJS registry retain before storing it in this Env's map.
                if needs_retain && thaw_quickjs::thaw_js_retain_handle(source) == 0 {
                    return Err(record_status(env, NAPI_INVALID_ARG));
                }
                let (id, duplicate_retain) = {
                    let Ok(owner) = env_mut(env) else {
                        if needs_retain { thaw_quickjs::thaw_js_release_handle(source); }
                        return Err(record_status(env, NAPI_INVALID_ARG));
                    };
                    let duplicate_retain = owner.quickjs_live_values.contains_key(&source) && needs_retain;
                    let owned = if let Some(owned) = owner.quickjs_live_values.get(&source).copied() {
                        owned
                    } else {
                        let owned = owner.alloc(Value::QuickJsHandle { handle: source, object_like: false });
                        owner.quickjs_live_values.insert(source, owned);
                        owned
                    };
                    let id = *owner.quickjs_symbol_ids.entry(source).or_insert_with(|| {
                        NEXT_SYMBOL_ID.fetch_add(1, Ordering::Relaxed)
                    });
                    owner.symbols.insert(id, owned);
                    (id, duplicate_retain)
                };
                if duplicate_retain { thaw_quickjs::thaw_js_release_handle(source); }
                return Ok(PropertyKey::Symbol(id));
            }
            let id = thaw_quickjs::thaw_js_napi_graph_symbol_register(owner_id, source);
            if id == 0 { return Err(record_status(env, NAPI_GENERIC_FAILURE)); }
            Ok(PropertyKey::Symbol(id))
        }
        Ok(Value::Number(value)) => Ok(PropertyKey::String(if *value == 0.0 {
            "0".into()
        } else {
            value.to_string()
        })),
        _ => Err(NAPI_STRING_EXPECTED),
    }
}

unsafe fn set_property_key(
    env: NapiEnv,
    object: NapiValue,
    key: PropertyKey,
    value: NapiValue,
    receiver: NapiValue,
) -> NapiStatus {
    #[cfg(feature = "quickjs")]
    if let Some(handle) = qjs_handle(object) {
        // This target already has its own JS [[Set]] implementation. The
        // writable-data receiver shortcut below is only for native targets.
        return qjs_set_property(env, handle, &key, value, false);
    }
    if receiver == object {
        if let PropertyKey::String(name) = &key {
            if let Ok(name) = CString::new(name.as_str()) {
                return napi_set_named_property(env, object, name.as_ptr(), value);
            }
            // A property-key value may contain U+0000 even though the named
            // C API cannot express it. Use the generic key path then.
        }
    }
    let _dispatch = ForeignCallbackGuard::new();
    let mut scope_sweep = ScopeMutationSweep::new(env);
    if let Some(accessor) = find_accessor(env, object, &key) {
        let Some(setter) = accessor.setter else {
            return record_status(env, NAPI_GENERIC_FAILURE);
        };
        let mut info = CallbackInfo {
            args: vec![value],
            this_arg: receiver,
            new_target: ptr::null_mut(),
            data: accessor.setter_data,
        };
        invoke_napi_callback(env, setter, &mut info);
        let status = if env_mut(env)
            .map(|env| env.exception.is_some())
            .unwrap_or(false)
        {
            NAPI_PENDING_EXCEPTION
        } else {
            NAPI_OK
        };
        return record_status(env, status);
    }
    if !matches!(value_ref(object), Ok(value) if is_object_value(value)) {
        return record_status(env, NAPI_OBJECT_EXPECTED);
    }
    let exists = own_property_value(env, object, &key).is_some();
    let property_owner = find_data_property_owner(env, object, &key);
    let writable = property_owner
        .map(|owner| property_attributes_for(env, owner, &key) & NAPI_WRITABLE != 0)
        .unwrap_or(true);
    if receiver != object {
        // [[Set]] writes a writable data property on its receiver. The
        // target's extensibility does not decide whether that receiver can
        // acquire an own property.
        if property_owner.is_some() && !writable {
            return record_status(env, NAPI_GENERIC_FAILURE);
        }
        #[cfg(feature = "quickjs")]
        if let Some(handle) = qjs_handle(receiver) {
            return qjs_set_property(env, handle, &key, value, true);
        }
        if !matches!(value_ref(receiver), Ok(value) if is_object_value(value))
            || accessor_for_owner(env, receiver as usize, &key).is_some()
        {
            return record_status(env, NAPI_GENERIC_FAILURE);
        }
        let receiver_has_own = own_property_value(env, receiver, &key).is_some();
        let Some(receiver_env) = env.as_ref() else {
            return record_status(env, NAPI_INVALID_ARG);
        };
        if receiver_env.frozen_objects.contains(&(receiver as usize))
            || (receiver_has_own
                && property_attributes_for(env, receiver as usize, &key) & NAPI_WRITABLE == 0)
            || (!receiver_has_own
                && receiver_env.nonextensible_objects.contains(&(receiver as usize)))
        {
            return record_status(env, NAPI_GENERIC_FAILURE);
        }
        let status = if is_array_length_property(receiver, &key) {
            scope_sweep.changed();
            set_array_length(env, receiver, value, false)
        } else {
            let Ok(receiver_env) = env_mut(env) else {
                return record_status(env, NAPI_INVALID_ARG);
            };
            set_own_property(receiver_env, receiver, &key, value)
        };
        if status != NAPI_OK { return record_status(env, status); }
        if !receiver_has_own {
            if let Ok(receiver_env) = env_mut(env) {
                record_property_order(receiver_env, receiver as usize, &key);
                receiver_env.property_attributes.insert(
                    (receiver as usize, key), NAPI_DEFAULT_PROPERTY_ATTRIBUTES);
            }
        }
        scope_sweep.changed();
        return NAPI_OK;
    }
    let Some(env_ref) = env.as_ref() else {
        return record_status(env, NAPI_INVALID_ARG);
    };
    if env_ref.frozen_objects.contains(&(object as usize))
        || (property_owner.is_some() && !writable)
        || (env_ref.nonextensible_objects.contains(&(object as usize)) && !exists)
    {
        return record_status(env, NAPI_GENERIC_FAILURE);
    }
    let status = if is_array_length_property(object, &key) {
        scope_sweep.changed();
        set_array_length(env, object, value, false)
    } else {
        let Ok(env_ref) = env_mut(env) else {
            return record_status(env, NAPI_INVALID_ARG);
        };
        set_own_property(env_ref, object, &key, value)
    };
    if status != NAPI_OK {
        return record_status(env, status);
    }
    if !exists {
        if let Ok(env) = env_mut(env) {
            record_property_order(env, object as usize, &key);
            env.property_attributes
                .insert((object as usize, key), NAPI_DEFAULT_PROPERTY_ATTRIBUTES);
        }
    }
    scope_sweep.changed();
    NAPI_OK
}

unsafe fn get_property_key(
    env: NapiEnv,
    object: NapiValue,
    key: &PropertyKey,
    out: *mut NapiValue,
    receiver: NapiValue,
) -> NapiStatus {
    #[cfg(feature = "quickjs")]
    if let Some(handle) = qjs_handle(object) {
        return qjs_get_property(env, handle, key, out);
    }
    // The named-string API historically returns undefined for non-objects;
    // preserve that path while accepting strings with embedded NUL here.
    if !matches!(key, PropertyKey::String(_))
        && !matches!(value_ref(object), Ok(value) if is_object_value(value)) {
        return record_status(env, NAPI_OBJECT_EXPECTED);
    }
    let value = find_property_value(env, object, key);
    if value.is_none() {
        if let Some(accessor) = find_accessor(env, object, key) {
            if let Some(getter) = accessor.getter {
                let mut info = CallbackInfo {
                    args: Vec::new(),
                    this_arg: receiver,
                    new_target: ptr::null_mut(),
                    data: accessor.getter_data,
                };
                let value = invoke_napi_callback(env, getter, &mut info);
                if env_mut(env)
                    .map(|env| env.exception.is_some())
                    .unwrap_or(false)
                {
                    return record_status(env, NAPI_PENDING_EXCEPTION);
                }
                let status = write_callback_value(env, out, value);
                return record_status(env, status);
            }
        }
    }
    let status = match value {
        Some(value) => write_scoped_value(env, out, value),
        None => napi_get_undefined(env, out),
    };
    record_status(env, status)
}

#[no_mangle]
pub unsafe extern "C" fn napi_set_property(
    env: NapiEnv,
    object: NapiValue,
    key: NapiValue,
    value: NapiValue,
) -> NapiStatus {
    if !value_belongs_to_environment(env, object)
        || !value_belongs_to_environment(env, key)
        || !value_belongs_to_environment(env, value)
    {
        return NAPI_INVALID_ARG;
    }
    #[cfg(feature = "quickjs")]
    if let (Some(handle), Some(key_handle)) = (qjs_handle(object), qjs_handle(key)) {
        return qjs_set_property_live_key(env, handle, key_handle, value, false);
    }
    let key = match property_key(env, key) {
        Ok(key) => key,
        Err(status) => return status,
    };
    let status = set_property_key(env, object, key, value, object);
    record_status(env, status)
}

#[no_mangle]
pub unsafe extern "C" fn napi_get_property(
    env: NapiEnv,
    object: NapiValue,
    key: NapiValue,
    out: *mut NapiValue,
) -> NapiStatus {
    if out.is_null()
        || !value_belongs_to_environment(env, object)
        || !value_belongs_to_environment(env, key)
    {
        return record_status(env, NAPI_INVALID_ARG);
    }
    #[cfg(feature = "quickjs")]
    if let (Some(handle), Some(key_handle)) = (qjs_handle(object), qjs_handle(key)) {
        return qjs_get_property_live_key(env, handle, key_handle, out);
    }
    let key = match property_key(env, key) {
        Ok(key) => key,
        Err(status) => return status,
    };
    let status = get_property_key(env, object, &key, out, object);
    record_status(env, status)
}

#[no_mangle]
pub unsafe extern "C" fn napi_has_property(
    env: NapiEnv,
    object: NapiValue,
    key: NapiValue,
    out: *mut bool,
) -> NapiStatus {
    if out.is_null()
        || !value_belongs_to_environment(env, object)
        || !value_belongs_to_environment(env, key)
    {
        return record_status(env, NAPI_INVALID_ARG);
    }
    #[cfg(feature = "quickjs")]
    if let (Some(handle), Some(key_handle)) = (qjs_handle(object), qjs_handle(key)) {
        return qjs_property_predicate_live_key(env, handle, key_handle, 0, out);
    }
    let key = match property_key(env, key) {
        Ok(key) => key,
        Err(status) => return status,
    };
    #[cfg(feature = "quickjs")]
    if let Some(handle) = qjs_handle(object) {
        return qjs_property_predicate(env, handle, &key, 0, out);
    }
    if !matches!(value_ref(object), Ok(value) if is_object_value(value)) {
        return record_status(env, NAPI_OBJECT_EXPECTED);
    }
    *out = find_property_value(env, object, &key).is_some()
        || find_accessor(env, object, &key).is_some();
    NAPI_OK
}

#[no_mangle]
pub unsafe extern "C" fn napi_has_own_property(
    env: NapiEnv,
    object: NapiValue,
    key: NapiValue,
    out: *mut bool,
) -> NapiStatus {
    if out.is_null()
        || !value_belongs_to_environment(env, object)
        || !value_belongs_to_environment(env, key)
    {
        return record_status(env, NAPI_INVALID_ARG);
    }
    #[cfg(feature = "quickjs")]
    if let (Some(handle), Some(key_handle)) = (qjs_handle(object), qjs_handle(key)) {
        return qjs_property_predicate_live_key(env, handle, key_handle, 1, out);
    }
    let key = match property_key(env, key) {
        Ok(key) => key,
        Err(status) => return status,
    };
    #[cfg(feature = "quickjs")]
    if let Some(handle) = qjs_handle(object) {
        return qjs_property_predicate(env, handle, &key, 1, out);
    }
    if !matches!(value_ref(object), Ok(value) if is_object_value(value)) {
        return record_status(env, NAPI_OBJECT_EXPECTED);
    }
    let own_value = own_property_value(env, object, &key).is_some();
    let own_accessor = accessor_for_owner(env, object as usize, &key).is_some();
    *out = own_value || own_accessor;
    NAPI_OK
}

#[no_mangle]
pub unsafe extern "C" fn napi_delete_property(
    env: NapiEnv,
    object: NapiValue,
    key: NapiValue,
    out: *mut bool,
) -> NapiStatus {
    let _dispatch = ForeignCallbackGuard::new();
    let mut scope_sweep = ScopeMutationSweep::new(env);
    if !value_belongs_to_environment(env, object) || !value_belongs_to_environment(env, key) {
        return NAPI_INVALID_ARG;
    }
    #[cfg(feature = "quickjs")]
    if let (Some(handle), Some(key_handle)) = (qjs_handle(object), qjs_handle(key)) {
        return qjs_property_predicate_live_key(env, handle, key_handle, 2, out);
    }
    let key = match property_key(env, key) {
        Ok(key) => key,
        Err(status) => return status,
    };
    #[cfg(feature = "quickjs")]
    if let Some(handle) = qjs_handle(object) {
        return qjs_property_predicate(env, handle, &key, 2, out);
    }
    if !matches!(value_ref(object), Ok(value) if is_object_value(value)) {
        return record_status(env, NAPI_OBJECT_EXPECTED);
    }
    let own = own_property_value(env, object, &key).is_some()
        || accessor_for_owner(env, object as usize, &key).is_some();
    if own
        && env
            .as_ref()
            .is_some_and(|env| env.sealed_objects.contains(&(object as usize)))
    {
        if let Some(out) = out.as_mut() {
            *out = false;
        }
        return NAPI_OK;
    }
    if fixed_index_exists(object, &key)
        || (own && property_attributes_for(env, object as usize, &key) & NAPI_CONFIGURABLE == 0)
    {
        if let Some(out) = out.as_mut() {
            *out = false;
        }
        return NAPI_OK;
    }
    let removed_accessor = if let Ok(owner) = env_mut(env) {
        let status = remove_own_property(owner, object, &key);
        if status != NAPI_OK {
            return record_status(env, status);
        }
        let removed = owner.accessors.remove(&(object as usize, key.clone()));
        remove_property_order(owner, object as usize, &key);
        owner.property_attributes.remove(&(object as usize, key));
        removed
    } else { None };
    // Releasing a JS accessor can run a finalizer and reenter N-API.
    drop(removed_accessor);
    if own { scope_sweep.changed(); }
    if let Some(out) = out.as_mut() {
        *out = true;
    }
    NAPI_OK
}

#[no_mangle]
pub unsafe extern "C" fn napi_has_named_property(
    env: NapiEnv,
    object: NapiValue,
    name: *const c_char,
    result: *mut bool,
) -> NapiStatus {
    if name.is_null() || result.is_null() || !value_belongs_to_environment(env, object) {
        return record_status(env, NAPI_INVALID_ARG);
    }
    let Ok(name) = text(name) else {
        return record_status(env, NAPI_INVALID_ARG);
    };
    let name = PropertyKey::String(name);
    #[cfg(feature = "quickjs")]
    if let Some(handle) = qjs_handle(object) {
        return qjs_property_predicate(env, handle, &name, 0, result);
    }
    if !matches!(value_ref(object), Ok(value) if is_object_value(value)) {
        return record_status(env, NAPI_OBJECT_EXPECTED);
    }
    *result = find_property_value(env, object, &name).is_some()
        || find_accessor(env, object, &name).is_some();
    NAPI_OK
}
