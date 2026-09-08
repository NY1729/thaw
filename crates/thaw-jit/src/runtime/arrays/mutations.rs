#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn dynamic_array_result(tag: u64, result: f64) -> f64 {
    let bits = result.to_bits();
    if bits & ARRAY_RESULT_TAG == 0 {
        return 0.0;
    }
    let Some(handle) = ARENA_ALLOC.with(|allocator| {
        allocator.get().map(|alloc| unsafe {
            alloc(std::mem::size_of::<usize>(), std::mem::align_of::<usize>())
        })
    }) else {
        CALL_ERROR.with(|error| error.set(ALLOCATION_FAILED.as_ptr().cast()));
        return 0.0;
    };
    if handle.is_null() {
        CALL_ERROR.with(|error| error.set(ALLOCATION_FAILED.as_ptr().cast()));
        return 0.0;
    }
    unsafe {
        handle
            .cast::<usize>()
            .write(bits as usize & !ARRAY_RESULT_TAG as usize)
    };
    dynamic_from_parts(tag as f64, f64::from_bits(handle as usize as u64))
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
    let (Some(slice), Some((data, _))) =
        (ARRAY_SLICE.with(Cell::get), unsafe { array_data(value) })
    else {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return 0.0;
    };
    let result = unsafe { slice(data, 8, start, end) };
    if result.is_null() {
        CALL_ERROR.with(|error| error.set(ALLOCATION_FAILED.as_ptr().cast()));
        0.0
    } else {
        array_result(result)
    }
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn array_concat(left: f64, right: f64) -> f64 {
    let (Some(concat), Some((left, _)), Some((right, _))) = (
        ARRAY_CONCAT.with(Cell::get),
        unsafe { array_data(left) },
        unsafe { array_data(right) },
    ) else {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return 0.0;
    };
    let result = unsafe { concat(left, right, 8) };
    if result.is_null() {
        CALL_ERROR.with(|error| error.set(ALLOCATION_FAILED.as_ptr().cast()));
        0.0
    } else {
        array_result(result)
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
    let (Some(append), Some((array, _))) =
        (ARRAY_APPEND.with(Cell::get), unsafe { array_data(array) })
    else {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return 0.0;
    };
    let result = unsafe { append(operation, array, value) };
    if result.is_null() {
        CALL_ERROR.with(|error| error.set(ALLOCATION_FAILED.as_ptr().cast()));
        0.0
    } else {
        array_result(result)
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
    let (Some(reverse), Some((data, _))) = (ARRAY_TO_REVERSED.with(Cell::get), unsafe {
        array_data(value)
    }) else {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return 0.0;
    };
    let result = unsafe { reverse(data, 8) };
    if result.is_null() {
        CALL_ERROR.with(|error| error.set(ALLOCATION_FAILED.as_ptr().cast()));
        0.0
    } else {
        array_result(result)
    }
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn array_reverse(value: f64) -> f64 {
    let (Some(reverse), Some((data, _))) =
        (ARRAY_REVERSE.with(Cell::get), unsafe { array_data(value) })
    else {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return 0.0;
    };
    let result = unsafe { reverse(data.cast_mut(), 8) };
    if result.is_null() {
        CALL_ERROR.with(|error| error.set(ALLOCATION_FAILED.as_ptr().cast()));
        0.0
    } else {
        array_result(result)
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
    let (Some(sort), Some((data, _))) = (ARRAY_TO_SORTED.with(Cell::get), unsafe {
        array_data(value)
    }) else {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return 0.0;
    };
    let result = unsafe { sort(operation, data) };
    if result.is_null() {
        CALL_ERROR.with(|error| error.set(ALLOCATION_FAILED.as_ptr().cast()));
        0.0
    } else {
        array_result(result)
    }
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
    let (Some(sort), Some((data, _))) = (ARRAY_SORT.with(Cell::get), unsafe { array_data(value) })
    else {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return 0.0;
    };
    let result = unsafe { sort(operation, data.cast_mut()) };
    if result.is_null() {
        CALL_ERROR.with(|error| error.set(ALLOCATION_FAILED.as_ptr().cast()));
        0.0
    } else {
        array_result(result)
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
    let (Some(fill), Some((data, _))) = (ARRAY_FILL.with(Cell::get), unsafe { array_data(array) })
    else {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return 0.0;
    };
    let result = unsafe { fill(operation, data.cast_mut(), value, start, end) };
    if result.is_null() {
        CALL_ERROR.with(|error| error.set(ALLOCATION_FAILED.as_ptr().cast()));
        0.0
    } else {
        array_result(result)
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
    let (Some(copy), Some((data, _))) = (ARRAY_COPY_WITHIN.with(Cell::get), unsafe {
        array_data(array)
    }) else {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return 0.0;
    };
    let result = unsafe { copy(data.cast_mut(), 8, target, start, end) };
    if result.is_null() {
        CALL_ERROR.with(|error| error.set(ALLOCATION_FAILED.as_ptr().cast()));
        0.0
    } else {
        array_result(result)
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
    let Some(push) = ARRAY_PUSH.with(Cell::get) else {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return 0.0;
    };
    let bits = array.to_bits();
    if bits & ARRAY_RESULT_TAG != 0 {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return 0.0;
    }
    let result = unsafe { push(operation, bits as usize as *mut *mut u8, value) };
    if result < 0.0 {
        CALL_ERROR.with(|error| error.set(ALLOCATION_FAILED.as_ptr().cast()));
        0.0
    } else {
        result
    }
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
    let Some(unshift) = ARRAY_UNSHIFT.with(Cell::get) else {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return 0.0;
    };
    let bits = array.to_bits();
    if bits & ARRAY_RESULT_TAG != 0 {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return 0.0;
    }
    let result = unsafe { unshift(operation, bits as usize as *mut *mut u8, value) };
    if result < 0.0 {
        CALL_ERROR.with(|error| error.set(ALLOCATION_FAILED.as_ptr().cast()));
        0.0
    } else {
        result
    }
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn array_set(operation: u8, array: f64, index: f64, value: f64) -> f64 {
    let Some(set) = ARRAY_SET.with(Cell::get) else {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return 0.0;
    };
    let bits = array.to_bits();
    if bits & ARRAY_RESULT_TAG != 0
        || unsafe { set(operation, bits as usize as *mut *mut u8, index, value) } != 1
    {
        CALL_ERROR.with(|error| error.set(ALLOCATION_FAILED.as_ptr().cast()));
        0.0
    } else {
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
    value
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
    let Some(remove) = ARRAY_REMOVE.with(Cell::get) else {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return 0.0;
    };
    let bits = array.to_bits();
    if bits & ARRAY_RESULT_TAG != 0 {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return 0.0;
    }
    let mut value = 0.0;
    match unsafe { remove(operation, bits as usize as *mut *mut u8, &mut value) } {
        1 => value,
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
    let Some(splice) = ARRAY_SPLICE.with(Cell::get) else {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return 0.0;
    };
    let bits = array.to_bits();
    let Some((inserts, _)) = (unsafe { array_data(inserts) }) else {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return 0.0;
    };
    if bits & ARRAY_RESULT_TAG != 0 {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return 0.0;
    }
    let removed = unsafe { splice(bits as usize as *mut *mut u8, start, delete_count, inserts) };
    if removed.is_null() {
        CALL_ERROR.with(|error| error.set(ALLOCATION_FAILED.as_ptr().cast()));
        0.0
    } else {
        array_result(removed)
    }
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn array_to_spliced(array: f64, start: f64, delete_count: f64, inserts: f64) -> f64 {
    let Some(splice) = ARRAY_SPLICE.with(Cell::get) else {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return 0.0;
    };
    let (Some((array, _)), Some((inserts, _))) =
        (unsafe { array_data(array) }, unsafe { array_data(inserts) })
    else {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return 0.0;
    };
    let mut output = array.cast_mut();
    let removed = unsafe { splice(&mut output, start, delete_count, inserts) };
    if removed.is_null() {
        CALL_ERROR.with(|error| error.set(ALLOCATION_FAILED.as_ptr().cast()));
        0.0
    } else {
        array_result(output)
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
    if CALL_PRESENT.with(Cell::get) {
        dynamic_from_parts(tag as f64, value)
    } else {
        0.0
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
    let (Some(replace), Some((data, _))) =
        (ARRAY_WITH.with(Cell::get), unsafe { array_data(array) })
    else {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return 0.0;
    };
    let result = unsafe { replace(operation, data, index, value) };
    if result.is_null() {
        CALL_ERROR.with(|error| error.set(INVALID_ARRAY_WITH_INDEX.as_ptr().cast()));
        0.0
    } else {
        array_result(result)
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
    let search = match (value.tag, needle.tag, operation) {
        (DYNAMIC_NUMBER_ARRAY_TAG, DYNAMIC_NUMBER_TAG, 0..=1) => operation,
        (DYNAMIC_STRING_ARRAY_TAG, DYNAMIC_STRING_TAG, 0..=1) => operation + 2,
        (DYNAMIC_BOOLEAN_ARRAY_TAG, DYNAMIC_BOOLEAN_TAG, 0..=1) => operation + 4,
        (DYNAMIC_NUMBER_ARRAY_TAG, DYNAMIC_NUMBER_TAG, 2) => 6,
        (DYNAMIC_STRING_ARRAY_TAG, DYNAMIC_STRING_TAG, 2) => 7,
        (DYNAMIC_BOOLEAN_ARRAY_TAG, DYNAMIC_BOOLEAN_TAG, 2) => 8,
        _ => {
            CALL_ERROR.with(|error| error.set(INVALID_DYNAMIC_VALUE.as_ptr().cast()));
            return 0.0;
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
        string_argument(value)
            .and_then(|value| value.encode_utf16().nth(index as usize))
            .map_or(f64::NAN, f64::from)
    }
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn string_char_at(value: f64, index: f64) -> f64 {
    let code = string_char_code_at(value, index);
    if code.is_nan() {
        arena_string(String::new())
    } else {
        arena_string(String::from_utf16_lossy(&[code as u16]))
    }
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn string_at(value: f64, index: f64) -> f64 {
    let Some(value) = (unsafe { string_argument(value) }) else {
        CALL_PRESENT.with(|present| present.set(false));
        return f64::from_bits(0);
    };
    let units = value.encode_utf16().collect::<Vec<_>>();
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
    arena_string(String::from_utf16_lossy(&[*unit]))
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn string_code_point_at(value: f64, index: f64) -> f64 {
    let Some(value) = (unsafe { string_argument(value) }) else {
        CALL_PRESENT.with(|present| present.set(false));
        return 0.0;
    };
    let units = value.encode_utf16().collect::<Vec<_>>();
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
        let Some(left) = string_argument(left) else {
            return 0.0;
        };
        let Some(right) = string_argument(right) else {
            return 0.0;
        };
        match left.encode_utf16().cmp(right.encode_utf16()) {
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
        string_argument(left) == string_argument(right)
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
extern "C" fn string_is_well_formed(_: f64) -> f64 {
    1.0
}
