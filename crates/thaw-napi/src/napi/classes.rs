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
    let _dispatch = ForeignCallbackGuard::new();
    // A later descriptor can fail after an earlier member changed an edge.
    // Sweep once after the entire (possibly partial) definition transaction.
    let mut scope_sweep = ScopeMutationSweep::new(env);
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
        #[cfg(feature = "quickjs")]
        if let Some(handle) = qjs_handle(object) {
            let attributes = descriptor.attributes & NAPI_DEFAULT_PROPERTY_ATTRIBUTES;
            let status = if descriptor.method.is_some() || descriptor.getter.is_some()
                || descriptor.setter.is_some() {
                qjs_define_callback_property(env, handle, key,
                    descriptor.getter, descriptor.setter,
                    if descriptor.getter.is_some() || descriptor.setter.is_some() {
                        None
                    } else { descriptor.method },
                    descriptor.data, attributes)
            } else {
                let value = if descriptor.value.is_null() {
                    let mut undefined = ptr::null_mut();
                    let status = napi_get_undefined(env, &mut undefined);
                    if status != NAPI_OK { return record_status(env, status); }
                    undefined
                } else { descriptor.value };
                qjs_define_data_property(env, handle, key, value, attributes)
            };
            if status != NAPI_OK { return status; }
            continue;
        }
        let Ok(key_name) = property_key(env, key) else {
            return record_status(env, NAPI_INVALID_ARG);
        };
        let attributes = descriptor.attributes & NAPI_DEFAULT_PROPERTY_ATTRIBUTES;
        if is_array_length_property(object, &key_name) {
            // The intrinsic descriptor is always nonenumerable and
            // nonconfigurable. A definition may omit its value and only make
            // it readonly; an ordinary property setter cannot express that.
            if descriptor.method.is_some() || descriptor.getter.is_some()
                || descriptor.setter.is_some()
                || attributes & (NAPI_ENUMERABLE | NAPI_CONFIGURABLE) != 0 {
                return record_status(env, NAPI_GENERIC_FAILURE);
            }
            let writable = property_attributes_for(env, object as usize, &key_name)
                & NAPI_WRITABLE != 0;
            let old_length = match value_ref(object) {
                Ok(Value::Array(values)) => values.len(),
                _ => return record_status(env, NAPI_OBJECT_EXPECTED),
            };
            if !writable && attributes & NAPI_WRITABLE != 0 {
                return record_status(env, NAPI_GENERIC_FAILURE);
            }
            let status = if descriptor.value.is_null() {
                NAPI_OK
            } else if !writable {
                match value_ref(descriptor.value).and_then(number_for_typedarray) {
                    Ok(number) if number == old_length as f64 => NAPI_OK,
                    _ => NAPI_GENERIC_FAILURE,
                }
            } else {
                // The native array may already have lost higher indices if
                // a lower readonly index makes ArraySetLength fail.
                scope_sweep.changed();
                set_array_length(env, object, descriptor.value,
                    attributes & NAPI_WRITABLE == 0)
            };
            if status != NAPI_OK && !(status == NAPI_GENERIC_FAILURE
                && writable && attributes & NAPI_WRITABLE == 0) {
                return record_status(env, status);
            }
            if descriptor.value.is_null() {
                if let Ok(owner) = env_mut(env) {
                    owner.property_attributes.insert((object as usize, key_name),
                        attributes & NAPI_WRITABLE);
                }
            }
            if status != NAPI_OK { return record_status(env, status); }
            continue;
        }
        if descriptor.getter.is_some() || descriptor.setter.is_some() {
            let old_value = own_property_value(env, object, &key_name);
            let old_accessor = accessor_for_owner(env, object as usize, &key_name);
            let exists = old_value.is_some() || old_accessor.is_some();
            let old_attributes = property_attributes_for(env, object as usize, &key_name);
            if fixed_index_exists(object, &key_name) {
                return record_status(env, NAPI_GENERIC_FAILURE);
            }
            if exists && old_attributes & NAPI_CONFIGURABLE == 0 {
                // A nonconfigurable descriptor cannot switch data/accessor
                // kind, change its enumerable bit, or replace either native
                // callback. An identical definition is a no-op.
                let same = old_accessor.as_ref().is_some_and(|old| {
                    #[cfg(feature = "quickjs")]
                    let native_only = old.js_owner.is_none();
                    #[cfg(not(feature = "quickjs"))]
                    let native_only = true;
                    old.getter.map(|callback| callback as usize)
                        == descriptor.getter.map(|callback| callback as usize)
                        && old.setter.map(|callback| callback as usize)
                            == descriptor.setter.map(|callback| callback as usize)
                        && old.getter_data == descriptor.data
                        && old.setter_data == descriptor.data
                        && native_only
                });
                if !same || attributes & (NAPI_ENUMERABLE | NAPI_CONFIGURABLE)
                    != old_attributes & (NAPI_ENUMERABLE | NAPI_CONFIGURABLE) {
                    return record_status(env, NAPI_GENERIC_FAILURE);
                }
                continue;
            }
            // Resolve the old slot and length guard before taking the Env's
            // mutable borrow.  own_property_value may allocate for an indexed
            // native view, so it must not reborrow the Env from inside that
            // mutation scope.
            let had_data = old_value.is_some();
            let beyond_readonly_length =
                array_index_exceeds_readonly_length(env, object, &key_name);
            let replaced = {
                let Ok(owner) = env_mut(env) else { return NAPI_INVALID_ARG; };
                if owner.sealed_objects.contains(&(object as usize))
                    || (owner.nonextensible_objects.contains(&(object as usize)) && !exists) {
                    return record_status(env, NAPI_GENERIC_FAILURE);
                }
                if beyond_readonly_length {
                    return record_status(env, NAPI_GENERIC_FAILURE);
                }
                if had_data {
                    let status = remove_own_property(owner, object, &key_name);
                    if status != NAPI_OK { return record_status(env, status); }
                }
                extend_array_for_own_index(object, &key_name);
                let replaced = owner.accessors.insert(
                    (object as usize, key_name.clone()),
                    Accessor {
                        getter: descriptor.getter,
                        setter: descriptor.setter,
                        getter_data: descriptor.data,
                        setter_data: descriptor.data,
                        getter_reflection: None,
                        setter_reflection: None,
                        #[cfg(feature = "quickjs")]
                        js_owner: None,
                    },
                );
                record_property_order(owner, object as usize, &key_name);
                owner.property_attributes
                    .insert((object as usize, key_name), attributes);
                replaced
            };
            // A replaced JS accessor may release its last QuickJS handle and
            // run a finalizer. Do so after the Env mutation borrow ends.
            drop(replaced);
            scope_sweep.changed();
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
        // Definition writes an own descriptor directly. A property setter
        // must not run, and a nonconfigurable predecessor must pass the same
        // compatibility checks as the trusted graph descriptor route.
        if is_array_length_property(object, &key_name) { scope_sweep.changed(); }
        let status = qjs_install_native_data_property(env, object, key_name,
            value, descriptor.method.is_some() || !descriptor.value.is_null(),
            attributes);
        if status != NAPI_OK {
            return record_status(env, status);
        }
        scope_sweep.changed();
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
    if env.finalizing || env.finalized {
        return record_status(env_ptr, NAPI_CLOSING);
    }
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
    if env.finalizing || env.finalized {
        return record_status(env_ptr, NAPI_CLOSING);
    }
    env.object_finalizers
        .entry(object as usize)
        .or_default()
        .push(FinalizeRecord {
            data,
            finalize,
            hint,
            backing: ptr::null_mut(),
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
    if env.finalized {
        return record_status(env_ptr, NAPI_CLOSING);
    }
    env.posted_finalizers.push(FinalizeRecord {
        data,
        finalize: Some(finalize),
        hint,
        backing: ptr::null_mut(),
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
    if env.finalizing || env.finalized {
        return record_status(env_ptr, NAPI_CLOSING);
    }
    if env.instance_data.is_some() {
        return record_status(env_ptr, NAPI_GENERIC_FAILURE);
    }
    env.instance_data = Some(FinalizeRecord {
        data,
        finalize,
        hint,
        backing: ptr::null_mut(),
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
    if env.finalizing || env.finalized {
        return record_status(env_ptr, NAPI_CLOSING);
    }
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
    if env.is_null() { return NAPI_INVALID_ARG; }
    // Async hooks can outlive this call. A stack/direct Env has no owner that
    // can retain it until the hook removes its handle.
    if (*env).owner != std::thread::current().id() { return NAPI_INVALID_ARG; }
    let (Ok(env_ref), Some(hook)) = (env_mut(env), hook) else {
        return record_status(env_ptr, NAPI_INVALID_ARG);
    };
    if !env_ref.host_managed || env_ref.finalizing || env_ref.finalized
        || (env_ref.shutdown_requested && !env_ref.async_cleanup_dispatching)
    {
        return record_status(env_ptr, NAPI_CLOSING);
    }
    let mut handle = Box::new(AsyncCleanupHookHandle {
        env: env as usize,
        hook,
        data: data as usize,
        state: AtomicU8::new(0),
    });
    let handle_ptr = (&mut *handle) as *mut AsyncCleanupHookHandle;
    let mut handles = async_cleanup_handles()
        .lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    handles.push(handle);
    env_ref.async_cleanup_hooks.push(handle_ptr);
    drop(handles);
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
        // A foreign thread may remove before owner dispatch. Leave a
        // tombstone in Env's Vec; owner begin skips state 2 under this lock.
        0 => {}
        1 => {
            ACTIVE_ASYNC_CLEANUP_HOOKS.fetch_sub(1, Ordering::AcqRel);
        }
        _ => {
            return NAPI_INVALID_ARG;
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
    napi_new_instance_with_target(env, constructor, constructor,
        ptr::null_mut(), argc, argv, result)
}

// The ordinary N-API entry uses its constructor as new.target. A trusted
// QuickJS Proxy construct trap may supply a different engine new.target;
// keep that extension private so the public N-API ABI remains unchanged.
unsafe fn napi_new_instance_with_target(
    env: NapiEnv,
    constructor: NapiValue,
    new_target: NapiValue,
    fallback_prototype: NapiValue,
    argc: usize,
    argv: *const NapiValue,
    result: *mut NapiValue,
) -> NapiStatus {
    if result.is_null() || (argc != 0 && argv.is_null()) {
        return record_status(env, NAPI_INVALID_ARG);
    }
    if !value_belongs_to_environment(env, new_target) {
        return record_status(env, NAPI_INVALID_ARG);
    }
    if !fallback_prototype.is_null()
        && (!value_belongs_to_environment(env, fallback_prototype)
            || !matches!(value_ref(fallback_prototype), Ok(value) if is_object_value(value))) {
        return record_status(env, NAPI_INVALID_ARG);
    }
    #[cfg(feature = "quickjs")]
    if !value_belongs_to_environment(env, constructor) {
        return record_status(env, NAPI_INVALID_ARG);
    }
    #[cfg(feature = "quickjs")]
    if let Some(handle) = qjs_handle(constructor) {
        let args = if argc == 0 { Vec::new() }
            else { std::slice::from_raw_parts(argv, argc).to_vec() };
        if args.iter().any(|arg| !value_belongs_to_environment(env, *arg)) {
            return record_status(env, NAPI_INVALID_ARG);
        }
        if env.as_ref().is_some_and(|owner| owner.exception.is_some()) {
            return record_status(env, NAPI_PENDING_EXCEPTION);
        }
        return qjs_construct_function(env, handle, &args, result);
    }
    if !value_belongs_to_environment(env, constructor)
        || (argc != 0
            && std::slice::from_raw_parts(argv, argc)
                .iter()
                .any(|value| !value_belongs_to_environment(env, *value)))
        || env.as_ref().is_some_and(|owner| owner.exception.is_some())
    {
        let status = if env.as_ref().is_some_and(|owner| owner.exception.is_some()) {
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
    // A property accessor is callable but is never a constructor. Its
    // reflection Function owns a descriptor snapshot solely for call-time
    // receiver dispatch; do not run it as a class initializer.
    if function._accessor_owner.is_some() {
        return record_status(env, NAPI_FUNCTION_EXPECTED);
    }
    // Get(new.target, "prototype") may invoke user code even when the
    // constructor itself is new.target. Snapshot the callback first, then
    // perform this read before taking an Env mutation borrow.
    let prototype = {
        let mut prototype = ptr::null_mut();
        let status = napi_get_named_property(env, new_target,
            c"prototype".as_ptr(), &mut prototype);
        if status != NAPI_OK { return record_status(env, status); }
        match value_ref(prototype) {
            Ok(value) if is_object_value(value) => Some(prototype),
            _ => (!fallback_prototype.is_null()).then_some(fallback_prototype),
        }
    };
    let Ok(host_env) = env_mut(env) else { return NAPI_INVALID_ARG; };
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
        new_target,
        data: function.data,
    };
    let returned = invoke_napi_callback(env, function.callback, &mut info);
    if env_mut(env)
        .map(|env| env.exception.is_some())
        .unwrap_or(false)
    {
        return record_status(env, NAPI_PENDING_EXCEPTION);
    }
    if !returned.is_null() && !value_belongs_to_environment(env, returned) {
        return NAPI_INVALID_ARG;
    }
    let explicit_object = matches!(returned.as_ref(), Some(value) if is_object_value(value));
    if explicit_object && matches!(value_ref(returned), Ok(Value::Object(_) | Value::Array(_))) {
        // A constructor's explicit return must remain that same object on
        // later graph transfers. Do not register it as a class instance:
        // its own prototype and instanceof identity are independent.
        let Ok(owner) = env_mut(env) else { return NAPI_INVALID_ARG; };
        owner.live_constructor_returns.insert(returned as usize);
    }
    *result = if explicit_object { returned } else { instance };
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
    #[cfg(feature = "quickjs")]
    if let (Some(object), Some(constructor)) = (qjs_handle(object), qjs_handle(constructor)) {
        return qjs_instanceof_handles(env, object, constructor, result);
    }
    #[cfg(feature = "quickjs")]
    if qjs_handle(object).is_some() || qjs_handle(constructor).is_some() {
        if !matches!(value_ref(constructor), Ok(Value::QuickJsHandle { .. } | Value::Function(_))) {
            return record_status(env, NAPI_FUNCTION_EXPECTED);
        }
        return qjs_instanceof(env, object, constructor, result);
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
    #[cfg(feature = "quickjs")]
    if let Some(handle) = qjs_handle(object) {
        if matches!(value_ref(object), Ok(Value::QuickJsHandle { object_like: false, .. })) {
            return record_status(env, NAPI_OBJECT_EXPECTED);
        }
        return qjs_get_prototype(env, handle, result);
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
        Some(prototype) => write_scoped_value(env_ptr, result, prototype),
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
    #[cfg(feature = "quickjs")]
    if let Some(handle) = qjs_handle(object) {
        if matches!(value_ref(object), Ok(Value::QuickJsHandle { object_like: false, .. })) {
            return record_status(env, NAPI_OBJECT_EXPECTED);
        }
        if !matches!(value_ref(prototype), Ok(value) if is_object_value(value) || matches!(value, Value::Null)) {
            return record_status(env, NAPI_OBJECT_EXPECTED);
        }
        return qjs_set_prototype(env, handle, prototype);
    }
    let _dispatch = ForeignCallbackGuard::new();
    let mut scope_sweep = ScopeMutationSweep::new(env);
    if !matches!(value_ref(object), Ok(value) if is_object_value(value)) {
        return record_status(env, NAPI_OBJECT_EXPECTED);
    }
    if !matches!(value_ref(prototype), Ok(value) if is_object_value(value) || matches!(value, Value::Null))
    {
        return record_status(env, NAPI_OBJECT_EXPECTED);
    }

    let object_id = object as usize;
    let prototype_id = prototype as usize;
    let prototype_is_null = matches!(value_ref(prototype), Ok(Value::Null));
    let Ok(env) = env_mut(env) else {
        return NAPI_INVALID_ARG;
    };
    if env.prototypes.get(&object_id).copied() == Some(prototype_id) {
        return NAPI_OK;
    }
    if env.nonextensible_objects.contains(&object_id) {
        return record_status(env, NAPI_GENERIC_FAILURE);
    }

    if !prototype_is_null {
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
    scope_sweep.changed();
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
    #[cfg(feature = "quickjs")]
    if let (Some(left), Some(right)) = (qjs_handle(left), qjs_handle(right)) {
        return qjs_strict_equals(env, left, right, result);
    }
    #[cfg(feature = "quickjs")]
    if let Some(handle) = qjs_handle(left) {
        if matches!(value_ref(right), Ok(Value::Symbol { .. })) {
            return qjs_strict_equals_native_symbol(env, handle, right, result);
        }
        if matches!(value_ref(right), Ok(Value::Undefined | Value::Null | Value::Bool(_)
            | Value::Number(_) | Value::String(_) | Value::BigInt { .. })) {
            return qjs_strict_equals_scalar(env, handle, right, result);
        }
    }
    #[cfg(feature = "quickjs")]
    if let Some(handle) = qjs_handle(right) {
        if matches!(value_ref(left), Ok(Value::Symbol { .. })) {
            return qjs_strict_equals_native_symbol(env, handle, left, result);
        }
        if matches!(value_ref(left), Ok(Value::Undefined | Value::Null | Value::Bool(_)
            | Value::Number(_) | Value::String(_) | Value::BigInt { .. })) {
            return qjs_strict_equals_scalar(env, handle, left, result);
        }
    }
    *result = match (value_ref(left), value_ref(right)) {
        (Ok(Value::Undefined), Ok(Value::Undefined)) | (Ok(Value::Null), Ok(Value::Null)) => true,
        (Ok(Value::Bool(left)), Ok(Value::Bool(right))) => left == right,
        (Ok(Value::Number(left)), Ok(Value::Number(right))) => left == right,
        (Ok(Value::String(left_text)), Ok(Value::String(right_text))) => {
            let left_units = env.as_ref().and_then(|owner| owner.utf16_strings.get(&(left as usize)));
            let right_units = env.as_ref().and_then(|owner| owner.utf16_strings.get(&(right as usize)));
            if left_units.is_none() && right_units.is_none() {
                left_text == right_text
            } else {
                let left = left_units.cloned().unwrap_or_else(|| left_text.encode_utf16().collect());
                let right = right_units.cloned().unwrap_or_else(|| right_text.encode_utf16().collect());
                left == right
            }
        },
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
