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
