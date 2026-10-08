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
static RELEASED_HANDLE_FINALIZED: AtomicUsize = AtomicUsize::new(0);
#[cfg(target_os = "linux")]
static UV_TIMER_FIRED: AtomicBool = AtomicBool::new(false);

#[cfg(feature = "quickjs")]
#[test]
fn quickjs_reference_throw_sets_napi_exception_without_spoofing_returned_object() {
    assert_eq!(
        thaw_quickjs::thaw_js_load(c"globalThis.__thaw_napi_reference_746291201 = () => ({ __thaw_error__: 'ordinary' }); globalThis.__thaw_napi_reference_746291202 = () => { throw new TypeError('callback \\u0002boom'); };".as_ptr()),
        1
    );
    let mut env = Env::new();
    let this_arg = env.alloc(Value::Undefined);
    for (reference, throws) in [(746291201_usize, false), (746291202, true)] {
        let bridge = Arc::new(ThawCallbackBridge {
            callback: ThawCallback::QuickJs(thaw_quickjs::thaw_js_call_reference),
            context: reference,
        });
        let mut info = CallbackInfo {
            args: vec![],
            this_arg,
            new_target: ptr::null_mut(),
            data: Arc::as_ptr(&bridge) as *mut c_void,
        };
        let result = unsafe { thaw_compiled_callback(&mut env as NapiEnv, &mut info) };
        if throws {
            assert!(result.is_null());
            let mut error = ptr::null_mut();
            unsafe {
                assert_eq!(
                    napi_get_and_clear_last_exception(&mut env, &mut error),
                    NAPI_OK
                );
            }
            assert!(
                matches!(unsafe { value_ref(error) }, Ok(Value::Error(message)) if message == "callback \u{2}boom")
            );
            let mut name = ptr::null_mut();
            unsafe {
                assert_eq!(
                    napi_get_named_property(&mut env, error, c"name".as_ptr(), &mut name),
                    NAPI_OK
                );
            }
            assert!(
                matches!(unsafe { value_ref(name) }, Ok(Value::String(value)) if value == "TypeError")
            );
            let forwarded = unsafe { describe_env_exception(&mut env, error) }.unwrap();
            let frame = thaw_arena::error_wire::parse_tagged(forwarded.as_bytes()).unwrap();
            assert_eq!(frame.chain, b"TypeError");
            assert_eq!(frame.display, b"callback \x02boom");
            assert_eq!(frame.suffix, b"");
        } else {
            let Ok(Value::Object(fields)) = (unsafe { value_ref(result) }) else {
                panic!("callback result was not an object")
            };
            let returned = fields
                .get(&PropertyKey::String("__thaw_error__".into()))
                .copied()
                .unwrap();
            assert!(
                matches!(unsafe { value_ref(returned) }, Ok(Value::String(value)) if value == "ordinary")
            );
            assert!(env.exception.is_none());
        }
    }
}

#[test]
fn exported_callback_exception_is_consumed_before_the_next_call() {
    unsafe extern "C" fn throws(env: NapiEnv, _info: NapiCallbackInfo) -> NapiValue {
        assert_eq!(
            napi_throw_type_error(env, ptr::null(), c"first failure".as_ptr()),
            NAPI_OK
        );
        ptr::null_mut()
    }
    unsafe extern "C" fn invalid_exception(env: NapiEnv, _info: NapiCallbackInfo) -> NapiValue {
        env_mut(env).unwrap().exception = Some(ptr::null_mut());
        ptr::null_mut()
    }
    unsafe extern "C" fn succeeds(env: NapiEnv, _info: NapiCallbackInfo) -> NapiValue {
        env_mut(env).unwrap().alloc(Value::Number(7.0))
    }

    let mut env = Box::new(Env::new());
    let env_ptr: NapiEnv = &mut *env;
    let callbacks = [
        ("stale-test::throw", throws as NapiCallback),
        ("stale-test::invalid", invalid_exception as NapiCallback),
        ("stale-test::good", succeeds as NapiCallback),
    ];
    let exports = callbacks
        .iter()
        .map(|(name, callback)| {
            let function = Function {
                callback: *callback,
                data: ptr::null_mut(),
                properties: HashMap::new(),
                _thaw_bridge: None,
                _accessor_owner: None,
            };
            let value = env.alloc(Value::Function(function.clone()));
            ((*name).to_string(), function, value)
        })
        .collect::<Vec<_>>();
    HOST.with(|host| {
        let mut host = host.borrow_mut();
        for (name, function, value) in exports {
            host.functions.insert(name.clone(), function);
            host.exports.insert(name, (env_ptr as usize, value));
        }
        host.module_envs.push(env);
    });

    unsafe {
        let first = thaw_napi_call_result(c"stale-test::throw".as_ptr(), c"[]".as_ptr());
        assert!(first.value.is_null());
        let first_error = CString::from_raw(first.error).into_string().unwrap();
        assert!(
            first_error.starts_with("\u{1}TypeError\u{1}\u{1e}E1:")
                && first_error.contains("first failure"),
            "{first_error}"
        );
        assert!((*env_ptr).exception.is_none());
        let good = thaw_napi_call_result(c"stale-test::good".as_ptr(), c"[]".as_ptr());
        assert!(good.error.is_null());
        assert_eq!(CStr::from_ptr(good.value).to_str().unwrap(), "7.0");

        let invalid = thaw_napi_call_result(c"stale-test::invalid".as_ptr(), c"[]".as_ptr());
        assert!(invalid.value.is_null());
        assert_eq!(
            CString::from_raw(invalid.error).into_string().unwrap(),
            "invalid exception"
        );
        assert!((*env_ptr).exception.is_none());
        let good_again = thaw_napi_call_typed_result(c"stale-test::good".as_ptr(), c"[]".as_ptr());
        assert!(good_again.error.is_null());
        assert_eq!(CStr::from_ptr(good_again.value).to_str().unwrap(), "7.0");
    }
    HOST.with(|host| {
        let mut host = host.borrow_mut();
        for name in [
            "stale-test::throw",
            "stale-test::invalid",
            "stale-test::good",
        ] {
            host.functions.remove(name);
            host.exports.remove(name);
        }
        host.module_envs
            .retain(|entry| (&**entry as *const Env).cast_mut() != env_ptr);
    });
}

#[test]
fn napi_result_error_preserves_embedded_nul_through_owned_and_legacy_paths() {
    unsafe extern "C" fn throws_nul(env: NapiEnv, _info: NapiCallbackInfo) -> NapiValue {
        let env = env_mut(env).unwrap();
        let error = alloc_error(env, "TypeError", "left\0right".into(), None);
        env.exception = Some(error);
        ptr::null_mut()
    }
    let mut env = Box::new(Env::new());
    let env_ptr: NapiEnv = &mut *env;
    let function = Function {
        callback: throws_nul,
        data: ptr::null_mut(),
        properties: HashMap::new(),
        _thaw_bridge: None,
        _accessor_owner: None,
    };
    let value = env.alloc(Value::Function(function.clone()));
    HOST.with(|host| {
        let mut host = host.borrow_mut();
        host.functions
            .insert("nul-result-test::throw".into(), function);
        host.exports
            .insert("nul-result-test::throw".into(), (env_ptr as usize, value));
        host.module_envs.push(env);
    });
    unsafe {
        let result = thaw_napi_call_result(c"nul-result-test::throw".as_ptr(), c"[]".as_ptr());
        assert!(result.value.is_null());
        let bytes = thaw_arena::NativeStr::from_ptr(result.error)
            .to_bytes()
            .to_vec();
        let frame = thaw_arena::error_wire::parse_tagged(&bytes).unwrap();
        assert_eq!(frame.chain, b"TypeError");
        assert_eq!(frame.display, b"left\0right");
        thaw_arena::destroy_string(result.error);
        let legacy = thaw_napi_call(c"nul-result-test::throw".as_ptr(), c"[]".as_ptr());
        let json = CStr::from_ptr(legacy).to_str().unwrap();
        let parsed: serde_json::Value = serde_json::from_str(json).unwrap();
        let framed = parsed["__thaw_error__"].as_str().unwrap();
        let frame = thaw_arena::error_wire::parse_tagged(framed.as_bytes()).unwrap();
        assert_eq!(frame.display, b"left\0right");
        drop(CString::from_raw(legacy.cast_mut()));
        assert!((*env_ptr).exception.is_none());
    }
    HOST.with(|host| {
        let mut host = host.borrow_mut();
        host.functions.remove("nul-result-test::throw");
        host.exports.remove("nul-result-test::throw");
        host.module_envs
            .retain(|entry| (&**entry as *const Env).cast_mut() != env_ptr);
    });
}

#[test]
fn napi_result_error_helpers_keep_owned_nul_bytes() {
    let text = text_result(Err("text\0error".into()));
    let handle = handle_error("handle\0error");
    unsafe {
        assert_eq!(
            thaw_arena::NativeStr::from_ptr(text.error).to_bytes(),
            b"text\0error"
        );
        assert_eq!(
            thaw_arena::NativeStr::from_ptr(handle.error).to_bytes(),
            b"handle\0error"
        );
        thaw_arena::destroy_string(text.error);
        thaw_arena::destroy_string(handle.error);
    }
}

#[test]
fn nested_export_getter_preserves_its_exception() {
    unsafe extern "C" fn throwing_getter(env: NapiEnv, _info: NapiCallbackInfo) -> NapiValue {
        assert_eq!(
            napi_throw_type_error(env, ptr::null(), c"original getter failure".as_ptr()),
            NAPI_OK
        );
        ptr::null_mut()
    }
    let mut env = Box::new(Env::new());
    let env_ptr: NapiEnv = &mut *env;
    let outer = env.alloc(Value::Object(HashMap::new()));
    let descriptor = NapiPropertyDescriptor {
        utf8name: c"Client".as_ptr(),
        name: ptr::null_mut(),
        method: None,
        getter: Some(throwing_getter),
        setter: None,
        value: ptr::null_mut(),
        attributes: NAPI_DEFAULT_PROPERTY_ATTRIBUTES,
        data: ptr::null_mut(),
    };
    unsafe {
        assert_eq!(
            napi_define_properties(env_ptr, outer, 1, &descriptor),
            NAPI_OK
        );
    }
    HOST.with(|host| {
        let mut host = host.borrow_mut();
        host.exports
            .insert("throwing-test::Outer".into(), (env_ptr as usize, outer));
        host.module_envs.push(env);
    });
    unsafe {
        let result = thaw_napi_get_export_typed_result(c"throwing-test::Outer.Client".as_ptr());
        assert_eq!(result.value, 0);
        assert!(!result.error.is_null());
        let error = CString::from_raw(result.error).into_string().unwrap();
        assert!(
            error.starts_with("\u{1}TypeError\u{1}\u{1e}E1:")
                && error.contains("original getter failure"),
            "{error}"
        );
        assert!(!error.contains("unknown native addon export"));
        assert!((*env_ptr).exception.is_none());
    }
    HOST.with(|host| {
        let mut host = host.borrow_mut();
        host.exports.remove("throwing-test::Outer");
        host.module_envs
            .retain(|entry| (&**entry as *const Env).cast_mut() != env_ptr);
    });
}

#[test]
fn package_qualified_exports_survive_opposite_load_orders() {
    for reverse in [false, true] {
        let mut first = Box::new(Env::new());
        let mut second = Box::new(Env::new());
        let first_env = (&mut *first) as NapiEnv;
        let second_env = (&mut *second) as NapiEnv;
        let first_client = first.alloc(Value::Object(HashMap::new()));
        let second_client = second.alloc(Value::Object(HashMap::new()));
        HOST.with(|host| {
            let mut host = host.borrow_mut();
            let entries = if reverse {
                [
                    ("second-test", second_env, second_client),
                    ("first-test", first_env, first_client),
                ]
            } else {
                [
                    ("first-test", first_env, first_client),
                    ("second-test", second_env, second_client),
                ]
            };
            for (package, env, client) in entries {
                register_loaded_exports(
                    &mut host,
                    env as usize,
                    Some(package),
                    vec![],
                    vec![("Client".into(), client)],
                );
            }
            host.module_envs.push(first);
            host.module_envs.push(second);
        });
        unsafe {
            assert_eq!(
                get_export_result(c"first-test::Client".as_ptr()).unwrap(),
                first_client as u64
            );
            assert_eq!(
                get_export_result(c"second-test::Client".as_ptr()).unwrap(),
                second_client as u64
            );
            assert_eq!(get_export_result(c"Client".as_ptr()).unwrap(), 0);
        }
        HOST.with(|host| {
            let mut host = host.borrow_mut();
            for name in ["Client", "first-test::Client", "second-test::Client"] {
                host.exports.remove(name);
            }
            host.qualified_packages.remove("first-test");
            host.qualified_packages.remove("second-test");
            host.module_envs.retain(|entry| {
                let pointer = (&**entry as *const Env).cast_mut();
                pointer != first_env && pointer != second_env
            });
        });
    }
}

#[test]
fn package_qualified_functions_do_not_publish_colliding_bare_names() {
    unsafe extern "C" fn first(env: NapiEnv, _info: NapiCallbackInfo) -> NapiValue {
        env_mut(env).unwrap().alloc(Value::Number(1.0))
    }
    unsafe extern "C" fn second(env: NapiEnv, _info: NapiCallbackInfo) -> NapiValue {
        env_mut(env).unwrap().alloc(Value::Number(2.0))
    }
    unsafe extern "C" fn legacy(env: NapiEnv, _info: NapiCallbackInfo) -> NapiValue {
        env_mut(env).unwrap().alloc(Value::Number(3.0))
    }
    for reverse in [false, true] {
        let mut first_env = Box::new(Env::new());
        let mut second_env = Box::new(Env::new());
        let mut legacy_env = Box::new(Env::new());
        let first_ptr = (&mut *first_env) as NapiEnv;
        let second_ptr = (&mut *second_env) as NapiEnv;
        let legacy_ptr = (&mut *legacy_env) as NapiEnv;
        let first_fn = Function {
            callback: first,
            data: ptr::null_mut(),
            properties: HashMap::new(),
            _thaw_bridge: None,
            _accessor_owner: None,
        };
        let second_fn = Function {
            callback: second,
            data: ptr::null_mut(),
            properties: HashMap::new(),
            _thaw_bridge: None,
            _accessor_owner: None,
        };
        let legacy_fn = Function {
            callback: legacy,
            data: ptr::null_mut(),
            properties: HashMap::new(),
            _thaw_bridge: None,
            _accessor_owner: None,
        };
        let first_value = first_env.alloc(Value::Function(first_fn.clone()));
        let second_value = second_env.alloc(Value::Function(second_fn.clone()));
        let legacy_value = legacy_env.alloc(Value::Function(legacy_fn.clone()));
        HOST.with(|host| {
            let mut host = host.borrow_mut();
            register_loaded_exports(
                &mut host,
                legacy_ptr as usize,
                None,
                vec![("same".into(), legacy_fn)],
                vec![("same".into(), legacy_value)],
            );
            let entries = if reverse {
                [
                    ("second-test", second_ptr, second_fn, second_value),
                    ("first-test", first_ptr, first_fn, first_value),
                ]
            } else {
                [
                    ("first-test", first_ptr, first_fn, first_value),
                    ("second-test", second_ptr, second_fn, second_value),
                ]
            };
            for (package, env, function, value) in entries {
                register_loaded_exports(
                    &mut host,
                    env as usize,
                    Some(package),
                    vec![("same".into(), function)],
                    vec![("same".into(), value)],
                );
            }
            host.module_envs.push(first_env);
            host.module_envs.push(second_env);
            host.module_envs.push(legacy_env);
        });
        unsafe {
            for (name, expected) in [
                (c"first-test::same", "1.0"),
                (c"second-test::same", "2.0"),
                (c"same", "3.0"),
            ] {
                let result = thaw_napi_call_result(name.as_ptr(), c"[]".as_ptr());
                assert!(result.error.is_null());
                assert_eq!(CStr::from_ptr(result.value).to_str().unwrap(), expected);
            }
        }
        HOST.with(|host| {
            let mut host = host.borrow_mut();
            for name in ["same", "first-test::same", "second-test::same"] {
                host.functions.remove(name);
                host.exports.remove(name);
            }
            host.qualified_packages.remove("first-test");
            host.qualified_packages.remove("second-test");
            host.module_envs.retain(|entry| {
                let pointer = (&**entry as *const Env).cast_mut();
                pointer != first_ptr && pointer != second_ptr && pointer != legacy_ptr
            });
        });
    }
}

#[test]
fn resolves_packaged_native_paths_next_to_the_executable() {
    let resolved = executable_relative_path("@executable/app.native/pkg/native.node").unwrap();
    assert_eq!(
        resolved,
        std::env::current_exe()
            .unwrap()
            .parent()
            .unwrap()
            .join("app.native/pkg/native.node")
    );
    assert_eq!(
        executable_relative_path("/tmp/native.node").unwrap(),
        std::path::Path::new("/tmp/native.node")
    );
}

#[test]
fn decodes_legacy_hex_and_compressed_embedded_payloads() {
    assert_eq!(decode_hex("deadbeef").unwrap(), [0xde, 0xad, 0xbe, 0xef]);
    assert_eq!(
        decode_hex("gz:H4sIAAAAAAAA/8tIzcnJBwCGphA2BQAAAA==").unwrap(),
        b"hello"
    );
}

unsafe extern "C" fn double_json_callback(
    _context: *mut c_void,
    args: *const c_char,
) -> *const c_char {
    let args: Vec<f64> = serde_json::from_str(CStr::from_ptr(args).to_str().unwrap()).unwrap();
    CString::new((args[0] * 2.0).to_string())
        .unwrap()
        .into_raw()
}

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

unsafe extern "C" fn released_handle_finalize(
    _env: NapiEnv,
    _data: *mut c_void,
    _hint: *mut c_void,
) {
    RELEASED_HANDLE_FINALIZED.fetch_add(1, Ordering::AcqRel);
}

#[test]
fn released_handle_waits_for_strong_napi_reference() {
    RELEASED_HANDLE_FINALIZED.store(0, Ordering::Release);
    let mut env = Box::new(Env::new());
    let env_ptr = (&mut *env) as NapiEnv;
    let value = env.alloc(Value::Object(HashMap::new()));
    let mut reference = ptr::null_mut();
    unsafe {
        assert_eq!(
            napi_wrap(
                env_ptr,
                value,
                ptr::null_mut(),
                Some(released_handle_finalize),
                ptr::null_mut(),
                &mut reference,
            ),
            NAPI_OK
        );
        assert_eq!(
            napi_reference_ref(env_ptr, reference, ptr::null_mut()),
            NAPI_OK
        );
    }
    HOST.with(|host| host.borrow_mut().module_envs.push(env));

    release_napi_handle(value as u64).unwrap();
    assert_eq!(RELEASED_HANDLE_FINALIZED.load(Ordering::Acquire), 0);
    unsafe {
        assert_eq!(
            napi_reference_unref(env_ptr, reference, ptr::null_mut()),
            NAPI_OK
        );
    }
    assert_eq!(RELEASED_HANDLE_FINALIZED.load(Ordering::Acquire), 1);
    HOST.with(|host| host.borrow_mut().module_envs.clear());
}

include!("tests/objects_classes.rs");

include!("tests/values.rs");

include!("tests/buffers.rs");

include!("tests/handles.rs");

include!("tests/async_runtime.rs");

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
            typedef enum { napi_undefined = 0, napi_null = 1, napi_boolean = 2, napi_number = 3, napi_string = 4, napi_symbol = 5, napi_object = 6, napi_function = 7, napi_external = 8, napi_bigint = 9 } napi_valuetype;
            extern napi_status napi_get_cb_info(napi_env, napi_callback_info, size_t*, napi_value*, napi_value*, void**);
            extern napi_status napi_get_value_double(napi_env, napi_value, double*);
            extern napi_status napi_get_value_bool(napi_env, napi_value, _Bool*);
            extern napi_status napi_get_value_string_utf8(napi_env, napi_value, char*, size_t, size_t*);
            extern napi_status napi_create_double(napi_env, double, napi_value*);
            extern napi_status napi_get_boolean(napi_env, _Bool, napi_value*);
            extern napi_status napi_get_undefined(napi_env, napi_value*);
            extern napi_status napi_typeof(napi_env, napi_value, napi_valuetype*);
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
            static double factory_factor;
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
            static napi_value multiply(napi_env env, napi_callback_info info) {
                size_t argc = 1; napi_value arg, result; double value; void* data;
                napi_get_cb_info(env, info, &argc, &arg, 0, &data);
                napi_get_value_double(env, arg, &value);
                napi_create_double(env, value * *(double*)data, &result); return result;
            }
            static napi_value multiplier(napi_env env, napi_callback_info info) {
                size_t argc = 1; napi_value arg, result;
                napi_get_cb_info(env, info, &argc, &arg, 0, 0);
                napi_get_value_double(env, arg, &factory_factor);
                napi_create_function(env, "multiply", 8, multiply, &factory_factor, &result); return result;
            }
            static napi_value apply(napi_env env, napi_callback_info info) {
                size_t argc = 2; napi_value argv[2], result, self;
                napi_get_cb_info(env, info, &argc, argv, 0, 0);
                napi_get_undefined(env, &self);
                napi_call_function(env, self, argv[0], 1, &argv[1], &result); return result;
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
            static napi_value is_undefined(napi_env env, napi_callback_info info) {
                size_t argc = 1; napi_value arg, result; napi_valuetype type;
                napi_get_cb_info(env, info, &argc, &arg, 0, 0);
                napi_typeof(env, arg, &type); napi_get_boolean(env, type == napi_undefined, &result); return result;
            }
            static napi_value return_undefined(napi_env env, napi_callback_info info) {
                (void)info; napi_value result; napi_get_undefined(env, &result); return result;
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
                napi_create_function(env, "multiplier", 10, multiplier, 0, &fn); napi_set_named_property(env, exports, "multiplier", fn);
                napi_create_function(env, "apply", 5, apply, 0, &fn); napi_set_named_property(env, exports, "apply", fn);
                napi_create_function(env, "negate", 6, negate, 0, &fn); napi_set_named_property(env, exports, "negate", fn);
                napi_create_function(env, "echo", 4, echo, 0, &fn); napi_set_named_property(env, exports, "echo", fn);
                napi_create_function(env, "isUndefined", 11, is_undefined, 0, &fn); napi_set_named_property(env, exports, "isUndefined", fn);
                napi_create_function(env, "returnUndefined", 15, return_undefined, 0, &fn); napi_set_named_property(env, exports, "returnUndefined", fn);
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
        let multiplier =
            thaw_napi_call_export_handle_typed_result(c"multiplier".as_ptr(), c"[3]".as_ptr());
        assert!(multiplier.error.is_null());
        assert_ne!(multiplier.value, 0);
        let result = thaw_napi_call_handle_typed_result(multiplier.value, c"[14]".as_ptr());
        assert!(result.error.is_null());
        assert_eq!(CStr::from_ptr(result.value).to_str().unwrap(), "42.0");
        let applied = thaw_napi_call_export_handle_with_function_typed_result(
            c"apply".as_ptr(),
            c"[21]".as_ptr(),
            0,
            Some(double_json_callback),
            ptr::null_mut(),
        );
        assert!(applied.error.is_null());
        match value_ref(applied.value as NapiValue).unwrap() {
            Value::Number(value) => assert_eq!(*value, 42.0),
            _ => panic!("apply returned a non-number"),
        }
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
        // Calls reuse the addon's environment; values are finalized with it.
        assert_eq!(CStr::from_ptr(result.value).to_str().unwrap(), "0.0");
        let typed = thaw_napi_call_typed_result(
            c"isUndefined".as_ptr(),
            c"[{\"$__thaw_napi_undefined$\":true}]".as_ptr(),
        );
        assert!(typed.error.is_null());
        assert_eq!(CStr::from_ptr(typed.value).to_str().unwrap(), "true");
        let typed = thaw_napi_call_typed_result(c"returnUndefined".as_ptr(), c"[]".as_ptr());
        assert!(typed.error.is_null());
        assert_eq!(
            CStr::from_ptr(typed.value).to_str().unwrap(),
            "{\"$__thaw_napi_undefined$\":true}"
        );
        let untyped = thaw_napi_call_result(c"returnUndefined".as_ptr(), c"[]".as_ptr());
        assert_eq!(CStr::from_ptr(untyped.value).to_str().unwrap(), "null");
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
fn runs_node_addon_api_official_binding_when_supplied() {
    let _guard = lock_async_test();

    fn same_json(left: &JsonValue, right: &JsonValue) -> bool {
        match (left, right) {
            (JsonValue::Number(left), JsonValue::Number(right)) => left.as_f64() == right.as_f64(),
            (JsonValue::Array(left), JsonValue::Array(right)) => {
                left.len() == right.len()
                    && left
                        .iter()
                        .zip(right)
                        .all(|(left, right)| same_json(left, right))
            }
            (JsonValue::Object(left), JsonValue::Object(right)) => {
                left.len() == right.len()
                    && left.iter().all(|(key, left)| {
                        right.get(key).is_some_and(|right| same_json(left, right))
                    })
            }
            _ => left == right,
        }
    }

    let Ok(path) = std::env::var("THAW_NODE_ADDON_API_BINDING") else {
        return;
    };
    let cases = serde_json::json!([
        ["basic_types_number", "toInt32", [42.75]],
        ["basic_types_number", "toUint32", [-1]],
        ["basic_types_number", "toDouble", [12.5]],
        ["basic_types_boolean", "operatorBool", [true]],
        ["basic_types_array", "getLength", [[1, 2, 3]]],
        ["basic_types_array", "get", [[1, "two"], 1]],
        ["basic_types_value", "isNull", [null]],
        ["basic_types_value", "isNumber", [42]],
        ["basic_types_value", "isString", ["text"]],
        ["basic_types_value", "isArray", [[1]]],
        ["basic_types_value", "toBoolean", [0]],
        ["buffer", "createBuffer", []],
        ["buffer", "createBufferCopy", []],
        ["object", "sum", [{"x": 20, "y": 22}]],
        ["globalObject", "createMockTestObject", []],
        ["globalObject", "getPropertyWithNapiValue", [2]],
        ["globalObject", "getPropertyWithCString", ["c_str_key"]],
        ["globalObject", "getPropertyWithCppString", ["cpp_string_key"]],
        ["globalObject", "getPropertyWithInt32", [15]],
        ["promise", "isPromise", [{}]],
        ["promise", "resolvePromise", ["resolved"]],
        ["asyncworker", "doWorkNoCallback", [true]],
        ["asyncworker", "doWorkAsyncResNoCallback", [true, {}]],
        ["threadsafe_function_existing_tsfn", "testCall", [{"blocking": false, "data": false}]],
        ["threadsafe_function_existing_tsfn", "testCall", [{"blocking": true, "data": false}]],
        ["typed_threadsafe_function_existing_tsfn", "testCall", [{"blocking": false, "data": false}]],
        ["typed_threadsafe_function_existing_tsfn", "testCall", [{"blocking": true, "data": false}]],
        ["handlescope", "createScope", []],
        ["handlescope", "createScopeFromExisting", []],
        ["handlescope", "escapeFromScope", []],
        ["handlescope", "escapeFromExistingScope", []],
        ["reference", "refMoveAssignTest", []],
        ["reference", "referenceRefTest", []],
        ["reference", "refResetTest", []]
    ]);
    let script = r#"
const binding = require(process.argv[1]);
const cases = JSON.parse(process.argv[2]);
(async () => {
  const results = [];
  for (const [group, method, args] of cases) {
    let value = binding[group][method](...args);
    if (value && typeof value.then === 'function') value = await value;
    results.push(value === undefined ? { $__thaw_napi_undefined$: true } : value);
  }
  process.stdout.write(JSON.stringify(results));
})().catch(error => { console.error(error); process.exit(1); });
"#;
    let node = Command::new("node")
        .args(["-e", script, &path, &cases.to_string()])
        .output()
        .unwrap();
    assert!(
        node.status.success(),
        "{}",
        String::from_utf8_lossy(&node.stderr)
    );
    let expected: Vec<JsonValue> = serde_json::from_slice(&node.stdout).unwrap();
    let node_error = Command::new("node")
        .args([
            "-e",
            "const b=require(process.argv[1]);try{b.error.throwErrorThatEscapesScope('official-error')}catch(e){process.stdout.write(JSON.stringify({name:e.name,message:e.message}))}",
            &path,
        ])
        .output()
        .unwrap();
    assert!(node_error.status.success());
    let expected_error: JsonValue = serde_json::from_slice(&node_error.stdout).unwrap();
    let path = CString::new(path).unwrap();
    unsafe {
        assert_eq!(thaw_napi_load(path.as_ptr()), 1);
        for (index, case) in cases.as_array().unwrap().iter().enumerate() {
            let case = case.as_array().unwrap();
            let group = CString::new(case[0].as_str().unwrap()).unwrap();
            let method = CString::new(case[1].as_str().unwrap()).unwrap();
            let args = CString::new(case[2].to_string()).unwrap();
            let receiver = thaw_napi_get_export(group.as_ptr());
            assert_ne!(receiver, 0, "missing official group {}", case[0]);
            let result =
                thaw_napi_call_method_typed_result(receiver, method.as_ptr(), args.as_ptr());
            assert!(
                result.error.is_null(),
                "{}.{}: {}; host error: {}",
                case[0],
                case[1],
                CStr::from_ptr(result.error).to_string_lossy(),
                HOST.with(|host| host.borrow().last_error.clone())
            );
            let actual: JsonValue =
                serde_json::from_str(CStr::from_ptr(result.value).to_str().unwrap()).unwrap();
            assert!(
                same_json(&actual, &expected[index]),
                "{}.{}: {actual} != {}",
                case[0],
                case[1],
                expected[index]
            );
        }

        let objectwrap = thaw_napi_get_export(c"objectwrap".as_ptr()) as NapiValue;
        let env = module_env_for_handle(objectwrap as u64).unwrap();
        let mut constructor = ptr::null_mut();
        assert_eq!(
            napi_get_named_property(env, objectwrap, c"Test".as_ptr(), &mut constructor),
            NAPI_OK
        );
        let mut instance = ptr::null_mut();
        assert_eq!(
            napi_new_instance(env, constructor, 0, ptr::null(), &mut instance),
            NAPI_OK
        );
        let result = thaw_napi_call_method_typed_result(
            instance as u64,
            c"testMethod".as_ptr(),
            c"[\"method\"]".as_ptr(),
        );
        assert!(result.error.is_null());
        assert_eq!(
            CStr::from_ptr(result.value).to_str().unwrap(),
            "\"method instance\""
        );

        let typedarray = thaw_napi_get_export(c"typedarray".as_ptr()) as NapiValue;
        let mut create = ptr::null_mut();
        assert_eq!(
            napi_get_named_property(env, typedarray, c"createTypedArray".as_ptr(), &mut create),
            NAPI_OK
        );
        let kind = env_mut(env).unwrap().alloc(Value::String("int8".into()));
        let length = env_mut(env).unwrap().alloc(Value::Number(4.0));
        let mut array = ptr::null_mut();
        assert_eq!(
            napi_call_function(
                env,
                typedarray,
                create,
                2,
                [kind, length].as_ptr(),
                &mut array
            ),
            NAPI_OK
        );
        assert!(matches!(
            value_ref(array),
            Ok(Value::TypedArray {
                array_type: 0,
                length: 4,
                ..
            })
        ));

        let arraybuffer = thaw_napi_get_export(c"arraybuffer".as_ptr()) as NapiValue;
        assert_eq!(
            napi_get_named_property(env, arraybuffer, c"createBuffer".as_ptr(), &mut create),
            NAPI_OK
        );
        let mut buffer = ptr::null_mut();
        assert_eq!(
            napi_call_function(env, arraybuffer, create, 0, ptr::null(), &mut buffer),
            NAPI_OK
        );
        assert!(matches!(value_ref(buffer), Ok(Value::ArrayBuffer { .. })));

        let functions = thaw_napi_get_export(c"function".as_ptr()) as NapiValue;
        let mut plain = ptr::null_mut();
        assert_eq!(
            napi_get_named_property(env, functions, c"plain".as_ptr(), &mut plain),
            NAPI_OK
        );
        assert_eq!(
            napi_get_named_property(env, plain, c"valueCallback".as_ptr(), &mut create),
            NAPI_OK
        );
        let mut object = ptr::null_mut();
        assert_eq!(
            napi_call_function(env, plain, create, 0, ptr::null(), &mut object),
            NAPI_OK
        );
        let mut foo = ptr::null_mut();
        assert_eq!(
            napi_get_named_property(env, object, c"foo".as_ptr(), &mut foo),
            NAPI_OK
        );
        assert!(matches!(value_ref(foo), Ok(Value::String(value)) if value == "bar"));

        let finalizers = thaw_napi_get_export(c"finalizer_order".as_ptr()) as NapiValue;
        assert_eq!(
            napi_get_named_property(
                env,
                finalizers,
                c"createExternalFinalizer".as_ptr(),
                &mut create,
            ),
            NAPI_OK
        );
        let mut external = ptr::null_mut();
        assert_eq!(
            napi_call_function(env, finalizers, create, 0, ptr::null(), &mut external),
            NAPI_OK
        );
        assert!(matches!(value_ref(external), Ok(Value::External(_))));

        let error = thaw_napi_get_export(c"error".as_ptr()) as NapiValue;
        let result = thaw_napi_call_method_typed_result(
            error as u64,
            c"throwErrorThatEscapesScope".as_ptr(),
            c"[\"official-error\"]".as_ptr(),
        );
        assert!(!result.error.is_null());
        let actual_error = CStr::from_ptr(result.error).to_string_lossy();
        assert!(actual_error.contains(expected_error["name"].as_str().unwrap()));
        assert!(actual_error.contains(expected_error["message"].as_str().unwrap()));

        let asyncworker = thaw_napi_get_export(c"asyncworker".as_ptr()) as NapiValue;
        assert_eq!(
            napi_get_named_property(
                env,
                asyncworker,
                c"tryCancelQueuedWork".as_ptr(),
                &mut create,
            ),
            NAPI_OK
        );
        let callback = env_mut(env).unwrap().alloc(Value::Function(Function {
            callback: bcrypt_async_callback,
            data: ptr::null_mut(),
            properties: HashMap::new(),
            _thaw_bridge: None,
            _accessor_owner: None,
        }));
        let echo = env_mut(env).unwrap().alloc(Value::String("echo".into()));
        let workers = env_mut(env).unwrap().alloc(Value::Number(0.0));
        let mut cancelled = ptr::null_mut();
        assert_eq!(
            napi_call_function(
                env,
                asyncworker,
                create,
                3,
                [callback, echo, workers].as_ptr(),
                &mut cancelled,
            ),
            NAPI_OK
        );
        assert_eq!(thaw_napi_run_async_work(), 1);
        assert!(BCRYPT_ASYNC_RESULT
            .get_or_init(|| Mutex::new(None))
            .lock()
            .unwrap()
            .is_none());
        assert_eq!(thaw_napi_unload_all(), 1);
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
            _accessor_owner: None,
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
                {"backend": "inotify"}
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
        let backend = env.alloc(Value::String("inotify".into()));
        let options = env.alloc(Value::Object(HashMap::from([(
            PropertyKey::String("backend".into()),
            backend,
        )])));
        let probe = Box::into_raw(Box::new(ParcelWatcherProbe {
            events: Mutex::new(Vec::new()),
        }));
        let callback = env.alloc(Value::Function(Function {
            callback: parcel_watcher_callback,
            data: probe.cast(),
            properties: HashMap::new(),
            _thaw_bridge: None,
            _accessor_owner: None,
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

#[test]
fn private_napi_result_graph_separates_value_kinds_from_user_fields() {
    let mut env = Env::new();
    let real_date = env.alloc(Value::Date(0.0));
    let timestamp = env.alloc(Value::Number(0.0));
    let ordinary_date_shape = env.alloc(Value::Object(HashMap::from([(
        PropertyKey::String("timestamp".into()),
        timestamp,
    )])));
    let real_undefined = env.alloc(Value::Undefined);
    let truth = env.alloc(Value::Bool(true));
    let ordinary_undefined_shape = env.alloc(Value::Object(HashMap::from([(
        PropertyKey::String("$__thaw_napi_undefined$".into()),
        truth,
    )])));
    let root = env.alloc(Value::Array(vec![
        Some(real_date),
        Some(ordinary_date_shape),
        Some(real_undefined),
        Some(ordinary_undefined_shape),
    ]));
    let graph: serde_json::Value = serde_json::from_str(
        &unsafe { napi_result_graph_for_env(&mut env as NapiEnv, root, true) }.unwrap(),
    )
    .unwrap();
    assert_eq!(graph["nodes"][1]["d"], 0.0);
    assert!(graph["nodes"][2].get("o").is_some());
    assert_eq!(graph["nodes"][0]["a"][2]["u"], 1);
    assert!(graph["nodes"][3].get("o").is_some());
}

#[test]
fn private_napi_result_graph_retains_cycle_aliases() {
    let mut env = Env::new();
    let root = env.alloc(Value::Array(vec![None]));
    let Value::Array(items) = unsafe { root.as_mut() }.unwrap() else {
        unreachable!()
    };
    items[0] = Some(root);
    let graph: serde_json::Value = serde_json::from_str(
        &unsafe { napi_result_graph_for_env(&mut env as NapiEnv, root, true) }.unwrap(),
    )
    .unwrap();
    assert_eq!(graph["root"]["r"], 0);
    assert_eq!(graph["nodes"][0]["a"][0]["r"], 0);
}

#[test]
fn private_napi_event_graph_keeps_user_marker_objects_ordinary() {
    unsafe extern "C" fn capture(
        context: *mut c_void,
        error: *const c_char,
        result: *const c_char,
    ) {
        let output = &mut *(context as *mut Option<(JsonValue, JsonValue)>);
        *output = Some((
            serde_json::from_str(CStr::from_ptr(error).to_str().unwrap()).unwrap(),
            serde_json::from_str(CStr::from_ptr(result).to_str().unwrap()).unwrap(),
        ));
    }
    let env = registered_test_env();
    let zero = env.alloc(Value::Number(0.0));
    let ordinary_date = env.alloc(Value::Object(HashMap::from([(
        PropertyKey::String("timestamp".into()),
        zero,
    )])));
    let real_date = env.alloc(Value::Date(0.0));
    let mut output: Option<(JsonValue, JsonValue)> = None;
    let bridge = Arc::new(ThawCallbackBridge {
        callback: ThawCallback::EventGraph(capture),
        context: (&mut output as *mut Option<(JsonValue, JsonValue)>) as usize,
    });
    let mut info = CallbackInfo {
        args: vec![ordinary_date, real_date],
        this_arg: ptr::null_mut(),
        new_target: ptr::null_mut(),
        data: Arc::as_ptr(&bridge) as *mut c_void,
    };
    unsafe {
        thaw_compiled_callback(&mut *env as NapiEnv, &mut info);
    }
    let (ordinary, date) = output.unwrap();
    assert!(ordinary["nodes"][0].get("o").is_some());
    assert_eq!(date["nodes"][0]["d"], 0.0);
}

#[test]
fn private_napi_argument_graph_preserves_origin_and_aliases() {
    let mut env = Env::new();
    let graph = r#"{"root":{"r":0},"nodes":[{"a":[{"r":1},{"r":2},{"u":1},{"r":3},{"r":2}]},{"d":0},{"o":[["timestamp",{"v":0}]]},{"o":[["$__thaw_napi_undefined$",{"v":true}]]}],"leases":[]}"#;
    let args =
        unsafe { parse_napi_arguments(&mut env as NapiEnv, graph, true, true, None) }.unwrap();
    assert!(matches!(unsafe { value_ref(args[0]) }, Ok(Value::Date(time)) if *time == 0.0));
    assert!(matches!(
        unsafe { value_ref(args[1]) },
        Ok(Value::Object(_))
    ));
    assert!(matches!(
        unsafe { value_ref(args[2]) },
        Ok(Value::Undefined)
    ));
    assert!(matches!(
        unsafe { value_ref(args[3]) },
        Ok(Value::Object(_))
    ));
    assert_eq!(args[1], args[4]);
    // The same envelope is ordinary data on the public plain-JSON ABI.
    assert!(
        unsafe { parse_napi_arguments(&mut env as NapiEnv, graph, true, false, None) }.is_err()
    );
}

#[test]
fn private_napi_value_callback_graph_preserves_arguments_and_result() {
    unsafe extern "C" fn callback(context: *mut c_void, args: *const c_char) -> *const c_char {
        let output = &mut *(context as *mut Option<JsonValue>);
        *output = Some(serde_json::from_str(CStr::from_ptr(args).to_str().unwrap()).unwrap());
        CString::new(r#"{"root":{"u":1},"nodes":[],"leases":[]}"#)
            .unwrap()
            .into_raw()
    }
    let mut env = Env::new();
    let zero = env.alloc(Value::Number(0.0));
    let ordinary_date = env.alloc(Value::Object(HashMap::from([(
        PropertyKey::String("timestamp".into()),
        zero,
    )])));
    let mut output: Option<JsonValue> = None;
    let bridge = Arc::new(ThawCallbackBridge {
        callback: ThawCallback::ValueGraph(callback),
        context: (&mut output as *mut Option<JsonValue>) as usize,
    });
    let mut info = CallbackInfo {
        args: vec![ordinary_date],
        this_arg: ptr::null_mut(),
        new_target: ptr::null_mut(),
        data: Arc::as_ptr(&bridge) as *mut c_void,
    };
    let result = unsafe { thaw_compiled_callback(&mut env as NapiEnv, &mut info) };
    assert!(matches!(unsafe { value_ref(result) }, Ok(Value::Undefined)));
    let graph = output.unwrap();
    assert_eq!(graph["nodes"][0]["a"][0]["r"], 1);
    assert!(graph["nodes"][1].get("o").is_some());
}

#[test]
fn private_napi_graph_wrapper_kinds_survive_roundtrip_without_shape_spoofing() {
    let mut env = Env::new();
    let graph: JsonValue = serde_json::from_str(r#"{"root":{"r":0},"nodes":[{"a":[{"r":1},{"r":2},{"r":3},{"r":4}]},{"m":{"r":5}},{"s":{"r":6}},{"re":["x","g",0]},{"o":[["__thaw_map_entries__",{"r":5}]]},{"a":[]},{"a":[]}],"leases":[]}"#).unwrap();
    let root = napi_graph_value(&mut env, &graph).unwrap();
    let encoded: JsonValue = serde_json::from_str(
        &unsafe { napi_result_graph_for_env(&mut env as NapiEnv, root, true) }.unwrap(),
    )
    .unwrap();
    assert!(encoded["nodes"][1].get("m").is_some());
    assert!(encoded["nodes"][2].get("s").is_some());
    assert!(encoded["nodes"][3].get("re").is_some());
    assert!(encoded["nodes"][4].get("o").is_some());
}

#[test]
fn private_napi_regexp_graph_last_index_keeps_nonfinite_number_kind() {
    for name in ["NaN", "Infinity", "-Infinity"] {
        let mut env = Env::new();
        let graph: JsonValue = serde_json::from_str(&format!(
            r#"{{"root":{{"r":0}},"nodes":[{{"re":["x","g","{name}"]}}],"leases":[]}}"#
        ))
        .unwrap();
        let root = napi_graph_value(&mut env, &graph).unwrap();
        let Value::Object(wrapper) = unsafe { value_ref(root) }.unwrap() else {
            panic!("expected RegExp wrapper")
        };
        let pattern = wrapper[&PropertyKey::String("__thaw_regexp__".into())];
        let Value::Object(fields) = unsafe { value_ref(pattern) }.unwrap() else {
            panic!("expected RegExp pattern")
        };
        let index = fields[&PropertyKey::String("lastIndex".into())];
        assert!(matches!(unsafe { value_ref(index) }, Ok(Value::Number(_))));
        let encoded: JsonValue = serde_json::from_str(
            &unsafe { napi_result_graph_for_env(&mut env as NapiEnv, root, true) }.unwrap(),
        )
        .unwrap();
        assert_eq!(encoded["nodes"][0]["re"][2], name);
    }
}

#[cfg(feature = "quickjs")]
#[test]
fn private_napi_graph_arguments_release_lease_when_export_is_missing() {
    assert_eq!(
        thaw_quickjs::thaw_js_load(c"globalThis.napiLeaseInput = {};".as_ptr()),
        1
    );
    let handle = thaw_quickjs::thaw_js_get_global(c"napiLeaseInput".as_ptr());
    assert_eq!(thaw_quickjs::thaw_js_retain_handle(handle), 1);
    let graph = CString::new(format!(
        r#"{{"root":{{"r":0}},"nodes":[{{"a":[]}}],"leases":[{handle}]}}"#
    ))
    .unwrap();
    let result = unsafe {
        thaw_napi_call_graph_result(c"missingNativeLeaseExport".as_ptr(), graph.as_ptr())
    };
    assert!(!result.error.is_null());
    unsafe {
        thaw_arena::destroy_string(result.error);
    }
    assert_eq!(thaw_quickjs::thaw_js_release_handle(handle), 1);
    assert_eq!(thaw_quickjs::thaw_js_release_handle(handle), 0);
}

#[cfg(feature = "quickjs")]
#[test]
fn public_graph_arguments_release_lease_before_native_callback() {
    unsafe extern "C" fn callback(env: NapiEnv, info: NapiCallbackInfo) -> NapiValue {
        assert_eq!(thaw_napi_begin_shutdown(), 0);
        assert_eq!((*info).args.len(), 0);
        env_mut(env).unwrap().alloc(Value::Number(7.0))
    }
    let mut env = Box::new(Env::new());
    let env_ptr: NapiEnv = &mut *env;
    let function = Function {
        callback,
        data: ptr::null_mut(),
        properties: HashMap::new(),
        _thaw_bridge: None,
        _accessor_owner: None,
    };
    let value = env.alloc(Value::Function(function.clone()));
    HOST.with(|host| {
        let mut host = host.borrow_mut();
        host.functions
            .insert("graph-lease-test::call".into(), function);
        host.exports
            .insert("graph-lease-test::call".into(), (env_ptr as usize, value));
        host.module_envs.push(env);
    });
    assert_eq!(
        thaw_quickjs::thaw_js_load(c"globalThis.napiPublicLeaseInput = {};".as_ptr()),
        1
    );
    let handle = thaw_quickjs::thaw_js_get_global(c"napiPublicLeaseInput".as_ptr());
    assert_eq!(thaw_quickjs::thaw_js_retain_handle(handle), 1);
    let graph = CString::new(format!(
        r#"{{"root":{{"r":0}},"nodes":[{{"a":[]}}],"leases":[{handle}]}}"#
    ))
    .unwrap();
    unsafe {
        let result =
            thaw_napi_call_graph_result(c"graph-lease-test::call".as_ptr(), graph.as_ptr());
        assert!(result.error.is_null());
        thaw_arena::destroy_string(result.value);
        assert_eq!(thaw_quickjs::thaw_js_release_handle(handle), 1);
        assert_eq!(thaw_quickjs::thaw_js_release_handle(handle), 0);
        // The valid lease must still be released when a later list entry is malformed.
        let malformed_handle = thaw_quickjs::thaw_js_get_global(c"napiPublicLeaseInput".as_ptr());
        assert_eq!(thaw_quickjs::thaw_js_retain_handle(malformed_handle), 1);
        let malformed = CString::new(format!(
            r#"{{"root":{{"r":0}},"nodes":[{{"a":[]}}],"leases":[{malformed_handle},0]}}"#
        ))
        .unwrap();
        let result =
            thaw_napi_call_graph_result(c"graph-lease-test::call".as_ptr(), malformed.as_ptr());
        assert!(!result.error.is_null());
        assert_eq!(
            thaw_arena::NativeStr::from_ptr(result.error).to_bytes(),
            b"invalid native argument graph lease"
        );
        thaw_arena::destroy_string(result.error);
        assert_eq!(thaw_quickjs::thaw_js_release_handle(malformed_handle), 1);
        assert_eq!(thaw_quickjs::thaw_js_release_handle(malformed_handle), 0);
    }
    HOST.with(|host| {
        let mut host = host.borrow_mut();
        host.functions.remove("graph-lease-test::call");
        host.exports.remove("graph-lease-test::call");
        host.module_envs
            .retain(|entry| (&**entry as *const Env).cast_mut() != env_ptr);
    });
}

#[test]
fn quickjs_private_wire_tracks_only_native_date_and_nonfinite_origins() {
    let mut env = Env::new();
    let date = env.alloc(Value::Date(f64::NAN));
    let nan = env.alloc(Value::Number(f64::NAN));
    let infinity = env.alloc(Value::Number(f64::INFINITY));
    let minus_infinity = env.alloc(Value::Number(f64::NEG_INFINITY));
    let zero = env.alloc(Value::Number(0.0));
    let literal_date = env.alloc(Value::Object(HashMap::from([(
        PropertyKey::String("timestamp".into()),
        zero,
    )])));
    let error_text = env.alloc(Value::String("ordinary".into()));
    let literal_error = env.alloc(Value::Object(HashMap::from([(
        PropertyKey::String("__thaw_napi_error__".into()),
        error_text,
    )])));
    let nested = env.alloc(Value::Object(HashMap::from([
        (PropertyKey::String("__proto__".into()), date),
        (PropertyKey::String("error".into()), literal_error),
    ])));
    let numeric_key_object = env.alloc(Value::Object(HashMap::from([(
        PropertyKey::String("0".into()),
        nan,
    )])));
    let root = env.alloc(Value::Array(vec![
        Some(literal_date),
        Some(nested),
        Some(nan),
        Some(infinity),
        Some(minus_infinity),
        Some(numeric_key_object),
    ]));
    let wire = unsafe { quickjs_reference_wire(&mut env as NapiEnv, root, true) }.unwrap();
    assert_eq!(wire.value[0]["timestamp"], 0.0);
    assert_eq!(wire.value[2], JsonValue::Null);
    assert!(wire.origins.contains(&serde_json::json!([[0], "plain"])));
    assert!(wire.origins.contains(&serde_json::json!([[1], "plain"])));
    assert!(wire
        .origins
        .contains(&serde_json::json!([[1, "__proto__"], "date", null])));
    assert!(wire
        .origins
        .contains(&serde_json::json!([[1, "error"], "plain"])));
    assert!(wire
        .origins
        .contains(&serde_json::json!([[2], "nonfinite", "NaN"])));
    assert!(wire
        .origins
        .contains(&serde_json::json!([[3], "nonfinite", "Infinity"])));
    assert!(wire
        .origins
        .contains(&serde_json::json!([[4], "nonfinite", "-Infinity"])));
    assert!(wire
        .origins
        .contains(&serde_json::json!([[5, "0"], "nonfinite", "NaN"])));
    assert!(!wire
        .origins
        .contains(&serde_json::json!([[5, 0], "nonfinite", "NaN"])));
    env.quickjs_references.insert(77, date);
    let referenced = unsafe { quickjs_reference_wire(&mut env as NapiEnv, root, true) }.unwrap();
    assert_eq!(
        referenced.value[1]["__proto__"]["__thaw_napi_ref__"],
        serde_json::json!(77)
    );
    assert!(!referenced
        .origins
        .iter()
        .any(|entry| entry[0] == serde_json::json!([1, "__proto__"])));
    // Public plain JSON keeps its existing lossy value contract.
    assert_eq!(
        unsafe { json_from_value_with_undefined(date, true) }.unwrap(),
        JsonValue::Null
    );
    assert_eq!(
        unsafe { json_from_value_with_undefined(nan, true) }.unwrap(),
        JsonValue::Null
    );
    // Public result walkers resolve the live owner before invoking accessors.
    let env = Box::new(env);
    HOST.with(|host| host.borrow_mut().module_envs.push(env));
    assert_eq!(
        unsafe { json_from_value_with_undefined(literal_date, true) }.unwrap()["timestamp"],
        0.0
    );
    HOST.with(|host| {
        drop(host.borrow_mut().module_envs.pop());
    });
}

// Proposed thaw-napi/src/tests.rs integration test. UNRUN; source-only.
// A native destructor executes inside libc::dlclose. Nested shutdown must
// fail while the outer release retains finalized Env Boxes and mapped libs.
#[cfg(target_os = "linux")]
#[test]
fn library_destructor_cannot_reenter_shutdown_or_load() {
    let _guard = lock_async_test();
    let dir =
        std::env::temp_dir().join(format!("thaw-napi-dlclose-reentry-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let trace = dir.join("destructor.trace");
    let addon = dir.join("reentry.node");
    let addon_c = dir.join("reentry.c");
    let text = format!(
        r#"
        #include <stdint.h>
        #include <stdio.h>
        typedef void* napi_env; typedef void* napi_value;
        extern uint8_t thaw_napi_unload_all(void);
        extern uint8_t thaw_napi_finish_shutdown(void);
        extern uint8_t thaw_napi_load(const char*);
        __attribute__((visibility("default"))) napi_value napi_register_module_v1(napi_env env,napi_value exports) {{
            (void)env; return exports;
        }}
        __attribute__((destructor)) static void nested_release(void) {{
            unsigned nested_unload = thaw_napi_unload_all();
            unsigned nested_finish = thaw_napi_finish_shutdown();
            unsigned nested_load = thaw_napi_load("{}");
            FILE* trace = fopen("{}", "w");
            if (trace) {{ fprintf(trace,"%u,%u,%u\n",nested_unload,nested_finish,nested_load); fclose(trace); }}
        }}
    "#,
        addon.display(),
        trace.display()
    );
    std::fs::write(&addon_c, text).unwrap();
    assert!(std::process::Command::new("cc")
        .args(["-shared", "-fPIC"])
        .arg(&addon_c)
        .arg("-o")
        .arg(&addon)
        .status()
        .unwrap()
        .success());
    let path = std::ffi::CString::new(addon.to_string_lossy().as_bytes()).unwrap();
    assert_eq!(unsafe { thaw_napi_load(path.as_ptr()) }, 1);
    assert_eq!(thaw_napi_begin_shutdown(), 1);
    assert_eq!(thaw_napi_poll_shutdown(), 1);
    assert_eq!(thaw_napi_finish_shutdown(), 1);
    assert_eq!(std::fs::read_to_string(&trace).unwrap(), "0,0,0\n");
    let _ = std::fs::remove_dir_all(dir);
}

// Source-only regression for the graph callback's borrowed-Env boundary.
// The external callback may reenter N-API; its synthetic encode root must
// already be gone, and a nested shutdown must observe the outer dispatch.
#[test]
fn graph_callback_reentry_has_no_synthetic_root_or_nested_shutdown() {
    unsafe extern "C" fn callback(context: *mut c_void, _args: *const c_char) -> *const c_char {
        let env = context as NapiEnv;
        assert_eq!(thaw_napi_begin_shutdown(), 0);
        assert_eq!((*env).values.len(), 1);
        assert!(env_mut(env).is_ok());
        CString::new(r#"{"root":{"u":1},"nodes":[],"leases":[]}"#)
            .unwrap()
            .into_raw()
    }
    let mut env = Env::new();
    let argument = env.alloc(Value::Number(7.0));
    let bridge = Arc::new(ThawCallbackBridge {
        callback: ThawCallback::ValueGraph(callback),
        context: (&mut env as *mut Env) as usize,
    });
    let mut info = CallbackInfo {
        args: vec![argument],
        this_arg: ptr::null_mut(),
        new_target: ptr::null_mut(),
        data: Arc::as_ptr(&bridge) as *mut c_void,
    };
    let result = unsafe { thaw_compiled_callback(&mut env, &mut info) };
    assert!(matches!(unsafe { value_ref(result) }, Ok(Value::Undefined)));
    assert_eq!(env.values.len(), 2); // argument + decoded result, no encode root
}

#[test]
fn graph_callback_encode_error_releases_synthetic_root() {
    static CALLED: AtomicBool = AtomicBool::new(false);
    unsafe extern "C" fn unexpected_callback(_: *mut c_void, _: *const c_char) -> *const c_char {
        CALLED.store(true, std::sync::atomic::Ordering::SeqCst);
        ptr::null()
    }
    let mut env = Env::new();
    let external = env.alloc(Value::External(ptr::null_mut()));
    let bridge = Arc::new(ThawCallbackBridge {
        callback: ThawCallback::ValueGraph(unexpected_callback),
        context: 0,
    });
    let mut info = CallbackInfo {
        args: vec![external],
        this_arg: ptr::null_mut(),
        new_target: ptr::null_mut(),
        data: Arc::as_ptr(&bridge) as *mut c_void,
    };
    assert!(unsafe { thaw_compiled_callback(&mut env, &mut info) }.is_null());
    assert!(!CALLED.load(std::sync::atomic::Ordering::SeqCst));
    assert_eq!(env.values.len(), 1);
}
// Source-only regression: a callback may ask to shut down its Host, but the
// surrounding conversion/dispatch must keep the Env live until return.
#[test]
fn value_callback_reentry_cannot_begin_shutdown() {
    unsafe extern "C" fn callback(_: *mut c_void, _: *const c_char) -> *const c_char {
        assert_eq!(thaw_napi_begin_shutdown(), 0);
        CString::new("null").unwrap().into_raw()
    }
    let mut env = Env::new();
    let bridge = Arc::new(ThawCallbackBridge {
        callback: ThawCallback::Value(callback),
        context: 0,
    });
    let mut info = CallbackInfo {
        args: Vec::new(),
        this_arg: ptr::null_mut(),
        new_target: ptr::null_mut(),
        data: Arc::as_ptr(&bridge) as *mut c_void,
    };
    let result = unsafe { thaw_compiled_callback(&mut env, &mut info) };
    assert!(matches!(unsafe { value_ref(result) }, Ok(Value::Null)));
}

#[test]
fn snapshot_walkers_read_own_getters_with_original_receiver() {
    unsafe extern "C" fn getter(env: NapiEnv, info: NapiCallbackInfo) -> NapiValue {
        let info = info.as_ref().unwrap();
        assert_eq!(info.this_arg as usize, info.data as usize);
        env_mut(env)
            .unwrap()
            .alloc(Value::String("from getter".into()))
    }
    let mut env = Box::new(Env::new());
    let env_ptr: NapiEnv = &mut *env;
    let child = env.alloc(Value::Object(HashMap::new()));
    let descriptor = NapiPropertyDescriptor {
        utf8name: c"computed".as_ptr(),
        name: ptr::null_mut(),
        method: None,
        getter: Some(getter),
        setter: None,
        value: ptr::null_mut(),
        attributes: 0,
        data: child.cast(),
    };
    unsafe {
        assert_eq!(
            napi_define_properties(env_ptr, child, 1, &descriptor),
            NAPI_OK
        );
    }
    let number = env.alloc(Value::Number(42.0));
    let Value::Object(fields) = (unsafe { child.as_mut() }).unwrap() else {
        unreachable!()
    };
    fields.insert(PropertyKey::String("a\0b".into()), number);
    let array = env.alloc(Value::Array(vec![None]));
    let index_descriptor = NapiPropertyDescriptor {
        utf8name: c"0".as_ptr(),
        data: array.cast(),
        ..descriptor
    };
    unsafe {
        assert_eq!(
            napi_define_properties(env_ptr, array, 1, &index_descriptor),
            NAPI_OK
        );
    }
    let root = env.alloc(Value::Object(HashMap::from([
        (PropertyKey::String("nested".into()), child),
        (PropertyKey::String("items".into()), array),
    ])));
    HOST.with(|host| host.borrow_mut().module_envs.push(env));
    let before_keys = unsafe { (*env_ptr).values.len() };
    for _ in 0..3 {
        let keys = unsafe { snapshot_own_string_keys(env_ptr, child) }.unwrap();
        assert!(keys.items.iter().any(|(name, _)| name == "computed"));
    }
    assert_eq!(unsafe { (*env_ptr).values.len() }, before_keys);
    let plain = unsafe { json_from_value_with_undefined(root, true) }.unwrap();
    assert_eq!(plain["nested"]["computed"], "from getter");
    assert_eq!(plain["nested"]["a\0b"], 42.0);
    assert_eq!(plain["items"][0], "from getter");
    let wire = unsafe { quickjs_reference_wire(env_ptr, root, true) }.unwrap();
    assert_eq!(wire.value["nested"]["computed"], "from getter");
    assert_eq!(wire.value["nested"]["a\0b"], 42.0);
    assert_eq!(wire.value["items"][0], "from getter");
    HOST.with(|host| {
        drop(host.borrow_mut().module_envs.pop());
    });
}

#[test]
fn graph_snapshot_reads_nested_getter_with_original_receiver() {
    unsafe extern "C" fn getter(env: NapiEnv, info: NapiCallbackInfo) -> NapiValue {
        let info = info.as_ref().unwrap();
        assert_eq!(info.this_arg as usize, info.data as usize);
        env_mut(env).unwrap().alloc(Value::Number(9.0))
    }
    let mut env = Env::new();
    let env_ptr: NapiEnv = &mut env;
    let child = env.alloc(Value::Object(HashMap::new()));
    let descriptor = NapiPropertyDescriptor {
        utf8name: c"computed".as_ptr(),
        name: ptr::null_mut(),
        method: None,
        getter: Some(getter),
        setter: None,
        value: ptr::null_mut(),
        attributes: 0,
        data: child.cast(),
    };
    unsafe {
        assert_eq!(
            napi_define_properties(env_ptr, child, 1, &descriptor),
            NAPI_OK
        );
    }
    let root = env.alloc(Value::Array(vec![Some(child)]));
    let graph: JsonValue =
        serde_json::from_str(&unsafe { napi_result_graph_for_env(env_ptr, root, true) }.unwrap())
            .unwrap();
    assert_eq!(
        graph["nodes"][1]["o"][0],
        serde_json::json!(["computed", {"v": 9.0}])
    );
}

#[test]
fn snapshot_getter_exception_is_consumed_once() {
    unsafe extern "C" fn getter(env: NapiEnv, _: NapiCallbackInfo) -> NapiValue {
        assert_eq!(
            napi_throw_type_error(env, ptr::null(), c"snapshot getter failed".as_ptr()),
            NAPI_OK
        );
        ptr::null_mut()
    }
    let mut env = Box::new(Env::new());
    let env_ptr: NapiEnv = &mut *env;
    let object = env.alloc(Value::Object(HashMap::new()));
    let descriptor = NapiPropertyDescriptor {
        utf8name: c"computed".as_ptr(),
        name: ptr::null_mut(),
        method: None,
        getter: Some(getter),
        setter: None,
        value: ptr::null_mut(),
        attributes: 0,
        data: ptr::null_mut(),
    };
    unsafe {
        assert_eq!(
            napi_define_properties(env_ptr, object, 1, &descriptor),
            NAPI_OK
        );
    }
    HOST.with(|host| host.borrow_mut().module_envs.push(env));
    let error = unsafe { json_from_value_with_undefined(object, true) }.unwrap_err();
    assert!(error.contains("snapshot getter failed"), "{error}");
    assert!(unsafe { (*env_ptr).exception.is_none() });
    let error = unsafe { quickjs_reference_wire(env_ptr, object, true) }
        .err()
        .unwrap();
    assert!(error.contains("snapshot getter failed"), "{error}");
    assert!(unsafe { (*env_ptr).exception.is_none() });
    HOST.with(|host| {
        drop(host.borrow_mut().module_envs.pop());
    });
}

#[test]
fn getter_reentry_updates_later_snapshot_property() {
    unsafe extern "C" fn getter(env: NapiEnv, info: NapiCallbackInfo) -> NapiValue {
        let object = info.as_ref().unwrap().this_arg;
        let nested = env_mut(env).unwrap().alloc(Value::Object(HashMap::new()));
        assert!(json_from_value_with_undefined_for_env(env, nested, true)
            .unwrap()
            .is_object());
        let changed = env_mut(env).unwrap().alloc(Value::Number(99.0));
        assert_eq!(
            napi_set_named_property(env, object, c"second".as_ptr(), changed),
            NAPI_OK
        );
        changed
    }
    let mut env = Box::new(Env::new());
    let env_ptr: NapiEnv = &mut *env;
    let old = env.alloc(Value::Number(1.0));
    let object = env.alloc(Value::Object(HashMap::from([(
        PropertyKey::String("second".into()),
        old,
    )])));
    let descriptor = NapiPropertyDescriptor {
        utf8name: c"first".as_ptr(),
        name: ptr::null_mut(),
        method: None,
        getter: Some(getter),
        setter: None,
        value: ptr::null_mut(),
        attributes: NAPI_DEFAULT_PROPERTY_ATTRIBUTES,
        data: ptr::null_mut(),
    };
    unsafe {
        assert_eq!(
            napi_define_properties(env_ptr, object, 1, &descriptor),
            NAPI_OK
        );
    }
    HOST.with(|host| host.borrow_mut().module_envs.push(env));
    let plain = unsafe { json_from_value_with_undefined(object, true) }.unwrap();
    assert_eq!(plain["first"], 99.0);
    assert_eq!(plain["second"], 99.0);
    let private = unsafe { quickjs_reference_wire(env_ptr, object, true) }.unwrap();
    assert_eq!(private.value["second"], 99.0);
    HOST.with(|host| {
        drop(host.borrow_mut().module_envs.pop());
    });
}

#[test]
fn settled_promise_getter_can_retry_settlement_without_borrow_panic() {
    unsafe extern "C" fn getter(env: NapiEnv, info: NapiCallbackInfo) -> NapiValue {
        let deferred = info.as_ref().unwrap().data.cast::<Deferred>();
        let retry_value = env_mut(env).unwrap().alloc(Value::Undefined);
        assert_eq!(
            napi_resolve_deferred(env, deferred, retry_value),
            NAPI_GENERIC_FAILURE
        );
        env_mut(env).unwrap().alloc(Value::Number(7.0))
    }
    let mut env = Box::new(Env::new());
    let env_ptr: NapiEnv = &mut *env;
    let object = env.alloc(Value::Object(HashMap::new()));
    let mut deferred = ptr::null_mut();
    let mut promise = ptr::null_mut();
    unsafe {
        assert_eq!(
            napi_create_promise(env_ptr, &mut deferred, &mut promise),
            NAPI_OK
        );
        let descriptor = NapiPropertyDescriptor {
            utf8name: c"computed".as_ptr(),
            name: ptr::null_mut(),
            method: None,
            getter: Some(getter),
            setter: None,
            value: ptr::null_mut(),
            attributes: NAPI_DEFAULT_PROPERTY_ATTRIBUTES,
            data: deferred.cast(),
        };
        assert_eq!(
            napi_define_properties(env_ptr, object, 1, &descriptor),
            NAPI_OK
        );
        assert_eq!(napi_resolve_deferred(env_ptr, deferred, object), NAPI_OK);
    }
    HOST.with(|host| host.borrow_mut().module_envs.push(env));
    let plain = unsafe { json_from_value_with_undefined_for_env(env_ptr, promise, true) }.unwrap();
    assert_eq!(plain["computed"], 7.0);
    #[cfg(feature = "quickjs")]
    unsafe {
        let target = CString::new((promise as u64).to_string()).unwrap();
        let wire = thaw_napi_handle_bridge(
            c"promise_state".as_ptr(),
            target.as_ptr(),
            c"".as_ptr(),
            c"[]".as_ptr(),
        );
        let result = CString::from_raw(wire.cast_mut()).into_string().unwrap();
        assert!(result.contains("\"kind\":\"resolved\""), "{result}");
        assert!(result.contains("\"computed\":7.0"), "{result}");
    }
    HOST.with(|host| {
        drop(host.borrow_mut().module_envs.pop());
    });
}

#[test]
fn rejected_promise_object_getter_can_retry_settlement_without_borrow_panic() {
    unsafe extern "C" fn getter(env: NapiEnv, info: NapiCallbackInfo) -> NapiValue {
        let deferred = info.as_ref().unwrap().data.cast::<Deferred>();
        let retry_value = env_mut(env).unwrap().alloc(Value::Undefined);
        assert_eq!(
            napi_reject_deferred(env, deferred, retry_value),
            NAPI_GENERIC_FAILURE
        );
        env_mut(env)
            .unwrap()
            .alloc(Value::String("original rejection".into()))
    }
    let mut env = Box::new(Env::new());
    let env_ptr: NapiEnv = &mut *env;
    let object = env.alloc(Value::Object(HashMap::new()));
    let mut deferred = ptr::null_mut();
    let mut promise = ptr::null_mut();
    unsafe {
        assert_eq!(
            napi_create_promise(env_ptr, &mut deferred, &mut promise),
            NAPI_OK
        );
        let descriptor = NapiPropertyDescriptor {
            utf8name: c"message".as_ptr(),
            name: ptr::null_mut(),
            method: None,
            getter: Some(getter),
            setter: None,
            value: ptr::null_mut(),
            attributes: NAPI_DEFAULT_PROPERTY_ATTRIBUTES,
            data: deferred.cast(),
        };
        assert_eq!(
            napi_define_properties(env_ptr, object, 1, &descriptor),
            NAPI_OK
        );
        assert_eq!(napi_reject_deferred(env_ptr, deferred, object), NAPI_OK);
    }
    HOST.with(|host| host.borrow_mut().module_envs.push(env));
    let error =
        unsafe { json_from_value_with_undefined_for_env(env_ptr, promise, true) }.unwrap_err();
    assert!(error.contains("original rejection"), "{error}");
    #[cfg(feature = "quickjs")]
    unsafe {
        let target = CString::new((promise as u64).to_string()).unwrap();
        let wire = thaw_napi_handle_bridge(
            c"promise_state".as_ptr(),
            target.as_ptr(),
            c"".as_ptr(),
            c"[]".as_ptr(),
        );
        let result = CString::from_raw(wire.cast_mut()).into_string().unwrap();
        assert!(result.contains("\"kind\":\"rejected\""), "{result}");
        assert!(result.contains("original rejection"), "{result}");
    }
    HOST.with(|host| {
        drop(host.borrow_mut().module_envs.pop());
    });
}

#[test]
fn exception_description_keeps_original_data_without_running_getter() {
    static DESCRIPTION_GETTER_RAN: AtomicBool = AtomicBool::new(false);
    unsafe extern "C" fn throwing_getter(env: NapiEnv, _: NapiCallbackInfo) -> NapiValue {
        DESCRIPTION_GETTER_RAN.store(true, std::sync::atomic::Ordering::SeqCst);
        assert_eq!(
            napi_throw_type_error(env, ptr::null(), c"secondary getter failure".as_ptr()),
            NAPI_OK
        );
        ptr::null_mut()
    }
    DESCRIPTION_GETTER_RAN.store(false, std::sync::atomic::Ordering::SeqCst);
    let mut env = Env::new();
    let env_ptr: NapiEnv = &mut env;
    let message = env.alloc(Value::String("original failure".into()));
    let thrown = env.alloc(Value::Object(HashMap::from([(
        PropertyKey::String("message".into()),
        message,
    )])));
    let descriptor = NapiPropertyDescriptor {
        utf8name: c"danger".as_ptr(),
        name: ptr::null_mut(),
        method: None,
        getter: Some(throwing_getter),
        setter: None,
        value: ptr::null_mut(),
        attributes: NAPI_DEFAULT_PROPERTY_ATTRIBUTES,
        data: ptr::null_mut(),
    };
    unsafe {
        assert_eq!(
            napi_define_properties(env_ptr, thrown, 1, &descriptor),
            NAPI_OK
        );
        assert_eq!(napi_throw(env_ptr, thrown), NAPI_OK);
        let report = take_env_exception(env_ptr).unwrap_err();
        assert!(report.contains("original failure"), "{report}");
        assert!(!report.contains("secondary getter failure"), "{report}");
        assert!(!DESCRIPTION_GETTER_RAN.load(std::sync::atomic::Ordering::SeqCst));
        assert!(env.exception.is_none());
    }
}

#[test]
fn snapshot_walkers_use_cross_environment_child_and_promise_owner() {
    unsafe extern "C" fn getter(env: NapiEnv, info: NapiCallbackInfo) -> NapiValue {
        assert_eq!(env as usize, info.as_ref().unwrap().data as usize);
        env_mut(env).unwrap().alloc(Value::Number(9.0))
    }
    let mut first = Box::new(Env::new());
    let mut second = Box::new(Env::new());
    let first_env: NapiEnv = &mut *first;
    let second_env: NapiEnv = &mut *second;
    let root = first.alloc(Value::Object(HashMap::new()));
    let child_undefined = second.alloc(Value::Undefined);
    let child = second.alloc(Value::Object(HashMap::from([(
        PropertyKey::String("optional".into()),
        child_undefined,
    )])));
    let descriptor = NapiPropertyDescriptor {
        utf8name: c"computed".as_ptr(),
        name: ptr::null_mut(),
        method: None,
        getter: Some(getter),
        setter: None,
        value: ptr::null_mut(),
        attributes: NAPI_DEFAULT_PROPERTY_ATTRIBUTES,
        data: second_env.cast(),
    };
    unsafe {
        assert_eq!(
            napi_define_properties(second_env, child, 1, &descriptor),
            NAPI_OK
        );
    }
    let mut deferred = ptr::null_mut();
    let mut promise = ptr::null_mut();
    unsafe {
        assert_eq!(
            napi_create_promise(first_env, &mut deferred, &mut promise),
            NAPI_OK
        );
    }
    HOST.with(|host| {
        let mut host = host.borrow_mut();
        host.module_envs.push(first);
        host.module_envs.push(second);
    });
    unsafe {
        assert_eq!(
            napi_set_named_property(first_env, root, c"child".as_ptr(), child),
            NAPI_OK
        );
        assert_eq!(napi_resolve_deferred(first_env, deferred, child), NAPI_OK);
        let plain = json_from_value_with_undefined_for_env(first_env, root, true).unwrap();
        assert_eq!(plain["child"]["computed"], 9.0);
        assert_eq!(
            plain["child"]["optional"],
            serde_json::json!({ (TYPED_UNDEFINED_KEY): true })
        );
        let settled = json_from_value_with_undefined_for_env(first_env, promise, true).unwrap();
        assert_eq!(settled["computed"], 9.0);
        assert_eq!(
            settled["optional"],
            serde_json::json!({ (TYPED_UNDEFINED_KEY): true })
        );
        let private = quickjs_reference_wire(first_env, root, true).unwrap();
        assert_eq!(private.value["child"]["computed"], 9.0);
        #[cfg(feature = "quickjs")]
        {
            let target = CString::new((promise as u64).to_string()).unwrap();
            let wire = thaw_napi_handle_bridge(
                c"promise_state".as_ptr(),
                target.as_ptr(),
                c"".as_ptr(),
                c"[]".as_ptr(),
            );
            let result = CString::from_raw(wire.cast_mut()).into_string().unwrap();
            assert!(result.contains("\"computed\":9.0"), "{result}");
        }
    }
    HOST.with(|host| {
        let mut host = host.borrow_mut();
        drop(host.module_envs.pop());
        drop(host.module_envs.pop());
    });
}

#[test]
fn graph_snapshot_uses_cross_environment_accessor_owner() {
    unsafe extern "C" fn getter(env: NapiEnv, info: NapiCallbackInfo) -> NapiValue {
        assert_eq!(env as usize, info.as_ref().unwrap().data as usize);
        env_mut(env).unwrap().alloc(Value::Number(11.0))
    }
    let mut first = Box::new(Env::new());
    let mut second = Box::new(Env::new());
    let first_env: NapiEnv = &mut *first;
    let second_env: NapiEnv = &mut *second;
    let root = first.alloc(Value::Array(vec![None]));
    let child = second.alloc(Value::Object(HashMap::new()));
    let descriptor = NapiPropertyDescriptor {
        utf8name: c"computed".as_ptr(),
        name: ptr::null_mut(),
        method: None,
        getter: Some(getter),
        setter: None,
        value: ptr::null_mut(),
        attributes: NAPI_DEFAULT_PROPERTY_ATTRIBUTES,
        data: second_env.cast(),
    };
    unsafe {
        assert_eq!(
            napi_define_properties(second_env, child, 1, &descriptor),
            NAPI_OK
        );
    }
    HOST.with(|host| {
        let mut host = host.borrow_mut();
        host.module_envs.push(first);
        host.module_envs.push(second);
    });
    unsafe {
        assert_eq!(napi_set_element(first_env, root, 0, child), NAPI_OK);
        let graph = napi_result_graph_for_env(first_env, root, true).unwrap();
        assert!(graph.contains("\"computed\""), "{graph}");
        assert!(graph.contains("11.0"), "{graph}");
    }
    HOST.with(|host| {
        let mut host = host.borrow_mut();
        drop(host.module_envs.pop());
        drop(host.module_envs.pop());
    });
}

#[test]
fn cross_environment_registered_function_reference_survives_private_snapshot() {
    unsafe extern "C" fn noop(env: NapiEnv, _: NapiCallbackInfo) -> NapiValue {
        env_mut(env).unwrap().alloc(Value::Undefined)
    }
    unsafe extern "C" fn returns_registered(_: NapiEnv, info: NapiCallbackInfo) -> NapiValue {
        info.as_ref().unwrap().data.cast::<Value>()
    }
    let mut first = Box::new(Env::new());
    let mut second = Box::new(Env::new());
    let first_env: NapiEnv = &mut *first;
    let root = first.alloc(Value::Object(HashMap::new()));
    let function = || {
        Value::Function(Function {
            callback: noop,
            data: ptr::null_mut(),
            properties: HashMap::new(),
            _thaw_bridge: None,
            _accessor_owner: None,
        })
    };
    let registered = second.alloc(function());
    let ordinary = second.alloc(function());
    second.quickjs_references.insert(991, registered);
    let descriptor = NapiPropertyDescriptor {
        utf8name: c"fromGetter".as_ptr(),
        name: ptr::null_mut(),
        method: None,
        getter: Some(returns_registered),
        setter: None,
        value: ptr::null_mut(),
        attributes: NAPI_DEFAULT_PROPERTY_ATTRIBUTES,
        data: registered.cast(),
    };
    unsafe {
        assert_eq!(
            napi_define_properties(first_env, root, 1, &descriptor),
            NAPI_OK
        );
    }
    HOST.with(|host| {
        let mut host = host.borrow_mut();
        host.module_envs.push(first);
        host.module_envs.push(second);
    });
    unsafe {
        assert_eq!(
            napi_set_named_property(first_env, root, c"registered".as_ptr(), registered),
            NAPI_OK
        );
        assert_eq!(
            napi_set_named_property(first_env, root, c"ordinary".as_ptr(), ordinary),
            NAPI_OK
        );
        let wire = quickjs_reference_wire(first_env, root, true).unwrap();
        assert_eq!(wire.value["registered"]["__thaw_napi_ref__"], 991);
        assert_eq!(wire.value["fromGetter"]["__thaw_napi_ref__"], 991);
        assert!(wire.value.get("ordinary").is_none());
    }
    HOST.with(|host| {
        let mut host = host.borrow_mut();
        drop(host.module_envs.pop());
        drop(host.module_envs.pop());
    });
}

#[test]
fn cyclic_exception_data_reports_original_failure_without_recursing_forever() {
    let mut env = Env::new();
    let env_ptr: NapiEnv = &mut env;
    let object = env.alloc(Value::Object(HashMap::new()));
    let Value::Object(fields) = (unsafe { object.as_mut() }).unwrap() else {
        unreachable!()
    };
    fields.insert(PropertyKey::String("self".into()), object);
    let array = env.alloc(Value::Array(vec![None]));
    let Value::Array(items) = (unsafe { array.as_mut() }).unwrap() else {
        unreachable!()
    };
    items[0] = Some(array);
    let mut deferred = ptr::null_mut();
    let mut promise = ptr::null_mut();
    unsafe {
        assert_eq!(
            napi_create_promise(env_ptr, &mut deferred, &mut promise),
            NAPI_OK
        );
        assert_eq!(napi_resolve_deferred(env_ptr, deferred, promise), NAPI_OK);
        for thrown in [object, array, promise] {
            assert_eq!(napi_throw(env_ptr, thrown), NAPI_OK);
            let report = take_env_exception(env_ptr).unwrap_err();
            assert_eq!(
                report,
                "native addon threw an unserializable exception value"
            );
            assert!(env.exception.is_none());
        }
    }
}

#[test]
fn repeated_acyclic_exception_child_is_serialized_on_each_branch() {
    let mut env = Env::new();
    let env_ptr: NapiEnv = &mut env;
    let code = env.alloc(Value::Number(5.0));
    let shared = env.alloc(Value::Object(HashMap::from([(
        PropertyKey::String("code".into()),
        code,
    )])));
    let root = env.alloc(Value::Object(HashMap::from([
        (PropertyKey::String("left".into()), shared),
        (PropertyKey::String("right".into()), shared),
    ])));
    unsafe {
        assert_eq!(napi_throw(env_ptr, root), NAPI_OK);
        let report = take_env_exception(env_ptr).unwrap_err();
        let parsed: JsonValue = serde_json::from_str(&report).unwrap();
        assert_eq!(parsed["left"]["code"], 5.0);
        assert_eq!(parsed["right"]["code"], 5.0);
        assert!(env.exception.is_none());
    }
}

// Unrun source regression: a graph hdl needs one Env retain beyond the
// producer's temporary transfer lease; repeated references share one carrier.
#[cfg(feature = "quickjs")]
#[test]
fn live_quickjs_graph_handle_survives_lease_and_roundtrips_mutation() {
    assert_eq!(
        thaw_quickjs::thaw_js_load(c"globalThis.napiLiveRoundtrip = { x: 4 };".as_ptr()),
        1
    );
    let handle = thaw_quickjs::thaw_js_get_global(c"napiLiveRoundtrip".as_ptr());
    assert_ne!(handle, 0);
    assert_eq!(thaw_quickjs::thaw_js_retain_handle(handle), 1); // wire transfer
    let graph = format!(
        r#"{{"root":{{"r":0}},"nodes":[{{"a":[{{"r":1}},{{"r":1}}]}},{{"hdl":{handle}}}],"leases":[{handle}]}}"#
    );
    let mut env = Env::new();
    let root = unsafe { parse_napi_graph_value(&mut env as NapiEnv, &graph) }.unwrap();
    let Value::Array(children) = unsafe { value_ref(root) }.unwrap() else {
        panic!("array root")
    };
    let live = children[0].unwrap();
    assert_eq!(Some(live), children[1]);
    assert!(
        matches!(unsafe { value_ref(live) }, Ok(Value::QuickJsHandle { handle: id, .. }) if *id == handle)
    );
    assert_eq!(thaw_quickjs::thaw_js_release_handle(handle), 1); // original only
    let nine = env.alloc(Value::Number(9.0));
    assert_eq!(
        unsafe { napi_set_named_property(&mut env, live, c"x".as_ptr(), nine) },
        NAPI_OK
    );
    let property = thaw_quickjs::thaw_js_get_property_result(handle, c"x".as_ptr());
    assert!(property.error.is_null());
    assert_eq!(unsafe { qjs_query(property.value, 2) }.unwrap(), "9");
    assert_eq!(thaw_quickjs::thaw_js_release_handle(property.value), 1);
    let encoded: JsonValue =
        serde_json::from_str(&unsafe { napi_result_graph_for_env(&mut env, live, true) }.unwrap())
            .unwrap();
    assert_eq!(encoded["nodes"][0]["hdl"], handle);
    assert_eq!(encoded["leases"][0], handle);
    assert_eq!(thaw_quickjs::thaw_js_release_handle(handle), 1); // outbound wire lease
    drop(env); // Env-owned retain
    assert_eq!(thaw_quickjs::thaw_js_release_handle(handle), 0);
}

// Unrun ownership/order control: a JS getter/setter installed on a native
// object retains its callbacks after the defining graph packet is gone.
// Replacing the descriptor releases those roots after the native mutation
// scope, including when the getter itself has entered QuickJS.
#[cfg(feature = "quickjs")]
#[test]
fn graph_defined_native_accessor_owns_callbacks_until_replaced() {
    thaw_quickjs::register_napi_bridge(
        thaw_napi_export_names,
        thaw_napi_call_typed_bridge,
        thaw_napi_handle_bridge,
        thaw_napi_poll_async_work,
        thaw_napi_async_work_pending,
        thaw_napi_graph_owner,
        release_napi_graph_reference,
    );
    assert_eq!(thaw_quickjs::thaw_js_load(c"globalThis.napiLiveGetter = function(){ return this.seed + 1 }; globalThis.napiLiveSetter = function(value){ this.seed = value };".as_ptr()), 1);
    let getter = thaw_quickjs::thaw_js_get_global(c"napiLiveGetter".as_ptr());
    let setter = thaw_quickjs::thaw_js_get_global(c"napiLiveSetter".as_ptr());
    assert_ne!(getter, 0);
    assert_ne!(setter, 0);
    let mut env = Box::new(Env::new());
    env.graph_owner_id = 7001;
    let env_ptr: NapiEnv = &mut *env;
    let seed = env.alloc(Value::Number(7.0));
    let object = env.alloc(Value::Object(HashMap::from([(
        PropertyKey::String("seed".into()),
        seed,
    )])));
    HOST.with(|host| host.borrow_mut().module_envs.push(env));
    let key = PropertyKey::String("computed".into());
    assert_ne!(thaw_quickjs::thaw_js_retain_handle(getter), 0);
    assert_ne!(thaw_quickjs::thaw_js_retain_handle(setter), 0);
    #[allow(clippy::arc_with_non_send_sync)]
    let callbacks = Arc::new(QuickJsAccessorRoots {
        getter,
        setter,
        getter_native: ptr::null_mut(),
        setter_native: ptr::null_mut(),
    });
    assert_eq!(
        unsafe {
            qjs_install_native_accessor(
                env_ptr,
                object,
                key.clone(),
                callbacks,
                true,
                true,
                NAPI_ENUMERABLE | NAPI_CONFIGURABLE,
            )
        },
        NAPI_OK
    );
    let mut first = ptr::null_mut();
    assert_eq!(
        unsafe { napi_get_named_property(env_ptr, object, c"computed".as_ptr(), &mut first) },
        NAPI_OK
    );
    let first_handle = unsafe { qjs_handle(first) }.expect("live getter result");
    assert_eq!(unsafe { qjs_query(first_handle, 2) }.unwrap(), "8");
    let replacement = unsafe { env_mut(env_ptr) }
        .unwrap()
        .alloc(Value::Number(12.0));
    assert_eq!(
        unsafe { napi_set_named_property(env_ptr, object, c"computed".as_ptr(), replacement) },
        NAPI_OK
    );
    let mut changed = ptr::null_mut();
    assert_eq!(
        unsafe { napi_get_named_property(env_ptr, object, c"computed".as_ptr(), &mut changed) },
        NAPI_OK
    );
    assert_eq!(
        unsafe { qjs_query(qjs_handle(changed).unwrap(), 2) }.unwrap(),
        "13"
    );
    let final_value = unsafe { env_mut(env_ptr) }
        .unwrap()
        .alloc(Value::Number(99.0));
    assert_eq!(
        unsafe {
            qjs_install_native_data_property(
                env_ptr,
                object,
                key,
                final_value,
                true,
                NAPI_DEFAULT_PROPERTY_ATTRIBUTES,
            )
        },
        NAPI_OK
    );
    let mut data = ptr::null_mut();
    assert_eq!(
        unsafe { napi_get_named_property(env_ptr, object, c"computed".as_ptr(), &mut data) },
        NAPI_OK
    );
    assert_eq!(data, final_value);
    let owned = HOST
        .with(|host| host.borrow_mut().module_envs.pop())
        .unwrap();
    drop(owned);
}

// Unrun end-to-end descriptor control: the trusted JS Proxy trap transfers
// accessor Function handles into a native descriptor, and the native getter
// and setter call back with the original object as `this`. Redefinition to a
// data property releases the old descriptor owner without touching the Env's
// unrelated graph handle cache.
#[cfg(feature = "quickjs")]
#[test]
fn graph_proxy_define_property_keeps_live_native_accessor() {
    thaw_quickjs::register_napi_bridge(
        thaw_napi_export_names,
        thaw_napi_call_typed_bridge,
        thaw_napi_handle_bridge,
        thaw_napi_poll_async_work,
        thaw_napi_async_work_pending,
        thaw_napi_graph_owner,
        release_napi_graph_reference,
    );
    let mut env = Box::new(Env::new());
    env.graph_owner_id = 7002;
    let env_ptr: NapiEnv = &mut *env;
    let zero = env.alloc(Value::Number(0.0));
    let object = env.alloc(Value::Object(HashMap::from([(
        PropertyKey::String("seed".into()),
        zero,
    )])));
    HOST.with(|host| host.borrow_mut().module_envs.push(env));
    let script = CString::new(format!(
        r#"
        globalThis.nativeDescriptorOwner =
          __thaw_json_graph_proxy_for_handle("{}");
        Object.defineProperty(nativeDescriptorOwner, "computed", {{
          get() {{ return this.seed + 3; }},
          set(value) {{ this.seed = value; }},
          enumerable: true, configurable: true
        }});
        globalThis.nativeAccessorIdentity = Reflect.getOwnPropertyDescriptor(
          nativeDescriptorOwner, 'computed').get ===
          Reflect.getOwnPropertyDescriptor(nativeDescriptorOwner, 'computed').get;
        nativeDescriptorOwner.computed = 8;
        globalThis.nativeDescriptorObserved = nativeDescriptorOwner.computed;
    "#,
        object as u64
    ))
    .unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(script.as_ptr()), 1);
    let identity = thaw_quickjs::thaw_js_get_global(c"nativeAccessorIdentity".as_ptr());
    assert_eq!(unsafe { qjs_query(identity, 2) }.unwrap(), "true");
    assert_eq!(thaw_quickjs::thaw_js_release_handle(identity), 1);
    let observed = thaw_quickjs::thaw_js_get_global(c"nativeDescriptorObserved".as_ptr());
    assert_eq!(unsafe { qjs_query(observed, 2) }.unwrap(), "11");
    assert_eq!(thaw_quickjs::thaw_js_release_handle(observed), 1);
    let mut native = ptr::null_mut();
    assert_eq!(
        unsafe { napi_get_named_property(env_ptr, object, c"computed".as_ptr(), &mut native) },
        NAPI_OK
    );
    assert_eq!(
        unsafe { qjs_query(qjs_handle(native).unwrap(), 2) }.unwrap(),
        "11"
    );
    assert_eq!(thaw_quickjs::thaw_js_load(c"globalThis.nativeOtherReceiver = { seed: 40 }; globalThis.nativeReceiverObserved = [Reflect.get(nativeDescriptorOwner, 'computed', nativeOtherReceiver), Reflect.set(nativeDescriptorOwner, 'computed', 27, nativeOtherReceiver), nativeOtherReceiver.seed, nativeDescriptorOwner.seed];".as_ptr()), 1);
    let receiver_observed = thaw_quickjs::thaw_js_get_global(c"nativeReceiverObserved".as_ptr());
    assert_eq!(
        unsafe { qjs_query(receiver_observed, 2) }.unwrap(),
        "43,true,27,8"
    );
    assert_eq!(thaw_quickjs::thaw_js_release_handle(receiver_observed), 1);
    assert_eq!(thaw_quickjs::thaw_js_load(c"globalThis.nativeDataReceiverObserved = (() => { let calls = 0; const accessor = {}; Object.defineProperty(accessor, 'seed', { set(value) { calls++; }, configurable: true }); const plain = {}; const accessorWrite = Reflect.set(nativeDescriptorOwner, 'seed', 61, accessor); const plainWrite = Reflect.set(nativeDescriptorOwner, 'seed', 62, plain); return [accessorWrite, calls, plainWrite, plain.seed, nativeDescriptorOwner.seed]; })();".as_ptr()), 1);
    let data_receiver = thaw_quickjs::thaw_js_get_global(c"nativeDataReceiverObserved".as_ptr());
    assert_eq!(
        unsafe { qjs_query(data_receiver, 2) }.unwrap(),
        "false,0,true,62,8"
    );
    assert_eq!(thaw_quickjs::thaw_js_release_handle(data_receiver), 1);
    assert_eq!(thaw_quickjs::thaw_js_load(c"globalThis.nativeThrownIdentity = (() => { const marker = { thrown: true }; Object.defineProperty(nativeDescriptorOwner, 'throwingGet', { get() { throw marker; }, configurable: true }); Object.defineProperty(nativeDescriptorOwner, 'throwingSet', { set(_) { throw marker; }, configurable: true }); let read = false, write = false; try { nativeDescriptorOwner.throwingGet; } catch (error) { read = error === marker; } try { nativeDescriptorOwner.throwingSet = 1; } catch (error) { write = error === marker; } return [read, write]; })();".as_ptr()), 1);
    let thrown_identity = thaw_quickjs::thaw_js_get_global(c"nativeThrownIdentity".as_ptr());
    assert_eq!(
        unsafe { qjs_query(thrown_identity, 2) }.unwrap(),
        "true,true"
    );
    assert_eq!(thaw_quickjs::thaw_js_release_handle(thrown_identity), 1);
    assert_eq!(thaw_quickjs::thaw_js_load(c"Object.defineProperty(nativeDescriptorOwner, 'attributeOnlySetter', { set(value) { this.seed = value; }, configurable: true }); Object.defineProperty(nativeDescriptorOwner, 'attributeOnlySetter', { configurable: false }); globalThis.nativeAttributeOnlySetter = [Object.getOwnPropertyDescriptor(nativeDescriptorOwner, 'attributeOnlySetter').get === undefined, Reflect.set(nativeDescriptorOwner, 'attributeOnlySetter', 19), nativeDescriptorOwner.attributeOnlySetter === undefined, nativeDescriptorOwner.seed === 19];".as_ptr()), 1);
    let attribute_only = thaw_quickjs::thaw_js_get_global(c"nativeAttributeOnlySetter".as_ptr());
    assert_eq!(
        unsafe { qjs_query(attribute_only, 2) }.unwrap(),
        "true,true,true,true"
    );
    assert_eq!(thaw_quickjs::thaw_js_release_handle(attribute_only), 1);
    assert_eq!(thaw_quickjs::thaw_js_load(c"Object.defineProperty(nativeDescriptorOwner, 'writeOnly', { set(value) { this.seed = value; }, configurable: false }); globalThis.nativeWriteOnlyObserved = [Reflect.set(nativeDescriptorOwner, 'writeOnly', 13), nativeDescriptorOwner.writeOnly === undefined, nativeDescriptorOwner.seed === 13]; Object.defineProperty(nativeDescriptorOwner, 'stable', { get() { return 9; }, configurable: false }); globalThis.nativeStableRedefined = Reflect.defineProperty(nativeDescriptorOwner, 'stable', Reflect.getOwnPropertyDescriptor(nativeDescriptorOwner, 'stable')); Object.defineProperty(nativeDescriptorOwner, 'computed', { value: 22, writable: true, configurable: true }); Object.defineProperty(nativeDescriptorOwner, 'fixed', { value: 1, writable: true, configurable: false }); Object.defineProperty(nativeDescriptorOwner, 'fixed', { value: 2 }); Object.defineProperty(nativeDescriptorOwner, 'fixed', { writable: false }); globalThis.nativeFixedObserved = nativeDescriptorOwner.fixed; Object.preventExtensions(nativeDescriptorOwner); globalThis.nativeIntegrityObserved = [Object.isExtensible(nativeDescriptorOwner), Reflect.set(nativeDescriptorOwner, 'added', 1), Reflect.ownKeys(nativeDescriptorOwner).includes('fixed'), Reflect.setPrototypeOf(nativeDescriptorOwner, Object.getPrototypeOf(nativeDescriptorOwner))];".as_ptr()), 1);
    let write_only = thaw_quickjs::thaw_js_get_global(c"nativeWriteOnlyObserved".as_ptr());
    assert_eq!(
        unsafe { qjs_query(write_only, 2) }.unwrap(),
        "true,true,true"
    );
    assert_eq!(thaw_quickjs::thaw_js_release_handle(write_only), 1);
    let redefined = thaw_quickjs::thaw_js_get_global(c"nativeStableRedefined".as_ptr());
    assert_eq!(unsafe { qjs_query(redefined, 2) }.unwrap(), "true");
    assert_eq!(thaw_quickjs::thaw_js_release_handle(redefined), 1);
    let fixed = thaw_quickjs::thaw_js_get_global(c"nativeFixedObserved".as_ptr());
    assert_eq!(unsafe { qjs_query(fixed, 2) }.unwrap(), "2");
    assert_eq!(thaw_quickjs::thaw_js_release_handle(fixed), 1);
    let integrity = thaw_quickjs::thaw_js_get_global(c"nativeIntegrityObserved".as_ptr());
    assert_eq!(
        unsafe { qjs_query(integrity, 2) }.unwrap(),
        "false,false,true,true"
    );
    assert_eq!(thaw_quickjs::thaw_js_release_handle(integrity), 1);
    let mut data = ptr::null_mut();
    assert_eq!(
        unsafe { napi_get_named_property(env_ptr, object, c"computed".as_ptr(), &mut data) },
        NAPI_OK
    );
    assert!(matches!(
        unsafe { value_ref(data) },
        Ok(Value::Number(22.0))
    ));
    assert_eq!(thaw_quickjs::thaw_js_load(c"globalThis.nativeNonextensibleReplace = [Reflect.defineProperty(nativeDescriptorOwner, 'computed', { get() { return this.seed + 1; }, configurable: true }), nativeDescriptorOwner.computed === nativeDescriptorOwner.seed + 1, Reflect.defineProperty(nativeDescriptorOwner, 'absentAfterPrevent', { get() { return 1; }, configurable: true })];".as_ptr()), 1);
    let replaced = thaw_quickjs::thaw_js_get_global(c"nativeNonextensibleReplace".as_ptr());
    assert_eq!(
        unsafe { qjs_query(replaced, 2) }.unwrap(),
        "true,true,false"
    );
    assert_eq!(thaw_quickjs::thaw_js_release_handle(replaced), 1);
    let computed = unsafe { env_mut(env_ptr) }
        .unwrap()
        .alloc(Value::String("computed".into()));
    let mut deleted = false;
    assert_eq!(
        unsafe { napi_delete_property(env_ptr, object, computed, &mut deleted) },
        NAPI_OK
    );
    assert!(deleted);
    assert_eq!(thaw_quickjs::thaw_js_load(c"globalThis.nativeDeletedObserved = [Reflect.ownKeys(nativeDescriptorOwner).includes('computed'), Reflect.getOwnPropertyDescriptor(nativeDescriptorOwner, 'computed') === undefined, nativeDescriptorOwner.computed === undefined]; delete globalThis.nativeDescriptorOwner;".as_ptr()), 1);
    let deleted = thaw_quickjs::thaw_js_get_global(c"nativeDeletedObserved".as_ptr());
    assert_eq!(unsafe { qjs_query(deleted, 2) }.unwrap(), "false,true,true");
    assert_eq!(thaw_quickjs::thaw_js_release_handle(deleted), 1);
    let owned = HOST
        .with(|host| host.borrow_mut().module_envs.pop())
        .unwrap();
    drop(owned);
}

// Unrun: a native accessor reflected into JS is the original callable.
// Its identity is stable while installed, and an extracted getter retains
// both the callback and its call-time receiver after native deletion.
#[cfg(feature = "quickjs")]
#[test]
fn graph_proxy_reflects_native_accessor_with_snapshot_lifetime() {
    unsafe extern "C" fn own_receiver(_: NapiEnv, info: NapiCallbackInfo) -> NapiValue {
        info.as_ref().map_or(ptr::null_mut(), |info| info.this_arg)
    }
    thaw_quickjs::register_napi_bridge(
        thaw_napi_export_names,
        thaw_napi_call_typed_bridge,
        thaw_napi_handle_bridge,
        thaw_napi_poll_async_work,
        thaw_napi_async_work_pending,
        thaw_napi_graph_owner,
        release_napi_graph_reference,
    );
    let mut owner = Box::new(Env::new());
    owner.graph_owner_id = 7041;
    let object = owner.alloc(Value::Object(HashMap::new()));
    let other = owner.alloc(Value::Object(HashMap::new()));
    let key = PropertyKey::String("live".into());
    owner.accessors.insert(
        (object as usize, key.clone()),
        Accessor {
            getter: Some(own_receiver),
            setter: None,
            getter_data: ptr::null_mut(),
            setter_data: ptr::null_mut(),
            getter_reflection: None,
            setter_reflection: None,
            js_owner: None,
        },
    );
    owner.property_attributes.insert(
        (object as usize, key.clone()),
        NAPI_CONFIGURABLE | NAPI_ENUMERABLE,
    );
    HOST.with(|host| host.borrow_mut().module_envs.push(owner));
    let script = CString::new(format!(
        r#"
        globalThis.nativeAccessorObject = __thaw_json_graph_proxy_for_handle("{}");
        globalThis.nativeAccessorOther = __thaw_json_graph_proxy_for_handle("{}");
        const first = Reflect.getOwnPropertyDescriptor(nativeAccessorObject, 'live').get;
        const again = Reflect.getOwnPropertyDescriptor(nativeAccessorObject, 'live').get;
        globalThis.nativeAccessorSame = first === again;
        globalThis.nativeAccessorExtracted = first;
        globalThis.nativeAccessorBefore = first.call(nativeAccessorOther) === nativeAccessorOther;
    "#,
        object as u64, other as u64
    ))
    .unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(script.as_ptr()), 1);
    let env = HOST
        .with(|host| host.borrow().module_envs.last().unwrap().as_ref() as *const Env as NapiEnv);
    let key_value = unsafe { env_mut(env) }
        .unwrap()
        .alloc(Value::String("live".into()));
    let mut deleted = false;
    assert_eq!(
        unsafe { napi_delete_property(env, object, key_value, &mut deleted) },
        NAPI_OK
    );
    assert!(deleted);
    assert_eq!(thaw_quickjs::thaw_js_load(c"globalThis.nativeAccessorAfter = nativeAccessorExtracted.call(nativeAccessorOther) === nativeAccessorOther; globalThis.nativeAccessorGone = Reflect.getOwnPropertyDescriptor(nativeAccessorObject, 'live') === undefined;".as_ptr()), 1);
    for name in [
        c"nativeAccessorSame".as_ptr(),
        c"nativeAccessorBefore".as_ptr(),
        c"nativeAccessorAfter".as_ptr(),
        c"nativeAccessorGone".as_ptr(),
    ] {
        let handle = thaw_quickjs::thaw_js_get_global(name);
        assert_eq!(unsafe { qjs_query(handle, 2) }.unwrap(), "true");
        assert_eq!(thaw_quickjs::thaw_js_release_handle(handle), 1);
    }
    assert_eq!(thaw_quickjs::thaw_js_load(c"delete globalThis.nativeAccessorObject; delete globalThis.nativeAccessorOther; delete globalThis.nativeAccessorExtracted;".as_ptr()), 1);
    let owned = HOST
        .with(|host| host.borrow_mut().module_envs.pop())
        .unwrap();
    drop(owned);
}

// Unrun: an Array with a native accessor must remain an Array through the
// live graph lane. Its holes, length, descriptor, and own-key order cannot
// be flattened into the ordinary object proxy or snapshot codec.
#[cfg(feature = "quickjs")]
#[test]
fn native_array_accessor_graph_keeps_array_identity_and_length() {
    unsafe extern "C" fn seven(_: NapiEnv, info: NapiCallbackInfo) -> NapiValue {
        info.as_ref()
            .map_or(ptr::null_mut(), |info| info.data as NapiValue)
    }
    thaw_quickjs::register_napi_bridge(
        thaw_napi_export_names,
        thaw_napi_call_typed_bridge,
        thaw_napi_handle_bridge,
        thaw_napi_poll_async_work,
        thaw_napi_async_work_pending,
        thaw_napi_graph_owner,
        release_napi_graph_reference,
    );
    let mut owner = Box::new(Env::new());
    owner.graph_owner_id = 7042;
    let one = owner.alloc(Value::Number(1.0));
    let three_value = owner.alloc(Value::Number(3.0));
    let seven_value = owner.alloc(Value::Number(7.0));
    let array = owner.alloc(Value::Array(vec![Some(one), None, Some(three_value)]));
    let extra = owner.alloc(Value::Number(11.0));
    assert_eq!(
        unsafe { napi_set_named_property(owner.as_mut(), array, c"extra".as_ptr(), extra) },
        NAPI_OK
    );
    owner.accessors.insert(
        (array as usize, PropertyKey::String("1".into())),
        Accessor {
            getter: Some(seven),
            setter: None,
            getter_data: seven_value.cast(),
            setter_data: ptr::null_mut(),
            getter_reflection: None,
            setter_reflection: None,
            js_owner: None,
        },
    );
    owner.property_attributes.insert(
        (array as usize, PropertyKey::String("1".into())),
        NAPI_CONFIGURABLE | NAPI_ENUMERABLE,
    );
    HOST.with(|host| host.borrow_mut().module_envs.push(owner));
    let env = HOST
        .with(|host| host.borrow().module_envs.last().unwrap().as_ref() as *const Env as NapiEnv);
    let wire = unsafe { napi_result_graph_for_env(env, array, true) }.unwrap();
    let encoded: serde_json::Value = serde_json::from_str(&wire).unwrap();
    assert_eq!(encoded["nodes"][0]["nh"], (array as u64).to_string());
    for token in encoded["napiLeases"].as_array().unwrap() {
        let token = token.as_str().unwrap().parse::<u64>().unwrap();
        assert_eq!(release_napi_graph_reference(token), 1);
    }
    let script = CString::new(format!(
        r#"
        globalThis.nativeLiveArray = __thaw_json_graph_proxy_for_handle("{}");
        globalThis.nativeLiveArrayObserved = [
          Array.isArray(nativeLiveArray), nativeLiveArray.length,
          nativeLiveArray[0], nativeLiveArray[1], nativeLiveArray[2],
          Reflect.ownKeys(nativeLiveArray).join(','),
          typeof Reflect.getOwnPropertyDescriptor(nativeLiveArray, '1').get
        ];
    "#,
        array as u64
    ))
    .unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(script.as_ptr()), 1);
    let observed = thaw_quickjs::thaw_js_get_global(c"nativeLiveArrayObserved".as_ptr());
    assert_eq!(
        unsafe { qjs_query(observed, 2) }.unwrap(),
        "true,3,1,7,3,0,1,2,length,extra,function"
    );
    assert_eq!(thaw_quickjs::thaw_js_release_handle(observed), 1);
    assert_eq!(thaw_quickjs::thaw_js_load(
        c"Object.defineProperty(nativeLiveArray, 'length', { writable: false }); globalThis.nativeLiveArrayReadonly = Object.getOwnPropertyDescriptor(nativeLiveArray, 'length').writable;".as_ptr()), 1);
    let readonly = thaw_quickjs::thaw_js_get_global(c"nativeLiveArrayReadonly".as_ptr());
    assert_eq!(unsafe { qjs_query(readonly, 2) }.unwrap(), "false");
    assert_eq!(thaw_quickjs::thaw_js_release_handle(readonly), 1);
    let three = unsafe { env.as_mut() }.unwrap().alloc(Value::Number(3.0));
    assert_eq!(
        unsafe { napi_set_named_property(env, array, c"length".as_ptr(), three) },
        NAPI_GENERIC_FAILURE
    );
    assert_eq!(thaw_quickjs::thaw_js_load(c"delete globalThis.nativeLiveArray; delete globalThis.nativeLiveArrayObserved; delete globalThis.nativeLiveArrayReadonly;".as_ptr()), 1);
    let owned = HOST
        .with(|host| host.borrow_mut().module_envs.pop())
        .unwrap();
    drop(owned);
}

// Unrun: the native length authority must match an Array Proxy target after
// descriptor changes. A sealed Array may still resize; a frozen or explicitly
// readonly length may not, including through the direct N-API setter.
#[cfg(feature = "quickjs")]
#[test]
fn native_array_length_attributes_survive_seal_freeze_and_live_definition() {
    let mut env = Env::new();
    let array = env.alloc(Value::Array(vec![None, None]));
    let length = PropertyKey::String("length".into());
    assert_eq!(
        unsafe { property_attributes_for(&mut env, array as usize, &length) },
        NAPI_WRITABLE
    );
    assert_eq!(unsafe { napi_object_seal(&mut env, array) }, NAPI_OK);
    assert_eq!(
        unsafe { property_attributes_for(&mut env, array as usize, &length) },
        NAPI_WRITABLE
    );
    let three = env.alloc(Value::Number(3.0));
    assert_eq!(
        unsafe { napi_set_named_property(&mut env, array, c"length".as_ptr(), three) },
        NAPI_OK
    );
    assert_eq!(unsafe { napi_object_freeze(&mut env, array) }, NAPI_OK);
    assert_eq!(
        unsafe { property_attributes_for(&mut env, array as usize, &length) },
        0
    );
    let four = env.alloc(Value::Number(4.0));
    assert_eq!(
        unsafe { napi_set_named_property(&mut env, array, c"length".as_ptr(), four) },
        NAPI_GENERIC_FAILURE
    );

    let other = env.alloc(Value::Array(vec![None]));
    let unused = env.alloc(Value::Undefined);
    assert_eq!(
        unsafe {
            qjs_install_native_data_property(&mut env, other, length.clone(), unused, false, 0)
        },
        NAPI_OK
    );
    assert_eq!(
        unsafe { property_attributes_for(&mut env, other as usize, &length) },
        0
    );
    let two = env.alloc(Value::Number(2.0));
    assert_eq!(
        unsafe { napi_set_named_property(&mut env, other, c"length".as_ptr(), two) },
        NAPI_GENERIC_FAILURE
    );
    assert_eq!(
        unsafe { napi_set_named_property(&mut env, other, c"1".as_ptr(), two) },
        NAPI_GENERIC_FAILURE
    );

    let defined = env.alloc(Value::Array(vec![None, None]));
    let length_descriptor = NapiPropertyDescriptor {
        utf8name: c"length".as_ptr(),
        name: ptr::null_mut(),
        method: None,
        getter: None,
        setter: None,
        value: ptr::null_mut(),
        attributes: 0,
        data: ptr::null_mut(),
    };
    assert_eq!(
        unsafe { napi_define_properties(&mut env, defined, 1, &length_descriptor) },
        NAPI_OK
    );
    assert_eq!(
        unsafe { property_attributes_for(&mut env, defined as usize, &length) },
        0
    );
    assert_eq!(
        unsafe { napi_set_named_property(&mut env, defined, c"length".as_ptr(), three) },
        NAPI_GENERIC_FAILURE
    );
    let enumerable_length = NapiPropertyDescriptor {
        attributes: NAPI_ENUMERABLE,
        ..length_descriptor
    };
    assert_eq!(
        unsafe { napi_define_properties(&mut env, defined, 1, &enumerable_length) },
        NAPI_GENERIC_FAILURE
    );
}

// Unrun: indexed accessor installation changes Array length, and shrinking
// deletes configurable keys in descending order but stops at a
// nonconfigurable element with the JS ArraySetLength partial result.
#[test]
fn native_array_length_shrink_releases_index_accessors_and_stops_at_fixed_key() {
    unsafe extern "C" fn seven(env: NapiEnv, _: NapiCallbackInfo) -> NapiValue {
        env_mut(env).unwrap().alloc(Value::Number(7.0))
    }
    let mut env = Box::new(Env::new());
    let array = env.alloc(Value::Array(vec![None]));
    let descriptor = NapiPropertyDescriptor {
        utf8name: c"3".as_ptr(),
        name: ptr::null_mut(),
        method: None,
        getter: Some(seven),
        setter: None,
        value: ptr::null_mut(),
        attributes: NAPI_CONFIGURABLE | NAPI_ENUMERABLE,
        data: ptr::null_mut(),
    };
    assert_eq!(
        unsafe { napi_define_properties(&mut *env, array, 1, &descriptor) },
        NAPI_OK
    );
    assert!(matches!(unsafe { value_ref(array) }, Ok(Value::Array(values)) if values.len() == 4));
    let two = env.alloc(Value::Number(2.0));
    assert_eq!(
        unsafe { napi_set_named_property(&mut *env, array, c"length".as_ptr(), two) },
        NAPI_OK
    );
    assert!(matches!(unsafe { value_ref(array) }, Ok(Value::Array(values)) if values.len() == 2));
    assert!(!env
        .accessors
        .contains_key(&(array as usize, PropertyKey::String("3".into()))));

    let fixed = env.alloc(Value::Number(8.0));
    let tail = env.alloc(Value::Number(9.0));
    let blocked = env.alloc(Value::Array(vec![None, None, Some(fixed), Some(tail)]));
    env.property_attributes.insert(
        (blocked as usize, PropertyKey::String("2".into())),
        NAPI_WRITABLE | NAPI_ENUMERABLE,
    );
    let zero = env.alloc(Value::Number(0.0));
    assert_eq!(
        unsafe { napi_set_named_property(&mut *env, blocked, c"length".as_ptr(), zero) },
        NAPI_GENERIC_FAILURE
    );
    assert!(
        matches!(unsafe { value_ref(blocked) }, Ok(Value::Array(values))
        if values.len() == 3 && values[2] == Some(fixed))
    );
    let shrink_and_freeze = NapiPropertyDescriptor {
        utf8name: c"length".as_ptr(),
        name: ptr::null_mut(),
        method: None,
        getter: None,
        setter: None,
        value: zero,
        attributes: 0,
        data: ptr::null_mut(),
    };
    assert_eq!(
        unsafe { napi_define_properties(&mut *env, blocked, 1, &shrink_and_freeze) },
        NAPI_GENERIC_FAILURE
    );
    assert_eq!(
        unsafe {
            property_attributes_for(
                &mut *env,
                blocked as usize,
                &PropertyKey::String("length".into()),
            )
        },
        0
    );
}

// Unrun: a native descriptor installed as nonconfigurable cannot be replaced
// by a different callback or converted to data through the accessor branch.
#[test]
fn native_nonconfigurable_accessor_redefinition_keeps_original_callback() {
    unsafe extern "C" fn first(env: NapiEnv, _: NapiCallbackInfo) -> NapiValue {
        env_mut(env).unwrap().alloc(Value::Number(1.0))
    }
    unsafe extern "C" fn second(env: NapiEnv, _: NapiCallbackInfo) -> NapiValue {
        env_mut(env).unwrap().alloc(Value::Number(2.0))
    }
    let mut env = Env::new();
    let object = env.alloc(Value::Object(HashMap::new()));
    let original = NapiPropertyDescriptor {
        utf8name: c"answer".as_ptr(),
        name: ptr::null_mut(),
        method: None,
        getter: Some(first),
        setter: None,
        value: ptr::null_mut(),
        attributes: NAPI_ENUMERABLE,
        data: ptr::null_mut(),
    };
    assert_eq!(
        unsafe { napi_define_properties(&mut env, object, 1, &original) },
        NAPI_OK
    );
    assert_eq!(
        unsafe { napi_define_properties(&mut env, object, 1, &original) },
        NAPI_OK
    );
    let changed = NapiPropertyDescriptor {
        getter: Some(second),
        ..original
    };
    assert_eq!(
        unsafe { napi_define_properties(&mut env, object, 1, &changed) },
        NAPI_GENERIC_FAILURE
    );
    let mut out = ptr::null_mut();
    assert_eq!(
        unsafe { napi_get_named_property(&mut env, object, c"answer".as_ptr(), &mut out) },
        NAPI_OK
    );
    assert!(matches!(unsafe { value_ref(out) }, Ok(Value::Number(value)) if *value == 1.0));
}

// Unrun: defining an own data descriptor replaces a configurable accessor
// without invoking its setter; a later readonly/nonconfigurable definition
// applies SameValue compatibility rather than ordinary assignment rules.
#[test]
fn native_data_definition_skips_setter_and_checks_readonly_compatibility() {
    unsafe extern "C" fn setter(_: NapiEnv, info: NapiCallbackInfo) -> NapiValue {
        let calls = info.as_ref().unwrap().data as *const AtomicUsize;
        (*calls).fetch_add(1, Ordering::SeqCst);
        ptr::null_mut()
    }
    let mut env = Env::new();
    let object = env.alloc(Value::Object(HashMap::new()));
    let calls = AtomicUsize::new(0);
    let accessor = NapiPropertyDescriptor {
        utf8name: c"answer".as_ptr(),
        name: ptr::null_mut(),
        method: None,
        getter: None,
        setter: Some(setter),
        value: ptr::null_mut(),
        attributes: NAPI_CONFIGURABLE,
        data: (&calls as *const AtomicUsize).cast_mut().cast(),
    };
    assert_eq!(
        unsafe { napi_define_properties(&mut env, object, 1, &accessor) },
        NAPI_OK
    );
    let one = env.alloc(Value::Number(1.0));
    let data = NapiPropertyDescriptor {
        utf8name: c"answer".as_ptr(),
        name: ptr::null_mut(),
        method: None,
        getter: None,
        setter: None,
        value: one,
        attributes: 0,
        data: ptr::null_mut(),
    };
    assert_eq!(
        unsafe { napi_define_properties(&mut env, object, 1, &data) },
        NAPI_OK
    );
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert_eq!(
        unsafe { napi_define_properties(&mut env, object, 1, &data) },
        NAPI_OK
    );
    let two = env.alloc(Value::Number(2.0));
    let changed = NapiPropertyDescriptor { value: two, ..data };
    assert_eq!(
        unsafe { napi_define_properties(&mut env, object, 1, &changed) },
        NAPI_GENERIC_FAILURE
    );
    let mut out = ptr::null_mut();
    assert_eq!(
        unsafe { napi_get_named_property(&mut env, object, c"answer".as_ptr(), &mut out) },
        NAPI_OK
    );
    assert!(matches!(unsafe { value_ref(out) }, Ok(Value::Number(value)) if *value == 1.0));

    // Omitting the data value still converts a configurable accessor into an
    // own Undefined slot; it must not merely remove the accessor metadata.
    let empty_accessor = NapiPropertyDescriptor {
        utf8name: c"empty".as_ptr(),
        ..accessor
    };
    assert_eq!(
        unsafe { napi_define_properties(&mut env, object, 1, &empty_accessor) },
        NAPI_OK
    );
    let empty_data = NapiPropertyDescriptor {
        utf8name: c"empty".as_ptr(),
        value: ptr::null_mut(),
        ..changed
    };
    assert_eq!(
        unsafe { napi_define_properties(&mut env, object, 1, &empty_data) },
        NAPI_OK
    );
    let empty_key = env.alloc(Value::String("empty".into()));
    let mut present = false;
    assert_eq!(
        unsafe { napi_has_own_property(&mut env, object, empty_key, &mut present) },
        NAPI_OK
    );
    assert!(present);
    assert_eq!(
        unsafe { napi_get_named_property(&mut env, object, c"empty".as_ptr(), &mut out) },
        NAPI_OK
    );
    assert!(matches!(unsafe { value_ref(out) }, Ok(Value::Undefined)));
}

// Unrun regression: target lookup must return the graph's transferred lease
// even though argument decoding was never entered.
#[cfg(feature = "quickjs")]
#[test]
fn live_quickjs_invalid_call_target_releases_argument_lease() {
    assert_eq!(
        thaw_quickjs::thaw_js_load(c"globalThis.napiLeaseValue = {};".as_ptr()),
        1
    );
    let handle = thaw_quickjs::thaw_js_get_global(c"napiLeaseValue".as_ptr());
    assert_ne!(handle, 0);
    assert_eq!(thaw_quickjs::thaw_js_retain_handle(handle), 1);
    let graph = std::ffi::CString::new(format!(
        r#"{{"root":{{"r":0}},"nodes":[{{"a":[{{"r":1}}]}},{{"hdl":{handle}}}],"leases":[{handle}]}}"#
    )).unwrap();
    let result = thaw_quickjs::thaw_js_call_handle_with_this_graph_result(0, graph.as_ptr());
    assert_eq!(result.value, 0);
    assert!(!result.error.is_null());
    unsafe {
        thaw_arena::destroy_string(result.error);
    }
    // The only remaining reference is the original get_global retain.
    assert_eq!(thaw_quickjs::thaw_js_release_handle(handle), 1);
    assert_eq!(thaw_quickjs::thaw_js_release_handle(handle), 0);
}

// Unrun control: a failed `new` target check has not decoded its argument
// graph, so the transferred handle lease must be returned at that boundary.
#[cfg(feature = "quickjs")]
#[test]
fn live_quickjs_invalid_constructor_releases_argument_lease() {
    assert_eq!(
        thaw_quickjs::thaw_js_load(c"globalThis.napiConstructorLease = {};".as_ptr()),
        1
    );
    let handle = thaw_quickjs::thaw_js_get_global(c"napiConstructorLease".as_ptr());
    assert_ne!(handle, 0);
    assert_eq!(thaw_quickjs::thaw_js_retain_handle(handle), 1);
    let graph = std::ffi::CString::new(format!(
        r#"{{"root":{{"r":0}},"nodes":[{{"a":[{{"r":1}}]}},{{"hdl":{handle}}}],"leases":[{handle}]}}"#
    )).unwrap();
    let result = thaw_quickjs::thaw_js_construct_handle_graph_args_result(0, graph.as_ptr());
    assert_eq!(result.value, 0);
    assert!(!result.error.is_null());
    unsafe {
        thaw_arena::destroy_string(result.error);
    }
    assert_eq!(thaw_quickjs::thaw_js_release_handle(handle), 1);
    assert_eq!(thaw_quickjs::thaw_js_release_handle(handle), 0);
}

// Unrun: the paired native Function node must select its native half in the
// owning Env before considering the generic hdl node route. A foreign Env
// must select only its independently retained JS carrier.
#[cfg(feature = "quickjs")]
#[test]
fn paired_native_function_graph_uses_owner_identity_and_foreign_carrier() {
    unsafe extern "C" fn native(env: NapiEnv, _: NapiCallbackInfo) -> NapiValue {
        env_mut(env).unwrap().alloc(Value::Undefined)
    }
    let mut owner = Env::new();
    let function = owner.alloc(Value::Function(Function {
        callback: native,
        data: ptr::null_mut(),
        properties: HashMap::new(),
        _thaw_bridge: None,
        _accessor_owner: None,
    }));
    let handle = 71_u64;
    let owner_carrier = owner.alloc(Value::QuickJsHandle {
        handle,
        object_like: true,
    });
    owner.quickjs_live_values.insert(handle, owner_carrier);
    let graph = serde_json::json!({
        "root": {"r": 0},
        "nodes": [{"nfn": (function as u64).to_string(), "hdl": handle}],
    });
    assert_eq!(napi_graph_value(&mut owner, &graph).unwrap(), function);

    let mut recipient = Env::new();
    let foreign_carrier = recipient.alloc(Value::QuickJsHandle {
        handle,
        object_like: true,
    });
    recipient
        .quickjs_live_values
        .insert(handle, foreign_carrier);
    assert_eq!(
        napi_graph_value(&mut recipient, &graph).unwrap(),
        foreign_carrier
    );
}

// Unrun: a foreign native Symbol's value token and property-key token share
// the source Symbol ID, but neither may dereference that pointer as a value
// belonging to the recipient Env.
#[cfg(feature = "quickjs")]
#[test]
fn paired_native_symbol_graph_keeps_value_and_key_identity_across_envs() {
    let mut owner = Env::new();
    let source = owner.alloc(Value::Symbol {
        id: 93,
        description: "shared".into(),
    });
    let mut recipient = Env::new();
    let handle = 73_u64;
    let carrier = recipient.alloc(Value::QuickJsHandle {
        handle,
        object_like: false,
    });
    recipient.quickjs_live_values.insert(handle, carrier);
    recipient.quickjs_symbol_ids.insert(handle, 93);
    let token = serde_json::json!({"nsy": (source as u64).to_string(), "hdl": handle});
    assert_eq!(
        napi_graph_token(&mut recipient, &token, &[]).unwrap(),
        carrier
    );
    let key = napi_graph_property_key(&mut recipient, &token, &[]).unwrap();
    assert_eq!(key, PropertyKey::Symbol(93));
}

#[cfg(feature = "quickjs")]
#[test]
fn foreign_symbol_identity_preflight_rejects_unknown_pointer_before_dereference() {
    let mut owner = Box::new(Env::new());
    owner.graph_owner_id = 1;
    let symbol = owner.alloc(Value::Symbol {
        id: 94,
        description: "live".into(),
    });
    HOST.with(|host| host.borrow_mut().module_envs.push(owner));
    assert_eq!(napi_graph_symbol_identity(symbol).unwrap(), 94);
    assert!(napi_graph_symbol_identity(1_usize as NapiValue).is_err());
    HOST.with(|host| {
        drop(host.borrow_mut().module_envs.pop());
    });
}

// Unrun control: an N-API property key carried by a live QuickJS Symbol
// retains the exact symbol, including across a setter/getter roundtrip.
#[cfg(feature = "quickjs")]
#[test]
fn live_quickjs_symbol_property_key_keeps_identity() {
    assert_eq!(thaw_quickjs::thaw_js_load(
        c"globalThis.napiSymbolKey = Symbol('same'); globalThis.napiSymbolObject = {[napiSymbolKey]: 6};".as_ptr()), 1);
    let key_handle = thaw_quickjs::thaw_js_get_global(c"napiSymbolKey".as_ptr());
    let object_handle = thaw_quickjs::thaw_js_get_global(c"napiSymbolObject".as_ptr());
    assert_ne!(key_handle, 0);
    assert_ne!(object_handle, 0);
    assert_eq!(thaw_quickjs::thaw_js_retain_handle(key_handle), 1);
    assert_eq!(thaw_quickjs::thaw_js_retain_handle(object_handle), 1);
    let graph = format!(
        r#"{{"root":{{"r":0}},"nodes":[{{"a":[{{"r":1}},{{"r":2}}]}},{{"hdl":{object_handle}}},{{"hdl":{key_handle}}}],"leases":[{object_handle},{key_handle}]}}"#
    );
    let mut env = Env::new();
    let root = unsafe { parse_napi_graph_value(&mut env as NapiEnv, &graph) }.unwrap();
    let Value::Array(items) = unsafe { value_ref(root) }.unwrap() else {
        panic!("array root")
    };
    let object = items[0].unwrap();
    let key = items[1].unwrap();
    let mut got = ptr::null_mut();
    assert_eq!(
        unsafe { napi_get_property(&mut env, object, key, &mut got) },
        NAPI_OK
    );
    let mut original = 0.0;
    assert_eq!(
        unsafe { napi_get_value_double(&mut env, got, &mut original) },
        NAPI_OK
    );
    assert_eq!(original, 6.0);
    let replacement = env.alloc(Value::Number(9.0));
    assert_eq!(
        unsafe { napi_set_property(&mut env, object, key, replacement) },
        NAPI_OK
    );
    let mut updated = ptr::null_mut();
    assert_eq!(
        unsafe { napi_get_property(&mut env, object, key, &mut updated) },
        NAPI_OK
    );
    let mut value = 0.0;
    assert_eq!(
        unsafe { napi_get_value_double(&mut env, updated, &mut value) },
        NAPI_OK
    );
    assert_eq!(value, 9.0);
    assert_eq!(thaw_quickjs::thaw_js_release_handle(key_handle), 1);
    assert_eq!(thaw_quickjs::thaw_js_release_handle(object_handle), 1);
    drop(env);
}

// Unrun: a QuickJS-origin string key keeps exact code units when used on a
// native object. Rust's lossy String conversion would merge these two keys.
#[cfg(feature = "quickjs")]
#[test]
fn live_quickjs_surrogate_string_keys_stay_distinct_on_native_object() {
    assert_eq!(thaw_quickjs::thaw_js_load(
        c"globalThis.napiHighKey = String.fromCharCode(0xd800); globalThis.napiLowKey = String.fromCharCode(0xdc00); globalThis.napiValidKey = 'a';".as_ptr()), 1);
    let high_handle = thaw_quickjs::thaw_js_get_global(c"napiHighKey".as_ptr());
    let low_handle = thaw_quickjs::thaw_js_get_global(c"napiLowKey".as_ptr());
    let valid_handle = thaw_quickjs::thaw_js_get_global(c"napiValidKey".as_ptr());
    assert_ne!(high_handle, 0);
    assert_ne!(low_handle, 0);
    assert_ne!(valid_handle, 0);
    let mut env = Env::new();
    let high = env.alloc(Value::QuickJsHandle {
        handle: high_handle,
        object_like: false,
    });
    let low = env.alloc(Value::QuickJsHandle {
        handle: low_handle,
        object_like: false,
    });
    let valid = env.alloc(Value::QuickJsHandle {
        handle: valid_handle,
        object_like: false,
    });
    let mut object = ptr::null_mut();
    assert_eq!(
        unsafe { napi_create_object(&mut env, &mut object) },
        NAPI_OK
    );
    let one = env.alloc(Value::Number(1.0));
    let two = env.alloc(Value::Number(2.0));
    assert_eq!(
        unsafe { napi_set_property(&mut env, object, high, one) },
        NAPI_OK
    );
    assert_eq!(
        unsafe { napi_set_property(&mut env, object, low, two) },
        NAPI_OK
    );
    let mut found = ptr::null_mut();
    assert_eq!(
        unsafe { napi_get_property(&mut env, object, high, &mut found) },
        NAPI_OK
    );
    assert_eq!(found, one);
    assert_eq!(
        unsafe { napi_get_property(&mut env, object, low, &mut found) },
        NAPI_OK
    );
    assert_eq!(found, two);
    let ordinary = env.alloc(Value::String("a".into()));
    assert_eq!(
        unsafe { napi_set_property(&mut env, object, valid, one) },
        NAPI_OK
    );
    assert_eq!(
        unsafe { napi_get_property(&mut env, object, ordinary, &mut found) },
        NAPI_OK
    );
    assert_eq!(found, one);
    assert_eq!(thaw_quickjs::thaw_js_release_handle(high_handle), 0);
    assert_eq!(thaw_quickjs::thaw_js_release_handle(low_handle), 0);
    assert_eq!(thaw_quickjs::thaw_js_release_handle(valid_handle), 0);
}

#[cfg(feature = "quickjs")]
#[test]
fn live_quickjs_object_keeps_native_symbol_key_and_integrity() {
    assert_eq!(
        thaw_quickjs::thaw_js_load(c"globalThis.napiSymbolObject = {};".as_ptr()),
        1
    );
    let handle = thaw_quickjs::thaw_js_get_global(c"napiSymbolObject".as_ptr());
    assert_ne!(handle, 0);
    assert_eq!(thaw_quickjs::thaw_js_retain_handle(handle), 1);
    let graph = format!(r#"{{"root":{{"r":0}},"nodes":[{{"hdl":{handle}}}],"leases":[{handle}]}}"#);
    let mut env = Env::new();
    let object = unsafe { parse_napi_graph_value(&mut env, &graph) }.unwrap();
    let description = env.alloc(Value::String("shared".into()));
    let mut key = ptr::null_mut();
    assert_eq!(
        unsafe { napi_create_symbol(&mut env, description, &mut key) },
        NAPI_OK
    );
    let mut distinct = ptr::null_mut();
    assert_eq!(
        unsafe { napi_create_symbol(&mut env, description, &mut distinct) },
        NAPI_OK
    );
    let mut equal = true;
    assert_eq!(
        unsafe { napi_strict_equals(&mut env, key, distinct, &mut equal) },
        NAPI_OK
    );
    assert!(!equal);
    let value = env.alloc(Value::Number(7.0));
    assert_eq!(
        unsafe { napi_set_property(&mut env, object, key, value) },
        NAPI_OK
    );
    let mut found = ptr::null_mut();
    assert_eq!(
        unsafe { napi_get_property(&mut env, object, key, &mut found) },
        NAPI_OK
    );
    let mut number = 0.0;
    assert_eq!(
        unsafe { napi_get_value_double(&mut env, found, &mut number) },
        NAPI_OK
    );
    assert_eq!(number, 7.0);
    let mut present = false;
    assert_eq!(
        unsafe { napi_has_own_property(&mut env, object, key, &mut present) },
        NAPI_OK
    );
    assert!(present);
    assert_eq!(
        unsafe { napi_has_own_property(&mut env, object, distinct, &mut present) },
        NAPI_OK
    );
    assert!(!present);
    assert_eq!(unsafe { napi_object_freeze(&mut env, object) }, NAPI_OK);
    assert_eq!(
        unsafe { napi_delete_property(&mut env, object, key, &mut present) },
        NAPI_OK
    );
    assert!(!present);
    assert_eq!(thaw_quickjs::thaw_js_release_handle(handle), 1);
    drop(env);
}

// Unrun regression: registry-handle identity alone is not JavaScript ===.
// One retained NaN handle compares false even with itself; ±0 compare true.
#[cfg(feature = "quickjs")]
#[test]
fn live_quickjs_strict_equality_uses_javascript_values() {
    assert_eq!(thaw_quickjs::thaw_js_load(
        c"globalThis.napiNan = NaN; globalThis.napiPositiveZero = 0; globalThis.napiNegativeZero = -0;".as_ptr()), 1);
    let nan = thaw_quickjs::thaw_js_get_global(c"napiNan".as_ptr());
    let positive = thaw_quickjs::thaw_js_get_global(c"napiPositiveZero".as_ptr());
    let negative = thaw_quickjs::thaw_js_get_global(c"napiNegativeZero".as_ptr());
    let mut env = Env::new();
    for handle in [nan, positive, negative] {
        assert_eq!(thaw_quickjs::thaw_js_retain_handle(handle), 1);
    }
    let graph = format!(
        r#"{{"root":{{"r":0}},"nodes":[{{"a":[{{"r":1}},{{"r":2}},{{"r":3}}]}},{{"hdl":{nan}}},{{"hdl":{positive}}},{{"hdl":{negative}}}],"leases":[{nan},{positive},{negative}]}}"#
    );
    let values = unsafe { parse_napi_graph_value(&mut env, &graph) }.unwrap();
    let Value::Array(values) = unsafe { value_ref(values) }.unwrap() else {
        panic!("array root")
    };
    let mut equal = true;
    assert_eq!(
        unsafe { napi_strict_equals(&mut env, values[0].unwrap(), values[0].unwrap(), &mut equal) },
        NAPI_OK
    );
    assert!(!equal);
    assert_eq!(
        unsafe { napi_strict_equals(&mut env, values[1].unwrap(), values[2].unwrap(), &mut equal) },
        NAPI_OK
    );
    assert!(equal);
    let native_zero = env.alloc(Value::Number(0.0));
    assert_eq!(
        unsafe { napi_strict_equals(&mut env, values[2].unwrap(), native_zero, &mut equal) },
        NAPI_OK
    );
    assert!(equal);
    assert_eq!(
        unsafe { napi_strict_equals(&mut env, native_zero, values[2].unwrap(), &mut equal) },
        NAPI_OK
    );
    assert!(equal);
    let native_nan = env.alloc(Value::Number(f64::NAN));
    assert_eq!(
        unsafe { napi_strict_equals(&mut env, values[0].unwrap(), native_nan, &mut equal) },
        NAPI_OK
    );
    assert!(!equal);
    for handle in [nan, positive, negative] {
        assert_eq!(thaw_quickjs::thaw_js_release_handle(handle), 1);
    }
    drop(env);
}

// Unrun source control: own-key metadata and prototype traversal must not
// read an accessor value; numeric keys and live Symbols keep their key kinds.
#[cfg(feature = "quickjs")]
#[test]
fn live_quickjs_property_names_filter_descriptors_without_getting_values() {
    assert_eq!(thaw_quickjs::thaw_js_load(c"globalThis.napiEnumReads = 0; globalThis.napiEnumSymbol = Symbol('enum'); globalThis.napiEnumObject = Object.create({ fromProto: 4, shadowed: 7 }); Object.defineProperty(napiEnumObject, '2', { value: 2, enumerable: true, configurable: true }); Object.defineProperty(napiEnumObject, 'lazy', { get() { napiEnumReads++; return 3; }, enumerable: true, configurable: true }); Object.defineProperty(napiEnumObject, 'hidden', { value: 5, enumerable: false, configurable: true }); Object.defineProperty(napiEnumObject, 'shadowed', { value: 8, enumerable: false, configurable: true }); Object.defineProperty(napiEnumObject, napiEnumSymbol, { value: 6, enumerable: true, configurable: true });".as_ptr()), 1);
    let handle = thaw_quickjs::thaw_js_get_global(c"napiEnumObject".as_ptr());
    assert_ne!(handle, 0);
    assert_eq!(thaw_quickjs::thaw_js_retain_handle(handle), 1);
    let graph = format!(r#"{{"root":{{"r":0}},"nodes":[{{"hdl":{handle}}}],"leases":[{handle}]}}"#);
    let mut env = Env::new();
    let object = unsafe { parse_napi_graph_value(&mut env, &graph) }.unwrap();
    let mut own = ptr::null_mut();
    assert_eq!(
        unsafe {
            napi_get_all_property_names(
                &mut env,
                object,
                NAPI_KEY_OWN_ONLY,
                NAPI_KEY_ALL_PROPERTIES,
                NAPI_KEY_KEEP_NUMBERS,
                &mut own,
            )
        },
        NAPI_OK
    );
    let Value::Array(own) = unsafe { value_ref(own) }.unwrap() else {
        panic!("own keys")
    };
    assert!(own.iter().flatten().any(
        |key| matches!(unsafe { value_ref(*key) }, Ok(Value::Number(number)) if *number == 2.0)
    ));
    assert!(own.iter().flatten().any(
        |key| matches!(unsafe { value_ref(*key) }, Ok(Value::String(name)) if name == "hidden")
    ));
    assert!(own
        .iter()
        .flatten()
        .any(|key| matches!(unsafe { value_ref(*key) }, Ok(Value::QuickJsHandle { .. }))));
    let mut visible = ptr::null_mut();
    assert_eq!(
        unsafe { napi_get_property_names(&mut env, object, &mut visible) },
        NAPI_OK
    );
    let Value::Array(visible) = unsafe { value_ref(visible) }.unwrap() else {
        panic!("visible keys")
    };
    assert!(visible.iter().flatten().any(
        |key| matches!(unsafe { value_ref(*key) }, Ok(Value::String(name)) if name == "fromProto")
    ));
    assert!(!visible.iter().flatten().any(
        |key| matches!(unsafe { value_ref(*key) }, Ok(Value::String(name)) if name == "hidden")
    ));
    assert!(!visible.iter().flatten().any(
        |key| matches!(unsafe { value_ref(*key) }, Ok(Value::String(name)) if name == "shadowed")
    ));
    assert!(!visible
        .iter()
        .flatten()
        .any(|key| matches!(unsafe { value_ref(*key) }, Ok(Value::QuickJsHandle { .. }))));
    let reads = thaw_quickjs::thaw_js_get_global(c"napiEnumReads".as_ptr());
    assert_eq!(unsafe { qjs_query(reads, 2) }.unwrap(), "0");
    assert_eq!(thaw_quickjs::thaw_js_release_handle(reads), 0);
    assert_eq!(thaw_quickjs::thaw_js_release_handle(handle), 1);
    drop(env);
}

// Unrun control: distinct unpaired surrogates are distinct property keys and
// survive N-API UTF-16 getter and the live QuickJS property bridge exactly.
#[cfg(feature = "quickjs")]
#[test]
fn live_quickjs_native_utf16_property_keys_keep_surrogate_units() {
    let mut env = Env::new();
    let mut high_string = ptr::null_mut();
    let mut low_string = ptr::null_mut();
    assert_eq!(
        unsafe { napi_create_string_utf16(&mut env, [0xd800_u16].as_ptr(), 1, &mut high_string) },
        NAPI_OK
    );
    assert_eq!(
        unsafe { napi_create_string_utf16(&mut env, [0xdc00_u16].as_ptr(), 1, &mut low_string) },
        NAPI_OK
    );
    let replacement = env.alloc(Value::String("\u{fffd}".into()));
    let mut equal = true;
    for (left, right) in [
        (high_string, low_string),
        (high_string, replacement),
        (low_string, replacement),
    ] {
        assert_eq!(
            unsafe { napi_strict_equals(&mut env, left, right, &mut equal) },
            NAPI_OK
        );
        assert!(!equal);
    }
    let joined_array = env.alloc(Value::Array(vec![Some(high_string), Some(low_string)]));
    let mut joined = ptr::null_mut();
    assert_eq!(
        unsafe { napi_coerce_to_string(&mut env, joined_array, &mut joined) },
        NAPI_OK
    );
    let mut joined_units = [0_u16; 4];
    let mut joined_count = 0;
    assert_eq!(
        unsafe {
            napi_get_value_string_utf16(
                &mut env,
                joined,
                joined_units.as_mut_ptr(),
                joined_units.len(),
                &mut joined_count,
            )
        },
        NAPI_OK
    );
    assert_eq!(
        (joined_count, &joined_units[..3]),
        (3, &[0xd800, b',' as u16, 0xdc00][..])
    );
    let valid_scalar = unsafe {
        parse_napi_graph_value(&mut env, r#"{"root":{"su":[97]},"nodes":[],"leases":[]}"#)
    }
    .unwrap();
    let ordinary_a = env.alloc(Value::String("a".into()));
    assert_eq!(
        unsafe { napi_strict_equals(&mut env, valid_scalar, ordinary_a, &mut equal) },
        NAPI_OK
    );
    assert!(equal);
    let valid_key_object = unsafe {
        parse_napi_graph_value(
            &mut env,
            r#"{"root":{"r":0},"nodes":[{"o":[[{"su":[97]},{"v":3}]]}],"leases":[]}"#,
        )
    }
    .unwrap();
    let mut canonical_value = ptr::null_mut();
    assert_eq!(
        unsafe {
            napi_get_named_property(
                &mut env,
                valid_key_object,
                c"a".as_ptr(),
                &mut canonical_value,
            )
        },
        NAPI_OK
    );
    let Ok(Value::Number(canonical_number)) = (unsafe { value_ref(canonical_value) }) else {
        panic!("canonical graph key must be visible as ordinary string property");
    };
    assert_eq!(*canonical_number, 3.0);
    let mut high = ptr::null_mut();
    let mut low = ptr::null_mut();
    assert_eq!(
        unsafe {
            node_api_create_property_key_utf16(&mut env, [0xd800_u16].as_ptr(), 1, &mut high)
        },
        NAPI_OK
    );
    assert_eq!(
        unsafe { node_api_create_property_key_utf16(&mut env, [0xdc00_u16].as_ptr(), 1, &mut low) },
        NAPI_OK
    );
    assert_ne!(high, low);
    let mut unit = [0_u16; 2];
    let mut written = 0;
    assert_eq!(
        unsafe {
            napi_get_value_string_utf16(&mut env, high, unit.as_mut_ptr(), unit.len(), &mut written)
        },
        NAPI_OK
    );
    assert_eq!((written, unit[0]), (1, 0xd800));
    assert_eq!(
        unsafe {
            napi_get_value_string_utf16(&mut env, low, unit.as_mut_ptr(), unit.len(), &mut written)
        },
        NAPI_OK
    );
    assert_eq!((written, unit[0]), (1, 0xdc00));
    let mut native = ptr::null_mut();
    assert_eq!(
        unsafe { napi_create_object(&mut env, &mut native) },
        NAPI_OK
    );
    let nul_units = [b'a' as u16, 0, b'b' as u16];
    let mut nul_key = ptr::null_mut();
    assert_eq!(
        unsafe {
            napi_create_string_utf16(&mut env, nul_units.as_ptr(), nul_units.len(), &mut nul_key)
        },
        NAPI_OK
    );
    let one = env.alloc(Value::Number(1.0));
    let two = env.alloc(Value::Number(2.0));
    assert_eq!(
        unsafe { napi_set_property(&mut env, native, nul_key, one) },
        NAPI_OK
    );
    let mut present = false;
    assert_eq!(
        unsafe { napi_has_property(&mut env, native, nul_key, &mut present) },
        NAPI_OK
    );
    assert!(present);
    let mut nul_value = ptr::null_mut();
    assert_eq!(
        unsafe { napi_get_property(&mut env, native, nul_key, &mut nul_value) },
        NAPI_OK
    );
    assert_eq!(nul_value, one);
    assert_eq!(
        unsafe { napi_set_property(&mut env, native, high, one) },
        NAPI_OK
    );
    assert_eq!(
        unsafe { napi_set_property(&mut env, native, low, two) },
        NAPI_OK
    );
    let mut found = ptr::null_mut();
    assert_eq!(
        unsafe { napi_get_property(&mut env, native, high, &mut found) },
        NAPI_OK
    );
    assert_eq!(found, one);
    assert_eq!(
        unsafe { napi_get_property(&mut env, native, low, &mut found) },
        NAPI_OK
    );
    assert_eq!(found, two);
    let encoded: JsonValue = serde_json::from_str(
        &unsafe { napi_result_graph_for_env(&mut env, native, true) }.unwrap(),
    )
    .unwrap();
    let entries = encoded["nodes"][0]["o"].as_array().unwrap();
    assert!(entries
        .iter()
        .any(|entry| entry[0]["su"] == serde_json::json!([0xd800])));
    assert!(entries
        .iter()
        .any(|entry| entry[0]["su"] == serde_json::json!([0xdc00])));
    assert!(entries.iter().any(|entry| entry[0] == "a\u{0}b"));
    let mut native_deleted = false;
    assert_eq!(
        unsafe { napi_delete_property(&mut env, native, nul_key, &mut native_deleted) },
        NAPI_OK
    );
    assert!(native_deleted);
    assert_eq!(
        unsafe { napi_has_property(&mut env, native, nul_key, &mut present) },
        NAPI_OK
    );
    assert!(!present);

    assert_eq!(
        thaw_quickjs::thaw_js_load(c"globalThis.napiUtf16Object = {};".as_ptr()),
        1
    );
    let handle = thaw_quickjs::thaw_js_get_global(c"napiUtf16Object".as_ptr());
    assert_ne!(handle, 0);
    assert_eq!(thaw_quickjs::thaw_js_retain_handle(handle), 1);
    let graph = format!(r#"{{"root":{{"r":0}},"nodes":[{{"hdl":{handle}}}],"leases":[{handle}]}}"#);
    let live = unsafe { parse_napi_graph_value(&mut env, &graph) }.unwrap();
    assert_eq!(
        unsafe { napi_set_property(&mut env, live, high, one) },
        NAPI_OK
    );
    assert_eq!(
        unsafe { napi_set_property(&mut env, live, low, two) },
        NAPI_OK
    );
    assert_eq!(
        unsafe { napi_set_property(&mut env, live, nul_key, one) },
        NAPI_OK
    );
    assert_eq!(
        thaw_quickjs::thaw_js_load(
            c"globalThis.napiSavedReflectSet = Reflect.set; Reflect.set = () => false;".as_ptr()
        ),
        1
    );
    assert_eq!(
        unsafe { napi_set_property(&mut env, live, nul_key, two) },
        NAPI_OK
    );
    assert_eq!(
        thaw_quickjs::thaw_js_load(
            c"Reflect.set = napiSavedReflectSet; delete globalThis.napiSavedReflectSet;".as_ptr()
        ),
        1
    );
    assert_eq!(
        unsafe { napi_has_property(&mut env, live, nul_key, &mut present) },
        NAPI_OK
    );
    assert!(present);
    assert_eq!(
        unsafe { napi_get_property(&mut env, live, nul_key, &mut nul_value) },
        NAPI_OK
    );
    let mut nul_number = 0.0;
    assert_eq!(
        unsafe { napi_get_value_double(&mut env, nul_value, &mut nul_number) },
        NAPI_OK
    );
    assert_eq!(nul_number, 2.0);
    let mut nul_names = ptr::null_mut();
    assert_eq!(
        unsafe {
            napi_get_all_property_names(
                &mut env,
                live,
                NAPI_KEY_OWN_ONLY,
                NAPI_KEY_ALL_PROPERTIES,
                NAPI_KEY_NUMBERS_TO_STRINGS,
                &mut nul_names,
            )
        },
        NAPI_OK
    );
    let Value::Array(nul_names) = unsafe { value_ref(nul_names) }.unwrap() else {
        panic!("NUL keys")
    };
    assert!(nul_names
        .iter()
        .flatten()
        .any(|key| matches!(unsafe { value_ref(*key) },
        Ok(Value::String(name)) if name == "a\u{0}b")));
    let mut deleted = false;
    assert_eq!(
        unsafe { napi_delete_property(&mut env, live, nul_key, &mut deleted) },
        NAPI_OK
    );
    assert!(deleted);
    assert_eq!(
        unsafe { napi_has_property(&mut env, live, nul_key, &mut present) },
        NAPI_OK
    );
    assert!(!present);
    assert_eq!(
        unsafe { napi_get_property(&mut env, live, high, &mut found) },
        NAPI_OK
    );
    let mut number = 0.0;
    assert_eq!(
        unsafe { napi_get_value_double(&mut env, found, &mut number) },
        NAPI_OK
    );
    assert_eq!(number, 1.0);
    assert_eq!(
        unsafe { napi_get_property(&mut env, live, low, &mut found) },
        NAPI_OK
    );
    assert_eq!(
        unsafe { napi_get_value_double(&mut env, found, &mut number) },
        NAPI_OK
    );
    assert_eq!(number, 2.0);
    let mut keys = ptr::null_mut();
    assert_eq!(
        unsafe {
            napi_get_all_property_names(
                &mut env,
                live,
                NAPI_KEY_OWN_ONLY,
                NAPI_KEY_ALL_PROPERTIES,
                NAPI_KEY_NUMBERS_TO_STRINGS,
                &mut keys,
            )
        },
        NAPI_OK
    );
    let Value::Array(names) = unsafe { value_ref(keys) }.unwrap() else {
        panic!("UTF-16 keys")
    };
    let names = names.clone();
    let mut units = names
        .iter()
        .flatten()
        .map(|name| {
            let mut unit = [0_u16; 2];
            let mut count = 0;
            assert_eq!(
                unsafe {
                    napi_get_value_string_utf16(
                        &mut env,
                        *name,
                        unit.as_mut_ptr(),
                        unit.len(),
                        &mut count,
                    )
                },
                NAPI_OK
            );
            assert_eq!(count, 1);
            unit[0]
        })
        .collect::<Vec<_>>();
    units.sort_unstable();
    assert_eq!(units, vec![0xd800, 0xdc00]);
    assert_eq!(thaw_quickjs::thaw_js_release_handle(handle), 1);
    drop(env);
}

// Unrun: N-API ToString observes the string-hint primitive hook once, retains
// exact UTF-16 units, and propagates the original thrown JavaScript value.
#[cfg(feature = "quickjs")]
#[test]
fn live_quickjs_string_coercion_preserves_effects_units_and_throw() {
    assert_eq!(thaw_quickjs::thaw_js_load(c"globalThis.napiCoercionReads = 0; globalThis.napiCoercionThrown = { token: 1 }; globalThis.napiCoercionValue = { [Symbol.toPrimitive](hint) { napiCoercionReads++; if (hint !== 'string') throw new Error('wrong hint'); return '\\uD800'; } }; globalThis.napiCoercionBad = { [Symbol.toPrimitive]() { throw napiCoercionThrown; } }; globalThis.napiCoercionSymbol = Symbol('native'); globalThis.napiCoercionNull = null;".as_ptr()), 1);
    let mut env = Env::new();
    let good_handle = thaw_quickjs::thaw_js_get_global(c"napiCoercionValue".as_ptr());
    let bad_handle = thaw_quickjs::thaw_js_get_global(c"napiCoercionBad".as_ptr());
    let symbol_handle = thaw_quickjs::thaw_js_get_global(c"napiCoercionSymbol".as_ptr());
    let null_handle = thaw_quickjs::thaw_js_get_global(c"napiCoercionNull".as_ptr());
    let thrown_handle = thaw_quickjs::thaw_js_get_global(c"napiCoercionThrown".as_ptr());
    for handle in [good_handle, bad_handle, symbol_handle, null_handle] {
        assert_eq!(thaw_quickjs::thaw_js_retain_handle(handle), 1);
    }
    let good = unsafe {
        parse_napi_graph_value(
            &mut env,
            &format!(
                r#"{{"root":{{"r":0}},"nodes":[{{"hdl":{good_handle}}}],"leases":[{good_handle}]}}"#
            ),
        )
    }
    .unwrap();
    let bad = unsafe {
        parse_napi_graph_value(
            &mut env,
            &format!(
                r#"{{"root":{{"r":0}},"nodes":[{{"hdl":{bad_handle}}}],"leases":[{bad_handle}]}}"#
            ),
        )
    }
    .unwrap();
    let symbol = unsafe { parse_napi_graph_value(&mut env, &format!(
        r#"{{"root":{{"r":0}},"nodes":[{{"hdl":{symbol_handle}}}],"leases":[{symbol_handle}]}}"#)) }.unwrap();
    let nullish = unsafe {
        parse_napi_graph_value(
            &mut env,
            &format!(
                r#"{{"root":{{"r":0}},"nodes":[{{"hdl":{null_handle}}}],"leases":[{null_handle}]}}"#
            ),
        )
    }
    .unwrap();
    let nested = env.alloc(Value::Array(vec![Some(good), Some(nullish)]));
    let mut output = ptr::null_mut();
    assert_eq!(
        unsafe { napi_coerce_to_string(&mut env, nested, &mut output) },
        NAPI_OK
    );
    let mut units = [0_u16; 3];
    let mut written = 0;
    assert_eq!(
        unsafe {
            napi_get_value_string_utf16(
                &mut env,
                output,
                units.as_mut_ptr(),
                units.len(),
                &mut written,
            )
        },
        NAPI_OK
    );
    assert_eq!((written, &units[..2]), (2, &[0xd800, b',' as u16][..]));
    let reads = thaw_quickjs::thaw_js_get_global(c"napiCoercionReads".as_ptr());
    assert_eq!(unsafe { qjs_query(reads, 10) }.unwrap(), "1");
    assert_eq!(
        unsafe { napi_coerce_to_string(&mut env, bad, &mut output) },
        NAPI_PENDING_EXCEPTION
    );
    let mut exception = ptr::null_mut();
    assert_eq!(
        unsafe { napi_get_and_clear_last_exception(&mut env, &mut exception) },
        NAPI_OK
    );
    let retained = unsafe { qjs_handle(exception) }.expect("original JS thrown value");
    let same = thaw_quickjs::thaw_js_strict_equal_handles_result(retained, thrown_handle);
    assert!(same.error.is_null());
    assert_eq!(same.value, 1);
    assert_eq!(thaw_quickjs::thaw_js_load(c"globalThis.napiSavedConcat = String.prototype.concat; String.prototype.concat = () => 'spoof';".as_ptr()), 1);
    assert_eq!(
        unsafe { napi_coerce_to_string(&mut env, symbol, &mut output) },
        NAPI_PENDING_EXCEPTION
    );
    assert_eq!(
        unsafe { napi_get_and_clear_last_exception(&mut env, &mut exception) },
        NAPI_OK
    );
    assert!(unsafe { qjs_handle(exception) }.is_some());
    assert_eq!(
        thaw_quickjs::thaw_js_load(
            c"String.prototype.concat = napiSavedConcat; delete globalThis.napiSavedConcat;"
                .as_ptr()
        ),
        1
    );
    for handle in [
        good_handle,
        bad_handle,
        symbol_handle,
        null_handle,
        thrown_handle,
        reads,
    ] {
        thaw_quickjs::thaw_js_release_handle(handle);
    }
}

// Unrun: QuickJS getter/setter failures reached through N-API retain the
// original thrown object. The public HostOperations setter keeps its older
// three-argument ABI; these N-API operations use the private result path.
#[cfg(feature = "quickjs")]
#[test]
fn live_quickjs_property_errors_preserve_original_exception_identity() {
    assert_eq!(thaw_quickjs::thaw_js_load(c"globalThis.napiPropertyThrown = { marker: 1 }; globalThis.napiPropertyTarget = {}; Object.defineProperty(napiPropertyTarget, 'read', { get() { throw napiPropertyThrown; } }); Object.defineProperty(napiPropertyTarget, 'write', { set(_) { throw napiPropertyThrown; } });".as_ptr()), 1);
    assert_eq!(thaw_quickjs::thaw_js_load(c"globalThis.napiSavedJSONParse = JSON.parse; JSON.parse = () => { throw new Error('spoofed parser'); };".as_ptr()), 1);
    let mut env = Env::new();
    let target_handle = thaw_quickjs::thaw_js_get_global(c"napiPropertyTarget".as_ptr());
    let thrown_handle = thaw_quickjs::thaw_js_get_global(c"napiPropertyThrown".as_ptr());
    assert_eq!(thaw_quickjs::thaw_js_retain_handle(target_handle), 1);
    let target = unsafe { parse_napi_graph_value(&mut env, &format!(
        r#"{{"root":{{"r":0}},"nodes":[{{"hdl":{target_handle}}}],"leases":[{target_handle}]}}"#)) }.unwrap();
    let mut value = ptr::null_mut();
    assert_eq!(
        unsafe { napi_get_named_property(&mut env, target, c"read".as_ptr(), &mut value) },
        NAPI_PENDING_EXCEPTION
    );
    let mut exception = ptr::null_mut();
    assert_eq!(
        unsafe { napi_get_and_clear_last_exception(&mut env, &mut exception) },
        NAPI_OK
    );
    let returned = unsafe { qjs_handle(exception) }.expect("original JS getter exception");
    let same = thaw_quickjs::thaw_js_strict_equal_handles_result(returned, thrown_handle);
    assert!(same.error.is_null());
    assert_eq!(same.value, 1);
    let argument = env.alloc(Value::Number(3.0));
    assert_eq!(
        unsafe { napi_set_named_property(&mut env, target, c"write".as_ptr(), argument) },
        NAPI_PENDING_EXCEPTION
    );
    assert_eq!(
        unsafe { napi_get_and_clear_last_exception(&mut env, &mut exception) },
        NAPI_OK
    );
    let returned = unsafe { qjs_handle(exception) }.expect("original JS setter exception");
    let same = thaw_quickjs::thaw_js_strict_equal_handles_result(returned, thrown_handle);
    assert!(same.error.is_null());
    assert_eq!(same.value, 1);
    assert_eq!(
        thaw_quickjs::thaw_js_load(
            c"JSON.parse = napiSavedJSONParse; delete globalThis.napiSavedJSONParse;".as_ptr()
        ),
        1
    );
    thaw_quickjs::thaw_js_release_handle(target_handle);
    thaw_quickjs::thaw_js_release_handle(thrown_handle);
}

// Unrun: a live native Function may lose a configurable own key after its
// callable Proxy target has been made nonextensible. The target shadow must
// then be removed; a native key that temporarily overrode the bound target's
// intrinsic name must instead restore that original descriptor.
#[cfg(feature = "quickjs")]
#[test]
fn native_function_proxy_reconciles_deleted_shadows_after_prevent_extensions() {
    unsafe extern "C" fn return_undefined(env: NapiEnv, _: NapiCallbackInfo) -> NapiValue {
        env_mut(env).unwrap().alloc(Value::Undefined)
    }
    thaw_quickjs::register_napi_bridge(
        thaw_napi_export_names,
        thaw_napi_call_typed_bridge,
        thaw_napi_handle_bridge,
        thaw_napi_poll_async_work,
        thaw_napi_async_work_pending,
        thaw_napi_graph_owner,
        release_napi_graph_reference,
    );
    let mut env = Box::new(Env::new());
    env.graph_owner_id = 7017;
    let env_ptr: NapiEnv = &mut *env;
    let function = env.alloc(Value::Function(Function {
        callback: return_undefined,
        data: ptr::null_mut(),
        properties: HashMap::new(),
        _thaw_bridge: None,
        _accessor_owner: None,
    }));
    HOST.with(|host| host.borrow_mut().module_envs.push(env));
    let source = CString::new(format!(
        r#"
        globalThis.liveNativeFunction = __thaw_json_graph_decode({{
            root: {{ r: 0 }}, nodes: [{{ nfn: "{}" }}]
        }});
        globalThis.originalNativeFunctionName = liveNativeFunction.name;
    "#,
        function as u64
    ))
    .unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let extra = unsafe { env_mut(env_ptr) }
        .unwrap()
        .alloc(Value::Number(3.0));
    let renamed = unsafe { env_mut(env_ptr) }
        .unwrap()
        .alloc(Value::String("addon name".into()));
    assert_eq!(
        unsafe { napi_set_named_property(env_ptr, function, c"extra".as_ptr(), extra) },
        NAPI_OK
    );
    assert_eq!(
        unsafe { napi_set_named_property(env_ptr, function, c"name".as_ptr(), renamed) },
        NAPI_OK
    );
    assert_eq!(
        thaw_quickjs::thaw_js_load(c"Object.preventExtensions(liveNativeFunction);".as_ptr()),
        1
    );
    for name in ["extra", "name"] {
        let key = unsafe { env_mut(env_ptr) }
            .unwrap()
            .alloc(Value::String(name.into()));
        let mut deleted = false;
        assert_eq!(
            unsafe { napi_delete_property(env_ptr, function, key, &mut deleted) },
            NAPI_OK
        );
        assert!(deleted);
    }
    assert_eq!(thaw_quickjs::thaw_js_load(c"globalThis.nativeFunctionShadowCleared = !Reflect.ownKeys(liveNativeFunction).includes('extra') && Reflect.getOwnPropertyDescriptor(liveNativeFunction, 'extra') === undefined && liveNativeFunction.extra === undefined && liveNativeFunction.name === originalNativeFunctionName; delete globalThis.liveNativeFunction; delete globalThis.originalNativeFunctionName;".as_ptr()), 1);
    let result = thaw_quickjs::thaw_js_get_global(c"nativeFunctionShadowCleared".as_ptr());
    assert_eq!(unsafe { qjs_query(result, 2) }.unwrap(), "true");
    assert_eq!(thaw_quickjs::thaw_js_release_handle(result), 1);
    let owned = HOST
        .with(|host| host.borrow_mut().module_envs.pop())
        .unwrap();
    drop(owned);
}

// Unrun: the callable proxy's construct trap must carry the engine's actual
// new.target into N-API, read its prototype once before the callback, and use
// that prototype only for the implicit instance. An explicit object returned
// by the native constructor remains authoritative.
#[cfg(feature = "quickjs")]
#[test]
fn native_function_proxy_construct_preserves_derived_new_target() {
    static OBSERVED_TARGET_TAG: AtomicUsize = AtomicUsize::new(0);
    unsafe extern "C" fn constructor(env: NapiEnv, info: NapiCallbackInfo) -> NapiValue {
        let mut new_target = ptr::null_mut();
        assert_eq!(napi_get_new_target(env, info, &mut new_target), NAPI_OK);
        let mut tag = ptr::null_mut();
        assert_eq!(
            napi_get_named_property(env, new_target, c"derivedTag".as_ptr(), &mut tag),
            NAPI_OK
        );
        let mut number = 0.0;
        let status = napi_get_value_double(env, tag, &mut number);
        let number = if status == NAPI_OK {
            number as usize
        } else {
            0
        };
        OBSERVED_TARGET_TAG.store(number, Ordering::SeqCst);
        if number == 23 {
            return info.as_ref().unwrap().data as NapiValue;
        }
        if number == 29 {
            let mut thrown = ptr::null_mut();
            assert_eq!(
                napi_get_named_property(env, new_target, c"throwObject".as_ptr(), &mut thrown),
                NAPI_OK
            );
            assert_eq!(napi_throw(env, thrown), NAPI_OK);
            return ptr::null_mut();
        }
        ptr::null_mut()
    }
    thaw_quickjs::register_napi_bridge(
        thaw_napi_export_names,
        thaw_napi_call_typed_bridge,
        thaw_napi_handle_bridge,
        thaw_napi_poll_async_work,
        thaw_napi_async_work_pending,
        thaw_napi_graph_owner,
        release_napi_graph_reference,
    );
    let mut env = Box::new(Env::new());
    env.graph_owner_id = 7018;
    let env_ptr: NapiEnv = &mut *env;
    let explicit_return = env.alloc(Value::Object(HashMap::new()));
    let function = env.alloc(Value::Function(Function {
        callback: constructor,
        data: explicit_return.cast(),
        properties: HashMap::new(),
        _thaw_bridge: None,
        _accessor_owner: None,
    }));
    let base_prototype = env.alloc(Value::Object(HashMap::new()));
    assert_eq!(
        unsafe {
            napi_set_named_property(env_ptr, function, c"prototype".as_ptr(), base_prototype)
        },
        NAPI_OK
    );
    HOST.with(|host| host.borrow_mut().module_envs.push(env));
    let source = CString::new(format!(
        r#"
        globalThis.nativeCtorWithTarget = __thaw_json_graph_decode({{
            root: {{ r: 0 }}, nodes: [{{ nfn: "{}" }}]
        }});
        globalThis.nativeCtorWithTargetResult = (() => {{
            let ownTargetPrototypeReads = 0;
            const ownTargetPrototype = {{ native: true }};
            Object.defineProperty(nativeCtorWithTarget, 'prototype', {{
                configurable: true,
                get() {{ ownTargetPrototypeReads++; return ownTargetPrototype; }}
            }});
            const direct = new nativeCtorWithTarget();
            const directTargetRead = ownTargetPrototypeReads === 1
                && Object.getPrototypeOf(direct) === ownTargetPrototype;
            class Derived extends nativeCtorWithTarget {{
                static derivedTag = 17;
            }}
            const derived = new Derived();
            let prototypeReads = 0;
            const desired = {{ derived: true }};
            const customTarget = new Proxy(function() {{}}, {{
                get(target, key, receiver) {{
                    if (key === 'prototype') {{ prototypeReads++; return desired; }}
                    if (key === 'derivedTag') return 23;
                    return Reflect.get(target, key, receiver);
                }}
            }});
            const explicit = Reflect.construct(nativeCtorWithTarget, [], customTarget);
            const explicitAgain = Reflect.construct(nativeCtorWithTarget, [], customTarget);
            const marker = {{ nativeThrow: true }};
            const throwTarget = new Proxy(function() {{}}, {{
                get(target, key, receiver) {{
                    if (key === 'derivedTag') return 29;
                    if (key === 'throwObject') return marker;
                    return Reflect.get(target, key, receiver);
                }}
            }});
            let thrownIdentity = false;
            try {{ Reflect.construct(nativeCtorWithTarget, [], throwTarget); }}
            catch (error) {{ thrownIdentity = error === marker; }}
            let primitivePrototypeReads = 0;
            const primitiveTarget = new Proxy(function() {{}}, {{
                get(target, key, receiver) {{
                    if (key === 'prototype') {{ primitivePrototypeReads++; return 7; }}
                    return Reflect.get(target, key, receiver);
                }}
            }});
            const ordinaryFallback = Reflect.construct(nativeCtorWithTarget,
                [], primitiveTarget);
            return [directTargetRead, derived instanceof Derived,
                Object.getPrototypeOf(derived) === Derived.prototype,
                prototypeReads === 2,
                Object.getPrototypeOf(explicit) !== desired,
                explicit === explicitAgain, thrownIdentity,
                primitivePrototypeReads === 1,
                Object.getPrototypeOf(ordinaryFallback) === Object.prototype];
        }})();
    "#,
        function as u64
    ))
    .unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let result = thaw_quickjs::thaw_js_get_global(c"nativeCtorWithTargetResult".as_ptr());
    assert_eq!(
        unsafe { qjs_query(result, 2) }.unwrap(),
        "true,true,true,true,true,true,true,true,true"
    );
    assert_eq!(OBSERVED_TARGET_TAG.load(Ordering::SeqCst), 0);
    assert_eq!(thaw_quickjs::thaw_js_release_handle(result), 1);
    assert_eq!(thaw_quickjs::thaw_js_load(
        c"delete globalThis.nativeCtorWithTargetResult; delete globalThis.nativeCtorWithTarget;".as_ptr()), 1);
    let owned = HOST
        .with(|host| host.borrow_mut().module_envs.pop())
        .unwrap();
    drop(owned);
}
