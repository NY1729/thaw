#[cfg(target_os = "linux")]
#[test]
fn poll_uv_loop_drives_only_the_host_owned_libuv_loop() {
    let _guard = lock_async_test();
    unsafe {
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
        let mut env = Env::new();
        let mut event_loop = ptr::null_mut();
        assert_eq!(napi_get_uv_event_loop(&mut env, &mut event_loop), NAPI_OK);
        assert!(!event_loop.is_null());
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
        assert_eq!(timer_init(event_loop, timer), 0);
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
        for _ in 0..100 {
            poll_uv_loop();
            if !uv_handles_open() { break; }
        }
        assert!(!uv_handles_open());
        libc::free(timer);
        assert!(close_owned_uv_loop());
        libc::dlclose(library);
    }
}

#[cfg(target_os = "linux")]
#[test]
fn recorded_main_default_loop_progresses_without_worker_polling_it() {
    let _guard = lock_async_test();
    unsafe {
        type UvDefaultLoop = unsafe extern "C" fn() -> *mut c_void;
        type UvHandleSize = unsafe extern "C" fn(i32) -> usize;
        type UvTimerInit = unsafe extern "C" fn(*mut c_void, *mut c_void) -> i32;
        type UvTimerStart = unsafe extern "C" fn(*mut c_void,
            Option<unsafe extern "C" fn(*mut c_void)>, u64, u64) -> i32;
        let library = libc::dlopen(c"libuv.so.1".as_ptr(), libc::RTLD_NOW | libc::RTLD_GLOBAL);
        assert!(!library.is_null());
        let default_loop = std::mem::transmute::<*mut c_void, UvDefaultLoop>(
            libc::dlsym(libc::RTLD_DEFAULT, c"uv_default_loop".as_ptr()))();
        assert!(!default_loop.is_null());
        // load_impl records this exact pointer in the process-main Host.
        HOST.with(|host| host.borrow_mut().main_default_uv_loop = Some(default_loop as usize));
        let owner_loop = default_loop as usize;
        assert!(known_uv_loops().contains(&owner_loop));
        assert!(!std::thread::spawn(move || known_uv_loops().contains(&owner_loop))
            .join().unwrap());
        let size = std::mem::transmute::<*mut c_void, UvHandleSize>(
            libc::dlsym(libc::RTLD_DEFAULT, c"uv_handle_size".as_ptr()));
        let init = std::mem::transmute::<*mut c_void, UvTimerInit>(
            libc::dlsym(libc::RTLD_DEFAULT, c"uv_timer_init".as_ptr()));
        let start = std::mem::transmute::<*mut c_void, UvTimerStart>(
            libc::dlsym(libc::RTLD_DEFAULT, c"uv_timer_start".as_ptr()));
        let timer = libc::calloc(1, size(13));
        assert!(!timer.is_null());
        assert_eq!(init(default_loop, timer), 0);
        UV_TIMER_FIRED.store(false, Ordering::Release);
        assert_eq!(start(timer, Some(test_uv_timer_callback), 1, 0), 0);
        for _ in 0..1000 {
            thaw_napi_poll_async_work();
            if UV_TIMER_FIRED.load(Ordering::Acquire) { break; }
            std::thread::sleep(Duration::from_millis(1));
        }
        assert!(UV_TIMER_FIRED.load(Ordering::Acquire));
        for _ in 0..100 { thaw_napi_poll_async_work(); if !uv_handles_open() { break; } }
        assert!(!uv_handles_open());
        HOST.with(|host| host.borrow_mut().main_default_uv_loop = None);
        libc::free(timer);
        libc::dlclose(library);
    }
}

#[cfg(target_os = "linux")]
#[test]
fn failed_addon_load_retains_libuv_reference_until_host_shutdown() {
    let _guard = lock_async_test();
    let before = HOST.with(|host| host.borrow().libraries.len());
    let path = format!("/tmp/thaw_missing_addon_{}.node", std::process::id());
    assert!(unsafe { load_impl(&path, None, None) }.is_err());
    assert_eq!(HOST.with(|host| host.borrow().libraries.len()), before + 1);
    assert_eq!(thaw_napi_begin_shutdown(), 1);
    assert_eq!(thaw_napi_poll_shutdown(), 1);
    assert_eq!(thaw_napi_finish_shutdown(), 1);
    assert!(HOST.with(|host| host.borrow().libraries.is_empty()));
}

#[test]
fn ready_events_stay_with_their_creating_worker_thread() {
    let _guard = lock_async_test();
    let other_owner = std::thread::spawn(|| std::thread::current().id()).join().unwrap();
    let make_work = |owner| Box::new(AsyncWork {
        env: 0,
        owner,
        execute: noop_execute,
        complete: None,
        data: 0,
        state: AtomicU8::new(ASYNC_COMPLETE_PENDING),
        completion_status: AtomicI32::new(NAPI_OK),
    });
    let local = make_work(std::thread::current().id());
    let other = make_work(other_owner);
    let local_address = (&*local) as *const AsyncWork as usize;
    let other_address = (&*other) as *const AsyncWork as usize;
    {
        let mut ready = ready_events().lock().unwrap();
        assert!(ready.is_empty());
        ready.push_back(ReadyEvent::AsyncCompletion(other_address));
        ready.push_back(ReadyEvent::AsyncCompletion(local_address));
    }
    assert_eq!(take_ready_event_for_current_thread(), Some(ReadyEvent::AsyncCompletion(local_address)));
    assert_eq!(ready_events().lock().unwrap().pop_front(), Some(ReadyEvent::AsyncCompletion(other_address)));
}

#[test]
fn async_sources_share_readiness_order() {
    let _guard = lock_async_test();
    let mut ready = ready_events()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    ready.clear();
    ready.push_back(ReadyEvent::AsyncCompletion(1));
    ready.push_back(ReadyEvent::ThreadsafeFunction(2));
    ready.push_back(ReadyEvent::AsyncCompletion(3));
    assert_eq!(ready.pop_front(), Some(ReadyEvent::AsyncCompletion(1)));
    assert_eq!(ready.pop_front(), Some(ReadyEvent::ThreadsafeFunction(2)));
    assert_eq!(ready.pop_front(), Some(ReadyEvent::AsyncCompletion(3)));
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

unsafe extern "C" fn ordered_finalizer_uses_napi(
    env: NapiEnv,
    data: *mut c_void,
    _hint: *mut c_void,
) {
    let mut value = ptr::null_mut();
    assert_eq!(napi_create_int32(env, 41, &mut value), NAPI_OK);
    assert!(value_belongs_to_environment(env, value));
    cleanup_probe(data);
}

unsafe extern "C" fn remove_cleanup_and_reenter(
    handle: *mut AsyncCleanupHookHandle,
    data: *mut c_void,
) {
    let env = (*handle).env;
    assert_eq!(napi_remove_async_cleanup_hook(handle), NAPI_OK);
    retire_owned_envs();
    assert!(HOST.with(|host| {
        let host = host.borrow();
        host.module_envs.iter().chain(host.pending_call_envs.iter())
            .any(|entry| (&**entry as *const Env as usize) == env)
    }));
    cleanup_probe(data);
}

unsafe extern "C" fn cleanup_finalizer_rejects_new_async_roots(
    env: NapiEnv,
    _data: *mut c_void,
    _hint: *mut c_void,
) {
    let mut work = ptr::null_mut();
    assert_eq!(napi_create_async_work(env, ptr::null_mut(), ptr::null_mut(),
        Some(noop_execute), None, ptr::null_mut(), &mut work), NAPI_CLOSING);
    let existing = (&mut *(*env).async_works[0]) as *mut AsyncWork;
    assert_eq!(napi_queue_async_work(env, existing), NAPI_CLOSING);
    assert_eq!(napi_add_env_cleanup_hook(env, Some(cleanup_probe), ptr::null_mut()), NAPI_CLOSING);
}

unsafe extern "C" fn queue_work_during_async_cleanup(
    handle: *mut AsyncCleanupHookHandle,
    _data: *mut c_void,
) {
    let env = (*handle).env as NapiEnv;
    let mut work = ptr::null_mut();
    assert_eq!(napi_create_async_work(env, ptr::null_mut(), ptr::null_mut(),
        Some(noop_execute), None, ptr::null_mut(), &mut work), NAPI_OK);
    assert_eq!(napi_queue_async_work(env, work), NAPI_OK);
    assert_eq!(napi_remove_async_cleanup_hook(handle), NAPI_OK);
}

unsafe extern "C" fn nested_unload_before_cleanup_completion(
    _env: NapiEnv,
    data: *mut c_void,
    _hint: *mut c_void,
) {
    assert_eq!(thaw_napi_unload_all(), 0);
    assert_eq!(napi_remove_async_cleanup_hook(data as *mut AsyncCleanupHookHandle), NAPI_OK);
}

static CALLED_AFTER_FINALIZATION: AtomicUsize = AtomicUsize::new(0);

unsafe extern "C" fn counted_native_callback(_env: NapiEnv, _info: NapiCallbackInfo) -> NapiValue {
    CALLED_AFTER_FINALIZATION.fetch_add(1, Ordering::AcqRel);
    ptr::null_mut()
}

unsafe extern "C" fn release_callback_data(
    _env: NapiEnv, data: *mut c_void, _hint: *mut c_void,
) {
    drop(Box::from_raw(data.cast::<u8>()));
}

unsafe extern "C" fn release_shared_backing(data: *mut c_void, _hint: *mut c_void) {
    drop(Box::from_raw(data.cast::<u8>()));
}

struct RetiredExternalProbe {
    prior_env: usize,
    buffer: usize,
    arraybuffer: usize,
    view: usize,
    shared: usize,
    shared_view: usize,
    empty_shared: usize,
}

unsafe extern "C" fn inspect_external_after_release(
    env: NapiEnv, data: *mut c_void, _hint: *mut c_void,
) {
    let handles = Box::from_raw(data.cast::<[usize; 2]>());
    let mut bytes = ptr::null_mut();
    let mut length = usize::MAX;
    assert_eq!(napi_get_buffer_info(env, handles[0] as NapiValue,
        &mut bytes, &mut length), NAPI_OK);
    assert!(bytes.is_null());
    assert_eq!(length, 0);
    assert_eq!(napi_get_arraybuffer_info(env, handles[1] as NapiValue,
        &mut bytes, &mut length), NAPI_OK);
    assert!(bytes.is_null());
    assert_eq!(length, 0);
}

unsafe extern "C" fn inspect_retired_external_values(
    env: NapiEnv, data: *mut c_void, _hint: *mut c_void,
) {
    let probe = Box::from_raw(data.cast::<RetiredExternalProbe>());
    let buffer = probe.buffer as NapiValue;
    let arraybuffer = probe.arraybuffer as NapiValue;
    let mut bytes = ptr::null_mut();
    let mut length = 0;
    assert_eq!(napi_get_buffer_info(env, buffer, &mut bytes, &mut length), NAPI_INVALID_ARG);
    assert_eq!(napi_get_buffer_info(probe.prior_env as NapiEnv, buffer,
        &mut bytes, &mut length), NAPI_INVALID_ARG);
    assert_eq!(napi_get_arraybuffer_info(env, arraybuffer,
        &mut bytes, &mut length), NAPI_INVALID_ARG);
    assert_eq!(json_from_value_with_undefined(buffer, true).unwrap(),
        serde_json::json!({ "type": "Buffer", "data": [] }));
    assert_eq!(json_from_value_with_undefined(probe.view as NapiValue, true).unwrap(),
        serde_json::json!({ "type": "Buffer", "data": [] }));
    assert_eq!(arraybuffer_parts(arraybuffer).unwrap(), (ptr::null_mut(), 0, true));
    assert_eq!(arraybuffer_parts(probe.shared as NapiValue).unwrap(),
        (ptr::null_mut(), 0, true));
    assert_eq!(arraybuffer_parts(probe.empty_shared as NapiValue).unwrap(),
        (ptr::null_mut(), 0, true));
    assert_eq!(json_from_value_with_undefined(probe.shared_view as NapiValue, true).unwrap(),
        serde_json::json!({ "type": "Buffer", "data": [] }));
    #[cfg(feature = "quickjs")]
    assert!(quickjs_bridge_value(buffer).is_err());
}

unsafe extern "C" fn queue_other_env_work_from_finalizer(
    _env: NapiEnv, data: *mut c_void, _hint: *mut c_void,
) {
    let other = data as NapiEnv;
    let mut work = ptr::null_mut();
    assert_eq!(napi_create_async_work(other, ptr::null_mut(), ptr::null_mut(),
        Some(noop_execute), None, ptr::null_mut(), &mut work), NAPI_OK);
    assert_eq!(napi_queue_async_work(other, work), NAPI_OK);
}

struct CrossEnvFinalizerProbe {
    env: usize,
    function: usize,
    this_arg: usize,
}

unsafe extern "C" fn call_prior_env_from_later_finalizer(
    _env: NapiEnv, data: *mut c_void, _hint: *mut c_void,
) {
    let probe = Box::from_raw(data as *mut CrossEnvFinalizerProbe);
    assert!(!HOST.with(|host| host.borrow().exports.contains_key("cleanup_prior_export")));
    // Both entrypoints reject a finalized Env in `env_mut` before their
    // operation-specific closing paths; no native callback may execute.
    assert_eq!(napi_call_function(probe.env as NapiEnv, probe.this_arg as NapiValue,
        probe.function as NapiValue, 0, ptr::null(), ptr::null_mut()), NAPI_INVALID_ARG);
    assert_eq!(node_api_post_finalizer(probe.env as NapiEnv,
        Some(nested_unload_in_posted_finalizer), ptr::null_mut(), ptr::null_mut()), NAPI_INVALID_ARG);
    assert_eq!(CALLED_AFTER_FINALIZATION.load(Ordering::Acquire), 0);
}

unsafe extern "C" fn nested_unload_in_async_completion(
    _env: NapiEnv, _status: NapiStatus, _data: *mut c_void,
) {
    assert_eq!(thaw_napi_unload_all(), 0);
}

unsafe extern "C" fn nested_unload_in_posted_finalizer(
    _env: NapiEnv, _data: *mut c_void, _hint: *mut c_void,
) {
    assert_eq!(thaw_napi_unload_all(), 0);
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

unsafe extern "C" fn noop_execute(_env: NapiEnv, _data: *mut c_void) {}

unsafe extern "C" fn probe_complete(_env: NapiEnv, status: NapiStatus, data: *mut c_void) {
    assert_eq!(status, NAPI_OK);
    let probe = &*(data as *const AsyncProbe);
    assert!(probe.executed.load(Ordering::Acquire));
    *probe.complete_thread.lock().unwrap() = Some(std::thread::current().id());
}

unsafe extern "C" fn resolve_promise_complete(
    env: NapiEnv,
    status: NapiStatus,
    data: *mut c_void,
) {
    assert_eq!(status, NAPI_OK);
    let mut value = ptr::null_mut();
    assert_eq!(napi_get_undefined(env, &mut value), NAPI_OK);
    assert_eq!(napi_resolve_deferred(env, data.cast(), value), NAPI_OK);
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
fn threadsafe_abort_finalizes_then_accepts_remaining_owner_releases() {
    let _guard = lock_async_test();
    unsafe extern "C" fn finalize(env: NapiEnv, data: *mut c_void, _hint: *mut c_void) {
        let finalizations = &*(data as *const AtomicUsize);
        finalizations.fetch_add(1, Ordering::AcqRel);
        let mut undefined = ptr::null_mut();
        assert_eq!(napi_get_undefined(env, &mut undefined), NAPI_OK);
    }
    unsafe extern "C" fn call_js(
        _env: NapiEnv,
        _function: NapiValue,
        _context: *mut c_void,
        _data: *mut c_void,
    ) {
    }
    let mut env = Env::new();
    let finalizations = AtomicUsize::new(0);
    let mut function = ptr::null_mut();
    unsafe {
        assert_eq!(
            napi_create_threadsafe_function(
                &mut env,
                ptr::null_mut(),
                ptr::null_mut(),
                ptr::null_mut(),
                0,
                3,
                (&finalizations as *const AtomicUsize).cast_mut().cast(),
                Some(finalize),
                ptr::null_mut(),
                Some(call_js),
                &mut function,
            ),
            NAPI_OK
        );
    }
    unsafe {
        assert_eq!(napi_release_threadsafe_function(function, 1), NAPI_OK);
        assert_eq!(napi_acquire_threadsafe_function(function), NAPI_CLOSING);
    }
    thaw_napi_poll_async_work();
    assert_eq!(finalizations.load(Ordering::Acquire), 1);
    unsafe { assert_eq!(napi_release_threadsafe_function(function, 0), NAPI_OK); }
    thaw_napi_poll_async_work();
    assert_eq!(finalizations.load(Ordering::Acquire), 1);
    unsafe { assert_eq!(napi_release_threadsafe_function(function, 0), NAPI_OK); }
    thaw_napi_poll_async_work();
    assert_eq!(finalizations.load(Ordering::Acquire), 1);
    thaw_napi_poll_async_work();
    assert_eq!(finalizations.load(Ordering::Acquire), 1);

    let mut closing_call = ptr::null_mut();
    unsafe {
        assert_eq!(
            napi_create_threadsafe_function(
                &mut env,
                ptr::null_mut(),
                ptr::null_mut(),
                ptr::null_mut(),
                0,
                2,
                (&finalizations as *const AtomicUsize).cast_mut().cast(),
                Some(finalize),
                ptr::null_mut(),
                Some(call_js),
                &mut closing_call,
            ),
            NAPI_OK
        );
    }
    unsafe { assert_eq!(napi_release_threadsafe_function(closing_call, 1), NAPI_OK); }
    thaw_napi_poll_async_work();
    assert_eq!(finalizations.load(Ordering::Acquire), 2);
    unsafe {
        assert_eq!(
            napi_call_threadsafe_function(closing_call, ptr::null_mut(), 0),
            NAPI_CLOSING
        );
    }
    thaw_napi_poll_async_work();
    assert_eq!(finalizations.load(Ordering::Acquire), 2);
}

#[test]
fn threadsafe_finalization_waits_for_reentrant_callback_to_return() {
    let _guard = lock_async_test();
    struct ReentryProbe {
        function: AtomicUsize,
        finalizations: AtomicUsize,
    }
    unsafe extern "C" fn call_js(
        _env: NapiEnv,
        _function: NapiValue,
        context: *mut c_void,
        _data: *mut c_void,
    ) {
        let probe = &*(context as *const ReentryProbe);
        let function = probe.function.load(Ordering::Acquire) as *mut ThreadsafeFunction;
        assert_eq!(napi_release_threadsafe_function(function, 0), NAPI_OK);
        thaw_napi_poll_async_work();
        assert_eq!(probe.finalizations.load(Ordering::Acquire), 0);
    }
    unsafe extern "C" fn finalize(_env: NapiEnv, data: *mut c_void, _hint: *mut c_void) {
        let probe = &*(data as *const ReentryProbe);
        probe.finalizations.fetch_add(1, Ordering::AcqRel);
    }
    let mut env = Env::new();
    let probe = ReentryProbe {
        function: AtomicUsize::new(0),
        finalizations: AtomicUsize::new(0),
    };
    let mut function = ptr::null_mut();
    unsafe {
        assert_eq!(
            napi_create_threadsafe_function(
                &mut env,
                ptr::null_mut(),
                ptr::null_mut(),
                ptr::null_mut(),
                0,
                1,
                (&probe as *const ReentryProbe).cast_mut().cast(),
                Some(finalize),
                (&probe as *const ReentryProbe).cast_mut().cast(),
                Some(call_js),
                &mut function,
            ),
            NAPI_OK
        );
    }
    probe.function.store(function as usize, Ordering::Release);
    unsafe {
        assert_eq!(napi_call_threadsafe_function(function, ptr::null_mut(), 0), NAPI_OK);
    }
    thaw_napi_poll_async_work();
    assert_eq!(probe.finalizations.load(Ordering::Acquire), 1);
    thaw_napi_poll_async_work();
    assert_eq!(probe.finalizations.load(Ordering::Acquire), 1);
}

#[test]
fn threadsafe_finalizer_and_remaining_owners_keep_module_env_live() {
    let _guard = lock_async_test();
    struct UnloadProbe {
        finalizations: AtomicUsize,
    }
    unsafe extern "C" fn finalize(env: NapiEnv, data: *mut c_void, _hint: *mut c_void) {
        let probe = &*(data as *const UnloadProbe);
        let mut undefined = ptr::null_mut();
        assert_eq!(napi_get_undefined(env, &mut undefined), NAPI_OK);
        assert_eq!(thaw_napi_unload_all(), 0);
        probe.finalizations.fetch_add(1, Ordering::AcqRel);
    }
    unsafe extern "C" fn call_js(
        _env: NapiEnv,
        _function: NapiValue,
        _context: *mut c_void,
        _data: *mut c_void,
    ) {
    }
    let mut env = Box::new(Env::new());
    let env_ptr = (&mut *env) as NapiEnv;
    let probe = UnloadProbe {
        finalizations: AtomicUsize::new(0),
    };
    let mut function = ptr::null_mut();
    unsafe {
        assert_eq!(
            napi_create_threadsafe_function(
                env_ptr,
                ptr::null_mut(),
                ptr::null_mut(),
                ptr::null_mut(),
                0,
                2,
                (&probe as *const UnloadProbe).cast_mut().cast(),
                Some(finalize),
                ptr::null_mut(),
                Some(call_js),
                &mut function,
            ),
            NAPI_OK
        );
    }
    HOST.with(|host| host.borrow_mut().module_envs.push(env));
    unsafe { assert_eq!(napi_release_threadsafe_function(function, 1), NAPI_OK); }
    thaw_napi_poll_async_work();
    assert_eq!(probe.finalizations.load(Ordering::Acquire), 1);
    assert_eq!(thaw_napi_unload_all(), 0);
    let unrelated_pending = unsafe {
        (&mut *env_ptr).alloc(Value::Promise(Rc::new(RefCell::new(PromiseState::Pending))))
    };
    assert_eq!(
        wait_for_promise(unrelated_pending).unwrap_err(),
        "native addon returned a Promise with no pending work"
    );
    unsafe { assert_eq!(napi_release_threadsafe_function(function, 0), NAPI_OK); }
    assert_eq!(thaw_napi_unload_all(), 1);
}


#[test]
fn method_with_callback_can_stop_its_own_pending_async_work() {
    unsafe extern "C" fn stop_work(_env: NapiEnv, info: NapiCallbackInfo) -> NapiValue {
        let gate = &*((*info).data as *const Arc<WorkerGate>);
        let mut state = gate.state.lock().unwrap();
        state.1 = true;
        gate.changed.notify_all();
        ptr::null_mut()
    }
    unsafe extern "C" fn unused_callback(
        _context: *mut c_void,
        _error: *const c_char,
        _result: *const c_char,
    ) {
        panic!("stop method must not invoke its callback");
    }

    let _guard = lock_async_test();
    let gate = Arc::new(WorkerGate {
        state: Mutex::new((0, false)),
        changed: Condvar::new(),
    });
    let gate_data = Box::into_raw(Box::new(Arc::clone(&gate)));
    let mut env = Box::new(Env::new());
    let env_ptr: NapiEnv = &mut *env;
    let method = env.alloc(Value::Function(Function {
        callback: stop_work,
        data: gate_data.cast(),
        properties: HashMap::new(),
        _thaw_bridge: None,
    }));
    let receiver = env.alloc(Value::Object(HashMap::from([(
        PropertyKey::String("stop".into()), method,
    )])));
    HOST.with(|host| host.borrow_mut().module_envs.push(env));
    let mut work = ptr::null_mut();
    unsafe {
        assert_eq!(napi_create_async_work(
            env_ptr, ptr::null_mut(), ptr::null_mut(),
            Some(blocking_execute), None, gate_data.cast(), &mut work,
        ), NAPI_OK);
        assert_eq!(napi_queue_async_work(env_ptr, work), NAPI_OK);
    }
    let mut state = gate.state.lock().unwrap();
    while state.0 == 0 {
        state = gate.changed.wait(state).unwrap();
    }
    drop(state);
    unsafe {
        let result = thaw_napi_call_method_with_callback_result(
            receiver as u64, c"stop".as_ptr(), c"[]".as_ptr(),
            Some(unused_callback), ptr::null_mut(), 1,
        );
        assert!(result.error.is_null());
        assert_eq!(CStr::from_ptr(result.value).to_str().unwrap(), "null");
        assert!(gate.state.lock().unwrap().1);
        assert_eq!(thaw_napi_run_async_work(), 1);
        assert_eq!(napi_delete_async_work(env_ptr, work), NAPI_OK);
        HOST.with(|host| {
            host.borrow_mut().module_envs.retain(|entry| (&**entry as *const Env).cast_mut() != env_ptr);
        });
        drop(Box::from_raw(gate_data));
    }
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
fn removed_async_cleanup_handle_rejects_removal_after_environment_drop() {
    unsafe extern "C" fn cleanup(_handle: *mut AsyncCleanupHookHandle, _data: *mut c_void) {}
    let mut env = Box::new(Env::new());
    env.host_managed = true;
    let mut handle = ptr::null_mut();
    unsafe {
        assert_eq!(napi_add_async_cleanup_hook(&mut *env, Some(cleanup), ptr::null_mut(), &mut handle), NAPI_OK);
        assert_eq!(napi_remove_async_cleanup_hook(handle), NAPI_OK);
    }
    drop(env);
    // Run under a memory checker to catch accesses through the retired Env.
    unsafe { assert_eq!(napi_remove_async_cleanup_hook(handle), NAPI_INVALID_ARG); }
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
    env.host_managed = true; // Synchronous internal hook-order fixture.
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
fn direct_environment_rejects_async_cleanup_without_pinned_owner() {
    let _guard = lock_async_test();
    HELD_ASYNC_CLEANUP.store(0, Ordering::Release);
    let mut env = Env::new();
    unsafe {
        assert_eq!(napi_add_async_cleanup_hook(&mut env, Some(hold_async_cleanup),
            ptr::null_mut(), ptr::null_mut()), NAPI_CLOSING);
    }
    drop(env);
    assert_eq!(HELD_ASYNC_CLEANUP.load(Ordering::Acquire), 0);
}

#[test]
fn foreign_remove_and_owner_begin_serialize_async_cleanup_handle() {
    unsafe extern "C" fn counted(_handle: *mut AsyncCleanupHookHandle, data: *mut c_void) {
        (*(data as *const AtomicUsize)).fetch_add(1, Ordering::AcqRel);
    }
    let _guard = lock_async_test();
    let mut env = Box::new(Env::new());
    env.host_managed = true;
    let env_ptr = &mut *env as NapiEnv;
    let calls = Box::into_raw(Box::new(AtomicUsize::new(0)));
    let mut handle = ptr::null_mut();
    unsafe {
        assert_eq!(napi_add_async_cleanup_hook(env_ptr, Some(counted), calls.cast(),
            &mut handle), NAPI_OK);
    }
    let barrier = Arc::new(std::sync::Barrier::new(2));
    let other_barrier = Arc::clone(&barrier);
    let handle_address = handle as usize;
    let remover = std::thread::spawn(move || {
        other_barrier.wait();
        unsafe { napi_remove_async_cleanup_hook(handle_address as *mut AsyncCleanupHookHandle) }
    });
    barrier.wait();
    unsafe { Env::begin_async_cleanup(env_ptr, false) };
    assert_eq!(remover.join().unwrap(), NAPI_OK);
    assert!(unsafe { (*calls).load(Ordering::Acquire) } <= 1);
    assert_eq!(ACTIVE_ASYNC_CLEANUP_HOOKS.load(Ordering::Acquire), 0);
    unsafe { drop(Box::from_raw(calls)) };
    drop(env);
}

#[test]
fn pending_async_cleanup_keeps_env_alive_until_completion_and_finalizers() {
    let _guard = lock_async_test();
    HELD_ASYNC_CLEANUP.store(0, Ordering::Release);
    let output = Arc::new(Mutex::new(Vec::new()));
    let mut env = Box::new(Env::new());
    env.host_managed = true;
    let env_ptr = &mut *env as NapiEnv;
    unsafe {
        assert_eq!(napi_add_async_cleanup_hook(env_ptr, Some(hold_async_cleanup),
            ptr::null_mut(), ptr::null_mut()), NAPI_OK);
    }
    env.cleanup_hooks.push(CleanupHookRecord {
        hook: cleanup_probe,
        data: Box::into_raw(Box::new(CleanupProbe { output: Arc::clone(&output), value: 2 })).cast(),
    });
    env.finalizers.push(FinalizeRecord {
        data: Box::into_raw(Box::new(CleanupProbe { output: Arc::clone(&output), value: 3 })).cast(),
        finalize: Some(ordered_finalizer_uses_napi),
        hint: ptr::null_mut(),
        backing: ptr::null_mut(),
    });
    HOST.with(|host| host.borrow_mut().pending_call_envs.push(env));
    retire_owned_envs();
    assert!(output.lock().unwrap().is_empty());
    assert!(HOST.with(|host| host.borrow().pending_call_envs.iter()
        .any(|entry| (&**entry as *const Env).cast_mut() == env_ptr)));
    let handle = HELD_ASYNC_CLEANUP.swap(0, Ordering::AcqRel) as *mut AsyncCleanupHookHandle;
    assert!(!handle.is_null());
    unsafe { assert_eq!(napi_remove_async_cleanup_hook(handle), NAPI_OK); }
    retire_owned_envs();
    assert_eq!(*output.lock().unwrap(), vec![2, 3]);
    assert!(HOST.with(|host| host.borrow().pending_call_envs.iter()
        .any(|entry| (&**entry as *const Env).cast_mut() == env_ptr && entry.finalized)));
    unsafe { assert_eq!(napi_remove_async_cleanup_hook(handle), NAPI_INVALID_ARG); }
    let retained = HOST.with(|host| host.borrow_mut().pending_call_envs.pop().unwrap());
    drop(retained);
}

#[test]
fn synchronous_async_cleanup_reentry_cannot_drop_dispatching_env() {
    let _guard = lock_async_test();
    let output = Arc::new(Mutex::new(Vec::new()));
    let mut env = Box::new(Env::new());
    env.host_managed = true;
    let env_ptr = &mut *env as NapiEnv;
    unsafe {
        assert_eq!(napi_add_async_cleanup_hook(env_ptr, Some(remove_cleanup_and_reenter),
            Box::into_raw(Box::new(CleanupProbe { output: Arc::clone(&output), value: 1 })).cast(),
            ptr::null_mut()), NAPI_OK);
    }
    env.cleanup_hooks.push(CleanupHookRecord {
        hook: cleanup_probe,
        data: Box::into_raw(Box::new(CleanupProbe { output: Arc::clone(&output), value: 2 })).cast(),
    });
    env.shutdown_requested = true;
    HOST.with(|host| host.borrow_mut().module_envs.push(env));
    retire_owned_envs();
    assert_eq!(*output.lock().unwrap(), vec![1, 2]);
    assert!(HOST.with(|host| host.borrow().module_envs.iter()
        .any(|entry| (&**entry as *const Env).cast_mut() == env_ptr && entry.finalized)));
    let retained = HOST.with(|host| host.borrow_mut().module_envs.pop().unwrap());
    drop(retained);
}

#[test]
fn finalizer_cannot_queue_new_async_work_or_late_cleanup_hook() {
    let _guard = lock_async_test();
    let mut env = Box::new(Env::new());
    let env_ptr = &mut *env as NapiEnv;
    let mut work = ptr::null_mut();
    unsafe {
        assert_eq!(napi_create_async_work(env_ptr, ptr::null_mut(), ptr::null_mut(),
            Some(noop_execute), None, ptr::null_mut(), &mut work), NAPI_OK);
    }
    env.finalizers.push(FinalizeRecord {
        data: ptr::null_mut(), finalize: Some(cleanup_finalizer_rejects_new_async_roots),
        hint: ptr::null_mut(),
        backing: ptr::null_mut(),
    });
    env.shutdown_requested = true;
    HOST.with(|host| host.borrow_mut().module_envs.push(env));
    retire_owned_envs();
    assert!(HOST.with(|host| host.borrow().module_envs.iter()
        .any(|entry| (&**entry as *const Env).cast_mut() == env_ptr && entry.finalized)));
    let retained = HOST.with(|host| host.borrow_mut().module_envs.pop().unwrap());
    drop(retained);
}

#[test]
fn unload_waits_for_work_queued_by_async_cleanup_hook() {
    let _guard = lock_async_test();
    let mut env = Box::new(Env::new());
    env.host_managed = true;
    let env_ptr = &mut *env as NapiEnv;
    unsafe {
        assert_eq!(napi_add_async_cleanup_hook(env_ptr,
            Some(queue_work_during_async_cleanup), ptr::null_mut(), ptr::null_mut()), NAPI_OK);
    }
    HOST.with(|host| host.borrow_mut().module_envs.push(env));
    assert_eq!(thaw_napi_unload_all(), 1);
    assert!(!HOST.with(|host| host.borrow().module_envs.iter()
        .any(|entry| (&**entry as *const Env).cast_mut() == env_ptr)));
}

#[test]
fn unload_guard_precedes_wait_for_an_already_active_cleanup_hook() {
    let _guard = lock_async_test();
    HELD_ASYNC_CLEANUP.store(0, Ordering::Release);
    let mut env = Box::new(Env::new());
    env.host_managed = true;
    let env_ptr = &mut *env as NapiEnv;
    unsafe {
        assert_eq!(napi_add_async_cleanup_hook(env_ptr, Some(hold_async_cleanup),
            ptr::null_mut(), ptr::null_mut()), NAPI_OK);
    }
    env.shutdown_requested = true;
    HOST.with(|host| host.borrow_mut().module_envs.push(env));
    retire_owned_envs();
    let handle = HELD_ASYNC_CLEANUP.swap(0, Ordering::AcqRel) as *mut AsyncCleanupHookHandle;
    assert!(!handle.is_null());
    unsafe {
        assert_eq!(node_api_post_finalizer(env_ptr, Some(nested_unload_before_cleanup_completion),
            handle.cast(), ptr::null_mut()), NAPI_OK);
    }
    assert_eq!(thaw_napi_unload_all(), 1);
    assert!(!HOST.with(|host| host.borrow().module_envs.iter()
        .any(|entry| (&**entry as *const Env).cast_mut() == env_ptr)));
}

#[test]
fn later_finalizer_cannot_dispatch_prior_env_callback_data() {
    let _guard = lock_async_test();
    CALLED_AFTER_FINALIZATION.store(0, Ordering::Release);
    let mut first = Box::new(Env::new());
    let first_env = &mut *first as NapiEnv;
    let this_arg = first.alloc(Value::Undefined);
    let callback_data = Box::into_raw(Box::new(7_u8)).cast::<c_void>();
    let function = first.alloc(Value::Function(Function {
        callback: counted_native_callback, data: callback_data,
        properties: HashMap::new(), _thaw_bridge: None,
    }));
    first.finalizers.push(FinalizeRecord {
        data: callback_data, finalize: Some(release_callback_data), hint: ptr::null_mut(),
        backing: ptr::null_mut(),
    });
    first.shutdown_requested = true;
    let mut second = Box::new(Env::new());
    second.shutdown_requested = true;
    second.finalizers.push(FinalizeRecord {
        data: Box::into_raw(Box::new(CrossEnvFinalizerProbe {
            env: first_env as usize, function: function as usize,
            this_arg: this_arg as usize,
        })).cast(),
        finalize: Some(call_prior_env_from_later_finalizer), hint: ptr::null_mut(),
        backing: ptr::null_mut(),
    });
    HOST.with(|host| {
        let mut host = host.borrow_mut();
        host.functions.insert("cleanup_prior_export".into(), Function {
            callback: counted_native_callback, data: callback_data,
            properties: HashMap::new(), _thaw_bridge: None,
        });
        host.exports.insert("cleanup_prior_export".into(), (first_env as usize, function));
        host.module_envs.extend([first, second]);
    });
    retire_owned_envs();
    assert_eq!(CALLED_AFTER_FINALIZATION.load(Ordering::Acquire), 0);
    let retained = HOST.with(|host| host.borrow_mut().module_envs.drain(..).collect::<Vec<_>>());
    drop(retained);
}

#[test]
fn later_finalizer_cannot_read_prior_env_external_backing_store() {
    let _guard = lock_async_test();
    let mut first = Box::new(Env::new());
    let first_env = &mut *first as NapiEnv;
    let buffer_data = Box::into_raw(Box::new(7_u8));
    let array_data = Box::into_raw(Box::new(9_u8));
    let shared_data = Box::into_raw(Box::new(11_u8));
    let buffer = first.alloc(Value::ExternalBuffer { data: buffer_data, length: 1 });
    let arraybuffer = first.alloc(Value::ExternalArrayBuffer {
        data: array_data, length: 1, detached: false,
    });
    let shared = first.alloc(Value::ExternalSharedArrayBuffer {
        data: shared_data, length: 1, retired: false,
    });
    let empty_shared = first.alloc(Value::ExternalSharedArrayBuffer {
        data: ptr::null_mut(), length: 0, retired: false,
    });
    assert_eq!(unsafe { arraybuffer_parts(empty_shared).unwrap() },
        (ptr::null_mut(), 0, false));
    let empty_view = first.alloc(Value::BufferView {
        array_buffer: empty_shared, byte_offset: 0, length: 0,
    });
    let mut empty_data = ptr::null_mut();
    let mut empty_length = usize::MAX;
    unsafe {
        assert_eq!(napi_get_buffer_info(first_env, empty_view,
            &mut empty_data, &mut empty_length), NAPI_OK);
    }
    assert!(empty_data.is_null());
    assert_eq!(empty_length, 0);
    first.finalizers.extend([(buffer_data, buffer), (array_data, arraybuffer)]
        .into_iter().map(|(data, backing)| FinalizeRecord {
            data: data.cast(), finalize: Some(release_callback_data), hint: ptr::null_mut(),
            backing,
        }));
    first.finalizers.push(FinalizeRecord {
        data: Box::into_raw(Box::new([buffer as usize, arraybuffer as usize])).cast(),
        finalize: Some(inspect_external_after_release), hint: ptr::null_mut(),
        backing: ptr::null_mut(),
    });
    first.noenv_finalizers.push(NoEnvFinalizeRecord {
        data: shared_data.cast(), finalize: Some(release_shared_backing),
        hint: ptr::null_mut(), backing: shared,
    });
    first.shutdown_requested = true;
    let mut second = Box::new(Env::new());
    let view = second.alloc(Value::BufferView {
        array_buffer: arraybuffer, byte_offset: 0, length: 1,
    });
    let shared_view = second.alloc(Value::BufferView {
        array_buffer: shared, byte_offset: 0, length: 1,
    });
    second.finalizers.push(FinalizeRecord {
        data: Box::into_raw(Box::new(RetiredExternalProbe {
            prior_env: first_env as usize, buffer: buffer as usize,
            arraybuffer: arraybuffer as usize, view: view as usize,
            shared: shared as usize, shared_view: shared_view as usize,
            empty_shared: empty_shared as usize,
        })).cast(),
        finalize: Some(inspect_retired_external_values), hint: ptr::null_mut(),
        backing: ptr::null_mut(),
    });
    second.shutdown_requested = true;
    HOST.with(|host| host.borrow_mut().module_envs.extend([first, second]));
    retire_owned_envs();
    let retained = HOST.with(|host| host.borrow_mut().module_envs.drain(..).collect::<Vec<_>>());
    drop(retained);
}

#[test]
fn finalizer_queued_work_keeps_other_env_owned_until_completion() {
    let _guard = lock_async_test();
    let mut first = Box::new(Env::new());
    first.shutdown_requested = true;
    let mut second = Box::new(Env::new());
    let second_env = &mut *second as NapiEnv;
    second.shutdown_requested = true;
    first.finalizers.push(FinalizeRecord {
        data: second_env.cast(), finalize: Some(queue_other_env_work_from_finalizer),
        hint: ptr::null_mut(),
        backing: ptr::null_mut(),
    });
    HOST.with(|host| host.borrow_mut().module_envs.extend([first, second]));
    retire_owned_envs();
    assert!(HOST.with(|host| host.borrow().module_envs.iter()
        .any(|env| (&**env as *const Env).cast_mut() == second_env && !env.finalized)));
    assert_eq!(thaw_napi_run_async_work(), 1);
    assert!(HOST.with(|host| host.borrow().module_envs.iter()
        .any(|env| (&**env as *const Env).cast_mut() == second_env && env.finalized)));
    let retained = HOST.with(|host| host.borrow_mut().module_envs.drain(..).collect::<Vec<_>>());
    drop(retained);
}

#[test]
fn nested_unload_cannot_retire_async_completion_stack() {
    let _guard = lock_async_test();
    let mut env = Box::new(Env::new());
    let env_ptr = &mut *env as NapiEnv;
    HOST.with(|host| host.borrow_mut().module_envs.push(env));
    let mut work = ptr::null_mut();
    unsafe {
        assert_eq!(napi_create_async_work(env_ptr, ptr::null_mut(), ptr::null_mut(),
            Some(noop_execute), Some(nested_unload_in_async_completion),
            ptr::null_mut(), &mut work), NAPI_OK);
        assert_eq!(napi_queue_async_work(env_ptr, work), NAPI_OK);
    }
    assert_eq!(thaw_napi_run_async_work(), 1);
    assert_eq!(thaw_napi_unload_all(), 1);
}

#[test]
fn nested_unload_cannot_retire_posted_finalizer_stack() {
    let _guard = lock_async_test();
    let mut env = Box::new(Env::new());
    let env_ptr = &mut *env as NapiEnv;
    HOST.with(|host| host.borrow_mut().module_envs.push(env));
    unsafe {
        assert_eq!(node_api_post_finalizer(env_ptr, Some(nested_unload_in_posted_finalizer),
            ptr::null_mut(), ptr::null_mut()), NAPI_OK);
    }
    assert_eq!(thaw_napi_poll_async_work(), 1);
    assert_eq!(thaw_napi_unload_all(), 1);
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
fn native_promise_can_settle_on_the_final_async_completion() {
    let _guard = lock_async_test();
    let mut env = Env::new();
    let mut deferred = ptr::null_mut();
    let mut promise = ptr::null_mut();
    let mut work = ptr::null_mut();
    unsafe {
        assert_eq!(napi_create_promise(&mut env, &mut deferred, &mut promise), NAPI_OK);
        assert_eq!(
            napi_create_async_work(
                &mut env,
                ptr::null_mut(),
                ptr::null_mut(),
                Some(noop_execute),
                Some(resolve_promise_complete),
                deferred.cast(),
                &mut work,
            ),
            NAPI_OK
        );
        assert_eq!(napi_queue_async_work(&mut env, work), NAPI_OK);
    }
    assert!(matches!(wait_for_promise(promise), Ok(value) if unsafe {
        matches!(value_ref(value), Ok(Value::Undefined))
    }));
    unsafe {
        assert_eq!(napi_delete_async_work(&mut env, work), NAPI_OK);
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
        let undefined_name = env.alloc(Value::Undefined);
        let mut work = ptr::null_mut();
        assert_eq!(
            napi_create_async_work(
                env_ptr,
                ptr::null_mut(),
                undefined_name,
                Some(noop_execute),
                None,
                ptr::null_mut(),
                &mut work
            ),
            NAPI_OK
        );
        assert_eq!(napi_delete_async_work(env_ptr, work), NAPI_OK);
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

unsafe extern "C" fn phased_shutdown_records_callback_error(
    env: NapiEnv, _data: *mut c_void, _hint: *mut c_void,
) {
    let env = &mut *env;
    let error = env.alloc(Value::Error("cleanup failure".into()));
    env.exception = Some(error);
}

#[test]
fn registered_async_cleanup_hook_does_not_block_normal_work_drain() {
    let _guard = lock_async_test();
    let output = Arc::new(Mutex::new(Vec::new()));
    let mut env = Box::new(Env::new());
    env.host_managed = true;
    unsafe {
        assert_eq!(napi_add_async_cleanup_hook(&mut *env, Some(async_cleanup_probe),
            Box::into_raw(Box::new(CleanupProbe {
                output: Arc::clone(&output), value: 1,
            })).cast(), ptr::null_mut()), NAPI_OK);
    }
    HOST.with(|host| host.borrow_mut().module_envs.push(env));
    assert!(!HOST.with(|host| host.borrow().has_active_cleanup()));
    assert_eq!(thaw_napi_run_async_work(), 0);
    assert!(output.lock().unwrap().is_empty());
    assert_eq!(thaw_napi_begin_shutdown(), 1);
    assert_eq!(thaw_napi_poll_shutdown(), 1);
    assert_eq!(*output.lock().unwrap(), vec![1]);
    assert_eq!(thaw_napi_finish_shutdown(), 1);
}

#[test]
fn phased_shutdown_retains_env_and_reports_errors_before_release() {
    let _guard = lock_async_test();
    let mut env = Box::new(Env::new());
    let env_ptr = &mut *env as NapiEnv;
    let previous = env.alloc(Value::Error("prior failure".into()));
    env.exception = Some(previous);
    env.finalizers.push(FinalizeRecord {
        data: ptr::null_mut(), finalize: Some(phased_shutdown_records_callback_error),
        hint: ptr::null_mut(), backing: ptr::null_mut(),
    });
    HOST.with(|host| host.borrow_mut().module_envs.push(env));
    assert_eq!(thaw_napi_begin_shutdown(), 1);
    assert_eq!(thaw_napi_poll_shutdown(), 1);
    assert!(HOST.with(|host| host.borrow().module_envs.iter()
        .any(|entry| (&**entry as *const Env).cast_mut() == env_ptr && entry.finalized)));
    assert_eq!(thaw_napi_finish_shutdown(), 0); // errors remain unreported
    for expected in ["prior failure", "cleanup failure"] {
        let error = thaw_napi_take_shutdown_error();
        assert!(!error.is_null());
        assert_eq!(unsafe { CStr::from_ptr(error) }.to_str().unwrap(), expected);
        unsafe { thaw_arena::destroy_string(error) };
    }
    assert!(thaw_napi_take_shutdown_error().is_null());
    assert_eq!(thaw_napi_finish_shutdown(), 1);
}

#[cfg(target_os = "linux")]
unsafe extern "C" fn close_and_free_uv_handle(handle: *mut c_void) {
    libc::free(handle);
}

#[cfg(target_os = "linux")]
#[test]
fn shutdown_waits_for_unreferenced_and_closing_registered_handles() {
    let _guard = lock_async_test();
    unsafe {
        type UvLoopSize = unsafe extern "C" fn() -> usize;
        type UvLoopInit = unsafe extern "C" fn(*mut c_void) -> i32;
        type UvLoopClose = unsafe extern "C" fn(*mut c_void) -> i32;
        type UvHandleSize = unsafe extern "C" fn(i32) -> usize;
        type UvTimerInit = unsafe extern "C" fn(*mut c_void, *mut c_void) -> i32;
        type UvTimerStart = unsafe extern "C" fn(
            *mut c_void, Option<unsafe extern "C" fn(*mut c_void)>, u64, u64,
        ) -> i32;
        type UvUnref = unsafe extern "C" fn(*mut c_void);
        type UvClose = unsafe extern "C" fn(
            *mut c_void, Option<unsafe extern "C" fn(*mut c_void)>,
        );
        let library = libc::dlopen(c"libuv.so.1".as_ptr(), libc::RTLD_NOW | libc::RTLD_GLOBAL);
        assert!(!library.is_null());
        macro_rules! uv_symbol {
            ($name:literal, $ty:ty) => {{
                let symbol = libc::dlsym(libc::RTLD_DEFAULT, $name.as_ptr());
                assert!(!symbol.is_null());
                std::mem::transmute::<*mut c_void, $ty>(symbol)
            }};
        }
        let loop_size = uv_symbol!(c"uv_loop_size", UvLoopSize);
        let loop_init = uv_symbol!(c"uv_loop_init", UvLoopInit);
        let loop_close = uv_symbol!(c"uv_loop_close", UvLoopClose);
        let handle_size = uv_symbol!(c"uv_handle_size", UvHandleSize);
        let timer_init = uv_symbol!(c"uv_timer_init", UvTimerInit);
        let timer_start = uv_symbol!(c"uv_timer_start", UvTimerStart);
        let unref = uv_symbol!(c"uv_unref", UvUnref);
        let close = uv_symbol!(c"uv_close", UvClose);
        let event_loop = libc::calloc(1, loop_size());
        assert!(!event_loop.is_null());
        assert_eq!(loop_init(event_loop), 0);
        assert_eq!(thaw_napi_register_uv_loop(event_loop), NAPI_OK);
        const UV_TIMER: i32 = 13;
        let timer = libc::calloc(1, handle_size(UV_TIMER));
        assert!(!timer.is_null());
        assert_eq!(timer_init(event_loop, timer), 0);
        assert_eq!(timer_start(timer, Some(test_uv_timer_callback), 10_000, 0), 0);
        unref(timer);
        HOST.with(|host| host.borrow_mut().pending_call_envs.push(Box::new(Env::new())));
        assert_eq!(thaw_napi_begin_shutdown(), 1);
        assert_eq!(thaw_napi_poll_shutdown(), 0);
        assert!(HOST.with(|host| !host.borrow().pending_call_envs[0].finalized));
        assert!(uv_handles_open());
        close(timer, Some(close_and_free_uv_handle));
        assert!(uv_handles_open());
        for _ in 0..100 {
            poll_uv_loop();
            if !uv_handles_open() { break; }
        }
        assert!(!uv_handles_open());
        assert_eq!(thaw_napi_unregister_uv_loop(event_loop), NAPI_OK);
        assert_eq!(thaw_napi_poll_shutdown(), 1);
        assert!(HOST.with(|host| host.borrow().pending_call_envs[0].finalized));
        assert_eq!(thaw_napi_finish_shutdown(), 1);
        assert_eq!(loop_close(event_loop), 0);
        libc::free(event_loop);
        libc::dlclose(library);
    }
}

#[cfg(target_os = "linux")]
#[test]
fn successful_default_loop_acquisition_records_the_env_receipt() {
    let _guard = lock_async_test();
    unsafe {
        let library = libc::dlopen(c"libuv.so.1".as_ptr(), libc::RTLD_NOW | libc::RTLD_GLOBAL);
        assert!(!library.is_null());
        let mut env = Env::new();
        let mut event_loop = ptr::null_mut();
        assert_eq!(napi_get_uv_event_loop(&mut env, &mut event_loop), NAPI_OK);
        assert!(!event_loop.is_null());
        assert_eq!(env.acquired_default_uv_loop, Some(event_loop as usize));
        assert_eq!(HOST.with(|host| host.borrow().owned_uv_loop), Some(event_loop as usize));
        let mut second = Env::new();
        let mut same_loop = ptr::null_mut();
        assert_eq!(napi_get_uv_event_loop(&mut second, &mut same_loop), NAPI_OK);
        assert_eq!(same_loop, event_loop);
        assert!(close_owned_uv_loop());
        libc::dlclose(library);
    }
}
