#[no_mangle]
pub unsafe extern "C" fn napi_define_properties(
    env: NapiEnv,
    object: NapiValue,
    count: usize,
    descriptors: *const NapiPropertyDescriptor,
) -> NapiStatus {
    if !value_belongs_to_environment(env, object) {
        return NAPI_INVALID_ARG;
    }
    if !matches!(value_ref(object), Ok(value) if is_object_value(value)) {
        return record_status(env, NAPI_OBJECT_EXPECTED);
    }
    let status = validate_property_descriptors(env, count, descriptors);
    if status != NAPI_OK {
        return record_status(env, status);
    }
    if count == 0 {
        return NAPI_OK;
    }
    for descriptor in std::slice::from_raw_parts(descriptors, count) {
        let key = if !descriptor.utf8name.is_null() {
            env_mut(env).map(|env| {
                env.alloc(Value::String(
                    CStr::from_ptr(descriptor.utf8name)
                        .to_string_lossy()
                        .into_owned(),
                ))
            })
        } else {
            Ok(descriptor.name)
        };
        let Ok(key) = key else {
            return record_status(env, NAPI_INVALID_ARG);
        };
        let Ok(key_name) = property_key(key) else {
            return record_status(env, NAPI_INVALID_ARG);
        };
        let attributes = descriptor.attributes & NAPI_DEFAULT_PROPERTY_ATTRIBUTES;
        if descriptor.getter.is_some() || descriptor.setter.is_some() {
            let Ok(env) = env_mut(env) else {
                return NAPI_INVALID_ARG;
            };
            if env.sealed_objects.contains(&(object as usize)) {
                return record_status(env, NAPI_GENERIC_FAILURE);
            }
            env.accessors.insert(
                (object as usize, key_name.clone()),
                Accessor {
                    getter: descriptor.getter,
                    setter: descriptor.setter,
                    data: descriptor.data,
                },
            );
            record_property_order(env, object as usize, &key_name);
            env.property_attributes
                .insert((object as usize, key_name), attributes);
            continue;
        }
        let value = if let Some(method) = descriptor.method {
            let mut value = ptr::null_mut();
            let status = napi_create_function(
                env,
                descriptor.utf8name,
                NAPI_AUTO_LENGTH,
                Some(method),
                descriptor.data,
                &mut value,
            );
            if status != NAPI_OK {
                return record_status(env, status);
            }
            value
        } else {
            descriptor.value
        };
        let status = napi_set_property(env, object, key, value);
        if status != NAPI_OK {
            return record_status(env, status);
        }
        if let Ok(env) = env_mut(env) {
            env.property_attributes
                .insert((object as usize, key_name), attributes);
        }
    }
    NAPI_OK
}

#[no_mangle]
pub unsafe extern "C" fn napi_type_tag_object(
    env: NapiEnv,
    object: NapiValue,
    type_tag: *const NapiTypeTag,
) -> NapiStatus {
    let Some(type_tag) = type_tag.as_ref().copied() else {
        return record_status(env, NAPI_INVALID_ARG);
    };
    if !value_belongs_to_environment(env, object) {
        return NAPI_INVALID_ARG;
    }
    if !matches!(value_ref(object), Ok(value) if is_object_value(value)) {
        return record_status(env, NAPI_OBJECT_EXPECTED);
    }
    if type_tag_for(env, object).is_some() {
        return record_status(env, NAPI_INVALID_ARG);
    }
    let Ok(env) = env_mut(env) else {
        return NAPI_INVALID_ARG;
    };
    env.type_tags.insert(object as usize, type_tag);
    NAPI_OK
}

#[no_mangle]
pub unsafe extern "C" fn napi_check_object_type_tag(
    env: NapiEnv,
    object: NapiValue,
    type_tag: *const NapiTypeTag,
    result: *mut bool,
) -> NapiStatus {
    let (Some(type_tag), Some(result)) = (type_tag.as_ref(), result.as_mut()) else {
        return record_status(env, NAPI_INVALID_ARG);
    };
    if !value_belongs_to_environment(env, object) {
        return NAPI_INVALID_ARG;
    }
    if !matches!(value_ref(object), Ok(value) if is_object_value(value)) {
        return record_status(env, NAPI_OBJECT_EXPECTED);
    }
    if env.is_null() {
        return NAPI_INVALID_ARG;
    }
    *result = type_tag_for(env, object).as_ref() == Some(type_tag);
    NAPI_OK
}

#[no_mangle]
pub unsafe extern "C" fn napi_wrap(
    env: NapiEnv,
    object: NapiValue,
    data: *mut c_void,
    finalize: Option<NapiFinalize>,
    hint: *mut c_void,
    result: *mut *mut Reference,
) -> NapiStatus {
    if !value_belongs_to_environment(env, object) {
        return NAPI_INVALID_ARG;
    }
    let env_ptr = env;
    let Ok(env) = env_mut(env) else {
        return NAPI_INVALID_ARG;
    };
    if !matches!(object.as_ref(), Some(value) if is_object_value(value)) {
        return record_status(env_ptr, NAPI_OBJECT_EXPECTED);
    }
    if env.wraps.contains_key(&(object as usize)) {
        return record_status(env_ptr, NAPI_GENERIC_FAILURE);
    }
    env.wraps.insert(
        object as usize,
        WrapRecord {
            data,
            finalize,
            hint,
        },
    );
    if !result.is_null() {
        *result = alloc_reference(env, object, 0);
    }
    NAPI_OK
}

#[no_mangle]
pub unsafe extern "C" fn napi_unwrap(
    env: NapiEnv,
    object: NapiValue,
    result: *mut *mut c_void,
) -> NapiStatus {
    if result.is_null() || !value_belongs_to_environment(env, object) {
        return record_status(env, NAPI_INVALID_ARG);
    }
    let env_ptr = env;
    let Ok(env) = env_mut(env) else {
        return NAPI_INVALID_ARG;
    };
    if !matches!(object.as_ref(), Some(value) if is_object_value(value)) {
        return record_status(env_ptr, NAPI_OBJECT_EXPECTED);
    }
    let Some(wrap) = env.wraps.get(&(object as usize)) else {
        return record_status(env_ptr, NAPI_INVALID_ARG);
    };
    *result = wrap.data;
    NAPI_OK
}

#[no_mangle]
pub unsafe extern "C" fn napi_remove_wrap(
    env: NapiEnv,
    object: NapiValue,
    result: *mut *mut c_void,
) -> NapiStatus {
    if !value_belongs_to_environment(env, object) {
        return NAPI_INVALID_ARG;
    }
    let env_ptr = env;
    let Ok(env) = env_mut(env) else {
        return NAPI_INVALID_ARG;
    };
    if !matches!(object.as_ref(), Some(value) if is_object_value(value)) {
        return record_status(env_ptr, NAPI_OBJECT_EXPECTED);
    }
    let Some(wrap) = env.wraps.remove(&(object as usize)) else {
        return record_status(env_ptr, NAPI_INVALID_ARG);
    };
    if !result.is_null() {
        *result = wrap.data;
    }
    NAPI_OK
}

#[no_mangle]
pub unsafe extern "C" fn napi_add_finalizer(
    env: NapiEnv,
    object: NapiValue,
    data: *mut c_void,
    finalize: Option<NapiFinalize>,
    hint: *mut c_void,
    result: *mut *mut Reference,
) -> NapiStatus {
    if !value_belongs_to_environment(env, object) {
        return NAPI_INVALID_ARG;
    }
    if !matches!(object.as_ref(), Some(value) if is_object_value(value)) {
        return record_status(env, NAPI_OBJECT_EXPECTED);
    }
    if finalize.is_none() {
        return record_status(env, NAPI_INVALID_ARG);
    }
    let Ok(env) = env_mut(env) else {
        return NAPI_INVALID_ARG;
    };
    env.object_finalizers
        .entry(object as usize)
        .or_default()
        .push(FinalizeRecord {
            data,
            finalize,
            hint,
        });
    if !result.is_null() {
        *result = alloc_reference(env, object, 0);
    }
    NAPI_OK
}

#[no_mangle]
pub unsafe extern "C" fn node_api_post_finalizer(
    env: NapiEnv,
    finalize: Option<NapiFinalize>,
    data: *mut c_void,
    hint: *mut c_void,
) -> NapiStatus {
    let env_ptr = env;
    let (Ok(env), Some(finalize)) = (env_mut(env), finalize) else {
        return record_status(env_ptr, NAPI_INVALID_ARG);
    };
    env.posted_finalizers.push(FinalizeRecord {
        data,
        finalize: Some(finalize),
        hint,
    });
    NAPI_OK
}

#[no_mangle]
pub unsafe extern "C" fn napi_set_instance_data(
    env: NapiEnv,
    data: *mut c_void,
    finalize: Option<NapiFinalize>,
    hint: *mut c_void,
) -> NapiStatus {
    let env_ptr = env;
    let Ok(env) = env_mut(env) else {
        return NAPI_INVALID_ARG;
    };
    if env.instance_data.is_some() {
        return record_status(env_ptr, NAPI_GENERIC_FAILURE);
    }
    env.instance_data = Some(FinalizeRecord {
        data,
        finalize,
        hint,
    });
    NAPI_OK
}

#[no_mangle]
pub unsafe extern "C" fn napi_get_instance_data(
    env: NapiEnv,
    result: *mut *mut c_void,
) -> NapiStatus {
    if result.is_null() {
        return record_status(env, NAPI_INVALID_ARG);
    }
    let Ok(env) = env_mut(env) else {
        return NAPI_INVALID_ARG;
    };
    *result = env
        .instance_data
        .as_ref()
        .map_or(ptr::null_mut(), |record| record.data);
    NAPI_OK
}

#[no_mangle]
pub unsafe extern "C" fn napi_add_env_cleanup_hook(
    env: NapiEnv,
    hook: Option<NapiCleanupHook>,
    data: *mut c_void,
) -> NapiStatus {
    let env_ptr = env;
    let (Ok(env), Some(hook)) = (env_mut(env), hook) else {
        return record_status(env_ptr, NAPI_INVALID_ARG);
    };
    if env
        .cleanup_hooks
        .iter()
        .any(|record| record.hook as usize == hook as usize && record.data == data)
    {
        return record_status(env_ptr, NAPI_INVALID_ARG);
    }
    env.cleanup_hooks.push(CleanupHookRecord { hook, data });
    NAPI_OK
}

#[no_mangle]
pub unsafe extern "C" fn napi_remove_env_cleanup_hook(
    env: NapiEnv,
    hook: Option<NapiCleanupHook>,
    data: *mut c_void,
) -> NapiStatus {
    let env_ptr = env;
    let (Ok(env), Some(hook)) = (env_mut(env), hook) else {
        return record_status(env_ptr, NAPI_INVALID_ARG);
    };
    let Some(index) = env
        .cleanup_hooks
        .iter()
        .position(|record| record.hook as usize == hook as usize && record.data == data)
    else {
        return record_status(env_ptr, NAPI_INVALID_ARG);
    };
    env.cleanup_hooks.remove(index);
    NAPI_OK
}

#[no_mangle]
pub unsafe extern "C" fn napi_add_async_cleanup_hook(
    env: NapiEnv,
    hook: Option<NapiAsyncCleanupHook>,
    data: *mut c_void,
    result: *mut *mut AsyncCleanupHookHandle,
) -> NapiStatus {
    let env_ptr = env;
    let (Ok(env_ref), Some(hook)) = (env_mut(env), hook) else {
        return record_status(env_ptr, NAPI_INVALID_ARG);
    };
    let mut handle = Box::new(AsyncCleanupHookHandle {
        env: env as usize,
        hook,
        data: data as usize,
        state: AtomicU8::new(0),
    });
    let handle_ptr = (&mut *handle) as *mut AsyncCleanupHookHandle;
    async_cleanup_handles()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .push(handle);
    env_ref.async_cleanup_hooks.push(handle_ptr);
    if let Some(result) = result.as_mut() {
        *result = handle_ptr;
    }
    NAPI_OK
}

#[no_mangle]
pub unsafe extern "C" fn napi_remove_async_cleanup_hook(
    handle: *mut AsyncCleanupHookHandle,
) -> NapiStatus {
    if handle.is_null() {
        return NAPI_INVALID_ARG;
    }
    let handles = async_cleanup_handles()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let Some(handle_ref) = handles
        .iter()
        .find(|candidate| std::ptr::eq(candidate.as_ref(), handle))
    else {
        return NAPI_INVALID_ARG;
    };
    match handle_ref.state.swap(2, Ordering::AcqRel) {
        0 => {
            if let Some(env) = (handle_ref.env as NapiEnv).as_mut() {
                env.async_cleanup_hooks
                    .retain(|candidate| *candidate != handle);
            }
        }
        1 => {
            ACTIVE_ASYNC_CLEANUP_HOOKS.fetch_sub(1, Ordering::AcqRel);
        }
        _ => {
            return record_status(handle_ref.env as NapiEnv, NAPI_INVALID_ARG);
        }
    }
    NAPI_OK
}

#[no_mangle]
pub unsafe extern "C" fn napi_define_class(
    env: NapiEnv,
    name: *const c_char,
    length: usize,
    constructor: Option<NapiCallback>,
    data: *mut c_void,
    property_count: usize,
    properties: *const NapiPropertyDescriptor,
    result: *mut NapiValue,
) -> NapiStatus {
    if env.is_null()
        || name.is_null()
        || constructor.is_none()
        || result.is_null()
        || (property_count != 0 && properties.is_null())
    {
        return record_status(env, NAPI_INVALID_ARG);
    }
    let descriptor_status = validate_property_descriptors(env, property_count, properties);
    if descriptor_status != NAPI_OK {
        return record_status(env, descriptor_status);
    }
    let status = napi_create_function(env, name, length, constructor, data, result);
    if status != NAPI_OK {
        return record_status(env, status);
    }
    let mut prototype = ptr::null_mut();
    let status = napi_create_object(env, &mut prototype);
    if status != NAPI_OK {
        return record_status(env, status);
    }
    let prototype_name = c"prototype";
    let status = napi_set_named_property(env, *result, prototype_name.as_ptr(), prototype);
    if status != NAPI_OK {
        return record_status(env, status);
    }
    let constructor_name = c"constructor";
    let status = napi_set_named_property(env, prototype, constructor_name.as_ptr(), *result);
    if status != NAPI_OK {
        return record_status(env, status);
    }
    let Ok(env_ref) = env_mut(env) else {
        return NAPI_INVALID_ARG;
    };
    env_ref.property_attributes.insert(
        ((*result) as usize, PropertyKey::String("prototype".into())),
        0,
    );
    env_ref.property_attributes.insert(
        (
            prototype as usize,
            PropertyKey::String("constructor".into()),
        ),
        NAPI_WRITABLE | NAPI_CONFIGURABLE,
    );
    if property_count != 0 {
        for descriptor in std::slice::from_raw_parts(properties, property_count) {
            const NAPI_STATIC: u32 = 1 << 10;
            let target = if descriptor.attributes & NAPI_STATIC != 0 {
                *result
            } else {
                prototype
            };
            let status = napi_define_properties(env, target, 1, descriptor);
            if status != NAPI_OK {
                return record_status(env, status);
            }
        }
    }
    NAPI_OK
}

#[no_mangle]
pub unsafe extern "C" fn napi_new_instance(
    env: NapiEnv,
    constructor: NapiValue,
    argc: usize,
    argv: *const NapiValue,
    result: *mut NapiValue,
) -> NapiStatus {
    if result.is_null() || (argc != 0 && argv.is_null()) {
        return record_status(env, NAPI_INVALID_ARG);
    }
    let Ok(host_env) = env_mut(env) else {
        return NAPI_INVALID_ARG;
    };
    if !value_belongs_to_environment(env, constructor)
        || (argc != 0
            && std::slice::from_raw_parts(argv, argc)
                .iter()
                .any(|value| !value_belongs_to_environment(env, *value)))
        || host_env.exception.is_some()
    {
        let status = if host_env.exception.is_some() {
            NAPI_PENDING_EXCEPTION
        } else {
            NAPI_INVALID_ARG
        };
        return record_status(env, status);
    }
    let function = match value_ref(constructor) {
        Ok(Value::Function(function)) => function.clone(),
        _ => return record_status(env, NAPI_FUNCTION_EXPECTED),
    };
    let prototype = function
        .properties
        .get(&PropertyKey::String("prototype".into()))
        .copied();
    let instance = host_env.alloc(Value::Object(HashMap::new()));
    host_env
        .instances
        .insert(instance as usize, constructor as usize);
    if let Some(prototype) = prototype {
        host_env
            .prototypes
            .insert(instance as usize, prototype as usize);
    }
    let args = if argc == 0 {
        Vec::new()
    } else {
        std::slice::from_raw_parts(argv, argc).to_vec()
    };
    let mut info = CallbackInfo {
        args,
        this_arg: instance,
        new_target: constructor,
        data: function.data,
    };
    let returned = (function.callback)(env, &mut info);
    if env_mut(env)
        .map(|env| env.exception.is_some())
        .unwrap_or(false)
    {
        return record_status(env, NAPI_PENDING_EXCEPTION);
    }
    if !returned.is_null() && !value_belongs_to_environment(env, returned) {
        return NAPI_INVALID_ARG;
    }
    *result = if matches!(returned.as_ref(), Some(value) if is_object_value(value)) {
        returned
    } else {
        instance
    };
    NAPI_OK
}

#[no_mangle]
pub unsafe extern "C" fn napi_instanceof(
    env: NapiEnv,
    object: NapiValue,
    constructor: NapiValue,
    result: *mut bool,
) -> NapiStatus {
    if env.is_null() || result.is_null() {
        return record_status(env, NAPI_INVALID_ARG);
    }
    if !value_belongs_to_environment(env, object) || !value_belongs_to_environment(env, constructor)
    {
        return NAPI_INVALID_ARG;
    }
    if !matches!(value_ref(constructor), Ok(Value::Function(_))) {
        return record_status(env, NAPI_FUNCTION_EXPECTED);
    }
    let constructor_name = find_property_value(
        env,
        constructor,
        &PropertyKey::String("name".into()),
    )
    .and_then(|name| match value_ref(name) {
        Ok(Value::String(name)) => Some(name.as_str()),
        _ => None,
    });
    if matches!(value_ref(object), Ok(Value::Date(_))) && constructor_name == Some("Date") {
        *result = true;
        return NAPI_OK;
    }
    if !matches!(value_ref(object), Ok(value) if is_object_value(value)) {
        *result = false;
        return NAPI_OK;
    }
    let prototype_key = PropertyKey::String("prototype".into());
    let Some(expected) = find_property_value(env, constructor, &prototype_key) else {
        return record_status(env, NAPI_GENERIC_FAILURE);
    };
    if !matches!(value_ref(expected), Ok(value) if is_object_value(value)) {
        return record_status(env, NAPI_GENERIC_FAILURE);
    }
    let mut current = prototype_for_owner(env, object as usize);
    let mut visited = HashSet::new();
    *result = false;
    while let Some(prototype) = current.filter(|prototype| visited.insert(*prototype)) {
        if prototype == expected as usize {
            *result = true;
            break;
        }
        current = prototype_for_owner(env, prototype);
    }
    NAPI_OK
}

#[no_mangle]
pub unsafe extern "C" fn napi_get_prototype(
    env: NapiEnv,
    object: NapiValue,
    result: *mut NapiValue,
) -> NapiStatus {
    if result.is_null() || !value_belongs_to_environment(env, object) {
        return record_status(env, NAPI_INVALID_ARG);
    }
    if !matches!(value_ref(object), Ok(value) if is_object_value(value)) {
        return record_status(env, NAPI_OBJECT_EXPECTED);
    }
    let env_ptr = env;
    let Ok(env) = env_mut(env) else {
        return record_status(env, NAPI_INVALID_ARG);
    };
    let prototype =
        prototype_for_owner(env_ptr, object as usize).map(|prototype| prototype as NapiValue);
    match prototype {
        Some(prototype) => write_value(result, prototype),
        None => {
            let undefined = env.alloc(Value::Undefined);
            write_value(result, undefined)
        }
    }
}

#[no_mangle]
pub unsafe extern "C" fn node_api_set_prototype(
    env: NapiEnv,
    object: NapiValue,
    prototype: NapiValue,
) -> NapiStatus {
    if !value_belongs_to_environment(env, object) || !value_belongs_to_environment(env, prototype) {
        return NAPI_INVALID_ARG;
    }
    if !matches!(value_ref(object), Ok(value) if is_object_value(value)) {
        return record_status(env, NAPI_OBJECT_EXPECTED);
    }
    if !matches!(value_ref(prototype), Ok(value) if is_object_value(value) || matches!(value, Value::Null))
    {
        return record_status(env, NAPI_OBJECT_EXPECTED);
    }

    let object_id = object as usize;
    let prototype_id = prototype as usize;
    let Ok(env) = env_mut(env) else {
        return NAPI_INVALID_ARG;
    };
    if env.prototypes.get(&object_id).copied() == Some(prototype_id) {
        return NAPI_OK;
    }
    if env.sealed_objects.contains(&object_id) {
        return record_status(env, NAPI_GENERIC_FAILURE);
    }

    if !matches!(value_ref(prototype), Ok(Value::Null)) {
        let mut ancestor = Some(prototype_id);
        let mut visited = HashSet::new();
        while let Some(current) = ancestor {
            if current == object_id {
                return record_status(env, NAPI_GENERIC_FAILURE);
            }
            if !visited.insert(current) {
                break;
            }
            ancestor = env.prototypes.get(&current).copied();
        }
    }
    env.prototypes.insert(object_id, prototype_id);
    NAPI_OK
}

#[no_mangle]
pub unsafe extern "C" fn napi_strict_equals(
    env: NapiEnv,
    left: NapiValue,
    right: NapiValue,
    result: *mut bool,
) -> NapiStatus {
    if result.is_null() {
        return record_status(env, NAPI_INVALID_ARG);
    }
    if !value_belongs_to_environment(env, left) || !value_belongs_to_environment(env, right) {
        return record_status(env, NAPI_INVALID_ARG);
    }
    *result = match (value_ref(left), value_ref(right)) {
        (Ok(Value::Undefined), Ok(Value::Undefined)) | (Ok(Value::Null), Ok(Value::Null)) => true,
        (Ok(Value::Bool(left)), Ok(Value::Bool(right))) => left == right,
        (Ok(Value::Number(left)), Ok(Value::Number(right))) => left == right,
        (Ok(Value::String(left)), Ok(Value::String(right))) => left == right,
        (Ok(Value::Symbol { id: left, .. }), Ok(Value::Symbol { id: right, .. })) => left == right,
        (
            Ok(Value::BigInt {
                negative: left_negative,
                words: left_words,
            }),
            Ok(Value::BigInt {
                negative: right_negative,
                words: right_words,
            }),
        ) => left_negative == right_negative && left_words == right_words,
        (Ok(_), Ok(_)) => left == right,
        _ => false,
    };
    NAPI_OK
}
