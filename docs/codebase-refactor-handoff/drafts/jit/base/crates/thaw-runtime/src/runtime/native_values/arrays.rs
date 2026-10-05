#[no_mangle]
/// Splits `value` on `separator` into the native `string[]` array layout,
/// matching `String.prototype.split` for a string separator (`RegExp`
/// separators are not supported). An empty separator splits into UTF-16
/// code units. `limit` is coerced with ECMAScript's `ToUint32`; `-1` is
/// also the generated code's omitted-limit sentinel and has the same
/// observable result as `u32::MAX` for a native array.
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
    let value = wtf8_decode_utf16(unsafe { wtf8_bytes(value) });
    let separator = wtf8_decode_utf16(unsafe { wtf8_bytes(separator) });
    let mut parts: Vec<Vec<u8>> = if separator.is_empty() {
        value
            .iter()
            .map(|unit| wtf8_encode_utf16(&[*unit]))
            .collect()
    } else {
        let mut parts = Vec::new();
        let mut start = 0;
        let mut index = 0;
        while index + separator.len() <= value.len() {
            if value[index..index + separator.len()] == separator[..] {
                parts.push(wtf8_encode_utf16(&value[start..index]));
                index += separator.len();
                start = index;
            } else {
                index += 1;
            }
        }
        parts.push(wtf8_encode_utf16(&value[start..]));
        parts
    };
    let limit = if limit == -1.0 {
        usize::MAX
    } else if limit.is_finite() {
        limit.trunc().rem_euclid(4_294_967_296.0) as usize
    } else {
        0
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
        let Some(part) = arena_wtf8(&part) else {
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
    let units = wtf8_decode_utf16(unsafe { wtf8_bytes(value) });
    // The string iterator yields one slot per code point: a valid surrogate
    // pair becomes one astral character, a lone surrogate stays one unit.
    let mut characters: Vec<Vec<u8>> = Vec::new();
    let mut index = 0;
    while index < units.len() {
        let unit = units[index];
        if (0xD800..=0xDBFF).contains(&unit)
            && index + 1 < units.len()
            && (0xDC00..=0xDFFF).contains(&units[index + 1])
        {
            characters.push(wtf8_encode_utf16(&[unit, units[index + 1]]));
            index += 2;
        } else {
            characters.push(wtf8_encode_utf16(&[unit]));
            index += 1;
        }
    }
    let output = thaw_arena::thaw_arena_alloc((characters.len() + 1) * 8, 8);
    if output.is_null() {
        return std::ptr::null_mut();
    }
    unsafe {
        output.cast::<u64>().write(characters.len() as u64);
    }
    for (index, character) in characters.into_iter().enumerate() {
        let Some(character) = arena_wtf8(&character) else {
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
pub unsafe extern "C" fn thaw_array_keys(
    array: *const u8,
    presence: *const u8,
    include_length: u8,
) -> *mut u8 {
    let length = if array.is_null() {
        0
    } else {
        unsafe { array.cast::<i64>().read() }.max(0) as usize
    };
    let present = |index| unsafe { array_index_exists(presence, index) };
    let key_count = (0..length).filter(|index| present(*index)).count() + usize::from(include_length != 0);
    let output = thaw_arena::thaw_arena_alloc((key_count + 1) * 8, 8);
    if output.is_null() {
        return output;
    }
    unsafe { output.cast::<i64>().write(key_count as i64) };
    let mut position = 0;
    for index in 0..length {
        if !present(index) {
            continue;
        }
        let Some(key) = arena_c_string(&index.to_string()) else {
            return std::ptr::null_mut();
        };
        unsafe {
            output
                .add(8 + position * 8)
                .cast::<*const u8>()
                .write_unaligned(key)
        };
        position += 1;
    }
    if include_length != 0 {
        unsafe {
            output.add(8 + position * 8).cast::<*const c_char>().write_unaligned(c"length".as_ptr());
        }
    }
    output
}

unsafe fn array_index_exists(presence: *const u8, index: usize) -> bool {
    presence.is_null()
        || index >= unsafe { presence.cast::<u64>().read() as usize }
        || unsafe { presence.add(8 + index).read() != 0 }
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

#[no_mangle]
/// Reverses an array presence mask in place.
///
/// # Safety
/// `presence` must be null or point to a writable Thaw presence mask.
pub unsafe extern "C" fn thaw_array_presence_reverse(presence: *mut u8) -> *mut u8 {
    if presence.is_null() {
        return presence;
    }
    let length = unsafe { presence.cast::<u64>().read() as usize };
    for left in 0..length / 2 {
        unsafe {
            std::ptr::swap(presence.add(8 + left), presence.add(8 + length - left - 1));
        }
    }
    presence
}

#[no_mangle]
/// Converts holes in a presence mask to present `undefined` entries.
///
/// # Safety
/// `presence` must be null or point to a writable Thaw presence mask.
pub unsafe extern "C" fn thaw_array_presence_densify(presence: *mut u8) -> *mut u8 {
    if presence.is_null() {
        return presence;
    }
    let length = unsafe { presence.cast::<u64>().read() as usize };
    for index in 0..length {
        let state = unsafe { presence.add(8 + index) };
        if unsafe { state.read() } == 0 {
            unsafe { state.write(2) };
        }
    }
    presence
}

#[no_mangle]
/// Keeps source holes but marks every visited map result as a concrete value.
///
/// # Safety
/// `presence` must be null or point to a writable Thaw presence mask.
pub unsafe extern "C" fn thaw_array_presence_mapped(presence: *mut u8) -> *mut u8 {
    if !presence.is_null() {
        let length = unsafe { presence.cast::<u64>().read() as usize };
        for index in 0..length {
            let state = unsafe { presence.add(8 + index) };
            if unsafe { state.read() } == 2 {
                unsafe { state.write(1) };
            }
        }
    }
    presence
}

#[no_mangle]
/// Marks undefined payloads in a tagged array before sorting.
/// `mode`: 0 = none, 1 = matching tag, 2 = all elements.
///
/// # Safety
/// `array` is a readable array of 16-byte tagged slots and `presence` is its writable mask or null.
pub unsafe extern "C" fn thaw_array_presence_tagged_sort(
    array: *const u8,
    mut presence: *mut u8,
    undefined_tag: u8,
    mode: u8,
) -> *mut u8 {
    let Some(length) = (unsafe { native_array_length(array) }) else {
        return presence;
    };
    let mask_len = if presence.is_null() { 0 } else { unsafe { presence.cast::<u64>().read() as usize } };
    if presence.is_null()
        && (mode == 0
            || (mode == 1
                && !(0..length)
                    .any(|index| unsafe { array.add(8 + index * 16).read() } == undefined_tag)))
    {
        return presence;
    }
    if mask_len < length {
        let previous = presence;
        presence = thaw_arena::thaw_arena_alloc(8 + length, 1);
        if presence.is_null() {
            return previous;
        }
        unsafe {
            presence.cast::<u64>().write(length as u64);
            std::ptr::write_bytes(presence.add(8), 1, length);
            if !previous.is_null() {
                std::ptr::copy_nonoverlapping(previous.add(8), presence.add(8), mask_len);
            }
        }
    }
    for index in 0..length {
        let state = unsafe { presence.add(8 + index) };
        if mode != 0
            && unsafe { state.read() } == 1
            && (mode == 2 || unsafe { array.add(8 + index * 16).read() } == undefined_tag)
        {
            unsafe { state.write(2) };
        }
    }
    presence
}

#[no_mangle]
/// Compacts concrete values before sorting, followed by undefined entries and holes.
/// Returns the number of concrete values that should be sorted.
///
/// # Safety
/// `array` and `presence` must describe the same writable Thaw array.
pub unsafe extern "C" fn thaw_array_presence_compact(
    array: *mut u8,
    presence: *mut u8,
    element_width: usize,
) -> usize {
    let Some(length) = (unsafe { native_array_length(array) }) else {
        return 0;
    };
    if presence.is_null() || element_width == 0 {
        return length;
    }
    let mask_len = unsafe { presence.cast::<u64>().read() as usize };
    let mut write = 0;
    let mut undefined_count = 0;
    for read in 0..length {
        let state = if read >= mask_len {
            1
        } else {
            unsafe { presence.add(8 + read).read() }
        };
        if state == 1 {
            if write != read {
                unsafe {
                    std::ptr::copy(
                        array.add(8 + read * element_width),
                        array.add(8 + write * element_width),
                        element_width,
                    );
                }
            }
            write += 1;
        } else if state == 2 {
            undefined_count += 1;
        }
    }
    unsafe {
        std::ptr::write_bytes(presence.add(8), 1, write);
        std::ptr::write_bytes(presence.add(8 + write), 2, undefined_count);
        std::ptr::write_bytes(
            presence.add(8 + write + undefined_count),
            0,
            length - write - undefined_count,
        );
    }
    write
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

#[no_mangle]
/// Applies `copyWithin` to an array presence mask.
///
/// # Safety
/// `presence` must be null or point to a writable Thaw presence mask.
pub unsafe extern "C" fn thaw_array_presence_copy_within(
    presence: *mut u8,
    target: f64,
    start: f64,
    end: f64,
) -> *mut u8 {
    if presence.is_null() {
        return presence;
    }
    let length = unsafe { presence.cast::<u64>().read() as usize };
    let target = relative_array_index(target, length);
    let start = relative_array_index(start, length);
    let end = relative_array_index(end, length);
    let count = end.saturating_sub(start).min(length - target);
    if count != 0 {
        unsafe { std::ptr::copy(presence.add(8 + start), presence.add(8 + target), count) };
    }
    presence
}

#[no_mangle]
/// Marks a filled range as present in an array presence mask.
///
/// # Safety
/// `presence` must be null or point to a writable Thaw presence mask.
pub unsafe extern "C" fn thaw_array_presence_fill(
    mut presence: *mut u8,
    length: usize,
    start: f64,
    end: f64,
    state: u8,
) -> *mut u8 {
    let start = relative_array_index(start, length);
    let end = relative_array_index(end, length);
    if end <= start {
        return presence;
    }
    if presence.is_null() {
        presence = unsafe { thaw_array_presence_set_state(presence, length, start, state) };
    }
    if !presence.is_null() {
        let mask_len = unsafe { presence.cast::<u64>().read() as usize };
        if start < mask_len {
            unsafe { std::ptr::write_bytes(presence.add(8 + start), state, end.min(mask_len) - start) };
        }
    }
    presence
}

#[no_mangle]
/// Marks one indexed assignment as present.
///
/// # Safety
/// `presence` must be null or point to a writable Thaw presence mask.
pub unsafe extern "C" fn thaw_array_presence_mark(presence: *mut u8, index: usize) {
    if presence.is_null() {
        return;
    }
    let length = unsafe { presence.cast::<u64>().read() as usize };
    if index < length {
        unsafe { presence.add(8 + index).write(1) };
    }
}

#[no_mangle]
/// Writes an exact array element state, allocating a sidecar for a dense array if needed.
///
/// # Safety
/// `presence` must be null or point to a writable Thaw presence mask.
pub unsafe extern "C" fn thaw_array_presence_set_state(
    mut presence: *mut u8,
    length: usize,
    index: usize,
    state: u8,
) -> *mut u8 {
    if index >= length {
        return presence;
    }
    if presence.is_null() {
        if state == 1 {
            return presence;
        }
        presence = thaw_arena::thaw_arena_alloc(8 + length, 1);
        if presence.is_null() {
            return presence;
        }
        unsafe {
            presence.cast::<u64>().write(length as u64);
            std::ptr::write_bytes(presence.add(8), 1, length);
        }
    }
    if index < unsafe { presence.cast::<u64>().read() as usize } {
        unsafe { presence.add(8 + index).write(state) };
    }
    presence
}

/// 16 zeroed bytes wide enough for any array element slot (`ASYNC_SLOT_BYTES`
/// in thaw-llvm), returned in place of a real element pointer for an
/// out-of-bounds/negative/non-integer read so a typed load past the end of
/// an array's arena allocation can't happen -- `arr[oob]` reads garbage
/// heap bytes instead of `undefined` (a wider-callers gap tracked
/// separately), but it must never read outside the array's own buffer.
static ARRAY_READ_SCRATCH: [u8; 16] = [0; 16];

#[no_mangle]
/// Returns a pointer to `array`'s element at `index`, or to a zeroed
/// scratch slot when `index` is out of bounds, negative, or non-integer.
/// Used for reads only; see `thaw_array_ensure_index` for the write path,
/// which grows the array instead.
///
/// # Safety
/// `array` must be null or point to a valid `[length][elem...]` native
/// array buffer.
pub unsafe extern "C" fn thaw_array_read_ptr(
    array: *const u8,
    element_width: usize,
    index: f64,
) -> *const u8 {
    if !index.is_finite() || index < 0.0 || index.fract() != 0.0 {
        return ARRAY_READ_SCRATCH.as_ptr();
    }
    let Some(length) = (unsafe { native_array_length(array) }) else {
        return ARRAY_READ_SCRATCH.as_ptr();
    };
    let index = index as usize;
    if index >= length {
        return ARRAY_READ_SCRATCH.as_ptr();
    }
    unsafe { array.add(8 + index * element_width) }
}

#[no_mangle]
/// Ensures an indexed write has storage, leaving skipped slots as holes.
///
/// # Safety
/// `handle` must point to a writable two-pointer native array handle.
pub unsafe extern "C" fn thaw_array_ensure_index(
    handle: *mut u8,
    element_width: usize,
    index: f64,
) -> *mut u8 {
    // ponytail: non-index array properties need an object-property side table;
    // this native buffer handles only canonical array indices.
    if handle.is_null() || element_width == 0 || !index.is_finite()
        || index < 0.0 || index.fract() != 0.0 || index > (u32::MAX as f64 - 1.0)
    {
        return std::ptr::null_mut();
    }
    let index = index as usize;
    let array = unsafe { handle.cast::<*mut u8>().read() };
    let Some(old_len) = (unsafe { native_array_length(array) }) else {
        return std::ptr::null_mut();
    };
    if index < old_len {
        return unsafe { array.add(8 + index * element_width) };
    }
    let new_len = index + 1;
    let Some(bytes) = new_len.checked_mul(element_width).and_then(|bytes| bytes.checked_add(8)) else {
        return std::ptr::null_mut();
    };
    let new_array = thaw_arena::thaw_arena_alloc(bytes, element_width.min(8));
    let new_mask = thaw_arena::thaw_arena_alloc(8 + new_len, 1);
    if new_array.is_null() || new_mask.is_null() {
        return std::ptr::null_mut();
    }
    let old_mask = unsafe { handle.add(8).cast::<*mut u8>().read() };
    unsafe {
        new_array.cast::<u64>().write(new_len as u64);
        std::ptr::copy_nonoverlapping(array.add(8), new_array.add(8), old_len * element_width);
        std::ptr::write_bytes(new_array.add(8 + old_len * element_width), 0, (new_len - old_len) * element_width);
        new_mask.cast::<u64>().write(new_len as u64);
        std::ptr::write_bytes(new_mask.add(8), 1, old_len);
        if !old_mask.is_null() {
            let mask_len = old_mask.cast::<u64>().read() as usize;
            std::ptr::copy_nonoverlapping(old_mask.add(8), new_mask.add(8), old_len.min(mask_len));
        }
        std::ptr::write_bytes(new_mask.add(8 + old_len), 0, new_len - old_len);
        handle.cast::<*mut u8>().write(new_array);
        handle.add(8).cast::<*mut u8>().write(new_mask);
        new_array.add(8 + index * element_width)
    }
}

#[no_mangle]
/// Resizes a native array in place, dropping truncated slots or adding holes.
///
/// # Safety
/// `handle` must point to a writable two-pointer native array handle.
pub unsafe extern "C" fn thaw_array_resize(
    handle: *mut u8,
    element_width: usize,
    new_length: f64,
) -> *mut u8 {
    if handle.is_null() || element_width == 0 || !new_length.is_finite()
        || new_length < 0.0 || new_length.fract() != 0.0 || new_length > u32::MAX as f64
    {
        return std::ptr::null_mut();
    }
    let array = unsafe { handle.cast::<*mut u8>().read() };
    let Some(old_len) = (unsafe { native_array_length(array) }) else {
        return std::ptr::null_mut();
    };
    let new_len = new_length as usize;
    if new_len == old_len {
        return handle;
    }
    if new_len > old_len {
        return if (unsafe { thaw_array_ensure_index(handle, element_width, (new_len - 1) as f64) }).is_null() {
            std::ptr::null_mut()
        } else {
            handle
        };
    }
    let Some(bytes) = new_len.checked_mul(element_width).and_then(|bytes| bytes.checked_add(8)) else {
        return std::ptr::null_mut();
    };
    let new_array = thaw_arena::thaw_arena_alloc(bytes, element_width.min(8));
    if new_array.is_null() {
        return std::ptr::null_mut();
    }
    let old_mask = unsafe { handle.add(8).cast::<*mut u8>().read() };
    let new_mask = if old_mask.is_null() {
        std::ptr::null_mut()
    } else {
        thaw_arena::thaw_arena_alloc(8 + new_len, 1)
    };
    if !old_mask.is_null() && new_mask.is_null() {
        return std::ptr::null_mut();
    }
    unsafe {
        new_array.cast::<u64>().write(new_len as u64);
        std::ptr::copy_nonoverlapping(array.add(8), new_array.add(8), new_len * element_width);
        if !new_mask.is_null() {
            new_mask.cast::<u64>().write(new_len as u64);
            std::ptr::write_bytes(new_mask.add(8), 1, new_len);
            let mask_len = old_mask.cast::<u64>().read() as usize;
            std::ptr::copy_nonoverlapping(old_mask.add(8), new_mask.add(8), new_len.min(mask_len));
        }
        handle.cast::<*mut u8>().write(new_array);
        handle.add(8).cast::<*mut u8>().write(new_mask);
    }
    handle
}

#[no_mangle]
/// Resizes a presence mask after push/unshift.
///
/// # Safety
/// `presence` must be null or point to a readable Thaw presence mask.
pub unsafe extern "C" fn thaw_array_presence_extend(
    presence: *const u8,
    old_len: usize,
    count: usize,
    prepend: u8,
    insert_states: *const u8,
) -> *mut u8 {
    let has_undefined = !insert_states.is_null()
        && (0..count).any(|index| unsafe { insert_states.add(index).read() != 1 });
    if presence.is_null() && !has_undefined {
        return std::ptr::null_mut();
    }
    let mask_len = if presence.is_null() { 0 } else { unsafe { presence.cast::<u64>().read() as usize } };
    let new_len = old_len + count;
    let output = thaw_arena::thaw_arena_alloc(8 + new_len, 1);
    if output.is_null() {
        return output;
    }
    unsafe {
        output.cast::<u64>().write(new_len as u64);
        std::ptr::write_bytes(output.add(8), 1, new_len);
        let destination = if prepend != 0 { count } else { 0 };
        if mask_len != 0 {
            std::ptr::copy_nonoverlapping(
                presence.add(8),
                output.add(8 + destination),
                old_len.min(mask_len),
            );
        }
        if !insert_states.is_null() && count != 0 {
            let offset = if prepend != 0 { 0 } else { old_len };
            std::ptr::copy_nonoverlapping(insert_states, output.add(8 + offset), count);
        }
    }
    output
}

#[no_mangle]
/// Resizes a presence mask after pop/shift.
///
/// # Safety
/// `presence` must be null or point to a readable Thaw presence mask.
pub unsafe extern "C" fn thaw_array_presence_remove(
    presence: *const u8,
    old_len: usize,
    shift: u8,
) -> *mut u8 {
    if presence.is_null() || old_len == 0 {
        return presence.cast_mut();
    }
    let mask_len = unsafe { presence.cast::<u64>().read() as usize };
    let new_len = old_len - 1;
    let output = thaw_arena::thaw_arena_alloc(8 + new_len, 1);
    if output.is_null() {
        return output;
    }
    unsafe {
        output.cast::<u64>().write(new_len as u64);
        std::ptr::write_bytes(output.add(8), 1, new_len);
        let source = usize::from(shift != 0);
        let available = mask_len.saturating_sub(source).min(new_len);
        std::ptr::copy_nonoverlapping(presence.add(8 + source), output.add(8), available);
    }
    output
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
/// Returns the presence mask corresponding to an array slice.
///
/// # Safety
/// `presence` must be null for a dense array or point to a readable Thaw
/// presence mask containing a `u64` length followed by one byte per entry.
pub unsafe extern "C" fn thaw_array_presence_slice(
    presence: *const u8,
    array_length: usize,
    start: f64,
    end: f64,
) -> *mut u8 {
    if presence.is_null() {
        return std::ptr::null_mut();
    }
    let start = relative_array_index(start, array_length);
    let end = relative_array_index(end, array_length);
    let count = end.saturating_sub(start);
    let output = thaw_arena::thaw_arena_alloc(8 + count, 1);
    if output.is_null() {
        return std::ptr::null_mut();
    }
    let mask_length = unsafe { presence.cast::<u64>().read() as usize };
    unsafe {
        output.cast::<u64>().write(count as u64);
        for index in 0..count {
            let source = start + index;
            output
                .add(8 + index)
                .write(if source < mask_length {
                    presence.add(8 + source).read()
                } else {
                    1
                });
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
/// Assigns one primitive slot for residual JIT code, growing the shared array
/// handle and zero-filling any gap when `index` is beyond the current end.
///
/// # Safety
/// `array` must point to a writable number/string/boolean Thaw array handle
/// matching `operation` (0/1/2).
pub unsafe extern "C" fn thaw_jit_array_set(
    operation: u8,
    array: *mut *mut u8,
    index: f64,
    value: f64,
) -> i8 {
    let Some(current) = (unsafe { array.as_ref() }).copied() else {
        return -1;
    };
    let Some(length) = (unsafe { native_array_length(current) }) else {
        return -1;
    };
    if !index.is_finite() || index < 0.0 || index.fract() != 0.0 || index > usize::MAX as f64 {
        return -1;
    }
    let index = index as usize;
    let slot = match operation {
        0 | 1 => value.to_bits(),
        2 => u64::from(value != 0.0),
        _ => return -1,
    };
    if index < length {
        unsafe { current.add(8 + index * 8).cast::<u64>().write_unaligned(slot) };
        return 1;
    }
    let Some(bytes) = index
        .checked_add(1)
        .and_then(|length| length.checked_mul(8))
        .and_then(|bytes| bytes.checked_add(8))
    else {
        return -1;
    };
    let output = thaw_arena::thaw_arena_alloc(bytes, 8);
    if output.is_null() {
        return -1;
    }
    unsafe {
        output.cast::<u64>().write((index + 1) as u64);
        std::ptr::copy_nonoverlapping(current.add(8), output.add(8), length * 8);
        output.add(8 + length * 8).write_bytes(0, (index - length) * 8);
        output.add(8 + index * 8).cast::<u64>().write_unaligned(slot);
        array.write(output);
    }
    1
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

#[no_mangle]
/// Rebuilds the receiver and removed-value presence masks for `splice`.
///
/// # Safety
/// `presence` must be null or point to a readable Thaw presence mask and
/// `out_removed` must be writable for one pointer.
pub unsafe extern "C" fn thaw_array_presence_splice(
    presence: *const u8,
    old_len: usize,
    start: f64,
    delete_count: f64,
    insert_count: usize,
    insert_states: *const u8,
    out_removed: *mut *mut u8,
) -> *mut u8 {
    let has_undefined = !insert_states.is_null()
        && (0..insert_count).any(|index| unsafe { insert_states.add(index).read() != 1 });
    if presence.is_null() && !has_undefined {
        unsafe { out_removed.write(std::ptr::null_mut()) };
        return std::ptr::null_mut();
    }
    let mask_len = if presence.is_null() { 0 } else { unsafe { presence.cast::<u64>().read() as usize } };
    let start = relative_array_index(start, old_len);
    let delete_count = if delete_count.is_nan() {
        0
    } else {
        (delete_count.max(0.0) as usize).min(old_len - start)
    };
    let new_len = old_len - delete_count + insert_count;
    let removed = thaw_arena::thaw_arena_alloc(8 + delete_count, 1);
    let output = thaw_arena::thaw_arena_alloc(8 + new_len, 1);
    if removed.is_null() || output.is_null() {
        unsafe { out_removed.write(std::ptr::null_mut()) };
        return std::ptr::null_mut();
    }
    let present = |index: usize| unsafe {
        if index < mask_len {
            presence.add(8 + index).read()
        } else {
            1
        }
    };
    unsafe {
        removed.cast::<u64>().write(delete_count as u64);
        for index in 0..delete_count {
            removed.add(8 + index).write(present(start + index));
        }
        output.cast::<u64>().write(new_len as u64);
        for index in 0..start {
            output.add(8 + index).write(present(index));
        }
        if insert_count != 0 {
            if insert_states.is_null() {
                std::ptr::write_bytes(output.add(8 + start), 1, insert_count);
            } else {
                std::ptr::copy_nonoverlapping(insert_states, output.add(8 + start), insert_count);
            }
        }
        let tail_start = start + delete_count;
        for index in tail_start..old_len {
            output
                .add(8 + start + insert_count + index - tail_start)
                .write(present(index));
        }
        out_removed.write(removed);
    }
    output
}

#[no_mangle]
/// Mutates a primitive array handle for residual-JIT `splice` and returns the
/// removed primitive array.
///
/// # Safety
/// `array` must point to a writable Thaw primitive-array handle and `inserts`
/// must point to a readable Thaw primitive array with the same element type.
pub unsafe extern "C" fn thaw_jit_array_splice(
    array: *mut *mut u8,
    start: f64,
    delete_count: f64,
    inserts: *const u8,
) -> *mut u8 {
    let (Some(current), Some(insert_count)) = (
        unsafe { array.as_ref() }.copied(),
        unsafe { native_array_length(inserts) },
    ) else {
        return std::ptr::null_mut();
    };
    let mut removed = std::ptr::null_mut();
    let output = unsafe {
        thaw_array_splice(
            current,
            8,
            start,
            delete_count,
            inserts.add(8),
            insert_count,
            &mut removed,
        )
    };
    if output.is_null() {
        return std::ptr::null_mut();
    }
    unsafe { array.write(output) };
    removed
}

unsafe fn native_array_slots(array: *mut u8) -> Option<&'static mut [u64]> {
    let length = unsafe { native_array_length(array) }?;
    Some(unsafe { std::slice::from_raw_parts_mut(array.add(8).cast::<u64>(), length) })
}

unsafe fn native_string_sort_units(value: u64) -> Vec<u16> {
    let pointer = value as usize as *const c_char;
    if pointer.is_null() {
        Vec::new()
    } else {
        wtf8_decode_utf16(unsafe { wtf8_bytes(pointer) })
    }
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

unsafe fn thaw_number_array_numeric_sort(array: *mut u8, descending: bool) -> *mut u8 {
    let Some(slots) = (unsafe { native_array_slots(array) }) else {
        return std::ptr::null_mut();
    };
    slots.sort_by(|left, right| {
        let left = f64::from_bits(*left);
        let right = f64::from_bits(*right);
        let difference = if descending {
            right - left
        } else {
            left - right
        };
        if difference.is_nan() || difference == 0.0 {
            std::cmp::Ordering::Equal
        } else if difference < 0.0 {
            std::cmp::Ordering::Less
        } else {
            std::cmp::Ordering::Greater
        }
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
        unsafe { native_string_sort_units(*left) }.cmp(&unsafe { native_string_sort_units(*right) })
    });
    array
}

unsafe fn thaw_string_array_sort_descending(array: *mut u8) -> *mut u8 {
    let Some(slots) = (unsafe { native_array_slots(array) }) else {
        return std::ptr::null_mut();
    };
    slots.sort_by(|left, right| {
        unsafe { native_string_sort_units(*right) }.cmp(&unsafe { native_string_sort_units(*left) })
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
        3..=5 => {
            let output = unsafe { thaw_array_slice(array, 8, 0.0, f64::INFINITY) };
            if output.is_null() {
                output
            } else if operation == 5 {
                unsafe { thaw_string_array_sort_descending(output) }
            } else {
                unsafe { thaw_number_array_numeric_sort(output, operation == 4) }
            }
        }
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
        3 | 4 => unsafe { thaw_number_array_numeric_sort(array, operation == 4) },
        5 => unsafe { thaw_string_array_sort_descending(array) },
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
pub unsafe extern "C" fn thaw_number_array_to_string(
    array: *const u8,
    presence: *const u8,
) -> *const c_char {
    unsafe { thaw_number_array_join(array, presence, c",".as_ptr()) }
}

#[no_mangle]
/// # Safety
///
/// `array` must point to a Thaw array containing `f64` element slots and
/// `separator` must point to a valid NUL-terminated C string.
pub unsafe extern "C" fn thaw_number_array_join(
    array: *const u8,
    presence: *const u8,
    separator: *const c_char,
) -> *const c_char {
    let Some(length) = (unsafe { native_array_length(array) }) else {
        return std::ptr::null();
    };
    if separator.is_null() {
        return std::ptr::null();
    }
    let separator = unsafe { wtf8_bytes(separator) };
    let mut result = Vec::new();
    for index in 0..length {
        if index != 0 {
            result.extend_from_slice(separator);
        }
        if !unsafe { array_index_present(presence, index) } {
            continue;
        }
        let slot = unsafe { array.add(8 + index * 8).cast::<f64>().read_unaligned() };
        result.extend_from_slice(javascript_number_string(slot).as_bytes());
    }
    arena_wtf8(&result).map_or(std::ptr::null(), |value| value.cast())
}

#[no_mangle]
/// # Safety
///
/// `array` must point to a Thaw array containing C-string pointer slots.
pub unsafe extern "C" fn thaw_string_array_to_string(
    array: *const u8,
    presence: *const u8,
) -> *const c_char {
    unsafe { thaw_string_array_join(array, presence, c",".as_ptr()) }
}

#[no_mangle]
/// # Safety
///
/// `array` must point to a Thaw array containing C-string pointer slots and
/// `separator` must point to a valid NUL-terminated C string.
pub unsafe extern "C" fn thaw_string_array_join(
    array: *const u8,
    presence: *const u8,
    separator: *const c_char,
) -> *const c_char {
    let Some(length) = (unsafe { native_array_length(array) }) else {
        return std::ptr::null();
    };
    if separator.is_null() {
        return std::ptr::null();
    }
    // WTF-8-preserving: concatenate raw bytes so a lone surrogate in an
    // element round-trips (the display path lossily renders it, not `join`).
    let separator = unsafe { wtf8_bytes(separator) };
    let mut result: Vec<u8> = Vec::new();
    for index in 0..length {
        if index != 0 {
            result.extend_from_slice(separator);
        }
        if !unsafe { array_index_present(presence, index) } {
            continue;
        }
        let slot = unsafe {
            array
                .add(8 + index * 8)
                .cast::<*const c_char>()
                .read_unaligned()
        };
        if !slot.is_null() {
            result.extend_from_slice(unsafe { wtf8_bytes(slot) });
        }
    }
    arena_wtf8(&result).map_or(std::ptr::null(), |value| value.cast())
}

#[no_mangle]
/// # Safety
///
/// `array` must point to a Thaw array containing boolean element slots.
pub unsafe extern "C" fn thaw_bool_array_to_string(
    array: *const u8,
    presence: *const u8,
) -> *const c_char {
    unsafe { thaw_bool_array_join(array, presence, c",".as_ptr()) }
}

#[no_mangle]
/// # Safety
///
/// `array` must point to a Thaw array containing boolean element slots and
/// `separator` must point to a valid NUL-terminated C string.
pub unsafe extern "C" fn thaw_bool_array_join(
    array: *const u8,
    presence: *const u8,
    separator: *const c_char,
) -> *const c_char {
    let Some(length) = (unsafe { native_array_length(array) }) else {
        return std::ptr::null();
    };
    if separator.is_null() {
        return std::ptr::null();
    }
    let separator = unsafe { wtf8_bytes(separator) };
    let mut result = Vec::new();
    for index in 0..length {
        if index != 0 {
            result.extend_from_slice(separator);
        }
        if !unsafe { array_index_present(presence, index) } {
            continue;
        }
        let slot = unsafe { array.add(8 + index * 8).read() };
        result.extend_from_slice(if slot == 0 { &b"false"[..] } else { &b"true"[..] });
    }
    arena_wtf8(&result).map_or(std::ptr::null(), |value| value.cast())
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
        0 => unsafe { thaw_number_array_join(array, std::ptr::null(), separator) },
        1 => unsafe { thaw_string_array_join(array, std::ptr::null(), separator) },
        2 => unsafe { thaw_bool_array_join(array, std::ptr::null(), separator) },
        _ => std::ptr::null(),
    }
}

#[no_mangle]
/// # Safety
///
/// `array` must point to any valid Thaw array. Elements are fixed objects.
pub unsafe extern "C" fn thaw_object_array_to_string(
    array: *const u8,
    presence: *const u8,
) -> *const c_char {
    unsafe { thaw_object_array_join(array, presence, c",".as_ptr()) }
}

#[no_mangle]
/// # Safety
///
/// `array` must point to any valid Thaw array whose elements are fixed objects,
/// and `separator` must point to a valid NUL-terminated C string.
pub unsafe extern "C" fn thaw_object_array_join(
    array: *const u8,
    presence: *const u8,
    separator: *const c_char,
) -> *const c_char {
    let Some(length) = (unsafe { native_array_length(array) }) else {
        return std::ptr::null();
    };
    if separator.is_null() {
        return std::ptr::null();
    }
    let separator = unsafe { wtf8_bytes(separator) };
    let mut result = Vec::new();
    for index in 0..length {
        if index != 0 {
            result.extend_from_slice(separator);
        }
        if unsafe { array_index_present(presence, index) } {
            result.extend_from_slice(b"[object Object]");
        }
    }
    arena_wtf8(&result).map_or(std::ptr::null(), |value| value.cast())
}

#[no_mangle]
/// # Safety
/// `array` and `presence` must describe a Thaw array; `separator` must be a C string.
pub unsafe extern "C" fn thaw_tagged_array_join(
    array: *const u8,
    presence: *const u8,
    separator: *const c_char,
    kind: u8,
    value_tag: u8,
    tags: *const c_char,
) -> *const c_char {
    let Some(length) = (unsafe { native_array_length(array) }) else {
        return std::ptr::null();
    };
    if separator.is_null() {
        return std::ptr::null();
    }
    if kind == 5 && tags.is_null() {
        return std::ptr::null();
    }
    let tags = if kind == 5 { unsafe { CStr::from_ptr(tags) }.to_bytes() } else { &[] };
    let separator = unsafe { wtf8_bytes(separator) };
    let mut result: Vec<u8> = Vec::new();
    for index in 0..length {
        if index != 0 {
            result.extend_from_slice(separator);
        }
        if kind == 4 || !unsafe { array_index_present(presence, index) } {
            continue;
        }
        let slot = unsafe { array.add(8 + index * 16) };
        let tag = unsafe { slot.read() };
        if kind != 5 && tag != value_tag {
            continue;
        }
        let payload = unsafe { slot.add(if kind == 2 { 1 } else { 8 }) };
        let value_kind = if kind == 5 {
            tags.get(tag as usize).copied().unwrap_or(b'u')
        } else {
            match kind { 0 => b'n', 1 => b's', 2 => b'b', 3 => b'o', _ => b'u' }
        };
        match value_kind {
            b'n' => result.extend_from_slice(
                javascript_number_string(unsafe { payload.cast::<f64>().read_unaligned() })
                    .as_bytes(),
            ),
            b's' => {
                let string = unsafe { payload.cast::<*const c_char>().read_unaligned() };
                if !string.is_null() {
                    result.extend_from_slice(unsafe { wtf8_bytes(string) });
                }
            }
            b'b' => result.extend_from_slice(if unsafe { payload.read() } == 0 {
                b"false"
            } else {
                b"true"
            }),
            b'o' => result.extend_from_slice(b"[object Object]"),
            _ => {}
        }
    }
    arena_wtf8(&result).map_or(std::ptr::null(), |value| value.cast())
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

unsafe fn array_index_present(presence: *const u8, index: usize) -> bool {
    presence.is_null()
        || index >= unsafe { presence.cast::<u64>().read() as usize }
        || unsafe { presence.add(8 + index).read() == 1 }
}

fn canonical_array_property_index(key: &str) -> Option<usize> {
    let index = key.parse::<u32>().ok()?;
    (key == index.to_string()).then_some(index as usize)
}

#[no_mangle]
/// Deletes an own native-array property without changing its length.
/// Returns 0 for a non-configurable own property, 1 for success, and 2 when
/// a dense-to-sparse mask allocation fails.
///
/// # Safety
/// `handle` must point to a writable two-pointer native array handle, and
/// `key` to a valid NUL-terminated string.
pub unsafe extern "C" fn thaw_array_delete_property(
    handle: *mut u8,
    key: *const c_char,
) -> u8 {
    if handle.is_null() || key.is_null() {
        return 0;
    }
    let Ok(key) = (unsafe { CStr::from_ptr(key) }).to_str() else {
        return 1;
    };
    if key == "length" {
        return 0;
    }
    let Some(index) = canonical_array_property_index(key) else {
        return 1;
    };
    let array = unsafe { handle.cast::<*mut u8>().read() };
    let Some(length) = (unsafe { native_array_length(array) }) else {
        return 0;
    };
    if index >= length {
        return 1;
    }
    let presence_slot = unsafe { handle.add(8).cast::<*mut u8>() };
    let mut presence = unsafe { presence_slot.read() };
    if !unsafe { array_index_exists(presence, index) } {
        return 1;
    }
    if thaw_object_state(handle, 1) {
        return 0;
    }
    let mask_length = if presence.is_null() {
        0
    } else {
        unsafe { presence.cast::<u64>().read() as usize }
    };
    if index >= mask_length {
        // Both the dense-mask setter and the short-mask extender allocate
        // `8 + length` bytes; check before either reaches its unchecked sum.
        if length.checked_add(8).is_none() {
            return 2;
        }
        if !presence.is_null() {
            // A live index beyond an older mask defaults to present. Extend
            // privately before writing state 0; a failed allocation leaves
            // the handle's original mask and its holes untouched.
            presence = unsafe {
                thaw_array_presence_extend(
                    presence, mask_length, length - mask_length, 0, std::ptr::null(),
                )
            };
            if presence.is_null() {
                return 2;
            }
        }
    }
    let updated = unsafe { thaw_array_presence_set_state(presence, length, index, 0) };
    if updated.is_null() {
        return 2;
    }
    unsafe { presence_slot.write(updated) };
    1
}

#[no_mangle]
/// # Safety
/// `array` is a readable Thaw array, `presence` its mask or null, and `key` a C string.
pub unsafe extern "C" fn thaw_array_has_property(
    array: *const u8,
    presence: *const u8,
    key: *const c_char,
    mode: u8,
) -> u8 {
    if key.is_null() {
        return 0;
    }
    let Ok(key) = (unsafe { CStr::from_ptr(key) }).to_str() else {
        return 0;
    };
    if key == "length" {
        return u8::from(mode != 2);
    }
    if mode == 0 && matches!(key,
        "\u{1f}@@iterator" | "\u{1f}@@unscopables"
        |
        "at" | "concat" | "copyWithin" | "entries" | "every" | "fill"
        | "filter" | "find" | "findIndex" | "findLast" | "findLastIndex"
        | "flat" | "flatMap" | "forEach" | "includes" | "indexOf"
        | "join" | "keys" | "lastIndexOf" | "map" | "pop" | "push"
        | "reduce" | "reduceRight" | "reverse" | "shift" | "slice"
        | "some" | "sort" | "splice" | "toLocaleString" | "toReversed"
        | "toSorted" | "toSpliced" | "toString" | "unshift" | "values"
        | "with" | "constructor" | "valueOf" | "hasOwnProperty"
        | "isPrototypeOf" | "propertyIsEnumerable" | "__proto__"
        | "__defineGetter__" | "__defineSetter__" | "__lookupGetter__"
        | "__lookupSetter__"
    ) {
        return 1;
    }
    let Some(index) = canonical_array_property_index(key) else {
        return 0;
    };
    if unsafe { native_array_length(array) }.is_none_or(|length| index >= length) {
        return 0;
    }
    u8::from(unsafe { array_index_exists(presence, index) })
}

#[no_mangle]
/// Finds an `undefined` element, optionally treating holes as undefined for includes.
///
/// # Safety
/// `array` must point to a Thaw array and `presence` to its mask or null.
pub unsafe extern "C" fn thaw_array_undefined_index_of(
    array: *const u8,
    presence: *const u8,
    from_index: f64,
    reverse: u8,
    element_kind: u8,
    include_holes: u8,
    union_tag: u8,
) -> f64 {
    let Some(length) = (unsafe { native_array_length(array) }) else {
        return -1.0;
    };
    let matches_undefined = |index: &usize| {
        let state = if !presence.is_null()
            && *index < unsafe { presence.cast::<u64>().read() as usize }
        {
            unsafe { presence.add(8 + *index).read() }
        } else {
            1
        };
        if state != 1 {
            return (state == 2 && (element_kind <= 3 || element_kind == 7))
                || (state == 0 && include_holes != 0);
        }
        match element_kind {
            1 => true,
            2 => (unsafe { array.add(8 + *index * 16).read() }) == 0,
            3 => (unsafe { array.add(8 + *index * 16).read() }) == 2,
            4 => (unsafe { array.add(8 + *index * 16).read() }) == 0,
            5 => (unsafe { array.add(8 + *index * 16).read() }) == 1,
            6 => true,
            7 | 8 => (unsafe { array.add(8 + *index * 16).read() }) == union_tag,
            _ => false,
        }
    };
    let found = if reverse == 0 {
        (array_search_start(length, from_index)..length).find(matches_undefined)
    } else {
        array_search_end(length, from_index)
            .and_then(|end| (0..=end).rev().find(matches_undefined))
    };
    found.map_or(-1.0, |index| index as f64)
}

#[no_mangle]
/// Searches concrete payloads in arrays of tagged values.
///
/// # Safety
/// `array`, `presence`, and `needle` must describe readable Thaw values of `kind`.
pub unsafe extern "C" fn thaw_tagged_array_index_of(
    array: *const u8,
    presence: *const u8,
    needle: *const u8,
    from_index: f64,
    reverse: u8,
    includes: u8,
    kind: u8,
    value_tag: u8,
    union_layout: u8,
) -> f64 {
    let Some(length) = (unsafe { native_array_length(array) }) else {
        return -1.0;
    };
    let matches_value = |index: &usize| {
        if !unsafe { array_index_present(presence, *index) } {
            return false;
        }
        let slot = unsafe { array.add(8 + *index * 16) };
        if unsafe { slot.read() } != value_tag {
            return false;
        }
        let payload = unsafe { slot.add(if kind == 2 && union_layout == 0 { 1 } else { 8 }) };
        match kind {
            0 => {
                let value = unsafe { payload.cast::<f64>().read_unaligned() };
                let expected = unsafe { needle.cast::<f64>().read_unaligned() };
                value == expected || (includes != 0 && value.is_nan() && expected.is_nan())
            }
            1 => {
                let value = unsafe { payload.cast::<*const c_char>().read_unaligned() };
                let expected = unsafe { needle.cast::<*const c_char>().read_unaligned() };
                !value.is_null() && !expected.is_null()
                    && unsafe { thaw_string_compare(value, expected) == 0 }
            }
            2 => (unsafe { payload.read() } != 0) == (unsafe { needle.read() } != 0),
            3 => (unsafe { payload.cast::<*const u8>().read_unaligned() })
                == unsafe { needle.cast::<*const u8>().read_unaligned() },
            _ => false,
        }
    };
    let found = if reverse == 0 {
        (array_search_start(length, from_index)..length).find(matches_value)
    } else {
        array_search_end(length, from_index)
            .and_then(|end| (0..=end).rev().find(matches_value))
    };
    found.map_or(-1.0, |index| index as f64)
}

unsafe fn number_array_search(
    array: *const u8,
    presence: *const u8,
    needle: f64,
    from_index: f64,
    same_value_zero: bool,
) -> f64 {
    let Some(length) = (unsafe { native_array_length(array) }) else {
        return -1.0;
    };
    for index in array_search_start(length, from_index)..length {
        if !unsafe { array_index_present(presence, index) } {
            continue;
        }
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
    presence: *const u8,
    needle: f64,
    from_index: f64,
) -> f64 {
    unsafe { number_array_search(array, presence, needle, from_index, false) }
}

#[no_mangle]
/// # Safety
///
/// `array` must point to a Thaw array containing `f64` element slots.
pub unsafe extern "C" fn thaw_number_array_includes(
    array: *const u8,
    presence: *const u8,
    needle: f64,
    from_index: f64,
) -> u8 {
    (unsafe { number_array_search(array, presence, needle, from_index, true) } >= 0.0).into()
}

#[no_mangle]
/// # Safety
/// `array` must point to a Thaw array containing `f64` element slots.
pub unsafe extern "C" fn thaw_number_array_last_index_of(
    array: *const u8,
    presence: *const u8,
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
        if !unsafe { array_index_present(presence, index) } {
            continue;
        }
        let slot = unsafe { array.add(8 + index * 8).cast::<f64>().read_unaligned() };
        if slot == needle {
            return index as f64;
        }
    }
    -1.0
}

unsafe fn string_array_search(
    array: *const u8,
    presence: *const u8,
    needle: *const c_char,
    from_index: f64,
) -> f64 {
    let Some(length) = (unsafe { native_array_length(array) }) else {
        return -1.0;
    };
    if needle.is_null() {
        return -1.0;
    }
    for index in array_search_start(length, from_index)..length {
        if !unsafe { array_index_present(presence, index) } {
            continue;
        }
        let slot = unsafe {
            array
                .add(8 + index * 8)
                .cast::<*const c_char>()
                .read_unaligned()
        };
        if !slot.is_null() && unsafe { thaw_string_compare(slot, needle) == 0 } {
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
    presence: *const u8,
    needle: *const c_char,
    from_index: f64,
) -> f64 {
    unsafe { string_array_search(array, presence, needle, from_index) }
}

#[no_mangle]
/// # Safety
///
/// `array` must point to a Thaw array containing C-string pointer slots and
/// `needle` must point to a valid NUL-terminated C string.
pub unsafe extern "C" fn thaw_string_array_includes(
    array: *const u8,
    presence: *const u8,
    needle: *const c_char,
    from_index: f64,
) -> u8 {
    (unsafe { string_array_search(array, presence, needle, from_index) } >= 0.0).into()
}

#[no_mangle]
/// # Safety
/// `array` must point to a Thaw array containing C-string pointer slots and
/// `needle` must point to a valid NUL-terminated C string.
pub unsafe extern "C" fn thaw_string_array_last_index_of(
    array: *const u8,
    presence: *const u8,
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
    for index in (0..=start).rev() {
        if !unsafe { array_index_present(presence, index) } {
            continue;
        }
        let slot = unsafe {
            array
                .add(8 + index * 8)
                .cast::<*const c_char>()
                .read_unaligned()
        };
        if !slot.is_null() && unsafe { thaw_string_compare(slot, needle) == 0 } {
            return index as f64;
        }
    }
    -1.0
}

unsafe fn bool_array_search(
    array: *const u8,
    presence: *const u8,
    needle: u8,
    from_index: f64,
) -> f64 {
    let Some(length) = (unsafe { native_array_length(array) }) else {
        return -1.0;
    };
    for index in array_search_start(length, from_index)..length {
        if !unsafe { array_index_present(presence, index) } {
            continue;
        }
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
    presence: *const u8,
    needle: u8,
    from_index: f64,
) -> f64 {
    unsafe { bool_array_search(array, presence, needle, from_index) }
}

#[no_mangle]
/// # Safety
///
/// `array` must point to a Thaw array containing boolean element slots.
pub unsafe extern "C" fn thaw_bool_array_includes(
    array: *const u8,
    presence: *const u8,
    needle: u8,
    from_index: f64,
) -> u8 {
    (unsafe { bool_array_search(array, presence, needle, from_index) } >= 0.0).into()
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
        0 => unsafe { thaw_number_array_index_of(array, std::ptr::null(), needle, from_index) },
        1 => f64::from(unsafe {
            thaw_number_array_includes(array, std::ptr::null(), needle, from_index)
        }),
        2 => unsafe {
            thaw_string_array_index_of(
                array,
                std::ptr::null(),
                needle.to_bits() as usize as *const c_char,
                from_index,
            )
        },
        3 => f64::from(unsafe {
            thaw_string_array_includes(
                array,
                std::ptr::null(),
                needle.to_bits() as usize as *const c_char,
                from_index,
            )
        }),
        4 => unsafe {
            thaw_bool_array_index_of(array, std::ptr::null(), (needle != 0.0).into(), from_index)
        },
        5 => f64::from(unsafe {
            thaw_bool_array_includes(
                array,
                std::ptr::null(),
                (needle != 0.0).into(),
                from_index,
            )
        }),
        6 => unsafe {
            thaw_number_array_last_index_of(array, std::ptr::null(), needle, from_index)
        },
        7 => unsafe {
            thaw_string_array_last_index_of(
                array,
                std::ptr::null(),
                needle.to_bits() as usize as *const c_char,
                from_index,
            )
        },
        8 => unsafe {
            thaw_bool_array_last_index_of(
                array,
                std::ptr::null(),
                (needle != 0.0).into(),
                from_index,
            )
        },
        _ => -1.0,
    }
}

#[no_mangle]
/// # Safety
/// `array` must point to a Thaw array containing boolean element slots.
pub unsafe extern "C" fn thaw_bool_array_last_index_of(
    array: *const u8,
    presence: *const u8,
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
        if !unsafe { array_index_present(presence, index) } {
            continue;
        }
        let slot = unsafe { array.add(8 + index * 8).read() };
        if (slot != 0) == (needle != 0) {
            return index as f64;
        }
    }
    -1.0
}

unsafe fn object_array_search(
    array: *const u8,
    presence: *const u8,
    needle: *const u8,
    from_index: f64,
) -> f64 {
    let Some(length) = (unsafe { native_array_length(array) }) else {
        return -1.0;
    };
    for index in array_search_start(length, from_index)..length {
        if !unsafe { array_index_present(presence, index) } {
            continue;
        }
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
    presence: *const u8,
    needle: *const u8,
    from_index: f64,
) -> f64 {
    unsafe { object_array_search(array, presence, needle, from_index) }
}

#[no_mangle]
/// # Safety
/// `array` must point to a Thaw array containing fixed-object pointer slots.
pub unsafe extern "C" fn thaw_object_array_includes(
    array: *const u8,
    presence: *const u8,
    needle: *const u8,
    from_index: f64,
) -> u8 {
    (unsafe { object_array_search(array, presence, needle, from_index) } >= 0.0).into()
}

#[no_mangle]
/// # Safety
/// `array` must point to a Thaw array containing fixed-object pointer slots.
pub unsafe extern "C" fn thaw_object_array_last_index_of(
    array: *const u8,
    presence: *const u8,
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
        if !unsafe { array_index_present(presence, index) } {
            continue;
        }
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

// `thaw_json_strict_equal` (`===`) / `thaw_json_same_value_zero`
// (`SameValueZero`, used by `AnyKey` in `maps.rs` for `Map<any, V>`/
// `Set<any>` key equality) both live in thaw-std now -- only that
// crate's `Value` has real access to its own shared Array/Object
// container's identity (`Rc::ptr_eq`, since a `Json` value's outer
// pointer is now a fresh `leak`ed wrapper on every read, not a stable
// identity). Declared here as opaque-pointer externs rather than a real
// Cargo dependency: each of thaw-arena/thaw-runtime/thaw-std is built as
// its own independent static archive and combined only at the final
// system-link step (see each crate's own `Cargo.toml` comment), so a
// real Rust-level dependency between them would duplicate every
// `#[no_mangle]` symbol they share -- resolved at link time the same
// way `thaw_string_to_number`/`thaw_date_to_iso_string` already are.
// Safety (both): `a`/`b` must each be null or point to a valid JSON `Value`.
unsafe extern "C" {
    pub fn thaw_json_strict_equal(a: *const u8, b: *const u8) -> u8;
    fn thaw_json_same_value_zero(a: *const u8, b: *const u8) -> u8;
    fn thaw_json_is_undefined(value: *const u8) -> u8;
}

fn json_same_value_zero(a: *const u8, b: *const u8) -> bool {
    unsafe { thaw_json_same_value_zero(a, b) != 0 }
}

/// Like `object_array_search`, but for `.indexOf()`/`.includes()` on a
/// `Json` (`any`-typed) array element -- comparing slots by real Strict
/// Equality (`thaw_json_strict_equal`) instead of raw pointer identity,
/// since a `Json` slot can hold a primitive value (two separately-boxed
/// occurrences of the same number/string/boolean must compare equal),
/// unlike a genuine fixed-layout object slot.
/// `same_value_zero`: `.indexOf()`/`.lastIndexOf()` use real `===`
/// (never find a `NaN` needle, matching the spec's own `Strict Equality
/// Comparison`), while `.includes()` uses `SameValueZero` (`NaN` does
/// find itself) -- the one place these two method families genuinely
/// disagree. Before `NaN`/`Infinity` gained a real, distinct `Json`
/// representation (`thaw_json_undefined`'s non-finite sibling,
/// thaw-std's `json.rs`), both fell back to the same indistinguishable
/// `Value::Null`, so `thaw_json_strict_equal` happened to "find" a
/// `NaN` needle too (`null === null`) -- masking this gap until then.
unsafe fn any_array_search(
    array: *const u8,
    presence: *const u8,
    needle: *const u8,
    from_index: f64,
    same_value_zero: bool,
) -> f64 {
    let Some(length) = (unsafe { native_array_length(array) }) else {
        return -1.0;
    };
    let mut needle_is_undefined = None;
    for index in array_search_start(length, from_index)..length {
        if !unsafe { array_index_present(presence, index) } {
            if (same_value_zero || unsafe { array_index_exists(presence, index) })
                && *needle_is_undefined.get_or_insert_with(|| unsafe { thaw_json_is_undefined(needle) != 0 })
            {
                return index as f64;
            }
            continue;
        }
        let slot = unsafe {
            array
                .add(8 + index * 8)
                .cast::<*const u8>()
                .read_unaligned()
        };
        let matches = if same_value_zero {
            json_same_value_zero(slot.cast(), needle.cast())
        } else {
            unsafe { thaw_json_strict_equal(slot.cast(), needle.cast()) != 0 }
        };
        if matches {
            return index as f64;
        }
    }
    -1.0
}

#[no_mangle]
/// # Safety
/// `array` must point to a Thaw array containing `Json` pointer slots.
pub unsafe extern "C" fn thaw_any_array_index_of(
    array: *const u8,
    presence: *const u8,
    needle: *const u8,
    from_index: f64,
) -> f64 {
    unsafe { any_array_search(array, presence, needle, from_index, false) }
}

#[no_mangle]
/// # Safety
/// `array` must point to a Thaw array containing `Json` pointer slots.
pub unsafe extern "C" fn thaw_any_array_includes(
    array: *const u8,
    presence: *const u8,
    needle: *const u8,
    from_index: f64,
) -> u8 {
    (unsafe { any_array_search(array, presence, needle, from_index, true) } >= 0.0).into()
}

#[no_mangle]
/// # Safety
/// `array` must point to a Thaw array containing `Json` pointer slots.
pub unsafe extern "C" fn thaw_any_array_last_index_of(
    array: *const u8,
    presence: *const u8,
    needle: *const u8,
    from_index: f64,
) -> f64 {
    let Some(length) = (unsafe { native_array_length(array) }) else {
        return -1.0;
    };
    let Some(start) = array_search_end(length, from_index) else {
        return -1.0;
    };
    let mut needle_is_undefined = None;
    for index in (0..=start).rev() {
        if !unsafe { array_index_present(presence, index) } {
            if unsafe { array_index_exists(presence, index) }
                && *needle_is_undefined.get_or_insert_with(|| unsafe { thaw_json_is_undefined(needle) != 0 })
            {
                return index as f64;
            }
            continue;
        }
        let slot = unsafe {
            array
                .add(8 + index * 8)
                .cast::<*const u8>()
                .read_unaligned()
        };
        if unsafe { thaw_json_strict_equal(slot.cast(), needle.cast()) } != 0 {
            return index as f64;
        }
    }
    -1.0
}

#[cfg(test)]
mod array_read_ptr_tests {
    use super::*;

    fn build_array(elements: &[f64]) -> *mut u8 {
        let bytes = 8 + elements.len() * 8;
        let array = thaw_arena::thaw_arena_alloc(bytes, 8);
        unsafe {
            array.cast::<u64>().write(elements.len() as u64);
            for (index, &value) in elements.iter().enumerate() {
                array.add(8 + index * 8).cast::<f64>().write(value);
            }
        }
        array
    }

    #[test]
    fn in_bounds_index_reads_the_real_element() {
        let array = build_array(&[10.0, 20.0, 30.0]);
        let ptr = unsafe { thaw_array_read_ptr(array, 8, 1.0) };
        assert_eq!(unsafe { ptr.cast::<f64>().read() }, 20.0);
    }

    #[test]
    fn out_of_range_negative_and_non_integer_indices_read_a_zeroed_slot_not_foreign_memory() {
        let array = build_array(&[10.0, 20.0, 30.0]);
        for index in [100.0, -5.0, 1.5, f64::NAN, f64::INFINITY] {
            let ptr = unsafe { thaw_array_read_ptr(array, 8, index) };
            assert_eq!(unsafe { ptr.cast::<f64>().read() }, 0.0);
        }
    }

    #[test]
    fn delete_extends_a_shorter_live_presence_mask_before_marking_the_index_absent() {
        let array = build_array(&[10.0, 20.0, 30.0, 40.0]);
        let mask = thaw_arena::thaw_arena_alloc(10, 1);
        let handle = thaw_arena::thaw_arena_alloc(16, 8);
        unsafe {
            mask.cast::<u64>().write(2);
            mask.add(8).write(0);
            mask.add(9).write(1);
            handle.cast::<*mut u8>().write(array);
            handle.add(8).cast::<*mut u8>().write(mask);
            assert_eq!(thaw_array_delete_property(handle, c"3".as_ptr()), 1);
            let updated = handle.add(8).cast::<*mut u8>().read();
            assert_eq!(updated.cast::<u64>().read(), 4);
            assert_eq!((0..4).map(|index| updated.add(8 + index).read()).collect::<Vec<_>>(), vec![0, 1, 1, 0]);
            assert_eq!(array.cast::<u64>().read(), 4);
            assert_eq!(thaw_array_has_property(array, updated, c"3".as_ptr(), 1), 0);
            assert_eq!(thaw_array_has_property(array, updated, c"2".as_ptr(), 1), 1);
        }
    }

    #[test]
    fn string_array_search_compares_utf16_units() {
        let canonical = arena_wtf8("😀".as_bytes()).unwrap().cast::<c_char>();
        let split_pair = arena_wtf8(&[0xed, 0xa0, 0xbd, 0xed, 0xb8, 0x80]).unwrap().cast::<c_char>();
        let different = arena_wtf8("😁".as_bytes()).unwrap().cast::<c_char>();
        for (stored, needle) in [(canonical, split_pair), (split_pair, canonical)] {
            let array = [1_u64, stored as usize as u64];
            let tagged = [1_u64, 1, stored as usize as u64];
            unsafe {
                let array = array.as_ptr().cast::<u8>();
                assert_eq!(thaw_string_array_index_of(array, std::ptr::null(), needle, 0.0), 0.0);
                assert_eq!(thaw_string_array_includes(array, std::ptr::null(), needle, 0.0), 1);
                assert_eq!(thaw_string_array_last_index_of(array, std::ptr::null(), needle, f64::INFINITY), 0.0);
                assert_eq!(thaw_string_array_includes(array, std::ptr::null(), different, 0.0), 0);
                for (reverse, includes) in [(0, 0), (0, 1), (1, 0)] {
                    assert_eq!(thaw_tagged_array_index_of(tagged.as_ptr().cast(), std::ptr::null(),
                        (&needle as *const *const c_char).cast(),
                        if reverse == 0 { 0.0 } else { f64::INFINITY },
                        reverse, includes, 1, 1, 0), 0.0);
                }
            }
        }
    }

    #[test]
    fn array_search_and_join_keep_registered_nul_and_wtf8_bytes() {
        let x = arena_wtf8(b"a\0x").unwrap().cast::<c_char>();
        let y = arena_wtf8(b"a\0y").unwrap().cast::<c_char>();
        let z = arena_wtf8(b"a\0z").unwrap().cast::<c_char>();
        let array = thaw_arena::thaw_arena_alloc(24, 8);
        let tagged = thaw_arena::thaw_arena_alloc(40, 8);
        let numbers = thaw_arena::thaw_arena_alloc(24, 8);
        let bools = thaw_arena::thaw_arena_alloc(24, 8);
        assert!(!array.is_null() && !tagged.is_null() && !numbers.is_null() && !bools.is_null());
        unsafe {
            array.cast::<u64>().write(2);
            array.add(8).cast::<*const c_char>().write(x);
            array.add(16).cast::<*const c_char>().write(y);
            assert_eq!(thaw_string_array_index_of(array, std::ptr::null(), y, 0.0), 1.0);
            assert_eq!(thaw_string_array_includes(array, std::ptr::null(), z, 0.0), 0);
            assert_eq!(thaw_string_array_last_index_of(array, std::ptr::null(), x, f64::INFINITY), 0.0);
            tagged.cast::<u64>().write(2);
            tagged.add(8).write(1);
            tagged.add(16).cast::<*const c_char>().write(x);
            tagged.add(24).write(1);
            tagged.add(32).cast::<*const c_char>().write(y);
            assert_eq!(thaw_tagged_array_index_of(tagged, std::ptr::null(), (&y as *const *const c_char).cast(), 0.0, 0, 0, 1, 1, 0), 1.0);
            assert_eq!(thaw_tagged_array_index_of(tagged, std::ptr::null(), (&z as *const *const c_char).cast(), 0.0, 0, 1, 1, 1, 0), -1.0);
            array.add(8).cast::<*const c_char>().write(y);
            array.add(16).cast::<*const c_char>().write(x);
            let sorted = thaw_string_array_to_sorted(array);
            assert!(!sorted.is_null());
            assert_eq!(sorted.add(8).cast::<*const c_char>().read(), x);
            assert_eq!(array.add(8).cast::<*const c_char>().read(), y);
            assert_eq!(thaw_string_array_sort(array), array);
            assert_eq!(array.add(8).cast::<*const c_char>().read(), x);
            assert_eq!(thaw_jit_array_sort(5, array), array);
            assert_eq!(array.add(8).cast::<*const c_char>().read(), y);
            numbers.cast::<u64>().write(2);
            numbers.add(8).cast::<f64>().write(1.0);
            numbers.add(16).cast::<f64>().write(2.0);
            bools.cast::<u64>().write(2);
            bools.add(8).write(0);
            bools.add(16).write(1);
            let separator = arena_wtf8(&[0, 0xed, 0xa0, 0x80]).unwrap().cast::<c_char>();
            assert_eq!(wtf8_bytes(thaw_number_array_join(numbers, std::ptr::null(), separator)), &[b'1', 0, 0xed, 0xa0, 0x80, b'2']);
            assert_eq!(wtf8_bytes(thaw_bool_array_join(bools, std::ptr::null(), separator)), b"false\0\xed\xa0\x80true");
            assert_eq!(wtf8_bytes(thaw_object_array_join(array, std::ptr::null(), separator)), b"[object Object]\0\xed\xa0\x80[object Object]");
        }
    }

    #[test]
    fn string_sort_and_to_sorted_compare_lone_surrogates_as_utf16_units() {
        let replacement = arena_wtf8(&wtf8_encode_utf16(&[0xFFFD])).expect("replacement");
        let lone_high = arena_wtf8(&wtf8_encode_utf16(&[0xD800])).expect("lone high surrogate");
        let array = thaw_arena::thaw_arena_alloc(24, 8);
        assert!(!array.is_null());
        unsafe {
            array.cast::<u64>().write(2);
            array.add(8).cast::<u64>().write(replacement as usize as u64);
            array.add(16).cast::<u64>().write(lone_high as usize as u64);
            let copy = thaw_string_array_to_sorted(array);
            assert!(!copy.is_null());
            assert_eq!(copy.add(8).cast::<u64>().read(), lone_high as usize as u64);
            assert_eq!(copy.add(16).cast::<u64>().read(), replacement as usize as u64);
            assert_eq!(array.add(8).cast::<u64>().read(), replacement as usize as u64);
            assert_eq!(thaw_string_array_sort(array), array);
            assert_eq!(array.add(8).cast::<u64>().read(), lone_high as usize as u64);
            assert_eq!(thaw_jit_array_sort(5, array), array);
            assert_eq!(array.add(8).cast::<u64>().read(), replacement as usize as u64);
        }
    }

    #[test]
    fn null_array_reads_a_zeroed_slot() {
        let ptr = unsafe { thaw_array_read_ptr(std::ptr::null(), 8, 0.0) };
        assert_eq!(unsafe { ptr.cast::<f64>().read() }, 0.0);
    }
}
