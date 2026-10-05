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
    let floor = value.floor();
    let rounded = if value - floor < 0.5 { floor } else { floor + 1.0 };
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
    (!pointer.is_null()).then(|| thaw_arena::NativeStr::from_ptr(pointer).to_string_lossy().into_owned())
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn string_length(value: f64) -> f64 {
    unsafe {
        string_utf16_argument(value)
            .map(|value| value.len() as f64)
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
    let handle = (bits & !SPARSE_ARRAY_RESULT_TAG) as usize as *const *const u8;
    if handle.is_null() {
        return None;
    }
    let data = unsafe { handle.read() };
    if data.is_null() {
        return None;
    }
    Some((data, unsafe { data.cast::<i64>().read() }.max(0) as usize))
}

// Untagged native handles are queried only through the explicitly installed
// callback. Legacy standalone JIT callers may supply one-word dense handles.
#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn array_index_state(value: f64, index: usize) -> u8 {
    if value.to_bits() & ARRAY_RESULT_TAG != 0 { return 1; }
    if value.to_bits() & SPARSE_ARRAY_RESULT_TAG != 0 {
        let handle = (value.to_bits() & !SPARSE_ARRAY_RESULT_TAG) as usize as *const *const u8;
        if handle.is_null() { return 0; }
        let presence = unsafe { handle.add(1).read() };
        if presence.is_null() { return 1; }
        return if index < unsafe { presence.cast::<u64>().read() as usize } {
            unsafe { presence.add(8 + index).read() }
        } else { 1 };
    }
    ARRAY_INDEX_PRESENT.with(Cell::get).map_or(1, |query| unsafe {
        query(value.to_bits() as usize as *const *const u8, index)
    })
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn array_index_present(value: f64, index: usize) -> bool {
    array_index_state(value, index) != 0
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn array_callback_root(value: f64) -> thaw_arena::ArenaRoot {
    // An internal dense result carries bit 1 and a sparse result carries
    // bit 2. The external native handle is untagged. Pin the actual arena
    // allocation while a callback can reset a traced arena generation.
    thaw_arena::ArenaRoot::new((value.to_bits() & !(ARRAY_RESULT_TAG | SPARSE_ARRAY_RESULT_TAG)) as usize)
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn mapped_string_roots(length: usize) -> Option<Vec<thaw_arena::ArenaRoot>> {
    let mut roots = Vec::new();
    if roots.try_reserve_exact(length).is_err() {
        CALL_ERROR.with(|error| error.set(ALLOCATION_FAILED.as_ptr().cast()));
        return None;
    }
    Some(roots)
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn array_result(pointer: *mut u8) -> f64 {
    f64::from_bits(pointer as usize as u64 | ARRAY_RESULT_TAG)
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn array_result_with_presence(data: *mut u8, presence: *mut u8) -> f64 {
    let _data_root = thaw_arena::ArenaRoot::new(data as usize);
    let _presence_root = thaw_arena::ArenaRoot::new(presence as usize);
    let Some(allocate) = ARENA_ALLOC.with(Cell::get) else {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return 0.0;
    };
    let handle = unsafe { allocate(2 * std::mem::size_of::<*mut u8>(), std::mem::align_of::<*mut u8>()) };
    if handle.is_null() {
        CALL_ERROR.with(|error| error.set(ALLOCATION_FAILED.as_ptr().cast()));
        return 0.0;
    }
    unsafe {
        let slots = handle.cast::<*mut u8>();
        slots.write(data);
        slots.add(1).write(presence);
    }
    f64::from_bits(handle as usize as u64 | SPARSE_ARRAY_RESULT_TAG)
}

// Collect mapped values before allocating the result: a callback may grow,
// truncate, or move the source array and can also reenter the arena. `None`
// is an actual hole, never a numeric NaN or an undefined value surrogate.
#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn mapped_array_result(values: &[Option<u64>]) -> f64 {
    let Some(allocate) = ARENA_ALLOC.with(Cell::get) else {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return 0.0;
    };
    let Some(size) = values.len().checked_mul(8).and_then(|bytes| bytes.checked_add(8)) else {
        CALL_ERROR.with(|error| error.set(ALLOCATION_FAILED.as_ptr().cast()));
        return 0.0;
    };
    let output = unsafe { allocate(size, 8) };
    if output.is_null() {
        CALL_ERROR.with(|error| error.set(ALLOCATION_FAILED.as_ptr().cast()));
        return 0.0;
    }
    unsafe { output.cast::<u64>().write(values.len() as u64) };
    for (index, value) in values.iter().enumerate() {
        unsafe { output.add(8 + index * 8).cast::<u64>().write_unaligned(value.unwrap_or(0)) };
    }
    if values.iter().all(Option::is_some) { return array_result(output); }
    let Some(presence_size) = values.len().checked_add(8) else {
        CALL_ERROR.with(|error| error.set(ALLOCATION_FAILED.as_ptr().cast()));
        return 0.0;
    };
    let presence = unsafe { allocate(presence_size, 8) };
    if presence.is_null() {
        CALL_ERROR.with(|error| error.set(ALLOCATION_FAILED.as_ptr().cast()));
        return 0.0;
    }
    unsafe { presence.cast::<u64>().write(values.len() as u64) };
    for (index, value) in values.iter().enumerate() {
        unsafe { presence.add(8 + index).write(u8::from(value.is_some())) };
    }
    array_result_with_presence(output, presence)
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn mapped_array_result_with_states(values: &[Option<u64>], states: &[u8]) -> f64 {
    if values.len() != states.len() || states.iter().any(|state| *state > 2) {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return 0.0;
    }
    let Some(allocate) = ARENA_ALLOC.with(Cell::get) else {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return 0.0;
    };
    let Some(size) = values.len().checked_mul(8).and_then(|bytes| bytes.checked_add(8)) else {
        CALL_ERROR.with(|error| error.set(ALLOCATION_FAILED.as_ptr().cast()));
        return 0.0;
    };
    let output = unsafe { allocate(size, 8) };
    if output.is_null() {
        CALL_ERROR.with(|error| error.set(ALLOCATION_FAILED.as_ptr().cast()));
        return 0.0;
    }
    unsafe { output.cast::<u64>().write(values.len() as u64) };
    for (index, value) in values.iter().enumerate() {
        unsafe { output.add(8 + index * 8).cast::<u64>().write_unaligned(value.unwrap_or(0)) };
    }
    if states.iter().all(|state| *state == 1) { return array_result(output); }
    let Some(presence_size) = states.len().checked_add(8) else {
        CALL_ERROR.with(|error| error.set(ALLOCATION_FAILED.as_ptr().cast()));
        return 0.0;
    };
    let presence = unsafe { allocate(presence_size, 8) };
    if presence.is_null() {
        CALL_ERROR.with(|error| error.set(ALLOCATION_FAILED.as_ptr().cast()));
        return 0.0;
    }
    unsafe { presence.cast::<u64>().write(states.len() as u64) };
    for (index, state) in states.iter().enumerate() {
        unsafe { presence.add(8 + index).write(*state) };
    }
    array_result_with_presence(output, presence)
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn mapped_slots(length: usize) -> Option<Vec<Option<u64>>> {
    let mut values = Vec::new();
    if values.try_reserve_exact(length).is_err() {
        CALL_ERROR.with(|error| error.set(ALLOCATION_FAILED.as_ptr().cast()));
        return None;
    }
    values.resize(length, None);
    Some(values)
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
    if value.to_bits() & SPARSE_ARRAY_RESULT_TAG != 0 {
        return f64::from_bits(value.to_bits() & !SPARSE_ARRAY_RESULT_TAG);
    }
    if value.to_bits() & ARRAY_RESULT_TAG == 0 {
        return value;
    }
    let (Some(allocate), Some((data, _))) =
        (ARENA_ALLOC.with(Cell::get), unsafe { array_data(value) })
    else {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return 0.0;
    };
    // The app reads this boxed handle as its own `{buffer, presence}` array
    // value (e.g. an array stored in a fixed-object field), so reserve the
    // presence word and leave it null -- JIT arrays are always dense. The
    // JIT itself only reads the buffer pointer in the first word.
    let handle = unsafe {
        allocate(
            std::mem::size_of::<*mut u8>() * 2,
            std::mem::align_of::<*mut u8>(),
        )
    };
    if handle.is_null() {
        CALL_ERROR.with(|error| error.set(ALLOCATION_FAILED.as_ptr().cast()));
        return 0.0;
    }
    unsafe {
        let slots = handle.cast::<*const u8>();
        slots.write(data);
        slots.add(1).write(std::ptr::null());
    }
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
    if !index.is_finite() || index < 0.0 || index >= length as f64
        || !array_index_present(value, index as usize) {
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
    if !index.is_finite() || index < 0.0 || index.fract() != 0.0 || index >= length as f64
        || !array_index_present(value, index as usize) {
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

// Keep a temporary mask alive across one native search/format call. The
// pointer aliases `owned` only for an external two-word native handle.
#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn array_native_presence(value: f64, length: usize) -> Option<(Option<Vec<u64>>, *const u8)> {
    let bits = value.to_bits();
    if bits & SPARSE_ARRAY_RESULT_TAG != 0 {
        let handle = (bits & !SPARSE_ARRAY_RESULT_TAG) as usize as *const *const u8;
        return Some((None, unsafe { handle.add(1).read() }));
    }
    if bits & ARRAY_RESULT_TAG != 0 { return Some((None, std::ptr::null())); }
    let Some(query) = ARRAY_INDEX_PRESENT.with(Cell::get) else {
        return Some((None, std::ptr::null()));
    };
    // The installed query is authoritative. A standalone legacy handle may
    // have only one word, so never read its second word here.
    let Some(words) = length.checked_add(7).map(|length| length / 8 + 1) else {
        CALL_ERROR.with(|error| error.set(ALLOCATION_FAILED.as_ptr().cast()));
        return None;
    };
    let mut mask = Vec::<u64>::new();
    if mask.try_reserve_exact(words).is_err() {
        CALL_ERROR.with(|error| error.set(ALLOCATION_FAILED.as_ptr().cast()));
        return None;
    }
    mask.resize(words, 0);
    mask[0] = length as u64;
    let bytes = mask.as_mut_ptr().cast::<u8>();
    let handle = bits as usize as *const *const u8;
    for index in 0..length {
        // Preserve state 2: it is an own explicit `undefined`, not a hole.
        unsafe { bytes.add(8 + index).write(query(handle, index)) };
        if CALL_ERROR.with(|error| !error.get().is_null()) { return None; }
    }
    let pointer = mask.as_ptr().cast::<u8>();
    Some((Some(mask), pointer))
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn array_search(operation: u8, value: f64, needle: f64, from_index: f64) -> f64 {
    let (Some(search), Some((data, length))) =
        (ARRAY_SEARCH.with(Cell::get), unsafe { array_data(value) })
    else {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return 0.0;
    };
    let Some((_owned_presence, presence)) = array_native_presence(value, length) else {
        return 0.0;
    };
    unsafe { search(operation, data, presence, needle, from_index) }
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn array_format(operation: u8, value: f64, separator: f64) -> f64 {
    let (Some(format), Some((data, length))) =
        (ARRAY_FORMAT.with(Cell::get), unsafe { array_data(value) })
    else {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return 0.0;
    };
    let Some((_owned_presence, presence)) = array_native_presence(value, length) else {
        return 0.0;
    };
    let result = unsafe {
        format(
            operation,
            data,
            presence,
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


#[cfg(all(test, target_arch = "x86_64", target_family = "unix"))]
#[test]
fn round_number_preserves_integers_and_ecmascript_ties() {
    for (input, expected) in [
        (4_503_599_627_370_497.0_f64, 4_503_599_627_370_497.0_f64),
        (0.49999999999999994, 0.0),
        (0.5, 1.0),
        (-1.5, -1.0),
        (-0.5, -0.0),
        (-0.1, -0.0),
        (-0.0, -0.0),
        (f64::INFINITY, f64::INFINITY),
        (f64::NEG_INFINITY, f64::NEG_INFINITY),
    ] {
        assert_eq!(round_number(input).to_bits(), expected.to_bits());
    }
    assert!(round_number(f64::NAN).is_nan());
}
