#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn number_array_map(value: f64, operation: impl Fn(f64, f64) -> f64) -> f64 {
    let (Some(allocate), Some((array, length))) =
        (ARENA_ALLOC.with(Cell::get), unsafe { array_data(value) })
    else {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return 0.0;
    };
    let Some(size) = length.checked_mul(8).and_then(|size| size.checked_add(8)) else {
        CALL_ERROR.with(|error| error.set(ALLOCATION_FAILED.as_ptr().cast()));
        return 0.0;
    };
    let output = unsafe { allocate(size, 8) };
    if output.is_null() {
        CALL_ERROR.with(|error| error.set(ALLOCATION_FAILED.as_ptr().cast()));
        return 0.0;
    }
    unsafe { output.cast::<u64>().write(length as u64) };
    for index in 0..length {
        let element = unsafe { array.add(8 + index * 8).cast::<f64>().read() };
        let mapped = operation(element, index as f64);
        unsafe { output.add(8 + index * 8).cast::<f64>().write(mapped) };
    }
    array_result(output)
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
type JitCallback = extern "C" fn(*const f64) -> f64;

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn compile_jit_callback(callback: f64) -> Option<(JitCallback, usize)> {
    let Some(callback) = (unsafe { string_argument(callback) }) else {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return None;
    };
    let symbol = format!("expr:{callback}:array-callback");
    let Some(program) = NumericProgram::parse(&symbol) else {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return None;
    };
    let required_args = program.required_args();
    if required_args > 16 {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return None;
    }
    let code = match compile(&symbol, &program) {
        Ok((code, _)) => code,
        Err(error) => {
            CALL_ERROR.with(|slot| slot.set(error));
            return None;
        }
    };
    Some((
        unsafe { std::mem::transmute::<*mut libc::c_void, JitCallback>(code) },
        required_args,
    ))
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn capture_arguments(value: f64, builtins: usize, required: usize) -> Option<Vec<f64>> {
    let Some((array, length)) = (unsafe { array_data(value) }) else {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return None;
    };
    if builtins + length > 16 || required > builtins + length {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return None;
    }
    Some(
        (0..length)
            .map(|index| unsafe { array.add(8 + index * 8).cast::<f64>().read_unaligned() })
            .collect(),
    )
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn call_jit_callback(callback: JitCallback, builtins: &[f64], captures: &[f64]) -> f64 {
    let mut arguments = [0.0; 16];
    arguments[..builtins.len()].copy_from_slice(builtins);
    arguments[builtins.len()..builtins.len() + captures.len()].copy_from_slice(captures);
    callback(arguments.as_ptr())
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn primitive_array_jit_map_impl(
    value: f64,
    callback: f64,
    encoded: f64,
    captures: Option<f64>,
) -> f64 {
    let encoded = encoded as u8;
    let source = encoded / 4;
    let target = encoded % 4;
    if source > 2 || target > 2 {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return 0.0;
    }
    let Some((callback, required)) = compile_jit_callback(callback) else {
        return 0.0;
    };
    let captures = if let Some(captures) = captures {
        let Some(captures) = capture_arguments(captures, 3, required) else {
            return 0.0;
        };
        captures
    } else {
        if required > 3 {
            CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
            return 0.0;
        }
        Vec::new()
    };
    let (Some(allocate), Some((array, length))) =
        (ARENA_ALLOC.with(Cell::get), unsafe { array_data(value) })
    else {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return 0.0;
    };
    let Some(size) = length.checked_mul(8).and_then(|size| size.checked_add(8)) else {
        CALL_ERROR.with(|error| error.set(ALLOCATION_FAILED.as_ptr().cast()));
        return 0.0;
    };
    let output = unsafe { allocate(size, 8) };
    if output.is_null() {
        CALL_ERROR.with(|error| error.set(ALLOCATION_FAILED.as_ptr().cast()));
        return 0.0;
    }
    unsafe { output.cast::<u64>().write(length as u64) };
    for index in 0..length {
        let element = unsafe { array_element(array, index, source) };
        let mapped = call_jit_callback(callback, &[element, index as f64, value], &captures);
        let mapped = if target == 1 {
            u64::from(mapped != 0.0 && !mapped.is_nan())
        } else {
            mapped.to_bits()
        };
        unsafe {
            output
                .add(8 + index * 8)
                .cast::<u64>()
                .write_unaligned(mapped)
        };
    }
    array_result(output)
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn primitive_array_jit_map(value: f64, callback: f64, encoded: f64) -> f64 {
    primitive_array_jit_map_impl(value, callback, encoded, None)
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn primitive_array_jit_map_captured(
    value: f64,
    callback: f64,
    captures: f64,
    encoded: f64,
) -> f64 {
    primitive_array_jit_map_impl(value, callback, encoded, Some(captures))
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn dynamic_array_jit_map_impl(
    value: f64,
    callback: f64,
    target: f64,
    captures: Option<f64>,
) -> f64 {
    let target = target as u8;
    let Some(array) = dynamic_primitive(value, None) else {
        CALL_ERROR.with(|error| error.set(INVALID_DYNAMIC_VALUE.as_ptr().cast()));
        return 0.0;
    };
    let (source, element_tag) = match array.tag {
        DYNAMIC_NUMBER_ARRAY_TAG => (0, DYNAMIC_NUMBER_TAG),
        DYNAMIC_BOOLEAN_ARRAY_TAG => (1, DYNAMIC_BOOLEAN_TAG),
        DYNAMIC_STRING_ARRAY_TAG => (2, DYNAMIC_STRING_TAG),
        _ => {
            CALL_ERROR.with(|error| error.set(INVALID_DYNAMIC_VALUE.as_ptr().cast()));
            return 0.0;
        }
    };
    if target > 2 {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return 0.0;
    }
    let Some((callback, required)) = compile_jit_callback(callback) else {
        return 0.0;
    };
    let captures = if let Some(captures) = captures {
        let Some(captures) = capture_arguments(captures, 5, required) else {
            return 0.0;
        };
        captures
    } else {
        if required > 5 {
            CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
            return 0.0;
        }
        Vec::new()
    };
    let (Some(allocate), Some((data, length))) = (ARENA_ALLOC.with(Cell::get), unsafe {
        array_data(f64::from_bits(array.payload))
    }) else {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return 0.0;
    };
    let Some(size) = length.checked_mul(8).and_then(|size| size.checked_add(8)) else {
        CALL_ERROR.with(|error| error.set(ALLOCATION_FAILED.as_ptr().cast()));
        return 0.0;
    };
    let output = unsafe { allocate(size, 8) };
    if output.is_null() {
        CALL_ERROR.with(|error| error.set(ALLOCATION_FAILED.as_ptr().cast()));
        return 0.0;
    }
    unsafe { output.cast::<u64>().write(length as u64) };
    for index in 0..length {
        let element = unsafe { array_element(data, index, source) };
        let mapped = call_jit_callback(
            callback,
            &[
                element_tag as f64,
                element,
                index as f64,
                array.tag as f64,
                f64::from_bits(array.payload),
            ],
            &captures,
        );
        let mapped = if target == 1 {
            u64::from(mapped != 0.0 && !mapped.is_nan())
        } else {
            mapped.to_bits()
        };
        unsafe {
            output
                .add(8 + index * 8)
                .cast::<u64>()
                .write_unaligned(mapped)
        };
    }
    array_result(output)
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn dynamic_array_jit_map(value: f64, callback: f64, target: f64) -> f64 {
    dynamic_array_jit_map_impl(value, callback, target, None)
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn dynamic_array_jit_map_captured(
    value: f64,
    callback: f64,
    captures: f64,
    target: f64,
) -> f64 {
    dynamic_array_jit_map_impl(value, callback, target, Some(captures))
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn dynamic_array_jit_scan_impl(value: f64, callback: f64, mode: f64, captures: Option<f64>) -> f64 {
    let mode = mode as u8;
    let Some(array) = dynamic_primitive(value, None) else {
        CALL_ERROR.with(|error| error.set(INVALID_DYNAMIC_VALUE.as_ptr().cast()));
        return 0.0;
    };
    let (kind, element_tag) = match array.tag {
        DYNAMIC_NUMBER_ARRAY_TAG => (0, DYNAMIC_NUMBER_TAG),
        DYNAMIC_BOOLEAN_ARRAY_TAG => (1, DYNAMIC_BOOLEAN_TAG),
        DYNAMIC_STRING_ARRAY_TAG => (2, DYNAMIC_STRING_TAG),
        _ => {
            CALL_ERROR.with(|error| error.set(INVALID_DYNAMIC_VALUE.as_ptr().cast()));
            return 0.0;
        }
    };
    if mode > 6 {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return 0.0;
    }
    let Some((callback, required)) = compile_jit_callback(callback) else {
        return 0.0;
    };
    let captures = if let Some(captures) = captures {
        let Some(captures) = capture_arguments(captures, 5, required) else {
            return 0.0;
        };
        captures
    } else {
        if required > 5 {
            CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
            return 0.0;
        }
        Vec::new()
    };
    let result = primitive_array_scan(f64::from_bits(array.payload), kind, mode, |data, index| {
        let element = unsafe { array_element(data, index, kind) };
        let result = call_jit_callback(
            callback,
            &[
                element_tag as f64,
                element,
                index as f64,
                array.tag as f64,
                f64::from_bits(array.payload),
            ],
            &captures,
        );
        result != 0.0 && !result.is_nan()
    });
    dynamic_array_scan_result(array.tag, mode, result)
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn dynamic_array_jit_scan(value: f64, callback: f64, mode: f64) -> f64 {
    dynamic_array_jit_scan_impl(value, callback, mode, None)
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn dynamic_array_jit_scan_captured(
    value: f64,
    callback: f64,
    captures: f64,
    mode: f64,
) -> f64 {
    dynamic_array_jit_scan_impl(value, callback, mode, Some(captures))
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn primitive_array_jit_scan(value: f64, callback: f64, encoded: f64) -> f64 {
    let encoded = encoded as u8;
    let kind = encoded / 8;
    let mode = encoded % 8;
    if kind > 2 || mode > 6 {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return 0.0;
    }
    let Some((callback, required)) = compile_jit_callback(callback) else {
        return 0.0;
    };
    if required > 3 {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return 0.0;
    }
    primitive_array_scan(value, kind, mode, |array, index| {
        let element = unsafe { array_element(array, index, kind) };
        let result = callback([element, index as f64, value].as_ptr());
        result != 0.0 && !result.is_nan()
    })
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn primitive_array_jit_scan_captured(
    value: f64,
    callback: f64,
    captures: f64,
    encoded: f64,
) -> f64 {
    let encoded = encoded as u8;
    let kind = encoded / 8;
    let mode = encoded % 8;
    if kind > 2 || mode > 6 {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return 0.0;
    }
    let Some((callback, required)) = compile_jit_callback(callback) else {
        return 0.0;
    };
    let Some(captures) = capture_arguments(captures, 3, required) else {
        return 0.0;
    };
    primitive_array_scan(value, kind, mode, |array, index| {
        let element = unsafe { array_element(array, index, kind) };
        let result = call_jit_callback(callback, &[element, index as f64, value], &captures);
        result != 0.0 && !result.is_nan()
    })
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn number_array_jit_reduce(
    value: f64,
    initial: f64,
    callback: f64,
    from_right: bool,
    has_initial: bool,
    captures: Option<f64>,
) -> f64 {
    let Some((callback, required)) = compile_jit_callback(callback) else {
        return 0.0;
    };
    let captures = if let Some(captures) = captures {
        let Some(captures) = capture_arguments(captures, 4, required) else {
            return 0.0;
        };
        captures
    } else {
        if required > 4 {
            CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
            return 0.0;
        }
        Vec::new()
    };
    let Some((array, length)) = (unsafe { array_data(value) }) else {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return 0.0;
    };
    if !has_initial && length == 0 {
        CALL_ERROR.with(|error| error.set(EMPTY_REDUCE.as_ptr().cast()));
        return 0.0;
    }
    let mut accumulator = if has_initial {
        initial
    } else {
        let index = if from_right { length - 1 } else { 0 };
        unsafe { array.add(8 + index * 8).cast::<f64>().read_unaligned() }
    };
    let mut apply = |index| {
        let element = unsafe { array.add(8 + index * 8).cast::<f64>().read_unaligned() };
        accumulator = call_jit_callback(
            callback,
            &[accumulator, element, index as f64, value],
            &captures,
        );
    };
    if from_right {
        for index in (0..if has_initial { length } else { length - 1 }).rev() {
            apply(index);
        }
    } else {
        for index in if has_initial { 0 } else { 1 }..length {
            apply(index);
        }
    }
    accumulator
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
macro_rules! jit_reduce_fn {
    ($name:ident, $from_right:expr, $has_initial:expr) => {
        extern "C" fn $name(value: f64, initial: f64, callback: f64) -> f64 {
            number_array_jit_reduce(value, initial, callback, $from_right, $has_initial, None)
        }
    };
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
jit_reduce_fn!(number_array_jit_reduce_initial, false, true);
#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
jit_reduce_fn!(number_array_jit_reduce_first, false, false);
#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
jit_reduce_fn!(number_array_jit_reduce_right_initial, true, true);
#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
jit_reduce_fn!(number_array_jit_reduce_right_last, true, false);

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
macro_rules! jit_reduce_captured_fn {
    ($name:ident, $from_right:expr, $has_initial:expr) => {
        extern "C" fn $name(value: f64, initial: f64, callback: f64, captures: f64) -> f64 {
            number_array_jit_reduce(
                value,
                initial,
                callback,
                $from_right,
                $has_initial,
                Some(captures),
            )
        }
    };
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
jit_reduce_captured_fn!(number_array_jit_reduce_initial_captured, false, true);
#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
jit_reduce_captured_fn!(number_array_jit_reduce_first_captured, false, false);
#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
jit_reduce_captured_fn!(number_array_jit_reduce_right_initial_captured, true, true);
#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
jit_reduce_captured_fn!(number_array_jit_reduce_right_last_captured, true, false);

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn dynamic_array_jit_reduce(
    value: f64,
    initial: f64,
    callback: f64,
    from_right: bool,
    captures: Option<f64>,
) -> f64 {
    let Some(array) = dynamic_primitive(value, None) else {
        CALL_ERROR.with(|error| error.set(INVALID_DYNAMIC_VALUE.as_ptr().cast()));
        return 0.0;
    };
    let (kind, element_tag) = match array.tag {
        DYNAMIC_NUMBER_ARRAY_TAG => (0, DYNAMIC_NUMBER_TAG),
        DYNAMIC_BOOLEAN_ARRAY_TAG => (1, DYNAMIC_BOOLEAN_TAG),
        DYNAMIC_STRING_ARRAY_TAG => (2, DYNAMIC_STRING_TAG),
        _ => {
            CALL_ERROR.with(|error| error.set(INVALID_DYNAMIC_VALUE.as_ptr().cast()));
            return 0.0;
        }
    };
    let Some((callback, required)) = compile_jit_callback(callback) else {
        return 0.0;
    };
    let captures = if let Some(captures) = captures {
        let Some(captures) = capture_arguments(captures, 6, required) else {
            return 0.0;
        };
        captures
    } else {
        if required > 6 {
            CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
            return 0.0;
        }
        Vec::new()
    };
    let Some((data, length)) = (unsafe { array_data(f64::from_bits(array.payload)) }) else {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return 0.0;
    };
    let mut accumulator = initial;
    let mut apply = |index| {
        let element = unsafe { array_element(data, index, kind) };
        accumulator = call_jit_callback(
            callback,
            &[
                accumulator,
                element_tag as f64,
                element,
                index as f64,
                array.tag as f64,
                f64::from_bits(array.payload),
            ],
            &captures,
        );
    };
    if from_right {
        for index in (0..length).rev() {
            apply(index);
        }
    } else {
        for index in 0..length {
            apply(index);
        }
    }
    accumulator
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn dynamic_array_jit_reduce_unseeded(
    value: f64,
    callback: f64,
    from_right: bool,
    captures: Option<f64>,
) -> f64 {
    let Some(array) = dynamic_primitive(value, None) else {
        CALL_ERROR.with(|error| error.set(INVALID_DYNAMIC_VALUE.as_ptr().cast()));
        return 0.0;
    };
    let (kind, element_tag) = match array.tag {
        DYNAMIC_NUMBER_ARRAY_TAG => (0, DYNAMIC_NUMBER_TAG),
        DYNAMIC_BOOLEAN_ARRAY_TAG => (1, DYNAMIC_BOOLEAN_TAG),
        DYNAMIC_STRING_ARRAY_TAG => (2, DYNAMIC_STRING_TAG),
        _ => {
            CALL_ERROR.with(|error| error.set(INVALID_DYNAMIC_VALUE.as_ptr().cast()));
            return 0.0;
        }
    };
    let Some((callback, required)) = compile_jit_callback(callback) else {
        return 0.0;
    };
    let captures = if let Some(captures) = captures {
        let Some(captures) = capture_arguments(captures, 7, required) else {
            return 0.0;
        };
        captures
    } else {
        if required > 7 {
            CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
            return 0.0;
        }
        Vec::new()
    };
    let Some((data, length)) = (unsafe { array_data(f64::from_bits(array.payload)) }) else {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return 0.0;
    };
    if length == 0 {
        CALL_ERROR.with(|error| error.set(EMPTY_REDUCE.as_ptr().cast()));
        return 0.0;
    }
    let first = if from_right { length - 1 } else { 0 };
    let first_payload = unsafe { array_element(data, first, kind) };
    let mut accumulator = DynamicPrimitive {
        tag: element_tag,
        payload: if element_tag == DYNAMIC_BOOLEAN_TAG {
            u64::from(first_payload != 0.0)
        } else {
            first_payload.to_bits()
        },
    };
    let mut apply = |index| -> Option<()> {
        let element = unsafe { array_element(data, index, kind) };
        let result = call_jit_callback(
            callback,
            &[
                accumulator.tag as f64,
                f64::from_bits(accumulator.payload),
                element_tag as f64,
                element,
                index as f64,
                array.tag as f64,
                f64::from_bits(array.payload),
            ],
            &captures,
        );
        let result = dynamic_primitive(result, None)?;
        accumulator = DynamicPrimitive {
            tag: result.tag,
            payload: result.payload,
        };
        Some(())
    };
    if from_right {
        for index in (0..length - 1).rev() {
            if apply(index).is_none() {
                CALL_ERROR.with(|error| error.set(INVALID_DYNAMIC_VALUE.as_ptr().cast()));
                return 0.0;
            }
        }
    } else {
        for index in 1..length {
            if apply(index).is_none() {
                CALL_ERROR.with(|error| error.set(INVALID_DYNAMIC_VALUE.as_ptr().cast()));
                return 0.0;
            }
        }
    }
    arena_dynamic(accumulator.tag, accumulator.payload)
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn dynamic_array_jit_reduce_left(value: f64, initial: f64, callback: f64) -> f64 {
    dynamic_array_jit_reduce(value, initial, callback, false, None)
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn dynamic_array_jit_reduce_right(value: f64, initial: f64, callback: f64) -> f64 {
    dynamic_array_jit_reduce(value, initial, callback, true, None)
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn dynamic_array_jit_reduce_left_captured(
    value: f64,
    initial: f64,
    callback: f64,
    captures: f64,
) -> f64 {
    dynamic_array_jit_reduce(value, initial, callback, false, Some(captures))
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn dynamic_array_jit_reduce_right_captured(
    value: f64,
    initial: f64,
    callback: f64,
    captures: f64,
) -> f64 {
    dynamic_array_jit_reduce(value, initial, callback, true, Some(captures))
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn dynamic_array_jit_reduce_left_unseeded(
    value: f64,
    _initial: f64,
    callback: f64,
) -> f64 {
    dynamic_array_jit_reduce_unseeded(value, callback, false, None)
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn dynamic_array_jit_reduce_right_unseeded(
    value: f64,
    _initial: f64,
    callback: f64,
) -> f64 {
    dynamic_array_jit_reduce_unseeded(value, callback, true, None)
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn dynamic_array_jit_reduce_left_unseeded_captured(
    value: f64,
    _initial: f64,
    callback: f64,
    captures: f64,
) -> f64 {
    dynamic_array_jit_reduce_unseeded(value, callback, false, Some(captures))
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn dynamic_array_jit_reduce_right_unseeded_captured(
    value: f64,
    _initial: f64,
    callback: f64,
    captures: f64,
) -> f64 {
    dynamic_array_jit_reduce_unseeded(value, callback, true, Some(captures))
}

