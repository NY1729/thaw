#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn dynamic_array_result(tag: u64, result: f64) -> f64 {
    if !CALL_ERROR.with(Cell::get).is_null() || result.to_bits() == 0 { return 0.0; }
    // Dense JIT results are tagged data pointers; a sparse map result is
    // already a native two-word {buffer, presence} handle.
    let handle = mutable_array_handle(result);
    if !CALL_ERROR.with(Cell::get).is_null() { return 0.0; }
    dynamic_from_parts(tag as f64, handle)
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn dynamic_array_slice(value: f64, start: f64, end: f64) -> f64 {
    dynamic_primitive(value, None).map_or_else(
        || {
            CALL_ERROR.with(|error| error.set(INVALID_DYNAMIC_VALUE.as_ptr().cast()));
            0.0
        },
        |dynamic| {
            if !matches!(
                dynamic.tag,
                DYNAMIC_NUMBER_ARRAY_TAG..=DYNAMIC_STRING_ARRAY_TAG
            ) {
                CALL_ERROR.with(|error| error.set(INVALID_DYNAMIC_VALUE.as_ptr().cast()));
                return 0.0;
            }
            let result = array_slice(f64::from_bits(dynamic.payload), start, end);
            dynamic_array_result(dynamic.tag, result)
        },
    )
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn array_slice(value: f64, start: f64, end: f64) -> f64 {
    let _source_root = array_callback_root(value);
    let (Some(slice), Some((data, length))) =
        (ARRAY_SLICE.with(Cell::get), unsafe { array_data(value) })
    else {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return 0.0;
    };
    let Some((_owned_presence, presence)) = array_native_presence(value, length) else {
        return 0.0;
    };
    let result = unsafe { slice(data, 8, start, end) };
    if result.is_null() {
        CALL_ERROR.with(|error| error.set(ALLOCATION_FAILED.as_ptr().cast()));
        0.0
    } else if presence.is_null() {
        array_result(result)
    } else {
        let _result_root = thaw_arena::ArenaRoot::new(result as usize);
        let Some(slice_presence) = ARRAY_PRESENCE_SLICE.with(Cell::get) else {
            CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
            return 0.0;
        };
        let sliced_presence = unsafe { slice_presence(presence, length, start, end) };
        if sliced_presence.is_null() {
            CALL_ERROR.with(|error| error.set(ALLOCATION_FAILED.as_ptr().cast()));
            0.0
        } else {
            array_result_with_presence(result, sliced_presence)
        }
    }
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn array_concat(left: f64, right: f64) -> f64 {
    let _left_root = array_callback_root(left);
    let _right_root = array_callback_root(right);
    let (Some(concat), Some((left_data, left_len)), Some((right_data, right_len))) = (
        ARRAY_CONCAT.with(Cell::get),
        unsafe { array_data(left) },
        unsafe { array_data(right) },
    ) else {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return 0.0;
    };
    let Some((_left_owned, left_presence)) = array_native_presence(left, left_len) else {
        return 0.0;
    };
    let Some((_right_owned, right_presence)) = array_native_presence(right, right_len) else {
        return 0.0;
    };
    let result = unsafe { concat(left_data, right_data, 8) };
    if result.is_null() {
        CALL_ERROR.with(|error| error.set(ALLOCATION_FAILED.as_ptr().cast()));
        0.0
    } else if left_presence.is_null() && right_presence.is_null() {
        array_result(result)
    } else {
        let _result_root = thaw_arena::ArenaRoot::new(result as usize);
        let Some(length) = left_len.checked_add(right_len) else {
            CALL_ERROR.with(|error| error.set(ALLOCATION_FAILED.as_ptr().cast()));
            return 0.0;
        };
        let Some(size) = length.checked_add(8) else {
            CALL_ERROR.with(|error| error.set(ALLOCATION_FAILED.as_ptr().cast()));
            return 0.0;
        };
        let Some(allocate) = ARENA_ALLOC.with(Cell::get) else {
            CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
            return 0.0;
        };
        let presence = unsafe { allocate(size, 8) };
        if presence.is_null() {
            CALL_ERROR.with(|error| error.set(ALLOCATION_FAILED.as_ptr().cast()));
            return 0.0;
        }
        unsafe {
            presence.cast::<u64>().write(length as u64);
            for (offset, source, source_len) in
                [(0, left_presence, left_len), (left_len, right_presence, right_len)]
            {
                let mask_len = if source.is_null() { 0 } else {
                    source.cast::<u64>().read_unaligned() as usize
                };
                for index in 0..source_len {
                    let state = if source.is_null() || index >= mask_len { 1 } else {
                        source.add(8 + index).read()
                    };
                    presence.add(8 + offset + index).write(state);
                }
            }
        }
        array_result_with_presence(result, presence)
    }
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn dynamic_array_concat(left: f64, right: f64) -> f64 {
    let (Some(left), Some(right)) = (
        dynamic_primitive(left, None),
        dynamic_primitive(right, None),
    ) else {
        CALL_ERROR.with(|error| error.set(INVALID_DYNAMIC_VALUE.as_ptr().cast()));
        return 0.0;
    };
    if left.tag != right.tag
        || !matches!(
            left.tag,
            DYNAMIC_NUMBER_ARRAY_TAG..=DYNAMIC_STRING_ARRAY_TAG
        )
    {
        CALL_ERROR.with(|error| error.set(INVALID_DYNAMIC_VALUE.as_ptr().cast()));
        return 0.0;
    }
    let result = array_concat(f64::from_bits(left.payload), f64::from_bits(right.payload));
    dynamic_array_result(left.tag, result)
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn array_append(operation: u8, array: f64, value: f64) -> f64 {
    let _source_root = array_callback_root(array);
    let (Some(append), Some((data, length))) =
        (ARRAY_APPEND.with(Cell::get), unsafe { array_data(array) })
    else {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return 0.0;
    };
    let Some((_owned_presence, source_presence)) = array_native_presence(array, length) else {
        return 0.0;
    };
    let result = unsafe { append(operation, data, value) };
    if result.is_null() {
        CALL_ERROR.with(|error| error.set(ALLOCATION_FAILED.as_ptr().cast()));
        0.0
    } else if source_presence.is_null() {
        array_result(result)
    } else {
        let _result_root = thaw_arena::ArenaRoot::new(result as usize);
        let Some(size) = length.checked_add(9) else {
            CALL_ERROR.with(|error| error.set(ALLOCATION_FAILED.as_ptr().cast()));
            return 0.0;
        };
        let Some(allocate) = ARENA_ALLOC.with(Cell::get) else {
            CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
            return 0.0;
        };
        let presence = unsafe { allocate(size, 8) };
        if presence.is_null() {
            CALL_ERROR.with(|error| error.set(ALLOCATION_FAILED.as_ptr().cast()));
            return 0.0;
        }
        unsafe {
            presence.cast::<u64>().write((length + 1) as u64);
            std::ptr::write_bytes(presence.add(8), 1, length + 1);
            let source_length = source_presence.cast::<u64>().read() as usize;
            std::ptr::copy_nonoverlapping(
                source_presence.add(8), presence.add(8), source_length.min(length),
            );
        }
        array_result_with_presence(result, presence)
    }
}

macro_rules! array_append_fn {
    ($name:ident, $operation:expr) => {
        #[cfg(all(target_arch = "x86_64", target_family = "unix"))]
        extern "C" fn $name(array: f64, value: f64) -> f64 {
            array_append($operation, array, value)
        }
    };
}

array_append_fn!(number_array_append, 0);
array_append_fn!(string_array_append, 1);
array_append_fn!(bool_array_append, 2);

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn dynamic_array_append(array: f64, value: f64) -> f64 {
    let (Some(array), Some(value)) = (
        dynamic_primitive(array, None),
        dynamic_primitive(value, None),
    ) else {
        CALL_ERROR.with(|error| error.set(INVALID_DYNAMIC_VALUE.as_ptr().cast()));
        return 0.0;
    };
    let operation = match (array.tag, value.tag) {
        (DYNAMIC_NUMBER_ARRAY_TAG, DYNAMIC_NUMBER_TAG) => 0,
        (DYNAMIC_STRING_ARRAY_TAG, DYNAMIC_STRING_TAG) => 1,
        (DYNAMIC_BOOLEAN_ARRAY_TAG, DYNAMIC_BOOLEAN_TAG) => 2,
        _ => {
            CALL_ERROR.with(|error| error.set(INVALID_DYNAMIC_VALUE.as_ptr().cast()));
            return 0.0;
        }
    };
    dynamic_array_result(
        array.tag,
        array_append(
            operation,
            f64::from_bits(array.payload),
            f64::from_bits(value.payload),
        ),
    )
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn array_to_reversed(value: f64) -> f64 {
    let _source_root = array_callback_root(value);
    let (Some(reverse), Some((data, length))) = (ARRAY_TO_REVERSED.with(Cell::get), unsafe {
        array_data(value)
    }) else {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return 0.0;
    };
    let Some((_owned_presence, source_presence)) = array_native_presence(value, length) else {
        return 0.0;
    };
    let result = unsafe { reverse(data, 8) };
    if result.is_null() {
        CALL_ERROR.with(|error| error.set(ALLOCATION_FAILED.as_ptr().cast()));
        0.0
    } else if source_presence.is_null() || unsafe {
        let mask_length = source_presence.cast::<u64>().read() as usize;
        (0..length.min(mask_length)).all(|index| source_presence.add(8 + index).read() == 1)
    } {
        array_result(result)
    } else {
        let _result_root = thaw_arena::ArenaRoot::new(result as usize);
        let Some(size) = length.checked_add(8) else {
            CALL_ERROR.with(|error| error.set(ALLOCATION_FAILED.as_ptr().cast()));
            return 0.0;
        };
        let Some(allocate) = ARENA_ALLOC.with(Cell::get) else {
            CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
            return 0.0;
        };
        let presence = unsafe { allocate(size, 8) };
        if presence.is_null() {
            CALL_ERROR.with(|error| error.set(ALLOCATION_FAILED.as_ptr().cast()));
            return 0.0;
        }
        unsafe {
            presence.cast::<u64>().write(length as u64);
            let source_length = source_presence.cast::<u64>().read_unaligned() as usize;
            for index in 0..length {
                let source = length - 1 - index;
                let state = if source < source_length {
                    source_presence.add(8 + source).read()
                } else { 1 };
                presence.add(8 + index).write(state);
            }
        }
        array_result_with_presence(result, presence)
    }
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn array_reverse(value: f64) -> f64 {
    let _source_root = array_callback_root(value);
    let (Some(reverse), Some((data, length))) =
        (ARRAY_REVERSE.with(Cell::get), unsafe { array_data(value) })
    else {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return 0.0;
    };
    let Some((_owned_presence, source_presence)) = array_native_presence(value, length) else {
        return 0.0;
    };
    // The installed membership callback certifies an external two-word
    // handle. A standalone one-word handle has no sidecar to update.
    let bits = value.to_bits();
    let handle = if bits & SPARSE_ARRAY_RESULT_TAG != 0 {
        (bits & !SPARSE_ARRAY_RESULT_TAG) as usize as *mut *mut u8
    } else if bits & ARRAY_RESULT_TAG == 0 && ARRAY_INDEX_PRESENT.with(Cell::get).is_some() {
        bits as usize as *mut *mut u8
    } else {
        std::ptr::null_mut()
    };
    let mut reversed_presence = std::ptr::null_mut();
    let mask_length = if source_presence.is_null() { 0 } else {
        unsafe { source_presence.cast::<u64>().read() as usize }
    };
    let needs_mask = !handle.is_null()
        && (0..length.min(mask_length)).any(|index| unsafe {
            source_presence.add(8 + index).read() != 1
        });
    if needs_mask {
        let Some(size) = length.checked_add(8) else {
            CALL_ERROR.with(|error| error.set(ALLOCATION_FAILED.as_ptr().cast()));
            return 0.0;
        };
        let Some(allocate) = ARENA_ALLOC.with(Cell::get) else {
            CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
            return 0.0;
        };
        reversed_presence = unsafe { allocate(size, 8) };
        if reversed_presence.is_null() {
            CALL_ERROR.with(|error| error.set(ALLOCATION_FAILED.as_ptr().cast()));
            return 0.0;
        }
        unsafe {
            reversed_presence.cast::<u64>().write(length as u64);
            for index in 0..length {
                let source_index = length - index - 1;
                let state = if source_index < mask_length {
                    source_presence.add(8 + source_index).read()
                } else { 1 };
                reversed_presence.add(8 + index).write(state);
            }
        }
    }
    let result = unsafe { reverse(data.cast_mut(), 8) };
    if result.is_null() {
        CALL_ERROR.with(|error| error.set(ALLOCATION_FAILED.as_ptr().cast()));
        0.0
    } else {
        if !handle.is_null() && !reversed_presence.is_null() {
            unsafe { handle.add(1).write(reversed_presence) };
        }
        value
    }
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn dynamic_array_reverse(value: f64, reverse: extern "C" fn(f64) -> f64) -> f64 {
    let Some(dynamic) = dynamic_primitive(value, None) else {
        CALL_ERROR.with(|error| error.set(INVALID_DYNAMIC_VALUE.as_ptr().cast()));
        return 0.0;
    };
    if !matches!(
        dynamic.tag,
        DYNAMIC_NUMBER_ARRAY_TAG..=DYNAMIC_STRING_ARRAY_TAG
    ) {
        CALL_ERROR.with(|error| error.set(INVALID_DYNAMIC_VALUE.as_ptr().cast()));
        return 0.0;
    }
    let result = reverse(f64::from_bits(dynamic.payload));
    dynamic_array_result(dynamic.tag, result)
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn dynamic_array_to_reversed(value: f64) -> f64 {
    dynamic_array_reverse(value, array_to_reversed)
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn dynamic_array_reverse_in_place(value: f64) -> f64 {
    dynamic_array_reverse(value, array_reverse)
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn array_to_sorted(operation: u8, value: f64) -> f64 {
    let _source_root = array_callback_root(value);
    let (Some(sort), Some((data, length))) = (ARRAY_TO_SORTED.with(Cell::get), unsafe {
        array_data(value)
    }) else {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return 0.0;
    };
    let Some((_owned_presence, presence)) = array_native_presence(value, length) else {
        return 0.0;
    };
    if !presence.is_null() && (0..length).any(|index| unsafe {
        index < presence.cast::<u64>().read() as usize && presence.add(8 + index).read() != 1
    }) {
        return array_sort_sparse(operation, value, data, length, presence, false);
    }
    let result = unsafe { sort(operation, data) };
    if result.is_null() {
        CALL_ERROR.with(|error| error.set(ALLOCATION_FAILED.as_ptr().cast()));
        0.0
    } else {
        array_result(result)
    }
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn array_sort_sparse(
    operation: u8, value: f64, data: *const u8, length: usize,
    presence: *const u8, in_place: bool,
) -> f64 {
    let (Some(sort), Some(allocate)) = (ARRAY_SORT.with(Cell::get), ARENA_ALLOC.with(Cell::get)) else {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return 0.0;
    };
    let Some(size) = length.checked_mul(8).and_then(|bytes| bytes.checked_add(8)) else {
        CALL_ERROR.with(|error| error.set(ALLOCATION_FAILED.as_ptr().cast()));
        return 0.0;
    };
    let sorted = unsafe { allocate(size, 8) };
    if sorted.is_null() {
        CALL_ERROR.with(|error| error.set(ALLOCATION_FAILED.as_ptr().cast()));
        return 0.0;
    }
    let _sorted_root = thaw_arena::ArenaRoot::new(sorted as usize);
    let mut present_count = 0;
    let mut undefined_count = 0;
    let mask_length = unsafe { presence.cast::<u64>().read() as usize };
    for index in 0..length {
        let state = if index < mask_length { unsafe { presence.add(8 + index).read() } } else { 1 };
        match state {
            1 => {
                unsafe { std::ptr::copy_nonoverlapping(data.add(8 + index * 8), sorted.add(8 + present_count * 8), 8) };
                present_count += 1;
            }
            2 => undefined_count += 1,
            0 => {},
            _ => {
                CALL_ERROR.with(|error| error.set(INVALID_DYNAMIC_VALUE.as_ptr().cast()));
                return 0.0;
            }
        }
    }
    unsafe { sorted.cast::<u64>().write(present_count as u64) };
    if unsafe { sort(operation, sorted) }.is_null() {
        CALL_ERROR.with(|error| error.set(ALLOCATION_FAILED.as_ptr().cast()));
        return 0.0;
    }
    unsafe {
        sorted.cast::<u64>().write(length as u64);
        std::ptr::write_bytes(sorted.add(8 + present_count * 8), 0, (length - present_count) * 8);
    }
    let Some(mask_size) = length.checked_add(8) else {
        CALL_ERROR.with(|error| error.set(ALLOCATION_FAILED.as_ptr().cast()));
        return 0.0;
    };
    let mask = unsafe { allocate(mask_size, 8) };
    if mask.is_null() {
        CALL_ERROR.with(|error| error.set(ALLOCATION_FAILED.as_ptr().cast()));
        return 0.0;
    }
    unsafe {
        mask.cast::<u64>().write(length as u64);
        std::ptr::write_bytes(mask.add(8), 1, present_count);
        std::ptr::write_bytes(mask.add(8 + present_count), 2, undefined_count);
        std::ptr::write_bytes(mask.add(8 + present_count + undefined_count), 0,
            length - present_count - undefined_count);
    }
    if !in_place { return array_result_with_presence(sorted, mask); }
    let bits = value.to_bits();
    let handle = if bits & SPARSE_ARRAY_RESULT_TAG != 0 {
        (bits & !SPARSE_ARRAY_RESULT_TAG) as usize as *mut *mut u8
    } else if bits & ARRAY_RESULT_TAG == 0 && ARRAY_INDEX_PRESENT.with(Cell::get).is_some() {
        bits as usize as *mut *mut u8
    } else {
        std::ptr::null_mut()
    };
    if handle.is_null() {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return 0.0;
    }
    unsafe {
        std::ptr::copy_nonoverlapping(sorted.add(8), data.cast_mut().add(8), length * 8);
        handle.add(1).write(mask);
    }
    value
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn number_array_to_sorted(value: f64) -> f64 {
    array_to_sorted(0, value)
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn string_array_to_sorted(value: f64) -> f64 {
    array_to_sorted(1, value)
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn bool_array_to_sorted(value: f64) -> f64 {
    array_to_sorted(2, value)
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn number_array_to_sorted_ascending(value: f64) -> f64 {
    array_to_sorted(3, value)
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn number_array_to_sorted_descending(value: f64) -> f64 {
    array_to_sorted(4, value)
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn string_array_to_sorted_descending(value: f64) -> f64 {
    array_to_sorted(5, value)
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn array_sort(operation: u8, value: f64) -> f64 {
    let _source_root = array_callback_root(value);
    let (Some(sort), Some((data, length))) = (ARRAY_SORT.with(Cell::get), unsafe { array_data(value) })
    else {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return 0.0;
    };
    let Some((_owned_presence, presence)) = array_native_presence(value, length) else {
        return 0.0;
    };
    if !presence.is_null() && (0..length).any(|index| unsafe {
        index < presence.cast::<u64>().read() as usize && presence.add(8 + index).read() != 1
    }) {
        return array_sort_sparse(operation, value, data, length, presence, true);
    }
    let result = unsafe { sort(operation, data.cast_mut()) };
    if result.is_null() {
        CALL_ERROR.with(|error| error.set(ALLOCATION_FAILED.as_ptr().cast()));
        0.0
    } else {
        if value.to_bits() & ARRAY_RESULT_TAG != 0 { array_result(result) } else { value }
    }
}

macro_rules! array_sort_fn {
    ($name:ident, $operation:expr) => {
        #[cfg(all(target_arch = "x86_64", target_family = "unix"))]
        extern "C" fn $name(value: f64) -> f64 {
            array_sort($operation, value)
        }
    };
}

array_sort_fn!(number_array_sort, 0);
array_sort_fn!(string_array_sort, 1);
array_sort_fn!(bool_array_sort, 2);
array_sort_fn!(number_array_sort_ascending, 3);
array_sort_fn!(number_array_sort_descending, 4);
array_sort_fn!(string_array_sort_descending, 5);

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn dynamic_array_sort(value: f64, in_place: bool) -> f64 {
    let Some(dynamic) = dynamic_primitive(value, None) else {
        CALL_ERROR.with(|error| error.set(INVALID_DYNAMIC_VALUE.as_ptr().cast()));
        return 0.0;
    };
    let operation = match dynamic.tag {
        DYNAMIC_NUMBER_ARRAY_TAG => 0,
        DYNAMIC_STRING_ARRAY_TAG => 1,
        DYNAMIC_BOOLEAN_ARRAY_TAG => 2,
        _ => {
            CALL_ERROR.with(|error| error.set(INVALID_DYNAMIC_VALUE.as_ptr().cast()));
            return 0.0;
        }
    };
    let value = f64::from_bits(dynamic.payload);
    let result = if in_place {
        array_sort(operation, value)
    } else {
        array_to_sorted(operation, value)
    };
    dynamic_array_result(dynamic.tag, result)
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn dynamic_array_to_sorted(value: f64) -> f64 {
    dynamic_array_sort(value, false)
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn dynamic_array_sort_in_place(value: f64) -> f64 {
    dynamic_array_sort(value, true)
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn array_fill(operation: u8, array: f64, value: f64, start: f64, end: f64) -> f64 {
    let _source_root = array_callback_root(array);
    let (Some(fill), Some((data, length))) = (ARRAY_FILL.with(Cell::get), unsafe { array_data(array) })
    else {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return 0.0;
    };
    let Some((_owned_presence, source_presence)) = array_native_presence(array, length) else {
        return 0.0;
    };
    let bits = array.to_bits();
    let handle = if bits & SPARSE_ARRAY_RESULT_TAG != 0 {
        (bits & !SPARSE_ARRAY_RESULT_TAG) as usize as *mut *mut u8
    } else if bits & ARRAY_RESULT_TAG == 0 && ARRAY_INDEX_PRESENT.with(Cell::get).is_some() {
        bits as usize as *mut *mut u8
    } else {
        std::ptr::null_mut()
    };
    let mut new_presence = std::ptr::null_mut();
    if !handle.is_null() && !source_presence.is_null() {
        let mask_length = unsafe { source_presence.cast::<u64>().read() as usize };
        let needs_mask = (0..length.min(mask_length))
            .any(|index| unsafe { source_presence.add(8 + index).read() != 1 });
        if needs_mask {
            let Some(size) = length.checked_add(8) else {
                CALL_ERROR.with(|error| error.set(ALLOCATION_FAILED.as_ptr().cast()));
                return 0.0;
            };
            let Some(allocate) = ARENA_ALLOC.with(Cell::get) else {
                CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
                return 0.0;
            };
            new_presence = unsafe { allocate(size, 8) };
            if new_presence.is_null() {
                CALL_ERROR.with(|error| error.set(ALLOCATION_FAILED.as_ptr().cast()));
                return 0.0;
            }
            unsafe {
                new_presence.cast::<u64>().write(length as u64);
                std::ptr::write_bytes(new_presence.add(8), 1, length);
                std::ptr::copy_nonoverlapping(
                    source_presence.add(8), new_presence.add(8), mask_length.min(length),
                );
            }
            let relative = |index: f64| -> usize {
                if index.is_nan() || index == f64::NEG_INFINITY { return 0; }
                if index == f64::INFINITY { return length; }
                let index = index.trunc();
                if index < 0.0 {
                    (length as f64 + index).max(0.0) as usize
                } else {
                    index.min(length as f64) as usize
                }
            };
            let first = relative(start);
            let last = relative(end);
            if first < last {
                unsafe { std::ptr::write_bytes(new_presence.add(8 + first), 1, last - first) };
            }
        }
    }
    let result = unsafe { fill(operation, data.cast_mut(), value, start, end) };
    if result.is_null() {
        CALL_ERROR.with(|error| error.set(ALLOCATION_FAILED.as_ptr().cast()));
        0.0
    } else {
        if !new_presence.is_null() {
            unsafe { handle.add(1).write(new_presence) };
        }
        if bits & ARRAY_RESULT_TAG != 0 { array_result(result) } else { array }
    }
}

macro_rules! array_fill_fn {
    ($name:ident, $operation:expr) => {
        #[cfg(all(target_arch = "x86_64", target_family = "unix"))]
        extern "C" fn $name(array: f64, value: f64, start: f64, end: f64) -> f64 {
            array_fill($operation, array, value, start, end)
        }
    };
}

array_fill_fn!(number_array_fill, 0);
array_fill_fn!(string_array_fill, 1);
array_fill_fn!(bool_array_fill, 2);

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn dynamic_array_fill(array: f64, value: f64, start: f64, end: f64) -> f64 {
    let (Some(array), Some(value)) = (
        dynamic_primitive(array, None),
        dynamic_primitive(value, None),
    ) else {
        CALL_ERROR.with(|error| error.set(INVALID_DYNAMIC_VALUE.as_ptr().cast()));
        return 0.0;
    };
    let operation = match (array.tag, value.tag) {
        (DYNAMIC_NUMBER_ARRAY_TAG, DYNAMIC_NUMBER_TAG) => 0,
        (DYNAMIC_STRING_ARRAY_TAG, DYNAMIC_STRING_TAG) => 1,
        (DYNAMIC_BOOLEAN_ARRAY_TAG, DYNAMIC_BOOLEAN_TAG) => 2,
        _ => {
            CALL_ERROR.with(|error| error.set(INVALID_DYNAMIC_VALUE.as_ptr().cast()));
            return 0.0;
        }
    };
    dynamic_array_result(
        array.tag,
        array_fill(
            operation,
            f64::from_bits(array.payload),
            f64::from_bits(value.payload),
            start,
            end,
        ),
    )
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn array_copy_within(array: f64, target: f64, start: f64, end: f64) -> f64 {
    let _source_root = array_callback_root(array);
    let (Some(copy), Some((data, length))) = (ARRAY_COPY_WITHIN.with(Cell::get), unsafe {
        array_data(array)
    }) else {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return 0.0;
    };
    let Some((_owned_presence, source_presence)) = array_native_presence(array, length) else {
        return 0.0;
    };
    let bits = array.to_bits();
    let handle = if bits & SPARSE_ARRAY_RESULT_TAG != 0 {
        (bits & !SPARSE_ARRAY_RESULT_TAG) as usize as *mut *mut u8
    } else if bits & ARRAY_RESULT_TAG == 0 && ARRAY_INDEX_PRESENT.with(Cell::get).is_some() {
        bits as usize as *mut *mut u8
    } else {
        std::ptr::null_mut()
    };
    let mut new_presence = std::ptr::null_mut();
    if !handle.is_null() && !source_presence.is_null() {
        let mask_length = unsafe { source_presence.cast::<u64>().read() as usize };
        let needs_mask = (0..length.min(mask_length))
            .any(|index| unsafe { source_presence.add(8 + index).read() != 1 });
        if needs_mask {
            let Some(size) = length.checked_add(8) else {
                CALL_ERROR.with(|error| error.set(ALLOCATION_FAILED.as_ptr().cast()));
                return 0.0;
            };
            let Some(allocate) = ARENA_ALLOC.with(Cell::get) else {
                CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
                return 0.0;
            };
            new_presence = unsafe { allocate(size, 8) };
            if new_presence.is_null() {
                CALL_ERROR.with(|error| error.set(ALLOCATION_FAILED.as_ptr().cast()));
                return 0.0;
            }
            unsafe {
                new_presence.cast::<u64>().write(length as u64);
                std::ptr::write_bytes(new_presence.add(8), 1, length);
                std::ptr::copy_nonoverlapping(
                    source_presence.add(8), new_presence.add(8), mask_length.min(length),
                );
            }
            let relative = |index: f64| -> usize {
                if index.is_nan() || index == f64::NEG_INFINITY { return 0; }
                if index == f64::INFINITY { return length; }
                let index = index.trunc();
                if index < 0.0 {
                    (length as f64 + index).max(0.0) as usize
                } else {
                    index.min(length as f64) as usize
                }
            };
            let target_index = relative(target);
            let source_index = relative(start);
            let count = relative(end).saturating_sub(source_index).min(length - target_index);
            if count != 0 {
                unsafe {
                    std::ptr::copy(
                        new_presence.add(8 + source_index),
                        new_presence.add(8 + target_index),
                        count,
                    );
                }
            }
        }
    }
    let result = unsafe { copy(data.cast_mut(), 8, target, start, end) };
    if result.is_null() {
        CALL_ERROR.with(|error| error.set(ALLOCATION_FAILED.as_ptr().cast()));
        0.0
    } else {
        if !new_presence.is_null() {
            unsafe { handle.add(1).write(new_presence) };
        }
        if bits & ARRAY_RESULT_TAG != 0 { array_result(result) } else { array }
    }
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn dynamic_array_copy_within(array: f64, target: f64, start: f64, end: f64) -> f64 {
    let Some(array) = dynamic_primitive(array, None) else {
        CALL_ERROR.with(|error| error.set(INVALID_DYNAMIC_VALUE.as_ptr().cast()));
        return 0.0;
    };
    if !matches!(
        array.tag,
        DYNAMIC_NUMBER_ARRAY_TAG..=DYNAMIC_STRING_ARRAY_TAG
    ) {
        CALL_ERROR.with(|error| error.set(INVALID_DYNAMIC_VALUE.as_ptr().cast()));
        return 0.0;
    }
    dynamic_array_result(
        array.tag,
        array_copy_within(f64::from_bits(array.payload), target, start, end),
    )
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn dynamic_array_with(array: f64, index: f64, value: f64) -> f64 {
    let (Some(array), Some(value)) = (
        dynamic_primitive(array, None),
        dynamic_primitive(value, None),
    ) else {
        CALL_ERROR.with(|error| error.set(INVALID_DYNAMIC_VALUE.as_ptr().cast()));
        return 0.0;
    };
    let operation = match (array.tag, value.tag) {
        (DYNAMIC_NUMBER_ARRAY_TAG, DYNAMIC_NUMBER_TAG) => 0,
        (DYNAMIC_STRING_ARRAY_TAG, DYNAMIC_STRING_TAG) => 1,
        (DYNAMIC_BOOLEAN_ARRAY_TAG, DYNAMIC_BOOLEAN_TAG) => 2,
        _ => {
            CALL_ERROR.with(|error| error.set(INVALID_DYNAMIC_VALUE.as_ptr().cast()));
            return 0.0;
        }
    };
    dynamic_array_result(
        array.tag,
        array_with(
            operation,
            f64::from_bits(array.payload),
            index,
            f64::from_bits(value.payload),
        ),
    )
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn array_push(operation: u8, array: f64, value: f64) -> f64 {
    let _source_root = array_callback_root(array);
    let Some(push) = ARRAY_PUSH.with(Cell::get) else {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return 0.0;
    };
    let bits = array.to_bits() & !SPARSE_ARRAY_RESULT_TAG;
    if bits & ARRAY_RESULT_TAG != 0 {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return 0.0;
    }
    let Some((_, length)) = (unsafe { array_data(array) }) else {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return 0.0;
    };
    let Some((handle, new_presence)) = array_insert_presence(array, length, false) else {
        return 0.0;
    };
    let result = unsafe { push(operation, bits as usize as *mut *mut u8, value) };
    if result < 0.0 {
        CALL_ERROR.with(|error| error.set(ALLOCATION_FAILED.as_ptr().cast()));
        0.0
    } else {
        if !new_presence.is_null() {
            unsafe { handle.add(1).write(new_presence) };
        }
        result
    }
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn array_insert_presence(
    array: f64, length: usize, unshift: bool,
) -> Option<(*mut *mut u8, *mut u8)> {
    let bits = array.to_bits();
    let handle = if bits & SPARSE_ARRAY_RESULT_TAG != 0 {
        (bits & !SPARSE_ARRAY_RESULT_TAG) as usize as *mut *mut u8
    } else if bits & ARRAY_RESULT_TAG == 0 && ARRAY_INDEX_PRESENT.with(Cell::get).is_some() {
        bits as usize as *mut *mut u8
    } else {
        std::ptr::null_mut()
    };
    let Some((_owned_presence, source_presence)) = array_native_presence(array, length) else {
        return None;
    };
    if handle.is_null() || source_presence.is_null() {
        return Some((handle, std::ptr::null_mut()));
    }
    let mask_length = unsafe { source_presence.cast::<u64>().read() as usize };
    if (0..length.min(mask_length)).all(|index| unsafe {
        source_presence.add(8 + index).read() == 1
    }) {
        return Some((handle, std::ptr::null_mut()));
    }
    let Some(size) = length.checked_add(9) else {
        CALL_ERROR.with(|error| error.set(ALLOCATION_FAILED.as_ptr().cast()));
        return None;
    };
    let Some(allocate) = ARENA_ALLOC.with(Cell::get) else {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return None;
    };
    let presence = unsafe { allocate(size, 8) };
    if presence.is_null() {
        CALL_ERROR.with(|error| error.set(ALLOCATION_FAILED.as_ptr().cast()));
        return None;
    }
    unsafe {
        presence.cast::<u64>().write((length + 1) as u64);
        std::ptr::write_bytes(presence.add(8), 1, length + 1);
        std::ptr::copy_nonoverlapping(
            source_presence.add(8), presence.add(8 + usize::from(unshift)),
            mask_length.min(length),
        );
    }
    Some((handle, presence))
}

macro_rules! array_push_fn {
    ($name:ident, $operation:expr) => {
        #[cfg(all(target_arch = "x86_64", target_family = "unix"))]
        extern "C" fn $name(array: f64, value: f64) -> f64 {
            array_push($operation, array, value)
        }
    };
}

array_push_fn!(number_array_push, 0);
array_push_fn!(string_array_push, 1);
array_push_fn!(bool_array_push, 2);

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn dynamic_array_insert(array: f64, value: f64, unshift: bool) -> f64 {
    let (Some(array), Some(value)) = (
        dynamic_primitive(array, None),
        dynamic_primitive(value, None),
    ) else {
        CALL_ERROR.with(|error| error.set(INVALID_DYNAMIC_VALUE.as_ptr().cast()));
        return 0.0;
    };
    let operation = match (array.tag, value.tag) {
        (DYNAMIC_NUMBER_ARRAY_TAG, DYNAMIC_NUMBER_TAG) => 0,
        (DYNAMIC_STRING_ARRAY_TAG, DYNAMIC_STRING_TAG) => 1,
        (DYNAMIC_BOOLEAN_ARRAY_TAG, DYNAMIC_BOOLEAN_TAG) => 2,
        _ => {
            CALL_ERROR.with(|error| error.set(INVALID_DYNAMIC_VALUE.as_ptr().cast()));
            return 0.0;
        }
    };
    let array = f64::from_bits(array.payload);
    let value = f64::from_bits(value.payload);
    if unshift {
        array_unshift(operation, array, value)
    } else {
        array_push(operation, array, value)
    }
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn dynamic_array_push(array: f64, value: f64) -> f64 {
    dynamic_array_insert(array, value, false)
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn dynamic_array_unshift(array: f64, value: f64) -> f64 {
    dynamic_array_insert(array, value, true)
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn array_unshift(operation: u8, array: f64, value: f64) -> f64 {
    let _source_root = array_callback_root(array);
    let Some(unshift) = ARRAY_UNSHIFT.with(Cell::get) else {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return 0.0;
    };
    let bits = array.to_bits() & !SPARSE_ARRAY_RESULT_TAG;
    if bits & ARRAY_RESULT_TAG != 0 {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return 0.0;
    }
    let Some((_, length)) = (unsafe { array_data(array) }) else {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return 0.0;
    };
    let Some((handle, new_presence)) = array_insert_presence(array, length, true) else {
        return 0.0;
    };
    let result = unsafe { unshift(operation, bits as usize as *mut *mut u8, value) };
    if result < 0.0 {
        CALL_ERROR.with(|error| error.set(ALLOCATION_FAILED.as_ptr().cast()));
        0.0
    } else {
        if !new_presence.is_null() {
            unsafe { handle.add(1).write(new_presence) };
        }
        result
    }
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn array_set(operation: u8, array: f64, index: f64, value: f64) -> f64 {
    let _source_root = array_callback_root(array);
    let Some(set) = ARRAY_SET.with(Cell::get) else {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return 0.0;
    };
    let bits = array.to_bits() & !SPARSE_ARRAY_RESULT_TAG;
    if bits & ARRAY_RESULT_TAG != 0 {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return 0.0;
    }
    let Some((_, length)) = (unsafe { array_data(array) }) else {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return 0.0;
    };
    if !index.is_finite() || index < 0.0 || index.fract() != 0.0 || index > usize::MAX as f64 {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return 0.0;
    }
    let index = index as usize;
    let Some((_owned_presence, source_presence)) = array_native_presence(array, length) else {
        return 0.0;
    };
    let new_presence = if source_presence.is_null() && index < length {
        std::ptr::null_mut()
    } else {
        let Some(new_len) = index.checked_add(1).map(|next| next.max(length)) else {
            CALL_ERROR.with(|error| error.set(ALLOCATION_FAILED.as_ptr().cast()));
            return 0.0;
        };
        let mut states = Vec::<u8>::new();
        if states.try_reserve_exact(new_len).is_err() {
            CALL_ERROR.with(|error| error.set(ALLOCATION_FAILED.as_ptr().cast()));
            return 0.0;
        }
        let mask_len = if source_presence.is_null() { 0 } else {
            unsafe { source_presence.cast::<u64>().read() as usize }
        };
        for current in 0..length {
            states.push(if current < mask_len {
                unsafe { source_presence.add(8 + current).read() }
            } else { 1 });
        }
        states.resize(new_len, 0);
        states[index] = 1;
        let Some(next) = array_presence_from_states(&states) else { return 0.0; };
        next
    };
    let handle = bits as usize as *mut *mut u8;
    if !new_presence.is_null() && array.to_bits() & SPARSE_ARRAY_RESULT_TAG == 0
        && ARRAY_INDEX_PRESENT.with(Cell::get).is_none() {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return 0.0;
    }
    if unsafe { set(operation, handle, index as f64, value) } != 1 {
        CALL_ERROR.with(|error| error.set(ALLOCATION_FAILED.as_ptr().cast()));
        0.0
    } else {
        if array.to_bits() & SPARSE_ARRAY_RESULT_TAG != 0
            || ARRAY_INDEX_PRESENT.with(Cell::get).is_some() {
            unsafe { handle.add(1).write(new_presence) };
        }
        value
    }
}

macro_rules! array_set_fn {
    ($name:ident, $operation:expr) => {
        #[cfg(all(target_arch = "x86_64", target_family = "unix"))]
        extern "C" fn $name(array: f64, index: f64, value: f64) -> f64 {
            array_set($operation, array, index, value)
        }
    };
}

array_set_fn!(number_array_set, 0);
array_set_fn!(string_array_set, 1);
array_set_fn!(bool_array_set, 2);

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn dynamic_array_set(array: f64, index: f64, value: f64) -> f64 {
    let (Some(array), Some(dynamic_value)) = (
        dynamic_primitive(array, None),
        dynamic_primitive(value, None),
    ) else {
        CALL_ERROR.with(|error| error.set(INVALID_DYNAMIC_VALUE.as_ptr().cast()));
        return 0.0;
    };
    let operation = match (array.tag, dynamic_value.tag) {
        (DYNAMIC_NUMBER_ARRAY_TAG, DYNAMIC_NUMBER_TAG) => 0,
        (DYNAMIC_STRING_ARRAY_TAG, DYNAMIC_STRING_TAG) => 1,
        (DYNAMIC_BOOLEAN_ARRAY_TAG, DYNAMIC_BOOLEAN_TAG) => 2,
        _ => {
            CALL_ERROR.with(|error| error.set(INVALID_DYNAMIC_VALUE.as_ptr().cast()));
            return 0.0;
        }
    };
    array_set(
        operation,
        f64::from_bits(array.payload),
        index,
        f64::from_bits(dynamic_value.payload),
    );
    if CALL_ERROR.with(|error| error.get().is_null()) { value } else { 0.0 }
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn number_source(source: &'static std::thread::LocalKey<Cell<Option<NumberSource>>>) -> f64 {
    let Some(source) = source.with(Cell::get) else {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return 0.0;
    };
    unsafe { source() }
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn math_random() -> f64 {
    number_source(&MATH_RANDOM)
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn date_now() -> f64 {
    number_source(&DATE_NOW)
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn performance_now() -> f64 {
    number_source(&PERFORMANCE_NOW)
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn process_pid() -> f64 {
    number_source(&PROCESS_PID)
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn process_ppid() -> f64 {
    number_source(&PROCESS_PPID)
}

macro_rules! array_unshift_fn {
    ($name:ident, $operation:expr) => {
        #[cfg(all(target_arch = "x86_64", target_family = "unix"))]
        extern "C" fn $name(array: f64, value: f64) -> f64 {
            array_unshift($operation, array, value)
        }
    };
}

array_unshift_fn!(number_array_unshift, 0);
array_unshift_fn!(string_array_unshift, 1);
array_unshift_fn!(bool_array_unshift, 2);

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn array_remove(operation: u8, array: f64) -> f64 {
    let _source_root = array_callback_root(array);
    let Some(remove) = ARRAY_REMOVE.with(Cell::get) else {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return 0.0;
    };
    let bits = array.to_bits() & !SPARSE_ARRAY_RESULT_TAG;
    if bits & ARRAY_RESULT_TAG != 0 {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return 0.0;
    }
    let Some((_, length)) = (unsafe { array_data(array) }) else {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return 0.0;
    };
    let Some((_owned_presence, source_presence)) = array_native_presence(array, length) else {
        return 0.0;
    };
    let handle = if array.to_bits() & SPARSE_ARRAY_RESULT_TAG != 0
        || ARRAY_INDEX_PRESENT.with(Cell::get).is_some() {
        bits as usize as *mut *mut u8
    } else {
        std::ptr::null_mut()
    };
    let removed_index = if operation < 3 { length.saturating_sub(1) } else { 0 };
    let mask_length = if source_presence.is_null() { 0 } else {
        unsafe { source_presence.cast::<u64>().read() as usize }
    };
    let state_at = |index: usize| -> u8 {
        if index < mask_length { unsafe { source_presence.add(8 + index).read() } } else { 1 }
    };
    let removed_state = if length == 0 { 0 } else { state_at(removed_index) };
    let mut new_presence = std::ptr::null_mut();
    let mut replace_presence = false;
    if !handle.is_null() && length != 0 && !source_presence.is_null() {
        replace_presence = true;
        let remaining = length - 1;
        let first = usize::from(operation >= 3);
        if (0..remaining).any(|index| state_at(index + first) != 1) {
            let Some(size) = remaining.checked_add(8) else {
                CALL_ERROR.with(|error| error.set(ALLOCATION_FAILED.as_ptr().cast()));
                return 0.0;
            };
            let Some(allocate) = ARENA_ALLOC.with(Cell::get) else {
                CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
                return 0.0;
            };
            new_presence = unsafe { allocate(size, 8) };
            if new_presence.is_null() {
                CALL_ERROR.with(|error| error.set(ALLOCATION_FAILED.as_ptr().cast()));
                return 0.0;
            }
            unsafe {
                new_presence.cast::<u64>().write(remaining as u64);
                for index in 0..remaining {
                    new_presence.add(8 + index).write(state_at(index + first));
                }
            }
        }
    }
    let mut value = 0.0;
    match unsafe { remove(operation, bits as usize as *mut *mut u8, &mut value) } {
        1 => {
            if replace_presence { unsafe { handle.add(1).write(new_presence) } }
            if removed_state == 1 { value } else {
                CALL_PRESENT.with(|present| present.set(false));
                0.0
            }
        }
        0 => {
            CALL_PRESENT.with(|present| present.set(false));
            0.0
        }
        _ => {
            CALL_ERROR.with(|error| error.set(ALLOCATION_FAILED.as_ptr().cast()));
            0.0
        }
    }
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn array_splice(array: f64, start: f64, delete_count: f64, inserts: f64) -> f64 {
    let _source_root = array_callback_root(array);
    let _inserts_root = array_callback_root(inserts);
    let Some(splice) = ARRAY_SPLICE.with(Cell::get) else {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return 0.0;
    };
    let bits = array.to_bits() & !SPARSE_ARRAY_RESULT_TAG;
    let (Some((_, source_length)), Some((insert_data, insert_length))) =
        (unsafe { array_data(array) }, unsafe { array_data(inserts) }) else {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return 0.0;
    };
    if bits & ARRAY_RESULT_TAG != 0 {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return 0.0;
    }
    let Some((_source_owned, source_presence)) = array_native_presence(array, source_length) else {
        return 0.0;
    };
    let Some((_insert_owned, insert_presence)) = array_native_presence(inserts, insert_length) else {
        return 0.0;
    };
    let Some((new_presence, removed_presence)) = array_splice_presence(
        source_presence, source_length, start, delete_count, insert_presence, insert_length, true,
    ) else { return 0.0; };
    let handle = bits as usize as *mut *mut u8;
    if !new_presence.is_null() && array.to_bits() & SPARSE_ARRAY_RESULT_TAG == 0
        && ARRAY_INDEX_PRESENT.with(Cell::get).is_none() {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return 0.0;
    }
    let removed = unsafe { splice(handle, start, delete_count, insert_data) };
    if removed.is_null() {
        CALL_ERROR.with(|error| error.set(ALLOCATION_FAILED.as_ptr().cast()));
        0.0
    } else {
        if array.to_bits() & SPARSE_ARRAY_RESULT_TAG != 0
            || ARRAY_INDEX_PRESENT.with(Cell::get).is_some() {
            unsafe { handle.add(1).write(new_presence) };
        }
        if removed_presence.is_null() { array_result(removed) }
        else { array_result_with_presence(removed, removed_presence) }
    }
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn array_presence_from_states(states: &[u8]) -> Option<*mut u8> {
    if states.iter().all(|state| *state == 1) { return Some(std::ptr::null_mut()); }
    let Some(size) = states.len().checked_add(8) else {
        CALL_ERROR.with(|error| error.set(ALLOCATION_FAILED.as_ptr().cast()));
        return None;
    };
    let Some(allocate) = ARENA_ALLOC.with(Cell::get) else {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return None;
    };
    let mask = unsafe { allocate(size, 8) };
    if mask.is_null() {
        CALL_ERROR.with(|error| error.set(ALLOCATION_FAILED.as_ptr().cast()));
        return None;
    }
    unsafe {
        mask.cast::<u64>().write(states.len() as u64);
        std::ptr::copy_nonoverlapping(states.as_ptr(), mask.add(8), states.len());
    }
    Some(mask)
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn array_splice_presence(
    source: *const u8, source_len: usize, start: f64, delete_count: f64,
    inserts: *const u8, insert_len: usize, want_removed: bool,
) -> Option<(*mut u8, *mut u8)> {
    let start = if start.is_nan() || start == f64::NEG_INFINITY { 0 }
        else if start == f64::INFINITY { source_len }
        else if start < 0.0 { (source_len as f64 + start.trunc()).max(0.0) as usize }
        else { start.trunc().min(source_len as f64) as usize };
    let removed_len = if delete_count.is_nan() { 0 } else {
        (delete_count.max(0.0) as usize).min(source_len - start)
    };
    let Some(new_len) = source_len.checked_sub(removed_len).and_then(|n| n.checked_add(insert_len)) else {
        CALL_ERROR.with(|error| error.set(ALLOCATION_FAILED.as_ptr().cast()));
        return None;
    };
    let mut next = Vec::<u8>::new();
    let mut removed = Vec::<u8>::new();
    if next.try_reserve_exact(new_len).is_err() || removed.try_reserve_exact(removed_len).is_err() {
        CALL_ERROR.with(|error| error.set(ALLOCATION_FAILED.as_ptr().cast()));
        return None;
    }
    let state = |mask: *const u8, index: usize| -> u8 {
        if mask.is_null() || index >= unsafe { mask.cast::<u64>().read() as usize } { 1 }
        else { unsafe { mask.add(8 + index).read() } }
    };
    for index in 0..start { next.push(state(source, index)); }
    for index in start..start + removed_len { removed.push(state(source, index)); }
    for index in 0..insert_len { next.push(state(inserts, index)); }
    for index in start + removed_len..source_len { next.push(state(source, index)); }
    let new_mask = array_presence_from_states(&next)?;
    let removed_mask = if want_removed { array_presence_from_states(&removed)? }
        else { std::ptr::null_mut() };
    Some((new_mask, removed_mask))
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn array_to_spliced(array: f64, start: f64, delete_count: f64, inserts: f64) -> f64 {
    let _source_root = array_callback_root(array);
    let _inserts_root = array_callback_root(inserts);
    let Some(splice) = ARRAY_SPLICE.with(Cell::get) else {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return 0.0;
    };
    let (Some((data, source_length)), Some((insert_data, insert_length))) =
        (unsafe { array_data(array) }, unsafe { array_data(inserts) })
    else {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return 0.0;
    };
    let Some((_source_owned, source_presence)) = array_native_presence(array, source_length) else {
        return 0.0;
    };
    let Some((_insert_owned, insert_presence)) = array_native_presence(inserts, insert_length) else {
        return 0.0;
    };
    let Some((new_presence, _removed_presence)) = array_splice_presence(
        source_presence, source_length, start, delete_count, insert_presence, insert_length, false,
    ) else { return 0.0; };
    let mut output = data.cast_mut();
    let removed = unsafe { splice(&mut output, start, delete_count, insert_data) };
    if removed.is_null() {
        CALL_ERROR.with(|error| error.set(ALLOCATION_FAILED.as_ptr().cast()));
        0.0
    } else {
        if new_presence.is_null() { array_result(output) }
        else { array_result_with_presence(output, new_presence) }
    }
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn dynamic_array_splice(
    array: f64,
    start: f64,
    delete_count: f64,
    inserts: f64,
    copy: bool,
) -> f64 {
    let (Some(array), Some(inserts)) = (
        dynamic_primitive(array, None),
        dynamic_primitive(inserts, None),
    ) else {
        CALL_ERROR.with(|error| error.set(INVALID_DYNAMIC_VALUE.as_ptr().cast()));
        return 0.0;
    };
    if array.tag != inserts.tag
        || !matches!(
            array.tag,
            DYNAMIC_NUMBER_ARRAY_TAG..=DYNAMIC_STRING_ARRAY_TAG
        )
    {
        CALL_ERROR.with(|error| error.set(INVALID_DYNAMIC_VALUE.as_ptr().cast()));
        return 0.0;
    }
    let splice = if copy { array_to_spliced } else { array_splice };
    dynamic_array_result(
        array.tag,
        splice(
            f64::from_bits(array.payload),
            start,
            delete_count,
            f64::from_bits(inserts.payload),
        ),
    )
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn dynamic_array_splice_in_place(
    array: f64,
    start: f64,
    delete_count: f64,
    inserts: f64,
) -> f64 {
    dynamic_array_splice(array, start, delete_count, inserts, false)
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn dynamic_array_to_spliced(
    array: f64,
    start: f64,
    delete_count: f64,
    inserts: f64,
) -> f64 {
    dynamic_array_splice(array, start, delete_count, inserts, true)
}

macro_rules! array_remove_fn {
    ($name:ident, $operation:expr) => {
        #[cfg(all(target_arch = "x86_64", target_family = "unix"))]
        extern "C" fn $name(array: f64) -> f64 {
            array_remove($operation, array)
        }
    };
}

array_remove_fn!(number_array_pop, 0);
array_remove_fn!(string_array_pop, 1);
array_remove_fn!(bool_array_pop, 2);
array_remove_fn!(number_array_shift, 3);
array_remove_fn!(string_array_shift, 4);
array_remove_fn!(bool_array_shift, 5);

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn dynamic_array_remove(array: f64, shift: bool) -> f64 {
    let Some(array) = dynamic_primitive(array, None) else {
        CALL_ERROR.with(|error| error.set(INVALID_DYNAMIC_VALUE.as_ptr().cast()));
        return 0.0;
    };
    let (operation, tag) = match array.tag {
        DYNAMIC_NUMBER_ARRAY_TAG => (0, DYNAMIC_NUMBER_TAG),
        DYNAMIC_STRING_ARRAY_TAG => (1, DYNAMIC_STRING_TAG),
        DYNAMIC_BOOLEAN_ARRAY_TAG => (2, DYNAMIC_BOOLEAN_TAG),
        _ => {
            CALL_ERROR.with(|error| error.set(INVALID_DYNAMIC_VALUE.as_ptr().cast()));
            return 0.0;
        }
    };
    let value = array_remove(
        operation + u8::from(shift) * 3,
        f64::from_bits(array.payload),
    );
    if CALL_ERROR.with(|error| !error.get().is_null()) { return 0.0; }
    let _value_root = (tag == DYNAMIC_STRING_TAG && CALL_PRESENT.with(Cell::get))
        .then(|| thaw_arena::ArenaRoot::new(value.to_bits() as usize));
    if CALL_PRESENT.with(Cell::get) {
        dynamic_from_parts(tag as f64, value)
    } else {
        dynamic_from_parts(DYNAMIC_UNDEFINED_TAG as f64, 0.0)
    }
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn dynamic_array_pop(array: f64) -> f64 {
    dynamic_array_remove(array, false)
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn dynamic_array_shift(array: f64) -> f64 {
    dynamic_array_remove(array, true)
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn array_with(operation: u8, array: f64, index: f64, value: f64) -> f64 {
    let _source_root = array_callback_root(array);
    let (Some(replace), Some((data, length))) =
        (ARRAY_WITH.with(Cell::get), unsafe { array_data(array) })
    else {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return 0.0;
    };
    let Some((_owned_presence, source_presence)) = array_native_presence(array, length) else {
        return 0.0;
    };
    let result = unsafe { replace(operation, data, index, value) };
    if result.is_null() {
        CALL_ERROR.with(|error| error.set(INVALID_ARRAY_WITH_INDEX.as_ptr().cast()));
        0.0
    } else if source_presence.is_null() {
        array_result(result)
    } else {
        let _result_root = thaw_arena::ArenaRoot::new(result as usize);
        let Some(size) = length.checked_add(8) else {
            CALL_ERROR.with(|error| error.set(ALLOCATION_FAILED.as_ptr().cast()));
            return 0.0;
        };
        let Some(allocate) = ARENA_ALLOC.with(Cell::get) else {
            CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
            return 0.0;
        };
        let presence = unsafe { allocate(size, 8) };
        if presence.is_null() {
            CALL_ERROR.with(|error| error.set(ALLOCATION_FAILED.as_ptr().cast()));
            return 0.0;
        }
        unsafe {
            presence.cast::<u64>().write(length as u64);
            std::ptr::write_bytes(presence.add(8), 1, length);
            let source_length = source_presence.cast::<u64>().read() as usize;
            std::ptr::copy_nonoverlapping(
                source_presence.add(8), presence.add(8), source_length.min(length),
            );
            let target = if index.is_nan() { 0.0 } else { index.trunc() };
            let target = if target < 0.0 { length as f64 + target } else { target };
            presence.add(8 + target as usize).write(1);
        }
        array_result_with_presence(result, presence)
    }
}

macro_rules! array_with_fn {
    ($name:ident, $operation:expr) => {
        #[cfg(all(target_arch = "x86_64", target_family = "unix"))]
        extern "C" fn $name(array: f64, index: f64, value: f64) -> f64 {
            array_with($operation, array, index, value)
        }
    };
}

array_with_fn!(number_array_with, 0);
array_with_fn!(string_array_with, 1);
array_with_fn!(bool_array_with, 2);

macro_rules! array_format_fn {
    ($name:ident, $operation:expr) => {
        #[cfg(all(target_arch = "x86_64", target_family = "unix"))]
        extern "C" fn $name(value: f64, separator: f64) -> f64 {
            array_format($operation, value, separator)
        }
    };
}

array_format_fn!(number_array_join, 0);
array_format_fn!(string_array_join, 1);
array_format_fn!(bool_array_join, 2);

macro_rules! array_search_fn {
    ($name:ident, $operation:expr) => {
        #[cfg(all(target_arch = "x86_64", target_family = "unix"))]
        extern "C" fn $name(value: f64, needle: f64, from_index: f64) -> f64 {
            array_search($operation, value, needle, from_index)
        }
    };
}

array_search_fn!(number_array_index_of, 0);
array_search_fn!(number_array_includes, 1);
array_search_fn!(string_array_index_of, 2);
array_search_fn!(string_array_includes, 3);
array_search_fn!(bool_array_index_of, 4);
array_search_fn!(bool_array_includes, 5);
array_search_fn!(number_array_last_index_of, 6);
array_search_fn!(string_array_last_index_of, 7);
array_search_fn!(bool_array_last_index_of, 8);

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn dynamic_array_search(operation: u8, value: f64, needle: f64, from_index: f64) -> f64 {
    let (Some(value), Some(needle)) = (
        dynamic_primitive(value, None),
        dynamic_primitive(needle, None),
    ) else {
        CALL_ERROR.with(|error| error.set(INVALID_DYNAMIC_VALUE.as_ptr().cast()));
        return 0.0;
    };
    if !matches!(value.tag, DYNAMIC_NUMBER_ARRAY_TAG..=DYNAMIC_STRING_ARRAY_TAG)
        || operation > 2
    {
        CALL_ERROR.with(|error| error.set(INVALID_DYNAMIC_VALUE.as_ptr().cast()));
        return 0.0;
    }
    if needle.tag == DYNAMIC_UNDEFINED_TAG {
        let array = f64::from_bits(value.payload);
        let Some((_, length)) = (unsafe { array_data(array) }) else {
            CALL_ERROR.with(|error| error.set(INVALID_DYNAMIC_VALUE.as_ptr().cast()));
            return 0.0;
        };
        if operation != 1 { return -1.0; }
        let start = if from_index.is_nan() {
            0
        } else if from_index >= 0.0 {
            from_index.trunc().min(length as f64) as usize
        } else {
            (length as f64 + from_index.trunc()).max(0.0) as usize
        };
        return f64::from((start..length).any(|index| !array_index_present(array, index)));
    }
    let search = match (value.tag, needle.tag, operation) {
        (DYNAMIC_NUMBER_ARRAY_TAG, DYNAMIC_NUMBER_TAG, 0..=1) => operation,
        (DYNAMIC_STRING_ARRAY_TAG, DYNAMIC_STRING_TAG, 0..=1) => operation + 2,
        (DYNAMIC_BOOLEAN_ARRAY_TAG, DYNAMIC_BOOLEAN_TAG, 0..=1) => operation + 4,
        (DYNAMIC_NUMBER_ARRAY_TAG, DYNAMIC_NUMBER_TAG, 2) => 6,
        (DYNAMIC_STRING_ARRAY_TAG, DYNAMIC_STRING_TAG, 2) => 7,
        (DYNAMIC_BOOLEAN_ARRAY_TAG, DYNAMIC_BOOLEAN_TAG, 2) => 8,
        _ => {
            if unsafe { array_data(f64::from_bits(value.payload)) }.is_none() {
                CALL_ERROR.with(|error| error.set(INVALID_DYNAMIC_VALUE.as_ptr().cast()));
                return 0.0;
            }
            return if operation == 1 { 0.0 } else { -1.0 };
        }
    };
    array_search(
        search,
        f64::from_bits(value.payload),
        f64::from_bits(needle.payload),
        from_index,
    )
}

macro_rules! dynamic_array_search_fn {
    ($name:ident, $operation:expr) => {
        #[cfg(all(target_arch = "x86_64", target_family = "unix"))]
        extern "C" fn $name(value: f64, needle: f64, from_index: f64) -> f64 {
            dynamic_array_search($operation, value, needle, from_index)
        }
    };
}

dynamic_array_search_fn!(dynamic_array_index_of, 0);
dynamic_array_search_fn!(dynamic_array_includes, 1);
dynamic_array_search_fn!(dynamic_array_last_index_of, 2);

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn string_truthy(value: f64) -> f64 {
    let value = value.to_bits() as usize as *const c_char;
    unsafe { (!value.is_null() && !CStr::from_ptr(value).to_bytes().is_empty()) as u8 as f64 }
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn string_char_code_at(value: f64, index: f64) -> f64 {
    if index.is_infinite() {
        return f64::NAN;
    }
    let index = if index.is_nan() { 0.0 } else { index.trunc() };
    if index < 0.0 || index > usize::MAX as f64 {
        return f64::NAN;
    }
    unsafe {
        string_utf16_argument(value)
            .and_then(|value| value.get(index as usize).copied())
            .map_or(f64::NAN, f64::from)
    }
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn string_char_at(value: f64, index: f64) -> f64 {
    let code = string_char_code_at(value, index);
    if code.is_nan() {
        arena_string(String::new())
    } else {
        arena_string_bytes(&thaw_arena::wtf8_encode_utf16(&[code as u16]))
    }
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn string_at(value: f64, index: f64) -> f64 {
    let Some(value) = (unsafe { string_utf16_argument(value) }) else {
        CALL_PRESENT.with(|present| present.set(false));
        return f64::from_bits(0);
    };
    let units = value;
    let index = if index.is_nan() { 0.0 } else { index.trunc() };
    let index = if index < 0.0 {
        units.len() as f64 + index
    } else {
        index
    };
    let Some(unit) = (index.is_finite() && index >= 0.0)
        .then(|| units.get(index as usize))
        .flatten()
    else {
        CALL_PRESENT.with(|present| present.set(false));
        return f64::from_bits(0);
    };
    arena_string_bytes(&thaw_arena::wtf8_encode_utf16(&[*unit]))
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn string_code_point_at(value: f64, index: f64) -> f64 {
    let Some(value) = (unsafe { string_utf16_argument(value) }) else {
        CALL_PRESENT.with(|present| present.set(false));
        return 0.0;
    };
    let units = value;
    let index = if index.is_nan() { 0.0 } else { index.trunc() };
    let Some(&first) = (index.is_finite() && index >= 0.0)
        .then(|| units.get(index as usize))
        .flatten()
    else {
        CALL_PRESENT.with(|present| present.set(false));
        return 0.0;
    };
    let Some(&second) = units.get(index as usize + 1) else {
        return f64::from(first);
    };
    if (0xd800..=0xdbff).contains(&first) && (0xdc00..=0xdfff).contains(&second) {
        f64::from(0x10000 + ((u32::from(first) - 0xd800) << 10) + u32::from(second) - 0xdc00)
    } else {
        f64::from(first)
    }
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn string_compare(left: f64, right: f64) -> f64 {
    unsafe {
        let Some(left) = string_utf16_argument(left) else {
            return 0.0;
        };
        let Some(right) = string_utf16_argument(right) else {
            return 0.0;
        };
        match left.cmp(&right) {
            std::cmp::Ordering::Less => -1.0,
            std::cmp::Ordering::Equal => 0.0,
            std::cmp::Ordering::Greater => 1.0,
        }
    }
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn number_same_value(left: f64, right: f64) -> f64 {
    f64::from(u8::from(
        (left.is_nan() && right.is_nan()) || left.to_bits() == right.to_bits(),
    ))
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn string_same_value(left: f64, right: f64) -> f64 {
    f64::from(u8::from(unsafe {
        string_utf16_argument(left) == string_utf16_argument(right)
    }))
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn reference_same_value(left: f64, right: f64) -> f64 {
    f64::from(u8::from(left.to_bits() == right.to_bits()))
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn type_of_number(_: f64) -> f64 {
    f64::from_bits(c"number".as_ptr() as usize as u64)
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn type_of_boolean(_: f64) -> f64 {
    f64::from_bits(c"boolean".as_ptr() as usize as u64)
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn type_of_string(_: f64) -> f64 {
    f64::from_bits(c"string".as_ptr() as usize as u64)
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn type_of_object(_: f64) -> f64 {
    f64::from_bits(c"object".as_ptr() as usize as u64)
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn string_is_well_formed(value: f64) -> f64 {
    let Some(units) = (unsafe { string_utf16_argument(value) }) else {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return 0.0;
    };
    f64::from(char::decode_utf16(units).all(|character| character.is_ok()))
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn string_to_well_formed(value: f64) -> f64 {
    let Some(units) = (unsafe { string_utf16_argument(value) }) else {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return f64::from_bits(0);
    };
    if char::decode_utf16(units.iter().copied()).all(|character| character.is_ok()) {
        return value;
    }
    let repaired: String = char::decode_utf16(units)
        .map(|character| character.unwrap_or(char::REPLACEMENT_CHARACTER)).collect();
    arena_string(repaired)
}
