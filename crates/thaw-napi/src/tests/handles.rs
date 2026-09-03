#[test]
fn async_contexts_validate_names_and_environment_ownership() {
    unsafe extern "C" fn return_undefined(env: NapiEnv, _info: NapiCallbackInfo) -> NapiValue {
        let mut result = ptr::null_mut();
        let _ = napi_get_undefined(env, &mut result);
        result
    }

    unsafe {
        let mut env = Env::new();
        let mut other_env = Env::new();
        let env_ptr: NapiEnv = &mut env;
        let mut name = ptr::null_mut();
        assert_eq!(
            napi_create_string_utf8(env_ptr, c"test-resource".as_ptr(), 13, &mut name),
            NAPI_OK
        );
        let mut context = ptr::null_mut();
        assert_eq!(
            napi_async_init(env_ptr, ptr::null_mut(), name, &mut context),
            NAPI_OK
        );
        assert!(!context.is_null());
        assert_eq!(
            napi_async_destroy(&mut other_env, context),
            NAPI_INVALID_ARG
        );
        let mut callback_scope = ptr::null_mut();
        assert_eq!(
            napi_open_callback_scope(env_ptr, ptr::null_mut(), context, &mut callback_scope),
            NAPI_OK
        );
        assert_eq!(
            napi_close_handle_scope(env_ptr, callback_scope),
            NAPI_HANDLE_SCOPE_MISMATCH
        );
        assert_eq!(napi_close_callback_scope(env_ptr, callback_scope), NAPI_OK);

        let mut function = ptr::null_mut();
        let mut receiver = ptr::null_mut();
        assert_eq!(
            napi_create_function(
                env_ptr,
                c"callback".as_ptr(),
                8,
                Some(return_undefined),
                ptr::null_mut(),
                &mut function
            ),
            NAPI_OK
        );
        assert_eq!(napi_get_undefined(env_ptr, &mut receiver), NAPI_OK);
        assert_eq!(
            napi_make_callback(
                env_ptr,
                context,
                receiver,
                function,
                0,
                ptr::null(),
                ptr::null_mut()
            ),
            NAPI_OK
        );
        assert_eq!(napi_async_destroy(env_ptr, context), NAPI_OK);
        assert_eq!(napi_async_destroy(env_ptr, context), NAPI_INVALID_ARG);
        let mut info = ptr::null();
        assert_eq!(napi_get_last_error_info(env_ptr, &mut info), NAPI_OK);
        assert_eq!((*info).error_code, NAPI_INVALID_ARG);
        assert_eq!(
            napi_open_callback_scope(env_ptr, ptr::null_mut(), context, &mut callback_scope),
            NAPI_INVALID_ARG
        );
        assert_eq!(
            napi_make_callback(
                env_ptr,
                context,
                receiver,
                function,
                0,
                ptr::null(),
                ptr::null_mut()
            ),
            NAPI_INVALID_ARG
        );

        let mut number = ptr::null_mut();
        assert_eq!(napi_create_int32(env_ptr, 1, &mut number), NAPI_OK);
        assert_eq!(
            napi_async_init(env_ptr, ptr::null_mut(), number, &mut context),
            NAPI_STRING_EXPECTED
        );
        assert_eq!(
            napi_async_init(&mut other_env, ptr::null_mut(), name, &mut context),
            NAPI_INVALID_ARG
        );
    }
}

#[test]
fn value_extractors_reject_handles_owned_by_another_environment() {
    unsafe {
        let mut env = Env::new();
        let env_ptr: NapiEnv = &mut env;
        let mut foreign_env = Env::new();
        let number = foreign_env.alloc(Value::Number(3.5));
        let boolean = foreign_env.alloc(Value::Bool(true));
        let string = foreign_env.alloc(Value::String("foreign".into()));
        let date = foreign_env.alloc(Value::Date(1.0));
        let bigint = foreign_env.alloc(Value::BigInt {
            negative: false,
            words: vec![7],
        });
        let array = foreign_env.alloc(Value::Array(vec![Some(number)]));
        let buffer = foreign_env.alloc(Value::Buffer(vec![1, 2]));
        let array_buffer = foreign_env.alloc(Value::ArrayBuffer {
            bytes: vec![0; 8],
            detached: false,
        });
        let typed_array = foreign_env.alloc(Value::TypedArray {
            array_type: 1,
            length: 2,
            array_buffer,
            byte_offset: 0,
        });
        let data_view = foreign_env.alloc(Value::DataView {
            length: 4,
            array_buffer,
            byte_offset: 0,
        });
        let promise =
            foreign_env.alloc(Value::Promise(Rc::new(RefCell::new(PromiseState::Pending))));
        let object = foreign_env.alloc(Value::Object(HashMap::new()));
        let error = foreign_env.alloc(Value::Error("foreign".into()));
        let shared = foreign_env.alloc(Value::SharedArrayBuffer(vec![0; 4]));

        let mut float = 0.0;
        let mut integer = 0_i64;
        let mut unsigned = 0_u64;
        let mut boolean_out = false;
        let mut lossless = false;
        let mut count = 0_usize;
        let mut sign = 0;
        let mut length = 0_usize;
        let mut length32 = 0_u32;
        let mut pointer = ptr::null_mut();
        let mut value_out = ptr::null_mut();
        let mut kind = 0;

        assert_eq!(
            napi_get_value_double(env_ptr, number, &mut float),
            NAPI_INVALID_ARG
        );
        assert_eq!(
            napi_get_value_int64(env_ptr, number, &mut integer),
            NAPI_INVALID_ARG
        );
        assert_eq!(
            napi_get_value_bool(env_ptr, boolean, &mut boolean_out),
            NAPI_INVALID_ARG
        );
        assert_eq!(
            napi_get_date_value(env_ptr, date, &mut float),
            NAPI_INVALID_ARG
        );
        assert_eq!(
            napi_is_date(env_ptr, date, &mut boolean_out),
            NAPI_INVALID_ARG
        );
        assert_eq!(
            napi_get_value_string_utf8(env_ptr, string, ptr::null_mut(), 0, &mut count),
            NAPI_INVALID_ARG
        );
        assert_eq!(
            napi_get_value_bigint_uint64(env_ptr, bigint, &mut unsigned, &mut lossless),
            NAPI_INVALID_ARG
        );
        assert_eq!(
            napi_get_value_bigint_words(env_ptr, bigint, &mut sign, &mut count, ptr::null_mut()),
            NAPI_INVALID_ARG
        );
        assert_eq!(
            napi_is_array(env_ptr, array, &mut boolean_out),
            NAPI_INVALID_ARG
        );
        assert_eq!(
            napi_get_array_length(env_ptr, array, &mut length32),
            NAPI_INVALID_ARG
        );
        assert_eq!(
            napi_is_promise(env_ptr, promise, &mut boolean_out),
            NAPI_INVALID_ARG
        );
        assert_eq!(
            napi_is_buffer(env_ptr, buffer, &mut boolean_out),
            NAPI_INVALID_ARG
        );
        assert_eq!(
            napi_get_buffer_info(env_ptr, buffer, &mut pointer, &mut length),
            NAPI_INVALID_ARG
        );
        assert_eq!(
            napi_is_arraybuffer(env_ptr, array_buffer, &mut boolean_out),
            NAPI_INVALID_ARG
        );
        assert_eq!(
            napi_get_arraybuffer_info(env_ptr, array_buffer, &mut pointer, &mut length),
            NAPI_INVALID_ARG
        );
        assert_eq!(
            napi_detach_arraybuffer(env_ptr, array_buffer),
            NAPI_INVALID_ARG
        );
        assert_eq!(
            napi_create_typedarray(env_ptr, 1, 1, array_buffer, 0, &mut value_out),
            NAPI_INVALID_ARG
        );
        assert_eq!(
            napi_get_typedarray_info(
                env_ptr,
                typed_array,
                &mut kind,
                &mut length,
                &mut pointer,
                &mut value_out,
                &mut count
            ),
            NAPI_INVALID_ARG
        );
        assert_eq!(
            napi_create_dataview(env_ptr, 1, array_buffer, 0, &mut value_out),
            NAPI_INVALID_ARG
        );
        assert_eq!(
            napi_get_dataview_info(
                env_ptr,
                data_view,
                &mut length,
                &mut pointer,
                &mut value_out,
                &mut count
            ),
            NAPI_INVALID_ARG
        );
        assert_eq!(napi_typeof(env_ptr, number, &mut kind), NAPI_INVALID_ARG);
        assert_eq!(
            napi_strict_equals(env_ptr, number, number, &mut boolean_out),
            NAPI_INVALID_ARG
        );
        assert_eq!(
            napi_coerce_to_number(env_ptr, number, &mut value_out),
            NAPI_INVALID_ARG
        );
        assert_eq!(
            napi_coerce_to_string(env_ptr, string, &mut value_out),
            NAPI_INVALID_ARG
        );
        assert_eq!(
            napi_coerce_to_object(env_ptr, boolean, &mut value_out),
            NAPI_INVALID_ARG
        );
        assert_eq!(
            napi_get_prototype(env_ptr, object, &mut value_out),
            NAPI_INVALID_ARG
        );
        assert_eq!(napi_object_seal(env_ptr, object), NAPI_INVALID_ARG);
        assert_eq!(napi_object_freeze(env_ptr, object), NAPI_INVALID_ARG);
        assert_eq!(
            napi_is_error(env_ptr, error, &mut boolean_out),
            NAPI_INVALID_ARG
        );
        assert_eq!(napi_throw(env_ptr, error), NAPI_INVALID_ARG);
        assert_eq!(
            node_api_is_sharedarraybuffer(env_ptr, shared, &mut boolean_out),
            NAPI_INVALID_ARG
        );
        let mut error_info = ptr::null();
        assert_eq!(
            napi_get_last_error_info(ptr::null_mut(), &mut error_info),
            NAPI_INVALID_ARG
        );
        assert_eq!(
            napi_get_uv_event_loop(ptr::null_mut(), &mut pointer),
            NAPI_INVALID_ARG
        );
    }
}

#[test]
fn references_enforce_environment_ownership_and_refcounts() {
    unsafe {
        let mut env = Env::new();
        let mut other_env = Env::new();
        let env_ptr: NapiEnv = &mut env;
        let value = env.alloc(Value::Object(HashMap::new()));
        let foreign = other_env.alloc(Value::Object(HashMap::new()));
        let mut reference = ptr::null_mut();
        assert_eq!(
            napi_create_reference(env_ptr, value, 1, &mut reference),
            NAPI_OK
        );
        assert_eq!(
            napi_create_reference(env_ptr, foreign, 1, &mut ptr::null_mut()),
            NAPI_INVALID_ARG
        );
        let mut actual = ptr::null_mut();
        assert_eq!(
            napi_get_reference_value(&mut other_env, reference, &mut actual),
            NAPI_INVALID_ARG
        );
        assert_eq!(
            napi_get_reference_value(env_ptr, reference, &mut actual),
            NAPI_OK
        );
        assert_eq!(actual, value);
        let mut count = 0;
        assert_eq!(napi_reference_ref(env_ptr, reference, &mut count), NAPI_OK);
        assert_eq!(count, 2);
        assert_eq!(
            napi_reference_unref(env_ptr, reference, ptr::null_mut()),
            NAPI_OK
        );
        assert_eq!(
            napi_reference_unref(env_ptr, reference, &mut count),
            NAPI_OK
        );
        assert_eq!(count, 0);
        assert_eq!(
            napi_reference_unref(env_ptr, reference, &mut count),
            NAPI_GENERIC_FAILURE
        );
        (*reference).count = u32::MAX;
        assert_eq!(
            napi_reference_ref(env_ptr, reference, &mut count),
            NAPI_GENERIC_FAILURE
        );
        assert_eq!(
            napi_delete_reference(&mut other_env, reference),
            NAPI_INVALID_ARG
        );
        assert_eq!(napi_delete_reference(env_ptr, reference), NAPI_OK);
        assert_eq!(napi_delete_reference(env_ptr, reference), NAPI_INVALID_ARG);
        assert_eq!(
            napi_get_reference_value(env_ptr, reference, &mut actual),
            NAPI_INVALID_ARG
        );
        assert_eq!(
            napi_reference_ref(env_ptr, reference, &mut count),
            NAPI_INVALID_ARG
        );
        assert_eq!(
            napi_reference_unref(env_ptr, reference, &mut count),
            NAPI_INVALID_ARG
        );
    }
}

#[test]
fn callback_boundaries_validate_values_and_fill_missing_arguments() {
    unsafe extern "C" fn inspect_arguments(env: NapiEnv, info: NapiCallbackInfo) -> NapiValue {
        let mut argc = 3;
        let mut argv = [ptr::null_mut(); 3];
        let mut this_arg = ptr::null_mut();
        assert_eq!(
            unsafe {
                napi_get_cb_info(
                    env,
                    info,
                    &mut argc,
                    argv.as_mut_ptr(),
                    &mut this_arg,
                    ptr::null_mut(),
                )
            },
            NAPI_OK
        );
        assert_eq!(argc, 1);
        assert!(matches!(
            unsafe { value_ref(argv[1]) },
            Ok(Value::Undefined)
        ));
        assert!(matches!(
            unsafe { value_ref(argv[2]) },
            Ok(Value::Undefined)
        ));
        ptr::null_mut()
    }

    unsafe extern "C" fn return_array(env: NapiEnv, info: NapiCallbackInfo) -> NapiValue {
        let mut target = ptr::null_mut();
        assert_eq!(
            unsafe { napi_get_new_target(env, info, &mut target) },
            NAPI_OK
        );
        assert!(!target.is_null());
        let mut array = ptr::null_mut();
        assert_eq!(unsafe { napi_create_array(env, &mut array) }, NAPI_OK);
        array
    }

    unsafe {
        let mut env = Env::new();
        let mut other_env = Env::new();
        let env_ptr: NapiEnv = &mut env;
        let mut function = ptr::null_mut();
        assert_eq!(
            napi_create_function(
                env_ptr,
                c"inspect".as_ptr(),
                NAPI_AUTO_LENGTH,
                Some(inspect_arguments),
                ptr::null_mut(),
                &mut function,
            ),
            NAPI_OK
        );
        let this_arg = env.alloc(Value::Object(HashMap::new()));
        let argument = env.alloc(Value::Number(1.0));
        let mut result = ptr::null_mut();
        assert_eq!(
            napi_call_function(env_ptr, this_arg, function, 1, &argument, &mut result),
            NAPI_OK
        );
        assert!(matches!(value_ref(result), Ok(Value::Undefined)));
        let foreign = other_env.alloc(Value::Number(2.0));
        assert_eq!(
            napi_call_function(env_ptr, this_arg, function, 1, &foreign, &mut result),
            NAPI_INVALID_ARG
        );

        let mut constructor = ptr::null_mut();
        assert_eq!(
            napi_define_class(
                env_ptr,
                c"ReturnsArray".as_ptr(),
                NAPI_AUTO_LENGTH,
                Some(return_array),
                ptr::null_mut(),
                0,
                ptr::null(),
                &mut constructor,
            ),
            NAPI_OK
        );
        assert_eq!(
            napi_new_instance(env_ptr, constructor, 0, ptr::null(), &mut result),
            NAPI_OK
        );
        assert!(matches!(value_ref(result), Ok(Value::Array(_))));

        assert_eq!(
            napi_call_function(env_ptr, this_arg, argument, 0, ptr::null(), &mut result),
            NAPI_FUNCTION_EXPECTED
        );
        let mut info = ptr::null();
        assert_eq!(napi_get_last_error_info(env_ptr, &mut info), NAPI_OK);
        assert_eq!((*info).error_code, NAPI_FUNCTION_EXPECTED);
        assert_eq!(
            napi_new_instance(env_ptr, argument, 0, ptr::null(), &mut result),
            NAPI_FUNCTION_EXPECTED
        );
        assert_eq!(napi_get_last_error_info(env_ptr, &mut info), NAPI_OK);
        assert_eq!((*info).error_code, NAPI_FUNCTION_EXPECTED);

        env.exception = Some(env.alloc(Value::Error("pending".into())));
        assert_eq!(
            napi_call_function(env_ptr, this_arg, function, 0, ptr::null(), &mut result),
            NAPI_PENDING_EXCEPTION
        );
        assert_eq!(napi_get_last_error_info(env_ptr, &mut info), NAPI_OK);
        assert_eq!((*info).error_code, NAPI_PENDING_EXCEPTION);
    }
}

#[test]
fn sealed_and_frozen_objects_enforce_integrity_levels() {
    unsafe {
        let mut env = Env::new();
        let env_ptr: NapiEnv = &mut env;
        let mut object = ptr::null_mut();
        let mut existing = ptr::null_mut();
        let mut added = ptr::null_mut();
        let mut value = ptr::null_mut();
        assert_eq!(napi_create_object(env_ptr, &mut object), NAPI_OK);
        assert_eq!(
            napi_create_string_utf8(env_ptr, c"existing".as_ptr(), 8, &mut existing),
            NAPI_OK
        );
        assert_eq!(
            napi_create_string_utf8(env_ptr, c"added".as_ptr(), 5, &mut added),
            NAPI_OK
        );
        assert_eq!(napi_create_int32(env_ptr, 1, &mut value), NAPI_OK);
        assert_eq!(napi_set_property(env_ptr, object, existing, value), NAPI_OK);
        assert_eq!(napi_object_seal(env_ptr, object), NAPI_OK);
        assert_eq!(napi_create_int32(env_ptr, 2, &mut value), NAPI_OK);
        assert_eq!(napi_set_property(env_ptr, object, existing, value), NAPI_OK);
        assert_eq!(
            napi_set_property(env_ptr, object, added, value),
            NAPI_GENERIC_FAILURE
        );
        let mut deleted = true;
        assert_eq!(
            napi_delete_property(env_ptr, object, existing, &mut deleted),
            NAPI_OK
        );
        assert!(!deleted);
        assert_eq!(napi_object_freeze(env_ptr, object), NAPI_OK);
        assert_eq!(
            napi_set_property(env_ptr, object, existing, value),
            NAPI_GENERIC_FAILURE
        );

        let mut array = ptr::null_mut();
        assert_eq!(
            napi_create_array_with_length(env_ptr, 1, &mut array),
            NAPI_OK
        );
        assert_eq!(napi_set_element(env_ptr, array, 0, value), NAPI_OK);
        assert_eq!(napi_object_seal(env_ptr, array), NAPI_OK);
        assert_eq!(napi_set_element(env_ptr, array, 0, value), NAPI_OK);
        assert_eq!(
            napi_set_element(env_ptr, array, 1, value),
            NAPI_GENERIC_FAILURE
        );
        assert_eq!(napi_object_freeze(env_ptr, array), NAPI_OK);
        assert_eq!(
            napi_set_element(env_ptr, array, 0, value),
            NAPI_GENERIC_FAILURE
        );

        let mut described = ptr::null_mut();
        assert_eq!(napi_create_object(env_ptr, &mut described), NAPI_OK);
        let descriptor = NapiPropertyDescriptor {
            utf8name: c"fixed".as_ptr(),
            name: ptr::null_mut(),
            method: None,
            getter: None,
            setter: None,
            value,
            attributes: 0,
            data: ptr::null_mut(),
        };
        assert_eq!(
            napi_define_properties(env_ptr, described, 1, &descriptor),
            NAPI_OK
        );
        assert_eq!(
            napi_set_named_property(env_ptr, described, c"fixed".as_ptr(), value),
            NAPI_GENERIC_FAILURE
        );
        assert_eq!(
            napi_create_string_utf8(env_ptr, c"fixed".as_ptr(), 5, &mut existing),
            NAPI_OK
        );
        deleted = true;
        assert_eq!(
            napi_delete_property(env_ptr, described, existing, &mut deleted),
            NAPI_OK
        );
        assert!(!deleted);
    }
}

#[test]
fn delete_property_allows_ignored_results_and_inherited_keys() {
    unsafe {
        let mut env = Env::new();
        let env_ptr: NapiEnv = &mut env;
        let mut prototype = ptr::null_mut();
        let mut object = ptr::null_mut();
        assert_eq!(napi_create_object(env_ptr, &mut prototype), NAPI_OK);
        assert_eq!(napi_create_object(env_ptr, &mut object), NAPI_OK);
        env.prototypes.insert(object as usize, prototype as usize);
        let key = env.alloc(Value::String("inherited".into()));
        let value = env.alloc(Value::Number(1.0));
        assert_eq!(napi_set_property(env_ptr, prototype, key, value), NAPI_OK);
        assert_eq!(napi_object_seal(env_ptr, object), NAPI_OK);
        assert_eq!(
            napi_delete_property(env_ptr, object, key, ptr::null_mut()),
            NAPI_OK
        );
        let mut present = false;
        assert_eq!(
            napi_has_property(env_ptr, object, key, &mut present),
            NAPI_OK
        );
        assert!(
            present,
            "deleting an inherited key must not affect its owner"
        );

        let own = env.alloc(Value::String("own".into()));
        let mut plain = ptr::null_mut();
        assert_eq!(napi_create_object(env_ptr, &mut plain), NAPI_OK);
        assert_eq!(napi_set_property(env_ptr, plain, own, value), NAPI_OK);
        assert_eq!(
            napi_delete_property(env_ptr, plain, own, ptr::null_mut()),
            NAPI_OK
        );
        assert_eq!(
            napi_has_own_property(env_ptr, plain, own, &mut present),
            NAPI_OK
        );
        assert!(!present);
    }
}

#[test]
fn range_errors_participate_in_exception_state() {
    unsafe {
        let mut env = Env::new();
        let env_ptr: NapiEnv = &mut env;
        assert_eq!(
            napi_throw_error(env_ptr, ptr::null(), ptr::null()),
            NAPI_INVALID_ARG
        );
        let mut info = ptr::null();
        assert_eq!(napi_get_last_error_info(env_ptr, &mut info), NAPI_OK);
        assert_eq!((*info).error_code, NAPI_INVALID_ARG);
        assert!(env.exception.is_none());
        let mut message = ptr::null_mut();
        assert_eq!(
            napi_create_string_utf8(env_ptr, c"outside range".as_ptr(), 13, &mut message),
            NAPI_OK
        );
        let mut error = ptr::null_mut();
        assert_eq!(
            napi_create_range_error(env_ptr, ptr::null_mut(), message, &mut error),
            NAPI_OK
        );
        let mut is_error = false;
        assert_eq!(napi_is_error(env_ptr, error, &mut is_error), NAPI_OK);
        assert!(is_error);
        assert_eq!(napi_throw(env_ptr, error), NAPI_OK);
        assert_eq!(
            napi_get_and_clear_last_exception(env_ptr, ptr::null_mut()),
            NAPI_INVALID_ARG
        );
        let mut pending = false;
        assert_eq!(napi_is_exception_pending(env_ptr, &mut pending), NAPI_OK);
        assert!(pending);
        assert_eq!(napi_get_last_error_info(env_ptr, &mut info), NAPI_OK);
        assert_eq!((*info).error_code, NAPI_INVALID_ARG);
        let mut caught = ptr::null_mut();
        assert_eq!(
            napi_get_and_clear_last_exception(env_ptr, &mut caught),
            NAPI_OK
        );
        assert_eq!(caught, error);
        assert_eq!(
            napi_throw_range_error(env_ptr, ptr::null(), c"again".as_ptr()),
            NAPI_OK
        );
        assert_eq!(napi_is_exception_pending(env_ptr, &mut pending), NAPI_OK);
        assert!(pending);
        assert_eq!(
            napi_get_and_clear_last_exception(env_ptr, &mut caught),
            NAPI_OK
        );
        assert_eq!(
            node_api_create_syntax_error(env_ptr, ptr::null_mut(), message, &mut error,),
            NAPI_OK
        );
        assert_eq!(napi_is_error(env_ptr, error, &mut is_error), NAPI_OK);
        assert!(is_error);
        assert_eq!(
            node_api_throw_syntax_error(env_ptr, ptr::null(), c"syntax".as_ptr()),
            NAPI_OK
        );
        assert_eq!(napi_is_exception_pending(env_ptr, &mut pending), NAPI_OK);
        assert!(pending);
    }
}

#[test]
fn error_values_expose_name_message_code_and_stringification() {
    unsafe {
        let mut env = Env::new();
        let env_ptr: NapiEnv = &mut env;
        let message = env.alloc(Value::String("failed".into()));
        let code = env.alloc(Value::String("E_TEST".into()));
        let mut error = ptr::null_mut();
        assert_eq!(
            napi_create_type_error(env_ptr, code, message, &mut error),
            NAPI_OK
        );
        let number = env.alloc(Value::Number(1.0));
        assert_eq!(
            napi_create_type_error(env_ptr, code, number, &mut error),
            NAPI_STRING_EXPECTED
        );
        let mut info = ptr::null();
        assert_eq!(napi_get_last_error_info(env_ptr, &mut info), NAPI_OK);
        assert_eq!((*info).error_code, NAPI_STRING_EXPECTED);
        for (name, expected) in [
            (c"name", "TypeError"),
            (c"message", "failed"),
            (c"code", "E_TEST"),
        ] {
            let mut actual = ptr::null_mut();
            assert_eq!(
                napi_get_named_property(env_ptr, error, name.as_ptr(), &mut actual),
                NAPI_OK
            );
            assert!(matches!(value_ref(actual), Ok(Value::String(value)) if value == expected));
        }
        let name_key = env.alloc(Value::String("name".into()));
        let message_key = env.alloc(Value::String("message".into()));
        let mut own = true;
        assert_eq!(
            napi_has_own_property(env_ptr, error, name_key, &mut own),
            NAPI_OK
        );
        assert!(!own);
        assert_eq!(
            napi_has_own_property(env_ptr, error, message_key, &mut own),
            NAPI_OK
        );
        assert!(own);

        let mut string = ptr::null_mut();
        assert_eq!(napi_coerce_to_string(env_ptr, error, &mut string), NAPI_OK);
        assert!(
            matches!(value_ref(string), Ok(Value::String(value)) if value == "TypeError: failed")
        );
        let custom_name = env.alloc(Value::String("CustomError".into()));
        assert_eq!(
            napi_set_named_property(env_ptr, error, c"name".as_ptr(), custom_name),
            NAPI_OK
        );
        assert_eq!(napi_coerce_to_string(env_ptr, error, &mut string), NAPI_OK);
        assert!(
            matches!(value_ref(string), Ok(Value::String(value)) if value == "CustomError: failed")
        );

        assert_eq!(
            napi_create_error(env_ptr, code, message, ptr::null_mut()),
            NAPI_INVALID_ARG
        );
        let number = env.alloc(Value::Number(1.0));
        assert_eq!(
            napi_create_error(env_ptr, number, message, &mut error),
            NAPI_STRING_EXPECTED
        );
    }
}

#[test]
fn own_properties_can_be_detected_and_deleted() {
    unsafe {
        let mut env = Env::new();
        let env_ptr: NapiEnv = &mut env;
        let mut object = ptr::null_mut();
        let mut key = ptr::null_mut();
        let mut value = ptr::null_mut();
        assert_eq!(napi_create_object(env_ptr, &mut object), NAPI_OK);
        assert_eq!(
            napi_create_string_utf8(env_ptr, c"answer".as_ptr(), 6, &mut key),
            NAPI_OK
        );
        assert_eq!(napi_create_int32(env_ptr, 42, &mut value), NAPI_OK);
        assert_eq!(napi_set_property(env_ptr, object, key, value), NAPI_OK);

        let mut present = false;
        assert_eq!(
            napi_has_own_property(env_ptr, object, key, &mut present),
            NAPI_OK
        );
        assert!(present);
        let mut deleted = false;
        assert_eq!(
            napi_delete_property(env_ptr, object, key, &mut deleted),
            NAPI_OK
        );
        assert!(deleted);
        assert_eq!(
            napi_has_own_property(env_ptr, object, key, &mut present),
            NAPI_OK
        );
        assert!(!present);
        assert_eq!(
            napi_has_property(env_ptr, object, key, &mut present),
            NAPI_OK
        );
        assert!(!present);
    }
}

#[test]
fn array_holes_and_promise_detection_follow_node_api() {
    unsafe {
        let mut env = Env::new();
        let env_ptr: NapiEnv = &mut env;
        let mut array = ptr::null_mut();
        assert_eq!(
            napi_create_array_with_length(env_ptr, 3, &mut array),
            NAPI_OK
        );
        let mut present = true;
        assert_eq!(napi_has_element(env_ptr, array, 1, &mut present), NAPI_OK);
        assert!(!present);
        let value = env.alloc(Value::Undefined);
        assert_eq!(napi_set_element(env_ptr, array, 1, value), NAPI_OK);
        assert_eq!(napi_has_element(env_ptr, array, 1, &mut present), NAPI_OK);
        assert!(present, "an explicit undefined value is not an array hole");
        let mut deleted = false;
        assert_eq!(
            napi_delete_element(env_ptr, array, 1, &mut deleted),
            NAPI_OK
        );
        assert!(deleted);
        assert_eq!(napi_has_element(env_ptr, array, 1, &mut present), NAPI_OK);
        assert!(!present);
        let mut length = 0;
        assert_eq!(napi_get_array_length(env_ptr, array, &mut length), NAPI_OK);
        assert_eq!(length, 3, "deleting an element must preserve array length");
        assert_eq!(
            json_from_value(array).unwrap(),
            serde_json::json!([null, null, null])
        );

        let mut deferred = ptr::null_mut();
        let mut promise = ptr::null_mut();
        assert_eq!(
            napi_create_promise(env_ptr, &mut deferred, &mut promise),
            NAPI_OK
        );
        assert_eq!(napi_is_promise(env_ptr, promise, &mut present), NAPI_OK);
        assert!(present);
        assert_eq!(napi_is_promise(env_ptr, array, &mut present), NAPI_OK);
        assert!(!present);
        let mut other_env = Env::new();
        let foreign_value = other_env.alloc(Value::Undefined);
        assert_eq!(
            napi_resolve_deferred(&mut other_env, deferred, foreign_value),
            NAPI_INVALID_ARG
        );
        assert_eq!(
            napi_resolve_deferred(env_ptr, deferred, foreign_value),
            NAPI_INVALID_ARG
        );
        assert_eq!(napi_resolve_deferred(env_ptr, deferred, value), NAPI_OK);
        assert_eq!(
            napi_resolve_deferred(env_ptr, deferred, value),
            NAPI_GENERIC_FAILURE
        );
        assert_eq!(
            napi_reject_deferred(env_ptr, deferred, value),
            NAPI_GENERIC_FAILURE
        );
    }
}

#[test]
fn element_apis_operate_on_objects_and_descriptors() {
    unsafe {
        let mut env = Env::new();
        let env_ptr: NapiEnv = &mut env;
        let mut object = ptr::null_mut();
        assert_eq!(napi_create_object(env_ptr, &mut object), NAPI_OK);
        let value = env.alloc(Value::Number(42.0));
        assert_eq!(napi_set_element(env_ptr, object, 7, value), NAPI_OK);
        let mut present = false;
        assert_eq!(napi_has_element(env_ptr, object, 7, &mut present), NAPI_OK);
        assert!(present);
        let mut actual = ptr::null_mut();
        assert_eq!(napi_get_element(env_ptr, object, 7, &mut actual), NAPI_OK);
        assert_eq!(actual, value);
        assert_eq!(
            napi_delete_element(env_ptr, object, 7, ptr::null_mut()),
            NAPI_OK
        );
        assert_eq!(napi_has_element(env_ptr, object, 7, &mut present), NAPI_OK);
        assert!(!present);

        let descriptor = NapiPropertyDescriptor {
            utf8name: c"8".as_ptr(),
            name: ptr::null_mut(),
            method: None,
            getter: None,
            setter: None,
            value,
            attributes: 0,
            data: ptr::null_mut(),
        };
        assert_eq!(
            napi_define_properties(env_ptr, object, 1, &descriptor),
            NAPI_OK
        );
        let mut deleted = true;
        assert_eq!(
            napi_delete_element(env_ptr, object, 8, &mut deleted),
            NAPI_OK
        );
        assert!(!deleted);
        assert_eq!(napi_has_element(env_ptr, object, 8, &mut present), NAPI_OK);
        assert!(present);
    }
}

#[cfg(feature = "quickjs")]
#[test]
fn run_script_evaluates_values_and_reports_exceptions() {
    unsafe {
        let mut env = Env::new();
        let env_ptr: NapiEnv = &mut env;
        let mut script = ptr::null_mut();
        assert_eq!(
            napi_create_string_utf8(
                env_ptr,
                c"{\"answer\":42}".as_ptr(),
                NAPI_AUTO_LENGTH,
                &mut script,
            ),
            NAPI_OK
        );
        let mut result = ptr::null_mut();
        assert_eq!(napi_run_script(env_ptr, script, &mut result), NAPI_OK);
        assert_eq!(
            json_from_value(result).unwrap(),
            serde_json::json!({"answer": 42.0})
        );

        assert_eq!(
            napi_create_string_utf8(
                env_ptr,
                c"globalThis.__thawNapiProbe = 40; ({ answer: __thawNapiProbe + 2 })".as_ptr(),
                NAPI_AUTO_LENGTH,
                &mut script,
            ),
            NAPI_OK
        );
        assert_eq!(napi_run_script(env_ptr, script, &mut result), NAPI_OK);
        assert_eq!(
            json_from_value(result).unwrap(),
            serde_json::json!({"answer": 42.0})
        );

        assert_eq!(
            napi_create_string_utf8(
                env_ptr,
                c"__thawNapiProbe + 2".as_ptr(),
                NAPI_AUTO_LENGTH,
                &mut script,
            ),
            NAPI_OK
        );
        assert_eq!(napi_run_script(env_ptr, script, &mut result), NAPI_OK);
        assert!(matches!(value_ref(result), Ok(Value::Number(42.0))));

        assert_eq!(
            napi_create_string_utf8(
                env_ptr,
                c"throw new Error('script failed')".as_ptr(),
                NAPI_AUTO_LENGTH,
                &mut script,
            ),
            NAPI_OK
        );
        assert_eq!(
            napi_run_script(env_ptr, script, &mut result),
            NAPI_PENDING_EXCEPTION
        );
        let mut pending = false;
        assert_eq!(napi_is_exception_pending(env_ptr, &mut pending), NAPI_OK);
        assert!(pending);
        assert_eq!(
            napi_get_and_clear_last_exception(env_ptr, &mut result),
            NAPI_OK
        );
        assert!(
            matches!(value_ref(result), Ok(Value::Error(message)) if message.contains("script failed"))
        );
    }
}

#[cfg(not(feature = "quickjs"))]
#[test]
fn run_script_without_quickjs_accepts_json_only() {
    unsafe {
        let mut env = Env::new();
        let env_ptr: NapiEnv = &mut env;
        let mut script = ptr::null_mut();
        assert_eq!(
            napi_create_string_utf8(
                env_ptr,
                c"{\"answer\":42}".as_ptr(),
                NAPI_AUTO_LENGTH,
                &mut script,
            ),
            NAPI_OK
        );
        let mut result = ptr::null_mut();
        assert_eq!(napi_run_script(env_ptr, script, &mut result), NAPI_OK);
        assert_eq!(
            json_from_value(result).unwrap(),
            serde_json::json!({"answer": 42.0})
        );

        assert_eq!(
            napi_create_string_utf8(env_ptr, c"1 + 1".as_ptr(), NAPI_AUTO_LENGTH, &mut script),
            NAPI_OK
        );
        assert_eq!(
            napi_run_script(env_ptr, script, &mut result),
            NAPI_PENDING_EXCEPTION
        );
        let mut pending = false;
        assert_eq!(napi_is_exception_pending(env_ptr, &mut pending), NAPI_OK);
        assert!(pending);
        assert_eq!(
            napi_get_and_clear_last_exception(env_ptr, &mut result),
            NAPI_OK
        );
        assert!(matches!(value_ref(result), Ok(Value::Error(message)) if message.contains("QuickJS feature")));
    }
}
