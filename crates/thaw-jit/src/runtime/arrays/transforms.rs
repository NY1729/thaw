#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn number_array_index_map(value: f64, encoded: f64) -> f64 {
    let encoded = encoded as u8;
    let reverse = encoded >= 8;
    let operation = encoded % 8;
    number_array_map(value, |element, index| {
        let (left, right) = if reverse {
            (index, element)
        } else {
            (element, index)
        };
        match operation {
            0 => left + right,
            1 => left - right,
            2 => left * right,
            3 => left / right,
            4 => left % right,
            5 => power(left, right),
            _ => {
                CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
                0.0
            }
        }
    })
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn number_array_select_map(value: f64, operand: f64, encoded: f64) -> f64 {
    let encoded = encoded as u8;
    let operation = encoded % 8;
    let mode = encoded / 8;
    if operation > 5 || mode > 3 {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return 0.0;
    }
    number_array_map(value, |element, _| {
        let (left, right) = if mode & 1 == 0 {
            (element, operand)
        } else {
            (operand, element)
        };
        let condition = match operation {
            0 => left < right,
            1 => left <= right,
            2 => left > right,
            3 => left >= right,
            4 => left == right,
            5 => left != right,
            _ => unreachable!(),
        };
        if condition == (mode & 2 == 0) {
            element
        } else {
            operand
        }
    })
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn number_array_branch_map(value: f64, operand: f64, encoded: f64) -> f64 {
    let encoded = encoded as u16;
    let operation = (encoded & 7) as u8;
    let reverse = encoded & 8 != 0;
    let true_branch = ((encoded >> 4) & 15) as u8;
    let false_branch = ((encoded >> 8) & 15) as u8;
    if operation > 5 || true_branch > 13 || false_branch > 13 {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return 0.0;
    }
    let branch = |element: f64, mode: u8| {
        let (operation, left, right) = if mode < 8 {
            (mode.saturating_sub(2), element, operand)
        } else {
            (mode - 8, operand, element)
        };
        match mode {
            0 => element,
            1 => operand,
            _ => match operation {
                0 => left + right,
                1 => left - right,
                2 => left * right,
                3 => left / right,
                4 => left % right,
                5 => power(left, right),
                _ => unreachable!(),
            },
        }
    };
    number_array_map(value, |element, _| {
        let (left, right) = if reverse {
            (operand, element)
        } else {
            (element, operand)
        };
        let condition = match operation {
            0 => left < right,
            1 => left <= right,
            2 => left > right,
            3 => left >= right,
            4 => left == right,
            5 => left != right,
            _ => unreachable!(),
        };
        branch(element, if condition { true_branch } else { false_branch })
    })
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
macro_rules! number_array_maps {
    ($forward:ident, $reverse:ident, $operation:expr) => {
        extern "C" fn $forward(value: f64, operand: f64) -> f64 {
            number_array_map(value, |element, _| $operation(element, operand))
        }
        extern "C" fn $reverse(value: f64, operand: f64) -> f64 {
            number_array_map(value, |element, _| $operation(operand, element))
        }
    };
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
number_array_maps!(
    number_array_map_add,
    number_array_map_add_reverse,
    |left, right| left + right
);
#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
number_array_maps!(
    number_array_map_subtract,
    number_array_map_subtract_reverse,
    |left, right| left - right
);
#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
number_array_maps!(
    number_array_map_multiply,
    number_array_map_multiply_reverse,
    |left, right| left * right
);
#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
number_array_maps!(
    number_array_map_divide,
    number_array_map_divide_reverse,
    |left, right| left / right
);
#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
number_array_maps!(
    number_array_map_remainder,
    number_array_map_remainder_reverse,
    |left, right| left % right
);
#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
number_array_maps!(
    number_array_map_power,
    number_array_map_power_reverse,
    |left, right| power(left, right)
);

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn number_array_map_negate(value: f64) -> f64 {
    number_array_map(value, |element, _| -element)
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn number_array_map_absolute(value: f64) -> f64 {
    number_array_map(value, |element, _| element.abs())
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn square_root_number(value: f64) -> f64 {
    value.sqrt()
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn number_array_map_math(value: f64, operation: f64) -> f64 {
    let function = match operation as u8 {
        0 => acos_number,
        1 => acosh_number,
        2 => asin_number,
        3 => asinh_number,
        4 => atan_number,
        5 => atanh_number,
        6 => cbrt_number,
        7 => ceil_number,
        8 => clz32_number,
        9 => cos_number,
        10 => cosh_number,
        11 => exp_number,
        12 => expm1_number,
        13 => floor_number,
        14 => fround_number,
        15 => log_number,
        16 => log1p_number,
        17 => log2_number,
        18 => log10_number,
        19 => round_number,
        20 => sign_number,
        21 => sin_number,
        22 => sinh_number,
        23 => square_root_number,
        24 => tan_number,
        25 => tanh_number,
        26 => truncate_number,
        _ => {
            CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
            return 0.0;
        }
    };
    number_array_map(value, |element, _| function(element))
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
extern "C" fn number_is_nan(value: f64) -> f64 {
    f64::from(u8::from(value.is_nan()))
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn number_is_finite(value: f64) -> f64 {
    f64::from(u8::from(value.is_finite()))
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn number_is_integer(value: f64) -> f64 {
    f64::from(u8::from(value.is_finite() && value.fract() == 0.0))
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn number_is_safe_integer(value: f64) -> f64 {
    f64::from(u8::from(
        value.is_finite() && value.fract() == 0.0 && value.abs() <= 9_007_199_254_740_991.0,
    ))
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
unsafe fn array_data(value: f64) -> Option<(*const u8, usize)> {
    let bits = value.to_bits();
    if bits & ARRAY_RESULT_TAG != 0 {
        let data = (bits & !ARRAY_RESULT_TAG) as usize as *const u8;
        return (!data.is_null())
            .then(|| (data, unsafe { data.cast::<i64>().read() }.max(0) as usize));
    }
    let handle = bits as usize as *const *const u8;
    if handle.is_null() {
        return None;
    }
    let data = unsafe { handle.read() };
    if data.is_null() {
        return None;
    }
    Some((data, unsafe { data.cast::<i64>().read() }.max(0) as usize))
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn array_result(pointer: *mut u8) -> f64 {
    f64::from_bits(pointer as usize as u64 | ARRAY_RESULT_TAG)
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn empty_array() -> f64 {
    let Some(allocate) = ARENA_ALLOC.with(Cell::get) else {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return 0.0;
    };
    let array = unsafe { allocate(8, 8) };
    if array.is_null() {
        CALL_ERROR.with(|error| error.set(ALLOCATION_FAILED.as_ptr().cast()));
        return 0.0;
    }
    unsafe { array.cast::<u64>().write(0) };
    array_result(array)
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn array_length(value: f64) -> f64 {
    unsafe { array_data(value) }.map_or(0.0, |(_, length)| length as f64)
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn array_value(value: f64) -> f64 {
    match unsafe { array_data(value) } {
        Some((data, _)) => f64::from_bits(data as usize as u64),
        None => {
            CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
            0.0
        }
    }
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn mutable_array_handle(value: f64) -> f64 {
    if value.to_bits() & ARRAY_RESULT_TAG == 0 {
        return value;
    }
    let (Some(allocate), Some((data, _))) =
        (ARENA_ALLOC.with(Cell::get), unsafe { array_data(value) })
    else {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return 0.0;
    };
    let handle = unsafe {
        allocate(
            std::mem::size_of::<*mut u8>(),
            std::mem::align_of::<*mut u8>(),
        )
    };
    if handle.is_null() {
        CALL_ERROR.with(|error| error.set(ALLOCATION_FAILED.as_ptr().cast()));
        return 0.0;
    }
    unsafe { handle.cast::<*const u8>().write(data) };
    f64::from_bits(handle as usize as u64)
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn string_to_array(value: f64) -> f64 {
    let Some(convert) = STRING_TO_ARRAY.with(Cell::get) else {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return 0.0;
    };
    let result = unsafe { convert(value.to_bits() as usize as *const c_char) };
    if result.is_null() {
        CALL_ERROR.with(|error| error.set(ALLOCATION_FAILED.as_ptr().cast()));
        0.0
    } else {
        array_result(result)
    }
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn string_from_char_code(value: f64) -> f64 {
    let Some(convert) = STRING_FROM_CHAR_CODE.with(Cell::get) else {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return 0.0;
    };
    let result = unsafe { convert(value) };
    if result.is_null() {
        CALL_ERROR.with(|error| error.set(ALLOCATION_FAILED.as_ptr().cast()));
        0.0
    } else {
        f64::from_bits(result as usize as u64)
    }
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn string_from_code_point(value: f64) -> f64 {
    let Some(convert) = STRING_FROM_CODE_POINT.with(Cell::get) else {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return 0.0;
    };
    let result = unsafe { convert(value) };
    if result.is_null() {
        CALL_ERROR.with(|error| error.set(INVALID_CODE_POINT.as_ptr().cast()));
        0.0
    } else {
        f64::from_bits(result as usize as u64)
    }
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn is_array(_: f64) -> f64 {
    1.0
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn is_not_array(_: f64) -> f64 {
    0.0
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn absent_value() -> f64 {
    CALL_ABSENCE.with(|absence| absence.set(1));
    CALL_PRESENT.with(|present| present.set(false));
    0.0
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn null_value() -> f64 {
    CALL_ABSENCE.with(|absence| absence.set(2));
    CALL_PRESENT.with(|present| present.set(false));
    0.0
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn preserve_absent_value() -> f64 {
    CALL_PRESENT.with(|present| present.set(false));
    0.0
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn missing_callable() -> f64 {
    CALL_ERROR.with(|error| error.set(VALUE_NOT_CALLABLE.as_ptr().cast()));
    0.0
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn take_present() -> f64 {
    f64::from(CALL_PRESENT.with(|present| present.replace(true)))
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
unsafe fn array_at(value: f64, index: f64, kind: u8) -> f64 {
    let Some((data, length)) = (unsafe { array_data(value) }) else {
        CALL_PRESENT.with(|present| present.set(false));
        return 0.0;
    };
    let index = if index.is_nan() { 0.0 } else { index.trunc() };
    let index = if index < 0.0 {
        length as f64 + index
    } else {
        index
    };
    if !index.is_finite() || index < 0.0 || index >= length as f64 {
        CALL_PRESENT.with(|present| present.set(false));
        return 0.0;
    }
    unsafe { array_element(data, index as usize, kind) }
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
unsafe fn array_element(data: *const u8, index: usize, kind: u8) -> f64 {
    let slot = unsafe { data.add(8 + index * 8) };
    match kind {
        0 => unsafe { slot.cast::<f64>().read_unaligned() },
        1 => f64::from(unsafe { slot.read() } != 0),
        2 => f64::from_bits(unsafe { slot.cast::<usize>().read_unaligned() } as u64),
        _ => 0.0,
    }
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
unsafe fn array_get(value: f64, index: f64, kind: u8) -> f64 {
    let Some((data, length)) = (unsafe { array_data(value) }) else {
        CALL_PRESENT.with(|present| present.set(false));
        return 0.0;
    };
    if !index.is_finite() || index < 0.0 || index.fract() != 0.0 || index >= length as f64 {
        CALL_PRESENT.with(|present| present.set(false));
        return 0.0;
    }
    unsafe { array_element(data, index as usize, kind) }
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn number_array_at(value: f64, index: f64) -> f64 {
    unsafe { array_at(value, index, 0) }
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn bool_array_at(value: f64, index: f64) -> f64 {
    unsafe { array_at(value, index, 1) }
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn string_array_at(value: f64, index: f64) -> f64 {
    unsafe { array_at(value, index, 2) }
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn dynamic_array_at(value: f64, index: f64) -> f64 {
    let Some(dynamic) = dynamic_primitive(value, None) else {
        CALL_ERROR.with(|error| error.set(INVALID_DYNAMIC_VALUE.as_ptr().cast()));
        return 0.0;
    };
    let (kind, tag) = match dynamic.tag {
        DYNAMIC_NUMBER_ARRAY_TAG => (0, DYNAMIC_NUMBER_TAG),
        DYNAMIC_BOOLEAN_ARRAY_TAG => (1, DYNAMIC_BOOLEAN_TAG),
        DYNAMIC_STRING_ARRAY_TAG => (2, DYNAMIC_STRING_TAG),
        _ => {
            CALL_ERROR.with(|error| error.set(INVALID_DYNAMIC_VALUE.as_ptr().cast()));
            return 0.0;
        }
    };
    let value = unsafe { array_at(f64::from_bits(dynamic.payload), index, kind) };
    if !CALL_PRESENT.with(Cell::get) {
        0.0
    } else {
        dynamic_from_parts(tag as f64, value)
    }
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn dynamic_array_length(value: f64) -> f64 {
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
    array_length(f64::from_bits(dynamic.payload))
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn number_array_get(value: f64, index: f64) -> f64 {
    unsafe { array_get(value, index, 0) }
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn bool_array_get(value: f64, index: f64) -> f64 {
    unsafe { array_get(value, index, 1) }
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn string_array_get(value: f64, index: f64) -> f64 {
    unsafe { array_get(value, index, 2) }
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn array_search(operation: u8, value: f64, needle: f64, from_index: f64) -> f64 {
    let (Some(search), Some((data, _))) =
        (ARRAY_SEARCH.with(Cell::get), unsafe { array_data(value) })
    else {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return 0.0;
    };
    unsafe { search(operation, data, needle, from_index) }
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn array_format(operation: u8, value: f64, separator: f64) -> f64 {
    let (Some(format), Some((data, _))) =
        (ARRAY_FORMAT.with(Cell::get), unsafe { array_data(value) })
    else {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return 0.0;
    };
    let result = unsafe {
        format(
            operation,
            data,
            separator.to_bits() as usize as *const c_char,
        )
    };
    if result.is_null() {
        CALL_ERROR.with(|error| error.set(ALLOCATION_FAILED.as_ptr().cast()));
        0.0
    } else {
        f64::from_bits(result as usize as u64)
    }
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn dynamic_array_join(value: f64, separator: f64) -> f64 {
    dynamic_primitive(value, None).map_or_else(
        || {
            CALL_ERROR.with(|error| error.set(INVALID_DYNAMIC_VALUE.as_ptr().cast()));
            0.0
        },
        |dynamic| {
            let operation = match dynamic.tag {
                DYNAMIC_NUMBER_ARRAY_TAG => 0,
                DYNAMIC_BOOLEAN_ARRAY_TAG => 2,
                DYNAMIC_STRING_ARRAY_TAG => 1,
                _ => {
                    CALL_ERROR.with(|error| error.set(INVALID_DYNAMIC_VALUE.as_ptr().cast()));
                    return 0.0;
                }
            };
            array_format(operation, f64::from_bits(dynamic.payload), separator)
        },
    )
}

