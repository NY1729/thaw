fn javascript_number_string(value: f64) -> String {
    if value.is_nan() {
        return "NaN".to_string();
    }
    if value == f64::INFINITY {
        return "Infinity".to_string();
    }
    if value == f64::NEG_INFINITY {
        return "-Infinity".to_string();
    }
    if value == 0.0 {
        return "0".to_string();
    }

    let negative = value.is_sign_negative();
    let mut buffer = ryu::Buffer::new();
    let rendered = buffer.format_finite(value.abs());
    let (mantissa, exponent) = rendered
        .split_once(['e', 'E'])
        .map_or((rendered, 0), |(mantissa, exponent)| {
            (mantissa, exponent.parse::<i32>().unwrap())
        });
    let decimal_position = mantissa.find('.').unwrap_or(mantissa.len()) as i32;
    let mut digits = mantissa
        .bytes()
        .filter(|byte| *byte != b'.')
        .collect::<Vec<_>>();
    let leading = digits.iter().take_while(|digit| **digit == b'0').count();
    digits.drain(..leading);
    let n = decimal_position - leading as i32 + exponent;
    while digits.len() > 1 && digits.last() == Some(&b'0') {
        digits.pop();
    }
    let digits = String::from_utf8(digits).unwrap();
    let mut result = String::new();
    if negative {
        result.push('-');
    }
    if n > 0 && n <= 21 {
        if digits.len() <= n as usize {
            result.push_str(&digits);
            result.extend(std::iter::repeat_n('0', n as usize - digits.len()));
        } else {
            result.push_str(&digits[..n as usize]);
            result.push('.');
            result.push_str(&digits[n as usize..]);
        }
    } else if n <= 0 && n > -6 {
        result.push_str("0.");
        result.extend(std::iter::repeat_n('0', (-n) as usize));
        result.push_str(&digits);
    } else {
        result.push(digits.as_bytes()[0] as char);
        if digits.len() > 1 {
            result.push('.');
            result.push_str(&digits[1..]);
        }
        result.push('e');
        let scientific_exponent = n - 1;
        if scientific_exponent >= 0 {
            result.push('+');
        }
        result.push_str(&scientific_exponent.to_string());
    }
    result
}

#[no_mangle]
pub extern "C" fn thaw_number_to_string(value: f64) -> *const c_char {
    let text = javascript_number_string(value);
    let destination = thaw_arena::thaw_arena_alloc(text.len() + 1, 1);
    if destination.is_null() {
        return std::ptr::null();
    }
    unsafe {
        std::ptr::copy_nonoverlapping(text.as_ptr(), destination, text.len());
        destination.add(text.len()).write(0);
    }
    destination.cast()
}

fn javascript_string_number(text: &str) -> f64 {
    let text =
        text.trim_matches(|character: char| character.is_whitespace() || character == '\u{feff}');
    if text.is_empty() {
        return 0.0;
    }
    match text {
        "Infinity" | "+Infinity" => return f64::INFINITY,
        "-Infinity" => return f64::NEG_INFINITY,
        _ => {}
    }
    if let Some(digits) = text.strip_prefix("0x").or_else(|| text.strip_prefix("0X")) {
        return power_of_two_radix_number(digits, 4);
    }
    if let Some(digits) = text.strip_prefix("0o").or_else(|| text.strip_prefix("0O")) {
        return power_of_two_radix_number(digits, 3);
    }
    if let Some(digits) = text.strip_prefix("0b").or_else(|| text.strip_prefix("0B")) {
        return power_of_two_radix_number(digits, 1);
    }

    let bytes = text.as_bytes();
    let mut index = usize::from(matches!(bytes.first(), Some(b'+') | Some(b'-')));
    let mut integer_digits = 0;
    while bytes.get(index).is_some_and(u8::is_ascii_digit) {
        integer_digits += 1;
        index += 1;
    }
    let mut fraction_digits = 0;
    if bytes.get(index) == Some(&b'.') {
        index += 1;
        while bytes.get(index).is_some_and(u8::is_ascii_digit) {
            fraction_digits += 1;
            index += 1;
        }
    }
    if integer_digits + fraction_digits == 0 {
        return f64::NAN;
    }
    if matches!(bytes.get(index), Some(b'e') | Some(b'E')) {
        index += 1;
        if matches!(bytes.get(index), Some(b'+') | Some(b'-')) {
            index += 1;
        }
        let exponent_start = index;
        while bytes.get(index).is_some_and(u8::is_ascii_digit) {
            index += 1;
        }
        if index == exponent_start {
            return f64::NAN;
        }
    }
    if index != bytes.len() {
        return f64::NAN;
    }
    text.parse().unwrap_or(f64::NAN)
}

fn power_of_two_radix_number(digits: &str, bits_per_digit: usize) -> f64 {
    if digits.is_empty() {
        return f64::NAN;
    }
    let radix = 1u32 << bits_per_digit;
    let mut bits = Vec::with_capacity(digits.len() * bits_per_digit);
    for character in digits.chars() {
        let Some(value) = character.to_digit(radix) else {
            return f64::NAN;
        };
        for shift in (0..bits_per_digit).rev() {
            bits.push((value & (1 << shift)) != 0);
        }
    }
    let Some(first_one) = bits.iter().position(|bit| *bit) else {
        return 0.0;
    };
    let bits = &bits[first_one..];
    if bits.len() <= 53 {
        return bits
            .iter()
            .fold(0u64, |value, bit| (value << 1) | u64::from(*bit)) as f64;
    }

    let mut significand = bits[..53]
        .iter()
        .fold(0u64, |value, bit| (value << 1) | u64::from(*bit));
    let halfway = bits[53];
    let sticky = bits[54..].iter().any(|bit| *bit);
    if halfway && (sticky || significand & 1 != 0) {
        significand += 1;
    }
    let mut exponent = bits.len() - 1;
    if significand == 1u64 << 53 {
        significand >>= 1;
        exponent += 1;
    }
    if exponent > 1023 {
        return f64::INFINITY;
    }
    let exponent_bits = ((exponent as u64 + 1023) << 52) & 0x7ff0_0000_0000_0000;
    let fraction_bits = significand & 0x000f_ffff_ffff_ffff;
    f64::from_bits(exponent_bits | fraction_bits)
}

#[no_mangle]
/// # Safety
///
/// `value` must be null or point to a valid NUL-terminated C string.
pub unsafe extern "C" fn thaw_string_to_number(value: *const c_char) -> f64 {
    if value.is_null() {
        return f64::NAN;
    }
    let text = unsafe { CStr::from_ptr(value) }.to_string_lossy();
    javascript_string_number(&text)
}

fn javascript_parse_float(text: &str) -> f64 {
    let text = text
        .trim_start_matches(|character: char| character.is_whitespace() || character == '\u{feff}');
    let bytes = text.as_bytes();
    let mut index = usize::from(matches!(bytes.first(), Some(b'+') | Some(b'-')));
    if text
        .get(index..)
        .is_some_and(|rest| rest.starts_with("Infinity"))
    {
        return if bytes.first() == Some(&b'-') {
            f64::NEG_INFINITY
        } else {
            f64::INFINITY
        };
    }
    let mut digits = 0;
    while bytes.get(index).is_some_and(u8::is_ascii_digit) {
        digits += 1;
        index += 1;
    }
    if bytes.get(index) == Some(&b'.') {
        index += 1;
        while bytes.get(index).is_some_and(u8::is_ascii_digit) {
            digits += 1;
            index += 1;
        }
    }
    if digits == 0 {
        return f64::NAN;
    }
    if matches!(bytes.get(index), Some(b'e') | Some(b'E')) {
        let exponent_mark = index;
        index += 1;
        if matches!(bytes.get(index), Some(b'+') | Some(b'-')) {
            index += 1;
        }
        let exponent_start = index;
        while bytes.get(index).is_some_and(u8::is_ascii_digit) {
            index += 1;
        }
        if index == exponent_start {
            index = exponent_mark;
        }
    }
    text[..index].parse().unwrap_or(f64::NAN)
}

fn javascript_parse_int(text: &str, radix: f64) -> f64 {
    let mut text = text
        .trim_start_matches(|character: char| character.is_whitespace() || character == '\u{feff}');
    let negative = text.starts_with('-');
    if matches!(text.as_bytes().first(), Some(b'+') | Some(b'-')) {
        text = &text[1..];
    }
    let radix = if radix.is_finite() {
        let unsigned = radix.trunc().rem_euclid(4_294_967_296.0) as u32;
        unsigned as i32
    } else {
        0
    };
    if radix != 0 && !(2..=36).contains(&radix) {
        return f64::NAN;
    }
    let mut radix = radix as u32;
    let has_hex_prefix = text.starts_with("0x") || text.starts_with("0X");
    if radix == 0 {
        radix = if has_hex_prefix { 16 } else { 10 };
    }
    if radix == 16 && has_hex_prefix {
        text = &text[2..];
    }
    let mut value = 0.0;
    let mut digits = 0;
    for character in text.chars() {
        let Some(digit) = character.to_digit(radix) else {
            break;
        };
        value = value * f64::from(radix) + f64::from(digit);
        digits += 1;
    }
    if digits == 0 {
        return f64::NAN;
    }
    if negative {
        -value
    } else {
        value
    }
}

#[no_mangle]
/// # Safety
///
/// `value` must be null or point to a valid NUL-terminated C string.
pub unsafe extern "C" fn thaw_parse_float(value: *const c_char) -> f64 {
    if value.is_null() {
        return f64::NAN;
    }
    let text = unsafe { CStr::from_ptr(value) }.to_string_lossy();
    javascript_parse_float(&text)
}

#[no_mangle]
/// # Safety
///
/// `value` must be null or point to a valid NUL-terminated C string.
pub unsafe extern "C" fn thaw_parse_int(value: *const c_char, radix: f64) -> f64 {
    if value.is_null() {
        return f64::NAN;
    }
    let text = unsafe { CStr::from_ptr(value) }.to_string_lossy();
    javascript_parse_int(&text, radix)
}

fn javascript_to_uint32(value: f64) -> u32 {
    if !value.is_finite() || value == 0.0 {
        0
    } else {
        value.trunc().rem_euclid(4_294_967_296.0) as u32
    }
}

#[no_mangle]
pub extern "C" fn thaw_math_fround(value: f64) -> f64 {
    f64::from(value as f32)
}

#[no_mangle]
pub extern "C" fn thaw_math_clz32(value: f64) -> f64 {
    f64::from(javascript_to_uint32(value).leading_zeros())
}

#[no_mangle]
pub extern "C" fn thaw_math_imul(left: f64, right: f64) -> f64 {
    let result = javascript_to_uint32(left).wrapping_mul(javascript_to_uint32(right));
    f64::from(result as i32)
}

static MATH_RANDOM_STATE: std::sync::atomic::AtomicU64 =
    std::sync::atomic::AtomicU64::new(0x6a09_e667_f3bc_c909);

#[no_mangle]
pub extern "C" fn thaw_math_random() -> f64 {
    use std::sync::atomic::Ordering;

    let mut current = MATH_RANDOM_STATE.load(Ordering::Relaxed);
    loop {
        let mut next = current;
        next ^= next << 13;
        next ^= next >> 7;
        next ^= next << 17;
        match MATH_RANDOM_STATE.compare_exchange_weak(
            current,
            next,
            Ordering::Relaxed,
            Ordering::Relaxed,
        ) {
            Ok(_) => return ((next >> 11) as f64) * (1.0 / 9_007_199_254_740_992.0),
            Err(observed) => current = observed,
        }
    }
}

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
/// Converts a UTF-8 string into the native `string[]` array layout, following
/// JavaScript string-iterator semantics (one Unicode scalar value per slot).
///
/// # Safety
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

unsafe fn native_array_length(array: *const u8) -> Option<usize> {
    (!array.is_null()).then(|| unsafe { array.cast::<u64>().read() as usize })
}

#[no_mangle]
pub extern "C" fn thaw_number_object_is(left: f64, right: f64) -> u8 {
    if left.is_nan() && right.is_nan() {
        return 1;
    }
    if left == 0.0 && right == 0.0 {
        return (left.is_sign_negative() == right.is_sign_negative()).into();
    }
    (left == right).into()
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

pub const THAW_FD_READABLE: u8 = 1;
pub const THAW_FD_WRITABLE: u8 = 2;

pub type HandlerFn = extern "C" fn(*const c_char) -> *const c_char;
pub type HandlerErrorSlot = *mut *const c_char;
pub type PromiseResumeFn = extern "C" fn(*mut u8, *const u8);
pub type PromiseTransformFn = extern "C" fn(*mut u8, *mut ThawPromise, *const u8);
pub type PromiseFinallyFn = extern "C" fn(*mut u8, *mut ThawPromise, *const u8, u8);
pub type FdWatcherFn = extern "C" fn(*mut u8, i16);
