#[no_mangle]
/// Compares UTF-8 native strings using JavaScript's UTF-16 code-unit order.
///
/// # Safety
///
/// Both pointers must reference valid NUL-terminated C strings.
pub unsafe extern "C" fn thaw_string_compare(left: *const c_char, right: *const c_char) -> i32 {
    if left.is_null() || right.is_null() {
        return 0;
    }
    let left = unsafe { CStr::from_ptr(left) }.to_string_lossy();
    let right = unsafe { CStr::from_ptr(right) }.to_string_lossy();
    let left = left.encode_utf16().collect::<Vec<_>>();
    let right = right.encode_utf16().collect::<Vec<_>>();
    match left.cmp(&right) {
        std::cmp::Ordering::Less => -1,
        std::cmp::Ordering::Equal => 0,
        std::cmp::Ordering::Greater => 1,
    }
}

#[no_mangle]
/// # Safety
/// `value` must reference a valid NUL-terminated UTF-8 string.
pub unsafe extern "C" fn thaw_string_to_lower_case(value: *const c_char) -> *const c_char {
    if value.is_null() {
        return std::ptr::null();
    }
    let value = unsafe { CStr::from_ptr(value) }.to_string_lossy();
    arena_c_string(&value.to_lowercase()).map_or(std::ptr::null(), |value| value.cast())
}

#[no_mangle]
/// # Safety
/// `value` must reference a valid NUL-terminated UTF-8 string.
pub unsafe extern "C" fn thaw_string_to_upper_case(value: *const c_char) -> *const c_char {
    if value.is_null() {
        return std::ptr::null();
    }
    let value = unsafe { CStr::from_ptr(value) }.to_string_lossy();
    arena_c_string(&value.to_uppercase()).map_or(std::ptr::null(), |value| value.cast())
}

#[no_mangle]
/// Percent-encodes every byte of `value`'s UTF-8 representation other than
/// the ASCII letters, digits and `- _ . ! ~ * ' ( )`, matching
/// `encodeURIComponent`.
///
/// # Safety
/// `value` must reference a valid NUL-terminated UTF-8 string.
pub unsafe extern "C" fn thaw_encode_uri_component(value: *const c_char) -> *const c_char {
    if value.is_null() {
        return std::ptr::null();
    }
    let value = unsafe { CStr::from_ptr(value) }.to_string_lossy();
    let mut output = String::with_capacity(value.len());
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'!' | b'~' | b'*'
            | b'\'' | b'(' | b')' => output.push(byte as char),
            _ => output.push_str(&format!("%{byte:02X}")),
        }
    }
    arena_c_string(&output).map_or(std::ptr::null(), |value| value.cast())
}

#[no_mangle]
/// Percent-encodes every byte of `value`'s UTF-8 representation other than
/// the `encodeURIComponent` unreserved set plus the URI reserved characters
/// `; / ? : @ & = + $ , #`, matching `encodeURI`.
///
/// # Safety
/// `value` must reference a valid NUL-terminated UTF-8 string.
pub unsafe extern "C" fn thaw_encode_uri(value: *const c_char) -> *const c_char {
    if value.is_null() {
        return std::ptr::null();
    }
    let value = unsafe { CStr::from_ptr(value) }.to_string_lossy();
    let mut output = String::with_capacity(value.len());
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'!' | b'~' | b'*'
            | b'\'' | b'(' | b')' | b';' | b'/' | b'?' | b':' | b'@' | b'&' | b'=' | b'+'
            | b'$' | b',' | b'#' => output.push(byte as char),
            _ => output.push_str(&format!("%{byte:02X}")),
        }
    }
    arena_c_string(&output).map_or(std::ptr::null(), |value| value.cast())
}

#[no_mangle]
/// Percent-decodes `value`, matching `decodeURIComponent`. Returns a null
/// pointer for a malformed percent-escape (missing/non-hex digits, or a
/// decoded byte sequence that is not valid UTF-8), letting the caller detect
/// the failure with `thaw_string_is_null` and throw a `URIError` as the
/// specification requires.
///
/// # Safety
/// `value` must reference a valid NUL-terminated UTF-8 string.
pub unsafe extern "C" fn thaw_decode_uri_component(value: *const c_char) -> *const c_char {
    if value.is_null() {
        return std::ptr::null();
    }
    let value = unsafe { CStr::from_ptr(value) }.to_string_lossy();
    let bytes = value.as_bytes();
    let mut output = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            let Some(hex) = bytes.get(index + 1..index + 3) else {
                return std::ptr::null();
            };
            let Ok(hex) = std::str::from_utf8(hex) else {
                return std::ptr::null();
            };
            let Ok(byte) = u8::from_str_radix(hex, 16) else {
                return std::ptr::null();
            };
            output.push(byte);
            index += 3;
        } else {
            output.push(bytes[index]);
            index += 1;
        }
    }
    let Ok(text) = String::from_utf8(output) else {
        return std::ptr::null();
    };
    arena_c_string(&text).map_or(std::ptr::null(), |value| value.cast())
}

#[no_mangle]
/// Percent-decodes `value` like `thaw_decode_uri_component`, except an
/// escape that would decode to one of the URI reserved characters
/// `; / ? : @ & = + $ , #` is left as the original three-character escape,
/// matching `decodeURI`. Returns a null pointer for a malformed
/// percent-escape or a decoded byte sequence that is not valid UTF-8.
///
/// # Safety
/// `value` must reference a valid NUL-terminated UTF-8 string.
pub unsafe extern "C" fn thaw_decode_uri(value: *const c_char) -> *const c_char {
    if value.is_null() {
        return std::ptr::null();
    }
    let value = unsafe { CStr::from_ptr(value) }.to_string_lossy();
    let bytes = value.as_bytes();
    let mut output = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            let Some(hex) = bytes.get(index + 1..index + 3) else {
                return std::ptr::null();
            };
            let Ok(hex_str) = std::str::from_utf8(hex) else {
                return std::ptr::null();
            };
            let Ok(byte) = u8::from_str_radix(hex_str, 16) else {
                return std::ptr::null();
            };
            if matches!(
                byte,
                b';' | b'/' | b'?' | b':' | b'@' | b'&' | b'=' | b'+' | b'$' | b',' | b'#'
            ) {
                output.extend_from_slice(&bytes[index..index + 3]);
            } else {
                output.push(byte);
            }
            index += 3;
        } else {
            output.push(bytes[index]);
            index += 1;
        }
    }
    let Ok(text) = String::from_utf8(output) else {
        return std::ptr::null();
    };
    arena_c_string(&text).map_or(std::ptr::null(), |value| value.cast())
}

#[no_mangle]
/// # Safety
/// `value` must point to a valid NUL-terminated UTF-8 string. `count` must be
/// finite, non-negative and already normalized to an integer.
pub unsafe extern "C" fn thaw_string_repeat(value: *const c_char, count: f64) -> *const c_char {
    if value.is_null() || !count.is_finite() || count < 0.0 {
        return std::ptr::null();
    }
    let value = unsafe { CStr::from_ptr(value) }.to_string_lossy();
    let count = count as usize;
    let Some(capacity) = value.len().checked_mul(count) else {
        return std::ptr::null();
    };
    let mut output = String::with_capacity(capacity);
    for _ in 0..count {
        output.push_str(&value);
    }
    arena_c_string(&output).map_or(std::ptr::null(), |value| value.cast())
}

/// # Safety
/// `value` and `pad` must point to valid NUL-terminated UTF-8 strings.
unsafe fn thaw_string_pad(
    value: *const c_char,
    pad: *const c_char,
    target_length: f64,
    at_start: bool,
) -> *const c_char {
    if value.is_null() || pad.is_null() {
        return std::ptr::null();
    }
    let value = unsafe { CStr::from_ptr(value) }.to_string_lossy();
    let pad = unsafe { CStr::from_ptr(pad) }.to_string_lossy();
    let units: Vec<u16> = value.encode_utf16().collect();
    let target_length = if target_length.is_finite() && target_length > 0.0 {
        target_length as usize
    } else {
        0
    };
    let pad_units: Vec<u16> = pad.encode_utf16().collect();
    if target_length <= units.len() || pad_units.is_empty() {
        return arena_c_string(&value).map_or(std::ptr::null(), |value| value.cast());
    }
    let needed = target_length - units.len();
    let mut filler = Vec::with_capacity(needed);
    while filler.len() < needed {
        filler.extend_from_slice(&pad_units);
    }
    filler.truncate(needed);
    let combined: Vec<u16> = if at_start {
        filler.into_iter().chain(units).collect()
    } else {
        units.into_iter().chain(filler).collect()
    };
    let combined = String::from_utf16_lossy(&combined);
    arena_c_string(&combined).map_or(std::ptr::null(), |value| value.cast())
}

#[no_mangle]
/// Left-pads `value` with `pad` to `target_length` UTF-16 code units,
/// matching `String.prototype.padStart`.
///
/// # Safety
/// `value` and `pad` must point to valid NUL-terminated UTF-8 strings.
/// `target_length` must already be normalized to a number.
pub unsafe extern "C" fn thaw_string_pad_start(
    value: *const c_char,
    pad: *const c_char,
    target_length: f64,
) -> *const c_char {
    unsafe { thaw_string_pad(value, pad, target_length, true) }
}

#[no_mangle]
/// Right-pads `value` with `pad` to `target_length` UTF-16 code units,
/// matching `String.prototype.padEnd`.
///
/// # Safety
/// `value` and `pad` must point to valid NUL-terminated UTF-8 strings.
/// `target_length` must already be normalized to a number.
pub unsafe extern "C" fn thaw_string_pad_end(
    value: *const c_char,
    pad: *const c_char,
    target_length: f64,
) -> *const c_char {
    unsafe { thaw_string_pad(value, pad, target_length, false) }
}

unsafe fn native_array_length(array: *const u8) -> Option<usize> {
    (!array.is_null()).then(|| unsafe { array.cast::<u64>().read() as usize })
}

#[no_mangle]
fn clamped_string_position(position: f64, length: usize) -> usize {
    if position.is_nan() || position == f64::NEG_INFINITY {
        0
    } else if position == f64::INFINITY {
        length
    } else {
        position.trunc().max(0.0).min(length as f64) as usize
    }
}

unsafe fn utf16_strings(
    value: *const c_char,
    search: *const c_char,
) -> Option<(Vec<u16>, Vec<u16>)> {
    if value.is_null() || search.is_null() {
        return None;
    }
    let value = unsafe { CStr::from_ptr(value) }
        .to_string_lossy()
        .encode_utf16()
        .collect();
    let search = unsafe { CStr::from_ptr(search) }
        .to_string_lossy()
        .encode_utf16()
        .collect();
    Some((value, search))
}

#[no_mangle]
/// Searches strings by JavaScript UTF-16 code-unit position.
///
/// # Safety
///
/// Both pointers must reference valid NUL-terminated C strings.
pub unsafe extern "C" fn thaw_string_index_of(
    value: *const c_char,
    search: *const c_char,
    position: f64,
) -> f64 {
    let Some((value, search)) = (unsafe { utf16_strings(value, search) }) else {
        return -1.0;
    };
    let start = clamped_string_position(position, value.len());
    if search.is_empty() {
        return start as f64;
    }
    value[start..]
        .windows(search.len())
        .position(|window| window == search)
        .map_or(-1.0, |index| (start + index) as f64)
}

#[no_mangle]
/// Searches backward using JavaScript UTF-16 code-unit positions.
///
/// # Safety
/// Both pointers must reference valid NUL-terminated C strings.
pub unsafe extern "C" fn thaw_string_last_index_of(
    value: *const c_char,
    search: *const c_char,
    position: f64,
) -> f64 {
    let Some((value, search)) = (unsafe { utf16_strings(value, search) }) else {
        return -1.0;
    };
    let position = clamped_string_position(position, value.len());
    if search.is_empty() {
        return position as f64;
    }
    if search.len() > value.len() {
        return -1.0;
    }
    let start = position.min(value.len() - search.len());
    (0..=start)
        .rev()
        .find(|&index| value.get(index..index + search.len()) == Some(search.as_slice()))
        .map_or(-1.0, |index| index as f64)
}

#[no_mangle]
/// # Safety
///
/// Both pointers must reference valid NUL-terminated C strings.
pub unsafe extern "C" fn thaw_string_includes(
    value: *const c_char,
    search: *const c_char,
    position: f64,
) -> u8 {
    (unsafe { thaw_string_index_of(value, search, position) } >= 0.0).into()
}

#[no_mangle]
/// # Safety
///
/// Both pointers must reference valid NUL-terminated C strings.
pub unsafe extern "C" fn thaw_string_starts_with(
    value: *const c_char,
    search: *const c_char,
    position: f64,
) -> u8 {
    let Some((value, search)) = (unsafe { utf16_strings(value, search) }) else {
        return 0;
    };
    let start = clamped_string_position(position, value.len());
    (value.get(start..start.saturating_add(search.len())) == Some(search.as_slice())).into()
}

#[no_mangle]
/// # Safety
///
/// Both pointers must reference valid NUL-terminated C strings.
pub unsafe extern "C" fn thaw_string_ends_with(
    value: *const c_char,
    search: *const c_char,
    end_position: f64,
) -> u8 {
    let Some((value, search)) = (unsafe { utf16_strings(value, search) }) else {
        return 0;
    };
    let end = clamped_string_position(end_position, value.len());
    if search.len() > end {
        return 0;
    }
    (value.get(end - search.len()..end) == Some(search.as_slice())).into()
}

fn is_javascript_whitespace(character: char) -> bool {
    matches!(
        character,
        '\u{0009}' | '\u{000b}' | '\u{000c}' | '\u{0020}' | '\u{00a0}' | '\u{1680}' | '\u{2000}'
            ..='\u{200a}'
                | '\u{202f}'
                | '\u{205f}'
                | '\u{3000}'
                | '\u{feff}'
                | '\u{000a}'
                | '\u{000d}'
                | '\u{2028}'
                | '\u{2029}'
    )
}

unsafe fn trim_javascript_string(value: *const c_char, start: bool, end: bool) -> *const c_char {
    if value.is_null() {
        return std::ptr::null();
    }
    let value = unsafe { CStr::from_ptr(value) }.to_string_lossy();
    let value = if start {
        value.trim_start_matches(is_javascript_whitespace)
    } else {
        value.as_ref()
    };
    let value = if end {
        value.trim_end_matches(is_javascript_whitespace)
    } else {
        value
    };
    arena_c_string(value).map_or(std::ptr::null(), |value| value.cast())
}

#[no_mangle]
/// # Safety
///
/// `value` must reference a valid NUL-terminated C string.
pub unsafe extern "C" fn thaw_string_trim(value: *const c_char) -> *const c_char {
    unsafe { trim_javascript_string(value, true, true) }
}

#[no_mangle]
/// # Safety
///
/// `value` must reference a valid NUL-terminated C string.
pub unsafe extern "C" fn thaw_string_trim_start(value: *const c_char) -> *const c_char {
    unsafe { trim_javascript_string(value, true, false) }
}

#[no_mangle]
/// # Safety
///
/// `value` must reference a valid NUL-terminated C string.
pub unsafe extern "C" fn thaw_string_trim_end(value: *const c_char) -> *const c_char {
    unsafe { trim_javascript_string(value, false, true) }
}

#[no_mangle]
/// Returns the JavaScript UTF-16 code-unit length of a native string.
///
/// # Safety
///
/// `value` must reference a valid NUL-terminated C string.
pub unsafe extern "C" fn thaw_string_length(value: *const c_char) -> f64 {
    if value.is_null() {
        return 0.0;
    }
    unsafe { CStr::from_ptr(value) }
        .to_string_lossy()
        .encode_utf16()
        .count() as f64
}

#[no_mangle]
/// Implements `String.prototype.charCodeAt` using UTF-16 code units.
///
/// # Safety
///
/// `value` must reference a valid NUL-terminated C string.
pub unsafe extern "C" fn thaw_string_char_code_at(value: *const c_char, index: f64) -> f64 {
    if value.is_null() || index.is_infinite() {
        return f64::NAN;
    }
    let index = if index.is_nan() { 0.0 } else { index.trunc() };
    if index < 0.0 || index > usize::MAX as f64 {
        return f64::NAN;
    }
    unsafe { CStr::from_ptr(value) }
        .to_string_lossy()
        .encode_utf16()
        .nth(index as usize)
        .map_or(f64::NAN, f64::from)
}

#[no_mangle]
/// Returns the Unicode code point at `index` (measured in UTF-16 code
/// units), combining a leading surrogate with a following trailing
/// surrogate into one astral code point. Returns `-1.0` when `index` is out
/// of bounds, letting the caller report `undefined` as
/// `String.prototype.codePointAt` requires; code points are otherwise never
/// negative.
///
/// # Safety
/// `value` must point to a valid NUL-terminated UTF-8 string. `index` must
/// already be normalized to an integer.
pub unsafe extern "C" fn thaw_string_code_point_at(value: *const c_char, index: f64) -> f64 {
    if value.is_null() || !index.is_finite() || index < 0.0 || index > usize::MAX as f64 {
        return -1.0;
    }
    let units: Vec<u16> = unsafe { CStr::from_ptr(value) }
        .to_string_lossy()
        .encode_utf16()
        .collect();
    let index = index as usize;
    let Some(&unit) = units.get(index) else {
        return -1.0;
    };
    if (0xD800..=0xDBFF).contains(&unit) {
        if let Some(&low) = units.get(index + 1) {
            if (0xDC00..=0xDFFF).contains(&low) {
                let code_point =
                    (unit as u32 - 0xD800) * 0x400 + (low as u32 - 0xDC00) + 0x10000;
                return code_point as f64;
            }
        }
    }
    unit as f64
}
