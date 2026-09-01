#[no_mangle]
/// Splits `value` on `separator` into the native `string[]` array layout,
/// matching `String.prototype.split` for a string separator (`RegExp`
/// separators are not supported). An empty separator splits into Unicode
/// scalar values. `limit` truncates the result and is treated as unlimited
/// when not finite or negative.
///
/// # Safety
/// `value` and `separator` must be null or point to valid NUL-terminated
/// UTF-8 strings.
pub unsafe extern "C" fn thaw_string_split(
    value: *const c_char,
    separator: *const c_char,
    limit: f64,
) -> *mut u8 {
    if value.is_null() || separator.is_null() {
        return std::ptr::null_mut();
    }
    let value = unsafe { CStr::from_ptr(value) }.to_string_lossy();
    let separator = unsafe { CStr::from_ptr(separator) }.to_string_lossy();
    let mut parts: Vec<String> = if separator.is_empty() {
        value.chars().map(|character| character.to_string()).collect()
    } else {
        value
            .split(separator.as_ref())
            .map(str::to_string)
            .collect()
    };
    let limit = if limit.is_finite() && limit >= 0.0 {
        limit as usize
    } else {
        usize::MAX
    };
    parts.truncate(limit);
    let output = thaw_arena::thaw_arena_alloc((parts.len() + 1) * 8, 8);
    if output.is_null() {
        return std::ptr::null_mut();
    }
    unsafe {
        output.cast::<u64>().write(parts.len() as u64);
    }
    for (index, part) in parts.into_iter().enumerate() {
        let Some(part) = arena_c_string(&part) else {
            return std::ptr::null_mut();
        };
        unsafe {
            output
                .add(8 + index * 8)
                .cast::<*const u8>()
                .write_unaligned(part);
        }
    }
    output
}

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
/// Fills a primitive array range for the residual JIT.
///
/// # Safety
/// `array` must point to a writable primitive Thaw array matching `operation`.
pub unsafe extern "C" fn thaw_jit_array_fill(
    operation: u8,
    array: *mut u8,
    value: f64,
    start: f64,
    end: f64,
) -> *mut u8 {
    let slot = match operation {
        0 | 1 => value.to_bits(),
        2 => u64::from(value != 0.0),
        _ => return std::ptr::null_mut(),
    };
    unsafe { thaw_array_fill(array, (&slot as *const u64).cast(), 8, start, end) }
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
/// Returns an arena-owned shallow concatenation of two native arrays.
///
/// # Safety
/// Both arrays must point to readable Thaw arrays whose elements occupy
/// `element_width` bytes, and `element_width` must be nonzero.
pub unsafe extern "C" fn thaw_array_concat(
    left: *const u8,
    right: *const u8,
    element_width: usize,
) -> *mut u8 {
    let (Some(left_len), Some(right_len)) = (
        unsafe { native_array_length(left) },
        unsafe { native_array_length(right) },
    ) else {
        return std::ptr::null_mut();
    };
    let Some(length) = left_len.checked_add(right_len) else {
        return std::ptr::null_mut();
    };
    let Some(payload_bytes) = length.checked_mul(element_width) else {
        return std::ptr::null_mut();
    };
    if element_width == 0 {
        return std::ptr::null_mut();
    }
    let output = thaw_arena::thaw_arena_alloc(8 + payload_bytes, element_width.min(8));
    if output.is_null() {
        return output;
    }
    unsafe {
        output.cast::<u64>().write(length as u64);
        std::ptr::copy_nonoverlapping(left.add(8), output.add(8), left_len * element_width);
        std::ptr::copy_nonoverlapping(
            right.add(8),
            output.add(8 + left_len * element_width),
            right_len * element_width,
        );
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

#[no_mangle]
/// `Array.prototype.push`. Appends `count` elements (each `element_width`
/// bytes, read consecutively from `values`) to `array` and returns a fresh
/// buffer with the combined contents; the codegen caller stores the result
/// back into the receiver's handle. `array` may be null, treated as an
/// empty array. Returns null only on allocation failure.
///
/// # Safety
///
/// `array` must be null or point to a readable Thaw array whose elements
/// occupy `element_width` bytes. `values` must be readable for
/// `count * element_width` bytes, and `element_width` must be nonzero.
pub unsafe extern "C" fn thaw_array_push_values(
    array: *const u8,
    element_width: usize,
    values: *const u8,
    count: usize,
) -> *mut u8 {
    let old_len = unsafe { native_array_length(array) }.unwrap_or(0);
    let new_len = old_len + count;
    let Some(payload_bytes) = new_len.checked_mul(element_width) else {
        return std::ptr::null_mut();
    };
    let output = thaw_arena::thaw_arena_alloc(8 + payload_bytes, element_width.min(8));
    if output.is_null() {
        return std::ptr::null_mut();
    }
    unsafe {
        output.cast::<u64>().write(new_len as u64);
        if old_len != 0 {
            std::ptr::copy_nonoverlapping(array.add(8), output.add(8), old_len * element_width);
        }
        if count != 0 {
            std::ptr::copy_nonoverlapping(
                values,
                output.add(8 + old_len * element_width),
                count * element_width,
            );
        }
    }
    output
}

#[no_mangle]
/// Appends one typed primitive value for the residual JIT.
///
/// # Safety
/// `array` must point to a readable primitive Thaw array matching `operation`.
pub unsafe extern "C" fn thaw_jit_array_append(
    operation: u8,
    array: *const u8,
    value: f64,
) -> *mut u8 {
    let slot = match operation {
        0 => value.to_bits(),
        1 => value.to_bits(),
        2 => u64::from(value != 0.0),
        _ => return std::ptr::null_mut(),
    };
    unsafe { thaw_array_push_values(array, 8, (&slot as *const u64).cast(), 1) }
}

#[no_mangle]
/// Appends one primitive value and updates the caller's native array handle.
/// Returns the new length, or `-1` on allocation or input failure.
///
/// # Safety
/// `array` must point to a writable primitive Thaw array handle matching
/// `operation`.
pub unsafe extern "C" fn thaw_jit_array_push(
    operation: u8,
    array: *mut *mut u8,
    value: f64,
) -> f64 {
    let Some(current) = array.as_ref().copied() else {
        return -1.0;
    };
    let output = unsafe { thaw_jit_array_append(operation, current, value) };
    if output.is_null() {
        return -1.0;
    }
    unsafe { array.write(output) };
    unsafe { native_array_length(output) }.map_or(-1.0, |length| length as f64)
}

#[no_mangle]
/// `Array.prototype.unshift`. Prepends `count` elements (each
/// `element_width` bytes, read consecutively from `values`) to `array` and
/// returns a fresh buffer with the combined contents. See
/// `thaw_array_push_values`, which this mirrors.
///
/// # Safety
///
/// Same contract as `thaw_array_push_values`.
pub unsafe extern "C" fn thaw_array_unshift_values(
    array: *const u8,
    element_width: usize,
    values: *const u8,
    count: usize,
) -> *mut u8 {
    let old_len = unsafe { native_array_length(array) }.unwrap_or(0);
    let new_len = old_len + count;
    let Some(payload_bytes) = new_len.checked_mul(element_width) else {
        return std::ptr::null_mut();
    };
    let output = thaw_arena::thaw_arena_alloc(8 + payload_bytes, element_width.min(8));
    if output.is_null() {
        return std::ptr::null_mut();
    }
    unsafe {
        output.cast::<u64>().write(new_len as u64);
        if count != 0 {
            std::ptr::copy_nonoverlapping(values, output.add(8), count * element_width);
        }
        if old_len != 0 {
            std::ptr::copy_nonoverlapping(
                array.add(8),
                output.add(8 + count * element_width),
                old_len * element_width,
            );
        }
    }
    output
}

#[no_mangle]
/// Prepends one primitive value and updates the caller's native array handle.
/// Returns the new length, or `-1` on allocation or input failure.
///
/// # Safety
/// `array` must point to a writable primitive Thaw array handle matching
/// `operation`.
pub unsafe extern "C" fn thaw_jit_array_unshift(
    operation: u8,
    array: *mut *mut u8,
    value: f64,
) -> f64 {
    let Some(current) = array.as_ref().copied() else {
        return -1.0;
    };
    let slot = match operation {
        0 | 1 => value.to_bits(),
        2 => u64::from(value != 0.0),
        _ => return -1.0,
    };
    let output = unsafe {
        thaw_array_unshift_values(current, 8, (&slot as *const u64).cast(), 1)
    };
    if output.is_null() {
        return -1.0;
    }
    unsafe { array.write(output) };
    unsafe { native_array_length(output) }.map_or(-1.0, |length| length as f64)
}

#[no_mangle]
/// `Array.prototype.pop`. Removes the last element of `array`, writing its
/// `element_width` bytes into `out_value` and returning a fresh buffer with
/// the remaining elements. If `array` is null or already empty, `out_value`
/// is zero-filled (codegen substitutes the element type's own zero value in
/// that case, matching `undefined`) and `array` is returned unchanged.
///
/// # Safety
///
/// `array` must be null or point to a readable Thaw array whose elements
/// occupy `element_width` bytes. `out_value` must be writable for
/// `element_width` bytes, and `element_width` must be nonzero.
pub unsafe extern "C" fn thaw_array_pop(
    array: *const u8,
    element_width: usize,
    out_value: *mut u8,
) -> *mut u8 {
    let old_len = unsafe { native_array_length(array) }.unwrap_or(0);
    if old_len == 0 {
        unsafe { out_value.write_bytes(0, element_width) };
        return array as *mut u8;
    }
    let new_len = old_len - 1;
    unsafe {
        std::ptr::copy_nonoverlapping(
            array.add(8 + new_len * element_width),
            out_value,
            element_width,
        );
    }
    let output = thaw_arena::thaw_arena_alloc(8 + new_len * element_width, element_width.min(8));
    if output.is_null() {
        return std::ptr::null_mut();
    }
    unsafe {
        output.cast::<u64>().write(new_len as u64);
        if new_len != 0 {
            std::ptr::copy_nonoverlapping(array.add(8), output.add(8), new_len * element_width);
        }
    }
    output
}

#[no_mangle]
/// `Array.prototype.shift`. Removes the first element of `array`, writing
/// its `element_width` bytes into `out_value` and returning a fresh buffer
/// with the remaining elements, shifted down by one slot. See
/// `thaw_array_pop`, which this mirrors.
///
/// # Safety
///
/// Same contract as `thaw_array_pop`.
pub unsafe extern "C" fn thaw_array_shift(
    array: *const u8,
    element_width: usize,
    out_value: *mut u8,
) -> *mut u8 {
    let old_len = unsafe { native_array_length(array) }.unwrap_or(0);
    if old_len == 0 {
        unsafe { out_value.write_bytes(0, element_width) };
        return array as *mut u8;
    }
    let new_len = old_len - 1;
    unsafe {
        std::ptr::copy_nonoverlapping(array.add(8), out_value, element_width);
    }
    let output = thaw_arena::thaw_arena_alloc(8 + new_len * element_width, element_width.min(8));
    if output.is_null() {
        return std::ptr::null_mut();
    }
    unsafe {
        output.cast::<u64>().write(new_len as u64);
        if new_len != 0 {
            std::ptr::copy_nonoverlapping(
                array.add(8 + element_width),
                output.add(8),
                new_len * element_width,
            );
        }
    }
    output
}

#[no_mangle]
/// Removes and returns one primitive value for residual-JIT `pop`/`shift`.
/// Operations 0..=2 select number/string/boolean `pop`; 3..=5 select the
/// corresponding `shift`. Returns 1 when a value was removed, 0 for an empty
/// array, and -1 on allocation or input failure.
///
/// # Safety
/// `array` must point to a writable primitive Thaw array handle matching
/// `operation`, and `out_value` must be writable for one `f64`.
pub unsafe extern "C" fn thaw_jit_array_remove(
    operation: u8,
    array: *mut *mut u8,
    out_value: *mut f64,
) -> i8 {
    let (Some(current), Some(out_value)) = (array.as_ref().copied(), out_value.as_mut()) else {
        return -1;
    };
    let Some(length) = (unsafe { native_array_length(current) }) else {
        return -1;
    };
    if length == 0 {
        *out_value = 0.0;
        return 0;
    }
    let mut slot = 0_u64;
    let output = match operation {
        0..=2 => unsafe { thaw_array_pop(current, 8, (&mut slot as *mut u64).cast()) },
        3..=5 => unsafe { thaw_array_shift(current, 8, (&mut slot as *mut u64).cast()) },
        _ => return -1,
    };
    if output.is_null() {
        return -1;
    }
    unsafe { array.write(output) };
    *out_value = match operation % 3 {
        0 | 1 => f64::from_bits(slot),
        2 => f64::from(slot != 0),
        _ => unreachable!(),
    };
    1
}

#[no_mangle]
/// `Array.prototype.splice`. Removes the elements of `array` in
/// `[start, start + delete_count)` (both clamped to the array's bounds, as
/// `Array.prototype.slice` clamps its own bounds) and inserts `insert_count`
/// elements (each `element_width` bytes, read consecutively from `values`)
/// in their place, returning a fresh buffer with the spliced contents. The
/// removed elements are written to a second freshly allocated array buffer,
/// whose pointer is written to `*out_removed` (null on allocation failure,
/// matching the overall null-on-failure return).
///
/// # Safety
///
/// `array` must be null or point to a readable Thaw array whose elements
/// occupy `element_width` bytes. `values` must be readable for
/// `insert_count * element_width` bytes, `out_removed` must be writable for
/// one pointer, and `element_width` must be nonzero.
pub unsafe extern "C" fn thaw_array_splice(
    array: *const u8,
    element_width: usize,
    start: f64,
    delete_count: f64,
    values: *const u8,
    insert_count: usize,
    out_removed: *mut *mut u8,
) -> *mut u8 {
    let old_len = unsafe { native_array_length(array) }.unwrap_or(0);
    let start = relative_array_index(start, old_len);
    let delete_count = if delete_count.is_nan() { 0.0 } else { delete_count.max(0.0) };
    let delete_count = (delete_count as usize).min(old_len - start);

    let removed = thaw_arena::thaw_arena_alloc(8 + delete_count * element_width, element_width.min(8));
    if removed.is_null() {
        unsafe { out_removed.write(std::ptr::null_mut()) };
        return std::ptr::null_mut();
    }
    unsafe {
        removed.cast::<u64>().write(delete_count as u64);
        if delete_count != 0 {
            std::ptr::copy_nonoverlapping(
                array.add(8 + start * element_width),
                removed.add(8),
                delete_count * element_width,
            );
        }
        out_removed.write(removed);
    }

    let new_len = old_len - delete_count + insert_count;
    let Some(payload_bytes) = new_len.checked_mul(element_width) else {
        return std::ptr::null_mut();
    };
    let output = thaw_arena::thaw_arena_alloc(8 + payload_bytes, element_width.min(8));
    if output.is_null() {
        return std::ptr::null_mut();
    }
    unsafe {
        output.cast::<u64>().write(new_len as u64);
        if start != 0 {
            std::ptr::copy_nonoverlapping(array.add(8), output.add(8), start * element_width);
        }
        if insert_count != 0 {
            std::ptr::copy_nonoverlapping(
                values,
                output.add(8 + start * element_width),
                insert_count * element_width,
            );
        }
        let tail_start = start + delete_count;
        let tail_len = old_len - tail_start;
        if tail_len != 0 {
            std::ptr::copy_nonoverlapping(
                array.add(8 + tail_start * element_width),
                output.add(8 + (start + insert_count) * element_width),
                tail_len * element_width,
            );
        }
    }
    output
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
/// Shared primitive-array default sorter for the residual JIT.
///
/// # Safety
/// `array` must satisfy the selected typed `toSorted` function's pointer
/// requirements.
pub unsafe extern "C" fn thaw_jit_array_to_sorted(operation: u8, array: *const u8) -> *mut u8 {
    match operation {
        0 => unsafe { thaw_number_array_to_sorted(array) },
        1 => unsafe { thaw_string_array_to_sorted(array) },
        2 => unsafe { thaw_bool_array_to_sorted(array) },
        _ => std::ptr::null_mut(),
    }
}

#[no_mangle]
/// Shared primitive-array in-place sorter for the residual JIT.
///
/// # Safety
/// `array` must point to a writable primitive Thaw array matching `operation`.
pub unsafe extern "C" fn thaw_jit_array_sort(operation: u8, array: *mut u8) -> *mut u8 {
    match operation {
        0 => unsafe { thaw_number_array_sort(array) },
        1 => unsafe { thaw_string_array_sort(array) },
        2 => unsafe { thaw_bool_array_sort(array) },
        _ => std::ptr::null_mut(),
    }
}

#[no_mangle]
/// Returns a primitive-array shallow copy with one element replaced.
/// `operation` selects number, string, or boolean element storage.
///
/// # Safety
/// `array` must point to a readable primitive Thaw array matching `operation`.
pub unsafe extern "C" fn thaw_jit_array_with(
    operation: u8,
    array: *const u8,
    index: f64,
    value: f64,
) -> *mut u8 {
    let Some(length) = (unsafe { native_array_length(array) }) else {
        return std::ptr::null_mut();
    };
    let index = if index.is_nan() { 0.0 } else { index.trunc() };
    let index = if index < 0.0 {
        length as f64 + index
    } else {
        index
    };
    if !index.is_finite() || index < 0.0 || index >= length as f64 || operation > 2 {
        return std::ptr::null_mut();
    }
    let output = unsafe { thaw_array_slice(array, 8, 0.0, f64::INFINITY) };
    if output.is_null() {
        return output;
    }
    let slot = unsafe { output.add(8 + index as usize * 8) };
    unsafe {
        match operation {
            0 => slot.cast::<f64>().write_unaligned(value),
            1 => slot
                .cast::<usize>()
                .write_unaligned(value.to_bits() as usize),
            2 => slot.cast::<u64>().write_unaligned(u64::from(value != 0.0)),
            _ => unreachable!(),
        }
    }
    output
}

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
/// Shared primitive-array string formatter for the residual JIT.
///
/// # Safety
/// `array` and `separator` must satisfy the selected typed `join` function's
/// pointer requirements.
pub unsafe extern "C" fn thaw_jit_array_format(
    operation: u8,
    array: *const u8,
    separator: *const c_char,
) -> *const c_char {
    match operation {
        0 => unsafe { thaw_number_array_join(array, separator) },
        1 => unsafe { thaw_string_array_join(array, separator) },
        2 => unsafe { thaw_bool_array_join(array, separator) },
        _ => std::ptr::null(),
    }
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
/// Shared typed-array search entry point for the residual JIT. `operation`
/// selects number/string/boolean `indexOf` or `includes` without duplicating
/// their JavaScript comparison and `fromIndex` rules in the JIT crate.
///
/// # Safety
/// `array` and a string `needle` must satisfy the selected typed-array
/// operation's requirements.
pub unsafe extern "C" fn thaw_jit_array_search(
    operation: u8,
    array: *const u8,
    needle: f64,
    from_index: f64,
) -> f64 {
    match operation {
        0 => unsafe { thaw_number_array_index_of(array, needle, from_index) },
        1 => f64::from(unsafe { thaw_number_array_includes(array, needle, from_index) }),
        2 => unsafe {
            thaw_string_array_index_of(
                array,
                needle.to_bits() as usize as *const c_char,
                from_index,
            )
        },
        3 => f64::from(unsafe {
            thaw_string_array_includes(
                array,
                needle.to_bits() as usize as *const c_char,
                from_index,
            )
        }),
        4 => unsafe { thaw_bool_array_index_of(array, (needle != 0.0).into(), from_index) },
        5 => f64::from(unsafe {
            thaw_bool_array_includes(array, (needle != 0.0).into(), from_index)
        }),
        6 => unsafe { thaw_number_array_last_index_of(array, needle, from_index) },
        7 => unsafe {
            thaw_string_array_last_index_of(
                array,
                needle.to_bits() as usize as *const c_char,
                from_index,
            )
        },
        8 => unsafe {
            thaw_bool_array_last_index_of(array, (needle != 0.0).into(), from_index)
        },
        _ => -1.0,
    }
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
