
use super::*;
use std::process::Command;
use std::sync::atomic::{AtomicBool, AtomicUsize};

static BCRYPT_ASYNC_RESULT: OnceLock<Mutex<Option<String>>> = OnceLock::new();
static ASYNC_TEST_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
static EXTERNAL_MEMORY_FINALIZED: AtomicUsize = AtomicUsize::new(0);
static EXTERNAL_SHARED_FINALIZED: AtomicUsize = AtomicUsize::new(0);
static EXTERNAL_STRING_FINALIZED: AtomicUsize = AtomicUsize::new(0);
static PLAIN_EXTERNAL_FINALIZED: AtomicUsize = AtomicUsize::new(0);
static HELD_ASYNC_CLEANUP: AtomicUsize = AtomicUsize::new(0);
static POSTED_FINALIZER_RAN: AtomicBool = AtomicBool::new(false);
#[cfg(target_os = "linux")]
static UV_TIMER_FIRED: AtomicBool = AtomicBool::new(false);

#[cfg(target_os = "linux")]
unsafe extern "C" fn test_uv_timer_callback(timer: *mut c_void) {
    type UvTimerStop = unsafe extern "C" fn(*mut c_void) -> i32;
    type UvClose = unsafe extern "C" fn(*mut c_void, Option<unsafe extern "C" fn(*mut c_void)>);
    let stop = libc::dlsym(libc::RTLD_DEFAULT, c"uv_timer_stop".as_ptr());
    let close = libc::dlsym(libc::RTLD_DEFAULT, c"uv_close".as_ptr());
    if !stop.is_null() {
        std::mem::transmute::<*mut c_void, UvTimerStop>(stop)(timer);
    }
    if !close.is_null() {
        std::mem::transmute::<*mut c_void, UvClose>(close)(timer, None);
    }
    UV_TIMER_FIRED.store(true, Ordering::Release);
}

unsafe extern "C" fn external_memory_finalize(
    _env: NapiEnv,
    _data: *mut c_void,
    _hint: *mut c_void,
) {
    EXTERNAL_MEMORY_FINALIZED.fetch_add(1, Ordering::AcqRel);
}

unsafe extern "C" fn external_shared_finalize(data: *mut c_void, hint: *mut c_void) {
    assert!(!data.is_null());
    EXTERNAL_SHARED_FINALIZED.fetch_add(*(hint as *const usize), Ordering::AcqRel);
}

unsafe extern "C" fn external_string_finalize(
    _env: NapiEnv,
    _data: *mut c_void,
    hint: *mut c_void,
) {
    EXTERNAL_STRING_FINALIZED.fetch_add(*(hint as *const usize), Ordering::AcqRel);
}

unsafe extern "C" fn plain_external_finalize(_env: NapiEnv, data: *mut c_void, hint: *mut c_void) {
    let total = *(data.cast::<usize>()) + *(hint.cast::<usize>());
    PLAIN_EXTERNAL_FINALIZED.fetch_add(total, Ordering::AcqRel);
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
            NAPI_INVALID_ARG
        );
        assert_eq!(napi_get_last_error_info(env_ptr, &mut info), NAPI_OK);
        assert_eq!((*info).error_code, NAPI_INVALID_ARG);
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

#[test]
fn typed_arrays_share_arraybuffer_storage_with_offsets() {
    unsafe {
        let mut env = Env::new();
        let env_ptr: NapiEnv = &mut env;
        let mut data = ptr::null_mut();
        let mut buffer = ptr::null_mut();
        assert_eq!(
            napi_create_arraybuffer(env_ptr, 32, &mut data, &mut buffer),
            NAPI_OK
        );
        assert!(!data.is_null());
        *(data as *mut u8).add(4) = 42;

        let mut view = ptr::null_mut();
        assert_eq!(
            napi_create_typedarray(env_ptr, 6, 3, buffer, 4, &mut view),
            NAPI_OK
        );
        let mut is_view = false;
        let mut is_buffer = false;
        assert_eq!(napi_is_typedarray(env_ptr, view, &mut is_view), NAPI_OK);
        assert_eq!(
            napi_is_arraybuffer(env_ptr, buffer, &mut is_buffer),
            NAPI_OK
        );
        assert!(is_view && is_buffer);

        let mut kind = -1;
        let mut length = 0;
        let mut view_data = ptr::null_mut();
        let mut backing = ptr::null_mut();
        let mut offset = 0;
        assert_eq!(
            napi_get_typedarray_info(
                env_ptr,
                view,
                &mut kind,
                &mut length,
                &mut view_data,
                &mut backing,
                &mut offset,
            ),
            NAPI_OK
        );
        assert_eq!(kind, 6);
        assert_eq!(length, 3);
        assert_eq!(offset, 4);
        assert_eq!(backing, buffer);
        assert_eq!(view_data, data.add(4));
        assert_eq!(*(view_data as *const u8), 42);

        assert_eq!(
            napi_create_typedarray(env_ptr, 6, 1, buffer, 2, &mut view),
            NAPI_INVALID_ARG
        );
        assert_eq!(
            napi_create_typedarray(env_ptr, 8, 5, buffer, 0, &mut view),
            NAPI_INVALID_ARG
        );
    }
}

#[test]
fn typed_array_indexes_apply_kind_specific_conversions() {
    unsafe {
        let mut env = Env::new();
        let env_ptr: NapiEnv = &mut env;
        let numeric_cases = [
            (0, -129.0, 127.0),
            (1, -1.0, 255.0),
            (2, 3.5, 4.0),
            (3, 65_535.0, -1.0),
            (4, -1.0, 65_535.0),
            (5, 4_294_967_295.0, -1.0),
            (6, -1.0, 4_294_967_295.0),
            (7, 1.25, 1.25),
            (8, 1.5, 1.5),
        ];
        for (kind, input, expected) in numeric_cases {
            let mut backing = ptr::null_mut();
            assert_eq!(
                napi_create_arraybuffer(
                    env_ptr,
                    typedarray_element_size(kind).unwrap(),
                    ptr::null_mut(),
                    &mut backing,
                ),
                NAPI_OK
            );
            let mut view = ptr::null_mut();
            assert_eq!(
                napi_create_typedarray(env_ptr, kind, 1, backing, 0, &mut view),
                NAPI_OK
            );
            let input = env.alloc(Value::Number(input));
            assert_eq!(napi_set_element(env_ptr, view, 0, input), NAPI_OK);
            let mut result = ptr::null_mut();
            assert_eq!(napi_get_element(env_ptr, view, 0, &mut result), NAPI_OK);
            assert!(
                matches!(value_ref(result), Ok(Value::Number(value)) if *value == expected),
                "typed array kind {kind} returned an unexpected value"
            );
            let mut deleted = true;
            assert_eq!(napi_delete_element(env_ptr, view, 0, &mut deleted), NAPI_OK);
            assert!(!deleted);
        }

        for (kind, negative, word) in [(9, true, 1_u64), (10, false, u64::MAX)] {
            let mut backing = ptr::null_mut();
            assert_eq!(
                napi_create_arraybuffer(env_ptr, 8, ptr::null_mut(), &mut backing),
                NAPI_OK
            );
            let mut view = ptr::null_mut();
            assert_eq!(
                napi_create_typedarray(env_ptr, kind, 1, backing, 0, &mut view),
                NAPI_OK
            );
            let input = env.alloc(Value::BigInt {
                negative,
                words: vec![word],
            });
            assert_eq!(napi_set_element(env_ptr, view, 0, input), NAPI_OK);
            let mut result = ptr::null_mut();
            assert_eq!(napi_get_element(env_ptr, view, 0, &mut result), NAPI_OK);
            assert!(matches!(
                value_ref(result),
                Ok(Value::BigInt { negative: actual_negative, words })
                    if *actual_negative == negative && words == &[word]
            ));
        }

        let mut data = ptr::null_mut();
        let mut backing = ptr::null_mut();
        assert_eq!(
            napi_create_arraybuffer(env_ptr, 8, &mut data, &mut backing),
            NAPI_OK
        );
        let mut offset_view = ptr::null_mut();
        assert_eq!(
            napi_create_typedarray(env_ptr, 4, 2, backing, 2, &mut offset_view),
            NAPI_OK
        );
        let value = env.alloc(Value::Number(513.0));
        assert_eq!(napi_set_element(env_ptr, offset_view, 1, value), NAPI_OK);
        assert_eq!(
            ptr::read_unaligned((data as *const u8).add(4).cast::<u16>()),
            513
        );
        assert_eq!(napi_detach_arraybuffer(env_ptr, backing), NAPI_OK);
        let mut present = true;
        assert_eq!(
            napi_has_element(env_ptr, offset_view, 1, &mut present),
            NAPI_OK
        );
        assert!(!present);
    }
}

#[test]
fn collection_metadata_properties_follow_javascript_descriptors() {
    unsafe {
        fn number(value: NapiValue) -> f64 {
            match unsafe { value_ref(value) }.unwrap() {
                Value::Number(value) => *value,
                _ => panic!("metadata property was not numeric"),
            }
        }

        let mut env = Env::new();
        let env_ptr: NapiEnv = &mut env;
        let mut array = ptr::null_mut();
        assert_eq!(
            napi_create_array_with_length(env_ptr, 3, &mut array),
            NAPI_OK
        );
        let mut actual = ptr::null_mut();
        assert_eq!(
            napi_get_named_property(env_ptr, array, c"length".as_ptr(), &mut actual),
            NAPI_OK
        );
        assert_eq!(number(actual), 3.0);
        let one = env.alloc(Value::Number(1.0));
        assert_eq!(
            napi_set_named_property(env_ptr, array, c"length".as_ptr(), one),
            NAPI_OK
        );
        let mut length = 0;
        assert_eq!(napi_get_array_length(env_ptr, array, &mut length), NAPI_OK);
        assert_eq!(length, 1);

        let length_key = env.alloc(Value::String("length".into()));
        let mut own = false;
        assert_eq!(
            napi_has_own_property(env_ptr, array, length_key, &mut own),
            NAPI_OK
        );
        assert!(own);
        let mut deleted = true;
        assert_eq!(
            napi_delete_property(env_ptr, array, length_key, &mut deleted),
            NAPI_OK
        );
        assert!(!deleted);

        let mut data = ptr::null_mut();
        let mut buffer = ptr::null_mut();
        assert_eq!(
            napi_create_buffer(env_ptr, 4, &mut data, &mut buffer),
            NAPI_OK
        );
        assert_eq!(
            napi_get_named_property(env_ptr, buffer, c"length".as_ptr(), &mut actual),
            NAPI_OK
        );
        assert_eq!(number(actual), 4.0);
        assert_eq!(
            napi_has_own_property(env_ptr, buffer, length_key, &mut own),
            NAPI_OK
        );
        assert!(!own, "Buffer length is inherited metadata");
        assert_eq!(
            napi_set_named_property(env_ptr, buffer, c"length".as_ptr(), one),
            NAPI_GENERIC_FAILURE
        );

        let mut backing = ptr::null_mut();
        assert_eq!(
            napi_create_arraybuffer(env_ptr, 16, &mut data, &mut backing),
            NAPI_OK
        );
        assert_eq!(
            napi_get_named_property(env_ptr, backing, c"byteLength".as_ptr(), &mut actual),
            NAPI_OK
        );
        assert_eq!(number(actual), 16.0);
        let mut view = ptr::null_mut();
        assert_eq!(
            napi_create_typedarray(env_ptr, 4, 3, backing, 2, &mut view),
            NAPI_OK
        );
        for (name, expected) in [(c"length", 3.0), (c"byteLength", 6.0), (c"byteOffset", 2.0)] {
            assert_eq!(
                napi_get_named_property(env_ptr, view, name.as_ptr(), &mut actual),
                NAPI_OK
            );
            assert_eq!(number(actual), expected);
        }
        assert_eq!(
            napi_get_named_property(env_ptr, view, c"buffer".as_ptr(), &mut actual),
            NAPI_OK
        );
        assert_eq!(actual, backing);
        assert_eq!(napi_detach_arraybuffer(env_ptr, backing), NAPI_OK);
        for name in [c"length", c"byteLength", c"byteOffset"] {
            assert_eq!(
                napi_get_named_property(env_ptr, view, name.as_ptr(), &mut actual),
                NAPI_OK
            );
            assert_eq!(number(actual), 0.0);
        }
    }
}

#[test]
fn buffers_can_view_arraybuffer_ranges_without_copying() {
    unsafe {
        let mut env = Env::new();
        let env_ptr: NapiEnv = &mut env;
        let mut data = ptr::null_mut();
        let mut array_buffer = ptr::null_mut();
        assert_eq!(
            napi_create_arraybuffer(env_ptr, 8, &mut data, &mut array_buffer),
            NAPI_OK
        );
        *(data as *mut u8).add(2) = 7;
        let mut buffer = ptr::null_mut();
        assert_eq!(
            node_api_create_buffer_from_arraybuffer(env_ptr, array_buffer, 2, 3, &mut buffer,),
            NAPI_OK
        );
        let mut view_data = ptr::null_mut();
        let mut length = 0;
        assert_eq!(
            napi_get_buffer_info(env_ptr, buffer, &mut view_data, &mut length),
            NAPI_OK
        );
        assert_eq!(view_data, (data as *mut u8).add(2).cast());
        assert_eq!(length, 3);
        *(view_data as *mut u8).add(1) = 9;
        assert_eq!(*(data as *mut u8).add(3), 9);

        let mut kind = -1;
        let mut backing = ptr::null_mut();
        let mut offset = 0;
        assert_eq!(
            napi_get_typedarray_info(
                env_ptr,
                buffer,
                &mut kind,
                &mut length,
                &mut view_data,
                &mut backing,
                &mut offset,
            ),
            NAPI_OK
        );
        assert_eq!((kind, length, backing, offset), (1, 3, array_buffer, 2));

        assert_eq!(
            node_api_create_buffer_from_arraybuffer(env_ptr, array_buffer, 7, 2, &mut buffer,),
            NAPI_PENDING_EXCEPTION
        );
        assert!(env.exception.take().is_some());
        assert_eq!(napi_detach_arraybuffer(env_ptr, array_buffer), NAPI_OK);
        assert_eq!(
            napi_get_buffer_info(env_ptr, buffer, &mut view_data, &mut length),
            NAPI_OK
        );
        assert!(view_data.is_null());
        assert_eq!(length, 0);
    }
}

#[test]
fn buffer_indexes_share_backing_memory_and_cannot_be_deleted() {
    unsafe {
        let mut env = Env::new();
        let env_ptr: NapiEnv = &mut env;
        let mut data = ptr::null_mut();
        let mut owned = ptr::null_mut();
        assert_eq!(
            napi_create_buffer(env_ptr, 3, &mut data, &mut owned),
            NAPI_OK
        );
        let negative = env.alloc(Value::Number(-1.0));
        assert_eq!(napi_set_element(env_ptr, owned, 1, negative), NAPI_OK);
        assert_eq!(*(data as *const u8).add(1), 255);
        let mut actual = ptr::null_mut();
        assert_eq!(napi_get_element(env_ptr, owned, 1, &mut actual), NAPI_OK);
        assert!(matches!(value_ref(actual), Ok(Value::Number(255.0))));

        let key = env.alloc(Value::String("2".into()));
        let wrapped = env.alloc(Value::String("258".into()));
        assert_eq!(napi_set_property(env_ptr, owned, key, wrapped), NAPI_OK);
        assert_eq!(*(data as *const u8).add(2), 2);
        let mut deleted = true;
        assert_eq!(
            napi_delete_element(env_ptr, owned, 2, &mut deleted),
            NAPI_OK
        );
        assert!(!deleted);

        let mut external_bytes = [4_u8, 5];
        let mut external = ptr::null_mut();
        assert_eq!(
            napi_create_external_buffer(
                env_ptr,
                external_bytes.len(),
                external_bytes.as_mut_ptr().cast(),
                None,
                ptr::null_mut(),
                &mut external,
            ),
            NAPI_OK
        );
        let seven = env.alloc(Value::Number(7.0));
        assert_eq!(napi_set_element(env_ptr, external, 0, seven), NAPI_OK);
        assert_eq!(external_bytes[0], 7);

        let mut array_buffer = ptr::null_mut();
        assert_eq!(
            napi_create_arraybuffer(env_ptr, 4, &mut data, &mut array_buffer),
            NAPI_OK
        );
        let mut view = ptr::null_mut();
        assert_eq!(
            node_api_create_buffer_from_arraybuffer(env_ptr, array_buffer, 1, 2, &mut view),
            NAPI_OK
        );
        assert_eq!(napi_set_element(env_ptr, view, 1, seven), NAPI_OK);
        assert_eq!(*(data as *const u8).add(2), 7);

        let mut names = ptr::null_mut();
        assert_eq!(
            napi_get_all_property_names(
                env_ptr,
                view,
                NAPI_KEY_OWN_ONLY,
                NAPI_KEY_ALL_PROPERTIES,
                NAPI_KEY_KEEP_NUMBERS,
                &mut names,
            ),
            NAPI_OK
        );
        let Ok(Value::Array(names)) = value_ref(names) else {
            panic!("buffer indexes were not returned as an array");
        };
        assert_eq!(names.len(), 2);
        assert!(matches!(
            value_ref(names[0].unwrap()),
            Ok(Value::Number(0.0))
        ));
        assert!(matches!(
            value_ref(names[1].unwrap()),
            Ok(Value::Number(1.0))
        ));
    }
}

#[test]
fn sharedarraybuffers_back_views_but_cannot_be_detached() {
    unsafe {
        let mut env = Env::new();
        let env_ptr: NapiEnv = &mut env;
        let mut data = ptr::null_mut();
        let mut shared = ptr::null_mut();
        assert_eq!(
            node_api_create_sharedarraybuffer(env_ptr, 16, &mut data, &mut shared),
            NAPI_OK
        );
        assert!(!data.is_null());
        let mut is_shared = false;
        assert_eq!(
            node_api_is_sharedarraybuffer(env_ptr, shared, &mut is_shared),
            NAPI_OK
        );
        assert!(is_shared);
        let mut is_arraybuffer = true;
        assert_eq!(
            napi_is_arraybuffer(env_ptr, shared, &mut is_arraybuffer),
            NAPI_OK
        );
        assert!(!is_arraybuffer);

        let mut view = ptr::null_mut();
        assert_eq!(
            napi_create_typedarray(env_ptr, 1, 4, shared, 3, &mut view),
            NAPI_OK
        );
        let mut view_data = ptr::null_mut();
        let mut length = 0;
        let mut backing = ptr::null_mut();
        let mut offset = 0;
        assert_eq!(
            napi_get_typedarray_info(
                env_ptr,
                view,
                ptr::null_mut(),
                &mut length,
                &mut view_data,
                &mut backing,
                &mut offset,
            ),
            NAPI_OK
        );
        assert_eq!((length, backing, offset), (4, shared, 3));
        *view_data.cast::<u8>() = 42;
        assert_eq!(*(data as *mut u8).add(3), 42);

        let mut buffer = ptr::null_mut();
        assert_eq!(
            node_api_create_buffer_from_arraybuffer(env_ptr, shared, 2, 5, &mut buffer),
            NAPI_OK
        );
        assert_eq!(
            napi_detach_arraybuffer(env_ptr, shared),
            NAPI_ARRAYBUFFER_EXPECTED
        );
        assert_eq!(
            napi_get_buffer_info(env_ptr, buffer, &mut view_data, &mut length),
            NAPI_OK
        );
        assert_eq!(length, 5);
    }
}

#[test]
fn external_sharedarraybuffers_share_memory_and_finalize_without_an_env() {
    unsafe {
        EXTERNAL_SHARED_FINALIZED.store(0, Ordering::Release);
        let mut storage = [1_u8, 2, 3, 4];
        let increment = 3_usize;
        let mut env = Env::new();
        let env_ptr: NapiEnv = &mut env;
        let mut shared = ptr::null_mut();
        assert_eq!(
            node_api_create_external_sharedarraybuffer(
                env_ptr,
                storage.as_mut_ptr().cast(),
                storage.len(),
                Some(external_shared_finalize),
                (&increment as *const usize).cast_mut().cast(),
                &mut shared,
            ),
            NAPI_OK
        );
        let mut is_shared = false;
        assert_eq!(
            node_api_is_sharedarraybuffer(env_ptr, shared, &mut is_shared),
            NAPI_OK
        );
        assert!(is_shared);

        let mut view = ptr::null_mut();
        assert_eq!(
            napi_create_typedarray(env_ptr, 1, storage.len(), shared, 0, &mut view),
            NAPI_OK
        );
        let seven = env.alloc(Value::Number(7.0));
        assert_eq!(napi_set_element(env_ptr, view, 2, seven), NAPI_OK);
        assert_eq!(storage[2], 7);
        assert_eq!(EXTERNAL_SHARED_FINALIZED.load(Ordering::Acquire), 0);
        drop(env);
        assert_eq!(EXTERNAL_SHARED_FINALIZED.load(Ordering::Acquire), 3);
    }
}

#[test]
fn dataviews_allow_unaligned_bounded_arraybuffer_views() {
    unsafe {
        let mut env = Env::new();
        let env_ptr: NapiEnv = &mut env;
        let mut data = ptr::null_mut();
        let mut buffer = ptr::null_mut();
        assert_eq!(
            napi_create_arraybuffer(env_ptr, 16, &mut data, &mut buffer),
            NAPI_OK
        );
        *(data as *mut u8).add(3) = 99;
        let mut view = ptr::null_mut();
        assert_eq!(
            napi_create_dataview(env_ptr, 7, buffer, 3, &mut view),
            NAPI_OK
        );
        let mut is_view = false;
        assert_eq!(napi_is_dataview(env_ptr, view, &mut is_view), NAPI_OK);
        assert!(is_view);

        let mut length = 0;
        let mut view_data = ptr::null_mut();
        let mut backing = ptr::null_mut();
        let mut offset = 0;
        assert_eq!(
            napi_get_dataview_info(
                env_ptr,
                view,
                &mut length,
                &mut view_data,
                &mut backing,
                &mut offset,
            ),
            NAPI_OK
        );
        assert_eq!(length, 7);
        assert_eq!(offset, 3);
        assert_eq!(backing, buffer);
        assert_eq!(*(view_data as *const u8), 99);
        assert_eq!(
            napi_create_dataview(env_ptr, 14, buffer, 3, &mut view),
            NAPI_INVALID_ARG
        );
    }
}

#[test]
fn external_buffers_share_memory_and_finalize_once() {
    unsafe {
        EXTERNAL_MEMORY_FINALIZED.store(0, Ordering::Release);
        let mut array_storage = [0_u8; 16];
        let mut buffer_storage = [1_u8, 2, 3, 4];
        let mut env = Env::new();
        let env_ptr: NapiEnv = &mut env;

        let mut array_buffer = ptr::null_mut();
        assert_eq!(
            napi_create_external_arraybuffer(
                env_ptr,
                array_storage.as_mut_ptr().cast(),
                array_storage.len(),
                Some(external_memory_finalize),
                ptr::null_mut(),
                &mut array_buffer,
            ),
            NAPI_OK
        );
        let mut view = ptr::null_mut();
        assert_eq!(
            napi_create_typedarray(env_ptr, 4, 3, array_buffer, 2, &mut view),
            NAPI_OK
        );
        let mut view_data = ptr::null_mut();
        assert_eq!(
            napi_get_typedarray_info(
                env_ptr,
                view,
                ptr::null_mut(),
                ptr::null_mut(),
                &mut view_data,
                ptr::null_mut(),
                ptr::null_mut(),
            ),
            NAPI_OK
        );
        *(view_data as *mut u8) = 42;
        assert_eq!(array_storage[2], 42);

        let mut buffer = ptr::null_mut();
        assert_eq!(
            napi_create_external_buffer(
                env_ptr,
                buffer_storage.len(),
                buffer_storage.as_mut_ptr().cast(),
                Some(external_memory_finalize),
                ptr::null_mut(),
                &mut buffer,
            ),
            NAPI_OK
        );
        let mut data = ptr::null_mut();
        let mut length = 0;
        assert_eq!(
            napi_get_buffer_info(env_ptr, buffer, &mut data, &mut length),
            NAPI_OK
        );
        assert_eq!(length, 4);
        *(data as *mut u8).add(1) = 9;
        assert_eq!(buffer_storage[1], 9);
        assert_eq!(EXTERNAL_MEMORY_FINALIZED.load(Ordering::Acquire), 0);
        drop(env);
        assert_eq!(EXTERNAL_MEMORY_FINALIZED.load(Ordering::Acquire), 2);
    }
}

#[test]
fn external_memory_adjustments_are_tracked_per_environment() {
    unsafe {
        let mut env = Env::new();
        let env_ptr: NapiEnv = &mut env;
        let mut adjusted = 0;
        assert_eq!(
            napi_adjust_external_memory(env_ptr, 4096, &mut adjusted),
            NAPI_OK
        );
        assert_eq!(adjusted, 4096);
        assert_eq!(
            napi_adjust_external_memory(env_ptr, -1024, &mut adjusted),
            NAPI_OK
        );
        assert_eq!(adjusted, 3072);
        env.external_memory = i64::MAX;
        assert_eq!(
            napi_adjust_external_memory(env_ptr, 1, &mut adjusted),
            NAPI_GENERIC_FAILURE
        );
        assert_eq!(env.external_memory, i64::MAX);
    }
}

#[test]
fn detached_arraybuffers_zero_existing_views() {
    unsafe {
        let mut env = Env::new();
        let env_ptr: NapiEnv = &mut env;
        let mut buffer = ptr::null_mut();
        assert_eq!(
            napi_create_arraybuffer(env_ptr, 16, ptr::null_mut(), &mut buffer),
            NAPI_OK
        );
        let mut view = ptr::null_mut();
        assert_eq!(
            napi_create_typedarray(env_ptr, 1, 8, buffer, 4, &mut view),
            NAPI_OK
        );
        let mut detached = true;
        assert_eq!(
            napi_is_detached_arraybuffer(env_ptr, buffer, &mut detached),
            NAPI_OK
        );
        assert!(!detached);
        assert_eq!(napi_detach_arraybuffer(env_ptr, buffer), NAPI_OK);
        assert_eq!(
            napi_is_detached_arraybuffer(env_ptr, buffer, &mut detached),
            NAPI_OK
        );
        assert!(detached);

        let mut data = ptr::dangling_mut::<c_void>();
        let mut length = usize::MAX;
        assert_eq!(
            napi_get_arraybuffer_info(env_ptr, buffer, &mut data, &mut length),
            NAPI_OK
        );
        assert!(data.is_null());
        assert_eq!(length, 0);
        assert_eq!(
            napi_get_typedarray_info(
                env_ptr,
                view,
                ptr::null_mut(),
                &mut length,
                &mut data,
                ptr::null_mut(),
                ptr::null_mut(),
            ),
            NAPI_OK
        );
        assert!(data.is_null());
        assert_eq!(length, 0);
        assert_eq!(
            napi_create_typedarray(env_ptr, 1, 0, buffer, 0, &mut view),
            NAPI_INVALID_ARG
        );
    }
}

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

#[test]
fn run_script_evaluates_values_and_reports_exceptions() {
    unsafe {
        let mut env = Env::new();
        let env_ptr: NapiEnv = &mut env;
        let mut script = ptr::null_mut();
        assert_eq!(
            napi_create_string_utf8(
                env_ptr,
                c"globalThis.__thawNapiProbe = 40; ({ answer: __thawNapiProbe + 2 })".as_ptr(),
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

#[cfg(target_os = "linux")]
#[test]
fn poll_uv_loop_drives_the_default_libuv_loop() {
    let _guard = lock_async_test();
    unsafe {
        type UvDefaultLoop = unsafe extern "C" fn() -> *mut c_void;
        type UvHandleSize = unsafe extern "C" fn(i32) -> usize;
        type UvTimerInit = unsafe extern "C" fn(*mut c_void, *mut c_void) -> i32;
        type UvTimerStart = unsafe extern "C" fn(
            *mut c_void,
            Option<unsafe extern "C" fn(*mut c_void)>,
            u64,
            u64,
        ) -> i32;

        let library = libc::dlopen(c"libuv.so.1".as_ptr(), libc::RTLD_NOW | libc::RTLD_GLOBAL);
        assert!(!library.is_null());
        let default_loop = std::mem::transmute::<*mut c_void, UvDefaultLoop>(libc::dlsym(
            libc::RTLD_DEFAULT,
            c"uv_default_loop".as_ptr(),
        ));
        let handle_size = std::mem::transmute::<*mut c_void, UvHandleSize>(libc::dlsym(
            libc::RTLD_DEFAULT,
            c"uv_handle_size".as_ptr(),
        ));
        let timer_init = std::mem::transmute::<*mut c_void, UvTimerInit>(libc::dlsym(
            libc::RTLD_DEFAULT,
            c"uv_timer_init".as_ptr(),
        ));
        let timer_start = std::mem::transmute::<*mut c_void, UvTimerStart>(libc::dlsym(
            libc::RTLD_DEFAULT,
            c"uv_timer_start".as_ptr(),
        ));
        const UV_TIMER: i32 = 13;
        let timer = libc::calloc(1, handle_size(UV_TIMER));
        assert!(!timer.is_null());
        assert_eq!(timer_init(default_loop(), timer), 0);
        UV_TIMER_FIRED.store(false, Ordering::Release);
        assert_eq!(timer_start(timer, Some(test_uv_timer_callback), 1, 0), 0);
        for _ in 0..1000 {
            poll_uv_loop();
            if UV_TIMER_FIRED.load(Ordering::Acquire) {
                break;
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        assert!(UV_TIMER_FIRED.load(Ordering::Acquire));
        poll_uv_loop();
        libc::free(timer);
        libc::dlclose(library);
    }
}

#[cfg(target_os = "linux")]
#[test]
fn async_drain_drives_registered_private_libuv_loops() {
    let _guard = lock_async_test();
    unsafe {
        type UvLoopSize = unsafe extern "C" fn() -> usize;
        type UvLoopInit = unsafe extern "C" fn(*mut c_void) -> i32;
        type UvLoopClose = unsafe extern "C" fn(*mut c_void) -> i32;
        type UvHandleSize = unsafe extern "C" fn(i32) -> usize;
        type UvTimerInit = unsafe extern "C" fn(*mut c_void, *mut c_void) -> i32;
        type UvTimerStart = unsafe extern "C" fn(
            *mut c_void,
            Option<unsafe extern "C" fn(*mut c_void)>,
            u64,
            u64,
        ) -> i32;

        let library = libc::dlopen(c"libuv.so.1".as_ptr(), libc::RTLD_NOW | libc::RTLD_GLOBAL);
        assert!(!library.is_null());
        let loop_size = std::mem::transmute::<*mut c_void, UvLoopSize>(libc::dlsym(
            libc::RTLD_DEFAULT,
            c"uv_loop_size".as_ptr(),
        ));
        let loop_init = std::mem::transmute::<*mut c_void, UvLoopInit>(libc::dlsym(
            libc::RTLD_DEFAULT,
            c"uv_loop_init".as_ptr(),
        ));
        let loop_close = std::mem::transmute::<*mut c_void, UvLoopClose>(libc::dlsym(
            libc::RTLD_DEFAULT,
            c"uv_loop_close".as_ptr(),
        ));
        let handle_size = std::mem::transmute::<*mut c_void, UvHandleSize>(libc::dlsym(
            libc::RTLD_DEFAULT,
            c"uv_handle_size".as_ptr(),
        ));
        let timer_init = std::mem::transmute::<*mut c_void, UvTimerInit>(libc::dlsym(
            libc::RTLD_DEFAULT,
            c"uv_timer_init".as_ptr(),
        ));
        let timer_start = std::mem::transmute::<*mut c_void, UvTimerStart>(libc::dlsym(
            libc::RTLD_DEFAULT,
            c"uv_timer_start".as_ptr(),
        ));

        let event_loop = libc::calloc(1, loop_size());
        assert!(!event_loop.is_null());
        assert_eq!(loop_init(event_loop), 0);
        assert_eq!(thaw_napi_register_uv_loop(event_loop), NAPI_OK);
        assert_eq!(thaw_napi_register_uv_loop(event_loop), NAPI_OK);
        const UV_TIMER: i32 = 13;
        let timer = libc::calloc(1, handle_size(UV_TIMER));
        assert!(!timer.is_null());
        assert_eq!(timer_init(event_loop, timer), 0);
        UV_TIMER_FIRED.store(false, Ordering::Release);
        assert_eq!(timer_start(timer, Some(test_uv_timer_callback), 1, 0), 0);
        thaw_napi_run_async_work();
        assert!(UV_TIMER_FIRED.load(Ordering::Acquire));
        assert_eq!(thaw_napi_unregister_uv_loop(event_loop), NAPI_OK);
        assert_eq!(thaw_napi_unregister_uv_loop(event_loop), NAPI_INVALID_ARG);
        assert_eq!(loop_close(event_loop), 0);
        libc::free(timer);
        libc::free(event_loop);
        libc::dlclose(library);
    }
}

unsafe extern "C" fn bcrypt_async_callback(env: NapiEnv, info: NapiCallbackInfo) -> NapiValue {
    let mut argc = 2;
    let mut args = [ptr::null_mut(); 2];
    assert_eq!(
        napi_get_cb_info(
            env,
            info,
            &mut argc,
            args.as_mut_ptr(),
            ptr::null_mut(),
            ptr::null_mut(),
        ),
        NAPI_OK
    );
    assert_eq!(argc, 2);
    let result = match value_ref(args[1]).unwrap() {
        Value::String(value) => value.clone(),
        _ => panic!("bcrypt callback did not receive a string"),
    };
    *BCRYPT_ASYNC_RESULT
        .get_or_init(|| Mutex::new(None))
        .lock()
        .unwrap() = Some(result);
    let mut undefined = ptr::null_mut();
    assert_eq!(napi_get_undefined(env, &mut undefined), NAPI_OK);
    undefined
}

unsafe extern "C" fn bcrypt_bridge_callback(
    context: *mut c_void,
    error: *const c_char,
    result: *const c_char,
) {
    let output = &*(context as *const Mutex<Option<(JsonValue, JsonValue)>>);
    *output.lock().unwrap() = Some((
        serde_json::from_str(CStr::from_ptr(error).to_str().unwrap()).unwrap(),
        serde_json::from_str(CStr::from_ptr(result).to_str().unwrap()).unwrap(),
    ));
}

struct AsyncProbe {
    main_thread: std::thread::ThreadId,
    execute_thread: Mutex<Option<std::thread::ThreadId>>,
    complete_thread: Mutex<Option<std::thread::ThreadId>>,
    executed: AtomicBool,
}

struct WorkerGate {
    state: Mutex<(usize, bool)>,
    changed: Condvar,
}

struct CancelProbe {
    executed: AtomicBool,
    status: AtomicI32,
}

struct ThreadsafeProbe {
    main_thread: std::thread::ThreadId,
    callback_threads: Mutex<Vec<std::thread::ThreadId>>,
    values: Mutex<Vec<f64>>,
    aborted: AtomicUsize,
    finalized: AtomicBool,
}

struct ParcelWatcherProbe {
    events: Mutex<Vec<(String, String)>>,
}

struct CleanupProbe {
    output: Arc<Mutex<Vec<u32>>>,
    value: u32,
}

unsafe extern "C" fn cleanup_probe(data: *mut c_void) {
    let probe = Box::from_raw(data as *mut CleanupProbe);
    probe.output.lock().unwrap().push(probe.value);
}

unsafe extern "C" fn async_cleanup_probe(handle: *mut AsyncCleanupHookHandle, data: *mut c_void) {
    cleanup_probe(data);
    assert_eq!(napi_remove_async_cleanup_hook(handle), NAPI_OK);
}

unsafe extern "C" fn hold_async_cleanup(handle: *mut AsyncCleanupHookHandle, _data: *mut c_void) {
    HELD_ASYNC_CLEANUP.store(handle as usize, Ordering::Release);
}

unsafe extern "C" fn posted_finalizer_uses_napi(
    env: NapiEnv,
    _data: *mut c_void,
    _hint: *mut c_void,
) {
    let mut value = ptr::null_mut();
    assert_eq!(
        napi_create_string_utf8(env, c"finalized".as_ptr(), 9, &mut value),
        NAPI_OK
    );
    assert!(matches!(value_ref(value), Ok(Value::String(text)) if text == "finalized"));
    POSTED_FINALIZER_RAN.store(true, Ordering::Release);
}

unsafe extern "C" fn parcel_watcher_callback(env: NapiEnv, info: NapiCallbackInfo) -> NapiValue {
    let info = info.as_ref().unwrap();
    let probe = &*(info.data as *const ParcelWatcherProbe);
    if let Some(events) = info.args.get(1).and_then(|value| match value_ref(*value) {
        Ok(Value::Array(events)) => Some(events),
        _ => None,
    }) {
        let mut collected = probe.events.lock().unwrap();
        for event in events {
            let Some(event) = event else {
                continue;
            };
            let Ok(Value::Object(fields)) = value_ref(*event) else {
                continue;
            };
            let path = fields
                .get(&PropertyKey::String("path".into()))
                .and_then(|value| match value_ref(*value) {
                    Ok(Value::String(value)) => Some(value.clone()),
                    _ => None,
                });
            let kind = fields
                .get(&PropertyKey::String("type".into()))
                .and_then(|value| match value_ref(*value) {
                    Ok(Value::String(value)) => Some(value.clone()),
                    _ => None,
                });
            if let (Some(path), Some(kind)) = (path, kind) {
                collected.push((path, kind));
            }
        }
    }
    let mut undefined = ptr::null_mut();
    assert_eq!(napi_get_undefined(env, &mut undefined), NAPI_OK);
    undefined
}

fn promise_is_resolved(value: NapiValue) -> bool {
    let Ok(Value::Promise(state)) = (unsafe { value_ref(value) }) else {
        return false;
    };
    matches!(*state.borrow(), PromiseState::Resolved(_))
}

unsafe extern "C" fn threadsafe_js_callback(env: NapiEnv, info: NapiCallbackInfo) -> NapiValue {
    let info = info.as_ref().unwrap();
    let probe = &*(info.data as *const ThreadsafeProbe);
    let value = match value_ref(info.args[0]).unwrap() {
        Value::Number(value) => *value,
        _ => panic!("thread-safe callback did not receive a number"),
    };
    probe.values.lock().unwrap().push(value);
    probe
        .callback_threads
        .lock()
        .unwrap()
        .push(std::thread::current().id());
    let mut undefined = ptr::null_mut();
    assert_eq!(napi_get_undefined(env, &mut undefined), NAPI_OK);
    undefined
}

unsafe extern "C" fn threadsafe_call_js(
    env: NapiEnv,
    function: NapiValue,
    context: *mut c_void,
    data: *mut c_void,
) {
    let probe = &*(context as *const ThreadsafeProbe);
    let value = *Box::from_raw(data as *mut f64);
    if env.is_null() {
        probe.aborted.fetch_add(1, Ordering::AcqRel);
        return;
    }
    let mut argument = ptr::null_mut();
    assert_eq!(napi_create_double(env, value, &mut argument), NAPI_OK);
    let mut undefined = ptr::null_mut();
    assert_eq!(napi_get_undefined(env, &mut undefined), NAPI_OK);
    assert_eq!(
        napi_call_function(env, undefined, function, 1, &argument, ptr::null_mut()),
        NAPI_OK
    );
}

unsafe extern "C" fn threadsafe_finalize(_env: NapiEnv, data: *mut c_void, _hint: *mut c_void) {
    let probe = &*(data as *const ThreadsafeProbe);
    probe.finalized.store(true, Ordering::Release);
}

unsafe extern "C" fn probe_execute(_env: NapiEnv, data: *mut c_void) {
    let probe = &*(data as *const AsyncProbe);
    *probe.execute_thread.lock().unwrap() = Some(std::thread::current().id());
    probe.executed.store(true, Ordering::Release);
}

unsafe extern "C" fn probe_complete(_env: NapiEnv, status: NapiStatus, data: *mut c_void) {
    assert_eq!(status, NAPI_OK);
    let probe = &*(data as *const AsyncProbe);
    assert!(probe.executed.load(Ordering::Acquire));
    *probe.complete_thread.lock().unwrap() = Some(std::thread::current().id());
}

unsafe extern "C" fn blocking_execute(_env: NapiEnv, data: *mut c_void) {
    let gate = &*(data as *const Arc<WorkerGate>);
    let mut state = gate.state.lock().unwrap();
    state.0 += 1;
    gate.changed.notify_all();
    while !state.1 {
        state = gate.changed.wait(state).unwrap();
    }
}

unsafe extern "C" fn cancelled_execute(_env: NapiEnv, data: *mut c_void) {
    let probe = &*(data as *const CancelProbe);
    probe.executed.store(true, Ordering::Release);
}

unsafe extern "C" fn cancelled_complete(_env: NapiEnv, status: NapiStatus, data: *mut c_void) {
    let probe = &*(data as *const CancelProbe);
    probe.status.store(status, Ordering::Release);
}

#[test]
fn threadsafe_function_queues_worker_calls_and_finalizes_on_main_thread() {
    let _guard = lock_async_test();
    let mut env = Env::new();
    let probe = Box::into_raw(Box::new(ThreadsafeProbe {
        main_thread: std::thread::current().id(),
        callback_threads: Mutex::new(Vec::new()),
        values: Mutex::new(Vec::new()),
        aborted: AtomicUsize::new(0),
        finalized: AtomicBool::new(false),
    }));
    let function = env.alloc(Value::Function(Function {
        callback: threadsafe_js_callback,
        data: probe.cast(),
        properties: HashMap::new(),
        _thaw_bridge: None,
    }));
    let mut threadsafe = ptr::null_mut();
    unsafe {
        assert_eq!(
            napi_create_threadsafe_function(
                &mut env,
                function,
                ptr::null_mut(),
                ptr::null_mut(),
                1,
                1,
                probe.cast(),
                Some(threadsafe_finalize),
                probe.cast(),
                Some(threadsafe_call_js),
                &mut threadsafe,
            ),
            NAPI_OK
        );
        let mut context = ptr::null_mut();
        assert_eq!(
            napi_get_threadsafe_function_context(threadsafe, &mut context),
            NAPI_OK
        );
        assert_eq!(context, probe.cast());
        assert_eq!(napi_acquire_threadsafe_function(threadsafe), NAPI_OK);
        assert_eq!(napi_release_threadsafe_function(threadsafe, 0), NAPI_OK);
        assert_eq!(thaw_napi_unload_all(), 0);
    }
    let address = threadsafe as usize;
    let worker = std::thread::spawn(move || unsafe {
        let threadsafe = address as *mut ThreadsafeFunction;
        let first = Box::into_raw(Box::new(20.0));
        assert_eq!(
            napi_call_threadsafe_function(threadsafe, first.cast(), 0),
            NAPI_OK
        );
        let second = Box::into_raw(Box::new(22.0));
        assert_eq!(
            napi_call_threadsafe_function(threadsafe, second.cast(), 1),
            NAPI_OK
        );
        assert_eq!(napi_release_threadsafe_function(threadsafe, 0), NAPI_OK);
    });

    assert_eq!(thaw_napi_run_async_work(), 2);
    worker.join().unwrap();
    unsafe {
        assert_eq!(
            napi_call_threadsafe_function(threadsafe, ptr::null_mut(), 0),
            NAPI_CLOSING
        );
        assert_eq!(napi_acquire_threadsafe_function(threadsafe), NAPI_CLOSING);
        assert_eq!(
            napi_release_threadsafe_function(threadsafe, 0),
            NAPI_CLOSING
        );
        assert_eq!(
            napi_ref_threadsafe_function(&mut env, threadsafe),
            NAPI_CLOSING
        );
        assert_eq!(
            napi_unref_threadsafe_function(&mut env, threadsafe),
            NAPI_CLOSING
        );
        let mut context = ptr::null_mut();
        assert_eq!(
            napi_get_threadsafe_function_context(threadsafe, &mut context),
            NAPI_OK
        );
        assert_eq!(context, probe.cast());
    }
    let probe = unsafe { Box::from_raw(probe) };
    assert_eq!(*probe.values.lock().unwrap(), vec![20.0, 22.0]);
    assert!(probe
        .callback_threads
        .lock()
        .unwrap()
        .iter()
        .all(|thread| *thread == probe.main_thread));
    assert!(probe.finalized.load(Ordering::Acquire));
    assert_eq!(probe.aborted.load(Ordering::Acquire), 0);
    assert_eq!(thaw_napi_unload_all(), 1);
}

#[test]
fn env_cleanup_hooks_run_in_reverse_and_can_be_removed() {
    let output = Arc::new(Mutex::new(Vec::new()));
    let first = Box::into_raw(Box::new(CleanupProbe {
        output: Arc::clone(&output),
        value: 1,
    }));
    let removed = Box::into_raw(Box::new(CleanupProbe {
        output: Arc::clone(&output),
        value: 2,
    }));
    let last = Box::into_raw(Box::new(CleanupProbe {
        output: Arc::clone(&output),
        value: 3,
    }));
    let mut env = Env::new();
    unsafe {
        assert_eq!(
            napi_add_env_cleanup_hook(&mut env, Some(cleanup_probe), first.cast()),
            NAPI_OK
        );
        assert_eq!(
            napi_add_env_cleanup_hook(&mut env, Some(cleanup_probe), removed.cast()),
            NAPI_OK
        );
        assert_eq!(
            napi_add_env_cleanup_hook(&mut env, Some(cleanup_probe), last.cast()),
            NAPI_OK
        );
        assert_eq!(
            napi_remove_env_cleanup_hook(&mut env, Some(cleanup_probe), removed.cast()),
            NAPI_OK
        );
        drop(Box::from_raw(removed));
    }
    drop(env);
    assert_eq!(*output.lock().unwrap(), vec![3, 1]);
}

#[test]
fn async_cleanup_hooks_complete_in_reverse_and_can_be_removed() {
    let output = Arc::new(Mutex::new(Vec::new()));
    let first = Box::into_raw(Box::new(CleanupProbe {
        output: Arc::clone(&output),
        value: 1,
    }));
    let removed = Box::into_raw(Box::new(CleanupProbe {
        output: Arc::clone(&output),
        value: 2,
    }));
    let last = Box::into_raw(Box::new(CleanupProbe {
        output: Arc::clone(&output),
        value: 3,
    }));
    let mut env = Env::new();
    let mut removed_handle = ptr::null_mut();
    unsafe {
        assert_eq!(
            napi_add_async_cleanup_hook(
                &mut env,
                Some(async_cleanup_probe),
                first.cast(),
                ptr::null_mut(),
            ),
            NAPI_OK
        );
        assert_eq!(
            napi_add_async_cleanup_hook(
                &mut env,
                Some(async_cleanup_probe),
                removed.cast(),
                &mut removed_handle,
            ),
            NAPI_OK
        );
        assert_eq!(
            napi_add_async_cleanup_hook(
                &mut env,
                Some(async_cleanup_probe),
                last.cast(),
                ptr::null_mut(),
            ),
            NAPI_OK
        );
        assert_eq!(napi_remove_async_cleanup_hook(removed_handle), NAPI_OK);
        assert_eq!(
            napi_remove_async_cleanup_hook(removed_handle),
            NAPI_INVALID_ARG
        );
        drop(Box::from_raw(removed));
    }
    drop(env);
    assert_eq!(*output.lock().unwrap(), vec![3, 1]);
    assert_eq!(ACTIVE_ASYNC_CLEANUP_HOOKS.load(Ordering::Acquire), 0);
}

#[test]
fn asynchronous_cleanup_remains_active_until_handle_removal() {
    let _guard = lock_async_test();
    HELD_ASYNC_CLEANUP.store(0, Ordering::Release);
    let mut env = Env::new();
    unsafe {
        assert_eq!(
            napi_add_async_cleanup_hook(
                &mut env,
                Some(hold_async_cleanup),
                ptr::null_mut(),
                ptr::null_mut(),
            ),
            NAPI_OK
        );
    }
    drop(env);
    let handle = HELD_ASYNC_CLEANUP.swap(0, Ordering::AcqRel);
    assert_ne!(handle, 0);
    assert_eq!(ACTIVE_ASYNC_CLEANUP_HOOKS.load(Ordering::Acquire), 1);
    unsafe {
        assert_eq!(
            napi_remove_async_cleanup_hook(handle as *mut AsyncCleanupHookHandle),
            NAPI_OK
        );
    }
    assert_eq!(ACTIVE_ASYNC_CLEANUP_HOOKS.load(Ordering::Acquire), 0);
}

#[test]
fn posted_finalizers_run_from_the_main_poller_with_live_env() {
    let _guard = lock_async_test();
    POSTED_FINALIZER_RAN.store(false, Ordering::Release);
    let mut env = Box::new(Env::new());
    let env_ptr: NapiEnv = &mut *env;
    HOST.with(|host| host.borrow_mut().pending_call_envs.push(env));
    unsafe {
        assert_eq!(
            node_api_post_finalizer(
                env_ptr,
                Some(posted_finalizer_uses_napi),
                ptr::null_mut(),
                ptr::null_mut(),
            ),
            NAPI_OK
        );
    }
    assert_eq!(thaw_napi_poll_async_work(), 1);
    assert!(POSTED_FINALIZER_RAN.load(Ordering::Acquire));
}

#[test]
fn fatal_exception_sets_a_single_process_failure_status() {
    let mut env = Env::new();
    let error = env.alloc(Value::Error("callback failed".into()));
    unsafe {
        assert_eq!(napi_fatal_exception(&mut env, error), NAPI_OK);
    }
    assert_eq!(thaw_napi_take_fatal_exception(), 1);
    assert_eq!(thaw_napi_take_fatal_exception(), 0);
}

#[test]
fn threadsafe_function_reports_full_deadlock_and_abort_cleanup() {
    let _guard = lock_async_test();
    let mut env = Env::new();
    let probe = Box::into_raw(Box::new(ThreadsafeProbe {
        main_thread: std::thread::current().id(),
        callback_threads: Mutex::new(Vec::new()),
        values: Mutex::new(Vec::new()),
        aborted: AtomicUsize::new(0),
        finalized: AtomicBool::new(false),
    }));
    let function = env.alloc(Value::Function(Function {
        callback: threadsafe_js_callback,
        data: probe.cast(),
        properties: HashMap::new(),
        _thaw_bridge: None,
    }));
    let mut threadsafe = ptr::null_mut();
    unsafe {
        assert_eq!(
            napi_create_threadsafe_function(
                &mut env,
                function,
                ptr::null_mut(),
                ptr::null_mut(),
                1,
                1,
                probe.cast(),
                Some(threadsafe_finalize),
                probe.cast(),
                Some(threadsafe_call_js),
                &mut threadsafe,
            ),
            NAPI_OK
        );
        let queued = Box::into_raw(Box::new(1.0));
        assert_eq!(
            napi_call_threadsafe_function(threadsafe, queued.cast(), 0),
            NAPI_OK
        );
        let full = Box::into_raw(Box::new(2.0));
        assert_eq!(
            napi_call_threadsafe_function(threadsafe, full.cast(), 0),
            NAPI_QUEUE_FULL
        );
        let mut info = ptr::null();
        assert_eq!(napi_get_last_error_info(&mut env, &mut info), NAPI_OK);
        assert_eq!((*info).error_code, NAPI_QUEUE_FULL);
        drop(Box::from_raw(full));
        let deadlock = Box::into_raw(Box::new(2.0));
        assert_eq!(
            napi_call_threadsafe_function(threadsafe, deadlock.cast(), 1),
            NAPI_WOULD_DEADLOCK
        );
        assert_eq!(napi_get_last_error_info(&mut env, &mut info), NAPI_OK);
        assert_eq!((*info).error_code, NAPI_WOULD_DEADLOCK);
        drop(Box::from_raw(deadlock));
        assert_eq!(
            napi_unref_threadsafe_function(&mut env, threadsafe),
            NAPI_OK
        );
        assert_eq!(napi_ref_threadsafe_function(&mut env, threadsafe), NAPI_OK);
        assert_eq!(napi_release_threadsafe_function(threadsafe, 1), NAPI_OK);
        assert_eq!(
            napi_call_threadsafe_function(threadsafe, ptr::null_mut(), 0),
            NAPI_CLOSING
        );
        assert_eq!(napi_get_last_error_info(&mut env, &mut info), NAPI_OK);
        assert_eq!((*info).error_code, NAPI_CLOSING);
    }
    assert_eq!(thaw_napi_run_async_work(), 0);
    let probe = unsafe { Box::from_raw(probe) };
    assert_eq!(probe.aborted.load(Ordering::Acquire), 1);
    assert!(probe.finalized.load(Ordering::Acquire));
    assert!(probe.values.lock().unwrap().is_empty());
}

#[test]
fn threadsafe_function_serializes_multiple_producers() {
    let _guard = lock_async_test();
    let mut env = Env::new();
    let probe = Box::into_raw(Box::new(ThreadsafeProbe {
        main_thread: std::thread::current().id(),
        callback_threads: Mutex::new(Vec::new()),
        values: Mutex::new(Vec::new()),
        aborted: AtomicUsize::new(0),
        finalized: AtomicBool::new(false),
    }));
    let function = env.alloc(Value::Function(Function {
        callback: threadsafe_js_callback,
        data: probe.cast(),
        properties: HashMap::new(),
        _thaw_bridge: None,
    }));
    let mut threadsafe = ptr::null_mut();
    unsafe {
        assert_eq!(
            napi_create_threadsafe_function(
                &mut env,
                function,
                ptr::null_mut(),
                ptr::null_mut(),
                8,
                4,
                probe.cast(),
                Some(threadsafe_finalize),
                probe.cast(),
                Some(threadsafe_call_js),
                &mut threadsafe,
            ),
            NAPI_OK
        );
    }
    let address = threadsafe as usize;
    let workers = (0..4)
        .map(|producer| {
            std::thread::spawn(move || unsafe {
                let threadsafe = address as *mut ThreadsafeFunction;
                for index in 0..25 {
                    let value = Box::into_raw(Box::new((producer * 25 + index) as f64));
                    assert_eq!(
                        napi_call_threadsafe_function(threadsafe, value.cast(), 1),
                        NAPI_OK
                    );
                }
                assert_eq!(napi_release_threadsafe_function(threadsafe, 0), NAPI_OK);
            })
        })
        .collect::<Vec<_>>();
    assert_eq!(thaw_napi_run_async_work(), 100);
    for worker in workers {
        worker.join().unwrap();
    }
    let probe = unsafe { Box::from_raw(probe) };
    let mut values = probe.values.lock().unwrap().clone();
    values.sort_by(|left, right| left.total_cmp(right));
    assert_eq!(
        values,
        (0..100).map(|value| value as f64).collect::<Vec<_>>()
    );
    assert!(probe.finalized.load(Ordering::Acquire));
}

#[test]
fn async_work_executes_on_a_worker_and_completes_on_the_draining_thread() {
    let _guard = lock_async_test();
    let mut env = Env::new();
    let probe = Box::new(AsyncProbe {
        main_thread: std::thread::current().id(),
        execute_thread: Mutex::new(None),
        complete_thread: Mutex::new(None),
        executed: AtomicBool::new(false),
    });
    let probe_ptr = Box::into_raw(probe);
    let mut work = ptr::null_mut();
    unsafe {
        assert_eq!(
            napi_create_async_work(
                &mut env,
                ptr::null_mut(),
                ptr::null_mut(),
                Some(probe_execute),
                Some(probe_complete),
                probe_ptr.cast(),
                &mut work,
            ),
            NAPI_OK
        );
        assert_eq!(napi_queue_async_work(&mut env, work), NAPI_OK);
        assert_eq!(napi_queue_async_work(&mut env, work), NAPI_GENERIC_FAILURE);
    }
    assert_eq!(thaw_napi_run_async_work(), 1);

    let probe = unsafe { Box::from_raw(probe_ptr) };
    assert_ne!(
        *probe.execute_thread.lock().unwrap(),
        Some(probe.main_thread)
    );
    assert_eq!(
        *probe.complete_thread.lock().unwrap(),
        Some(probe.main_thread)
    );
    unsafe {
        let mut other_env = Env::new();
        assert_eq!(
            napi_delete_async_work(&mut other_env, work),
            NAPI_INVALID_ARG
        );
        assert_eq!(napi_delete_async_work(&mut env, work), NAPI_OK);
        assert_eq!(napi_delete_async_work(&mut env, work), NAPI_GENERIC_FAILURE);
        assert_eq!(napi_queue_async_work(&mut env, work), NAPI_GENERIC_FAILURE);
        assert_eq!(napi_cancel_async_work(&mut env, work), NAPI_GENERIC_FAILURE);
    }

    let worker_count = async_worker_count();
    let gate = Arc::new(WorkerGate {
        state: Mutex::new((0, false)),
        changed: Condvar::new(),
    });
    let mut blockers = Vec::new();
    for _ in 0..worker_count {
        let gate_data = Box::into_raw(Box::new(Arc::clone(&gate)));
        let mut blocker = ptr::null_mut();
        unsafe {
            assert_eq!(
                napi_create_async_work(
                    &mut env,
                    ptr::null_mut(),
                    ptr::null_mut(),
                    Some(blocking_execute),
                    None,
                    gate_data.cast(),
                    &mut blocker,
                ),
                NAPI_OK
            );
            assert_eq!(napi_queue_async_work(&mut env, blocker), NAPI_OK);
        }
        blockers.push((blocker, gate_data));
    }
    let mut gate_state = gate.state.lock().unwrap();
    while gate_state.0 != worker_count {
        gate_state = gate.changed.wait(gate_state).unwrap();
    }
    drop(gate_state);

    let cancel_probe = Box::into_raw(Box::new(CancelProbe {
        executed: AtomicBool::new(false),
        status: AtomicI32::new(NAPI_OK),
    }));
    let mut cancelled_work = ptr::null_mut();
    unsafe {
        assert_eq!(
            napi_create_async_work(
                &mut env,
                ptr::null_mut(),
                ptr::null_mut(),
                Some(cancelled_execute),
                Some(cancelled_complete),
                cancel_probe.cast(),
                &mut cancelled_work,
            ),
            NAPI_OK
        );
        assert_eq!(napi_queue_async_work(&mut env, cancelled_work), NAPI_OK);
        assert_eq!(napi_cancel_async_work(&mut env, cancelled_work), NAPI_OK);
    }
    let mut gate_state = gate.state.lock().unwrap();
    gate_state.1 = true;
    gate.changed.notify_all();
    drop(gate_state);
    assert_eq!(thaw_napi_run_async_work(), worker_count + 1);
    let cancel_probe = unsafe { Box::from_raw(cancel_probe) };
    assert!(!cancel_probe.executed.load(Ordering::Acquire));
    assert_eq!(cancel_probe.status.load(Ordering::Acquire), NAPI_CANCELLED);
    unsafe {
        assert_eq!(napi_delete_async_work(&mut env, cancelled_work), NAPI_OK);
        for (blocker, gate_data) in blockers {
            assert_eq!(napi_delete_async_work(&mut env, blocker), NAPI_OK);
            drop(Box::from_raw(gate_data));
        }
    }
}

#[test]
fn async_creation_validates_resources_names_and_callback_environment() {
    unsafe {
        let mut env = Env::new();
        let env_ptr: NapiEnv = &mut env;
        let mut foreign_env = Env::new();
        let foreign_resource = foreign_env.alloc(Value::Object(HashMap::new()));
        let foreign_name = foreign_env.alloc(Value::String("foreign-work".into()));
        let number_name = env.alloc(Value::Number(1.0));
        let mut work = ptr::null_mut();
        assert_eq!(
            napi_create_async_work(
                env_ptr,
                foreign_resource,
                ptr::null_mut(),
                Some(probe_execute),
                None,
                ptr::null_mut(),
                &mut work
            ),
            NAPI_INVALID_ARG
        );
        assert_eq!(
            napi_create_async_work(
                env_ptr,
                ptr::null_mut(),
                foreign_name,
                Some(probe_execute),
                None,
                ptr::null_mut(),
                &mut work
            ),
            NAPI_INVALID_ARG
        );
        assert_eq!(
            napi_create_async_work(
                env_ptr,
                ptr::null_mut(),
                number_name,
                Some(probe_execute),
                None,
                ptr::null_mut(),
                &mut work
            ),
            NAPI_STRING_EXPECTED
        );

        let mut foreign_function = ptr::null_mut();
        assert_eq!(
            napi_create_function(
                &mut foreign_env,
                c"foreign".as_ptr(),
                7,
                Some(threadsafe_js_callback),
                ptr::null_mut(),
                &mut foreign_function
            ),
            NAPI_OK
        );
        let mut threadsafe = ptr::null_mut();
        assert_eq!(
            napi_create_threadsafe_function(
                env_ptr,
                foreign_function,
                ptr::null_mut(),
                ptr::null_mut(),
                0,
                1,
                ptr::null_mut(),
                None,
                ptr::null_mut(),
                None,
                &mut threadsafe
            ),
            NAPI_INVALID_ARG
        );
        assert_eq!(
            napi_create_threadsafe_function(
                env_ptr,
                ptr::null_mut(),
                ptr::null_mut(),
                number_name,
                0,
                1,
                ptr::null_mut(),
                None,
                ptr::null_mut(),
                Some(threadsafe_call_js),
                &mut threadsafe
            ),
            NAPI_STRING_EXPECTED
        );
    }
}

#[test]
fn loads_and_calls_a_real_napi_addon() {
    let dir = std::env::temp_dir().join(format!("thaw-napi-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let source = dir.join("addon.c");
    let addon = dir.join("addon.node");
    std::fs::write(&source, r#"
            #include <stddef.h>
            #include <stdlib.h>
            typedef void* napi_env; typedef void* napi_value; typedef void* napi_callback_info;
            typedef int napi_status;
            extern napi_status napi_get_cb_info(napi_env, napi_callback_info, size_t*, napi_value*, napi_value*, void**);
            extern napi_status napi_get_value_double(napi_env, napi_value, double*);
            extern napi_status napi_get_value_bool(napi_env, napi_value, _Bool*);
            extern napi_status napi_get_value_string_utf8(napi_env, napi_value, char*, size_t, size_t*);
            extern napi_status napi_create_double(napi_env, double, napi_value*);
            extern napi_status napi_get_boolean(napi_env, _Bool, napi_value*);
            extern napi_status napi_create_string_utf8(napi_env, const char*, size_t, napi_value*);
            extern napi_status napi_create_function(napi_env, const char*, size_t, napi_value (*)(napi_env,napi_callback_info), void*, napi_value*);
            extern napi_status napi_set_named_property(napi_env, napi_value, const char*, napi_value);
            extern napi_status napi_get_named_property(napi_env, napi_value, const char*, napi_value*);
            extern napi_status napi_call_function(napi_env, napi_value, napi_value, size_t, const napi_value*, napi_value*);
            extern napi_status napi_wrap(napi_env, napi_value, void*, void (*)(napi_env,void*,void*), void*, void**);
            extern napi_status napi_unwrap(napi_env, napi_value, void**);
            extern napi_status napi_define_class(napi_env, const char*, size_t, napi_value (*)(napi_env,napi_callback_info), void*, size_t, const void*, napi_value*);
            extern napi_status napi_new_instance(napi_env, napi_value, size_t, const napi_value*, napi_value*);
            extern napi_status napi_instanceof(napi_env, napi_value, napi_value, _Bool*);
            extern napi_status node_api_get_module_file_name(napi_env, const char**);
            typedef napi_value (*napi_callback)(napi_env,napi_callback_info);
            typedef struct { const char* utf8name; napi_value name; napi_callback method; napi_callback getter; napi_callback setter; napi_value value; unsigned attributes; void* data; } napi_property_descriptor;
            typedef struct { double value; } native_box;
            static napi_value box_constructor;
            static int finalized_count;
            static void finalize_box(napi_env env, void* data, void* hint) {
                (void)env; (void)hint; free(data); finalized_count++;
            }
            static napi_value box_new(napi_env env, napi_callback_info info) {
                size_t argc = 1; napi_value arg, self; double value;
                napi_get_cb_info(env, info, &argc, &arg, &self, 0);
                napi_get_value_double(env, arg, &value);
                native_box* box = malloc(sizeof(*box)); box->value = value;
                napi_wrap(env, self, box, finalize_box, 0, 0); return self;
            }
            static napi_value box_get(napi_env env, napi_callback_info info) {
                size_t argc = 0; napi_value self, result; native_box* box;
                napi_get_cb_info(env, info, &argc, 0, &self, 0);
                napi_unwrap(env, self, (void**)&box);
                napi_create_double(env, box->value, &result); return result;
            }
            static napi_value roundtrip(napi_env env, napi_callback_info info) {
                size_t argc = 1; napi_value arg, instance, method, result, property; double a, b; _Bool matches;
                napi_get_cb_info(env, info, &argc, &arg, 0, 0);
                napi_new_instance(env, box_constructor, 1, &arg, &instance);
                napi_instanceof(env, instance, box_constructor, &matches);
                if (!matches) return 0;
                napi_get_named_property(env, instance, "get", &method);
                napi_call_function(env, instance, method, 0, 0, &result);
                napi_get_named_property(env, instance, "value", &property);
                napi_get_value_double(env, result, &a); napi_get_value_double(env, property, &b);
                napi_create_double(env, a + b, &result); return result;
            }
            static napi_value finalized(napi_env env, napi_callback_info info) {
                (void)info; napi_value result;
                napi_create_double(env, finalized_count, &result); return result;
            }
            static napi_value add(napi_env env, napi_callback_info info) {
                size_t argc = 2; napi_value argv[2]; double a, b; napi_value result;
                napi_get_cb_info(env, info, &argc, argv, 0, 0);
                napi_get_value_double(env, argv[0], &a); napi_get_value_double(env, argv[1], &b);
                napi_create_double(env, a + b, &result); return result;
            }
            static napi_value negate(napi_env env, napi_callback_info info) {
                size_t argc = 1; napi_value arg, result; _Bool value;
                napi_get_cb_info(env, info, &argc, &arg, 0, 0);
                napi_get_value_bool(env, arg, &value); napi_get_boolean(env, !value, &result); return result;
            }
            static napi_value echo(napi_env env, napi_callback_info info) {
                size_t argc = 1, length; napi_value arg, result; char text[64];
                napi_get_cb_info(env, info, &argc, &arg, 0, 0);
                napi_get_value_string_utf8(env, arg, text, sizeof(text), &length);
                napi_create_string_utf8(env, text, length, &result); return result;
            }
            static napi_value module_file(napi_env env, napi_callback_info info) {
                (void)info; const char* path; napi_value result;
                node_api_get_module_file_name(env, &path);
                napi_create_string_utf8(env, path, (size_t)-1, &result); return result;
            }
            __attribute__((visibility("default"))) napi_value napi_register_module_v1(napi_env env, napi_value exports) {
                napi_value fn; napi_property_descriptor properties[2] = {
                    { "get", 0, box_get, 0, 0, 0, 0, 0 },
                    { "value", 0, 0, box_get, 0, 0, 0, 0 }
                };
                napi_define_class(env, "NativeBox", 9, box_new, 0, 2, properties, &box_constructor);
                napi_set_named_property(env, exports, "NativeBox", box_constructor);
                napi_create_function(env, "roundtrip", 9, roundtrip, 0, &fn); napi_set_named_property(env, exports, "roundtrip", fn);
                napi_create_function(env, "finalized", 9, finalized, 0, &fn); napi_set_named_property(env, exports, "finalized", fn);
                napi_create_function(env, "add", 3, add, 0, &fn); napi_set_named_property(env, exports, "add", fn);
                napi_create_function(env, "negate", 6, negate, 0, &fn); napi_set_named_property(env, exports, "negate", fn);
                napi_create_function(env, "echo", 4, echo, 0, &fn); napi_set_named_property(env, exports, "echo", fn);
                napi_create_function(env, "moduleFile", 10, module_file, 0, &fn); napi_set_named_property(env, exports, "moduleFile", fn);
                return exports;
            }
        "#).unwrap();
    assert!(Command::new("cc")
        .args(["-shared", "-fPIC"])
        .arg(&source)
        .arg("-o")
        .arg(&addon)
        .status()
        .unwrap()
        .success());
    let path = CString::new(addon.to_string_lossy().as_bytes()).unwrap();
    let name = CString::new("add").unwrap();
    let args = CString::new("[20,22]").unwrap();
    unsafe {
        assert_eq!(thaw_napi_load(path.as_ptr()), 1);
        let result = thaw_napi_call_result(name.as_ptr(), args.as_ptr());
        assert!(result.error.is_null());
        assert_eq!(CStr::from_ptr(result.value).to_str().unwrap(), "42.0");
        let negate = CString::new("negate").unwrap();
        let boolean = CString::new("[true]").unwrap();
        let result = thaw_napi_call_result(negate.as_ptr(), boolean.as_ptr());
        assert_eq!(CStr::from_ptr(result.value).to_str().unwrap(), "false");
        let echo = CString::new("echo").unwrap();
        let string = CString::new("[\"hello\"]").unwrap();
        let result = thaw_napi_call_result(echo.as_ptr(), string.as_ptr());
        assert_eq!(CStr::from_ptr(result.value).to_str().unwrap(), "\"hello\"");
        let result = thaw_napi_call_result(c"moduleFile".as_ptr(), c"[]".as_ptr());
        let module_file: String =
            serde_json::from_str(CStr::from_ptr(result.value).to_str().unwrap()).unwrap();
        assert!(module_file.starts_with("file://"));
        assert!(module_file.ends_with("addon.node"));
        let result = thaw_napi_call_result(c"roundtrip".as_ptr(), c"[42]".as_ptr());
        assert!(result.error.is_null());
        assert_eq!(CStr::from_ptr(result.value).to_str().unwrap(), "84.0");
        let result = thaw_napi_call_result(c"finalized".as_ptr(), c"[]".as_ptr());
        assert!(result.error.is_null());
        assert_eq!(CStr::from_ptr(result.value).to_str().unwrap(), "1.0");
        let constructor = thaw_napi_get_export(c"NativeBox".as_ptr());
        assert_ne!(constructor, 0);
        let instance = thaw_napi_construct_handle_result(constructor, c"[21]".as_ptr());
        assert!(instance.error.is_null());
        assert_ne!(instance.value, 0);
        let result = thaw_napi_call_method_result(instance.value, c"get".as_ptr(), c"[]".as_ptr());
        assert!(result.error.is_null());
        assert_eq!(CStr::from_ptr(result.value).to_str().unwrap(), "21.0");
    }
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn real_c_addon_calls_back_through_a_threadsafe_function() {
    let _guard = lock_async_test();
    let dir = std::env::temp_dir().join(format!("thaw-napi-tsfn-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let source = dir.join("addon.c");
    let addon = dir.join("addon.node");
    std::fs::write(
            &source,
            r#"
            #include <pthread.h>
            #include <stddef.h>
            #include <stdlib.h>
            typedef void* napi_env; typedef void* napi_value; typedef void* napi_callback_info;
            typedef void* napi_threadsafe_function; typedef int napi_status;
            typedef napi_value (*napi_callback)(napi_env,napi_callback_info);
            typedef void (*napi_finalize)(napi_env,void*,void*);
            typedef void (*napi_threadsafe_function_call_js)(napi_env,napi_value,void*,void*);
            extern napi_status napi_get_cb_info(napi_env,napi_callback_info,size_t*,napi_value*,napi_value*,void**);
            extern napi_status napi_get_null(napi_env,napi_value*);
            extern napi_status napi_get_undefined(napi_env,napi_value*);
            extern napi_status napi_create_double(napi_env,double,napi_value*);
            extern napi_status napi_call_function(napi_env,napi_value,napi_value,size_t,const napi_value*,napi_value*);
            extern napi_status napi_create_function(napi_env,const char*,size_t,napi_callback,void*,napi_value*);
            extern napi_status napi_set_named_property(napi_env,napi_value,const char*,napi_value);
            extern napi_status napi_create_threadsafe_function(napi_env,napi_value,napi_value,napi_value,size_t,size_t,void*,napi_finalize,void*,napi_threadsafe_function_call_js,napi_threadsafe_function*);
            extern napi_status napi_call_threadsafe_function(napi_threadsafe_function,void*,int);
            extern napi_status napi_release_threadsafe_function(napi_threadsafe_function,int);

            static void call_js(napi_env env, napi_value callback, void* context, void* data) {
                (void)context; napi_value recv, args[2];
                napi_get_undefined(env, &recv); napi_get_null(env, &args[0]);
                napi_create_double(env, *(double*)data, &args[1]); free(data);
                napi_call_function(env, recv, callback, 2, args, 0);
            }
            static void* worker(void* raw) {
                napi_threadsafe_function tsfn = raw;
                double* value = malloc(sizeof(*value)); *value = 42;
                napi_call_threadsafe_function(tsfn, value, 1);
                napi_release_threadsafe_function(tsfn, 0); return 0;
            }
            static napi_value queue(napi_env env, napi_callback_info info) {
                size_t argc = 1; napi_value callback, result; napi_threadsafe_function tsfn;
                pthread_t thread; napi_get_cb_info(env, info, &argc, &callback, 0, 0);
                if (napi_create_threadsafe_function(env, callback, 0, 0, 1, 1, 0, 0, 0, call_js, &tsfn) != 0) return 0;
                pthread_create(&thread, 0, worker, tsfn); pthread_detach(thread);
                napi_get_undefined(env, &result); return result;
            }
            __attribute__((visibility("default"))) napi_value napi_register_module_v1(napi_env env, napi_value exports) {
                napi_value fn; napi_create_function(env, "tsfn_queue", 10, queue, 0, &fn);
                napi_set_named_property(env, exports, "tsfn_queue", fn); return exports;
            }
            "#,
        )
        .unwrap();
    assert!(Command::new("cc")
        .args(["-shared", "-fPIC", "-pthread"])
        .arg(&source)
        .arg("-o")
        .arg(&addon)
        .status()
        .unwrap()
        .success());
    let path = CString::new(addon.to_string_lossy().as_bytes()).unwrap();
    let output: *mut Mutex<Option<(JsonValue, JsonValue)>> =
        Box::into_raw(Box::new(Mutex::new(None)));
    unsafe {
        assert_eq!(thaw_napi_load(path.as_ptr()), 1);
        let queued = thaw_napi_call_with_callback_result(
            c"tsfn_queue".as_ptr(),
            c"[]".as_ptr(),
            Some(bcrypt_bridge_callback),
            output.cast(),
        );
        assert!(queued.error.is_null());
        assert_eq!(thaw_napi_run_async_work(), 1);
        let output = Box::from_raw(output);
        let (error, result) = output.lock().unwrap().take().unwrap();
        assert!(error.is_null());
        assert_eq!(result, JsonValue::from(42.0));
    }
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn runs_utf8_validate_prebuild_when_supplied() {
    let Ok(path) = std::env::var("THAW_UTF8_VALIDATE_NODE") else {
        return;
    };
    let path = CString::new(path).unwrap();
    let name = CString::new("isValidUTF8").unwrap();
    let valid = CString::new(r#"[{"type":"Buffer","data":[240,144,128,128]}]"#).unwrap();
    let invalid = CString::new(r#"[{"type":"Buffer","data":[255]}]"#).unwrap();
    unsafe {
        assert_eq!(thaw_napi_load_named(path.as_ptr(), name.as_ptr()), 1);
        let result = thaw_napi_call_result(name.as_ptr(), valid.as_ptr());
        assert!(result.error.is_null());
        assert_eq!(CStr::from_ptr(result.value).to_str().unwrap(), "true");
        let result = thaw_napi_call_result(name.as_ptr(), invalid.as_ptr());
        assert!(result.error.is_null());
        assert_eq!(CStr::from_ptr(result.value).to_str().unwrap(), "false");
    }
}

#[test]
fn runs_bcrypt_prebuild_when_supplied() {
    let _guard = lock_async_test();
    let Ok(path) = std::env::var("THAW_BCRYPT_NODE") else {
        return;
    };
    let path = CString::new(path).unwrap();
    unsafe {
        assert_eq!(thaw_napi_load(path.as_ptr()), 1);
        let salt_args = CString::new(
            r#"["b",4,{"type":"Buffer","data":[0,1,2,3,4,5,6,7,8,9,10,11,12,13,14,15]}]"#,
        )
        .unwrap();
        let salt_name = c"gen_salt_sync";
        let salt = thaw_napi_call_result(salt_name.as_ptr(), salt_args.as_ptr());
        assert!(salt.error.is_null());
        let salt_json = CStr::from_ptr(salt.value).to_string_lossy().into_owned();
        let salt: String = serde_json::from_str(&salt_json).unwrap();
        assert!(salt.starts_with("$2b$04$"), "{salt}");

        let encrypt_args =
            CString::new(serde_json::to_string(&serde_json::json!(["password", salt])).unwrap())
                .unwrap();
        let encrypted = thaw_napi_call_result(c"encrypt_sync".as_ptr(), encrypt_args.as_ptr());
        assert!(encrypted.error.is_null());
        let hash_json = CStr::from_ptr(encrypted.value)
            .to_string_lossy()
            .into_owned();
        let hash: String = serde_json::from_str(&hash_json).unwrap();
        assert!(hash.starts_with("$2b$04$"), "{hash}");

        let (function, env_address) = HOST.with(|host| {
            let mut host = host.borrow_mut();
            let function = host.functions.get("gen_salt").unwrap().clone();
            let env = host.module_envs.last_mut().unwrap();
            (function, (&mut **env as *mut Env) as usize)
        });
        let env = &mut *(env_address as NapiEnv);
        let minor = env.alloc(Value::String("b".into()));
        let rounds = env.alloc(Value::Number(4.0));
        let seed = env.alloc(Value::Buffer((0..16).collect()));
        let callback = env.alloc(Value::Function(Function {
            callback: bcrypt_async_callback,
            data: ptr::null_mut(),
            properties: HashMap::new(),
            _thaw_bridge: None,
        }));
        let mut info = CallbackInfo {
            args: vec![minor, rounds, seed, callback],
            this_arg: ptr::null_mut(),
            new_target: ptr::null_mut(),
            data: function.data,
        };
        (function.callback)(env, &mut info);
        assert_eq!(thaw_napi_run_async_work(), 1);
        let async_salt = BCRYPT_ASYNC_RESULT
            .get()
            .unwrap()
            .lock()
            .unwrap()
            .take()
            .unwrap();
        assert!(async_salt.starts_with("$2b$04$"), "{async_salt}");

        let bridge_output: *mut Mutex<Option<(JsonValue, JsonValue)>> =
            Box::into_raw(Box::new(Mutex::new(None)));
        let bridge_args = CString::new(
            r#"["b",4,{"type":"Buffer","data":[0,1,2,3,4,5,6,7,8,9,10,11,12,13,14,15]}]"#,
        )
        .unwrap();
        let queued = thaw_napi_call_with_callback_result(
            c"gen_salt".as_ptr(),
            bridge_args.as_ptr(),
            Some(bcrypt_bridge_callback),
            bridge_output.cast(),
        );
        assert!(queued.error.is_null());
        assert_eq!(thaw_napi_run_async_work(), 1);
        let bridge_output = Box::from_raw(bridge_output);
        let (error, result) = bridge_output.lock().unwrap().take().unwrap();
        assert!(error.is_null());
        assert!(result.as_str().unwrap().starts_with("$2b$04$"));
    }
}

#[test]
fn loads_sqlite3_prebuild_when_supplied() {
    let _guard = lock_async_test();
    let Ok(path) = std::env::var("THAW_SQLITE3_NODE") else {
        return;
    };
    let path = CString::new(path).unwrap();
    unsafe {
        assert_eq!(thaw_napi_load(path.as_ptr()), 1);
    }
    HOST.with(|host| {
        let host = host.borrow();
        assert!(host.functions.contains_key("Database"));
        assert!(host.functions.contains_key("Statement"));
        assert!(host.functions.contains_key("Backup"));
    });
    unsafe {
        let constructor = thaw_napi_get_export(c"Database".as_ptr());
        assert_ne!(constructor, 0);
        let database = thaw_napi_construct_handle_result(constructor, c"[\":memory:\",6]".as_ptr());
        assert!(
            database.error.is_null(),
            "{}",
            if database.error.is_null() {
                "unknown constructor error".into()
            } else {
                CStr::from_ptr(database.error)
                    .to_string_lossy()
                    .into_owned()
            }
        );
        assert_ne!(database.value, 0);
        thaw_napi_run_async_work();
    }
}

#[test]
fn loads_serialport_class_prebuild_when_supplied() {
    let Ok(path) = std::env::var("THAW_SERIALPORT_NODE") else {
        return;
    };
    let path = CString::new(path).unwrap();
    unsafe {
        assert_eq!(thaw_napi_load(path.as_ptr()), 1);
        let poller = HOST.with(|host| host.borrow().functions.get("Poller").cloned());
        assert!(
            poller.is_some(),
            "serialport did not export its Poller class"
        );
    }
}

#[test]
fn loads_parcel_watcher_prebuild_when_supplied() {
    let _guard = lock_async_test();
    let Ok(path) = std::env::var("THAW_PARCEL_WATCHER_NODE") else {
        return;
    };
    let path = CString::new(path).unwrap();
    unsafe {
        assert_eq!(thaw_napi_load(path.as_ptr()), 1);
        for name in [
            "subscribe",
            "unsubscribe",
            "writeSnapshot",
            "getEventsSince",
        ] {
            assert!(
                HOST.with(|host| host.borrow().functions.contains_key(name)),
                "parcel watcher did not export `{name}`"
            );
        }

        let snapshot_dir = std::env::temp_dir().join(format!(
            "thaw-parcel-snapshot-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::create_dir_all(&snapshot_dir).unwrap();
        std::fs::write(snapshot_dir.join("before.txt"), "before").unwrap();
        let snapshot = snapshot_dir.join("snapshot.bin");
        let args = CString::new(
            serde_json::to_string(&serde_json::json!([
                snapshot_dir.to_string_lossy(),
                snapshot.to_string_lossy(),
                {}
            ]))
            .unwrap(),
        )
        .unwrap();
        let result = thaw_napi_call_result(c"writeSnapshot".as_ptr(), args.as_ptr());
        assert!(
            result.error.is_null(),
            "{}",
            if result.error.is_null() {
                "unknown error".into()
            } else {
                CStr::from_ptr(result.error).to_string_lossy().into_owned()
            }
        );
        assert_eq!(CStr::from_ptr(result.value).to_str().unwrap(), "null");
        assert!(snapshot.is_file());
        let missing = snapshot_dir.join("missing-snapshot.bin");
        let args = CString::new(
            serde_json::to_string(&serde_json::json!([
                snapshot_dir.to_string_lossy(),
                missing.to_string_lossy(),
                {}
            ]))
            .unwrap(),
        )
        .unwrap();
        let rejected = thaw_napi_call_result(c"getEventsSince".as_ptr(), args.as_ptr());
        assert!(
            !rejected.error.is_null(),
            "missing snapshot Promise resolved"
        );

        let (subscribe, unsubscribe, env_address) = HOST.with(|host| {
            let mut host = host.borrow_mut();
            let subscribe = host.functions.get("subscribe").unwrap().clone();
            let unsubscribe = host.functions.get("unsubscribe").unwrap().clone();
            let env = host.module_envs.last_mut().unwrap();
            (subscribe, unsubscribe, (&mut **env as *mut Env) as usize)
        });
        let env = &mut *(env_address as NapiEnv);
        let dir = std::env::temp_dir().join(format!(
            "thaw-parcel-watcher-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let directory = env.alloc(Value::String(dir.to_string_lossy().into_owned()));
        let options = env.alloc(Value::Object(HashMap::new()));
        let probe = Box::into_raw(Box::new(ParcelWatcherProbe {
            events: Mutex::new(Vec::new()),
        }));
        let callback = env.alloc(Value::Function(Function {
            callback: parcel_watcher_callback,
            data: probe.cast(),
            properties: HashMap::new(),
            _thaw_bridge: None,
        }));
        let this_arg = env.alloc(Value::Undefined);
        let mut subscribe_info = CallbackInfo {
            args: vec![directory, callback, options],
            this_arg,
            new_target: ptr::null_mut(),
            data: subscribe.data,
        };
        let subscribed = (subscribe.callback)(env, &mut subscribe_info);
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while !promise_is_resolved(subscribed) {
            thaw_napi_poll_async_work();
            assert!(
                std::time::Instant::now() < deadline,
                "subscribe Promise timed out"
            );
            std::thread::sleep(Duration::from_millis(5));
        }

        let watched_file = dir.join("created.txt");
        std::fs::write(&watched_file, "thaw").unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while (*probe).events.lock().unwrap().is_empty() {
            thaw_napi_poll_async_work();
            assert!(
                std::time::Instant::now() < deadline,
                "watch event timed out"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!((*probe).events.lock().unwrap().iter().any(|(event, kind)| {
            event == &watched_file.to_string_lossy() && (kind == "create" || kind == "update")
        }));

        let mut unsubscribe_info = CallbackInfo {
            args: vec![directory, callback, options],
            this_arg,
            new_target: ptr::null_mut(),
            data: unsubscribe.data,
        };
        let unsubscribed = (unsubscribe.callback)(env, &mut unsubscribe_info);
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while !promise_is_resolved(unsubscribed) {
            thaw_napi_poll_async_work();
            assert!(
                std::time::Instant::now() < deadline,
                "unsubscribe Promise timed out"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
        thaw_napi_run_async_work();
        drop(Box::from_raw(probe));
        let _ = std::fs::remove_dir_all(dir);
        let _ = std::fs::remove_dir_all(snapshot_dir);
    }
}

#[test]
fn parcel_watcher_callback_bridge_reuses_identity_when_supplied() {
    let _guard = lock_async_test();
    let Ok(path) = std::env::var("THAW_PARCEL_WATCHER_NODE") else {
        return;
    };
    let path = CString::new(path).unwrap();
    let dir = std::env::temp_dir().join(format!(
        "thaw-parcel-bridge-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let args =
        CString::new(serde_json::to_string(&serde_json::json!([dir.to_string_lossy()])).unwrap())
            .unwrap();
    let output: *mut Mutex<Option<(JsonValue, JsonValue)>> =
        Box::into_raw(Box::new(Mutex::new(None)));
    unsafe {
        assert_eq!(thaw_napi_load(path.as_ptr()), 1);
        let subscribed = thaw_napi_call_with_callback_result(
            c"subscribe".as_ptr(),
            args.as_ptr(),
            Some(bcrypt_bridge_callback),
            output.cast(),
        );
        assert!(subscribed.error.is_null());
        let watched_file = dir.join("bridge-event.txt");
        std::fs::write(&watched_file, "event").unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while (*output).lock().unwrap().is_none() {
            thaw_napi_poll_async_work();
            assert!(
                std::time::Instant::now() < deadline,
                "bridge event timed out"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
        let event = (*output).lock().unwrap().take().unwrap();
        assert!(event.0.is_null());
        assert!(event.1.as_array().is_some_and(|events| {
            events.iter().any(|event| {
                event.get("path").and_then(JsonValue::as_str)
                    == Some(watched_file.to_string_lossy().as_ref())
            })
        }));
        let unsubscribed = thaw_napi_call_with_callback_result(
            c"unsubscribe".as_ptr(),
            args.as_ptr(),
            Some(bcrypt_bridge_callback),
            output.cast(),
        );
        assert!(
            unsubscribed.error.is_null(),
            "{}",
            if unsubscribed.error.is_null() {
                "unknown error".into()
            } else {
                CStr::from_ptr(unsubscribed.error)
                    .to_string_lossy()
                    .into_owned()
            }
        );
        assert_eq!(thaw_napi_run_async_work(), 0);
        assert_eq!(LIVE_THREADSAFE_FUNCTIONS.load(Ordering::Acquire), 0);
        drop(Box::from_raw(output));
    }
    let _ = std::fs::remove_dir_all(dir);
}
