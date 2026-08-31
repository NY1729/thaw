//! Small runtime JIT for residual operations that Thaw cannot specialize AOT.
//!
//! This intentionally is not a JavaScript engine. It accepts a compact numeric
//! expression IR and emits one W^X-protected native code stub per symbol.

use std::cell::Cell;
use std::collections::HashMap;
use std::ffi::{CStr, CString};
use std::os::raw::c_char;
use std::ptr;
use std::sync::{Mutex, OnceLock};

static INVALID_SYMBOL: &[u8] = b"invalid JIT symbol\0";
const ABSENT_STATUS: *const c_char = ptr::dangling();
#[cfg(not(all(target_arch = "x86_64", target_family = "unix")))]
static UNSUPPORTED_TARGET: &[u8] = b"JIT target is not supported\0";
#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
static ALLOCATION_FAILED: &[u8] = b"failed to allocate JIT code\0";
#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
static INVALID_REPEAT_COUNT: &[u8] = b"invalid string repeat count\0";

pub type ArenaAlloc = unsafe extern "C" fn(usize, usize) -> *mut u8;
pub type NumberToString = unsafe extern "C" fn(f64) -> *const c_char;

thread_local! {
    static ARENA_ALLOC: Cell<Option<ArenaAlloc>> = const { Cell::new(None) };
    static NUMBER_TO_STRING: Cell<Option<NumberToString>> = const { Cell::new(None) };
    static CALL_ERROR: Cell<*const c_char> = const { Cell::new(ptr::null()) };
    static CALL_PRESENT: Cell<bool> = const { Cell::new(true) };
}

static STRING_CONSTANTS: OnceLock<Mutex<HashMap<String, CString>>> = OnceLock::new();

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
#[link(name = "m")]
extern "C" {
    fn fmod(left: f64, right: f64) -> f64;
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn minimum(left: f64, right: f64) -> f64 {
    if left.is_nan() || right.is_nan() {
        return f64::NAN;
    }
    if left == right {
        return if left == 0.0 {
            f64::from_bits(left.to_bits() | right.to_bits())
        } else {
            left
        };
    }
    if left < right {
        left
    } else {
        right
    }
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn maximum(left: f64, right: f64) -> f64 {
    if left.is_nan() || right.is_nan() {
        return f64::NAN;
    }
    if left == right {
        return if left == 0.0 {
            f64::from_bits(left.to_bits() & right.to_bits())
        } else {
            left
        };
    }
    if left > right {
        left
    } else {
        right
    }
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn floor_number(value: f64) -> f64 {
    value.floor()
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn ceil_number(value: f64) -> f64 {
    value.ceil()
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn truncate_number(value: f64) -> f64 {
    value.trunc()
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn round_number(value: f64) -> f64 {
    if !value.is_finite() || value == 0.0 {
        return value;
    }
    let rounded = (value + 0.5).floor();
    if rounded == 0.0 && value.is_sign_negative() {
        -0.0
    } else {
        rounded
    }
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
macro_rules! unary_math_helpers {
    ($($function:ident => $method:ident),+ $(,)?) => {
        $(extern "C" fn $function(value: f64) -> f64 { value.$method() })+
    };
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
unary_math_helpers! {
    acos_number => acos,
    acosh_number => acosh,
    asin_number => asin,
    asinh_number => asinh,
    atan_number => atan,
    atanh_number => atanh,
    cbrt_number => cbrt,
    cos_number => cos,
    cosh_number => cosh,
    exp_number => exp,
    expm1_number => exp_m1,
    log_number => ln,
    log1p_number => ln_1p,
    log2_number => log2,
    log10_number => log10,
    sin_number => sin,
    sinh_number => sinh,
    tan_number => tan,
    tanh_number => tanh,
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn sign_number(value: f64) -> f64 {
    if value.is_nan() || value == 0.0 {
        value
    } else if value.is_sign_positive() {
        1.0
    } else {
        -1.0
    }
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn atan2_number(y: f64, x: f64) -> f64 {
    y.atan2(x)
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn hypot_number(left: f64, right: f64) -> f64 {
    left.hypot(right)
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn fround_number(value: f64) -> f64 {
    value as f32 as f64
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn to_uint32(value: f64) -> u32 {
    if !value.is_finite() || value == 0.0 {
        return 0;
    }
    let mut modulo = value.trunc() % 4_294_967_296.0;
    if modulo < 0.0 {
        modulo += 4_294_967_296.0;
    }
    modulo as u32
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn clz32_number(value: f64) -> f64 {
    to_uint32(value).leading_zeros() as f64
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn imul_number(left: f64, right: f64) -> f64 {
    to_uint32(left).wrapping_mul(to_uint32(right)) as i32 as f64
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
unsafe fn string_argument(value: f64) -> Option<String> {
    let pointer = value.to_bits() as usize as *const c_char;
    (!pointer.is_null()).then(|| CStr::from_ptr(pointer).to_string_lossy().into_owned())
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn string_length(value: f64) -> f64 {
    unsafe {
        string_argument(value)
            .map(|value| value.encode_utf16().count() as f64)
            .unwrap_or(0.0)
    }
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
extern "C" fn string_is_well_formed(_: f64) -> f64 {
    1.0
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn clamped_string_position(position: f64, length: usize) -> usize {
    if position.is_nan() || position == f64::NEG_INFINITY {
        0
    } else if position == f64::INFINITY {
        length
    } else {
        position.trunc().clamp(0.0, length as f64) as usize
    }
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
unsafe fn positioned_string_search(value: f64, search: f64, position: f64, kind: u8) -> f64 {
    let Some(value) = string_argument(value) else {
        return -1.0;
    };
    let Some(search) = string_argument(search) else {
        return -1.0;
    };
    let value = value.encode_utf16().collect::<Vec<_>>();
    let search = search.encode_utf16().collect::<Vec<_>>();
    let position = clamped_string_position(position, value.len());
    match kind {
        0 => f64::from(value.get(position..position.saturating_add(search.len())) == Some(&search)),
        1 => f64::from(
            search.len() <= position
                && value.get(position - search.len()..position) == Some(&search),
        ),
        2 | 3 => {
            if search.is_empty() {
                return if kind == 2 { 1.0 } else { position as f64 };
            }
            let found = value[position..]
                .windows(search.len())
                .position(|window| window == search)
                .map(|index| position + index);
            if kind == 2 {
                f64::from(found.is_some())
            } else {
                found.map_or(-1.0, |index| index as f64)
            }
        }
        4 => {
            if search.is_empty() {
                return position as f64;
            }
            if search.len() > value.len() {
                return -1.0;
            }
            let start = position.min(value.len() - search.len());
            (0..=start)
                .rev()
                .find(|index| value.get(*index..*index + search.len()) == Some(&search))
                .map_or(-1.0, |index| index as f64)
        }
        _ => unreachable!(),
    }
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn string_index_of(value: f64, search: f64) -> f64 {
    unsafe { positioned_string_search(value, search, 0.0, 3) }
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn string_last_index_of(value: f64, search: f64) -> f64 {
    unsafe { positioned_string_search(value, search, f64::INFINITY, 4) }
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
macro_rules! positioned_searches {
    ($($function:ident, $kind:expr);+ $(;)?) => {
        $(extern "C" fn $function(value: f64, search: f64, position: f64) -> f64 {
            unsafe { positioned_string_search(value, search, position, $kind) }
        })+
    };
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
positioned_searches! {
    string_starts_with_at, 0;
    string_ends_with_at, 1;
    string_includes_at, 2;
    string_index_of_at, 3;
    string_last_index_of_at, 4;
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn string_starts_with(value: f64, search: f64) -> f64 {
    string_starts_with_at(value, search, 0.0)
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn string_ends_with(value: f64, search: f64) -> f64 {
    string_ends_with_at(value, search, f64::INFINITY)
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn string_includes(value: f64, search: f64) -> f64 {
    string_includes_at(value, search, 0.0)
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn arena_string(value: String) -> f64 {
    let size = value.len() + 1;
    let Some(output) =
        ARENA_ALLOC.with(|allocator| allocator.get().map(|alloc| unsafe { alloc(size, 1) }))
    else {
        CALL_ERROR.with(|error| error.set(ALLOCATION_FAILED.as_ptr().cast()));
        return f64::from_bits(0);
    };
    if output.is_null() {
        CALL_ERROR.with(|error| error.set(ALLOCATION_FAILED.as_ptr().cast()));
        return f64::from_bits(0);
    }
    unsafe {
        std::ptr::copy_nonoverlapping(value.as_ptr(), output, value.len());
        output.add(value.len()).write(0);
    }
    f64::from_bits(output as usize as u64)
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn number_to_string(value: f64) -> f64 {
    let Some(format) = NUMBER_TO_STRING.with(Cell::get) else {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return f64::from_bits(0);
    };
    let value = unsafe { format(value) };
    if value.is_null() {
        CALL_ERROR.with(|error| error.set(ALLOCATION_FAILED.as_ptr().cast()));
        f64::from_bits(0)
    } else {
        f64::from_bits(value as usize as u64)
    }
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn boolean_to_string(value: f64) -> f64 {
    arena_string(if value != 0.0 { "true" } else { "false" }.into())
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn string_to_lower_case(value: f64) -> f64 {
    unsafe {
        string_argument(value).map_or(f64::from_bits(0), |value| {
            arena_string(value.to_lowercase())
        })
    }
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn string_to_upper_case(value: f64) -> f64 {
    unsafe {
        string_argument(value).map_or(f64::from_bits(0), |value| {
            arena_string(value.to_uppercase())
        })
    }
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
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

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
unsafe fn trim_string(value: f64, start: bool, end: bool) -> f64 {
    let Some(value) = string_argument(value) else {
        return f64::from_bits(0);
    };
    let value = if start {
        value.trim_start_matches(is_javascript_whitespace)
    } else {
        value.as_str()
    };
    let value = if end {
        value.trim_end_matches(is_javascript_whitespace)
    } else {
        value
    };
    arena_string(value.to_owned())
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn string_trim(value: f64) -> f64 {
    unsafe { trim_string(value, true, true) }
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn string_trim_start(value: f64) -> f64 {
    unsafe { trim_string(value, true, false) }
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn string_trim_end(value: f64) -> f64 {
    unsafe { trim_string(value, false, true) }
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn string_repeat(value: f64, count: f64) -> f64 {
    unsafe {
        let Some(value) = string_argument(value) else {
            return f64::from_bits(0);
        };
        let count = if count.is_nan() || count == 0.0 {
            0
        } else if !count.is_finite() || count < 0.0 {
            CALL_ERROR.with(|error| error.set(INVALID_REPEAT_COUNT.as_ptr().cast()));
            return f64::from_bits(0);
        } else {
            count.trunc() as usize
        };
        if value.len().checked_mul(count).is_none() {
            CALL_ERROR.with(|error| error.set(INVALID_REPEAT_COUNT.as_ptr().cast()));
            return f64::from_bits(0);
        }
        arena_string(value.repeat(count))
    }
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn normalize_string_index(index: f64, length: f64, negative_from_end: bool) -> usize {
    let index = if index.is_nan() { 0.0 } else { index.trunc() };
    if negative_from_end && index < 0.0 {
        (length + index).max(0.0) as usize
    } else {
        index.clamp(0.0, length) as usize
    }
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
unsafe fn string_range(
    value: f64,
    start: f64,
    end: f64,
    negative_from_end: bool,
    swap: bool,
) -> f64 {
    let Some(value) = string_argument(value) else {
        return f64::from_bits(0);
    };
    let value = value.encode_utf16().collect::<Vec<_>>();
    let length = value.len() as f64;
    let mut start = normalize_string_index(start, length, negative_from_end);
    let mut end = normalize_string_index(end, length, negative_from_end);
    if swap && start > end {
        std::mem::swap(&mut start, &mut end);
    }
    arena_string(String::from_utf16_lossy(&value[start..end.max(start)]))
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn string_slice(value: f64, start: f64) -> f64 {
    unsafe { string_range(value, start, f64::INFINITY, true, false) }
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn string_substring(value: f64, start: f64) -> f64 {
    unsafe { string_range(value, start, f64::INFINITY, false, true) }
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn string_slice_range(value: f64, start: f64, end: f64) -> f64 {
    unsafe { string_range(value, start, end, true, false) }
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn string_substring_range(value: f64, start: f64, end: f64) -> f64 {
    unsafe { string_range(value, start, end, false, true) }
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
unsafe fn string_pad(value: f64, target_length: f64, pad: f64, at_start: bool) -> f64 {
    let (Some(value), Some(pad)) = (string_argument(value), string_argument(pad)) else {
        return f64::from_bits(0);
    };
    let units = value.encode_utf16().collect::<Vec<_>>();
    let target_length = if target_length.is_finite() && target_length > 0.0 {
        target_length as usize
    } else {
        0
    };
    let pad = pad.encode_utf16().collect::<Vec<_>>();
    if target_length <= units.len() || pad.is_empty() {
        return arena_string(value);
    }
    let needed = target_length - units.len();
    let filler = pad.into_iter().cycle().take(needed);
    let combined = if at_start {
        filler.chain(units).collect::<Vec<_>>()
    } else {
        units.into_iter().chain(filler).collect()
    };
    arena_string(String::from_utf16_lossy(&combined))
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn string_pad_start(value: f64, target_length: f64, pad: f64) -> f64 {
    unsafe { string_pad(value, target_length, pad, true) }
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn string_pad_end(value: f64, target_length: f64, pad: f64) -> f64 {
    unsafe { string_pad(value, target_length, pad, false) }
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
unsafe fn string_replace(value: f64, search: f64, replacement: f64, all: bool) -> f64 {
    let (Some(value), Some(search), Some(replacement)) = (
        string_argument(value),
        string_argument(search),
        string_argument(replacement),
    ) else {
        return f64::from_bits(0);
    };
    arena_string(if all {
        value.replace(&search, &replacement)
    } else {
        value.replacen(&search, &replacement, 1)
    })
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn string_replace_first(value: f64, search: f64, replacement: f64) -> f64 {
    unsafe { string_replace(value, search, replacement, false) }
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn string_replace_all(value: f64, search: f64, replacement: f64) -> f64 {
    unsafe { string_replace(value, search, replacement, true) }
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn string_concat(left: f64, right: f64) -> f64 {
    unsafe {
        let left = left.to_bits() as usize as *const c_char;
        let right = right.to_bits() as usize as *const c_char;
        if left.is_null() || right.is_null() {
            CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
            return f64::from_bits(0);
        }
        let left = CStr::from_ptr(left).to_bytes();
        let right = CStr::from_ptr(right).to_bytes();
        let Some(size) = left
            .len()
            .checked_add(right.len())
            .and_then(|length| length.checked_add(1))
        else {
            CALL_ERROR.with(|error| error.set(ALLOCATION_FAILED.as_ptr().cast()));
            return f64::from_bits(0);
        };
        let Some(output) =
            ARENA_ALLOC.with(|allocator| allocator.get().map(|alloc| alloc(size, 1)))
        else {
            CALL_ERROR.with(|error| error.set(ALLOCATION_FAILED.as_ptr().cast()));
            return f64::from_bits(0);
        };
        if output.is_null() {
            CALL_ERROR.with(|error| error.set(ALLOCATION_FAILED.as_ptr().cast()));
            return f64::from_bits(0);
        }
        std::ptr::copy_nonoverlapping(left.as_ptr(), output, left.len());
        std::ptr::copy_nonoverlapping(right.as_ptr(), output.add(left.len()), right.len());
        output.add(size - 1).write(0);
        f64::from_bits(output as usize as u64)
    }
}

fn intern_string(encoded: &str) -> Option<*const c_char> {
    if !encoded.len().is_multiple_of(2) {
        return None;
    }
    let bytes = (0..encoded.len())
        .step_by(2)
        .map(|index| u8::from_str_radix(&encoded[index..index + 2], 16).ok())
        .collect::<Option<Vec<_>>>()?;
    let value = CString::new(bytes).ok()?;
    let mut constants = STRING_CONSTANTS
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .unwrap();
    let value = constants.entry(encoded.to_owned()).or_insert(value);
    Some(value.as_ptr())
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn bit_and(left: f64, right: f64) -> f64 {
    (to_uint32(left) & to_uint32(right)) as i32 as f64
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn bit_or(left: f64, right: f64) -> f64 {
    (to_uint32(left) | to_uint32(right)) as i32 as f64
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn bit_xor(left: f64, right: f64) -> f64 {
    (to_uint32(left) ^ to_uint32(right)) as i32 as f64
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn shift_left(left: f64, right: f64) -> f64 {
    (to_uint32(left) << (to_uint32(right) & 31)) as i32 as f64
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn shift_right(left: f64, right: f64) -> f64 {
    ((to_uint32(left) as i32) >> (to_uint32(right) & 31)) as f64
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn shift_right_unsigned(left: f64, right: f64) -> f64 {
    (to_uint32(left) >> (to_uint32(right) & 31)) as f64
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn bit_not(value: f64) -> f64 {
    (!to_uint32(value)) as i32 as f64
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn is_odd_integer(value: f64) -> bool {
    value.is_finite()
        && value.trunc() == value
        && value.abs() < 9_007_199_254_740_992.0
        && value.abs() % 2.0 == 1.0
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn power(base: f64, exponent: f64) -> f64 {
    if exponent.is_nan() {
        return f64::NAN;
    }
    if exponent == 0.0 {
        return 1.0;
    }
    if base.is_nan() {
        return f64::NAN;
    }
    let odd = is_odd_integer(exponent);
    if base.is_infinite() {
        if base.is_sign_positive() {
            return if exponent > 0.0 { f64::INFINITY } else { 0.0 };
        }
        return if exponent > 0.0 {
            if odd {
                f64::NEG_INFINITY
            } else {
                f64::INFINITY
            }
        } else if odd {
            -0.0
        } else {
            0.0
        };
    }
    if base == 0.0 {
        return if exponent > 0.0 {
            if base.is_sign_negative() && odd {
                -0.0
            } else {
                0.0
            }
        } else if base.is_sign_negative() && odd {
            f64::NEG_INFINITY
        } else {
            f64::INFINITY
        };
    }
    if exponent.is_infinite() {
        return match base.abs().partial_cmp(&1.0) {
            Some(std::cmp::Ordering::Greater) if exponent.is_sign_positive() => f64::INFINITY,
            Some(std::cmp::Ordering::Greater) => 0.0,
            Some(std::cmp::Ordering::Less) if exponent.is_sign_positive() => 0.0,
            Some(std::cmp::Ordering::Less) => f64::INFINITY,
            _ => f64::NAN,
        };
    }
    if base < 0.0 && exponent.trunc() != exponent {
        return f64::NAN;
    }
    base.powf(exponent)
}

#[repr(C)]
pub struct ThawJitResult {
    pub value: f64,
    pub error: *const c_char,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum NumericOp {
    Add,
    Subtract,
    Multiply,
    Divide,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CompareOp {
    Less,
    LessEqual,
    Greater,
    GreaterEqual,
    Equal,
    NotEqual,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum UnaryMath {
    Acos,
    Acosh,
    Asin,
    Asinh,
    Atan,
    Atanh,
    Cbrt,
    Ceil,
    Clz32,
    Cos,
    Cosh,
    Exp,
    Expm1,
    Floor,
    Fround,
    Log,
    Log1p,
    Log2,
    Log10,
    Round,
    Sign,
    Sin,
    Sinh,
    SquareRoot,
    Tan,
    Tanh,
    Truncate,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum BitwiseOp {
    And,
    Or,
    ShiftLeft,
    ShiftRight,
    ShiftRightUnsigned,
    Xor,
}

impl BitwiseOp {
    #[cfg(all(target_arch = "x86_64", target_family = "unix"))]
    fn function(self) -> extern "C" fn(f64, f64) -> f64 {
        match self {
            Self::And => bit_and,
            Self::Or => bit_or,
            Self::ShiftLeft => shift_left,
            Self::ShiftRight => shift_right,
            Self::ShiftRightUnsigned => shift_right_unsigned,
            Self::Xor => bit_xor,
        }
    }
}

impl UnaryMath {
    #[cfg(all(target_arch = "x86_64", target_family = "unix"))]
    fn function(self) -> extern "C" fn(f64) -> f64 {
        match self {
            Self::Acos => acos_number,
            Self::Acosh => acosh_number,
            Self::Asin => asin_number,
            Self::Asinh => asinh_number,
            Self::Atan => atan_number,
            Self::Atanh => atanh_number,
            Self::Cbrt => cbrt_number,
            Self::Ceil => ceil_number,
            Self::Clz32 => clz32_number,
            Self::Cos => cos_number,
            Self::Cosh => cosh_number,
            Self::Exp => exp_number,
            Self::Expm1 => expm1_number,
            Self::Floor => floor_number,
            Self::Fround => fround_number,
            Self::Log => log_number,
            Self::Log1p => log1p_number,
            Self::Log2 => log2_number,
            Self::Log10 => log10_number,
            Self::Round => round_number,
            Self::Sign => sign_number,
            Self::Sin => sin_number,
            Self::Sinh => sinh_number,
            Self::SquareRoot => unreachable!("square root emits SSE2 directly"),
            Self::Tan => tan_number,
            Self::Tanh => tanh_number,
            Self::Truncate => truncate_number,
        }
    }
}

impl NumericOp {
    fn parse(operation: &str) -> Option<Self> {
        match operation {
            "add" => Some(Self::Add),
            "sub" => Some(Self::Subtract),
            "mul" => Some(Self::Multiply),
            "div" => Some(Self::Divide),
            _ => None,
        }
    }

    #[cfg(all(target_arch = "x86_64", target_family = "unix"))]
    fn opcode(self) -> u8 {
        match self {
            Self::Add => 0x58,
            Self::Subtract => 0x5c,
            Self::Multiply => 0x59,
            Self::Divide => 0x5e,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum NumericValue {
    Argument(u8),
    Constant(f64),
    Operation(NumericOp),
    Compare(CompareOp),
    Bitwise(BitwiseOp),
    BitNot,
    Absolute,
    Negate,
    Maximum,
    Minimum,
    Atan2,
    Hypot,
    Imul,
    StringCompare,
    StringCharAt,
    StringCharCodeAt,
    StringAt,
    StringCodePointAt,
    StringConcat,
    NumberToString,
    BooleanToString,
    StringConstant(*const c_char),
    StringEndsWith,
    StringEndsWithAt,
    StringIncludes,
    StringIncludesAt,
    StringIsWellFormed,
    StringIndexOf,
    StringIndexOfAt,
    StringLastIndexOf,
    StringLastIndexOfAt,
    StringLength,
    StringPadEnd,
    StringPadStart,
    StringStartsWith,
    StringStartsWithAt,
    StringRepeat,
    StringReplace,
    StringReplaceAll,
    StringSlice,
    StringSliceRange,
    StringSubstring,
    StringSubstringRange,
    StringToLowerCase,
    StringToWellFormed,
    StringToUpperCase,
    StringTrim,
    StringTrimEnd,
    StringTrimStart,
    Power,
    UnaryMath(UnaryMath),
    Remainder,
    Select,
    AsBoolean,
}

struct NumericProgram(Vec<NumericValue>);

impl NumericProgram {
    fn parse(symbol: &str) -> Option<Self> {
        if let Some(encoded) = symbol.strip_prefix("expr:") {
            let encoded = encoded.split_once(':')?.0;
            let values = encoded
                .split(',')
                .map(|token| match token {
                    "x" => Some(NumericValue::Argument(0)),
                    "y" => Some(NumericValue::Argument(1)),
                    "+" => Some(NumericValue::Operation(NumericOp::Add)),
                    "-" => Some(NumericValue::Operation(NumericOp::Subtract)),
                    "*" => Some(NumericValue::Operation(NumericOp::Multiply)),
                    "/" => Some(NumericValue::Operation(NumericOp::Divide)),
                    "<" => Some(NumericValue::Compare(CompareOp::Less)),
                    "<=" => Some(NumericValue::Compare(CompareOp::LessEqual)),
                    ">" => Some(NumericValue::Compare(CompareOp::Greater)),
                    ">=" => Some(NumericValue::Compare(CompareOp::GreaterEqual)),
                    "==" => Some(NumericValue::Compare(CompareOp::Equal)),
                    "!=" => Some(NumericValue::Compare(CompareOp::NotEqual)),
                    "band" => Some(NumericValue::Bitwise(BitwiseOp::And)),
                    "bor" => Some(NumericValue::Bitwise(BitwiseOp::Or)),
                    "bxor" => Some(NumericValue::Bitwise(BitwiseOp::Xor)),
                    "shl" => Some(NumericValue::Bitwise(BitwiseOp::ShiftLeft)),
                    "shr" => Some(NumericValue::Bitwise(BitwiseOp::ShiftRight)),
                    "ushr" => Some(NumericValue::Bitwise(BitwiseOp::ShiftRightUnsigned)),
                    "bnot" => Some(NumericValue::BitNot),
                    "abs" => Some(NumericValue::Absolute),
                    "neg" => Some(NumericValue::Negate),
                    "max" => Some(NumericValue::Maximum),
                    "min" => Some(NumericValue::Minimum),
                    "atan2" => Some(NumericValue::Atan2),
                    "hypot" => Some(NumericValue::Hypot),
                    "imul" => Some(NumericValue::Imul),
                    "strcmp" => Some(NumericValue::StringCompare),
                    "charat" => Some(NumericValue::StringCharAt),
                    "charcodeat" => Some(NumericValue::StringCharCodeAt),
                    "at" => Some(NumericValue::StringAt),
                    "codepointat" => Some(NumericValue::StringCodePointAt),
                    "concat" => Some(NumericValue::StringConcat),
                    "numstr" => Some(NumericValue::NumberToString),
                    "boolstr" => Some(NumericValue::BooleanToString),
                    "endswith" => Some(NumericValue::StringEndsWith),
                    "endswith2" => Some(NumericValue::StringEndsWithAt),
                    "includes" => Some(NumericValue::StringIncludes),
                    "includes2" => Some(NumericValue::StringIncludesAt),
                    "iswellformed" => Some(NumericValue::StringIsWellFormed),
                    "indexof" => Some(NumericValue::StringIndexOf),
                    "indexof2" => Some(NumericValue::StringIndexOfAt),
                    "lastindexof" => Some(NumericValue::StringLastIndexOf),
                    "lastindexof2" => Some(NumericValue::StringLastIndexOfAt),
                    "strlen" => Some(NumericValue::StringLength),
                    "padend" => Some(NumericValue::StringPadEnd),
                    "padstart" => Some(NumericValue::StringPadStart),
                    "startswith" => Some(NumericValue::StringStartsWith),
                    "startswith2" => Some(NumericValue::StringStartsWithAt),
                    "repeat" => Some(NumericValue::StringRepeat),
                    "replace" => Some(NumericValue::StringReplace),
                    "replaceall" => Some(NumericValue::StringReplaceAll),
                    "slice" => Some(NumericValue::StringSlice),
                    "slice2" => Some(NumericValue::StringSliceRange),
                    "substring" => Some(NumericValue::StringSubstring),
                    "substring2" => Some(NumericValue::StringSubstringRange),
                    "tolowercase" => Some(NumericValue::StringToLowerCase),
                    "towellformed" => Some(NumericValue::StringToWellFormed),
                    "touppercase" => Some(NumericValue::StringToUpperCase),
                    "trim" => Some(NumericValue::StringTrim),
                    "trimend" => Some(NumericValue::StringTrimEnd),
                    "trimstart" => Some(NumericValue::StringTrimStart),
                    "pow" => Some(NumericValue::Power),
                    "acos" => Some(NumericValue::UnaryMath(UnaryMath::Acos)),
                    "acosh" => Some(NumericValue::UnaryMath(UnaryMath::Acosh)),
                    "asin" => Some(NumericValue::UnaryMath(UnaryMath::Asin)),
                    "asinh" => Some(NumericValue::UnaryMath(UnaryMath::Asinh)),
                    "atan" => Some(NumericValue::UnaryMath(UnaryMath::Atan)),
                    "atanh" => Some(NumericValue::UnaryMath(UnaryMath::Atanh)),
                    "cbrt" => Some(NumericValue::UnaryMath(UnaryMath::Cbrt)),
                    "ceil" => Some(NumericValue::UnaryMath(UnaryMath::Ceil)),
                    "clz32" => Some(NumericValue::UnaryMath(UnaryMath::Clz32)),
                    "cos" => Some(NumericValue::UnaryMath(UnaryMath::Cos)),
                    "cosh" => Some(NumericValue::UnaryMath(UnaryMath::Cosh)),
                    "exp" => Some(NumericValue::UnaryMath(UnaryMath::Exp)),
                    "expm1" => Some(NumericValue::UnaryMath(UnaryMath::Expm1)),
                    "floor" => Some(NumericValue::UnaryMath(UnaryMath::Floor)),
                    "fround" => Some(NumericValue::UnaryMath(UnaryMath::Fround)),
                    "log" => Some(NumericValue::UnaryMath(UnaryMath::Log)),
                    "log1p" => Some(NumericValue::UnaryMath(UnaryMath::Log1p)),
                    "log2" => Some(NumericValue::UnaryMath(UnaryMath::Log2)),
                    "log10" => Some(NumericValue::UnaryMath(UnaryMath::Log10)),
                    "round" => Some(NumericValue::UnaryMath(UnaryMath::Round)),
                    "sign" => Some(NumericValue::UnaryMath(UnaryMath::Sign)),
                    "sin" => Some(NumericValue::UnaryMath(UnaryMath::Sin)),
                    "sinh" => Some(NumericValue::UnaryMath(UnaryMath::Sinh)),
                    "sqrt" => Some(NumericValue::UnaryMath(UnaryMath::SquareRoot)),
                    "tan" => Some(NumericValue::UnaryMath(UnaryMath::Tan)),
                    "tanh" => Some(NumericValue::UnaryMath(UnaryMath::Tanh)),
                    "trunc" => Some(NumericValue::UnaryMath(UnaryMath::Truncate)),
                    "%" => Some(NumericValue::Remainder),
                    "?" => Some(NumericValue::Select),
                    "asbool" => Some(NumericValue::AsBoolean),
                    value => value
                        .strip_prefix('a')
                        .or_else(|| value.strip_prefix('b'))
                        .or_else(|| value.strip_prefix('s'))
                        .and_then(|index| index.parse::<u8>().ok())
                        .filter(|index| *index < 16)
                        .map(NumericValue::Argument)
                        .or_else(|| {
                            value
                                .strip_prefix('t')
                                .and_then(intern_string)
                                .map(NumericValue::StringConstant)
                        })
                        .or_else(|| {
                            value
                                .strip_prefix('c')
                                .filter(|bits| bits.len() == 16)
                                .and_then(|bits| u64::from_str_radix(bits, 16).ok())
                                .map(|bits| NumericValue::Constant(f64::from_bits(bits)))
                        }),
                })
                .collect::<Option<Vec<_>>>()?;
            (!values.is_empty() && values.len() <= 128).then_some(Self(values))
        } else {
            let operation = symbol.split_once(':').map_or(symbol, |pair| pair.0);
            Some(Self(vec![
                NumericValue::Argument(0),
                NumericValue::Argument(1),
                NumericValue::Operation(NumericOp::parse(operation)?),
            ]))
        }
    }

    #[cfg(all(target_arch = "x86_64", target_family = "unix"))]
    fn machine_code(&self) -> Option<Vec<u8>> {
        let mut code = Vec::with_capacity(self.0.len() * 12 + 8);
        let mut depth = 0u8;
        for value in &self.0 {
            match value {
                NumericValue::Argument(index) => {
                    if depth == 8 {
                        return None;
                    }
                    code.extend_from_slice(&[0xf2, 0x0f, 0x10, 0x47 | (depth << 3), index * 8]);
                    depth += 1;
                }
                NumericValue::Constant(value) => {
                    if depth == 8 {
                        return None;
                    }
                    code.extend_from_slice(&[0x48, 0xb8]);
                    code.extend_from_slice(&value.to_bits().to_le_bytes());
                    code.extend_from_slice(&[0x66, 0x48, 0x0f, 0x6e, 0xc0 | (depth << 3)]);
                    depth += 1;
                }
                NumericValue::StringConstant(value) => {
                    if depth == 8 {
                        return None;
                    }
                    code.extend_from_slice(&[0x48, 0xb8]);
                    code.extend_from_slice(&(*value as usize as u64).to_le_bytes());
                    code.extend_from_slice(&[0x66, 0x48, 0x0f, 0x6e, 0xc0 | (depth << 3)]);
                    depth += 1;
                }
                NumericValue::Operation(operation) => {
                    if depth < 2 {
                        return None;
                    }
                    let right = depth - 1;
                    let left = depth - 2;
                    code.extend_from_slice(&[
                        0xf2,
                        0x0f,
                        operation.opcode(),
                        0xc0 | (left << 3) | right,
                    ]);
                    depth -= 1;
                }
                NumericValue::Compare(operation) => {
                    if depth < 2 {
                        return None;
                    }
                    let right = depth - 1;
                    let left = depth - 2;
                    emit_compare(&mut code, left, right, *operation);
                    depth -= 1;
                }
                NumericValue::Bitwise(operation) => {
                    if depth < 2 {
                        return None;
                    }
                    emit_binary_call(
                        &mut code,
                        operation.function() as *const () as u64,
                        depth - 2,
                    );
                    depth -= 1;
                }
                NumericValue::BitNot => {
                    if depth == 0 {
                        return None;
                    }
                    emit_unary_call(&mut code, bit_not as *const () as u64, depth - 1);
                }
                NumericValue::Absolute => {
                    if depth == 0 {
                        return None;
                    }
                    let value = depth - 1;
                    emit_bit_operation(&mut code, value, 0xf0);
                }
                NumericValue::Negate => {
                    if depth == 0 {
                        return None;
                    }
                    let value = depth - 1;
                    emit_bit_operation(&mut code, value, 0xf8);
                }
                NumericValue::Remainder => {
                    if depth < 2 {
                        return None;
                    }
                    emit_binary_call(&mut code, fmod as *const () as u64, depth - 2);
                    depth -= 1;
                }
                NumericValue::Minimum | NumericValue::Maximum => {
                    if depth < 2 {
                        return None;
                    }
                    let function = if matches!(value, NumericValue::Minimum) {
                        minimum
                    } else {
                        maximum
                    };
                    emit_binary_call(&mut code, function as *const () as u64, depth - 2);
                    depth -= 1;
                }
                NumericValue::Power => {
                    if depth < 2 {
                        return None;
                    }
                    emit_binary_call(&mut code, power as *const () as u64, depth - 2);
                    depth -= 1;
                }
                NumericValue::Atan2 | NumericValue::Hypot | NumericValue::Imul => {
                    if depth < 2 {
                        return None;
                    }
                    let function = match value {
                        NumericValue::Atan2 => atan2_number,
                        NumericValue::Hypot => hypot_number,
                        NumericValue::Imul => imul_number,
                        _ => unreachable!(),
                    };
                    emit_binary_call(&mut code, function as *const () as u64, depth - 2);
                    depth -= 1;
                }
                NumericValue::StringCompare => {
                    if depth < 2 {
                        return None;
                    }
                    emit_binary_call(&mut code, string_compare as *const () as u64, depth - 2);
                    depth -= 1;
                }
                NumericValue::StringCharAt
                | NumericValue::StringCharCodeAt
                | NumericValue::StringAt
                | NumericValue::StringCodePointAt => {
                    if depth < 2 {
                        return None;
                    }
                    let function = match value {
                        NumericValue::StringCharAt => string_char_at,
                        NumericValue::StringCharCodeAt => string_char_code_at,
                        NumericValue::StringAt => string_at,
                        NumericValue::StringCodePointAt => string_code_point_at,
                        _ => unreachable!(),
                    };
                    emit_binary_call(&mut code, function as *const () as u64, depth - 2);
                    depth -= 1;
                }
                NumericValue::StringConcat => {
                    if depth < 2 {
                        return None;
                    }
                    emit_binary_call(&mut code, string_concat as *const () as u64, depth - 2);
                    depth -= 1;
                }
                NumericValue::NumberToString | NumericValue::BooleanToString => {
                    if depth == 0 {
                        return None;
                    }
                    let function = if matches!(value, NumericValue::NumberToString) {
                        number_to_string
                    } else {
                        boolean_to_string
                    };
                    emit_unary_call(&mut code, function as *const () as u64, depth - 1);
                }
                NumericValue::StringStartsWith
                | NumericValue::StringEndsWith
                | NumericValue::StringIncludes
                | NumericValue::StringIndexOf
                | NumericValue::StringLastIndexOf
                | NumericValue::StringRepeat
                | NumericValue::StringSlice
                | NumericValue::StringSubstring => {
                    if depth < 2 {
                        return None;
                    }
                    let function = match value {
                        NumericValue::StringStartsWith => string_starts_with,
                        NumericValue::StringEndsWith => string_ends_with,
                        NumericValue::StringIncludes => string_includes,
                        NumericValue::StringIndexOf => string_index_of,
                        NumericValue::StringLastIndexOf => string_last_index_of,
                        NumericValue::StringRepeat => string_repeat,
                        NumericValue::StringSlice => string_slice,
                        NumericValue::StringSubstring => string_substring,
                        _ => unreachable!(),
                    };
                    emit_binary_call(&mut code, function as *const () as u64, depth - 2);
                    depth -= 1;
                }
                NumericValue::StringSliceRange
                | NumericValue::StringSubstringRange
                | NumericValue::StringPadStart
                | NumericValue::StringPadEnd
                | NumericValue::StringStartsWithAt
                | NumericValue::StringEndsWithAt
                | NumericValue::StringIncludesAt
                | NumericValue::StringIndexOfAt
                | NumericValue::StringLastIndexOfAt
                | NumericValue::StringReplace
                | NumericValue::StringReplaceAll => {
                    if depth < 3 {
                        return None;
                    }
                    let function = match value {
                        NumericValue::StringSliceRange => string_slice_range,
                        NumericValue::StringSubstringRange => string_substring_range,
                        NumericValue::StringPadStart => string_pad_start,
                        NumericValue::StringPadEnd => string_pad_end,
                        NumericValue::StringStartsWithAt => string_starts_with_at,
                        NumericValue::StringEndsWithAt => string_ends_with_at,
                        NumericValue::StringIncludesAt => string_includes_at,
                        NumericValue::StringIndexOfAt => string_index_of_at,
                        NumericValue::StringLastIndexOfAt => string_last_index_of_at,
                        NumericValue::StringReplace => string_replace_first,
                        NumericValue::StringReplaceAll => string_replace_all,
                        _ => unreachable!(),
                    };
                    emit_ternary_call(&mut code, function as *const () as u64, depth - 3);
                    depth -= 2;
                }
                NumericValue::StringToLowerCase
                | NumericValue::StringToUpperCase
                | NumericValue::StringTrim
                | NumericValue::StringTrimStart
                | NumericValue::StringTrimEnd => {
                    if depth == 0 {
                        return None;
                    }
                    let function = match value {
                        NumericValue::StringToLowerCase => string_to_lower_case,
                        NumericValue::StringToUpperCase => string_to_upper_case,
                        NumericValue::StringTrim => string_trim,
                        NumericValue::StringTrimStart => string_trim_start,
                        NumericValue::StringTrimEnd => string_trim_end,
                        _ => unreachable!(),
                    };
                    emit_unary_call(&mut code, function as *const () as u64, depth - 1);
                }
                NumericValue::StringLength => {
                    if depth == 0 {
                        return None;
                    }
                    emit_unary_call(&mut code, string_length as *const () as u64, depth - 1);
                }
                NumericValue::StringIsWellFormed => {
                    if depth == 0 {
                        return None;
                    }
                    emit_unary_call(
                        &mut code,
                        string_is_well_formed as *const () as u64,
                        depth - 1,
                    );
                }
                NumericValue::StringToWellFormed => {
                    if depth == 0 {
                        return None;
                    }
                }
                NumericValue::UnaryMath(operation) => {
                    if depth == 0 {
                        return None;
                    }
                    if *operation == UnaryMath::SquareRoot {
                        let register = depth - 1;
                        code.extend_from_slice(&[
                            0xf2,
                            0x0f,
                            0x51,
                            0xc0 | (register << 3) | register,
                        ]);
                    } else {
                        emit_unary_call(
                            &mut code,
                            operation.function() as *const () as u64,
                            depth - 1,
                        );
                    }
                }
                NumericValue::Select => {
                    if depth < 3 {
                        return None;
                    }
                    let condition = depth - 3;
                    let consequent = depth - 2;
                    let alternate = depth - 1;
                    // JavaScript ToBoolean treats NaN and both signed zeroes as false.
                    code.extend_from_slice(&[
                        0x66,
                        0x0f,
                        0x2e,
                        0xc0 | (condition << 3) | condition,
                        0x7a,
                        0x13,
                    ]);
                    code.extend_from_slice(&[
                        0x66,
                        0x48,
                        0x0f,
                        0x7e,
                        0xc0 | (condition << 3),
                        0x48,
                        0xd1,
                        0xe0,
                        0x48,
                        0x85,
                        0xc0,
                        0x74,
                        0x06,
                    ]);
                    emit_move(&mut code, condition, consequent);
                    code.extend_from_slice(&[0xeb, 0x04]);
                    emit_move(&mut code, condition, alternate);
                    depth -= 2;
                }
                NumericValue::AsBoolean => {
                    if depth == 0 {
                        return None;
                    }
                }
            }
        }
        (depth == 1).then(|| {
            code.push(0xc3);
            code
        })
    }

    fn required_args(&self) -> usize {
        self.0
            .iter()
            .filter_map(|value| match value {
                NumericValue::Argument(index) => Some(*index as usize + 1),
                _ => None,
            })
            .max()
            .unwrap_or(0)
    }
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn emit_move(code: &mut Vec<u8>, destination: u8, source: u8) {
    code.extend_from_slice(&[0x66, 0x0f, 0x28, 0xc0 | (destination << 3) | source]);
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn emit_bit_operation(code: &mut Vec<u8>, value: u8, operation: u8) {
    code.extend_from_slice(&[
        0x66,
        0x48,
        0x0f,
        0x7e,
        0xc0 | (value << 3),
        0x48,
        0x0f,
        0xba,
        operation,
        0x3f,
        0x66,
        0x48,
        0x0f,
        0x6e,
        0xc0 | (value << 3),
    ]);
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn emit_spill(code: &mut Vec<u8>, registers: u8) {
    code.extend_from_slice(&[0x48, 0x83, 0xec, 0x48]);
    // The generated function keeps its argument-array base in caller-saved
    // RDI, so every native helper call must preserve it as well as live XMMs.
    code.extend_from_slice(&[0x48, 0x89, 0x7c, 0x24, 0x40]);
    for register in 0..registers {
        code.extend_from_slice(&[0xf2, 0x0f, 0x11, 0x44 | (register << 3), 0x24, register * 8]);
    }
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn emit_restore(code: &mut Vec<u8>, registers: u8) {
    for register in 0..registers {
        code.extend_from_slice(&[0xf2, 0x0f, 0x10, 0x44 | (register << 3), 0x24, register * 8]);
    }
    code.extend_from_slice(&[0x48, 0x8b, 0x7c, 0x24, 0x40]);
    code.extend_from_slice(&[0x48, 0x83, 0xc4, 0x48]);
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn emit_unary_call(code: &mut Vec<u8>, function: u64, value: u8) {
    emit_spill(code, value);
    emit_move(code, 0, value);
    code.extend_from_slice(&[0x48, 0xb8]);
    code.extend_from_slice(&function.to_le_bytes());
    code.extend_from_slice(&[0xff, 0xd0]);
    emit_move(code, value, 0);
    emit_restore(code, value);
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn emit_binary_call(code: &mut Vec<u8>, function: u64, left: u8) {
    emit_spill(code, left);
    emit_move(code, 0, left);
    emit_move(code, 1, left + 1);
    code.extend_from_slice(&[0x48, 0xb8]);
    code.extend_from_slice(&function.to_le_bytes());
    code.extend_from_slice(&[0xff, 0xd0]);
    emit_move(code, left, 0);
    emit_restore(code, left);
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn emit_ternary_call(code: &mut Vec<u8>, function: u64, left: u8) {
    emit_spill(code, left);
    emit_move(code, 0, left);
    emit_move(code, 1, left + 1);
    emit_move(code, 2, left + 2);
    code.extend_from_slice(&[0x48, 0xb8]);
    code.extend_from_slice(&function.to_le_bytes());
    code.extend_from_slice(&[0xff, 0xd0]);
    emit_move(code, left, 0);
    emit_restore(code, left);
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn emit_compare(code: &mut Vec<u8>, left: u8, right: u8, operation: CompareOp) {
    code.extend_from_slice(&[0x66, 0x0f, 0x2e, 0xc0 | (left << 3) | right]);
    let condition = match operation {
        CompareOp::Less => 0x92,
        CompareOp::LessEqual => 0x96,
        CompareOp::Greater => 0x97,
        CompareOp::GreaterEqual => 0x93,
        CompareOp::Equal => 0x94,
        CompareOp::NotEqual => 0x95,
    };
    code.extend_from_slice(&[0x0f, condition, 0xc0]);
    match operation {
        CompareOp::Less | CompareOp::LessEqual | CompareOp::Equal => {
            code.extend_from_slice(&[0x0f, 0x9b, 0xc2, 0x20, 0xd0]);
        }
        CompareOp::NotEqual => {
            code.extend_from_slice(&[0x0f, 0x9a, 0xc2, 0x08, 0xd0]);
        }
        CompareOp::Greater | CompareOp::GreaterEqual => {}
    }
    code.extend_from_slice(&[0x0f, 0xb6, 0xc0, 0xf2, 0x0f, 0x2a, 0xc0 | (left << 3)]);
}

struct Code(*mut libc::c_void);

unsafe impl Send for Code {}
unsafe impl Sync for Code {}

impl Drop for Code {
    fn drop(&mut self) {
        unsafe { libc::munmap(self.0, page_size()) };
    }
}

fn page_size() -> usize {
    let size = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
    if size > 0 {
        size as usize
    } else {
        4096
    }
}

fn cache() -> &'static Mutex<HashMap<String, Code>> {
    static CACHE: OnceLock<Mutex<HashMap<String, Code>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn compile(symbol: &str, program: &NumericProgram) -> Result<*mut libc::c_void, *const c_char> {
    let mut cache = cache()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some(code) = cache.get(symbol) {
        return Ok(code.0);
    }
    let bytes = program
        .machine_code()
        .ok_or_else(|| INVALID_SYMBOL.as_ptr().cast())?;
    let size = page_size();
    let memory = unsafe {
        libc::mmap(
            ptr::null_mut(),
            size,
            libc::PROT_READ | libc::PROT_WRITE,
            libc::MAP_PRIVATE | libc::MAP_ANONYMOUS,
            -1,
            0,
        )
    };
    if memory == libc::MAP_FAILED {
        return Err(ALLOCATION_FAILED.as_ptr().cast());
    }
    unsafe {
        ptr::copy_nonoverlapping(bytes.as_ptr(), memory.cast(), bytes.len());
        if libc::mprotect(memory, size, libc::PROT_READ | libc::PROT_EXEC) != 0 {
            libc::munmap(memory, size);
            return Err(ALLOCATION_FAILED.as_ptr().cast());
        }
    }
    cache.insert(symbol.to_owned(), Code(memory));
    Ok(memory)
}

#[cfg(not(all(target_arch = "x86_64", target_family = "unix")))]
fn compile(_symbol: &str, _program: &NumericProgram) -> Result<*mut libc::c_void, *const c_char> {
    Err(UNSUPPORTED_TARGET.as_ptr().cast())
}

/// Compiles a validated numeric expression on first use and executes it.
///
/// # Safety
///
/// `symbol` must point to a live NUL-terminated string for this call. When
/// `arg_count` is nonzero, `args` must reference at least that many `f64`s.
/// `arena_alloc`, when supplied for a string-returning program, must return a
/// writable allocation of the requested size and alignment. `number_to_string`
/// must return an arena-backed NUL-terminated string when numeric coercion is used.
#[no_mangle]
pub unsafe extern "C" fn thaw_jit_call_f64(
    symbol: *const c_char,
    args: *const f64,
    arg_count: usize,
    arena_alloc: Option<ArenaAlloc>,
    number_to_string: Option<NumberToString>,
) -> ThawJitResult {
    let Some(symbol) = (!symbol.is_null())
        .then(|| CStr::from_ptr(symbol).to_str().ok())
        .flatten()
    else {
        return ThawJitResult {
            value: 0.0,
            error: INVALID_SYMBOL.as_ptr().cast(),
        };
    };
    let Some(program) = NumericProgram::parse(symbol) else {
        return ThawJitResult {
            value: 0.0,
            error: INVALID_SYMBOL.as_ptr().cast(),
        };
    };
    if program.required_args() > arg_count || (arg_count != 0 && args.is_null()) {
        return ThawJitResult {
            value: 0.0,
            error: INVALID_SYMBOL.as_ptr().cast(),
        };
    }
    let code = match compile(symbol, &program) {
        Ok(code) => code,
        Err(error) => return ThawJitResult { value: 0.0, error },
    };
    let function = std::mem::transmute::<*mut libc::c_void, extern "C" fn(*const f64) -> f64>(code);
    let previous_allocator = ARENA_ALLOC.with(|allocator| allocator.replace(arena_alloc));
    let previous_formatter = NUMBER_TO_STRING.with(|formatter| formatter.replace(number_to_string));
    let previous_error = CALL_ERROR.with(|error| error.replace(ptr::null()));
    let previous_present = CALL_PRESENT.with(|present| present.replace(true));
    let value = function(args);
    let error = CALL_ERROR.with(|error| error.replace(previous_error));
    let present = CALL_PRESENT.with(|state| state.replace(previous_present));
    ARENA_ALLOC.with(|allocator| allocator.set(previous_allocator));
    NUMBER_TO_STRING.with(|formatter| formatter.set(previous_formatter));
    if !error.is_null() {
        return ThawJitResult { value: 0.0, error };
    }
    if !present {
        return ThawJitResult {
            value: 0.0,
            error: ABSENT_STATUS,
        };
    }
    ThawJitResult {
        value,
        error: ptr::null(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::CString;

    fn call(symbol: &CString, args: &[f64]) -> ThawJitResult {
        unsafe extern "C" fn allocate(size: usize, _: usize) -> *mut u8 {
            unsafe { libc::malloc(size).cast() }
        }
        unsafe extern "C" fn format_number(_: f64) -> *const c_char {
            c"42".as_ptr()
        }
        unsafe {
            thaw_jit_call_f64(
                symbol.as_ptr(),
                args.as_ptr(),
                args.len(),
                Some(allocate),
                Some(format_number),
            )
        }
    }

    #[test]
    fn specializes_numeric_operations_and_reuses_code() {
        for (symbol, expected) in [
            ("add:test", 42.0),
            ("sub:test", 0.0),
            ("mul:test", 441.0),
            ("div:test", 1.0),
        ] {
            let symbol = CString::new(symbol).unwrap();
            let result = call(&symbol, &[21.0, 21.0]);
            assert!(result.error.is_null());
            assert_eq!(result.value, expected);
            let repeated = call(&symbol, &[20.0, 22.0]);
            assert!(repeated.error.is_null());
        }

        let symbol = CString::new("expr:x,y,+,c4000000000000000,*:compound").unwrap();
        let result = call(&symbol, &[19.0, 2.0]);
        assert!(result.error.is_null());
        assert_eq!(result.value, 42.0);

        let symbol = CString::new("expr:a0,a1,+,a2,+:three_args").unwrap();
        let result = call(&symbol, &[10.0, 11.0, 21.0]);
        assert!(result.error.is_null());
        assert_eq!(result.value, 42.0);
        assert!(!call(&symbol, &[10.0, 11.0]).error.is_null());

        let symbol =
            CString::new("expr:x,y,<,c4045000000000000,cc000000000000000,?:conditional").unwrap();
        let comparison = CString::new("expr:x,y,<:comparison").unwrap();
        assert_eq!(call(&comparison, &[2.0, 3.0]).value, 1.0);
        assert_eq!(call(&symbol, &[2.0, 3.0]).value, 42.0);
        assert_eq!(call(&symbol, &[3.0, 2.0]).value, -2.0);

        for (operator, expected) in [("<", 0.0), ("==", 0.0), ("!=", 1.0)] {
            let symbol = CString::new(format!("expr:x,y,{operator}:nan")).unwrap();
            let result = call(&symbol, &[f64::NAN, 1.0]);
            assert!(result.error.is_null());
            assert_eq!(result.value, expected);
        }

        let symbol =
            CString::new("expr:x,c4045000000000000,cc000000000000000,?:truthiness").unwrap();
        assert_eq!(call(&symbol, &[-0.0]).value, -2.0);
        assert_eq!(call(&symbol, &[f64::NAN]).value, -2.0);

        let symbol = CString::new("expr:a0,neg:negate").unwrap();
        assert_eq!(call(&symbol, &[42.0]).value, -42.0);
        assert_eq!(call(&symbol, &[-0.0]).value.to_bits(), 0.0f64.to_bits());

        let symbol = CString::new("expr:c4045000000000000:constant").unwrap();
        assert_eq!(call(&symbol, &[]).value, 42.0);

        let symbol = CString::new("expr:a0,a1,%:remainder").unwrap();
        assert_eq!(call(&symbol, &[5.5, 2.0]).value, 1.5);
        assert_eq!(call(&symbol, &[-5.5, 2.0]).value, -1.5);
        assert!(call(&symbol, &[f64::INFINITY, 2.0]).value.is_nan());

        let symbol = CString::new("expr:a0,abs:absolute").unwrap();
        assert_eq!(call(&symbol, &[-42.0]).value, 42.0);
        assert_eq!(call(&symbol, &[-0.0]).value.to_bits(), 0.0f64.to_bits());
        assert!(call(&symbol, &[f64::NAN]).value.is_nan());

        let minimum = CString::new("expr:a0,a1,min:minimum").unwrap();
        let maximum = CString::new("expr:a0,a1,max:maximum").unwrap();
        assert_eq!(call(&minimum, &[42.0, 43.0]).value, 42.0);
        assert_eq!(call(&maximum, &[41.0, 42.0]).value, 42.0);
        assert!(call(&minimum, &[f64::NAN, 42.0]).value.is_nan());
        assert!(call(&maximum, &[42.0, f64::NAN]).value.is_nan());
        assert_eq!(
            call(&minimum, &[0.0, -0.0]).value.to_bits(),
            (-0.0f64).to_bits()
        );
        assert_eq!(
            call(&maximum, &[-0.0, 0.0]).value.to_bits(),
            0.0f64.to_bits()
        );

        for (operation, input, expected) in [
            ("floor", -1.2, -2.0),
            ("ceil", -1.2, -1.0),
            ("trunc", -1.2, -1.0),
            ("round", -1.5, -1.0),
            ("sqrt", 9.0, 3.0),
        ] {
            let symbol = CString::new(format!("expr:a0,{operation}:{operation}")).unwrap();
            assert_eq!(call(&symbol, &[input]).value, expected);
        }
        let round = CString::new("expr:a0,round:round_zero").unwrap();
        assert_eq!(call(&round, &[-0.5]).value.to_bits(), (-0.0f64).to_bits());
        let square_root = CString::new("expr:a0,sqrt:sqrt_nan").unwrap();
        assert!(call(&square_root, &[-1.0]).value.is_nan());

        for (operation, input, expected) in [
            ("acos", 1.0, 0.0),
            ("acosh", 1.0, 0.0),
            ("asin", 0.0, 0.0),
            ("asinh", 0.0, 0.0),
            ("atan", 0.0, 0.0),
            ("atanh", 0.0, 0.0),
            ("cbrt", 8.0, 2.0),
            ("cos", 0.0, 1.0),
            ("cosh", 0.0, 1.0),
            ("exp", 0.0, 1.0),
            ("expm1", 0.0, 0.0),
            ("log", 1.0, 0.0),
            ("log1p", 0.0, 0.0),
            ("log2", 8.0, 3.0),
            ("log10", 100.0, 2.0),
            ("sign", -8.0, -1.0),
            ("sin", 0.0, 0.0),
            ("sinh", 0.0, 0.0),
            ("tan", 0.0, 0.0),
            ("tanh", 0.0, 0.0),
        ] {
            let symbol = CString::new(format!("expr:a0,{operation}:{operation}")).unwrap();
            let result = call(&symbol, &[input]);
            assert!(result.error.is_null());
            assert!((result.value - expected).abs() < 1e-12);
        }
        let sign = CString::new("expr:a0,sign:sign_special").unwrap();
        assert_eq!(call(&sign, &[-0.0]).value.to_bits(), (-0.0f64).to_bits());
        assert!(call(&sign, &[f64::NAN]).value.is_nan());
        let atan2 = CString::new("expr:a0,a1,atan2:atan2").unwrap();
        assert!((call(&atan2, &[1.0, 1.0]).value - std::f64::consts::FRAC_PI_4).abs() < 1e-12);
        let hypot = CString::new("expr:a0,a1,hypot:hypot").unwrap();
        assert_eq!(call(&hypot, &[3.0, 4.0]).value, 5.0);
        let nested_unary = CString::new("expr:a0,a1,sin,+:nested_unary").unwrap();
        assert!((call(&nested_unary, &[2.0, 1.0]).value - (2.0 + 1.0f64.sin())).abs() < 1e-12);
        let nested_binary = CString::new("expr:a0,a1,a2,hypot,+:nested_binary").unwrap();
        assert_eq!(call(&nested_binary, &[7.0, 3.0, 4.0]).value, 12.0);
        let clz32 = CString::new("expr:a0,clz32:clz32").unwrap();
        assert_eq!(call(&clz32, &[0.0]).value, 32.0);
        assert_eq!(call(&clz32, &[1.0]).value, 31.0);
        assert_eq!(call(&clz32, &[-1.0]).value, 0.0);
        let fround = CString::new("expr:a0,fround:fround").unwrap();
        assert_eq!(call(&fround, &[1.337]).value, 1.337f64 as f32 as f64);
        assert_eq!(call(&fround, &[-0.0]).value.to_bits(), (-0.0f64).to_bits());
        let imul = CString::new("expr:a0,a1,imul:imul").unwrap();
        assert_eq!(call(&imul, &[4_294_967_295.0, 5.0]).value, -5.0);
        let text = CString::new("a😀").unwrap();
        let text_argument = f64::from_bits(text.as_ptr() as usize as u64);
        let string_length = CString::new("expr:s0,strlen:string_length").unwrap();
        assert_eq!(call(&string_length, &[text_argument]).value, 3.0);
        for (operation, index, expected) in [
            ("charcodeat", 0.0, 97.0),
            ("charcodeat", 1.0, 55_357.0),
            ("charcodeat", 2.0, 56_832.0),
            ("charcodeat", -1.0, f64::NAN),
        ] {
            let character = CString::new(format!("expr:s0,a1,{operation}:{operation}")).unwrap();
            let actual = call(&character, &[text_argument, index]).value;
            if expected.is_nan() {
                assert!(actual.is_nan());
            } else {
                assert_eq!(actual, expected);
            }
        }
        for (index, expected) in [(0.0, "a"), (1.0, "�"), (-1.0, "")] {
            let character = CString::new("expr:s0,a1,charat:charat").unwrap();
            let result = call(&character, &[text_argument, index]);
            let result = result.value.to_bits() as usize as *mut c_char;
            assert_eq!(
                unsafe { CStr::from_ptr(result) }.to_str().unwrap(),
                expected
            );
            unsafe { libc::free(result.cast()) };
        }
        let at = CString::new("expr:s0,a1,at:at").unwrap();
        let result = call(&at, &[text_argument, -1.0]);
        assert!(result.error.is_null());
        let value = result.value.to_bits() as usize as *mut c_char;
        assert_eq!(unsafe { CStr::from_ptr(value) }.to_str().unwrap(), "�");
        unsafe { libc::free(value.cast()) };
        assert_eq!(call(&at, &[text_argument, 3.0]).error, ABSENT_STATUS);
        let code_point = CString::new("expr:s0,a1,codepointat:codepointat").unwrap();
        let result = call(&code_point, &[text_argument, 1.0]);
        assert!(result.error.is_null());
        assert_eq!(result.value, 128_512.0);
        assert_eq!(
            call(&code_point, &[text_argument, 3.0]).error,
            ABSENT_STATUS
        );
        let supplementary = CString::new("𐀀").unwrap();
        let bmp = CString::new("\u{e000}").unwrap();
        let string_less =
            CString::new("expr:s0,s1,strcmp,c0000000000000000,<:string_utf16_comparison").unwrap();
        assert_eq!(
            call(
                &string_less,
                &[
                    f64::from_bits(supplementary.as_ptr() as usize as u64),
                    f64::from_bits(bmp.as_ptr() as usize as u64),
                ],
            )
            .value,
            1.0
        );
        let value = CString::new("prefix").unwrap();
        for (operation, search, expected) in [
            ("startswith", "pre", 1.0),
            ("endswith", "fix", 1.0),
            ("includes", "ref", 1.0),
            ("includes", "xyz", 0.0),
        ] {
            let search = CString::new(search).unwrap();
            let predicate = CString::new(format!("expr:s0,s1,{operation}:{operation}")).unwrap();
            assert_eq!(
                call(
                    &predicate,
                    &[
                        f64::from_bits(value.as_ptr() as usize as u64),
                        f64::from_bits(search.as_ptr() as usize as u64),
                    ],
                )
                .value,
                expected
            );
        }
        let positioned = CString::new("😀abc😀").unwrap();
        for (operation, search, position, expected) in [
            ("startswith2", "abc", 2.0, 1.0),
            ("endswith2", "😀", 7.0, 1.0),
            ("includes2", "😀", 1.0, 1.0),
            ("indexof2", "😀", 1.0, 5.0),
            ("lastindexof2", "😀", 4.0, 0.0),
            ("indexof2", "", f64::INFINITY, 7.0),
        ] {
            let search = CString::new(search).unwrap();
            let search_at = CString::new(format!("expr:s0,s1,a2,{operation}:{operation}")).unwrap();
            assert_eq!(
                call(
                    &search_at,
                    &[
                        f64::from_bits(positioned.as_ptr() as usize as u64),
                        f64::from_bits(search.as_ptr() as usize as u64),
                        position,
                    ],
                )
                .value,
                expected
            );
        }
        let indexed = CString::new("😀a😀").unwrap();
        let emoji = CString::new("😀").unwrap();
        for (operation, expected) in [("indexof", 0.0), ("lastindexof", 3.0)] {
            let index = CString::new(format!("expr:s0,s1,{operation}:{operation}")).unwrap();
            assert_eq!(
                call(
                    &index,
                    &[
                        f64::from_bits(indexed.as_ptr() as usize as u64),
                        f64::from_bits(emoji.as_ptr() as usize as u64),
                    ],
                )
                .value,
                expected
            );
        }
        for (operation, expected) in [("tolowercase", "straße"), ("touppercase", "STRASSE")] {
            let input = CString::new("Straße").unwrap();
            let convert = CString::new(format!("expr:s0,{operation}:{operation}")).unwrap();
            let result = call(&convert, &[f64::from_bits(input.as_ptr() as usize as u64)]);
            let result = result.value.to_bits() as usize as *mut c_char;
            assert_eq!(
                unsafe { CStr::from_ptr(result) }.to_str().unwrap(),
                expected
            );
            unsafe { libc::free(result.cast()) };
        }
        let well_formed = CString::new("expr:s0,iswellformed:iswellformed").unwrap();
        assert_eq!(call(&well_formed, &[text_argument]).value, 1.0);
        let preserve = CString::new("expr:s0,towellformed:towellformed").unwrap();
        assert_eq!(call(&preserve, &[text_argument]).value, text_argument);
        let whitespace = CString::new("\u{feff}  value\u{3000}").unwrap();
        for (operation, expected) in [
            ("trim", "value"),
            ("trimstart", "value\u{3000}"),
            ("trimend", "\u{feff}  value"),
        ] {
            let trim = CString::new(format!("expr:s0,{operation}:{operation}")).unwrap();
            let result = call(
                &trim,
                &[f64::from_bits(whitespace.as_ptr() as usize as u64)],
            );
            let result = result.value.to_bits() as usize as *mut c_char;
            assert_eq!(
                unsafe { CStr::from_ptr(result) }.to_str().unwrap(),
                expected
            );
            unsafe { libc::free(result.cast()) };
        }
        let repeated = CString::new("ab").unwrap();
        let repeat = CString::new("expr:s0,a1,repeat:repeat").unwrap();
        let result = call(
            &repeat,
            &[f64::from_bits(repeated.as_ptr() as usize as u64), 2.9],
        );
        let result = result.value.to_bits() as usize as *mut c_char;
        assert_eq!(unsafe { CStr::from_ptr(result) }.to_str().unwrap(), "abab");
        unsafe { libc::free(result.cast()) };
        assert!(!call(
            &repeat,
            &[f64::from_bits(repeated.as_ptr() as usize as u64), -1.0],
        )
        .error
        .is_null());
        let sliced = CString::new("😀abcd").unwrap();
        for (operation, start, expected) in [
            ("slice", -2.0, "cd"),
            ("slice", 2.0, "abcd"),
            ("substring", -2.0, "😀abcd"),
            ("substring", 4.9, "cd"),
        ] {
            let suffix = CString::new(format!("expr:s0,a1,{operation}:{operation}")).unwrap();
            let result = call(
                &suffix,
                &[f64::from_bits(sliced.as_ptr() as usize as u64), start],
            );
            let result = result.value.to_bits() as usize as *mut c_char;
            assert_eq!(
                unsafe { CStr::from_ptr(result) }.to_str().unwrap(),
                expected
            );
            unsafe { libc::free(result.cast()) };
        }
        for (operation, start, end, expected) in [
            ("slice2", -4.0, -1.0, "abc"),
            ("slice2", 5.0, 2.0, ""),
            ("substring2", 5.0, 2.0, "abc"),
            ("substring2", f64::NAN, 2.9, "😀"),
        ] {
            let range = CString::new(format!("expr:s0,a1,a2,{operation}:{operation}")).unwrap();
            let result = call(
                &range,
                &[f64::from_bits(sliced.as_ptr() as usize as u64), start, end],
            );
            let result = result.value.to_bits() as usize as *mut c_char;
            assert_eq!(
                unsafe { CStr::from_ptr(result) }.to_str().unwrap(),
                expected
            );
            unsafe { libc::free(result.cast()) };
        }
        let padded = CString::new("😀").unwrap();
        let padding = CString::new("ab").unwrap();
        for (operation, expected) in [("padstart", "a😀"), ("padend", "😀a")] {
            let pad = CString::new(format!("expr:s0,a1,s2,{operation}:{operation}")).unwrap();
            let result = call(
                &pad,
                &[
                    f64::from_bits(padded.as_ptr() as usize as u64),
                    3.0,
                    f64::from_bits(padding.as_ptr() as usize as u64),
                ],
            );
            let result = result.value.to_bits() as usize as *mut c_char;
            assert_eq!(
                unsafe { CStr::from_ptr(result) }.to_str().unwrap(),
                expected
            );
            unsafe { libc::free(result.cast()) };
        }
        let replace_value = CString::new("aba").unwrap();
        for (operation, search, replacement, expected) in [
            ("replace", "a", "x", "xba"),
            ("replaceall", "a", "x", "xbx"),
            ("replaceall", "", "-", "-a-b-a-"),
        ] {
            let search = CString::new(search).unwrap();
            let replacement = CString::new(replacement).unwrap();
            let replace = CString::new(format!("expr:s0,s1,s2,{operation}:{operation}")).unwrap();
            let result = call(
                &replace,
                &[
                    f64::from_bits(replace_value.as_ptr() as usize as u64),
                    f64::from_bits(search.as_ptr() as usize as u64),
                    f64::from_bits(replacement.as_ptr() as usize as u64),
                ],
            );
            let result = result.value.to_bits() as usize as *mut c_char;
            assert_eq!(
                unsafe { CStr::from_ptr(result) }.to_str().unwrap(),
                expected
            );
            unsafe { libc::free(result.cast()) };
        }
        let combined = CString::new("expr:s0,t707265,startswith,s0,t666978,endswith,s0,t707265,startswith,?,s0,t726566,includes,s0,t707265,startswith,s0,t666978,endswith,s0,t707265,startswith,?,?:combined_string_predicates").unwrap();
        assert_eq!(
            call(&combined, &[f64::from_bits(value.as_ptr() as usize as u64)]).value,
            1.0
        );
        let name = CString::new("世界").unwrap();
        let concatenate = CString::new("expr:t686920,s0,concat:concatenate").unwrap();
        let result = call(
            &concatenate,
            &[f64::from_bits(name.as_ptr() as usize as u64)],
        );
        assert!(result.error.is_null());
        let result = result.value.to_bits() as usize as *mut c_char;
        assert_eq!(
            unsafe { CStr::from_ptr(result) }.to_str().unwrap(),
            "hi 世界"
        );
        unsafe { libc::free(result.cast()) };
        let argument = [f64::from_bits(name.as_ptr() as usize as u64)];
        let missing_allocator =
            unsafe { thaw_jit_call_f64(concatenate.as_ptr(), argument.as_ptr(), 1, None, None) };
        assert!(!missing_allocator.error.is_null());

        for (expression, args, expected) in [
            ("a0,a1,bor", [4_294_967_297.0, 0.0], 1.0),
            ("a0,a1,band", [f64::NAN, 1.0], 0.0),
            ("a0,a1,bxor", [43.0, 1.0], 42.0),
            ("a0,a1,shl", [1.0, 33.0], 2.0),
            ("a0,a1,shr", [-4.0, 1.0], -2.0),
            ("a0,a1,ushr", [-1.0, 0.0], 4_294_967_295.0),
        ] {
            let symbol = CString::new(format!("expr:{expression}:bitwise")).unwrap();
            assert_eq!(call(&symbol, &args).value, expected);
        }
        let bit_not = CString::new("expr:a0,bnot:bit_not").unwrap();
        assert_eq!(call(&bit_not, &[0.0]).value, -1.0);

        let power = CString::new("expr:a0,a1,pow:power").unwrap();
        assert_eq!(call(&power, &[2.0, 5.0]).value, 32.0);
        assert_eq!(call(&power, &[f64::NAN, 0.0]).value, 1.0);
        assert!(call(&power, &[1.0, f64::INFINITY]).value.is_nan());
        assert!(call(&power, &[-2.0, 0.5]).value.is_nan());
        assert_eq!(
            call(&power, &[-0.0, 3.0]).value.to_bits(),
            (-0.0f64).to_bits()
        );
        assert_eq!(call(&power, &[-0.0, -3.0]).value, f64::NEG_INFINITY);
    }

    #[test]
    fn converts_primitives_to_arena_strings() {
        for (symbol, argument, expected) in [
            ("expr:a0,numstr:number-string", 42.0, "42"),
            ("expr:b0,boolstr:boolean-string", 1.0, "true"),
            ("expr:b0,boolstr:false-string", 0.0, "false"),
        ] {
            let symbol = CString::new(symbol).unwrap();
            let result = call(&symbol, &[argument]);
            assert!(result.error.is_null());
            assert_eq!(
                unsafe { CStr::from_ptr(result.value.to_bits() as usize as *const c_char) }
                    .to_str()
                    .unwrap(),
                expected
            );
        }
    }
}
