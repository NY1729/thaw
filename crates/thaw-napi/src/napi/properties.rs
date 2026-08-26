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
    if let Some(accessor) = find_accessor(env, object, &name) {
        let Some(setter) = accessor.setter else {
            return record_status(env, NAPI_GENERIC_FAILURE);
        };
        let mut info = CallbackInfo {
            args: vec![value],
            this_arg: object,
            new_target: ptr::null_mut(),
            data: accessor.data,
        };
        setter(env, &mut info);
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
    let (frozen, sealed) = env
        .as_ref()
        .map(|env| {
            (
                env.frozen_objects.contains(&(object as usize)),
                env.sealed_objects.contains(&(object as usize)),
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
    if frozen || (inherited_owner.is_some() && !writable) || (sealed && !exists) {
        return record_status(env, NAPI_GENERIC_FAILURE);
    }
    let status = match env_mut(env) {
        Ok(env) => set_own_property(env, object, &name, value),
        Err(status) => status,
    };
    if status == NAPI_OK && !exists {
        if let Ok(env) = env_mut(env) {
            record_property_order(env, object as usize, &name);
            env.property_attributes
                .insert((object as usize, name), NAPI_DEFAULT_PROPERTY_ATTRIBUTES);
        }
    }
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
    let value = find_property_value(env, object, &name);
    if value.is_none() {
        let accessor = find_accessor(env, object, &name);
        if let Some(getter) = accessor.and_then(|accessor| accessor.getter) {
            let mut info = CallbackInfo {
                args: Vec::new(),
                this_arg: object,
                new_target: ptr::null_mut(),
                data: accessor.unwrap().data,
            };
            let value = getter(env, &mut info);
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
        Some(value) => write_value(out, value),
        None => napi_get_undefined(env, out),
    };
    record_status(env, status)
}

unsafe fn property_key(value: NapiValue) -> Result<PropertyKey, NapiStatus> {
    match value_ref(value) {
        Ok(Value::String(value)) => Ok(PropertyKey::String(value.clone())),
        Ok(Value::Symbol { id, .. }) => Ok(PropertyKey::Symbol(*id)),
        _ => Err(NAPI_STRING_EXPECTED),
    }
}

unsafe fn set_property_key(
    env: NapiEnv,
    object: NapiValue,
    key: PropertyKey,
    value: NapiValue,
) -> NapiStatus {
    if let PropertyKey::String(name) = &key {
        let Ok(name) = CString::new(name.as_str()) else {
            return record_status(env, NAPI_INVALID_ARG);
        };
        return napi_set_named_property(env, object, name.as_ptr(), value);
    }
    if let Some(accessor) = find_accessor(env, object, &key) {
        let Some(setter) = accessor.setter else {
            return record_status(env, NAPI_GENERIC_FAILURE);
        };
        let mut info = CallbackInfo {
            args: vec![value],
            this_arg: object,
            new_target: ptr::null_mut(),
            data: accessor.data,
        };
        setter(env, &mut info);
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
    let Some(env_ref) = env.as_ref() else {
        return record_status(env, NAPI_INVALID_ARG);
    };
    let property_owner = find_data_property_owner(env, object, &key);
    let writable = property_owner
        .map(|owner| property_attributes_for(env, owner, &key) & NAPI_WRITABLE != 0)
        .unwrap_or(true);
    if env_ref.frozen_objects.contains(&(object as usize))
        || (property_owner.is_some() && !writable)
        || (env_ref.sealed_objects.contains(&(object as usize)) && !exists)
    {
        return record_status(env, NAPI_GENERIC_FAILURE);
    }
    let Ok(env_ref) = env_mut(env) else {
        return record_status(env, NAPI_INVALID_ARG);
    };
    let status = set_own_property(env_ref, object, &key, value);
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
    NAPI_OK
}

unsafe fn get_property_key(
    env: NapiEnv,
    object: NapiValue,
    key: &PropertyKey,
    out: *mut NapiValue,
) -> NapiStatus {
    if let PropertyKey::String(name) = key {
        let Ok(name) = CString::new(name.as_str()) else {
            return record_status(env, NAPI_INVALID_ARG);
        };
        return napi_get_named_property(env, object, name.as_ptr(), out);
    }
    if !matches!(value_ref(object), Ok(value) if is_object_value(value)) {
        return record_status(env, NAPI_OBJECT_EXPECTED);
    }
    let value = find_property_value(env, object, key);
    if value.is_none() {
        if let Some(accessor) = find_accessor(env, object, key) {
            if let Some(getter) = accessor.getter {
                let mut info = CallbackInfo {
                    args: Vec::new(),
                    this_arg: object,
                    new_target: ptr::null_mut(),
                    data: accessor.data,
                };
                let value = getter(env, &mut info);
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
        Some(value) => write_value(out, value),
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
    let Ok(key) = property_key(key) else {
        return record_status(env, NAPI_INVALID_ARG);
    };
    let status = set_property_key(env, object, key, value);
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
    let Ok(key) = property_key(key) else {
        return record_status(env, NAPI_INVALID_ARG);
    };
    let status = get_property_key(env, object, &key, out);
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
    let Ok(key) = property_key(key) else {
        return record_status(env, NAPI_INVALID_ARG);
    };
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
    let Ok(key) = property_key(key) else {
        return record_status(env, NAPI_INVALID_ARG);
    };
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
    if !value_belongs_to_environment(env, object) || !value_belongs_to_environment(env, key) {
        return NAPI_INVALID_ARG;
    }
    let Ok(key) = property_key(key) else {
        return record_status(env, NAPI_INVALID_ARG);
    };
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
    if let Ok(env) = env_mut(env) {
        let status = remove_own_property(env, object, &key);
        if status != NAPI_OK {
            return record_status(env, status);
        }
        env.accessors.remove(&(object as usize, key.clone()));
        remove_property_order(env, object as usize, &key);
        env.property_attributes.remove(&(object as usize, key));
    }
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
    if !matches!(value_ref(object), Ok(value) if is_object_value(value)) {
        return record_status(env, NAPI_OBJECT_EXPECTED);
    }
    *result = find_property_value(env, object, &name).is_some()
        || find_accessor(env, object, &name).is_some();
    NAPI_OK
}

