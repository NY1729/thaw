//! Minimal Node-API host for loading `.node` addons.

// Every exported unsafe function in this crate implements the Node-API C ABI
// and shares its pointer-validity contract with the native addon caller.
#![allow(clippy::missing_safety_doc)]

use base64::Engine;
use flate2::read::GzDecoder;
use libc::{c_char, c_void};
use serde_json::Value as JsonValue;
use std::cell::RefCell;
use std::collections::{HashMap, HashSet, VecDeque};
use std::ffi::{CStr, CString};
use std::io::{Read, Write};
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
type ThawNativeValueCallback = unsafe extern "C" fn(*mut c_void, *const c_char) -> *const c_char;
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ReadyEvent {
    AsyncCompletion(usize),
    ThreadsafeFunction(usize),
}

static READY_EVENTS: OnceLock<Mutex<VecDeque<ReadyEvent>>> = OnceLock::new();
static ASYNC_POOL: OnceLock<Option<Arc<AsyncPool>>> = OnceLock::new();
static ACTIVE_ASYNC_WORK: AtomicUsize = AtomicUsize::new(0);
static ACTIVE_THREADSAFE_FUNCTIONS: AtomicUsize = AtomicUsize::new(0);
static LIVE_THREADSAFE_FUNCTIONS: AtomicUsize = AtomicUsize::new(0);
static FATAL_EXCEPTION_PENDING: AtomicBool = AtomicBool::new(false);
static ACTIVE_ASYNC_CLEANUP_HOOKS: AtomicUsize = AtomicUsize::new(0);
static NEXT_SYMBOL_ID: AtomicU64 = AtomicU64::new(1);
static GLOBAL_SYMBOLS: OnceLock<Mutex<HashMap<String, u64>>> = OnceLock::new();
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

fn ready_events() -> &'static Mutex<VecDeque<ReadyEvent>> {
    READY_EVENTS.get_or_init(|| Mutex::new(VecDeque::new()))
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
        ready_events()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push_back(ReadyEvent::ThreadsafeFunction(function as usize));
    }
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
        ready_events()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push_back(ReadyEvent::AsyncCompletion(work_address));
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
    callback: ThawCallback,
    context: usize,
}

enum ThawCallback {
    Event(ThawNativeCallback),
    Value(ThawNativeValueCallback),
    #[cfg(feature = "quickjs")]
    QuickJs(ThawNativeValueCallback),
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
    object_finalizers: HashMap<usize, Vec<FinalizeRecord>>,
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
    quickjs_references: HashMap<u64, NapiValue>,
    #[cfg(feature = "quickjs")]
    released_handles: HashSet<usize>,
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
            object_finalizers: HashMap::new(),
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
            quickjs_references: HashMap::new(),
            #[cfg(feature = "quickjs")]
            released_handles: HashSet::new(),
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
        for record in std::mem::take(&mut self.object_finalizers)
            .into_values()
            .flatten()
        {
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
    embedded_files: Vec<std::fs::File>,
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
            embedded_files: Vec::new(),
            module_envs: Vec::new(),
            pending_call_envs: Vec::new(),
            last_error: String::new(),
        }
    }
}

impl Drop for Host {
    fn drop(&mut self) {
        if ACTIVE_ASYNC_WORK.load(Ordering::Acquire) != 0
            || LIVE_THREADSAFE_FUNCTIONS.load(Ordering::Acquire) != 0
            || ACTIVE_ASYNC_CLEANUP_HOOKS.load(Ordering::Acquire) != 0
        {
            // ponytail: At process exit the OS reclaims these environments; running
            // addon finalizers after thread-local HOST destruction is invalid.
            for env in self.module_envs.drain(..) {
                Box::leak(env);
            }
            for env in self.pending_call_envs.drain(..) {
                Box::leak(env);
            }
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

include!("napi/property_storage.rs");

include!("napi/module_host.rs");

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
