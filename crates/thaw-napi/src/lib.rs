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
        return NAPI_INVALID_ARG;
    }
    for descriptor in std::slice::from_raw_parts(descriptors, count) {
        if descriptor.utf8name.is_null() && !value_belongs_to_environment(env, descriptor.name) {
            return NAPI_INVALID_ARG;
        }
        if descriptor.method.is_none()
            && descriptor.getter.is_none()
            && descriptor.setter.is_none()
            && !descriptor.value.is_null()
            && !value_belongs_to_environment(env, descriptor.value)
        {
            return NAPI_INVALID_ARG;
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

unsafe fn record_status(env: NapiEnv, status: NapiStatus) -> NapiStatus {
    if status != NAPI_OK {
        if let Some(env) = env.as_mut() {
            env.last_error_info.error_code = status;
            env.last_error_info.engine_error_code = 0;
            env.last_error_info.engine_reserved = ptr::null_mut();
            env.last_error_info.error_message = ptr::null();
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
        return NAPI_INVALID_ARG;
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
    if result.is_null()
        || !value_belongs_to_environment(env, left)
        || !value_belongs_to_environment(env, right)
    {
        return NAPI_INVALID_ARG;
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
    let message = text(message).unwrap_or_else(|_| "native addon error".into());
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
    let message = text(message).unwrap_or_else(|_| "native addon error".into());
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
pub unsafe extern "C" fn napi_get_version(env: NapiEnv, out: *mut u32) -> NapiStatus {
    if env.is_null() || out.is_null() {
        NAPI_INVALID_ARG
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
        return NAPI_INVALID_ARG;
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
        return NAPI_INVALID_ARG;
    }
    let source = match value_ref(script) {
        Ok(Value::String(source)) => source.clone(),
        _ => return NAPI_STRING_EXPECTED,
    };
    let evaluated = thaw_quickjs::eval_json(&source);
    let Ok(env) = env_mut(env) else {
        return NAPI_INVALID_ARG;
    };
    match evaluated {
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
    }
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
        return NAPI_OBJECT_EXPECTED;
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
        return NAPI_OBJECT_EXPECTED;
    }
    let status = napi_object_seal(env, object);
    if status != NAPI_OK {
        return status;
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

#[no_mangle]
pub unsafe extern "C" fn napi_create_buffer(
    env: NapiEnv,
    length: usize,
    data: *mut *mut c_void,
    out: *mut NapiValue,
) -> NapiStatus {
    let Ok(env) = env_for_value_output(env, out) else {
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
pub unsafe extern "C" fn napi_create_external_buffer(
    env: NapiEnv,
    length: usize,
    data: *mut c_void,
    finalize: Option<NapiFinalize>,
    hint: *mut c_void,
    out: *mut NapiValue,
) -> NapiStatus {
    if out.is_null() || (length != 0 && data.is_null()) {
        return NAPI_INVALID_ARG;
    }
    let Ok(env) = env_mut(env) else {
        return NAPI_INVALID_ARG;
    };
    let value = env.alloc(Value::ExternalBuffer {
        data: data.cast(),
        length,
    });
    if finalize.is_some() {
        env.finalizers.push(FinalizeRecord {
            data,
            finalize,
            hint,
        });
    }
    write_value(out, value)
}

#[no_mangle]
pub unsafe extern "C" fn node_api_create_buffer_from_arraybuffer(
    env: NapiEnv,
    array_buffer: NapiValue,
    byte_offset: usize,
    byte_length: usize,
    out: *mut NapiValue,
) -> NapiStatus {
    if out.is_null() || !value_belongs_to_environment(env, array_buffer) {
        return record_status(env, NAPI_INVALID_ARG);
    }
    let Ok((_, buffer_length, detached)) = arraybuffer_parts(array_buffer) else {
        return record_status(env, NAPI_ARRAYBUFFER_EXPECTED);
    };
    if detached {
        return record_status(env, NAPI_INVALID_ARG);
    }
    let in_bounds = byte_offset
        .checked_add(byte_length)
        .is_some_and(|end| end <= buffer_length);
    let Ok(env) = env_mut(env) else {
        return NAPI_INVALID_ARG;
    };
    if !in_bounds {
        let error = env.alloc(Value::Error(
            "Buffer range exceeds ArrayBuffer bounds".into(),
        ));
        env.exception = Some(error);
        return record_status(env, NAPI_PENDING_EXCEPTION);
    }
    let value = env.alloc(Value::BufferView {
        array_buffer,
        byte_offset,
        length: byte_length,
    });
    write_value(out, value)
}

#[no_mangle]
pub unsafe extern "C" fn napi_get_buffer_info(
    env: NapiEnv,
    value: NapiValue,
    data: *mut *mut c_void,
    length: *mut usize,
) -> NapiStatus {
    if !value_belongs_to_environment(env, value) {
        return NAPI_INVALID_ARG;
    }
    let status = match value.as_mut() {
        Some(Value::Buffer(bytes)) => {
            if !data.is_null() {
                *data = bytes.as_mut_ptr().cast();
            }
            if !length.is_null() {
                *length = bytes.len();
            }
            NAPI_OK
        }
        Some(Value::ExternalBuffer {
            data: external,
            length: external_length,
        }) => {
            if !data.is_null() {
                *data = external.cast();
            }
            if !length.is_null() {
                *length = *external_length;
            }
            NAPI_OK
        }
        Some(Value::BufferView {
            array_buffer,
            byte_offset,
            length: view_length,
        }) => {
            let (bytes, _, detached) = match arraybuffer_parts(*array_buffer) {
                Ok(parts) => parts,
                Err(status) => return record_status(env, status),
            };
            if !data.is_null() {
                *data = if detached {
                    ptr::null_mut()
                } else {
                    bytes.add(*byte_offset).cast()
                };
            }
            if !length.is_null() {
                *length = if detached { 0 } else { *view_length };
            }
            NAPI_OK
        }
        _ => NAPI_INVALID_ARG,
    };
    record_status(env, status)
}

#[no_mangle]
pub unsafe extern "C" fn napi_is_buffer(
    env: NapiEnv,
    value: NapiValue,
    out: *mut bool,
) -> NapiStatus {
    if out.is_null() || !value_belongs_to_environment(env, value) {
        return record_status(env, NAPI_INVALID_ARG);
    }
    *out = matches!(
        value_ref(value),
        Ok(Value::Buffer(_) | Value::ExternalBuffer { .. } | Value::BufferView { .. })
    );
    NAPI_OK
}

unsafe fn arraybuffer_parts(value: NapiValue) -> Result<(*mut u8, usize, bool), NapiStatus> {
    match value.as_mut() {
        Some(Value::ArrayBuffer { bytes, detached }) => {
            Ok((bytes.as_mut_ptr(), bytes.len(), *detached))
        }
        Some(Value::SharedArrayBuffer(bytes)) => Ok((bytes.as_mut_ptr(), bytes.len(), false)),
        Some(Value::ExternalSharedArrayBuffer { data, length }) => Ok((*data, *length, false)),
        Some(Value::ExternalArrayBuffer {
            data,
            length,
            detached,
        }) => Ok((*data, *length, *detached)),
        _ => Err(NAPI_ARRAYBUFFER_EXPECTED),
    }
}

fn typedarray_element_size(array_type: i32) -> Option<usize> {
    match array_type {
        0..=2 => Some(1),
        3 | 4 => Some(2),
        5..=7 => Some(4),
        8..=10 => Some(8),
        _ => None,
    }
}

#[no_mangle]
pub unsafe extern "C" fn napi_create_arraybuffer(
    env: NapiEnv,
    length: usize,
    data: *mut *mut c_void,
    out: *mut NapiValue,
) -> NapiStatus {
    let Ok(env) = env_for_value_output(env, out) else {
        return NAPI_INVALID_ARG;
    };
    let value = env.alloc(Value::ArrayBuffer {
        bytes: vec![0; length],
        detached: false,
    });
    if !data.is_null() {
        let Some(Value::ArrayBuffer { bytes, .. }) = value.as_mut() else {
            unreachable!();
        };
        *data = bytes.as_mut_ptr().cast();
    }
    write_value(out, value)
}

#[no_mangle]
pub unsafe extern "C" fn napi_create_external_arraybuffer(
    env: NapiEnv,
    data: *mut c_void,
    length: usize,
    finalize: Option<NapiFinalize>,
    hint: *mut c_void,
    out: *mut NapiValue,
) -> NapiStatus {
    if out.is_null() || (length != 0 && data.is_null()) {
        return NAPI_INVALID_ARG;
    }
    let Ok(env) = env_mut(env) else {
        return NAPI_INVALID_ARG;
    };
    let value = env.alloc(Value::ExternalArrayBuffer {
        data: data.cast(),
        length,
        detached: false,
    });
    if finalize.is_some() {
        env.finalizers.push(FinalizeRecord {
            data,
            finalize,
            hint,
        });
    }
    write_value(out, value)
}

#[no_mangle]
pub unsafe extern "C" fn node_api_create_external_sharedarraybuffer(
    env: NapiEnv,
    data: *mut c_void,
    length: usize,
    finalize: Option<NodeApiNoEnvFinalize>,
    hint: *mut c_void,
    out: *mut NapiValue,
) -> NapiStatus {
    if out.is_null() || (length != 0 && data.is_null()) {
        return NAPI_INVALID_ARG;
    }
    let Ok(env) = env_mut(env) else {
        return NAPI_INVALID_ARG;
    };
    let value = env.alloc(Value::ExternalSharedArrayBuffer {
        data: data.cast(),
        length,
    });
    if finalize.is_some() {
        env.noenv_finalizers.push(NoEnvFinalizeRecord {
            data,
            finalize,
            hint,
        });
    }
    write_value(out, value)
}

#[no_mangle]
pub unsafe extern "C" fn node_api_create_sharedarraybuffer(
    env: NapiEnv,
    length: usize,
    data: *mut *mut c_void,
    out: *mut NapiValue,
) -> NapiStatus {
    if out.is_null() {
        return NAPI_INVALID_ARG;
    }
    let Ok(env) = env_mut(env) else {
        return NAPI_INVALID_ARG;
    };
    let value = env.alloc(Value::SharedArrayBuffer(vec![0; length]));
    if let Some(data) = data.as_mut() {
        let Some(Value::SharedArrayBuffer(bytes)) = value.as_mut() else {
            unreachable!();
        };
        *data = bytes.as_mut_ptr().cast();
    }
    write_value(out, value)
}

#[no_mangle]
pub unsafe extern "C" fn node_api_is_sharedarraybuffer(
    env: NapiEnv,
    value: NapiValue,
    result: *mut bool,
) -> NapiStatus {
    if !value_belongs_to_environment(env, value) {
        return NAPI_INVALID_ARG;
    }
    let Some(result) = result.as_mut() else {
        return NAPI_INVALID_ARG;
    };
    *result = matches!(
        value_ref(value),
        Ok(Value::SharedArrayBuffer(_) | Value::ExternalSharedArrayBuffer { .. })
    );
    NAPI_OK
}

#[no_mangle]
pub unsafe extern "C" fn napi_adjust_external_memory(
    env: NapiEnv,
    change_in_bytes: i64,
    adjusted_value: *mut i64,
) -> NapiStatus {
    if adjusted_value.is_null() {
        return NAPI_INVALID_ARG;
    }
    let Ok(env) = env_mut(env) else {
        return NAPI_INVALID_ARG;
    };
    let Some(adjusted) = env.external_memory.checked_add(change_in_bytes) else {
        return NAPI_GENERIC_FAILURE;
    };
    env.external_memory = adjusted;
    *adjusted_value = adjusted;
    NAPI_OK
}

#[no_mangle]
pub unsafe extern "C" fn napi_is_arraybuffer(
    env: NapiEnv,
    value: NapiValue,
    out: *mut bool,
) -> NapiStatus {
    if out.is_null() || !value_belongs_to_environment(env, value) {
        return NAPI_INVALID_ARG;
    }
    *out = matches!(
        value_ref(value),
        Ok(Value::ArrayBuffer { .. } | Value::ExternalArrayBuffer { .. })
    );
    NAPI_OK
}

#[no_mangle]
pub unsafe extern "C" fn napi_get_arraybuffer_info(
    env: NapiEnv,
    value: NapiValue,
    data: *mut *mut c_void,
    length: *mut usize,
) -> NapiStatus {
    if !value_belongs_to_environment(env, value) {
        return NAPI_INVALID_ARG;
    }
    let Ok((bytes, byte_length, detached)) = arraybuffer_parts(value) else {
        return record_status(env, NAPI_ARRAYBUFFER_EXPECTED);
    };
    if !data.is_null() {
        *data = if detached {
            ptr::null_mut()
        } else {
            bytes.cast()
        };
    }
    if !length.is_null() {
        *length = if detached { 0 } else { byte_length };
    }
    NAPI_OK
}

#[no_mangle]
pub unsafe extern "C" fn napi_detach_arraybuffer(env: NapiEnv, value: NapiValue) -> NapiStatus {
    if !value_belongs_to_environment(env, value) {
        return NAPI_INVALID_ARG;
    }
    let status = match value.as_mut() {
        Some(Value::ArrayBuffer { detached, .. })
        | Some(Value::ExternalArrayBuffer { detached, .. }) => {
            *detached = true;
            NAPI_OK
        }
        _ => NAPI_ARRAYBUFFER_EXPECTED,
    };
    record_status(env, status)
}

#[no_mangle]
pub unsafe extern "C" fn napi_is_detached_arraybuffer(
    env: NapiEnv,
    value: NapiValue,
    out: *mut bool,
) -> NapiStatus {
    if out.is_null() || !value_belongs_to_environment(env, value) {
        return record_status(env, NAPI_INVALID_ARG);
    }
    *out = match value_ref(value) {
        Ok(Value::ArrayBuffer { detached, .. })
        | Ok(Value::ExternalArrayBuffer { detached, .. }) => *detached,
        _ => return record_status(env, NAPI_ARRAYBUFFER_EXPECTED),
    };
    NAPI_OK
}

#[no_mangle]
pub unsafe extern "C" fn napi_create_typedarray(
    env: NapiEnv,
    array_type: i32,
    length: usize,
    array_buffer: NapiValue,
    byte_offset: usize,
    out: *mut NapiValue,
) -> NapiStatus {
    if out.is_null() || !value_belongs_to_environment(env, array_buffer) {
        return record_status(env, NAPI_INVALID_ARG);
    }
    let Some(element_size) = typedarray_element_size(array_type) else {
        return record_status(env, NAPI_INVALID_ARG);
    };
    let Ok((_, buffer_length, detached)) = arraybuffer_parts(array_buffer) else {
        return record_status(env, NAPI_ARRAYBUFFER_EXPECTED);
    };
    if detached {
        return record_status(env, NAPI_INVALID_ARG);
    }
    let Some(byte_length) = length.checked_mul(element_size) else {
        return record_status(env, NAPI_INVALID_ARG);
    };
    let Some(end) = byte_offset.checked_add(byte_length) else {
        return record_status(env, NAPI_INVALID_ARG);
    };
    if !byte_offset.is_multiple_of(element_size) || end > buffer_length {
        return record_status(env, NAPI_INVALID_ARG);
    }
    let Ok(env) = env_mut(env) else {
        return NAPI_INVALID_ARG;
    };
    let value = env.alloc(Value::TypedArray {
        array_type,
        length,
        array_buffer,
        byte_offset,
    });
    write_value(out, value)
}

#[no_mangle]
pub unsafe extern "C" fn napi_is_typedarray(
    env: NapiEnv,
    value: NapiValue,
    out: *mut bool,
) -> NapiStatus {
    if out.is_null() || !value_belongs_to_environment(env, value) {
        return NAPI_INVALID_ARG;
    }
    *out = matches!(
        value_ref(value),
        Ok(Value::TypedArray { .. }
            | Value::Buffer(_)
            | Value::ExternalBuffer { .. }
            | Value::BufferView { .. })
    );
    NAPI_OK
}

#[no_mangle]
pub unsafe extern "C" fn napi_create_dataview(
    env: NapiEnv,
    length: usize,
    array_buffer: NapiValue,
    byte_offset: usize,
    out: *mut NapiValue,
) -> NapiStatus {
    if out.is_null() || !value_belongs_to_environment(env, array_buffer) {
        return record_status(env, NAPI_INVALID_ARG);
    }
    let Ok((_, buffer_length, detached)) = arraybuffer_parts(array_buffer) else {
        return record_status(env, NAPI_ARRAYBUFFER_EXPECTED);
    };
    if detached {
        return record_status(env, NAPI_INVALID_ARG);
    }
    let Some(end) = byte_offset.checked_add(length) else {
        return record_status(env, NAPI_INVALID_ARG);
    };
    if end > buffer_length {
        return record_status(env, NAPI_INVALID_ARG);
    }
    let Ok(env) = env_mut(env) else {
        return NAPI_INVALID_ARG;
    };
    let value = env.alloc(Value::DataView {
        length,
        array_buffer,
        byte_offset,
    });
    write_value(out, value)
}

#[no_mangle]
pub unsafe extern "C" fn napi_is_dataview(
    env: NapiEnv,
    value: NapiValue,
    out: *mut bool,
) -> NapiStatus {
    if out.is_null() || !value_belongs_to_environment(env, value) {
        return NAPI_INVALID_ARG;
    }
    *out = matches!(value_ref(value), Ok(Value::DataView { .. }));
    NAPI_OK
}

#[no_mangle]
pub unsafe extern "C" fn napi_get_dataview_info(
    env: NapiEnv,
    value: NapiValue,
    length: *mut usize,
    data: *mut *mut c_void,
    array_buffer: *mut NapiValue,
    byte_offset: *mut usize,
) -> NapiStatus {
    if !value_belongs_to_environment(env, value) {
        return NAPI_INVALID_ARG;
    }
    let Some(Value::DataView {
        length: view_length,
        array_buffer: backing,
        byte_offset: offset,
    }) = value.as_mut()
    else {
        return record_status(env, NAPI_INVALID_ARG);
    };
    let (view_length, backing, offset) = (*view_length, *backing, *offset);
    let Ok((bytes, _, detached)) = arraybuffer_parts(backing) else {
        return record_status(env, NAPI_ARRAYBUFFER_EXPECTED);
    };
    if !length.is_null() {
        *length = if detached { 0 } else { view_length };
    }
    if !data.is_null() {
        *data = if detached {
            ptr::null_mut()
        } else {
            bytes.add(offset).cast()
        };
    }
    if !array_buffer.is_null() {
        *array_buffer = backing;
    }
    if !byte_offset.is_null() {
        *byte_offset = offset;
    }
    NAPI_OK
}

#[no_mangle]
pub unsafe extern "C" fn napi_get_typedarray_info(
    env: NapiEnv,
    value: NapiValue,
    array_type: *mut i32,
    length: *mut usize,
    data: *mut *mut c_void,
    array_buffer: *mut NapiValue,
    byte_offset: *mut usize,
) -> NapiStatus {
    if !value_belongs_to_environment(env, value) {
        return NAPI_INVALID_ARG;
    }
    match value.as_mut() {
        Some(Value::TypedArray {
            array_type: kind,
            length: view_length,
            array_buffer: backing,
            byte_offset: offset,
        }) => {
            let (kind, view_length, backing, offset) = (*kind, *view_length, *backing, *offset);
            let Ok((bytes, _, detached)) = arraybuffer_parts(backing) else {
                return record_status(env, NAPI_ARRAYBUFFER_EXPECTED);
            };
            if !array_type.is_null() {
                *array_type = kind;
            }
            if !length.is_null() {
                *length = if detached { 0 } else { view_length };
            }
            if !data.is_null() {
                *data = if detached {
                    ptr::null_mut()
                } else {
                    bytes.add(offset).cast()
                };
            }
            if !array_buffer.is_null() {
                *array_buffer = backing;
            }
            if !byte_offset.is_null() {
                *byte_offset = offset;
            }
            NAPI_OK
        }
        Some(Value::Buffer(bytes)) => {
            let buffer_length = bytes.len();
            if !array_type.is_null() {
                *array_type = 1;
            }
            if !length.is_null() {
                *length = buffer_length;
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
        Some(Value::ExternalBuffer {
            data: external,
            length: buffer_length,
        }) => {
            if !array_type.is_null() {
                *array_type = 1;
            }
            if !length.is_null() {
                *length = *buffer_length;
            }
            if !data.is_null() {
                *data = external.cast();
            }
            if !array_buffer.is_null() {
                *array_buffer = value;
            }
            if !byte_offset.is_null() {
                *byte_offset = 0;
            }
            NAPI_OK
        }
        Some(Value::BufferView {
            array_buffer: backing,
            byte_offset: offset,
            length: view_length,
        }) => {
            let (bytes, _, detached) = match arraybuffer_parts(*backing) {
                Ok(parts) => parts,
                Err(status) => return record_status(env, status),
            };
            if !array_type.is_null() {
                *array_type = 1;
            }
            if !length.is_null() {
                *length = if detached { 0 } else { *view_length };
            }
            if !data.is_null() {
                *data = if detached {
                    ptr::null_mut()
                } else {
                    bytes.add(*offset).cast()
                };
            }
            if !array_buffer.is_null() {
                *array_buffer = *backing;
            }
            if !byte_offset.is_null() {
                *byte_offset = *offset;
            }
            NAPI_OK
        }
        _ => record_status(env, NAPI_INVALID_ARG),
    }
}

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
        return NAPI_INVALID_ARG;
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
    let (Some(env), Some(result)) = (env.as_ref(), result.as_mut()) else {
        return NAPI_INVALID_ARG;
    };
    *result = env.module_file_name.as_ptr();
    NAPI_OK
}

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
        && !matches!(value_ref(async_resource_name), Ok(Value::String(_)))
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
    async_completions()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .push_back(work as usize);
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
    let mut completed = drain_posted_finalizers();
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

    unsafe extern "C" fn plain_external_finalize(
        _env: NapiEnv,
        data: *mut c_void,
        hint: *mut c_void,
    ) {
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
            assert!(
                matches!(value_ref(names[2].unwrap()), Ok(Value::String(name)) if name == "alpha")
            );
            assert!(
                matches!(value_ref(names[3].unwrap()), Ok(Value::String(name)) if name == "beta")
            );
            assert_eq!(names[4], Some(symbol));
        }
    }

    #[test]
    fn generic_properties_cover_all_javascript_object_kinds() {
        unsafe extern "C" fn return_callback_data(
            _env: NapiEnv,
            info: NapiCallbackInfo,
        ) -> NapiValue {
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
        unsafe extern "C" fn return_callback_data(
            _env: NapiEnv,
            info: NapiCallbackInfo,
        ) -> NapiValue {
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
                napi_get_named_property(
                    env_ptr,
                    constructor,
                    c"prototype".as_ptr(),
                    &mut prototype,
                ),
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
                napi_create_string_utf16(
                    env_ptr,
                    utf16.as_ptr(),
                    NAPI_AUTO_LENGTH,
                    &mut utf16_value,
                ),
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

            let mut other_info = ptr::null();
            assert_eq!(
                napi_get_last_error_info(&mut other_env, &mut other_info),
                NAPI_OK
            );
            assert_eq!((*other_info).error_code, NAPI_OK);

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
                assert!(
                    matches!(value_ref(result), Ok(Value::Number(number)) if *number == expected)
                );
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
                napi_get_value_bigint_words(
                    env_ptr,
                    value,
                    &mut sign,
                    &mut count,
                    copy.as_mut_ptr(),
                ),
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
                napi_get_value_bigint_words(
                    env_ptr,
                    bigint,
                    &mut sign,
                    &mut count,
                    ptr::null_mut()
                ),
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
            let mut pending = false;
            assert_eq!(napi_is_exception_pending(env_ptr, &mut pending), NAPI_OK);
            assert!(pending);
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

    unsafe extern "C" fn async_cleanup_probe(
        handle: *mut AsyncCleanupHookHandle,
        data: *mut c_void,
    ) {
        cleanup_probe(data);
        assert_eq!(napi_remove_async_cleanup_hook(handle), NAPI_OK);
    }

    unsafe extern "C" fn hold_async_cleanup(
        handle: *mut AsyncCleanupHookHandle,
        _data: *mut c_void,
    ) {
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
