#[test]
fn handle_scopes_enforce_environment_order_kind_and_single_escape() {
    unsafe {
        let mut env = Env::new();
        let env_ptr: NapiEnv = &mut env;
        let mut other_env = Env::new();
        let other_env_ptr: NapiEnv = &mut other_env;
        let mut outer = ptr::null_mut();
        let mut inner = ptr::null_mut();
        assert_eq!(napi_open_handle_scope(env_ptr, &mut outer), NAPI_OK);
        assert_eq!(
            napi_open_escapable_handle_scope(env_ptr, &mut inner),
            NAPI_OK
        );
        assert_ne!(outer, inner);
        assert_eq!(
            napi_close_handle_scope(env_ptr, outer),
            NAPI_HANDLE_SCOPE_MISMATCH
        );
        let mut info = ptr::null();
        assert_eq!(napi_get_last_error_info(env_ptr, &mut info), NAPI_OK);
        assert_eq!((*info).error_code, NAPI_HANDLE_SCOPE_MISMATCH);
        assert_eq!(
            napi_close_escapable_handle_scope(other_env_ptr, inner),
            NAPI_INVALID_ARG
        );

        let value = env.alloc(Value::Number(42.0));
        let mut escaped = ptr::null_mut();
        assert_eq!(
            napi_escape_handle(env_ptr, inner, value, &mut escaped),
            NAPI_OK
        );
        assert_eq!(escaped, value);
        assert_eq!(
            napi_escape_handle(env_ptr, inner, value, &mut escaped),
            NAPI_ESCAPE_CALLED_TWICE
        );
        assert_eq!(napi_get_last_error_info(env_ptr, &mut info), NAPI_OK);
        assert_eq!((*info).error_code, NAPI_ESCAPE_CALLED_TWICE);
        assert_eq!(
            napi_close_handle_scope(env_ptr, inner),
            NAPI_HANDLE_SCOPE_MISMATCH
        );
        assert_eq!(napi_close_escapable_handle_scope(env_ptr, inner), NAPI_OK);
        assert_eq!(
            napi_close_escapable_handle_scope(env_ptr, inner),
            NAPI_HANDLE_SCOPE_MISMATCH
        );
        assert_eq!(napi_close_handle_scope(env_ptr, outer), NAPI_OK);
        assert_eq!(
            napi_close_handle_scope(env_ptr, outer),
            NAPI_HANDLE_SCOPE_MISMATCH
        );
    }
}

#[test]
fn symbols_have_unique_identity_and_property_namespace() {
    unsafe {
        let mut env = Env::new();
        let env_ptr: NapiEnv = &mut env;
        let mut description = ptr::null_mut();
        assert_eq!(
            napi_create_string_utf8(env_ptr, c"same".as_ptr(), 4, &mut description),
            NAPI_OK
        );
        let mut first = ptr::null_mut();
        let mut second = ptr::null_mut();
        assert_eq!(
            napi_create_symbol(env_ptr, description, &mut first),
            NAPI_OK
        );
        assert_eq!(
            napi_create_symbol(env_ptr, description, &mut second),
            NAPI_OK
        );
        let mut equal = false;
        assert_eq!(
            napi_strict_equals(env_ptr, first, first, &mut equal),
            NAPI_OK
        );
        assert!(equal);
        assert_eq!(
            napi_strict_equals(env_ptr, first, second, &mut equal),
            NAPI_OK
        );
        assert!(!equal);
        let mut registered_first = ptr::null_mut();
        let mut registered_second = ptr::null_mut();
        assert_eq!(
            node_api_symbol_for(
                env_ptr,
                c"shared".as_ptr(),
                NAPI_AUTO_LENGTH,
                &mut registered_first,
            ),
            NAPI_OK
        );
        assert_eq!(
            node_api_symbol_for(env_ptr, c"shared".as_ptr(), 6, &mut registered_second),
            NAPI_OK
        );
        assert_eq!(registered_first, registered_second);
        assert_eq!(
            napi_strict_equals(env_ptr, registered_first, registered_second, &mut equal,),
            NAPI_OK
        );
        assert!(equal);

        let mut object = ptr::null_mut();
        assert_eq!(napi_create_object(env_ptr, &mut object), NAPI_OK);
        let string_value = env.alloc(Value::Number(1.0));
        let first_value = env.alloc(Value::Number(2.0));
        let second_value = env.alloc(Value::Number(3.0));
        assert_eq!(
            napi_set_named_property(env_ptr, object, c"same".as_ptr(), string_value),
            NAPI_OK
        );
        assert_eq!(
            napi_set_property(env_ptr, object, first, first_value),
            NAPI_OK
        );
        assert_eq!(
            napi_set_property(env_ptr, object, second, second_value),
            NAPI_OK
        );
        for (key, expected) in [(first, first_value), (second, second_value)] {
            let mut actual = ptr::null_mut();
            assert_eq!(
                napi_get_property(env_ptr, object, key, &mut actual),
                NAPI_OK
            );
            assert_eq!(actual, expected);
            let mut present = false;
            assert_eq!(
                napi_has_own_property(env_ptr, object, key, &mut present),
                NAPI_OK
            );
            assert!(present);
        }
        let json = json_from_value(object).unwrap();
        assert_eq!(json, serde_json::json!({"same": 1.0}));
    }
}

#[test]
fn value_creation_rejects_null_outputs_before_mutating_the_environment() {
    unsafe extern "C" fn noop_callback(_env: NapiEnv, _info: NapiCallbackInfo) -> NapiValue {
        ptr::null_mut()
    }

    unsafe {
        let mut env = Env::new();
        let env_ptr: NapiEnv = &mut env;
        let baseline = env.values.len();
        assert_eq!(
            napi_get_undefined(env_ptr, ptr::null_mut()),
            NAPI_INVALID_ARG
        );
        assert_eq!(napi_get_null(env_ptr, ptr::null_mut()), NAPI_INVALID_ARG);
        assert_eq!(
            napi_get_boolean(env_ptr, true, ptr::null_mut()),
            NAPI_INVALID_ARG
        );
        assert_eq!(
            napi_create_double(env_ptr, 1.0, ptr::null_mut()),
            NAPI_INVALID_ARG
        );
        assert_eq!(
            napi_create_date(env_ptr, 1.0, ptr::null_mut()),
            NAPI_INVALID_ARG
        );
        assert_eq!(
            napi_create_bigint_int64(env_ptr, 1, ptr::null_mut()),
            NAPI_INVALID_ARG
        );
        assert_eq!(
            napi_create_bigint_uint64(env_ptr, 1, ptr::null_mut()),
            NAPI_INVALID_ARG
        );
        assert_eq!(
            napi_create_bigint_words(env_ptr, 0, 0, ptr::null(), ptr::null_mut()),
            NAPI_INVALID_ARG
        );
        assert_eq!(
            napi_create_string_utf8(env_ptr, c"value".as_ptr(), 5, ptr::null_mut()),
            NAPI_INVALID_ARG
        );
        assert_eq!(
            napi_create_string_latin1(env_ptr, c"value".as_ptr(), 5, ptr::null_mut()),
            NAPI_INVALID_ARG
        );
        let utf16 = [b'v' as u16, 0];
        assert_eq!(
            napi_create_string_utf16(env_ptr, utf16.as_ptr(), 1, ptr::null_mut()),
            NAPI_INVALID_ARG
        );
        assert_eq!(
            napi_create_object(env_ptr, ptr::null_mut()),
            NAPI_INVALID_ARG
        );
        assert_eq!(
            napi_create_array_with_length(env_ptr, 2, ptr::null_mut()),
            NAPI_INVALID_ARG
        );
        let sentinel = ptr::dangling_mut::<c_void>();
        let mut data = sentinel;
        assert_eq!(
            napi_create_buffer(env_ptr, 2, &mut data, ptr::null_mut()),
            NAPI_INVALID_ARG
        );
        assert_eq!(data, sentinel);
        assert_eq!(
            napi_create_arraybuffer(env_ptr, 2, &mut data, ptr::null_mut()),
            NAPI_INVALID_ARG
        );
        assert_eq!(data, sentinel);
        assert_eq!(env.values.len(), baseline);

        let array_buffer = env.alloc(Value::ArrayBuffer {
            bytes: vec![0; 8],
            detached: false,
        });
        let after_backing = env.values.len();
        assert_eq!(
            napi_create_typedarray(env_ptr, 1, 1, array_buffer, 0, ptr::null_mut()),
            NAPI_INVALID_ARG
        );
        assert_eq!(
            napi_create_dataview(env_ptr, 1, array_buffer, 0, ptr::null_mut()),
            NAPI_INVALID_ARG
        );
        assert_eq!(
            napi_create_symbol(env_ptr, ptr::null_mut(), ptr::null_mut()),
            NAPI_INVALID_ARG
        );
        assert_eq!(env.values.len(), after_backing);

        let mut foreign_env = Env::new();
        let foreign_description = foreign_env.alloc(Value::String("foreign".into()));
        let mut symbol = ptr::null_mut();
        assert_eq!(
            napi_create_symbol(env_ptr, foreign_description, &mut symbol),
            NAPI_INVALID_ARG
        );
        assert!(symbol.is_null());
        assert_eq!(env.values.len(), after_backing);

        let mut output = ptr::null_mut();
        let invalid_bytes = ptr::dangling::<c_char>();
        let invalid_utf16 = ptr::dangling::<u16>();
        let invalid_words = ptr::dangling::<u64>();
        assert_eq!(
            napi_create_string_utf8(ptr::null_mut(), invalid_bytes, 1, &mut output),
            NAPI_INVALID_ARG
        );
        assert_eq!(
            napi_create_string_latin1(ptr::null_mut(), invalid_bytes, 1, &mut output),
            NAPI_INVALID_ARG
        );
        assert_eq!(
            napi_create_string_utf16(ptr::null_mut(), invalid_utf16, 1, &mut output),
            NAPI_INVALID_ARG
        );
        assert_eq!(
            node_api_create_property_key_utf8(ptr::null_mut(), invalid_bytes, 1, &mut output),
            NAPI_INVALID_ARG
        );
        assert_eq!(
            node_api_symbol_for(ptr::null_mut(), invalid_bytes, 1, &mut output),
            NAPI_INVALID_ARG
        );
        assert_eq!(
            napi_create_bigint_words(ptr::null_mut(), 0, 1, invalid_words, &mut output),
            NAPI_INVALID_ARG
        );
        assert_eq!(
            napi_create_function(
                ptr::null_mut(),
                invalid_bytes,
                1,
                Some(noop_callback),
                ptr::null_mut(),
                &mut output
            ),
            NAPI_INVALID_ARG
        );
    }
}

#[test]
fn all_property_names_filter_strings_symbols_and_attributes() {
    unsafe {
        let mut env = Env::new();
        let env_ptr: NapiEnv = &mut env;
        let mut object = ptr::null_mut();
        assert_eq!(napi_create_object(env_ptr, &mut object), NAPI_OK);
        let value = env.alloc(Value::Number(1.0));
        let descriptors = [
            NapiPropertyDescriptor {
                utf8name: c"visible".as_ptr(),
                name: ptr::null_mut(),
                method: None,
                getter: None,
                setter: None,
                value,
                attributes: NAPI_ENUMERABLE,
                data: ptr::null_mut(),
            },
            NapiPropertyDescriptor {
                utf8name: c"hidden".as_ptr(),
                name: ptr::null_mut(),
                method: None,
                getter: None,
                setter: None,
                value,
                attributes: 0,
                data: ptr::null_mut(),
            },
            NapiPropertyDescriptor {
                utf8name: c"2".as_ptr(),
                name: ptr::null_mut(),
                method: None,
                getter: None,
                setter: None,
                value,
                attributes: 0,
                data: ptr::null_mut(),
            },
        ];
        assert_eq!(
            napi_define_properties(env_ptr, object, descriptors.len(), descriptors.as_ptr()),
            NAPI_OK
        );
        let mut symbol = ptr::null_mut();
        assert_eq!(
            napi_create_symbol(env_ptr, ptr::null_mut(), &mut symbol),
            NAPI_OK
        );
        assert_eq!(napi_set_property(env_ptr, object, symbol, value), NAPI_OK);

        let mut names_value = ptr::null_mut();
        assert_eq!(
            napi_get_all_property_names(
                env_ptr,
                object,
                NAPI_KEY_OWN_ONLY,
                NAPI_ENUMERABLE | NAPI_KEY_SKIP_SYMBOLS,
                NAPI_KEY_NUMBERS_TO_STRINGS,
                &mut names_value,
            ),
            NAPI_OK
        );
        let Ok(Value::Array(names)) = value_ref(names_value) else {
            panic!("property names were not returned as an array");
        };
        assert_eq!(names.len(), 1);
        assert!(
            matches!(names[0].and_then(|value| value_ref(value).ok()), Some(Value::String(name)) if name == "visible")
        );

        assert_eq!(
            napi_get_all_property_names(
                env_ptr,
                object,
                NAPI_KEY_OWN_ONLY,
                NAPI_KEY_SKIP_STRINGS,
                NAPI_KEY_NUMBERS_TO_STRINGS,
                &mut names_value,
            ),
            NAPI_OK
        );
        let Ok(Value::Array(names)) = value_ref(names_value) else {
            panic!("symbol names were not returned as an array");
        };
        assert_eq!(names.as_slice(), &[Some(symbol)]);

        assert_eq!(
            napi_get_all_property_names(
                env_ptr,
                object,
                NAPI_KEY_OWN_ONLY,
                NAPI_KEY_ALL_PROPERTIES,
                NAPI_KEY_KEEP_NUMBERS,
                &mut names_value,
            ),
            NAPI_OK
        );
        let Ok(Value::Array(names)) = value_ref(names_value) else {
            panic!("all property names were not returned as an array");
        };
        assert_eq!(names.len(), 4);
        assert!(matches!(
            names[0].and_then(|value| value_ref(value).ok()),
            Some(Value::Number(2.0))
        ));

        let mut array = ptr::null_mut();
        assert_eq!(
            napi_create_array_with_length(env_ptr, 2, &mut array),
            NAPI_OK
        );
        assert_eq!(napi_set_element(env_ptr, array, 0, value), NAPI_OK);
        assert_eq!(napi_set_element(env_ptr, array, 1, value), NAPI_OK);
        assert_eq!(
            napi_get_all_property_names(
                env_ptr,
                array,
                NAPI_KEY_OWN_ONLY,
                NAPI_KEY_ALL_PROPERTIES,
                NAPI_KEY_KEEP_NUMBERS,
                &mut names_value,
            ),
            NAPI_OK
        );
        let Ok(Value::Array(names)) = value_ref(names_value) else {
            panic!("array keys were not returned as an array");
        };
        assert!(matches!(
            names[0].and_then(|value| value_ref(value).ok()),
            Some(Value::Number(0.0))
        ));
        assert!(matches!(
            names[1].and_then(|value| value_ref(value).ok()),
            Some(Value::Number(1.0))
        ));
    }
}

#[test]
fn property_names_follow_javascript_key_order() {
    unsafe {
        let mut env = Env::new();
        let env_ptr: NapiEnv = &mut env;
        let mut object = ptr::null_mut();
        assert_eq!(napi_create_object(env_ptr, &mut object), NAPI_OK);
        let value = env.alloc(Value::Number(1.0));
        assert_eq!(
            napi_set_named_property(env_ptr, object, c"beta".as_ptr(), value),
            NAPI_OK
        );
        assert_eq!(napi_set_element(env_ptr, object, 10, value), NAPI_OK);
        assert_eq!(
            napi_set_named_property(env_ptr, object, c"alpha".as_ptr(), value),
            NAPI_OK
        );
        assert_eq!(napi_set_element(env_ptr, object, 2, value), NAPI_OK);
        let mut symbol = ptr::null_mut();
        assert_eq!(
            napi_create_symbol(env_ptr, ptr::null_mut(), &mut symbol),
            NAPI_OK
        );
        assert_eq!(napi_set_property(env_ptr, object, symbol, value), NAPI_OK);

        let mut deleted = false;
        assert_eq!(
            napi_delete_property(
                env_ptr,
                object,
                env.alloc(Value::String("beta".into())),
                &mut deleted
            ),
            NAPI_OK
        );
        assert!(deleted);
        assert_eq!(
            napi_set_named_property(env_ptr, object, c"beta".as_ptr(), value),
            NAPI_OK
        );

        let mut names_value = ptr::null_mut();
        assert_eq!(
            napi_get_all_property_names(
                env_ptr,
                object,
                NAPI_KEY_OWN_ONLY,
                NAPI_KEY_ALL_PROPERTIES,
                NAPI_KEY_KEEP_NUMBERS,
                &mut names_value,
            ),
            NAPI_OK
        );
        let Ok(Value::Array(names)) = value_ref(names_value) else {
            panic!("property names were not returned as an array");
        };
        assert_eq!(names.len(), 5);
        assert!(matches!(
            value_ref(names[0].unwrap()),
            Ok(Value::Number(2.0))
        ));
        assert!(matches!(
            value_ref(names[1].unwrap()),
            Ok(Value::Number(10.0))
        ));
        assert!(matches!(value_ref(names[2].unwrap()), Ok(Value::String(name)) if name == "alpha"));
        assert!(matches!(value_ref(names[3].unwrap()), Ok(Value::String(name)) if name == "beta"));
        assert_eq!(names[4], Some(symbol));
    }
}

#[test]
fn generic_properties_cover_all_javascript_object_kinds() {
    unsafe extern "C" fn return_callback_data(_env: NapiEnv, info: NapiCallbackInfo) -> NapiValue {
        info.as_ref().unwrap().data as NapiValue
    }
    unsafe {
        let mut env = Env::new();
        let env_ptr: NapiEnv = &mut env;
        let objects = [
            env.alloc(Value::Array(Vec::new())),
            env.alloc(Value::Buffer(vec![1, 2])),
            env.alloc(Value::TypedArray {
                array_type: 1,
                length: 0,
                array_buffer: ptr::null_mut(),
                byte_offset: 0,
            }),
            env.alloc(Value::Promise(Rc::new(RefCell::new(PromiseState::Pending)))),
        ];
        let value = env.alloc(Value::Number(42.0));
        let mut symbol = ptr::null_mut();
        assert_eq!(
            napi_create_symbol(env_ptr, ptr::null_mut(), &mut symbol),
            NAPI_OK
        );
        for object in objects {
            assert_eq!(
                napi_set_named_property(env_ptr, object, c"custom".as_ptr(), value),
                NAPI_OK
            );
            assert_eq!(napi_set_property(env_ptr, object, symbol, value), NAPI_OK);
            let mut actual = ptr::null_mut();
            assert_eq!(
                napi_get_named_property(env_ptr, object, c"custom".as_ptr(), &mut actual),
                NAPI_OK
            );
            assert_eq!(actual, value);
            assert_eq!(
                napi_get_property(env_ptr, object, symbol, &mut actual),
                NAPI_OK
            );
            assert_eq!(actual, value);
            let mut present = false;
            assert_eq!(
                napi_has_own_property(env_ptr, object, symbol, &mut present),
                NAPI_OK
            );
            assert!(present);

            let mut names = ptr::null_mut();
            assert_eq!(
                napi_get_all_property_names(
                    env_ptr,
                    object,
                    NAPI_KEY_OWN_ONLY,
                    NAPI_KEY_ALL_PROPERTIES,
                    NAPI_KEY_NUMBERS_TO_STRINGS,
                    &mut names,
                ),
                NAPI_OK
            );
            let Ok(Value::Array(names)) = value_ref(names) else {
                panic!("property names were not returned as an array");
            };
            assert!(names.iter().flatten().any(
                |name| matches!(value_ref(*name), Ok(Value::String(name)) if name == "custom")
            ));

            assert_eq!(napi_object_seal(env_ptr, object), NAPI_OK);
            assert_eq!(
                napi_set_named_property(env_ptr, object, c"newKey".as_ptr(), value),
                NAPI_GENERIC_FAILURE
            );
            let mut deleted = true;
            assert_eq!(
                napi_delete_property(env_ptr, object, symbol, &mut deleted),
                NAPI_OK
            );
            assert!(!deleted);
        }

        let array = objects[0];
        assert_eq!(
            napi_set_named_property(env_ptr, array, c"2".as_ptr(), value),
            NAPI_GENERIC_FAILURE,
            "the sealed array must reject a new numeric element"
        );

        let error = env.alloc(Value::Error("host".into()));
        let descriptor = NapiPropertyDescriptor {
            utf8name: ptr::null(),
            name: symbol,
            method: None,
            getter: Some(return_callback_data),
            setter: None,
            value: ptr::null_mut(),
            attributes: NAPI_ENUMERABLE,
            data: value.cast(),
        };
        assert_eq!(
            napi_define_properties(env_ptr, error, 1, &descriptor),
            NAPI_OK
        );
        let mut actual = ptr::null_mut();
        assert_eq!(
            napi_get_property(env_ptr, error, symbol, &mut actual),
            NAPI_OK
        );
        assert_eq!(actual, value);
    }
}

#[test]
fn property_apis_reject_foreign_objects_keys_and_values() {
    unsafe extern "C" fn return_callback_data(_env: NapiEnv, info: NapiCallbackInfo) -> NapiValue {
        info.as_ref().unwrap().data as NapiValue
    }

    unsafe {
        let mut env = Env::new();
        let env_ptr: NapiEnv = &mut env;
        let object = env.alloc(Value::Object(HashMap::new()));
        let key = env.alloc(Value::String("key".into()));
        let value = env.alloc(Value::Number(1.0));
        let mut foreign_env = Env::new();
        let foreign_object = foreign_env.alloc(Value::Object(HashMap::new()));
        let foreign_key = foreign_env.alloc(Value::String("foreign".into()));
        let foreign_value = foreign_env.alloc(Value::Number(2.0));
        let foreign_array_buffer = foreign_env.alloc(Value::ArrayBuffer {
            bytes: vec![0; 4],
            detached: false,
        });
        let mut value_out = ptr::null_mut();
        let mut present = false;

        assert_eq!(
            napi_set_named_property(env_ptr, foreign_object, c"key".as_ptr(), value),
            NAPI_INVALID_ARG
        );
        assert_eq!(
            napi_set_named_property(env_ptr, object, c"key".as_ptr(), foreign_value),
            NAPI_INVALID_ARG
        );
        assert_eq!(
            napi_get_named_property(env_ptr, foreign_object, c"key".as_ptr(), &mut value_out),
            NAPI_INVALID_ARG
        );
        assert_eq!(
            napi_has_named_property(env_ptr, foreign_object, c"key".as_ptr(), &mut present),
            NAPI_INVALID_ARG
        );
        assert_eq!(
            napi_set_property(env_ptr, object, foreign_key, value),
            NAPI_INVALID_ARG
        );
        assert_eq!(
            napi_set_property(env_ptr, object, key, foreign_value),
            NAPI_INVALID_ARG
        );
        assert_eq!(
            napi_get_property(env_ptr, foreign_object, key, &mut value_out),
            NAPI_INVALID_ARG
        );
        assert_eq!(
            napi_has_property(env_ptr, object, foreign_key, &mut present),
            NAPI_INVALID_ARG
        );
        assert_eq!(
            napi_has_own_property(env_ptr, foreign_object, key, &mut present),
            NAPI_INVALID_ARG
        );
        assert_eq!(
            napi_delete_property(env_ptr, object, foreign_key, &mut present),
            NAPI_INVALID_ARG
        );

        let descriptor = NapiPropertyDescriptor {
            utf8name: ptr::null(),
            name: foreign_key,
            method: None,
            getter: None,
            setter: None,
            value,
            attributes: NAPI_DEFAULT_PROPERTY_ATTRIBUTES,
            data: ptr::null_mut(),
        };
        assert_eq!(
            napi_define_properties(env_ptr, object, 1, &descriptor),
            NAPI_INVALID_ARG
        );
        let descriptor = NapiPropertyDescriptor {
            name: key,
            value: foreign_value,
            ..descriptor
        };
        assert_eq!(
            napi_define_properties(env_ptr, object, 1, &descriptor),
            NAPI_INVALID_ARG
        );
        assert_eq!(
            napi_define_properties(env_ptr, foreign_object, 0, ptr::null()),
            NAPI_INVALID_ARG
        );
        assert_eq!(
            napi_set_element(env_ptr, foreign_object, 0, value),
            NAPI_INVALID_ARG
        );
        assert_eq!(
            napi_set_element(env_ptr, object, 0, foreign_value),
            NAPI_INVALID_ARG
        );
        assert_eq!(
            napi_get_element(env_ptr, foreign_object, 0, &mut value_out),
            NAPI_INVALID_ARG
        );
        assert_eq!(
            napi_has_element(env_ptr, foreign_object, 0, &mut present),
            NAPI_INVALID_ARG
        );
        assert_eq!(
            napi_delete_element(env_ptr, foreign_object, 0, &mut present),
            NAPI_INVALID_ARG
        );
        assert_eq!(
            napi_get_property_names(env_ptr, foreign_object, &mut value_out),
            NAPI_INVALID_ARG
        );
        assert_eq!(
            napi_run_script(env_ptr, foreign_key, &mut value_out),
            NAPI_INVALID_ARG
        );
        assert_eq!(
            node_api_create_buffer_from_arraybuffer(
                env_ptr,
                foreign_array_buffer,
                0,
                1,
                &mut value_out
            ),
            NAPI_INVALID_ARG
        );
        assert_eq!(
            napi_create_error(env_ptr, ptr::null_mut(), foreign_key, &mut value_out),
            NAPI_INVALID_ARG
        );
        assert_eq!(
            napi_fatal_exception(env_ptr, foreign_value),
            NAPI_INVALID_ARG
        );

        let mut function = ptr::null_mut();
        assert_eq!(
            napi_create_function(
                env_ptr,
                c"foreignResult".as_ptr(),
                13,
                Some(return_callback_data),
                foreign_value.cast(),
                &mut function
            ),
            NAPI_OK
        );
        assert_eq!(
            napi_call_function(env_ptr, object, function, 0, ptr::null(), &mut value_out),
            NAPI_INVALID_ARG
        );
        assert_eq!(
            napi_new_instance(env_ptr, function, 0, ptr::null(), &mut value_out),
            NAPI_INVALID_ARG
        );
        let accessor_descriptors = [
            NapiPropertyDescriptor {
                utf8name: c"foreignGetter".as_ptr(),
                name: ptr::null_mut(),
                method: None,
                getter: Some(return_callback_data),
                setter: None,
                value: ptr::null_mut(),
                attributes: NAPI_DEFAULT_PROPERTY_ATTRIBUTES,
                data: foreign_value.cast(),
            },
            NapiPropertyDescriptor {
                utf8name: c"nullGetter".as_ptr(),
                name: ptr::null_mut(),
                method: None,
                getter: Some(return_callback_data),
                setter: None,
                value: ptr::null_mut(),
                attributes: NAPI_DEFAULT_PROPERTY_ATTRIBUTES,
                data: ptr::null_mut(),
            },
        ];
        assert_eq!(
            napi_define_properties(
                env_ptr,
                object,
                accessor_descriptors.len(),
                accessor_descriptors.as_ptr()
            ),
            NAPI_OK
        );
        assert_eq!(
            napi_get_named_property(env_ptr, object, c"foreignGetter".as_ptr(), &mut value_out),
            NAPI_INVALID_ARG
        );
        assert_eq!(
            napi_get_named_property(env_ptr, object, c"nullGetter".as_ptr(), &mut value_out),
            NAPI_OK
        );
        assert!(matches!(value_ref(value_out), Ok(Value::Undefined)));
    }
}

#[test]
fn descriptor_batches_are_validated_before_any_property_or_class_creation() {
    unsafe extern "C" fn constructor(_env: NapiEnv, _info: NapiCallbackInfo) -> NapiValue {
        ptr::null_mut()
    }

    unsafe {
        let mut env = Env::new();
        let env_ptr: NapiEnv = &mut env;
        let object = env.alloc(Value::Object(HashMap::new()));
        let value = env.alloc(Value::Number(1.0));
        let mut foreign_env = Env::new();
        let foreign_key = foreign_env.alloc(Value::String("foreign".into()));
        let descriptors = [
            NapiPropertyDescriptor {
                utf8name: c"first".as_ptr(),
                name: ptr::null_mut(),
                method: None,
                getter: None,
                setter: None,
                value,
                attributes: NAPI_DEFAULT_PROPERTY_ATTRIBUTES,
                data: ptr::null_mut(),
            },
            NapiPropertyDescriptor {
                utf8name: ptr::null(),
                name: foreign_key,
                method: None,
                getter: None,
                setter: None,
                value,
                attributes: NAPI_DEFAULT_PROPERTY_ATTRIBUTES,
                data: ptr::null_mut(),
            },
        ];
        let values_before = env.values.len();
        assert_eq!(
            napi_define_properties(env_ptr, object, descriptors.len(), descriptors.as_ptr()),
            NAPI_INVALID_ARG
        );
        assert_eq!(env.values.len(), values_before);
        let mut present = true;
        assert_eq!(
            napi_has_named_property(env_ptr, object, c"first".as_ptr(), &mut present),
            NAPI_OK
        );
        assert!(!present);

        let mut class = ptr::null_mut();
        assert_eq!(
            napi_define_class(
                env_ptr,
                c"Batch".as_ptr(),
                5,
                Some(constructor),
                ptr::null_mut(),
                descriptors.len(),
                descriptors.as_ptr(),
                &mut class
            ),
            NAPI_INVALID_ARG
        );
        assert!(class.is_null());
        assert_eq!(env.values.len(), values_before);
    }
}

#[test]
fn class_instances_expose_their_prototype() {
    unsafe {
        let mut env = Env::new();
        let env_ptr: NapiEnv = &mut env;
        let mut constructor = ptr::null_mut();
        assert_eq!(
            napi_define_class(
                env_ptr,
                c"Box".as_ptr(),
                3,
                Some(thaw_compiled_callback),
                ptr::null_mut(),
                0,
                ptr::null(),
                &mut constructor,
            ),
            NAPI_OK
        );
        let mut expected = ptr::null_mut();
        assert_eq!(
            napi_get_named_property(env_ptr, constructor, c"prototype".as_ptr(), &mut expected),
            NAPI_OK
        );
        let mut instance = ptr::null_mut();
        assert_eq!(
            napi_new_instance(env_ptr, constructor, 0, ptr::null(), &mut instance),
            NAPI_OK
        );
        let mut actual = ptr::null_mut();
        assert_eq!(napi_get_prototype(env_ptr, instance, &mut actual), NAPI_OK);
        assert_eq!(actual, expected);

        let inherited = env.alloc(Value::Number(7.0));
        assert_eq!(
            napi_set_named_property(env_ptr, expected, c"late".as_ptr(), inherited),
            NAPI_OK
        );
        assert_eq!(
            napi_get_named_property(env_ptr, instance, c"late".as_ptr(), &mut actual),
            NAPI_OK
        );
        assert_eq!(actual, inherited);
        let mut key = ptr::null_mut();
        assert_eq!(
            napi_create_string_utf8(env_ptr, c"late".as_ptr(), 4, &mut key),
            NAPI_OK
        );
        let mut present = true;
        assert_eq!(
            napi_has_own_property(env_ptr, instance, key, &mut present),
            NAPI_OK
        );
        assert!(!present);
        assert_eq!(
            napi_has_property(env_ptr, instance, key, &mut present),
            NAPI_OK
        );
        assert!(present);

        let mut names = ptr::null_mut();
        assert_eq!(
            napi_get_all_property_names(
                env_ptr,
                instance,
                NAPI_KEY_OWN_ONLY,
                NAPI_KEY_ALL_PROPERTIES,
                NAPI_KEY_NUMBERS_TO_STRINGS,
                &mut names,
            ),
            NAPI_OK
        );
        assert!(matches!(value_ref(names), Ok(Value::Array(values)) if values.is_empty()));
        assert_eq!(
            napi_get_all_property_names(
                env_ptr,
                instance,
                NAPI_KEY_INCLUDE_PROTOTYPES,
                NAPI_KEY_ALL_PROPERTIES,
                NAPI_KEY_NUMBERS_TO_STRINGS,
                &mut names,
            ),
            NAPI_OK
        );
        assert!(
            matches!(value_ref(names), Ok(Value::Array(values)) if values.iter().any(
                |value| matches!(value.and_then(|value| value_ref(value).ok()), Some(Value::String(name)) if name == "late")
            ))
        );

        let own = env.alloc(Value::Number(9.0));
        assert_eq!(napi_set_property(env_ptr, instance, key, own), NAPI_OK);
        assert_eq!(
            napi_get_named_property(env_ptr, instance, c"late".as_ptr(), &mut actual),
            NAPI_OK
        );
        assert_eq!(actual, own);
        assert_eq!(
            napi_get_named_property(env_ptr, expected, c"late".as_ptr(), &mut actual),
            NAPI_OK
        );
        assert_eq!(actual, inherited);
        let inherited_element = env.alloc(Value::Number(13.0));
        assert_eq!(
            napi_set_element(env_ptr, expected, 5, inherited_element),
            NAPI_OK
        );
        assert_eq!(
            napi_has_element(env_ptr, instance, 5, &mut present),
            NAPI_OK
        );
        assert!(present);
        assert_eq!(napi_get_element(env_ptr, instance, 5, &mut actual), NAPI_OK);
        assert_eq!(actual, inherited_element);
        assert_eq!(
            napi_delete_element(env_ptr, instance, 5, ptr::null_mut()),
            NAPI_OK
        );
        assert_eq!(
            napi_has_element(env_ptr, instance, 5, &mut present),
            NAPI_OK
        );
        assert!(
            present,
            "deleting an inherited element must not alter its prototype"
        );

        let locked = env.alloc(Value::Number(11.0));
        let locked_descriptor = NapiPropertyDescriptor {
            utf8name: c"locked".as_ptr(),
            name: ptr::null_mut(),
            method: None,
            getter: None,
            setter: None,
            value: locked,
            attributes: 0,
            data: ptr::null_mut(),
        };
        assert_eq!(
            napi_define_properties(env_ptr, expected, 1, &locked_descriptor),
            NAPI_OK
        );
        assert_eq!(
            napi_set_named_property(env_ptr, instance, c"locked".as_ptr(), own),
            NAPI_GENERIC_FAILURE
        );
        assert_eq!(
            napi_get_named_property(env_ptr, instance, c"locked".as_ptr(), &mut actual),
            NAPI_OK
        );
        assert_eq!(actual, locked);

        let mut plain = ptr::null_mut();
        assert_eq!(napi_create_object(env_ptr, &mut plain), NAPI_OK);
        assert_eq!(napi_get_prototype(env_ptr, plain, &mut actual), NAPI_OK);
        assert!(matches!(value_ref(actual), Ok(Value::Undefined)));
    }
}

#[test]
fn experimental_set_prototype_enforces_object_graph_rules() {
    unsafe {
        let mut env = Env::new();
        let mut other_env = Env::new();
        let env_ptr: NapiEnv = &mut env;
        let mut object = ptr::null_mut();
        let mut prototype = ptr::null_mut();
        let mut child = ptr::null_mut();
        assert_eq!(napi_create_object(env_ptr, &mut object), NAPI_OK);
        assert_eq!(napi_create_object(env_ptr, &mut prototype), NAPI_OK);
        assert_eq!(napi_create_object(env_ptr, &mut child), NAPI_OK);

        assert_eq!(node_api_set_prototype(env_ptr, object, prototype), NAPI_OK);
        let mut actual = ptr::null_mut();
        assert_eq!(napi_get_prototype(env_ptr, object, &mut actual), NAPI_OK);
        assert_eq!(actual, prototype);
        assert_eq!(node_api_set_prototype(env_ptr, child, object), NAPI_OK);
        assert_eq!(
            node_api_set_prototype(env_ptr, prototype, child),
            NAPI_GENERIC_FAILURE
        );

        let number = env.alloc(Value::Number(1.0));
        assert_eq!(
            node_api_set_prototype(env_ptr, object, number),
            NAPI_OBJECT_EXPECTED
        );
        let foreign = other_env.alloc(Value::Object(HashMap::new()));
        assert_eq!(
            node_api_set_prototype(env_ptr, object, foreign),
            NAPI_INVALID_ARG
        );

        let null = env.alloc(Value::Null);
        assert_eq!(node_api_set_prototype(env_ptr, object, null), NAPI_OK);
        assert_eq!(napi_object_seal(env_ptr, object), NAPI_OK);
        assert_eq!(node_api_set_prototype(env_ptr, object, null), NAPI_OK);
        assert_eq!(
            node_api_set_prototype(env_ptr, object, prototype),
            NAPI_GENERIC_FAILURE
        );
    }
}

#[test]
fn experimental_create_object_with_properties_preflights_inputs() {
    unsafe {
        let mut env = Env::new();
        let mut other_env = Env::new();
        let env_ptr: NapiEnv = &mut env;
        let prototype = env.alloc(Value::Object(HashMap::new()));
        let first_name = env.alloc(Value::String("first".into()));
        let second_name = env.alloc(Value::String("second".into()));
        let first_value = env.alloc(Value::Number(1.0));
        let second_value = env.alloc(Value::Number(2.0));
        let mut names = [first_name, second_name];
        let mut values = [first_value, second_value];
        let mut object = ptr::null_mut();
        assert_eq!(
            node_api_create_object_with_properties(
                env_ptr,
                prototype,
                names.as_mut_ptr(),
                values.as_mut_ptr(),
                names.len(),
                &mut object,
            ),
            NAPI_OK
        );
        let mut actual = ptr::null_mut();
        assert_eq!(napi_get_prototype(env_ptr, object, &mut actual), NAPI_OK);
        assert_eq!(actual, prototype);
        assert_eq!(
            napi_get_named_property(env_ptr, object, c"first".as_ptr(), &mut actual),
            NAPI_OK
        );
        assert_eq!(actual, first_value);
        assert_eq!(
            napi_get_named_property(env_ptr, object, c"second".as_ptr(), &mut actual),
            NAPI_OK
        );
        assert_eq!(actual, second_value);

        let values_before = env.values.len();
        let foreign = other_env.alloc(Value::Number(3.0));
        values[1] = foreign;
        object = first_value;
        assert_eq!(
            node_api_create_object_with_properties(
                env_ptr,
                prototype,
                names.as_mut_ptr(),
                values.as_mut_ptr(),
                names.len(),
                &mut object,
            ),
            NAPI_INVALID_ARG
        );
        assert_eq!(object, first_value);
        assert_eq!(env.values.len(), values_before);

        let null = env.alloc(Value::Null);
        object = ptr::null_mut();
        assert_eq!(
            node_api_create_object_with_properties(
                env_ptr,
                null,
                ptr::null_mut(),
                ptr::null_mut(),
                0,
                &mut object,
            ),
            NAPI_OK
        );
        assert_eq!(napi_get_prototype(env_ptr, object, &mut actual), NAPI_OK);
        assert_eq!(actual, null);
    }
}

#[test]
fn functions_and_classes_expose_standard_metadata_descriptors() {
    unsafe {
        let mut env = Env::new();
        let env_ptr: NapiEnv = &mut env;
        let mut function = ptr::null_mut();
        assert_eq!(
            napi_create_function(
                env_ptr,
                b"hello-extra".as_ptr().cast(),
                5,
                Some(thaw_compiled_callback),
                ptr::null_mut(),
                &mut function,
            ),
            NAPI_OK
        );
        let mut actual = ptr::null_mut();
        assert_eq!(
            napi_get_named_property(env_ptr, function, c"name".as_ptr(), &mut actual),
            NAPI_OK
        );
        assert!(matches!(value_ref(actual), Ok(Value::String(name)) if name == "hello"));
        assert_eq!(
            napi_get_named_property(env_ptr, function, c"length".as_ptr(), &mut actual),
            NAPI_OK
        );
        assert!(matches!(value_ref(actual), Ok(Value::Number(0.0))));
        let replacement = env.alloc(Value::String("changed".into()));
        assert_eq!(
            napi_set_named_property(env_ptr, function, c"name".as_ptr(), replacement),
            NAPI_GENERIC_FAILURE
        );
        let name_key = env.alloc(Value::String("name".into()));
        let mut deleted = false;
        assert_eq!(
            napi_delete_property(env_ptr, function, name_key, &mut deleted),
            NAPI_OK
        );
        assert!(deleted);

        let mut constructor = ptr::null_mut();
        assert_eq!(
            napi_define_class(
                env_ptr,
                c"Box".as_ptr(),
                3,
                Some(thaw_compiled_callback),
                ptr::null_mut(),
                0,
                ptr::null(),
                &mut constructor,
            ),
            NAPI_OK
        );
        let mut prototype = ptr::null_mut();
        assert_eq!(
            napi_get_named_property(env_ptr, constructor, c"prototype".as_ptr(), &mut prototype,),
            NAPI_OK
        );
        assert_eq!(
            napi_get_named_property(env_ptr, prototype, c"constructor".as_ptr(), &mut actual,),
            NAPI_OK
        );
        assert_eq!(actual, constructor);
        assert_eq!(
            napi_set_named_property(env_ptr, constructor, c"prototype".as_ptr(), prototype,),
            NAPI_GENERIC_FAILURE
        );
        let prototype_key = env.alloc(Value::String("prototype".into()));
        deleted = true;
        assert_eq!(
            napi_delete_property(env_ptr, constructor, prototype_key, &mut deleted),
            NAPI_OK
        );
        assert!(!deleted);
    }
}

#[test]
fn instanceof_walks_constructor_prototype_chains() {
    unsafe {
        let mut env = Env::new();
        let env_ptr: NapiEnv = &mut env;
        let mut base = ptr::null_mut();
        let mut derived = ptr::null_mut();
        for (name, out) in [(c"Base", &mut base), (c"Derived", &mut derived)] {
            assert_eq!(
                napi_define_class(
                    env_ptr,
                    name.as_ptr(),
                    NAPI_AUTO_LENGTH,
                    Some(thaw_compiled_callback),
                    ptr::null_mut(),
                    0,
                    ptr::null(),
                    out,
                ),
                NAPI_OK
            );
        }
        let mut base_prototype = ptr::null_mut();
        let mut derived_prototype = ptr::null_mut();
        assert_eq!(
            napi_get_named_property(env_ptr, base, c"prototype".as_ptr(), &mut base_prototype,),
            NAPI_OK
        );
        assert_eq!(
            napi_get_named_property(
                env_ptr,
                derived,
                c"prototype".as_ptr(),
                &mut derived_prototype,
            ),
            NAPI_OK
        );
        env.prototypes
            .insert(derived_prototype as usize, base_prototype as usize);
        let mut instance = ptr::null_mut();
        assert_eq!(
            napi_new_instance(env_ptr, derived, 0, ptr::null(), &mut instance),
            NAPI_OK
        );
        let mut matches = false;
        assert_eq!(
            napi_instanceof(env_ptr, instance, derived, &mut matches),
            NAPI_OK
        );
        assert!(matches);
        assert_eq!(
            napi_instanceof(env_ptr, instance, base, &mut matches),
            NAPI_OK
        );
        assert!(matches);

        let primitive = env.alloc(Value::Number(1.0));
        assert_eq!(
            napi_instanceof(env_ptr, primitive, base, &mut matches),
            NAPI_OK
        );
        assert!(!matches);
        assert_eq!(
            napi_instanceof(env_ptr, instance, primitive, &mut matches),
            NAPI_FUNCTION_EXPECTED
        );
        let mut plain = ptr::null_mut();
        assert_eq!(napi_create_object(env_ptr, &mut plain), NAPI_OK);
        assert_eq!(napi_instanceof(env_ptr, plain, base, &mut matches), NAPI_OK);
        assert!(!matches);
    }
}

#[test]
fn object_type_tags_are_stable_and_unique() {
    unsafe {
        let mut env = Env::new();
        let env_ptr: NapiEnv = &mut env;
        let mut object = ptr::null_mut();
        assert_eq!(napi_create_object(env_ptr, &mut object), NAPI_OK);
        let tag = NapiTypeTag {
            lower: 0x0123_4567_89ab_cdef,
            upper: 0xfedc_ba98_7654_3210,
        };
        let other = NapiTypeTag {
            lower: tag.lower,
            upper: tag.upper ^ 1,
        };
        assert_eq!(napi_type_tag_object(env_ptr, object, &tag), NAPI_OK);
        let mut matches = false;
        assert_eq!(
            napi_check_object_type_tag(env_ptr, object, &tag, &mut matches),
            NAPI_OK
        );
        assert!(matches);
        assert_eq!(
            napi_check_object_type_tag(env_ptr, object, &other, &mut matches),
            NAPI_OK
        );
        assert!(!matches);
        assert_eq!(
            napi_type_tag_object(env_ptr, object, &other),
            NAPI_INVALID_ARG
        );
        let mut info = ptr::null();
        assert_eq!(napi_get_last_error_info(env_ptr, &mut info), NAPI_OK);
        assert_eq!((*info).error_code, NAPI_INVALID_ARG);

        let number = env.alloc(Value::Number(1.0));
        assert_eq!(
            napi_type_tag_object(env_ptr, number, &tag),
            NAPI_OBJECT_EXPECTED
        );
        assert_eq!(napi_get_last_error_info(env_ptr, &mut info), NAPI_OK);
        assert_eq!((*info).error_code, NAPI_OBJECT_EXPECTED);
        assert_eq!(
            napi_check_object_type_tag(env_ptr, number, &tag, &mut matches),
            NAPI_OBJECT_EXPECTED
        );
        let mut foreign_env = Env::new();
        let foreign = foreign_env.alloc(Value::Object(HashMap::new()));
        assert_eq!(
            napi_type_tag_object(env_ptr, foreign, &tag),
            NAPI_INVALID_ARG
        );
        assert_eq!(
            napi_check_object_type_tag(env_ptr, foreign, &tag, &mut matches),
            NAPI_INVALID_ARG
        );
    }
}

fn lock_async_test() -> std::sync::MutexGuard<'static, ()> {
    ASYNC_TEST_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

