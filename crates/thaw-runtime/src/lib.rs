//! A minimal AWS Lambda custom runtime client. This is what turns a
//! compiled Thaw program with a `function handler(event: string): string`
//! into an actual deployable Lambda function: it polls the Runtime API for
//! the next invocation, calls the handler with the raw event JSON, and
//! posts the handler's raw string result back.
//!
//! Deliberately a hand-rolled blocking HTTP/1.1 client over `TcpStream`
//! rather than hyper/tokio: the Runtime API is a fixed, local-only, plain
//! HTTP endpoint with no concurrency to speak of (one invocation at a time
//! per execution environment), and pulling in a full async runtime here
//! would fight the entire point of Thaw -- small binaries, fast cold
//! start. User-code `fetch()` uses the separate fd-driven HTTP/TLS state
//! machine later in this crate; the blocking client here remains specific to
//! the Lambda Runtime API.
//!
//! Known limitations, acceptable for what this talks to: no TLS, no
//! chunked transfer-encoding, one request per TCP connection.

use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::ffi::{CStr, CString};
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream, ToSocketAddrs};
use std::os::raw::c_char;
use std::os::unix::io::RawFd;
use std::sync::{Arc, Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant};

use rustls::pki_types::ServerName;
use rustls::{ClientConfig, ClientConnection, RootCertStore};

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

unsafe fn native_array_length(array: *const u8) -> Option<usize> {
    (!array.is_null()).then(|| unsafe { array.cast::<u64>().read() as usize })
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

pub const THAW_FD_READABLE: u8 = 1;
pub const THAW_FD_WRITABLE: u8 = 2;

pub type HandlerFn = extern "C" fn(*const c_char) -> *const c_char;
pub type HandlerErrorSlot = *mut *const c_char;
pub type PromiseResumeFn = extern "C" fn(*mut u8, *const u8);
pub type PromiseTransformFn = extern "C" fn(*mut u8, *mut ThawPromise, *const u8);
pub type PromiseFinallyFn = extern "C" fn(*mut u8, *mut ThawPromise, *const u8, u8);
pub type FdWatcherFn = extern "C" fn(*mut u8, i16);

#[derive(Clone, Copy)]
struct PromiseSubscription {
    resume: PromiseResumeFn,
    frame: *mut u8,
}

thread_local! {
    static READY_CONTINUATIONS: RefCell<VecDeque<(PromiseSubscription, *const u8)>> =
        const { RefCell::new(VecDeque::new()) };
    static TIMERS: RefCell<Vec<PromiseTimer>> = const { RefCell::new(Vec::new()) };
    static FD_WAITS: RefCell<Vec<PromiseFdWait>> = const { RefCell::new(Vec::new()) };
    static FD_WATCHERS: RefCell<Vec<FdWatcher>> = const { RefCell::new(Vec::new()) };
    static ACTIVE_PROMISE_JOINS: Cell<usize> = const { Cell::new(0) };
}

struct PromiseTimer {
    deadline: Instant,
    promise: *mut ThawPromise,
}

struct PromiseFdWait {
    fd: libc::c_int,
    interests: u8,
    promise: *mut ThawPromise,
    deadline: Option<Instant>,
}

#[derive(Clone, Copy)]
struct FdWatcher {
    id: u64,
    fd: libc::c_int,
    interests: u8,
    callback: FdWatcherFn,
    context: *mut u8,
}

static NEXT_FD_WATCHER_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

static INVALID_FD_ERROR: &[u8] = b"invalid file descriptor\0";
static FD_TIMEOUT_ERROR: &[u8] = b"file descriptor wait timed out\0";
static PROMISE_ALL_INVALID_ERROR: &[u8] = b"Promise.all received an invalid promise\0";
static PROMISE_RACE_EMPTY_ERROR: &[u8] = b"Promise.race requires at least one promise\0";
static PROMISE_RACE_INVALID_ERROR: &[u8] = b"Promise.race received an invalid promise\0";
static PROMISE_CYCLE_ERROR: &[u8] = b"Chaining cycle detected for promise\0";
static PROMISE_ANY_REJECTED_ERROR: &[u8] = b"All promises were rejected\0";
static PROMISE_SETTLED_FULFILLED: &[u8] = b"fulfilled\0";
static PROMISE_SETTLED_REJECTED: &[u8] = b"rejected\0";
static PROMISE_SETTLED_EMPTY_REASON: &[u8] = b"\0";

fn poll_fd_waits(timeout: Option<Duration>) -> usize {
    let wait_count = FD_WAITS.with(|waits| waits.borrow().len());
    let mut pollfds = FD_WAITS.with(|waits| {
        waits
            .borrow()
            .iter()
            .map(|wait| libc::pollfd {
                fd: wait.fd,
                events: (if wait.interests & THAW_FD_READABLE != 0 {
                    libc::POLLIN
                } else {
                    0
                }) | (if wait.interests & THAW_FD_WRITABLE != 0 {
                    libc::POLLOUT
                } else {
                    0
                }),
                revents: 0,
            })
            .collect::<Vec<_>>()
    });
    FD_WATCHERS.with(|watchers| {
        pollfds.extend(watchers.borrow().iter().map(|watcher| libc::pollfd {
            fd: watcher.fd,
            events: poll_events(watcher.interests),
            revents: 0,
        }));
    });
    if pollfds.is_empty() {
        return 0;
    }
    let fd_delay = FD_WAITS.with(|waits| {
        let now = Instant::now();
        waits
            .borrow()
            .iter()
            .filter_map(|wait| wait.deadline)
            .map(|deadline| deadline.saturating_duration_since(now))
            .min()
    });
    let timeout = match (timeout, fd_delay) {
        (Some(left), Some(right)) => Some(left.min(right)),
        (Some(delay), None) | (None, Some(delay)) => Some(delay),
        (None, None) => None,
    };
    let timeout_ms = match timeout {
        Some(duration) => duration.as_millis().saturating_add(1).min(i32::MAX as u128) as i32,
        None => -1,
    };
    let ready = unsafe {
        libc::poll(
            pollfds.as_mut_ptr(),
            pollfds.len() as libc::nfds_t,
            timeout_ms,
        )
    };
    if ready < 0 {
        return 0;
    }

    let completed = FD_WAITS.with(|waits| {
        let mut waits = waits.borrow_mut();
        let mut completed = Vec::new();
        let now = Instant::now();
        for index in (0..wait_count).rev() {
            let timed_out = waits[index]
                .deadline
                .is_some_and(|deadline| deadline <= now);
            if pollfds[index].revents != 0 || timed_out {
                completed.push((
                    waits.swap_remove(index).promise,
                    pollfds[index].revents,
                    timed_out,
                ));
            }
        }
        completed
    });
    let count = completed.len();
    for (promise, events, timed_out) in completed {
        if timed_out && events == 0 {
            thaw_promise_reject(promise, FD_TIMEOUT_ERROR.as_ptr());
        } else if events & libc::POLLNVAL != 0 {
            thaw_promise_reject(promise, INVALID_FD_ERROR.as_ptr());
        } else {
            thaw_promise_resolve(promise, std::ptr::dangling::<u8>());
        }
    }
    let ready_watchers = FD_WATCHERS.with(|watchers| {
        watchers
            .borrow()
            .iter()
            .zip(&pollfds[wait_count..])
            .filter(|(_, pollfd)| pollfd.revents != 0)
            .map(|(watcher, pollfd)| (*watcher, pollfd.revents))
            .collect::<Vec<_>>()
    });
    let watcher_count = ready_watchers.len();
    for (watcher, events) in ready_watchers {
        (watcher.callback)(watcher.context, events);
    }
    count + watcher_count
}

fn has_fd_waits() -> bool {
    FD_WAITS.with(|waits| !waits.borrow().is_empty())
        || FD_WATCHERS.with(|watchers| !watchers.borrow().is_empty())
}

fn poll_events(interests: u8) -> i16 {
    (if interests & THAW_FD_READABLE != 0 {
        libc::POLLIN
    } else {
        0
    }) | (if interests & THAW_FD_WRITABLE != 0 {
        libc::POLLOUT
    } else {
        0
    })
}

#[no_mangle]
pub extern "C" fn thaw_runtime_watch_fd(
    fd: libc::c_int,
    interests: u8,
    callback: FdWatcherFn,
    context: *mut u8,
) -> u64 {
    if fd < 0 || interests == 0 || interests & !(THAW_FD_READABLE | THAW_FD_WRITABLE) != 0 {
        return 0;
    }
    let id = NEXT_FD_WATCHER_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    FD_WATCHERS.with(|watchers| {
        watchers.borrow_mut().push(FdWatcher {
            id,
            fd,
            interests,
            callback,
            context,
        });
    });
    id
}

#[no_mangle]
pub extern "C" fn thaw_runtime_unwatch_fd(id: u64) -> bool {
    FD_WATCHERS.with(|watchers| {
        let mut watchers = watchers.borrow_mut();
        let Some(index) = watchers.iter().position(|watcher| watcher.id == id) else {
            return false;
        };
        watchers.swap_remove(index);
        true
    })
}

/// Blocks until the next timer or fd event, dispatches it, and drains one
/// queued continuation. Returns false when no event source remains.
#[no_mangle]
pub extern "C" fn thaw_runtime_run_one_event() -> bool {
    promote_due_timers();
    if thaw_runtime_poll_one() != 0 {
        return true;
    }
    let delay = next_timer_delay();
    if has_fd_waits() {
        poll_fd_waits(delay);
        let _ = thaw_runtime_poll_one();
        true
    } else if let Some(delay) = delay {
        if !delay.is_zero() {
            thread::sleep(delay);
        }
        promote_due_timers();
        let _ = thaw_runtime_poll_one();
        true
    } else {
        false
    }
}

fn enqueue_continuation(subscription: PromiseSubscription, result: *const u8) {
    READY_CONTINUATIONS.with(|ready| ready.borrow_mut().push_back((subscription, result)));
}

fn promote_due_timers() {
    let now = Instant::now();
    let due = TIMERS.with(|timers| {
        let mut timers = timers.borrow_mut();
        let mut due = Vec::new();
        let mut i = 0;
        while i < timers.len() {
            if timers[i].deadline <= now {
                due.push(timers.swap_remove(i).promise);
            } else {
                i += 1;
            }
        }
        due
    });
    for promise in due {
        // A timer has no payload; a non-null sentinel distinguishes a
        // resolved timer from an invalid/no-progress result at the ABI edge.
        let result = std::ptr::dangling::<u8>();
        thaw_promise_resolve(promise, result);
    }
}

fn next_timer_delay() -> Option<Duration> {
    let now = Instant::now();
    TIMERS.with(|timers| {
        timers
            .borrow()
            .iter()
            .map(|timer| timer.deadline.saturating_duration_since(now))
            .min()
    })
}

/// Runs one ready continuation or dispatches ready I/O and returns 1. Returns
/// 0 when neither source is ready. I/O and QuickJS integrations can alternate
/// their own polling with this function without a multi-threaded executor.
#[no_mangle]
pub extern "C" fn thaw_runtime_poll_one() -> u8 {
    promote_due_timers();
    let io_events = poll_fd_waits(Some(Duration::ZERO));
    let next = READY_CONTINUATIONS.with(|ready| ready.borrow_mut().pop_front());
    let Some((subscription, result)) = next else {
        return u8::from(io_events != 0);
    };
    (subscription.resume)(subscription.frame, result);
    1
}

/// Returns a Promise which settles when `fd` becomes readable or writable.
/// `interests` is a bitmask of `THAW_FD_READABLE`/`THAW_FD_WRITABLE`.
/// The caller retains ownership of the descriptor and must keep it open until
/// settlement or Promise destruction.
#[no_mangle]
pub extern "C" fn thaw_runtime_wait_fd(fd: libc::c_int, interests: u8) -> *mut ThawPromise {
    thaw_runtime_wait_fd_until(fd, interests, None)
}

/// Like `thaw_runtime_wait_fd`, but rejects if readiness is not observed
/// within `milliseconds`.
#[no_mangle]
pub extern "C" fn thaw_runtime_wait_fd_timeout(
    fd: libc::c_int,
    interests: u8,
    milliseconds: u64,
) -> *mut ThawPromise {
    thaw_runtime_wait_fd_until(
        fd,
        interests,
        Some(Instant::now() + Duration::from_millis(milliseconds)),
    )
}

fn thaw_runtime_wait_fd_until(
    fd: libc::c_int,
    interests: u8,
    deadline: Option<Instant>,
) -> *mut ThawPromise {
    let promise = thaw_promise_new();
    if fd < 0 || interests == 0 || interests & !(THAW_FD_READABLE | THAW_FD_WRITABLE) != 0 {
        thaw_promise_reject(promise, INVALID_FD_ERROR.as_ptr());
        return promise;
    }
    FD_WAITS.with(|waits| {
        waits.borrow_mut().push(PromiseFdWait {
            fd,
            interests,
            promise,
            deadline,
        });
    });
    promise
}

#[derive(Clone, Copy)]
enum AsyncHttpState {
    Resolving,
    Connecting,
    TlsHandshaking,
    Writing,
    Reading,
}

struct AsyncHttpGet {
    fd: RawFd,
    request: Vec<u8>,
    written: usize,
    response: Vec<u8>,
    state: AsyncHttpState,
    completion: *mut ThawPromise,
    readiness: *mut ThawPromise,
    deadline: Instant,
    tls: Option<ClientConnection>,
    tls_config: Arc<ClientConfig>,
    use_tls: bool,
    host: String,
    port: u16,
    path: String,
    redirects: usize,
    resolution: Option<SharedDnsResolution>,
}

type SharedDnsResolution = Arc<Mutex<Option<Result<Vec<SocketAddr>, String>>>>;

impl Drop for AsyncHttpGet {
    fn drop(&mut self) {
        if self.fd >= 0 {
            unsafe { libc::close(self.fd) };
        }
    }
}

fn parse_http_url(url: &str) -> Result<(bool, String, u16, String), String> {
    let (tls, rest, default_port) = if let Some(rest) = url.strip_prefix("http://") {
        (false, rest, 80)
    } else if let Some(rest) = url.strip_prefix("https://") {
        (true, rest, 443)
    } else {
        return Err("async fetch URL must use http:// or https://".to_string());
    };
    let (authority, path) = match rest.split_once('/') {
        Some((authority, path)) => (authority, format!("/{path}")),
        None => (rest, "/".to_string()),
    };
    if authority.is_empty() {
        return Err("HTTP URL is missing a host".to_string());
    }
    let (host, port) = if let Some(bracketed) = authority.strip_prefix('[') {
        let (host, suffix) = bracketed
            .split_once(']')
            .ok_or("malformed bracketed IPv6 host")?;
        let port = match suffix.strip_prefix(':') {
            Some(port) => port.parse().map_err(|_| "invalid HTTP port")?,
            None if suffix.is_empty() => default_port,
            None => return Err("malformed bracketed IPv6 authority".to_string()),
        };
        (host.to_string(), port)
    } else if let Some((host, port)) = authority.rsplit_once(':') {
        if port.chars().all(|ch| ch.is_ascii_digit()) {
            (
                host.to_string(),
                port.parse().map_err(|_| "invalid HTTP port")?,
            )
        } else {
            (authority.to_string(), default_port)
        }
    } else {
        (authority.to_string(), default_port)
    };
    let path = path
        .split_once('#')
        .map(|(path, _)| path.to_string())
        .unwrap_or(path);
    Ok((tls, host, port, path))
}

fn tls_client_config() -> Arc<ClientConfig> {
    static CONFIG: OnceLock<Arc<ClientConfig>> = OnceLock::new();
    CONFIG
        .get_or_init(|| {
            let roots = RootCertStore::from_iter(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
            Arc::new(
                ClientConfig::builder()
                    .with_root_certificates(roots)
                    .with_no_client_auth(),
            )
        })
        .clone()
}

fn open_nonblocking_socket(address: SocketAddr) -> Result<RawFd, String> {
    let domain = if address.is_ipv4() {
        libc::AF_INET
    } else {
        libc::AF_INET6
    };
    let fd = unsafe {
        libc::socket(
            domain,
            libc::SOCK_STREAM | libc::SOCK_NONBLOCK | libc::SOCK_CLOEXEC,
            0,
        )
    };
    if fd < 0 {
        return Err(format!(
            "creating socket: {}",
            std::io::Error::last_os_error()
        ));
    }
    let result = match address {
        SocketAddr::V4(address) => {
            let raw = libc::sockaddr_in {
                sin_family: libc::AF_INET as libc::sa_family_t,
                sin_port: address.port().to_be(),
                sin_addr: libc::in_addr {
                    s_addr: u32::from_ne_bytes(address.ip().octets()),
                },
                sin_zero: [0; 8],
            };
            unsafe {
                libc::connect(
                    fd,
                    (&raw as *const libc::sockaddr_in).cast(),
                    std::mem::size_of::<libc::sockaddr_in>() as libc::socklen_t,
                )
            }
        }
        SocketAddr::V6(address) => {
            let raw = libc::sockaddr_in6 {
                sin6_family: libc::AF_INET6 as libc::sa_family_t,
                sin6_port: address.port().to_be(),
                sin6_flowinfo: address.flowinfo(),
                sin6_addr: libc::in6_addr {
                    s6_addr: address.ip().octets(),
                },
                sin6_scope_id: address.scope_id(),
            };
            unsafe {
                libc::connect(
                    fd,
                    (&raw as *const libc::sockaddr_in6).cast(),
                    std::mem::size_of::<libc::sockaddr_in6>() as libc::socklen_t,
                )
            }
        }
    };
    if result == 0 || std::io::Error::last_os_error().raw_os_error() == Some(libc::EINPROGRESS) {
        Ok(fd)
    } else {
        let error = std::io::Error::last_os_error();
        unsafe { libc::close(fd) };
        Err(format!("connecting HTTP socket: {error}"))
    }
}

type DnsResult = Arc<Mutex<Option<Result<Vec<SocketAddr>, String>>>>;

fn start_dns_resolution(host: String, port: u16) -> Result<(RawFd, DnsResult), String> {
    let mut pipe = [-1; 2];
    if unsafe { libc::pipe2(pipe.as_mut_ptr(), libc::O_NONBLOCK | libc::O_CLOEXEC) } != 0 {
        return Err(format!(
            "creating DNS completion pipe: {}",
            std::io::Error::last_os_error()
        ));
    }
    let result = Arc::new(Mutex::new(None));
    let worker_result = result.clone();
    let read_fd = pipe[0];
    let write_fd = pipe[1];
    let spawn = thread::Builder::new()
        .name("thaw-dns".to_string())
        .spawn(move || {
            let resolved = (host.as_str(), port)
                .to_socket_addrs()
                .map(|addresses| addresses.collect::<Vec<_>>())
                .map_err(|error| format!("resolving {host}: {error}"))
                .and_then(|addresses| {
                    if addresses.is_empty() {
                        Err(format!("no address found for {host}"))
                    } else {
                        Ok(addresses)
                    }
                });
            *worker_result
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(resolved);
            let byte = [1u8];
            unsafe {
                libc::write(write_fd, byte.as_ptr().cast(), byte.len());
                libc::close(write_fd);
            }
        });
    if let Err(error) = spawn {
        unsafe {
            libc::close(read_fd);
            libc::close(write_fd);
        }
        return Err(format!("starting DNS resolver: {error}"));
    }
    Ok((read_fd, result))
}

struct NonblockingSocket(RawFd);

impl Read for NonblockingSocket {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        let read = unsafe { libc::recv(self.0, buffer.as_mut_ptr().cast(), buffer.len(), 0) };
        if read >= 0 {
            Ok(read as usize)
        } else {
            Err(std::io::Error::last_os_error())
        }
    }
}

impl Write for NonblockingSocket {
    fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
        let written = unsafe {
            libc::send(
                self.0,
                buffer.as_ptr().cast(),
                buffer.len(),
                libc::MSG_NOSIGNAL,
            )
        };
        if written >= 0 {
            Ok(written as usize)
        } else {
            Err(std::io::Error::last_os_error())
        }
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn flush_tls(task: *mut AsyncHttpGet) -> Result<bool, String> {
    let fd = unsafe { (*task).fd };
    let tls = unsafe { (*task).tls.as_mut().expect("TLS state is present") };
    let mut socket = NonblockingSocket(fd);
    while tls.wants_write() {
        match tls.write_tls(&mut socket) {
            Ok(0) => return Err("TLS socket closed while writing".to_string()),
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => return Ok(false),
            Err(error) => return Err(format!("writing TLS records: {error}")),
        }
    }
    Ok(true)
}

fn read_tls_records(task: *mut AsyncHttpGet) -> Result<bool, String> {
    let fd = unsafe { (*task).fd };
    let tls = unsafe { (*task).tls.as_mut().expect("TLS state is present") };
    let mut socket = NonblockingSocket(fd);
    loop {
        match tls.read_tls(&mut socket) {
            Ok(0) => return Ok(true),
            Ok(_) => {
                tls.process_new_packets()
                    .map_err(|error| format!("processing TLS records: {error}"))?;
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => return Ok(false),
            Err(error) => return Err(format!("reading TLS records: {error}")),
        }
    }
}

fn tls_interests(task: *mut AsyncHttpGet) -> u8 {
    let tls = unsafe { (*task).tls.as_ref().expect("TLS state is present") };
    let mut interests = 0;
    if tls.wants_read() {
        interests |= THAW_FD_READABLE;
    }
    if tls.wants_write() {
        interests |= THAW_FD_WRITABLE;
    }
    if interests == 0 {
        THAW_FD_READABLE
    } else {
        interests
    }
}

fn async_http_error(task: *mut AsyncHttpGet, message: String) {
    let task = unsafe { Box::from_raw(task) };
    let error = arena_c_string(&message).unwrap_or(INVALID_FD_ERROR.as_ptr());
    thaw_promise_reject(task.completion, error);
}

fn arena_c_string(value: &str) -> Option<*const u8> {
    let value = CString::new(value).ok()?;
    let bytes = value.as_bytes_with_nul();
    let destination = thaw_arena::thaw_arena_alloc(bytes.len(), 1);
    if destination.is_null() {
        return None;
    }
    unsafe { std::ptr::copy_nonoverlapping(bytes.as_ptr(), destination, bytes.len()) };
    Some(destination)
}

fn arena_pointer_slot(value: *const u8) -> Option<*const u8> {
    let slot = thaw_arena::thaw_arena_alloc(
        std::mem::size_of::<*const u8>(),
        std::mem::align_of::<*const u8>(),
    ) as *mut *const u8;
    if slot.is_null() {
        return None;
    }
    unsafe { slot.write(value) };
    Some(slot.cast())
}

fn http_authority(use_tls: bool, host: &str, port: u16) -> String {
    let host = if host.contains(':') {
        format!("[{host}]")
    } else {
        host.to_string()
    };
    if port == if use_tls { 443 } else { 80 } {
        host
    } else {
        format!("{host}:{port}")
    }
}

fn normalize_http_path(path: &str) -> String {
    let (path, suffix) = path
        .find(['?', '#'])
        .map(|index| (&path[..index], &path[index..]))
        .unwrap_or((path, ""));
    let mut parts = Vec::new();
    for part in path.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            part => parts.push(part),
        }
    }
    format!("/{}{suffix}", parts.join("/"))
}

fn redirect_url(task: &AsyncHttpGet, location: &str) -> Result<String, String> {
    if location.starts_with("http://") || location.starts_with("https://") {
        return Ok(location.to_string());
    }
    let scheme = if task.use_tls { "https" } else { "http" };
    if location.starts_with("//") {
        return Ok(format!("{scheme}:{location}"));
    }
    let authority = http_authority(task.use_tls, &task.host, task.port);
    let path = if location.starts_with('/') {
        normalize_http_path(location)
    } else if location.starts_with('?') || location.starts_with('#') {
        let base = task
            .path
            .find(['?', '#'])
            .map(|index| &task.path[..index])
            .unwrap_or(&task.path);
        format!("{base}{location}")
    } else {
        let directory = task.path.rsplit_once('/').map(|(dir, _)| dir).unwrap_or("");
        normalize_http_path(&format!("{directory}/{location}"))
    };
    Ok(format!("{scheme}://{authority}{path}"))
}

fn restart_async_http(task: *mut AsyncHttpGet, location: &str) -> Result<(), String> {
    let task_ref = unsafe { &mut *task };
    if task_ref.redirects >= 10 {
        return Err("HTTP redirect limit exceeded (10)".to_string());
    }
    let target = redirect_url(task_ref, location)?;
    let (use_tls, host, port, path) = parse_http_url(&target)?;
    let tls = if use_tls {
        let server_name = ServerName::try_from(host.clone())
            .map_err(|_| format!("invalid TLS server name `{host}`"))?;
        Some(
            ClientConnection::new(task_ref.tls_config.clone(), server_name)
                .map_err(|error| format!("creating redirected TLS client: {error}"))?,
        )
    } else {
        None
    };
    let (fd, resolution) = start_dns_resolution(host.clone(), port)?;
    unsafe { libc::close(task_ref.fd) };
    let authority = http_authority(use_tls, &host, port);
    task_ref.fd = fd;
    task_ref.request = format!(
        "GET {path} HTTP/1.1\r\nHost: {authority}\r\nConnection: close\r\nAccept: */*\r\n\r\n"
    )
    .into_bytes();
    task_ref.written = 0;
    task_ref.response.clear();
    task_ref.state = AsyncHttpState::Resolving;
    task_ref.tls = tls;
    task_ref.use_tls = use_tls;
    task_ref.host = host;
    task_ref.port = port;
    task_ref.path = path;
    task_ref.redirects += 1;
    task_ref.resolution = Some(resolution);
    schedule_async_http(task, THAW_FD_READABLE);
    Ok(())
}

struct ParsedHttpResponse {
    status: u16,
    body: Vec<u8>,
    location: Option<String>,
}

fn find_bytes(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

fn decode_chunked(body: &[u8]) -> Result<Option<Vec<u8>>, String> {
    let mut cursor = 0;
    let mut decoded = Vec::new();
    loop {
        let Some(line_end) = find_bytes(&body[cursor..], b"\r\n") else {
            return Ok(None);
        };
        let line_end = cursor + line_end;
        let size_text =
            std::str::from_utf8(&body[cursor..line_end]).map_err(|_| "chunk size is not ASCII")?;
        let size_text = size_text.split(';').next().unwrap_or("").trim();
        let size = usize::from_str_radix(size_text, 16)
            .map_err(|_| format!("invalid chunk size `{size_text}`"))?;
        cursor = line_end + 2;
        if size == 0 {
            if body.len() < cursor + 2 {
                return Ok(None);
            }
            if &body[cursor..cursor + 2] == b"\r\n" {
                return Ok(Some(decoded));
            }
            return if find_bytes(&body[cursor..], b"\r\n\r\n").is_some() {
                Ok(Some(decoded))
            } else {
                Ok(None)
            };
        }
        let chunk_end = cursor
            .checked_add(size)
            .ok_or("chunk size overflows address space")?;
        if body.len() < chunk_end + 2 {
            return Ok(None);
        }
        if &body[chunk_end..chunk_end + 2] != b"\r\n" {
            return Err("chunk data is missing its CRLF terminator".to_string());
        }
        decoded.extend_from_slice(&body[cursor..chunk_end]);
        cursor = chunk_end + 2;
    }
}

fn parse_http_response(response: &[u8], eof: bool) -> Result<Option<ParsedHttpResponse>, String> {
    let Some(header_end) = find_bytes(response, b"\r\n\r\n") else {
        return if eof {
            Err("malformed HTTP response (incomplete headers)".to_string())
        } else {
            Ok(None)
        };
    };
    let head = std::str::from_utf8(&response[..header_end])
        .map_err(|_| "HTTP response headers are not valid UTF-8")?;
    let mut lines = head.lines();
    let status = lines
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|status| status.parse::<u16>().ok())
        .ok_or("malformed HTTP status line")?;
    let mut content_length = None;
    let mut chunked = false;
    let mut location = None;
    for line in lines {
        let Some((name, value)) = line.split_once(':') else {
            return Err(format!("malformed HTTP header `{line}`"));
        };
        if name.eq_ignore_ascii_case("content-length") {
            content_length = Some(
                value
                    .trim()
                    .parse::<usize>()
                    .map_err(|_| "invalid Content-Length header")?,
            );
        }
        if name.eq_ignore_ascii_case("transfer-encoding")
            && value
                .split(',')
                .any(|encoding| encoding.trim().eq_ignore_ascii_case("chunked"))
        {
            chunked = true;
        }
        if name.eq_ignore_ascii_case("location") {
            location = Some(value.trim().to_string());
        }
    }
    let body = &response[header_end + 4..];
    if chunked {
        return decode_chunked(body).map(|body| {
            body.map(|body| ParsedHttpResponse {
                status,
                body,
                location,
            })
        });
    }
    if let Some(length) = content_length {
        if body.len() < length {
            return if eof {
                Err(format!(
                    "HTTP body ended after {} bytes, expected {length}",
                    body.len()
                ))
            } else {
                Ok(None)
            };
        }
        return Ok(Some(ParsedHttpResponse {
            status,
            body: body[..length].to_vec(),
            location,
        }));
    }
    if eof {
        Ok(Some(ParsedHttpResponse {
            status,
            body: body.to_vec(),
            location,
        }))
    } else {
        Ok(None)
    }
}

fn async_http_finish(task: *mut AsyncHttpGet, response: ParsedHttpResponse) {
    if matches!(response.status, 301 | 302 | 303 | 307 | 308) {
        if let Some(location) = &response.location {
            if let Err(error) = restart_async_http(task, location) {
                async_http_error(task, error);
            }
            return;
        }
    }
    let task = unsafe { Box::from_raw(task) };
    let completion = task.completion;
    if !(200..=399).contains(&response.status) {
        let message = format!("HTTP request failed with status {}", response.status);
        drop(task);
        let error = arena_c_string(&message).unwrap_or(INVALID_FD_ERROR.as_ptr());
        thaw_promise_reject(completion, error);
        return;
    }
    let body = std::str::from_utf8(&response.body)
        .ok()
        .and_then(arena_c_string)
        .ok_or(());
    drop(task);
    match body {
        Ok(body) => {
            if let Some(result_slot) = arena_pointer_slot(body) {
                thaw_promise_resolve(completion, result_slot);
            } else {
                thaw_promise_reject(completion, INVALID_FD_ERROR.as_ptr());
            }
        }
        Err(_) => {
            let error = arena_c_string("HTTP body contains a NUL byte")
                .unwrap_or(INVALID_FD_ERROR.as_ptr());
            thaw_promise_reject(completion, error);
        }
    }
}

fn schedule_async_http(task: *mut AsyncHttpGet, interests: u8) {
    let task_ref = unsafe { &*task };
    let remaining = task_ref.deadline.saturating_duration_since(Instant::now());
    let milliseconds = remaining.as_millis().max(1).min(u64::MAX as u128) as u64;
    let readiness = thaw_runtime_wait_fd_timeout(task_ref.fd, interests, milliseconds);
    unsafe { (*task).readiness = readiness };
    unsafe { thaw_promise_subscribe(readiness, resume_async_http, task.cast()) };
}

fn complete_async_http_if_ready(task: *mut AsyncHttpGet, eof: bool) -> bool {
    let parsed = unsafe { parse_http_response(&(*task).response, eof) };
    match parsed {
        Ok(Some(response)) => {
            async_http_finish(task, response);
            true
        }
        Ok(None) => false,
        Err(error) => {
            async_http_error(task, error);
            true
        }
    }
}

extern "C" fn resume_async_http(frame: *mut u8, _result: *const u8) {
    let task = frame.cast::<AsyncHttpGet>();
    let readiness = unsafe { (*task).readiness };
    let readiness_state = unsafe { thaw_promise_state(readiness) };
    unsafe { thaw_promise_destroy(readiness) };
    unsafe { (*task).readiness = std::ptr::null_mut() };
    if readiness_state == 2 {
        async_http_error(
            task,
            "HTTP operation timed out or descriptor failed".to_string(),
        );
        return;
    }

    loop {
        match unsafe { (*task).state } {
            AsyncHttpState::Resolving => {
                let mut byte = [0u8; 1];
                unsafe {
                    libc::read((*task).fd, byte.as_mut_ptr().cast(), byte.len());
                    libc::close((*task).fd);
                    (*task).fd = -1;
                }
                let resolution = unsafe { (*task).resolution.take() }.and_then(|result| {
                    result
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner())
                        .take()
                });
                let addresses = match resolution {
                    Some(Ok(addresses)) => addresses,
                    Some(Err(error)) => {
                        async_http_error(task, error);
                        return;
                    }
                    None => {
                        async_http_error(
                            task,
                            "DNS resolver completed without a result".to_string(),
                        );
                        return;
                    }
                };
                let mut last_error = None;
                let mut socket = None;
                for address in addresses {
                    match open_nonblocking_socket(address) {
                        Ok(fd) => {
                            socket = Some(fd);
                            break;
                        }
                        Err(error) => last_error = Some(error),
                    }
                }
                let Some(fd) = socket else {
                    async_http_error(
                        task,
                        last_error.unwrap_or_else(|| "DNS returned no usable address".to_string()),
                    );
                    return;
                };
                unsafe {
                    (*task).fd = fd;
                    (*task).state = AsyncHttpState::Connecting;
                }
                schedule_async_http(task, THAW_FD_WRITABLE);
                return;
            }
            AsyncHttpState::Connecting => {
                let mut error = 0;
                let mut length = std::mem::size_of::<libc::c_int>() as libc::socklen_t;
                let result = unsafe {
                    libc::getsockopt(
                        (*task).fd,
                        libc::SOL_SOCKET,
                        libc::SO_ERROR,
                        (&mut error as *mut libc::c_int).cast(),
                        &mut length,
                    )
                };
                if result != 0 || error != 0 {
                    let error = if error != 0 {
                        std::io::Error::from_raw_os_error(error)
                    } else {
                        std::io::Error::last_os_error()
                    };
                    async_http_error(task, format!("connecting HTTP socket: {error}"));
                    return;
                }
                unsafe {
                    (*task).state = if (*task).tls.is_some() {
                        AsyncHttpState::TlsHandshaking
                    } else {
                        AsyncHttpState::Writing
                    }
                };
            }
            AsyncHttpState::TlsHandshaking => {
                if let Err(error) = flush_tls(task) {
                    async_http_error(task, error);
                    return;
                }
                let eof = match read_tls_records(task) {
                    Ok(eof) => eof,
                    Err(error) => {
                        async_http_error(task, error);
                        return;
                    }
                };
                if eof {
                    async_http_error(task, "TLS peer closed during handshake".to_string());
                    return;
                }
                if unsafe { !(*task).tls.as_ref().unwrap().is_handshaking() } {
                    unsafe { (*task).state = AsyncHttpState::Writing };
                    continue;
                }
                schedule_async_http(task, tls_interests(task));
                return;
            }
            AsyncHttpState::Writing => {
                if unsafe { (*task).tls.is_some() } {
                    let task_ref = unsafe { &mut *task };
                    while task_ref.written < task_ref.request.len() {
                        let remaining = &task_ref.request[task_ref.written..];
                        match task_ref.tls.as_mut().unwrap().writer().write(remaining) {
                            Ok(0) => break,
                            Ok(written) => task_ref.written += written,
                            Err(error) => {
                                async_http_error(task, format!("buffering TLS request: {error}"));
                                return;
                            }
                        }
                    }
                    let flushed = match flush_tls(task) {
                        Ok(flushed) => flushed,
                        Err(error) => {
                            async_http_error(task, error);
                            return;
                        }
                    };
                    if unsafe { (*task).written == (*task).request.len() } && flushed {
                        unsafe { (*task).state = AsyncHttpState::Reading };
                        continue;
                    }
                    schedule_async_http(task, tls_interests(task));
                    return;
                }
                let task_ref = unsafe { &mut *task };
                while task_ref.written < task_ref.request.len() {
                    let remaining = &task_ref.request[task_ref.written..];
                    let written = unsafe {
                        libc::send(
                            task_ref.fd,
                            remaining.as_ptr().cast(),
                            remaining.len(),
                            libc::MSG_NOSIGNAL,
                        )
                    };
                    if written > 0 {
                        task_ref.written += written as usize;
                        continue;
                    }
                    let error = std::io::Error::last_os_error();
                    if error.kind() == std::io::ErrorKind::WouldBlock {
                        schedule_async_http(task, THAW_FD_WRITABLE);
                    } else {
                        async_http_error(task, format!("writing HTTP request: {error}"));
                    }
                    return;
                }
                task_ref.state = AsyncHttpState::Reading;
            }
            AsyncHttpState::Reading => {
                if unsafe { (*task).tls.is_some() } {
                    if let Err(error) = flush_tls(task) {
                        async_http_error(task, error);
                        return;
                    }
                    let eof = match read_tls_records(task) {
                        Ok(eof) => eof,
                        Err(error) => {
                            async_http_error(task, error);
                            return;
                        }
                    };
                    let mut plaintext = [0u8; 8192];
                    loop {
                        let read =
                            unsafe { (*task).tls.as_mut().unwrap().reader().read(&mut plaintext) };
                        match read {
                            Ok(0) => break,
                            Ok(read) => {
                                unsafe { &mut (*task).response }
                                    .extend_from_slice(&plaintext[..read]);
                                if complete_async_http_if_ready(task, false) {
                                    return;
                                }
                            }
                            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => break,
                            Err(error) if error.kind() == std::io::ErrorKind::UnexpectedEof => {
                                if complete_async_http_if_ready(task, true) {
                                    return;
                                }
                                async_http_error(
                                    task,
                                    "TLS peer closed before the HTTP response completed"
                                        .to_string(),
                                );
                                return;
                            }
                            Err(error) => {
                                async_http_error(
                                    task,
                                    format!("reading decrypted HTTP response: {error}"),
                                );
                                return;
                            }
                        }
                    }
                    if complete_async_http_if_ready(task, eof) {
                        return;
                    }
                    if eof {
                        async_http_error(
                            task,
                            "TLS peer closed before the HTTP response completed".to_string(),
                        );
                        return;
                    }
                    schedule_async_http(task, tls_interests(task));
                    return;
                }
                let mut buffer = [0u8; 8192];
                loop {
                    let read = unsafe {
                        libc::recv((*task).fd, buffer.as_mut_ptr().cast(), buffer.len(), 0)
                    };
                    if read > 0 {
                        unsafe { &mut (*task).response }
                            .extend_from_slice(&buffer[..read as usize]);
                        if complete_async_http_if_ready(task, false) {
                            return;
                        }
                        continue;
                    }
                    if read == 0 {
                        if !complete_async_http_if_ready(task, true) {
                            async_http_error(
                                task,
                                "HTTP peer closed before the response completed".to_string(),
                            );
                        }
                        return;
                    }
                    let error = std::io::Error::last_os_error();
                    if error.kind() == std::io::ErrorKind::WouldBlock {
                        schedule_async_http(task, THAW_FD_READABLE);
                    } else {
                        async_http_error(task, format!("reading HTTP response: {error}"));
                    }
                    return;
                }
            }
        }
    }
}

/// Starts a non-blocking HTTP GET and returns its completion Promise. DNS,
/// connect, write, and read completion are all driven through the event loop.
#[no_mangle]
pub extern "C" fn thaw_http_get_async(url: *const c_char) -> *mut ThawPromise {
    thaw_http_get_async_timeout(url, 30_000)
}

/// Starts a non-blocking HTTP GET with a total connect/write/read timeout.
#[no_mangle]
pub extern "C" fn thaw_http_get_async_timeout(
    url: *const c_char,
    timeout_ms: u64,
) -> *mut ThawPromise {
    thaw_http_get_async_with_config(url, timeout_ms, None)
}

fn thaw_http_get_async_with_config(
    url: *const c_char,
    timeout_ms: u64,
    tls_config: Option<Arc<ClientConfig>>,
) -> *mut ThawPromise {
    let completion = thaw_promise_new();
    let start = (|| -> Result<*mut AsyncHttpGet, String> {
        let url = unsafe { CStr::from_ptr(url) }
            .to_str()
            .map_err(|_| "HTTP URL is not valid UTF-8")?;
        let (use_tls, host, port, path) = parse_http_url(url)?;
        let tls_config = tls_config.unwrap_or_else(tls_client_config);
        let tls = if use_tls {
            let server_name = ServerName::try_from(host.clone())
                .map_err(|_| format!("invalid TLS server name `{host}`"))?;
            Some(
                ClientConnection::new(tls_config.clone(), server_name)
                    .map_err(|error| format!("creating TLS client: {error}"))?,
            )
        } else {
            None
        };
        let (fd, resolution) = start_dns_resolution(host.clone(), port)?;
        let authority = http_authority(use_tls, &host, port);
        let request = format!(
            "GET {path} HTTP/1.1\r\nHost: {authority}\r\nConnection: close\r\nAccept: */*\r\n\r\n"
        )
        .into_bytes();
        Ok(Box::into_raw(Box::new(AsyncHttpGet {
            fd,
            request,
            written: 0,
            response: Vec::new(),
            state: AsyncHttpState::Resolving,
            completion,
            readiness: std::ptr::null_mut(),
            deadline: Instant::now() + Duration::from_millis(timeout_ms),
            tls,
            tls_config,
            use_tls,
            host,
            port,
            path,
            redirects: 0,
            resolution: Some(resolution),
        })))
    })();
    match start {
        Ok(task) => schedule_async_http(task, THAW_FD_READABLE),
        Err(message) => {
            let error = arena_c_string(&message).unwrap_or(INVALID_FD_ERROR.as_ptr());
            thaw_promise_reject(completion, error);
        }
    }
    completion
}

/// Drains all continuations which are ready now and returns how many ran.
/// Callbacks may enqueue more work; that work is included in the same drain.
#[no_mangle]
pub extern "C" fn thaw_runtime_run_until_idle() -> usize {
    let mut count = 0;
    while thaw_runtime_poll_one() != 0 {
        count += 1;
    }
    count
}

/// Drives detached Promise combinator children to completion. A rejected
/// parent may already have resumed user code, but its child async frames still
/// borrow the current request arena and must finish before that arena resets.
#[no_mangle]
pub extern "C" fn thaw_runtime_drain_detached() -> usize {
    let mut count = 0;
    while ACTIVE_PROMISE_JOINS.with(Cell::get) != 0 {
        if thaw_runtime_poll_one() != 0 {
            count += 1;
            continue;
        }
        let delay = next_timer_delay();
        if has_fd_waits() {
            poll_fd_waits(delay);
        } else if let Some(delay) = delay {
            if !delay.is_zero() {
                std::thread::sleep(delay);
            }
        } else {
            break;
        }
    }
    count
}

/// Creates a promise resolved by the runtime timer queue after at least
/// `milliseconds`. No worker thread is created.
#[no_mangle]
pub extern "C" fn thaw_sleep_ms(milliseconds: u64) -> *mut ThawPromise {
    let promise = thaw_promise_new();
    TIMERS.with(|timers| {
        timers.borrow_mut().push(PromiseTimer {
            deadline: Instant::now() + Duration::from_millis(milliseconds),
            promise,
        });
    });
    promise
}

/// Drives ready continuations and timers until `promise` settles. Returns its
/// result/error pointer; use `thaw_promise_state` to distinguish fulfillment
/// from rejection. Returns null for an invalid handle or no possible progress.
///
/// # Safety
///
/// `promise` must be null or point to a live `ThawPromise` for the duration of
/// this call. No other thread may mutate or destroy it concurrently.
#[no_mangle]
pub unsafe extern "C" fn thaw_runtime_run_until_resolved(promise: *const ThawPromise) -> *const u8 {
    loop {
        let Some(promise_ref) = (unsafe { promise.as_ref() }) else {
            return std::ptr::null();
        };
        if let Some(result) = promise_ref.result {
            return result;
        }
        if thaw_runtime_poll_one() != 0 {
            continue;
        }
        if unsafe { thaw_promise_state(promise) } != 0 {
            continue;
        }
        let delay = next_timer_delay();
        if has_fd_waits() {
            poll_fd_waits(delay);
        } else if let Some(delay) = delay {
            if !delay.is_zero() {
                std::thread::sleep(delay);
            }
        } else {
            return std::ptr::null();
        }
    }
}

/// Opaque C-ABI promise shared by generated coroutines and runtime bridges.
/// Result ownership remains with the producer and must outlive all resume
/// callbacks. V2 coroutine frames will normally keep results in the request
/// arena, so the Lambda request boundary remains the lifetime boundary.
#[repr(C)]
pub struct ThawPromise {
    result: Option<*const u8>,
    rejected: bool,
    subscribers: Vec<PromiseSubscription>,
}

/// Allocates an unresolved promise. Pair every successful call with
/// `thaw_promise_destroy` after no coroutine can reference the handle.
#[no_mangle]
pub extern "C" fn thaw_promise_new() -> *mut ThawPromise {
    Box::into_raw(Box::new(ThawPromise {
        result: None,
        rejected: false,
        subscribers: Vec::new(),
    }))
}

/// Returns 0 for pending, 1 for fulfilled, 2 for rejected, and 255 for an
/// invalid handle.
///
/// # Safety
///
/// `promise` must be null or point to a live `ThawPromise` that is not being
/// mutated or destroyed concurrently.
#[no_mangle]
pub unsafe extern "C" fn thaw_promise_state(promise: *const ThawPromise) -> u8 {
    let Some(promise) = (unsafe { promise.as_ref() }) else {
        return u8::MAX;
    };
    match (promise.result.is_some(), promise.rejected) {
        (false, _) => 0,
        (true, false) => 1,
        (true, true) => 2,
    }
}

/// Registers a coroutine continuation. If already resolved, the callback is
/// queued immediately, but never invoked reentrantly inside this function.
/// Returns 1 on success and 0 for an invalid handle.
///
/// # Safety
///
/// `promise` must be null or point to a live, exclusively accessible
/// `ThawPromise`. `frame` must remain valid whenever `resume` can be invoked.
#[no_mangle]
pub unsafe extern "C" fn thaw_promise_subscribe(
    promise: *mut ThawPromise,
    resume: PromiseResumeFn,
    frame: *mut u8,
) -> u8 {
    let Some(promise) = (unsafe { promise.as_mut() }) else {
        return 0;
    };
    if let Some(result) = promise.result {
        enqueue_continuation(PromiseSubscription { resume, frame }, result);
    } else {
        promise
            .subscribers
            .push(PromiseSubscription { resume, frame });
    }
    1
}

/// Resolves a promise exactly once and queues every current subscriber in
/// registration order. Returns 0 for a null handle or repeated resolution.
#[no_mangle]
pub extern "C" fn thaw_promise_resolve(promise: *mut ThawPromise, result: *const u8) -> u8 {
    settle_promise(promise, result, false)
}

/// Rejects a promise exactly once and queues all subscribers. The error is an
/// opaque producer-owned pointer, using the same lifetime contract as a
/// fulfilled result. Returns 0 for a null handle or repeated settlement.
#[no_mangle]
pub extern "C" fn thaw_promise_reject(promise: *mut ThawPromise, error: *const u8) -> u8 {
    settle_promise(promise, error, true)
}

struct PromiseChainState {
    output: *mut ThawPromise,
    input: *mut ThawPromise,
    callback: PromiseTransformFn,
    context: *mut u8,
    on_rejected: bool,
}

extern "C" fn resume_promise_chain(frame: *mut u8, result: *const u8) {
    let state = unsafe { Box::from_raw(frame.cast::<PromiseChainState>()) };
    let rejected = unsafe { thaw_promise_state(state.input) } == 2;
    if rejected == state.on_rejected {
        (state.callback)(state.context, state.output, result);
    } else if rejected {
        thaw_promise_reject(state.output, result);
    } else {
        thaw_promise_resolve(state.output, result);
    }
    unsafe { thaw_promise_destroy(state.input) };
}

/// Creates the Promise returned by `.then` or `.catch`. The input handle is
/// consumed. The callback is invoked only for the selected settlement kind;
/// the other kind is forwarded without changing its payload.
///
/// # Safety
///
/// `input` must point to a live `ThawPromise`. `context` must remain valid
/// until `callback` runs.
#[no_mangle]
pub unsafe extern "C" fn thaw_promise_chain(
    input: *mut ThawPromise,
    callback: PromiseTransformFn,
    context: *mut u8,
    on_rejected: u8,
) -> *mut ThawPromise {
    let output = thaw_promise_new();
    if input.is_null() {
        thaw_promise_reject(output, PROMISE_ALL_INVALID_ERROR.as_ptr());
        return output;
    }
    let state = Box::into_raw(Box::new(PromiseChainState {
        output,
        input,
        callback,
        context,
        on_rejected: on_rejected != 0,
    }));
    thaw_promise_subscribe(input, resume_promise_chain, state.cast());
    output
}

struct PromiseAdoptState {
    output: *mut ThawPromise,
    input: *mut ThawPromise,
}

extern "C" fn resume_promise_adopt(frame: *mut u8, result: *const u8) {
    let state = unsafe { Box::from_raw(frame.cast::<PromiseAdoptState>()) };
    if unsafe { thaw_promise_state(state.input) } == 2 {
        thaw_promise_reject(state.output, result);
    } else {
        thaw_promise_resolve(state.output, result);
    }
    unsafe { thaw_promise_destroy(state.input) };
}

/// Makes `output` follow `input`, implementing Promise callback flattening.
/// The input handle is consumed.
///
/// # Safety
///
/// Both arguments must point to distinct live promises.
#[no_mangle]
pub unsafe extern "C" fn thaw_promise_adopt(
    output: *mut ThawPromise,
    input: *mut ThawPromise,
) -> u8 {
    if output.is_null() || input.is_null() {
        return 0;
    }
    if output == input {
        thaw_promise_reject(output, PROMISE_CYCLE_ERROR.as_ptr());
        return 0;
    }
    let state = Box::into_raw(Box::new(PromiseAdoptState { output, input }));
    thaw_promise_subscribe(input, resume_promise_adopt, state.cast());
    1
}

struct PromiseFinallyState {
    output: *mut ThawPromise,
    input: *mut ThawPromise,
    callback: PromiseFinallyFn,
    context: *mut u8,
}

extern "C" fn resume_promise_finally(frame: *mut u8, result: *const u8) {
    let state = unsafe { Box::from_raw(frame.cast::<PromiseFinallyState>()) };
    let rejected = unsafe { thaw_promise_state(state.input) } == 2;
    (state.callback)(state.context, state.output, result, u8::from(rejected));
    unsafe { thaw_promise_destroy(state.input) };
}

/// Runs a `.finally` callback for either settlement kind. The callback owns
/// forwarding or replacing the original settlement. The input is consumed.
///
/// # Safety
///
/// `input` must point to a live Promise and `context` must outlive callback
/// dispatch.
#[no_mangle]
pub unsafe extern "C" fn thaw_promise_finally(
    input: *mut ThawPromise,
    callback: PromiseFinallyFn,
    context: *mut u8,
) -> *mut ThawPromise {
    let output = thaw_promise_new();
    if input.is_null() {
        thaw_promise_reject(output, PROMISE_ALL_INVALID_ERROR.as_ptr());
        return output;
    }
    let state = Box::into_raw(Box::new(PromiseFinallyState {
        output,
        input,
        callback,
        context,
    }));
    thaw_promise_subscribe(input, resume_promise_finally, state.cast());
    output
}

struct PromiseFinallyAdoptState {
    output: *mut ThawPromise,
    input: *mut ThawPromise,
    original: *const u8,
    original_rejected: bool,
}

extern "C" fn resume_promise_finally_adopt(frame: *mut u8, result: *const u8) {
    let state = unsafe { Box::from_raw(frame.cast::<PromiseFinallyAdoptState>()) };
    if unsafe { thaw_promise_state(state.input) } == 2 {
        thaw_promise_reject(state.output, result);
    } else if state.original_rejected {
        thaw_promise_reject(state.output, state.original);
    } else {
        thaw_promise_resolve(state.output, state.original);
    }
    unsafe { thaw_promise_destroy(state.input) };
}

/// Waits for a Promise returned by `.finally`, then forwards the original
/// settlement unless the returned Promise rejects.
///
/// # Safety
///
/// `output` and `input` must be distinct live Promises. `original` must remain
/// valid until `input` settles.
#[no_mangle]
pub unsafe extern "C" fn thaw_promise_finally_adopt(
    output: *mut ThawPromise,
    input: *mut ThawPromise,
    original: *const u8,
    original_rejected: u8,
) -> u8 {
    if output.is_null() || input.is_null() || output == input {
        return 0;
    }
    let state = Box::into_raw(Box::new(PromiseFinallyAdoptState {
        output,
        input,
        original,
        original_rejected: original_rejected != 0,
    }));
    thaw_promise_subscribe(input, resume_promise_finally_adopt, state.cast());
    1
}

struct PromiseAllState {
    output: *mut ThawPromise,
    remaining: usize,
    rejected: bool,
    first_error: *const u8,
    result: *mut u64,
    result_slot: *mut *const u8,
    element_sizes: Vec<usize>,
}

struct PromiseAllChild {
    state: *mut PromiseAllState,
    promise: *mut ThawPromise,
    indices: Vec<usize>,
}

extern "C" fn resume_promise_all_child(frame: *mut u8, result: *const u8) {
    let child = unsafe { Box::from_raw(frame.cast::<PromiseAllChild>()) };
    let state = unsafe { &mut *child.state };
    let child_state = unsafe { thaw_promise_state(child.promise) };
    if child_state == 2 {
        if !state.rejected {
            state.rejected = true;
            state.first_error = result;
            thaw_promise_reject(state.output, result);
        }
    } else if !state.rejected {
        for index in &child.indices {
            let destination = unsafe { state.result.add(index + 1).cast::<u8>() };
            unsafe {
                destination.write_bytes(0, size_of::<u64>());
                std::ptr::copy_nonoverlapping(result, destination, state.element_sizes[*index]);
            }
        }
    }
    unsafe { thaw_promise_destroy(child.promise) };
    state.remaining -= 1;
    if state.remaining == 0 {
        if !state.rejected {
            thaw_promise_resolve(state.output, state.result_slot.cast());
        }
        ACTIVE_PROMISE_JOINS.with(|active| active.set(active.get() - 1));
        unsafe { drop(Box::from_raw(child.state)) };
    }
}

/// Joins homogeneous Promise handles without serializing them.
/// The result uses Thaw's `[i64 len][8-byte slots...]` array layout in the
/// request arena and therefore remains valid after the returned promise is
/// destroyed. Child handles are consumed by this call.
///
/// # Safety
///
/// `promises` must reference `len` live handles returned by Thaw async
/// functions. Each handle must be unique and must not be used after this call.
#[no_mangle]
pub unsafe extern "C" fn thaw_promise_all_slots(
    promises: *const *mut ThawPromise,
    len: usize,
    element_size: usize,
) -> *mut ThawPromise {
    let element_sizes = vec![element_size; len];
    unsafe { thaw_promise_all_typed(promises, element_sizes.as_ptr(), len) }
}

/// Joins Promise handles using one result-copy size per input position.
/// Sizes must be between one byte and one eight-byte array slot.
///
/// # Safety
///
/// `promises` and `element_sizes` must each reference `len` readable entries.
/// Promise handles are consumed and must not be used after this call.
#[no_mangle]
pub unsafe extern "C" fn thaw_promise_all_typed(
    promises: *const *mut ThawPromise,
    element_sizes: *const usize,
    len: usize,
) -> *mut ThawPromise {
    let output = thaw_promise_new();
    if len != 0 && element_sizes.is_null() {
        thaw_promise_reject(output, PROMISE_ALL_INVALID_ERROR.as_ptr());
        return output;
    }
    let element_sizes = if len == 0 {
        Vec::new()
    } else {
        unsafe { std::slice::from_raw_parts(element_sizes, len) }.to_vec()
    };
    if element_sizes
        .iter()
        .any(|size| *size == 0 || *size > size_of::<u64>())
    {
        thaw_promise_reject(output, PROMISE_ALL_INVALID_ERROR.as_ptr());
        return output;
    }
    let allocation =
        thaw_arena::thaw_arena_alloc((len + 2) * size_of::<u64>(), align_of::<u64>()).cast::<u64>();
    if allocation.is_null() {
        thaw_promise_reject(output, PROMISE_ALL_INVALID_ERROR.as_ptr());
        return output;
    }
    let result_slot = allocation.cast::<*const u8>();
    let result = unsafe { allocation.add(1) };
    unsafe { result_slot.write(result.cast()) };
    unsafe { result.write(len as u64) };
    if len == 0 {
        thaw_promise_resolve(output, result_slot.cast());
        return output;
    }
    if promises.is_null() {
        thaw_promise_reject(output, PROMISE_ALL_INVALID_ERROR.as_ptr());
        return output;
    }
    let mut grouped = Vec::<(*mut ThawPromise, Vec<usize>)>::new();
    let mut invalid = false;
    for index in 0..len {
        let promise = unsafe { *promises.add(index) };
        if promise.is_null() {
            invalid = true;
        } else {
            if let Some((_, indices)) = grouped
                .iter_mut()
                .find(|(existing, _)| *existing == promise)
            {
                indices.push(index);
            } else {
                grouped.push((promise, vec![index]));
            }
        }
    }
    let state = Box::into_raw(Box::new(PromiseAllState {
        output,
        remaining: grouped.len(),
        rejected: invalid,
        first_error: if invalid {
            PROMISE_ALL_INVALID_ERROR.as_ptr()
        } else {
            std::ptr::null()
        },
        result,
        result_slot,
        element_sizes,
    }));
    if !grouped.is_empty() {
        ACTIVE_PROMISE_JOINS.with(|active| active.set(active.get() + 1));
    }
    if invalid {
        thaw_promise_reject(output, PROMISE_ALL_INVALID_ERROR.as_ptr());
    }
    for (promise, indices) in grouped {
        let child = Box::into_raw(Box::new(PromiseAllChild {
            state,
            promise,
            indices,
        }));
        unsafe {
            thaw_promise_subscribe(promise, resume_promise_all_child, child.cast());
        }
    }
    if unsafe { (*state).remaining } == 0 {
        let error = unsafe { (*state).first_error };
        thaw_promise_reject(output, error);
        unsafe { drop(Box::from_raw(state)) };
    }
    output
}

/// Backward-compatible number-only entry point.
///
/// # Safety
///
/// The same contract as [`thaw_promise_all_slots`] applies.
#[no_mangle]
pub unsafe extern "C" fn thaw_promise_all_f64(
    promises: *const *mut ThawPromise,
    len: usize,
) -> *mut ThawPromise {
    unsafe { thaw_promise_all_slots(promises, len, size_of::<f64>()) }
}

struct PromiseRaceState {
    output: *mut ThawPromise,
    remaining: usize,
    settled: bool,
}

struct PromiseRaceChild {
    state: *mut PromiseRaceState,
    promise: *mut ThawPromise,
}

extern "C" fn resume_promise_race_child(frame: *mut u8, result: *const u8) {
    let child = unsafe { Box::from_raw(frame.cast::<PromiseRaceChild>()) };
    let state = unsafe { &mut *child.state };
    if !state.settled {
        state.settled = true;
        if unsafe { thaw_promise_state(child.promise) } == 2 {
            thaw_promise_reject(state.output, result);
        } else {
            thaw_promise_resolve(state.output, result);
        }
    }
    unsafe { thaw_promise_destroy(child.promise) };
    state.remaining -= 1;
    if state.remaining == 0 {
        ACTIVE_PROMISE_JOINS.with(|active| active.set(active.get() - 1));
        unsafe { drop(Box::from_raw(child.state)) };
    }
}

/// Settles with the first input Promise to fulfill or reject. Input handles
/// are deduplicated and consumed; slower children continue to be drained.
///
/// # Safety
///
/// `promises` must reference `len` readable Promise handles. Each distinct
/// handle must be live and must not be used after this call.
#[no_mangle]
pub unsafe extern "C" fn thaw_promise_race(
    promises: *const *mut ThawPromise,
    len: usize,
) -> *mut ThawPromise {
    let output = thaw_promise_new();
    if len == 0 {
        thaw_promise_reject(output, PROMISE_RACE_EMPTY_ERROR.as_ptr());
        return output;
    }
    if promises.is_null() {
        thaw_promise_reject(output, PROMISE_RACE_INVALID_ERROR.as_ptr());
        return output;
    }
    let mut unique = Vec::<*mut ThawPromise>::new();
    let mut invalid = false;
    for index in 0..len {
        let promise = unsafe { *promises.add(index) };
        if promise.is_null() {
            invalid = true;
        } else if !unique.contains(&promise) {
            unique.push(promise);
        }
    }
    let state = Box::into_raw(Box::new(PromiseRaceState {
        output,
        remaining: unique.len(),
        settled: invalid,
    }));
    if !unique.is_empty() {
        ACTIVE_PROMISE_JOINS.with(|active| active.set(active.get() + 1));
    }
    if invalid {
        thaw_promise_reject(output, PROMISE_RACE_INVALID_ERROR.as_ptr());
    }
    for promise in unique {
        let child = Box::into_raw(Box::new(PromiseRaceChild { state, promise }));
        unsafe { thaw_promise_subscribe(promise, resume_promise_race_child, child.cast()) };
    }
    if unsafe { (*state).remaining } == 0 {
        unsafe { drop(Box::from_raw(state)) };
    }
    output
}

struct PromiseAnyState {
    output: *mut ThawPromise,
    remaining: usize,
    fulfilled: bool,
}

struct PromiseAnyChild {
    state: *mut PromiseAnyState,
    promise: *mut ThawPromise,
}

extern "C" fn resume_promise_any_child(frame: *mut u8, result: *const u8) {
    let child = unsafe { Box::from_raw(frame.cast::<PromiseAnyChild>()) };
    let state = unsafe { &mut *child.state };
    if !state.fulfilled && unsafe { thaw_promise_state(child.promise) } == 1 {
        state.fulfilled = true;
        thaw_promise_resolve(state.output, result);
    }
    unsafe { thaw_promise_destroy(child.promise) };
    state.remaining -= 1;
    if state.remaining == 0 {
        if !state.fulfilled {
            thaw_promise_reject(state.output, PROMISE_ANY_REJECTED_ERROR.as_ptr());
        }
        ACTIVE_PROMISE_JOINS.with(|active| active.set(active.get() - 1));
        unsafe { drop(Box::from_raw(child.state)) };
    }
}

/// Resolves with the first fulfilled input Promise. Rejections are ignored
/// until every distinct input has rejected, at which point an aggregate error
/// message is used to reject the output. Slower children are still drained.
///
/// # Safety
///
/// `promises` must reference `len` readable Promise handles. Each distinct
/// non-null handle is consumed and must not be used after this call.
#[no_mangle]
pub unsafe extern "C" fn thaw_promise_any(
    promises: *const *mut ThawPromise,
    len: usize,
) -> *mut ThawPromise {
    let output = thaw_promise_new();
    if len == 0 || promises.is_null() {
        thaw_promise_reject(output, PROMISE_ANY_REJECTED_ERROR.as_ptr());
        return output;
    }
    let mut unique = Vec::<*mut ThawPromise>::new();
    for index in 0..len {
        let promise = unsafe { *promises.add(index) };
        if !promise.is_null() && !unique.contains(&promise) {
            unique.push(promise);
        }
    }
    if unique.is_empty() {
        thaw_promise_reject(output, PROMISE_ANY_REJECTED_ERROR.as_ptr());
        return output;
    }
    let state = Box::into_raw(Box::new(PromiseAnyState {
        output,
        remaining: unique.len(),
        fulfilled: false,
    }));
    ACTIVE_PROMISE_JOINS.with(|active| active.set(active.get() + 1));
    for promise in unique {
        let child = Box::into_raw(Box::new(PromiseAnyChild { state, promise }));
        unsafe { thaw_promise_subscribe(promise, resume_promise_any_child, child.cast()) };
    }
    output
}

struct PromiseAllSettledState {
    output: *mut ThawPromise,
    remaining: usize,
    result_slot: *mut *const u8,
    result: *mut u64,
    objects: *mut u64,
    element_size: usize,
}

struct PromiseAllSettledChild {
    state: *mut PromiseAllSettledState,
    promise: *mut ThawPromise,
    indices: Vec<usize>,
}

extern "C" fn resume_promise_all_settled_child(frame: *mut u8, value: *const u8) {
    let child = unsafe { Box::from_raw(frame.cast::<PromiseAllSettledChild>()) };
    let state = unsafe { &mut *child.state };
    let rejected = unsafe { thaw_promise_state(child.promise) } == 2;
    for index in &child.indices {
        let object = unsafe { state.objects.add(index * 3) };
        unsafe {
            object.write(if rejected {
                PROMISE_SETTLED_REJECTED.as_ptr() as u64
            } else {
                PROMISE_SETTLED_FULFILLED.as_ptr() as u64
            });
            object.add(1).write(0);
            if !rejected {
                std::ptr::copy_nonoverlapping(
                    value,
                    object.add(1).cast::<u8>(),
                    state.element_size,
                );
            }
            object.add(2).write(if rejected {
                value as u64
            } else {
                PROMISE_SETTLED_EMPTY_REASON.as_ptr() as u64
            });
            state.result.add(index + 1).write(object as u64);
        }
    }
    unsafe { thaw_promise_destroy(child.promise) };
    state.remaining -= 1;
    if state.remaining == 0 {
        thaw_promise_resolve(state.output, state.result_slot.cast());
        ACTIVE_PROMISE_JOINS.with(|active| active.set(active.get() - 1));
        unsafe { drop(Box::from_raw(child.state)) };
    }
}

/// Waits for every distinct input and resolves with an input-ordered array of
/// `{ status, value, reason }` object pointers. Rejections become result
/// entries and never reject the output Promise.
///
/// # Safety
///
/// `promises` must reference `len` readable Promise handles. Each distinct
/// handle is consumed. `element_size` must fit one eight-byte value slot.
#[no_mangle]
pub unsafe extern "C" fn thaw_promise_all_settled(
    promises: *const *mut ThawPromise,
    len: usize,
    element_size: usize,
) -> *mut ThawPromise {
    let output = thaw_promise_new();
    if element_size == 0 || element_size > size_of::<u64>() || (len != 0 && promises.is_null()) {
        thaw_promise_reject(output, PROMISE_ALL_INVALID_ERROR.as_ptr());
        return output;
    }
    let allocation =
        thaw_arena::thaw_arena_alloc((len + 2) * size_of::<u64>(), align_of::<u64>()).cast::<u64>();
    let objects = thaw_arena::thaw_arena_alloc(
        len.saturating_mul(3).saturating_mul(size_of::<u64>()),
        align_of::<u64>(),
    )
    .cast::<u64>();
    if allocation.is_null() || (len != 0 && objects.is_null()) {
        thaw_promise_reject(output, PROMISE_ALL_INVALID_ERROR.as_ptr());
        return output;
    }
    let result_slot = allocation.cast::<*const u8>();
    let result = unsafe { allocation.add(1) };
    unsafe {
        result_slot.write(result.cast());
        result.write(len as u64);
    }
    if len == 0 {
        thaw_promise_resolve(output, result_slot.cast());
        return output;
    }
    let mut grouped = Vec::<(*mut ThawPromise, Vec<usize>)>::new();
    for index in 0..len {
        let promise = unsafe { *promises.add(index) };
        if promise.is_null() {
            thaw_promise_reject(output, PROMISE_ALL_INVALID_ERROR.as_ptr());
            return output;
        }
        if let Some((_, indices)) = grouped
            .iter_mut()
            .find(|(existing, _)| *existing == promise)
        {
            indices.push(index);
        } else {
            grouped.push((promise, vec![index]));
        }
    }
    let state = Box::into_raw(Box::new(PromiseAllSettledState {
        output,
        remaining: grouped.len(),
        result_slot,
        result,
        objects,
        element_size,
    }));
    ACTIVE_PROMISE_JOINS.with(|active| active.set(active.get() + 1));
    for (promise, indices) in grouped {
        let child = Box::into_raw(Box::new(PromiseAllSettledChild {
            state,
            promise,
            indices,
        }));
        unsafe { thaw_promise_subscribe(promise, resume_promise_all_settled_child, child.cast()) };
    }
    output
}

fn settle_promise(promise: *mut ThawPromise, result: *const u8, rejected: bool) -> u8 {
    let Some(promise) = (unsafe { promise.as_mut() }) else {
        return 0;
    };
    if promise.result.is_some() {
        return 0;
    }
    promise.result = Some(result);
    promise.rejected = rejected;
    let subscribers = std::mem::take(&mut promise.subscribers);
    for subscriber in subscribers {
        enqueue_continuation(subscriber, result);
    }
    1
}

/// Destroys a promise handle. Passing null is a no-op. The caller must not
/// destroy a promise from inside one of its resume callbacks.
///
/// # Safety
///
/// `promise` must be null or a pointer returned by `thaw_promise_new` that has
/// not already been destroyed and is no longer referenced by any subscriber.
#[no_mangle]
pub unsafe extern "C" fn thaw_promise_destroy(promise: *mut ThawPromise) {
    if !promise.is_null() {
        TIMERS.with(|timers| timers.borrow_mut().retain(|timer| timer.promise != promise));
        FD_WAITS.with(|waits| waits.borrow_mut().retain(|wait| wait.promise != promise));
        drop(Box::from_raw(promise));
    }
}

/// Reclaims request-owned generated values on every exit path. The handler
/// result is copied into a Rust `String` before this guard is dropped, so the
/// response never borrows reclaimed arena memory.
struct InvocationArenaReset;

impl Drop for InvocationArenaReset {
    fn drop(&mut self) {
        thaw_runtime_drain_detached();
        thaw_arena::thaw_arena_reset();
    }
}

/// Runs the event loop forever. Compiled by `thaw-llvm::hir_codegen` as the
/// process entry point whenever a program defines `handler` instead of
/// `main` (see that module for the `int main(void)` wrapper that calls
/// this).
#[no_mangle]
pub extern "C" fn thaw_runtime_run(handler: HandlerFn, error_slot: HandlerErrorSlot) -> ! {
    let runtime_api = std::env::var("AWS_LAMBDA_RUNTIME_API").expect(
        "AWS_LAMBDA_RUNTIME_API is not set -- is this running inside a Lambda execution environment?",
    );

    loop {
        if let Err(err) = handle_one_invocation(&runtime_api, handler, error_slot) {
            eprintln!("thaw-runtime: {err}");
        }
    }
}

fn handle_one_invocation(
    runtime_api: &str,
    handler: HandlerFn,
    error_slot: HandlerErrorSlot,
) -> Result<(), String> {
    let _arena_reset = InvocationArenaReset;
    let next = http_request(
        runtime_api,
        "GET",
        "/2018-06-01/runtime/invocation/next",
        None,
    )?;

    let request_id = next
        .header("lambda-runtime-aws-request-id")
        .ok_or("response from .../invocation/next is missing the request id header")?
        .to_string();

    let event_cstring = CString::new(next.body).map_err(|e| e.to_string())?;
    if !error_slot.is_null() {
        unsafe { *error_slot = std::ptr::null() };
    }
    let result_ptr = handler(event_cstring.as_ptr());
    if result_ptr.is_null() {
        let message = if error_slot.is_null() || unsafe { (*error_slot).is_null() } {
            "handler failed with an uncaught Thaw exception".to_string()
        } else {
            unsafe { CStr::from_ptr(*error_slot) }
                .to_string_lossy()
                .into_owned()
        };
        let error_path = format!("/2018-06-01/runtime/invocation/{request_id}/error");
        let body = format!(
            "{{\"errorMessage\":{},\"errorType\":\"ThawError\"}}",
            json_string(&message)
        );
        http_request(runtime_api, "POST", &error_path, Some(&body))?;
        return Ok(());
    }
    let result = unsafe { CStr::from_ptr(result_ptr) }
        .to_string_lossy()
        .into_owned();

    let response_path = format!("/2018-06-01/runtime/invocation/{request_id}/response");
    http_request(runtime_api, "POST", &response_path, Some(&result))?;
    Ok(())
}

fn json_string(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    for ch in value.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            ch if ch.is_control() => out.push_str(&format!("\\u{:04x}", ch as u32)),
            ch => out.push(ch),
        }
    }
    out.push('"');
    out
}

struct HttpResponse {
    status: u32,
    headers: Vec<(String, String)>,
    body: String,
}

impl HttpResponse {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }
}

fn http_request(
    host: &str,
    method: &str,
    path: &str,
    body: Option<&str>,
) -> Result<HttpResponse, String> {
    let mut stream = TcpStream::connect(host).map_err(|e| format!("connecting to {host}: {e}"))?;

    let mut request = format!("{method} {path} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n");
    if let Some(body) = body {
        request.push_str(&format!(
            "Content-Type: application/json\r\nContent-Length: {}\r\n",
            body.len()
        ));
    }
    request.push_str("\r\n");
    if let Some(body) = body {
        request.push_str(body);
    }

    stream
        .write_all(request.as_bytes())
        .map_err(|e| format!("writing request: {e}"))?;
    // Half-close the write side so the peer's `read_to_end` (used for
    // request bodies on our mock server, and in principle by any HTTP/1.1
    // peer reading an unbounded body) sees EOF instead of blocking forever
    // waiting for more of *our* request -- otherwise both sides can end up
    // blocked in `read_to_end` at once.
    stream
        .shutdown(std::net::Shutdown::Write)
        .map_err(|e| format!("shutting down write half: {e}"))?;

    let mut raw = Vec::new();
    stream
        .read_to_end(&mut raw)
        .map_err(|e| format!("reading response: {e}"))?;
    let raw = String::from_utf8_lossy(&raw);

    let (head, body) = raw
        .split_once("\r\n\r\n")
        .ok_or("malformed HTTP response (no header/body separator)")?;

    let mut lines = head.lines();
    let status_line = lines.next().ok_or("empty HTTP response")?;
    let status = status_line
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse::<u32>().ok())
        .ok_or_else(|| format!("malformed status line: {status_line}"))?;

    let headers = lines
        .filter_map(|line| line.split_once(':'))
        .map(|(k, v)| (k.trim().to_string(), v.trim().to_string()))
        .collect();

    let response = HttpResponse {
        status,
        headers,
        body: body.to_string(),
    };

    if response.status >= 400 {
        return Err(format!(
            "{method} {path} -> HTTP {}: {}",
            response.status, response.body
        ));
    }

    Ok(response)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;
    use std::process::Command;

    use rustls::pki_types::{CertificateDer, PrivatePkcs8KeyDer};
    use rustls::{ServerConfig, ServerConnection, StreamOwned};
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::mpsc;

    static TLS_TEST_DIR_SEQUENCE: AtomicU64 = AtomicU64::new(0);

    #[test]
    fn formats_numbers_with_javascript_string_boundaries() {
        let cases = [
            (0.0, "0"),
            (-0.0, "0"),
            (f64::NAN, "NaN"),
            (f64::INFINITY, "Infinity"),
            (f64::NEG_INFINITY, "-Infinity"),
            (1.5, "1.5"),
            (1e20, "100000000000000000000"),
            (1e21, "1e+21"),
            (1e-6, "0.000001"),
            (1e-7, "1e-7"),
            (1.2345678901234567, "1.2345678901234567"),
        ];
        for (value, expected) in cases {
            assert_eq!(javascript_number_string(value), expected, "value={value:?}");
        }
    }

    #[test]
    fn parses_strings_with_javascript_number_grammar() {
        let cases = [
            ("", 0.0),
            ("  \n\t", 0.0),
            ("42", 42.0),
            ("+1.5", 1.5),
            (".25", 0.25),
            ("2.", 2.0),
            ("1e3", 1000.0),
            ("0xff", 255.0),
            ("\u{feff}1\u{feff}", 1.0),
            ("0x10000000000000000", 18_446_744_073_709_551_616.0),
            ("0x20000000000001", 9_007_199_254_740_992.0),
            ("0x20000000000003", 9_007_199_254_740_996.0),
            ("0o10", 8.0),
            ("0b101", 5.0),
            ("Infinity", f64::INFINITY),
            ("-Infinity", f64::NEG_INFINITY),
        ];
        for (text, expected) in cases {
            assert_eq!(javascript_string_number(text), expected, "text={text:?}");
        }
        assert!(javascript_string_number("nope").is_nan());
        assert!(javascript_string_number("1e").is_nan());
        assert!(javascript_string_number("+0x1").is_nan());
        assert!(javascript_string_number("inf").is_nan());
        assert!(javascript_string_number("-0").is_sign_negative());
    }

    #[test]
    fn parses_float_and_integer_prefixes_like_javascript() {
        assert_eq!(javascript_parse_float("  -12.5px"), -12.5);
        assert_eq!(javascript_parse_float("1e2rest"), 100.0);
        assert_eq!(javascript_parse_float("1e+"), 1.0);
        assert_eq!(javascript_parse_float("+Infinity!"), f64::INFINITY);
        assert!(javascript_parse_float("0x10").is_sign_positive());
        assert_eq!(javascript_parse_float("0x10"), 0.0);
        assert!(javascript_parse_float("words").is_nan());

        assert_eq!(javascript_parse_int("  -0x10more", 0.0), -16.0);
        assert_eq!(javascript_parse_int("11", 2.0), 3.0);
        assert_eq!(javascript_parse_int("0x20", 16.0), 32.0);
        assert_eq!(javascript_parse_int("010", 0.0), 10.0);
        assert_eq!(javascript_parse_int("15px", 10.0), 15.0);
        assert!(javascript_parse_int("10", 1.0).is_nan());
        assert!(javascript_parse_int("xyz", 36.0).is_finite());
        assert!(javascript_parse_int("-0", 10.0).is_sign_negative());
    }

    #[test]
    fn compares_strings_in_javascript_utf16_order() {
        let supplementary = CString::new("\u{10000}").unwrap();
        let bmp = CString::new("\u{e000}").unwrap();
        assert_eq!(
            unsafe { thaw_string_compare(supplementary.as_ptr(), bmp.as_ptr()) },
            -1
        );
        assert_eq!(
            unsafe { thaw_string_compare(bmp.as_ptr(), supplementary.as_ptr()) },
            1
        );
        assert_eq!(
            unsafe { thaw_string_compare(bmp.as_ptr(), bmp.as_ptr()) },
            0
        );
    }

    fn thaw_runtime_run_until_resolved(promise: *const ThawPromise) -> *const u8 {
        unsafe { super::thaw_runtime_run_until_resolved(promise) }
    }

    fn thaw_promise_state(promise: *const ThawPromise) -> u8 {
        unsafe { super::thaw_promise_state(promise) }
    }

    fn thaw_promise_subscribe(
        promise: *mut ThawPromise,
        resume: PromiseResumeFn,
        frame: *mut u8,
    ) -> u8 {
        unsafe { super::thaw_promise_subscribe(promise, resume, frame) }
    }

    extern "C" fn echo_handler(event: *const c_char) -> *const c_char {
        let event = unsafe { CStr::from_ptr(event) }
            .to_string_lossy()
            .into_owned();
        // Leaked on purpose: matches the arena/global-lifetime string model
        // compiled Thaw code uses (nothing frees heap strings yet).
        CString::new(format!("echo:{event}")).unwrap().into_raw() as *const c_char
    }

    static TEST_HANDLER_ERROR: &[u8] = b"handler exploded\0";
    static mut TEST_PENDING_EXCEPTION: *const c_char = std::ptr::null();

    extern "C" fn failing_handler(_: *const c_char) -> *const c_char {
        unsafe { TEST_PENDING_EXCEPTION = TEST_HANDLER_ERROR.as_ptr().cast() };
        std::ptr::null()
    }

    #[repr(C)]
    struct ResumeRecord {
        calls: usize,
        result: *const u8,
    }

    extern "C" fn record_resume(frame: *mut u8, result: *const u8) {
        let record = unsafe { &mut *(frame as *mut ResumeRecord) };
        record.calls += 1;
        record.result = result;
    }

    struct ChainNumberContext {
        calls: usize,
        add: f64,
    }

    extern "C" fn transform_chain_number(
        context: *mut u8,
        output: *mut ThawPromise,
        result: *const u8,
    ) {
        let context = unsafe { &mut *context.cast::<ChainNumberContext>() };
        context.calls += 1;
        let value = unsafe { *result.cast::<f64>() } + context.add;
        let slot = thaw_arena::thaw_arena_alloc(size_of::<f64>(), align_of::<f64>()).cast::<f64>();
        unsafe { slot.write(value) };
        thaw_promise_resolve(output, slot.cast());
    }

    struct TimedValueContext {
        timer: *mut ThawPromise,
        output: *mut ThawPromise,
        value: *const u8,
    }

    extern "C" fn resolve_timed_value(frame: *mut u8, _: *const u8) {
        let context = unsafe { Box::from_raw(frame.cast::<TimedValueContext>()) };
        unsafe { thaw_promise_destroy(context.timer) };
        thaw_promise_resolve(context.output, context.value);
    }

    fn timed_value(milliseconds: u64, value: f64) -> *mut ThawPromise {
        let timer = thaw_sleep_ms(milliseconds);
        let output = thaw_promise_new();
        let slot = thaw_arena::thaw_arena_alloc(size_of::<f64>(), align_of::<f64>()).cast::<f64>();
        unsafe { slot.write(value) };
        let context = Box::into_raw(Box::new(TimedValueContext {
            timer,
            output,
            value: slot.cast(),
        }));
        assert_eq!(
            thaw_promise_subscribe(timer, resolve_timed_value, context.cast()),
            1
        );
        output
    }

    fn local_tls_configs() -> (Arc<ClientConfig>, Arc<ServerConfig>) {
        let sequence = TLS_TEST_DIR_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("thaw-tls-test-{}-{sequence}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let key_pem = dir.join("key.pem");
        let cert_pem = dir.join("cert.pem");
        let key_der = dir.join("key.der");
        let cert_der = dir.join("cert.der");
        assert!(Command::new("openssl")
            .args([
                "req",
                "-x509",
                "-newkey",
                "rsa:2048",
                "-nodes",
                "-days",
                "1",
                "-subj",
                "/CN=localhost",
                "-addext",
                "subjectAltName=DNS:localhost,IP:127.0.0.1",
                "-addext",
                "basicConstraints=critical,CA:FALSE",
                "-addext",
                "keyUsage=critical,digitalSignature,keyEncipherment",
                "-addext",
                "extendedKeyUsage=serverAuth",
                "-keyout",
            ])
            .arg(&key_pem)
            .arg("-out")
            .arg(&cert_pem)
            .output()
            .unwrap()
            .status
            .success());
        assert!(Command::new("openssl")
            .args(["x509", "-in"])
            .arg(&cert_pem)
            .args(["-outform", "DER", "-out"])
            .arg(&cert_der)
            .status()
            .unwrap()
            .success());
        assert!(Command::new("openssl")
            .args(["pkcs8", "-topk8", "-nocrypt", "-in"])
            .arg(&key_pem)
            .args(["-outform", "DER", "-out"])
            .arg(&key_der)
            .status()
            .unwrap()
            .success());

        let certificate = CertificateDer::from(std::fs::read(&cert_der).unwrap());
        let private_key = PrivatePkcs8KeyDer::from(std::fs::read(&key_der).unwrap()).into();
        let mut roots = RootCertStore::empty();
        roots.add(certificate.clone()).unwrap();
        let client = Arc::new(
            ClientConfig::builder()
                .with_root_certificates(roots)
                .with_no_client_auth(),
        );
        let server = Arc::new(
            ServerConfig::builder()
                .with_no_client_auth()
                .with_single_cert(vec![certificate], private_key)
                .unwrap(),
        );
        let _ = std::fs::remove_dir_all(dir);
        (client, server)
    }

    #[test]
    fn promise_chain_selects_callbacks_for_then_and_catch() {
        let input = thaw_promise_new();
        let mut then_context = ChainNumberContext { calls: 0, add: 2.0 };
        let chained = unsafe {
            thaw_promise_chain(
                input,
                transform_chain_number,
                (&mut then_context as *mut ChainNumberContext).cast(),
                0,
            )
        };
        let value = 40.0f64;
        thaw_promise_resolve(input, (&value as *const f64).cast());
        let result = thaw_runtime_run_until_resolved(chained);
        assert_eq!(unsafe { *result.cast::<f64>() }, 42.0);
        assert_eq!(then_context.calls, 1);
        unsafe { thaw_promise_destroy(chained) };

        let input = thaw_promise_new();
        let mut catch_context = ChainNumberContext { calls: 0, add: 1.0 };
        let chained = unsafe {
            thaw_promise_chain(
                input,
                transform_chain_number,
                (&mut catch_context as *mut ChainNumberContext).cast(),
                1,
            )
        };
        let value = 7.0f64;
        thaw_promise_resolve(input, (&value as *const f64).cast());
        let result = thaw_runtime_run_until_resolved(chained);
        assert_eq!(unsafe { *result.cast::<f64>() }, 7.0);
        assert_eq!(catch_context.calls, 0);
        unsafe { thaw_promise_destroy(chained) };

        let cycle = thaw_promise_new();
        assert_eq!(unsafe { thaw_promise_adopt(cycle, cycle) }, 0);
        let error = thaw_runtime_run_until_resolved(cycle);
        assert_eq!(thaw_promise_state(cycle), 2);
        assert_eq!(
            unsafe { CStr::from_ptr(error.cast()) }.to_string_lossy(),
            "Chaining cycle detected for promise"
        );
        unsafe { thaw_promise_destroy(cycle) };
    }

    #[test]
    fn promise_resumes_subscribers_in_registration_order() {
        let promise = thaw_promise_new();
        assert_eq!(thaw_promise_state(promise), 0);

        let mut first = ResumeRecord {
            calls: 0,
            result: std::ptr::null(),
        };
        let mut second = ResumeRecord {
            calls: 0,
            result: std::ptr::null(),
        };
        assert_eq!(
            thaw_promise_subscribe(
                promise,
                record_resume,
                (&mut first as *mut ResumeRecord).cast()
            ),
            1
        );
        assert_eq!(
            thaw_promise_subscribe(
                promise,
                record_resume,
                (&mut second as *mut ResumeRecord).cast()
            ),
            1
        );

        let result = 42u8;
        assert_eq!(thaw_promise_resolve(promise, &result), 1);
        assert_eq!(thaw_promise_state(promise), 1);
        assert_eq!(first.calls, 0);
        assert_eq!(second.calls, 0);
        assert_eq!(thaw_runtime_run_until_idle(), 2);
        assert_eq!(first.calls, 1);
        assert_eq!(second.calls, 1);
        assert_eq!(first.result, &result);
        assert_eq!(second.result, &result);
        assert_eq!(thaw_promise_resolve(promise, &result), 0);

        unsafe { thaw_promise_destroy(promise) };
    }

    #[test]
    fn subscribing_after_resolution_queues_resume_immediately() {
        let promise = thaw_promise_new();
        let result = 7u8;
        assert_eq!(thaw_promise_resolve(promise, &result), 1);

        let mut record = ResumeRecord {
            calls: 0,
            result: std::ptr::null(),
        };
        assert_eq!(
            thaw_promise_subscribe(
                promise,
                record_resume,
                (&mut record as *mut ResumeRecord).cast()
            ),
            1
        );
        assert_eq!(record.calls, 0);
        assert_eq!(thaw_runtime_poll_one(), 1);
        assert_eq!(record.calls, 1);
        assert_eq!(record.result, &result);
        assert_eq!(thaw_runtime_poll_one(), 0);

        unsafe { thaw_promise_destroy(promise) };
    }

    #[test]
    fn promise_abi_rejects_invalid_handles() {
        assert_eq!(thaw_promise_state(std::ptr::null()), u8::MAX);
        assert_eq!(
            thaw_promise_subscribe(std::ptr::null_mut(), record_resume, std::ptr::null_mut()),
            0
        );
        assert_eq!(
            thaw_promise_resolve(std::ptr::null_mut(), std::ptr::null()),
            0
        );
        assert_eq!(
            thaw_promise_reject(std::ptr::null_mut(), std::ptr::null()),
            0
        );
        unsafe { thaw_promise_destroy(std::ptr::null_mut()) };
    }

    #[test]
    fn rejected_promise_queues_subscribers_and_settles_once() {
        let promise = thaw_promise_new();
        let mut record = ResumeRecord {
            calls: 0,
            result: std::ptr::null(),
        };
        assert_eq!(
            thaw_promise_subscribe(
                promise,
                record_resume,
                (&mut record as *mut ResumeRecord).cast()
            ),
            1
        );
        let error = 99u8;
        assert_eq!(thaw_promise_reject(promise, &error), 1);
        assert_eq!(thaw_promise_state(promise), 2);
        assert_eq!(thaw_promise_resolve(promise, &error), 0);
        assert_eq!(record.calls, 0);
        assert_eq!(thaw_runtime_run_until_idle(), 1);
        assert_eq!(record.calls, 1);
        assert_eq!(record.result, &error);
        assert_eq!(thaw_runtime_run_until_resolved(promise), &error);
        unsafe { thaw_promise_destroy(promise) };
    }

    #[test]
    fn promise_all_preserves_input_order_and_resolves_empty_inputs() {
        let first = thaw_promise_new();
        let second = thaw_promise_new();
        let children = [first, second];
        let joined = unsafe { thaw_promise_all_f64(children.as_ptr(), children.len()) };
        let first_value = 3.0f64;
        let second_value = 7.0f64;
        assert_eq!(
            thaw_promise_resolve(second, (&second_value as *const f64).cast()),
            1
        );
        assert_eq!(thaw_promise_state(joined), 0);
        assert_eq!(
            thaw_promise_resolve(first, (&first_value as *const f64).cast()),
            1
        );
        thaw_runtime_run_until_idle();
        assert_eq!(thaw_promise_state(joined), 1);
        let result = unsafe { *thaw_runtime_run_until_resolved(joined).cast::<*const u64>() };
        assert_eq!(unsafe { result.read() }, 2);
        assert_eq!(f64::from_bits(unsafe { result.add(1).read() }), 3.0);
        assert_eq!(f64::from_bits(unsafe { result.add(2).read() }), 7.0);
        unsafe { thaw_promise_destroy(joined) };

        let empty = unsafe { thaw_promise_all_f64(std::ptr::null(), 0) };
        assert_eq!(thaw_promise_state(empty), 1);
        let result = unsafe { *thaw_runtime_run_until_resolved(empty).cast::<*const u64>() };
        assert_eq!(unsafe { result.read() }, 0);
        unsafe { thaw_promise_destroy(empty) };
    }

    #[test]
    fn promise_all_typed_copies_position_sizes_and_deduplicates_handles() {
        let flag = thaw_promise_new();
        let pointer = thaw_promise_new();
        let children = [flag, pointer, flag];
        let sizes = [1usize, 8, 1];
        let joined =
            unsafe { thaw_promise_all_typed(children.as_ptr(), sizes.as_ptr(), children.len()) };
        let pointer_value = 0x1234_5678_9abc_def0u64;
        let flag_value = 1u8;
        assert_eq!(
            thaw_promise_resolve(pointer, (&pointer_value as *const u64).cast()),
            1
        );
        assert_eq!(thaw_promise_resolve(flag, &flag_value), 1);
        thaw_runtime_run_until_idle();
        let result = unsafe { *thaw_runtime_run_until_resolved(joined).cast::<*const u64>() };
        assert_eq!(unsafe { result.read() }, 3);
        assert_eq!(unsafe { result.add(1).read() }, 1);
        assert_eq!(unsafe { result.add(2).read() }, pointer_value);
        assert_eq!(unsafe { result.add(3).read() }, 1);
        unsafe { thaw_promise_destroy(joined) };
    }

    #[test]
    fn promise_all_rejects_with_the_first_observed_error() {
        let first = thaw_promise_new();
        let second = thaw_promise_new();
        let children = [first, second];
        let joined = unsafe { thaw_promise_all_f64(children.as_ptr(), children.len()) };
        let error = b"joined failure\0";
        assert_eq!(thaw_promise_reject(second, error.as_ptr()), 1);
        thaw_runtime_run_until_idle();
        assert_eq!(thaw_promise_state(joined), 2);
        let value = 1.0f64;
        assert_eq!(
            thaw_promise_resolve(first, (&value as *const f64).cast()),
            1
        );
        thaw_runtime_run_until_idle();
        assert_eq!(thaw_promise_state(joined), 2);
        assert_eq!(thaw_runtime_run_until_resolved(joined), error.as_ptr());
        unsafe { thaw_promise_destroy(joined) };
    }

    #[test]
    fn rejected_promise_all_drains_children_after_parent_destruction() {
        let failed = thaw_promise_new();
        let slow = timed_value(25, 4.0);
        let children = [failed, slow];
        let joined = unsafe { thaw_promise_all_f64(children.as_ptr(), children.len()) };
        let error = b"early failure\0";
        assert_eq!(thaw_promise_reject(failed, error.as_ptr()), 1);
        thaw_runtime_run_until_idle();
        assert_eq!(thaw_promise_state(joined), 2);
        unsafe { thaw_promise_destroy(joined) };
        assert_eq!(ACTIVE_PROMISE_JOINS.with(Cell::get), 1);
        thaw_runtime_drain_detached();
        assert_eq!(ACTIVE_PROMISE_JOINS.with(Cell::get), 0);
    }

    #[test]
    fn promise_all_drives_children_concurrently() {
        let started = Instant::now();
        let children = [
            timed_value(80, 1.0),
            timed_value(80, 2.0),
            timed_value(80, 3.0),
        ];
        let joined = unsafe { thaw_promise_all_f64(children.as_ptr(), children.len()) };
        assert!(!thaw_runtime_run_until_resolved(joined).is_null());
        let elapsed = started.elapsed();
        assert!(
            elapsed < Duration::from_millis(180),
            "three 80ms children ran serially: {elapsed:?}"
        );
        unsafe { thaw_promise_destroy(joined) };
    }

    #[test]
    fn promise_race_uses_completion_order_and_drains_the_loser() {
        let slow = timed_value(30, 1.0);
        let fast = timed_value(2, 2.0);
        let children = [slow, fast];
        let raced = unsafe { thaw_promise_race(children.as_ptr(), children.len()) };
        let result = thaw_runtime_run_until_resolved(raced);
        assert_eq!(unsafe { result.cast::<f64>().read_unaligned() }, 2.0);
        assert_eq!(ACTIVE_PROMISE_JOINS.with(Cell::get), 1);
        unsafe { thaw_promise_destroy(raced) };
        thaw_runtime_drain_detached();
        assert_eq!(ACTIVE_PROMISE_JOINS.with(Cell::get), 0);
    }

    #[test]
    fn promise_race_forwards_first_rejection_and_deduplicates_handles() {
        let failed = thaw_promise_new();
        let slow = timed_value(20, 4.0);
        let children = [failed, slow, failed];
        let raced = unsafe { thaw_promise_race(children.as_ptr(), children.len()) };
        let error = b"race failure\0";
        assert_eq!(thaw_promise_reject(failed, error.as_ptr()), 1);
        thaw_runtime_run_until_idle();
        assert_eq!(thaw_promise_state(raced), 2);
        assert_eq!(thaw_runtime_run_until_resolved(raced), error.as_ptr());
        unsafe { thaw_promise_destroy(raced) };
        thaw_runtime_drain_detached();
        assert_eq!(ACTIVE_PROMISE_JOINS.with(Cell::get), 0);
    }

    #[test]
    fn promise_race_rejects_an_empty_input() {
        let raced = unsafe { thaw_promise_race(std::ptr::null(), 0) };
        assert_eq!(thaw_promise_state(raced), 2);
        assert_eq!(
            thaw_runtime_run_until_resolved(raced),
            PROMISE_RACE_EMPTY_ERROR.as_ptr()
        );
        unsafe { thaw_promise_destroy(raced) };
    }

    #[test]
    fn promise_any_ignores_rejections_and_uses_the_first_fulfillment() {
        let failed = thaw_promise_new();
        let slow = timed_value(25, 1.0);
        let fast = timed_value(2, 2.0);
        let children = [failed, slow, fast, failed];
        let any = unsafe { thaw_promise_any(children.as_ptr(), children.len()) };
        let error = b"ignored failure\0";
        assert_eq!(thaw_promise_reject(failed, error.as_ptr()), 1);
        let result = thaw_runtime_run_until_resolved(any);
        assert_eq!(unsafe { result.cast::<f64>().read_unaligned() }, 2.0);
        unsafe { thaw_promise_destroy(any) };
        thaw_runtime_drain_detached();
        assert_eq!(ACTIVE_PROMISE_JOINS.with(Cell::get), 0);
    }

    #[test]
    fn promise_any_rejects_only_after_every_input_rejects() {
        let first = thaw_promise_new();
        let second = thaw_promise_new();
        let children = [first, second];
        let any = unsafe { thaw_promise_any(children.as_ptr(), children.len()) };
        let first_error = [b'f', b'i', b'r', b's', b't', 0];
        let second_error = [b's', b'e', b'c', b'o', b'n', b'd', 0];
        assert_eq!(thaw_promise_reject(first, first_error.as_ptr()), 1);
        thaw_runtime_run_until_idle();
        assert_eq!(thaw_promise_state(any), 0);
        assert_eq!(thaw_promise_reject(second, second_error.as_ptr()), 1);
        thaw_runtime_run_until_idle();
        assert_eq!(thaw_promise_state(any), 2);
        assert_eq!(
            thaw_runtime_run_until_resolved(any),
            PROMISE_ANY_REJECTED_ERROR.as_ptr()
        );
        unsafe { thaw_promise_destroy(any) };
    }

    #[test]
    fn promise_any_rejects_an_empty_input() {
        let any = unsafe { thaw_promise_any(std::ptr::null(), 0) };
        assert_eq!(thaw_promise_state(any), 2);
        assert_eq!(
            thaw_runtime_run_until_resolved(any),
            PROMISE_ANY_REJECTED_ERROR.as_ptr()
        );
        unsafe { thaw_promise_destroy(any) };
    }

    #[test]
    fn promise_all_settled_preserves_order_and_turns_rejections_into_values() {
        let fulfilled = thaw_promise_new();
        let rejected = thaw_promise_new();
        let children = [fulfilled, rejected, fulfilled];
        let settled = unsafe {
            thaw_promise_all_settled(children.as_ptr(), children.len(), size_of::<f64>())
        };
        let error = b"settled failure\0";
        let number = 7.0f64;
        assert_eq!(thaw_promise_reject(rejected, error.as_ptr()), 1);
        thaw_runtime_run_until_idle();
        assert_eq!(thaw_promise_state(settled), 0);
        assert_eq!(
            thaw_promise_resolve(fulfilled, (&number as *const f64).cast()),
            1
        );
        thaw_runtime_run_until_idle();
        let result = unsafe { *thaw_runtime_run_until_resolved(settled).cast::<*const u64>() };
        assert_eq!(unsafe { result.read() }, 3);
        let first = unsafe { result.add(1).read() as *const u64 };
        let second = unsafe { result.add(2).read() as *const u64 };
        let third = unsafe { result.add(3).read() as *const u64 };
        assert_eq!(
            unsafe { first.read() as *const u8 },
            PROMISE_SETTLED_FULFILLED.as_ptr()
        );
        assert_eq!(f64::from_bits(unsafe { first.add(1).read() }), 7.0);
        assert_eq!(
            unsafe { first.add(2).read() as *const u8 },
            PROMISE_SETTLED_EMPTY_REASON.as_ptr()
        );
        assert_eq!(
            unsafe { second.read() as *const u8 },
            PROMISE_SETTLED_REJECTED.as_ptr()
        );
        assert_eq!(unsafe { second.add(1).read() }, 0);
        assert_eq!(unsafe { second.add(2).read() as *const u8 }, error.as_ptr());
        assert_eq!(
            unsafe { third.read() as *const u8 },
            PROMISE_SETTLED_FULFILLED.as_ptr()
        );
        assert_eq!(f64::from_bits(unsafe { third.add(1).read() }), 7.0);
        unsafe { thaw_promise_destroy(settled) };
    }

    #[test]
    fn promise_all_settled_resolves_an_empty_input() {
        let settled = unsafe { thaw_promise_all_settled(std::ptr::null(), 0, size_of::<f64>()) };
        assert_eq!(thaw_promise_state(settled), 1);
        let result = unsafe { *thaw_runtime_run_until_resolved(settled).cast::<*const u64>() };
        assert_eq!(unsafe { result.read() }, 0);
        unsafe { thaw_promise_destroy(settled) };
    }

    #[test]
    fn promise_all_settled_fulfills_when_every_input_rejects() {
        let first = thaw_promise_new();
        let second = thaw_promise_new();
        let children = [first, second];
        let settled = unsafe {
            thaw_promise_all_settled(children.as_ptr(), children.len(), size_of::<f64>())
        };
        let first_error = [b'f', b'i', b'r', b's', b't', 0];
        let second_error = [b's', b'e', b'c', b'o', b'n', b'd', 0];
        assert_eq!(thaw_promise_reject(first, first_error.as_ptr()), 1);
        assert_eq!(thaw_promise_reject(second, second_error.as_ptr()), 1);
        thaw_runtime_run_until_idle();
        assert_eq!(thaw_promise_state(settled), 1);
        unsafe { thaw_promise_destroy(settled) };
    }

    #[test]
    fn promise_all_deduplicates_repeated_handles() {
        let child = thaw_promise_new();
        let children = [child, child];
        let joined = unsafe { thaw_promise_all_f64(children.as_ptr(), children.len()) };
        let value = 9.0f64;
        assert_eq!(
            thaw_promise_resolve(child, (&value as *const f64).cast()),
            1
        );
        thaw_runtime_run_until_idle();
        assert_eq!(thaw_promise_state(joined), 1);
        let result = unsafe { *thaw_runtime_run_until_resolved(joined).cast::<*const u64>() };
        assert_eq!(f64::from_bits(unsafe { result.add(1).read() }), 9.0);
        assert_eq!(f64::from_bits(unsafe { result.add(2).read() }), 9.0);
        unsafe { thaw_promise_destroy(joined) };
    }

    #[test]
    fn timer_promise_is_driven_without_a_worker_thread() {
        let promise = thaw_sleep_ms(1);
        let mut record = ResumeRecord {
            calls: 0,
            result: std::ptr::null(),
        };
        assert_eq!(
            thaw_promise_subscribe(
                promise,
                record_resume,
                (&mut record as *mut ResumeRecord).cast()
            ),
            1
        );

        let result = thaw_runtime_run_until_resolved(promise);
        assert!(!result.is_null());
        assert_eq!(record.calls, 1);
        assert_eq!(record.result, result);
        unsafe { thaw_promise_destroy(promise) };
    }

    #[test]
    fn fd_readiness_resolves_and_resumes_a_subscriber() {
        let mut fds = [0; 2];
        assert_eq!(unsafe { libc::pipe(fds.as_mut_ptr()) }, 0);
        let promise = thaw_runtime_wait_fd(fds[0], THAW_FD_READABLE);
        let mut record = ResumeRecord {
            calls: 0,
            result: std::ptr::null(),
        };
        assert_eq!(
            thaw_promise_subscribe(
                promise,
                record_resume,
                (&mut record as *mut ResumeRecord).cast(),
            ),
            1
        );
        assert_eq!(
            unsafe { libc::write(fds[1], b"ready".as_ptr().cast(), 5) },
            5
        );
        assert!(!thaw_runtime_run_until_resolved(promise).is_null());
        assert_eq!(thaw_promise_state(promise), 1);
        assert_eq!(record.calls, 1);
        assert_eq!(thaw_runtime_run_until_idle(), 0);
        unsafe { thaw_promise_destroy(promise) };
        unsafe {
            libc::close(fds[0]);
            libc::close(fds[1]);
        }
    }

    #[test]
    fn persistent_fd_watcher_dispatches_through_the_shared_event_loop() {
        extern "C" fn record_watcher(context: *mut u8, events: i16) {
            let record = unsafe { &mut *(context as *mut (usize, i16)) };
            record.0 += 1;
            record.1 = events;
        }

        let mut fds = [0; 2];
        assert_eq!(unsafe { libc::pipe(fds.as_mut_ptr()) }, 0);
        let mut record = (0usize, 0i16);
        let watcher = thaw_runtime_watch_fd(
            fds[0],
            THAW_FD_READABLE,
            record_watcher,
            (&mut record as *mut (usize, i16)).cast(),
        );
        assert_ne!(watcher, 0);
        assert_eq!(
            unsafe { libc::write(fds[1], b"ready".as_ptr().cast(), 5) },
            5
        );
        assert!(thaw_runtime_run_one_event());
        assert_eq!(record.0, 1);
        assert_ne!(record.1 & libc::POLLIN, 0);
        assert!(thaw_runtime_unwatch_fd(watcher));
        assert!(!thaw_runtime_unwatch_fd(watcher));
        unsafe {
            libc::close(fds[0]);
            libc::close(fds[1]);
        }
    }

    #[test]
    fn timer_completes_while_a_persistent_watcher_is_idle() {
        extern "C" fn ignore_watcher(_context: *mut u8, _events: i16) {}

        let mut fds = [0; 2];
        assert_eq!(unsafe { libc::pipe(fds.as_mut_ptr()) }, 0);
        let watcher = thaw_runtime_watch_fd(
            fds[0],
            THAW_FD_READABLE,
            ignore_watcher,
            std::ptr::null_mut(),
        );
        let timer = thaw_sleep_ms(1);
        assert!(!thaw_runtime_run_until_resolved(timer).is_null());
        assert_eq!(thaw_promise_state(timer), 1);
        assert!(thaw_runtime_unwatch_fd(watcher));
        unsafe {
            thaw_promise_destroy(timer);
            libc::close(fds[0]);
            libc::close(fds[1]);
        }
    }

    #[test]
    fn timer_can_finish_while_an_fd_wait_remains_pending() {
        let mut fds = [0; 2];
        assert_eq!(unsafe { libc::pipe(fds.as_mut_ptr()) }, 0);
        let fd_promise = thaw_runtime_wait_fd(fds[0], THAW_FD_READABLE);
        let timer = thaw_sleep_ms(1);
        assert!(!thaw_runtime_run_until_resolved(timer).is_null());
        assert_eq!(thaw_promise_state(timer), 1);
        assert_eq!(thaw_promise_state(fd_promise), 0);
        unsafe {
            thaw_promise_destroy(timer);
            thaw_promise_destroy(fd_promise);
            libc::close(fds[0]);
            libc::close(fds[1]);
        }
    }

    #[test]
    fn dns_resolution_completes_through_the_fd_event_loop() {
        let (fd, result) = start_dns_resolution("localhost".to_string(), 80).unwrap();
        let readiness = thaw_runtime_wait_fd_timeout(fd, THAW_FD_READABLE, 5_000);
        assert!(!thaw_runtime_run_until_resolved(readiness).is_null());
        assert_eq!(thaw_promise_state(readiness), 1);
        let resolved = result
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take()
            .expect("DNS worker must publish before notifying the pipe")
            .unwrap();
        assert!(!resolved.is_empty());
        unsafe {
            thaw_promise_destroy(readiness);
            libc::close(fd);
        }
    }

    #[test]
    fn invalid_fd_wait_is_rejected() {
        let promise = thaw_runtime_wait_fd(-1, THAW_FD_READABLE);
        assert_eq!(thaw_promise_state(promise), 2);
        assert_eq!(
            thaw_runtime_run_until_resolved(promise),
            INVALID_FD_ERROR.as_ptr()
        );
        unsafe { thaw_promise_destroy(promise) };
    }

    #[test]
    fn fd_wait_timeout_rejects_without_readiness() {
        let mut fds = [0; 2];
        assert_eq!(unsafe { libc::pipe(fds.as_mut_ptr()) }, 0);
        let promise = thaw_runtime_wait_fd_timeout(fds[0], THAW_FD_READABLE, 1);
        assert!(!thaw_runtime_run_until_resolved(promise).is_null());
        assert_eq!(thaw_promise_state(promise), 2);
        unsafe {
            thaw_promise_destroy(promise);
            libc::close(fds[0]);
            libc::close(fds[1]);
        }
    }

    #[test]
    fn parses_async_http_urls() {
        assert_eq!(
            parse_http_url("http://example.com:8080/api?q=1").unwrap(),
            (
                false,
                "example.com".to_string(),
                8080,
                "/api?q=1".to_string()
            )
        );
        assert_eq!(
            parse_http_url("https://[::1]/").unwrap(),
            (true, "::1".to_string(), 443, "/".to_string())
        );
    }

    #[test]
    fn incrementally_parses_content_length_response() {
        let partial = b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\n\r\nhe";
        assert!(parse_http_response(partial, false).unwrap().is_none());
        let complete = b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\n\r\nhelloignored";
        let parsed = parse_http_response(complete, false).unwrap().unwrap();
        assert_eq!(parsed.status, 200);
        assert_eq!(parsed.body, b"hello");
        assert!(parse_http_response(partial, true).is_err());
        let redirect = b"HTTP/1.1 302 Found\r\nLocation: ../next\r\nContent-Length: 0\r\n\r\n";
        let parsed = parse_http_response(redirect, false).unwrap().unwrap();
        assert_eq!(parsed.location.as_deref(), Some("../next"));
    }

    #[test]
    fn incrementally_decodes_chunked_response() {
        let partial = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n4\r\nWi";
        assert!(parse_http_response(partial, false).unwrap().is_none());
        let complete = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n4;name=value\r\nWiki\r\n5\r\npedia\r\n0\r\nX-End: yes\r\n\r\n";
        let parsed = parse_http_response(complete, false).unwrap().unwrap();
        assert_eq!(parsed.body, b"Wikipedia");
    }

    #[test]
    fn async_http_rejects_unsupported_scheme_without_blocking() {
        let url = CString::new("ftp://example.com/").unwrap();
        let promise = thaw_http_get_async(url.as_ptr());
        assert_eq!(thaw_promise_state(promise), 2);
        assert!(!thaw_runtime_run_until_resolved(promise).is_null());
        unsafe { thaw_promise_destroy(promise) };
    }

    #[test]
    fn async_http_and_timer_share_the_event_loop() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let (mut connection, _) = listener.accept().unwrap();
            let mut request = [0u8; 1024];
            let _ = connection.read(&mut request).unwrap();
            std::thread::sleep(Duration::from_millis(20));
            connection
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok")
                .unwrap();
        });
        let url = CString::new(format!("http://{addr}/data")).unwrap();
        let http = thaw_http_get_async_timeout(url.as_ptr(), 1_000);
        let timer = thaw_sleep_ms(1);
        assert!(!thaw_runtime_run_until_resolved(timer).is_null());
        assert_eq!(thaw_promise_state(timer), 1);
        assert_eq!(thaw_promise_state(http), 0);
        let result_slot = thaw_runtime_run_until_resolved(http) as *const *const c_char;
        assert_eq!(thaw_promise_state(http), 1);
        let body = unsafe { CStr::from_ptr(*result_slot) }.to_string_lossy();
        assert_eq!(body, "ok");
        server.join().unwrap();
        unsafe {
            thaw_promise_destroy(timer);
            thaw_promise_destroy(http);
        }
    }

    #[test]
    fn async_http_finishes_content_length_before_keep_alive_closes() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let (mut connection, _) = listener.accept().unwrap();
            let mut request = [0u8; 1024];
            let _ = connection.read(&mut request).unwrap();
            connection
                .write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\nConnection: keep-alive\r\n\r\nhello",
                )
                .unwrap();
            std::thread::sleep(Duration::from_millis(300));
        });
        let url = CString::new(format!("http://{addr}/keep-alive")).unwrap();
        let started = Instant::now();
        let promise = thaw_http_get_async_timeout(url.as_ptr(), 500);
        let result = thaw_runtime_run_until_resolved(promise);
        assert_eq!(
            thaw_promise_state(promise),
            1,
            "HTTP fetch rejected: {}",
            unsafe { CStr::from_ptr(result.cast()) }.to_string_lossy()
        );
        let result_slot = result as *const *const c_char;
        assert!(started.elapsed() < Duration::from_millis(150));
        assert_eq!(
            unsafe { CStr::from_ptr(*result_slot) }.to_string_lossy(),
            "hello"
        );
        server.join().unwrap();
        unsafe { thaw_promise_destroy(promise) };
    }

    #[test]
    fn async_http_decodes_chunked_keep_alive_response() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let (mut connection, _) = listener.accept().unwrap();
            let mut request = [0u8; 1024];
            let _ = connection.read(&mut request).unwrap();
            connection
                .write_all(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: keep-alive\r\n\r\n5\r\nhello\r\n1\r\n \r\n5\r\nworld\r\n0\r\n\r\n")
                .unwrap();
            std::thread::sleep(Duration::from_millis(20));
        });
        let url = CString::new(format!("http://{addr}/chunked")).unwrap();
        let promise = thaw_http_get_async_timeout(url.as_ptr(), 500);
        let result_slot = thaw_runtime_run_until_resolved(promise) as *const *const c_char;
        assert_eq!(thaw_promise_state(promise), 1);
        assert_eq!(
            unsafe { CStr::from_ptr(*result_slot) }.to_string_lossy(),
            "hello world"
        );
        server.join().unwrap();
        unsafe { thaw_promise_destroy(promise) };
    }

    #[test]
    fn async_https_handshakes_and_reads_a_verified_response() {
        let (client_config, server_config) = local_tls_configs();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let (connection, _) = listener.accept().unwrap();
            let tls = ServerConnection::new(server_config).unwrap();
            let mut stream = StreamOwned::new(tls, connection);
            let mut request = [0u8; 1024];
            let _ = stream.read(&mut request).unwrap();
            stream
                .write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Length: 6\r\nConnection: keep-alive\r\n\r\nsecure",
                )
                .unwrap();
            stream.flush().unwrap();
            std::thread::sleep(Duration::from_millis(20));
        });
        let url = CString::new(format!("https://{addr}/secure")).unwrap();
        let promise = thaw_http_get_async_with_config(url.as_ptr(), 1_000, Some(client_config));
        let result = thaw_runtime_run_until_resolved(promise);
        assert_eq!(
            thaw_promise_state(promise),
            1,
            "TLS fetch rejected: {}",
            unsafe { CStr::from_ptr(result.cast()) }.to_string_lossy()
        );
        let result_slot = result as *const *const c_char;
        assert_eq!(
            unsafe { CStr::from_ptr(*result_slot) }.to_string_lossy(),
            "secure"
        );
        server.join().unwrap();
        unsafe { thaw_promise_destroy(promise) };
    }

    #[test]
    fn async_https_rejects_an_untrusted_certificate() {
        let (_client_config, server_config) = local_tls_configs();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let (connection, _) = listener.accept().unwrap();
            let tls = ServerConnection::new(server_config).unwrap();
            let mut stream = StreamOwned::new(tls, connection);
            let mut request = [0u8; 64];
            let _ = stream.read(&mut request);
        });
        let url = CString::new(format!("https://{addr}/untrusted")).unwrap();
        let promise = thaw_http_get_async_timeout(url.as_ptr(), 1_000);
        assert!(!thaw_runtime_run_until_resolved(promise).is_null());
        assert_eq!(thaw_promise_state(promise), 2);
        server.join().unwrap();
        unsafe { thaw_promise_destroy(promise) };
    }

    #[test]
    fn async_https_handshake_obeys_the_total_timeout() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let (_connection, _) = listener.accept().unwrap();
            std::thread::sleep(Duration::from_millis(50));
        });
        let url = CString::new(format!("https://{addr}/stall")).unwrap();
        let promise = thaw_http_get_async_timeout(url.as_ptr(), 5);
        assert!(!thaw_runtime_run_until_resolved(promise).is_null());
        assert_eq!(thaw_promise_state(promise), 2);
        server.join().unwrap();
        unsafe { thaw_promise_destroy(promise) };
    }

    #[test]
    fn async_http_redirect_can_upgrade_to_https() {
        let (client_config, server_config) = local_tls_configs();
        let http_listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let http_addr = http_listener.local_addr().unwrap();
        let tls_listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let tls_addr = tls_listener.local_addr().unwrap();
        let redirect_server = std::thread::spawn(move || {
            let (mut connection, _) = http_listener.accept().unwrap();
            let mut request = [0u8; 1024];
            let _ = connection.read(&mut request).unwrap();
            connection
                .write_all(
                    format!(
                        "HTTP/1.1 302 Found\r\nLocation: https://{tls_addr}/secure\r\nContent-Length: 0\r\n\r\n"
                    )
                    .as_bytes(),
                )
                .unwrap();
        });
        let tls_server = std::thread::spawn(move || {
            let (connection, _) = tls_listener.accept().unwrap();
            let tls = ServerConnection::new(server_config).unwrap();
            let mut stream = StreamOwned::new(tls, connection);
            let mut request = [0u8; 1024];
            let _ = stream.read(&mut request).unwrap();
            stream
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 8\r\n\r\nupgraded")
                .unwrap();
            stream.flush().unwrap();
        });
        let url = CString::new(format!("http://{http_addr}/upgrade")).unwrap();
        let promise = thaw_http_get_async_with_config(url.as_ptr(), 2_000, Some(client_config));
        let result_slot = thaw_runtime_run_until_resolved(promise) as *const *const c_char;
        assert_eq!(thaw_promise_state(promise), 1);
        assert_eq!(
            unsafe { CStr::from_ptr(*result_slot) }.to_string_lossy(),
            "upgraded"
        );
        redirect_server.join().unwrap();
        tls_server.join().unwrap();
        unsafe { thaw_promise_destroy(promise) };
    }

    #[test]
    fn async_http_timeout_rejects_a_stalled_response() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let (mut connection, _) = listener.accept().unwrap();
            let mut request = [0u8; 1024];
            let _ = connection.read(&mut request).unwrap();
            std::thread::sleep(Duration::from_millis(50));
        });
        let url = CString::new(format!("http://{addr}/slow")).unwrap();
        let promise = thaw_http_get_async_timeout(url.as_ptr(), 5);
        assert!(!thaw_runtime_run_until_resolved(promise).is_null());
        assert_eq!(thaw_promise_state(promise), 2);
        server.join().unwrap();
        unsafe { thaw_promise_destroy(promise) };
    }

    #[test]
    fn async_http_rejects_when_peer_disconnects_without_a_response() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let (mut connection, _) = listener.accept().unwrap();
            let mut request = [0u8; 1024];
            let _ = connection.read(&mut request).unwrap();
        });
        let url = CString::new(format!("http://{addr}/disconnect")).unwrap();
        let promise = thaw_http_get_async_timeout(url.as_ptr(), 1_000);
        assert!(!thaw_runtime_run_until_resolved(promise).is_null());
        assert_eq!(thaw_promise_state(promise), 2);
        server.join().unwrap();
        unsafe { thaw_promise_destroy(promise) };
    }

    #[test]
    fn async_http_follows_a_relative_redirect() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let (mut first, _) = listener.accept().unwrap();
            let mut request = [0u8; 1024];
            let read = first.read(&mut request).unwrap();
            assert!(String::from_utf8_lossy(&request[..read]).starts_with("GET /one/start "));
            first
                .write_all(b"HTTP/1.1 302 Found\r\nLocation: ../final\r\nContent-Length: 0\r\n\r\n")
                .unwrap();
            drop(first);

            let (mut second, _) = listener.accept().unwrap();
            let read = second.read(&mut request).unwrap();
            assert!(String::from_utf8_lossy(&request[..read]).starts_with("GET /final "));
            second
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 10\r\n\r\nredirected")
                .unwrap();
        });
        let url = CString::new(format!("http://{addr}/one/start")).unwrap();
        let promise = thaw_http_get_async_timeout(url.as_ptr(), 1_000);
        let result_slot = thaw_runtime_run_until_resolved(promise) as *const *const c_char;
        assert_eq!(thaw_promise_state(promise), 1);
        assert_eq!(
            unsafe { CStr::from_ptr(*result_slot) }.to_string_lossy(),
            "redirected"
        );
        server.join().unwrap();
        unsafe { thaw_promise_destroy(promise) };
    }

    #[test]
    fn async_http_redirect_limit_rejects_a_loop() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            for _ in 0..11 {
                let (mut connection, _) = listener.accept().unwrap();
                let mut request = [0u8; 1024];
                let _ = connection.read(&mut request).unwrap();
                connection
                    .write_all(
                        b"HTTP/1.1 302 Found\r\nLocation: /loop\r\nContent-Length: 0\r\n\r\n",
                    )
                    .unwrap();
            }
        });
        let url = CString::new(format!("http://{addr}/loop")).unwrap();
        let promise = thaw_http_get_async_timeout(url.as_ptr(), 2_000);
        assert!(!thaw_runtime_run_until_resolved(promise).is_null());
        assert_eq!(thaw_promise_state(promise), 2);
        server.join().unwrap();
        unsafe { thaw_promise_destroy(promise) };
    }

    #[test]
    fn unresolved_promise_without_an_event_source_reports_no_progress() {
        let promise = thaw_promise_new();
        assert!(thaw_runtime_run_until_resolved(promise).is_null());
        unsafe { thaw_promise_destroy(promise) };
    }

    #[test]
    fn invocation_guard_resets_request_arena() {
        thaw_arena::thaw_arena_reset();
        let first = thaw_arena::thaw_arena_alloc(32, 8);
        assert!(!first.is_null());

        {
            let _guard = InvocationArenaReset;
            let second = thaw_arena::thaw_arena_alloc(32, 8);
            assert_ne!(first, second);
        }

        let after_reset = thaw_arena::thaw_arena_alloc(32, 8);
        assert_eq!(first, after_reset);
        thaw_arena::thaw_arena_reset();
    }

    #[test]
    fn polls_an_invocation_and_posts_the_handler_result() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap().to_string();

        let (tx, rx) = mpsc::channel();
        let server = std::thread::spawn(move || {
            let (mut conn, _) = listener.accept().unwrap();
            let mut buf = [0u8; 4096];
            let n = conn.read(&mut buf).unwrap();
            let req = String::from_utf8_lossy(&buf[..n]);
            assert!(req.starts_with("GET /2018-06-01/runtime/invocation/next"));

            let body = "\"hello\"";
            let response = format!(
                "HTTP/1.1 200 OK\r\nLambda-Runtime-Aws-Request-Id: req-123\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            conn.write_all(response.as_bytes()).unwrap();
            drop(conn);

            let (mut conn, _) = listener.accept().unwrap();
            let mut buf = Vec::new();
            conn.read_to_end(&mut buf).unwrap();
            tx.send(String::from_utf8_lossy(&buf).into_owned()).unwrap();

            let response =
                "HTTP/1.1 202 Accepted\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
            conn.write_all(response.as_bytes()).unwrap();
        });

        handle_one_invocation(&addr, echo_handler, std::ptr::null_mut()).unwrap();
        server.join().unwrap();

        let post_request = rx.recv().unwrap();
        assert!(post_request.starts_with("POST /2018-06-01/runtime/invocation/req-123/response"));
        assert!(post_request.ends_with("echo:\"hello\""));
    }

    #[test]
    fn posts_uncaught_handler_exception_to_the_lambda_error_endpoint() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap().to_string();
        let (tx, rx) = mpsc::channel();
        let server = std::thread::spawn(move || {
            let (mut conn, _) = listener.accept().unwrap();
            let mut request = [0u8; 4096];
            let _ = conn.read(&mut request).unwrap();
            conn.write_all(
                b"HTTP/1.1 200 OK\r\nLambda-Runtime-Aws-Request-Id: req-error\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}",
            )
            .unwrap();
            drop(conn);

            let (mut conn, _) = listener.accept().unwrap();
            let mut request = Vec::new();
            conn.read_to_end(&mut request).unwrap();
            tx.send(String::from_utf8_lossy(&request).into_owned())
                .unwrap();
            conn.write_all(
                b"HTTP/1.1 202 Accepted\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
            )
            .unwrap();
        });

        handle_one_invocation(&addr, failing_handler, &raw mut TEST_PENDING_EXCEPTION).unwrap();
        server.join().unwrap();
        let request = rx.recv().unwrap();
        assert!(request.starts_with("POST /2018-06-01/runtime/invocation/req-error/error"));
        assert!(request.contains(r#"{"errorMessage":"handler exploded","errorType":"ThawError"}"#));
    }

    #[test]
    fn surfaces_http_error_status_as_an_error() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap().to_string();

        let server = std::thread::spawn(move || {
            let (mut conn, _) = listener.accept().unwrap();
            let mut buf = [0u8; 4096];
            let _ = conn.read(&mut buf).unwrap();
            let body = "boom";
            let response = format!(
                "HTTP/1.1 500 Internal Server Error\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            conn.write_all(response.as_bytes()).unwrap();
        });

        let err = handle_one_invocation(&addr, echo_handler, std::ptr::null_mut()).unwrap_err();
        assert!(err.contains("500"));
        server.join().unwrap();
    }
}
