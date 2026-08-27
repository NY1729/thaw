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
pub extern "C" fn thaw_number_object_is(left: f64, right: f64) -> u8 {
    if left.is_nan() && right.is_nan() {
        return 1;
    }
    if left == 0.0 && right == 0.0 {
        return (left.is_sign_negative() == right.is_sign_negative()).into();
    }
    (left == right).into()
}
