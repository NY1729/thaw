impl NumericProgram {
    #[cfg(all(target_arch = "x86_64", target_family = "unix"))]
    fn machine_code(&self) -> Option<Vec<u8>> {
        let mut code = Vec::with_capacity(self.0.len() * 12 + 8);
        let mut depth = 0u8;
        let mut branches = Vec::new();
        let mut loops = Vec::new();
        let mut guards = Vec::new();
        let mut switches = Vec::new();
        let mut tries = Vec::new();
        let mut catches = Vec::new();
        let mut results = Vec::new();
        for value in &self.0 {
            match value {
                NumericValue::Argument(index) => {
                    if depth == 8 {
                        return None;
                    }
                    code.extend_from_slice(&[0xf2, 0x0f, 0x10, 0x47 | (depth << 3), index * 8]);
                    depth += 1;
                }
                NumericValue::DynamicArgument(index) => {
                    if depth == 8 {
                        return None;
                    }
                    code.extend_from_slice(&[0xf2, 0x0f, 0x10, 0x47 | (depth << 3), index * 8]);
                    code.extend_from_slice(&[
                        0xf2,
                        0x0f,
                        0x10,
                        0x47 | ((depth + 1) << 3),
                        (index + 1) * 8,
                    ]);
                    emit_binary_call(&mut code, dynamic_from_parts as *const () as u64, depth);
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
                NumericValue::IsFinite
                | NumericValue::IsInteger
                | NumericValue::IsNaN
                | NumericValue::IsSafeInteger => {
                    if depth == 0 {
                        return None;
                    }
                    let function = match value {
                        NumericValue::IsFinite => number_is_finite,
                        NumericValue::IsInteger => number_is_integer,
                        NumericValue::IsNaN => number_is_nan,
                        NumericValue::IsSafeInteger => number_is_safe_integer,
                        _ => unreachable!(),
                    };
                    emit_unary_call(&mut code, function as *const () as u64, depth - 1);
                }
                NumericValue::StringCompare => {
                    if depth < 2 {
                        return None;
                    }
                    emit_binary_call(&mut code, string_compare as *const () as u64, depth - 2);
                    depth -= 1;
                }
                NumericValue::NumberSameValue
                | NumericValue::StringSameValue
                | NumericValue::ReferenceSameValue => {
                    if depth < 2 {
                        return None;
                    }
                    let function = match value {
                        NumericValue::NumberSameValue => number_same_value,
                        NumericValue::StringSameValue => string_same_value,
                        NumericValue::ReferenceSameValue => reference_same_value,
                        _ => unreachable!(),
                    };
                    emit_binary_call(&mut code, function as *const () as u64, depth - 2);
                    depth -= 1;
                }
                NumericValue::TypeOfNumber
                | NumericValue::TypeOfBoolean
                | NumericValue::TypeOfString
                | NumericValue::TypeOfObject
                | NumericValue::TypeOfDynamic => {
                    if depth == 0 {
                        return None;
                    }
                    let function = match value {
                        NumericValue::TypeOfNumber => type_of_number,
                        NumericValue::TypeOfBoolean => type_of_boolean,
                        NumericValue::TypeOfString => type_of_string,
                        NumericValue::TypeOfObject => type_of_object,
                        NumericValue::TypeOfDynamic => type_of_dynamic,
                        _ => unreachable!(),
                    };
                    emit_unary_call(&mut code, function as *const () as u64, depth - 1);
                }
                NumericValue::DynamicToBoolean => {
                    if depth == 0 {
                        return None;
                    }
                    emit_unary_call(&mut code, dynamic_to_boolean as *const () as u64, depth - 1);
                }
                NumericValue::DynamicAdd | NumericValue::DynamicCompare(_) => {
                    if depth < 2 {
                        return None;
                    }
                    let function = match value {
                        NumericValue::DynamicAdd => dynamic_add,
                        NumericValue::DynamicCompare(operation) => [
                            dynamic_less,
                            dynamic_less_equal,
                            dynamic_greater,
                            dynamic_greater_equal,
                            dynamic_equal,
                            dynamic_not_equal,
                            dynamic_strict_equal,
                            dynamic_strict_not_equal,
                        ][usize::from(*operation)],
                        _ => unreachable!(),
                    };
                    emit_binary_call(&mut code, function as *const () as u64, depth - 2);
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
                NumericValue::NumberToString
                | NumericValue::BooleanToString
                | NumericValue::StringToNumber
                | NumericValue::DynamicToString
                | NumericValue::DynamicToNumber
                | NumericValue::TagNumber
                | NumericValue::TagString
                | NumericValue::TagBoolean
                | NumericValue::TagAggregate(_)
                | NumericValue::DynamicTag
                | NumericValue::UntagNumber
                | NumericValue::UntagString
                | NumericValue::UntagBoolean
                | NumericValue::UntagNumberArray
                | NumericValue::UntagBooleanArray
                | NumericValue::UntagStringArray
                | NumericValue::UntagArray
                | NumericValue::UntagNumberDictionary
                | NumericValue::UntagBooleanDictionary
                | NumericValue::UntagStringDictionary
                | NumericValue::UntagDictionary
                | NumericValue::UntagObject
                | NumericValue::UntagTuple
                | NumericValue::ParseFloat
                | NumericValue::NumberToExponentialShortest => {
                    if depth == 0 {
                        return None;
                    }
                    let function = match value {
                        NumericValue::NumberToString => number_to_string,
                        NumericValue::BooleanToString => boolean_to_string,
                        NumericValue::StringToNumber => string_to_number,
                        NumericValue::DynamicToString => dynamic_to_string,
                        NumericValue::DynamicToNumber => dynamic_to_number,
                        NumericValue::TagNumber => tag_number,
                        NumericValue::TagString => tag_string,
                        NumericValue::TagBoolean => tag_boolean,
                        NumericValue::TagAggregate(kind) => [
                            tag_number_array,
                            tag_boolean_array,
                            tag_string_array,
                            tag_number_dictionary,
                            tag_boolean_dictionary,
                            tag_string_dictionary,
                            tag_object,
                            tag_tuple,
                        ][*kind as usize],
                        NumericValue::DynamicTag => dynamic_tag,
                        NumericValue::UntagNumber => untag_number,
                        NumericValue::UntagString => untag_string,
                        NumericValue::UntagBoolean => untag_boolean,
                        NumericValue::UntagNumberArray => untag_number_array,
                        NumericValue::UntagBooleanArray => untag_boolean_array,
                        NumericValue::UntagStringArray => untag_string_array,
                        NumericValue::UntagArray => untag_array,
                        NumericValue::UntagNumberDictionary => untag_number_dictionary,
                        NumericValue::UntagBooleanDictionary => untag_boolean_dictionary,
                        NumericValue::UntagStringDictionary => untag_string_dictionary,
                        NumericValue::UntagDictionary => untag_dictionary,
                        NumericValue::UntagObject => untag_object,
                        NumericValue::UntagTuple => untag_tuple,
                        NumericValue::ParseFloat => parse_float,
                        NumericValue::NumberToExponentialShortest => number_to_exponential_shortest,
                        _ => unreachable!(),
                    };
                    emit_unary_call(&mut code, function as *const () as u64, depth - 1);
                }
                NumericValue::ObjectField(kind, offset) => {
                    if depth == 0 || depth == 8 {
                        return None;
                    }
                    let offset = f64::from(*offset);
                    code.extend_from_slice(&[0x48, 0xb8]);
                    code.extend_from_slice(&offset.to_bits().to_le_bytes());
                    code.extend_from_slice(&[0x66, 0x48, 0x0f, 0x6e, 0xc0 | (depth << 3)]);
                    let function = [
                        object_number_field,
                        object_boolean_field,
                        object_string_field,
                    ][usize::from(*kind)];
                    emit_binary_call(&mut code, function as *const () as u64, depth - 1);
                }
                NumericValue::OptionalObjectField(kind, offset) => {
                    if depth == 0 || depth == 8 {
                        return None;
                    }
                    let offset = f64::from(*offset);
                    code.extend_from_slice(&[0x48, 0xb8]);
                    code.extend_from_slice(&offset.to_bits().to_le_bytes());
                    code.extend_from_slice(&[0x66, 0x48, 0x0f, 0x6e, 0xc0 | (depth << 3)]);
                    let function = [
                        object_optional_number_field,
                        object_optional_boolean_field,
                        object_optional_pointer_field,
                    ][usize::from(*kind)];
                    emit_binary_call(&mut code, function as *const () as u64, depth - 1);
                }
                NumericValue::NullishObjectField(kind, offset) => {
                    if depth == 0 || depth == 8 {
                        return None;
                    }
                    let offset = f64::from(*offset);
                    code.extend_from_slice(&[0x48, 0xb8]);
                    code.extend_from_slice(&offset.to_bits().to_le_bytes());
                    code.extend_from_slice(&[0x66, 0x48, 0x0f, 0x6e, 0xc0 | (depth << 3)]);
                    let function = [
                        object_nullish_number_field,
                        object_nullish_boolean_field,
                        object_nullish_pointer_field,
                    ][usize::from(*kind)];
                    emit_binary_call(&mut code, function as *const () as u64, depth - 1);
                }
                NumericValue::OptionalTupleField(kind, index) => {
                    if depth == 0 || depth == 8 {
                        return None;
                    }
                    let index = f64::from(*index);
                    code.extend_from_slice(&[0x48, 0xb8]);
                    code.extend_from_slice(&index.to_bits().to_le_bytes());
                    code.extend_from_slice(&[0x66, 0x48, 0x0f, 0x6e, 0xc0 | (depth << 3)]);
                    let function = [
                        optional_tuple_number_field,
                        optional_tuple_boolean_field,
                        optional_tuple_pointer_field,
                    ][usize::from(*kind)];
                    emit_binary_call(&mut code, function as *const () as u64, depth - 1);
                }
                NumericValue::NullishTupleField(kind, index) => {
                    if depth == 0 || depth == 8 {
                        return None;
                    }
                    let index = f64::from(*index);
                    code.extend_from_slice(&[0x48, 0xb8]);
                    code.extend_from_slice(&index.to_bits().to_le_bytes());
                    code.extend_from_slice(&[0x66, 0x48, 0x0f, 0x6e, 0xc0 | (depth << 3)]);
                    let function = [
                        nullish_tuple_number_field,
                        nullish_tuple_boolean_field,
                        nullish_tuple_pointer_field,
                    ][usize::from(*kind)];
                    emit_binary_call(&mut code, function as *const () as u64, depth - 1);
                }
                NumericValue::FixedObjectNew(size) => {
                    if depth == 8 {
                        return None;
                    }
                    let size = f64::from(*size);
                    code.extend_from_slice(&[0x48, 0xb8]);
                    code.extend_from_slice(&size.to_bits().to_le_bytes());
                    code.extend_from_slice(&[0x66, 0x48, 0x0f, 0x6e, 0xc0 | (depth << 3)]);
                    emit_unary_call(&mut code, fixed_object_new as *const () as u64, depth);
                    depth += 1;
                }
                NumericValue::FixedObjectSet(kind, offset) => {
                    if depth < 2 || depth == 8 {
                        return None;
                    }
                    let offset = f64::from(*offset);
                    code.extend_from_slice(&[0x48, 0xb8]);
                    code.extend_from_slice(&offset.to_bits().to_le_bytes());
                    code.extend_from_slice(&[0x66, 0x48, 0x0f, 0x6e, 0xc0 | (depth << 3)]);
                    let function = [
                        fixed_object_set_number,
                        fixed_object_set_boolean,
                        fixed_object_set_string,
                        fixed_object_set_byte,
                    ][usize::from(*kind)];
                    emit_ternary_call(&mut code, function as *const () as u64, depth - 2);
                    depth -= 1;
                }
                NumericValue::TaggedObjectNumberUpdate(mode, offset) => {
                    if depth == 0 || depth > 6 {
                        return None;
                    }
                    for (index, value) in [f64::from(*offset), f64::from(*mode)]
                        .into_iter()
                        .enumerate()
                    {
                        code.extend_from_slice(&[0x48, 0xb8]);
                        code.extend_from_slice(&value.to_bits().to_le_bytes());
                        code.extend_from_slice(&[
                            0x66,
                            0x48,
                            0x0f,
                            0x6e,
                            0xc0 | ((depth + index as u8) << 3),
                        ]);
                    }
                    emit_ternary_call(
                        &mut code,
                        tagged_object_number_update as *const () as u64,
                        depth - 1,
                    );
                }
                NumericValue::TaggedObjectNumberAssign(mode, offset) => {
                    if !(2..=6).contains(&depth) {
                        return None;
                    }
                    for (index, value) in [f64::from(*offset), f64::from(*mode)]
                        .into_iter()
                        .enumerate()
                    {
                        code.extend_from_slice(&[0x48, 0xb8]);
                        code.extend_from_slice(&value.to_bits().to_le_bytes());
                        code.extend_from_slice(&[
                            0x66,
                            0x48,
                            0x0f,
                            0x6e,
                            0xc0 | ((depth + index as u8) << 3),
                        ]);
                    }
                    emit_quaternary_call(
                        &mut code,
                        tagged_object_number_assign as *const () as u64,
                        depth - 2,
                    );
                    depth -= 1;
                }
                NumericValue::FixedTupleNew(length) => {
                    if depth == 8 {
                        return None;
                    }
                    let length = f64::from(*length);
                    code.extend_from_slice(&[0x48, 0xb8]);
                    code.extend_from_slice(&length.to_bits().to_le_bytes());
                    code.extend_from_slice(&[0x66, 0x48, 0x0f, 0x6e, 0xc0 | (depth << 3)]);
                    emit_unary_call(&mut code, fixed_tuple_new as *const () as u64, depth);
                    depth += 1;
                }
                NumericValue::FixedWideTupleNew(length) => {
                    if depth == 8 {
                        return None;
                    }
                    let length = f64::from(*length);
                    code.extend_from_slice(&[0x48, 0xb8]);
                    code.extend_from_slice(&length.to_bits().to_le_bytes());
                    code.extend_from_slice(&[0x66, 0x48, 0x0f, 0x6e, 0xc0 | (depth << 3)]);
                    emit_unary_call(&mut code, fixed_wide_tuple_new as *const () as u64, depth);
                    depth += 1;
                }
                NumericValue::FixedTupleSet(kind, index) => {
                    if depth < 2 || depth == 8 {
                        return None;
                    }
                    let index = f64::from(*index);
                    code.extend_from_slice(&[0x48, 0xb8]);
                    code.extend_from_slice(&index.to_bits().to_le_bytes());
                    code.extend_from_slice(&[0x66, 0x48, 0x0f, 0x6e, 0xc0 | (depth << 3)]);
                    let function = [
                        fixed_tuple_set_number,
                        fixed_tuple_set_boolean,
                        fixed_tuple_set_pointer,
                    ][usize::from(*kind)];
                    emit_ternary_call(&mut code, function as *const () as u64, depth - 2);
                    depth -= 1;
                }
                NumericValue::FixedWideTupleSet(kind, index, mode) => {
                    if depth < 2 || depth == 8 {
                        return None;
                    }
                    let index = f64::from(*index);
                    code.extend_from_slice(&[0x48, 0xb8]);
                    code.extend_from_slice(&index.to_bits().to_le_bytes());
                    code.extend_from_slice(&[0x66, 0x48, 0x0f, 0x6e, 0xc0 | (depth << 3)]);
                    let functions = if *mode == 1 {
                        [
                            fixed_wide_tuple_set_optional_number,
                            fixed_wide_tuple_set_optional_boolean,
                            fixed_wide_tuple_set_optional_pointer,
                            fixed_wide_tuple_set_byte,
                        ]
                    } else if *mode == 2 {
                        [
                            fixed_wide_tuple_set_nullish_number,
                            fixed_wide_tuple_set_nullish_boolean,
                            fixed_wide_tuple_set_nullish_pointer,
                            fixed_wide_tuple_set_byte,
                        ]
                    } else {
                        [
                            fixed_wide_tuple_set_number,
                            fixed_wide_tuple_set_boolean,
                            fixed_wide_tuple_set_pointer,
                            fixed_wide_tuple_set_byte,
                        ]
                    };
                    emit_ternary_call(
                        &mut code,
                        functions[usize::from(*kind)] as *const () as u64,
                        depth - 2,
                    );
                    depth -= 1;
                }
                NumericValue::ExcludeNumber
                | NumericValue::ExcludeString
                | NumericValue::ExcludeBoolean
                | NumericValue::ExcludeArray
                | NumericValue::ExcludeObject => {}
                NumericValue::GlobalGet => {
                    if depth == 0 {
                        return None;
                    }
                    emit_unary_call(&mut code, global_get as *const () as u64, depth - 1);
                }
                NumericValue::GlobalSet => {
                    if depth < 2 {
                        return None;
                    }
                    emit_binary_call(&mut code, global_set as *const () as u64, depth - 2);
                    depth -= 1;
                }
                NumericValue::GlobalInit => {
                    if depth < 2 {
                        return None;
                    }
                    emit_binary_call(&mut code, global_init as *const () as u64, depth - 2);
                    depth -= 1;
                }
                NumericValue::CallableEntryGet => {
                    if depth < 2 {
                        return None;
                    }
                    emit_binary_call(&mut code, callable_entry_get as *const () as u64, depth - 2);
                    depth -= 1;
                }
                NumericValue::CallableEntrySet => {
                    if depth < 3 {
                        return None;
                    }
                    emit_ternary_call(&mut code, callable_entry_set as *const () as u64, depth - 3);
                    depth -= 2;
                }
                NumericValue::ParseInt
                | NumericValue::NumberToFixed
                | NumericValue::NumberToPrecision
                | NumericValue::NumberToRadixString
                | NumericValue::NumberToExponential => {
                    if depth < 2 {
                        return None;
                    }
                    let function = match value {
                        NumericValue::ParseInt => parse_int,
                        NumericValue::NumberToFixed => number_to_fixed,
                        NumericValue::NumberToPrecision => number_to_precision,
                        NumericValue::NumberToRadixString => number_to_radix_string,
                        NumericValue::NumberToExponential => number_to_exponential,
                        _ => unreachable!(),
                    };
                    emit_binary_call(&mut code, function as *const () as u64, depth - 2);
                    depth -= 1;
                }
                NumericValue::StringStartsWith
                | NumericValue::StringEndsWith
                | NumericValue::StringIncludes
                | NumericValue::StringIndexOf
                | NumericValue::StringLastIndexOf
                | NumericValue::StringRepeat
                | NumericValue::StringNormalize
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
                        NumericValue::StringNormalize => string_normalize,
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
                NumericValue::StringSplit => {
                    if depth < 3 {
                        return None;
                    }
                    emit_ternary_call(&mut code, string_split as *const () as u64, depth - 3);
                    depth -= 2;
                }
                NumericValue::StringToArray => {
                    if depth == 0 {
                        return None;
                    }
                    emit_unary_call(&mut code, string_to_array as *const () as u64, depth - 1);
                }
                NumericValue::StringFromCharCode | NumericValue::StringFromCodePoint => {
                    if depth == 0 {
                        return None;
                    }
                    let function = match value {
                        NumericValue::StringFromCharCode => string_from_char_code,
                        NumericValue::StringFromCodePoint => string_from_code_point,
                        _ => unreachable!(),
                    };
                    emit_unary_call(&mut code, function as *const () as u64, depth - 1);
                }
                NumericValue::NumberArrayMin | NumericValue::NumberArrayMax => {
                    if depth == 0 {
                        return None;
                    }
                    let function = match value {
                        NumericValue::NumberArrayMin => array_min,
                        NumericValue::NumberArrayMax => array_max,
                        _ => unreachable!(),
                    };
                    emit_unary_call(&mut code, function as *const () as u64, depth - 1);
                }
                NumericValue::NumberArrayHypot => {
                    if depth == 0 {
                        return None;
                    }
                    emit_unary_call(&mut code, array_hypot as *const () as u64, depth - 1);
                }
                NumericValue::NumberArrayReduce(operation, has_initial, from_right) => {
                    if depth < 2 {
                        return None;
                    }
                    type ReduceFn = extern "C" fn(f64, f64) -> f64;
                    let functions: [ReduceFn; 4] = match operation {
                        NumericReduceOp::Add => [
                            number_array_reduce_add,
                            number_array_reduce_add_first,
                            number_array_reduce_add_right,
                            number_array_reduce_add_last,
                        ],
                        NumericReduceOp::Subtract => [
                            number_array_reduce_subtract,
                            number_array_reduce_subtract_first,
                            number_array_reduce_subtract_right,
                            number_array_reduce_subtract_last,
                        ],
                        NumericReduceOp::Multiply => [
                            number_array_reduce_multiply,
                            number_array_reduce_multiply_first,
                            number_array_reduce_multiply_right,
                            number_array_reduce_multiply_last,
                        ],
                        NumericReduceOp::Divide => [
                            number_array_reduce_divide,
                            number_array_reduce_divide_first,
                            number_array_reduce_divide_right,
                            number_array_reduce_divide_last,
                        ],
                        NumericReduceOp::Remainder => [
                            number_array_reduce_remainder,
                            number_array_reduce_remainder_first,
                            number_array_reduce_remainder_right,
                            number_array_reduce_remainder_last,
                        ],
                        NumericReduceOp::Power => [
                            number_array_reduce_power,
                            number_array_reduce_power_first,
                            number_array_reduce_power_right,
                            number_array_reduce_power_last,
                        ],
                        NumericReduceOp::Minimum => [
                            number_array_reduce_minimum,
                            number_array_reduce_minimum_first,
                            number_array_reduce_minimum_right,
                            number_array_reduce_minimum_last,
                        ],
                        NumericReduceOp::Maximum => [
                            number_array_reduce_maximum,
                            number_array_reduce_maximum_first,
                            number_array_reduce_maximum_right,
                            number_array_reduce_maximum_last,
                        ],
                    };
                    let function = functions[*from_right as usize * 2 + usize::from(!*has_initial)];
                    emit_binary_call(&mut code, function as *const () as u64, depth - 2);
                    depth -= 1;
                }
                NumericValue::NumberArrayJitReduce(has_initial, from_right, captured) => {
                    if *captured {
                        if depth < 4 {
                            return None;
                        }
                        let function = match (has_initial, from_right) {
                            (true, false) => number_array_jit_reduce_initial_captured,
                            (false, false) => number_array_jit_reduce_first_captured,
                            (true, true) => number_array_jit_reduce_right_initial_captured,
                            (false, true) => number_array_jit_reduce_right_last_captured,
                        };
                        emit_quaternary_call(&mut code, function as *const () as u64, depth - 4);
                        depth -= 3;
                    } else {
                        if depth < 3 {
                            return None;
                        }
                        let function = match (has_initial, from_right) {
                            (true, false) => number_array_jit_reduce_initial,
                            (false, false) => number_array_jit_reduce_first,
                            (true, true) => number_array_jit_reduce_right_initial,
                            (false, true) => number_array_jit_reduce_right_last,
                        };
                        emit_ternary_call(&mut code, function as *const () as u64, depth - 3);
                        depth -= 2;
                    }
                }
                NumericValue::NumberArrayQuantifier(operation, every) => {
                    if depth < 2 {
                        return None;
                    }
                    type QuantifierFn = extern "C" fn(f64, f64) -> f64;
                    let functions: [QuantifierFn; 2] = match operation {
                        CompareOp::Less => [number_array_some_lt, number_array_every_lt],
                        CompareOp::LessEqual => [number_array_some_lte, number_array_every_lte],
                        CompareOp::Greater => [number_array_some_gt, number_array_every_gt],
                        CompareOp::GreaterEqual => [number_array_some_gte, number_array_every_gte],
                        CompareOp::Equal => [number_array_some_eq, number_array_every_eq],
                        CompareOp::NotEqual => [number_array_some_ne, number_array_every_ne],
                    };
                    emit_binary_call(
                        &mut code,
                        functions[usize::from(*every)] as *const () as u64,
                        depth - 2,
                    );
                    depth -= 1;
                }
                NumericValue::NumberArrayFind(operation, mode) => {
                    if depth < 2 {
                        return None;
                    }
                    type FindFn = extern "C" fn(f64, f64) -> f64;
                    let functions: [FindFn; 4] = match operation {
                        CompareOp::Less => [
                            number_array_find_lt,
                            number_array_find_index_lt,
                            number_array_find_last_lt,
                            number_array_find_last_index_lt,
                        ],
                        CompareOp::LessEqual => [
                            number_array_find_lte,
                            number_array_find_index_lte,
                            number_array_find_last_lte,
                            number_array_find_last_index_lte,
                        ],
                        CompareOp::Greater => [
                            number_array_find_gt,
                            number_array_find_index_gt,
                            number_array_find_last_gt,
                            number_array_find_last_index_gt,
                        ],
                        CompareOp::GreaterEqual => [
                            number_array_find_gte,
                            number_array_find_index_gte,
                            number_array_find_last_gte,
                            number_array_find_last_index_gte,
                        ],
                        CompareOp::Equal => [
                            number_array_find_eq,
                            number_array_find_index_eq,
                            number_array_find_last_eq,
                            number_array_find_last_index_eq,
                        ],
                        CompareOp::NotEqual => [
                            number_array_find_ne,
                            number_array_find_index_ne,
                            number_array_find_last_ne,
                            number_array_find_last_index_ne,
                        ],
                    };
                    emit_binary_call(
                        &mut code,
                        functions[*mode as usize] as *const () as u64,
                        depth - 2,
                    );
                    depth -= 1;
                }
                NumericValue::NumberArrayFilter(operation) => {
                    if depth < 2 {
                        return None;
                    }
                    let function = match operation {
                        CompareOp::Less => number_array_filter_lt,
                        CompareOp::LessEqual => number_array_filter_lte,
                        CompareOp::Greater => number_array_filter_gt,
                        CompareOp::GreaterEqual => number_array_filter_gte,
                        CompareOp::Equal => number_array_filter_eq,
                        CompareOp::NotEqual => number_array_filter_ne,
                    };
                    emit_binary_call(&mut code, function as *const () as u64, depth - 2);
                    depth -= 1;
                }
                NumericValue::PrimitiveArrayTruthy(kind, mode) => {
                    if depth == 0 || depth == 8 {
                        return None;
                    }
                    code.extend_from_slice(&[0x48, 0xb8]);
                    code.extend_from_slice(&f64::from(kind * 8 + mode).to_bits().to_le_bytes());
                    code.extend_from_slice(&[0x66, 0x48, 0x0f, 0x6e, 0xc0 | (depth << 3)]);
                    emit_binary_call(
                        &mut code,
                        primitive_array_truthy as *const () as u64,
                        depth - 1,
                    );
                }
                NumericValue::DynamicArrayTruthy(mode) => {
                    if depth == 0 || depth == 8 {
                        return None;
                    }
                    code.extend_from_slice(&[0x48, 0xb8]);
                    code.extend_from_slice(&f64::from(*mode).to_bits().to_le_bytes());
                    code.extend_from_slice(&[0x66, 0x48, 0x0f, 0x6e, 0xc0 | (depth << 3)]);
                    emit_binary_call(
                        &mut code,
                        dynamic_array_truthy as *const () as u64,
                        depth - 1,
                    );
                }
                NumericValue::PrimitiveArrayCompare(kind, operation, mode) => {
                    if depth < 2 || depth == 8 {
                        return None;
                    }
                    code.extend_from_slice(&[0x48, 0xb8]);
                    code.extend_from_slice(
                        &f64::from(kind * 64 + mode * 8 + *operation as u8)
                            .to_bits()
                            .to_le_bytes(),
                    );
                    code.extend_from_slice(&[0x66, 0x48, 0x0f, 0x6e, 0xc0 | (depth << 3)]);
                    emit_ternary_call(
                        &mut code,
                        primitive_array_compare as *const () as u64,
                        depth - 2,
                    );
                    depth -= 1;
                }
                NumericValue::DynamicArrayCompare(operation, mode) => {
                    if depth < 2 || depth == 8 {
                        return None;
                    }
                    code.extend_from_slice(&[0x48, 0xb8]);
                    code.extend_from_slice(
                        &f64::from(mode * 8 + operation).to_bits().to_le_bytes(),
                    );
                    code.extend_from_slice(&[0x66, 0x48, 0x0f, 0x6e, 0xc0 | (depth << 3)]);
                    emit_ternary_call(
                        &mut code,
                        dynamic_array_compare as *const () as u64,
                        depth - 2,
                    );
                    depth -= 1;
                }
                NumericValue::PrimitiveArrayMap(kind, operation) => {
                    if depth == 0 || depth == 8 {
                        return None;
                    }
                    code.extend_from_slice(&[0x48, 0xb8]);
                    code.extend_from_slice(
                        &f64::from(kind * 8 + operation).to_bits().to_le_bytes(),
                    );
                    code.extend_from_slice(&[0x66, 0x48, 0x0f, 0x6e, 0xc0 | (depth << 3)]);
                    emit_binary_call(
                        &mut code,
                        primitive_array_map as *const () as u64,
                        depth - 1,
                    );
                }
                NumericValue::PrimitiveArrayConvert(source, target) => {
                    if depth == 0 || depth == 8 {
                        return None;
                    }
                    code.extend_from_slice(&[0x48, 0xb8]);
                    code.extend_from_slice(&f64::from(source * 4 + target).to_bits().to_le_bytes());
                    code.extend_from_slice(&[0x66, 0x48, 0x0f, 0x6e, 0xc0 | (depth << 3)]);
                    emit_binary_call(
                        &mut code,
                        primitive_array_convert as *const () as u64,
                        depth - 1,
                    );
                }
                NumericValue::DynamicArrayConvert(target) => {
                    if depth == 0 || depth == 8 {
                        return None;
                    }
                    code.extend_from_slice(&[0x48, 0xb8]);
                    code.extend_from_slice(&f64::from(*target).to_bits().to_le_bytes());
                    code.extend_from_slice(&[0x66, 0x48, 0x0f, 0x6e, 0xc0 | (depth << 3)]);
                    emit_binary_call(
                        &mut code,
                        dynamic_array_convert as *const () as u64,
                        depth - 1,
                    );
                }
                NumericValue::DynamicArrayMapIdentity => {
                    if depth == 0 {
                        return None;
                    }
                    emit_unary_call(
                        &mut code,
                        dynamic_array_map_identity as *const () as u64,
                        depth - 1,
                    );
                }
                NumericValue::DynamicArrayJitMap(target, captured) => {
                    if depth == 8 {
                        return None;
                    }
                    code.extend_from_slice(&[0x48, 0xb8]);
                    code.extend_from_slice(&f64::from(*target).to_bits().to_le_bytes());
                    code.extend_from_slice(&[0x66, 0x48, 0x0f, 0x6e, 0xc0 | (depth << 3)]);
                    if *captured {
                        if depth < 3 {
                            return None;
                        }
                        emit_quaternary_call(
                            &mut code,
                            dynamic_array_jit_map_captured as *const () as u64,
                            depth - 3,
                        );
                        depth -= 2;
                    } else {
                        if depth < 2 {
                            return None;
                        }
                        emit_ternary_call(
                            &mut code,
                            dynamic_array_jit_map as *const () as u64,
                            depth - 2,
                        );
                        depth -= 1;
                    }
                }
                NumericValue::DynamicArrayJitScan(mode, captured) => {
                    if depth == 8 {
                        return None;
                    }
                    code.extend_from_slice(&[0x48, 0xb8]);
                    code.extend_from_slice(&f64::from(*mode).to_bits().to_le_bytes());
                    code.extend_from_slice(&[0x66, 0x48, 0x0f, 0x6e, 0xc0 | (depth << 3)]);
                    if *captured {
                        if depth < 3 {
                            return None;
                        }
                        emit_quaternary_call(
                            &mut code,
                            dynamic_array_jit_scan_captured as *const () as u64,
                            depth - 3,
                        );
                        depth -= 2;
                    } else {
                        if depth < 2 {
                            return None;
                        }
                        emit_ternary_call(
                            &mut code,
                            dynamic_array_jit_scan as *const () as u64,
                            depth - 2,
                        );
                        depth -= 1;
                    }
                }
                NumericValue::DynamicArrayJitReduce(has_initial, from_right, captured) => {
                    if *captured {
                        if depth < 4 {
                            return None;
                        }
                        emit_quaternary_call(
                            &mut code,
                            if !*has_initial && *from_right {
                                dynamic_array_jit_reduce_right_unseeded_captured
                            } else if !*has_initial {
                                dynamic_array_jit_reduce_left_unseeded_captured
                            } else if *from_right {
                                dynamic_array_jit_reduce_right_captured
                            } else {
                                dynamic_array_jit_reduce_left_captured
                            } as *const () as u64,
                            depth - 4,
                        );
                        depth -= 3;
                    } else {
                        if depth < 3 {
                            return None;
                        }
                        emit_ternary_call(
                            &mut code,
                            if !*has_initial && *from_right {
                                dynamic_array_jit_reduce_right_unseeded
                            } else if !*has_initial {
                                dynamic_array_jit_reduce_left_unseeded
                            } else if *from_right {
                                dynamic_array_jit_reduce_right
                            } else {
                                dynamic_array_jit_reduce_left
                            } as *const () as u64,
                            depth - 3,
                        );
                        depth -= 2;
                    }
                }
                NumericValue::NumberArrayMap(operation, reverse) => {
                    if depth < 2 {
                        return None;
                    }
                    let functions = match operation {
                        NumericReduceOp::Add => {
                            [number_array_map_add, number_array_map_add_reverse]
                        }
                        NumericReduceOp::Subtract => {
                            [number_array_map_subtract, number_array_map_subtract_reverse]
                        }
                        NumericReduceOp::Multiply => {
                            [number_array_map_multiply, number_array_map_multiply_reverse]
                        }
                        NumericReduceOp::Divide => {
                            [number_array_map_divide, number_array_map_divide_reverse]
                        }
                        NumericReduceOp::Remainder => [
                            number_array_map_remainder,
                            number_array_map_remainder_reverse,
                        ],
                        NumericReduceOp::Power => {
                            [number_array_map_power, number_array_map_power_reverse]
                        }
                        NumericReduceOp::Minimum | NumericReduceOp::Maximum => return None,
                    };
                    emit_binary_call(
                        &mut code,
                        functions[usize::from(*reverse)] as *const () as u64,
                        depth - 2,
                    );
                    depth -= 1;
                }
                NumericValue::PrimitiveArrayJitMap(source, target, captured) => {
                    if depth == 8 {
                        return None;
                    }
                    code.extend_from_slice(&[0x48, 0xb8]);
                    code.extend_from_slice(&f64::from(source * 4 + target).to_bits().to_le_bytes());
                    code.extend_from_slice(&[0x66, 0x48, 0x0f, 0x6e, 0xc0 | (depth << 3)]);
                    if *captured {
                        if depth < 3 {
                            return None;
                        }
                        emit_quaternary_call(
                            &mut code,
                            primitive_array_jit_map_captured as *const () as u64,
                            depth - 3,
                        );
                        depth -= 2;
                    } else {
                        if depth < 2 {
                            return None;
                        }
                        emit_ternary_call(
                            &mut code,
                            primitive_array_jit_map as *const () as u64,
                            depth - 2,
                        );
                        depth -= 1;
                    }
                }
                NumericValue::PrimitiveArrayJitScan(kind, mode, captured) => {
                    if depth == 8 {
                        return None;
                    }
                    code.extend_from_slice(&[0x48, 0xb8]);
                    code.extend_from_slice(&f64::from(kind * 8 + mode).to_bits().to_le_bytes());
                    code.extend_from_slice(&[0x66, 0x48, 0x0f, 0x6e, 0xc0 | (depth << 3)]);
                    if *captured {
                        if depth < 3 {
                            return None;
                        }
                        emit_quaternary_call(
                            &mut code,
                            primitive_array_jit_scan_captured as *const () as u64,
                            depth - 3,
                        );
                        depth -= 2;
                    } else {
                        if depth < 2 {
                            return None;
                        }
                        emit_ternary_call(
                            &mut code,
                            primitive_array_jit_scan as *const () as u64,
                            depth - 2,
                        );
                        depth -= 1;
                    }
                }
                NumericValue::NumberArrayIndexMap(operation, reverse) => {
                    if depth == 0 || depth == 8 {
                        return None;
                    }
                    let operation = match operation {
                        NumericReduceOp::Add => 0,
                        NumericReduceOp::Subtract => 1,
                        NumericReduceOp::Multiply => 2,
                        NumericReduceOp::Divide => 3,
                        NumericReduceOp::Remainder => 4,
                        NumericReduceOp::Power => 5,
                        NumericReduceOp::Minimum | NumericReduceOp::Maximum => return None,
                    } + if *reverse { 8 } else { 0 };
                    code.extend_from_slice(&[0x48, 0xb8]);
                    code.extend_from_slice(&f64::from(operation).to_bits().to_le_bytes());
                    code.extend_from_slice(&[0x66, 0x48, 0x0f, 0x6e, 0xc0 | (depth << 3)]);
                    emit_binary_call(
                        &mut code,
                        number_array_index_map as *const () as u64,
                        depth - 1,
                    );
                }
                NumericValue::NumberArraySelectMap(operation, mode) => {
                    if depth < 2 || depth == 8 {
                        return None;
                    }
                    code.extend_from_slice(&[0x48, 0xb8]);
                    code.extend_from_slice(
                        &f64::from(*operation as u8 + mode * 8)
                            .to_bits()
                            .to_le_bytes(),
                    );
                    code.extend_from_slice(&[0x66, 0x48, 0x0f, 0x6e, 0xc0 | (depth << 3)]);
                    emit_ternary_call(
                        &mut code,
                        number_array_select_map as *const () as u64,
                        depth - 2,
                    );
                    depth -= 1;
                }
                NumericValue::NumberArrayBranchMap(encoded) => {
                    if depth < 2 || depth == 8 {
                        return None;
                    }
                    code.extend_from_slice(&[0x48, 0xb8]);
                    code.extend_from_slice(&f64::from(*encoded).to_bits().to_le_bytes());
                    code.extend_from_slice(&[0x66, 0x48, 0x0f, 0x6e, 0xc0 | (depth << 3)]);
                    emit_ternary_call(
                        &mut code,
                        number_array_branch_map as *const () as u64,
                        depth - 2,
                    );
                    depth -= 1;
                }
                NumericValue::NumberArrayUnaryMap(absolute) => {
                    if depth == 0 {
                        return None;
                    }
                    let function = if *absolute {
                        number_array_map_absolute
                    } else {
                        number_array_map_negate
                    };
                    emit_unary_call(&mut code, function as *const () as u64, depth - 1);
                }
                NumericValue::NumberArrayMathMap(operation) => {
                    if depth == 0 || depth == 8 {
                        return None;
                    }
                    code.extend_from_slice(&[0x48, 0xb8]);
                    code.extend_from_slice(&f64::from(*operation as u8).to_bits().to_le_bytes());
                    code.extend_from_slice(&[0x66, 0x48, 0x0f, 0x6e, 0xc0 | (depth << 3)]);
                    emit_binary_call(
                        &mut code,
                        number_array_map_math as *const () as u64,
                        depth - 1,
                    );
                }
                NumericValue::ArraySlice => {
                    if depth < 3 {
                        return None;
                    }
                    emit_ternary_call(&mut code, array_slice as *const () as u64, depth - 3);
                    depth -= 2;
                }
                NumericValue::DynamicArraySlice => {
                    if depth < 3 {
                        return None;
                    }
                    emit_ternary_call(
                        &mut code,
                        dynamic_array_slice as *const () as u64,
                        depth - 3,
                    );
                    depth -= 2;
                }
                NumericValue::ArrayConcat => {
                    if depth < 2 {
                        return None;
                    }
                    emit_binary_call(&mut code, array_concat as *const () as u64, depth - 2);
                    depth -= 1;
                }
                NumericValue::DynamicArrayConcat => {
                    if depth < 2 {
                        return None;
                    }
                    emit_binary_call(
                        &mut code,
                        dynamic_array_concat as *const () as u64,
                        depth - 2,
                    );
                    depth -= 1;
                }
                NumericValue::NumberArrayAppend
                | NumericValue::StringArrayAppend
                | NumericValue::BoolArrayAppend
                | NumericValue::DynamicArrayAppend => {
                    if depth < 2 {
                        return None;
                    }
                    let function = match value {
                        NumericValue::NumberArrayAppend => number_array_append,
                        NumericValue::StringArrayAppend => string_array_append,
                        NumericValue::BoolArrayAppend => bool_array_append,
                        NumericValue::DynamicArrayAppend => dynamic_array_append,
                        _ => unreachable!(),
                    };
                    emit_binary_call(&mut code, function as *const () as u64, depth - 2);
                    depth -= 1;
                }
                NumericValue::ArrayToReversed => {
                    if depth == 0 {
                        return None;
                    }
                    emit_unary_call(&mut code, array_to_reversed as *const () as u64, depth - 1);
                }
                NumericValue::ArrayReverse => {
                    if depth == 0 {
                        return None;
                    }
                    emit_unary_call(&mut code, array_reverse as *const () as u64, depth - 1);
                }
                NumericValue::DynamicArrayToReversed | NumericValue::DynamicArrayReverse => {
                    if depth == 0 {
                        return None;
                    }
                    let function = if *value == NumericValue::DynamicArrayReverse {
                        dynamic_array_reverse_in_place
                    } else {
                        dynamic_array_to_reversed
                    };
                    emit_unary_call(&mut code, function as *const () as u64, depth - 1);
                }
                NumericValue::NumberArrayToSorted
                | NumericValue::StringArrayToSorted
                | NumericValue::BoolArrayToSorted
                | NumericValue::DynamicArrayToSorted => {
                    if depth == 0 {
                        return None;
                    }
                    let function = match value {
                        NumericValue::NumberArrayToSorted => number_array_to_sorted,
                        NumericValue::StringArrayToSorted => string_array_to_sorted,
                        NumericValue::BoolArrayToSorted => bool_array_to_sorted,
                        NumericValue::DynamicArrayToSorted => dynamic_array_to_sorted,
                        _ => unreachable!(),
                    };
                    emit_unary_call(&mut code, function as *const () as u64, depth - 1);
                }
                NumericValue::NumberArraySort
                | NumericValue::StringArraySort
                | NumericValue::BoolArraySort
                | NumericValue::DynamicArraySort => {
                    if depth == 0 {
                        return None;
                    }
                    let function = match value {
                        NumericValue::NumberArraySort => number_array_sort,
                        NumericValue::StringArraySort => string_array_sort,
                        NumericValue::BoolArraySort => bool_array_sort,
                        NumericValue::DynamicArraySort => dynamic_array_sort_in_place,
                        _ => unreachable!(),
                    };
                    emit_unary_call(&mut code, function as *const () as u64, depth - 1);
                }
                NumericValue::NumberArrayToSortedBy(descending)
                | NumericValue::NumberArraySortBy(descending) => {
                    if depth == 0 {
                        return None;
                    }
                    let function = match (value, descending) {
                        (NumericValue::NumberArrayToSortedBy(_), false) => {
                            number_array_to_sorted_ascending
                        }
                        (NumericValue::NumberArrayToSortedBy(_), true) => {
                            number_array_to_sorted_descending
                        }
                        (NumericValue::NumberArraySortBy(_), false) => number_array_sort_ascending,
                        (NumericValue::NumberArraySortBy(_), true) => number_array_sort_descending,
                        _ => unreachable!(),
                    };
                    emit_unary_call(&mut code, function as *const () as u64, depth - 1);
                }
                NumericValue::StringArrayToSortedDescending
                | NumericValue::StringArraySortDescending => {
                    if depth == 0 {
                        return None;
                    }
                    let function = match value {
                        NumericValue::StringArrayToSortedDescending => {
                            string_array_to_sorted_descending
                        }
                        NumericValue::StringArraySortDescending => string_array_sort_descending,
                        _ => unreachable!(),
                    };
                    emit_unary_call(&mut code, function as *const () as u64, depth - 1);
                }
                NumericValue::NumberArrayFill
                | NumericValue::StringArrayFill
                | NumericValue::BoolArrayFill
                | NumericValue::DynamicArrayFill => {
                    if depth < 4 {
                        return None;
                    }
                    let function = match value {
                        NumericValue::NumberArrayFill => number_array_fill,
                        NumericValue::StringArrayFill => string_array_fill,
                        NumericValue::BoolArrayFill => bool_array_fill,
                        NumericValue::DynamicArrayFill => dynamic_array_fill,
                        _ => unreachable!(),
                    };
                    emit_quaternary_call(&mut code, function as *const () as u64, depth - 4);
                    depth -= 3;
                }
                NumericValue::ArrayCopyWithin | NumericValue::DynamicArrayCopyWithin => {
                    if depth < 4 {
                        return None;
                    }
                    let function = if *value == NumericValue::DynamicArrayCopyWithin {
                        dynamic_array_copy_within
                    } else {
                        array_copy_within
                    };
                    emit_quaternary_call(&mut code, function as *const () as u64, depth - 4);
                    depth -= 3;
                }
                NumericValue::ArraySplice => {
                    if depth < 4 {
                        return None;
                    }
                    emit_quaternary_call(&mut code, array_splice as *const () as u64, depth - 4);
                    depth -= 3;
                }
                NumericValue::ArrayToSpliced => {
                    if depth < 4 {
                        return None;
                    }
                    emit_quaternary_call(
                        &mut code,
                        array_to_spliced as *const () as u64,
                        depth - 4,
                    );
                    depth -= 3;
                }
                NumericValue::DynamicArraySplice | NumericValue::DynamicArrayToSpliced => {
                    if depth < 4 {
                        return None;
                    }
                    let function = if *value == NumericValue::DynamicArrayToSpliced {
                        dynamic_array_to_spliced
                    } else {
                        dynamic_array_splice_in_place
                    };
                    emit_quaternary_call(&mut code, function as *const () as u64, depth - 4);
                    depth -= 3;
                }
                NumericValue::NumberArrayPush
                | NumericValue::StringArrayPush
                | NumericValue::BoolArrayPush
                | NumericValue::DynamicArrayPush => {
                    if depth < 2 {
                        return None;
                    }
                    let function = match value {
                        NumericValue::NumberArrayPush => number_array_push,
                        NumericValue::StringArrayPush => string_array_push,
                        NumericValue::BoolArrayPush => bool_array_push,
                        NumericValue::DynamicArrayPush => dynamic_array_push,
                        _ => unreachable!(),
                    };
                    emit_binary_call(&mut code, function as *const () as u64, depth - 2);
                    depth -= 1;
                }
                NumericValue::NumberArrayUnshift
                | NumericValue::StringArrayUnshift
                | NumericValue::BoolArrayUnshift
                | NumericValue::DynamicArrayUnshift => {
                    if depth < 2 {
                        return None;
                    }
                    let function = match value {
                        NumericValue::NumberArrayUnshift => number_array_unshift,
                        NumericValue::StringArrayUnshift => string_array_unshift,
                        NumericValue::BoolArrayUnshift => bool_array_unshift,
                        NumericValue::DynamicArrayUnshift => dynamic_array_unshift,
                        _ => unreachable!(),
                    };
                    emit_binary_call(&mut code, function as *const () as u64, depth - 2);
                    depth -= 1;
                }
                NumericValue::NumberArraySet
                | NumericValue::StringArraySet
                | NumericValue::BoolArraySet
                | NumericValue::DynamicArraySet => {
                    if depth < 3 {
                        return None;
                    }
                    let function = match value {
                        NumericValue::NumberArraySet => number_array_set,
                        NumericValue::StringArraySet => string_array_set,
                        NumericValue::BoolArraySet => bool_array_set,
                        NumericValue::DynamicArraySet => dynamic_array_set,
                        _ => unreachable!(),
                    };
                    emit_ternary_call(&mut code, function as *const () as u64, depth - 3);
                    depth -= 2;
                }
                NumericValue::AggregateLocalSet(kind, index) => {
                    if depth < 2 || *index >= depth - 2 {
                        return None;
                    }
                    let function = match kind {
                        0 => number_array_set,
                        1 => bool_array_set,
                        2 => string_array_set,
                        3 => number_dictionary_set,
                        4 => bool_dictionary_set,
                        5 => string_dictionary_set,
                        _ => return None,
                    };
                    let destination = depth - 2;
                    emit_spill(&mut code, destination);
                    emit_move(&mut code, 0, *index);
                    emit_move(&mut code, 1, destination);
                    emit_move(&mut code, 2, destination + 1);
                    code.extend_from_slice(&[0x48, 0xb8]);
                    code.extend_from_slice(&(function as *const () as u64).to_le_bytes());
                    code.extend_from_slice(&[0xff, 0xd0]);
                    emit_move(&mut code, destination, 0);
                    emit_restore(&mut code, destination);
                    depth -= 1;
                }
                NumericValue::AggregateLocalArrayInsert(kind, index, unshift) => {
                    if depth == 0 || *index >= depth - 1 {
                        return None;
                    }
                    let function = match (kind, unshift) {
                        (0, false) => number_array_push,
                        (1, false) => bool_array_push,
                        (2, false) => string_array_push,
                        (0, true) => number_array_unshift,
                        (1, true) => bool_array_unshift,
                        (2, true) => string_array_unshift,
                        _ => return None,
                    };
                    let destination = depth - 1;
                    emit_spill(&mut code, destination);
                    emit_move(&mut code, 0, *index);
                    emit_move(&mut code, 1, destination);
                    code.extend_from_slice(&[0x48, 0xb8]);
                    code.extend_from_slice(&(function as *const () as u64).to_le_bytes());
                    code.extend_from_slice(&[0xff, 0xd0]);
                    emit_move(&mut code, destination, 0);
                    emit_restore(&mut code, destination);
                }
                NumericValue::NumberArrayPostSet => {
                    if depth < 4 {
                        return None;
                    }
                    let left = depth - 4;
                    emit_spill(&mut code, depth);
                    emit_move(&mut code, 0, left);
                    emit_move(&mut code, 1, left + 1);
                    emit_move(&mut code, 2, left + 3);
                    code.extend_from_slice(&[0x48, 0xb8]);
                    code.extend_from_slice(&(number_array_set as *const () as u64).to_le_bytes());
                    code.extend_from_slice(&[0xff, 0xd0]);
                    emit_restore(&mut code, depth);
                    emit_move(&mut code, left, left + 2);
                    depth -= 3;
                }
                NumericValue::ArrayValue => {
                    if depth == 0 {
                        return None;
                    }
                    emit_unary_call(&mut code, array_value as *const () as u64, depth - 1);
                }
                NumericValue::MutableArrayHandle => {
                    if depth == 0 {
                        return None;
                    }
                    emit_unary_call(
                        &mut code,
                        mutable_array_handle as *const () as u64,
                        depth - 1,
                    );
                }
                NumericValue::EmptyArray => {
                    if depth > 7 {
                        return None;
                    }
                    emit_spill(&mut code, depth);
                    code.extend_from_slice(&[0x48, 0xb8]);
                    code.extend_from_slice(&(empty_array as *const () as u64).to_le_bytes());
                    code.extend_from_slice(&[0xff, 0xd0]);
                    emit_move(&mut code, depth, 0);
                    emit_restore(&mut code, depth);
                    depth += 1;
                }
                NumericValue::NumberArrayPop
                | NumericValue::StringArrayPop
                | NumericValue::BoolArrayPop
                | NumericValue::DynamicArrayPop
                | NumericValue::NumberArrayShift
                | NumericValue::StringArrayShift
                | NumericValue::BoolArrayShift
                | NumericValue::DynamicArrayShift => {
                    if depth == 0 {
                        return None;
                    }
                    let function = match value {
                        NumericValue::NumberArrayPop => number_array_pop,
                        NumericValue::StringArrayPop => string_array_pop,
                        NumericValue::BoolArrayPop => bool_array_pop,
                        NumericValue::DynamicArrayPop => dynamic_array_pop,
                        NumericValue::NumberArrayShift => number_array_shift,
                        NumericValue::StringArrayShift => string_array_shift,
                        NumericValue::BoolArrayShift => bool_array_shift,
                        NumericValue::DynamicArrayShift => dynamic_array_shift,
                        _ => unreachable!(),
                    };
                    emit_unary_call(&mut code, function as *const () as u64, depth - 1);
                }
                NumericValue::Drop => {
                    if depth == 0 {
                        return None;
                    }
                    depth -= 1;
                }
                NumericValue::DropUnder => {
                    if depth < 2 {
                        return None;
                    }
                    emit_move(&mut code, depth - 2, depth - 1);
                    depth -= 1;
                }
                NumericValue::Duplicate => {
                    if !(1..=7).contains(&depth) {
                        return None;
                    }
                    emit_move(&mut code, depth, depth - 1);
                    depth += 1;
                }
                NumericValue::DuplicatePair => {
                    if !(2..=6).contains(&depth) {
                        return None;
                    }
                    emit_move(&mut code, depth, depth - 2);
                    emit_move(&mut code, depth + 1, depth - 1);
                    depth += 2;
                }
                NumericValue::LocalGet(index) => {
                    if *index >= depth || depth == 8 {
                        return None;
                    }
                    emit_move(&mut code, depth, *index);
                    depth += 1;
                }
                NumericValue::LocalSet(index) => {
                    if depth == 0 || *index >= depth - 1 {
                        return None;
                    }
                    emit_move(&mut code, *index, depth - 1);
                    depth -= 1;
                }
                NumericValue::LoopStart => loops.push(LoopPatch {
                    start: code.len(),
                    continue_target: None,
                    base_depth: depth,
                    condition_exits: Vec::new(),
                    continues: Vec::new(),
                    breaks: Vec::new(),
                }),
                NumericValue::LoopWhile => {
                    let loop_patch = loops.last_mut()?;
                    if depth != loop_patch.base_depth + 1 {
                        return None;
                    }
                    let condition = depth - 1;
                    code.extend_from_slice(&[
                        0x66,
                        0x0f,
                        0x2e,
                        0xc0 | (condition << 3) | condition,
                    ]);
                    let parity = emit_near_jump(&mut code, 0x8a);
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
                    ]);
                    loop_patch
                        .condition_exits
                        .extend([parity, emit_near_jump(&mut code, 0x84)]);
                    depth -= 1;
                }
                NumericValue::LoopContinuePoint => {
                    let loop_patch = loops.last_mut()?;
                    if depth != loop_patch.base_depth || loop_patch.continue_target.is_some() {
                        return None;
                    }
                    for jump in loop_patch.continues.drain(..) {
                        patch_near_jump(&mut code, jump)?;
                    }
                    loop_patch.continue_target = Some(code.len());
                }
                NumericValue::LoopBreak(target_depth) => {
                    let target = loops
                        .len()
                        .checked_sub(usize::from(*target_depth).checked_add(1)?)?;
                    let loop_patch = loops.get_mut(target)?;
                    if depth < loop_patch.base_depth {
                        return None;
                    }
                    loop_patch.breaks.push(emit_unconditional_jump(&mut code));
                }
                NumericValue::LoopContinue(target_depth) => {
                    let target = loops
                        .len()
                        .checked_sub(usize::from(*target_depth).checked_add(1)?)?;
                    let loop_patch = loops.get_mut(target)?;
                    if depth < loop_patch.base_depth {
                        return None;
                    }
                    if let Some(target) = loop_patch.continue_target {
                        emit_backward_jump(&mut code, target)?;
                    } else {
                        loop_patch
                            .continues
                            .push(emit_unconditional_jump(&mut code));
                    }
                }
                NumericValue::LoopEnd => {
                    let loop_patch = loops.pop()?;
                    if depth != loop_patch.base_depth
                        || loop_patch.continue_target.is_none()
                        || !loop_patch.continues.is_empty()
                    {
                        return None;
                    }
                    emit_backward_jump(&mut code, loop_patch.start)?;
                    for exit in loop_patch
                        .condition_exits
                        .into_iter()
                        .chain(loop_patch.breaks)
                    {
                        patch_near_jump(&mut code, exit)?;
                    }
                }
                NumericValue::GuardStart => {
                    if depth == 0 {
                        return None;
                    }
                    let condition = depth - 1;
                    code.extend_from_slice(&[
                        0x66,
                        0x0f,
                        0x2e,
                        0xc0 | (condition << 3) | condition,
                    ]);
                    let parity = emit_near_jump(&mut code, 0x8a);
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
                    ]);
                    let zero = emit_near_jump(&mut code, 0x84);
                    depth -= 1;
                    guards.push((depth, vec![parity, zero], Vec::new(), false));
                }
                NumericValue::GuardAlternate => {
                    let (base_depth, false_exits, end_exits, has_alternate) = guards.last_mut()?;
                    if *has_alternate || depth != *base_depth {
                        return None;
                    }
                    end_exits.push(emit_unconditional_jump(&mut code));
                    for exit in false_exits.drain(..) {
                        patch_near_jump(&mut code, exit)?;
                    }
                    *has_alternate = true;
                }
                NumericValue::GuardEnd => {
                    let (base_depth, false_exits, end_exits, _) = guards.pop()?;
                    if depth != base_depth {
                        return None;
                    }
                    for exit in false_exits.into_iter().chain(end_exits) {
                        patch_near_jump(&mut code, exit)?;
                    }
                }
                NumericValue::SwitchStart => {
                    if depth == 0 {
                        return None;
                    }
                    switches.push(SwitchPatch {
                        base_depth: depth,
                        next_case: Vec::new(),
                        fallthrough: None,
                        breaks: Vec::new(),
                        has_case: false,
                        has_default: false,
                        default_body: None,
                    });
                }
                NumericValue::SwitchCaseStart => {
                    let switch = switches.last_mut()?;
                    if depth != switch.base_depth {
                        return None;
                    }
                    if switch.has_case {
                        switch.fallthrough = Some(emit_unconditional_jump(&mut code));
                    }
                    for next in switch.next_case.drain(..) {
                        patch_near_jump(&mut code, next)?;
                    }
                    switch.has_case = true;
                }
                NumericValue::SwitchCaseBody => {
                    let switch = switches.last_mut()?;
                    if depth != switch.base_depth + 1 {
                        return None;
                    }
                    let condition = depth - 1;
                    code.extend_from_slice(&[
                        0x66,
                        0x0f,
                        0x2e,
                        0xc0 | (condition << 3) | condition,
                    ]);
                    let parity = emit_near_jump(&mut code, 0x8a);
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
                    ]);
                    let mismatch = emit_near_jump(&mut code, 0x84);
                    switch.next_case.extend([parity, mismatch]);
                    if let Some(fallthrough) = switch.fallthrough.take() {
                        patch_near_jump(&mut code, fallthrough)?;
                    }
                    depth -= 1;
                }
                NumericValue::SwitchDefault => {
                    let switch = switches.last_mut()?;
                    if depth != switch.base_depth || switch.has_default {
                        return None;
                    }
                    let fallthrough = switch.has_case.then(|| emit_unconditional_jump(&mut code));
                    for mismatch in switch.next_case.drain(..) {
                        patch_near_jump(&mut code, mismatch)?;
                    }
                    switch.next_case.push(emit_unconditional_jump(&mut code));
                    let body = code.len();
                    if let Some(fallthrough) = fallthrough {
                        patch_near_jump(&mut code, fallthrough)?;
                    }
                    switch.has_default = true;
                    switch.default_body = Some(body);
                }
                NumericValue::SwitchBreak => {
                    let switch = switches.last_mut()?;
                    if depth < switch.base_depth {
                        return None;
                    }
                    switch.breaks.push(emit_unconditional_jump(&mut code));
                }
                NumericValue::SwitchEnd => {
                    let switch = switches.pop()?;
                    if depth != switch.base_depth {
                        return None;
                    }
                    let unmatched = switch.default_body.unwrap_or(code.len());
                    for exit in switch.next_case {
                        patch_jump_to(&mut code, exit, unmatched)?;
                    }
                    for exit in switch.breaks {
                        patch_near_jump(&mut code, exit)?;
                    }
                    depth -= 1;
                }
                NumericValue::TryStart | NumericValue::TaggedTryStart => tries.push(TryPatch {
                    base_depth: depth,
                    throws: Vec::new(),
                    tagged: matches!(value, NumericValue::TaggedTryStart),
                }),
                NumericValue::Throw => {
                    let exception = tries.last_mut()?;
                    if exception.tagged || depth <= exception.base_depth {
                        return None;
                    }
                    emit_move(&mut code, exception.base_depth, depth - 1);
                    exception.throws.push(emit_unconditional_jump(&mut code));
                    depth -= 1;
                }
                NumericValue::TaggedThrow(tag) => {
                    let exception = tries.last_mut()?;
                    if !exception.tagged
                        || depth <= exception.base_depth
                        || exception.base_depth >= 7
                    {
                        return None;
                    }
                    emit_move(&mut code, exception.base_depth + 1, depth - 1);
                    code.extend_from_slice(&[0x48, 0xb8]);
                    code.extend_from_slice(&f64::from(*tag).to_bits().to_le_bytes());
                    code.extend_from_slice(&[
                        0x66,
                        0x48,
                        0x0f,
                        0x6e,
                        0xc0 | (exception.base_depth << 3),
                    ]);
                    exception.throws.push(emit_unconditional_jump(&mut code));
                    depth -= 1;
                }
                NumericValue::CheckError => {
                    let exception = tries.last_mut()?;
                    if exception.tagged && exception.base_depth >= 7 {
                        return None;
                    }
                    emit_spill(&mut code, depth);
                    code.extend_from_slice(&[0x48, 0xb8]);
                    code.extend_from_slice(&(take_call_error as *const () as u64).to_le_bytes());
                    code.extend_from_slice(&[0xff, 0xd0, 0x66, 0x48, 0x0f, 0x7e, 0xc0]);
                    emit_restore(&mut code, depth);
                    code.extend_from_slice(&[0x48, 0x85, 0xc0]);
                    let no_error = emit_near_jump(&mut code, 0x84);
                    let value_slot = exception.base_depth + u8::from(exception.tagged);
                    code.extend_from_slice(&[0x66, 0x48, 0x0f, 0x6e, 0xc0 | (value_slot << 3)]);
                    if exception.tagged {
                        code.extend_from_slice(&[0x48, 0xb8]);
                        code.extend_from_slice(&2.0_f64.to_bits().to_le_bytes());
                        code.extend_from_slice(&[
                            0x66,
                            0x48,
                            0x0f,
                            0x6e,
                            0xc0 | (exception.base_depth << 3),
                        ]);
                    }
                    exception.throws.push(emit_unconditional_jump(&mut code));
                    patch_near_jump(&mut code, no_error)?;
                }
                NumericValue::UncaughtNumberThrow
                | NumericValue::UncaughtBooleanThrow
                | NumericValue::UncaughtStringThrow
                | NumericValue::UncaughtNumberArrayThrow
                | NumericValue::UncaughtBooleanArrayThrow
                | NumericValue::UncaughtStringArrayThrow
                | NumericValue::UncaughtDictionaryThrow => {
                    if depth == 0 {
                        return None;
                    }
                    let function = match value {
                        NumericValue::UncaughtNumberThrow => uncaught_number_throw,
                        NumericValue::UncaughtBooleanThrow => uncaught_boolean_throw,
                        NumericValue::UncaughtStringThrow => uncaught_string_throw,
                        NumericValue::UncaughtNumberArrayThrow => uncaught_number_array_throw,
                        NumericValue::UncaughtBooleanArrayThrow => uncaught_boolean_array_throw,
                        NumericValue::UncaughtStringArrayThrow => uncaught_string_array_throw,
                        NumericValue::UncaughtDictionaryThrow => uncaught_dictionary_throw,
                        _ => unreachable!(),
                    };
                    emit_unary_call(&mut code, function as *const () as u64, depth - 1);
                    code.push(0xc3);
                    depth -= 1;
                }
                NumericValue::CatchStart => {
                    let exception = tries.pop()?;
                    if depth < exception.base_depth || exception.throws.is_empty() {
                        return None;
                    }
                    let result_depth = depth - exception.base_depth;
                    let normal_exit = emit_unconditional_jump(&mut code);
                    for jump in exception.throws {
                        patch_near_jump(&mut code, jump)?;
                    }
                    catches.push((exception.base_depth, normal_exit, result_depth));
                    depth = exception.base_depth + if exception.tagged { 2 } else { 1 };
                }
                NumericValue::TryEnd => {
                    let (base_depth, normal_exit, result_depth) = catches.pop()?;
                    if depth != base_depth + result_depth {
                        return None;
                    }
                    patch_near_jump(&mut code, normal_exit)?;
                }
                NumericValue::ResultStart => results.push(ResultPatch {
                    base_depth: depth,
                    result_depth: None,
                    exits: Vec::new(),
                }),
                NumericValue::ResultReturn(count) => {
                    let continuation_depth = loops
                        .last()
                        .map(|loop_patch| loop_patch.base_depth)
                        .into_iter()
                        .chain(guards.last().map(|guard| guard.0))
                        .chain(switches.last().map(|switch| switch.base_depth))
                        .max();
                    let result = results.last_mut()?;
                    if depth <= result.base_depth {
                        return None;
                    }
                    let result_depth = if *count == 0 {
                        depth - result.base_depth
                    } else {
                        *count
                    };
                    if depth < result.base_depth + result_depth {
                        return None;
                    }
                    if result
                        .result_depth
                        .replace(result_depth)
                        .is_some_and(|expected| expected != result_depth)
                    {
                        return None;
                    }
                    let source = depth - result_depth;
                    for offset in 0..result_depth {
                        emit_move(&mut code, result.base_depth + offset, source + offset);
                    }
                    result.exits.push(emit_unconditional_jump(&mut code));
                    depth = continuation_depth
                        .unwrap_or(result.base_depth)
                        .max(result.base_depth);
                }
                NumericValue::ResultEnd => {
                    let result = results.pop()?;
                    if depth != result.base_depth + result.result_depth? {
                        return None;
                    }
                    for exit in result.exits {
                        patch_near_jump(&mut code, exit)?;
                    }
                }
                NumericValue::EarlyReturn => {
                    let loop_patch = loops.last()?;
                    if depth <= loop_patch.base_depth {
                        return None;
                    }
                    emit_move(&mut code, 0, depth - 1);
                    code.push(0xc3);
                    depth -= 1;
                }
                NumericValue::MathRandom
                | NumericValue::DateNow
                | NumericValue::PerformanceNow
                | NumericValue::ProcessPid
                | NumericValue::ProcessPpid
                | NumericValue::MissingCallable
                | NumericValue::Absent
                | NumericValue::Null
                | NumericValue::PreserveAbsent => {
                    if depth > 7 {
                        return None;
                    }
                    emit_spill(&mut code, depth);
                    code.extend_from_slice(&[0x48, 0xb8]);
                    let function = match value {
                        NumericValue::MathRandom => math_random,
                        NumericValue::DateNow => date_now,
                        NumericValue::PerformanceNow => performance_now,
                        NumericValue::ProcessPid => process_pid,
                        NumericValue::ProcessPpid => process_ppid,
                        NumericValue::MissingCallable => missing_callable,
                        NumericValue::Absent => absent_value,
                        NumericValue::Null => null_value,
                        NumericValue::PreserveAbsent => preserve_absent_value,
                        _ => unreachable!(),
                    };
                    code.extend_from_slice(&(function as *const () as u64).to_le_bytes());
                    code.extend_from_slice(&[0xff, 0xd0]);
                    emit_move(&mut code, depth, 0);
                    emit_restore(&mut code, depth);
                    depth += 1;
                }
                NumericValue::NumberArrayWith
                | NumericValue::StringArrayWith
                | NumericValue::BoolArrayWith
                | NumericValue::DynamicArrayWith => {
                    if depth < 3 {
                        return None;
                    }
                    let function = match value {
                        NumericValue::NumberArrayWith => number_array_with,
                        NumericValue::StringArrayWith => string_array_with,
                        NumericValue::BoolArrayWith => bool_array_with,
                        NumericValue::DynamicArrayWith => dynamic_array_with,
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
                NumericValue::ArrayLength => {
                    if depth == 0 {
                        return None;
                    }
                    emit_unary_call(&mut code, array_length as *const () as u64, depth - 1);
                }
                NumericValue::DynamicArrayLength => {
                    if depth == 0 {
                        return None;
                    }
                    emit_unary_call(
                        &mut code,
                        dynamic_array_length as *const () as u64,
                        depth - 1,
                    );
                }
                NumericValue::IsArray | NumericValue::IsNotArray => {
                    if depth == 0 {
                        return None;
                    }
                    emit_unary_call(
                        &mut code,
                        if *value == NumericValue::IsArray {
                            is_array
                        } else {
                            is_not_array
                        } as *const () as u64,
                        depth - 1,
                    );
                }
                NumericValue::DynamicIsArray => {
                    if depth == 0 {
                        return None;
                    }
                    emit_unary_call(&mut code, dynamic_is_array as *const () as u64, depth - 1);
                }
                NumericValue::NumberArrayAt
                | NumericValue::BoolArrayAt
                | NumericValue::StringArrayAt
                | NumericValue::DynamicArrayAt
                | NumericValue::NumberArrayGet
                | NumericValue::BoolArrayGet
                | NumericValue::StringArrayGet => {
                    if depth < 2 {
                        return None;
                    }
                    let function = match value {
                        NumericValue::NumberArrayAt => number_array_at,
                        NumericValue::BoolArrayAt => bool_array_at,
                        NumericValue::StringArrayAt => string_array_at,
                        NumericValue::DynamicArrayAt => dynamic_array_at,
                        NumericValue::NumberArrayGet => number_array_get,
                        NumericValue::BoolArrayGet => bool_array_get,
                        NumericValue::StringArrayGet => string_array_get,
                        _ => unreachable!(),
                    };
                    emit_binary_call(&mut code, function as *const () as u64, depth - 2);
                    depth -= 1;
                }
                NumericValue::NumberDictionaryGet
                | NumericValue::BoolDictionaryGet
                | NumericValue::StringDictionaryGet
                | NumericValue::DictionaryDelete
                | NumericValue::DictionaryHasOwn
                | NumericValue::DictionaryIn
                | NumericValue::DictionaryAssign => {
                    if depth < 2 {
                        return None;
                    }
                    let function = match value {
                        NumericValue::NumberDictionaryGet => number_dictionary_get,
                        NumericValue::BoolDictionaryGet => bool_dictionary_get,
                        NumericValue::StringDictionaryGet => string_dictionary_get,
                        NumericValue::DictionaryDelete => dictionary_delete,
                        NumericValue::DictionaryHasOwn => dictionary_has_own,
                        NumericValue::DictionaryIn => dictionary_in,
                        NumericValue::DictionaryAssign => dictionary_assign,
                        _ => unreachable!(),
                    };
                    emit_binary_call(&mut code, function as *const () as u64, depth - 2);
                    depth -= 1;
                }
                NumericValue::DictionaryKeys
                | NumericValue::NumberDictionaryValues
                | NumericValue::BoolDictionaryValues
                | NumericValue::StringDictionaryValues
                | NumericValue::NumberDictionaryEntries
                | NumericValue::BoolDictionaryEntries
                | NumericValue::StringDictionaryEntries
                | NumericValue::DictionaryFromNumberEntries
                | NumericValue::DictionaryFromBoolEntries
                | NumericValue::DictionaryFromStringEntries => {
                    if depth == 0 {
                        return None;
                    }
                    let function = match value {
                        NumericValue::DictionaryKeys => dictionary_keys,
                        NumericValue::NumberDictionaryValues => number_dictionary_values,
                        NumericValue::BoolDictionaryValues => bool_dictionary_values,
                        NumericValue::StringDictionaryValues => string_dictionary_values,
                        NumericValue::NumberDictionaryEntries => number_dictionary_entries,
                        NumericValue::BoolDictionaryEntries => bool_dictionary_entries,
                        NumericValue::StringDictionaryEntries => string_dictionary_entries,
                        NumericValue::DictionaryFromNumberEntries => dictionary_from_number_entries,
                        NumericValue::DictionaryFromBoolEntries => dictionary_from_bool_entries,
                        NumericValue::DictionaryFromStringEntries => dictionary_from_string_entries,
                        _ => unreachable!(),
                    };
                    emit_unary_call(&mut code, function as *const () as u64, depth - 1);
                }
                NumericValue::NumberDictionarySet
                | NumericValue::StringDictionarySet
                | NumericValue::BoolDictionarySet => {
                    if depth < 3 {
                        return None;
                    }
                    let function = match value {
                        NumericValue::NumberDictionarySet => number_dictionary_set,
                        NumericValue::StringDictionarySet => string_dictionary_set,
                        NumericValue::BoolDictionarySet => bool_dictionary_set,
                        _ => unreachable!(),
                    };
                    emit_ternary_call(&mut code, function as *const () as u64, depth - 3);
                    depth -= 2;
                }
                NumericValue::EmptyDictionary => {
                    if depth == 8 {
                        return None;
                    }
                    emit_spill(&mut code, depth);
                    code.extend_from_slice(&[0x48, 0xb8]);
                    code.extend_from_slice(&(empty_dictionary as *const () as u64).to_le_bytes());
                    code.extend_from_slice(&[0xff, 0xd0]);
                    emit_move(&mut code, depth, 0);
                    emit_restore(&mut code, depth);
                    depth += 1;
                }
                NumericValue::DictionaryLength => {
                    if depth == 0 {
                        return None;
                    }
                    emit_unary_call(&mut code, dictionary_length as *const () as u64, depth - 1);
                }
                NumericValue::DictionaryKeyAt => {
                    if depth < 2 {
                        return None;
                    }
                    emit_binary_call(&mut code, dictionary_key_at as *const () as u64, depth - 2);
                    depth -= 1;
                }
                NumericValue::DictionaryAppend(kind) => {
                    if depth < 3 {
                        return None;
                    }
                    let function = match kind {
                        0 => number_dictionary_set,
                        1 => bool_dictionary_set,
                        2 => string_dictionary_set,
                        _ => return None,
                    };
                    let object = depth - 3;
                    emit_spill(&mut code, depth);
                    emit_move(&mut code, 0, object);
                    emit_move(&mut code, 1, object + 1);
                    emit_move(&mut code, 2, object + 2);
                    code.extend_from_slice(&[0x48, 0xb8]);
                    code.extend_from_slice(&(function as *const () as u64).to_le_bytes());
                    code.extend_from_slice(&[0xff, 0xd0]);
                    emit_restore(&mut code, depth);
                    depth -= 2;
                }
                NumericValue::DictionaryStaticAppend(kind, key) => {
                    if depth < 2 {
                        return None;
                    }
                    let function = match kind {
                        0 => number_dictionary_set,
                        1 => bool_dictionary_set,
                        2 => string_dictionary_set,
                        _ => return None,
                    };
                    let object = depth - 2;
                    emit_spill(&mut code, depth);
                    emit_move(&mut code, 0, object);
                    emit_move(&mut code, 2, object + 1);
                    code.extend_from_slice(&[0x48, 0xb8]);
                    code.extend_from_slice(&(*key as usize as u64).to_le_bytes());
                    code.extend_from_slice(&[0x66, 0x48, 0x0f, 0x6e, 0xc8]);
                    code.extend_from_slice(&[0x48, 0xb8]);
                    code.extend_from_slice(&(function as *const () as u64).to_le_bytes());
                    code.extend_from_slice(&[0xff, 0xd0]);
                    emit_restore(&mut code, depth);
                    depth -= 1;
                }
                NumericValue::NumberDictionaryPostSet => {
                    if depth < 4 {
                        return None;
                    }
                    let left = depth - 4;
                    emit_spill(&mut code, depth);
                    emit_move(&mut code, 0, left);
                    emit_move(&mut code, 1, left + 1);
                    emit_move(&mut code, 2, left + 3);
                    code.extend_from_slice(&[0x48, 0xb8]);
                    code.extend_from_slice(
                        &(number_dictionary_set as *const () as u64).to_le_bytes(),
                    );
                    code.extend_from_slice(&[0xff, 0xd0]);
                    emit_restore(&mut code, depth);
                    emit_move(&mut code, left, left + 2);
                    depth -= 3;
                }
                NumericValue::NumberArrayJoin
                | NumericValue::BoolArrayJoin
                | NumericValue::StringArrayJoin
                | NumericValue::DynamicArrayJoin => {
                    if depth < 2 {
                        return None;
                    }
                    let function = match value {
                        NumericValue::NumberArrayJoin => number_array_join,
                        NumericValue::BoolArrayJoin => bool_array_join,
                        NumericValue::StringArrayJoin => string_array_join,
                        NumericValue::DynamicArrayJoin => dynamic_array_join,
                        _ => unreachable!(),
                    };
                    emit_binary_call(&mut code, function as *const () as u64, depth - 2);
                    depth -= 1;
                }
                NumericValue::NumberArrayIncludes
                | NumericValue::BoolArrayIncludes
                | NumericValue::StringArrayIncludes
                | NumericValue::DynamicArrayIncludes
                | NumericValue::NumberArrayIndexOf
                | NumericValue::BoolArrayIndexOf
                | NumericValue::StringArrayIndexOf
                | NumericValue::DynamicArrayIndexOf
                | NumericValue::NumberArrayLastIndexOf
                | NumericValue::BoolArrayLastIndexOf
                | NumericValue::StringArrayLastIndexOf
                | NumericValue::DynamicArrayLastIndexOf => {
                    if depth < 3 {
                        return None;
                    }
                    let function = match value {
                        NumericValue::NumberArrayIncludes => number_array_includes,
                        NumericValue::BoolArrayIncludes => bool_array_includes,
                        NumericValue::StringArrayIncludes => string_array_includes,
                        NumericValue::DynamicArrayIncludes => dynamic_array_includes,
                        NumericValue::NumberArrayIndexOf => number_array_index_of,
                        NumericValue::BoolArrayIndexOf => bool_array_index_of,
                        NumericValue::StringArrayIndexOf => string_array_index_of,
                        NumericValue::DynamicArrayIndexOf => dynamic_array_index_of,
                        NumericValue::NumberArrayLastIndexOf => number_array_last_index_of,
                        NumericValue::BoolArrayLastIndexOf => bool_array_last_index_of,
                        NumericValue::StringArrayLastIndexOf => string_array_last_index_of,
                        NumericValue::DynamicArrayLastIndexOf => dynamic_array_last_index_of,
                        _ => unreachable!(),
                    };
                    emit_ternary_call(&mut code, function as *const () as u64, depth - 3);
                    depth -= 2;
                }
                NumericValue::StringTruthy => {
                    if depth == 0 {
                        return None;
                    }
                    emit_unary_call(&mut code, string_truthy as *const () as u64, depth - 1);
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
                NumericValue::ShortCircuit(and) => {
                    if depth < 2 {
                        return None;
                    }
                    let condition = depth - 1;
                    code.extend_from_slice(&[
                        0x66,
                        0x0f,
                        0x2e,
                        0xc0 | (condition << 3) | condition,
                    ]);
                    let parity = emit_near_jump(&mut code, 0x8a);
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
                    ]);
                    let truth = emit_near_jump(&mut code, if *and { 0x84 } else { 0x85 });
                    let mut exits = vec![truth];
                    if *and {
                        exits.push(parity);
                    } else {
                        patch_near_jump(&mut code, parity)?;
                    }
                    depth -= 2;
                    branches.push((depth, exits, false, Some(1)));
                }
                NumericValue::ConditionalStart => {
                    if depth == 0 {
                        return None;
                    }
                    let condition = depth - 1;
                    code.extend_from_slice(&[
                        0x66,
                        0x0f,
                        0x2e,
                        0xc0 | (condition << 3) | condition,
                    ]);
                    let parity = emit_near_jump(&mut code, 0x8a);
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
                    ]);
                    let zero = emit_near_jump(&mut code, 0x84);
                    depth -= 1;
                    branches.push((depth, vec![parity, zero], true, None));
                }
                NumericValue::PresentConditionalStart => {
                    if depth == 0 || depth == 8 {
                        return None;
                    }
                    emit_spill(&mut code, depth);
                    code.extend_from_slice(&[0x48, 0xb8]);
                    code.extend_from_slice(&(take_present as *const () as u64).to_le_bytes());
                    code.extend_from_slice(&[0xff, 0xd0]);
                    emit_move(&mut code, depth, 0);
                    emit_restore(&mut code, depth);
                    code.extend_from_slice(&[0x66, 0x0f, 0x2e, 0xc0 | (depth << 3) | depth]);
                    let parity = emit_near_jump(&mut code, 0x8a);
                    code.extend_from_slice(&[
                        0x66,
                        0x48,
                        0x0f,
                        0x7e,
                        0xc0 | (depth << 3),
                        0x48,
                        0xd1,
                        0xe0,
                        0x48,
                        0x85,
                        0xc0,
                    ]);
                    let absent = emit_near_jump(&mut code, 0x84);
                    branches.push((depth - 1, vec![parity, absent], true, None));
                }
                NumericValue::ConditionalAlternate => {
                    let (base_depth, exits, awaits_alternate, result_depth) =
                        branches.last_mut()?;
                    if !*awaits_alternate || depth <= *base_depth {
                        return None;
                    }
                    *result_depth = Some(depth - *base_depth);
                    let end = emit_unconditional_jump(&mut code);
                    for exit in exits.drain(..) {
                        patch_near_jump(&mut code, exit)?;
                    }
                    exits.push(end);
                    *awaits_alternate = false;
                    depth = *base_depth;
                }
                NumericValue::ShortCircuitEnd => {
                    let (base_depth, exits, awaits_alternate, result_depth) = branches.pop()?;
                    if awaits_alternate || depth != base_depth + result_depth? {
                        return None;
                    }
                    for exit in exits {
                        patch_near_jump(&mut code, exit)?;
                    }
                }
                NumericValue::AsBoolean => {
                    if depth == 0 {
                        return None;
                    }
                }
                NumericValue::BooleanNot => {
                    if depth == 0 {
                        return None;
                    }
                    emit_unary_call(&mut code, boolean_not as *const () as u64, depth - 1);
                }
                NumericValue::StrictMismatch(result) => {
                    if depth < 2 {
                        return None;
                    }
                    let destination = depth - 2;
                    code.extend_from_slice(&[0x48, 0xb8]);
                    code.extend_from_slice(&f64::from(u8::from(*result)).to_bits().to_le_bytes());
                    code.extend_from_slice(&[0x66, 0x48, 0x0f, 0x6e, 0xc0 | (destination << 3)]);
                    depth -= 1;
                }
                NumericValue::Recur(arity) => {
                    if depth < *arity {
                        return None;
                    }
                    let base = depth - arity;
                    emit_recursive_call(&mut code, base, *arity)?;
                    depth = base + 1;
                }
            }
        }
        (depth == 1
            && branches.is_empty()
            && loops.is_empty()
            && guards.is_empty()
            && switches.is_empty()
            && results.is_empty())
        .then(|| {
            code.push(0xc3);
            code
        })
    }
}
