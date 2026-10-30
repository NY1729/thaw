// Arbitrary-precision native `bigint`.
//
// A `bigint` value is one tagged `i64` handle: low bit 1 is an inline signed
// 63-bit integer (`(v << 1) | 1`); low bit 0 is a pointer to a leaked,
// immutable `num_bigint::BigInt` that never fits the inline range (so a heap
// handle never equals an inline one); raw 0 is a zero-initialised slot and
// reads as `0n`. Every operation decodes, computes and re-encodes here, so
// compiled code never inspects a handle itself.
// ponytail: heap values are leaked (Lambda lifetime) and add/sub/mul have no
// LLVM inline fast path; add both if big-loop profiles ask for it.
use num_bigint::{BigInt, Sign};
use num_integer::Integer;
use num_traits::{Signed, ToPrimitive, Zero};

const BIGINT_MAX_BITS: u64 = 1 << 30;

fn bigint_encode(value: BigInt) -> i64 {
    if let Some(small) = value.to_i64() {
        if (-(1i64 << 62)..(1i64 << 62)).contains(&small) {
            return (small << 1) | 1;
        }
    }
    Box::into_raw(Box::new(value)) as i64
}

fn bigint_decode(handle: i64) -> BigInt {
    if handle & 1 == 1 {
        BigInt::from(handle >> 1)
    } else if handle == 0 || handle & 7 != 0 || handle < 4096 {
        // Zero-initialised slot, or a stray non-pointer: read as 0n instead of dereferencing it.
        BigInt::zero()
    } else {
        unsafe { (*(handle as *const BigInt)).clone() }
    }
}

fn bigint_inline(handle: i64) -> Option<i64> {
    (handle & 1 == 1).then_some(handle >> 1)
}

fn bigint_c_string(text: &str) -> *const c_char {
    arena_c_string(text).map_or(std::ptr::null(), |value| value.cast())
}

/// A raw (untagged) signed 64-bit integer as a `bigint`.
#[no_mangle]
pub extern "C" fn thaw_bigint_from_i64(value: i64) -> i64 {
    bigint_encode(BigInt::from(value))
}

/// A raw 64-bit pattern read as an unsigned integer, as a `bigint`.
#[no_mangle]
pub extern "C" fn thaw_bigint_from_u64(value: u64) -> i64 {
    bigint_encode(BigInt::from(value))
}

/// The low 64 bits of the two's-complement value (`BigInt.asIntN(64, v)` as a raw `i64`).
#[no_mangle]
pub extern "C" fn thaw_bigint_to_i64_wrapping(handle: i64) -> i64 {
    if let Some(small) = bigint_inline(handle) {
        return small;
    }
    let value = bigint_decode(handle);
    let mask = (BigInt::from(1u8) << 64usize) - 1;
    let masked: BigInt = value & mask;
    let low = masked.to_u64().unwrap_or(0);
    low as i64
}

/// `BigInt(number)` for an integral finite number (callers reject the rest).
#[no_mangle]
pub extern "C" fn thaw_bigint_from_f64(value: f64) -> i64 {
    use num_traits::FromPrimitive;
    bigint_encode(BigInt::from_f64(value.trunc()).unwrap_or_default())
}

#[no_mangle]
pub extern "C" fn thaw_bigint_to_f64(handle: i64) -> f64 {
    if let Some(small) = bigint_inline(handle) {
        return small as f64;
    }
    let value = bigint_decode(handle);
    value.to_f64().unwrap_or(if value.is_negative() { f64::NEG_INFINITY } else { f64::INFINITY })
}

fn bigint_parse(text: &str) -> Option<BigInt> {
    let text = text.trim_matches(|c: char| c.is_whitespace() || c == '\u{feff}');
    if text.is_empty() {
        return Some(BigInt::zero());
    }
    let (radix, digits, negative) = match text.get(..2) {
        Some("0x" | "0X") => (16, &text[2..], false),
        Some("0o" | "0O") => (8, &text[2..], false),
        Some("0b" | "0B") => (2, &text[2..], false),
        _ => match text.as_bytes()[0] {
            b'-' => (10, &text[1..], true),
            b'+' => (10, &text[1..], false),
            _ => (10, text, false),
        },
    };
    if digits.is_empty() || !digits.bytes().all(|b| (b as char).is_digit(radix)) {
        return None;
    }
    let magnitude = BigInt::parse_bytes(digits.as_bytes(), radix)?;
    Some(if negative { -magnitude } else { magnitude })
}

/// `BigInt(string)`: the parsed value, or `0n` when the text is not a valid
/// StringToBigInt (callers check `thaw_bigint_parse_ok` first and throw).
///
/// # Safety
/// `value` must be null or a valid NUL-terminated C string.
#[no_mangle]
pub unsafe extern "C" fn thaw_i64_from_string(value: *const c_char) -> i64 {
    let text = unsafe { CStr::from_ptr(value) }.to_string_lossy();
    bigint_encode(bigint_parse(&text).unwrap_or_default())
}

/// Whether `value` is a valid StringToBigInt text.
///
/// # Safety
/// `value` must be null or a valid NUL-terminated C string.
#[no_mangle]
pub unsafe extern "C" fn thaw_bigint_parse_ok(value: *const c_char) -> bool {
    bigint_parse(&unsafe { CStr::from_ptr(value) }.to_string_lossy()).is_some()
}

/// `SyntaxError` text of a failed `BigInt(string)`.
///
/// # Safety
/// `value` must be a valid NUL-terminated C string.
#[no_mangle]
pub unsafe extern "C" fn thaw_bigint_parse_error_message(value: *const c_char) -> *const c_char {
    let text = unsafe { CStr::from_ptr(value) }.to_string_lossy();
    bigint_c_string(&format!("Cannot convert {text} to a BigInt"))
}

#[no_mangle]
pub extern "C" fn thaw_i64_to_string(handle: i64) -> *const c_char {
    bigint_c_string(&bigint_decode(handle).to_string())
}

/// `console.log`'s rendering: decimal digits with a trailing `n`.
#[no_mangle]
pub extern "C" fn thaw_i64_to_bigint_string(handle: i64) -> *const c_char {
    bigint_c_string(&format!("{}n", bigint_decode(handle)))
}

/// `BigInt.prototype.toString(radix)`; callers have validated `radix` in 2..=36.
#[no_mangle]
pub extern "C" fn thaw_i64_to_radix_string(handle: i64, radix: f64) -> *const c_char {
    let radix = if radix.is_finite() { radix.trunc() as u32 } else { 10 }.clamp(2, 36);
    bigint_c_string(&bigint_decode(handle).to_str_radix(radix))
}

fn bigint_mod_pow2(value: &BigInt, bits: u64) -> BigInt {
    let bits = bits.min(BIGINT_MAX_BITS) as usize;
    value.mod_floor(&(BigInt::from(1u8) << bits))
}

/// `BigInt.asUintN(bits, value)`; `bits` is an already-validated index.
#[no_mangle]
pub extern "C" fn thaw_i64_as_uint_n(handle: i64, bits: f64) -> i64 {
    let bits = bits as u64;
    if bits == 0 {
        return thaw_bigint_from_i64(0);
    }
    bigint_encode(bigint_mod_pow2(&bigint_decode(handle), bits))
}

/// `BigInt.asIntN(bits, value)`; `bits` is an already-validated index.
#[no_mangle]
pub extern "C" fn thaw_i64_as_int_n(handle: i64, bits: f64) -> i64 {
    let bits = (bits as u64).min(BIGINT_MAX_BITS);
    if bits == 0 {
        return thaw_bigint_from_i64(0);
    }
    let value = bigint_decode(handle);
    let wrapped = bigint_mod_pow2(&value, bits);
    let limit = BigInt::from(1u8) << (bits as usize - 1);
    bigint_encode(if wrapped >= limit { wrapped - (BigInt::from(1u8) << bits as usize) } else { wrapped })
}

/// Binary operator codes shared with `thaw_bigint_binop_error` and codegen.
const BIGINT_DIV: i32 = 0;
const BIGINT_MOD: i32 = 1;
const BIGINT_POW: i32 = 2;
const BIGINT_SHL: i32 = 3;
const BIGINT_SHR: i32 = 4;

/// 0 when the operation is defined; 1 `Division by zero`, 2 negative
/// exponent, 3 `Maximum BigInt size exceeded`.
#[no_mangle]
pub extern "C" fn thaw_bigint_binop_error(op: i32, left: i64, right: i64) -> i32 {
    match op {
        BIGINT_DIV | BIGINT_MOD if bigint_decode(right).is_zero() => 1,
        BIGINT_POW => {
            let exponent = bigint_decode(right);
            if exponent.is_negative() {
                return 2;
            }
            let base = bigint_decode(left);
            if base.is_zero() || base.abs() == BigInt::from(1u8) || exponent.is_zero() {
                return 0;
            }
            match exponent.to_u64() {
                Some(e) if base.bits().saturating_mul(e) <= BIGINT_MAX_BITS => 0,
                _ => 3,
            }
        }
        BIGINT_SHL | BIGINT_SHR => {
            let amount = bigint_decode(right);
            let left_shift = (op == BIGINT_SHL) != amount.is_negative();
            if left_shift && !bigint_decode(left).is_zero()
                && amount.abs().to_u64().is_none_or(|a| a.saturating_add(bigint_decode(left).bits()) > BIGINT_MAX_BITS)
            {
                3
            } else {
                0
            }
        }
        _ => 0,
    }
}

macro_rules! bigint_binary_fn {
    ($name:ident, $small:expr, $big:expr) => {
        #[no_mangle]
        pub extern "C" fn $name(left: i64, right: i64) -> i64 {
            if let (Some(a), Some(b)) = (bigint_inline(left), bigint_inline(right)) {
                let small: fn(i128, i128) -> i128 = $small;
                return bigint_encode(BigInt::from(small(a as i128, b as i128)));
            }
            let big: fn(BigInt, BigInt) -> BigInt = $big;
            bigint_encode(big(bigint_decode(left), bigint_decode(right)))
        }
    };
}

bigint_binary_fn!(thaw_bigint_add, |a, b| a + b, |a, b| a + b);
bigint_binary_fn!(thaw_bigint_sub, |a, b| a - b, |a, b| a - b);
bigint_binary_fn!(thaw_bigint_mul, |a, b| a * b, |a, b| a * b);
bigint_binary_fn!(thaw_bigint_div, |a, b| a / b, |a, b| a / b);
bigint_binary_fn!(thaw_bigint_mod, |a, b| a % b, |a, b| a % b);
bigint_binary_fn!(thaw_bigint_and, |a, b| a & b, |a, b| a & b);
bigint_binary_fn!(thaw_bigint_or, |a, b| a | b, |a, b| a | b);
bigint_binary_fn!(thaw_bigint_xor, |a, b| a ^ b, |a, b| a ^ b);

fn bigint_shift(value: BigInt, amount: &BigInt, left: bool) -> BigInt {
    let left = left != amount.is_negative();
    let Some(count) = amount.abs().to_u64().and_then(|c| usize::try_from(c).ok()) else {
        return if left || !value.is_negative() { BigInt::zero() } else { BigInt::from(-1) };
    };
    if left { value << count } else { value >> count }
}

#[no_mangle]
pub extern "C" fn thaw_bigint_shl(left: i64, right: i64) -> i64 {
    bigint_encode(bigint_shift(bigint_decode(left), &bigint_decode(right), true))
}

#[no_mangle]
pub extern "C" fn thaw_bigint_shr(left: i64, right: i64) -> i64 {
    bigint_encode(bigint_shift(bigint_decode(left), &bigint_decode(right), false))
}

/// `base ** exp`; callers check `thaw_bigint_binop_error` first.
#[no_mangle]
pub extern "C" fn thaw_bigint_pow(base: i64, exp: i64) -> i64 {
    use num_traits::Pow;
    let base = bigint_decode(base);
    let Some(exp) = bigint_decode(exp).to_u64() else { return thaw_bigint_from_i64(0) };
    if base.is_zero() || exp == 0 {
        return thaw_bigint_from_i64(i64::from(exp == 0 || !base.is_zero()));
    }
    if base == BigInt::from(1u8) {
        return thaw_bigint_from_i64(1);
    }
    if base == BigInt::from(-1) {
        return thaw_bigint_from_i64(if exp % 2 == 0 { 1 } else { -1 });
    }
    bigint_encode(Pow::pow(base, exp as u32))
}

#[no_mangle]
pub extern "C" fn thaw_bigint_neg(value: i64) -> i64 {
    bigint_encode(-bigint_decode(value))
}

/// `~value`, i.e. `-value - 1`.
#[no_mangle]
pub extern "C" fn thaw_bigint_not(value: i64) -> i64 {
    bigint_encode(-bigint_decode(value) - 1)
}

/// `-1`/`0`/`1` ordering of two bigints.
#[no_mangle]
pub extern "C" fn thaw_bigint_cmp(left: i64, right: i64) -> i32 {
    if let (Some(a), Some(b)) = (bigint_inline(left), bigint_inline(right)) {
        return a.cmp(&b) as i32;
    }
    bigint_decode(left).cmp(&bigint_decode(right)) as i32
}

/// Exact ordering of a bigint against a number: `-1`/`0`/`1`, or `2` when the
/// number is NaN (every comparison false).
#[no_mangle]
pub extern "C" fn thaw_bigint_cmp_f64(left: i64, right: f64) -> i32 {
    use num_traits::FromPrimitive;
    if right.is_nan() {
        return 2;
    }
    if right.is_infinite() {
        return if right > 0.0 { -1 } else { 1 };
    }
    let whole = BigInt::from_f64(right.trunc()).unwrap_or_default();
    match bigint_decode(left).cmp(&whole) {
        std::cmp::Ordering::Equal if right.fract() > 0.0 => -1,
        std::cmp::Ordering::Equal if right.fract() < 0.0 => 1,
        order => order as i32,
    }
}

#[no_mangle]
pub extern "C" fn thaw_bigint_is_zero(value: i64) -> bool {
    bigint_decode(value).is_zero()
}

/// `bigint` truthiness / sign helpers for codegen.
#[no_mangle]
pub extern "C" fn thaw_bigint_sign(value: i64) -> i32 {
    match bigint_decode(value).sign() {
        Sign::Minus => -1,
        Sign::NoSign => 0,
        Sign::Plus => 1,
    }
}

#[cfg(test)]
mod bigint_arbitrary_tests {
    use super::*;

    fn big(text: &str) -> i64 {
        let text = CString::new(text).unwrap();
        unsafe { thaw_i64_from_string(text.as_ptr()) }
    }

    fn show(handle: i64) -> String {
        unsafe { CStr::from_ptr(thaw_i64_to_string(handle)) }.to_string_lossy().into_owned()
    }

    #[test]
    fn arithmetic_leaves_the_inline_range_without_wrapping() {
        let two = big("2");
        assert_eq!(show(thaw_bigint_pow(two, big("64"))), "18446744073709551616");
        assert_eq!(show(thaw_bigint_pow(two, big("100"))), "1267650600228229401496703205376");
        assert_eq!(show(thaw_bigint_pow(big("10"), big("20"))), "100000000000000000000");
        assert_eq!(show(thaw_bigint_shl(big("1"), big("70"))), "1180591620717411303424");
        assert_eq!(show(thaw_bigint_mul(big("9223372036854775807"), big("9223372036854775807"))),
            "85070591730234615847396907784232501249");
        let product = (1..=25).fold(big("1"), |acc, i| thaw_bigint_mul(acc, big(&i.to_string())));
        assert_eq!(show(product), "15511210043330985984000000");
        assert_eq!(show(thaw_bigint_sub(big("-9223372036854775808"), big("1"))), "-9223372036854775809");
        assert_eq!(thaw_bigint_cmp(thaw_bigint_sub(thaw_bigint_pow(two, big("70")), big("1")), thaw_bigint_pow(two, big("70"))), -1);
    }

    #[test]
    fn division_shift_and_bit_semantics_match_javascript() {
        assert_eq!(show(thaw_bigint_div(big("-7"), big("2"))), "-3");
        assert_eq!(show(thaw_bigint_mod(big("-7"), big("2"))), "-1");
        assert_eq!(show(thaw_bigint_shr(big("-5"), big("1"))), "-3");
        assert_eq!(show(thaw_bigint_shl(big("5"), big("-1"))), "2");
        assert_eq!(show(thaw_bigint_not(big("5"))), "-6");
        assert_eq!(show(thaw_bigint_and(big("-1"), big("255"))), "255");
        assert_eq!(thaw_bigint_binop_error(BIGINT_DIV, big("1"), big("0")), 1);
        assert_eq!(thaw_bigint_binop_error(BIGINT_POW, big("2"), big("-1")), 2);
        assert_eq!(thaw_bigint_binop_error(BIGINT_POW, big("2"), big("100000000000")), 3);
        assert_eq!(thaw_bigint_binop_error(BIGINT_SHL, big("1"), big("100000000000")), 3);
    }

    #[test]
    fn width_conversions_and_parsing() {
        assert_eq!(show(thaw_i64_as_uint_n(big("-1"), 64.0)), "18446744073709551615");
        assert_eq!(show(thaw_i64_as_int_n(big("18446744073709551615"), 64.0)), "-1");
        assert_eq!(show(thaw_i64_as_int_n(big("255"), 8.0)), "-1");
        assert_eq!(show(thaw_i64_as_uint_n(big("-1"), 100.0)), "1267650600228229401496703205375");
        assert_eq!(show(thaw_bigint_from_f64(1e21)), "1000000000000000000000");
        assert_eq!(show(big("0xff")), "255");
        assert_eq!(show(big("  -12  ")), "-12");
        assert_eq!(show(big("")), "0");
        for bad in ["1.5", "1e3", "12a", "-0x1", "--1", "_1"] {
            let text = CString::new(bad).unwrap();
            assert!(!unsafe { thaw_bigint_parse_ok(text.as_ptr()) }, "{bad}");
        }
        assert_eq!(thaw_bigint_cmp_f64(big("3"), 3.5), -1);
        assert_eq!(thaw_bigint_cmp_f64(big("4"), 3.5), 1);
        assert_eq!(thaw_bigint_cmp_f64(thaw_bigint_pow(big("2"), big("70")), 1e30), -1);
        assert_eq!(thaw_bigint_to_f64(thaw_bigint_pow(big("2"), big("70"))), 2f64.powi(70));
        assert_eq!(thaw_bigint_to_i64_wrapping(thaw_bigint_pow(big("2"), big("64"))), 0);
        assert_eq!(thaw_bigint_to_i64_wrapping(thaw_bigint_from_u64(u64::MAX)), -1);
        // raw zero (an uninitialised slot) reads as 0n
        assert_eq!(show(0), "0");
    }

    #[test]
    fn number_ordering_is_exact_above_2_pow_53() {
        assert_eq!(thaw_bigint_cmp_f64(big("9007199254740993"), 9007199254740992.0), 1);
        assert_eq!(thaw_bigint_cmp_f64(big("9007199254740992"), 9007199254740992.0), 0);
        assert_eq!(thaw_bigint_cmp_f64(big("1180591620717411303424"), 2f64.powi(70)), 0);
        assert_eq!(thaw_bigint_cmp_f64(big("1180591620717411303425"), 2f64.powi(70)), 1);
        assert_eq!(thaw_bigint_cmp_f64(big("1"), f64::NAN), 2);
        assert_eq!(thaw_bigint_cmp_f64(big("-5"), f64::NEG_INFINITY), 1);
    }

    #[test]
    fn heap_backed_values_are_equal_map_keys_and_array_search_hits() {
        let (one, two) = (thaw_bigint_pow(big("2"), big("70")), thaw_bigint_pow(big("2"), big("70")));
        assert_ne!(one, two);
        assert_eq!(<BigKey as KeyKind>::hash(one as u64), <BigKey as KeyKind>::hash(two as u64));
        assert!(<BigKey as KeyKind>::eq(one as u64, two as u64));
        assert!(!<BigKey as KeyKind>::eq(one as u64, big("5") as u64));
        let map = unsafe { thaw_map_new() };
        unsafe {
            assert_eq!(thaw_map_big_set(map, one, 1), 1);
            assert_eq!(thaw_map_big_set(map, two, 2), 1);
            assert_eq!(thaw_map_big_set(map, big("5"), 3), 1);
            assert_eq!(thaw_map_size(map), 2.0);
            assert_eq!(thaw_map_big_get_i64(map, thaw_bigint_pow(big("2"), big("70"))), 2);
            assert_eq!(thaw_map_big_has(map, big("6")), 0);
            assert_eq!(thaw_map_big_delete(map, two), 1);
            assert_eq!(thaw_map_size(map), 1.0);
        }
    }
}
