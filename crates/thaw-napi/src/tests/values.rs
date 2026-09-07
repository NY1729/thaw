#[test]
fn latin1_and_utf16_strings_follow_napi_buffer_contracts() {
    unsafe {
        let mut env = Env::new();
        let env_ptr: NapiEnv = &mut env;

        let latin1 = [0xe9_u8, 0];
        let mut latin1_value = ptr::null_mut();
        assert_eq!(
            napi_create_string_latin1(
                env_ptr,
                latin1.as_ptr().cast(),
                NAPI_AUTO_LENGTH,
                &mut latin1_value,
            ),
            NAPI_OK
        );
        let mut utf8 = [0_i8; 3];
        let mut written = 0;
        assert_eq!(
            napi_get_value_string_utf8(
                env_ptr,
                latin1_value,
                utf8.as_mut_ptr(),
                utf8.len(),
                &mut written,
            ),
            NAPI_OK
        );
        assert_eq!(written, 2);
        assert_eq!(utf8.map(|byte| byte as u8), [0xc3, 0xa9, 0]);

        let utf16 = [0x41_u16, 0xd83d, 0xde03, 0];
        let mut utf16_value = ptr::null_mut();
        assert_eq!(
            napi_create_string_utf16(env_ptr, utf16.as_ptr(), NAPI_AUTO_LENGTH, &mut utf16_value,),
            NAPI_OK
        );
        assert_eq!(
            napi_get_value_string_utf16(env_ptr, utf16_value, ptr::null_mut(), 0, &mut written,),
            NAPI_OK
        );
        assert_eq!(written, 3);
        let mut utf16_copy = [0_u16; 4];
        assert_eq!(
            napi_get_value_string_utf16(
                env_ptr,
                utf16_value,
                utf16_copy.as_mut_ptr(),
                utf16_copy.len(),
                &mut written,
            ),
            NAPI_OK
        );
        assert_eq!(written, 3);
        assert_eq!(utf16_copy, utf16);

        let mut truncated = [0_i8; 5];
        assert_eq!(
            napi_get_value_string_utf8(
                env_ptr,
                utf16_value,
                truncated.as_mut_ptr(),
                truncated.len(),
                &mut written,
            ),
            NAPI_OK
        );
        assert_eq!(written, 1);
        assert_eq!(truncated[0], b'A' as i8);
        assert_eq!(truncated[1], 0);
    }
}

#[test]
fn optimized_property_keys_share_identity_across_encodings() {
    unsafe {
        let mut env = Env::new();
        let env_ptr: NapiEnv = &mut env;
        let latin1 = [0xe9_u8, 0];
        let utf8 = [0xc3_u8, 0xa9, 0];
        let utf16 = [0x00e9_u16, 0];
        let mut latin1_key = ptr::null_mut();
        let mut utf8_key = ptr::null_mut();
        let mut utf16_key = ptr::null_mut();
        assert_eq!(
            node_api_create_property_key_latin1(
                env_ptr,
                latin1.as_ptr().cast(),
                NAPI_AUTO_LENGTH,
                &mut latin1_key,
            ),
            NAPI_OK
        );
        assert_eq!(
            node_api_create_property_key_utf8(env_ptr, utf8.as_ptr().cast(), 2, &mut utf8_key,),
            NAPI_OK
        );
        assert_eq!(
            node_api_create_property_key_utf16(
                env_ptr,
                utf16.as_ptr(),
                NAPI_AUTO_LENGTH,
                &mut utf16_key,
            ),
            NAPI_OK
        );
        assert_eq!(latin1_key, utf8_key);
        assert_eq!(utf8_key, utf16_key);
        let mut object = ptr::null_mut();
        assert_eq!(napi_create_object(env_ptr, &mut object), NAPI_OK);
        let value = env.alloc(Value::Number(42.0));
        assert_eq!(
            napi_set_property(env_ptr, object, latin1_key, value),
            NAPI_OK
        );
        let mut actual = ptr::null_mut();
        assert_eq!(
            napi_get_property(env_ptr, object, utf16_key, &mut actual),
            NAPI_OK
        );
        assert_eq!(actual, value);
    }
}

#[test]
fn external_strings_report_copying_and_finalize_immediately() {
    unsafe {
        EXTERNAL_STRING_FINALIZED.store(0, Ordering::Release);
        let mut env = Env::new();
        let env_ptr: NapiEnv = &mut env;
        let mut latin1 = [0xe9_u8, 0];
        let mut utf16 = [0x03bb_u16, 0];
        let mut value = ptr::null_mut();
        let mut copied = false;
        let latin1_hint = 1_usize;
        let utf16_hint = 2_usize;
        assert_eq!(
            node_api_create_external_string_latin1(
                env_ptr,
                latin1.as_mut_ptr().cast(),
                NAPI_AUTO_LENGTH,
                Some(external_string_finalize),
                (&latin1_hint as *const usize).cast_mut().cast(),
                &mut value,
                &mut copied,
            ),
            NAPI_OK
        );
        assert!(copied);
        assert!(matches!(value_ref(value), Ok(Value::String(text)) if text == "é"));
        copied = false;
        assert_eq!(
            node_api_create_external_string_utf16(
                env_ptr,
                utf16.as_mut_ptr(),
                NAPI_AUTO_LENGTH,
                Some(external_string_finalize),
                (&utf16_hint as *const usize).cast_mut().cast(),
                &mut value,
                &mut copied,
            ),
            NAPI_OK
        );
        assert!(copied);
        assert!(matches!(value_ref(value), Ok(Value::String(text)) if text == "λ"));
        assert_eq!(EXTERNAL_STRING_FINALIZED.load(Ordering::Acquire), 3);
        drop(env);
        assert_eq!(
            EXTERNAL_STRING_FINALIZED.load(Ordering::Acquire),
            3,
            "copied external strings must not finalize twice at Env teardown"
        );
    }
}

#[test]
fn integer_creation_roundtrips_through_napi_number_accessors() {
    unsafe {
        let mut env = Env::new();
        let env_ptr: NapiEnv = &mut env;

        let mut value = ptr::null_mut();
        assert_eq!(
            napi_create_int32(env_ptr, -2_000_000_000, &mut value),
            NAPI_OK
        );
        let mut signed32 = 0;
        assert_eq!(napi_get_value_int32(env_ptr, value, &mut signed32), NAPI_OK);
        assert_eq!(signed32, -2_000_000_000);

        assert_eq!(
            napi_create_uint32(env_ptr, 4_000_000_000, &mut value),
            NAPI_OK
        );
        let mut unsigned32 = 0;
        assert_eq!(
            napi_get_value_uint32(env_ptr, value, &mut unsigned32),
            NAPI_OK
        );
        assert_eq!(unsigned32, 4_000_000_000);
        let negative = env.alloc(Value::Number(-1.0));
        assert_eq!(
            napi_get_value_uint32(env_ptr, negative, &mut unsigned32),
            NAPI_OK
        );
        assert_eq!(unsigned32, u32::MAX);

        assert_eq!(
            napi_create_int64(env_ptr, 9_007_199_254_740_991, &mut value),
            NAPI_OK
        );
        let mut signed64 = 0;
        assert_eq!(napi_get_value_int64(env_ptr, value, &mut signed64), NAPI_OK);
        assert_eq!(signed64, 9_007_199_254_740_991);
    }
}

#[test]
fn last_error_info_tracks_value_extraction_failures_per_environment() {
    unsafe {
        let mut env = Env::new();
        let mut other_env = Env::new();
        let env_ptr: NapiEnv = &mut env;
        let string = env.alloc(Value::String("not a number".into()));
        let number = env.alloc(Value::Number(7.0));
        let mut output = 0.0;
        assert_eq!(
            napi_get_value_double(env_ptr, string, &mut output),
            NAPI_NUMBER_EXPECTED
        );
        let mut info = ptr::null();
        assert_eq!(napi_get_last_error_info(env_ptr, &mut info), NAPI_OK);
        assert_eq!((*info).error_code, NAPI_NUMBER_EXPECTED);
        assert_eq!(
            CStr::from_ptr((*info).error_message).to_bytes(),
            b"Number expected"
        );
        assert_eq!((*info).engine_error_code, 0);
        assert!((*info).engine_reserved.is_null());

        assert_eq!(napi_get_value_double(env_ptr, number, &mut output), NAPI_OK);
        assert_eq!(napi_get_last_error_info(env_ptr, &mut info), NAPI_OK);
        assert_eq!(
            (*info).error_code,
            NAPI_NUMBER_EXPECTED,
            "a successful call must not clear the last error"
        );

        assert_eq!(
            napi_get_value_double(env_ptr, number, ptr::null_mut()),
            NAPI_INVALID_ARG
        );
        assert_eq!(napi_get_last_error_info(env_ptr, &mut info), NAPI_OK);
        assert_eq!((*info).error_code, NAPI_INVALID_ARG);
        assert_eq!(
            CStr::from_ptr((*info).error_message).to_bytes(),
            b"Invalid argument"
        );

        let mut other_info = ptr::null();
        assert_eq!(
            napi_get_last_error_info(&mut other_env, &mut other_info),
            NAPI_OK
        );
        assert_eq!((*other_info).error_code, NAPI_OK);
        assert!((*other_info).error_message.is_null());

        let foreign = other_env.alloc(Value::Object(HashMap::new()));
        let mut value_type = 0;
        assert_eq!(
            napi_typeof(env_ptr, foreign, &mut value_type),
            NAPI_INVALID_ARG
        );
        assert_eq!(napi_get_last_error_info(env_ptr, &mut info), NAPI_OK);
        assert_eq!((*info).error_code, NAPI_INVALID_ARG);

        env.last_error_info.error_code = NAPI_OK;
        let values_before = env.values.len();
        assert_eq!(
            napi_create_object(env_ptr, ptr::null_mut()),
            NAPI_INVALID_ARG
        );
        assert_eq!(napi_get_last_error_info(env_ptr, &mut info), NAPI_OK);
        assert_eq!((*info).error_code, NAPI_INVALID_ARG);
        assert_eq!(env.values.len(), values_before);

        env.last_error_info.error_code = NAPI_OK;
        assert_eq!(
            napi_strict_equals(env_ptr, number, number, ptr::null_mut()),
            NAPI_INVALID_ARG
        );
        assert_eq!(napi_get_last_error_info(env_ptr, &mut info), NAPI_OK);
        assert_eq!((*info).error_code, NAPI_INVALID_ARG);

        env.last_error_info.error_code = NAPI_OK;
        assert_eq!(
            node_api_get_module_file_name(env_ptr, ptr::null_mut()),
            NAPI_INVALID_ARG
        );
        assert_eq!(napi_get_last_error_info(env_ptr, &mut info), NAPI_OK);
        assert_eq!((*info).error_code, NAPI_INVALID_ARG);
    }
}

#[test]
fn property_operations_record_type_and_state_errors() {
    unsafe {
        let mut env = Env::new();
        let env_ptr: NapiEnv = &mut env;
        let object = env.alloc(Value::Object(HashMap::new()));
        let key = env.alloc(Value::String("new".into()));
        let value = env.alloc(Value::Number(1.0));
        let mut info = ptr::null();

        assert_eq!(
            napi_define_properties(env_ptr, object, 1, ptr::null()),
            NAPI_INVALID_ARG
        );
        assert_eq!(napi_get_last_error_info(env_ptr, &mut info), NAPI_OK);
        assert_eq!((*info).error_code, NAPI_INVALID_ARG);

        assert_eq!(napi_object_seal(env_ptr, object), NAPI_OK);
        assert_eq!(
            napi_set_property(env_ptr, object, key, value),
            NAPI_GENERIC_FAILURE
        );
        assert_eq!(napi_get_last_error_info(env_ptr, &mut info), NAPI_OK);
        assert_eq!((*info).error_code, NAPI_GENERIC_FAILURE);

        let mut present = false;
        assert_eq!(
            napi_has_property(env_ptr, value, key, &mut present),
            NAPI_OBJECT_EXPECTED
        );
        assert_eq!(napi_get_last_error_info(env_ptr, &mut info), NAPI_OK);
        assert_eq!((*info).error_code, NAPI_OBJECT_EXPECTED);

        let mut actual = ptr::null_mut();
        assert_eq!(
            napi_get_property(env_ptr, object, value, &mut actual),
            NAPI_OK
        );
        assert!(matches!(value_ref(actual), Ok(Value::Undefined)));
        assert_eq!(napi_get_last_error_info(env_ptr, &mut info), NAPI_OK);
        assert_eq!((*info).error_code, NAPI_OBJECT_EXPECTED);
    }
}

#[test]
fn buffer_and_view_operations_record_type_and_bounds_errors() {
    unsafe {
        let mut env = Env::new();
        let env_ptr: NapiEnv = &mut env;
        let number = env.alloc(Value::Number(1.0));
        let mut info = ptr::null();

        let mut buffer = ptr::null_mut();
        assert_eq!(
            napi_create_external_buffer(
                env_ptr,
                1,
                ptr::null_mut(),
                None,
                ptr::null_mut(),
                &mut buffer,
            ),
            NAPI_INVALID_ARG
        );
        assert_eq!(napi_get_last_error_info(env_ptr, &mut info), NAPI_OK);
        assert_eq!((*info).error_code, NAPI_INVALID_ARG);
        assert_eq!(
            napi_is_buffer(env_ptr, number, ptr::null_mut()),
            NAPI_INVALID_ARG
        );
        assert_eq!(napi_get_last_error_info(env_ptr, &mut info), NAPI_OK);
        assert_eq!((*info).error_code, NAPI_INVALID_ARG);

        assert_eq!(
            napi_get_arraybuffer_info(env_ptr, number, ptr::null_mut(), ptr::null_mut()),
            NAPI_ARRAYBUFFER_EXPECTED
        );
        assert_eq!(napi_get_last_error_info(env_ptr, &mut info), NAPI_OK);
        assert_eq!((*info).error_code, NAPI_ARRAYBUFFER_EXPECTED);

        assert_eq!(
            napi_get_dataview_info(
                env_ptr,
                number,
                ptr::null_mut(),
                ptr::null_mut(),
                ptr::null_mut(),
                ptr::null_mut(),
            ),
            NAPI_INVALID_ARG
        );
        assert_eq!(napi_get_last_error_info(env_ptr, &mut info), NAPI_OK);
        assert_eq!((*info).error_code, NAPI_INVALID_ARG);

        let mut array_buffer = ptr::null_mut();
        assert_eq!(
            napi_create_arraybuffer(env_ptr, 8, ptr::null_mut(), &mut array_buffer),
            NAPI_OK
        );
        let mut view = ptr::null_mut();
        assert_eq!(
            napi_create_typedarray(env_ptr, 4, 1, array_buffer, 1, &mut view),
            NAPI_INVALID_ARG
        );
        assert_eq!(napi_get_last_error_info(env_ptr, &mut info), NAPI_OK);
        assert_eq!((*info).error_code, NAPI_INVALID_ARG);

        assert_eq!(
            node_api_create_buffer_from_arraybuffer(env_ptr, array_buffer, 7, 2, &mut view,),
            NAPI_PENDING_EXCEPTION
        );
        assert_eq!(napi_get_last_error_info(env_ptr, &mut info), NAPI_OK);
        assert_eq!((*info).error_code, NAPI_PENDING_EXCEPTION);
    }
}

#[test]
fn deferred_reference_and_async_handle_errors_update_last_error() {
    unsafe {
        let mut env = Env::new();
        let env_ptr: NapiEnv = &mut env;
        let value = env.alloc(Value::Number(1.0));
        let mut info = ptr::null();

        let mut reference = ptr::null_mut();
        assert_eq!(
            napi_create_reference(env_ptr, value, 0, &mut reference),
            NAPI_OK
        );
        assert_eq!(
            napi_reference_unref(env_ptr, reference, ptr::null_mut()),
            NAPI_GENERIC_FAILURE
        );
        assert_eq!(napi_get_last_error_info(env_ptr, &mut info), NAPI_OK);
        assert_eq!((*info).error_code, NAPI_GENERIC_FAILURE);
        assert_eq!(napi_delete_reference(env_ptr, reference), NAPI_OK);
        assert_eq!(
            napi_get_reference_value(env_ptr, reference, ptr::null_mut()),
            NAPI_INVALID_ARG
        );
        assert_eq!(napi_get_last_error_info(env_ptr, &mut info), NAPI_OK);
        assert_eq!((*info).error_code, NAPI_INVALID_ARG);

        let mut deferred = ptr::null_mut();
        let mut promise = ptr::null_mut();
        assert_eq!(
            napi_create_promise(env_ptr, &mut deferred, &mut promise),
            NAPI_OK
        );
        assert_eq!(napi_resolve_deferred(env_ptr, deferred, value), NAPI_OK);
        assert_eq!(
            napi_resolve_deferred(env_ptr, deferred, value),
            NAPI_GENERIC_FAILURE
        );
        assert_eq!(napi_get_last_error_info(env_ptr, &mut info), NAPI_OK);
        assert_eq!((*info).error_code, NAPI_GENERIC_FAILURE);

        let name = env.alloc(Value::Number(2.0));
        let mut work = ptr::null_mut();
        assert_eq!(
            napi_create_async_work(
                env_ptr,
                ptr::null_mut(),
                name,
                Some(probe_execute),
                None,
                ptr::null_mut(),
                &mut work,
            ),
            NAPI_STRING_EXPECTED
        );
        assert_eq!(napi_get_last_error_info(env_ptr, &mut info), NAPI_OK);
        assert_eq!((*info).error_code, NAPI_STRING_EXPECTED);
    }
}

#[test]
fn value_creation_errors_update_last_error_without_allocating() {
    unsafe {
        let mut env = Env::new();
        let env_ptr: NapiEnv = &mut env;
        let mut value = ptr::null_mut();
        let mut info = ptr::null();
        let values_before = env.values.len();

        assert_eq!(
            napi_create_string_utf8(env_ptr, ptr::null(), 0, &mut value),
            NAPI_INVALID_ARG
        );
        assert_eq!(napi_get_last_error_info(env_ptr, &mut info), NAPI_OK);
        assert_eq!((*info).error_code, NAPI_INVALID_ARG);
        assert_eq!(env.values.len(), values_before);

        let number = env.alloc(Value::Number(1.0));
        assert_eq!(
            napi_create_symbol(env_ptr, number, &mut value),
            NAPI_STRING_EXPECTED
        );
        assert_eq!(napi_get_last_error_info(env_ptr, &mut info), NAPI_OK);
        assert_eq!((*info).error_code, NAPI_STRING_EXPECTED);

        assert_eq!(
            napi_create_function(env_ptr, ptr::null(), 0, None, ptr::null_mut(), &mut value,),
            NAPI_INVALID_ARG
        );
        assert_eq!(napi_get_last_error_info(env_ptr, &mut info), NAPI_OK);
        assert_eq!((*info).error_code, NAPI_INVALID_ARG);
    }
}

#[test]
fn date_values_preserve_milliseconds_and_object_type() {
    unsafe {
        let mut env = Env::new();
        let env_ptr: NapiEnv = &mut env;
        let mut date = ptr::null_mut();
        assert_eq!(
            napi_create_date(env_ptr, 1_725_000_000_123.5, &mut date),
            NAPI_OK
        );
        let mut is_date = false;
        assert_eq!(napi_is_date(env_ptr, date, &mut is_date), NAPI_OK);
        assert!(is_date);
        let mut milliseconds = 0.0;
        assert_eq!(
            napi_get_date_value(env_ptr, date, &mut milliseconds),
            NAPI_OK
        );
        assert_eq!(milliseconds, 1_725_000_000_123.5);
        let mut value_type = -1;
        assert_eq!(napi_typeof(env_ptr, date, &mut value_type), NAPI_OK);
        assert_eq!(value_type, 6);

        let mut number = ptr::null_mut();
        assert_eq!(napi_create_double(env_ptr, 1.0, &mut number), NAPI_OK);
        assert_eq!(napi_is_date(env_ptr, number, &mut is_date), NAPI_OK);
        assert!(!is_date);
        assert_eq!(
            napi_get_date_value(env_ptr, number, &mut milliseconds),
            NAPI_DATE_EXPECTED
        );
    }
}

#[test]
fn bigint_64_bit_conversions_report_losslessness() {
    unsafe {
        let mut env = Env::new();
        let env_ptr: NapiEnv = &mut env;
        let mut value = ptr::null_mut();
        assert_eq!(
            napi_create_bigint_int64(env_ptr, i64::MIN, &mut value),
            NAPI_OK
        );
        let mut signed = 0;
        let mut lossless = false;
        assert_eq!(
            napi_get_value_bigint_int64(env_ptr, value, &mut signed, &mut lossless),
            NAPI_OK
        );
        assert_eq!(signed, i64::MIN);
        assert!(lossless);
        let mut unsigned = 0;
        assert_eq!(
            napi_get_value_bigint_uint64(env_ptr, value, &mut unsigned, &mut lossless),
            NAPI_OK
        );
        assert_eq!(unsigned, 1_u64 << 63);
        assert!(!lossless);

        assert_eq!(
            napi_create_bigint_uint64(env_ptr, u64::MAX, &mut value),
            NAPI_OK
        );
        assert_eq!(
            napi_get_value_bigint_uint64(env_ptr, value, &mut unsigned, &mut lossless),
            NAPI_OK
        );
        assert_eq!(unsigned, u64::MAX);
        assert!(lossless);
        assert_eq!(
            napi_get_value_bigint_int64(env_ptr, value, &mut signed, &mut lossless),
            NAPI_OK
        );
        assert_eq!(signed, -1);
        assert!(!lossless);
        let mut value_type = -1;
        assert_eq!(napi_typeof(env_ptr, value, &mut value_type), NAPI_OK);
        assert_eq!(value_type, 9);
    }
}

#[test]
fn coerce_to_object_preserves_existing_object_identity() {
    unsafe {
        let mut env = Env::new();
        let env_ptr: NapiEnv = &mut env;
        let object_values = [
            env.alloc(Value::Array(Vec::new())),
            env.alloc(Value::Buffer(vec![1, 2, 3])),
            env.alloc(Value::Date(123.0)),
            env.alloc(Value::Error("failure".into())),
            env.alloc(Value::Promise(Rc::new(RefCell::new(PromiseState::Pending)))),
        ];
        for value in object_values {
            let mut result = ptr::null_mut();
            assert_eq!(napi_coerce_to_object(env_ptr, value, &mut result), NAPI_OK);
            assert_eq!(result, value);
        }

        let primitive = env.alloc(Value::Number(42.0));
        let mut boxed = ptr::null_mut();
        assert_eq!(
            napi_coerce_to_object(env_ptr, primitive, &mut boxed),
            NAPI_OK
        );
        assert_ne!(boxed, primitive);
        assert!(matches!(value_ref(boxed), Ok(Value::Object(_))));
    }
}

#[test]
fn coercions_follow_ecmascript_primitive_rules() {
    unsafe {
        let mut env = Env::new();
        let env_ptr: NapiEnv = &mut env;
        let mut result = ptr::null_mut();

        for (input, expected) in [("", 0.0), ("  0x2a  ", 42.0), ("0b101", 5.0)] {
            let value = env.alloc(Value::String(input.into()));
            assert_eq!(napi_coerce_to_number(env_ptr, value, &mut result), NAPI_OK);
            assert!(matches!(value_ref(result), Ok(Value::Number(number)) if *number == expected));
        }

        let one = env.alloc(Value::Number(1.0));
        let text = env.alloc(Value::String("x".into()));
        let nested = env.alloc(Value::Array(vec![Some(text), None]));
        let array = env.alloc(Value::Array(vec![Some(one), None, Some(nested)]));
        assert_eq!(napi_coerce_to_string(env_ptr, array, &mut result), NAPI_OK);
        assert!(matches!(value_ref(result), Ok(Value::String(value)) if value == "1,,x,"));

        let bigint = env.alloc(Value::BigInt {
            negative: false,
            words: vec![0, 1],
        });
        assert_eq!(napi_coerce_to_string(env_ptr, bigint, &mut result), NAPI_OK);
        assert!(
            matches!(value_ref(result), Ok(Value::String(value)) if value == "18446744073709551616")
        );
        assert_eq!(
            napi_coerce_to_number(env_ptr, bigint, &mut result),
            NAPI_PENDING_EXCEPTION
        );
        assert!(env.exception.take().is_some());

        let symbol = env.alloc(Value::Symbol {
            id: 999,
            description: "token".into(),
        });
        assert_eq!(
            napi_coerce_to_string(env_ptr, symbol, &mut result),
            NAPI_PENDING_EXCEPTION
        );
        let mut info = ptr::null();
        assert_eq!(napi_get_last_error_info(env_ptr, &mut info), NAPI_OK);
        assert_eq!((*info).error_code, NAPI_PENDING_EXCEPTION);
        assert!(env.exception.take().is_some());

        let mut length = 0;
        assert_eq!(
            napi_get_array_length(env_ptr, one, &mut length),
            NAPI_ARRAY_EXPECTED
        );
        assert_eq!(napi_get_last_error_info(env_ptr, &mut info), NAPI_OK);
        assert_eq!((*info).error_code, NAPI_ARRAY_EXPECTED);
        let mut present = false;
        assert_eq!(
            napi_has_element(env_ptr, one, 0, &mut present),
            NAPI_OBJECT_EXPECTED
        );
        assert_eq!(napi_get_last_error_info(env_ptr, &mut info), NAPI_OK);
        assert_eq!((*info).error_code, NAPI_OBJECT_EXPECTED);
    }
}

#[test]
fn native_wraps_accept_all_javascript_object_kinds() {
    unsafe extern "C" fn noop_finalize(_env: NapiEnv, _data: *mut c_void, _hint: *mut c_void) {}
    unsafe {
        let mut env = Env::new();
        let env_ptr: NapiEnv = &mut env;
        let array = env.alloc(Value::Array(Vec::new()));
        let buffer = env.alloc(Value::Buffer(vec![1]));
        let promise = env.alloc(Value::Promise(Rc::new(RefCell::new(PromiseState::Pending))));
        let mut payloads = [11_u32, 22, 33];
        for (object, payload) in [array, buffer, promise]
            .into_iter()
            .zip(payloads.iter_mut())
        {
            let data = (payload as *mut u32).cast();
            assert_eq!(
                napi_wrap(
                    env_ptr,
                    object,
                    data,
                    None,
                    ptr::null_mut(),
                    ptr::null_mut(),
                ),
                NAPI_OK
            );
            let mut actual = ptr::null_mut();
            assert_eq!(napi_unwrap(env_ptr, object, &mut actual), NAPI_OK);
            assert_eq!(actual, data);
            assert_eq!(napi_remove_wrap(env_ptr, object, &mut actual), NAPI_OK);
            assert_eq!(actual, data);
            assert_eq!(
                napi_add_finalizer(
                    env_ptr,
                    object,
                    ptr::null_mut(),
                    Some(noop_finalize),
                    ptr::null_mut(),
                    ptr::null_mut(),
                ),
                NAPI_OK
            );
        }

        let primitive = env.alloc(Value::Number(1.0));
        assert_eq!(
            napi_wrap(
                env_ptr,
                primitive,
                ptr::null_mut(),
                None,
                ptr::null_mut(),
                ptr::null_mut(),
            ),
            NAPI_OBJECT_EXPECTED
        );
        let mut info = ptr::null();
        assert_eq!(napi_get_last_error_info(env_ptr, &mut info), NAPI_OK);
        assert_eq!((*info).error_code, NAPI_OBJECT_EXPECTED);
        let mut foreign_env = Env::new();
        let foreign = foreign_env.alloc(Value::Object(HashMap::new()));
        let mut actual = ptr::null_mut();
        assert_eq!(
            napi_wrap(
                env_ptr,
                foreign,
                ptr::null_mut(),
                None,
                ptr::null_mut(),
                ptr::null_mut(),
            ),
            NAPI_INVALID_ARG
        );
        assert_eq!(napi_unwrap(env_ptr, foreign, &mut actual), NAPI_INVALID_ARG);
        assert_eq!(
            napi_remove_wrap(env_ptr, foreign, &mut actual),
            NAPI_INVALID_ARG
        );
        assert_eq!(
            napi_add_finalizer(
                env_ptr,
                foreign,
                ptr::null_mut(),
                Some(noop_finalize),
                ptr::null_mut(),
                ptr::null_mut(),
            ),
            NAPI_INVALID_ARG
        );
    }
}

#[test]
fn typeof_distinguishes_externals_and_predicates_validate_handles() {
    unsafe {
        let mut env = Env::new();
        let env_ptr: NapiEnv = &mut env;
        let external = env.alloc(Value::External(ptr::null_mut()));
        let mut value_type = -1;
        assert_eq!(napi_typeof(env_ptr, external, &mut value_type), NAPI_OK);
        assert_eq!(value_type, 8);
        assert_eq!(
            napi_typeof(ptr::null_mut(), external, &mut value_type),
            NAPI_INVALID_ARG
        );
        assert_eq!(
            napi_typeof(env_ptr, ptr::null_mut(), &mut value_type),
            NAPI_INVALID_ARG
        );

        let mut result = false;
        assert_eq!(
            napi_is_array(env_ptr, ptr::null_mut(), &mut result),
            NAPI_INVALID_ARG
        );
        let array = env.alloc(Value::Array(Vec::new()));
        assert_eq!(
            napi_is_array(ptr::null_mut(), array, &mut result),
            NAPI_INVALID_ARG
        );
        assert_eq!(napi_is_array(env_ptr, array, &mut result), NAPI_OK);
        assert!(result);
    }
}

#[test]
fn plain_externals_preserve_data_and_finalize_once() {
    PLAIN_EXTERNAL_FINALIZED.store(0, Ordering::Release);
    let mut data = 17_usize;
    let mut hint = 25_usize;
    let external;
    unsafe {
        let mut env = Env::new();
        let env_ptr: NapiEnv = &mut env;
        external = {
            let mut value = ptr::null_mut();
            assert_eq!(
                napi_create_external(
                    env_ptr,
                    (&mut data as *mut usize).cast(),
                    Some(plain_external_finalize),
                    (&mut hint as *mut usize).cast(),
                    &mut value,
                ),
                NAPI_OK
            );
            value
        };
        let mut actual = ptr::null_mut();
        assert_eq!(
            napi_get_value_external(env_ptr, external, &mut actual),
            NAPI_OK
        );
        assert_eq!(actual, (&mut data as *mut usize).cast());
        let mut other_env = Env::new();
        assert_eq!(
            napi_get_value_external(&mut other_env, external, &mut actual),
            NAPI_INVALID_ARG
        );
        assert_eq!(
            napi_create_external(
                env_ptr,
                ptr::null_mut(),
                Some(plain_external_finalize),
                ptr::null_mut(),
                ptr::null_mut(),
            ),
            NAPI_INVALID_ARG
        );
        assert_eq!(PLAIN_EXTERNAL_FINALIZED.load(Ordering::Acquire), 0);
    }
    assert!(!external.is_null());
    assert_eq!(PLAIN_EXTERNAL_FINALIZED.load(Ordering::Acquire), 42);
}

#[test]
fn version_queries_match_the_exported_node_api_surface() {
    unsafe {
        let mut env = Env::new();
        let env_ptr: NapiEnv = &mut env;
        let mut version = 0;
        assert_eq!(napi_get_version(env_ptr, &mut version), NAPI_OK);
        assert_eq!(version, 10);
        assert_eq!(
            napi_get_version(ptr::null_mut(), &mut version),
            NAPI_INVALID_ARG
        );
        let mut first = ptr::null();
        let mut second = ptr::null();
        assert_eq!(napi_get_node_version(env_ptr, &mut first), NAPI_OK);
        assert_eq!(napi_get_node_version(env_ptr, &mut second), NAPI_OK);
        assert_eq!(first, second);
        assert!(!first.is_null());
        assert_eq!(CStr::from_ptr((*first).release).to_bytes(), b"thaw");
        assert_eq!(
            napi_get_node_version(ptr::null_mut(), &mut first),
            NAPI_INVALID_ARG
        );
    }
}

#[test]
fn bigint_words_preserve_little_endian_magnitude_and_sign() {
    unsafe {
        let mut env = Env::new();
        let env_ptr: NapiEnv = &mut env;
        let source = [1_u64, 2, 3, 0];
        let mut value = ptr::null_mut();
        assert_eq!(
            napi_create_bigint_words(env_ptr, 1, source.len(), source.as_ptr(), &mut value),
            NAPI_OK
        );
        let mut sign = 0;
        let mut count = 0;
        assert_eq!(
            napi_get_value_bigint_words(env_ptr, value, &mut sign, &mut count, ptr::null_mut(),),
            NAPI_OK
        );
        assert_eq!(sign, 1);
        assert_eq!(count, 3);

        let mut copy = [0_u64; 2];
        count = copy.len();
        assert_eq!(
            napi_get_value_bigint_words(env_ptr, value, &mut sign, &mut count, copy.as_mut_ptr(),),
            NAPI_OK
        );
        assert_eq!(count, 3);
        assert_eq!(copy, [1, 2]);

        assert_eq!(
            napi_create_bigint_words(env_ptr, 1, 0, ptr::null(), &mut value),
            NAPI_OK
        );
        count = 0;
        assert_eq!(
            napi_get_value_bigint_words(env_ptr, value, &mut sign, &mut count, ptr::null_mut(),),
            NAPI_OK
        );
        assert_eq!(sign, 0);
        assert_eq!(count, 1);
    }
}

#[test]
fn strict_equality_compares_bigint_values_and_date_coercion_uses_milliseconds() {
    unsafe {
        let mut env = Env::new();
        let env_ptr: NapiEnv = &mut env;
        let words = [7_u64, 9];
        let mut left = ptr::null_mut();
        let mut right = ptr::null_mut();
        assert_eq!(
            napi_create_bigint_words(env_ptr, 1, words.len(), words.as_ptr(), &mut left),
            NAPI_OK
        );
        assert_eq!(
            napi_create_bigint_words(env_ptr, 1, words.len(), words.as_ptr(), &mut right),
            NAPI_OK
        );
        assert_ne!(left, right);
        let mut equal = false;
        assert_eq!(
            napi_strict_equals(env_ptr, left, right, &mut equal),
            NAPI_OK
        );
        assert!(equal);
        let positive = env.alloc(Value::BigInt {
            negative: false,
            words: words.to_vec(),
        });
        assert_eq!(
            napi_strict_equals(env_ptr, left, positive, &mut equal),
            NAPI_OK
        );
        assert!(!equal);

        let mut date = ptr::null_mut();
        assert_eq!(napi_create_date(env_ptr, 1234.5, &mut date), NAPI_OK);
        let mut number = ptr::null_mut();
        assert_eq!(napi_coerce_to_number(env_ptr, date, &mut number), NAPI_OK);
        assert!(matches!(value_ref(number), Ok(Value::Number(1234.5))));
        assert_eq!(
            napi_strict_equals(ptr::null_mut(), left, right, &mut equal),
            NAPI_INVALID_ARG
        );
    }
}
