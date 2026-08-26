#[no_mangle]
pub unsafe extern "C" fn napi_get_property_names(
    env: NapiEnv,
    object: NapiValue,
    out: *mut NapiValue,
) -> NapiStatus {
    napi_get_all_property_names(
        env,
        object,
        NAPI_KEY_INCLUDE_PROTOTYPES,
        NAPI_ENUMERABLE | NAPI_KEY_SKIP_SYMBOLS,
        NAPI_KEY_NUMBERS_TO_STRINGS,
        out,
    )
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
enum EnumeratedPropertyKey {
    Number(usize),
    Property(PropertyKey),
}

#[no_mangle]
pub unsafe extern "C" fn napi_get_all_property_names(
    env: NapiEnv,
    object: NapiValue,
    key_mode: i32,
    key_filter: u32,
    key_conversion: i32,
    out: *mut NapiValue,
) -> NapiStatus {
    if out.is_null()
        || !value_belongs_to_environment(env, object)
        || !matches!(key_mode, NAPI_KEY_INCLUDE_PROTOTYPES | NAPI_KEY_OWN_ONLY)
        || !matches!(
            key_conversion,
            NAPI_KEY_KEEP_NUMBERS | NAPI_KEY_NUMBERS_TO_STRINGS
        )
    {
        return record_status(env, NAPI_INVALID_ARG);
    }
    let env_ptr = env;
    let Ok(env) = env_mut(env) else {
        return NAPI_INVALID_ARG;
    };
    if !matches!(value_ref(object), Ok(value) if is_object_value(value)) {
        return record_status(env_ptr, NAPI_OBJECT_EXPECTED);
    }
    let mut keys = Vec::new();
    let mut current = Some(object as usize);
    let mut visited = HashSet::new();
    while let Some(owner) = current.filter(|owner| visited.insert(*owner)) {
        let value = owner as NapiValue;
        let mut property_keys = match value_ref(value) {
            Ok(Value::Object(properties)) => properties.keys().cloned().collect::<Vec<_>>(),
            Ok(Value::Function(function)) => {
                function.properties.keys().cloned().collect::<Vec<_>>()
            }
            Ok(Value::Array(values)) => {
                keys.extend(
                    values
                        .iter()
                        .enumerate()
                        .filter(|(_, value)| value.is_some())
                        .map(|(index, _)| (EnumeratedPropertyKey::Number(index), owner)),
                );
                vec![PropertyKey::String("length".into())]
            }
            Ok(Value::Buffer(bytes)) => {
                keys.extend(
                    (0..bytes.len()).map(|index| (EnumeratedPropertyKey::Number(index), owner)),
                );
                Vec::new()
            }
            Ok(Value::ExternalBuffer { data, length }) => {
                if !data.is_null() {
                    keys.extend(
                        (0..*length).map(|index| (EnumeratedPropertyKey::Number(index), owner)),
                    );
                }
                Vec::new()
            }
            Ok(Value::BufferView {
                array_buffer,
                length,
                ..
            }) => {
                if arraybuffer_parts(*array_buffer).is_ok_and(|(_, _, detached)| !detached) {
                    keys.extend(
                        (0..*length).map(|index| (EnumeratedPropertyKey::Number(index), owner)),
                    );
                }
                Vec::new()
            }
            Ok(Value::TypedArray {
                array_buffer,
                length,
                ..
            }) => {
                if arraybuffer_parts(*array_buffer).is_ok_and(|(_, _, detached)| !detached) {
                    keys.extend(
                        (0..*length).map(|index| (EnumeratedPropertyKey::Number(index), owner)),
                    );
                }
                Vec::new()
            }
            _ => Vec::new(),
        };
        property_keys.extend(host_property_keys_for_owner(env_ptr, owner));
        property_keys.extend(accessors_for_owner(env_ptr, owner));
        property_keys.sort_by(|left, right| {
            let category = |key: &PropertyKey| match (property_array_index(key), key) {
                (Some(index), _) => (0, index, usize::MAX),
                (None, PropertyKey::String(_)) => {
                    (1, 0, property_order_for_owner(env_ptr, owner, key))
                }
                (None, PropertyKey::Symbol(_)) => {
                    (2, 0, property_order_for_owner(env_ptr, owner, key))
                }
            };
            category(left)
                .cmp(&category(right))
                .then_with(|| left.cmp(right))
        });
        property_keys.dedup();
        keys.extend(property_keys.into_iter().map(|key| {
            let key = property_array_index(&key)
                .map(EnumeratedPropertyKey::Number)
                .unwrap_or(EnumeratedPropertyKey::Property(key));
            (key, owner)
        }));
        current = if key_mode == NAPI_KEY_INCLUDE_PROTOTYPES {
            prototype_for_owner(env_ptr, owner)
        } else {
            None
        };
    }
    let attribute_filter = key_filter & NAPI_DEFAULT_PROPERTY_ATTRIBUTES;
    keys.retain(|(key, owner)| match key {
        EnumeratedPropertyKey::Number(index) => {
            let key = PropertyKey::String(index.to_string());
            key_filter & NAPI_KEY_SKIP_STRINGS == 0
                && (attribute_filter == NAPI_KEY_ALL_PROPERTIES
                    || property_attributes_for(env_ptr, *owner, &key) & attribute_filter
                        == attribute_filter)
        }
        EnumeratedPropertyKey::Property(PropertyKey::String(_)) => {
            key_filter & NAPI_KEY_SKIP_STRINGS == 0
                && (attribute_filter == NAPI_KEY_ALL_PROPERTIES
                    || property_attributes_for(
                        env_ptr,
                        *owner,
                        match key {
                            EnumeratedPropertyKey::Property(key) => key,
                            _ => unreachable!(),
                        },
                    ) & attribute_filter
                        == attribute_filter)
        }
        EnumeratedPropertyKey::Property(PropertyKey::Symbol(_)) => {
            key_filter & NAPI_KEY_SKIP_SYMBOLS == 0
                && (attribute_filter == NAPI_KEY_ALL_PROPERTIES
                    || property_attributes_for(
                        env_ptr,
                        *owner,
                        match key {
                            EnumeratedPropertyKey::Property(key) => key,
                            _ => unreachable!(),
                        },
                    ) & attribute_filter
                        == attribute_filter)
        }
    });
    let mut seen = HashSet::new();
    keys.retain(|(key, _)| seen.insert(key.clone()));
    let mut values = Vec::with_capacity(keys.len());
    for (key, _) in keys {
        let value = match key {
            EnumeratedPropertyKey::Number(index) if key_conversion == NAPI_KEY_KEEP_NUMBERS => {
                env.alloc(Value::Number(index as f64))
            }
            EnumeratedPropertyKey::Number(index) => env.alloc(Value::String(index.to_string())),
            EnumeratedPropertyKey::Property(PropertyKey::String(name)) => {
                env.alloc(Value::String(name))
            }
            EnumeratedPropertyKey::Property(PropertyKey::Symbol(id)) => {
                let Some(value) = symbol_for(env_ptr, id) else {
                    return NAPI_GENERIC_FAILURE;
                };
                value
            }
        };
        values.push(value);
    }
    let result = env.alloc(Value::Array(values.into_iter().map(Some).collect()));
    write_value(out, result)
}

#[no_mangle]
pub unsafe extern "C" fn napi_object_seal(env: NapiEnv, object: NapiValue) -> NapiStatus {
    if !value_belongs_to_environment(env, object) {
        return NAPI_INVALID_ARG;
    }
    if !matches!(value_ref(object), Ok(value) if is_object_value(value)) {
        return record_status(env, NAPI_OBJECT_EXPECTED);
    }
    let Ok(env) = env_mut(env) else {
        return NAPI_INVALID_ARG;
    };
    let mut names = match value_ref(object) {
        Ok(Value::Object(properties)) => properties.keys().cloned().collect::<Vec<_>>(),
        Ok(Value::Function(function)) => function.properties.keys().cloned().collect(),
        Ok(Value::Array(values)) => values
            .iter()
            .enumerate()
            .filter(|(_, value)| value.is_some())
            .map(|(index, _)| PropertyKey::String(index.to_string()))
            .collect(),
        _ => Vec::new(),
    };
    names.extend(
        env.host_properties
            .get(&(object as usize))
            .into_iter()
            .flat_map(|properties| properties.keys().cloned()),
    );
    names.extend(
        env.accessors
            .keys()
            .filter(|(owner, _)| *owner == object as usize)
            .map(|(_, name)| name.clone()),
    );
    names.sort();
    names.dedup();
    for name in names {
        *env.property_attributes
            .entry((object as usize, name))
            .or_insert(NAPI_DEFAULT_PROPERTY_ATTRIBUTES) &= !NAPI_CONFIGURABLE;
    }
    env.sealed_objects.insert(object as usize);
    NAPI_OK
}

#[no_mangle]
pub unsafe extern "C" fn napi_object_freeze(env: NapiEnv, object: NapiValue) -> NapiStatus {
    if !value_belongs_to_environment(env, object) {
        return NAPI_INVALID_ARG;
    }
    if !matches!(value_ref(object), Ok(value) if is_object_value(value)) {
        return record_status(env, NAPI_OBJECT_EXPECTED);
    }
    let status = napi_object_seal(env, object);
    if status != NAPI_OK {
        return record_status(env, status);
    }
    let Ok(env) = env_mut(env) else {
        return NAPI_INVALID_ARG;
    };
    for ((owner, _), attributes) in &mut env.property_attributes {
        if *owner == object as usize {
            *attributes &= !NAPI_WRITABLE;
        }
    }
    env.frozen_objects.insert(object as usize);
    NAPI_OK
}

#[no_mangle]
pub unsafe extern "C" fn napi_get_uv_event_loop(env: NapiEnv, out: *mut *mut c_void) -> NapiStatus {
    if env.is_null() || out.is_null() {
        return record_status(env, NAPI_INVALID_ARG);
    }
    #[cfg(target_os = "linux")]
    {
        let symbol = libc::dlsym(libc::RTLD_DEFAULT, c"uv_default_loop".as_ptr());
        if symbol.is_null() {
            return record_status(env, NAPI_GENERIC_FAILURE);
        }
        let default_loop =
            std::mem::transmute::<*mut c_void, unsafe extern "C" fn() -> *mut c_void>(symbol);
        *out = default_loop();
        if (*out).is_null() {
            return record_status(env, NAPI_GENERIC_FAILURE);
        }
        NAPI_OK
    }
    #[cfg(not(target_os = "linux"))]
    {
        *out = ptr::null_mut();
        NAPI_GENERIC_FAILURE
    }
}

/// Thaw host extension for addons or generated native shims which own a
/// non-default libuv loop. Registered loops are driven on the generated
/// program's main thread alongside Node-API async completions.
#[no_mangle]
pub unsafe extern "C" fn thaw_napi_register_uv_loop(event_loop: *mut c_void) -> NapiStatus {
    if event_loop.is_null() {
        return NAPI_INVALID_ARG;
    }
    let mut loops = registered_uv_loops()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let address = event_loop as usize;
    if !loops.contains(&address) {
        loops.push(address);
    }
    NAPI_OK
}

/// Stops host polling for a private libuv loop before its owner closes and
/// frees the `uv_loop_t` storage.
#[no_mangle]
pub unsafe extern "C" fn thaw_napi_unregister_uv_loop(event_loop: *mut c_void) -> NapiStatus {
    if event_loop.is_null() {
        return NAPI_INVALID_ARG;
    }
    let mut loops = registered_uv_loops()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let address = event_loop as usize;
    let Some(index) = loops.iter().position(|registered| *registered == address) else {
        return NAPI_INVALID_ARG;
    };
    loops.swap_remove(index);
    NAPI_OK
}

#[no_mangle]
pub unsafe extern "C" fn napi_typeof(env: NapiEnv, value: NapiValue, out: *mut i32) -> NapiStatus {
    if out.is_null() || !value_belongs_to_environment(env, value) {
        return record_status(env, NAPI_INVALID_ARG);
    }
    *out = match value_ref(value) {
        Ok(Value::Undefined) => 0,
        Ok(Value::Null) => 1,
        Ok(Value::Bool(_)) => 2,
        Ok(Value::Number(_)) => 3,
        Ok(Value::String(_)) => 4,
        Ok(Value::Symbol { .. }) => 5,
        Ok(Value::Function(_)) => 7,
        Ok(Value::External(_)) => 8,
        Ok(Value::BigInt { .. }) => 9,
        Ok(_) => 6,
        Err(_) => return record_status(env, NAPI_INVALID_ARG),
    };
    NAPI_OK
}

#[no_mangle]
pub unsafe extern "C" fn napi_is_array(
    env: NapiEnv,
    value: NapiValue,
    out: *mut bool,
) -> NapiStatus {
    if out.is_null() || !value_belongs_to_environment(env, value) {
        return record_status(env, NAPI_INVALID_ARG);
    }
    *out = matches!(value_ref(value), Ok(Value::Array(_)));
    NAPI_OK
}

#[no_mangle]
pub unsafe extern "C" fn napi_is_promise(
    env: NapiEnv,
    value: NapiValue,
    result: *mut bool,
) -> NapiStatus {
    if !value_belongs_to_environment(env, value) {
        return NAPI_INVALID_ARG;
    }
    let Some(result) = result.as_mut() else {
        return record_status(env, NAPI_INVALID_ARG);
    };
    *result = matches!(value_ref(value), Ok(Value::Promise(_)));
    NAPI_OK
}

#[no_mangle]
pub unsafe extern "C" fn napi_get_array_length(
    env: NapiEnv,
    value: NapiValue,
    out: *mut u32,
) -> NapiStatus {
    if out.is_null() || !value_belongs_to_environment(env, value) {
        return record_status(env, NAPI_INVALID_ARG);
    }
    let status = match value_ref(value) {
        Ok(Value::Array(values)) => {
            *out = values.len() as u32;
            NAPI_OK
        }
        _ => NAPI_ARRAY_EXPECTED,
    };
    record_status(env, status)
}

#[no_mangle]
pub unsafe extern "C" fn napi_set_element(
    env: NapiEnv,
    object: NapiValue,
    index: u32,
    value: NapiValue,
) -> NapiStatus {
    if !value_belongs_to_environment(env, object) || !value_belongs_to_environment(env, value) {
        return NAPI_INVALID_ARG;
    }
    let status = set_property_key(env, object, PropertyKey::String(index.to_string()), value);
    record_status(env, status)
}

#[no_mangle]
pub unsafe extern "C" fn napi_has_element(
    env: NapiEnv,
    object: NapiValue,
    index: u32,
    result: *mut bool,
) -> NapiStatus {
    if !value_belongs_to_environment(env, object) {
        return NAPI_INVALID_ARG;
    }
    let Some(result) = result.as_mut() else {
        return record_status(env, NAPI_INVALID_ARG);
    };
    let key = PropertyKey::String(index.to_string());
    if !matches!(value_ref(object), Ok(value) if is_object_value(value)) {
        return record_status(env, NAPI_OBJECT_EXPECTED);
    }
    *result = find_property_value(env, object, &key).is_some()
        || find_accessor(env, object, &key).is_some();
    NAPI_OK
}

#[no_mangle]
pub unsafe extern "C" fn napi_delete_element(
    env: NapiEnv,
    object: NapiValue,
    index: u32,
    result: *mut bool,
) -> NapiStatus {
    if !value_belongs_to_environment(env, object) {
        return NAPI_INVALID_ARG;
    }
    let key = PropertyKey::String(index.to_string());
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
        if let Some(result) = result.as_mut() {
            *result = false;
        }
        return NAPI_OK;
    }
    if fixed_index_exists(object, &key)
        || (own && property_attributes_for(env, object as usize, &key) & NAPI_CONFIGURABLE == 0)
    {
        if let Some(result) = result.as_mut() {
            *result = false;
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
    if let Some(result) = result.as_mut() {
        *result = true;
    }
    NAPI_OK
}

#[no_mangle]
pub unsafe extern "C" fn napi_get_element(
    env: NapiEnv,
    object: NapiValue,
    index: u32,
    out: *mut NapiValue,
) -> NapiStatus {
    if out.is_null() || !value_belongs_to_environment(env, object) {
        return record_status(env, NAPI_INVALID_ARG);
    }
    let key = PropertyKey::String(index.to_string());
    if !matches!(value_ref(object), Ok(value) if is_object_value(value)) {
        return record_status(env, NAPI_OBJECT_EXPECTED);
    }
    let value = find_property_value(env, object, &key);
    if value.is_none() {
        if let Some(accessor) = find_accessor(env, object, &key) {
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

