#[cfg(feature = "quickjs")]
#[test]
fn callback_native_handle_paths_keep_user_marker_objects_ordinary() {
    let mut env = Box::new(Env::new());
    let native = env.alloc(Value::Object(HashMap::new()));
    env.instances.insert(native as usize, 1);
    let marker = env.alloc(Value::String("7".into()));
    let ordinary = env.alloc(Value::Object(HashMap::from([(
        PropertyKey::String("__thaw_napi_handle__".into()), marker,
    )])));
    let nested = env.alloc(Value::Array(vec![Some(native)]));
    let path_object = env.alloc(Value::Object(HashMap::from([(
        PropertyKey::String("__proto__".into()), native,
    )])));
    let env_ptr = (&mut *env) as *mut Env;
    HOST.with(|host| host.borrow_mut().module_envs.push(env));
    let paths = unsafe {
        callback_native_handle_paths(&[native, nested, ordinary, path_object]).unwrap()
    };
    assert_eq!(paths, vec![
        serde_json::json!([[0], (native as u64).to_string()]),
        serde_json::json!([[1, 0], (native as u64).to_string()]),
        serde_json::json!([[3, "__proto__"], (native as u64).to_string()]),
    ]);
    HOST.with(|host| {
        host.borrow_mut().module_envs.retain(|entry| (&**entry as *const Env).cast_mut() != env_ptr);
    });
}

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
        let numeric_key = env.alloc(Value::Number(2.0));
        assert_eq!(
            napi_set_property(env_ptr, object, numeric_key, second_value),
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
        let json = json_from_value_with_undefined_for_env(env_ptr, object, false).unwrap();
        assert_eq!(json, serde_json::json!({"2": 3.0, "same": 1.0}));
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
fn global_date_regexp_and_symbol_are_available() {
    unsafe {
        let mut env = Env::new();
        let env_ptr: NapiEnv = &mut env;
        let mut global = ptr::null_mut();
        assert_eq!(napi_get_global(env_ptr, &mut global), NAPI_OK);
        for name in [c"Date", c"RegExp"] {
            let mut constructor = ptr::null_mut();
            assert_eq!(
                napi_get_named_property(env_ptr, global, name.as_ptr(), &mut constructor),
                NAPI_OK
            );
            assert!(matches!(value_ref(constructor), Ok(Value::Function(_))));
        }
        let mut symbol = ptr::null_mut();
        let mut iterator = ptr::null_mut();
        assert_eq!(
            napi_get_named_property(env_ptr, global, c"Symbol".as_ptr(), &mut symbol),
            NAPI_OK
        );
        assert!(matches!(value_ref(symbol), Ok(Value::Function(_))));
        assert_eq!(
            napi_get_named_property(env_ptr, symbol, c"iterator".as_ptr(), &mut iterator),
            NAPI_OK
        );
        assert!(matches!(value_ref(iterator), Ok(Value::Symbol { .. })));
        let mut date = ptr::null_mut();
        let mut date_constructor = ptr::null_mut();
        let mut matches = false;
        assert_eq!(napi_create_date(env_ptr, 42.0, &mut date), NAPI_OK);
        assert_eq!(
            napi_get_named_property(
                env_ptr,
                global,
                c"Date".as_ptr(),
                &mut date_constructor,
            ),
            NAPI_OK
        );
        assert_eq!(
            napi_instanceof(env_ptr, date, date_constructor, &mut matches),
            NAPI_OK
        );
        assert!(matches);
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

#[test]
fn handle_scope_retires_only_unreachable_values_and_never_reuses_raw_tokens() {
    unsafe {
        let mut env = Env::new();
        let env_ptr: NapiEnv = &mut env;
        let mut outer = ptr::null_mut();
        let mut inner = ptr::null_mut();
        assert_eq!(napi_open_handle_scope(env_ptr, &mut outer), NAPI_OK);
        let parent = env.alloc(Value::Object(HashMap::new()));
        assert_eq!(napi_open_escapable_handle_scope(env_ptr, &mut inner), NAPI_OK);
        let abandoned = env.alloc(Value::String("temporary".into()));
        let child = env.alloc(Value::Number(7.0));
        let escaped = env.alloc(Value::Object(HashMap::from([(
            PropertyKey::String("child".into()), child,
        )])));
        let mut result = ptr::null_mut();
        assert_eq!(napi_escape_handle(env_ptr, inner, escaped, &mut result), NAPI_OK);
        assert_eq!(napi_close_escapable_handle_scope(env_ptr, inner), NAPI_OK);
        assert!(!env.values.contains(&abandoned));
        assert_eq!(value_ref(abandoned).err(), Some(NAPI_INVALID_ARG));
        assert_eq!(result, escaped);
        assert!(env.values.contains(&child));
        assert!(env.values.contains(&escaped));
        let later = env.alloc(Value::String("later".into()));
        assert_ne!(later, abandoned);
        assert!(env.values.contains(&parent));
        assert_eq!(napi_close_handle_scope(env_ptr, outer), NAPI_OK);
        assert!(!env.values.contains(&parent));
        assert!(!env.values.contains(&escaped));
        assert!(!env.values.contains(&child));
        assert!(!env.values.contains(&later));
    }
}

#[test]
fn handle_scope_keeps_referenced_and_object_reachable_children() {
    unsafe {
        let mut env = Env::new();
        let env_ptr: NapiEnv = &mut env;
        let owner = env.alloc(Value::Object(HashMap::new()));
        let mut scope = ptr::null_mut();
        assert_eq!(napi_open_handle_scope(env_ptr, &mut scope), NAPI_OK);
        let child = env.alloc(Value::Number(9.0));
        let referenced = env.alloc(Value::String("held".into()));
        if let Value::Object(fields) = &mut *owner {
            fields.insert(PropertyKey::String("child".into()), child);
        }
        let mut reference = ptr::null_mut();
        assert_eq!(napi_create_reference(env_ptr, referenced, 1, &mut reference), NAPI_OK);
        assert_eq!(napi_close_handle_scope(env_ptr, scope), NAPI_OK);
        assert!(env.values.contains(&child));
        assert!(env.values.contains(&referenced));
        assert!(matches!(value_ref(child), Ok(Value::Number(9.0))));
        assert_eq!(napi_delete_reference(env_ptr, reference), NAPI_OK);
        assert!(!env.values.contains(&referenced));
        assert_eq!(value_ref(referenced).err(), Some(NAPI_INVALID_ARG));
    }
}

// Unrun: a weak reference must not resurrect a scoped value after collection.
// The failed ref leaves both the count and the caller's output unchanged.
#[test]
fn collected_weak_reference_cannot_be_strengthened() {
    unsafe {
        let mut env = Env::new();
        let env_ptr: NapiEnv = &mut env;
        let mut scope = ptr::null_mut();
        assert_eq!(napi_open_handle_scope(env_ptr, &mut scope), NAPI_OK);
        let value = env.alloc(Value::Object(HashMap::new()));
        let mut reference = ptr::null_mut();
        assert_eq!(napi_create_reference(env_ptr, value, 0, &mut reference), NAPI_OK);
        assert_eq!(napi_close_handle_scope(env_ptr, scope), NAPI_OK);
        assert!(!env.values.contains(&value));
        let mut referenced_value = value;
        assert_eq!(napi_get_reference_value(env_ptr, reference, &mut referenced_value), NAPI_OK);
        assert!(referenced_value.is_null());
        let mut count = 77;
        assert_eq!(napi_reference_ref(env_ptr, reference, &mut count), NAPI_GENERIC_FAILURE);
        assert_eq!(count, 77);
        assert_eq!((*reference).count, 0);
        assert_eq!(napi_delete_reference(env_ptr, reference), NAPI_OK);
    }
}

// Unrun: local Symbol metadata is weak, but a live property key holds the
// Symbol through the same graph edge as an object field value.
#[test]
fn unreferenced_local_symbol_is_collected_with_its_scope() {
    unsafe {
        let mut env = Env::new();
        let env_ptr: NapiEnv = &mut env;
        let mut scope = ptr::null_mut();
        assert_eq!(napi_open_handle_scope(env_ptr, &mut scope), NAPI_OK);
        let mut symbol = ptr::null_mut();
        assert_eq!(napi_create_symbol(env_ptr, ptr::null_mut(), &mut symbol), NAPI_OK);
        let Value::Symbol { id, .. } = value_ref(symbol).unwrap() else { panic!("Symbol expected") };
        let id = *id;
        let mut reference = ptr::null_mut();
        assert_eq!(napi_create_reference(env_ptr, symbol, 0, &mut reference), NAPI_OK);
        assert_eq!(napi_close_handle_scope(env_ptr, scope), NAPI_OK);
        let mut actual = symbol;
        assert_eq!(napi_get_reference_value(env_ptr, reference, &mut actual), NAPI_OK);
        assert!(actual.is_null());
        assert!(!env.symbols.contains_key(&id));
        assert_eq!(napi_reference_ref(env_ptr, reference, ptr::null_mut()), NAPI_GENERIC_FAILURE);
        assert_eq!(napi_delete_reference(env_ptr, reference), NAPI_OK);
    }
}

// Unrun: a live property key retains its otherwise weak local Symbol.
#[test]
fn local_symbol_weak_reference_follows_property_key_reachability() {
    unsafe {
        let mut env = Env::new();
        let env_ptr: NapiEnv = &mut env;
        let object = env.alloc(Value::Object(HashMap::new()));
        let mut scope = ptr::null_mut();
        assert_eq!(napi_open_handle_scope(env_ptr, &mut scope), NAPI_OK);
        let mut symbol = ptr::null_mut();
        assert_eq!(napi_create_symbol(env_ptr, ptr::null_mut(), &mut symbol), NAPI_OK);
        let value = env.alloc(Value::Number(3.0));
        assert_eq!(napi_set_property(env_ptr, object, symbol, value), NAPI_OK);
        let mut reference = ptr::null_mut();
        assert_eq!(napi_create_reference(env_ptr, symbol, 0, &mut reference), NAPI_OK);
        assert_eq!(napi_close_handle_scope(env_ptr, scope), NAPI_OK);
        let mut recipient = ptr::null_mut();
        assert_eq!(napi_open_handle_scope(env_ptr, &mut recipient), NAPI_OK);
        let mut actual = ptr::null_mut();
        assert_eq!(napi_get_reference_value(env_ptr, reference, &mut actual), NAPI_OK);
        assert_eq!(actual, symbol);
        let mut deleted = false;
        assert_eq!(napi_delete_property(env_ptr, object, actual, &mut deleted), NAPI_OK);
        assert!(deleted);
        assert!(matches!(value_ref(actual), Ok(Value::Symbol { .. })));
        assert_eq!(napi_close_handle_scope(env_ptr, recipient), NAPI_OK);
        assert_eq!(napi_get_reference_value(env_ptr, reference, &mut actual), NAPI_OK);
        assert!(actual.is_null());
        assert_eq!(napi_reference_ref(env_ptr, reference, ptr::null_mut()), NAPI_GENERIC_FAILURE);
        assert_eq!(napi_delete_reference(env_ptr, reference), NAPI_OK);
    }
}

// Unrun: returning an old child in a new handle scope grants that scope a
// handle. Removing the parent edge must not retire it until the recipient
// scope closes, even for an ordinary (non-Symbol) object.
#[test]
fn property_get_roots_existing_child_in_recipient_scope() {
    unsafe {
        let mut env = Env::new();
        let env_ptr: NapiEnv = &mut env;
        let parent = env.alloc(Value::Object(HashMap::new()));
        let mut creator = ptr::null_mut();
        assert_eq!(napi_open_handle_scope(env_ptr, &mut creator), NAPI_OK);
        let child = env.alloc(Value::Object(HashMap::new()));
        assert_eq!(napi_set_named_property(env_ptr, parent, c"child".as_ptr(), child), NAPI_OK);
        assert_eq!(napi_close_handle_scope(env_ptr, creator), NAPI_OK);
        let mut recipient = ptr::null_mut();
        assert_eq!(napi_open_handle_scope(env_ptr, &mut recipient), NAPI_OK);
        let mut returned = ptr::null_mut();
        assert_eq!(napi_get_named_property(env_ptr, parent, c"child".as_ptr(), &mut returned), NAPI_OK);
        assert_eq!(returned, child);
        let key = env.alloc(Value::String("child".into()));
        let mut deleted = false;
        assert_eq!(napi_delete_property(env_ptr, parent, key, &mut deleted), NAPI_OK);
        assert!(deleted);
        assert!(matches!(value_ref(returned), Ok(Value::Object(_))));
        assert_eq!(napi_close_handle_scope(env_ptr, recipient), NAPI_OK);
        assert!(value_ref(returned).is_err());
    }
}

// Unrun: a native callback can return a value allocated in an earlier closed
// scope. Both call_function and make_callback must grant the caller's scope
// a handle before the last object edge is removed.
#[test]
fn callback_result_roots_existing_value_in_recipient_scope() {
    unsafe extern "C" fn return_data(_: NapiEnv, info: NapiCallbackInfo) -> NapiValue {
        (*info).data as NapiValue
    }
    unsafe {
        let mut env = Env::new();
        let env_ptr: NapiEnv = &mut env;
        let parent = env.alloc(Value::Object(HashMap::new()));
        let mut creator = ptr::null_mut();
        assert_eq!(napi_open_handle_scope(env_ptr, &mut creator), NAPI_OK);
        let child = env.alloc(Value::Object(HashMap::new()));
        assert_eq!(napi_set_named_property(env_ptr, parent, c"child".as_ptr(), child), NAPI_OK);
        assert_eq!(napi_close_handle_scope(env_ptr, creator), NAPI_OK);
        let mut function = ptr::null_mut();
        assert_eq!(napi_create_function(env_ptr, c"returnData".as_ptr(), NAPI_AUTO_LENGTH,
            Some(return_data), child.cast(), &mut function), NAPI_OK);
        let mut recipient = ptr::null_mut();
        assert_eq!(napi_open_handle_scope(env_ptr, &mut recipient), NAPI_OK);
        let mut first = ptr::null_mut();
        let mut second = ptr::null_mut();
        assert_eq!(napi_call_function(env_ptr, parent, function, 0, ptr::null(), &mut first), NAPI_OK);
        assert_eq!(napi_make_callback(env_ptr, ptr::null_mut(), parent, function, 0,
            ptr::null(), &mut second), NAPI_OK);
        assert_eq!((first, second), (child, child));
        let key = env.alloc(Value::String("child".into()));
        let mut deleted = false;
        assert_eq!(napi_delete_property(env_ptr, parent, key, &mut deleted), NAPI_OK);
        assert!(deleted);
        assert!(matches!(value_ref(first), Ok(Value::Object(_))));
        assert_eq!(napi_close_handle_scope(env_ptr, recipient), NAPI_OK);
        assert!(value_ref(child).is_err());
    }
}

// Unrun: clearing the exception transfers its last Env root to the caller's
// scope. A sweep before that scope closes must leave the handle usable.
#[test]
fn cleared_exception_roots_returned_handle_before_removing_exception_root() {
    unsafe {
        let mut env = Env::new();
        let env_ptr: NapiEnv = &mut env;
        let mut creator = ptr::null_mut();
        assert_eq!(napi_open_handle_scope(env_ptr, &mut creator), NAPI_OK);
        let value = env.alloc(Value::Object(HashMap::new()));
        env.exception = Some(value);
        assert_eq!(napi_close_handle_scope(env_ptr, creator), NAPI_OK);
        let mut recipient = ptr::null_mut();
        assert_eq!(napi_open_handle_scope(env_ptr, &mut recipient), NAPI_OK);
        let mut actual = ptr::null_mut();
        assert_eq!(napi_get_and_clear_last_exception(env_ptr, &mut actual), NAPI_OK);
        assert_eq!(actual, value);
        assert!(env.exception.is_none());
        sweep_pending_scope_values(env_ptr);
        assert!(matches!(value_ref(actual), Ok(Value::Object(_))));
        assert_eq!(napi_close_handle_scope(env_ptr, recipient), NAPI_OK);
        assert!(value_ref(actual).is_err());
    }
}

// Unrun: the identity-preserving coercion fast paths also return a new
// handle to their caller; they cannot rely on the creating scope.
#[test]
fn identity_coercions_root_existing_recipient_values() {
    unsafe {
        for string_value in [false, true] {
            let mut env = Env::new();
            let env_ptr: NapiEnv = &mut env;
            let parent = env.alloc(Value::Object(HashMap::new()));
            let mut creator = ptr::null_mut();
            assert_eq!(napi_open_handle_scope(env_ptr, &mut creator), NAPI_OK);
            let child = if string_value { env.alloc(Value::String("held".into())) }
                else { env.alloc(Value::Object(HashMap::new())) };
            assert_eq!(napi_set_named_property(env_ptr, parent, c"child".as_ptr(), child), NAPI_OK);
            assert_eq!(napi_close_handle_scope(env_ptr, creator), NAPI_OK);
            let mut recipient = ptr::null_mut();
            assert_eq!(napi_open_handle_scope(env_ptr, &mut recipient), NAPI_OK);
            let mut actual = ptr::null_mut();
            let status = if string_value { napi_coerce_to_string(env_ptr, child, &mut actual) }
                else { napi_coerce_to_object(env_ptr, child, &mut actual) };
            assert_eq!(status, NAPI_OK);
            assert_eq!(actual, child);
            let key = env.alloc(Value::String("child".into()));
            let mut deleted = false;
            assert_eq!(napi_delete_property(env_ptr, parent, key, &mut deleted), NAPI_OK);
            assert!(deleted);
            assert!(value_ref(actual).is_ok());
            assert_eq!(napi_close_handle_scope(env_ptr, recipient), NAPI_OK);
            assert!(value_ref(actual).is_err());
        }
    }
}

// Unrun: a later callback-info argument may reenter through Symbol pinning
// and close the scope prepared for an earlier argument. The batch must fail
// before writing any of its output pointers.
#[test]
fn callback_info_batch_revalidates_earlier_scope_roots() {
    unsafe {
        let mut env = Env::new();
        let env_ptr: NapiEnv = &mut env;
        let parent = env.alloc(Value::Object(HashMap::new()));
        let child = env.alloc(Value::Object(HashMap::new()));
        assert_eq!(napi_set_named_property(env_ptr, parent, c"child".as_ptr(), child), NAPI_OK);
        let mut first_scope = ptr::null_mut();
        assert_eq!(napi_open_handle_scope(env_ptr, &mut first_scope), NAPI_OK);
        let first_root = root_existing_value(env_ptr, child).expect("first recipient root");
        let generation = env.value_generations[&(child as usize)];
        assert!(prepared_scoped_values_still_rooted(env_ptr,
            Some(first_scope.cast()), &[(child, generation)]));
        assert_eq!(napi_close_handle_scope(env_ptr, first_scope), NAPI_OK);
        let mut next_scope = ptr::null_mut();
        assert_eq!(napi_open_handle_scope(env_ptr, &mut next_scope), NAPI_OK);
        assert!(!prepared_scoped_values_still_rooted(env_ptr,
            Some(first_scope.cast()), &[(child, generation)]));
        rollback_existing_value_root(env_ptr, first_root);
        assert!(matches!(value_ref(child), Ok(Value::Object(_))));
        assert_eq!(napi_close_handle_scope(env_ptr, next_scope), NAPI_OK);
    }
}

// Unrun: Function owns a separate property map; Array properties live in the
// hosted side table. Both key edges must retain a local Symbol until delete.
#[test]
fn local_symbol_key_survives_function_and_array_property_maps() {
    unsafe {
        for function_owner in [true, false] {
            let mut env = Env::new();
            let env_ptr: NapiEnv = &mut env;
            let mut owner = ptr::null_mut();
            let status = if function_owner {
                napi_create_function(env_ptr, c"owner".as_ptr(), NAPI_AUTO_LENGTH,
                    Some(thaw_compiled_callback), ptr::null_mut(), &mut owner)
            } else {
                napi_create_array(env_ptr, &mut owner)
            };
            assert_eq!(status, NAPI_OK);
            let mut scope = ptr::null_mut();
            assert_eq!(napi_open_handle_scope(env_ptr, &mut scope), NAPI_OK);
            let mut symbol = ptr::null_mut();
            assert_eq!(napi_create_symbol(env_ptr, ptr::null_mut(), &mut symbol), NAPI_OK);
            let value = env.alloc(Value::Number(1.0));
            assert_eq!(napi_set_property(env_ptr, owner, symbol, value), NAPI_OK);
            let mut reference = ptr::null_mut();
            assert_eq!(napi_create_reference(env_ptr, symbol, 0, &mut reference), NAPI_OK);
            assert_eq!(napi_close_handle_scope(env_ptr, scope), NAPI_OK);
            let mut recipient = ptr::null_mut();
            assert_eq!(napi_open_handle_scope(env_ptr, &mut recipient), NAPI_OK);
            let mut actual = ptr::null_mut();
            assert_eq!(napi_get_reference_value(env_ptr, reference, &mut actual), NAPI_OK);
            assert_eq!(actual, symbol);
            let mut deleted = false;
            assert_eq!(napi_delete_property(env_ptr, owner, actual, &mut deleted), NAPI_OK);
            assert!(deleted);
            assert!(matches!(value_ref(actual), Ok(Value::Symbol { .. })));
            assert_eq!(napi_close_handle_scope(env_ptr, recipient), NAPI_OK);
            assert_eq!(napi_get_reference_value(env_ptr, reference, &mut actual), NAPI_OK);
            assert!(actual.is_null());
            assert_eq!(napi_delete_reference(env_ptr, reference), NAPI_OK);
        }
    }
}

// Unrun: Symbol.for's registry is a strong root, unlike a local Symbol.
#[test]
fn registered_symbol_survives_a_zero_count_reference_and_scope_close() {
    unsafe {
        let mut env = Env::new();
        let env_ptr: NapiEnv = &mut env;
        let mut scope = ptr::null_mut();
        assert_eq!(napi_open_handle_scope(env_ptr, &mut scope), NAPI_OK);
        let mut symbol = ptr::null_mut();
        assert_eq!(node_api_symbol_for(env_ptr, c"scope-registered".as_ptr(),
            NAPI_AUTO_LENGTH, &mut symbol), NAPI_OK);
        let mut reference = ptr::null_mut();
        assert_eq!(napi_create_reference(env_ptr, symbol, 0, &mut reference), NAPI_OK);
        assert_eq!(napi_close_handle_scope(env_ptr, scope), NAPI_OK);
        let mut actual = ptr::null_mut();
        assert_eq!(napi_get_reference_value(env_ptr, reference, &mut actual), NAPI_OK);
        assert_eq!(actual, symbol);
        assert_eq!(napi_reference_ref(env_ptr, reference, ptr::null_mut()), NAPI_OK);
        assert_eq!(napi_delete_reference(env_ptr, reference), NAPI_OK);
    }
}

// Unrun: absence of a trusted JS owner is an unknown weak-target state. It
// must not be turned into a collected/null result or a successful 0->1 ref.
#[cfg(feature = "quickjs")]
#[test]
fn js_origin_symbol_weak_query_failure_is_not_collection_proof() {
    unsafe {
        let mut env = Env::new();
        env.graph_owner_id = next_graph_owner_id().unwrap();
        let env_ptr: NapiEnv = &mut env;
        let mut symbol = ptr::null_mut();
        assert_eq!(napi_create_symbol(env_ptr, ptr::null_mut(), &mut symbol), NAPI_OK);
        let Value::Symbol { id, .. } = value_ref(symbol).unwrap() else { panic!("Symbol expected") };
        env.js_origin_symbol_ids.insert(*id);
        env.js_live_symbol_ids.insert(*id);
        let mut reference = ptr::null_mut();
        assert_eq!(napi_create_reference(env_ptr, symbol, 0, &mut reference), NAPI_OK);
        let mut output = symbol;
        assert_eq!(napi_get_reference_value(env_ptr, reference, &mut output), NAPI_GENERIC_FAILURE);
        assert_eq!(output, symbol);
        assert_eq!((*reference).value, symbol);
        let mut count = 73;
        assert_eq!(napi_reference_ref(env_ptr, reference, &mut count), NAPI_GENERIC_FAILURE);
        assert_eq!(count, 73);
        assert_eq!((*reference).count, 0);
        assert_eq!(napi_delete_reference(env_ptr, reference), NAPI_OK);
    }
}

#[test]
fn deferred_state_keeps_promise_alive_after_creating_scope_closes() {
    unsafe {
        let mut env = Env::new();
        let env_ptr: NapiEnv = &mut env;
        let mut scope = ptr::null_mut();
        assert_eq!(napi_open_handle_scope(env_ptr, &mut scope), NAPI_OK);
        let mut deferred = ptr::null_mut();
        let mut promise = ptr::null_mut();
        assert_eq!(napi_create_promise(env_ptr, &mut deferred, &mut promise), NAPI_OK);
        assert_eq!(napi_close_handle_scope(env_ptr, scope), NAPI_OK);
        assert!(env.values.contains(&promise));
        assert!(matches!(value_ref(promise), Ok(Value::Promise(_))));
    }
}

#[test]
fn scoped_unexposed_instance_and_unrooted_wrap_both_retire() {
    unsafe {
        let mut env = Env::new();
        let env_ptr: NapiEnv = &mut env;
        let mut scope = ptr::null_mut();
        assert_eq!(napi_open_handle_scope(env_ptr, &mut scope), NAPI_OK);
        let native = env.alloc(Value::Object(HashMap::new()));
        env.instances.insert(native as usize, 0);
        let wrapped = env.alloc(Value::Object(HashMap::new()));
        env.wraps.insert(wrapped as usize, WrapRecord {
            data: ptr::null_mut(), finalize: None,
            hint: ptr::null_mut(),
        });
        let temporary = env.alloc(Value::Number(1.0));
        assert_eq!(napi_close_handle_scope(env_ptr, scope), NAPI_OK);
        assert!(!env.values.contains(&native));
        assert!(!env.instances.contains_key(&(native as usize)));
        assert!(!env.values.contains(&wrapped));
        assert!(!env.wraps.contains_key(&(wrapped as usize)));
        assert!(!env.values.contains(&temporary));
    }
}

#[test]
fn scoped_graph_metadata_does_not_pin_an_unreachable_value() {
    unsafe {
        let mut env = Env::new();
        let env_ptr: NapiEnv = &mut env;
        let mut scope = ptr::null_mut();
        assert_eq!(napi_open_handle_scope(env_ptr, &mut scope), NAPI_OK);
        let wrapper = env.alloc(Value::Object(HashMap::new()));
        let child = env.alloc(Value::Number(3.0));
        env.graph_wrappers.insert(wrapper as usize, NapiGraphWrapperKind::Map);
        env.host_properties.insert(wrapper as usize, HashMap::from([(
            PropertyKey::String("child".into()), child,
        )]));
        assert_eq!(napi_close_handle_scope(env_ptr, scope), NAPI_OK);
        assert!(!env.values.contains(&wrapper));
        assert!(!env.values.contains(&child));
        assert!(!env.graph_wrappers.contains_key(&(wrapper as usize)));
        assert!(!env.host_properties.contains_key(&(wrapper as usize)));
    }
}

#[test]
fn scoped_external_without_finalizer_retires_with_its_backing() {
    unsafe {
        let mut env = Env::new();
        let env_ptr: NapiEnv = &mut env;
        let mut scope = ptr::null_mut();
        assert_eq!(napi_open_handle_scope(env_ptr, &mut scope), NAPI_OK);
        let temporary = env.alloc(Value::External(ptr::null_mut()));
        let backing = env.alloc(Value::External(ptr::null_mut()));
        env.finalizers.push(FinalizeRecord {
            data: ptr::null_mut(), finalize: None, hint: ptr::null_mut(),
            backing: backing.cast(),
        });
        assert_eq!(napi_close_handle_scope(env_ptr, scope), NAPI_OK);
        assert!(!env.values.contains(&temporary));
        assert!(!env.values.contains(&backing));
        assert!(env.finalizers.is_empty());
    }
}

#[test]
fn scoped_accessor_metadata_retires_with_unreachable_owner() {
    unsafe {
        let mut env = Env::new();
        let env_ptr: NapiEnv = &mut env;
        let mut scope = ptr::null_mut();
        assert_eq!(napi_open_handle_scope(env_ptr, &mut scope), NAPI_OK);
        let owner = env.alloc(Value::Object(HashMap::new()));
        let getter = env.alloc(Value::Number(8.0));
        env.accessors.insert((owner as usize, PropertyKey::String("x".into())), Accessor {
            getter: None, setter: None,
            getter_data: ptr::null_mut(), setter_data: ptr::null_mut(),
            getter_reflection: Some(getter), setter_reflection: None,
            #[cfg(feature = "quickjs")]
            js_owner: None,
        });
        assert_eq!(napi_close_handle_scope(env_ptr, scope), NAPI_OK);
        assert!(!env.values.contains(&owner));
        assert!(!env.values.contains(&getter));
        assert!(env.accessors.is_empty());
    }
}

#[test]
fn root_escape_survives_host_borrow_contention_and_later_sweep() {
    unsafe {
        let mut env = Env::new();
        env.host_managed = true;
        let env_ptr: NapiEnv = &mut env;
        let mut scope = ptr::null_mut();
        assert_eq!(napi_open_escapable_handle_scope(env_ptr, &mut scope), NAPI_OK);
        let escaped = env.alloc(Value::Number(19.0));
        let mut result = ptr::null_mut();
        assert_eq!(napi_escape_handle(env_ptr, scope, escaped, &mut result), NAPI_OK);
        HOST.with(|host| {
            let _occupied = host.borrow_mut();
            assert_eq!(napi_close_escapable_handle_scope(env_ptr, scope), NAPI_OK);
        });
        assert_eq!(result, escaped);
        assert!(env.pending_scope_values.iter().any(|(value, _)| *value == escaped));
        sweep_pending_scope_values(env_ptr);
        assert!(env.values.contains(&escaped));
        assert!(matches!(value_ref(escaped), Ok(Value::Number(19.0))));
    }
}

std::thread_local! {
    static SCOPED_FINALIZER_CALLS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

unsafe extern "C" fn scoped_finalizer_creates_new_value(
    env: NapiEnv, _: *mut c_void, _: *mut c_void,
) {
    SCOPED_FINALIZER_CALLS.with(|calls| calls.set(calls.get() + 1));
    let mut next = ptr::null_mut();
    assert_eq!(napi_create_double(env, 11.0, &mut next), NAPI_OK);
    assert!(matches!(value_ref(next), Ok(Value::Number(11.0))));
}

#[test]
fn unrooted_scoped_finalizer_runs_once_after_retirement_and_reentry_is_live() {
    unsafe {
        SCOPED_FINALIZER_CALLS.with(|calls| calls.set(0));
        let mut env = Env::new();
        let env_ptr: NapiEnv = &mut env;
        let mut scope = ptr::null_mut();
        assert_eq!(napi_open_handle_scope(env_ptr, &mut scope), NAPI_OK);
        let object = env.alloc(Value::Object(HashMap::new()));
        env.wraps.insert(object as usize, WrapRecord {
            data: ptr::null_mut(), finalize: Some(scoped_finalizer_creates_new_value),
            hint: ptr::null_mut(),
        });
        assert_eq!(napi_close_handle_scope(env_ptr, scope), NAPI_OK);
        assert_eq!(SCOPED_FINALIZER_CALLS.with(|calls| calls.get()), 1);
        assert!(!env.values.contains(&object));
        assert!(env.finalized_handles.contains(&(object as usize)));
        assert_eq!(value_ref(object).err(), Some(NAPI_INVALID_ARG));
        drop(env);
        assert_eq!(SCOPED_FINALIZER_CALLS.with(|calls| calls.get()), 1);
    }
}

#[cfg(feature = "quickjs")]
#[test]
fn scoped_accessor_js_roots_drop_only_after_owner_metadata_is_removed() {
    unsafe {
        let mut env = Env::new();
        let env_ptr: NapiEnv = &mut env;
        let mut scope = ptr::null_mut();
        assert_eq!(napi_open_handle_scope(env_ptr, &mut scope), NAPI_OK);
        let owner = env.alloc(Value::Object(HashMap::new()));
        #[allow(clippy::arc_with_non_send_sync)]
        let roots = Arc::new(QuickJsAccessorRoots {
            getter: 0, setter: 0,
            getter_native: ptr::null_mut(), setter_native: ptr::null_mut(),
        });
        env.accessors.insert((owner as usize, PropertyKey::String("x".into())), Accessor {
            getter: None, setter: None,
            getter_data: ptr::null_mut(), setter_data: ptr::null_mut(),
            getter_reflection: None, setter_reflection: None,
            js_owner: Some(roots.clone()),
        });
        assert_eq!(Arc::strong_count(&roots), 2);
        assert_eq!(napi_close_handle_scope(env_ptr, scope), NAPI_OK);
        assert!(!env.values.contains(&owner));
        assert!(env.accessors.is_empty());
        assert_eq!(Arc::strong_count(&roots), 1);
    }
}

#[test]
fn native_parent_edge_mutations_sweep_scoped_children_after_publication() {
    unsafe {
        SCOPED_FINALIZER_CALLS.with(|calls| calls.set(0));
        let mut env = Env::new();
        let env_ptr: NapiEnv = &mut env;
        let parent = env.alloc(Value::Object(HashMap::new()));
        let replacement = env.alloc(Value::Number(7.0));
        let key = env.alloc(Value::String("child".into()));
        let mut scope = ptr::null_mut();
        assert_eq!(napi_open_handle_scope(env_ptr, &mut scope), NAPI_OK);
        let child = env.alloc(Value::Object(HashMap::new()));
        env.wraps.insert(child as usize, WrapRecord {
            data: ptr::null_mut(), finalize: Some(scoped_finalizer_creates_new_value),
            hint: ptr::null_mut(),
        });
        assert_eq!(napi_set_named_property(env_ptr, parent, c"child".as_ptr(), child), NAPI_OK);
        assert_eq!(napi_close_handle_scope(env_ptr, scope), NAPI_OK);
        assert!(env.values.contains(&child));
        assert_eq!(SCOPED_FINALIZER_CALLS.with(|calls| calls.get()), 0);
        assert_eq!(napi_set_named_property(env_ptr, parent, c"child".as_ptr(), replacement), NAPI_OK);
        assert!(!env.values.contains(&child));
        assert_eq!(SCOPED_FINALIZER_CALLS.with(|calls| calls.get()), 1);

        let mut next_scope = ptr::null_mut();
        assert_eq!(napi_open_handle_scope(env_ptr, &mut next_scope), NAPI_OK);
        let next_child = env.alloc(Value::Object(HashMap::new()));
        env.wraps.insert(next_child as usize, WrapRecord {
            data: ptr::null_mut(), finalize: Some(scoped_finalizer_creates_new_value),
            hint: ptr::null_mut(),
        });
        assert_eq!(napi_set_property(env_ptr, parent, key, next_child), NAPI_OK);
        assert_eq!(napi_close_handle_scope(env_ptr, next_scope), NAPI_OK);
        let mut deleted = false;
        env.property_attributes.insert((parent as usize, PropertyKey::String("child".into())),
            NAPI_WRITABLE | NAPI_ENUMERABLE);
        assert_eq!(napi_delete_property(env_ptr, parent, key, &mut deleted), NAPI_OK);
        assert!(!deleted);
        assert!(env.values.contains(&next_child));
        env.property_attributes.insert((parent as usize, PropertyKey::String("child".into())),
            NAPI_DEFAULT_PROPERTY_ATTRIBUTES);
        assert_eq!(napi_delete_property(env_ptr, parent, key, &mut deleted), NAPI_OK);
        assert!(deleted);
        assert!(!env.values.contains(&next_child));
        assert_eq!(SCOPED_FINALIZER_CALLS.with(|calls| calls.get()), 2);
    }
}

#[test]
fn native_array_truncation_and_prototype_change_sweep_scoped_children() {
    unsafe {
        let mut env = Env::new();
        let env_ptr: NapiEnv = &mut env;
        let array = env.alloc(Value::Array(Vec::new()));
        let object = env.alloc(Value::Object(HashMap::new()));
        let new_length = env.alloc(Value::Number(0.0));
        let null = env.alloc(Value::Null);
        let mut scope = ptr::null_mut();
        assert_eq!(napi_open_handle_scope(env_ptr, &mut scope), NAPI_OK);
        let element = env.alloc(Value::Object(HashMap::new()));
        let prototype = env.alloc(Value::Object(HashMap::new()));
        assert_eq!(napi_set_element(env_ptr, array, 0, element), NAPI_OK);
        assert_eq!(node_api_set_prototype(env_ptr, object, prototype), NAPI_OK);
        assert_eq!(napi_close_handle_scope(env_ptr, scope), NAPI_OK);
        assert!(env.values.contains(&element));
        assert!(env.values.contains(&prototype));
        assert_eq!(napi_set_named_property(env_ptr, array, c"length".as_ptr(), new_length), NAPI_OK);
        assert!(!env.values.contains(&element));
        assert_eq!(node_api_set_prototype(env_ptr, object, null), NAPI_OK);
        assert!(!env.values.contains(&prototype));
    }
}

#[test]
fn partial_array_length_failure_still_sweeps_deleted_high_index() {
    unsafe {
        let mut env = Env::new();
        let env_ptr: NapiEnv = &mut env;
        let array = env.alloc(Value::Array(Vec::new()));
        let blocker = env.alloc(Value::Number(1.0));
        let zero = env.alloc(Value::Number(0.0));
        assert_eq!(napi_set_element(env_ptr, array, 1, blocker), NAPI_OK);
        env.property_attributes.insert((array as usize, PropertyKey::String("1".into())),
            NAPI_WRITABLE | NAPI_ENUMERABLE);
        let mut scope = ptr::null_mut();
        assert_eq!(napi_open_handle_scope(env_ptr, &mut scope), NAPI_OK);
        let child = env.alloc(Value::Object(HashMap::new()));
        assert_eq!(napi_set_element(env_ptr, array, 2, child), NAPI_OK);
        assert_eq!(napi_close_handle_scope(env_ptr, scope), NAPI_OK);
        assert!(env.values.contains(&child));
        assert_eq!(napi_set_named_property(env_ptr, array, c"length".as_ptr(), zero),
            NAPI_GENERIC_FAILURE);
        assert!(!env.values.contains(&child));
        assert!(matches!(value_ref(blocker), Ok(Value::Number(1.0))));
    }
}

std::thread_local! {
    static THROWING_SCOPED_FINALIZER_CALLS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

unsafe extern "C" fn throwing_scoped_finalizer(
    env: NapiEnv, data: *mut c_void, _: *mut c_void,
) {
    THROWING_SCOPED_FINALIZER_CALLS.with(|calls| calls.set(calls.get() + 1));
    let mut reentrant = ptr::null_mut();
    assert_eq!(napi_create_double(env, 13.0, &mut reentrant), NAPI_OK);
    if !data.is_null() {
        assert_eq!(napi_set_named_property(env, data.cast(), c"duringFinalizer".as_ptr(), reentrant), NAPI_OK);
    }
    assert_eq!(napi_get_named_property(env, ptr::null_mut(), c"invalid".as_ptr(), &mut reentrant),
        NAPI_INVALID_ARG);
    let thrown = (*env).alloc(Value::Error("scoped finalizer failure".into()));
    assert_eq!(napi_throw(env, thrown), NAPI_OK);
}

#[test]
fn mutation_finalizer_reports_own_error_and_preserves_original_exception() {
    unsafe {
        THROWING_SCOPED_FINALIZER_CALLS.with(|calls| calls.set(0));
        let mut env = Env::new();
        let env_ptr: NapiEnv = &mut env;
        let parent = env.alloc(Value::Object(HashMap::new()));
        let replacement = env.alloc(Value::Number(2.0));
        let mut scope = ptr::null_mut();
        assert_eq!(napi_open_handle_scope(env_ptr, &mut scope), NAPI_OK);
        let child = env.alloc(Value::Object(HashMap::new()));
        let original = env.alloc(Value::Error("original exception".into()));
        env.exception = Some(original);
        env.wraps.insert(child as usize, WrapRecord {
            data: parent.cast(), finalize: Some(throwing_scoped_finalizer),
            hint: ptr::null_mut(),
        });
        assert_eq!(napi_set_named_property(env_ptr, parent, c"child".as_ptr(), child), NAPI_OK);
        assert_eq!(napi_close_handle_scope(env_ptr, scope), NAPI_OK);
        assert!(env.values.contains(&child));
        assert!(env.values.contains(&original));
        assert_eq!(record_status(env_ptr, NAPI_NUMBER_EXPECTED), NAPI_NUMBER_EXPECTED);
        let message = env.last_error_info.error_message;
        assert_eq!(napi_set_named_property(env_ptr, parent, c"child".as_ptr(), replacement), NAPI_OK);
        assert!(!env.values.contains(&child));
        assert_eq!(THROWING_SCOPED_FINALIZER_CALLS.with(|calls| calls.get()), 1);
        assert_eq!(env.exception, Some(original));
        assert!(env.values.contains(&original));
        assert_eq!(env.last_error_info.error_code, NAPI_NUMBER_EXPECTED);
        assert_eq!(env.last_error_info.error_message, message);
        assert!(HOST.with(|host| host.borrow().shutdown_errors.back()
            .is_some_and(|text| text.contains("scoped finalizer failure"))));
    }
}

#[test]
fn partial_length_failure_preserves_failure_status_after_throwing_finalizer() {
    unsafe {
        let mut env = Env::new();
        let env_ptr: NapiEnv = &mut env;
        let array = env.alloc(Value::Array(Vec::new()));
        let blocker = env.alloc(Value::Number(1.0));
        let zero = env.alloc(Value::Number(0.0));
        assert_eq!(napi_set_element(env_ptr, array, 1, blocker), NAPI_OK);
        env.property_attributes.insert((array as usize, PropertyKey::String("1".into())),
            NAPI_WRITABLE | NAPI_ENUMERABLE);
        let mut scope = ptr::null_mut();
        assert_eq!(napi_open_handle_scope(env_ptr, &mut scope), NAPI_OK);
        let child = env.alloc(Value::Object(HashMap::new()));
        env.wraps.insert(child as usize, WrapRecord {
            data: array.cast(), finalize: Some(throwing_scoped_finalizer),
            hint: ptr::null_mut(),
        });
        assert_eq!(napi_set_element(env_ptr, array, 2, child), NAPI_OK);
        assert_eq!(napi_close_handle_scope(env_ptr, scope), NAPI_OK);
        assert_eq!(napi_set_named_property(env_ptr, array, c"length".as_ptr(), zero),
            NAPI_GENERIC_FAILURE);
        assert!(!env.values.contains(&child));
        assert_eq!(env.last_error_info.error_code, NAPI_GENERIC_FAILURE);
        assert_eq!(env.exception, None);
        assert!(HOST.with(|host| host.borrow().shutdown_errors.back()
            .is_some_and(|text| text.contains("scoped finalizer failure"))));
    }
}
