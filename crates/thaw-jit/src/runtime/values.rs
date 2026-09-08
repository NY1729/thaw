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

struct DynamicPrimitive {
    tag: u64,
    payload: u64,
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn arena_dynamic(tag: u64, payload: u64) -> f64 {
    let Some(output) = ARENA_ALLOC.with(|allocator| {
        allocator.get().map(|alloc| unsafe {
            alloc(
                std::mem::size_of::<DynamicPrimitive>(),
                std::mem::align_of::<DynamicPrimitive>(),
            )
        })
    }) else {
        CALL_ERROR.with(|error| error.set(ALLOCATION_FAILED.as_ptr().cast()));
        return 0.0;
    };
    let Some(output) = (!output.is_null()).then(|| output.cast::<DynamicPrimitive>()) else {
        CALL_ERROR.with(|error| error.set(ALLOCATION_FAILED.as_ptr().cast()));
        return 0.0;
    };
    unsafe { output.write(DynamicPrimitive { tag, payload }) };
    DYNAMIC_VALUES.with(|values| {
        values.borrow_mut().insert(output as usize);
    });
    f64::from_bits(output as usize as u64)
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn tag_number(value: f64) -> f64 {
    arena_dynamic(DYNAMIC_NUMBER_TAG, value.to_bits())
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn tag_string(value: f64) -> f64 {
    arena_dynamic(DYNAMIC_STRING_TAG, value.to_bits())
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn tag_boolean(value: f64) -> f64 {
    arena_dynamic(DYNAMIC_BOOLEAN_TAG, u64::from(value != 0.0))
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn tag_aggregate(value: f64, tag: u64) -> f64 {
    let value = if matches!(tag, DYNAMIC_NUMBER_ARRAY_TAG..=DYNAMIC_STRING_ARRAY_TAG) {
        mutable_array_handle(value)
    } else {
        value
    };
    if CALL_ERROR.with(Cell::get).is_null() {
        arena_dynamic(tag, value.to_bits())
    } else {
        0.0
    }
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
macro_rules! aggregate_tagger {
    ($name:ident, $tag:expr) => {
        extern "C" fn $name(value: f64) -> f64 {
            tag_aggregate(value, $tag)
        }
    };
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
aggregate_tagger!(tag_number_array, DYNAMIC_NUMBER_ARRAY_TAG);
#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
aggregate_tagger!(tag_boolean_array, DYNAMIC_BOOLEAN_ARRAY_TAG);
#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
aggregate_tagger!(tag_string_array, DYNAMIC_STRING_ARRAY_TAG);
#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
aggregate_tagger!(tag_number_dictionary, DYNAMIC_NUMBER_DICTIONARY_TAG);
#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
aggregate_tagger!(tag_boolean_dictionary, DYNAMIC_BOOLEAN_DICTIONARY_TAG);
#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
aggregate_tagger!(tag_string_dictionary, DYNAMIC_STRING_DICTIONARY_TAG);
#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
aggregate_tagger!(tag_object, DYNAMIC_OBJECT_TAG);
#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
aggregate_tagger!(tag_tuple, DYNAMIC_TUPLE_TAG);

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn dynamic_from_parts(tag: f64, payload: f64) -> f64 {
    let tag = tag as u64;
    if !(DYNAMIC_NUMBER_TAG..=DYNAMIC_TUPLE_TAG).contains(&tag) {
        CALL_ERROR.with(|error| error.set(INVALID_DYNAMIC_VALUE.as_ptr().cast()));
        return 0.0;
    }
    arena_dynamic(
        tag,
        if tag == DYNAMIC_BOOLEAN_TAG {
            u64::from(payload != 0.0)
        } else {
            payload.to_bits()
        },
    )
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn dynamic_primitive(value: f64, expected: Option<u64>) -> Option<&'static DynamicPrimitive> {
    let pointer = value.to_bits() as usize as *const DynamicPrimitive;
    if !DYNAMIC_VALUES.with(|values| values.borrow().contains(&(pointer as usize))) {
        return None;
    }
    let dynamic = unsafe { pointer.as_ref() }?;
    (DYNAMIC_NUMBER_TAG..=DYNAMIC_TUPLE_TAG)
        .contains(&dynamic.tag)
        .then_some(dynamic)
        .filter(|dynamic| expected.is_none_or(|tag| dynamic.tag == tag))
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn dynamic_tag(value: f64) -> f64 {
    dynamic_primitive(value, None).map_or_else(
        || {
            CALL_ERROR.with(|error| error.set(INVALID_DYNAMIC_VALUE.as_ptr().cast()));
            0.0
        },
        |dynamic| dynamic.tag as f64,
    )
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn type_of_dynamic(value: f64) -> f64 {
    dynamic_primitive(value, None).map_or_else(
        || {
            CALL_ERROR.with(|error| error.set(INVALID_DYNAMIC_VALUE.as_ptr().cast()));
            0.0
        },
        |dynamic| {
            f64::from_bits(match dynamic.tag {
                DYNAMIC_NUMBER_TAG => c"number".as_ptr(),
                DYNAMIC_STRING_TAG => c"string".as_ptr(),
                DYNAMIC_BOOLEAN_TAG => c"boolean".as_ptr(),
                DYNAMIC_NUMBER_ARRAY_TAG..=DYNAMIC_TUPLE_TAG => c"object".as_ptr(),
                _ => unreachable!(),
            } as usize as u64)
        },
    )
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn dynamic_to_boolean(value: f64) -> f64 {
    dynamic_primitive(value, None).map_or_else(
        || {
            CALL_ERROR.with(|error| error.set(INVALID_DYNAMIC_VALUE.as_ptr().cast()));
            0.0
        },
        |dynamic| match dynamic.tag {
            DYNAMIC_NUMBER_TAG => {
                let value = f64::from_bits(dynamic.payload);
                f64::from(value != 0.0 && !value.is_nan())
            }
            DYNAMIC_STRING_TAG => f64::from(unsafe {
                (dynamic.payload as usize as *const c_char)
                    .as_ref()
                    .is_some_and(|value| *value != 0)
            }),
            DYNAMIC_BOOLEAN_TAG => f64::from(dynamic.payload != 0),
            DYNAMIC_NUMBER_ARRAY_TAG..=DYNAMIC_TUPLE_TAG => 1.0,
            _ => unreachable!(),
        },
    )
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn dynamic_is_array(value: f64) -> f64 {
    dynamic_primitive(value, None).map_or_else(
        || {
            CALL_ERROR.with(|error| error.set(INVALID_DYNAMIC_VALUE.as_ptr().cast()));
            0.0
        },
        |dynamic| {
            f64::from(matches!(
                dynamic.tag,
                DYNAMIC_NUMBER_ARRAY_TAG..=DYNAMIC_STRING_ARRAY_TAG | DYNAMIC_TUPLE_TAG
            ))
        },
    )
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn untag_number(value: f64) -> f64 {
    untag_dynamic(value, DYNAMIC_NUMBER_TAG)
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn untag_string(value: f64) -> f64 {
    untag_dynamic(value, DYNAMIC_STRING_TAG)
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn untag_boolean(value: f64) -> f64 {
    dynamic_primitive(value, Some(DYNAMIC_BOOLEAN_TAG)).map_or_else(
        || {
            CALL_ERROR.with(|error| error.set(INVALID_DYNAMIC_VALUE.as_ptr().cast()));
            0.0
        },
        |dynamic| dynamic.payload as f64,
    )
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn untag_number_array(value: f64) -> f64 {
    untag_dynamic(value, DYNAMIC_NUMBER_ARRAY_TAG)
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn untag_boolean_array(value: f64) -> f64 {
    untag_dynamic(value, DYNAMIC_BOOLEAN_ARRAY_TAG)
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn untag_string_array(value: f64) -> f64 {
    untag_dynamic(value, DYNAMIC_STRING_ARRAY_TAG)
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn untag_array(value: f64) -> f64 {
    dynamic_primitive(value, None).map_or_else(
        || {
            CALL_ERROR.with(|error| error.set(INVALID_DYNAMIC_VALUE.as_ptr().cast()));
            0.0
        },
        |dynamic| {
            if matches!(
                dynamic.tag,
                DYNAMIC_NUMBER_ARRAY_TAG..=DYNAMIC_STRING_ARRAY_TAG | DYNAMIC_TUPLE_TAG
            ) {
                f64::from_bits(dynamic.payload)
            } else {
                CALL_ERROR.with(|error| error.set(INVALID_DYNAMIC_VALUE.as_ptr().cast()));
                0.0
            }
        },
    )
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn untag_number_dictionary(value: f64) -> f64 {
    untag_dynamic(value, DYNAMIC_NUMBER_DICTIONARY_TAG)
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn untag_boolean_dictionary(value: f64) -> f64 {
    untag_dynamic(value, DYNAMIC_BOOLEAN_DICTIONARY_TAG)
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn untag_string_dictionary(value: f64) -> f64 {
    untag_dynamic(value, DYNAMIC_STRING_DICTIONARY_TAG)
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn untag_dictionary(value: f64) -> f64 {
    dynamic_primitive(value, None).map_or_else(
        || {
            CALL_ERROR.with(|error| error.set(INVALID_DYNAMIC_VALUE.as_ptr().cast()));
            0.0
        },
        |dynamic| {
            if matches!(
                dynamic.tag,
                DYNAMIC_NUMBER_DICTIONARY_TAG..=DYNAMIC_STRING_DICTIONARY_TAG
            ) {
                f64::from_bits(dynamic.payload)
            } else {
                CALL_ERROR.with(|error| error.set(INVALID_DYNAMIC_VALUE.as_ptr().cast()));
                0.0
            }
        },
    )
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn untag_object(value: f64) -> f64 {
    untag_dynamic(value, DYNAMIC_OBJECT_TAG)
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn untag_tuple(value: f64) -> f64 {
    untag_dynamic(value, DYNAMIC_TUPLE_TAG)
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn object_number_field(object: f64, offset: f64) -> f64 {
    let pointer = object.to_bits() as usize as *const u8;
    if pointer.is_null() || !offset.is_finite() || offset < 0.0 {
        CALL_ERROR.with(|error| error.set(INVALID_DYNAMIC_VALUE.as_ptr().cast()));
        return 0.0;
    }
    unsafe { pointer.add(offset as usize).cast::<f64>().read() }
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn object_boolean_field(object: f64, offset: f64) -> f64 {
    let pointer = object.to_bits() as usize as *const u8;
    if pointer.is_null() || !offset.is_finite() || offset < 0.0 {
        CALL_ERROR.with(|error| error.set(INVALID_DYNAMIC_VALUE.as_ptr().cast()));
        return 0.0;
    }
    f64::from(unsafe { pointer.add(offset as usize).read() != 0 })
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn object_string_field(object: f64, offset: f64) -> f64 {
    let pointer = object.to_bits() as usize as *const u8;
    if pointer.is_null() || !offset.is_finite() || offset < 0.0 {
        CALL_ERROR.with(|error| error.set(INVALID_DYNAMIC_VALUE.as_ptr().cast()));
        return 0.0;
    }
    f64::from_bits(unsafe { pointer.add(offset as usize).cast::<usize>().read() } as u64)
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn object_optional_field(object: f64, offset: f64, kind: u8) -> f64 {
    object_tagged_field(object, offset, kind, 1)
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn object_tagged_field(object: f64, offset: f64, kind: u8, present_tag: u8) -> f64 {
    let pointer = object.to_bits() as usize as *const u8;
    if pointer.is_null() || !offset.is_finite() || offset < 0.0 || offset.fract() != 0.0 {
        CALL_ERROR.with(|error| error.set(INVALID_DYNAMIC_VALUE.as_ptr().cast()));
        return 0.0;
    }
    let tag = unsafe { pointer.add(offset as usize).read() };
    if tag != present_tag {
        if present_tag == 0 {
            CALL_ABSENCE.with(|absence| absence.set(if tag == 1 { 2 } else { 1 }));
        }
        CALL_PRESENT.with(|present| present.set(false));
        return 0.0;
    }
    match kind {
        0 => object_number_field(object, offset + 8.0),
        1 => object_boolean_field(object, offset + 1.0),
        2 => object_string_field(object, offset + 8.0),
        _ => unreachable!(),
    }
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
macro_rules! optional_object_getter {
    ($name:ident, $kind:expr) => {
        extern "C" fn $name(object: f64, offset: f64) -> f64 {
            object_optional_field(object, offset, $kind)
        }
    };
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
optional_object_getter!(object_optional_number_field, 0);
#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
optional_object_getter!(object_optional_boolean_field, 1);
#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
optional_object_getter!(object_optional_pointer_field, 2);

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
macro_rules! nullish_object_getter {
    ($name:ident, $kind:expr) => {
        extern "C" fn $name(object: f64, offset: f64) -> f64 {
            object_tagged_field(object, offset, $kind, 0)
        }
    };
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
nullish_object_getter!(object_nullish_number_field, 0);
#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
nullish_object_getter!(object_nullish_boolean_field, 1);
#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
nullish_object_getter!(object_nullish_pointer_field, 2);

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn optional_tuple_field(tuple: f64, index: f64, kind: u8) -> f64 {
    tagged_tuple_field(tuple, index, kind, 1)
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn tagged_tuple_field(tuple: f64, index: f64, kind: u8, present_tag: u8) -> f64 {
    let Some((data, length)) = (unsafe { array_data(tuple) }) else {
        CALL_ERROR.with(|error| error.set(INVALID_DYNAMIC_VALUE.as_ptr().cast()));
        return 0.0;
    };
    if !index.is_finite() || index < 0.0 || index.fract() != 0.0 || index as usize >= length {
        CALL_ERROR.with(|error| error.set(INVALID_DYNAMIC_VALUE.as_ptr().cast()));
        return 0.0;
    }
    let field = unsafe { data.add(8 + index as usize * 16) };
    if unsafe { field.read() } != present_tag {
        CALL_PRESENT.with(|present| present.set(false));
        return 0.0;
    }
    unsafe {
        match kind {
            0 => field.add(8).cast::<f64>().read(),
            1 => f64::from(field.add(1).read() != 0),
            2 => f64::from_bits(field.add(8).cast::<usize>().read() as u64),
            _ => unreachable!(),
        }
    }
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
macro_rules! optional_tuple_getter {
    ($name:ident, $kind:expr) => {
        extern "C" fn $name(tuple: f64, index: f64) -> f64 {
            optional_tuple_field(tuple, index, $kind)
        }
    };
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
optional_tuple_getter!(optional_tuple_number_field, 0);
#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
optional_tuple_getter!(optional_tuple_boolean_field, 1);
#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
optional_tuple_getter!(optional_tuple_pointer_field, 2);

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
macro_rules! nullish_tuple_getter {
    ($name:ident, $kind:expr) => {
        extern "C" fn $name(tuple: f64, index: f64) -> f64 {
            tagged_tuple_field(tuple, index, $kind, 0)
        }
    };
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
nullish_tuple_getter!(nullish_tuple_number_field, 0);
#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
nullish_tuple_getter!(nullish_tuple_boolean_field, 1);
#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
nullish_tuple_getter!(nullish_tuple_pointer_field, 2);

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn fixed_object_new(size: f64) -> f64 {
    if !size.is_finite() || size <= 0.0 || size.fract() != 0.0 {
        CALL_ERROR.with(|error| error.set(INVALID_DYNAMIC_VALUE.as_ptr().cast()));
        return 0.0;
    }
    let Some(output) = ARENA_ALLOC.with(|allocator| {
        allocator
            .get()
            .map(|allocate| unsafe { allocate(size as usize, 8) })
    }) else {
        CALL_ERROR.with(|error| error.set(ALLOCATION_FAILED.as_ptr().cast()));
        return 0.0;
    };
    if output.is_null() {
        CALL_ERROR.with(|error| error.set(ALLOCATION_FAILED.as_ptr().cast()));
        return 0.0;
    }
    unsafe { output.write_bytes(0, size as usize) };
    f64::from_bits(output as usize as u64)
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn fixed_object_set(object: f64, value: f64, offset: f64, kind: u8) -> f64 {
    let pointer = object.to_bits() as usize as *mut u8;
    if pointer.is_null() || !offset.is_finite() || offset < 0.0 || offset.fract() != 0.0 {
        CALL_ERROR.with(|error| error.set(INVALID_DYNAMIC_VALUE.as_ptr().cast()));
        return 0.0;
    }
    unsafe {
        let field = pointer.add(offset as usize);
        match kind {
            0 => field.cast::<f64>().write(value),
            1 => field.write(u8::from(value != 0.0)),
            2 => field.cast::<usize>().write(value.to_bits() as usize),
            3 => field.write(value as u8),
            _ => unreachable!(),
        }
    }
    object
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn tagged_object_number_update(object: f64, offset: f64, mode: f64) -> f64 {
    let pointer = object.to_bits() as usize as *mut u8;
    if pointer.is_null()
        || !offset.is_finite()
        || offset < 0.0
        || offset.fract() != 0.0
        || !mode.is_finite()
        || mode < 0.0
        || mode.fract() != 0.0
        || mode >= 12.0
    {
        CALL_ERROR.with(|error| error.set(INVALID_DYNAMIC_VALUE.as_ptr().cast()));
        return 0.0;
    }
    let mode = mode as u8;
    let semantic = mode / 4;
    let tag = unsafe { pointer.add(offset as usize).read() };
    let present = match semantic {
        0 | 1 => tag == 1,
        2 => tag == 0,
        _ => unreachable!(),
    };
    let old = if present {
        unsafe { pointer.add(offset as usize + 8).cast::<f64>().read() }
    } else {
        match (semantic, tag) {
            (0, 0) | (2, 2) => f64::NAN,
            (1, 0) | (2, 1) => 0.0,
            _ => {
                CALL_ERROR.with(|error| error.set(INVALID_DYNAMIC_VALUE.as_ptr().cast()));
                return 0.0;
            }
        }
    };
    let new = if mode % 4 >= 2 { old - 1.0 } else { old + 1.0 };
    unsafe {
        pointer.add(offset as usize + 8).cast::<f64>().write(new);
        pointer
            .add(offset as usize)
            .write(if semantic == 2 { 0 } else { 1 });
    }
    if mode.is_multiple_of(2) {
        new
    } else {
        old
    }
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn tagged_object_number_assign(object: f64, right: f64, offset: f64, mode: f64) -> f64 {
    let pointer = object.to_bits() as usize as *mut u8;
    if pointer.is_null()
        || !offset.is_finite()
        || offset < 0.0
        || offset.fract() != 0.0
        || !mode.is_finite()
        || mode < 0.0
        || mode.fract() != 0.0
        || mode >= 36.0
    {
        CALL_ERROR.with(|error| error.set(INVALID_DYNAMIC_VALUE.as_ptr().cast()));
        return 0.0;
    }
    let mode = mode as u8;
    let semantic = mode / 12;
    let operation = mode % 12;
    let tag = unsafe { pointer.add(offset as usize).read() };
    let present = match semantic {
        0 | 1 => tag == 1,
        2 => tag == 0,
        _ => unreachable!(),
    };
    let left = if present {
        unsafe { pointer.add(offset as usize + 8).cast::<f64>().read() }
    } else {
        match (semantic, tag) {
            (0, 0) | (2, 2) => f64::NAN,
            (1, 0) | (2, 1) => 0.0,
            _ => {
                CALL_ERROR.with(|error| error.set(INVALID_DYNAMIC_VALUE.as_ptr().cast()));
                return 0.0;
            }
        }
    };
    let value = match operation {
        0 => left + right,
        1 => left - right,
        2 => left * right,
        3 => left / right,
        4 => unsafe { fmod(left, right) },
        5 => shift_left(left, right),
        6 => shift_right(left, right),
        7 => shift_right_unsigned(left, right),
        8 => bit_or(left, right),
        9 => bit_xor(left, right),
        10 => bit_and(left, right),
        11 => power(left, right),
        _ => unreachable!(),
    };
    unsafe {
        pointer.add(offset as usize + 8).cast::<f64>().write(value);
        pointer
            .add(offset as usize)
            .write(if semantic == 2 { 0 } else { 1 });
    }
    value
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
macro_rules! fixed_object_setter {
    ($name:ident, $kind:expr) => {
        extern "C" fn $name(object: f64, value: f64, offset: f64) -> f64 {
            fixed_object_set(object, value, offset, $kind)
        }
    };
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fixed_object_setter!(fixed_object_set_number, 0);
#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fixed_object_setter!(fixed_object_set_boolean, 1);
#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fixed_object_setter!(fixed_object_set_string, 2);
#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fixed_object_setter!(fixed_object_set_byte, 3);

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn fixed_tuple_new(length: f64) -> f64 {
    fixed_tuple_new_with_stride(length, 8)
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn fixed_wide_tuple_new(length: f64) -> f64 {
    fixed_tuple_new_with_stride(length, 16)
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn fixed_tuple_new_with_stride(length: f64, stride: usize) -> f64 {
    if !length.is_finite() || length < 0.0 || length.fract() != 0.0 {
        CALL_ERROR.with(|error| error.set(INVALID_DYNAMIC_VALUE.as_ptr().cast()));
        return 0.0;
    }
    let Some(size) = (length as usize)
        .checked_mul(stride)
        .and_then(|size| size.checked_add(8))
    else {
        CALL_ERROR.with(|error| error.set(ALLOCATION_FAILED.as_ptr().cast()));
        return 0.0;
    };
    let Some(output) =
        ARENA_ALLOC.with(|allocator| allocator.get().map(|allocate| unsafe { allocate(size, 8) }))
    else {
        CALL_ERROR.with(|error| error.set(ALLOCATION_FAILED.as_ptr().cast()));
        return 0.0;
    };
    if output.is_null() {
        CALL_ERROR.with(|error| error.set(ALLOCATION_FAILED.as_ptr().cast()));
        return 0.0;
    }
    unsafe {
        output.write_bytes(0, size);
        output.cast::<u64>().write(length as u64);
    }
    mutable_array_handle(array_result(output))
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn fixed_wide_tuple_set(tuple: f64, value: f64, index: f64, kind: u8, mode: u8) -> f64 {
    let Some((data, length)) = (unsafe { array_data(tuple) }) else {
        CALL_ERROR.with(|error| error.set(INVALID_DYNAMIC_VALUE.as_ptr().cast()));
        return 0.0;
    };
    if !index.is_finite() || index < 0.0 || index.fract() != 0.0 || index as usize >= length {
        CALL_ERROR.with(|error| error.set(INVALID_DYNAMIC_VALUE.as_ptr().cast()));
        return 0.0;
    }
    unsafe {
        let field = data.cast_mut().add(8 + index as usize * 16);
        let payload = if mode == 1 {
            field.write(1);
            field.add(if kind == 1 { 1 } else { 8 })
        } else if mode == 2 {
            field.add(if kind == 1 { 1 } else { 8 })
        } else {
            field
        };
        match kind {
            0 => payload.cast::<f64>().write(value),
            1 => payload.write(u8::from(value != 0.0)),
            2 => payload.cast::<usize>().write(value.to_bits() as usize),
            3 => payload.write(value as u8),
            _ => unreachable!(),
        }
    }
    tuple
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
macro_rules! fixed_wide_tuple_setter {
    ($name:ident, $kind:expr, $mode:expr) => {
        extern "C" fn $name(tuple: f64, value: f64, index: f64) -> f64 {
            fixed_wide_tuple_set(tuple, value, index, $kind, $mode)
        }
    };
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fixed_wide_tuple_setter!(fixed_wide_tuple_set_number, 0, 0);
#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fixed_wide_tuple_setter!(fixed_wide_tuple_set_boolean, 1, 0);
#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fixed_wide_tuple_setter!(fixed_wide_tuple_set_pointer, 2, 0);
#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fixed_wide_tuple_setter!(fixed_wide_tuple_set_byte, 3, 0);
#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fixed_wide_tuple_setter!(fixed_wide_tuple_set_optional_number, 0, 1);
#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fixed_wide_tuple_setter!(fixed_wide_tuple_set_optional_boolean, 1, 1);
#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fixed_wide_tuple_setter!(fixed_wide_tuple_set_optional_pointer, 2, 1);
#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fixed_wide_tuple_setter!(fixed_wide_tuple_set_nullish_number, 0, 2);
#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fixed_wide_tuple_setter!(fixed_wide_tuple_set_nullish_boolean, 1, 2);
#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fixed_wide_tuple_setter!(fixed_wide_tuple_set_nullish_pointer, 2, 2);

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn fixed_tuple_set(tuple: f64, value: f64, index: f64, kind: u8) -> f64 {
    let Some((data, length)) = (unsafe { array_data(tuple) }) else {
        CALL_ERROR.with(|error| error.set(INVALID_DYNAMIC_VALUE.as_ptr().cast()));
        return 0.0;
    };
    if !index.is_finite() || index < 0.0 || index.fract() != 0.0 || index as usize >= length {
        CALL_ERROR.with(|error| error.set(INVALID_DYNAMIC_VALUE.as_ptr().cast()));
        return 0.0;
    }
    unsafe {
        let element = data.cast_mut().add(8 + index as usize * 8);
        match kind {
            0 => element.cast::<f64>().write(value),
            1 => element.write(u8::from(value != 0.0)),
            2 => element.cast::<usize>().write(value.to_bits() as usize),
            _ => unreachable!(),
        }
    }
    tuple
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
macro_rules! fixed_tuple_setter {
    ($name:ident, $kind:expr) => {
        extern "C" fn $name(tuple: f64, value: f64, index: f64) -> f64 {
            fixed_tuple_set(tuple, value, index, $kind)
        }
    };
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fixed_tuple_setter!(fixed_tuple_set_number, 0);
#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fixed_tuple_setter!(fixed_tuple_set_boolean, 1);
#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fixed_tuple_setter!(fixed_tuple_set_pointer, 2);

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn untag_dynamic(value: f64, expected: u64) -> f64 {
    dynamic_primitive(value, Some(expected)).map_or_else(
        || {
            CALL_ERROR.with(|error| error.set(INVALID_DYNAMIC_VALUE.as_ptr().cast()));
            0.0
        },
        |dynamic| f64::from_bits(dynamic.payload),
    )
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
extern "C" fn dynamic_to_string(value: f64) -> f64 {
    dynamic_primitive(value, None).map_or_else(
        || {
            CALL_ERROR.with(|error| error.set(INVALID_DYNAMIC_VALUE.as_ptr().cast()));
            0.0
        },
        |dynamic| match dynamic.tag {
            DYNAMIC_NUMBER_TAG => number_to_string(f64::from_bits(dynamic.payload)),
            DYNAMIC_STRING_TAG => f64::from_bits(dynamic.payload),
            DYNAMIC_BOOLEAN_TAG => boolean_to_string(dynamic.payload as f64),
            DYNAMIC_NUMBER_ARRAY_TAG => array_format(
                0,
                f64::from_bits(dynamic.payload),
                f64::from_bits(c",".as_ptr() as usize as u64),
            ),
            DYNAMIC_BOOLEAN_ARRAY_TAG => array_format(
                2,
                f64::from_bits(dynamic.payload),
                f64::from_bits(c",".as_ptr() as usize as u64),
            ),
            DYNAMIC_STRING_ARRAY_TAG => array_format(
                1,
                f64::from_bits(dynamic.payload),
                f64::from_bits(c",".as_ptr() as usize as u64),
            ),
            DYNAMIC_NUMBER_DICTIONARY_TAG..=DYNAMIC_TUPLE_TAG => {
                arena_string("[object Object]".into())
            }
            _ => unreachable!(),
        },
    )
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn string_to_number(value: f64) -> f64 {
    let Some(parse) = STRING_TO_NUMBER.with(Cell::get) else {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return f64::NAN;
    };
    let value = value.to_bits() as usize as *const c_char;
    if value.is_null() {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        f64::NAN
    } else {
        unsafe { parse(value) }
    }
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn dynamic_to_number(value: f64) -> f64 {
    dynamic_primitive(value, None).map_or_else(
        || {
            CALL_ERROR.with(|error| error.set(INVALID_DYNAMIC_VALUE.as_ptr().cast()));
            0.0
        },
        |dynamic| match dynamic.tag {
            DYNAMIC_NUMBER_TAG => f64::from_bits(dynamic.payload),
            DYNAMIC_STRING_TAG => string_to_number(f64::from_bits(dynamic.payload)),
            DYNAMIC_BOOLEAN_TAG => dynamic.payload as f64,
            DYNAMIC_NUMBER_ARRAY_TAG..=DYNAMIC_TUPLE_TAG => {
                string_to_number(dynamic_to_string(value))
            }
            _ => unreachable!(),
        },
    )
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn dynamic_add(left: f64, right: f64) -> f64 {
    let Some((left_value, right_value)) =
        dynamic_primitive(left, None).zip(dynamic_primitive(right, None))
    else {
        CALL_ERROR.with(|error| error.set(INVALID_DYNAMIC_VALUE.as_ptr().cast()));
        return 0.0;
    };
    if !matches!(left_value.tag, DYNAMIC_NUMBER_TAG | DYNAMIC_BOOLEAN_TAG)
        || !matches!(right_value.tag, DYNAMIC_NUMBER_TAG | DYNAMIC_BOOLEAN_TAG)
    {
        let value = string_concat(dynamic_to_string(left), dynamic_to_string(right));
        arena_dynamic(DYNAMIC_STRING_TAG, value.to_bits())
    } else {
        let value = dynamic_to_number(left) + dynamic_to_number(right);
        arena_dynamic(DYNAMIC_NUMBER_TAG, value.to_bits())
    }
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn dynamic_compare(left: f64, right: f64, operation: u8) -> f64 {
    let Some((left_value, right_value)) =
        dynamic_primitive(left, None).zip(dynamic_primitive(right, None))
    else {
        CALL_ERROR.with(|error| error.set(INVALID_DYNAMIC_VALUE.as_ptr().cast()));
        return 0.0;
    };
    let strict_equal = || {
        if left_value.tag != right_value.tag {
            return false;
        }
        match left_value.tag {
            DYNAMIC_NUMBER_TAG => {
                f64::from_bits(left_value.payload) == f64::from_bits(right_value.payload)
            }
            DYNAMIC_STRING_TAG => {
                string_same_value(
                    f64::from_bits(left_value.payload),
                    f64::from_bits(right_value.payload),
                ) != 0.0
            }
            DYNAMIC_BOOLEAN_TAG => left_value.payload == right_value.payload,
            DYNAMIC_NUMBER_ARRAY_TAG..=DYNAMIC_TUPLE_TAG => {
                left_value.payload == right_value.payload
            }
            _ => unreachable!(),
        }
    };
    let equal = || {
        strict_equal()
            || (left_value.tag != right_value.tag
                && !(left_value.tag >= DYNAMIC_NUMBER_ARRAY_TAG
                    && right_value.tag >= DYNAMIC_NUMBER_ARRAY_TAG)
                && dynamic_to_number(left) == dynamic_to_number(right))
    };
    let result = match operation {
        0..=3 => {
            let left_string =
                left_value.tag == DYNAMIC_STRING_TAG || left_value.tag >= DYNAMIC_NUMBER_ARRAY_TAG;
            let right_string = right_value.tag == DYNAMIC_STRING_TAG
                || right_value.tag >= DYNAMIC_NUMBER_ARRAY_TAG;
            if left_string && right_string {
                let ordering = string_compare(dynamic_to_string(left), dynamic_to_string(right));
                match operation {
                    0 => ordering < 0.0,
                    1 => ordering <= 0.0,
                    2 => ordering > 0.0,
                    3 => ordering >= 0.0,
                    _ => unreachable!(),
                }
            } else {
                let left = dynamic_to_number(left);
                let right = dynamic_to_number(right);
                match operation {
                    0 => left < right,
                    1 => left <= right,
                    2 => left > right,
                    3 => left >= right,
                    _ => unreachable!(),
                }
            }
        }
        4 => equal(),
        5 => !equal(),
        6 => strict_equal(),
        7 => !strict_equal(),
        _ => unreachable!(),
    };
    f64::from(result)
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
macro_rules! dynamic_compare_functions {
    ($($name:ident => $operation:literal),+ $(,)?) => {
        $(extern "C" fn $name(left: f64, right: f64) -> f64 {
            dynamic_compare(left, right, $operation)
        })+
    };
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
dynamic_compare_functions!(
    dynamic_less => 0,
    dynamic_less_equal => 1,
    dynamic_greater => 2,
    dynamic_greater_equal => 3,
    dynamic_equal => 4,
    dynamic_not_equal => 5,
    dynamic_strict_equal => 6,
    dynamic_strict_not_equal => 7,
);

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn parse_float(value: f64) -> f64 {
    let Some(parse) = PARSE_FLOAT.with(Cell::get) else {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return f64::NAN;
    };
    let value = value.to_bits() as usize as *const c_char;
    if value.is_null() {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        f64::NAN
    } else {
        unsafe { parse(value) }
    }
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn parse_int(value: f64, radix: f64) -> f64 {
    let Some(parse) = PARSE_INT.with(Cell::get) else {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return f64::NAN;
    };
    let value = value.to_bits() as usize as *const c_char;
    if value.is_null() {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        f64::NAN
    } else {
        unsafe { parse(value, radix) }
    }
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
fn formatted_number(operation: u8, value: f64, argument: f64) -> f64 {
    let Some(format) = NUMBER_FORMAT.with(Cell::get) else {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return f64::from_bits(0);
    };
    let value = unsafe { format(operation, value, argument) };
    if value.is_null() {
        CALL_ERROR.with(|error| error.set(ALLOCATION_FAILED.as_ptr().cast()));
        f64::from_bits(0)
    } else {
        f64::from_bits(value as usize as u64)
    }
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn number_to_fixed(value: f64, digits: f64) -> f64 {
    let digits = if digits.is_nan() { 0.0 } else { digits.trunc() };
    if !(0.0..=100.0).contains(&digits) {
        CALL_ERROR.with(|error| error.set(INVALID_FIXED_DIGITS.as_ptr().cast()));
        return f64::from_bits(0);
    }
    formatted_number(0, value, digits)
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn number_to_precision(value: f64, precision: f64) -> f64 {
    let precision = if precision.is_nan() {
        0.0
    } else {
        precision.trunc()
    };
    if !(1.0..=100.0).contains(&precision) {
        CALL_ERROR.with(|error| error.set(INVALID_PRECISION.as_ptr().cast()));
        return f64::from_bits(0);
    }
    formatted_number(1, value, precision)
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn number_to_radix_string(value: f64, radix: f64) -> f64 {
    let radix = if radix.is_nan() { 0.0 } else { radix.trunc() };
    if !(2.0..=36.0).contains(&radix) {
        CALL_ERROR.with(|error| error.set(INVALID_RADIX.as_ptr().cast()));
        return f64::from_bits(0);
    }
    if radix == 10.0 {
        return number_to_string(value);
    }
    formatted_number(2, value, radix)
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn number_to_exponential(value: f64, digits: f64) -> f64 {
    let digits = if digits.is_nan() { 0.0 } else { digits.trunc() };
    if !(0.0..=100.0).contains(&digits) {
        CALL_ERROR.with(|error| error.set(INVALID_EXPONENTIAL_DIGITS.as_ptr().cast()));
        return f64::from_bits(0);
    }
    formatted_number(3, value, digits)
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn number_to_exponential_shortest(value: f64) -> f64 {
    formatted_number(3, value, -1.0)
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
extern "C" fn string_normalize(value: f64, form: f64) -> f64 {
    let Some(normalize) = STRING_NORMALIZE.with(Cell::get) else {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return 0.0;
    };
    let normalized = unsafe {
        normalize(
            value.to_bits() as usize as *const c_char,
            form.to_bits() as usize as *const c_char,
        )
    };
    if normalized.is_null() {
        CALL_ERROR.with(|error| error.set(INVALID_NORMALIZATION_FORM.as_ptr().cast()));
        0.0
    } else {
        f64::from_bits(normalized as usize as u64)
    }
}

#[cfg(all(target_arch = "x86_64", target_family = "unix"))]
extern "C" fn string_split(value: f64, separator: f64, limit: f64) -> f64 {
    let Some(split) = STRING_SPLIT.with(Cell::get) else {
        CALL_ERROR.with(|error| error.set(INVALID_SYMBOL.as_ptr().cast()));
        return 0.0;
    };
    let result = unsafe {
        split(
            value.to_bits() as usize as *const c_char,
            separator.to_bits() as usize as *const c_char,
            limit,
        )
    };
    if result.is_null() {
        CALL_ERROR.with(|error| error.set(ALLOCATION_FAILED.as_ptr().cast()));
        0.0
    } else {
        array_result(result)
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
