#[no_mangle]
pub unsafe extern "C" fn napi_open_handle_scope(env: NapiEnv, out: *mut *mut c_void) -> NapiStatus {
    open_handle_scope(env, out, HandleScopeKind::Normal)
}
#[no_mangle]
pub unsafe extern "C" fn napi_close_handle_scope(env: NapiEnv, scope: *mut c_void) -> NapiStatus {
    close_handle_scope(env, scope, HandleScopeKind::Normal)
}

#[no_mangle]
pub unsafe extern "C" fn napi_open_callback_scope(
    env: NapiEnv,
    resource: NapiValue,
    context: *mut c_void,
    out: *mut *mut c_void,
) -> NapiStatus {
    if context.is_null()
        || async_context_mut(env, context).is_err()
        || (!resource.is_null() && !value_belongs_to_environment(env, resource))
    {
        return record_status(env, NAPI_INVALID_ARG);
    }
    let status = open_handle_scope(env, out, HandleScopeKind::Callback);
    record_status(env, status)
}

#[no_mangle]
pub unsafe extern "C" fn napi_close_callback_scope(env: NapiEnv, scope: *mut c_void) -> NapiStatus {
    close_handle_scope(env, scope, HandleScopeKind::Callback)
}

#[no_mangle]
pub unsafe extern "C" fn napi_async_init(
    env: NapiEnv,
    resource: NapiValue,
    resource_name: NapiValue,
    out: *mut *mut c_void,
) -> NapiStatus {
    if out.is_null()
        || resource_name.is_null()
        || !value_belongs_to_environment(env, resource_name)
        || (!resource.is_null() && !value_belongs_to_environment(env, resource))
    {
        return record_status(env, NAPI_INVALID_ARG);
    }
    let Ok(env_ref) = env_mut(env) else {
        return NAPI_INVALID_ARG;
    };
    let name = match value_ref(resource_name) {
        Ok(Value::String(name)) => name.clone(),
        _ => return record_status(env, NAPI_STRING_EXPECTED),
    };
    let mut context = Box::new(AsyncContext {
        env: env as usize,
        resource,
        resource_name: name,
        destroyed: false,
    });
    let context_ptr = (&mut *context) as *mut AsyncContext;
    env_ref.async_contexts.push(context);
    *out = context_ptr.cast();
    NAPI_OK
}

#[no_mangle]
pub unsafe extern "C" fn napi_async_destroy(env: NapiEnv, context: *mut c_void) -> NapiStatus {
    let Ok(context) = async_context_mut(env, context) else {
        return record_status(env, NAPI_INVALID_ARG);
    };
    let _ = (context.resource, &context.resource_name);
    context.destroyed = true;
    NAPI_OK
}

#[no_mangle]
pub unsafe extern "C" fn napi_create_threadsafe_function(
    env: NapiEnv,
    function: NapiValue,
    async_resource: NapiValue,
    async_resource_name: NapiValue,
    max_queue_size: usize,
    initial_thread_count: usize,
    thread_finalize_data: *mut c_void,
    thread_finalize_callback: Option<NapiFinalize>,
    context: *mut c_void,
    call_js_callback: Option<NapiThreadsafeFunctionCallJs>,
    result: *mut *mut ThreadsafeFunction,
) -> NapiStatus {
    if result.is_null()
        || initial_thread_count == 0
        || (function.is_null() && call_js_callback.is_none())
        || (!function.is_null() && !value_belongs_to_environment(env, function))
        || (!async_resource.is_null() && !value_belongs_to_environment(env, async_resource))
        || (!async_resource_name.is_null()
            && !value_belongs_to_environment(env, async_resource_name))
    {
        return record_status(env, NAPI_INVALID_ARG);
    }
    if !function.is_null() && !matches!(value_ref(function), Ok(Value::Function(_))) {
        return record_status(env, NAPI_INVALID_ARG);
    }
    if !async_resource_name.is_null()
        && !matches!(value_ref(async_resource_name), Ok(Value::String(_)))
    {
        return record_status(env, NAPI_STRING_EXPECTED);
    }
    let mut threadsafe = Box::new(ThreadsafeFunction {
        env: env as usize,
        function: function as usize,
        context: context as usize,
        call_js: call_js_callback,
        finalize_data: thread_finalize_data as usize,
        finalize: thread_finalize_callback,
        max_queue_size,
        creator: std::thread::current().id(),
        referenced: AtomicBool::new(true),
        state: Mutex::new(ThreadsafeState {
            queue: VecDeque::new(),
            thread_count: initial_thread_count,
            closing: false,
            aborting: false,
            scheduled: false,
        }),
        space_available: Condvar::new(),
    });
    ACTIVE_THREADSAFE_FUNCTIONS.fetch_add(1, Ordering::AcqRel);
    LIVE_THREADSAFE_FUNCTIONS.fetch_add(1, Ordering::AcqRel);
    let threadsafe_ptr = (&mut *threadsafe) as *mut ThreadsafeFunction;
    threadsafe_functions()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .push(threadsafe);
    *result = threadsafe_ptr;
    NAPI_OK
}

#[no_mangle]
pub unsafe extern "C" fn napi_get_threadsafe_function_context(
    function: *mut ThreadsafeFunction,
    result: *mut *mut c_void,
) -> NapiStatus {
    let Ok(function) = threadsafe_function_ref(function) else {
        return NAPI_INVALID_ARG;
    };
    let Some(result) = result.as_mut() else {
        return record_threadsafe_status(function, NAPI_INVALID_ARG);
    };
    *result = function.context as *mut c_void;
    NAPI_OK
}

#[no_mangle]
pub unsafe extern "C" fn napi_call_threadsafe_function(
    function: *mut ThreadsafeFunction,
    data: *mut c_void,
    mode: i32,
) -> NapiStatus {
    let Ok(function_ref) = threadsafe_function_ref(function) else {
        return NAPI_INVALID_ARG;
    };
    if mode != 0 && mode != 1 {
        return record_threadsafe_status(function_ref, NAPI_INVALID_ARG);
    }
    let mut state = function_ref
        .state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    loop {
        if state.closing {
            return record_threadsafe_status(function_ref, NAPI_CLOSING);
        }
        if function_ref.max_queue_size == 0 || state.queue.len() < function_ref.max_queue_size {
            break;
        }
        if mode == 0 {
            return record_threadsafe_status(function_ref, NAPI_QUEUE_FULL);
        }
        if std::thread::current().id() == function_ref.creator {
            return record_threadsafe_status(function_ref, NAPI_WOULD_DEADLOCK);
        }
        state = function_ref
            .space_available
            .wait(state)
            .unwrap_or_else(|poisoned| poisoned.into_inner());
    }
    state.queue.push_back(data as usize);
    schedule_threadsafe(function, &mut state);
    NAPI_OK
}

#[no_mangle]
pub unsafe extern "C" fn napi_release_threadsafe_function(
    function: *mut ThreadsafeFunction,
    mode: i32,
) -> NapiStatus {
    let Ok(function_ref) = threadsafe_function_ref(function) else {
        return NAPI_INVALID_ARG;
    };
    if mode != 0 && mode != 1 {
        return record_threadsafe_status(function_ref, NAPI_INVALID_ARG);
    }
    let mut state = function_ref
        .state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if state.closing || state.thread_count == 0 {
        return record_threadsafe_status(function_ref, NAPI_CLOSING);
    }
    if mode == 1 {
        state.thread_count = 0;
        state.closing = true;
        state.aborting = true;
    } else {
        state.thread_count -= 1;
        if state.thread_count == 0 {
            state.closing = true;
        }
    }
    function_ref.space_available.notify_all();
    if state.closing {
        schedule_threadsafe(function, &mut state);
    }
    NAPI_OK
}

#[no_mangle]
pub unsafe extern "C" fn napi_acquire_threadsafe_function(
    function: *mut ThreadsafeFunction,
) -> NapiStatus {
    let Ok(function_ref) = threadsafe_function_ref(function) else {
        return NAPI_INVALID_ARG;
    };
    let mut state = function_ref
        .state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if state.closing {
        return record_threadsafe_status(function_ref, NAPI_CLOSING);
    }
    state.thread_count = state.thread_count.saturating_add(1);
    NAPI_OK
}

#[no_mangle]
pub unsafe extern "C" fn napi_ref_threadsafe_function(
    env: NapiEnv,
    function: *mut ThreadsafeFunction,
) -> NapiStatus {
    let Ok(function) = threadsafe_function_ref(function) else {
        return NAPI_INVALID_ARG;
    };
    if env.is_null() || function.env != env as usize {
        return record_status(env, NAPI_INVALID_ARG);
    }
    if function
        .state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .closing
    {
        return record_status(env, NAPI_CLOSING);
    }
    if !function.referenced.swap(true, Ordering::AcqRel) {
        ACTIVE_THREADSAFE_FUNCTIONS.fetch_add(1, Ordering::AcqRel);
    }
    NAPI_OK
}

#[no_mangle]
pub unsafe extern "C" fn napi_unref_threadsafe_function(
    env: NapiEnv,
    function: *mut ThreadsafeFunction,
) -> NapiStatus {
    let Ok(function) = threadsafe_function_ref(function) else {
        return NAPI_INVALID_ARG;
    };
    if env.is_null() || function.env != env as usize {
        return record_status(env, NAPI_INVALID_ARG);
    }
    if function
        .state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .closing
    {
        return record_status(env, NAPI_CLOSING);
    }
    if function.referenced.swap(false, Ordering::AcqRel) {
        ACTIVE_THREADSAFE_FUNCTIONS.fetch_sub(1, Ordering::AcqRel);
    }
    NAPI_OK
}
#[no_mangle]
pub unsafe extern "C" fn napi_open_escapable_handle_scope(
    env: NapiEnv,
    out: *mut *mut c_void,
) -> NapiStatus {
    open_handle_scope(env, out, HandleScopeKind::Escapable)
}
#[no_mangle]
pub unsafe extern "C" fn napi_close_escapable_handle_scope(
    env: NapiEnv,
    scope: *mut c_void,
) -> NapiStatus {
    close_handle_scope(env, scope, HandleScopeKind::Escapable)
}
#[no_mangle]
pub unsafe extern "C" fn napi_escape_handle(
    env: NapiEnv,
    scope: *mut c_void,
    value: NapiValue,
    out: *mut NapiValue,
) -> NapiStatus {
    if value.is_null() || out.is_null() || !value_belongs_to_environment(env, value) {
        return record_status(env, NAPI_INVALID_ARG);
    }
    let Ok(env_ref) = env_mut(env) else {
        return NAPI_INVALID_ARG;
    };
    let scope_ptr = scope.cast::<HandleScope>();
    let Some(scope_ref) = env_ref
        .handle_scopes
        .iter_mut()
        .find(|candidate| std::ptr::eq(candidate.as_ref(), scope_ptr))
    else {
        return record_status(env, NAPI_INVALID_ARG);
    };
    if scope_ref.closed
        || scope_ref.kind != HandleScopeKind::Escapable
        || env_ref.active_handle_scopes.last().copied() != Some(scope_ptr)
    {
        return record_status(env, NAPI_HANDLE_SCOPE_MISMATCH);
    }
    if scope_ref.escaped {
        return record_status(env, NAPI_ESCAPE_CALLED_TWICE);
    }
    scope_ref.escaped = true;
    *out = value;
    NAPI_OK
}

#[no_mangle]
pub unsafe extern "C" fn napi_create_promise(
    env: NapiEnv,
    deferred: *mut *mut Deferred,
    promise: *mut NapiValue,
) -> NapiStatus {
    if deferred.is_null() || promise.is_null() {
        return record_status(env, NAPI_INVALID_ARG);
    }
    let Ok(env_ref) = env_mut(env) else {
        return NAPI_INVALID_ARG;
    };
    let state = Rc::new(RefCell::new(PromiseState::Pending));
    *promise = env_ref.alloc(Value::Promise(Rc::clone(&state)));
    let mut deferred_handle = Box::new(Deferred {
        env: env as usize,
        state,
    });
    let deferred_ptr = (&mut *deferred_handle) as *mut Deferred;
    env_ref.deferreds.push(deferred_handle);
    *deferred = deferred_ptr;
    NAPI_OK
}

unsafe fn settle_deferred(
    env: NapiEnv,
    deferred: *mut Deferred,
    value: NapiValue,
    rejected: bool,
) -> NapiStatus {
    if !value_belongs_to_environment(env, value) {
        return NAPI_INVALID_ARG;
    }
    let Ok(env_ref) = env_mut(env) else {
        return NAPI_INVALID_ARG;
    };
    let Some(deferred_ref) = env_ref
        .deferreds
        .iter_mut()
        .find(|candidate| std::ptr::eq(candidate.as_ref(), deferred))
    else {
        return record_status(env, NAPI_INVALID_ARG);
    };
    if deferred_ref.env != env as usize {
        return record_status(env, NAPI_INVALID_ARG);
    }
    let mut state = deferred_ref.state.borrow_mut();
    if !matches!(*state, PromiseState::Pending) {
        return record_status(env, NAPI_GENERIC_FAILURE);
    }
    *state = if rejected {
        PromiseState::Rejected(value)
    } else {
        PromiseState::Resolved(value)
    };
    NAPI_OK
}

#[no_mangle]
pub unsafe extern "C" fn napi_resolve_deferred(
    env: NapiEnv,
    deferred: *mut Deferred,
    value: NapiValue,
) -> NapiStatus {
    settle_deferred(env, deferred, value, false)
}

#[no_mangle]
pub unsafe extern "C" fn napi_reject_deferred(
    env: NapiEnv,
    deferred: *mut Deferred,
    value: NapiValue,
) -> NapiStatus {
    settle_deferred(env, deferred, value, true)
}

#[no_mangle]
pub unsafe extern "C" fn napi_create_async_work(
    env: NapiEnv,
    async_resource: NapiValue,
    async_resource_name: NapiValue,
    execute: Option<NapiAsyncExecuteCallback>,
    complete: Option<NapiAsyncCompleteCallback>,
    data: *mut c_void,
    result: *mut *mut AsyncWork,
) -> NapiStatus {
    if execute.is_none()
        || result.is_null()
        || (!async_resource.is_null() && !value_belongs_to_environment(env, async_resource))
        || (!async_resource_name.is_null()
            && !value_belongs_to_environment(env, async_resource_name))
    {
        return record_status(env, NAPI_INVALID_ARG);
    }
    if !async_resource_name.is_null()
        && !matches!(
            value_ref(async_resource_name),
            Ok(Value::String(_) | Value::Undefined)
        )
    {
        return record_status(env, NAPI_STRING_EXPECTED);
    }
    let Ok(env_ref) = env_mut(env) else {
        return NAPI_INVALID_ARG;
    };
    let mut work = Box::new(AsyncWork {
        env: env as usize,
        execute: execute.unwrap(),
        complete,
        data: data as usize,
        state: AtomicU8::new(ASYNC_CREATED),
        completion_status: AtomicI32::new(NAPI_OK),
    });
    let work_ptr = (&mut *work) as *mut AsyncWork;
    env_ref.async_works.push(work);
    *result = work_ptr;
    NAPI_OK
}

#[no_mangle]
pub unsafe extern "C" fn napi_queue_async_work(env: NapiEnv, work: *mut AsyncWork) -> NapiStatus {
    let Ok(work_ref) = async_work_ref(env, work) else {
        return record_status(env, NAPI_INVALID_ARG);
    };
    if work_ref
        .state
        .compare_exchange(
            ASYNC_CREATED,
            ASYNC_QUEUED,
            Ordering::AcqRel,
            Ordering::Acquire,
        )
        .is_err()
    {
        return record_status(env, NAPI_GENERIC_FAILURE);
    }

    let Some(pool) = async_pool() else {
        work_ref.state.store(ASYNC_CREATED, Ordering::Release);
        return record_status(env, NAPI_GENERIC_FAILURE);
    };
    ACTIVE_ASYNC_WORK.fetch_add(1, Ordering::AcqRel);
    pool.queue
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .push_back(work as usize);
    pool.ready.notify_one();
    NAPI_OK
}

#[no_mangle]
pub unsafe extern "C" fn napi_cancel_async_work(env: NapiEnv, work: *mut AsyncWork) -> NapiStatus {
    let Ok(work_ref) = async_work_ref(env, work) else {
        return record_status(env, NAPI_INVALID_ARG);
    };
    let Some(pool) = async_pool() else {
        return record_status(env, NAPI_GENERIC_FAILURE);
    };
    let mut queue = pool
        .queue
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if work_ref.state.load(Ordering::Acquire) != ASYNC_QUEUED {
        return record_status(env, NAPI_GENERIC_FAILURE);
    }
    let Some(position) = queue.iter().position(|queued| *queued == work as usize) else {
        return record_status(env, NAPI_GENERIC_FAILURE);
    };
    queue.remove(position);
    work_ref
        .completion_status
        .store(NAPI_CANCELLED, Ordering::Release);
    work_ref
        .state
        .store(ASYNC_COMPLETE_PENDING, Ordering::Release);
    drop(queue);
    ready_events()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .push_back(ReadyEvent::AsyncCompletion(work as usize));
    NAPI_OK
}

#[no_mangle]
pub unsafe extern "C" fn napi_delete_async_work(env: NapiEnv, work: *mut AsyncWork) -> NapiStatus {
    let Ok(work_ref) = async_work_ref(env, work) else {
        return record_status(env, NAPI_INVALID_ARG);
    };
    let state = work_ref.state.load(Ordering::Acquire);
    if state != ASYNC_CREATED && state != ASYNC_COMPLETED {
        return record_status(env, NAPI_GENERIC_FAILURE);
    }
    if work_ref
        .state
        .compare_exchange(state, ASYNC_DELETED, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        return record_status(env, NAPI_GENERIC_FAILURE);
    }
    NAPI_OK
}

fn run_one_threadsafe_callback(address: usize) -> bool {
    let function_ptr = address as *mut ThreadsafeFunction;
    let function = unsafe { &*function_ptr };
    let (data, aborting, finalize) = {
        let mut state = function
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        state.scheduled = false;
        let data = state.queue.pop_front();
        if data.is_some() {
            function.space_available.notify_one();
        }
        if !state.queue.is_empty() {
            schedule_threadsafe(function_ptr, &mut state);
        }
        let finalize = state.closing && state.queue.is_empty();
        (data, state.aborting, finalize)
    };

    if let Some(data) = data {
        unsafe {
            if let Some(call_js) = function.call_js {
                if aborting {
                    call_js(
                        ptr::null_mut(),
                        ptr::null_mut(),
                        function.context as *mut c_void,
                        data as *mut c_void,
                    );
                } else {
                    call_js(
                        function.env as NapiEnv,
                        function.function as NapiValue,
                        function.context as *mut c_void,
                        data as *mut c_void,
                    );
                }
            } else if !aborting {
                let env = function.env as NapiEnv;
                let mut undefined = ptr::null_mut();
                if napi_get_undefined(env, &mut undefined) == NAPI_OK {
                    let _ = napi_call_function(
                        env,
                        undefined,
                        function.function as NapiValue,
                        0,
                        ptr::null(),
                        ptr::null_mut(),
                    );
                }
            }
        }
    }

    if finalize {
        LIVE_THREADSAFE_FUNCTIONS.fetch_sub(1, Ordering::AcqRel);
        if function.referenced.swap(false, Ordering::AcqRel) {
            ACTIVE_THREADSAFE_FUNCTIONS.fetch_sub(1, Ordering::AcqRel);
        }
        if let Some(finalize) = function.finalize {
            unsafe {
                finalize(
                    function.env as NapiEnv,
                    function.finalize_data as *mut c_void,
                    function.context as *mut c_void,
                );
            }
        }
    }
    data.is_some() && !aborting
}

fn run_one_async_completion(work_address: usize) {
    let work = unsafe { &*(work_address as *const AsyncWork) };
    let env = work.env as NapiEnv;
    let data = work.data as *mut c_void;
    let complete = work.complete;
    let status = work.completion_status.load(Ordering::Acquire);
    work.state.store(ASYNC_COMPLETED, Ordering::Release);
    ACTIVE_ASYNC_WORK.fetch_sub(1, Ordering::AcqRel);
    if let Some(complete) = complete {
        unsafe { complete(env, status, data) };
    }
    if let Err(error) = unsafe { take_env_exception(env) } {
        HOST.with(|host| host.borrow_mut().last_error = error);
    }
}

fn drain_posted_finalizers() -> usize {
    let mut completed = 0;
    loop {
        let batches = HOST.with(|host| {
            let mut host = host.borrow_mut();
            let mut batches = Vec::new();
            let mut collect = |envs: &mut Vec<Box<Env>>| {
                for env in envs {
                    if !env.posted_finalizers.is_empty() {
                        batches.push((
                            (&mut **env as NapiEnv) as usize,
                            std::mem::take(&mut env.posted_finalizers),
                        ));
                    }
                }
            };
            collect(&mut host.module_envs);
            collect(&mut host.pending_call_envs);
            batches
        });
        if batches.is_empty() {
            return completed;
        }
        for (env, records) in batches {
            for record in records {
                if let Some(finalize) = record.finalize {
                    unsafe { finalize(env as NapiEnv, record.data, record.hint) };
                    completed += 1;
                }
            }
        }
    }
}

#[cfg(target_os = "linux")]
unsafe fn poll_uv_loop() -> bool {
    type UvDefaultLoop = unsafe extern "C" fn() -> *mut c_void;
    type UvRun = unsafe extern "C" fn(*mut c_void, i32) -> i32;
    type UvLoopAlive = unsafe extern "C" fn(*const c_void) -> i32;

    let default_loop = libc::dlsym(libc::RTLD_DEFAULT, c"uv_default_loop".as_ptr());
    let run = libc::dlsym(libc::RTLD_DEFAULT, c"uv_run".as_ptr());
    let alive = libc::dlsym(libc::RTLD_DEFAULT, c"uv_loop_alive".as_ptr());
    if default_loop.is_null() || run.is_null() || alive.is_null() {
        return false;
    }
    let default_loop = std::mem::transmute::<*mut c_void, UvDefaultLoop>(default_loop);
    let run = std::mem::transmute::<*mut c_void, UvRun>(run);
    let alive = std::mem::transmute::<*mut c_void, UvLoopAlive>(alive);
    const UV_RUN_NOWAIT: i32 = 2;
    let default_loop = default_loop();
    let mut loops = registered_uv_loops()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone();
    if !default_loop.is_null() && !loops.contains(&(default_loop as usize)) {
        loops.push(default_loop as usize);
    }
    let mut any_alive = false;
    for event_loop in loops {
        let event_loop = event_loop as *mut c_void;
        run(event_loop, UV_RUN_NOWAIT);
        any_alive |= alive(event_loop) != 0;
    }
    any_alive
}

#[cfg(not(target_os = "linux"))]
unsafe fn poll_uv_loop() -> bool {
    false
}

/// Runs every callback which is ready now without waiting for producers.
#[no_mangle]
pub extern "C" fn thaw_napi_poll_async_work() -> usize {
    let mut completed = drain_posted_finalizers();
    unsafe {
        poll_uv_loop();
    }
    loop {
        let mut progressed = false;
        loop {
            let event = ready_events()
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .pop_front();
            let Some(event) = event else { break };
            completed += match event {
                ReadyEvent::ThreadsafeFunction(address) => {
                    usize::from(run_one_threadsafe_callback(address))
                }
                ReadyEvent::AsyncCompletion(address) => {
                    run_one_async_completion(address);
                    1
                }
            };
            progressed = true;
        }
        let finalized = drain_posted_finalizers();
        completed += finalized;
        progressed |= finalized != 0;
        if !progressed {
            break;
        }
    }
    if LIVE_THREADSAFE_FUNCTIONS.load(Ordering::Acquire) == 0
        && ACTIVE_ASYNC_WORK.load(Ordering::Acquire) == 0
    {
        HOST.with(|host| {
            let mut host = host.borrow_mut();
            host.compiled_callbacks.clear();
            host.pending_call_envs.clear();
        });
    }
    completed
}

#[no_mangle]
pub extern "C" fn thaw_napi_async_work_pending() -> u8 {
    u8::from(ACTIVE_ASYNC_WORK.load(Ordering::Acquire) != 0)
}

/// Runs queued completion callbacks on the calling thread and waits until all
/// work submitted by native addons has completed. Generated executables call
/// this once user `main` returns.
#[no_mangle]
pub extern "C" fn thaw_napi_run_async_work() -> usize {
    let mut completed = 0;
    loop {
        completed += thaw_napi_poll_async_work();
        let ready = !ready_events()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .is_empty();
        let uv_alive = unsafe { poll_uv_loop() };
        if ACTIVE_ASYNC_WORK.load(Ordering::Acquire) == 0
            && ACTIVE_THREADSAFE_FUNCTIONS.load(Ordering::Acquire) == 0
            && !ready
            && !uv_alive
        {
            break;
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    if LIVE_THREADSAFE_FUNCTIONS.load(Ordering::Acquire) == 0 {
        HOST.with(|host| {
            let mut host = host.borrow_mut();
            host.compiled_callbacks.clear();
            host.pending_call_envs.clear();
        });
    }
    completed
}
