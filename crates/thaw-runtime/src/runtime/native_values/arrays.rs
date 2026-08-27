#[no_mangle]
/// Converts a UTF-8 string into the native `string[]` array layout, following
/// JavaScript string-iterator semantics (one Unicode scalar value per slot).
///
/// # Safety
///
/// `value` must be null or point to a valid NUL-terminated UTF-8 string.
pub unsafe extern "C" fn thaw_string_to_array(value: *const c_char) -> *mut u8 {
    if value.is_null() {
        return std::ptr::null_mut();
    }
    let value = unsafe { CStr::from_ptr(value) }.to_string_lossy();
    let characters = value.chars().collect::<Vec<_>>();
    let output = thaw_arena::thaw_arena_alloc((characters.len() + 1) * 8, 8);
    if output.is_null() {
        return std::ptr::null_mut();
    }
    unsafe {
        output.cast::<u64>().write(characters.len() as u64);
    }
    for (index, character) in characters.into_iter().enumerate() {
        let Some(character) = arena_c_string(&character.to_string()) else {
            return std::ptr::null_mut();
        };
        unsafe {
            output
                .add(8 + index * 8)
                .cast::<*const u8>()
                .write_unaligned(character);
        }
    }
    output
}

#[no_mangle]
/// Returns the enumerable numeric keys of a native array or tuple.
///
/// # Safety
/// `array` must be null or point to a native array layout beginning with its
/// signed 64-bit element count.
pub unsafe extern "C" fn thaw_array_keys(array: *const u8) -> *mut u8 {
    let length = if array.is_null() {
        0
    } else {
        unsafe { array.cast::<i64>().read() }.max(0) as usize
    };
    let output = thaw_arena::thaw_arena_alloc((length + 1) * 8, 8);
    if output.is_null() {
        return output;
    }
    unsafe { output.cast::<i64>().write(length as i64) };
    for index in 0..length {
        let Some(key) = arena_c_string(&index.to_string()) else {
            return std::ptr::null_mut();
        };
        unsafe {
            output
                .add(8 + index * 8)
                .cast::<*const u8>()
                .write_unaligned(key)
        };
    }
    output
}

#[no_mangle]
/// Reverses a fixed-width native array in place and returns the receiver.
///
/// # Safety
///
/// `array` must point to a writable Thaw array whose elements occupy
/// `element_width` bytes, and `element_width` must be nonzero.
pub unsafe extern "C" fn thaw_array_reverse(array: *mut u8, element_width: usize) -> *mut u8 {
    let Some(length) = (unsafe { native_array_length(array) }) else {
        return std::ptr::null_mut();
    };
    if element_width == 0 {
        return std::ptr::null_mut();
    }
    for left in 0..length / 2 {
        let right = length - left - 1;
        unsafe {
            std::ptr::swap_nonoverlapping(
                array.add(8 + left * element_width),
                array.add(8 + right * element_width),
                element_width,
            );
        }
    }
    array
}

fn relative_array_index(index: f64, length: usize) -> usize {
    if index.is_nan() || index == f64::NEG_INFINITY {
        return 0;
    }
    if index == f64::INFINITY {
        return length;
    }
    let index = index.trunc();
    if index < 0.0 {
        (length as f64 + index).max(0.0) as usize
    } else {
        index.min(length as f64) as usize
    }
}

#[no_mangle]
/// Implements in-place `Array.prototype.copyWithin` for fixed-width slots.
///
/// # Safety
///
/// `array` must point to a writable Thaw array whose elements occupy
/// `element_width` bytes, and `element_width` must be nonzero.
pub unsafe extern "C" fn thaw_array_copy_within(
    array: *mut u8,
    element_width: usize,
    target: f64,
    start: f64,
    end: f64,
) -> *mut u8 {
    let Some(length) = (unsafe { native_array_length(array) }) else {
        return std::ptr::null_mut();
    };
    if element_width == 0 {
        return std::ptr::null_mut();
    }
    let target = relative_array_index(target, length);
    let start = relative_array_index(start, length);
    let end = relative_array_index(end, length);
    let count = end.saturating_sub(start).min(length - target);
    if count != 0 {
        unsafe {
            std::ptr::copy(
                array.add(8 + start * element_width),
                array.add(8 + target * element_width),
                count * element_width,
            );
        }
    }
    array
}

unsafe fn fill_array_slots<T: Copy>(array: *mut u8, value: T, start: f64, end: f64) -> *mut u8 {
    let Some(length) = (unsafe { native_array_length(array) }) else {
        return std::ptr::null_mut();
    };
    let start = relative_array_index(start, length);
    let end = relative_array_index(end, length);
    for index in start..end {
        unsafe { array.add(8 + index * 8).cast::<T>().write_unaligned(value) };
    }
    array
}

#[no_mangle]
/// # Safety
/// `array` must point to a writable Thaw number array.
pub unsafe extern "C" fn thaw_number_array_fill(
    array: *mut u8,
    value: f64,
    start: f64,
    end: f64,
) -> *mut u8 {
    unsafe { fill_array_slots(array, value, start, end) }
}

#[no_mangle]
/// # Safety
/// `array` must point to a writable Thaw pointer-slot array.
pub unsafe extern "C" fn thaw_pointer_array_fill(
    array: *mut u8,
    value: *const u8,
    start: f64,
    end: f64,
) -> *mut u8 {
    unsafe { fill_array_slots(array, value, start, end) }
}

#[no_mangle]
/// # Safety
/// `array` must point to a writable Thaw boolean array.
pub unsafe extern "C" fn thaw_bool_array_fill(
    array: *mut u8,
    value: u8,
    start: f64,
    end: f64,
) -> *mut u8 {
    unsafe { fill_array_slots(array, value, start, end) }
}

#[no_mangle]
/// Fills a native array range by copying one complete element into every slot.
///
/// # Safety
/// `array` must point to a writable Thaw array whose elements occupy
/// `element_width` bytes. `value` must point to at least `element_width`
/// readable bytes and must not overlap `array`.
pub unsafe extern "C" fn thaw_array_fill(
    array: *mut u8,
    value: *const u8,
    element_width: usize,
    start: f64,
    end: f64,
) -> *mut u8 {
    let Some(length) = (unsafe { native_array_length(array) }) else {
        return std::ptr::null_mut();
    };
    if value.is_null() || element_width == 0 {
        return std::ptr::null_mut();
    }
    let start = relative_array_index(start, length);
    let end = relative_array_index(end, length);
    for index in start..end {
        unsafe {
            std::ptr::copy_nonoverlapping(
                value,
                array.add(8 + index * element_width),
                element_width,
            );
        }
    }
    array
}

#[no_mangle]
/// Returns an arena-owned shallow copy of a native array range.
///
/// # Safety
///
/// `array` must point to a readable Thaw array whose elements occupy
/// `element_width` bytes, and `element_width` must be nonzero.
pub unsafe extern "C" fn thaw_array_slice(
    array: *const u8,
    element_width: usize,
    start: f64,
    end: f64,
) -> *mut u8 {
    let Some(length) = (unsafe { native_array_length(array) }) else {
        return std::ptr::null_mut();
    };
    if element_width == 0 {
        return std::ptr::null_mut();
    }
    let start = relative_array_index(start, length);
    let end = relative_array_index(end, length);
    let count = end.saturating_sub(start);
    let output = thaw_arena::thaw_arena_alloc(8 + count * element_width, element_width.min(8));
    if output.is_null() {
        return std::ptr::null_mut();
    }
    unsafe {
        output.cast::<u64>().write(count as u64);
        if count != 0 {
            std::ptr::copy_nonoverlapping(
                array.add(8 + start * element_width),
                output.add(8),
                count * element_width,
            );
        }
    }
    output
}

#[no_mangle]
/// Returns an arena-owned reversed shallow copy of a native array.
///
/// # Safety
///
/// `array` must point to a readable Thaw array whose elements occupy
/// `element_width` bytes, and `element_width` must be nonzero.
pub unsafe extern "C" fn thaw_array_to_reversed(array: *const u8, element_width: usize) -> *mut u8 {
    let output = unsafe { thaw_array_slice(array, element_width, 0.0, f64::INFINITY) };
    if output.is_null() {
        return output;
    }
    unsafe { thaw_array_reverse(output, element_width) }
}

unsafe fn native_array_slots(array: *mut u8) -> Option<&'static mut [u64]> {
    let length = unsafe { native_array_length(array) }?;
    Some(unsafe { std::slice::from_raw_parts_mut(array.add(8).cast::<u64>(), length) })
}

#[no_mangle]
/// # Safety
/// `array` must point to a writable Thaw number array.
pub unsafe extern "C" fn thaw_number_array_sort(array: *mut u8) -> *mut u8 {
    let Some(slots) = (unsafe { native_array_slots(array) }) else {
        return std::ptr::null_mut();
    };
    slots.sort_by(|left, right| {
        javascript_number_string(f64::from_bits(*left))
            .encode_utf16()
            .cmp(javascript_number_string(f64::from_bits(*right)).encode_utf16())
    });
    array
}

#[no_mangle]
/// # Safety
/// `array` must point to a writable Thaw C-string pointer array.
pub unsafe extern "C" fn thaw_string_array_sort(array: *mut u8) -> *mut u8 {
    let Some(slots) = (unsafe { native_array_slots(array) }) else {
        return std::ptr::null_mut();
    };
    slots.sort_by(|left, right| {
        let string = |value: u64| {
            let pointer = value as usize as *const c_char;
            if pointer.is_null() {
                Vec::new()
            } else {
                unsafe { CStr::from_ptr(pointer) }
                    .to_string_lossy()
                    .encode_utf16()
                    .collect::<Vec<_>>()
            }
        };
        string(*left).cmp(&string(*right))
    });
    array
}

#[no_mangle]
/// # Safety
/// `array` must point to a writable Thaw boolean array.
pub unsafe extern "C" fn thaw_bool_array_sort(array: *mut u8) -> *mut u8 {
    let Some(slots) = (unsafe { native_array_slots(array) }) else {
        return std::ptr::null_mut();
    };
    slots.sort_by_key(|slot| *slot != 0);
    array
}

#[no_mangle]
/// # Safety
/// `array` must point to a writable Thaw fixed-object pointer array.
pub unsafe extern "C" fn thaw_object_array_sort(array: *mut u8) -> *mut u8 {
    if (unsafe { native_array_length(array) }).is_none() {
        return std::ptr::null_mut();
    }
    // Every fixed object converts to the same default sort key,
    // "[object Object]". Stable sorting therefore preserves source order.
    array
}

macro_rules! array_to_sorted {
    ($name:ident, $sort:ident) => {
        #[no_mangle]
        /// # Safety
        /// `array` must point to a readable Thaw array of the matching type.
        pub unsafe extern "C" fn $name(array: *const u8) -> *mut u8 {
            let output = unsafe { thaw_array_slice(array, 8, 0.0, f64::INFINITY) };
            if output.is_null() {
                return output;
            }
            unsafe { $sort(output) }
        }
    };
}

array_to_sorted!(thaw_number_array_to_sorted, thaw_number_array_sort);
array_to_sorted!(thaw_string_array_to_sorted, thaw_string_array_sort);
array_to_sorted!(thaw_bool_array_to_sorted, thaw_bool_array_sort);
array_to_sorted!(thaw_object_array_to_sorted, thaw_object_array_sort);

#[no_mangle]
/// # Safety
///
/// `array` must point to a Thaw array containing `f64` element slots.
pub unsafe extern "C" fn thaw_number_array_to_string(array: *const u8) -> *const c_char {
    unsafe { thaw_number_array_join(array, c",".as_ptr()) }
}

#[no_mangle]
/// # Safety
///
/// `array` must point to a Thaw array containing `f64` element slots and
/// `separator` must point to a valid NUL-terminated C string.
pub unsafe extern "C" fn thaw_number_array_join(
    array: *const u8,
    separator: *const c_char,
) -> *const c_char {
    let Some(length) = (unsafe { native_array_length(array) }) else {
        return std::ptr::null();
    };
    if separator.is_null() {
        return std::ptr::null();
    }
    let separator = unsafe { CStr::from_ptr(separator) }.to_string_lossy();
    let mut result = String::new();
    for index in 0..length {
        if index != 0 {
            result.push_str(&separator);
        }
        let slot = unsafe { array.add(8 + index * 8).cast::<f64>().read_unaligned() };
        result.push_str(&javascript_number_string(slot));
    }
    arena_c_string(&result).map_or(std::ptr::null(), |value| value.cast())
}

#[no_mangle]
/// # Safety
///
/// `array` must point to a Thaw array containing C-string pointer slots.
pub unsafe extern "C" fn thaw_string_array_to_string(array: *const u8) -> *const c_char {
    unsafe { thaw_string_array_join(array, c",".as_ptr()) }
}

#[no_mangle]
/// # Safety
///
/// `array` must point to a Thaw array containing C-string pointer slots and
/// `separator` must point to a valid NUL-terminated C string.
pub unsafe extern "C" fn thaw_string_array_join(
    array: *const u8,
    separator: *const c_char,
) -> *const c_char {
    let Some(length) = (unsafe { native_array_length(array) }) else {
        return std::ptr::null();
    };
    if separator.is_null() {
        return std::ptr::null();
    }
    let separator = unsafe { CStr::from_ptr(separator) }.to_string_lossy();
    let mut result = String::new();
    for index in 0..length {
        if index != 0 {
            result.push_str(&separator);
        }
        let slot = unsafe {
            array
                .add(8 + index * 8)
                .cast::<*const c_char>()
                .read_unaligned()
        };
        if !slot.is_null() {
            result.push_str(&unsafe { CStr::from_ptr(slot) }.to_string_lossy());
        }
    }
    arena_c_string(&result).map_or(std::ptr::null(), |value| value.cast())
}

#[no_mangle]
/// # Safety
///
/// `array` must point to a Thaw array containing boolean element slots.
pub unsafe extern "C" fn thaw_bool_array_to_string(array: *const u8) -> *const c_char {
    unsafe { thaw_bool_array_join(array, c",".as_ptr()) }
}

#[no_mangle]
/// # Safety
///
/// `array` must point to a Thaw array containing boolean element slots and
/// `separator` must point to a valid NUL-terminated C string.
pub unsafe extern "C" fn thaw_bool_array_join(
    array: *const u8,
    separator: *const c_char,
) -> *const c_char {
    let Some(length) = (unsafe { native_array_length(array) }) else {
        return std::ptr::null();
    };
    if separator.is_null() {
        return std::ptr::null();
    }
    let separator = unsafe { CStr::from_ptr(separator) }.to_string_lossy();
    let mut result = String::new();
    for index in 0..length {
        if index != 0 {
            result.push_str(&separator);
        }
        let slot = unsafe { array.add(8 + index * 8).read() };
        result.push_str(if slot == 0 { "false" } else { "true" });
    }
    arena_c_string(&result).map_or(std::ptr::null(), |value| value.cast())
}

#[no_mangle]
/// # Safety
///
/// `array` must point to any valid Thaw array. Elements are fixed objects.
pub unsafe extern "C" fn thaw_object_array_to_string(array: *const u8) -> *const c_char {
    unsafe { thaw_object_array_join(array, c",".as_ptr()) }
}

#[no_mangle]
/// # Safety
///
/// `array` must point to any valid Thaw array whose elements are fixed objects,
/// and `separator` must point to a valid NUL-terminated C string.
pub unsafe extern "C" fn thaw_object_array_join(
    array: *const u8,
    separator: *const c_char,
) -> *const c_char {
    let Some(length) = (unsafe { native_array_length(array) }) else {
        return std::ptr::null();
    };
    if separator.is_null() {
        return std::ptr::null();
    }
    let separator = unsafe { CStr::from_ptr(separator) }.to_string_lossy();
    let result = std::iter::repeat_n("[object Object]", length)
        .collect::<Vec<_>>()
        .join(&separator);
    arena_c_string(&result).map_or(std::ptr::null(), |value| value.cast())
}

fn array_search_start(length: usize, from_index: f64) -> usize {
    if from_index.is_nan() || from_index == f64::NEG_INFINITY {
        return 0;
    }
    if from_index == f64::INFINITY {
        return length;
    }
    let index = from_index.trunc();
    if index >= length as f64 {
        length
    } else if index >= 0.0 {
        index as usize
    } else {
        (length as f64 + index).max(0.0) as usize
    }
}

fn array_search_end(length: usize, from_index: f64) -> Option<usize> {
    if length == 0 || from_index == f64::NEG_INFINITY {
        return None;
    }
    if from_index == f64::INFINITY {
        return Some(length - 1);
    }
    let index = if from_index.is_nan() {
        0.0
    } else {
        from_index.trunc()
    };
    if index >= 0.0 {
        Some((index as usize).min(length - 1))
    } else {
        let relative = length as f64 + index;
        (relative >= 0.0).then_some(relative as usize)
    }
}

unsafe fn number_array_search(
    array: *const u8,
    needle: f64,
    from_index: f64,
    same_value_zero: bool,
) -> f64 {
    let Some(length) = (unsafe { native_array_length(array) }) else {
        return -1.0;
    };
    for index in array_search_start(length, from_index)..length {
        let slot = unsafe { array.add(8 + index * 8).cast::<f64>().read_unaligned() };
        if slot == needle || (same_value_zero && slot.is_nan() && needle.is_nan()) {
            return index as f64;
        }
    }
    -1.0
}

#[no_mangle]
/// # Safety
///
/// `array` must point to a Thaw array containing `f64` element slots.
pub unsafe extern "C" fn thaw_number_array_index_of(
    array: *const u8,
    needle: f64,
    from_index: f64,
) -> f64 {
    unsafe { number_array_search(array, needle, from_index, false) }
}

#[no_mangle]
/// # Safety
///
/// `array` must point to a Thaw array containing `f64` element slots.
pub unsafe extern "C" fn thaw_number_array_includes(
    array: *const u8,
    needle: f64,
    from_index: f64,
) -> u8 {
    (unsafe { number_array_search(array, needle, from_index, true) } >= 0.0).into()
}

#[no_mangle]
/// # Safety
/// `array` must point to a Thaw array containing `f64` element slots.
pub unsafe extern "C" fn thaw_number_array_last_index_of(
    array: *const u8,
    needle: f64,
    from_index: f64,
) -> f64 {
    let Some(length) = (unsafe { native_array_length(array) }) else {
        return -1.0;
    };
    let Some(start) = array_search_end(length, from_index) else {
        return -1.0;
    };
    for index in (0..=start).rev() {
        let slot = unsafe { array.add(8 + index * 8).cast::<f64>().read_unaligned() };
        if slot == needle {
            return index as f64;
        }
    }
    -1.0
}

unsafe fn string_array_search(array: *const u8, needle: *const c_char, from_index: f64) -> f64 {
    let Some(length) = (unsafe { native_array_length(array) }) else {
        return -1.0;
    };
    if needle.is_null() {
        return -1.0;
    }
    let needle = unsafe { CStr::from_ptr(needle) }.to_bytes();
    for index in array_search_start(length, from_index)..length {
        let slot = unsafe {
            array
                .add(8 + index * 8)
                .cast::<*const c_char>()
                .read_unaligned()
        };
        if !slot.is_null() && unsafe { CStr::from_ptr(slot) }.to_bytes() == needle {
            return index as f64;
        }
    }
    -1.0
}

#[no_mangle]
/// # Safety
///
/// `array` must point to a Thaw array containing C-string pointer slots and
/// `needle` must point to a valid NUL-terminated C string.
pub unsafe extern "C" fn thaw_string_array_index_of(
    array: *const u8,
    needle: *const c_char,
    from_index: f64,
) -> f64 {
    unsafe { string_array_search(array, needle, from_index) }
}

#[no_mangle]
/// # Safety
///
/// `array` must point to a Thaw array containing C-string pointer slots and
/// `needle` must point to a valid NUL-terminated C string.
pub unsafe extern "C" fn thaw_string_array_includes(
    array: *const u8,
    needle: *const c_char,
    from_index: f64,
) -> u8 {
    (unsafe { string_array_search(array, needle, from_index) } >= 0.0).into()
}

#[no_mangle]
/// # Safety
/// `array` must point to a Thaw array containing C-string pointer slots and
/// `needle` must point to a valid NUL-terminated C string.
pub unsafe extern "C" fn thaw_string_array_last_index_of(
    array: *const u8,
    needle: *const c_char,
    from_index: f64,
) -> f64 {
    let Some(length) = (unsafe { native_array_length(array) }) else {
        return -1.0;
    };
    if needle.is_null() {
        return -1.0;
    }
    let Some(start) = array_search_end(length, from_index) else {
        return -1.0;
    };
    let needle = unsafe { CStr::from_ptr(needle) }.to_bytes();
    for index in (0..=start).rev() {
        let slot = unsafe {
            array
                .add(8 + index * 8)
                .cast::<*const c_char>()
                .read_unaligned()
        };
        if !slot.is_null() && unsafe { CStr::from_ptr(slot) }.to_bytes() == needle {
            return index as f64;
        }
    }
    -1.0
}

unsafe fn bool_array_search(array: *const u8, needle: u8, from_index: f64) -> f64 {
    let Some(length) = (unsafe { native_array_length(array) }) else {
        return -1.0;
    };
    for index in array_search_start(length, from_index)..length {
        let slot = unsafe { array.add(8 + index * 8).read() };
        if (slot != 0) == (needle != 0) {
            return index as f64;
        }
    }
    -1.0
}

#[no_mangle]
/// # Safety
///
/// `array` must point to a Thaw array containing boolean element slots.
pub unsafe extern "C" fn thaw_bool_array_index_of(
    array: *const u8,
    needle: u8,
    from_index: f64,
) -> f64 {
    unsafe { bool_array_search(array, needle, from_index) }
}

#[no_mangle]
/// # Safety
///
/// `array` must point to a Thaw array containing boolean element slots.
pub unsafe extern "C" fn thaw_bool_array_includes(
    array: *const u8,
    needle: u8,
    from_index: f64,
) -> u8 {
    (unsafe { bool_array_search(array, needle, from_index) } >= 0.0).into()
}

#[no_mangle]
/// # Safety
/// `array` must point to a Thaw array containing boolean element slots.
pub unsafe extern "C" fn thaw_bool_array_last_index_of(
    array: *const u8,
    needle: u8,
    from_index: f64,
) -> f64 {
    let Some(length) = (unsafe { native_array_length(array) }) else {
        return -1.0;
    };
    let Some(start) = array_search_end(length, from_index) else {
        return -1.0;
    };
    for index in (0..=start).rev() {
        let slot = unsafe { array.add(8 + index * 8).read() };
        if (slot != 0) == (needle != 0) {
            return index as f64;
        }
    }
    -1.0
}

unsafe fn object_array_search(array: *const u8, needle: *const u8, from_index: f64) -> f64 {
    let Some(length) = (unsafe { native_array_length(array) }) else {
        return -1.0;
    };
    for index in array_search_start(length, from_index)..length {
        let slot = unsafe {
            array
                .add(8 + index * 8)
                .cast::<*const u8>()
                .read_unaligned()
        };
        if slot == needle {
            return index as f64;
        }
    }
    -1.0
}

#[no_mangle]
/// # Safety
/// `array` must point to a Thaw array containing fixed-object pointer slots.
pub unsafe extern "C" fn thaw_object_array_index_of(
    array: *const u8,
    needle: *const u8,
    from_index: f64,
) -> f64 {
    unsafe { object_array_search(array, needle, from_index) }
}

#[no_mangle]
/// # Safety
/// `array` must point to a Thaw array containing fixed-object pointer slots.
pub unsafe extern "C" fn thaw_object_array_includes(
    array: *const u8,
    needle: *const u8,
    from_index: f64,
) -> u8 {
    (unsafe { object_array_search(array, needle, from_index) } >= 0.0).into()
}

#[no_mangle]
/// # Safety
/// `array` must point to a Thaw array containing fixed-object pointer slots.
pub unsafe extern "C" fn thaw_object_array_last_index_of(
    array: *const u8,
    needle: *const u8,
    from_index: f64,
) -> f64 {
    let Some(length) = (unsafe { native_array_length(array) }) else {
        return -1.0;
    };
    let Some(start) = array_search_end(length, from_index) else {
        return -1.0;
    };
    for index in (0..=start).rev() {
        let slot = unsafe {
            array
                .add(8 + index * 8)
                .cast::<*const u8>()
                .read_unaligned()
        };
        if slot == needle {
            return index as f64;
        }
    }
    -1.0
}
