//! Minimal Node-API host for loading `.node` addons.

// Every exported unsafe function in this crate implements the Node-API C ABI
// and shares its pointer-validity contract with the native addon caller.
#![allow(clippy::missing_safety_doc)]

use libc::{c_char, c_void};
use serde_json::Value as JsonValue;
use std::cell::RefCell;
use std::collections::{HashMap, HashSet, VecDeque};
use std::ffi::{CStr, CString};
use std::io::Write;
#[cfg(target_os = "linux")]
use std::os::fd::FromRawFd;
use std::ptr;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU64, AtomicU8, AtomicUsize, Ordering};
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
type NodeApiNoEnvFinalize = unsafe extern "C" fn(*mut c_void, *mut c_void);
type NapiCleanupHook = unsafe extern "C" fn(*mut c_void);
type NapiAsyncCleanupHook = unsafe extern "C" fn(*mut AsyncCleanupHookHandle, *mut c_void);
type NapiThreadsafeFunctionCallJs =
    unsafe extern "C" fn(NapiEnv, NapiValue, *mut c_void, *mut c_void);
const NAPI_OK: NapiStatus = 0;

const NAPI_INVALID_ARG: NapiStatus = 1;
const NAPI_OBJECT_EXPECTED: NapiStatus = 2;
const NAPI_FUNCTION_EXPECTED: NapiStatus = 5;
const NAPI_GENERIC_FAILURE: NapiStatus = 9;
const NAPI_CANCELLED: NapiStatus = 11;
const NAPI_ESCAPE_CALLED_TWICE: NapiStatus = 12;
const NAPI_HANDLE_SCOPE_MISMATCH: NapiStatus = 13;
const NAPI_QUEUE_FULL: NapiStatus = 15;
const NAPI_CLOSING: NapiStatus = 16;
const NAPI_WOULD_DEADLOCK: NapiStatus = 21;
const NAPI_NUMBER_EXPECTED: NapiStatus = 6;
const NAPI_STRING_EXPECTED: NapiStatus = 3;
const NAPI_BOOLEAN_EXPECTED: NapiStatus = 7;
const NAPI_ARRAY_EXPECTED: NapiStatus = 8;
const NAPI_DATE_EXPECTED: NapiStatus = 18;
const NAPI_ARRAYBUFFER_EXPECTED: NapiStatus = 19;
const NAPI_BIGINT_EXPECTED: NapiStatus = 17;
const NAPI_AUTO_LENGTH: usize = usize::MAX;
const NAPI_WRITABLE: u32 = 1;
const NAPI_ENUMERABLE: u32 = 2;
const NAPI_CONFIGURABLE: u32 = 4;
const NAPI_DEFAULT_PROPERTY_ATTRIBUTES: u32 = NAPI_WRITABLE | NAPI_ENUMERABLE | NAPI_CONFIGURABLE;
const NAPI_KEY_INCLUDE_PROTOTYPES: i32 = 0;
const NAPI_KEY_OWN_ONLY: i32 = 1;
const NAPI_KEY_ALL_PROPERTIES: u32 = 0;
const NAPI_KEY_SKIP_STRINGS: u32 = 1 << 3;
const NAPI_KEY_SKIP_SYMBOLS: u32 = 1 << 4;
const NAPI_KEY_KEEP_NUMBERS: i32 = 0;
const NAPI_KEY_NUMBERS_TO_STRINGS: i32 = 1;

const ASYNC_CREATED: u8 = 0;
const ASYNC_QUEUED: u8 = 1;
const ASYNC_EXECUTING: u8 = 2;
const ASYNC_COMPLETE_PENDING: u8 = 3;
const ASYNC_COMPLETED: u8 = 4;
const ASYNC_DELETED: u8 = 5;

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
static ACTIVE_ASYNC_CLEANUP_HOOKS: AtomicUsize = AtomicUsize::new(0);
static NEXT_SYMBOL_ID: AtomicU64 = AtomicU64::new(1);
static GLOBAL_SYMBOLS: OnceLock<Mutex<HashMap<String, u64>>> = OnceLock::new();
static THREADSAFE_READY: OnceLock<Mutex<VecDeque<usize>>> = OnceLock::new();
#[allow(clippy::vec_box)]
static THREADSAFE_FUNCTIONS: OnceLock<Mutex<Vec<Box<ThreadsafeFunction>>>> = OnceLock::new();
#[allow(clippy::vec_box)]
static ASYNC_CLEANUP_HANDLES: OnceLock<Mutex<Vec<Box<AsyncCleanupHookHandle>>>> = OnceLock::new();
static REGISTERED_UV_LOOPS: OnceLock<Mutex<Vec<usize>>> = OnceLock::new();

fn registered_uv_loops() -> &'static Mutex<Vec<usize>> {
    REGISTERED_UV_LOOPS.get_or_init(|| Mutex::new(Vec::new()))
}

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

pub struct AsyncCleanupHookHandle {
    env: usize,
    hook: NapiAsyncCleanupHook,
    data: usize,
    state: AtomicU8,
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

#[allow(clippy::vec_box)]
fn threadsafe_functions() -> &'static Mutex<Vec<Box<ThreadsafeFunction>>> {
    THREADSAFE_FUNCTIONS.get_or_init(|| Mutex::new(Vec::new()))
}

#[allow(clippy::vec_box)]
fn async_cleanup_handles() -> &'static Mutex<Vec<Box<AsyncCleanupHookHandle>>> {
    ASYNC_CLEANUP_HANDLES.get_or_init(|| Mutex::new(Vec::new()))
}

unsafe fn threadsafe_function_ref<'a>(
    function: *mut ThreadsafeFunction,
) -> Result<&'a ThreadsafeFunction, NapiStatus> {
    if function.is_null() {
        return Err(NAPI_INVALID_ARG);
    }
    let functions = threadsafe_functions()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let owned = functions
        .iter()
        .any(|candidate| std::ptr::eq(candidate.as_ref(), function));
    drop(functions);
    if owned {
        Ok(&*function)
    } else {
        Err(NAPI_INVALID_ARG)
    }
}

unsafe fn record_threadsafe_status(
    function: &ThreadsafeFunction,
    status: NapiStatus,
) -> NapiStatus {
    record_status(function.env as NapiEnv, status)
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
    properties: HashMap<PropertyKey, NapiValue>,
    _thaw_bridge: Option<Arc<ThawCallbackBridge>>,
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum PropertyKey {
    String(String),
    Symbol(u64),
}

impl From<String> for PropertyKey {
    fn from(value: String) -> Self {
        Self::String(value)
    }
}

impl From<&str> for PropertyKey {
    fn from(value: &str) -> Self {
        Self::String(value.to_owned())
    }
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
    Date(f64),
    BigInt {
        negative: bool,
        words: Vec<u64>,
    },
    String(String),
    Object(HashMap<PropertyKey, NapiValue>),
    Array(Vec<Option<NapiValue>>),
    Buffer(Vec<u8>),
    ExternalBuffer {
        data: *mut u8,
        length: usize,
    },
    BufferView {
        array_buffer: NapiValue,
        byte_offset: usize,
        length: usize,
    },
    ArrayBuffer {
        bytes: Vec<u8>,
        detached: bool,
    },
    SharedArrayBuffer(Vec<u8>),
    ExternalSharedArrayBuffer {
        data: *mut u8,
        length: usize,
    },
    ExternalArrayBuffer {
        data: *mut u8,
        length: usize,
        detached: bool,
    },
    TypedArray {
        array_type: i32,
        length: usize,
        array_buffer: NapiValue,
        byte_offset: usize,
    },
    DataView {
        length: usize,
        array_buffer: NapiValue,
        byte_offset: usize,
    },
    External(*mut c_void),
    Symbol {
        id: u64,
        description: String,
    },
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
    env: usize,
    value: NapiValue,
    count: u32,
    deleted: bool,
}

struct AsyncContext {
    env: usize,
    resource: NapiValue,
    resource_name: String,
    destroyed: bool,
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

unsafe fn validate_property_descriptors(
    env: NapiEnv,
    count: usize,
    descriptors: *const NapiPropertyDescriptor,
) -> NapiStatus {
    if count == 0 {
        return NAPI_OK;
    }
    if env.is_null() || descriptors.is_null() {
        return record_status(env, NAPI_INVALID_ARG);
    }
    for descriptor in std::slice::from_raw_parts(descriptors, count) {
        if descriptor.utf8name.is_null() && !value_belongs_to_environment(env, descriptor.name) {
            return record_status(env, NAPI_INVALID_ARG);
        }
        if descriptor.method.is_none()
            && descriptor.getter.is_none()
            && descriptor.setter.is_none()
            && !descriptor.value.is_null()
            && !value_belongs_to_environment(env, descriptor.value)
        {
            return record_status(env, NAPI_INVALID_ARG);
        }
    }
    NAPI_OK
}

#[repr(C)]
pub struct NapiExtendedErrorInfo {
    error_message: *const c_char,
    engine_reserved: *mut c_void,
    engine_error_code: u32,
    error_code: NapiStatus,
}

#[repr(C)]
#[derive(Clone, Copy, Eq, PartialEq)]
pub struct NapiTypeTag {
    lower: u64,
    upper: u64,
}

pub struct Env {
    values: Vec<NapiValue>,
    global: NapiValue,
    exception: Option<NapiValue>,
    wraps: HashMap<usize, WrapRecord>,
    instances: HashMap<usize, usize>,
    prototypes: HashMap<usize, usize>,
    accessors: HashMap<(usize, PropertyKey), Accessor>,
    finalizers: Vec<FinalizeRecord>,
    noenv_finalizers: Vec<NoEnvFinalizeRecord>,
    posted_finalizers: Vec<FinalizeRecord>,
    instance_data: Option<FinalizeRecord>,
    cleanup_hooks: Vec<CleanupHookRecord>,
    async_cleanup_hooks: Vec<*mut AsyncCleanupHookHandle>,
    external_memory: i64,
    sealed_objects: HashSet<usize>,
    frozen_objects: HashSet<usize>,
    property_attributes: HashMap<(usize, PropertyKey), u32>,
    property_order: HashMap<usize, Vec<PropertyKey>>,
    host_properties: HashMap<usize, HashMap<PropertyKey, NapiValue>>,
    error_names: HashMap<usize, String>,
    symbols: HashMap<u64, NapiValue>,
    type_tags: HashMap<usize, NapiTypeTag>,
    property_keys: HashMap<String, NapiValue>,
    module_file_name: CString,
    last_error_info: NapiExtendedErrorInfo,
    // Box keeps the opaque C handle stable when the owning vector grows.
    #[allow(clippy::vec_box)]
    handle_scopes: Vec<Box<HandleScope>>,
    active_handle_scopes: Vec<*mut HandleScope>,
    // Box keeps async_context pointers stable and retained long enough to reject reuse.
    #[allow(clippy::vec_box)]
    async_contexts: Vec<Box<AsyncContext>>,
    // Box keeps deferred pointers stable and allows safe repeated-call rejection.
    #[allow(clippy::vec_box)]
    deferreds: Vec<Box<Deferred>>,
    // Box keeps async-work addresses stable for worker and completion queues.
    #[allow(clippy::vec_box)]
    async_works: Vec<Box<AsyncWork>>,
    // Box keeps reference handles stable so deleted handles can be rejected safely.
    #[allow(clippy::vec_box)]
    references: Vec<Box<Reference>>,
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum HandleScopeKind {
    Normal,
    Escapable,
    Callback,
}

struct HandleScope {
    env: usize,
    kind: HandleScopeKind,
    closed: bool,
    escaped: bool,
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

struct NoEnvFinalizeRecord {
    data: *mut c_void,
    finalize: Option<NodeApiNoEnvFinalize>,
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
            prototypes: HashMap::new(),
            accessors: HashMap::new(),
            finalizers: Vec::new(),
            noenv_finalizers: Vec::new(),
            posted_finalizers: Vec::new(),
            instance_data: None,
            cleanup_hooks: Vec::new(),
            async_cleanup_hooks: Vec::new(),
            external_memory: 0,
            sealed_objects: HashSet::new(),
            frozen_objects: HashSet::new(),
            property_attributes: HashMap::new(),
            property_order: HashMap::new(),
            host_properties: HashMap::new(),
            error_names: HashMap::new(),
            symbols: HashMap::new(),
            type_tags: HashMap::new(),
            property_keys: HashMap::new(),
            module_file_name: CString::new("").unwrap(),
            last_error_info: NapiExtendedErrorInfo {
                error_message: ptr::null(),
                engine_reserved: ptr::null_mut(),
                engine_error_code: 0,
                error_code: NAPI_OK,
            },
            handle_scopes: Vec::new(),
            active_handle_scopes: Vec::new(),
            async_contexts: Vec::new(),
            deferreds: Vec::new(),
            async_works: Vec::new(),
            references: Vec::new(),
        }
    }

    fn alloc(&mut self, value: Value) -> NapiValue {
        let value = Box::into_raw(Box::new(value));
        self.values.push(value);
        value
    }
}

fn alloc_reference(env: &mut Env, value: NapiValue, count: u32) -> *mut Reference {
    let mut reference = Box::new(Reference {
        env: env as *mut Env as usize,
        value,
        count,
        deleted: false,
    });
    let reference_ptr = (&mut *reference) as *mut Reference;
    env.references.push(reference);
    reference_ptr
}

unsafe fn reference_mut<'a>(
    env: NapiEnv,
    reference: *mut Reference,
) -> Result<&'a mut Reference, NapiStatus> {
    let Ok(env_ref) = env_mut(env) else {
        return Err(NAPI_INVALID_ARG);
    };
    let Some(reference) = env_ref
        .references
        .iter_mut()
        .find(|candidate| std::ptr::eq(candidate.as_ref(), reference))
    else {
        record_status(env, NAPI_INVALID_ARG);
        return Err(NAPI_INVALID_ARG);
    };
    if reference.deleted || reference.env != env as usize {
        record_status(env, NAPI_INVALID_ARG);
        return Err(NAPI_INVALID_ARG);
    }
    Ok(reference)
}

unsafe fn async_context_mut<'a>(
    env: NapiEnv,
    context: *mut c_void,
) -> Result<&'a mut AsyncContext, NapiStatus> {
    let Ok(env_ref) = env_mut(env) else {
        return Err(NAPI_INVALID_ARG);
    };
    let context_ptr = context.cast::<AsyncContext>();
    let Some(context_ref) = env_ref
        .async_contexts
        .iter_mut()
        .find(|candidate| std::ptr::eq(candidate.as_ref(), context_ptr))
    else {
        record_status(env, NAPI_INVALID_ARG);
        return Err(NAPI_INVALID_ARG);
    };
    if context_ref.env != env as usize || context_ref.destroyed {
        record_status(env, NAPI_INVALID_ARG);
        return Err(NAPI_INVALID_ARG);
    }
    Ok(context_ref)
}

unsafe fn async_work_ref<'a>(
    env: NapiEnv,
    work: *mut AsyncWork,
) -> Result<&'a AsyncWork, NapiStatus> {
    let Ok(env_ref) = env_mut(env) else {
        return Err(NAPI_INVALID_ARG);
    };
    let work = env_ref
        .async_works
        .iter()
        .find(|candidate| std::ptr::eq(candidate.as_ref(), work))
        .map(Box::as_ref)
        .ok_or(NAPI_INVALID_ARG);
    if work.is_err() {
        record_status(env, NAPI_INVALID_ARG);
    }
    work
}

unsafe fn open_handle_scope(
    env: NapiEnv,
    out: *mut *mut c_void,
    kind: HandleScopeKind,
) -> NapiStatus {
    let (Ok(env_ref), Some(out)) = (env_mut(env), out.as_mut()) else {
        return record_status(env, NAPI_INVALID_ARG);
    };
    let mut scope = Box::new(HandleScope {
        env: env as usize,
        kind,
        closed: false,
        escaped: false,
    });
    let scope_ptr = (&mut *scope) as *mut HandleScope;
    env_ref.handle_scopes.push(scope);
    env_ref.active_handle_scopes.push(scope_ptr);
    *out = scope_ptr.cast();
    NAPI_OK
}

unsafe fn close_handle_scope(
    env: NapiEnv,
    scope: *mut c_void,
    kind: HandleScopeKind,
) -> NapiStatus {
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
    if scope_ref.env != env as usize {
        return record_status(env, NAPI_INVALID_ARG);
    }
    if scope_ref.closed
        || scope_ref.kind != kind
        || env_ref.active_handle_scopes.last().copied() != Some(scope_ptr)
    {
        return record_status(env, NAPI_HANDLE_SCOPE_MISMATCH);
    }
    scope_ref.closed = true;
    env_ref.active_handle_scopes.pop();
    NAPI_OK
}

impl Drop for Env {
    fn drop(&mut self) {
        for handle in std::mem::take(&mut self.async_cleanup_hooks)
            .into_iter()
            .rev()
        {
            let Some(handle_ref) = (unsafe { handle.as_ref() }) else {
                continue;
            };
            if handle_ref
                .state
                .compare_exchange(0, 1, Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
            {
                ACTIVE_ASYNC_CLEANUP_HOOKS.fetch_add(1, Ordering::AcqRel);
                unsafe { (handle_ref.hook)(handle, handle_ref.data as *mut c_void) };
            }
        }
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
        for record in std::mem::take(&mut self.noenv_finalizers) {
            if let Some(finalize) = record.finalize {
                unsafe { finalize(record.data, record.hint) };
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
        while !self.posted_finalizers.is_empty() {
            for record in std::mem::take(&mut self.posted_finalizers) {
                if let Some(finalize) = record.finalize {
                    unsafe { finalize(self, record.data, record.hint) };
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

fn status_message(status: NapiStatus) -> *const c_char {
    let message: &'static [u8] = match status {
        NAPI_INVALID_ARG => b"Invalid argument\0",
        NAPI_OBJECT_EXPECTED => b"Object expected\0",
        NAPI_STRING_EXPECTED => b"String expected\0",
        NAPI_FUNCTION_EXPECTED => b"Function expected\0",
        NAPI_NUMBER_EXPECTED => b"Number expected\0",
        NAPI_BOOLEAN_EXPECTED => b"Boolean expected\0",
        NAPI_ARRAY_EXPECTED => b"Array expected\0",
        NAPI_GENERIC_FAILURE => b"Generic failure\0",
        NAPI_PENDING_EXCEPTION => b"An exception is pending\0",
        NAPI_CANCELLED => b"Operation cancelled\0",
        NAPI_ESCAPE_CALLED_TWICE => b"Escape called twice\0",
        NAPI_HANDLE_SCOPE_MISMATCH => b"Handle scope mismatch\0",
        NAPI_QUEUE_FULL => b"Queue full\0",
        NAPI_CLOSING => b"Resource is closing\0",
        NAPI_BIGINT_EXPECTED => b"BigInt expected\0",
        NAPI_DATE_EXPECTED => b"Date expected\0",
        NAPI_ARRAYBUFFER_EXPECTED => b"ArrayBuffer expected\0",
        NAPI_WOULD_DEADLOCK => b"Operation would deadlock\0",
        _ => b"N-API error\0",
    };
    message.as_ptr().cast()
}

unsafe fn record_status(env: NapiEnv, status: NapiStatus) -> NapiStatus {
    if status != NAPI_OK {
        if let Some(env) = env.as_mut() {
            env.last_error_info.error_code = status;
            env.last_error_info.engine_error_code = 0;
            env.last_error_info.engine_reserved = ptr::null_mut();
            env.last_error_info.error_message = status_message(status);
        }
    }
    status
}

unsafe fn env_for_value_output<'a>(
    env: NapiEnv,
    out: *mut NapiValue,
) -> Result<&'a mut Env, NapiStatus> {
    if out.is_null() {
        record_status(env, NAPI_INVALID_ARG);
        Err(NAPI_INVALID_ARG)
    } else {
        env_mut(env)
    }
}

unsafe fn value_ref<'a>(value: NapiValue) -> Result<&'a Value, NapiStatus> {
    value.as_ref().ok_or(NAPI_INVALID_ARG)
}

unsafe fn value_belongs_to_environment(env: NapiEnv, value: NapiValue) -> bool {
    if value.is_null() {
        record_status(env, NAPI_INVALID_ARG);
        return false;
    }
    let belongs = env.as_ref().is_some_and(|env| env.values.contains(&value))
        || HOST.with(|host| {
            host.borrow()
                .module_envs
                .iter()
                .any(|module_env| module_env.values.contains(&value))
        });
    if !belongs {
        record_status(env, NAPI_INVALID_ARG);
    }
    belongs
}

fn is_object_value(value: &Value) -> bool {
    matches!(
        value,
        Value::Object(_)
            | Value::Array(_)
            | Value::Buffer(_)
            | Value::ExternalBuffer { .. }
            | Value::BufferView { .. }
            | Value::ArrayBuffer { .. }
            | Value::SharedArrayBuffer(_)
            | Value::ExternalSharedArrayBuffer { .. }
            | Value::ExternalArrayBuffer { .. }
            | Value::TypedArray { .. }
            | Value::DataView { .. }
            | Value::Function(_)
            | Value::Promise(_)
            | Value::Error(_)
            | Value::Date(_)
    )
}

fn property_array_index(key: &PropertyKey) -> Option<usize> {
    let PropertyKey::String(name) = key else {
        return None;
    };
    let index = name.parse::<u32>().ok()?;
    (index != u32::MAX && index.to_string() == *name).then_some(index as usize)
}

fn record_property_order(env: &mut Env, owner: usize, key: &PropertyKey) {
    let order = env.property_order.entry(owner).or_default();
    if !order.contains(key) {
        order.push(key.clone());
    }
}

fn remove_property_order(env: &mut Env, owner: usize, key: &PropertyKey) {
    if let Some(order) = env.property_order.get_mut(&owner) {
        order.retain(|candidate| candidate != key);
    }
}

unsafe fn host_property_for_owner(
    env: NapiEnv,
    owner: usize,
    key: &PropertyKey,
) -> Option<NapiValue> {
    env.as_ref()
        .and_then(|env| env.host_properties.get(&owner)?.get(key).copied())
        .or_else(|| {
            HOST.with(|host| {
                let host = host.borrow();
                host.module_envs
                    .iter()
                    .chain(host.pending_call_envs.iter())
                    .find_map(|candidate| candidate.host_properties.get(&owner)?.get(key).copied())
            })
        })
}

unsafe fn host_property_keys_for_owner(env: NapiEnv, owner: usize) -> Vec<PropertyKey> {
    let mut keys = env
        .as_ref()
        .and_then(|env| env.host_properties.get(&owner))
        .map(|properties| properties.keys().cloned().collect::<Vec<_>>())
        .unwrap_or_default();
    HOST.with(|host| {
        let host = host.borrow();
        for candidate in host.module_envs.iter().chain(host.pending_call_envs.iter()) {
            if let Some(properties) = candidate.host_properties.get(&owner) {
                keys.extend(properties.keys().cloned());
            }
        }
    });
    keys
}

unsafe fn property_order_for_owner(env: NapiEnv, owner: usize, key: &PropertyKey) -> usize {
    env.as_ref()
        .and_then(|env| env.property_order.get(&owner))
        .and_then(|order| order.iter().position(|candidate| candidate == key))
        .or_else(|| {
            HOST.with(|host| {
                let host = host.borrow();
                host.module_envs
                    .iter()
                    .chain(host.pending_call_envs.iter())
                    .find_map(|candidate| {
                        candidate
                            .property_order
                            .get(&owner)?
                            .iter()
                            .position(|candidate| candidate == key)
                    })
            })
        })
        .unwrap_or(usize::MAX)
}

unsafe fn error_name_for_owner(env: NapiEnv, owner: usize) -> Option<String> {
    env.as_ref()
        .and_then(|env| env.error_names.get(&owner).cloned())
        .or_else(|| {
            HOST.with(|host| {
                let host = host.borrow();
                host.module_envs
                    .iter()
                    .chain(host.pending_call_envs.iter())
                    .find_map(|candidate| candidate.error_names.get(&owner).cloned())
            })
        })
}

unsafe fn typedarray_index_parts(object: NapiValue, key: &PropertyKey) -> Option<(i32, *mut u8)> {
    let index = property_array_index(key)?;
    let Value::TypedArray {
        array_type,
        length,
        array_buffer,
        byte_offset,
    } = value_ref(object).ok()?
    else {
        return None;
    };
    if index >= *length {
        return None;
    }
    let (bytes, _, detached) = arraybuffer_parts(*array_buffer).ok()?;
    if detached {
        return None;
    }
    let element_size = typedarray_element_size(*array_type)?;
    Some((*array_type, bytes.add(*byte_offset + index * element_size)))
}

unsafe fn read_typedarray_index(
    env: NapiEnv,
    object: NapiValue,
    key: &PropertyKey,
) -> Option<NapiValue> {
    let (array_type, data) = typedarray_index_parts(object, key)?;
    let env = env_mut(env).ok()?;
    Some(match array_type {
        0 => env.alloc(Value::Number(ptr::read_unaligned(data.cast::<i8>()) as f64)),
        1 | 2 => env.alloc(Value::Number(ptr::read_unaligned(data) as f64)),
        3 => env.alloc(Value::Number(ptr::read_unaligned(data.cast::<i16>()) as f64)),
        4 => env.alloc(Value::Number(ptr::read_unaligned(data.cast::<u16>()) as f64)),
        5 => env.alloc(Value::Number(ptr::read_unaligned(data.cast::<i32>()) as f64)),
        6 => env.alloc(Value::Number(ptr::read_unaligned(data.cast::<u32>()) as f64)),
        7 => env.alloc(Value::Number(ptr::read_unaligned(data.cast::<f32>()) as f64)),
        8 => env.alloc(Value::Number(ptr::read_unaligned(data.cast::<f64>()))),
        9 => {
            let value = ptr::read_unaligned(data.cast::<i64>());
            env.alloc(Value::BigInt {
                negative: value.is_negative(),
                words: vec![value.unsigned_abs()],
            })
        }
        10 => env.alloc(Value::BigInt {
            negative: false,
            words: vec![ptr::read_unaligned(data.cast::<u64>())],
        }),
        _ => return None,
    })
}

fn number_for_typedarray(value: &Value) -> Result<f64, NapiStatus> {
    Ok(match value {
        Value::Undefined => f64::NAN,
        Value::Null => 0.0,
        Value::Bool(value) => u8::from(*value) as f64,
        Value::Number(value) => *value,
        Value::String(value) => javascript_number_from_string(value),
        Value::BigInt { .. } | Value::Symbol { .. } => return Err(NAPI_GENERIC_FAILURE),
        _ => f64::NAN,
    })
}

fn integer_modulo(number: f64, bits: u32) -> u64 {
    if !number.is_finite() || number == 0.0 {
        return 0;
    }
    number.trunc().rem_euclid(2_f64.powi(bits as i32)) as u64
}

fn uint8_clamp(number: f64) -> u8 {
    if number.is_nan() || number <= 0.0 {
        return 0;
    }
    if number >= 255.0 {
        return 255;
    }
    let floor = number.floor();
    let fraction = number - floor;
    if fraction > 0.5 || (fraction == 0.5 && floor as u64 % 2 == 1) {
        floor as u8 + 1
    } else {
        floor as u8
    }
}

unsafe fn write_typedarray_index(
    env: &mut Env,
    object: NapiValue,
    key: &PropertyKey,
    value: NapiValue,
) -> Option<NapiStatus> {
    let (array_type, data) = typedarray_index_parts(object, key)?;
    let value_ref = match value_ref(value) {
        Ok(value) => value,
        Err(status) => return Some(status),
    };
    let status = match array_type {
        0..=8 => {
            let number = match number_for_typedarray(value_ref) {
                Ok(number) => number,
                Err(_) => {
                    let error =
                        env.alloc(Value::Error("typed array value has the wrong type".into()));
                    env.exception = Some(error);
                    return Some(NAPI_PENDING_EXCEPTION);
                }
            };
            match array_type {
                0 => ptr::write_unaligned(data.cast::<i8>(), integer_modulo(number, 8) as u8 as i8),
                1 => ptr::write_unaligned(data, integer_modulo(number, 8) as u8),
                2 => ptr::write_unaligned(data, uint8_clamp(number)),
                3 => ptr::write_unaligned(
                    data.cast::<i16>(),
                    integer_modulo(number, 16) as u16 as i16,
                ),
                4 => ptr::write_unaligned(data.cast::<u16>(), integer_modulo(number, 16) as u16),
                5 => ptr::write_unaligned(
                    data.cast::<i32>(),
                    integer_modulo(number, 32) as u32 as i32,
                ),
                6 => ptr::write_unaligned(data.cast::<u32>(), integer_modulo(number, 32) as u32),
                7 => ptr::write_unaligned(data.cast::<f32>(), number as f32),
                8 => ptr::write_unaligned(data.cast::<f64>(), number),
                _ => unreachable!(),
            }
            NAPI_OK
        }
        9 | 10 => {
            let Value::BigInt { negative, words } = value_ref else {
                let error = env.alloc(Value::Error("BigInt typed arrays require a BigInt".into()));
                env.exception = Some(error);
                return Some(NAPI_PENDING_EXCEPTION);
            };
            let magnitude = words.first().copied().unwrap_or(0);
            let bits = if *negative {
                0_u64.wrapping_sub(magnitude)
            } else {
                magnitude
            };
            ptr::write_unaligned(data.cast::<u64>(), bits);
            NAPI_OK
        }
        _ => NAPI_INVALID_ARG,
    };
    Some(status)
}

unsafe fn intrinsic_property_value(
    env: NapiEnv,
    object: NapiValue,
    key: &PropertyKey,
) -> Option<NapiValue> {
    let PropertyKey::String(name) = key else {
        return None;
    };
    if matches!(value_ref(object), Ok(Value::Error(_))) && name == "name" {
        let name = error_name_for_owner(env, object as usize).unwrap_or_else(|| "Error".into());
        return env_mut(env).ok().map(|env| env.alloc(Value::String(name)));
    }
    let env = env_mut(env).ok()?;
    match value_ref(object).ok()? {
        Value::Array(values) if name == "length" => {
            Some(env.alloc(Value::Number(values.len() as f64)))
        }
        Value::Buffer(bytes) if name == "length" => {
            Some(env.alloc(Value::Number(bytes.len() as f64)))
        }
        Value::ExternalBuffer { length, .. } if name == "length" => {
            Some(env.alloc(Value::Number(*length as f64)))
        }
        Value::BufferView {
            array_buffer,
            length,
            ..
        } if name == "length" => {
            let detached = arraybuffer_parts(*array_buffer)
                .map(|(_, _, detached)| detached)
                .unwrap_or(true);
            Some(env.alloc(Value::Number(if detached { 0.0 } else { *length as f64 })))
        }
        Value::ArrayBuffer { bytes, detached } if name == "byteLength" => {
            Some(env.alloc(Value::Number(if *detached {
                0.0
            } else {
                bytes.len() as f64
            })))
        }
        Value::ExternalArrayBuffer {
            length, detached, ..
        } if name == "byteLength" => {
            Some(env.alloc(Value::Number(if *detached { 0.0 } else { *length as f64 })))
        }
        Value::SharedArrayBuffer(bytes) if name == "byteLength" => {
            Some(env.alloc(Value::Number(bytes.len() as f64)))
        }
        Value::ExternalSharedArrayBuffer { length, .. } if name == "byteLength" => {
            Some(env.alloc(Value::Number(*length as f64)))
        }
        Value::TypedArray {
            array_type,
            length,
            array_buffer,
            byte_offset,
        } => {
            let detached = arraybuffer_parts(*array_buffer)
                .map(|(_, _, detached)| detached)
                .unwrap_or(true);
            match name.as_str() {
                "length" => {
                    Some(env.alloc(Value::Number(if detached { 0.0 } else { *length as f64 })))
                }
                "byteLength" => Some(env.alloc(Value::Number(if detached {
                    0.0
                } else {
                    (*length * typedarray_element_size(*array_type)?) as f64
                }))),
                "byteOffset" => Some(env.alloc(Value::Number(if detached {
                    0.0
                } else {
                    *byte_offset as f64
                }))),
                "buffer" => Some(*array_buffer),
                _ => None,
            }
        }
        Value::DataView {
            length,
            array_buffer,
            byte_offset,
        } => {
            let detached = arraybuffer_parts(*array_buffer)
                .map(|(_, _, detached)| detached)
                .unwrap_or(true);
            match name.as_str() {
                "byteLength" => {
                    Some(env.alloc(Value::Number(if detached { 0.0 } else { *length as f64 })))
                }
                "byteOffset" => Some(env.alloc(Value::Number(if detached {
                    0.0
                } else {
                    *byte_offset as f64
                }))),
                "buffer" => Some(*array_buffer),
                _ => None,
            }
        }
        _ => None,
    }
}

unsafe fn intrinsic_property_keys(object: NapiValue) -> &'static [&'static str] {
    match value_ref(object) {
        Ok(
            Value::Array(_)
            | Value::Buffer(_)
            | Value::ExternalBuffer { .. }
            | Value::BufferView { .. },
        ) => &["length"],
        Ok(
            Value::ArrayBuffer { .. }
            | Value::ExternalArrayBuffer { .. }
            | Value::SharedArrayBuffer(_)
            | Value::ExternalSharedArrayBuffer { .. },
        ) => &["byteLength"],
        Ok(Value::TypedArray { .. }) => &["length", "byteLength", "byteOffset", "buffer"],
        Ok(Value::DataView { .. }) => &["byteLength", "byteOffset", "buffer"],
        _ => &[],
    }
}

unsafe fn intrinsic_property_attributes(object: NapiValue, key: &PropertyKey) -> Option<u32> {
    let PropertyKey::String(name) = key else {
        return None;
    };
    if !intrinsic_property_keys(object).contains(&name.as_str()) {
        return None;
    }
    Some(
        if matches!(value_ref(object), Ok(Value::Array(_))) && name == "length" {
            NAPI_WRITABLE
        } else {
            0
        },
    )
}

unsafe fn own_property_value(
    env: NapiEnv,
    owner: NapiValue,
    key: &PropertyKey,
) -> Option<NapiValue> {
    if matches!(value_ref(owner), Ok(Value::Array(_))) {
        if let Some(value) = intrinsic_property_value(env, owner, key) {
            return Some(value);
        }
    }
    match value_ref(owner) {
        Ok(Value::Object(properties)) => properties.get(key).copied(),
        Ok(Value::Function(function)) => function.properties.get(key).copied(),
        Ok(Value::Array(values)) => property_array_index(key)
            .and_then(|index| values.get(index).copied().flatten())
            .or_else(|| host_property_for_owner(env, owner as usize, key)),
        Ok(Value::Buffer(bytes)) => property_array_index(key)
            .and_then(|index| bytes.get(index).copied())
            .and_then(|byte| {
                env_mut(env)
                    .ok()
                    .map(|env| env.alloc(Value::Number(byte.into())))
            })
            .or_else(|| host_property_for_owner(env, owner as usize, key)),
        Ok(Value::ExternalBuffer { data, length }) => property_array_index(key)
            .filter(|index| *index < *length && !data.is_null())
            .and_then(|index| {
                env_mut(env)
                    .ok()
                    .map(|env| env.alloc(Value::Number((*data.add(index)).into())))
            })
            .or_else(|| host_property_for_owner(env, owner as usize, key)),
        Ok(Value::BufferView {
            array_buffer,
            byte_offset,
            length,
        }) => property_array_index(key)
            .filter(|index| *index < *length)
            .and_then(|index| {
                let (bytes, _, detached) = arraybuffer_parts(*array_buffer).ok()?;
                (!detached).then(|| *bytes.add(*byte_offset + index))
            })
            .and_then(|byte| {
                env_mut(env)
                    .ok()
                    .map(|env| env.alloc(Value::Number(byte.into())))
            })
            .or_else(|| host_property_for_owner(env, owner as usize, key)),
        Ok(Value::TypedArray { .. }) => read_typedarray_index(env, owner, key)
            .or_else(|| host_property_for_owner(env, owner as usize, key)),
        Ok(value) if is_object_value(value) => host_property_for_owner(env, owner as usize, key),
        _ => None,
    }
}

unsafe fn set_own_property(
    env: &mut Env,
    object: NapiValue,
    key: &PropertyKey,
    value: NapiValue,
) -> NapiStatus {
    if intrinsic_property_attributes(object, key).is_some() {
        if matches!(value_ref(object), Ok(Value::Array(_)))
            && matches!(key, PropertyKey::String(name) if name == "length")
        {
            let number = match value_ref(value).and_then(number_for_typedarray) {
                Ok(number)
                    if number.is_finite()
                        && number >= 0.0
                        && number <= u32::MAX as f64
                        && number.fract() == 0.0 =>
                {
                    number as usize
                }
                _ => {
                    let error = env.alloc(Value::Error("invalid array length".into()));
                    env.exception = Some(error);
                    return NAPI_PENDING_EXCEPTION;
                }
            };
            let Some(Value::Array(values)) = object.as_mut() else {
                unreachable!();
            };
            values.resize(number, None);
            return NAPI_OK;
        }
        return NAPI_GENERIC_FAILURE;
    }
    match object.as_mut() {
        Some(Value::Object(properties)) => {
            properties.insert(key.clone(), value);
        }
        Some(Value::Function(function)) => {
            function.properties.insert(key.clone(), value);
        }
        Some(Value::Array(values)) if property_array_index(key).is_some() => {
            let index = property_array_index(key).unwrap();
            if values.len() <= index {
                values.resize(index.saturating_add(1), None);
            }
            values[index] = Some(value);
        }
        Some(Value::Buffer(bytes)) if property_array_index(key).is_some() => {
            if let Some(byte) = bytes.get_mut(property_array_index(key).unwrap()) {
                *byte = match uint8_from_value(value) {
                    Ok(value) => value,
                    Err(status) => return status,
                };
            }
        }
        Some(Value::ExternalBuffer { data, length }) if property_array_index(key).is_some() => {
            let index = property_array_index(key).unwrap();
            if index < *length && !data.is_null() {
                *data.add(index) = match uint8_from_value(value) {
                    Ok(value) => value,
                    Err(status) => return status,
                };
            }
        }
        Some(Value::BufferView {
            array_buffer,
            byte_offset,
            length,
        }) if property_array_index(key).is_some() => {
            let index = property_array_index(key).unwrap();
            if index < *length {
                let Ok((bytes, _, detached)) = arraybuffer_parts(*array_buffer) else {
                    return NAPI_ARRAYBUFFER_EXPECTED;
                };
                if !detached {
                    *bytes.add(*byte_offset + index) = match uint8_from_value(value) {
                        Ok(value) => value,
                        Err(status) => return status,
                    };
                }
            }
        }
        Some(Value::TypedArray { .. }) if property_array_index(key).is_some() => {
            if let Some(status) = write_typedarray_index(env, object, key, value) {
                return status;
            }
        }
        Some(object_value) if is_object_value(object_value) => {
            env.host_properties
                .entry(object as usize)
                .or_default()
                .insert(key.clone(), value);
        }
        _ => return NAPI_OBJECT_EXPECTED,
    }
    NAPI_OK
}

unsafe fn uint8_from_value(value: NapiValue) -> Result<u8, NapiStatus> {
    let number = match value_ref(value)? {
        Value::Undefined => f64::NAN,
        Value::Null => 0.0,
        Value::Bool(value) => u8::from(*value) as f64,
        Value::Number(value) => *value,
        Value::String(value) => javascript_number_from_string(value),
        Value::BigInt { .. } | Value::Symbol { .. } => return Err(NAPI_GENERIC_FAILURE),
        _ => f64::NAN,
    };
    Ok(if number.is_finite() {
        number.trunc().rem_euclid(256.0) as u8
    } else {
        0
    })
}

unsafe fn fixed_index_exists(object: NapiValue, key: &PropertyKey) -> bool {
    if matches!(value_ref(object), Ok(Value::Array(_)))
        && intrinsic_property_attributes(object, key).is_some()
    {
        return true;
    }
    let Some(index) = property_array_index(key) else {
        return false;
    };
    match value_ref(object) {
        Ok(Value::Buffer(bytes)) => index < bytes.len(),
        Ok(Value::ExternalBuffer { data, length }) => index < *length && !data.is_null(),
        Ok(Value::BufferView {
            array_buffer,
            length,
            ..
        }) => {
            index < *length
                && arraybuffer_parts(*array_buffer).is_ok_and(|(_, _, detached)| !detached)
        }
        Ok(Value::TypedArray { .. }) => typedarray_index_parts(object, key).is_some(),
        _ => false,
    }
}

unsafe fn remove_own_property(env: &mut Env, object: NapiValue, key: &PropertyKey) -> NapiStatus {
    match object.as_mut() {
        Some(Value::Object(properties)) => {
            properties.remove(key);
        }
        Some(Value::Function(function)) => {
            function.properties.remove(key);
        }
        Some(Value::Array(values)) if property_array_index(key).is_some() => {
            if let Some(value) = values.get_mut(property_array_index(key).unwrap()) {
                *value = None;
            }
        }
        Some(object_value) if is_object_value(object_value) => {
            if let Some(properties) = env.host_properties.get_mut(&(object as usize)) {
                properties.remove(key);
            }
        }
        _ => return NAPI_OBJECT_EXPECTED,
    }
    NAPI_OK
}

unsafe fn find_accessor(env: NapiEnv, object: NapiValue, name: &PropertyKey) -> Option<Accessor> {
    let mut current = Some(object as usize);
    let mut visited = HashSet::new();
    while let Some(owner) = current.filter(|owner| visited.insert(*owner)) {
        let has_data_property = own_property_value(env, owner as NapiValue, name).is_some();
        if has_data_property {
            return None;
        }
        if let Some(accessor) = accessor_for_owner(env, owner, name) {
            return Some(accessor);
        }
        current = prototype_for_owner(env, owner);
    }
    None
}

unsafe fn accessor_for_owner(env: NapiEnv, owner: usize, key: &PropertyKey) -> Option<Accessor> {
    env.as_ref()
        .and_then(|env| env.accessors.get(&(owner, key.clone())).copied())
        .or_else(|| {
            HOST.with(|host| {
                host.borrow()
                    .module_envs
                    .iter()
                    .find_map(|module_env| module_env.accessors.get(&(owner, key.clone())).copied())
            })
        })
}

unsafe fn prototype_for_owner(env: NapiEnv, owner: usize) -> Option<usize> {
    env.as_ref()
        .and_then(|env| env.prototypes.get(&owner).copied())
        .or_else(|| {
            HOST.with(|host| {
                host.borrow()
                    .module_envs
                    .iter()
                    .find_map(|module_env| module_env.prototypes.get(&owner).copied())
            })
        })
}

unsafe fn accessors_for_owner(env: NapiEnv, owner: usize) -> Vec<PropertyKey> {
    let mut keys = env
        .as_ref()
        .map(|env| {
            env.accessors
                .keys()
                .filter(|(accessor_owner, _)| *accessor_owner == owner)
                .map(|(_, key)| key.clone())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    HOST.with(|host| {
        for module_env in &host.borrow().module_envs {
            keys.extend(
                module_env
                    .accessors
                    .keys()
                    .filter(|(accessor_owner, _)| *accessor_owner == owner)
                    .map(|(_, key)| key.clone()),
            );
        }
    });
    keys
}

unsafe fn property_attributes_for(env: NapiEnv, owner: usize, key: &PropertyKey) -> u32 {
    if let Some(attributes) = intrinsic_property_attributes(owner as NapiValue, key) {
        return attributes;
    }
    env.as_ref()
        .and_then(|env| env.property_attributes.get(&(owner, key.clone())).copied())
        .or_else(|| {
            HOST.with(|host| {
                host.borrow().module_envs.iter().find_map(|module_env| {
                    module_env
                        .property_attributes
                        .get(&(owner, key.clone()))
                        .copied()
                })
            })
        })
        .unwrap_or(NAPI_DEFAULT_PROPERTY_ATTRIBUTES)
}

unsafe fn symbol_for(env: NapiEnv, id: u64) -> Option<NapiValue> {
    env.as_ref()
        .and_then(|env| env.symbols.get(&id).copied())
        .or_else(|| {
            HOST.with(|host| {
                host.borrow()
                    .module_envs
                    .iter()
                    .find_map(|module_env| module_env.symbols.get(&id).copied())
            })
        })
}

unsafe fn type_tag_for(env: NapiEnv, object: NapiValue) -> Option<NapiTypeTag> {
    env.as_ref()
        .and_then(|env| env.type_tags.get(&(object as usize)).copied())
        .or_else(|| {
            HOST.with(|host| {
                host.borrow()
                    .module_envs
                    .iter()
                    .find_map(|module_env| module_env.type_tags.get(&(object as usize)).copied())
            })
        })
}

unsafe fn find_property_value(
    env: NapiEnv,
    object: NapiValue,
    key: &PropertyKey,
) -> Option<NapiValue> {
    let mut current = Some(object);
    let mut visited = HashSet::new();
    while let Some(value) = current.filter(|value| visited.insert(*value as usize)) {
        let property = own_property_value(env, value, key);
        if property.is_some() {
            return property;
        }
        if let Some(property) = intrinsic_property_value(env, value, key) {
            return Some(property);
        }
        if accessor_for_owner(env, value as usize, key).is_some() {
            return None;
        }
        current = prototype_for_owner(env, value as usize).map(|prototype| prototype as NapiValue);
    }
    None
}

unsafe fn find_data_property_owner(
    env: NapiEnv,
    object: NapiValue,
    key: &PropertyKey,
) -> Option<usize> {
    let mut current = Some(object as usize);
    let mut visited = HashSet::new();
    while let Some(owner) = current.filter(|owner| visited.insert(*owner)) {
        let contains = own_property_value(env, owner as NapiValue, key).is_some();
        if contains {
            return Some(owner);
        }
        if accessor_for_owner(env, owner, key).is_some() {
            return None;
        }
        current = prototype_for_owner(env, owner);
    }
    None
}

unsafe fn write_value(out: *mut NapiValue, value: NapiValue) -> NapiStatus {
    if out.is_null() {
        return NAPI_INVALID_ARG;
    }
    *out = value;
    NAPI_OK
}

unsafe fn write_callback_value(env: NapiEnv, out: *mut NapiValue, value: NapiValue) -> NapiStatus {
    if out.is_null() {
        return NAPI_INVALID_ARG;
    }
    if value.is_null() {
        return napi_get_undefined(env, out);
    }
    if !value_belongs_to_environment(env, value) {
        return NAPI_INVALID_ARG;
    }
    write_value(out, value)
}

#[no_mangle]
pub unsafe extern "C" fn napi_module_register(module: *mut NapiModule) {
    if let Some(module) = module.as_ref() {
        PENDING_MODULE.with(|slot| *slot.borrow_mut() = Some(*module));
    }
}

type RegisterV1 = unsafe extern "C" fn(NapiEnv, NapiValue) -> NapiValue;

unsafe fn load_impl(path: &str, root_name: Option<&str>) -> Result<(), String> {
    let path_text = path;
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
    let absolute_path =
        std::fs::canonicalize(path_text).unwrap_or_else(|_| std::path::PathBuf::from(path_text));
    env.module_file_name = CString::new(format!("file://{}", absolute_path.to_string_lossy()))
        .unwrap_or_else(|_| CString::new("").unwrap());
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
                    if let PropertyKey::String(name) = name {
                        functions.push((name.clone(), function.clone()));
                        exported_values.push((name.clone(), *value));
                    }
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
        || ACTIVE_ASYNC_CLEANUP_HOOKS.load(Ordering::Acquire) != 0
    {
        HOST.with(|host| {
            host.borrow_mut().last_error =
                "cannot unload N-API addons while asynchronous work or cleanup is active".into();
        });
        return 0;
    }
    HOST.with(|host| {
        let mut host = host.borrow_mut();
        host.functions.clear();
        host.exports.clear();
        host.compiled_callbacks.clear();
        host.pending_call_envs.clear();
        // Cleanup hooks and native finalizers must run while their addon code
        // is still mapped.
        host.module_envs.clear();
        if ACTIVE_ASYNC_CLEANUP_HOOKS.load(Ordering::Acquire) != 0 {
            host.last_error =
                "cannot unload N-API addons while asynchronous cleanup is active".into();
            return 0;
        }
        for handle in host.libraries.drain(..).rev() {
            unsafe {
                libc::dlclose(handle);
            }
        }
        1
    })
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
                .map(|value| Some(value_from_json(env, value)))
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
                .map(|(key, value)| (key.clone().into(), value_from_json(env, value)))
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
        Value::Number(value) | Value::Date(value) => serde_json::Number::from_f64(*value)
            .map(JsonValue::Number)
            .unwrap_or(JsonValue::Null),
        Value::String(value) | Value::Error(value) => JsonValue::String(value.clone()),
        Value::Symbol { .. } => return Err("cannot JSON-encode a Symbol".into()),
        Value::Array(values) => JsonValue::Array(
            values
                .iter()
                .map(|value| match value {
                    Some(value) => json_from_value(*value),
                    None => Ok(JsonValue::Null),
                })
                .collect::<Result<_, _>>()?,
        ),
        Value::Object(values) => JsonValue::Object(
            values
                .iter()
                .filter_map(|(key, value)| match key {
                    PropertyKey::String(key) => {
                        Some(json_from_value(*value).map(|value| (key.clone(), value)))
                    }
                    PropertyKey::Symbol(_) => None,
                })
                .collect::<Result<_, String>>()?,
        ),
        Value::Buffer(values) => {
            JsonValue::Array(values.iter().map(|value| JsonValue::from(*value)).collect())
        }
        Value::ExternalBuffer { data, length } => {
            let bytes = if *length == 0 {
                &[]
            } else {
                std::slice::from_raw_parts(*data, *length)
            };
            JsonValue::Array(bytes.iter().map(|value| JsonValue::from(*value)).collect())
        }
        Value::BufferView {
            array_buffer,
            byte_offset,
            length,
        } => {
            let (data, _, detached) = arraybuffer_parts(*array_buffer)
                .map_err(|_| "invalid Buffer backing ArrayBuffer")?;
            let bytes = if detached || *length == 0 {
                &[]
            } else {
                std::slice::from_raw_parts(data.add(*byte_offset), *length)
            };
            JsonValue::Array(bytes.iter().map(|value| JsonValue::from(*value)).collect())
        }
        Value::ArrayBuffer { .. }
        | Value::SharedArrayBuffer(_)
        | Value::ExternalSharedArrayBuffer { .. }
        | Value::ExternalArrayBuffer { .. }
        | Value::TypedArray { .. }
        | Value::DataView { .. } => {
            return Err("cannot JSON-encode an ArrayBuffer view".into());
        }
        Value::Function(_) => return Err("cannot JSON-encode a function".into()),
        Value::External(_) => return Err("cannot JSON-encode an external value".into()),
        Value::BigInt { .. } => return Err("cannot JSON-encode a BigInt".into()),
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

fn module_file_name_for_export(name: &str) -> Option<CString> {
    HOST.with(|host| {
        let host = host.borrow();
        let (env, _) = host.exports.get(name)?;
        unsafe { Some((*(*env as NapiEnv)).module_file_name.clone()) }
    })
}

unsafe fn call_impl(name: &str, args_json: &str) -> Result<String, String> {
    let args: Vec<JsonValue> = serde_json::from_str(args_json)
        .map_err(|error| format!("invalid argument JSON: {error}"))?;
    let function = HOST
        .with(|host| host.borrow().functions.get(name).cloned())
        .ok_or_else(|| format!("no such native addon function `{name}`"))?;
    let mut env = Box::new(Env::new());
    if let Some(module_file_name) = module_file_name_for_export(name) {
        env.module_file_name = module_file_name;
    }
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
        if let Some(module_file_name) = module_file_name_for_export(&name) {
            env.module_file_name = module_file_name;
        }
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
    let Ok(env) = env_for_value_output(env, out) else {
        return NAPI_INVALID_ARG;
    };
    let value = env.alloc(Value::Undefined);
    write_value(out, value)
}
#[no_mangle]
pub unsafe extern "C" fn napi_get_null(env: NapiEnv, out: *mut NapiValue) -> NapiStatus {
    let Ok(env) = env_for_value_output(env, out) else {
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
    let Ok(env) = env_for_value_output(env, out) else {
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
    let Ok(env) = env_for_value_output(env, out) else {
        return NAPI_INVALID_ARG;
    };
    let value = env.alloc(Value::Number(value));
    write_value(out, value)
}

#[no_mangle]
pub unsafe extern "C" fn napi_create_int32(
    env: NapiEnv,
    value: i32,
    out: *mut NapiValue,
) -> NapiStatus {
    napi_create_double(env, f64::from(value), out)
}

#[no_mangle]
pub unsafe extern "C" fn napi_create_uint32(
    env: NapiEnv,
    value: u32,
    out: *mut NapiValue,
) -> NapiStatus {
    napi_create_double(env, f64::from(value), out)
}

#[no_mangle]
pub unsafe extern "C" fn napi_create_int64(
    env: NapiEnv,
    value: i64,
    out: *mut NapiValue,
) -> NapiStatus {
    napi_create_double(env, value as f64, out)
}

#[no_mangle]
pub unsafe extern "C" fn napi_create_date(
    env: NapiEnv,
    value: f64,
    out: *mut NapiValue,
) -> NapiStatus {
    let Ok(env) = env_for_value_output(env, out) else {
        return NAPI_INVALID_ARG;
    };
    let value = env.alloc(Value::Date(value));
    write_value(out, value)
}

#[no_mangle]
pub unsafe extern "C" fn napi_is_date(
    env: NapiEnv,
    value: NapiValue,
    out: *mut bool,
) -> NapiStatus {
    if out.is_null() || !value_belongs_to_environment(env, value) {
        return record_status(env, NAPI_INVALID_ARG);
    }
    *out = matches!(value_ref(value), Ok(Value::Date(_)));
    NAPI_OK
}

#[no_mangle]
pub unsafe extern "C" fn napi_get_date_value(
    env: NapiEnv,
    value: NapiValue,
    out: *mut f64,
) -> NapiStatus {
    if out.is_null() || !value_belongs_to_environment(env, value) {
        return record_status(env, NAPI_INVALID_ARG);
    }
    let status = match value_ref(value) {
        Ok(Value::Date(milliseconds)) => {
            *out = *milliseconds;
            NAPI_OK
        }
        _ => NAPI_DATE_EXPECTED,
    };
    record_status(env, status)
}

#[no_mangle]
pub unsafe extern "C" fn napi_create_bigint_int64(
    env: NapiEnv,
    value: i64,
    out: *mut NapiValue,
) -> NapiStatus {
    let Ok(env) = env_for_value_output(env, out) else {
        return NAPI_INVALID_ARG;
    };
    let magnitude = value.unsigned_abs();
    let value = env.alloc(Value::BigInt {
        negative: value.is_negative(),
        words: vec![magnitude],
    });
    write_value(out, value)
}

#[no_mangle]
pub unsafe extern "C" fn napi_create_bigint_uint64(
    env: NapiEnv,
    value: u64,
    out: *mut NapiValue,
) -> NapiStatus {
    let Ok(env) = env_for_value_output(env, out) else {
        return NAPI_INVALID_ARG;
    };
    let value = env.alloc(Value::BigInt {
        negative: false,
        words: vec![value],
    });
    write_value(out, value)
}

#[no_mangle]
pub unsafe extern "C" fn napi_create_bigint_words(
    env: NapiEnv,
    sign_bit: i32,
    word_count: usize,
    words: *const u64,
    out: *mut NapiValue,
) -> NapiStatus {
    if out.is_null() || sign_bit != 0 && sign_bit != 1 || (word_count != 0 && words.is_null()) {
        return record_status(env, NAPI_INVALID_ARG);
    }
    let Ok(env) = env_for_value_output(env, out) else {
        return NAPI_INVALID_ARG;
    };
    let mut magnitude = if word_count == 0 {
        Vec::new()
    } else {
        std::slice::from_raw_parts(words, word_count).to_vec()
    };
    while magnitude.last() == Some(&0) {
        magnitude.pop();
    }
    if magnitude.is_empty() {
        magnitude.push(0);
    }
    let value = env.alloc(Value::BigInt {
        negative: sign_bit == 1 && magnitude != [0],
        words: magnitude,
    });
    write_value(out, value)
}

#[no_mangle]
pub unsafe extern "C" fn napi_get_value_bigint_int64(
    env: NapiEnv,
    value: NapiValue,
    out: *mut i64,
    lossless: *mut bool,
) -> NapiStatus {
    if out.is_null() || lossless.is_null() || !value_belongs_to_environment(env, value) {
        return record_status(env, NAPI_INVALID_ARG);
    }
    let Value::BigInt { negative, words } = (match value_ref(value) {
        Ok(value) => value,
        Err(_) => return record_status(env, NAPI_INVALID_ARG),
    }) else {
        return record_status(env, NAPI_BIGINT_EXPECTED);
    };
    let low = words.first().copied().unwrap_or(0);
    *out = if *negative {
        low.wrapping_neg() as i64
    } else {
        low as i64
    };
    *lossless = words.len() <= 1
        && if *negative {
            low <= (i64::MAX as u64) + 1
        } else {
            low <= i64::MAX as u64
        };
    NAPI_OK
}

#[no_mangle]
pub unsafe extern "C" fn napi_get_value_bigint_uint64(
    env: NapiEnv,
    value: NapiValue,
    out: *mut u64,
    lossless: *mut bool,
) -> NapiStatus {
    if out.is_null() || lossless.is_null() || !value_belongs_to_environment(env, value) {
        return record_status(env, NAPI_INVALID_ARG);
    }
    let Value::BigInt { negative, words } = (match value_ref(value) {
        Ok(value) => value,
        Err(_) => return record_status(env, NAPI_INVALID_ARG),
    }) else {
        return record_status(env, NAPI_BIGINT_EXPECTED);
    };
    let low = words.first().copied().unwrap_or(0);
    *out = if *negative { low.wrapping_neg() } else { low };
    *lossless = !*negative && words.len() <= 1;
    NAPI_OK
}

#[no_mangle]
pub unsafe extern "C" fn napi_get_value_bigint_words(
    env: NapiEnv,
    value: NapiValue,
    sign_bit: *mut i32,
    word_count: *mut usize,
    words: *mut u64,
) -> NapiStatus {
    if sign_bit.is_null() || word_count.is_null() || !value_belongs_to_environment(env, value) {
        return record_status(env, NAPI_INVALID_ARG);
    }
    let Value::BigInt {
        negative,
        words: magnitude,
    } = (match value_ref(value) {
        Ok(value) => value,
        Err(_) => return record_status(env, NAPI_INVALID_ARG),
    })
    else {
        return record_status(env, NAPI_BIGINT_EXPECTED);
    };
    *sign_bit = i32::from(*negative);
    let capacity = *word_count;
    *word_count = magnitude.len();
    if !words.is_null() && capacity != 0 {
        ptr::copy_nonoverlapping(magnitude.as_ptr(), words, capacity.min(magnitude.len()));
    }
    NAPI_OK
}

#[no_mangle]
pub unsafe extern "C" fn napi_create_string_utf8(
    env: NapiEnv,
    value: *const c_char,
    length: usize,
    out: *mut NapiValue,
) -> NapiStatus {
    if value.is_null() {
        return record_status(env, NAPI_INVALID_ARG);
    }
    let Ok(env) = env_for_value_output(env, out) else {
        return NAPI_INVALID_ARG;
    };
    let bytes = if length == NAPI_AUTO_LENGTH {
        CStr::from_ptr(value).to_bytes()
    } else {
        std::slice::from_raw_parts(value.cast(), length)
    };
    let string = String::from_utf8_lossy(bytes).into_owned();
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
        return record_status(env, NAPI_INVALID_ARG);
    }
    let Ok(env) = env_for_value_output(env, out) else {
        return NAPI_INVALID_ARG;
    };
    let bytes = if length == NAPI_AUTO_LENGTH {
        CStr::from_ptr(value).to_bytes()
    } else {
        std::slice::from_raw_parts(value.cast::<u8>(), length)
    };
    let string: String = bytes.iter().map(|byte| char::from(*byte)).collect();
    let value = env.alloc(Value::String(string));
    write_value(out, value)
}

#[no_mangle]
pub unsafe extern "C" fn napi_create_string_utf16(
    env: NapiEnv,
    value: *const u16,
    length: usize,
    out: *mut NapiValue,
) -> NapiStatus {
    if value.is_null() {
        return record_status(env, NAPI_INVALID_ARG);
    }
    let Ok(env) = env_for_value_output(env, out) else {
        return NAPI_INVALID_ARG;
    };
    let length = if length == NAPI_AUTO_LENGTH {
        let mut length = 0;
        while *value.add(length) != 0 {
            length += 1;
        }
        length
    } else {
        length
    };
    let string = String::from_utf16_lossy(std::slice::from_raw_parts(value, length));
    let value = env.alloc(Value::String(string));
    write_value(out, value)
}

fn intern_property_key(env: &mut Env, key: String) -> NapiValue {
    if let Some(value) = env.property_keys.get(&key).copied() {
        return value;
    }
    let value = env.alloc(Value::String(key.clone()));
    env.property_keys.insert(key, value);
    value
}

#[no_mangle]
pub unsafe extern "C" fn node_api_create_property_key_utf8(
    env: NapiEnv,
    value: *const c_char,
    length: usize,
    out: *mut NapiValue,
) -> NapiStatus {
    if value.is_null() {
        return record_status(env, NAPI_INVALID_ARG);
    }
    let Ok(env) = env_for_value_output(env, out) else {
        return NAPI_INVALID_ARG;
    };
    let bytes = if length == NAPI_AUTO_LENGTH {
        CStr::from_ptr(value).to_bytes()
    } else {
        std::slice::from_raw_parts(value.cast::<u8>(), length)
    };
    let key = String::from_utf8_lossy(bytes).into_owned();
    write_value(out, intern_property_key(env, key))
}

#[no_mangle]
pub unsafe extern "C" fn node_api_create_property_key_latin1(
    env: NapiEnv,
    value: *const c_char,
    length: usize,
    out: *mut NapiValue,
) -> NapiStatus {
    if value.is_null() {
        return record_status(env, NAPI_INVALID_ARG);
    }
    let Ok(env) = env_for_value_output(env, out) else {
        return NAPI_INVALID_ARG;
    };
    let bytes = if length == NAPI_AUTO_LENGTH {
        CStr::from_ptr(value).to_bytes()
    } else {
        std::slice::from_raw_parts(value.cast::<u8>(), length)
    };
    let key = bytes.iter().map(|byte| char::from(*byte)).collect();
    write_value(out, intern_property_key(env, key))
}

#[no_mangle]
pub unsafe extern "C" fn node_api_create_property_key_utf16(
    env: NapiEnv,
    value: *const u16,
    length: usize,
    out: *mut NapiValue,
) -> NapiStatus {
    if value.is_null() {
        return record_status(env, NAPI_INVALID_ARG);
    }
    let Ok(env) = env_for_value_output(env, out) else {
        return NAPI_INVALID_ARG;
    };
    let length = if length == NAPI_AUTO_LENGTH {
        let mut length = 0;
        while *value.add(length) != 0 {
            length += 1;
        }
        length
    } else {
        length
    };
    let key = String::from_utf16_lossy(std::slice::from_raw_parts(value, length));
    write_value(out, intern_property_key(env, key))
}

#[no_mangle]
pub unsafe extern "C" fn node_api_create_external_string_latin1(
    env: NapiEnv,
    value: *mut c_char,
    length: usize,
    finalize: Option<NapiFinalize>,
    hint: *mut c_void,
    out: *mut NapiValue,
    copied: *mut bool,
) -> NapiStatus {
    if value.is_null() || out.is_null() || copied.is_null() {
        return record_status(env, NAPI_INVALID_ARG);
    }
    let status = napi_create_string_latin1(env, value, length, out);
    if status != NAPI_OK {
        return record_status(env, status);
    }
    *copied = true;
    if let Some(finalize) = finalize {
        finalize(env, value.cast(), hint);
    }
    NAPI_OK
}

#[no_mangle]
pub unsafe extern "C" fn node_api_create_external_string_utf16(
    env: NapiEnv,
    value: *mut u16,
    length: usize,
    finalize: Option<NapiFinalize>,
    hint: *mut c_void,
    out: *mut NapiValue,
    copied: *mut bool,
) -> NapiStatus {
    if value.is_null() || out.is_null() || copied.is_null() {
        return record_status(env, NAPI_INVALID_ARG);
    }
    let status = napi_create_string_utf16(env, value, length, out);
    if status != NAPI_OK {
        return record_status(env, status);
    }
    *copied = true;
    if let Some(finalize) = finalize {
        finalize(env, value.cast(), hint);
    }
    NAPI_OK
}

#[no_mangle]
pub unsafe extern "C" fn napi_create_symbol(
    env: NapiEnv,
    description: NapiValue,
    out: *mut NapiValue,
) -> NapiStatus {
    if out.is_null() || (!description.is_null() && !value_belongs_to_environment(env, description))
    {
        return record_status(env, NAPI_INVALID_ARG);
    }
    let description = if description.is_null() {
        String::new()
    } else {
        match value_ref(description) {
            Ok(Value::String(value)) => value.clone(),
            _ => return record_status(env, NAPI_STRING_EXPECTED),
        }
    };
    let Ok(env) = env_mut(env) else {
        return NAPI_INVALID_ARG;
    };
    let id = NEXT_SYMBOL_ID.fetch_add(1, Ordering::Relaxed);
    let value = env.alloc(Value::Symbol { id, description });
    env.symbols.insert(id, value);
    write_value(out, value)
}

#[no_mangle]
pub unsafe extern "C" fn node_api_symbol_for(
    env: NapiEnv,
    description: *const c_char,
    length: usize,
    out: *mut NapiValue,
) -> NapiStatus {
    if description.is_null() {
        return record_status(env, NAPI_INVALID_ARG);
    }
    let Ok(env) = env_for_value_output(env, out) else {
        return NAPI_INVALID_ARG;
    };
    let bytes = if length == NAPI_AUTO_LENGTH {
        CStr::from_ptr(description).to_bytes()
    } else {
        std::slice::from_raw_parts(description.cast::<u8>(), length)
    };
    let description = String::from_utf8_lossy(bytes).into_owned();
    let id = *GLOBAL_SYMBOLS
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .entry(description.clone())
        .or_insert_with(|| NEXT_SYMBOL_ID.fetch_add(1, Ordering::Relaxed));
    let value = if let Some(value) = env.symbols.get(&id).copied() {
        value
    } else {
        let value = env.alloc(Value::Symbol { id, description });
        env.symbols.insert(id, value);
        value
    };
    write_value(out, value)
}

#[no_mangle]
pub unsafe extern "C" fn napi_create_external(
    env: NapiEnv,
    data: *mut c_void,
    finalize: Option<unsafe extern "C" fn(NapiEnv, *mut c_void, *mut c_void)>,
    hint: *mut c_void,
    out: *mut NapiValue,
) -> NapiStatus {
    if out.is_null() {
        return record_status(env, NAPI_INVALID_ARG);
    }
    let Ok(env) = env_mut(env) else {
        return NAPI_INVALID_ARG;
    };
    let value = env.alloc(Value::External(data));
    if finalize.is_some() {
        env.finalizers.push(FinalizeRecord {
            data,
            finalize,
            hint,
        });
    }
    *out = value;
    NAPI_OK
}

#[no_mangle]
pub unsafe extern "C" fn napi_get_value_external(
    env: NapiEnv,
    value: NapiValue,
    out: *mut *mut c_void,
) -> NapiStatus {
    if out.is_null() || !value_belongs_to_environment(env, value) {
        return record_status(env, NAPI_INVALID_ARG);
    }
    let status = match value_ref(value) {
        Ok(Value::External(data)) => {
            *out = *data;
            NAPI_OK
        }
        _ => NAPI_INVALID_ARG,
    };
    record_status(env, status)
}

#[no_mangle]
pub unsafe extern "C" fn napi_create_object(env: NapiEnv, out: *mut NapiValue) -> NapiStatus {
    let Ok(env) = env_for_value_output(env, out) else {
        return NAPI_INVALID_ARG;
    };
    let value = env.alloc(Value::Object(HashMap::new()));
    write_value(out, value)
}

#[no_mangle]
pub unsafe extern "C" fn node_api_create_object_with_properties(
    env: NapiEnv,
    prototype_or_null: NapiValue,
    property_names: *mut NapiValue,
    property_values: *mut NapiValue,
    property_count: usize,
    out: *mut NapiValue,
) -> NapiStatus {
    if out.is_null() || !value_belongs_to_environment(env, prototype_or_null) {
        return record_status(env, NAPI_INVALID_ARG);
    }
    if !matches!(value_ref(prototype_or_null), Ok(value) if is_object_value(value) || matches!(value, Value::Null))
    {
        return record_status(env, NAPI_OBJECT_EXPECTED);
    }
    if property_count != 0 && (property_names.is_null() || property_values.is_null()) {
        return record_status(env, NAPI_INVALID_ARG);
    }

    let (names, values) = if property_count == 0 {
        (&[][..], &[][..])
    } else {
        (
            std::slice::from_raw_parts(property_names, property_count),
            std::slice::from_raw_parts(property_values, property_count),
        )
    };
    for (&name, &value) in names.iter().zip(values) {
        if !value_belongs_to_environment(env, name)
            || !value_belongs_to_environment(env, value)
            || property_key(name).is_err()
        {
            return record_status(env, NAPI_INVALID_ARG);
        }
    }

    let mut object = ptr::null_mut();
    let status = napi_create_object(env, &mut object);
    if status != NAPI_OK {
        return record_status(env, status);
    }
    let status = node_api_set_prototype(env, object, prototype_or_null);
    if status != NAPI_OK {
        return record_status(env, status);
    }
    for (&name, &value) in names.iter().zip(values) {
        let status = napi_set_property(env, object, name, value);
        if status != NAPI_OK {
            return record_status(env, status);
        }
    }
    write_value(out, object)
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
    let Ok(env) = env_for_value_output(env, out) else {
        return NAPI_INVALID_ARG;
    };
    let value = env.alloc(Value::Array(vec![None; length]));
    write_value(out, value)
}

#[no_mangle]
pub unsafe extern "C" fn napi_create_function(
    env: NapiEnv,
    name: *const c_char,
    length: usize,
    callback: Option<NapiCallback>,
    data: *mut c_void,
    out: *mut NapiValue,
) -> NapiStatus {
    let Some(callback) = callback else {
        return record_status(env, NAPI_INVALID_ARG);
    };
    if out.is_null() {
        return record_status(env, NAPI_INVALID_ARG);
    }
    let Ok(env) = env_for_value_output(env, out) else {
        return NAPI_INVALID_ARG;
    };
    let function_name = if name.is_null() {
        String::new()
    } else {
        let bytes = if length == NAPI_AUTO_LENGTH {
            CStr::from_ptr(name).to_bytes()
        } else {
            std::slice::from_raw_parts(name.cast::<u8>(), length)
        };
        String::from_utf8_lossy(bytes).into_owned()
    };
    let value = env.alloc(Value::Function(Function {
        callback,
        data,
        properties: HashMap::new(),
        _thaw_bridge: None,
    }));
    let name_value = env.alloc(Value::String(function_name));
    let length_value = env.alloc(Value::Number(0.0));
    let Some(Value::Function(function)) = value.as_mut() else {
        unreachable!();
    };
    function
        .properties
        .insert(PropertyKey::String("name".into()), name_value);
    function
        .properties
        .insert(PropertyKey::String("length".into()), length_value);
    for key in [
        PropertyKey::String("name".into()),
        PropertyKey::String("length".into()),
    ] {
        record_property_order(env, value as usize, &key);
        env.property_attributes
            .insert((value as usize, key), NAPI_CONFIGURABLE);
    }
    write_value(out, value)
}

#[no_mangle]
pub unsafe extern "C" fn napi_get_cb_info(
    env: NapiEnv,
    info: NapiCallbackInfo,
    argc: *mut usize,
    argv: *mut NapiValue,
    this_arg: *mut NapiValue,
    data: *mut *mut c_void,
) -> NapiStatus {
    if env.is_null() {
        return NAPI_INVALID_ARG;
    }
    let Some(info) = info.as_ref() else {
        return record_status(env, NAPI_INVALID_ARG);
    };
    if !argc.is_null() {
        let capacity = *argc;
        *argc = info.args.len();
        if !argv.is_null() {
            let copied = capacity.min(info.args.len());
            ptr::copy_nonoverlapping(info.args.as_ptr(), argv, copied);
            if copied < capacity {
                let mut undefined = ptr::null_mut();
                let status = napi_get_undefined(env, &mut undefined);
                if status != NAPI_OK {
                    return record_status(env, status);
                }
                for index in copied..capacity {
                    *argv.add(index) = undefined;
                }
            }
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
    env: NapiEnv,
    info: NapiCallbackInfo,
    result: *mut NapiValue,
) -> NapiStatus {
    if env.is_null() || result.is_null() {
        return record_status(env, NAPI_INVALID_ARG);
    }
    let Some(info) = info.as_ref() else {
        return record_status(env, NAPI_INVALID_ARG);
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
    if !value_belongs_to_environment(env, object) || !value_belongs_to_environment(env, value) {
        return NAPI_INVALID_ARG;
    }
    let Ok(name) = text(name) else {
        return record_status(env, NAPI_INVALID_ARG);
    };
    let name = PropertyKey::String(name);
    if let Some(accessor) = find_accessor(env, object, &name) {
        let Some(setter) = accessor.setter else {
            return record_status(env, NAPI_GENERIC_FAILURE);
        };
        let mut info = CallbackInfo {
            args: vec![value],
            this_arg: object,
            new_target: ptr::null_mut(),
            data: accessor.data,
        };
        setter(env, &mut info);
        let status = if env_mut(env)
            .map(|env| env.exception.is_some())
            .unwrap_or(false)
        {
            NAPI_PENDING_EXCEPTION
        } else {
            NAPI_OK
        };
        return record_status(env, status);
    }
    let inherited_owner = find_data_property_owner(env, object, &name);
    let (frozen, sealed) = env
        .as_ref()
        .map(|env| {
            (
                env.frozen_objects.contains(&(object as usize)),
                env.sealed_objects.contains(&(object as usize)),
            )
        })
        .unwrap_or((false, false));
    let writable = inherited_owner
        .map(|owner| property_attributes_for(env, owner, &name) & NAPI_WRITABLE != 0)
        .unwrap_or(true);
    if !matches!(value_ref(object), Ok(value) if is_object_value(value)) {
        return record_status(env, NAPI_OBJECT_EXPECTED);
    }
    let exists = own_property_value(env, object, &name).is_some();
    if frozen || (inherited_owner.is_some() && !writable) || (sealed && !exists) {
        return record_status(env, NAPI_GENERIC_FAILURE);
    }
    let status = match env_mut(env) {
        Ok(env) => set_own_property(env, object, &name, value),
        Err(status) => status,
    };
    if status == NAPI_OK && !exists {
        if let Ok(env) = env_mut(env) {
            record_property_order(env, object as usize, &name);
            env.property_attributes
                .insert((object as usize, name), NAPI_DEFAULT_PROPERTY_ATTRIBUTES);
        }
    }
    record_status(env, status)
}

#[no_mangle]
pub unsafe extern "C" fn napi_get_named_property(
    env: NapiEnv,
    object: NapiValue,
    name: *const c_char,
    out: *mut NapiValue,
) -> NapiStatus {
    if out.is_null() || !value_belongs_to_environment(env, object) {
        return record_status(env, NAPI_INVALID_ARG);
    }
    let Ok(name) = text(name) else {
        return record_status(env, NAPI_INVALID_ARG);
    };
    let name = PropertyKey::String(name);
    let value = find_property_value(env, object, &name);
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
                return record_status(env, NAPI_PENDING_EXCEPTION);
            }
            let status = write_callback_value(env, out, value);
            return record_status(env, status);
        }
    }
    let status = match value {
        Some(value) => write_value(out, value),
        None => napi_get_undefined(env, out),
    };
    record_status(env, status)
}

unsafe fn property_key(value: NapiValue) -> Result<PropertyKey, NapiStatus> {
    match value_ref(value) {
        Ok(Value::String(value)) => Ok(PropertyKey::String(value.clone())),
        Ok(Value::Symbol { id, .. }) => Ok(PropertyKey::Symbol(*id)),
        _ => Err(NAPI_STRING_EXPECTED),
    }
}

unsafe fn set_property_key(
    env: NapiEnv,
    object: NapiValue,
    key: PropertyKey,
    value: NapiValue,
) -> NapiStatus {
    if let PropertyKey::String(name) = &key {
        let Ok(name) = CString::new(name.as_str()) else {
            return record_status(env, NAPI_INVALID_ARG);
        };
        return napi_set_named_property(env, object, name.as_ptr(), value);
    }
    if let Some(accessor) = find_accessor(env, object, &key) {
        let Some(setter) = accessor.setter else {
            return record_status(env, NAPI_GENERIC_FAILURE);
        };
        let mut info = CallbackInfo {
            args: vec![value],
            this_arg: object,
            new_target: ptr::null_mut(),
            data: accessor.data,
        };
        setter(env, &mut info);
        let status = if env_mut(env)
            .map(|env| env.exception.is_some())
            .unwrap_or(false)
        {
            NAPI_PENDING_EXCEPTION
        } else {
            NAPI_OK
        };
        return record_status(env, status);
    }
    if !matches!(value_ref(object), Ok(value) if is_object_value(value)) {
        return record_status(env, NAPI_OBJECT_EXPECTED);
    }
    let exists = own_property_value(env, object, &key).is_some();
    let Some(env_ref) = env.as_ref() else {
        return record_status(env, NAPI_INVALID_ARG);
    };
    let property_owner = find_data_property_owner(env, object, &key);
    let writable = property_owner
        .map(|owner| property_attributes_for(env, owner, &key) & NAPI_WRITABLE != 0)
        .unwrap_or(true);
    if env_ref.frozen_objects.contains(&(object as usize))
        || (property_owner.is_some() && !writable)
        || (env_ref.sealed_objects.contains(&(object as usize)) && !exists)
    {
        return record_status(env, NAPI_GENERIC_FAILURE);
    }
    let Ok(env_ref) = env_mut(env) else {
        return record_status(env, NAPI_INVALID_ARG);
    };
    let status = set_own_property(env_ref, object, &key, value);
    if status != NAPI_OK {
        return record_status(env, status);
    }
    if !exists {
        if let Ok(env) = env_mut(env) {
            record_property_order(env, object as usize, &key);
            env.property_attributes
                .insert((object as usize, key), NAPI_DEFAULT_PROPERTY_ATTRIBUTES);
        }
    }
    NAPI_OK
}

unsafe fn get_property_key(
    env: NapiEnv,
    object: NapiValue,
    key: &PropertyKey,
    out: *mut NapiValue,
) -> NapiStatus {
    if let PropertyKey::String(name) = key {
        let Ok(name) = CString::new(name.as_str()) else {
            return record_status(env, NAPI_INVALID_ARG);
        };
        return napi_get_named_property(env, object, name.as_ptr(), out);
    }
    if !matches!(value_ref(object), Ok(value) if is_object_value(value)) {
        return record_status(env, NAPI_OBJECT_EXPECTED);
    }
    let value = find_property_value(env, object, key);
    if value.is_none() {
        if let Some(accessor) = find_accessor(env, object, key) {
            if let Some(getter) = accessor.getter {
                let mut info = CallbackInfo {
                    args: Vec::new(),
                    this_arg: object,
                    new_target: ptr::null_mut(),
                    data: accessor.data,
                };
                let value = getter(env, &mut info);
                if env_mut(env)
                    .map(|env| env.exception.is_some())
                    .unwrap_or(false)
                {
                    return record_status(env, NAPI_PENDING_EXCEPTION);
                }
                let status = write_callback_value(env, out, value);
                return record_status(env, status);
            }
        }
    }
    let status = match value {
        Some(value) => write_value(out, value),
        None => napi_get_undefined(env, out),
    };
    record_status(env, status)
}

#[no_mangle]
pub unsafe extern "C" fn napi_set_property(
    env: NapiEnv,
    object: NapiValue,
    key: NapiValue,
    value: NapiValue,
) -> NapiStatus {
    if !value_belongs_to_environment(env, object)
        || !value_belongs_to_environment(env, key)
        || !value_belongs_to_environment(env, value)
    {
        return NAPI_INVALID_ARG;
    }
    let Ok(key) = property_key(key) else {
        return record_status(env, NAPI_INVALID_ARG);
    };
    let status = set_property_key(env, object, key, value);
    record_status(env, status)
}

#[no_mangle]
pub unsafe extern "C" fn napi_get_property(
    env: NapiEnv,
    object: NapiValue,
    key: NapiValue,
    out: *mut NapiValue,
) -> NapiStatus {
    if out.is_null()
        || !value_belongs_to_environment(env, object)
        || !value_belongs_to_environment(env, key)
    {
        return record_status(env, NAPI_INVALID_ARG);
    }
    let Ok(key) = property_key(key) else {
        return record_status(env, NAPI_INVALID_ARG);
    };
    let status = get_property_key(env, object, &key, out);
    record_status(env, status)
}

#[no_mangle]
pub unsafe extern "C" fn napi_has_property(
    env: NapiEnv,
    object: NapiValue,
    key: NapiValue,
    out: *mut bool,
) -> NapiStatus {
    if out.is_null()
        || !value_belongs_to_environment(env, object)
        || !value_belongs_to_environment(env, key)
    {
        return record_status(env, NAPI_INVALID_ARG);
    }
    let Ok(key) = property_key(key) else {
        return record_status(env, NAPI_INVALID_ARG);
    };
    if !matches!(value_ref(object), Ok(value) if is_object_value(value)) {
        return record_status(env, NAPI_OBJECT_EXPECTED);
    }
    *out = find_property_value(env, object, &key).is_some()
        || find_accessor(env, object, &key).is_some();
    NAPI_OK
}

#[no_mangle]
pub unsafe extern "C" fn napi_has_own_property(
    env: NapiEnv,
    object: NapiValue,
    key: NapiValue,
    out: *mut bool,
) -> NapiStatus {
    if out.is_null()
        || !value_belongs_to_environment(env, object)
        || !value_belongs_to_environment(env, key)
    {
        return record_status(env, NAPI_INVALID_ARG);
    }
    let Ok(key) = property_key(key) else {
        return record_status(env, NAPI_INVALID_ARG);
    };
    if !matches!(value_ref(object), Ok(value) if is_object_value(value)) {
        return record_status(env, NAPI_OBJECT_EXPECTED);
    }
    let own_value = own_property_value(env, object, &key).is_some();
    let own_accessor = accessor_for_owner(env, object as usize, &key).is_some();
    *out = own_value || own_accessor;
    NAPI_OK
}

#[no_mangle]
pub unsafe extern "C" fn napi_delete_property(
    env: NapiEnv,
    object: NapiValue,
    key: NapiValue,
    out: *mut bool,
) -> NapiStatus {
    if !value_belongs_to_environment(env, object) || !value_belongs_to_environment(env, key) {
        return NAPI_INVALID_ARG;
    }
    let Ok(key) = property_key(key) else {
        return record_status(env, NAPI_INVALID_ARG);
    };
    if !matches!(value_ref(object), Ok(value) if is_object_value(value)) {
        return record_status(env, NAPI_OBJECT_EXPECTED);
    }
    let own = own_property_value(env, object, &key).is_some()
        || accessor_for_owner(env, object as usize, &key).is_some();
    if own
        && env
            .as_ref()
            .is_some_and(|env| env.sealed_objects.contains(&(object as usize)))
    {
        if let Some(out) = out.as_mut() {
            *out = false;
        }
        return NAPI_OK;
    }
    if fixed_index_exists(object, &key)
        || (own && property_attributes_for(env, object as usize, &key) & NAPI_CONFIGURABLE == 0)
    {
        if let Some(out) = out.as_mut() {
            *out = false;
        }
        return NAPI_OK;
    }
    if let Ok(env) = env_mut(env) {
        let status = remove_own_property(env, object, &key);
        if status != NAPI_OK {
            return record_status(env, status);
        }
        env.accessors.remove(&(object as usize, key.clone()));
        remove_property_order(env, object as usize, &key);
        env.property_attributes.remove(&(object as usize, key));
    }
    if let Some(out) = out.as_mut() {
        *out = true;
    }
    NAPI_OK
}

#[no_mangle]
pub unsafe extern "C" fn napi_has_named_property(
    env: NapiEnv,
    object: NapiValue,
    name: *const c_char,
    result: *mut bool,
) -> NapiStatus {
    if name.is_null() || result.is_null() || !value_belongs_to_environment(env, object) {
        return record_status(env, NAPI_INVALID_ARG);
    }
    let Ok(name) = text(name) else {
        return record_status(env, NAPI_INVALID_ARG);
    };
    let name = PropertyKey::String(name);
    if !matches!(value_ref(object), Ok(value) if is_object_value(value)) {
        return record_status(env, NAPI_OBJECT_EXPECTED);
    }
    *result = find_property_value(env, object, &name).is_some()
        || find_accessor(env, object, &name).is_some();
    NAPI_OK
}

#[no_mangle]
pub unsafe extern "C" fn napi_define_properties(
    env: NapiEnv,
    object: NapiValue,
    count: usize,
    descriptors: *const NapiPropertyDescriptor,
) -> NapiStatus {
    if !value_belongs_to_environment(env, object) {
        return NAPI_INVALID_ARG;
    }
    if !matches!(value_ref(object), Ok(value) if is_object_value(value)) {
        return record_status(env, NAPI_OBJECT_EXPECTED);
    }
    let status = validate_property_descriptors(env, count, descriptors);
    if status != NAPI_OK {
        return record_status(env, status);
    }
    if count == 0 {
        return NAPI_OK;
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
            return record_status(env, NAPI_INVALID_ARG);
        };
        let Ok(key_name) = property_key(key) else {
            return record_status(env, NAPI_INVALID_ARG);
        };
        let attributes = descriptor.attributes & NAPI_DEFAULT_PROPERTY_ATTRIBUTES;
        if descriptor.getter.is_some() || descriptor.setter.is_some() {
            let Ok(env) = env_mut(env) else {
                return NAPI_INVALID_ARG;
            };
            if env.sealed_objects.contains(&(object as usize)) {
                return record_status(env, NAPI_GENERIC_FAILURE);
            }
            env.accessors.insert(
                (object as usize, key_name.clone()),
                Accessor {
                    getter: descriptor.getter,
                    setter: descriptor.setter,
                    data: descriptor.data,
                },
            );
            record_property_order(env, object as usize, &key_name);
            env.property_attributes
                .insert((object as usize, key_name), attributes);
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
                return record_status(env, status);
            }
            value
        } else {
            descriptor.value
        };
        let status = napi_set_property(env, object, key, value);
        if status != NAPI_OK {
            return record_status(env, status);
        }
        if let Ok(env) = env_mut(env) {
            env.property_attributes
                .insert((object as usize, key_name), attributes);
        }
    }
    NAPI_OK
}

#[no_mangle]
pub unsafe extern "C" fn napi_type_tag_object(
    env: NapiEnv,
    object: NapiValue,
    type_tag: *const NapiTypeTag,
) -> NapiStatus {
    let Some(type_tag) = type_tag.as_ref().copied() else {
        return record_status(env, NAPI_INVALID_ARG);
    };
    if !value_belongs_to_environment(env, object) {
        return NAPI_INVALID_ARG;
    }
    if !matches!(value_ref(object), Ok(value) if is_object_value(value)) {
        return record_status(env, NAPI_OBJECT_EXPECTED);
    }
    if type_tag_for(env, object).is_some() {
        return record_status(env, NAPI_INVALID_ARG);
    }
    let Ok(env) = env_mut(env) else {
        return NAPI_INVALID_ARG;
    };
    env.type_tags.insert(object as usize, type_tag);
    NAPI_OK
}

#[no_mangle]
pub unsafe extern "C" fn napi_check_object_type_tag(
    env: NapiEnv,
    object: NapiValue,
    type_tag: *const NapiTypeTag,
    result: *mut bool,
) -> NapiStatus {
    let (Some(type_tag), Some(result)) = (type_tag.as_ref(), result.as_mut()) else {
        return record_status(env, NAPI_INVALID_ARG);
    };
    if !value_belongs_to_environment(env, object) {
        return NAPI_INVALID_ARG;
    }
    if !matches!(value_ref(object), Ok(value) if is_object_value(value)) {
        return record_status(env, NAPI_OBJECT_EXPECTED);
    }
    if env.is_null() {
        return NAPI_INVALID_ARG;
    }
    *result = type_tag_for(env, object).as_ref() == Some(type_tag);
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
    if !value_belongs_to_environment(env, object) {
        return NAPI_INVALID_ARG;
    }
    let env_ptr = env;
    let Ok(env) = env_mut(env) else {
        return NAPI_INVALID_ARG;
    };
    if !matches!(object.as_ref(), Some(value) if is_object_value(value)) {
        return record_status(env_ptr, NAPI_OBJECT_EXPECTED);
    }
    if env.wraps.contains_key(&(object as usize)) {
        return record_status(env_ptr, NAPI_GENERIC_FAILURE);
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
        *result = alloc_reference(env, object, 0);
    }
    NAPI_OK
}

#[no_mangle]
pub unsafe extern "C" fn napi_unwrap(
    env: NapiEnv,
    object: NapiValue,
    result: *mut *mut c_void,
) -> NapiStatus {
    if result.is_null() || !value_belongs_to_environment(env, object) {
        return record_status(env, NAPI_INVALID_ARG);
    }
    let env_ptr = env;
    let Ok(env) = env_mut(env) else {
        return NAPI_INVALID_ARG;
    };
    if !matches!(object.as_ref(), Some(value) if is_object_value(value)) {
        return record_status(env_ptr, NAPI_OBJECT_EXPECTED);
    }
    let Some(wrap) = env.wraps.get(&(object as usize)) else {
        return record_status(env_ptr, NAPI_INVALID_ARG);
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
    if !value_belongs_to_environment(env, object) {
        return NAPI_INVALID_ARG;
    }
    let env_ptr = env;
    let Ok(env) = env_mut(env) else {
        return NAPI_INVALID_ARG;
    };
    if !matches!(object.as_ref(), Some(value) if is_object_value(value)) {
        return record_status(env_ptr, NAPI_OBJECT_EXPECTED);
    }
    let Some(wrap) = env.wraps.remove(&(object as usize)) else {
        return record_status(env_ptr, NAPI_INVALID_ARG);
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
    if !value_belongs_to_environment(env, object) {
        return NAPI_INVALID_ARG;
    }
    if !matches!(object.as_ref(), Some(value) if is_object_value(value)) {
        return record_status(env, NAPI_OBJECT_EXPECTED);
    }
    if finalize.is_none() {
        return record_status(env, NAPI_INVALID_ARG);
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
        *result = alloc_reference(env, object, 0);
    }
    NAPI_OK
}

#[no_mangle]
pub unsafe extern "C" fn node_api_post_finalizer(
    env: NapiEnv,
    finalize: Option<NapiFinalize>,
    data: *mut c_void,
    hint: *mut c_void,
) -> NapiStatus {
    let env_ptr = env;
    let (Ok(env), Some(finalize)) = (env_mut(env), finalize) else {
        return record_status(env_ptr, NAPI_INVALID_ARG);
    };
    env.posted_finalizers.push(FinalizeRecord {
        data,
        finalize: Some(finalize),
        hint,
    });
    NAPI_OK
}

#[no_mangle]
pub unsafe extern "C" fn napi_set_instance_data(
    env: NapiEnv,
    data: *mut c_void,
    finalize: Option<NapiFinalize>,
    hint: *mut c_void,
) -> NapiStatus {
    let env_ptr = env;
    let Ok(env) = env_mut(env) else {
        return NAPI_INVALID_ARG;
    };
    if env.instance_data.is_some() {
        return record_status(env_ptr, NAPI_GENERIC_FAILURE);
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
        return record_status(env, NAPI_INVALID_ARG);
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
    let env_ptr = env;
    let (Ok(env), Some(hook)) = (env_mut(env), hook) else {
        return record_status(env_ptr, NAPI_INVALID_ARG);
    };
    if env
        .cleanup_hooks
        .iter()
        .any(|record| record.hook as usize == hook as usize && record.data == data)
    {
        return record_status(env_ptr, NAPI_INVALID_ARG);
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
    let env_ptr = env;
    let (Ok(env), Some(hook)) = (env_mut(env), hook) else {
        return record_status(env_ptr, NAPI_INVALID_ARG);
    };
    let Some(index) = env
        .cleanup_hooks
        .iter()
        .position(|record| record.hook as usize == hook as usize && record.data == data)
    else {
        return record_status(env_ptr, NAPI_INVALID_ARG);
    };
    env.cleanup_hooks.remove(index);
    NAPI_OK
}

#[no_mangle]
pub unsafe extern "C" fn napi_add_async_cleanup_hook(
    env: NapiEnv,
    hook: Option<NapiAsyncCleanupHook>,
    data: *mut c_void,
    result: *mut *mut AsyncCleanupHookHandle,
) -> NapiStatus {
    let env_ptr = env;
    let (Ok(env_ref), Some(hook)) = (env_mut(env), hook) else {
        return record_status(env_ptr, NAPI_INVALID_ARG);
    };
    let mut handle = Box::new(AsyncCleanupHookHandle {
        env: env as usize,
        hook,
        data: data as usize,
        state: AtomicU8::new(0),
    });
    let handle_ptr = (&mut *handle) as *mut AsyncCleanupHookHandle;
    async_cleanup_handles()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .push(handle);
    env_ref.async_cleanup_hooks.push(handle_ptr);
    if let Some(result) = result.as_mut() {
        *result = handle_ptr;
    }
    NAPI_OK
}

#[no_mangle]
pub unsafe extern "C" fn napi_remove_async_cleanup_hook(
    handle: *mut AsyncCleanupHookHandle,
) -> NapiStatus {
    if handle.is_null() {
        return NAPI_INVALID_ARG;
    }
    let handles = async_cleanup_handles()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let Some(handle_ref) = handles
        .iter()
        .find(|candidate| std::ptr::eq(candidate.as_ref(), handle))
    else {
        return NAPI_INVALID_ARG;
    };
    match handle_ref.state.swap(2, Ordering::AcqRel) {
        0 => {
            if let Some(env) = (handle_ref.env as NapiEnv).as_mut() {
                env.async_cleanup_hooks
                    .retain(|candidate| *candidate != handle);
            }
        }
        1 => {
            ACTIVE_ASYNC_CLEANUP_HOOKS.fetch_sub(1, Ordering::AcqRel);
        }
        _ => {
            return record_status(handle_ref.env as NapiEnv, NAPI_INVALID_ARG);
        }
    }
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
        return record_status(env, NAPI_INVALID_ARG);
    }
    let descriptor_status = validate_property_descriptors(env, property_count, properties);
    if descriptor_status != NAPI_OK {
        return record_status(env, descriptor_status);
    }
    let status = napi_create_function(env, name, length, constructor, data, result);
    if status != NAPI_OK {
        return record_status(env, status);
    }
    let mut prototype = ptr::null_mut();
    let status = napi_create_object(env, &mut prototype);
    if status != NAPI_OK {
        return record_status(env, status);
    }
    let prototype_name = c"prototype";
    let status = napi_set_named_property(env, *result, prototype_name.as_ptr(), prototype);
    if status != NAPI_OK {
        return record_status(env, status);
    }
    let constructor_name = c"constructor";
    let status = napi_set_named_property(env, prototype, constructor_name.as_ptr(), *result);
    if status != NAPI_OK {
        return record_status(env, status);
    }
    let Ok(env_ref) = env_mut(env) else {
        return NAPI_INVALID_ARG;
    };
    env_ref.property_attributes.insert(
        ((*result) as usize, PropertyKey::String("prototype".into())),
        0,
    );
    env_ref.property_attributes.insert(
        (
            prototype as usize,
            PropertyKey::String("constructor".into()),
        ),
        NAPI_WRITABLE | NAPI_CONFIGURABLE,
    );
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
                return record_status(env, status);
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
        return record_status(env, NAPI_INVALID_ARG);
    }
    let Ok(host_env) = env_mut(env) else {
        return NAPI_INVALID_ARG;
    };
    if !value_belongs_to_environment(env, constructor)
        || (argc != 0
            && std::slice::from_raw_parts(argv, argc)
                .iter()
                .any(|value| !value_belongs_to_environment(env, *value)))
        || host_env.exception.is_some()
    {
        let status = if host_env.exception.is_some() {
            NAPI_PENDING_EXCEPTION
        } else {
            NAPI_INVALID_ARG
        };
        return record_status(env, status);
    }
    let function = match value_ref(constructor) {
        Ok(Value::Function(function)) => function.clone(),
        _ => return record_status(env, NAPI_FUNCTION_EXPECTED),
    };
    let prototype = function
        .properties
        .get(&PropertyKey::String("prototype".into()))
        .copied();
    let instance = host_env.alloc(Value::Object(HashMap::new()));
    host_env
        .instances
        .insert(instance as usize, constructor as usize);
    if let Some(prototype) = prototype {
        host_env
            .prototypes
            .insert(instance as usize, prototype as usize);
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
        return record_status(env, NAPI_PENDING_EXCEPTION);
    }
    if !returned.is_null() && !value_belongs_to_environment(env, returned) {
        return NAPI_INVALID_ARG;
    }
    *result = if matches!(returned.as_ref(), Some(value) if is_object_value(value)) {
        returned
    } else {
        instance
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
    if env.is_null() || result.is_null() {
        return record_status(env, NAPI_INVALID_ARG);
    }
    if !value_belongs_to_environment(env, object) || !value_belongs_to_environment(env, constructor)
    {
        return NAPI_INVALID_ARG;
    }
    if !matches!(value_ref(constructor), Ok(Value::Function(_))) {
        return record_status(env, NAPI_FUNCTION_EXPECTED);
    }
    if !matches!(value_ref(object), Ok(value) if is_object_value(value)) {
        *result = false;
        return NAPI_OK;
    }
    let prototype_key = PropertyKey::String("prototype".into());
    let Some(expected) = find_property_value(env, constructor, &prototype_key) else {
        return record_status(env, NAPI_GENERIC_FAILURE);
    };
    if !matches!(value_ref(expected), Ok(value) if is_object_value(value)) {
        return record_status(env, NAPI_GENERIC_FAILURE);
    }
    let mut current = prototype_for_owner(env, object as usize);
    let mut visited = HashSet::new();
    *result = false;
    while let Some(prototype) = current.filter(|prototype| visited.insert(*prototype)) {
        if prototype == expected as usize {
            *result = true;
            break;
        }
        current = prototype_for_owner(env, prototype);
    }
    NAPI_OK
}

#[no_mangle]
pub unsafe extern "C" fn napi_get_prototype(
    env: NapiEnv,
    object: NapiValue,
    result: *mut NapiValue,
) -> NapiStatus {
    if result.is_null() || !value_belongs_to_environment(env, object) {
        return record_status(env, NAPI_INVALID_ARG);
    }
    if !matches!(value_ref(object), Ok(value) if is_object_value(value)) {
        return record_status(env, NAPI_OBJECT_EXPECTED);
    }
    let env_ptr = env;
    let Ok(env) = env_mut(env) else {
        return record_status(env, NAPI_INVALID_ARG);
    };
    let prototype =
        prototype_for_owner(env_ptr, object as usize).map(|prototype| prototype as NapiValue);
    match prototype {
        Some(prototype) => write_value(result, prototype),
        None => {
            let undefined = env.alloc(Value::Undefined);
            write_value(result, undefined)
        }
    }
}

#[no_mangle]
pub unsafe extern "C" fn node_api_set_prototype(
    env: NapiEnv,
    object: NapiValue,
    prototype: NapiValue,
) -> NapiStatus {
    if !value_belongs_to_environment(env, object) || !value_belongs_to_environment(env, prototype) {
        return NAPI_INVALID_ARG;
    }
    if !matches!(value_ref(object), Ok(value) if is_object_value(value)) {
        return record_status(env, NAPI_OBJECT_EXPECTED);
    }
    if !matches!(value_ref(prototype), Ok(value) if is_object_value(value) || matches!(value, Value::Null))
    {
        return record_status(env, NAPI_OBJECT_EXPECTED);
    }

    let object_id = object as usize;
    let prototype_id = prototype as usize;
    let Ok(env) = env_mut(env) else {
        return NAPI_INVALID_ARG;
    };
    if env.prototypes.get(&object_id).copied() == Some(prototype_id) {
        return NAPI_OK;
    }
    if env.sealed_objects.contains(&object_id) {
        return record_status(env, NAPI_GENERIC_FAILURE);
    }

    if !matches!(value_ref(prototype), Ok(Value::Null)) {
        let mut ancestor = Some(prototype_id);
        let mut visited = HashSet::new();
        while let Some(current) = ancestor {
            if current == object_id {
                return record_status(env, NAPI_GENERIC_FAILURE);
            }
            if !visited.insert(current) {
                break;
            }
            ancestor = env.prototypes.get(&current).copied();
        }
    }
    env.prototypes.insert(object_id, prototype_id);
    NAPI_OK
}

#[no_mangle]
pub unsafe extern "C" fn napi_strict_equals(
    env: NapiEnv,
    left: NapiValue,
    right: NapiValue,
    result: *mut bool,
) -> NapiStatus {
    if result.is_null() {
        return record_status(env, NAPI_INVALID_ARG);
    }
    if !value_belongs_to_environment(env, left) || !value_belongs_to_environment(env, right) {
        return record_status(env, NAPI_INVALID_ARG);
    }
    *result = match (value_ref(left), value_ref(right)) {
        (Ok(Value::Undefined), Ok(Value::Undefined)) | (Ok(Value::Null), Ok(Value::Null)) => true,
        (Ok(Value::Bool(left)), Ok(Value::Bool(right))) => left == right,
        (Ok(Value::Number(left)), Ok(Value::Number(right))) => left == right,
        (Ok(Value::String(left)), Ok(Value::String(right))) => left == right,
        (Ok(Value::Symbol { id: left, .. }), Ok(Value::Symbol { id: right, .. })) => left == right,
        (
            Ok(Value::BigInt {
                negative: left_negative,
                words: left_words,
            }),
            Ok(Value::BigInt {
                negative: right_negative,
                words: right_words,
            }),
        ) => left_negative == right_negative && left_words == right_words,
        (Ok(_), Ok(_)) => left == right,
        _ => false,
    };
    NAPI_OK
}

#[no_mangle]
pub unsafe extern "C" fn napi_get_value_double(
    env: NapiEnv,
    value: NapiValue,
    out: *mut f64,
) -> NapiStatus {
    if out.is_null() || !value_belongs_to_environment(env, value) {
        return record_status(env, NAPI_INVALID_ARG);
    }
    let status = match value_ref(value) {
        Ok(Value::Number(number)) => {
            *out = *number;
            NAPI_OK
        }
        _ => NAPI_NUMBER_EXPECTED,
    };
    record_status(env, status)
}
#[no_mangle]
pub unsafe extern "C" fn napi_get_value_int32(
    env: NapiEnv,
    value: NapiValue,
    out: *mut i32,
) -> NapiStatus {
    if out.is_null() || !value_belongs_to_environment(env, value) {
        return record_status(env, NAPI_INVALID_ARG);
    }
    let status = match value_ref(value) {
        Ok(Value::Number(number)) => {
            *out = *number as i32;
            NAPI_OK
        }
        _ => NAPI_NUMBER_EXPECTED,
    };
    record_status(env, status)
}

#[no_mangle]
pub unsafe extern "C" fn napi_get_value_uint32(
    env: NapiEnv,
    value: NapiValue,
    out: *mut u32,
) -> NapiStatus {
    if out.is_null() || !value_belongs_to_environment(env, value) {
        return record_status(env, NAPI_INVALID_ARG);
    }
    let status = match value_ref(value) {
        Ok(Value::Number(number)) => {
            *out = *number as u32;
            NAPI_OK
        }
        _ => NAPI_NUMBER_EXPECTED,
    };
    record_status(env, status)
}

#[no_mangle]
pub unsafe extern "C" fn napi_get_value_int64(
    env: NapiEnv,
    value: NapiValue,
    out: *mut i64,
) -> NapiStatus {
    if out.is_null() || !value_belongs_to_environment(env, value) {
        return record_status(env, NAPI_INVALID_ARG);
    }
    let status = match value_ref(value) {
        Ok(Value::Number(number)) => {
            *out = *number as i64;
            NAPI_OK
        }
        _ => NAPI_NUMBER_EXPECTED,
    };
    record_status(env, status)
}

#[no_mangle]
pub unsafe extern "C" fn napi_coerce_to_bool(
    env: NapiEnv,
    value: NapiValue,
    out: *mut NapiValue,
) -> NapiStatus {
    if out.is_null() || !value_belongs_to_environment(env, value) {
        return record_status(env, NAPI_INVALID_ARG);
    }
    let boolean = match value_ref(value) {
        Ok(Value::Undefined | Value::Null) => false,
        Ok(Value::Bool(value)) => *value,
        Ok(Value::Number(value)) => *value != 0.0 && !value.is_nan(),
        Ok(Value::String(value)) => !value.is_empty(),
        Ok(_) => true,
        Err(status) => return record_status(env, status),
    };
    let status = napi_get_boolean(env, boolean, out);
    record_status(env, status)
}

fn javascript_number_from_string(value: &str) -> f64 {
    let value = value.trim();
    if value.is_empty() {
        return 0.0;
    }
    match value {
        "Infinity" | "+Infinity" => return f64::INFINITY,
        "-Infinity" => return f64::NEG_INFINITY,
        _ => {}
    }
    let radix = [
        ("0x", 16),
        ("0X", 16),
        ("0b", 2),
        ("0B", 2),
        ("0o", 8),
        ("0O", 8),
    ]
    .into_iter()
    .find_map(|(prefix, radix)| value.strip_prefix(prefix).map(|digits| (digits, radix)));
    if let Some((digits, radix)) = radix {
        return u128::from_str_radix(digits, radix)
            .map(|number| number as f64)
            .unwrap_or(f64::NAN);
    }
    value.parse().unwrap_or(f64::NAN)
}

fn bigint_to_decimal(negative: bool, words: &[u64]) -> String {
    const BASE: u128 = 1_000_000_000;
    let mut decimal = vec![0_u32];
    for word in words.iter().rev() {
        let mut carry = *word as u128;
        for limb in &mut decimal {
            let value = (*limb as u128) * (1_u128 << 64) + carry;
            *limb = (value % BASE) as u32;
            carry = value / BASE;
        }
        while carry != 0 {
            decimal.push((carry % BASE) as u32);
            carry /= BASE;
        }
    }
    while decimal.len() > 1 && decimal.last() == Some(&0) {
        decimal.pop();
    }
    let mut result = decimal.last().unwrap_or(&0).to_string();
    for limb in decimal.iter().rev().skip(1) {
        result.push_str(&format!("{limb:09}"));
    }
    if negative && result != "0" {
        result.insert(0, '-');
    }
    result
}

unsafe fn javascript_string(
    env: NapiEnv,
    value: NapiValue,
    arrays: &mut HashSet<usize>,
) -> Result<String, ()> {
    Ok(match value_ref(value).map_err(|_| ())? {
        Value::Undefined => "undefined".into(),
        Value::Null => "null".into(),
        Value::Bool(value) => value.to_string(),
        Value::Number(value) if *value == 0.0 => "0".into(),
        Value::Number(value) if value.is_infinite() => {
            if value.is_sign_negative() {
                "-Infinity".into()
            } else {
                "Infinity".into()
            }
        }
        Value::Number(value) => value.to_string(),
        Value::BigInt { negative, words } => bigint_to_decimal(*negative, words),
        Value::String(value) => value.clone(),
        Value::Error(message) => {
            let name_key = PropertyKey::String("name".into());
            let name = find_property_value(env, value, &name_key)
                .and_then(|name| match value_ref(name) {
                    Ok(Value::String(name)) => Some(name.clone()),
                    _ => None,
                })
                .unwrap_or_else(|| "Error".into());
            match (name.is_empty(), message.is_empty()) {
                (true, _) => message.clone(),
                (_, true) => name,
                _ => format!("{name}: {message}"),
            }
        }
        Value::Symbol { .. } => return Err(()),
        Value::Array(values) => {
            if !arrays.insert(value as usize) {
                return Ok(String::new());
            }
            let result = values
                .iter()
                .map(|item| {
                    match item.and_then(|item| value_ref(item).ok().map(|value| (item, value))) {
                        None | Some((_, Value::Undefined | Value::Null)) => Ok(String::new()),
                        Some((item, _)) => javascript_string(env, item, arrays),
                    }
                })
                .collect::<Result<Vec<_>, _>>()?
                .join(",");
            arrays.remove(&(value as usize));
            result
        }
        _ => "[object Object]".into(),
    })
}

unsafe fn coercion_type_error(env: NapiEnv, message: &str) -> NapiStatus {
    let Ok(env) = env_mut(env) else {
        return NAPI_INVALID_ARG;
    };
    let error = env.alloc(Value::Error(message.into()));
    env.exception = Some(error);
    record_status(env, NAPI_PENDING_EXCEPTION)
}

#[no_mangle]
pub unsafe extern "C" fn napi_coerce_to_number(
    env: NapiEnv,
    value: NapiValue,
    out: *mut NapiValue,
) -> NapiStatus {
    if out.is_null() || !value_belongs_to_environment(env, value) {
        return record_status(env, NAPI_INVALID_ARG);
    }
    let number = match value_ref(value) {
        Ok(Value::Undefined) => f64::NAN,
        Ok(Value::Null) => 0.0,
        Ok(Value::Bool(value)) => u8::from(*value) as f64,
        Ok(Value::Number(value)) => *value,
        Ok(Value::Date(value)) => *value,
        Ok(Value::String(value)) => javascript_number_from_string(value),
        Ok(Value::BigInt { .. } | Value::Symbol { .. }) => {
            return coercion_type_error(env, "value cannot be converted to a number");
        }
        Ok(Value::Array(_)) => match javascript_string(env, value, &mut HashSet::new()) {
            Ok(value) => javascript_number_from_string(&value),
            Err(()) => return coercion_type_error(env, "value cannot be converted to a number"),
        },
        Ok(_) => f64::NAN,
        Err(status) => return record_status(env, status),
    };
    let status = napi_create_double(env, number, out);
    record_status(env, status)
}

#[no_mangle]
pub unsafe extern "C" fn napi_coerce_to_string(
    env: NapiEnv,
    value: NapiValue,
    out: *mut NapiValue,
) -> NapiStatus {
    if out.is_null() || !value_belongs_to_environment(env, value) {
        return record_status(env, NAPI_INVALID_ARG);
    }
    let string = match javascript_string(env, value, &mut HashSet::new()) {
        Ok(string) => string,
        Err(()) => return coercion_type_error(env, "a Symbol cannot be converted to a string"),
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
    if out.is_null() || !value_belongs_to_environment(env, value) {
        return record_status(env, NAPI_INVALID_ARG);
    }
    if matches!(value_ref(value), Ok(value) if is_object_value(value)) {
        return write_value(out, value);
    }
    if matches!(value_ref(value), Ok(Value::Undefined | Value::Null)) {
        return record_status(env, NAPI_INVALID_ARG);
    }
    let Ok(env) = env_mut(env) else {
        return NAPI_INVALID_ARG;
    };
    let object = env.alloc(Value::Object(HashMap::from([("value".into(), value)])));
    write_value(out, object)
}
#[no_mangle]
pub unsafe extern "C" fn napi_get_value_bool(
    env: NapiEnv,
    value: NapiValue,
    out: *mut bool,
) -> NapiStatus {
    if out.is_null() || !value_belongs_to_environment(env, value) {
        return record_status(env, NAPI_INVALID_ARG);
    }
    let status = match value_ref(value) {
        Ok(Value::Bool(boolean)) => {
            *out = *boolean;
            NAPI_OK
        }
        _ => NAPI_BOOLEAN_EXPECTED,
    };
    record_status(env, status)
}

#[no_mangle]
pub unsafe extern "C" fn napi_get_value_string_utf8(
    env: NapiEnv,
    value: NapiValue,
    buffer: *mut c_char,
    size: usize,
    written: *mut usize,
) -> NapiStatus {
    if !value_belongs_to_environment(env, value) {
        return record_status(env, NAPI_INVALID_ARG);
    }
    let string = match value_ref(value) {
        Ok(Value::String(string)) => string,
        _ => return record_status(env, NAPI_STRING_EXPECTED),
    };
    let mut count = string.len();
    if !buffer.is_null() && size > 0 {
        count = string.len().min(size - 1);
        while !string.is_char_boundary(count) {
            count -= 1;
        }
        ptr::copy_nonoverlapping(string.as_ptr(), buffer.cast(), count);
        *buffer.add(count) = 0;
    }
    if !written.is_null() {
        *written = count;
    }
    NAPI_OK
}

#[no_mangle]
pub unsafe extern "C" fn napi_get_value_string_latin1(
    env: NapiEnv,
    value: NapiValue,
    buffer: *mut c_char,
    size: usize,
    written: *mut usize,
) -> NapiStatus {
    if !value_belongs_to_environment(env, value) {
        return record_status(env, NAPI_INVALID_ARG);
    }
    let string = match value_ref(value) {
        Ok(Value::String(string)) => string,
        _ => return record_status(env, NAPI_STRING_EXPECTED),
    };
    let encoded: Vec<u8> = string.encode_utf16().map(|unit| unit as u8).collect();
    let count = if buffer.is_null() || size == 0 {
        encoded.len()
    } else {
        let count = encoded.len().min(size - 1);
        ptr::copy_nonoverlapping(encoded.as_ptr(), buffer.cast(), count);
        *buffer.add(count) = 0;
        count
    };
    if !written.is_null() {
        *written = count;
    }
    NAPI_OK
}

#[no_mangle]
pub unsafe extern "C" fn napi_get_value_string_utf16(
    env: NapiEnv,
    value: NapiValue,
    buffer: *mut u16,
    size: usize,
    written: *mut usize,
) -> NapiStatus {
    if !value_belongs_to_environment(env, value) {
        return record_status(env, NAPI_INVALID_ARG);
    }
    let string = match value_ref(value) {
        Ok(Value::String(string)) => string,
        _ => return record_status(env, NAPI_STRING_EXPECTED),
    };
    let encoded: Vec<u16> = string.encode_utf16().collect();
    let count = if buffer.is_null() || size == 0 {
        encoded.len()
    } else {
        let count = encoded.len().min(size - 1);
        ptr::copy_nonoverlapping(encoded.as_ptr(), buffer, count);
        *buffer.add(count) = 0;
        count
    };
    if !written.is_null() {
        *written = count;
    }
    NAPI_OK
}

#[no_mangle]
pub unsafe extern "C" fn napi_throw_error(
    env: NapiEnv,
    code: *const c_char,
    message: *const c_char,
) -> NapiStatus {
    let Ok(message) = text(message) else {
        return record_status(env, NAPI_INVALID_ARG);
    };
    let Ok(env) = env_mut(env) else {
        return NAPI_INVALID_ARG;
    };
    let code = (!code.is_null()).then(|| env.alloc(Value::String(text(code).unwrap_or_default())));
    let error = alloc_error(env, "Error", message, code);
    env.exception = Some(error);
    NAPI_OK
}
#[no_mangle]
pub unsafe extern "C" fn napi_throw_type_error(
    env: NapiEnv,
    code: *const c_char,
    message: *const c_char,
) -> NapiStatus {
    throw_error_kind(env, "TypeError", code, message)
}
#[no_mangle]
pub unsafe extern "C" fn napi_throw_range_error(
    env: NapiEnv,
    code: *const c_char,
    message: *const c_char,
) -> NapiStatus {
    throw_error_kind(env, "RangeError", code, message)
}

#[no_mangle]
pub unsafe extern "C" fn node_api_throw_syntax_error(
    env: NapiEnv,
    code: *const c_char,
    message: *const c_char,
) -> NapiStatus {
    throw_error_kind(env, "SyntaxError", code, message)
}

unsafe fn throw_error_kind(
    env: NapiEnv,
    name: &str,
    code: *const c_char,
    message: *const c_char,
) -> NapiStatus {
    let Ok(message) = text(message) else {
        return record_status(env, NAPI_INVALID_ARG);
    };
    let Ok(env) = env_mut(env) else {
        return NAPI_INVALID_ARG;
    };
    let code = (!code.is_null()).then(|| env.alloc(Value::String(text(code).unwrap_or_default())));
    let error = alloc_error(env, name, message, code);
    env.exception = Some(error);
    NAPI_OK
}
#[no_mangle]
pub unsafe extern "C" fn napi_is_exception_pending(env: NapiEnv, out: *mut bool) -> NapiStatus {
    if out.is_null() {
        return record_status(env, NAPI_INVALID_ARG);
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
    if out.is_null() {
        return record_status(env, NAPI_INVALID_ARG);
    }
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
pub unsafe extern "C" fn napi_get_version(env: NapiEnv, out: *mut u32) -> NapiStatus {
    if env.is_null() || out.is_null() {
        record_status(env, NAPI_INVALID_ARG)
    } else {
        *out = 10;
        NAPI_OK
    }
}

#[no_mangle]
pub unsafe extern "C" fn napi_get_global(env: NapiEnv, out: *mut NapiValue) -> NapiStatus {
    let Ok(env) = env_mut(env) else {
        return NAPI_INVALID_ARG;
    };
    if out.is_null() {
        return record_status(env, NAPI_INVALID_ARG);
    }
    if env.global.is_null() {
        env.global = env.alloc(Value::Object(HashMap::new()));
    }
    *out = env.global;
    NAPI_OK
}

#[no_mangle]
pub unsafe extern "C" fn napi_run_script(
    env: NapiEnv,
    script: NapiValue,
    result: *mut NapiValue,
) -> NapiStatus {
    if result.is_null() || !value_belongs_to_environment(env, script) {
        return record_status(env, NAPI_INVALID_ARG);
    }
    let source = match value_ref(script) {
        Ok(Value::String(source)) => source.clone(),
        _ => return record_status(env, NAPI_STRING_EXPECTED),
    };
    let evaluated = thaw_quickjs::eval_json(&source);
    let env_ptr = env;
    let Ok(env) = env_mut(env) else {
        return NAPI_INVALID_ARG;
    };
    let status = match evaluated {
        Ok(Some(json)) => match serde_json::from_str::<JsonValue>(&json) {
            Ok(json) => write_value(result, value_from_json(env, &json)),
            Err(error) => {
                env.exception = Some(env.alloc(Value::Error(format!(
                    "failed to decode script result: {error}"
                ))));
                NAPI_PENDING_EXCEPTION
            }
        },
        Ok(None) => {
            let undefined = env.alloc(Value::Undefined);
            write_value(result, undefined)
        }
        Err(error) => {
            env.exception = Some(env.alloc(Value::Error(error)));
            NAPI_PENDING_EXCEPTION
        }
    };
    record_status(env_ptr, status)
}

#[no_mangle]
pub unsafe extern "C" fn napi_get_property_names(
    env: NapiEnv,
    object: NapiValue,
    out: *mut NapiValue,
) -> NapiStatus {
    napi_get_all_property_names(
        env,
        object,
        NAPI_KEY_INCLUDE_PROTOTYPES,
        NAPI_ENUMERABLE | NAPI_KEY_SKIP_SYMBOLS,
        NAPI_KEY_NUMBERS_TO_STRINGS,
        out,
    )
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
enum EnumeratedPropertyKey {
    Number(usize),
    Property(PropertyKey),
}

#[no_mangle]
pub unsafe extern "C" fn napi_get_all_property_names(
    env: NapiEnv,
    object: NapiValue,
    key_mode: i32,
    key_filter: u32,
    key_conversion: i32,
    out: *mut NapiValue,
) -> NapiStatus {
    if out.is_null()
        || !value_belongs_to_environment(env, object)
        || !matches!(key_mode, NAPI_KEY_INCLUDE_PROTOTYPES | NAPI_KEY_OWN_ONLY)
        || !matches!(
            key_conversion,
            NAPI_KEY_KEEP_NUMBERS | NAPI_KEY_NUMBERS_TO_STRINGS
        )
    {
        return record_status(env, NAPI_INVALID_ARG);
    }
    let env_ptr = env;
    let Ok(env) = env_mut(env) else {
        return NAPI_INVALID_ARG;
    };
    if !matches!(value_ref(object), Ok(value) if is_object_value(value)) {
        return record_status(env_ptr, NAPI_OBJECT_EXPECTED);
    }
    let mut keys = Vec::new();
    let mut current = Some(object as usize);
    let mut visited = HashSet::new();
    while let Some(owner) = current.filter(|owner| visited.insert(*owner)) {
        let value = owner as NapiValue;
        let mut property_keys = match value_ref(value) {
            Ok(Value::Object(properties)) => properties.keys().cloned().collect::<Vec<_>>(),
            Ok(Value::Function(function)) => {
                function.properties.keys().cloned().collect::<Vec<_>>()
            }
            Ok(Value::Array(values)) => {
                keys.extend(
                    values
                        .iter()
                        .enumerate()
                        .filter(|(_, value)| value.is_some())
                        .map(|(index, _)| (EnumeratedPropertyKey::Number(index), owner)),
                );
                vec![PropertyKey::String("length".into())]
            }
            Ok(Value::Buffer(bytes)) => {
                keys.extend(
                    (0..bytes.len()).map(|index| (EnumeratedPropertyKey::Number(index), owner)),
                );
                Vec::new()
            }
            Ok(Value::ExternalBuffer { data, length }) => {
                if !data.is_null() {
                    keys.extend(
                        (0..*length).map(|index| (EnumeratedPropertyKey::Number(index), owner)),
                    );
                }
                Vec::new()
            }
            Ok(Value::BufferView {
                array_buffer,
                length,
                ..
            }) => {
                if arraybuffer_parts(*array_buffer).is_ok_and(|(_, _, detached)| !detached) {
                    keys.extend(
                        (0..*length).map(|index| (EnumeratedPropertyKey::Number(index), owner)),
                    );
                }
                Vec::new()
            }
            Ok(Value::TypedArray {
                array_buffer,
                length,
                ..
            }) => {
                if arraybuffer_parts(*array_buffer).is_ok_and(|(_, _, detached)| !detached) {
                    keys.extend(
                        (0..*length).map(|index| (EnumeratedPropertyKey::Number(index), owner)),
                    );
                }
                Vec::new()
            }
            _ => Vec::new(),
        };
        property_keys.extend(host_property_keys_for_owner(env_ptr, owner));
        property_keys.extend(accessors_for_owner(env_ptr, owner));
        property_keys.sort_by(|left, right| {
            let category = |key: &PropertyKey| match (property_array_index(key), key) {
                (Some(index), _) => (0, index, usize::MAX),
                (None, PropertyKey::String(_)) => {
                    (1, 0, property_order_for_owner(env_ptr, owner, key))
                }
                (None, PropertyKey::Symbol(_)) => {
                    (2, 0, property_order_for_owner(env_ptr, owner, key))
                }
            };
            category(left)
                .cmp(&category(right))
                .then_with(|| left.cmp(right))
        });
        property_keys.dedup();
        keys.extend(property_keys.into_iter().map(|key| {
            let key = property_array_index(&key)
                .map(EnumeratedPropertyKey::Number)
                .unwrap_or(EnumeratedPropertyKey::Property(key));
            (key, owner)
        }));
        current = if key_mode == NAPI_KEY_INCLUDE_PROTOTYPES {
            prototype_for_owner(env_ptr, owner)
        } else {
            None
        };
    }
    let attribute_filter = key_filter & NAPI_DEFAULT_PROPERTY_ATTRIBUTES;
    keys.retain(|(key, owner)| match key {
        EnumeratedPropertyKey::Number(index) => {
            let key = PropertyKey::String(index.to_string());
            key_filter & NAPI_KEY_SKIP_STRINGS == 0
                && (attribute_filter == NAPI_KEY_ALL_PROPERTIES
                    || property_attributes_for(env_ptr, *owner, &key) & attribute_filter
                        == attribute_filter)
        }
        EnumeratedPropertyKey::Property(PropertyKey::String(_)) => {
            key_filter & NAPI_KEY_SKIP_STRINGS == 0
                && (attribute_filter == NAPI_KEY_ALL_PROPERTIES
                    || property_attributes_for(
                        env_ptr,
                        *owner,
                        match key {
                            EnumeratedPropertyKey::Property(key) => key,
                            _ => unreachable!(),
                        },
                    ) & attribute_filter
                        == attribute_filter)
        }
        EnumeratedPropertyKey::Property(PropertyKey::Symbol(_)) => {
            key_filter & NAPI_KEY_SKIP_SYMBOLS == 0
                && (attribute_filter == NAPI_KEY_ALL_PROPERTIES
                    || property_attributes_for(
                        env_ptr,
                        *owner,
                        match key {
                            EnumeratedPropertyKey::Property(key) => key,
                            _ => unreachable!(),
                        },
                    ) & attribute_filter
                        == attribute_filter)
        }
    });
    let mut seen = HashSet::new();
    keys.retain(|(key, _)| seen.insert(key.clone()));
    let mut values = Vec::with_capacity(keys.len());
    for (key, _) in keys {
        let value = match key {
            EnumeratedPropertyKey::Number(index) if key_conversion == NAPI_KEY_KEEP_NUMBERS => {
                env.alloc(Value::Number(index as f64))
            }
            EnumeratedPropertyKey::Number(index) => env.alloc(Value::String(index.to_string())),
            EnumeratedPropertyKey::Property(PropertyKey::String(name)) => {
                env.alloc(Value::String(name))
            }
            EnumeratedPropertyKey::Property(PropertyKey::Symbol(id)) => {
                let Some(value) = symbol_for(env_ptr, id) else {
                    return NAPI_GENERIC_FAILURE;
                };
                value
            }
        };
        values.push(value);
    }
    let result = env.alloc(Value::Array(values.into_iter().map(Some).collect()));
    write_value(out, result)
}

#[no_mangle]
pub unsafe extern "C" fn napi_object_seal(env: NapiEnv, object: NapiValue) -> NapiStatus {
    if !value_belongs_to_environment(env, object) {
        return NAPI_INVALID_ARG;
    }
    if !matches!(value_ref(object), Ok(value) if is_object_value(value)) {
        return record_status(env, NAPI_OBJECT_EXPECTED);
    }
    let Ok(env) = env_mut(env) else {
        return NAPI_INVALID_ARG;
    };
    let mut names = match value_ref(object) {
        Ok(Value::Object(properties)) => properties.keys().cloned().collect::<Vec<_>>(),
        Ok(Value::Function(function)) => function.properties.keys().cloned().collect(),
        Ok(Value::Array(values)) => values
            .iter()
            .enumerate()
            .filter(|(_, value)| value.is_some())
            .map(|(index, _)| PropertyKey::String(index.to_string()))
            .collect(),
        _ => Vec::new(),
    };
    names.extend(
        env.host_properties
            .get(&(object as usize))
            .into_iter()
            .flat_map(|properties| properties.keys().cloned()),
    );
    names.extend(
        env.accessors
            .keys()
            .filter(|(owner, _)| *owner == object as usize)
            .map(|(_, name)| name.clone()),
    );
    names.sort();
    names.dedup();
    for name in names {
        *env.property_attributes
            .entry((object as usize, name))
            .or_insert(NAPI_DEFAULT_PROPERTY_ATTRIBUTES) &= !NAPI_CONFIGURABLE;
    }
    env.sealed_objects.insert(object as usize);
    NAPI_OK
}

#[no_mangle]
pub unsafe extern "C" fn napi_object_freeze(env: NapiEnv, object: NapiValue) -> NapiStatus {
    if !value_belongs_to_environment(env, object) {
        return NAPI_INVALID_ARG;
    }
    if !matches!(value_ref(object), Ok(value) if is_object_value(value)) {
        return record_status(env, NAPI_OBJECT_EXPECTED);
    }
    let status = napi_object_seal(env, object);
    if status != NAPI_OK {
        return record_status(env, status);
    }
    let Ok(env) = env_mut(env) else {
        return NAPI_INVALID_ARG;
    };
    for ((owner, _), attributes) in &mut env.property_attributes {
        if *owner == object as usize {
            *attributes &= !NAPI_WRITABLE;
        }
    }
    env.frozen_objects.insert(object as usize);
    NAPI_OK
}

#[no_mangle]
pub unsafe extern "C" fn napi_get_uv_event_loop(env: NapiEnv, out: *mut *mut c_void) -> NapiStatus {
    if env.is_null() || out.is_null() {
        return record_status(env, NAPI_INVALID_ARG);
    }
    #[cfg(target_os = "linux")]
    {
        let symbol = libc::dlsym(libc::RTLD_DEFAULT, c"uv_default_loop".as_ptr());
        if symbol.is_null() {
            return record_status(env, NAPI_GENERIC_FAILURE);
        }
        let default_loop =
            std::mem::transmute::<*mut c_void, unsafe extern "C" fn() -> *mut c_void>(symbol);
        *out = default_loop();
        if (*out).is_null() {
            return record_status(env, NAPI_GENERIC_FAILURE);
        }
        NAPI_OK
    }
    #[cfg(not(target_os = "linux"))]
    {
        *out = ptr::null_mut();
        NAPI_GENERIC_FAILURE
    }
}

/// Thaw host extension for addons or generated native shims which own a
/// non-default libuv loop. Registered loops are driven on the generated
/// program's main thread alongside Node-API async completions.
#[no_mangle]
pub unsafe extern "C" fn thaw_napi_register_uv_loop(event_loop: *mut c_void) -> NapiStatus {
    if event_loop.is_null() {
        return NAPI_INVALID_ARG;
    }
    let mut loops = registered_uv_loops()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let address = event_loop as usize;
    if !loops.contains(&address) {
        loops.push(address);
    }
    NAPI_OK
}

/// Stops host polling for a private libuv loop before its owner closes and
/// frees the `uv_loop_t` storage.
#[no_mangle]
pub unsafe extern "C" fn thaw_napi_unregister_uv_loop(event_loop: *mut c_void) -> NapiStatus {
    if event_loop.is_null() {
        return NAPI_INVALID_ARG;
    }
    let mut loops = registered_uv_loops()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let address = event_loop as usize;
    let Some(index) = loops.iter().position(|registered| *registered == address) else {
        return NAPI_INVALID_ARG;
    };
    loops.swap_remove(index);
    NAPI_OK
}

#[no_mangle]
pub unsafe extern "C" fn napi_typeof(env: NapiEnv, value: NapiValue, out: *mut i32) -> NapiStatus {
    if out.is_null() || !value_belongs_to_environment(env, value) {
        return record_status(env, NAPI_INVALID_ARG);
    }
    *out = match value_ref(value) {
        Ok(Value::Undefined) => 0,
        Ok(Value::Null) => 1,
        Ok(Value::Bool(_)) => 2,
        Ok(Value::Number(_)) => 3,
        Ok(Value::String(_)) => 4,
        Ok(Value::Symbol { .. }) => 5,
        Ok(Value::Function(_)) => 7,
        Ok(Value::External(_)) => 8,
        Ok(Value::BigInt { .. }) => 9,
        Ok(_) => 6,
        Err(_) => return record_status(env, NAPI_INVALID_ARG),
    };
    NAPI_OK
}

#[no_mangle]
pub unsafe extern "C" fn napi_is_array(
    env: NapiEnv,
    value: NapiValue,
    out: *mut bool,
) -> NapiStatus {
    if out.is_null() || !value_belongs_to_environment(env, value) {
        return record_status(env, NAPI_INVALID_ARG);
    }
    *out = matches!(value_ref(value), Ok(Value::Array(_)));
    NAPI_OK
}

#[no_mangle]
pub unsafe extern "C" fn napi_is_promise(
    env: NapiEnv,
    value: NapiValue,
    result: *mut bool,
) -> NapiStatus {
    if !value_belongs_to_environment(env, value) {
        return NAPI_INVALID_ARG;
    }
    let Some(result) = result.as_mut() else {
        return record_status(env, NAPI_INVALID_ARG);
    };
    *result = matches!(value_ref(value), Ok(Value::Promise(_)));
    NAPI_OK
}

#[no_mangle]
pub unsafe extern "C" fn napi_get_array_length(
    env: NapiEnv,
    value: NapiValue,
    out: *mut u32,
) -> NapiStatus {
    if out.is_null() || !value_belongs_to_environment(env, value) {
        return record_status(env, NAPI_INVALID_ARG);
    }
    let status = match value_ref(value) {
        Ok(Value::Array(values)) => {
            *out = values.len() as u32;
            NAPI_OK
        }
        _ => NAPI_ARRAY_EXPECTED,
    };
    record_status(env, status)
}

#[no_mangle]
pub unsafe extern "C" fn napi_set_element(
    env: NapiEnv,
    object: NapiValue,
    index: u32,
    value: NapiValue,
) -> NapiStatus {
    if !value_belongs_to_environment(env, object) || !value_belongs_to_environment(env, value) {
        return NAPI_INVALID_ARG;
    }
    let status = set_property_key(env, object, PropertyKey::String(index.to_string()), value);
    record_status(env, status)
}

#[no_mangle]
pub unsafe extern "C" fn napi_has_element(
    env: NapiEnv,
    object: NapiValue,
    index: u32,
    result: *mut bool,
) -> NapiStatus {
    if !value_belongs_to_environment(env, object) {
        return NAPI_INVALID_ARG;
    }
    let Some(result) = result.as_mut() else {
        return record_status(env, NAPI_INVALID_ARG);
    };
    let key = PropertyKey::String(index.to_string());
    if !matches!(value_ref(object), Ok(value) if is_object_value(value)) {
        return record_status(env, NAPI_OBJECT_EXPECTED);
    }
    *result = find_property_value(env, object, &key).is_some()
        || find_accessor(env, object, &key).is_some();
    NAPI_OK
}

#[no_mangle]
pub unsafe extern "C" fn napi_delete_element(
    env: NapiEnv,
    object: NapiValue,
    index: u32,
    result: *mut bool,
) -> NapiStatus {
    if !value_belongs_to_environment(env, object) {
        return NAPI_INVALID_ARG;
    }
    let key = PropertyKey::String(index.to_string());
    if !matches!(value_ref(object), Ok(value) if is_object_value(value)) {
        return record_status(env, NAPI_OBJECT_EXPECTED);
    }
    let own = own_property_value(env, object, &key).is_some()
        || accessor_for_owner(env, object as usize, &key).is_some();
    if own
        && env
            .as_ref()
            .is_some_and(|env| env.sealed_objects.contains(&(object as usize)))
    {
        if let Some(result) = result.as_mut() {
            *result = false;
        }
        return NAPI_OK;
    }
    if fixed_index_exists(object, &key)
        || (own && property_attributes_for(env, object as usize, &key) & NAPI_CONFIGURABLE == 0)
    {
        if let Some(result) = result.as_mut() {
            *result = false;
        }
        return NAPI_OK;
    }
    if let Ok(env) = env_mut(env) {
        let status = remove_own_property(env, object, &key);
        if status != NAPI_OK {
            return record_status(env, status);
        }
        env.accessors.remove(&(object as usize, key.clone()));
        remove_property_order(env, object as usize, &key);
        env.property_attributes.remove(&(object as usize, key));
    }
    if let Some(result) = result.as_mut() {
        *result = true;
    }
    NAPI_OK
}

#[no_mangle]
pub unsafe extern "C" fn napi_get_element(
    env: NapiEnv,
    object: NapiValue,
    index: u32,
    out: *mut NapiValue,
) -> NapiStatus {
    if out.is_null() || !value_belongs_to_environment(env, object) {
        return record_status(env, NAPI_INVALID_ARG);
    }
    let key = PropertyKey::String(index.to_string());
    if !matches!(value_ref(object), Ok(value) if is_object_value(value)) {
        return record_status(env, NAPI_OBJECT_EXPECTED);
    }
    let value = find_property_value(env, object, &key);
    if value.is_none() {
        if let Some(accessor) = find_accessor(env, object, &key) {
            if let Some(getter) = accessor.getter {
                let mut info = CallbackInfo {
                    args: Vec::new(),
                    this_arg: object,
                    new_target: ptr::null_mut(),
                    data: accessor.data,
                };
                let value = getter(env, &mut info);
                if env_mut(env)
                    .map(|env| env.exception.is_some())
                    .unwrap_or(false)
                {
                    return record_status(env, NAPI_PENDING_EXCEPTION);
                }
                let status = write_callback_value(env, out, value);
                return record_status(env, status);
            }
        }
    }
    let status = match value {
        Some(value) => write_value(out, value),
        None => napi_get_undefined(env, out),
    };
    record_status(env, status)
}

include!("napi/buffers.rs");

#[no_mangle]
pub unsafe extern "C" fn napi_create_reference(
    env: NapiEnv,
    value: NapiValue,
    initial_count: u32,
    out: *mut *mut Reference,
) -> NapiStatus {
    let Ok(env_ref) = env_mut(env) else {
        return NAPI_INVALID_ARG;
    };
    if value.is_null() || out.is_null() || !value_belongs_to_environment(env, value) {
        return record_status(env, NAPI_INVALID_ARG);
    }
    *out = alloc_reference(env_ref, value, initial_count);
    NAPI_OK
}

#[no_mangle]
pub unsafe extern "C" fn napi_delete_reference(
    env: NapiEnv,
    reference: *mut Reference,
) -> NapiStatus {
    let Ok(reference) = reference_mut(env, reference) else {
        return record_status(env, NAPI_INVALID_ARG);
    };
    reference.deleted = true;
    NAPI_OK
}

#[no_mangle]
pub unsafe extern "C" fn napi_get_reference_value(
    env: NapiEnv,
    reference: *mut Reference,
    out: *mut NapiValue,
) -> NapiStatus {
    let Ok(reference) = reference_mut(env, reference) else {
        return record_status(env, NAPI_INVALID_ARG);
    };
    let _ = reference.count;
    let status = write_value(out, reference.value);
    record_status(env, status)
}

#[no_mangle]
pub unsafe extern "C" fn napi_reference_ref(
    env: NapiEnv,
    reference: *mut Reference,
    result: *mut u32,
) -> NapiStatus {
    let Ok(reference) = reference_mut(env, reference) else {
        return record_status(env, NAPI_INVALID_ARG);
    };
    let Some(count) = reference.count.checked_add(1) else {
        return record_status(env, NAPI_GENERIC_FAILURE);
    };
    reference.count = count;
    if !result.is_null() {
        *result = reference.count;
    }
    NAPI_OK
}

#[no_mangle]
pub unsafe extern "C" fn napi_reference_unref(
    env: NapiEnv,
    reference: *mut Reference,
    result: *mut u32,
) -> NapiStatus {
    let Ok(reference) = reference_mut(env, reference) else {
        return record_status(env, NAPI_INVALID_ARG);
    };
    if reference.count == 0 {
        return record_status(env, NAPI_GENERIC_FAILURE);
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
    let Ok(env_ref) = env_mut(env) else {
        return NAPI_INVALID_ARG;
    };
    if env_ref.exception.is_some() {
        return record_status(env, NAPI_PENDING_EXCEPTION);
    }
    if this_arg.is_null()
        || !value_belongs_to_environment(env, this_arg)
        || !value_belongs_to_environment(env, function)
        || (argc != 0 && argv.is_null())
    {
        return record_status(env, NAPI_INVALID_ARG);
    }
    if argc != 0
        && std::slice::from_raw_parts(argv, argc)
            .iter()
            .any(|value| !value_belongs_to_environment(env, *value))
    {
        return record_status(env, NAPI_INVALID_ARG);
    }
    let function = match value_ref(function) {
        Ok(Value::Function(function)) => function.clone(),
        _ => return record_status(env, NAPI_FUNCTION_EXPECTED),
    };
    let args = if argc == 0 {
        Vec::new()
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
        return record_status(env, NAPI_PENDING_EXCEPTION);
    }
    if !result.is_null() && !value_belongs_to_environment(env, result) {
        return NAPI_INVALID_ARG;
    }
    let status = if out.is_null() {
        NAPI_OK
    } else if result.is_null() {
        napi_get_undefined(env, out)
    } else {
        write_value(out, result)
    };
    record_status(env, status)
}

#[no_mangle]
pub unsafe extern "C" fn napi_make_callback(
    env: NapiEnv,
    async_context: *mut c_void,
    this_arg: NapiValue,
    function: NapiValue,
    argc: usize,
    argv: *const NapiValue,
    result: *mut NapiValue,
) -> NapiStatus {
    if !async_context.is_null() && async_context_mut(env, async_context).is_err() {
        return record_status(env, NAPI_INVALID_ARG);
    }
    let status = napi_call_function(env, this_arg, function, argc, argv, result);
    record_status(env, status)
}

const NAPI_PENDING_EXCEPTION: NapiStatus = 10;

fn alloc_error(env: &mut Env, name: &str, message: String, code: Option<NapiValue>) -> NapiValue {
    let value = env.alloc(Value::Error(message.clone()));
    let message_value = env.alloc(Value::String(message));
    let owner = value as usize;
    env.error_names.insert(owner, name.into());
    let properties = env.host_properties.entry(owner).or_default();
    properties.insert(PropertyKey::String("message".into()), message_value);
    if let Some(code) = code {
        properties.insert(PropertyKey::String("code".into()), code);
    }
    let message_key = PropertyKey::String("message".into());
    record_property_order(env, owner, &message_key);
    env.property_attributes
        .insert((owner, message_key), NAPI_WRITABLE | NAPI_CONFIGURABLE);
    if code.is_some() {
        let code_key = PropertyKey::String("code".into());
        record_property_order(env, owner, &code_key);
        env.property_attributes
            .insert((owner, code_key), NAPI_DEFAULT_PROPERTY_ATTRIBUTES);
    }
    value
}

unsafe fn create_error_kind(
    env: NapiEnv,
    name: &str,
    code: NapiValue,
    message: NapiValue,
    out: *mut NapiValue,
) -> NapiStatus {
    if out.is_null()
        || !value_belongs_to_environment(env, message)
        || (!code.is_null() && !value_belongs_to_environment(env, code))
    {
        return record_status(env, NAPI_INVALID_ARG);
    }
    let message = match value_ref(message) {
        Ok(Value::String(message)) => message.clone(),
        _ => return record_status(env, NAPI_STRING_EXPECTED),
    };
    if !code.is_null() && !matches!(value_ref(code), Ok(Value::String(_))) {
        return record_status(env, NAPI_STRING_EXPECTED);
    }
    let Ok(env) = env_mut(env) else {
        return NAPI_INVALID_ARG;
    };
    write_value(
        out,
        alloc_error(env, name, message, (!code.is_null()).then_some(code)),
    )
}

#[no_mangle]
pub unsafe extern "C" fn napi_create_error(
    env: NapiEnv,
    code: NapiValue,
    message: NapiValue,
    out: *mut NapiValue,
) -> NapiStatus {
    create_error_kind(env, "Error", code, message, out)
}

#[no_mangle]
pub unsafe extern "C" fn napi_create_type_error(
    env: NapiEnv,
    code: NapiValue,
    message: NapiValue,
    out: *mut NapiValue,
) -> NapiStatus {
    create_error_kind(env, "TypeError", code, message, out)
}

#[no_mangle]
pub unsafe extern "C" fn napi_create_range_error(
    env: NapiEnv,
    code: NapiValue,
    message: NapiValue,
    out: *mut NapiValue,
) -> NapiStatus {
    create_error_kind(env, "RangeError", code, message, out)
}

#[no_mangle]
pub unsafe extern "C" fn node_api_create_syntax_error(
    env: NapiEnv,
    code: NapiValue,
    message: NapiValue,
    out: *mut NapiValue,
) -> NapiStatus {
    create_error_kind(env, "SyntaxError", code, message, out)
}

#[no_mangle]
pub unsafe extern "C" fn napi_is_error(
    env: NapiEnv,
    value: NapiValue,
    out: *mut bool,
) -> NapiStatus {
    if out.is_null() || !value_belongs_to_environment(env, value) {
        return record_status(env, NAPI_INVALID_ARG);
    }
    *out = matches!(value_ref(value), Ok(Value::Error(_)));
    NAPI_OK
}

#[no_mangle]
pub unsafe extern "C" fn napi_throw(env: NapiEnv, error: NapiValue) -> NapiStatus {
    if !value_belongs_to_environment(env, error) {
        return NAPI_INVALID_ARG;
    }
    let Ok(env) = env_mut(env) else {
        return NAPI_INVALID_ARG;
    };
    env.exception = Some(error);
    NAPI_OK
}

#[no_mangle]
pub unsafe extern "C" fn napi_get_last_error_info(
    env: NapiEnv,
    out: *mut *const NapiExtendedErrorInfo,
) -> NapiStatus {
    let Some(env) = env.as_mut() else {
        return NAPI_INVALID_ARG;
    };
    if out.is_null() {
        env.last_error_info.error_code = NAPI_INVALID_ARG;
        env.last_error_info.error_message = status_message(NAPI_INVALID_ARG);
        return NAPI_INVALID_ARG;
    }
    *out = &env.last_error_info;
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
    if !value_belongs_to_environment(env, error) {
        return NAPI_INVALID_ARG;
    }
    let Ok(env) = env_mut(env) else {
        return NAPI_INVALID_ARG;
    };
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
    env: NapiEnv,
    out: *mut *const NapiNodeVersion,
) -> NapiStatus {
    if env.is_null() || out.is_null() {
        NAPI_INVALID_ARG
    } else {
        *out = &NODE_VERSION;
        NAPI_OK
    }
}

#[no_mangle]
pub unsafe extern "C" fn node_api_get_module_file_name(
    env: NapiEnv,
    result: *mut *const c_char,
) -> NapiStatus {
    let Some(env_ref) = env.as_ref() else {
        return NAPI_INVALID_ARG;
    };
    let Some(result) = result.as_mut() else {
        return record_status(env, NAPI_INVALID_ARG);
    };
    *result = env_ref.module_file_name.as_ptr();
    NAPI_OK
}

include!("napi/async_runtime.rs");

#[cfg(test)]
mod tests;
