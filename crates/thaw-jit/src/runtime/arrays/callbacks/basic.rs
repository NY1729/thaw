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

