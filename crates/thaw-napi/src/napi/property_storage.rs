fn property_array_index(key: &PropertyKey) -> Option<usize> {
    let PropertyKey::String(name) = key else {
        return None;
    };
    let index = name.parse::<u32>().ok()?;
    (index != u32::MAX && index.to_string() == *name).then_some(index as usize)
}

fn record_property_order(env: &mut Env, owner: usize, key: &PropertyKey) {
    let order = env.property_order.entry(owner).or_default();
    if !order.contains(key) {
        order.push(key.clone());
    }
}

fn remove_property_order(env: &mut Env, owner: usize, key: &PropertyKey) {
    if let Some(order) = env.property_order.get_mut(&owner) {
        order.retain(|candidate| candidate != key);
    }
}

unsafe fn host_property_for_owner(
    env: NapiEnv,
    owner: usize,
    key: &PropertyKey,
) -> Option<NapiValue> {
    env.as_ref()
        .and_then(|env| env.host_properties.get(&owner)?.get(key).copied())
        .or_else(|| {
            HOST.with(|host| {
                let host = host.borrow();
                host.module_envs
                    .iter()
                    .chain(host.pending_call_envs.iter())
                    .find_map(|candidate| candidate.host_properties.get(&owner)?.get(key).copied())
            })
        })
}

unsafe fn host_property_keys_for_owner(env: NapiEnv, owner: usize) -> Vec<PropertyKey> {
    let mut keys = env
        .as_ref()
        .and_then(|env| env.host_properties.get(&owner))
        .map(|properties| properties.keys().cloned().collect::<Vec<_>>())
        .unwrap_or_default();
    HOST.with(|host| {
        let host = host.borrow();
        for candidate in host.module_envs.iter().chain(host.pending_call_envs.iter()) {
            if let Some(properties) = candidate.host_properties.get(&owner) {
                keys.extend(properties.keys().cloned());
            }
        }
    });
    keys
}

unsafe fn property_order_for_owner(env: NapiEnv, owner: usize, key: &PropertyKey) -> usize {
    env.as_ref()
        .and_then(|env| env.property_order.get(&owner))
        .and_then(|order| order.iter().position(|candidate| candidate == key))
        .or_else(|| {
            HOST.with(|host| {
                let host = host.borrow();
                host.module_envs
                    .iter()
                    .chain(host.pending_call_envs.iter())
                    .find_map(|candidate| {
                        candidate
                            .property_order
                            .get(&owner)?
                            .iter()
                            .position(|candidate| candidate == key)
                    })
            })
        })
        .unwrap_or(usize::MAX)
}

unsafe fn error_name_for_owner(env: NapiEnv, owner: usize) -> Option<String> {
    env.as_ref()
        .and_then(|env| env.error_names.get(&owner).cloned())
        .or_else(|| {
            HOST.with(|host| {
                let host = host.borrow();
                host.module_envs
                    .iter()
                    .chain(host.pending_call_envs.iter())
                    .find_map(|candidate| candidate.error_names.get(&owner).cloned())
            })
        })
}

unsafe fn typedarray_index_parts(object: NapiValue, key: &PropertyKey) -> Option<(i32, *mut u8)> {
    let index = property_array_index(key)?;
    let Value::TypedArray {
        array_type,
        length,
        array_buffer,
        byte_offset,
    } = value_ref(object).ok()?
    else {
        return None;
    };
    if index >= *length {
        return None;
    }
    let (bytes, _, detached) = arraybuffer_parts(*array_buffer).ok()?;
    if detached {
        return None;
    }
    let element_size = typedarray_element_size(*array_type)?;
    Some((*array_type, bytes.add(*byte_offset + index * element_size)))
}

unsafe fn read_typedarray_index(
    env: NapiEnv,
    object: NapiValue,
    key: &PropertyKey,
) -> Option<NapiValue> {
    let (array_type, data) = typedarray_index_parts(object, key)?;
    let env = env_mut(env).ok()?;
    Some(match array_type {
        0 => env.alloc(Value::Number(ptr::read_unaligned(data.cast::<i8>()) as f64)),
        1 | 2 => env.alloc(Value::Number(ptr::read_unaligned(data) as f64)),
        3 => env.alloc(Value::Number(ptr::read_unaligned(data.cast::<i16>()) as f64)),
        4 => env.alloc(Value::Number(ptr::read_unaligned(data.cast::<u16>()) as f64)),
        5 => env.alloc(Value::Number(ptr::read_unaligned(data.cast::<i32>()) as f64)),
        6 => env.alloc(Value::Number(ptr::read_unaligned(data.cast::<u32>()) as f64)),
        7 => env.alloc(Value::Number(ptr::read_unaligned(data.cast::<f32>()) as f64)),
        8 => env.alloc(Value::Number(ptr::read_unaligned(data.cast::<f64>()))),
        9 => {
            let value = ptr::read_unaligned(data.cast::<i64>());
            env.alloc(Value::BigInt {
                negative: value.is_negative(),
                words: vec![value.unsigned_abs()],
            })
        }
        10 => env.alloc(Value::BigInt {
            negative: false,
            words: vec![ptr::read_unaligned(data.cast::<u64>())],
        }),
        _ => return None,
    })
}

fn number_for_typedarray(value: &Value) -> Result<f64, NapiStatus> {
    Ok(match value {
        Value::Undefined => f64::NAN,
        Value::Null => 0.0,
        Value::Bool(value) => u8::from(*value) as f64,
        Value::Number(value) => *value,
        Value::String(value) => javascript_number_from_string(value),
        Value::BigInt { .. } | Value::Symbol { .. } => return Err(NAPI_GENERIC_FAILURE),
        _ => f64::NAN,
    })
}

fn integer_modulo(number: f64, bits: u32) -> u64 {
    if !number.is_finite() || number == 0.0 {
        return 0;
    }
    number.trunc().rem_euclid(2_f64.powi(bits as i32)) as u64
}

fn uint8_clamp(number: f64) -> u8 {
    if number.is_nan() || number <= 0.0 {
        return 0;
    }
    if number >= 255.0 {
        return 255;
    }
    let floor = number.floor();
    let fraction = number - floor;
    if fraction > 0.5 || (fraction == 0.5 && floor as u64 % 2 == 1) {
        floor as u8 + 1
    } else {
        floor as u8
    }
}

unsafe fn write_typedarray_index(
    env: &mut Env,
    object: NapiValue,
    key: &PropertyKey,
    value: NapiValue,
) -> Option<NapiStatus> {
    let (array_type, data) = typedarray_index_parts(object, key)?;
    let value_ref = match value_ref(value) {
        Ok(value) => value,
        Err(status) => return Some(status),
    };
    let status = match array_type {
        0..=8 => {
            let number = match number_for_typedarray(value_ref) {
                Ok(number) => number,
                Err(_) => {
                    let error =
                        env.alloc(Value::Error("typed array value has the wrong type".into()));
                    env.exception = Some(error);
                    return Some(NAPI_PENDING_EXCEPTION);
                }
            };
            match array_type {
                0 => ptr::write_unaligned(data.cast::<i8>(), integer_modulo(number, 8) as u8 as i8),
                1 => ptr::write_unaligned(data, integer_modulo(number, 8) as u8),
                2 => ptr::write_unaligned(data, uint8_clamp(number)),
                3 => ptr::write_unaligned(
                    data.cast::<i16>(),
                    integer_modulo(number, 16) as u16 as i16,
                ),
                4 => ptr::write_unaligned(data.cast::<u16>(), integer_modulo(number, 16) as u16),
                5 => ptr::write_unaligned(
                    data.cast::<i32>(),
                    integer_modulo(number, 32) as u32 as i32,
                ),
                6 => ptr::write_unaligned(data.cast::<u32>(), integer_modulo(number, 32) as u32),
                7 => ptr::write_unaligned(data.cast::<f32>(), number as f32),
                8 => ptr::write_unaligned(data.cast::<f64>(), number),
                _ => unreachable!(),
            }
            NAPI_OK
        }
        9 | 10 => {
            let Value::BigInt { negative, words } = value_ref else {
                let error = env.alloc(Value::Error("BigInt typed arrays require a BigInt".into()));
                env.exception = Some(error);
                return Some(NAPI_PENDING_EXCEPTION);
            };
            let magnitude = words.first().copied().unwrap_or(0);
            let bits = if *negative {
                0_u64.wrapping_sub(magnitude)
            } else {
                magnitude
            };
            ptr::write_unaligned(data.cast::<u64>(), bits);
            NAPI_OK
        }
        _ => NAPI_INVALID_ARG,
    };
    Some(status)
}

unsafe fn intrinsic_property_value(
    env: NapiEnv,
    object: NapiValue,
    key: &PropertyKey,
) -> Option<NapiValue> {
    let PropertyKey::String(name) = key else {
        return None;
    };
    if matches!(value_ref(object), Ok(Value::Error(_))) && name == "name" {
        let name = error_name_for_owner(env, object as usize).unwrap_or_else(|| "Error".into());
        return env_mut(env).ok().map(|env| env.alloc(Value::String(name)));
    }
    let env = env_mut(env).ok()?;
    match value_ref(object).ok()? {
        Value::Array(values) if name == "length" => {
            Some(env.alloc(Value::Number(values.len() as f64)))
        }
        Value::Buffer(bytes) if name == "length" => {
            Some(env.alloc(Value::Number(bytes.len() as f64)))
        }
        Value::ExternalBuffer { length, .. } if name == "length" => {
            Some(env.alloc(Value::Number(*length as f64)))
        }
        Value::BufferView {
            array_buffer,
            length,
            ..
        } if name == "length" => {
            let detached = arraybuffer_parts(*array_buffer)
                .map(|(_, _, detached)| detached)
                .unwrap_or(true);
            Some(env.alloc(Value::Number(if detached { 0.0 } else { *length as f64 })))
        }
        Value::ArrayBuffer { bytes, detached } if name == "byteLength" => {
            Some(env.alloc(Value::Number(if *detached {
                0.0
            } else {
                bytes.len() as f64
            })))
        }
        Value::ExternalArrayBuffer {
            length, detached, ..
        } if name == "byteLength" => {
            Some(env.alloc(Value::Number(if *detached { 0.0 } else { *length as f64 })))
        }
        Value::SharedArrayBuffer(bytes) if name == "byteLength" => {
            Some(env.alloc(Value::Number(bytes.len() as f64)))
        }
        Value::ExternalSharedArrayBuffer { length, .. } if name == "byteLength" => {
            Some(env.alloc(Value::Number(*length as f64)))
        }
        Value::TypedArray {
            array_type,
            length,
            array_buffer,
            byte_offset,
        } => {
            let detached = arraybuffer_parts(*array_buffer)
                .map(|(_, _, detached)| detached)
                .unwrap_or(true);
            match name.as_str() {
                "length" => {
                    Some(env.alloc(Value::Number(if detached { 0.0 } else { *length as f64 })))
                }
                "byteLength" => Some(env.alloc(Value::Number(if detached {
                    0.0
                } else {
                    (*length * typedarray_element_size(*array_type)?) as f64
                }))),
                "byteOffset" => Some(env.alloc(Value::Number(if detached {
                    0.0
                } else {
                    *byte_offset as f64
                }))),
                "buffer" => Some(*array_buffer),
                _ => None,
            }
        }
        Value::DataView {
            length,
            array_buffer,
            byte_offset,
        } => {
            let detached = arraybuffer_parts(*array_buffer)
                .map(|(_, _, detached)| detached)
                .unwrap_or(true);
            match name.as_str() {
                "byteLength" => {
                    Some(env.alloc(Value::Number(if detached { 0.0 } else { *length as f64 })))
                }
                "byteOffset" => Some(env.alloc(Value::Number(if detached {
                    0.0
                } else {
                    *byte_offset as f64
                }))),
                "buffer" => Some(*array_buffer),
                _ => None,
            }
        }
        _ => None,
    }
}

unsafe fn intrinsic_property_keys(object: NapiValue) -> &'static [&'static str] {
    match value_ref(object) {
        Ok(
            Value::Array(_)
            | Value::Buffer(_)
            | Value::ExternalBuffer { .. }
            | Value::BufferView { .. },
        ) => &["length"],
        Ok(
            Value::ArrayBuffer { .. }
            | Value::ExternalArrayBuffer { .. }
            | Value::SharedArrayBuffer(_)
            | Value::ExternalSharedArrayBuffer { .. },
        ) => &["byteLength"],
        Ok(Value::TypedArray { .. }) => &["length", "byteLength", "byteOffset", "buffer"],
        Ok(Value::DataView { .. }) => &["byteLength", "byteOffset", "buffer"],
        _ => &[],
    }
}

unsafe fn intrinsic_property_attributes(object: NapiValue, key: &PropertyKey) -> Option<u32> {
    let PropertyKey::String(name) = key else {
        return None;
    };
    if !intrinsic_property_keys(object).contains(&name.as_str()) {
        return None;
    }
    Some(
        if matches!(value_ref(object), Ok(Value::Array(_))) && name == "length" {
            NAPI_WRITABLE
        } else {
            0
        },
    )
}

unsafe fn own_property_value(
    env: NapiEnv,
    owner: NapiValue,
    key: &PropertyKey,
) -> Option<NapiValue> {
    if matches!(value_ref(owner), Ok(Value::Array(_))) {
        if let Some(value) = intrinsic_property_value(env, owner, key) {
            return Some(value);
        }
    }
    match value_ref(owner) {
        Ok(Value::Object(properties)) => properties.get(key).copied(),
        Ok(Value::Function(function)) => function.properties.get(key).copied(),
        Ok(Value::Array(values)) => property_array_index(key)
            .and_then(|index| values.get(index).copied().flatten())
            .or_else(|| host_property_for_owner(env, owner as usize, key)),
        Ok(Value::Buffer(bytes)) => property_array_index(key)
            .and_then(|index| bytes.get(index).copied())
            .and_then(|byte| {
                env_mut(env)
                    .ok()
                    .map(|env| env.alloc(Value::Number(byte.into())))
            })
            .or_else(|| host_property_for_owner(env, owner as usize, key)),
        Ok(Value::ExternalBuffer { data, length }) => property_array_index(key)
            .filter(|index| *index < *length && !data.is_null())
            .and_then(|index| {
                env_mut(env)
                    .ok()
                    .map(|env| env.alloc(Value::Number((*data.add(index)).into())))
            })
            .or_else(|| host_property_for_owner(env, owner as usize, key)),
        Ok(Value::BufferView {
            array_buffer,
            byte_offset,
            length,
        }) => property_array_index(key)
            .filter(|index| *index < *length)
            .and_then(|index| {
                let (bytes, _, detached) = arraybuffer_parts(*array_buffer).ok()?;
                (!detached).then(|| *bytes.add(*byte_offset + index))
            })
            .and_then(|byte| {
                env_mut(env)
                    .ok()
                    .map(|env| env.alloc(Value::Number(byte.into())))
            })
            .or_else(|| host_property_for_owner(env, owner as usize, key)),
        Ok(Value::TypedArray { .. }) => read_typedarray_index(env, owner, key)
            .or_else(|| host_property_for_owner(env, owner as usize, key)),
        Ok(value) if is_object_value(value) => host_property_for_owner(env, owner as usize, key),
        _ => None,
    }
}

unsafe fn set_own_property(
    env: &mut Env,
    object: NapiValue,
    key: &PropertyKey,
    value: NapiValue,
) -> NapiStatus {
    if intrinsic_property_attributes(object, key).is_some() {
        if matches!(value_ref(object), Ok(Value::Array(_)))
            && matches!(key, PropertyKey::String(name) if name == "length")
        {
            let number = match value_ref(value).and_then(number_for_typedarray) {
                Ok(number)
                    if number.is_finite()
                        && number >= 0.0
                        && number <= u32::MAX as f64
                        && number.fract() == 0.0 =>
                {
                    number as usize
                }
                _ => {
                    let error = env.alloc(Value::Error("invalid array length".into()));
                    env.exception = Some(error);
                    return NAPI_PENDING_EXCEPTION;
                }
            };
            let Some(Value::Array(values)) = object.as_mut() else {
                unreachable!();
            };
            values.resize(number, None);
            return NAPI_OK;
        }
        return NAPI_GENERIC_FAILURE;
    }
    match object.as_mut() {
        Some(Value::Object(properties)) => {
            properties.insert(key.clone(), value);
        }
        Some(Value::Function(function)) => {
            function.properties.insert(key.clone(), value);
        }
        Some(Value::Array(values)) if property_array_index(key).is_some() => {
            let index = property_array_index(key).unwrap();
            if values.len() <= index {
                values.resize(index.saturating_add(1), None);
            }
            values[index] = Some(value);
        }
        Some(Value::Buffer(bytes)) if property_array_index(key).is_some() => {
            if let Some(byte) = bytes.get_mut(property_array_index(key).unwrap()) {
                *byte = match uint8_from_value(value) {
                    Ok(value) => value,
                    Err(status) => return status,
                };
            }
        }
        Some(Value::ExternalBuffer { data, length }) if property_array_index(key).is_some() => {
            let index = property_array_index(key).unwrap();
            if index < *length && !data.is_null() {
                *data.add(index) = match uint8_from_value(value) {
                    Ok(value) => value,
                    Err(status) => return status,
                };
            }
        }
        Some(Value::BufferView {
            array_buffer,
            byte_offset,
            length,
        }) if property_array_index(key).is_some() => {
            let index = property_array_index(key).unwrap();
            if index < *length {
                let Ok((bytes, _, detached)) = arraybuffer_parts(*array_buffer) else {
                    return NAPI_ARRAYBUFFER_EXPECTED;
                };
                if !detached {
                    *bytes.add(*byte_offset + index) = match uint8_from_value(value) {
                        Ok(value) => value,
                        Err(status) => return status,
                    };
                }
            }
        }
        Some(Value::TypedArray { .. }) if property_array_index(key).is_some() => {
            if let Some(status) = write_typedarray_index(env, object, key, value) {
                return status;
            }
        }
        Some(object_value) if is_object_value(object_value) => {
            env.host_properties
                .entry(object as usize)
                .or_default()
                .insert(key.clone(), value);
        }
        _ => return NAPI_OBJECT_EXPECTED,
    }
    NAPI_OK
}

unsafe fn uint8_from_value(value: NapiValue) -> Result<u8, NapiStatus> {
    let number = match value_ref(value)? {
        Value::Undefined => f64::NAN,
        Value::Null => 0.0,
        Value::Bool(value) => u8::from(*value) as f64,
        Value::Number(value) => *value,
        Value::String(value) => javascript_number_from_string(value),
        Value::BigInt { .. } | Value::Symbol { .. } => return Err(NAPI_GENERIC_FAILURE),
        _ => f64::NAN,
    };
    Ok(if number.is_finite() {
        number.trunc().rem_euclid(256.0) as u8
    } else {
        0
    })
}

unsafe fn fixed_index_exists(object: NapiValue, key: &PropertyKey) -> bool {
    if matches!(value_ref(object), Ok(Value::Array(_)))
        && intrinsic_property_attributes(object, key).is_some()
    {
        return true;
    }
    let Some(index) = property_array_index(key) else {
        return false;
    };
    match value_ref(object) {
        Ok(Value::Buffer(bytes)) => index < bytes.len(),
        Ok(Value::ExternalBuffer { data, length }) => index < *length && !data.is_null(),
        Ok(Value::BufferView {
            array_buffer,
            length,
            ..
        }) => {
            index < *length
                && arraybuffer_parts(*array_buffer).is_ok_and(|(_, _, detached)| !detached)
        }
        Ok(Value::TypedArray { .. }) => typedarray_index_parts(object, key).is_some(),
        _ => false,
    }
}

unsafe fn remove_own_property(env: &mut Env, object: NapiValue, key: &PropertyKey) -> NapiStatus {
    match object.as_mut() {
        Some(Value::Object(properties)) => {
            properties.remove(key);
        }
        Some(Value::Function(function)) => {
            function.properties.remove(key);
        }
        Some(Value::Array(values)) if property_array_index(key).is_some() => {
            if let Some(value) = values.get_mut(property_array_index(key).unwrap()) {
                *value = None;
            }
        }
        Some(object_value) if is_object_value(object_value) => {
            if let Some(properties) = env.host_properties.get_mut(&(object as usize)) {
                properties.remove(key);
            }
        }
        _ => return NAPI_OBJECT_EXPECTED,
    }
    NAPI_OK
}

unsafe fn find_accessor(env: NapiEnv, object: NapiValue, name: &PropertyKey) -> Option<Accessor> {
    let mut current = Some(object as usize);
    let mut visited = HashSet::new();
    while let Some(owner) = current.filter(|owner| visited.insert(*owner)) {
        let has_data_property = own_property_value(env, owner as NapiValue, name).is_some();
        if has_data_property {
            return None;
        }
        if let Some(accessor) = accessor_for_owner(env, owner, name) {
            return Some(accessor);
        }
        current = prototype_for_owner(env, owner);
    }
    None
}

unsafe fn accessor_for_owner(env: NapiEnv, owner: usize, key: &PropertyKey) -> Option<Accessor> {
    env.as_ref()
        .and_then(|env| env.accessors.get(&(owner, key.clone())).copied())
        .or_else(|| {
            HOST.with(|host| {
                host.borrow()
                    .module_envs
                    .iter()
                    .find_map(|module_env| module_env.accessors.get(&(owner, key.clone())).copied())
            })
        })
}

unsafe fn prototype_for_owner(env: NapiEnv, owner: usize) -> Option<usize> {
    env.as_ref()
        .and_then(|env| env.prototypes.get(&owner).copied())
        .or_else(|| {
            HOST.with(|host| {
                host.borrow()
                    .module_envs
                    .iter()
                    .find_map(|module_env| module_env.prototypes.get(&owner).copied())
            })
        })
}

unsafe fn accessors_for_owner(env: NapiEnv, owner: usize) -> Vec<PropertyKey> {
    let mut keys = env
        .as_ref()
        .map(|env| {
            env.accessors
                .keys()
                .filter(|(accessor_owner, _)| *accessor_owner == owner)
                .map(|(_, key)| key.clone())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    HOST.with(|host| {
        for module_env in &host.borrow().module_envs {
            keys.extend(
                module_env
                    .accessors
                    .keys()
                    .filter(|(accessor_owner, _)| *accessor_owner == owner)
                    .map(|(_, key)| key.clone()),
            );
        }
    });
    keys
}

unsafe fn property_attributes_for(env: NapiEnv, owner: usize, key: &PropertyKey) -> u32 {
    if let Some(attributes) = intrinsic_property_attributes(owner as NapiValue, key) {
        return attributes;
    }
    env.as_ref()
        .and_then(|env| env.property_attributes.get(&(owner, key.clone())).copied())
        .or_else(|| {
            HOST.with(|host| {
                host.borrow().module_envs.iter().find_map(|module_env| {
                    module_env
                        .property_attributes
                        .get(&(owner, key.clone()))
                        .copied()
                })
            })
        })
        .unwrap_or(NAPI_DEFAULT_PROPERTY_ATTRIBUTES)
}

unsafe fn symbol_for(env: NapiEnv, id: u64) -> Option<NapiValue> {
    env.as_ref()
        .and_then(|env| env.symbols.get(&id).copied())
        .or_else(|| {
            HOST.with(|host| {
                host.borrow()
                    .module_envs
                    .iter()
                    .find_map(|module_env| module_env.symbols.get(&id).copied())
            })
        })
}

unsafe fn type_tag_for(env: NapiEnv, object: NapiValue) -> Option<NapiTypeTag> {
    env.as_ref()
        .and_then(|env| env.type_tags.get(&(object as usize)).copied())
        .or_else(|| {
            HOST.with(|host| {
                host.borrow()
                    .module_envs
                    .iter()
                    .find_map(|module_env| module_env.type_tags.get(&(object as usize)).copied())
            })
        })
}

unsafe fn find_property_value(
    env: NapiEnv,
    object: NapiValue,
    key: &PropertyKey,
) -> Option<NapiValue> {
    let mut current = Some(object);
    let mut visited = HashSet::new();
    while let Some(value) = current.filter(|value| visited.insert(*value as usize)) {
        let property = own_property_value(env, value, key);
        if property.is_some() {
            return property;
        }
        if let Some(property) = intrinsic_property_value(env, value, key) {
            return Some(property);
        }
        if accessor_for_owner(env, value as usize, key).is_some() {
            return None;
        }
        current = prototype_for_owner(env, value as usize).map(|prototype| prototype as NapiValue);
    }
    None
}

unsafe fn find_data_property_owner(
    env: NapiEnv,
    object: NapiValue,
    key: &PropertyKey,
) -> Option<usize> {
    let mut current = Some(object as usize);
    let mut visited = HashSet::new();
    while let Some(owner) = current.filter(|owner| visited.insert(*owner)) {
        let contains = own_property_value(env, owner as NapiValue, key).is_some();
        if contains {
            return Some(owner);
        }
        if accessor_for_owner(env, owner, key).is_some() {
            return None;
        }
        current = prototype_for_owner(env, owner);
    }
    None
}

unsafe fn write_value(out: *mut NapiValue, value: NapiValue) -> NapiStatus {
    if out.is_null() {
        return NAPI_INVALID_ARG;
    }
    *out = value;
    NAPI_OK
}

unsafe fn write_callback_value(env: NapiEnv, out: *mut NapiValue, value: NapiValue) -> NapiStatus {
    if out.is_null() {
        return NAPI_INVALID_ARG;
    }
    if value.is_null() {
        return napi_get_undefined(env, out);
    }
    if !value_belongs_to_environment(env, value) {
        return NAPI_INVALID_ARG;
    }
    write_value(out, value)
}

