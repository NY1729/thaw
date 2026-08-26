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

include!("napi/values.rs");

include!("napi/properties.rs");

include!("napi/classes.rs");

include!("napi/coercions.rs");

include!("napi/collections.rs");

include!("napi/buffers.rs");

include!("napi/references_errors.rs");

include!("napi/async_runtime.rs");

#[cfg(test)]
mod tests;
