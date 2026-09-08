#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn array_extreme(value: f64, is_min: bool) -> f64 {
    let Some((array, length)) = (unsafe { array_data(value) }) else {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return 0.0;
    };
    let mut result = if is_min {
        f64::INFINITY
    } else {
        f64::NEG_INFINITY
    };
    for index in 0..length {
        let value = unsafe { array.add(8 + index * 8).cast::<f64>().read() };
        result = if is_min {
            minimum(result, value)
        } else {
            maximum(result, value)
        };
    }
    result
}

extern "C" fn array_min(value: f64) -> f64 {
    array_extreme(value, true)
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn array_max(value: f64) -> f64 {
    array_extreme(value, false)
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn array_hypot(value: f64) -> f64 {
    let Some((array, length)) = (unsafe { array_data(value) }) else {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return 0.0;
    };
    let mut result = 0.0f64;
    for index in 0..length {
        result = result.hypot(unsafe { array.add(8 + index * 8).cast::<f64>().read() });
    }
    result
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn number_array_reduce(
    value: f64,
    initial: Option<f64>,
    from_right: bool,
    operation: impl Fn(f64, f64) -> f64,
) -> f64 {
    let Some((array, length)) = (unsafe { array_data(value) }) else {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return 0.0;
    };
    if from_right {
        let (mut accumulator, end) = match initial {
            Some(initial) => (initial, length),
            None if length != 0 => (
                unsafe { array.add(8 + (length - 1) * 8).cast::<f64>().read() },
                length - 1,
            ),
            None => {
                CALL_ERROR.with(|error| error.set(EMPTY_REDUCE.as_ptr().cast()));
                return 0.0;
            }
        };
        for index in (0..end).rev() {
            accumulator = operation(accumulator, unsafe {
                array.add(8 + index * 8).cast::<f64>().read()
            });
        }
        return accumulator;
    }
    let (mut accumulator, start) = match initial {
        Some(initial) => (initial, 0),
        None if length != 0 => (unsafe { array.add(8).cast::<f64>().read() }, 1),
        None => {
            CALL_ERROR.with(|error| error.set(EMPTY_REDUCE.as_ptr().cast()));
            return 0.0;
        }
    };
    for index in start..length {
        accumulator = operation(accumulator, unsafe {
            array.add(8 + index * 8).cast::<f64>().read()
        });
    }
    accumulator
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
macro_rules! number_array_reducers {
    ($forward:ident, $first:ident, $reverse:ident, $last:ident, $operation:expr) => {
        extern "C" fn $forward(value: f64, accumulator: f64) -> f64 {
            number_array_reduce(value, Some(accumulator), false, $operation)
        }
        extern "C" fn $first(value: f64, _unused: f64) -> f64 {
            number_array_reduce(value, None, false, $operation)
        }
        extern "C" fn $reverse(value: f64, accumulator: f64) -> f64 {
            number_array_reduce(value, Some(accumulator), true, $operation)
        }
        extern "C" fn $last(value: f64, _unused: f64) -> f64 {
            number_array_reduce(value, None, true, $operation)
        }
    };
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
number_array_reducers!(
    number_array_reduce_add,
    number_array_reduce_add_first,
    number_array_reduce_add_right,
    number_array_reduce_add_last,
    |left, right| left + right
);
#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
number_array_reducers!(
    number_array_reduce_subtract,
    number_array_reduce_subtract_first,
    number_array_reduce_subtract_right,
    number_array_reduce_subtract_last,
    |left, right| left - right
);
#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
number_array_reducers!(
    number_array_reduce_multiply,
    number_array_reduce_multiply_first,
    number_array_reduce_multiply_right,
    number_array_reduce_multiply_last,
    |left, right| left * right
);
#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
number_array_reducers!(
    number_array_reduce_divide,
    number_array_reduce_divide_first,
    number_array_reduce_divide_right,
    number_array_reduce_divide_last,
    |left, right| left / right
);
#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
number_array_reducers!(
    number_array_reduce_remainder,
    number_array_reduce_remainder_first,
    number_array_reduce_remainder_right,
    number_array_reduce_remainder_last,
    |left, right| left % right
);
#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
number_array_reducers!(
    number_array_reduce_power,
    number_array_reduce_power_first,
    number_array_reduce_power_right,
    number_array_reduce_power_last,
    |left, right| power(left, right)
);
#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
number_array_reducers!(
    number_array_reduce_minimum,
    number_array_reduce_minimum_first,
    number_array_reduce_minimum_right,
    number_array_reduce_minimum_last,
    |left, right| minimum(left, right)
);
#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
number_array_reducers!(
    number_array_reduce_maximum,
    number_array_reduce_maximum_first,
    number_array_reduce_maximum_right,
    number_array_reduce_maximum_last,
    |left, right| maximum(left, right)
);

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn number_array_quantify(
    value: f64,
    operand: f64,
    every: bool,
    predicate: impl Fn(f64, f64) -> bool,
) -> f64 {
    let Some((array, length)) = (unsafe { array_data(value) }) else {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return 0.0;
    };
    for index in 0..length {
        let matches = predicate(
            unsafe { array.add(8 + index * 8).cast::<f64>().read() },
            operand,
        );
        if matches != every {
            return f64::from(!every);
        }
    }
    f64::from(every)
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
macro_rules! number_array_quantifiers {
    ($some:ident, $every:ident, $predicate:expr) => {
        extern "C" fn $some(value: f64, operand: f64) -> f64 {
            number_array_quantify(value, operand, false, $predicate)
        }
        extern "C" fn $every(value: f64, operand: f64) -> f64 {
            number_array_quantify(value, operand, true, $predicate)
        }
    };
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
number_array_quantifiers!(
    number_array_some_lt,
    number_array_every_lt,
    |left, right| left < right
);
#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
number_array_quantifiers!(
    number_array_some_lte,
    number_array_every_lte,
    |left, right| left <= right
);
#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
number_array_quantifiers!(
    number_array_some_gt,
    number_array_every_gt,
    |left, right| left > right
);
#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
number_array_quantifiers!(
    number_array_some_gte,
    number_array_every_gte,
    |left, right| left >= right
);
#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
number_array_quantifiers!(
    number_array_some_eq,
    number_array_every_eq,
    |left, right| left == right
);
#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
number_array_quantifiers!(
    number_array_some_ne,
    number_array_every_ne,
    |left, right| left != right
);

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn number_array_find(
    value: f64,
    operand: f64,
    mode: u8,
    predicate: impl Fn(f64, f64) -> bool,
) -> f64 {
    let Some((array, length)) = (unsafe { array_data(value) }) else {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return 0.0;
    };
    let matches = |index: usize| {
        predicate(
            unsafe { array.add(8 + index * 8).cast::<f64>().read() },
            operand,
        )
    };
    let index = if mode >= 2 {
        (0..length).rev().find(|index| matches(*index))
    } else {
        (0..length).find(|index| matches(*index))
    };
    match (index, mode % 2) {
        (Some(index), 0) => unsafe { array.add(8 + index * 8).cast::<f64>().read() },
        (Some(index), 1) => index as f64,
        (None, 0) => {
            CALL_PRESENT.with(|present| present.set(false));
            0.0
        }
        (None, 1) => -1.0,
        _ => unreachable!(),
    }
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
macro_rules! number_array_finders {
    ($find:ident, $find_index:ident, $find_last:ident, $find_last_index:ident, $predicate:expr) => {
        extern "C" fn $find(value: f64, operand: f64) -> f64 {
            number_array_find(value, operand, 0, $predicate)
        }
        extern "C" fn $find_index(value: f64, operand: f64) -> f64 {
            number_array_find(value, operand, 1, $predicate)
        }
        extern "C" fn $find_last(value: f64, operand: f64) -> f64 {
            number_array_find(value, operand, 2, $predicate)
        }
        extern "C" fn $find_last_index(value: f64, operand: f64) -> f64 {
            number_array_find(value, operand, 3, $predicate)
        }
    };
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
number_array_finders!(
    number_array_find_lt,
    number_array_find_index_lt,
    number_array_find_last_lt,
    number_array_find_last_index_lt,
    |left, right| left < right
);
#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
number_array_finders!(
    number_array_find_lte,
    number_array_find_index_lte,
    number_array_find_last_lte,
    number_array_find_last_index_lte,
    |left, right| left <= right
);
#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
number_array_finders!(
    number_array_find_gt,
    number_array_find_index_gt,
    number_array_find_last_gt,
    number_array_find_last_index_gt,
    |left, right| left > right
);
#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
number_array_finders!(
    number_array_find_gte,
    number_array_find_index_gte,
    number_array_find_last_gte,
    number_array_find_last_index_gte,
    |left, right| left >= right
);
#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
number_array_finders!(
    number_array_find_eq,
    number_array_find_index_eq,
    number_array_find_last_eq,
    number_array_find_last_index_eq,
    |left, right| left == right
);
#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
number_array_finders!(
    number_array_find_ne,
    number_array_find_index_ne,
    number_array_find_last_ne,
    number_array_find_last_index_ne,
    |left, right| left != right
);

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn number_array_filter(value: f64, operand: f64, predicate: impl Fn(f64, f64) -> bool) -> f64 {
    let (Some(allocate), Some((array, length))) =
        (ARENA_ALLOC.with(Cell::get), unsafe { array_data(value) })
    else {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return 0.0;
    };
    let Some(size) = length.checked_mul(8).and_then(|size| size.checked_add(8)) else {
        CALL_ERROR.with(|error| error.set(ALLOCATION_FAILED.as_ptr().cast()));
        return 0.0;
    };
    let output = unsafe { allocate(size, 8) };
    if output.is_null() {
        CALL_ERROR.with(|error| error.set(ALLOCATION_FAILED.as_ptr().cast()));
        return 0.0;
    }
    let mut selected = 0;
    for index in 0..length {
        let value = unsafe { array.add(8 + index * 8).cast::<f64>().read() };
        if predicate(value, operand) {
            unsafe { output.add(8 + selected * 8).cast::<f64>().write(value) };
            selected += 1;
        }
    }
    unsafe { output.cast::<u64>().write(selected as u64) };
    array_result(output)
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
macro_rules! number_array_filters {
    ($name:ident, $predicate:expr) => {
        extern "C" fn $name(value: f64, operand: f64) -> f64 {
            number_array_filter(value, operand, $predicate)
        }
    };
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
number_array_filters!(number_array_filter_lt, |left, right| left < right);
#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
number_array_filters!(number_array_filter_lte, |left, right| left <= right);
#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
number_array_filters!(number_array_filter_gt, |left, right| left > right);
#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
number_array_filters!(number_array_filter_gte, |left, right| left >= right);
#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
number_array_filters!(number_array_filter_eq, |left, right| left == right);
#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
number_array_filters!(number_array_filter_ne, |left, right| left != right);

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn primitive_array_scan(
    value: f64,
    kind: u8,
    mode: u8,
    matches: impl Fn(*const u8, usize) -> bool,
) -> f64 {
    let Some((array, length)) = (unsafe { array_data(value) }) else {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return 0.0;
    };
    if kind > 2 || mode > 6 {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return 0.0;
    }
    if mode < 2 {
        return f64::from(if mode == 1 {
            (0..length).all(|index| matches(array, index))
        } else {
            (0..length).any(|index| matches(array, index))
        });
    }
    if mode < 6 {
        let index = if mode >= 4 {
            (0..length).rev().find(|index| matches(array, *index))
        } else {
            (0..length).find(|index| matches(array, *index))
        };
        return match (index, mode % 2) {
            (Some(index), 0) => unsafe { array_element(array, index, kind) },
            (Some(index), 1) => index as f64,
            (None, 0) => {
                CALL_PRESENT.with(|present| present.set(false));
                0.0
            }
            (None, 1) => -1.0,
            _ => unreachable!(),
        };
    }
    let Some(allocate) = ARENA_ALLOC.with(Cell::get) else {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return 0.0;
    };
    let Some(size) = length.checked_mul(8).and_then(|size| size.checked_add(8)) else {
        CALL_ERROR.with(|error| error.set(ALLOCATION_FAILED.as_ptr().cast()));
        return 0.0;
    };
    let output = unsafe { allocate(size, 8) };
    if output.is_null() {
        CALL_ERROR.with(|error| error.set(ALLOCATION_FAILED.as_ptr().cast()));
        return 0.0;
    }
    let mut selected = 0;
    for index in 0..length {
        if matches(array, index) {
            unsafe {
                output
                    .add(8 + selected * 8)
                    .cast::<u64>()
                    .write_unaligned(array.add(8 + index * 8).cast::<u64>().read_unaligned())
            };
            selected += 1;
        }
    }
    unsafe { output.cast::<u64>().write(selected as u64) };
    array_result(output)
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn primitive_array_truthy(value: f64, encoded: f64) -> f64 {
    let encoded = encoded as u8;
    let kind = encoded / 8;
    let mode = encoded % 8;
    primitive_array_scan(value, kind, mode, |array, index| unsafe {
        let slot = array.add(8 + index * 8);
        match kind {
            0 => {
                let value = slot.cast::<f64>().read_unaligned();
                value != 0.0 && !value.is_nan()
            }
            1 => slot.read() != 0,
            2 => {
                let value = slot.cast::<*const c_char>().read_unaligned();
                !value.is_null() && !CStr::from_ptr(value).to_bytes().is_empty()
            }
            _ => false,
        }
    })
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn dynamic_array_truthy(value: f64, mode: f64) -> f64 {
    let mode = mode as u8;
    let Some(array) = dynamic_primitive(value, None) else {
        CALL_ERROR.with(|error| error.set(INVALID_DYNAMIC_VALUE.as_ptr().cast()));
        return 0.0;
    };
    let kind = match array.tag {
        DYNAMIC_NUMBER_ARRAY_TAG => 0,
        DYNAMIC_BOOLEAN_ARRAY_TAG => 1,
        DYNAMIC_STRING_ARRAY_TAG => 2,
        _ => {
            CALL_ERROR.with(|error| error.set(INVALID_DYNAMIC_VALUE.as_ptr().cast()));
            return 0.0;
        }
    };
    let result = primitive_array_truthy(f64::from_bits(array.payload), f64::from(kind * 8 + mode));
    dynamic_array_scan_result(array.tag, mode, result)
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn dynamic_array_scan_result(array_tag: u64, mode: u8, result: f64) -> f64 {
    match mode {
        2 | 4 if CALL_PRESENT.with(Cell::get) => dynamic_from_parts(
            match array_tag {
                DYNAMIC_NUMBER_ARRAY_TAG => DYNAMIC_NUMBER_TAG,
                DYNAMIC_BOOLEAN_ARRAY_TAG => DYNAMIC_BOOLEAN_TAG,
                DYNAMIC_STRING_ARRAY_TAG => DYNAMIC_STRING_TAG,
                _ => unreachable!(),
            } as f64,
            result,
        ),
        6 => dynamic_array_result(array_tag, result),
        _ => result,
    }
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn primitive_array_compare(value: f64, operand: f64, encoded: f64) -> f64 {
    let encoded = encoded as u8;
    let kind = encoded / 64;
    let mode = encoded / 8 % 8;
    let operation = encoded % 8;
    if !(1..=2).contains(&kind) || operation > 5 {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return 0.0;
    }
    primitive_array_scan(value, kind, mode, |array, index| unsafe {
        let element = array_element(array, index, kind);
        let ordering = if kind == 1 {
            element.total_cmp(&operand)
        } else {
            string_compare(element, operand).total_cmp(&0.0)
        };
        match operation {
            0 => ordering.is_lt(),
            1 => !ordering.is_gt(),
            2 => ordering.is_gt(),
            3 => !ordering.is_lt(),
            4 => ordering.is_eq(),
            5 => !ordering.is_eq(),
            _ => false,
        }
    })
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn dynamic_array_compare(value: f64, operand: f64, encoded: f64) -> f64 {
    let encoded = encoded as u8;
    let mode = encoded / 8;
    let operation = encoded % 8;
    let Some((array, operand)) =
        dynamic_primitive(value, None).zip(dynamic_primitive(operand, None))
    else {
        CALL_ERROR.with(|error| error.set(INVALID_DYNAMIC_VALUE.as_ptr().cast()));
        return 0.0;
    };
    let (kind, element_tag) = match array.tag {
        DYNAMIC_NUMBER_ARRAY_TAG => (0, DYNAMIC_NUMBER_TAG),
        DYNAMIC_BOOLEAN_ARRAY_TAG => (1, DYNAMIC_BOOLEAN_TAG),
        DYNAMIC_STRING_ARRAY_TAG => (2, DYNAMIC_STRING_TAG),
        _ => {
            CALL_ERROR.with(|error| error.set(INVALID_DYNAMIC_VALUE.as_ptr().cast()));
            return 0.0;
        }
    };
    if operand.tag > DYNAMIC_BOOLEAN_TAG || mode > 6 || operation > 7 {
        CALL_ERROR.with(|error| error.set(INVALID_DYNAMIC_VALUE.as_ptr().cast()));
        return 0.0;
    }
    let result = primitive_array_scan(f64::from_bits(array.payload), kind, mode, |data, index| {
        let element = unsafe { array_element(data, index, kind) };
        let same = || match element_tag {
            DYNAMIC_NUMBER_TAG => element == f64::from_bits(operand.payload),
            DYNAMIC_BOOLEAN_TAG => (element != 0.0) == (operand.payload != 0),
            DYNAMIC_STRING_TAG => {
                string_same_value(element, f64::from_bits(operand.payload)) != 0.0
            }
            _ => unreachable!(),
        };
        let number = |tag, payload| match tag {
            DYNAMIC_NUMBER_TAG => f64::from_bits(payload),
            DYNAMIC_BOOLEAN_TAG => f64::from(payload != 0),
            DYNAMIC_STRING_TAG => string_to_number(f64::from_bits(payload)),
            _ => f64::NAN,
        };
        let element_payload = if element_tag == DYNAMIC_BOOLEAN_TAG {
            u64::from(element != 0.0)
        } else {
            element.to_bits()
        };
        match operation {
            0..=3 => {
                if element_tag == DYNAMIC_STRING_TAG && operand.tag == DYNAMIC_STRING_TAG {
                    let ordering = string_compare(element, f64::from_bits(operand.payload));
                    match operation {
                        0 => ordering < 0.0,
                        1 => ordering <= 0.0,
                        2 => ordering > 0.0,
                        _ => ordering >= 0.0,
                    }
                } else {
                    let left = number(element_tag, element_payload);
                    let right = number(operand.tag, operand.payload);
                    match operation {
                        0 => left < right,
                        1 => left <= right,
                        2 => left > right,
                        _ => left >= right,
                    }
                }
            }
            4 => {
                element_tag == operand.tag && same()
                    || element_tag != operand.tag
                        && number(element_tag, element_payload)
                            == number(operand.tag, operand.payload)
            }
            5 => {
                !(element_tag == operand.tag && same()
                    || element_tag != operand.tag
                        && number(element_tag, element_payload)
                            == number(operand.tag, operand.payload))
            }
            6 => element_tag == operand.tag && same(),
            7 => element_tag != operand.tag || !same(),
            _ => false,
        }
    });
    dynamic_array_scan_result(array.tag, mode, result)
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn primitive_array_map(value: f64, encoded: f64) -> f64 {
    let encoded = encoded as u8;
    let kind = encoded / 8;
    let operation = encoded % 8;
    if !(1..=2).contains(&kind) || !matches!((kind, operation), (1, 0 | 1) | (2, 0 | 2..=7)) {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return 0.0;
    }
    let (Some(allocate), Some((array, length))) =
        (ARENA_ALLOC.with(Cell::get), unsafe { array_data(value) })
    else {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return 0.0;
    };
    let Some(size) = length.checked_mul(8).and_then(|size| size.checked_add(8)) else {
        CALL_ERROR.with(|error| error.set(ALLOCATION_FAILED.as_ptr().cast()));
        return 0.0;
    };
    let output = unsafe { allocate(size, 8) };
    if output.is_null() {
        CALL_ERROR.with(|error| error.set(ALLOCATION_FAILED.as_ptr().cast()));
        return 0.0;
    }
    unsafe { output.cast::<u64>().write(length as u64) };
    for index in 0..length {
        let slot = unsafe { array.add(8 + index * 8).cast::<u64>().read_unaligned() };
        let mapped = match operation {
            0 => slot,
            1 => u64::from(slot == 0),
            2 => string_to_lower_case(f64::from_bits(slot)).to_bits(),
            3 => string_to_upper_case(f64::from_bits(slot)).to_bits(),
            4 => string_trim(f64::from_bits(slot)).to_bits(),
            5 => string_trim_start(f64::from_bits(slot)).to_bits(),
            6 => string_trim_end(f64::from_bits(slot)).to_bits(),
            7 => string_length(f64::from_bits(slot)).to_bits(),
            _ => unreachable!(),
        };
        unsafe {
            output
                .add(8 + index * 8)
                .cast::<u64>()
                .write_unaligned(mapped)
        };
    }
    array_result(output)
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn primitive_array_convert(value: f64, encoded: f64) -> f64 {
    let encoded = encoded as u8;
    let source = encoded / 4;
    let target = encoded % 4;
    if source > 2 || target > 2 {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return 0.0;
    }
    let (Some(allocate), Some((array, length))) =
        (ARENA_ALLOC.with(Cell::get), unsafe { array_data(value) })
    else {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return 0.0;
    };
    let Some(size) = length.checked_mul(8).and_then(|size| size.checked_add(8)) else {
        CALL_ERROR.with(|error| error.set(ALLOCATION_FAILED.as_ptr().cast()));
        return 0.0;
    };
    let output = unsafe { allocate(size, 8) };
    if output.is_null() {
        CALL_ERROR.with(|error| error.set(ALLOCATION_FAILED.as_ptr().cast()));
        return 0.0;
    }
    unsafe { output.cast::<u64>().write(length as u64) };
    for index in 0..length {
        let slot = unsafe { array.add(8 + index * 8).cast::<u64>().read_unaligned() };
        let value = match source {
            0 | 2 => f64::from_bits(slot),
            1 => f64::from(u8::from(slot != 0)),
            _ => unreachable!(),
        };
        let mapped = match (source, target) {
            (_, 0) if source != 2 => value,
            (2, 0) => string_to_number(value),
            (0, 1) => f64::from(u8::from(value != 0.0 && !value.is_nan())),
            (1, 1) => value,
            (2, 1) => string_truthy(value),
            (0, 2) => number_to_string(value),
            (1, 2) => boolean_to_string(value),
            (2, 2) => value,
            _ => unreachable!(),
        };
        let mapped = if target == 1 {
            u64::from(mapped != 0.0)
        } else {
            mapped.to_bits()
        };
        unsafe {
            output
                .add(8 + index * 8)
                .cast::<u64>()
                .write_unaligned(mapped)
        };
    }
    array_result(output)
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn dynamic_array_convert(value: f64, target: f64) -> f64 {
    let Some(array) = dynamic_primitive(value, None) else {
        CALL_ERROR.with(|error| error.set(INVALID_DYNAMIC_VALUE.as_ptr().cast()));
        return 0.0;
    };
    let source = match array.tag {
        DYNAMIC_NUMBER_ARRAY_TAG => 0,
        DYNAMIC_BOOLEAN_ARRAY_TAG => 1,
        DYNAMIC_STRING_ARRAY_TAG => 2,
        _ => {
            CALL_ERROR.with(|error| error.set(INVALID_DYNAMIC_VALUE.as_ptr().cast()));
            return 0.0;
        }
    };
    primitive_array_convert(
        f64::from_bits(array.payload),
        f64::from(source * 4 + target as u8),
    )
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn dynamic_array_map_identity(value: f64) -> f64 {
    let Some(array) = dynamic_primitive(value, None) else {
        CALL_ERROR.with(|error| error.set(INVALID_DYNAMIC_VALUE.as_ptr().cast()));
        return 0.0;
    };
    let source = match array.tag {
        DYNAMIC_NUMBER_ARRAY_TAG => 0,
        DYNAMIC_BOOLEAN_ARRAY_TAG => 1,
        DYNAMIC_STRING_ARRAY_TAG => 2,
        _ => {
            CALL_ERROR.with(|error| error.set(INVALID_DYNAMIC_VALUE.as_ptr().cast()));
            return 0.0;
        }
    };
    let result = primitive_array_convert(
        f64::from_bits(array.payload),
        f64::from(source * 4 + source),
    );
    dynamic_array_result(array.tag, result)
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn number_array_map(value: f64, operation: impl Fn(f64, f64) -> f64) -> f64 {
    let (Some(allocate), Some((array, length))) =
        (ARENA_ALLOC.with(Cell::get), unsafe { array_data(value) })
    else {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return 0.0;
    };
    let Some(size) = length.checked_mul(8).and_then(|size| size.checked_add(8)) else {
        CALL_ERROR.with(|error| error.set(ALLOCATION_FAILED.as_ptr().cast()));
        return 0.0;
    };
    let output = unsafe { allocate(size, 8) };
    if output.is_null() {
        CALL_ERROR.with(|error| error.set(ALLOCATION_FAILED.as_ptr().cast()));
        return 0.0;
    }
    unsafe { output.cast::<u64>().write(length as u64) };
    for index in 0..length {
        let element = unsafe { array.add(8 + index * 8).cast::<f64>().read() };
        let mapped = operation(element, index as f64);
        unsafe { output.add(8 + index * 8).cast::<f64>().write(mapped) };
    }
    array_result(output)
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
type JitCallback = extern "C" fn(*const f64) -> f64;

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn compile_jit_callback(callback: f64) -> Option<(JitCallback, usize)> {
    let Some(callback) = (unsafe { string_argument(callback) }) else {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return None;
    };
    let symbol = format!("expr:{callback}:array-callback");
    let Some(program) = NumericProgram::parse(&symbol) else {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return None;
    };
    let required_args = program.required_args();
    if required_args > 16 {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return None;
    }
    let code = match compile(&symbol, &program) {
        Ok((code, _)) => code,
        Err(error) => {
            CALL_ERROR.with(|slot| slot.set(error));
            return None;
        }
    };
    Some((
        unsafe { std::mem::transmute::<*mut libc::c_void, JitCallback>(code) },
        required_args,
    ))
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn capture_arguments(value: f64, builtins: usize, required: usize) -> Option<Vec<f64>> {
    let Some((array, length)) = (unsafe { array_data(value) }) else {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return None;
    };
    if builtins + length > 16 || required > builtins + length {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return None;
    }
    Some(
        (0..length)
            .map(|index| unsafe { array.add(8 + index * 8).cast::<f64>().read_unaligned() })
            .collect(),
    )
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn call_jit_callback(callback: JitCallback, builtins: &[f64], captures: &[f64]) -> f64 {
    let mut arguments = [0.0; 16];
    arguments[..builtins.len()].copy_from_slice(builtins);
    arguments[builtins.len()..builtins.len() + captures.len()].copy_from_slice(captures);
    callback(arguments.as_ptr())
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn primitive_array_jit_map_impl(
    value: f64,
    callback: f64,
    encoded: f64,
    captures: Option<f64>,
) -> f64 {
    let encoded = encoded as u8;
    let source = encoded / 4;
    let target = encoded % 4;
    if source > 2 || target > 2 {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return 0.0;
    }
    let Some((callback, required)) = compile_jit_callback(callback) else {
        return 0.0;
    };
    let captures = if let Some(captures) = captures {
        let Some(captures) = capture_arguments(captures, 3, required) else {
            return 0.0;
        };
        captures
    } else {
        if required > 3 {
            CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
            return 0.0;
        }
        Vec::new()
    };
    let (Some(allocate), Some((array, length))) =
        (ARENA_ALLOC.with(Cell::get), unsafe { array_data(value) })
    else {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return 0.0;
    };
    let Some(size) = length.checked_mul(8).and_then(|size| size.checked_add(8)) else {
        CALL_ERROR.with(|error| error.set(ALLOCATION_FAILED.as_ptr().cast()));
        return 0.0;
    };
    let output = unsafe { allocate(size, 8) };
    if output.is_null() {
        CALL_ERROR.with(|error| error.set(ALLOCATION_FAILED.as_ptr().cast()));
        return 0.0;
    }
    unsafe { output.cast::<u64>().write(length as u64) };
    for index in 0..length {
        let element = unsafe { array_element(array, index, source) };
        let mapped = call_jit_callback(callback, &[element, index as f64, value], &captures);
        let mapped = if target == 1 {
            u64::from(mapped != 0.0 && !mapped.is_nan())
        } else {
            mapped.to_bits()
        };
        unsafe {
            output
                .add(8 + index * 8)
                .cast::<u64>()
                .write_unaligned(mapped)
        };
    }
    array_result(output)
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn primitive_array_jit_map(value: f64, callback: f64, encoded: f64) -> f64 {
    primitive_array_jit_map_impl(value, callback, encoded, None)
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn primitive_array_jit_map_captured(
    value: f64,
    callback: f64,
    captures: f64,
    encoded: f64,
) -> f64 {
    primitive_array_jit_map_impl(value, callback, encoded, Some(captures))
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn dynamic_array_jit_map_impl(
    value: f64,
    callback: f64,
    target: f64,
    captures: Option<f64>,
) -> f64 {
    let target = target as u8;
    let Some(array) = dynamic_primitive(value, None) else {
        CALL_ERROR.with(|error| error.set(INVALID_DYNAMIC_VALUE.as_ptr().cast()));
        return 0.0;
    };
    let (source, element_tag) = match array.tag {
        DYNAMIC_NUMBER_ARRAY_TAG => (0, DYNAMIC_NUMBER_TAG),
        DYNAMIC_BOOLEAN_ARRAY_TAG => (1, DYNAMIC_BOOLEAN_TAG),
        DYNAMIC_STRING_ARRAY_TAG => (2, DYNAMIC_STRING_TAG),
        _ => {
            CALL_ERROR.with(|error| error.set(INVALID_DYNAMIC_VALUE.as_ptr().cast()));
            return 0.0;
        }
    };
    if target > 2 {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return 0.0;
    }
    let Some((callback, required)) = compile_jit_callback(callback) else {
        return 0.0;
    };
    let captures = if let Some(captures) = captures {
        let Some(captures) = capture_arguments(captures, 5, required) else {
            return 0.0;
        };
        captures
    } else {
        if required > 5 {
            CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
            return 0.0;
        }
        Vec::new()
    };
    let (Some(allocate), Some((data, length))) = (ARENA_ALLOC.with(Cell::get), unsafe {
        array_data(f64::from_bits(array.payload))
    }) else {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return 0.0;
    };
    let Some(size) = length.checked_mul(8).and_then(|size| size.checked_add(8)) else {
        CALL_ERROR.with(|error| error.set(ALLOCATION_FAILED.as_ptr().cast()));
        return 0.0;
    };
    let output = unsafe { allocate(size, 8) };
    if output.is_null() {
        CALL_ERROR.with(|error| error.set(ALLOCATION_FAILED.as_ptr().cast()));
        return 0.0;
    }
    unsafe { output.cast::<u64>().write(length as u64) };
    for index in 0..length {
        let element = unsafe { array_element(data, index, source) };
        let mapped = call_jit_callback(
            callback,
            &[
                element_tag as f64,
                element,
                index as f64,
                array.tag as f64,
                f64::from_bits(array.payload),
            ],
            &captures,
        );
        let mapped = if target == 1 {
            u64::from(mapped != 0.0 && !mapped.is_nan())
        } else {
            mapped.to_bits()
        };
        unsafe {
            output
                .add(8 + index * 8)
                .cast::<u64>()
                .write_unaligned(mapped)
        };
    }
    array_result(output)
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn dynamic_array_jit_map(value: f64, callback: f64, target: f64) -> f64 {
    dynamic_array_jit_map_impl(value, callback, target, None)
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn dynamic_array_jit_map_captured(
    value: f64,
    callback: f64,
    captures: f64,
    target: f64,
) -> f64 {
    dynamic_array_jit_map_impl(value, callback, target, Some(captures))
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn dynamic_array_jit_scan_impl(value: f64, callback: f64, mode: f64, captures: Option<f64>) -> f64 {
    let mode = mode as u8;
    let Some(array) = dynamic_primitive(value, None) else {
        CALL_ERROR.with(|error| error.set(INVALID_DYNAMIC_VALUE.as_ptr().cast()));
        return 0.0;
    };
    let (kind, element_tag) = match array.tag {
        DYNAMIC_NUMBER_ARRAY_TAG => (0, DYNAMIC_NUMBER_TAG),
        DYNAMIC_BOOLEAN_ARRAY_TAG => (1, DYNAMIC_BOOLEAN_TAG),
        DYNAMIC_STRING_ARRAY_TAG => (2, DYNAMIC_STRING_TAG),
        _ => {
            CALL_ERROR.with(|error| error.set(INVALID_DYNAMIC_VALUE.as_ptr().cast()));
            return 0.0;
        }
    };
    if mode > 6 {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return 0.0;
    }
    let Some((callback, required)) = compile_jit_callback(callback) else {
        return 0.0;
    };
    let captures = if let Some(captures) = captures {
        let Some(captures) = capture_arguments(captures, 5, required) else {
            return 0.0;
        };
        captures
    } else {
        if required > 5 {
            CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
            return 0.0;
        }
        Vec::new()
    };
    let result = primitive_array_scan(f64::from_bits(array.payload), kind, mode, |data, index| {
        let element = unsafe { array_element(data, index, kind) };
        let result = call_jit_callback(
            callback,
            &[
                element_tag as f64,
                element,
                index as f64,
                array.tag as f64,
                f64::from_bits(array.payload),
            ],
            &captures,
        );
        result != 0.0 && !result.is_nan()
    });
    dynamic_array_scan_result(array.tag, mode, result)
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn dynamic_array_jit_scan(value: f64, callback: f64, mode: f64) -> f64 {
    dynamic_array_jit_scan_impl(value, callback, mode, None)
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn dynamic_array_jit_scan_captured(
    value: f64,
    callback: f64,
    captures: f64,
    mode: f64,
) -> f64 {
    dynamic_array_jit_scan_impl(value, callback, mode, Some(captures))
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn primitive_array_jit_scan(value: f64, callback: f64, encoded: f64) -> f64 {
    let encoded = encoded as u8;
    let kind = encoded / 8;
    let mode = encoded % 8;
    if kind > 2 || mode > 6 {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return 0.0;
    }
    let Some((callback, required)) = compile_jit_callback(callback) else {
        return 0.0;
    };
    if required > 3 {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return 0.0;
    }
    primitive_array_scan(value, kind, mode, |array, index| {
        let element = unsafe { array_element(array, index, kind) };
        let result = callback([element, index as f64, value].as_ptr());
        result != 0.0 && !result.is_nan()
    })
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn primitive_array_jit_scan_captured(
    value: f64,
    callback: f64,
    captures: f64,
    encoded: f64,
) -> f64 {
    let encoded = encoded as u8;
    let kind = encoded / 8;
    let mode = encoded % 8;
    if kind > 2 || mode > 6 {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return 0.0;
    }
    let Some((callback, required)) = compile_jit_callback(callback) else {
        return 0.0;
    };
    let Some(captures) = capture_arguments(captures, 3, required) else {
        return 0.0;
    };
    primitive_array_scan(value, kind, mode, |array, index| {
        let element = unsafe { array_element(array, index, kind) };
        let result = call_jit_callback(callback, &[element, index as f64, value], &captures);
        result != 0.0 && !result.is_nan()
    })
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn number_array_jit_reduce(
    value: f64,
    initial: f64,
    callback: f64,
    from_right: bool,
    has_initial: bool,
    captures: Option<f64>,
) -> f64 {
    let Some((callback, required)) = compile_jit_callback(callback) else {
        return 0.0;
    };
    let captures = if let Some(captures) = captures {
        let Some(captures) = capture_arguments(captures, 4, required) else {
            return 0.0;
        };
        captures
    } else {
        if required > 4 {
            CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
            return 0.0;
        }
        Vec::new()
    };
    let Some((array, length)) = (unsafe { array_data(value) }) else {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return 0.0;
    };
    if !has_initial && length == 0 {
        CALL_ERROR.with(|error| error.set(EMPTY_REDUCE.as_ptr().cast()));
        return 0.0;
    }
    let mut accumulator = if has_initial {
        initial
    } else {
        let index = if from_right { length - 1 } else { 0 };
        unsafe { array.add(8 + index * 8).cast::<f64>().read_unaligned() }
    };
    let mut apply = |index| {
        let element = unsafe { array.add(8 + index * 8).cast::<f64>().read_unaligned() };
        accumulator = call_jit_callback(
            callback,
            &[accumulator, element, index as f64, value],
            &captures,
        );
    };
    if from_right {
        for index in (0..if has_initial { length } else { length - 1 }).rev() {
            apply(index);
        }
    } else {
        for index in if has_initial { 0 } else { 1 }..length {
            apply(index);
        }
    }
    accumulator
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
macro_rules! jit_reduce_fn {
    ($name:ident, $from_right:expr, $has_initial:expr) => {
        extern "C" fn $name(value: f64, initial: f64, callback: f64) -> f64 {
            number_array_jit_reduce(value, initial, callback, $from_right, $has_initial, None)
        }
    };
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
jit_reduce_fn!(number_array_jit_reduce_initial, false, true);
#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
jit_reduce_fn!(number_array_jit_reduce_first, false, false);
#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
jit_reduce_fn!(number_array_jit_reduce_right_initial, true, true);
#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
jit_reduce_fn!(number_array_jit_reduce_right_last, true, false);

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
macro_rules! jit_reduce_captured_fn {
    ($name:ident, $from_right:expr, $has_initial:expr) => {
        extern "C" fn $name(value: f64, initial: f64, callback: f64, captures: f64) -> f64 {
            number_array_jit_reduce(
                value,
                initial,
                callback,
                $from_right,
                $has_initial,
                Some(captures),
            )
        }
    };
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
jit_reduce_captured_fn!(number_array_jit_reduce_initial_captured, false, true);
#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
jit_reduce_captured_fn!(number_array_jit_reduce_first_captured, false, false);
#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
jit_reduce_captured_fn!(number_array_jit_reduce_right_initial_captured, true, true);
#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
jit_reduce_captured_fn!(number_array_jit_reduce_right_last_captured, true, false);

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn dynamic_array_jit_reduce(
    value: f64,
    initial: f64,
    callback: f64,
    from_right: bool,
    captures: Option<f64>,
) -> f64 {
    let Some(array) = dynamic_primitive(value, None) else {
        CALL_ERROR.with(|error| error.set(INVALID_DYNAMIC_VALUE.as_ptr().cast()));
        return 0.0;
    };
    let (kind, element_tag) = match array.tag {
        DYNAMIC_NUMBER_ARRAY_TAG => (0, DYNAMIC_NUMBER_TAG),
        DYNAMIC_BOOLEAN_ARRAY_TAG => (1, DYNAMIC_BOOLEAN_TAG),
        DYNAMIC_STRING_ARRAY_TAG => (2, DYNAMIC_STRING_TAG),
        _ => {
            CALL_ERROR.with(|error| error.set(INVALID_DYNAMIC_VALUE.as_ptr().cast()));
            return 0.0;
        }
    };
    let Some((callback, required)) = compile_jit_callback(callback) else {
        return 0.0;
    };
    let captures = if let Some(captures) = captures {
        let Some(captures) = capture_arguments(captures, 6, required) else {
            return 0.0;
        };
        captures
    } else {
        if required > 6 {
            CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
            return 0.0;
        }
        Vec::new()
    };
    let Some((data, length)) = (unsafe { array_data(f64::from_bits(array.payload)) }) else {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return 0.0;
    };
    let mut accumulator = initial;
    let mut apply = |index| {
        let element = unsafe { array_element(data, index, kind) };
        accumulator = call_jit_callback(
            callback,
            &[
                accumulator,
                element_tag as f64,
                element,
                index as f64,
                array.tag as f64,
                f64::from_bits(array.payload),
            ],
            &captures,
        );
    };
    if from_right {
        for index in (0..length).rev() {
            apply(index);
        }
    } else {
        for index in 0..length {
            apply(index);
        }
    }
    accumulator
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn dynamic_array_jit_reduce_unseeded(
    value: f64,
    callback: f64,
    from_right: bool,
    captures: Option<f64>,
) -> f64 {
    let Some(array) = dynamic_primitive(value, None) else {
        CALL_ERROR.with(|error| error.set(INVALID_DYNAMIC_VALUE.as_ptr().cast()));
        return 0.0;
    };
    let (kind, element_tag) = match array.tag {
        DYNAMIC_NUMBER_ARRAY_TAG => (0, DYNAMIC_NUMBER_TAG),
        DYNAMIC_BOOLEAN_ARRAY_TAG => (1, DYNAMIC_BOOLEAN_TAG),
        DYNAMIC_STRING_ARRAY_TAG => (2, DYNAMIC_STRING_TAG),
        _ => {
            CALL_ERROR.with(|error| error.set(INVALID_DYNAMIC_VALUE.as_ptr().cast()));
            return 0.0;
        }
    };
    let Some((callback, required)) = compile_jit_callback(callback) else {
        return 0.0;
    };
    let captures = if let Some(captures) = captures {
        let Some(captures) = capture_arguments(captures, 7, required) else {
            return 0.0;
        };
        captures
    } else {
        if required > 7 {
            CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
            return 0.0;
        }
        Vec::new()
    };
    let Some((data, length)) = (unsafe { array_data(f64::from_bits(array.payload)) }) else {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return 0.0;
    };
    if length == 0 {
        CALL_ERROR.with(|error| error.set(EMPTY_REDUCE.as_ptr().cast()));
        return 0.0;
    }
    let first = if from_right { length - 1 } else { 0 };
    let first_payload = unsafe { array_element(data, first, kind) };
    let mut accumulator = DynamicPrimitive {
        tag: element_tag,
        payload: if element_tag == DYNAMIC_BOOLEAN_TAG {
            u64::from(first_payload != 0.0)
        } else {
            first_payload.to_bits()
        },
    };
    let mut apply = |index| -> Option<()> {
        let element = unsafe { array_element(data, index, kind) };
        let result = call_jit_callback(
            callback,
            &[
                accumulator.tag as f64,
                f64::from_bits(accumulator.payload),
                element_tag as f64,
                element,
                index as f64,
                array.tag as f64,
                f64::from_bits(array.payload),
            ],
            &captures,
        );
        let result = dynamic_primitive(result, None)?;
        accumulator = DynamicPrimitive {
            tag: result.tag,
            payload: result.payload,
        };
        Some(())
    };
    if from_right {
        for index in (0..length - 1).rev() {
            if apply(index).is_none() {
                CALL_ERROR.with(|error| error.set(INVALID_DYNAMIC_VALUE.as_ptr().cast()));
                return 0.0;
            }
        }
    } else {
        for index in 1..length {
            if apply(index).is_none() {
                CALL_ERROR.with(|error| error.set(INVALID_DYNAMIC_VALUE.as_ptr().cast()));
                return 0.0;
            }
        }
    }
    arena_dynamic(accumulator.tag, accumulator.payload)
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn dynamic_array_jit_reduce_left(value: f64, initial: f64, callback: f64) -> f64 {
    dynamic_array_jit_reduce(value, initial, callback, false, None)
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn dynamic_array_jit_reduce_right(value: f64, initial: f64, callback: f64) -> f64 {
    dynamic_array_jit_reduce(value, initial, callback, true, None)
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn dynamic_array_jit_reduce_left_captured(
    value: f64,
    initial: f64,
    callback: f64,
    captures: f64,
) -> f64 {
    dynamic_array_jit_reduce(value, initial, callback, false, Some(captures))
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn dynamic_array_jit_reduce_right_captured(
    value: f64,
    initial: f64,
    callback: f64,
    captures: f64,
) -> f64 {
    dynamic_array_jit_reduce(value, initial, callback, true, Some(captures))
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn dynamic_array_jit_reduce_left_unseeded(
    value: f64,
    _initial: f64,
    callback: f64,
) -> f64 {
    dynamic_array_jit_reduce_unseeded(value, callback, false, None)
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn dynamic_array_jit_reduce_right_unseeded(
    value: f64,
    _initial: f64,
    callback: f64,
) -> f64 {
    dynamic_array_jit_reduce_unseeded(value, callback, true, None)
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn dynamic_array_jit_reduce_left_unseeded_captured(
    value: f64,
    _initial: f64,
    callback: f64,
    captures: f64,
) -> f64 {
    dynamic_array_jit_reduce_unseeded(value, callback, false, Some(captures))
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn dynamic_array_jit_reduce_right_unseeded_captured(
    value: f64,
    _initial: f64,
    callback: f64,
    captures: f64,
) -> f64 {
    dynamic_array_jit_reduce_unseeded(value, callback, true, Some(captures))
}

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

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn dynamic_array_result(tag: u64, result: f64) -> f64 {
    let bits = result.to_bits();
    if bits & ARRAY_RESULT_TAG == 0 {
        return 0.0;
    }
    let Some(handle) = ARENA_ALLOC.with(|allocator| {
        allocator.get().map(|alloc| unsafe {
            alloc(std::mem::size_of::<usize>(), std::mem::align_of::<usize>())
        })
    }) else {
        CALL_ERROR.with(|error| error.set(ALLOCATION_FAILED.as_ptr().cast()));
        return 0.0;
    };
    if handle.is_null() {
        CALL_ERROR.with(|error| error.set(ALLOCATION_FAILED.as_ptr().cast()));
        return 0.0;
    }
    unsafe {
        handle
            .cast::<usize>()
            .write(bits as usize & !ARRAY_RESULT_TAG as usize)
    };
    dynamic_from_parts(tag as f64, f64::from_bits(handle as usize as u64))
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn dynamic_array_slice(value: f64, start: f64, end: f64) -> f64 {
    dynamic_primitive(value, None).map_or_else(
        || {
            CALL_ERROR.with(|error| error.set(INVALID_DYNAMIC_VALUE.as_ptr().cast()));
            0.0
        },
        |dynamic| {
            if !matches!(
                dynamic.tag,
                DYNAMIC_NUMBER_ARRAY_TAG..=DYNAMIC_STRING_ARRAY_TAG
            ) {
                CALL_ERROR.with(|error| error.set(INVALID_DYNAMIC_VALUE.as_ptr().cast()));
                return 0.0;
            }
            let result = array_slice(f64::from_bits(dynamic.payload), start, end);
            dynamic_array_result(dynamic.tag, result)
        },
    )
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn array_slice(value: f64, start: f64, end: f64) -> f64 {
    let (Some(slice), Some((data, _))) =
        (ARRAY_SLICE.with(Cell::get), unsafe { array_data(value) })
    else {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return 0.0;
    };
    let result = unsafe { slice(data, 8, start, end) };
    if result.is_null() {
        CALL_ERROR.with(|error| error.set(ALLOCATION_FAILED.as_ptr().cast()));
        0.0
    } else {
        array_result(result)
    }
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn array_concat(left: f64, right: f64) -> f64 {
    let (Some(concat), Some((left, _)), Some((right, _))) = (
        ARRAY_CONCAT.with(Cell::get),
        unsafe { array_data(left) },
        unsafe { array_data(right) },
    ) else {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return 0.0;
    };
    let result = unsafe { concat(left, right, 8) };
    if result.is_null() {
        CALL_ERROR.with(|error| error.set(ALLOCATION_FAILED.as_ptr().cast()));
        0.0
    } else {
        array_result(result)
    }
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn dynamic_array_concat(left: f64, right: f64) -> f64 {
    let (Some(left), Some(right)) = (
        dynamic_primitive(left, None),
        dynamic_primitive(right, None),
    ) else {
        CALL_ERROR.with(|error| error.set(INVALID_DYNAMIC_VALUE.as_ptr().cast()));
        return 0.0;
    };
    if left.tag != right.tag
        || !matches!(
            left.tag,
            DYNAMIC_NUMBER_ARRAY_TAG..=DYNAMIC_STRING_ARRAY_TAG
        )
    {
        CALL_ERROR.with(|error| error.set(INVALID_DYNAMIC_VALUE.as_ptr().cast()));
        return 0.0;
    }
    let result = array_concat(f64::from_bits(left.payload), f64::from_bits(right.payload));
    dynamic_array_result(left.tag, result)
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn array_append(operation: u8, array: f64, value: f64) -> f64 {
    let (Some(append), Some((array, _))) =
        (ARRAY_APPEND.with(Cell::get), unsafe { array_data(array) })
    else {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return 0.0;
    };
    let result = unsafe { append(operation, array, value) };
    if result.is_null() {
        CALL_ERROR.with(|error| error.set(ALLOCATION_FAILED.as_ptr().cast()));
        0.0
    } else {
        array_result(result)
    }
}

macro_rules! array_append_fn {
    ($name:ident, $operation:expr) => {
        #[cfg(all(target_arch = "x86_64", target_family = "unix"))]
        extern "C" fn $name(array: f64, value: f64) -> f64 {
            array_append($operation, array, value)
        }
    };
}

array_append_fn!(number_array_append, 0);
array_append_fn!(string_array_append, 1);
array_append_fn!(bool_array_append, 2);

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn dynamic_array_append(array: f64, value: f64) -> f64 {
    let (Some(array), Some(value)) = (
        dynamic_primitive(array, None),
        dynamic_primitive(value, None),
    ) else {
        CALL_ERROR.with(|error| error.set(INVALID_DYNAMIC_VALUE.as_ptr().cast()));
        return 0.0;
    };
    let operation = match (array.tag, value.tag) {
        (DYNAMIC_NUMBER_ARRAY_TAG, DYNAMIC_NUMBER_TAG) => 0,
        (DYNAMIC_STRING_ARRAY_TAG, DYNAMIC_STRING_TAG) => 1,
        (DYNAMIC_BOOLEAN_ARRAY_TAG, DYNAMIC_BOOLEAN_TAG) => 2,
        _ => {
            CALL_ERROR.with(|error| error.set(INVALID_DYNAMIC_VALUE.as_ptr().cast()));
            return 0.0;
        }
    };
    dynamic_array_result(
        array.tag,
        array_append(
            operation,
            f64::from_bits(array.payload),
            f64::from_bits(value.payload),
        ),
    )
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn array_to_reversed(value: f64) -> f64 {
    let (Some(reverse), Some((data, _))) = (ARRAY_TO_REVERSED.with(Cell::get), unsafe {
        array_data(value)
    }) else {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return 0.0;
    };
    let result = unsafe { reverse(data, 8) };
    if result.is_null() {
        CALL_ERROR.with(|error| error.set(ALLOCATION_FAILED.as_ptr().cast()));
        0.0
    } else {
        array_result(result)
    }
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn array_reverse(value: f64) -> f64 {
    let (Some(reverse), Some((data, _))) =
        (ARRAY_REVERSE.with(Cell::get), unsafe { array_data(value) })
    else {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return 0.0;
    };
    let result = unsafe { reverse(data.cast_mut(), 8) };
    if result.is_null() {
        CALL_ERROR.with(|error| error.set(ALLOCATION_FAILED.as_ptr().cast()));
        0.0
    } else {
        array_result(result)
    }
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn dynamic_array_reverse(value: f64, reverse: extern "C" fn(f64) -> f64) -> f64 {
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
    let result = reverse(f64::from_bits(dynamic.payload));
    dynamic_array_result(dynamic.tag, result)
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn dynamic_array_to_reversed(value: f64) -> f64 {
    dynamic_array_reverse(value, array_to_reversed)
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn dynamic_array_reverse_in_place(value: f64) -> f64 {
    dynamic_array_reverse(value, array_reverse)
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn array_to_sorted(operation: u8, value: f64) -> f64 {
    let (Some(sort), Some((data, _))) = (ARRAY_TO_SORTED.with(Cell::get), unsafe {
        array_data(value)
    }) else {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return 0.0;
    };
    let result = unsafe { sort(operation, data) };
    if result.is_null() {
        CALL_ERROR.with(|error| error.set(ALLOCATION_FAILED.as_ptr().cast()));
        0.0
    } else {
        array_result(result)
    }
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn number_array_to_sorted(value: f64) -> f64 {
    array_to_sorted(0, value)
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn string_array_to_sorted(value: f64) -> f64 {
    array_to_sorted(1, value)
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn bool_array_to_sorted(value: f64) -> f64 {
    array_to_sorted(2, value)
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn number_array_to_sorted_ascending(value: f64) -> f64 {
    array_to_sorted(3, value)
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn number_array_to_sorted_descending(value: f64) -> f64 {
    array_to_sorted(4, value)
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn string_array_to_sorted_descending(value: f64) -> f64 {
    array_to_sorted(5, value)
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn array_sort(operation: u8, value: f64) -> f64 {
    let (Some(sort), Some((data, _))) = (ARRAY_SORT.with(Cell::get), unsafe { array_data(value) })
    else {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return 0.0;
    };
    let result = unsafe { sort(operation, data.cast_mut()) };
    if result.is_null() {
        CALL_ERROR.with(|error| error.set(ALLOCATION_FAILED.as_ptr().cast()));
        0.0
    } else {
        array_result(result)
    }
}

macro_rules! array_sort_fn {
    ($name:ident, $operation:expr) => {
        #[cfg(all(target_arch = "x86_64", target_family = "unix"))]
        extern "C" fn $name(value: f64) -> f64 {
            array_sort($operation, value)
        }
    };
}

array_sort_fn!(number_array_sort, 0);
array_sort_fn!(string_array_sort, 1);
array_sort_fn!(bool_array_sort, 2);
array_sort_fn!(number_array_sort_ascending, 3);
array_sort_fn!(number_array_sort_descending, 4);
array_sort_fn!(string_array_sort_descending, 5);

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn dynamic_array_sort(value: f64, in_place: bool) -> f64 {
    let Some(dynamic) = dynamic_primitive(value, None) else {
        CALL_ERROR.with(|error| error.set(INVALID_DYNAMIC_VALUE.as_ptr().cast()));
        return 0.0;
    };
    let operation = match dynamic.tag {
        DYNAMIC_NUMBER_ARRAY_TAG => 0,
        DYNAMIC_STRING_ARRAY_TAG => 1,
        DYNAMIC_BOOLEAN_ARRAY_TAG => 2,
        _ => {
            CALL_ERROR.with(|error| error.set(INVALID_DYNAMIC_VALUE.as_ptr().cast()));
            return 0.0;
        }
    };
    let value = f64::from_bits(dynamic.payload);
    let result = if in_place {
        array_sort(operation, value)
    } else {
        array_to_sorted(operation, value)
    };
    dynamic_array_result(dynamic.tag, result)
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn dynamic_array_to_sorted(value: f64) -> f64 {
    dynamic_array_sort(value, false)
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn dynamic_array_sort_in_place(value: f64) -> f64 {
    dynamic_array_sort(value, true)
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn array_fill(operation: u8, array: f64, value: f64, start: f64, end: f64) -> f64 {
    let (Some(fill), Some((data, _))) = (ARRAY_FILL.with(Cell::get), unsafe { array_data(array) })
    else {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return 0.0;
    };
    let result = unsafe { fill(operation, data.cast_mut(), value, start, end) };
    if result.is_null() {
        CALL_ERROR.with(|error| error.set(ALLOCATION_FAILED.as_ptr().cast()));
        0.0
    } else {
        array_result(result)
    }
}

macro_rules! array_fill_fn {
    ($name:ident, $operation:expr) => {
        #[cfg(all(target_arch = "x86_64", target_family = "unix"))]
        extern "C" fn $name(array: f64, value: f64, start: f64, end: f64) -> f64 {
            array_fill($operation, array, value, start, end)
        }
    };
}

array_fill_fn!(number_array_fill, 0);
array_fill_fn!(string_array_fill, 1);
array_fill_fn!(bool_array_fill, 2);

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn dynamic_array_fill(array: f64, value: f64, start: f64, end: f64) -> f64 {
    let (Some(array), Some(value)) = (
        dynamic_primitive(array, None),
        dynamic_primitive(value, None),
    ) else {
        CALL_ERROR.with(|error| error.set(INVALID_DYNAMIC_VALUE.as_ptr().cast()));
        return 0.0;
    };
    let operation = match (array.tag, value.tag) {
        (DYNAMIC_NUMBER_ARRAY_TAG, DYNAMIC_NUMBER_TAG) => 0,
        (DYNAMIC_STRING_ARRAY_TAG, DYNAMIC_STRING_TAG) => 1,
        (DYNAMIC_BOOLEAN_ARRAY_TAG, DYNAMIC_BOOLEAN_TAG) => 2,
        _ => {
            CALL_ERROR.with(|error| error.set(INVALID_DYNAMIC_VALUE.as_ptr().cast()));
            return 0.0;
        }
    };
    dynamic_array_result(
        array.tag,
        array_fill(
            operation,
            f64::from_bits(array.payload),
            f64::from_bits(value.payload),
            start,
            end,
        ),
    )
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn array_copy_within(array: f64, target: f64, start: f64, end: f64) -> f64 {
    let (Some(copy), Some((data, _))) = (ARRAY_COPY_WITHIN.with(Cell::get), unsafe {
        array_data(array)
    }) else {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return 0.0;
    };
    let result = unsafe { copy(data.cast_mut(), 8, target, start, end) };
    if result.is_null() {
        CALL_ERROR.with(|error| error.set(ALLOCATION_FAILED.as_ptr().cast()));
        0.0
    } else {
        array_result(result)
    }
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn dynamic_array_copy_within(array: f64, target: f64, start: f64, end: f64) -> f64 {
    let Some(array) = dynamic_primitive(array, None) else {
        CALL_ERROR.with(|error| error.set(INVALID_DYNAMIC_VALUE.as_ptr().cast()));
        return 0.0;
    };
    if !matches!(
        array.tag,
        DYNAMIC_NUMBER_ARRAY_TAG..=DYNAMIC_STRING_ARRAY_TAG
    ) {
        CALL_ERROR.with(|error| error.set(INVALID_DYNAMIC_VALUE.as_ptr().cast()));
        return 0.0;
    }
    dynamic_array_result(
        array.tag,
        array_copy_within(f64::from_bits(array.payload), target, start, end),
    )
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn dynamic_array_with(array: f64, index: f64, value: f64) -> f64 {
    let (Some(array), Some(value)) = (
        dynamic_primitive(array, None),
        dynamic_primitive(value, None),
    ) else {
        CALL_ERROR.with(|error| error.set(INVALID_DYNAMIC_VALUE.as_ptr().cast()));
        return 0.0;
    };
    let operation = match (array.tag, value.tag) {
        (DYNAMIC_NUMBER_ARRAY_TAG, DYNAMIC_NUMBER_TAG) => 0,
        (DYNAMIC_STRING_ARRAY_TAG, DYNAMIC_STRING_TAG) => 1,
        (DYNAMIC_BOOLEAN_ARRAY_TAG, DYNAMIC_BOOLEAN_TAG) => 2,
        _ => {
            CALL_ERROR.with(|error| error.set(INVALID_DYNAMIC_VALUE.as_ptr().cast()));
            return 0.0;
        }
    };
    dynamic_array_result(
        array.tag,
        array_with(
            operation,
            f64::from_bits(array.payload),
            index,
            f64::from_bits(value.payload),
        ),
    )
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn array_push(operation: u8, array: f64, value: f64) -> f64 {
    let Some(push) = ARRAY_PUSH.with(Cell::get) else {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return 0.0;
    };
    let bits = array.to_bits();
    if bits & ARRAY_RESULT_TAG != 0 {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return 0.0;
    }
    let result = unsafe { push(operation, bits as usize as *mut *mut u8, value) };
    if result < 0.0 {
        CALL_ERROR.with(|error| error.set(ALLOCATION_FAILED.as_ptr().cast()));
        0.0
    } else {
        result
    }
}

macro_rules! array_push_fn {
    ($name:ident, $operation:expr) => {
        #[cfg(all(target_arch = "x86_64", target_family = "unix"))]
        extern "C" fn $name(array: f64, value: f64) -> f64 {
            array_push($operation, array, value)
        }
    };
}

array_push_fn!(number_array_push, 0);
array_push_fn!(string_array_push, 1);
array_push_fn!(bool_array_push, 2);

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn dynamic_array_insert(array: f64, value: f64, unshift: bool) -> f64 {
    let (Some(array), Some(value)) = (
        dynamic_primitive(array, None),
        dynamic_primitive(value, None),
    ) else {
        CALL_ERROR.with(|error| error.set(INVALID_DYNAMIC_VALUE.as_ptr().cast()));
        return 0.0;
    };
    let operation = match (array.tag, value.tag) {
        (DYNAMIC_NUMBER_ARRAY_TAG, DYNAMIC_NUMBER_TAG) => 0,
        (DYNAMIC_STRING_ARRAY_TAG, DYNAMIC_STRING_TAG) => 1,
        (DYNAMIC_BOOLEAN_ARRAY_TAG, DYNAMIC_BOOLEAN_TAG) => 2,
        _ => {
            CALL_ERROR.with(|error| error.set(INVALID_DYNAMIC_VALUE.as_ptr().cast()));
            return 0.0;
        }
    };
    let array = f64::from_bits(array.payload);
    let value = f64::from_bits(value.payload);
    if unshift {
        array_unshift(operation, array, value)
    } else {
        array_push(operation, array, value)
    }
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn dynamic_array_push(array: f64, value: f64) -> f64 {
    dynamic_array_insert(array, value, false)
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn dynamic_array_unshift(array: f64, value: f64) -> f64 {
    dynamic_array_insert(array, value, true)
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn array_unshift(operation: u8, array: f64, value: f64) -> f64 {
    let Some(unshift) = ARRAY_UNSHIFT.with(Cell::get) else {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return 0.0;
    };
    let bits = array.to_bits();
    if bits & ARRAY_RESULT_TAG != 0 {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return 0.0;
    }
    let result = unsafe { unshift(operation, bits as usize as *mut *mut u8, value) };
    if result < 0.0 {
        CALL_ERROR.with(|error| error.set(ALLOCATION_FAILED.as_ptr().cast()));
        0.0
    } else {
        result
    }
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn array_set(operation: u8, array: f64, index: f64, value: f64) -> f64 {
    let Some(set) = ARRAY_SET.with(Cell::get) else {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return 0.0;
    };
    let bits = array.to_bits();
    if bits & ARRAY_RESULT_TAG != 0
        || unsafe { set(operation, bits as usize as *mut *mut u8, index, value) } != 1
    {
        CALL_ERROR.with(|error| error.set(ALLOCATION_FAILED.as_ptr().cast()));
        0.0
    } else {
        value
    }
}

macro_rules! array_set_fn {
    ($name:ident, $operation:expr) => {
        #[cfg(all(target_arch = "x86_64", target_family = "unix"))]
        extern "C" fn $name(array: f64, index: f64, value: f64) -> f64 {
            array_set($operation, array, index, value)
        }
    };
}

array_set_fn!(number_array_set, 0);
array_set_fn!(string_array_set, 1);
array_set_fn!(bool_array_set, 2);

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn dynamic_array_set(array: f64, index: f64, value: f64) -> f64 {
    let (Some(array), Some(dynamic_value)) = (
        dynamic_primitive(array, None),
        dynamic_primitive(value, None),
    ) else {
        CALL_ERROR.with(|error| error.set(INVALID_DYNAMIC_VALUE.as_ptr().cast()));
        return 0.0;
    };
    let operation = match (array.tag, dynamic_value.tag) {
        (DYNAMIC_NUMBER_ARRAY_TAG, DYNAMIC_NUMBER_TAG) => 0,
        (DYNAMIC_STRING_ARRAY_TAG, DYNAMIC_STRING_TAG) => 1,
        (DYNAMIC_BOOLEAN_ARRAY_TAG, DYNAMIC_BOOLEAN_TAG) => 2,
        _ => {
            CALL_ERROR.with(|error| error.set(INVALID_DYNAMIC_VALUE.as_ptr().cast()));
            return 0.0;
        }
    };
    array_set(
        operation,
        f64::from_bits(array.payload),
        index,
        f64::from_bits(dynamic_value.payload),
    );
    value
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn number_source(source: &'static std::thread::LocalKey<Cell<Option<NumberSource>>>) -> f64 {
    let Some(source) = source.with(Cell::get) else {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return 0.0;
    };
    unsafe { source() }
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn math_random() -> f64 {
    number_source(&MATH_RANDOM)
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn date_now() -> f64 {
    number_source(&DATE_NOW)
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn performance_now() -> f64 {
    number_source(&PERFORMANCE_NOW)
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn process_pid() -> f64 {
    number_source(&PROCESS_PID)
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn process_ppid() -> f64 {
    number_source(&PROCESS_PPID)
}

macro_rules! array_unshift_fn {
    ($name:ident, $operation:expr) => {
        #[cfg(all(target_arch = "x86_64", target_family = "unix"))]
        extern "C" fn $name(array: f64, value: f64) -> f64 {
            array_unshift($operation, array, value)
        }
    };
}

array_unshift_fn!(number_array_unshift, 0);
array_unshift_fn!(string_array_unshift, 1);
array_unshift_fn!(bool_array_unshift, 2);

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn array_remove(operation: u8, array: f64) -> f64 {
    let Some(remove) = ARRAY_REMOVE.with(Cell::get) else {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return 0.0;
    };
    let bits = array.to_bits();
    if bits & ARRAY_RESULT_TAG != 0 {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return 0.0;
    }
    let mut value = 0.0;
    match unsafe { remove(operation, bits as usize as *mut *mut u8, &mut value) } {
        1 => value,
        0 => {
            CALL_PRESENT.with(|present| present.set(false));
            0.0
        }
        _ => {
            CALL_ERROR.with(|error| error.set(ALLOCATION_FAILED.as_ptr().cast()));
            0.0
        }
    }
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn array_splice(array: f64, start: f64, delete_count: f64, inserts: f64) -> f64 {
    let Some(splice) = ARRAY_SPLICE.with(Cell::get) else {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return 0.0;
    };
    let bits = array.to_bits();
    let Some((inserts, _)) = (unsafe { array_data(inserts) }) else {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return 0.0;
    };
    if bits & ARRAY_RESULT_TAG != 0 {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return 0.0;
    }
    let removed = unsafe { splice(bits as usize as *mut *mut u8, start, delete_count, inserts) };
    if removed.is_null() {
        CALL_ERROR.with(|error| error.set(ALLOCATION_FAILED.as_ptr().cast()));
        0.0
    } else {
        array_result(removed)
    }
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn array_to_spliced(array: f64, start: f64, delete_count: f64, inserts: f64) -> f64 {
    let Some(splice) = ARRAY_SPLICE.with(Cell::get) else {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return 0.0;
    };
    let (Some((array, _)), Some((inserts, _))) =
        (unsafe { array_data(array) }, unsafe { array_data(inserts) })
    else {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return 0.0;
    };
    let mut output = array.cast_mut();
    let removed = unsafe { splice(&mut output, start, delete_count, inserts) };
    if removed.is_null() {
        CALL_ERROR.with(|error| error.set(ALLOCATION_FAILED.as_ptr().cast()));
        0.0
    } else {
        array_result(output)
    }
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn dynamic_array_splice(
    array: f64,
    start: f64,
    delete_count: f64,
    inserts: f64,
    copy: bool,
) -> f64 {
    let (Some(array), Some(inserts)) = (
        dynamic_primitive(array, None),
        dynamic_primitive(inserts, None),
    ) else {
        CALL_ERROR.with(|error| error.set(INVALID_DYNAMIC_VALUE.as_ptr().cast()));
        return 0.0;
    };
    if array.tag != inserts.tag
        || !matches!(
            array.tag,
            DYNAMIC_NUMBER_ARRAY_TAG..=DYNAMIC_STRING_ARRAY_TAG
        )
    {
        CALL_ERROR.with(|error| error.set(INVALID_DYNAMIC_VALUE.as_ptr().cast()));
        return 0.0;
    }
    let splice = if copy { array_to_spliced } else { array_splice };
    dynamic_array_result(
        array.tag,
        splice(
            f64::from_bits(array.payload),
            start,
            delete_count,
            f64::from_bits(inserts.payload),
        ),
    )
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn dynamic_array_splice_in_place(
    array: f64,
    start: f64,
    delete_count: f64,
    inserts: f64,
) -> f64 {
    dynamic_array_splice(array, start, delete_count, inserts, false)
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn dynamic_array_to_spliced(
    array: f64,
    start: f64,
    delete_count: f64,
    inserts: f64,
) -> f64 {
    dynamic_array_splice(array, start, delete_count, inserts, true)
}

macro_rules! array_remove_fn {
    ($name:ident, $operation:expr) => {
        #[cfg(all(target_arch = "x86_64", target_family = "unix"))]
        extern "C" fn $name(array: f64) -> f64 {
            array_remove($operation, array)
        }
    };
}

array_remove_fn!(number_array_pop, 0);
array_remove_fn!(string_array_pop, 1);
array_remove_fn!(bool_array_pop, 2);
array_remove_fn!(number_array_shift, 3);
array_remove_fn!(string_array_shift, 4);
array_remove_fn!(bool_array_shift, 5);

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn dynamic_array_remove(array: f64, shift: bool) -> f64 {
    let Some(array) = dynamic_primitive(array, None) else {
        CALL_ERROR.with(|error| error.set(INVALID_DYNAMIC_VALUE.as_ptr().cast()));
        return 0.0;
    };
    let (operation, tag) = match array.tag {
        DYNAMIC_NUMBER_ARRAY_TAG => (0, DYNAMIC_NUMBER_TAG),
        DYNAMIC_STRING_ARRAY_TAG => (1, DYNAMIC_STRING_TAG),
        DYNAMIC_BOOLEAN_ARRAY_TAG => (2, DYNAMIC_BOOLEAN_TAG),
        _ => {
            CALL_ERROR.with(|error| error.set(INVALID_DYNAMIC_VALUE.as_ptr().cast()));
            return 0.0;
        }
    };
    let value = array_remove(
        operation + u8::from(shift) * 3,
        f64::from_bits(array.payload),
    );
    if CALL_PRESENT.with(Cell::get) {
        dynamic_from_parts(tag as f64, value)
    } else {
        0.0
    }
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn dynamic_array_pop(array: f64) -> f64 {
    dynamic_array_remove(array, false)
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn dynamic_array_shift(array: f64) -> f64 {
    dynamic_array_remove(array, true)
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn array_with(operation: u8, array: f64, index: f64, value: f64) -> f64 {
    let (Some(replace), Some((data, _))) =
        (ARRAY_WITH.with(Cell::get), unsafe { array_data(array) })
    else {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return 0.0;
    };
    let result = unsafe { replace(operation, data, index, value) };
    if result.is_null() {
        CALL_ERROR.with(|error| error.set(INVALID_ARRAY_WITH_INDEX.as_ptr().cast()));
        0.0
    } else {
        array_result(result)
    }
}

macro_rules! array_with_fn {
    ($name:ident, $operation:expr) => {
        #[cfg(all(target_arch = "x86_64", target_family = "unix"))]
        extern "C" fn $name(array: f64, index: f64, value: f64) -> f64 {
            array_with($operation, array, index, value)
        }
    };
}

array_with_fn!(number_array_with, 0);
array_with_fn!(string_array_with, 1);
array_with_fn!(bool_array_with, 2);

macro_rules! array_format_fn {
    ($name:ident, $operation:expr) => {
        #[cfg(all(target_arch = "x86_64", target_family = "unix"))]
        extern "C" fn $name(value: f64, separator: f64) -> f64 {
            array_format($operation, value, separator)
        }
    };
}

array_format_fn!(number_array_join, 0);
array_format_fn!(string_array_join, 1);
array_format_fn!(bool_array_join, 2);

macro_rules! array_search_fn {
    ($name:ident, $operation:expr) => {
        #[cfg(all(target_arch = "x86_64", target_family = "unix"))]
        extern "C" fn $name(value: f64, needle: f64, from_index: f64) -> f64 {
            array_search($operation, value, needle, from_index)
        }
    };
}

array_search_fn!(number_array_index_of, 0);
array_search_fn!(number_array_includes, 1);
array_search_fn!(string_array_index_of, 2);
array_search_fn!(string_array_includes, 3);
array_search_fn!(bool_array_index_of, 4);
array_search_fn!(bool_array_includes, 5);
array_search_fn!(number_array_last_index_of, 6);
array_search_fn!(string_array_last_index_of, 7);
array_search_fn!(bool_array_last_index_of, 8);

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn dynamic_array_search(operation: u8, value: f64, needle: f64, from_index: f64) -> f64 {
    let (Some(value), Some(needle)) = (
        dynamic_primitive(value, None),
        dynamic_primitive(needle, None),
    ) else {
        CALL_ERROR.with(|error| error.set(INVALID_DYNAMIC_VALUE.as_ptr().cast()));
        return 0.0;
    };
    let search = match (value.tag, needle.tag, operation) {
        (DYNAMIC_NUMBER_ARRAY_TAG, DYNAMIC_NUMBER_TAG, 0..=1) => operation,
        (DYNAMIC_STRING_ARRAY_TAG, DYNAMIC_STRING_TAG, 0..=1) => operation + 2,
        (DYNAMIC_BOOLEAN_ARRAY_TAG, DYNAMIC_BOOLEAN_TAG, 0..=1) => operation + 4,
        (DYNAMIC_NUMBER_ARRAY_TAG, DYNAMIC_NUMBER_TAG, 2) => 6,
        (DYNAMIC_STRING_ARRAY_TAG, DYNAMIC_STRING_TAG, 2) => 7,
        (DYNAMIC_BOOLEAN_ARRAY_TAG, DYNAMIC_BOOLEAN_TAG, 2) => 8,
        _ => {
            CALL_ERROR.with(|error| error.set(INVALID_DYNAMIC_VALUE.as_ptr().cast()));
            return 0.0;
        }
    };
    array_search(
        search,
        f64::from_bits(value.payload),
        f64::from_bits(needle.payload),
        from_index,
    )
}

macro_rules! dynamic_array_search_fn {
    ($name:ident, $operation:expr) => {
        #[cfg(all(target_arch = "x86_64", target_family = "unix"))]
        extern "C" fn $name(value: f64, needle: f64, from_index: f64) -> f64 {
            dynamic_array_search($operation, value, needle, from_index)
        }
    };
}

dynamic_array_search_fn!(dynamic_array_index_of, 0);
dynamic_array_search_fn!(dynamic_array_includes, 1);
dynamic_array_search_fn!(dynamic_array_last_index_of, 2);

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn string_truthy(value: f64) -> f64 {
    let value = value.to_bits() as usize as *const c_char;
    unsafe { (!value.is_null() && !CStr::from_ptr(value).to_bytes().is_empty()) as u8 as f64 }
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
extern "C" fn number_same_value(left: f64, right: f64) -> f64 {
    f64::from(u8::from(
        (left.is_nan() && right.is_nan()) || left.to_bits() == right.to_bits(),
    ))
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn string_same_value(left: f64, right: f64) -> f64 {
    f64::from(u8::from(unsafe {
        string_argument(left) == string_argument(right)
    }))
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn reference_same_value(left: f64, right: f64) -> f64 {
    f64::from(u8::from(left.to_bits() == right.to_bits()))
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn type_of_number(_: f64) -> f64 {
    f64::from_bits(c"number".as_ptr() as usize as u64)
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn type_of_boolean(_: f64) -> f64 {
    f64::from_bits(c"boolean".as_ptr() as usize as u64)
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn type_of_string(_: f64) -> f64 {
    f64::from_bits(c"string".as_ptr() as usize as u64)
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn type_of_object(_: f64) -> f64 {
    f64::from_bits(c"object".as_ptr() as usize as u64)
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn string_is_well_formed(_: f64) -> f64 {
    1.0
}
