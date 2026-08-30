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
