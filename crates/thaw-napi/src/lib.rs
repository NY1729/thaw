//! Minimal Node-API host for loading `.node` addons.

// Every exported unsafe function in this crate implements the Node-API C ABI
// and shares its pointer-validity contract with the native addon caller.
#![allow(clippy::missing_safety_doc)]

use libc::{c_char, c_void};
use serde_json::Value as JsonValue;
use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};
use std::ffi::{CStr, CString};
use std::io::Write;
#[cfg(target_os = "linux")]
use std::os::fd::FromRawFd;
use std::ptr;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU8, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::time::Duration;

type NapiEnv = *mut Env;
type NapiValue = *mut Value;
type NapiCallbackInfo = *mut CallbackInfo;
type NapiStatus = i32;
type NapiCallback = unsafe extern "C" fn(NapiEnv, NapiCallbackInfo) -> NapiValue;
type NapiAsyncExecuteCallback = unsafe extern "C" fn(NapiEnv, *mut c_void);
type NapiAsyncCompleteCallback = unsafe extern "C" fn(NapiEnv, NapiStatus, *mut c_void);
type ThawNativeCallback = unsafe extern "C" fn(*mut c_void, *const c_char, *const c_char);
type NapiFinalize = unsafe extern "C" fn(NapiEnv, *mut c_void, *mut c_void);
type NapiCleanupHook = unsafe extern "C" fn(*mut c_void);
type NapiThreadsafeFunctionCallJs =
    unsafe extern "C" fn(NapiEnv, NapiValue, *mut c_void, *mut c_void);
const NAPI_OK: NapiStatus = 0;

const NAPI_INVALID_ARG: NapiStatus = 1;
const NAPI_GENERIC_FAILURE: NapiStatus = 9;
const NAPI_CANCELLED: NapiStatus = 11;
const NAPI_QUEUE_FULL: NapiStatus = 15;
const NAPI_CLOSING: NapiStatus = 16;
const NAPI_WOULD_DEADLOCK: NapiStatus = 21;
const NAPI_NUMBER_EXPECTED: NapiStatus = 6;
const NAPI_STRING_EXPECTED: NapiStatus = 3;
const NAPI_BOOLEAN_EXPECTED: NapiStatus = 7;
const NAPI_AUTO_LENGTH: usize = usize::MAX;

const ASYNC_CREATED: u8 = 0;
const ASYNC_QUEUED: u8 = 1;
const ASYNC_EXECUTING: u8 = 2;
const ASYNC_COMPLETE_PENDING: u8 = 3;
const ASYNC_COMPLETED: u8 = 4;

pub struct AsyncWork {
    env: usize,
    execute: NapiAsyncExecuteCallback,
    complete: Option<NapiAsyncCompleteCallback>,
    data: usize,
    state: AtomicU8,
    completion_status: AtomicI32,
}

struct AsyncPool {
    queue: Mutex<VecDeque<usize>>,
    ready: Condvar,
}

static ASYNC_COMPLETIONS: OnceLock<Mutex<VecDeque<usize>>> = OnceLock::new();
static ASYNC_POOL: OnceLock<Option<Arc<AsyncPool>>> = OnceLock::new();
static ACTIVE_ASYNC_WORK: AtomicUsize = AtomicUsize::new(0);
static ACTIVE_THREADSAFE_FUNCTIONS: AtomicUsize = AtomicUsize::new(0);
static LIVE_THREADSAFE_FUNCTIONS: AtomicUsize = AtomicUsize::new(0);
static FATAL_EXCEPTION_PENDING: AtomicBool = AtomicBool::new(false);
static THREADSAFE_READY: OnceLock<Mutex<VecDeque<usize>>> = OnceLock::new();

pub struct ThreadsafeFunction {
    env: usize,
    function: usize,
    context: usize,
    call_js: Option<NapiThreadsafeFunctionCallJs>,
    finalize_data: usize,
    finalize: Option<NapiFinalize>,
    max_queue_size: usize,
    creator: std::thread::ThreadId,
    referenced: AtomicBool,
    state: Mutex<ThreadsafeState>,
    space_available: Condvar,
}

struct ThreadsafeState {
    queue: VecDeque<usize>,
    thread_count: usize,
    closing: bool,
    aborting: bool,
    scheduled: bool,
}

fn threadsafe_ready() -> &'static Mutex<VecDeque<usize>> {
    THREADSAFE_READY.get_or_init(|| Mutex::new(VecDeque::new()))
}

fn schedule_threadsafe(function: *mut ThreadsafeFunction, state: &mut ThreadsafeState) {
    if !state.scheduled {
        state.scheduled = true;
        threadsafe_ready()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push_back(function as usize);
    }
}

fn async_completions() -> &'static Mutex<VecDeque<usize>> {
    ASYNC_COMPLETIONS.get_or_init(|| Mutex::new(VecDeque::new()))
}

fn async_worker_count() -> usize {
    std::thread::available_parallelism()
        .map(usize::from)
        .unwrap_or(2)
        .clamp(1, 4)
}

fn async_pool() -> Option<&'static Arc<AsyncPool>> {
    ASYNC_POOL
        .get_or_init(|| {
            let pool = Arc::new(AsyncPool {
                queue: Mutex::new(VecDeque::new()),
                ready: Condvar::new(),
            });
            for index in 0..async_worker_count() {
                let worker_pool = Arc::clone(&pool);
                if std::thread::Builder::new()
                    .name(format!("thaw-napi-worker-{index}"))
                    .spawn(move || async_worker(worker_pool))
                    .is_err()
                {
                    return None;
                }
            }
            Some(pool)
        })
        .as_ref()
}

fn async_worker(pool: Arc<AsyncPool>) {
    loop {
        let work_address = {
            let mut queue = pool
                .queue
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            while queue.is_empty() {
                queue = pool
                    .ready
                    .wait(queue)
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
            }
            let work_address = queue.pop_front().unwrap();
            let work = unsafe { &*(work_address as *const AsyncWork) };
            work.state.store(ASYNC_EXECUTING, Ordering::Release);
            work_address
        };
        let work = unsafe { &*(work_address as *const AsyncWork) };
        unsafe {
            (work.execute)(work.env as NapiEnv, work.data as *mut c_void);
        }
        work.completion_status.store(NAPI_OK, Ordering::Release);
        work.state.store(ASYNC_COMPLETE_PENDING, Ordering::Release);
        async_completions()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push_back(work_address);
    }
}

#[derive(Clone)]
pub struct Function {
    callback: NapiCallback,
    data: *mut c_void,
    properties: HashMap<String, NapiValue>,
    _thaw_bridge: Option<Arc<ThawCallbackBridge>>,
}

struct ThawCallbackBridge {
    callback: ThawNativeCallback,
    context: usize,
}

pub enum Value {
    Undefined,
    Null,
    Bool(bool),
    Number(f64),
    String(String),
    Object(HashMap<String, NapiValue>),
    Array(Vec<NapiValue>),
    Buffer(Vec<u8>),
    External(*mut c_void),
    Symbol(String),
    Function(Function),
    Promise(Rc<RefCell<PromiseState>>),
    Error(String),
}

pub enum PromiseState {
    Pending,
    Resolved(NapiValue),
    Rejected(NapiValue),
}

pub struct Deferred {
    env: usize,
    state: Rc<RefCell<PromiseState>>,
}

pub struct Reference {
    value: NapiValue,
    count: u32,
}

#[repr(C)]
pub struct NapiPropertyDescriptor {
    utf8name: *const c_char,
    name: NapiValue,
    method: Option<NapiCallback>,
    getter: Option<NapiCallback>,
    setter: Option<NapiCallback>,
    value: NapiValue,
    attributes: u32,
    data: *mut c_void,
}

#[repr(C)]
pub struct NapiExtendedErrorInfo {
    error_message: *const c_char,
    engine_reserved: *mut c_void,
    engine_error_code: u32,
    error_code: NapiStatus,
}

static LAST_ERROR_INFO: NapiExtendedErrorInfo = NapiExtendedErrorInfo {
    error_message: ptr::null(),
    engine_reserved: ptr::null_mut(),
    engine_error_code: 0,
    error_code: NAPI_OK,
};
unsafe impl Sync for NapiExtendedErrorInfo {}

pub struct Env {
    values: Vec<NapiValue>,
    global: NapiValue,
    exception: Option<NapiValue>,
    wraps: HashMap<usize, WrapRecord>,
    instances: HashMap<usize, usize>,
    accessors: HashMap<(usize, String), Accessor>,
    finalizers: Vec<FinalizeRecord>,
    instance_data: Option<FinalizeRecord>,
    cleanup_hooks: Vec<CleanupHookRecord>,
}

#[derive(Clone, Copy)]
struct CleanupHookRecord {
    hook: NapiCleanupHook,
    data: *mut c_void,
}

struct FinalizeRecord {
    data: *mut c_void,
    finalize: Option<NapiFinalize>,
    hint: *mut c_void,
}

struct WrapRecord {
    data: *mut c_void,
    finalize: Option<NapiFinalize>,
    hint: *mut c_void,
}

#[derive(Clone, Copy)]
struct Accessor {
    getter: Option<NapiCallback>,
    setter: Option<NapiCallback>,
    data: *mut c_void,
}

impl Env {
    fn new() -> Self {
        Self {
            values: Vec::new(),
            global: ptr::null_mut(),
            exception: None,
            wraps: HashMap::new(),
            instances: HashMap::new(),
            accessors: HashMap::new(),
            finalizers: Vec::new(),
            instance_data: None,
            cleanup_hooks: Vec::new(),
        }
    }

    fn alloc(&mut self, value: Value) -> NapiValue {
        let value = Box::into_raw(Box::new(value));
        self.values.push(value);
        value
    }
}

impl Drop for Env {
    fn drop(&mut self) {
        for hook in std::mem::take(&mut self.cleanup_hooks).into_iter().rev() {
            unsafe { (hook.hook)(hook.data) };
        }
        if let Some(record) = self.instance_data.take() {
            if let Some(finalize) = record.finalize {
                unsafe { finalize(self, record.data, record.hint) };
            }
        }
        for record in std::mem::take(&mut self.finalizers) {
            if let Some(finalize) = record.finalize {
                unsafe { finalize(self, record.data, record.hint) };
            }
        }
        let wraps = std::mem::take(&mut self.wraps);
        for wrap in wraps.into_values() {
            if let Some(finalize) = wrap.finalize {
                unsafe {
                    finalize(self, wrap.data, wrap.hint);
                }
            }
        }
        for value in self.values.drain(..) {
            unsafe {
                drop(Box::from_raw(value));
            }
        }
    }
}

pub struct CallbackInfo {
    args: Vec<NapiValue>,
    this_arg: NapiValue,
    new_target: NapiValue,
    data: *mut c_void,
}

struct Host {
    functions: HashMap<String, Function>,
    exports: HashMap<String, (usize, NapiValue)>,
    compiled_callbacks: HashMap<(usize, usize), NapiValue>,
    libraries: Vec<*mut c_void>,
    // Addons retain `napi_env` pointers, so moving an Env during Vec growth
    // would invalidate foreign pointers. The Box provides stable addresses.
    #[allow(clippy::vec_box)]
    module_envs: Vec<Box<Env>>,
    // Async addons retain the `napi_env`; boxes keep those addresses stable
    // while unrelated calls grow the pending vector.
    #[allow(clippy::vec_box)]
    pending_call_envs: Vec<Box<Env>>,
    last_error: String,
}

impl Host {
    fn new() -> Self {
        Self {
            functions: HashMap::new(),
            exports: HashMap::new(),
            compiled_callbacks: HashMap::new(),
            libraries: Vec::new(),
            module_envs: Vec::new(),
            pending_call_envs: Vec::new(),
            last_error: String::new(),
        }
    }
}

thread_local! {
    static HOST: RefCell<Host> = RefCell::new(Host::new());
    static PENDING_MODULE: RefCell<Option<NapiModule>> = const { RefCell::new(None) };
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct NapiModule {
    nm_version: i32,
    nm_flags: u32,
    nm_filename: *const c_char,
    nm_register_func: Option<unsafe extern "C" fn(NapiEnv, NapiValue) -> NapiValue>,
    nm_modname: *const c_char,
    nm_priv: *mut c_void,
    reserved: [*mut c_void; 4],
}

#[repr(C)]
pub struct ThawResult {
    pub value: *mut c_char,
    pub error: *mut c_char,
}

#[repr(C)]
pub struct ThawNapiHandleResult {
    pub value: u64,
    pub error: *mut c_char,
}

unsafe fn text(ptr: *const c_char) -> Result<String, String> {
    if ptr.is_null() {
        return Err("null string pointer".into());
    }
    Ok(CStr::from_ptr(ptr).to_string_lossy().into_owned())
}

unsafe fn env_mut<'a>(env: NapiEnv) -> Result<&'a mut Env, NapiStatus> {
    env.as_mut().ok_or(NAPI_INVALID_ARG)
}

unsafe fn value_ref<'a>(value: NapiValue) -> Result<&'a Value, NapiStatus> {
    value.as_ref().ok_or(NAPI_INVALID_ARG)
}

unsafe fn find_accessor(env: NapiEnv, object: NapiValue, name: &str) -> Option<Accessor> {
    if let Some(accessor) = env.as_ref().and_then(|env| {
        env.accessors
            .get(&(object as usize, name.to_string()))
            .copied()
    }) {
        return Some(accessor);
    }
    HOST.with(|host| {
        host.borrow().module_envs.iter().find_map(|module_env| {
            module_env
                .accessors
                .get(&(object as usize, name.to_string()))
                .copied()
        })
    })
}

unsafe fn find_accessors(env: NapiEnv, object: NapiValue) -> Vec<(String, Accessor)> {
    let from = |env: &Env| {
        env.accessors
            .iter()
            .filter(|((owner, _), _)| *owner == object as usize)
            .map(|((_, name), accessor)| (name.clone(), *accessor))
            .collect::<Vec<_>>()
    };
    let current = env.as_ref().map(from).unwrap_or_default();
    if !current.is_empty() {
        return current;
    }
    HOST.with(|host| {
        host.borrow()
            .module_envs
            .iter()
            .find_map(|module_env| {
                let accessors = from(module_env);
                (!accessors.is_empty()).then_some(accessors)
            })
            .unwrap_or_default()
    })
}

unsafe fn write_value(out: *mut NapiValue, value: NapiValue) -> NapiStatus {
    if out.is_null() {
        return NAPI_INVALID_ARG;
    }
    *out = value;
    NAPI_OK
}

#[no_mangle]
pub unsafe extern "C" fn napi_module_register(module: *mut NapiModule) {
    if let Some(module) = module.as_ref() {
        PENDING_MODULE.with(|slot| *slot.borrow_mut() = Some(*module));
    }
}

type RegisterV1 = unsafe extern "C" fn(NapiEnv, NapiValue) -> NapiValue;

unsafe fn load_impl(path: &str, root_name: Option<&str>) -> Result<(), String> {
    let path = CString::new(path).map_err(|_| "addon path contains NUL".to_string())?;
    PENDING_MODULE.with(|slot| slot.borrow_mut().take());
    // Node exports libuv from its executable. Some otherwise portable N-API
    // addons (notably serialport) call that API directly, so expose the system
    // libuv with global symbol visibility before resolving the addon.
    #[cfg(target_os = "linux")]
    let uv_handle = libc::dlopen(c"libuv.so.1".as_ptr(), libc::RTLD_NOW | libc::RTLD_GLOBAL);
    libc::dlerror();
    let handle = libc::dlopen(path.as_ptr(), libc::RTLD_NOW | libc::RTLD_LOCAL);
    if handle.is_null() {
        let error = libc::dlerror();
        return Err(if error.is_null() {
            "dlopen failed".into()
        } else {
            CStr::from_ptr(error).to_string_lossy().into_owned()
        });
    }
    let direct = libc::dlsym(handle, c"napi_register_module_v1".as_ptr());
    let registered = PENDING_MODULE.with(|slot| slot.borrow_mut().take());
    let init: RegisterV1 = if !direct.is_null() {
        std::mem::transmute::<*mut c_void, RegisterV1>(direct)
    } else if let Some(module) = registered {
        module
            .nm_register_func
            .ok_or("registered N-API module has no init function")?
    } else {
        libc::dlclose(handle);
        return Err(
            "addon exports neither napi_register_module_v1 nor a registered N-API module".into(),
        );
    };

    let mut env = Box::new(Env::new());
    let env_ptr = &mut *env as NapiEnv;
    let exports = env.alloc(Value::Object(HashMap::new()));
    let returned = init(env_ptr, exports);
    let exports = if returned.is_null() {
        exports
    } else {
        returned
    };
    if let Some(exception) = env.exception {
        let message = json_from_value(exception)?.to_string();
        libc::dlclose(handle);
        return Err(format!("addon initialization threw: {message}"));
    }
    let mut functions = Vec::new();
    let mut exported_values = Vec::new();
    match value_ref(exports).map_err(|_| "invalid exports value")? {
        Value::Object(object) => {
            for (name, value) in object {
                if let Value::Function(function) =
                    value_ref(*value).map_err(|_| "invalid export value")?
                {
                    functions.push((name.clone(), function.clone()));
                    exported_values.push((name.clone(), *value));
                }
            }
        }
        Value::Function(function) => {
            let name = root_name.unwrap_or("default").to_string();
            functions.push((name.clone(), function.clone()));
            exported_values.push((name, exports));
        }
        _ => return Err("addon initialization returned neither an object nor a function".into()),
    }
    HOST.with(|host| {
        let mut host = host.borrow_mut();
        host.functions.extend(functions);
        host.exports.extend(
            exported_values
                .into_iter()
                .map(|(name, value)| (name, (env_ptr as usize, value))),
        );
        #[cfg(target_os = "linux")]
        if !uv_handle.is_null() {
            host.libraries.push(uv_handle);
        }
        host.libraries.push(handle);
        host.module_envs.push(env);
    });
    Ok(())
}

#[no_mangle]
pub unsafe extern "C" fn thaw_napi_load(path: *const c_char) -> u8 {
    match text(path).and_then(|path| load_impl(&path, None)) {
        Ok(()) => 1,
        Err(error) => {
            HOST.with(|host| host.borrow_mut().last_error = error.clone());
            eprintln!("thaw-napi: {error}");
            0
        }
    }
}

#[no_mangle]
pub unsafe extern "C" fn thaw_napi_load_named(path: *const c_char, root_name: *const c_char) -> u8 {
    let result = text(path)
        .and_then(|path| text(root_name).and_then(|root_name| load_impl(&path, Some(&root_name))));
    match result {
        Ok(()) => 1,
        Err(error) => {
            HOST.with(|host| host.borrow_mut().last_error = error.clone());
            eprintln!("thaw-napi: {error}");
            0
        }
    }
}

#[no_mangle]
pub extern "C" fn thaw_napi_unload_all() -> u8 {
    if ACTIVE_ASYNC_WORK.load(Ordering::Acquire) != 0
        || LIVE_THREADSAFE_FUNCTIONS.load(Ordering::Acquire) != 0
    {
        HOST.with(|host| {
            host.borrow_mut().last_error =
                "cannot unload N-API addons while async work or thread-safe functions are active"
                    .into();
        });
        return 0;
    }
    HOST.with(|host| {
        let mut host = host.borrow_mut();
        host.functions.clear();
        host.compiled_callbacks.clear();
        host.pending_call_envs.clear();
        // Cleanup hooks and native finalizers must run while their addon code
        // is still mapped.
        host.module_envs.clear();
        for handle in host.libraries.drain(..).rev() {
            unsafe {
                libc::dlclose(handle);
            }
        }
    });
    1
}

fn decode_hex(input: &str) -> Result<Vec<u8>, String> {
    if !input.len().is_multiple_of(2) {
        return Err("embedded addon hex has an odd length".into());
    }
    input
        .as_bytes()
        .chunks_exact(2)
        .map(|pair| {
            let text = std::str::from_utf8(pair).map_err(|error| error.to_string())?;
            u8::from_str_radix(text, 16).map_err(|error| error.to_string())
        })
        .collect()
}

#[cfg(target_os = "linux")]
fn load_embedded_impl(bytes: &[u8], root_name: Option<&str>) -> Result<(), String> {
    let name = CString::new("thaw-native-addon").unwrap();
    let fd = unsafe { libc::memfd_create(name.as_ptr(), libc::MFD_CLOEXEC) };
    if fd < 0 {
        return Err(format!(
            "memfd_create failed: {}",
            std::io::Error::last_os_error()
        ));
    }
    let mut file = unsafe { std::fs::File::from_raw_fd(fd) };
    file.write_all(bytes)
        .map_err(|error| format!("failed to write embedded addon: {error}"))?;
    unsafe { load_impl(&format!("/proc/self/fd/{fd}"), root_name) }
}

#[cfg(not(target_os = "linux"))]
fn load_embedded_impl(bytes: &[u8], root_name: Option<&str>) -> Result<(), String> {
    let mut file = tempfile::Builder::new()
        .prefix("thaw-native-addon-")
        .suffix(".node")
        .tempfile()
        .map_err(|error| format!("failed to create embedded addon file: {error}"))?;
    file.write_all(bytes)
        .map_err(|error| format!("failed to write embedded addon: {error}"))?;
    file.flush()
        .map_err(|error| format!("failed to flush embedded addon: {error}"))?;
    unsafe { load_impl(&file.path().to_string_lossy(), root_name) }
}

#[no_mangle]
pub unsafe extern "C" fn thaw_napi_load_embedded_hex(
    hex: *const c_char,
    root_name: *const c_char,
) -> u8 {
    let result = text(hex).and_then(|hex| {
        let bytes = decode_hex(&hex)?;
        let root_name = text(root_name)?;
        let root_name = (!root_name.is_empty()).then_some(root_name.as_str());
        load_embedded_impl(&bytes, root_name)
    });
    match result {
        Ok(()) => 1,
        Err(error) => {
            HOST.with(|host| host.borrow_mut().last_error = error.clone());
            eprintln!("thaw-napi: {error}");
            0
        }
    }
}

fn value_from_json(env: &mut Env, json: &JsonValue) -> NapiValue {
    match json {
        JsonValue::Null => env.alloc(Value::Null),
        JsonValue::Bool(value) => env.alloc(Value::Bool(*value)),
        JsonValue::Number(value) => env.alloc(Value::Number(value.as_f64().unwrap_or(0.0))),
        JsonValue::String(value) => env.alloc(Value::String(value.clone())),
        JsonValue::Array(values) => {
            let values = values
                .iter()
                .map(|value| value_from_json(env, value))
                .collect();
            env.alloc(Value::Array(values))
        }
        JsonValue::Object(values) => {
            if values.get("type").and_then(JsonValue::as_str) == Some("Buffer") {
                if let Some(bytes) = values.get("data").and_then(JsonValue::as_array) {
                    return env.alloc(Value::Buffer(
                        bytes
                            .iter()
                            .map(|value| value.as_u64().unwrap_or(0) as u8)
                            .collect(),
                    ));
                }
            }
            let values = values
                .iter()
                .map(|(key, value)| (key.clone(), value_from_json(env, value)))
                .collect();
            env.alloc(Value::Object(values))
        }
    }
}

unsafe fn json_from_value(value: NapiValue) -> Result<JsonValue, String> {
    Ok(match value_ref(value).map_err(|_| "invalid napi_value")? {
        Value::Undefined => JsonValue::Null,
        Value::Null => JsonValue::Null,
        Value::Bool(value) => JsonValue::Bool(*value),
        Value::Number(value) => serde_json::Number::from_f64(*value)
            .map(JsonValue::Number)
            .unwrap_or(JsonValue::Null),
        Value::String(value) | Value::Error(value) | Value::Symbol(value) => {
            JsonValue::String(value.clone())
        }
        Value::Array(values) => JsonValue::Array(
            values
                .iter()
                .map(|value| json_from_value(*value))
                .collect::<Result<_, _>>()?,
        ),
        Value::Object(values) => JsonValue::Object(
            values
                .iter()
                .map(|(key, value)| Ok((key.clone(), json_from_value(*value)?)))
                .collect::<Result<_, String>>()?,
        ),
        Value::Buffer(values) => {
            JsonValue::Array(values.iter().map(|value| JsonValue::from(*value)).collect())
        }
        Value::Function(_) => return Err("cannot JSON-encode a function".into()),
        Value::External(_) => return Err("cannot JSON-encode an external value".into()),
        Value::Promise(state) => match &*state.borrow() {
            PromiseState::Pending => return Err("native addon returned a pending Promise".into()),
            PromiseState::Resolved(value) => json_from_value(*value)?,
            PromiseState::Rejected(value) => {
                let message = match value_ref(*value).map_err(|_| "invalid Promise rejection")? {
                    Value::Error(message) | Value::String(message) => message.clone(),
                    _ => json_from_value(*value)?.to_string(),
                };
                return Err(message);
            }
        },
    })
}

fn wait_for_promise(value: NapiValue) -> Result<NapiValue, String> {
    let state = match unsafe { value_ref(value) } {
        Ok(Value::Promise(state)) => Rc::clone(state),
        _ => return Ok(value),
    };
    loop {
        let settled = {
            let state = state.borrow();
            match *state {
                PromiseState::Pending => None,
                PromiseState::Resolved(value) => return Ok(value),
                PromiseState::Rejected(value) => Some(value),
            }
        };
        if let Some(value) = settled {
            let message = unsafe {
                match value_ref(value).map_err(|_| "invalid Promise rejection")? {
                    Value::Error(message) | Value::String(message) => message.clone(),
                    _ => json_from_value(value)?.to_string(),
                }
            };
            return Err(message);
        }
        if ACTIVE_ASYNC_WORK.load(Ordering::Acquire) == 0 {
            return Err("native addon returned a Promise with no pending work".into());
        }
        thaw_napi_poll_async_work();
        std::thread::sleep(Duration::from_millis(1));
    }
}

unsafe fn call_impl(name: &str, args_json: &str) -> Result<String, String> {
    let args: Vec<JsonValue> = serde_json::from_str(args_json)
        .map_err(|error| format!("invalid argument JSON: {error}"))?;
    let function = HOST
        .with(|host| host.borrow().functions.get(name).cloned())
        .ok_or_else(|| format!("no such native addon function `{name}`"))?;
    let mut env = Box::new(Env::new());
    let args = args
        .iter()
        .map(|value| value_from_json(&mut env, value))
        .collect();
    let this_arg = env.alloc(Value::Undefined);
    let mut info = CallbackInfo {
        args,
        this_arg,
        new_target: ptr::null_mut(),
        data: function.data,
    };
    let result = (function.callback)(&mut *env, &mut info);
    if let Some(exception) = env.exception {
        let message = match value_ref(exception).map_err(|_| "invalid exception")? {
            Value::Error(message) | Value::String(message) => message.clone(),
            _ => json_from_value(exception)?.to_string(),
        };
        return Err(message);
    }
    let result = wait_for_promise(result)?;
    serde_json::to_string(&json_from_value(result)?).map_err(|error| error.to_string())
}

#[no_mangle]
pub unsafe extern "C" fn thaw_napi_call_result(
    name: *const c_char,
    args: *const c_char,
) -> ThawResult {
    let result = text(name).and_then(|name| text(args).and_then(|args| call_impl(&name, &args)));
    match result {
        Ok(value) => ThawResult {
            value: CString::new(value).unwrap().into_raw(),
            error: ptr::null_mut(),
        },
        Err(error) => ThawResult {
            value: ptr::null_mut(),
            error: CString::new(error).unwrap_or_default().into_raw(),
        },
    }
}

#[no_mangle]
pub unsafe extern "C" fn thaw_napi_get_export(name: *const c_char) -> u64 {
    let Ok(name) = text(name) else {
        return 0;
    };
    HOST.with(|host| {
        host.borrow()
            .exports
            .get(&name)
            .map(|(_, value)| *value as u64)
            .unwrap_or(0)
    })
}

fn handle_error(error: impl Into<String>) -> ThawNapiHandleResult {
    ThawNapiHandleResult {
        value: 0,
        error: CString::new(error.into()).unwrap_or_default().into_raw(),
    }
}

unsafe fn module_env_for_handle(handle: u64) -> Result<NapiEnv, String> {
    if handle == 0 {
        return Err("invalid native addon handle 0".into());
    }
    HOST.with(|host| {
        host.borrow()
            .module_envs
            .iter()
            .find(|env| env.values.contains(&(handle as NapiValue)))
            .map(|env| (&**env as *const Env).cast_mut())
            .ok_or_else(|| format!("unknown native addon handle {handle}"))
    })
}

unsafe fn module_arguments(env: NapiEnv, args: *const c_char) -> Result<Vec<NapiValue>, String> {
    let args: Vec<JsonValue> = serde_json::from_str(&text(args)?)
        .map_err(|error| format!("invalid argument JSON: {error}"))?;
    let env = env_mut(env).map_err(|_| "invalid native addon environment")?;
    Ok(args
        .iter()
        .map(|value| value_from_json(env, value))
        .collect())
}

unsafe fn take_env_exception(env: NapiEnv) -> Result<(), String> {
    let Some(exception) = env_mut(env)
        .map_err(|_| "invalid native addon environment")?
        .exception
        .take()
    else {
        return Ok(());
    };
    let message = match value_ref(exception).map_err(|_| "invalid exception")? {
        Value::Error(message) | Value::String(message) => message.clone(),
        _ => json_from_value(exception)?.to_string(),
    };
    Err(message)
}

#[no_mangle]
pub unsafe extern "C" fn thaw_napi_construct_handle_result(
    constructor: u64,
    args: *const c_char,
) -> ThawNapiHandleResult {
    let result = (|| -> Result<u64, String> {
        let env = module_env_for_handle(constructor)?;
        let values = module_arguments(env, args)?;
        let mut instance = ptr::null_mut();
        let status = napi_new_instance(
            env,
            constructor as NapiValue,
            values.len(),
            values.as_ptr(),
            &mut instance,
        );
        take_env_exception(env)?;
        if status != NAPI_OK || instance.is_null() {
            return Err(format!(
                "native addon constructor failed with status {status}"
            ));
        }
        Ok(instance as u64)
    })();
    match result {
        Ok(value) => ThawNapiHandleResult {
            value,
            error: ptr::null_mut(),
        },
        Err(error) => handle_error(error),
    }
}

#[no_mangle]
pub unsafe extern "C" fn thaw_napi_call_method_result(
    receiver: u64,
    method: *const c_char,
    args: *const c_char,
) -> ThawResult {
    let result = (|| -> Result<String, String> {
        let env = module_env_for_handle(receiver)?;
        let method_name = text(method)?;
        let values = module_arguments(env, args)?;
        let mut callable = ptr::null_mut();
        let method_name_c = CString::new(method_name.clone()).map_err(|_| "method contains NUL")?;
        let status = napi_get_named_property(
            env,
            receiver as NapiValue,
            method_name_c.as_ptr(),
            &mut callable,
        );
        if status != NAPI_OK {
            take_env_exception(env)?;
            return Err(format!(
                "failed to get native method `{method_name}`: status {status}"
            ));
        }
        let function = match value_ref(callable).map_err(|_| "invalid native method")? {
            Value::Function(function) => function.clone(),
            _ => return Err(format!("native property `{method_name}` is not callable")),
        };
        let mut info = CallbackInfo {
            args: values,
            this_arg: receiver as NapiValue,
            new_target: ptr::null_mut(),
            data: function.data,
        };
        let value = (function.callback)(env, &mut info);
        take_env_exception(env)?;
        let value = wait_for_promise(value)?;
        serde_json::to_string(&json_from_value(value)?).map_err(|error| error.to_string())
    })();
    match result {
        Ok(value) => ThawResult {
            value: CString::new(value).unwrap_or_default().into_raw(),
            error: ptr::null_mut(),
        },
        Err(error) => ThawResult {
            value: ptr::null_mut(),
            error: CString::new(error).unwrap_or_default().into_raw(),
        },
    }
}

#[no_mangle]
pub unsafe extern "C" fn thaw_napi_get_property_result(
    receiver: u64,
    property: *const c_char,
) -> ThawResult {
    let result = (|| -> Result<String, String> {
        let env = module_env_for_handle(receiver)?;
        let property_name = text(property)?;
        let property_name_c =
            CString::new(property_name.clone()).map_err(|_| "property contains NUL")?;
        let mut value = ptr::null_mut();
        let status = napi_get_named_property(
            env,
            receiver as NapiValue,
            property_name_c.as_ptr(),
            &mut value,
        );
        take_env_exception(env)?;
        if status != NAPI_OK || value.is_null() {
            return Err(format!(
                "failed to get native property `{property_name}`: status {status}"
            ));
        }
        let value = wait_for_promise(value)?;
        serde_json::to_string(&json_from_value(value)?).map_err(|error| error.to_string())
    })();
    match result {
        Ok(value) => ThawResult {
            value: CString::new(value).unwrap_or_default().into_raw(),
            error: ptr::null_mut(),
        },
        Err(error) => ThawResult {
            value: ptr::null_mut(),
            error: CString::new(error).unwrap_or_default().into_raw(),
        },
    }
}

#[no_mangle]
pub unsafe extern "C" fn thaw_napi_set_property_result(
    receiver: u64,
    property: *const c_char,
    args: *const c_char,
) -> ThawResult {
    let result = (|| -> Result<String, String> {
        let env = module_env_for_handle(receiver)?;
        let property_name = text(property)?;
        let property_name_c =
            CString::new(property_name.clone()).map_err(|_| "property contains NUL")?;
        let values = module_arguments(env, args)?;
        let [value] = values.as_slice() else {
            return Err("native property setter expects exactly one value".into());
        };
        let status =
            napi_set_named_property(env, receiver as NapiValue, property_name_c.as_ptr(), *value);
        take_env_exception(env)?;
        if status != NAPI_OK {
            return Err(format!(
                "failed to set native property `{property_name}`: status {status}"
            ));
        }
        serde_json::to_string(&json_from_value(*value)?).map_err(|error| error.to_string())
    })();
    match result {
        Ok(value) => ThawResult {
            value: CString::new(value).unwrap_or_default().into_raw(),
            error: ptr::null_mut(),
        },
        Err(error) => ThawResult {
            value: ptr::null_mut(),
            error: CString::new(error).unwrap_or_default().into_raw(),
        },
    }
}

unsafe extern "C" fn thaw_compiled_callback(_env: NapiEnv, info: NapiCallbackInfo) -> NapiValue {
    let Some(info) = info.as_ref() else {
        return ptr::null_mut();
    };
    let Some(bridge) = (info.data as *const ThawCallbackBridge).as_ref() else {
        return ptr::null_mut();
    };
    let error = info
        .args
        .first()
        .copied()
        .map(|value| json_from_value(value).unwrap_or(JsonValue::Null))
        .unwrap_or(JsonValue::Null);
    let result = info
        .args
        .get(1)
        .copied()
        .map(|value| json_from_value(value).unwrap_or(JsonValue::Null))
        .unwrap_or(JsonValue::Null);
    let error = CString::new(serde_json::to_string(&error).unwrap()).unwrap();
    let result = CString::new(serde_json::to_string(&result).unwrap()).unwrap();
    (bridge.callback)(
        bridge.context as *mut c_void,
        error.as_ptr(),
        result.as_ptr(),
    );
    ptr::null_mut()
}

#[no_mangle]
pub unsafe extern "C" fn thaw_napi_call_with_callback_result(
    name: *const c_char,
    args: *const c_char,
    callback: Option<ThawNativeCallback>,
    context: *mut c_void,
) -> ThawResult {
    let result = (|| -> Result<String, String> {
        let name = text(name)?;
        let args: Vec<JsonValue> = serde_json::from_str(&text(args)?)
            .map_err(|error| format!("invalid argument JSON: {error}"))?;
        let function = HOST
            .with(|host| host.borrow().functions.get(&name).cloned())
            .ok_or_else(|| format!("no such native addon function `{name}`"))?;
        let callback = callback.ok_or("native addon callback is null")?;
        // Generated call sites create a small ABI adapter per invocation,
        // while the closure allocation itself remains stable. Use that
        // closure context as the identity so subscribe/unsubscribe calls made
        // at different source locations still receive the same napi_value.
        let callback_key = (
            context as usize,
            if context.is_null() {
                callback as usize
            } else {
                0
            },
        );
        let mut env = Box::new(Env::new());
        let mut values: Vec<NapiValue> = args
            .iter()
            .map(|value| value_from_json(&mut env, value))
            .collect();
        let cached_callback =
            HOST.with(|host| host.borrow().compiled_callbacks.get(&callback_key).copied());
        let mut created_callback = None;
        let callback_value = if let Some(callback) = cached_callback {
            callback
        } else {
            let bridge = Arc::new(ThawCallbackBridge {
                callback,
                context: context as usize,
            });
            let bridge_data = Arc::as_ptr(&bridge) as *mut c_void;
            let value = env.alloc(Value::Function(Function {
                callback: thaw_compiled_callback,
                data: bridge_data,
                properties: HashMap::new(),
                _thaw_bridge: Some(bridge),
            }));
            created_callback = Some(value);
            value
        };
        values.push(callback_value);
        let this_arg = env.alloc(Value::Undefined);
        let mut info = CallbackInfo {
            args: values,
            this_arg,
            new_target: ptr::null_mut(),
            data: function.data,
        };
        let value = (function.callback)(&mut *env, &mut info);
        if let Some(exception) = env.exception {
            let message = match value_ref(exception).map_err(|_| "invalid exception")? {
                Value::Error(message) | Value::String(message) => message.clone(),
                _ => json_from_value(exception)?.to_string(),
            };
            return Err(message);
        }
        let value = wait_for_promise(value)?;
        let value = if value.is_null() {
            "null".to_string()
        } else {
            serde_json::to_string(&json_from_value(value)?).map_err(|error| error.to_string())?
        };
        if ACTIVE_ASYNC_WORK.load(Ordering::Acquire) != 0
            || LIVE_THREADSAFE_FUNCTIONS.load(Ordering::Acquire) != 0
        {
            if let Some(callback) = created_callback {
                HOST.with(|host| {
                    host.borrow_mut()
                        .compiled_callbacks
                        .insert(callback_key, callback);
                });
            }
            HOST.with(|host| host.borrow_mut().pending_call_envs.push(env));
        } else {
            HOST.with(|host| {
                let mut host = host.borrow_mut();
                host.compiled_callbacks.remove(&callback_key);
                host.pending_call_envs.clear();
            });
        }
        Ok(value)
    })();
    match result {
        Ok(value) => ThawResult {
            value: CString::new(value).unwrap().into_raw(),
            error: ptr::null_mut(),
        },
        Err(error) => ThawResult {
            value: ptr::null_mut(),
            error: CString::new(error).unwrap_or_default().into_raw(),
        },
    }
}

#[no_mangle]
pub unsafe extern "C" fn thaw_napi_call_method_with_callback_result(
    receiver: u64,
    method: *const c_char,
    args: *const c_char,
    callback: Option<ThawNativeCallback>,
    context: *mut c_void,
    discard_result: u8,
) -> ThawResult {
    let result = (|| -> Result<String, String> {
        thaw_napi_run_async_work();
        let env = module_env_for_handle(receiver)?;
        let method_name = text(method)?;
        let args: Vec<JsonValue> = serde_json::from_str(&text(args)?)
            .map_err(|error| format!("invalid argument JSON: {error}"))?;
        let mut callable = ptr::null_mut();
        let method_name_c = CString::new(method_name.clone()).map_err(|_| "method contains NUL")?;
        let status = napi_get_named_property(
            env,
            receiver as NapiValue,
            method_name_c.as_ptr(),
            &mut callable,
        );
        if status != NAPI_OK {
            take_env_exception(env)?;
            return Err(format!(
                "failed to get native method `{method_name}`: status {status}"
            ));
        }
        let function = match value_ref(callable).map_err(|_| "invalid native method")? {
            Value::Function(function) => function.clone(),
            _ => return Err(format!("native property `{method_name}` is not callable")),
        };
        let callback = callback.ok_or("native addon callback is null")?;
        let callback_key = (
            context as usize,
            if context.is_null() {
                callback as usize
            } else {
                0
            },
        );
        let mut values: Vec<NapiValue> = args
            .iter()
            .map(|value| value_from_json(&mut *env, value))
            .collect();
        let cached_callback =
            HOST.with(|host| host.borrow().compiled_callbacks.get(&callback_key).copied());
        let callback_value = if let Some(callback) = cached_callback {
            callback
        } else {
            let bridge = Arc::new(ThawCallbackBridge {
                callback,
                context: context as usize,
            });
            let bridge_data = Arc::as_ptr(&bridge) as *mut c_void;
            let value = (*env).alloc(Value::Function(Function {
                callback: thaw_compiled_callback,
                data: bridge_data,
                properties: HashMap::new(),
                _thaw_bridge: Some(bridge),
            }));
            HOST.with(|host| {
                host.borrow_mut()
                    .compiled_callbacks
                    .insert(callback_key, value);
            });
            value
        };
        values.push(callback_value);
        let mut info = CallbackInfo {
            args: values,
            this_arg: receiver as NapiValue,
            new_target: ptr::null_mut(),
            data: function.data,
        };
        let value = (function.callback)(env, &mut info);
        take_env_exception(env)?;
        let value = wait_for_promise(value)?;
        if discard_result != 0 || value.is_null() {
            Ok("null".to_string())
        } else {
            serde_json::to_string(&json_from_value(value)?).map_err(|error| error.to_string())
        }
    })();
    match result {
        Ok(value) => ThawResult {
            value: CString::new(value).unwrap_or_default().into_raw(),
            error: ptr::null_mut(),
        },
        Err(error) => ThawResult {
            value: ptr::null_mut(),
            error: CString::new(error).unwrap_or_default().into_raw(),
        },
    }
}

#[no_mangle]
pub unsafe extern "C" fn thaw_napi_call(name: *const c_char, args: *const c_char) -> *const c_char {
    let result = thaw_napi_call_result(name, args);
    if result.error.is_null() {
        result.value
    } else {
        let error = CStr::from_ptr(result.error).to_string_lossy();
        CString::new(format!(
            "{{\"__thaw_error__\":{}}}",
            serde_json::to_string(error.as_ref()).unwrap()
        ))
        .unwrap()
        .into_raw()
    }
}

#[no_mangle]
pub unsafe extern "C" fn napi_get_undefined(env: NapiEnv, out: *mut NapiValue) -> NapiStatus {
    let Ok(env) = env_mut(env) else {
        return NAPI_INVALID_ARG;
    };
    let value = env.alloc(Value::Undefined);
    write_value(out, value)
}
#[no_mangle]
pub unsafe extern "C" fn napi_get_null(env: NapiEnv, out: *mut NapiValue) -> NapiStatus {
    let Ok(env) = env_mut(env) else {
        return NAPI_INVALID_ARG;
    };
    let value = env.alloc(Value::Null);
    write_value(out, value)
}
#[no_mangle]
pub unsafe extern "C" fn napi_get_boolean(
    env: NapiEnv,
    value: bool,
    out: *mut NapiValue,
) -> NapiStatus {
    let Ok(env) = env_mut(env) else {
        return NAPI_INVALID_ARG;
    };
    let value = env.alloc(Value::Bool(value));
    write_value(out, value)
}
#[no_mangle]
pub unsafe extern "C" fn napi_create_double(
    env: NapiEnv,
    value: f64,
    out: *mut NapiValue,
) -> NapiStatus {
    let Ok(env) = env_mut(env) else {
        return NAPI_INVALID_ARG;
    };
    let value = env.alloc(Value::Number(value));
    write_value(out, value)
}

#[no_mangle]
pub unsafe extern "C" fn napi_create_string_utf8(
    env: NapiEnv,
    value: *const c_char,
    length: usize,
    out: *mut NapiValue,
) -> NapiStatus {
    if value.is_null() {
        return NAPI_INVALID_ARG;
    }
    let bytes = if length == NAPI_AUTO_LENGTH {
        CStr::from_ptr(value).to_bytes()
    } else {
        std::slice::from_raw_parts(value.cast(), length)
    };
    let string = String::from_utf8_lossy(bytes).into_owned();
    let Ok(env) = env_mut(env) else {
        return NAPI_INVALID_ARG;
    };
    let value = env.alloc(Value::String(string));
    write_value(out, value)
}

#[no_mangle]
pub unsafe extern "C" fn napi_create_string_latin1(
    env: NapiEnv,
    value: *const c_char,
    length: usize,
    out: *mut NapiValue,
) -> NapiStatus {
    if value.is_null() {
        return NAPI_INVALID_ARG;
    }
    let bytes = if length == NAPI_AUTO_LENGTH {
        CStr::from_ptr(value).to_bytes()
    } else {
        std::slice::from_raw_parts(value.cast::<u8>(), length)
    };
    let string: String = bytes.iter().map(|byte| char::from(*byte)).collect();
    let Ok(env) = env_mut(env) else {
        return NAPI_INVALID_ARG;
    };
    let value = env.alloc(Value::String(string));
    write_value(out, value)
}

#[no_mangle]
pub unsafe extern "C" fn napi_create_symbol(
    env: NapiEnv,
    description: NapiValue,
    out: *mut NapiValue,
) -> NapiStatus {
    let description = if description.is_null() {
        String::new()
    } else {
        match value_ref(description) {
            Ok(Value::String(value)) => value.clone(),
            _ => return NAPI_STRING_EXPECTED,
        }
    };
    let Ok(env) = env_mut(env) else {
        return NAPI_INVALID_ARG;
    };
    let value = env.alloc(Value::Symbol(description));
    write_value(out, value)
}

#[no_mangle]
pub unsafe extern "C" fn napi_create_external(
    env: NapiEnv,
    data: *mut c_void,
    _finalize: Option<unsafe extern "C" fn(NapiEnv, *mut c_void, *mut c_void)>,
    _hint: *mut c_void,
    out: *mut NapiValue,
) -> NapiStatus {
    let Ok(env) = env_mut(env) else {
        return NAPI_INVALID_ARG;
    };
    let value = env.alloc(Value::External(data));
    write_value(out, value)
}

#[no_mangle]
pub unsafe extern "C" fn napi_get_value_external(
    _env: NapiEnv,
    value: NapiValue,
    out: *mut *mut c_void,
) -> NapiStatus {
    if out.is_null() {
        return NAPI_INVALID_ARG;
    }
    match value_ref(value) {
        Ok(Value::External(data)) => {
            *out = *data;
            NAPI_OK
        }
        _ => NAPI_INVALID_ARG,
    }
}

#[no_mangle]
pub unsafe extern "C" fn napi_create_object(env: NapiEnv, out: *mut NapiValue) -> NapiStatus {
    let Ok(env) = env_mut(env) else {
        return NAPI_INVALID_ARG;
    };
    let value = env.alloc(Value::Object(HashMap::new()));
    write_value(out, value)
}
#[no_mangle]
pub unsafe extern "C" fn napi_create_array(env: NapiEnv, out: *mut NapiValue) -> NapiStatus {
    napi_create_array_with_length(env, 0, out)
}
#[no_mangle]
pub unsafe extern "C" fn napi_create_array_with_length(
    env: NapiEnv,
    length: usize,
    out: *mut NapiValue,
) -> NapiStatus {
    let Ok(env) = env_mut(env) else {
        return NAPI_INVALID_ARG;
    };
    let undefined = env.alloc(Value::Undefined);
    let value = env.alloc(Value::Array(vec![undefined; length]));
    write_value(out, value)
}

#[no_mangle]
pub unsafe extern "C" fn napi_create_function(
    env: NapiEnv,
    _name: *const c_char,
    _length: usize,
    callback: Option<NapiCallback>,
    data: *mut c_void,
    out: *mut NapiValue,
) -> NapiStatus {
    let Some(callback) = callback else {
        return NAPI_INVALID_ARG;
    };
    let Ok(env) = env_mut(env) else {
        return NAPI_INVALID_ARG;
    };
    let value = env.alloc(Value::Function(Function {
        callback,
        data,
        properties: HashMap::new(),
        _thaw_bridge: None,
    }));
    write_value(out, value)
}

#[no_mangle]
pub unsafe extern "C" fn napi_get_cb_info(
    _env: NapiEnv,
    info: NapiCallbackInfo,
    argc: *mut usize,
    argv: *mut NapiValue,
    this_arg: *mut NapiValue,
    data: *mut *mut c_void,
) -> NapiStatus {
    let Some(info) = info.as_ref() else {
        return NAPI_INVALID_ARG;
    };
    if !argc.is_null() {
        let capacity = *argc;
        *argc = info.args.len();
        if !argv.is_null() {
            ptr::copy_nonoverlapping(info.args.as_ptr(), argv, capacity.min(info.args.len()));
        }
    }
    if !this_arg.is_null() {
        *this_arg = info.this_arg;
    }
    if !data.is_null() {
        *data = info.data;
    }
    NAPI_OK
}

#[no_mangle]
pub unsafe extern "C" fn napi_get_new_target(
    _env: NapiEnv,
    info: NapiCallbackInfo,
    result: *mut NapiValue,
) -> NapiStatus {
    if result.is_null() {
        return NAPI_INVALID_ARG;
    }
    let Some(info) = info.as_ref() else {
        return NAPI_INVALID_ARG;
    };
    *result = info.new_target;
    NAPI_OK
}

#[no_mangle]
pub unsafe extern "C" fn napi_set_named_property(
    env: NapiEnv,
    object: NapiValue,
    name: *const c_char,
    value: NapiValue,
) -> NapiStatus {
    let Ok(name) = text(name) else {
        return NAPI_INVALID_ARG;
    };
    let accessor = find_accessor(env, object, &name);
    if let Some(setter) = accessor.and_then(|accessor| accessor.setter) {
        let mut info = CallbackInfo {
            args: vec![value],
            this_arg: object,
            new_target: ptr::null_mut(),
            data: accessor.unwrap().data,
        };
        setter(env, &mut info);
        return if env_mut(env)
            .map(|env| env.exception.is_some())
            .unwrap_or(false)
        {
            NAPI_PENDING_EXCEPTION
        } else {
            NAPI_OK
        };
    }
    match object.as_mut() {
        Some(Value::Object(values)) => {
            values.insert(name, value);
            NAPI_OK
        }
        Some(Value::Function(function)) => {
            function.properties.insert(name, value);
            NAPI_OK
        }
        _ => NAPI_INVALID_ARG,
    }
}

#[no_mangle]
pub unsafe extern "C" fn napi_get_named_property(
    env: NapiEnv,
    object: NapiValue,
    name: *const c_char,
    out: *mut NapiValue,
) -> NapiStatus {
    let Ok(name) = text(name) else {
        return NAPI_INVALID_ARG;
    };
    let value = match value_ref(object) {
        Ok(Value::Object(values)) => values.get(&name).copied(),
        Ok(Value::Function(function)) => function.properties.get(&name).copied(),
        _ => None,
    };
    if value.is_none() {
        let accessor = find_accessor(env, object, &name);
        if let Some(getter) = accessor.and_then(|accessor| accessor.getter) {
            let mut info = CallbackInfo {
                args: Vec::new(),
                this_arg: object,
                new_target: ptr::null_mut(),
                data: accessor.unwrap().data,
            };
            let value = getter(env, &mut info);
            if env_mut(env)
                .map(|env| env.exception.is_some())
                .unwrap_or(false)
            {
                return NAPI_PENDING_EXCEPTION;
            }
            return write_value(out, value);
        }
    }
    match value {
        Some(value) => write_value(out, value),
        None => napi_get_undefined(env, out),
    }
}

unsafe fn property_key(value: NapiValue) -> Result<String, NapiStatus> {
    match value_ref(value) {
        Ok(Value::String(value) | Value::Symbol(value)) => Ok(value.clone()),
        _ => Err(NAPI_STRING_EXPECTED),
    }
}

#[no_mangle]
pub unsafe extern "C" fn napi_set_property(
    env: NapiEnv,
    object: NapiValue,
    key: NapiValue,
    value: NapiValue,
) -> NapiStatus {
    let Ok(key) = property_key(key) else {
        return NAPI_INVALID_ARG;
    };
    let key = match CString::new(key) {
        Ok(key) => key,
        Err(_) => return NAPI_INVALID_ARG,
    };
    napi_set_named_property(env, object, key.as_ptr(), value)
}

#[no_mangle]
pub unsafe extern "C" fn napi_get_property(
    env: NapiEnv,
    object: NapiValue,
    key: NapiValue,
    out: *mut NapiValue,
) -> NapiStatus {
    let Ok(key) = property_key(key) else {
        return NAPI_INVALID_ARG;
    };
    let key = match CString::new(key) {
        Ok(key) => key,
        Err(_) => return NAPI_INVALID_ARG,
    };
    napi_get_named_property(env, object, key.as_ptr(), out)
}

#[no_mangle]
pub unsafe extern "C" fn napi_has_property(
    env: NapiEnv,
    object: NapiValue,
    key: NapiValue,
    out: *mut bool,
) -> NapiStatus {
    if out.is_null() {
        return NAPI_INVALID_ARG;
    }
    let Ok(key) = property_key(key) else {
        return NAPI_INVALID_ARG;
    };
    *out = match value_ref(object) {
        Ok(Value::Object(values)) => values.contains_key(&key),
        Ok(Value::Function(function)) => function.properties.contains_key(&key),
        _ => false,
    } || find_accessor(env, object, &key).is_some();
    NAPI_OK
}

#[no_mangle]
pub unsafe extern "C" fn napi_has_named_property(
    env: NapiEnv,
    object: NapiValue,
    name: *const c_char,
    result: *mut bool,
) -> NapiStatus {
    if name.is_null() || result.is_null() {
        return NAPI_INVALID_ARG;
    }
    let Ok(name) = text(name) else {
        return NAPI_INVALID_ARG;
    };
    let Ok(env) = env_mut(env) else {
        return NAPI_INVALID_ARG;
    };
    *result = match value_ref(object) {
        Ok(Value::Object(properties)) => properties.contains_key(&name),
        Ok(Value::Function(function)) => function.properties.contains_key(&name),
        _ => false,
    } || env.accessors.contains_key(&(object as usize, name));
    NAPI_OK
}

#[no_mangle]
pub unsafe extern "C" fn napi_define_properties(
    env: NapiEnv,
    object: NapiValue,
    count: usize,
    descriptors: *const NapiPropertyDescriptor,
) -> NapiStatus {
    if count == 0 {
        return NAPI_OK;
    }
    if descriptors.is_null() {
        return NAPI_INVALID_ARG;
    }
    for descriptor in std::slice::from_raw_parts(descriptors, count) {
        let key = if !descriptor.utf8name.is_null() {
            env_mut(env).map(|env| {
                env.alloc(Value::String(
                    CStr::from_ptr(descriptor.utf8name)
                        .to_string_lossy()
                        .into_owned(),
                ))
            })
        } else {
            Ok(descriptor.name)
        };
        let Ok(key) = key else {
            return NAPI_INVALID_ARG;
        };
        if descriptor.getter.is_some() || descriptor.setter.is_some() {
            let Ok(key_name) = property_key(key) else {
                return NAPI_INVALID_ARG;
            };
            let Ok(env) = env_mut(env) else {
                return NAPI_INVALID_ARG;
            };
            env.accessors.insert(
                (object as usize, key_name),
                Accessor {
                    getter: descriptor.getter,
                    setter: descriptor.setter,
                    data: descriptor.data,
                },
            );
            continue;
        }
        let value = if let Some(method) = descriptor.method {
            let mut value = ptr::null_mut();
            let status = napi_create_function(
                env,
                descriptor.utf8name,
                NAPI_AUTO_LENGTH,
                Some(method),
                descriptor.data,
                &mut value,
            );
            if status != NAPI_OK {
                return status;
            }
            value
        } else {
            descriptor.value
        };
        let status = napi_set_property(env, object, key, value);
        if status != NAPI_OK {
            return status;
        }
    }
    NAPI_OK
}

#[no_mangle]
pub unsafe extern "C" fn napi_wrap(
    env: NapiEnv,
    object: NapiValue,
    data: *mut c_void,
    finalize: Option<NapiFinalize>,
    hint: *mut c_void,
    result: *mut *mut Reference,
) -> NapiStatus {
    let Ok(env) = env_mut(env) else {
        return NAPI_INVALID_ARG;
    };
    if !matches!(object.as_ref(), Some(Value::Object(_) | Value::Function(_))) {
        return NAPI_INVALID_ARG;
    }
    if env.wraps.contains_key(&(object as usize)) {
        return NAPI_GENERIC_FAILURE;
    }
    env.wraps.insert(
        object as usize,
        WrapRecord {
            data,
            finalize,
            hint,
        },
    );
    if !result.is_null() {
        *result = Box::into_raw(Box::new(Reference {
            value: object,
            count: 0,
        }));
    }
    NAPI_OK
}

#[no_mangle]
pub unsafe extern "C" fn napi_unwrap(
    env: NapiEnv,
    object: NapiValue,
    result: *mut *mut c_void,
) -> NapiStatus {
    if result.is_null() {
        return NAPI_INVALID_ARG;
    }
    let Ok(env) = env_mut(env) else {
        return NAPI_INVALID_ARG;
    };
    let Some(wrap) = env.wraps.get(&(object as usize)) else {
        return NAPI_INVALID_ARG;
    };
    *result = wrap.data;
    NAPI_OK
}

#[no_mangle]
pub unsafe extern "C" fn napi_remove_wrap(
    env: NapiEnv,
    object: NapiValue,
    result: *mut *mut c_void,
) -> NapiStatus {
    let Ok(env) = env_mut(env) else {
        return NAPI_INVALID_ARG;
    };
    let Some(wrap) = env.wraps.remove(&(object as usize)) else {
        return NAPI_INVALID_ARG;
    };
    if !result.is_null() {
        *result = wrap.data;
    }
    NAPI_OK
}

#[no_mangle]
pub unsafe extern "C" fn napi_add_finalizer(
    env: NapiEnv,
    object: NapiValue,
    data: *mut c_void,
    finalize: Option<NapiFinalize>,
    hint: *mut c_void,
    result: *mut *mut Reference,
) -> NapiStatus {
    if !matches!(object.as_ref(), Some(Value::Object(_) | Value::Function(_))) || finalize.is_none()
    {
        return NAPI_INVALID_ARG;
    }
    let Ok(env) = env_mut(env) else {
        return NAPI_INVALID_ARG;
    };
    env.finalizers.push(FinalizeRecord {
        data,
        finalize,
        hint,
    });
    if !result.is_null() {
        *result = Box::into_raw(Box::new(Reference {
            value: object,
            count: 0,
        }));
    }
    NAPI_OK
}

#[no_mangle]
pub unsafe extern "C" fn napi_set_instance_data(
    env: NapiEnv,
    data: *mut c_void,
    finalize: Option<NapiFinalize>,
    hint: *mut c_void,
) -> NapiStatus {
    let Ok(env) = env_mut(env) else {
        return NAPI_INVALID_ARG;
    };
    if env.instance_data.is_some() {
        return NAPI_GENERIC_FAILURE;
    }
    env.instance_data = Some(FinalizeRecord {
        data,
        finalize,
        hint,
    });
    NAPI_OK
}

#[no_mangle]
pub unsafe extern "C" fn napi_get_instance_data(
    env: NapiEnv,
    result: *mut *mut c_void,
) -> NapiStatus {
    if result.is_null() {
        return NAPI_INVALID_ARG;
    }
    let Ok(env) = env_mut(env) else {
        return NAPI_INVALID_ARG;
    };
    *result = env
        .instance_data
        .as_ref()
        .map_or(ptr::null_mut(), |record| record.data);
    NAPI_OK
}

#[no_mangle]
pub unsafe extern "C" fn napi_add_env_cleanup_hook(
    env: NapiEnv,
    hook: Option<NapiCleanupHook>,
    data: *mut c_void,
) -> NapiStatus {
    let (Ok(env), Some(hook)) = (env_mut(env), hook) else {
        return NAPI_INVALID_ARG;
    };
    if env
        .cleanup_hooks
        .iter()
        .any(|record| record.hook as usize == hook as usize && record.data == data)
    {
        return NAPI_INVALID_ARG;
    }
    env.cleanup_hooks.push(CleanupHookRecord { hook, data });
    NAPI_OK
}

#[no_mangle]
pub unsafe extern "C" fn napi_remove_env_cleanup_hook(
    env: NapiEnv,
    hook: Option<NapiCleanupHook>,
    data: *mut c_void,
) -> NapiStatus {
    let (Ok(env), Some(hook)) = (env_mut(env), hook) else {
        return NAPI_INVALID_ARG;
    };
    let Some(index) = env
        .cleanup_hooks
        .iter()
        .position(|record| record.hook as usize == hook as usize && record.data == data)
    else {
        return NAPI_INVALID_ARG;
    };
    env.cleanup_hooks.remove(index);
    NAPI_OK
}

#[no_mangle]
pub unsafe extern "C" fn napi_define_class(
    env: NapiEnv,
    name: *const c_char,
    length: usize,
    constructor: Option<NapiCallback>,
    data: *mut c_void,
    property_count: usize,
    properties: *const NapiPropertyDescriptor,
    result: *mut NapiValue,
) -> NapiStatus {
    if env.is_null()
        || name.is_null()
        || constructor.is_none()
        || result.is_null()
        || (property_count != 0 && properties.is_null())
    {
        return NAPI_INVALID_ARG;
    }
    let status = napi_create_function(env, name, length, constructor, data, result);
    if status != NAPI_OK {
        return status;
    }
    let mut prototype = ptr::null_mut();
    let status = napi_create_object(env, &mut prototype);
    if status != NAPI_OK {
        return status;
    }
    let prototype_name = c"prototype";
    let status = napi_set_named_property(env, *result, prototype_name.as_ptr(), prototype);
    if status != NAPI_OK {
        return status;
    }
    if property_count != 0 {
        for descriptor in std::slice::from_raw_parts(properties, property_count) {
            const NAPI_STATIC: u32 = 1 << 10;
            let target = if descriptor.attributes & NAPI_STATIC != 0 {
                *result
            } else {
                prototype
            };
            let status = napi_define_properties(env, target, 1, descriptor);
            if status != NAPI_OK {
                return status;
            }
        }
    }
    NAPI_OK
}

#[no_mangle]
pub unsafe extern "C" fn napi_new_instance(
    env: NapiEnv,
    constructor: NapiValue,
    argc: usize,
    argv: *const NapiValue,
    result: *mut NapiValue,
) -> NapiStatus {
    if result.is_null() || (argc != 0 && argv.is_null()) {
        return NAPI_INVALID_ARG;
    }
    let function = match value_ref(constructor) {
        Ok(Value::Function(function)) => function.clone(),
        _ => return NAPI_INVALID_ARG,
    };
    let Ok(host_env) = env_mut(env) else {
        return NAPI_INVALID_ARG;
    };
    let prototype = function.properties.get("prototype").copied();
    let properties = prototype
        .and_then(|value| match value_ref(value) {
            Ok(Value::Object(properties)) => Some(properties.clone()),
            _ => None,
        })
        .unwrap_or_default();
    let prototype_accessors = prototype
        .map(|prototype| find_accessors(env, prototype))
        .unwrap_or_default();
    let instance = host_env.alloc(Value::Object(properties));
    host_env
        .instances
        .insert(instance as usize, constructor as usize);
    for (name, accessor) in prototype_accessors {
        host_env
            .accessors
            .insert((instance as usize, name), accessor);
    }
    let args = if argc == 0 {
        Vec::new()
    } else {
        std::slice::from_raw_parts(argv, argc).to_vec()
    };
    let mut info = CallbackInfo {
        args,
        this_arg: instance,
        new_target: constructor,
        data: function.data,
    };
    let returned = (function.callback)(env, &mut info);
    if env_mut(env)
        .map(|env| env.exception.is_some())
        .unwrap_or(false)
    {
        return NAPI_PENDING_EXCEPTION;
    }
    *result = match returned.as_ref() {
        Some(Value::Object(_) | Value::Function(_)) => returned,
        _ => instance,
    };
    NAPI_OK
}

#[no_mangle]
pub unsafe extern "C" fn napi_instanceof(
    env: NapiEnv,
    object: NapiValue,
    constructor: NapiValue,
    result: *mut bool,
) -> NapiStatus {
    if result.is_null() {
        return NAPI_INVALID_ARG;
    }
    let Ok(env) = env_mut(env) else {
        return NAPI_INVALID_ARG;
    };
    *result = env.instances.get(&(object as usize)).copied() == Some(constructor as usize);
    NAPI_OK
}

#[no_mangle]
pub unsafe extern "C" fn napi_strict_equals(
    _env: NapiEnv,
    left: NapiValue,
    right: NapiValue,
    result: *mut bool,
) -> NapiStatus {
    if left.is_null() || right.is_null() || result.is_null() {
        return NAPI_INVALID_ARG;
    }
    *result = match (value_ref(left), value_ref(right)) {
        (Ok(Value::Undefined), Ok(Value::Undefined)) | (Ok(Value::Null), Ok(Value::Null)) => true,
        (Ok(Value::Bool(left)), Ok(Value::Bool(right))) => left == right,
        (Ok(Value::Number(left)), Ok(Value::Number(right))) => left == right,
        (Ok(Value::String(left)), Ok(Value::String(right))) => left == right,
        (Ok(_), Ok(_)) => left == right,
        _ => false,
    };
    NAPI_OK
}

#[no_mangle]
pub unsafe extern "C" fn napi_get_value_double(
    _env: NapiEnv,
    value: NapiValue,
    out: *mut f64,
) -> NapiStatus {
    if out.is_null() {
        return NAPI_INVALID_ARG;
    }
    match value_ref(value) {
        Ok(Value::Number(number)) => {
            *out = *number;
            NAPI_OK
        }
        _ => NAPI_NUMBER_EXPECTED,
    }
}
#[no_mangle]
pub unsafe extern "C" fn napi_get_value_int32(
    _env: NapiEnv,
    value: NapiValue,
    out: *mut i32,
) -> NapiStatus {
    if out.is_null() {
        return NAPI_INVALID_ARG;
    }
    match value_ref(value) {
        Ok(Value::Number(number)) => {
            *out = *number as i32;
            NAPI_OK
        }
        _ => NAPI_NUMBER_EXPECTED,
    }
}

#[no_mangle]
pub unsafe extern "C" fn napi_get_value_uint32(
    _env: NapiEnv,
    value: NapiValue,
    out: *mut u32,
) -> NapiStatus {
    if out.is_null() {
        return NAPI_INVALID_ARG;
    }
    match value_ref(value) {
        Ok(Value::Number(number)) => {
            *out = *number as u32;
            NAPI_OK
        }
        _ => NAPI_NUMBER_EXPECTED,
    }
}

#[no_mangle]
pub unsafe extern "C" fn napi_get_value_int64(
    _env: NapiEnv,
    value: NapiValue,
    out: *mut i64,
) -> NapiStatus {
    if out.is_null() {
        return NAPI_INVALID_ARG;
    }
    match value_ref(value) {
        Ok(Value::Number(number)) => {
            *out = *number as i64;
            NAPI_OK
        }
        _ => NAPI_NUMBER_EXPECTED,
    }
}

#[no_mangle]
pub unsafe extern "C" fn napi_coerce_to_bool(
    env: NapiEnv,
    value: NapiValue,
    out: *mut NapiValue,
) -> NapiStatus {
    let boolean = match value_ref(value) {
        Ok(Value::Undefined | Value::Null) => false,
        Ok(Value::Bool(value)) => *value,
        Ok(Value::Number(value)) => *value != 0.0 && !value.is_nan(),
        Ok(Value::String(value)) => !value.is_empty(),
        Ok(_) => true,
        Err(status) => return status,
    };
    napi_get_boolean(env, boolean, out)
}

#[no_mangle]
pub unsafe extern "C" fn napi_coerce_to_number(
    env: NapiEnv,
    value: NapiValue,
    out: *mut NapiValue,
) -> NapiStatus {
    let number = match value_ref(value) {
        Ok(Value::Undefined) => f64::NAN,
        Ok(Value::Null) => 0.0,
        Ok(Value::Bool(value)) => u8::from(*value) as f64,
        Ok(Value::Number(value)) => *value,
        Ok(Value::String(value)) => value.trim().parse().unwrap_or(f64::NAN),
        Ok(_) => f64::NAN,
        Err(status) => return status,
    };
    napi_create_double(env, number, out)
}

#[no_mangle]
pub unsafe extern "C" fn napi_coerce_to_string(
    env: NapiEnv,
    value: NapiValue,
    out: *mut NapiValue,
) -> NapiStatus {
    let string = match value_ref(value) {
        Ok(Value::Undefined) => "undefined".into(),
        Ok(Value::Null) => "null".into(),
        Ok(Value::Bool(value)) => value.to_string(),
        Ok(Value::Number(value)) => value.to_string(),
        Ok(Value::String(value) | Value::Symbol(value) | Value::Error(value)) => value.clone(),
        Ok(Value::Array(_)) => "".into(),
        Ok(_) => "[object Object]".into(),
        Err(status) => return status,
    };
    let Ok(env) = env_mut(env) else {
        return NAPI_INVALID_ARG;
    };
    let value = env.alloc(Value::String(string));
    write_value(out, value)
}

#[no_mangle]
pub unsafe extern "C" fn napi_coerce_to_object(
    env: NapiEnv,
    value: NapiValue,
    out: *mut NapiValue,
) -> NapiStatus {
    if matches!(value_ref(value), Ok(Value::Object(_) | Value::Function(_))) {
        return write_value(out, value);
    }
    if matches!(value_ref(value), Ok(Value::Undefined | Value::Null)) {
        return NAPI_INVALID_ARG;
    }
    let Ok(env) = env_mut(env) else {
        return NAPI_INVALID_ARG;
    };
    let object = env.alloc(Value::Object(HashMap::from([("value".into(), value)])));
    write_value(out, object)
}
#[no_mangle]
pub unsafe extern "C" fn napi_get_value_bool(
    _env: NapiEnv,
    value: NapiValue,
    out: *mut bool,
) -> NapiStatus {
    if out.is_null() {
        return NAPI_INVALID_ARG;
    }
    match value_ref(value) {
        Ok(Value::Bool(boolean)) => {
            *out = *boolean;
            NAPI_OK
        }
        _ => NAPI_BOOLEAN_EXPECTED,
    }
}

#[no_mangle]
pub unsafe extern "C" fn napi_get_value_string_utf8(
    _env: NapiEnv,
    value: NapiValue,
    buffer: *mut c_char,
    size: usize,
    written: *mut usize,
) -> NapiStatus {
    let string = match value_ref(value) {
        Ok(Value::String(string)) => string,
        _ => return NAPI_STRING_EXPECTED,
    };
    if !written.is_null() {
        *written = string.len();
    }
    if !buffer.is_null() && size > 0 {
        let count = string.len().min(size - 1);
        ptr::copy_nonoverlapping(string.as_ptr(), buffer.cast(), count);
        *buffer.add(count) = 0;
    }
    NAPI_OK
}

#[no_mangle]
pub unsafe extern "C" fn napi_throw_error(
    env: NapiEnv,
    _code: *const c_char,
    message: *const c_char,
) -> NapiStatus {
    let message = text(message).unwrap_or_else(|_| "native addon error".into());
    let Ok(env) = env_mut(env) else {
        return NAPI_INVALID_ARG;
    };
    let error = env.alloc(Value::Error(message));
    env.exception = Some(error);
    NAPI_OK
}
#[no_mangle]
pub unsafe extern "C" fn napi_throw_type_error(
    env: NapiEnv,
    code: *const c_char,
    message: *const c_char,
) -> NapiStatus {
    napi_throw_error(env, code, message)
}
#[no_mangle]
pub unsafe extern "C" fn napi_is_exception_pending(env: NapiEnv, out: *mut bool) -> NapiStatus {
    if out.is_null() {
        return NAPI_INVALID_ARG;
    }
    let Ok(env) = env_mut(env) else {
        return NAPI_INVALID_ARG;
    };
    *out = env.exception.is_some();
    NAPI_OK
}
#[no_mangle]
pub unsafe extern "C" fn napi_get_and_clear_last_exception(
    env: NapiEnv,
    out: *mut NapiValue,
) -> NapiStatus {
    let Ok(env) = env_mut(env) else {
        return NAPI_INVALID_ARG;
    };
    let value = env
        .exception
        .take()
        .unwrap_or_else(|| env.alloc(Value::Undefined));
    write_value(out, value)
}
#[no_mangle]
pub unsafe extern "C" fn napi_get_version(_env: NapiEnv, out: *mut u32) -> NapiStatus {
    if out.is_null() {
        NAPI_INVALID_ARG
    } else {
        *out = 8;
        NAPI_OK
    }
}

#[no_mangle]
pub unsafe extern "C" fn napi_get_global(env: NapiEnv, out: *mut NapiValue) -> NapiStatus {
    let Ok(env) = env_mut(env) else {
        return NAPI_INVALID_ARG;
    };
    if out.is_null() {
        return NAPI_INVALID_ARG;
    }
    if env.global.is_null() {
        env.global = env.alloc(Value::Object(HashMap::new()));
    }
    *out = env.global;
    NAPI_OK
}

#[no_mangle]
pub unsafe extern "C" fn napi_get_property_names(
    env: NapiEnv,
    object: NapiValue,
    out: *mut NapiValue,
) -> NapiStatus {
    let Ok(env) = env_mut(env) else {
        return NAPI_INVALID_ARG;
    };
    if out.is_null() {
        return NAPI_INVALID_ARG;
    }
    let mut names = match value_ref(object) {
        Ok(Value::Object(properties)) => properties.keys().cloned().collect::<Vec<_>>(),
        Ok(Value::Function(function)) => function.properties.keys().cloned().collect::<Vec<_>>(),
        Ok(Value::Array(values)) => (0..values.len()).map(|index| index.to_string()).collect(),
        _ => return NAPI_INVALID_ARG,
    };
    names.sort();
    let names = names
        .into_iter()
        .map(|name| env.alloc(Value::String(name)))
        .collect();
    let result = env.alloc(Value::Array(names));
    write_value(out, result)
}

#[no_mangle]
pub unsafe extern "C" fn napi_get_uv_event_loop(
    _env: NapiEnv,
    out: *mut *mut c_void,
) -> NapiStatus {
    if out.is_null() {
        return NAPI_INVALID_ARG;
    }
    #[cfg(target_os = "linux")]
    {
        let symbol = libc::dlsym(libc::RTLD_DEFAULT, c"uv_default_loop".as_ptr());
        if symbol.is_null() {
            return NAPI_GENERIC_FAILURE;
        }
        let default_loop =
            std::mem::transmute::<*mut c_void, unsafe extern "C" fn() -> *mut c_void>(symbol);
        *out = default_loop();
        if (*out).is_null() {
            return NAPI_GENERIC_FAILURE;
        }
        NAPI_OK
    }
    #[cfg(not(target_os = "linux"))]
    {
        *out = ptr::null_mut();
        NAPI_GENERIC_FAILURE
    }
}

#[no_mangle]
pub unsafe extern "C" fn napi_typeof(_env: NapiEnv, value: NapiValue, out: *mut i32) -> NapiStatus {
    if out.is_null() {
        return NAPI_INVALID_ARG;
    }
    *out = match value_ref(value) {
        Ok(Value::Undefined) => 0,
        Ok(Value::Null) => 1,
        Ok(Value::Bool(_)) => 2,
        Ok(Value::Number(_)) => 3,
        Ok(Value::String(_)) => 4,
        Ok(Value::Symbol(_)) => 5,
        Ok(Value::Function(_)) => 7,
        Ok(_) => 6,
        Err(_) => return NAPI_INVALID_ARG,
    };
    NAPI_OK
}

#[no_mangle]
pub unsafe extern "C" fn napi_is_array(
    _env: NapiEnv,
    value: NapiValue,
    out: *mut bool,
) -> NapiStatus {
    if out.is_null() {
        return NAPI_INVALID_ARG;
    }
    *out = matches!(value_ref(value), Ok(Value::Array(_)));
    NAPI_OK
}

#[no_mangle]
pub unsafe extern "C" fn napi_get_array_length(
    _env: NapiEnv,
    value: NapiValue,
    out: *mut u32,
) -> NapiStatus {
    if out.is_null() {
        return NAPI_INVALID_ARG;
    }
    match value_ref(value) {
        Ok(Value::Array(values)) => {
            *out = values.len() as u32;
            NAPI_OK
        }
        _ => NAPI_INVALID_ARG,
    }
}

#[no_mangle]
pub unsafe extern "C" fn napi_set_element(
    env: NapiEnv,
    array: NapiValue,
    index: u32,
    value: NapiValue,
) -> NapiStatus {
    let Ok(host_env) = env_mut(env) else {
        return NAPI_INVALID_ARG;
    };
    match array.as_mut() {
        Some(Value::Array(values)) => {
            while values.len() <= index as usize {
                values.push(host_env.alloc(Value::Undefined));
            }
            values[index as usize] = value;
            NAPI_OK
        }
        _ => NAPI_INVALID_ARG,
    }
}

#[no_mangle]
pub unsafe extern "C" fn napi_get_element(
    env: NapiEnv,
    array: NapiValue,
    index: u32,
    out: *mut NapiValue,
) -> NapiStatus {
    let value = match value_ref(array) {
        Ok(Value::Array(values)) => values.get(index as usize).copied(),
        _ => None,
    };
    match value {
        Some(value) => write_value(out, value),
        None => napi_get_undefined(env, out),
    }
}

#[no_mangle]
pub unsafe extern "C" fn napi_create_buffer(
    env: NapiEnv,
    length: usize,
    data: *mut *mut c_void,
    out: *mut NapiValue,
) -> NapiStatus {
    let Ok(env) = env_mut(env) else {
        return NAPI_INVALID_ARG;
    };
    let value = env.alloc(Value::Buffer(vec![0; length]));
    if !data.is_null() {
        if let Value::Buffer(bytes) = &mut *value {
            *data = bytes.as_mut_ptr().cast();
        }
    }
    write_value(out, value)
}

#[no_mangle]
pub unsafe extern "C" fn napi_create_buffer_copy(
    env: NapiEnv,
    length: usize,
    source: *const c_void,
    data: *mut *mut c_void,
    out: *mut NapiValue,
) -> NapiStatus {
    if source.is_null() && length != 0 {
        return NAPI_INVALID_ARG;
    }
    let status = napi_create_buffer(env, length, data, out);
    if status == NAPI_OK && length != 0 {
        let target = if data.is_null() {
            match (*out).as_mut() {
                Some(Value::Buffer(bytes)) => bytes.as_mut_ptr().cast(),
                _ => return NAPI_INVALID_ARG,
            }
        } else {
            *data
        };
        ptr::copy_nonoverlapping(source.cast::<u8>(), target.cast::<u8>(), length);
    }
    status
}

#[no_mangle]
pub unsafe extern "C" fn napi_get_buffer_info(
    _env: NapiEnv,
    value: NapiValue,
    data: *mut *mut c_void,
    length: *mut usize,
) -> NapiStatus {
    match value.as_mut() {
        Some(Value::Buffer(bytes)) => {
            if !data.is_null() {
                *data = bytes.as_mut_ptr().cast();
            }
            if !length.is_null() {
                *length = bytes.len();
            }
            NAPI_OK
        }
        _ => NAPI_INVALID_ARG,
    }
}

#[no_mangle]
pub unsafe extern "C" fn napi_is_buffer(
    _env: NapiEnv,
    value: NapiValue,
    out: *mut bool,
) -> NapiStatus {
    if out.is_null() {
        return NAPI_INVALID_ARG;
    }
    *out = matches!(value_ref(value), Ok(Value::Buffer(_)));
    NAPI_OK
}

#[no_mangle]
pub unsafe extern "C" fn napi_get_typedarray_info(
    _env: NapiEnv,
    value: NapiValue,
    array_type: *mut i32,
    length: *mut usize,
    data: *mut *mut c_void,
    array_buffer: *mut NapiValue,
    byte_offset: *mut usize,
) -> NapiStatus {
    let Some(Value::Buffer(bytes)) = value.as_mut() else {
        return NAPI_INVALID_ARG;
    };
    if !array_type.is_null() {
        *array_type = 1;
    }
    if !length.is_null() {
        *length = bytes.len();
    }
    if !data.is_null() {
        *data = bytes.as_mut_ptr().cast();
    }
    if !array_buffer.is_null() {
        *array_buffer = value;
    }
    if !byte_offset.is_null() {
        *byte_offset = 0;
    }
    NAPI_OK
}

#[no_mangle]
pub unsafe extern "C" fn napi_create_reference(
    _env: NapiEnv,
    value: NapiValue,
    initial_count: u32,
    out: *mut *mut Reference,
) -> NapiStatus {
    if value.is_null() || out.is_null() {
        return NAPI_INVALID_ARG;
    }
    *out = Box::into_raw(Box::new(Reference {
        value,
        count: initial_count,
    }));
    NAPI_OK
}

#[no_mangle]
pub unsafe extern "C" fn napi_delete_reference(
    _env: NapiEnv,
    reference: *mut Reference,
) -> NapiStatus {
    if reference.is_null() {
        return NAPI_INVALID_ARG;
    }
    drop(Box::from_raw(reference));
    NAPI_OK
}

#[no_mangle]
pub unsafe extern "C" fn napi_get_reference_value(
    _env: NapiEnv,
    reference: *mut Reference,
    out: *mut NapiValue,
) -> NapiStatus {
    let Some(reference) = reference.as_ref() else {
        return NAPI_INVALID_ARG;
    };
    let _ = reference.count;
    write_value(out, reference.value)
}

#[no_mangle]
pub unsafe extern "C" fn napi_reference_ref(
    _env: NapiEnv,
    reference: *mut Reference,
    result: *mut u32,
) -> NapiStatus {
    let Some(reference) = reference.as_mut() else {
        return NAPI_INVALID_ARG;
    };
    reference.count = reference.count.saturating_add(1);
    if !result.is_null() {
        *result = reference.count;
    }
    NAPI_OK
}

#[no_mangle]
pub unsafe extern "C" fn napi_reference_unref(
    _env: NapiEnv,
    reference: *mut Reference,
    result: *mut u32,
) -> NapiStatus {
    let Some(reference) = reference.as_mut() else {
        return NAPI_INVALID_ARG;
    };
    if reference.count == 0 {
        return NAPI_GENERIC_FAILURE;
    }
    reference.count -= 1;
    if !result.is_null() {
        *result = reference.count;
    }
    NAPI_OK
}

#[no_mangle]
pub unsafe extern "C" fn napi_call_function(
    env: NapiEnv,
    this_arg: NapiValue,
    function: NapiValue,
    argc: usize,
    argv: *const NapiValue,
    out: *mut NapiValue,
) -> NapiStatus {
    let function = match value_ref(function) {
        Ok(Value::Function(function)) => function.clone(),
        _ => return NAPI_INVALID_ARG,
    };
    let args = if argc == 0 {
        Vec::new()
    } else if argv.is_null() {
        return NAPI_INVALID_ARG;
    } else {
        std::slice::from_raw_parts(argv, argc).to_vec()
    };
    let mut info = CallbackInfo {
        args,
        this_arg,
        new_target: ptr::null_mut(),
        data: function.data,
    };
    let result = (function.callback)(env, &mut info);
    if env_mut(env)
        .map(|env| env.exception.is_some())
        .unwrap_or(false)
    {
        return NAPI_PENDING_EXCEPTION;
    }
    if out.is_null() {
        NAPI_OK
    } else {
        write_value(out, result)
    }
}

#[no_mangle]
pub unsafe extern "C" fn napi_make_callback(
    env: NapiEnv,
    _async_context: *mut c_void,
    this_arg: NapiValue,
    function: NapiValue,
    argc: usize,
    argv: *const NapiValue,
    result: *mut NapiValue,
) -> NapiStatus {
    napi_call_function(env, this_arg, function, argc, argv, result)
}

const NAPI_PENDING_EXCEPTION: NapiStatus = 10;

#[no_mangle]
pub unsafe extern "C" fn napi_create_error(
    env: NapiEnv,
    _code: NapiValue,
    message: NapiValue,
    out: *mut NapiValue,
) -> NapiStatus {
    let message = match value_ref(message) {
        Ok(Value::String(message)) => message.clone(),
        _ => return NAPI_STRING_EXPECTED,
    };
    let Ok(env) = env_mut(env) else {
        return NAPI_INVALID_ARG;
    };
    let value = env.alloc(Value::Error(message));
    write_value(out, value)
}

#[no_mangle]
pub unsafe extern "C" fn napi_create_type_error(
    env: NapiEnv,
    code: NapiValue,
    message: NapiValue,
    out: *mut NapiValue,
) -> NapiStatus {
    napi_create_error(env, code, message, out)
}

#[no_mangle]
pub unsafe extern "C" fn napi_throw(env: NapiEnv, error: NapiValue) -> NapiStatus {
    let Ok(env) = env_mut(env) else {
        return NAPI_INVALID_ARG;
    };
    if error.is_null() {
        return NAPI_INVALID_ARG;
    }
    env.exception = Some(error);
    NAPI_OK
}

#[no_mangle]
pub unsafe extern "C" fn napi_get_last_error_info(
    _env: NapiEnv,
    out: *mut *const NapiExtendedErrorInfo,
) -> NapiStatus {
    if out.is_null() {
        return NAPI_INVALID_ARG;
    }
    *out = &LAST_ERROR_INFO;
    NAPI_OK
}

#[no_mangle]
pub unsafe extern "C" fn napi_fatal_error(
    _location: *const c_char,
    _location_len: usize,
    message: *const c_char,
    message_len: usize,
) -> ! {
    let message = if message.is_null() {
        "N-API fatal error".into()
    } else if message_len == NAPI_AUTO_LENGTH {
        CStr::from_ptr(message).to_string_lossy().into_owned()
    } else {
        String::from_utf8_lossy(std::slice::from_raw_parts(message.cast(), message_len))
            .into_owned()
    };
    eprintln!("thaw-napi: {message}");
    std::process::abort()
}

#[no_mangle]
pub unsafe extern "C" fn napi_fatal_exception(env: NapiEnv, error: NapiValue) -> NapiStatus {
    let Ok(env) = env_mut(env) else {
        return NAPI_INVALID_ARG;
    };
    if error.is_null() {
        return NAPI_INVALID_ARG;
    }
    let message = match value_ref(error) {
        Ok(Value::Error(message) | Value::String(message)) => message.clone(),
        _ => "unhandled N-API callback exception".into(),
    };
    eprintln!("thaw-napi: fatal exception: {message}");
    env.exception = Some(error);
    FATAL_EXCEPTION_PENDING.store(true, Ordering::Release);
    NAPI_OK
}

#[no_mangle]
pub extern "C" fn thaw_napi_take_fatal_exception() -> u8 {
    u8::from(FATAL_EXCEPTION_PENDING.swap(false, Ordering::AcqRel))
}

#[repr(C)]
pub struct NapiNodeVersion {
    major: u32,
    minor: u32,
    patch: u32,
    release: *const c_char,
}
static NODE_RELEASE: &[u8] = b"thaw\0";
static NODE_VERSION: NapiNodeVersion = NapiNodeVersion {
    major: 20,
    minor: 0,
    patch: 0,
    release: NODE_RELEASE.as_ptr().cast(),
};
unsafe impl Sync for NapiNodeVersion {}
#[no_mangle]
pub unsafe extern "C" fn napi_get_node_version(
    _env: NapiEnv,
    out: *mut *const NapiNodeVersion,
) -> NapiStatus {
    if out.is_null() {
        NAPI_INVALID_ARG
    } else {
        *out = &NODE_VERSION;
        NAPI_OK
    }
}

#[no_mangle]
pub unsafe extern "C" fn napi_open_handle_scope(env: NapiEnv, out: *mut *mut c_void) -> NapiStatus {
    if env.is_null() || out.is_null() {
        NAPI_INVALID_ARG
    } else {
        *out = env.cast();
        NAPI_OK
    }
}
#[no_mangle]
pub unsafe extern "C" fn napi_close_handle_scope(_env: NapiEnv, _scope: *mut c_void) -> NapiStatus {
    NAPI_OK
}

#[no_mangle]
pub unsafe extern "C" fn napi_open_callback_scope(
    env: NapiEnv,
    _resource: NapiValue,
    _context: *mut c_void,
    out: *mut *mut c_void,
) -> NapiStatus {
    napi_open_handle_scope(env, out)
}

#[no_mangle]
pub unsafe extern "C" fn napi_close_callback_scope(env: NapiEnv, scope: *mut c_void) -> NapiStatus {
    napi_close_handle_scope(env, scope)
}

#[no_mangle]
pub unsafe extern "C" fn napi_async_destroy(env: NapiEnv, _context: *mut c_void) -> NapiStatus {
    if env.is_null() {
        NAPI_INVALID_ARG
    } else {
        NAPI_OK
    }
}

#[no_mangle]
pub unsafe extern "C" fn napi_create_threadsafe_function(
    env: NapiEnv,
    function: NapiValue,
    _async_resource: NapiValue,
    _async_resource_name: NapiValue,
    max_queue_size: usize,
    initial_thread_count: usize,
    thread_finalize_data: *mut c_void,
    thread_finalize_callback: Option<NapiFinalize>,
    context: *mut c_void,
    call_js_callback: Option<NapiThreadsafeFunctionCallJs>,
    result: *mut *mut ThreadsafeFunction,
) -> NapiStatus {
    if env.is_null()
        || result.is_null()
        || initial_thread_count == 0
        || (function.is_null() && call_js_callback.is_none())
    {
        return NAPI_INVALID_ARG;
    }
    if !function.is_null() && !matches!(value_ref(function), Ok(Value::Function(_))) {
        return NAPI_INVALID_ARG;
    }
    let threadsafe = Box::new(ThreadsafeFunction {
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
    *result = Box::into_raw(threadsafe);
    NAPI_OK
}

#[no_mangle]
pub unsafe extern "C" fn napi_call_threadsafe_function(
    function: *mut ThreadsafeFunction,
    data: *mut c_void,
    mode: i32,
) -> NapiStatus {
    let Some(function_ref) = function.as_ref() else {
        return NAPI_INVALID_ARG;
    };
    if mode != 0 && mode != 1 {
        return NAPI_INVALID_ARG;
    }
    let mut state = function_ref
        .state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    loop {
        if state.closing {
            return NAPI_CLOSING;
        }
        if function_ref.max_queue_size == 0 || state.queue.len() < function_ref.max_queue_size {
            break;
        }
        if mode == 0 {
            return NAPI_QUEUE_FULL;
        }
        if std::thread::current().id() == function_ref.creator {
            return NAPI_WOULD_DEADLOCK;
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
    let Some(function_ref) = function.as_ref() else {
        return NAPI_INVALID_ARG;
    };
    if mode != 0 && mode != 1 {
        return NAPI_INVALID_ARG;
    }
    let mut state = function_ref
        .state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if state.closing || state.thread_count == 0 {
        return NAPI_CLOSING;
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
    let Some(function_ref) = function.as_ref() else {
        return NAPI_INVALID_ARG;
    };
    let mut state = function_ref
        .state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if state.closing {
        return NAPI_CLOSING;
    }
    state.thread_count = state.thread_count.saturating_add(1);
    NAPI_OK
}

#[no_mangle]
pub unsafe extern "C" fn napi_ref_threadsafe_function(
    env: NapiEnv,
    function: *mut ThreadsafeFunction,
) -> NapiStatus {
    let Some(function) = function.as_ref() else {
        return NAPI_INVALID_ARG;
    };
    if env.is_null() || function.env != env as usize {
        return NAPI_INVALID_ARG;
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
    let Some(function) = function.as_ref() else {
        return NAPI_INVALID_ARG;
    };
    if env.is_null() || function.env != env as usize {
        return NAPI_INVALID_ARG;
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
    napi_open_handle_scope(env, out)
}
#[no_mangle]
pub unsafe extern "C" fn napi_close_escapable_handle_scope(
    env: NapiEnv,
    scope: *mut c_void,
) -> NapiStatus {
    napi_close_handle_scope(env, scope)
}
#[no_mangle]
pub unsafe extern "C" fn napi_escape_handle(
    _env: NapiEnv,
    _scope: *mut c_void,
    value: NapiValue,
    out: *mut NapiValue,
) -> NapiStatus {
    write_value(out, value)
}

#[no_mangle]
pub unsafe extern "C" fn napi_create_promise(
    env: NapiEnv,
    deferred: *mut *mut Deferred,
    promise: *mut NapiValue,
) -> NapiStatus {
    if deferred.is_null() || promise.is_null() {
        return NAPI_INVALID_ARG;
    }
    let Ok(env_ref) = env_mut(env) else {
        return NAPI_INVALID_ARG;
    };
    let state = Rc::new(RefCell::new(PromiseState::Pending));
    *promise = env_ref.alloc(Value::Promise(Rc::clone(&state)));
    *deferred = Box::into_raw(Box::new(Deferred {
        env: env as usize,
        state,
    }));
    NAPI_OK
}

unsafe fn settle_deferred(
    env: NapiEnv,
    deferred: *mut Deferred,
    value: NapiValue,
    rejected: bool,
) -> NapiStatus {
    let Some(deferred_ref) = deferred.as_ref() else {
        return NAPI_INVALID_ARG;
    };
    if env.is_null() || value.is_null() || deferred_ref.env != env as usize {
        return NAPI_INVALID_ARG;
    }
    let mut state = deferred_ref.state.borrow_mut();
    if !matches!(*state, PromiseState::Pending) {
        return NAPI_GENERIC_FAILURE;
    }
    *state = if rejected {
        PromiseState::Rejected(value)
    } else {
        PromiseState::Resolved(value)
    };
    drop(state);
    drop(Box::from_raw(deferred));
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
    _async_resource: NapiValue,
    _async_resource_name: NapiValue,
    execute: Option<NapiAsyncExecuteCallback>,
    complete: Option<NapiAsyncCompleteCallback>,
    data: *mut c_void,
    result: *mut *mut AsyncWork,
) -> NapiStatus {
    if env.is_null() || execute.is_none() || result.is_null() {
        return NAPI_INVALID_ARG;
    }
    let work = Box::new(AsyncWork {
        env: env as usize,
        execute: execute.unwrap(),
        complete,
        data: data as usize,
        state: AtomicU8::new(ASYNC_CREATED),
        completion_status: AtomicI32::new(NAPI_OK),
    });
    *result = Box::into_raw(work);
    NAPI_OK
}

#[no_mangle]
pub unsafe extern "C" fn napi_queue_async_work(env: NapiEnv, work: *mut AsyncWork) -> NapiStatus {
    let Some(work_ref) = work.as_ref() else {
        return NAPI_INVALID_ARG;
    };
    if env.is_null() || work_ref.env != env as usize {
        return NAPI_INVALID_ARG;
    }
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
        return NAPI_GENERIC_FAILURE;
    }

    let Some(pool) = async_pool() else {
        work_ref.state.store(ASYNC_CREATED, Ordering::Release);
        return NAPI_GENERIC_FAILURE;
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
    let Some(work_ref) = work.as_ref() else {
        return NAPI_INVALID_ARG;
    };
    if env.is_null() || work_ref.env != env as usize {
        return NAPI_INVALID_ARG;
    }
    let Some(pool) = async_pool() else {
        return NAPI_GENERIC_FAILURE;
    };
    let mut queue = pool
        .queue
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if work_ref.state.load(Ordering::Acquire) != ASYNC_QUEUED {
        return NAPI_GENERIC_FAILURE;
    }
    let Some(position) = queue.iter().position(|queued| *queued == work as usize) else {
        return NAPI_GENERIC_FAILURE;
    };
    queue.remove(position);
    work_ref
        .completion_status
        .store(NAPI_CANCELLED, Ordering::Release);
    work_ref
        .state
        .store(ASYNC_COMPLETE_PENDING, Ordering::Release);
    drop(queue);
    async_completions()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .push_back(work as usize);
    NAPI_OK
}

#[no_mangle]
pub unsafe extern "C" fn napi_delete_async_work(env: NapiEnv, work: *mut AsyncWork) -> NapiStatus {
    let Some(work_ref) = work.as_ref() else {
        return NAPI_INVALID_ARG;
    };
    if env.is_null() || work_ref.env != env as usize {
        return NAPI_INVALID_ARG;
    }
    let state = work_ref.state.load(Ordering::Acquire);
    if state != ASYNC_CREATED && state != ASYNC_COMPLETED {
        return NAPI_GENERIC_FAILURE;
    }
    drop(Box::from_raw(work));
    NAPI_OK
}

fn run_one_threadsafe_callback() -> Option<bool> {
    let address = threadsafe_ready()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .pop_front()?;
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
        unsafe { drop(Box::from_raw(function_ptr)) };
    }
    Some(data.is_some() && !aborting)
}

fn run_one_async_completion() -> Option<()> {
    let work_address = async_completions()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .pop_front()?;
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
    Some(())
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
    let event_loop = default_loop();
    if event_loop.is_null() {
        return false;
    }
    const UV_RUN_NOWAIT: i32 = 2;
    run(event_loop, UV_RUN_NOWAIT);
    alive(event_loop) != 0
}

#[cfg(not(target_os = "linux"))]
unsafe fn poll_uv_loop() -> bool {
    false
}

/// Runs every callback which is ready now without waiting for producers.
#[no_mangle]
pub extern "C" fn thaw_napi_poll_async_work() -> usize {
    let mut completed = 0;
    unsafe {
        poll_uv_loop();
    }
    loop {
        let mut progressed = false;
        while let Some(called) = run_one_threadsafe_callback() {
            completed += usize::from(called);
            progressed = true;
        }
        while run_one_async_completion().is_some() {
            completed += 1;
            progressed = true;
        }
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

/// Runs queued completion callbacks on the calling thread and waits until all
/// work submitted by native addons has completed. Generated executables call
/// this once user `main` returns.
#[no_mangle]
pub extern "C" fn thaw_napi_run_async_work() -> usize {
    let mut completed = 0;
    loop {
        completed += thaw_napi_poll_async_work();
        let threadsafe_pending = !threadsafe_ready()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .is_empty();
        let uv_alive = unsafe { poll_uv_loop() };
        if ACTIVE_ASYNC_WORK.load(Ordering::Acquire) == 0
            && ACTIVE_THREADSAFE_FUNCTIONS.load(Ordering::Acquire) == 0
            && !threadsafe_pending
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;
    use std::sync::atomic::AtomicBool;

    static BCRYPT_ASYNC_RESULT: OnceLock<Mutex<Option<String>>> = OnceLock::new();
    static ASYNC_TEST_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
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

    fn lock_async_test() -> std::sync::MutexGuard<'static, ()> {
        ASYNC_TEST_LOCK
            .get_or_init(|| Mutex::new(()))
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
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

    unsafe extern "C" fn parcel_watcher_callback(
        env: NapiEnv,
        info: NapiCallbackInfo,
    ) -> NapiValue {
        let info = info.as_ref().unwrap();
        let probe = &*(info.data as *const ParcelWatcherProbe);
        if let Some(events) = info.args.get(1).and_then(|value| match value_ref(*value) {
            Ok(Value::Array(events)) => Some(events),
            _ => None,
        }) {
            let mut collected = probe.events.lock().unwrap();
            for event in events {
                let Ok(Value::Object(fields)) = value_ref(*event) else {
                    continue;
                };
                let path = fields
                    .get("path")
                    .and_then(|value| match value_ref(*value) {
                        Ok(Value::String(value)) => Some(value.clone()),
                        _ => None,
                    });
                let kind = fields
                    .get("type")
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
            drop(Box::from_raw(full));
            let deadlock = Box::into_raw(Box::new(2.0));
            assert_eq!(
                napi_call_threadsafe_function(threadsafe, deadlock.cast(), 1),
                NAPI_WOULD_DEADLOCK
            );
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
            assert_eq!(napi_delete_async_work(&mut env, work), NAPI_OK);
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
            let result =
                thaw_napi_call_method_result(instance.value, c"get".as_ptr(), c"[]".as_ptr());
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

            let encrypt_args = CString::new(
                serde_json::to_string(&serde_json::json!(["password", salt])).unwrap(),
            )
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
            let database =
                thaw_napi_construct_handle_result(constructor, c"[\":memory:\",6]".as_ptr());
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
        let args = CString::new(
            serde_json::to_string(&serde_json::json!([dir.to_string_lossy()])).unwrap(),
        )
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
}
