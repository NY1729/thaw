#[no_mangle]
pub unsafe extern "C" fn napi_create_buffer(
    env: NapiEnv,
    length: usize,
    data: *mut *mut c_void,
    out: *mut NapiValue,
) -> NapiStatus {
    let Ok(env) = env_for_value_output(env, out) else {
        return NAPI_INVALID_ARG;
    };
    let value = env.alloc(Value::Buffer(vec![0; length]));
    if !data.is_null() {
        if let Value::Buffer(bytes) = &mut *value {
            *data = bytes.as_mut_ptr().cast();
        }
    }
    write_value(out, value)
}

#[no_mangle]
pub unsafe extern "C" fn napi_create_buffer_copy(
    env: NapiEnv,
    length: usize,
    source: *const c_void,
    data: *mut *mut c_void,
    out: *mut NapiValue,
) -> NapiStatus {
    if source.is_null() && length != 0 {
        return record_status(env, NAPI_INVALID_ARG);
    }
    let status = napi_create_buffer(env, length, data, out);
    if status == NAPI_OK && length != 0 {
        let target = if data.is_null() {
            match (*out).as_mut() {
                Some(Value::Buffer(bytes)) => bytes.as_mut_ptr().cast(),
                _ => return record_status(env, NAPI_INVALID_ARG),
            }
        } else {
            *data
        };
        ptr::copy_nonoverlapping(source.cast::<u8>(), target.cast::<u8>(), length);
    }
    status
}

#[no_mangle]
pub unsafe extern "C" fn napi_create_external_buffer(
    env: NapiEnv,
    length: usize,
    data: *mut c_void,
    finalize: Option<NapiFinalize>,
    hint: *mut c_void,
    out: *mut NapiValue,
) -> NapiStatus {
    if out.is_null() || (length != 0 && data.is_null()) {
        return record_status(env, NAPI_INVALID_ARG);
    }
    let Ok(env) = env_mut(env) else {
        return NAPI_INVALID_ARG;
    };
    let value = env.alloc(Value::ExternalBuffer {
        data: data.cast(),
        length,
    });
    if finalize.is_some() {
        env.finalizers.push(FinalizeRecord {
            data,
            finalize,
            hint,
        });
    }
    write_value(out, value)
}

#[no_mangle]
pub unsafe extern "C" fn node_api_create_buffer_from_arraybuffer(
    env: NapiEnv,
    array_buffer: NapiValue,
    byte_offset: usize,
    byte_length: usize,
    out: *mut NapiValue,
) -> NapiStatus {
    if out.is_null() || !value_belongs_to_environment(env, array_buffer) {
        return record_status(env, NAPI_INVALID_ARG);
    }
    let Ok((_, buffer_length, detached)) = arraybuffer_parts(array_buffer) else {
        return record_status(env, NAPI_ARRAYBUFFER_EXPECTED);
    };
    if detached {
        return record_status(env, NAPI_INVALID_ARG);
    }
    let in_bounds = byte_offset
        .checked_add(byte_length)
        .is_some_and(|end| end <= buffer_length);
    let Ok(env) = env_mut(env) else {
        return NAPI_INVALID_ARG;
    };
    if !in_bounds {
        let error = env.alloc(Value::Error(
            "Buffer range exceeds ArrayBuffer bounds".into(),
        ));
        env.exception = Some(error);
        return record_status(env, NAPI_PENDING_EXCEPTION);
    }
    let value = env.alloc(Value::BufferView {
        array_buffer,
        byte_offset,
        length: byte_length,
    });
    write_value(out, value)
}

#[no_mangle]
pub unsafe extern "C" fn napi_get_buffer_info(
    env: NapiEnv,
    value: NapiValue,
    data: *mut *mut c_void,
    length: *mut usize,
) -> NapiStatus {
    if !value_belongs_to_environment(env, value) {
        return NAPI_INVALID_ARG;
    }
    let status = match value.as_mut() {
        Some(Value::Buffer(bytes)) => {
            if !data.is_null() {
                *data = bytes.as_mut_ptr().cast();
            }
            if !length.is_null() {
                *length = bytes.len();
            }
            NAPI_OK
        }
        Some(Value::ExternalBuffer {
            data: external,
            length: external_length,
        }) => {
            if !data.is_null() {
                *data = external.cast();
            }
            if !length.is_null() {
                *length = *external_length;
            }
            NAPI_OK
        }
        Some(Value::BufferView {
            array_buffer,
            byte_offset,
            length: view_length,
        }) => {
            let (bytes, _, detached) = match arraybuffer_parts(*array_buffer) {
                Ok(parts) => parts,
                Err(status) => return record_status(env, status),
            };
            if !data.is_null() {
                *data = if detached {
                    ptr::null_mut()
                } else {
                    bytes.add(*byte_offset).cast()
                };
            }
            if !length.is_null() {
                *length = if detached { 0 } else { *view_length };
            }
            NAPI_OK
        }
        _ => NAPI_INVALID_ARG,
    };
    record_status(env, status)
}

#[no_mangle]
pub unsafe extern "C" fn napi_is_buffer(
    env: NapiEnv,
    value: NapiValue,
    out: *mut bool,
) -> NapiStatus {
    if out.is_null() || !value_belongs_to_environment(env, value) {
        return record_status(env, NAPI_INVALID_ARG);
    }
    *out = matches!(
        value_ref(value),
        Ok(Value::Buffer(_) | Value::ExternalBuffer { .. } | Value::BufferView { .. })
    );
    NAPI_OK
}

unsafe fn arraybuffer_parts(value: NapiValue) -> Result<(*mut u8, usize, bool), NapiStatus> {
    match value.as_mut() {
        Some(Value::ArrayBuffer { bytes, detached }) => {
            Ok((bytes.as_mut_ptr(), bytes.len(), *detached))
        }
        Some(Value::SharedArrayBuffer(bytes)) => Ok((bytes.as_mut_ptr(), bytes.len(), false)),
        Some(Value::ExternalSharedArrayBuffer { data, length }) => Ok((*data, *length, false)),
        Some(Value::ExternalArrayBuffer {
            data,
            length,
            detached,
        }) => Ok((*data, *length, *detached)),
        _ => Err(NAPI_ARRAYBUFFER_EXPECTED),
    }
}

fn typedarray_element_size(array_type: i32) -> Option<usize> {
    match array_type {
        0..=2 => Some(1),
        3 | 4 => Some(2),
        5..=7 => Some(4),
        8..=10 => Some(8),
        _ => None,
    }
}

#[no_mangle]
pub unsafe extern "C" fn napi_create_arraybuffer(
    env: NapiEnv,
    length: usize,
    data: *mut *mut c_void,
    out: *mut NapiValue,
) -> NapiStatus {
    let Ok(env) = env_for_value_output(env, out) else {
        return NAPI_INVALID_ARG;
    };
    let value = env.alloc(Value::ArrayBuffer {
        bytes: vec![0; length],
        detached: false,
    });
    if !data.is_null() {
        let Some(Value::ArrayBuffer { bytes, .. }) = value.as_mut() else {
            unreachable!();
        };
        *data = bytes.as_mut_ptr().cast();
    }
    write_value(out, value)
}

#[no_mangle]
pub unsafe extern "C" fn napi_create_external_arraybuffer(
    env: NapiEnv,
    data: *mut c_void,
    length: usize,
    finalize: Option<NapiFinalize>,
    hint: *mut c_void,
    out: *mut NapiValue,
) -> NapiStatus {
    if out.is_null() || (length != 0 && data.is_null()) {
        return record_status(env, NAPI_INVALID_ARG);
    }
    let Ok(env) = env_mut(env) else {
        return NAPI_INVALID_ARG;
    };
    let value = env.alloc(Value::ExternalArrayBuffer {
        data: data.cast(),
        length,
        detached: false,
    });
    if finalize.is_some() {
        env.finalizers.push(FinalizeRecord {
            data,
            finalize,
            hint,
        });
    }
    write_value(out, value)
}

#[no_mangle]
pub unsafe extern "C" fn node_api_create_external_sharedarraybuffer(
    env: NapiEnv,
    data: *mut c_void,
    length: usize,
    finalize: Option<NodeApiNoEnvFinalize>,
    hint: *mut c_void,
    out: *mut NapiValue,
) -> NapiStatus {
    if out.is_null() || (length != 0 && data.is_null()) {
        return record_status(env, NAPI_INVALID_ARG);
    }
    let Ok(env) = env_mut(env) else {
        return NAPI_INVALID_ARG;
    };
    let value = env.alloc(Value::ExternalSharedArrayBuffer {
        data: data.cast(),
        length,
    });
    if finalize.is_some() {
        env.noenv_finalizers.push(NoEnvFinalizeRecord {
            data,
            finalize,
            hint,
        });
    }
    write_value(out, value)
}

#[no_mangle]
pub unsafe extern "C" fn node_api_create_sharedarraybuffer(
    env: NapiEnv,
    length: usize,
    data: *mut *mut c_void,
    out: *mut NapiValue,
) -> NapiStatus {
    if out.is_null() {
        return record_status(env, NAPI_INVALID_ARG);
    }
    let Ok(env) = env_mut(env) else {
        return NAPI_INVALID_ARG;
    };
    let value = env.alloc(Value::SharedArrayBuffer(vec![0; length]));
    if let Some(data) = data.as_mut() {
        let Some(Value::SharedArrayBuffer(bytes)) = value.as_mut() else {
            unreachable!();
        };
        *data = bytes.as_mut_ptr().cast();
    }
    write_value(out, value)
}

#[no_mangle]
pub unsafe extern "C" fn node_api_is_sharedarraybuffer(
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
    *result = matches!(
        value_ref(value),
        Ok(Value::SharedArrayBuffer(_) | Value::ExternalSharedArrayBuffer { .. })
    );
    NAPI_OK
}

#[no_mangle]
pub unsafe extern "C" fn napi_adjust_external_memory(
    env: NapiEnv,
    change_in_bytes: i64,
    adjusted_value: *mut i64,
) -> NapiStatus {
    if adjusted_value.is_null() {
        return record_status(env, NAPI_INVALID_ARG);
    }
    let env_ptr = env;
    let Ok(env) = env_mut(env) else {
        return NAPI_INVALID_ARG;
    };
    let Some(adjusted) = env.external_memory.checked_add(change_in_bytes) else {
        return record_status(env_ptr, NAPI_GENERIC_FAILURE);
    };
    env.external_memory = adjusted;
    *adjusted_value = adjusted;
    NAPI_OK
}

#[no_mangle]
pub unsafe extern "C" fn napi_is_arraybuffer(
    env: NapiEnv,
    value: NapiValue,
    out: *mut bool,
) -> NapiStatus {
    if out.is_null() || !value_belongs_to_environment(env, value) {
        return record_status(env, NAPI_INVALID_ARG);
    }
    *out = matches!(
        value_ref(value),
        Ok(Value::ArrayBuffer { .. } | Value::ExternalArrayBuffer { .. })
    );
    NAPI_OK
}

#[no_mangle]
pub unsafe extern "C" fn napi_get_arraybuffer_info(
    env: NapiEnv,
    value: NapiValue,
    data: *mut *mut c_void,
    length: *mut usize,
) -> NapiStatus {
    if !value_belongs_to_environment(env, value) {
        return NAPI_INVALID_ARG;
    }
    let Ok((bytes, byte_length, detached)) = arraybuffer_parts(value) else {
        return record_status(env, NAPI_ARRAYBUFFER_EXPECTED);
    };
    if !data.is_null() {
        *data = if detached {
            ptr::null_mut()
        } else {
            bytes.cast()
        };
    }
    if !length.is_null() {
        *length = if detached { 0 } else { byte_length };
    }
    NAPI_OK
}

#[no_mangle]
pub unsafe extern "C" fn napi_detach_arraybuffer(env: NapiEnv, value: NapiValue) -> NapiStatus {
    if !value_belongs_to_environment(env, value) {
        return NAPI_INVALID_ARG;
    }
    let status = match value.as_mut() {
        Some(Value::ArrayBuffer { detached, .. })
        | Some(Value::ExternalArrayBuffer { detached, .. }) => {
            *detached = true;
            NAPI_OK
        }
        _ => NAPI_ARRAYBUFFER_EXPECTED,
    };
    record_status(env, status)
}

#[no_mangle]
pub unsafe extern "C" fn napi_is_detached_arraybuffer(
    env: NapiEnv,
    value: NapiValue,
    out: *mut bool,
) -> NapiStatus {
    if out.is_null() || !value_belongs_to_environment(env, value) {
        return record_status(env, NAPI_INVALID_ARG);
    }
    *out = match value_ref(value) {
        Ok(Value::ArrayBuffer { detached, .. })
        | Ok(Value::ExternalArrayBuffer { detached, .. }) => *detached,
        _ => return record_status(env, NAPI_ARRAYBUFFER_EXPECTED),
    };
    NAPI_OK
}

#[no_mangle]
pub unsafe extern "C" fn napi_create_typedarray(
    env: NapiEnv,
    array_type: i32,
    length: usize,
    array_buffer: NapiValue,
    byte_offset: usize,
    out: *mut NapiValue,
) -> NapiStatus {
    if out.is_null() || !value_belongs_to_environment(env, array_buffer) {
        return record_status(env, NAPI_INVALID_ARG);
    }
    let Some(element_size) = typedarray_element_size(array_type) else {
        return record_status(env, NAPI_INVALID_ARG);
    };
    let Ok((_, buffer_length, detached)) = arraybuffer_parts(array_buffer) else {
        return record_status(env, NAPI_ARRAYBUFFER_EXPECTED);
    };
    if detached {
        return record_status(env, NAPI_INVALID_ARG);
    }
    let Some(byte_length) = length.checked_mul(element_size) else {
        return record_status(env, NAPI_INVALID_ARG);
    };
    let Some(end) = byte_offset.checked_add(byte_length) else {
        return record_status(env, NAPI_INVALID_ARG);
    };
    if !byte_offset.is_multiple_of(element_size) || end > buffer_length {
        return record_status(env, NAPI_INVALID_ARG);
    }
    let Ok(env) = env_mut(env) else {
        return NAPI_INVALID_ARG;
    };
    let value = env.alloc(Value::TypedArray {
        array_type,
        length,
        array_buffer,
        byte_offset,
    });
    write_value(out, value)
}

#[no_mangle]
pub unsafe extern "C" fn napi_is_typedarray(
    env: NapiEnv,
    value: NapiValue,
    out: *mut bool,
) -> NapiStatus {
    if out.is_null() || !value_belongs_to_environment(env, value) {
        return record_status(env, NAPI_INVALID_ARG);
    }
    *out = matches!(
        value_ref(value),
        Ok(Value::TypedArray { .. }
            | Value::Buffer(_)
            | Value::ExternalBuffer { .. }
            | Value::BufferView { .. })
    );
    NAPI_OK
}

#[no_mangle]
pub unsafe extern "C" fn napi_create_dataview(
    env: NapiEnv,
    length: usize,
    array_buffer: NapiValue,
    byte_offset: usize,
    out: *mut NapiValue,
) -> NapiStatus {
    if out.is_null() || !value_belongs_to_environment(env, array_buffer) {
        return record_status(env, NAPI_INVALID_ARG);
    }
    let Ok((_, buffer_length, detached)) = arraybuffer_parts(array_buffer) else {
        return record_status(env, NAPI_ARRAYBUFFER_EXPECTED);
    };
    if detached {
        return record_status(env, NAPI_INVALID_ARG);
    }
    let Some(end) = byte_offset.checked_add(length) else {
        return record_status(env, NAPI_INVALID_ARG);
    };
    if end > buffer_length {
        return record_status(env, NAPI_INVALID_ARG);
    }
    let Ok(env) = env_mut(env) else {
        return NAPI_INVALID_ARG;
    };
    let value = env.alloc(Value::DataView {
        length,
        array_buffer,
        byte_offset,
    });
    write_value(out, value)
}

#[no_mangle]
pub unsafe extern "C" fn napi_is_dataview(
    env: NapiEnv,
    value: NapiValue,
    out: *mut bool,
) -> NapiStatus {
    if out.is_null() || !value_belongs_to_environment(env, value) {
        return record_status(env, NAPI_INVALID_ARG);
    }
    *out = matches!(value_ref(value), Ok(Value::DataView { .. }));
    NAPI_OK
}

#[no_mangle]
pub unsafe extern "C" fn napi_get_dataview_info(
    env: NapiEnv,
    value: NapiValue,
    length: *mut usize,
    data: *mut *mut c_void,
    array_buffer: *mut NapiValue,
    byte_offset: *mut usize,
) -> NapiStatus {
    if !value_belongs_to_environment(env, value) {
        return NAPI_INVALID_ARG;
    }
    let Some(Value::DataView {
        length: view_length,
        array_buffer: backing,
        byte_offset: offset,
    }) = value.as_mut()
    else {
        return record_status(env, NAPI_INVALID_ARG);
    };
    let (view_length, backing, offset) = (*view_length, *backing, *offset);
    let Ok((bytes, _, detached)) = arraybuffer_parts(backing) else {
        return record_status(env, NAPI_ARRAYBUFFER_EXPECTED);
    };
    if !length.is_null() {
        *length = if detached { 0 } else { view_length };
    }
    if !data.is_null() {
        *data = if detached {
            ptr::null_mut()
        } else {
            bytes.add(offset).cast()
        };
    }
    if !array_buffer.is_null() {
        *array_buffer = backing;
    }
    if !byte_offset.is_null() {
        *byte_offset = offset;
    }
    NAPI_OK
}

#[no_mangle]
pub unsafe extern "C" fn napi_get_typedarray_info(
    env: NapiEnv,
    value: NapiValue,
    array_type: *mut i32,
    length: *mut usize,
    data: *mut *mut c_void,
    array_buffer: *mut NapiValue,
    byte_offset: *mut usize,
) -> NapiStatus {
    if !value_belongs_to_environment(env, value) {
        return NAPI_INVALID_ARG;
    }
    match value.as_mut() {
        Some(Value::TypedArray {
            array_type: kind,
            length: view_length,
            array_buffer: backing,
            byte_offset: offset,
        }) => {
            let (kind, view_length, backing, offset) = (*kind, *view_length, *backing, *offset);
            let Ok((bytes, _, detached)) = arraybuffer_parts(backing) else {
                return record_status(env, NAPI_ARRAYBUFFER_EXPECTED);
            };
            if !array_type.is_null() {
                *array_type = kind;
            }
            if !length.is_null() {
                *length = if detached { 0 } else { view_length };
            }
            if !data.is_null() {
                *data = if detached {
                    ptr::null_mut()
                } else {
                    bytes.add(offset).cast()
                };
            }
            if !array_buffer.is_null() {
                *array_buffer = backing;
            }
            if !byte_offset.is_null() {
                *byte_offset = offset;
            }
            NAPI_OK
        }
        Some(Value::Buffer(bytes)) => {
            let buffer_length = bytes.len();
            if !array_type.is_null() {
                *array_type = 1;
            }
            if !length.is_null() {
                *length = buffer_length;
            }
            if !data.is_null() {
                *data = bytes.as_mut_ptr().cast();
            }
            if !array_buffer.is_null() {
                *array_buffer = value;
            }
            if !byte_offset.is_null() {
                *byte_offset = 0;
            }
            NAPI_OK
        }
        Some(Value::ExternalBuffer {
            data: external,
            length: buffer_length,
        }) => {
            if !array_type.is_null() {
                *array_type = 1;
            }
            if !length.is_null() {
                *length = *buffer_length;
            }
            if !data.is_null() {
                *data = external.cast();
            }
            if !array_buffer.is_null() {
                *array_buffer = value;
            }
            if !byte_offset.is_null() {
                *byte_offset = 0;
            }
            NAPI_OK
        }
        Some(Value::BufferView {
            array_buffer: backing,
            byte_offset: offset,
            length: view_length,
        }) => {
            let (bytes, _, detached) = match arraybuffer_parts(*backing) {
                Ok(parts) => parts,
                Err(status) => return record_status(env, status),
            };
            if !array_type.is_null() {
                *array_type = 1;
            }
            if !length.is_null() {
                *length = if detached { 0 } else { *view_length };
            }
            if !data.is_null() {
                *data = if detached {
                    ptr::null_mut()
                } else {
                    bytes.add(*offset).cast()
                };
            }
            if !array_buffer.is_null() {
                *array_buffer = *backing;
            }
            if !byte_offset.is_null() {
                *byte_offset = *offset;
            }
            NAPI_OK
        }
        _ => record_status(env, NAPI_INVALID_ARG),
    }
}

