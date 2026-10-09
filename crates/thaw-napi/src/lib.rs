//! Minimal Node-API host for loading `.node` addons.

// Every exported unsafe function in this crate implements the Node-API C ABI
// and shares its pointer-validity contract with the native addon caller.
#![allow(clippy::missing_safety_doc)]

use base64::Engine;
use flate2::read::GzDecoder;
use libc::{c_char, c_void};
use serde_json::Value as JsonValue;
use std::cell::{Cell, RefCell};
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

#[path = "napi/version.rs"]
mod napi_version;

// With the `quickjs` feature (on by default), this crate transitively
// needs thaw-quickjs's `thaw_js_dynamic_object_query`, which calls
// thaw-std's `thaw_json_parse` as an `extern "C"` declaration resolved at
// link time (no real Rust-level dependency between these archives -- see
// thaw-runtime's own `Cargo.toml` comment) -- without this marker import,
// `cargo test` never actually links thaw-std's rlib in and that symbol
// stays unresolved.
#[cfg(test)]
use thaw_std as _;
// thaw-std's own code calls a couple of thaw-runtime helpers the same
// "resolved at link time" way (`thaw_date_to_iso_string`/
// `thaw_string_to_number`/the `fetch`/`http` event-loop hooks).
#[cfg(test)]
use thaw_runtime as _;

type NapiEnv = *mut Env;
type NapiValue = *mut Value;
type NapiCallbackInfo = *mut CallbackInfo;
type NapiStatus = i32;
type NapiCallback = unsafe extern "C" fn(NapiEnv, NapiCallbackInfo) -> NapiValue;

thread_local! {
    static FOREIGN_CALLBACK_DEPTH: Cell<usize> = const { Cell::new(0) };
    // A graph callback may return an nh value into a still-running addon
    // invocation. Its wire reference must outlive the callback trampoline.
    static NAPI_RECIPIENT_GRAPH_PINS: RefCell<Vec<u64>> = const { RefCell::new(Vec::new()) };
}

struct ForeignCallbackGuard(bool);

impl ForeignCallbackGuard {
    fn new() -> Self {
        Self(
            FOREIGN_CALLBACK_DEPTH
                .try_with(|depth| depth.set(depth.get() + 1))
                .is_ok(),
        )
    }

    fn active() -> bool {
        FOREIGN_CALLBACK_DEPTH
            .try_with(|depth| depth.get() != 0)
            .unwrap_or(false)
    }
}

impl Drop for ForeignCallbackGuard {
    fn drop(&mut self) {
        if self.0 {
            let _ = FOREIGN_CALLBACK_DEPTH.try_with(|depth| {
                if depth.get() == 1 {
                    // Keep this guard active while finalizers can reenter.
                    loop {
                        let pending = NAPI_RECIPIENT_GRAPH_PINS
                            .try_with(|pins| std::mem::take(&mut *pins.borrow_mut()))
                            .unwrap_or_default();
                        if pending.is_empty() {
                            break;
                        }
                        for reference in pending {
                            release_napi_graph_reference(reference);
                        }
                    }
                }
                depth.set(depth.get() - 1);
            });
        }
    }
}

unsafe fn invoke_napi_callback(
    env: NapiEnv,
    callback: NapiCallback,
    info: NapiCallbackInfo,
) -> NapiValue {
    if env
        .as_ref()
        .is_some_and(|env| env.shutdown_requested || env.finalizing || env.finalized)
    {
        return closing_napi_callback(env, info);
    }
    let _dispatch = ForeignCallbackGuard::new();
    callback(env, info)
}

unsafe extern "C" fn closing_napi_callback(env: NapiEnv, _info: NapiCallbackInfo) -> NapiValue {
    if let Ok(env) = env_mut(env) {
        let error = env.alloc(Value::Error("N-API addon is closing".into()));
        env.exception = Some(error);
    }
    ptr::null_mut()
}
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
    owner: std::thread::ThreadId,
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

impl ReadyEvent {
    // Pointers stay Box-stable until their completion/finalizer runs. Only
    // their creating Host thread may execute a JS-facing ready callback.
    unsafe fn owner(self) -> std::thread::ThreadId {
        match self {
            Self::AsyncCompletion(address) => (*(address as *const AsyncWork)).owner,
            Self::ThreadsafeFunction(address) => (*(address as *const ThreadsafeFunction)).creator,
        }
    }
}

static READY_EVENTS: OnceLock<Mutex<VecDeque<ReadyEvent>>> = OnceLock::new();
static ASYNC_POOL: OnceLock<Option<Arc<AsyncPool>>> = OnceLock::new();
static ACTIVE_ASYNC_WORK: AtomicUsize = AtomicUsize::new(0);
static ACTIVE_THREADSAFE_FUNCTIONS: AtomicUsize = AtomicUsize::new(0);
static UNFINALIZED_THREADSAFE_FUNCTIONS: AtomicUsize = AtomicUsize::new(0);
static LIVE_THREADSAFE_FUNCTIONS: AtomicUsize = AtomicUsize::new(0);
static FATAL_EXCEPTION_PENDING: AtomicBool = AtomicBool::new(false);
static ACTIVE_ASYNC_CLEANUP_HOOKS: AtomicUsize = AtomicUsize::new(0);
static NEXT_SYMBOL_ID: AtomicU64 = AtomicU64::new(1);
static GLOBAL_SYMBOLS: OnceLock<Mutex<HashMap<Vec<u16>, u64>>> = OnceLock::new();
#[allow(clippy::vec_box)]
static THREADSAFE_FUNCTIONS: OnceLock<Mutex<Vec<Box<ThreadsafeFunction>>>> = OnceLock::new();
#[allow(clippy::vec_box)]
static ASYNC_CLEANUP_HANDLES: OnceLock<Mutex<Vec<Box<AsyncCleanupHookHandle>>>> = OnceLock::new();
thread_local! {
    // Libuv loops must only be driven on the thread that registered them.
    static REGISTERED_UV_LOOPS: RefCell<Vec<usize>> = const { RefCell::new(Vec::new()) };
}

fn registered_uv_loops() -> Vec<usize> {
    REGISTERED_UV_LOOPS.with(|loops| loops.borrow().clone())
}

#[cfg(target_os = "linux")]
fn record_process_default_uv_loop() {
    // Record the process main thread's default-loop receipt while the loader
    // owns libuv. Polling later uses only this recorded owner/pointer; Worker
    // Hosts never infer ownership from a global symbol at poll time.
    let current_tid = unsafe { libc::syscall(libc::SYS_gettid) };
    let main_tid = unsafe { libc::getpid() } as libc::c_long;
    if current_tid != main_tid {
        return;
    }
    type UvDefaultLoop = unsafe extern "C" fn() -> *mut c_void;
    let symbol = unsafe { libc::dlsym(libc::RTLD_DEFAULT, c"uv_default_loop".as_ptr()) };
    if symbol.is_null() {
        return;
    }
    let loop_ptr = unsafe { std::mem::transmute::<*mut c_void, UvDefaultLoop>(symbol)() };
    if !loop_ptr.is_null() {
        HOST.with(|host| host.borrow_mut().main_default_uv_loop = Some(loop_ptr as usize));
    }
}

fn known_uv_loops() -> Vec<usize> {
    let mut loops = registered_uv_loops();
    let owned = HOST.with(|host| host.borrow().owned_uv_loop);
    if let Some(owned) = owned {
        if !loops.contains(&owned) {
            loops.push(owned);
        }
    }
    let default_loop = HOST.with(|host| host.borrow().main_default_uv_loop);
    if let Some(default_loop) = default_loop {
        if !loops.contains(&default_loop) {
            loops.push(default_loop);
        }
    }
    loops
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
    callbacks_in_flight: usize,
    finalized: bool,
    finalizer_completed: bool,
    live_released: bool,
}

fn retire_threadsafe_if_ready(state: &mut ThreadsafeState) -> bool {
    if state.finalizer_completed && state.thread_count == 0 && !state.live_released {
        state.live_released = true;
        true
    } else {
        false
    }
}

fn ready_events() -> &'static Mutex<VecDeque<ReadyEvent>> {
    READY_EVENTS.get_or_init(|| Mutex::new(VecDeque::new()))
}

fn take_ready_event_for_current_thread() -> Option<ReadyEvent> {
    let owner = std::thread::current().id();
    let mut ready = ready_events()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let position = ready
        .iter()
        .position(|event| unsafe { event.owner() == owner })?;
    ready.remove(position)
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
        // The queue entry owns one strong reference (see napi_queue_async_work);
        // keep it until this worker is completely done with the item.
        let work_ref = unsafe { Arc::from_raw(work_address as *const AsyncWork) };
        let work = &*work_ref;
        unsafe {
            (work.execute)(work.env as NapiEnv, work.data as *mut c_void);
        }
        work.completion_status.store(NAPI_OK, Ordering::Release);
        work.state.store(ASYNC_COMPLETE_PENDING, Ordering::Release);
        ready_events()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push_back(ReadyEvent::AsyncCompletion(work_address));
        drop(work_ref);
    }
}

#[derive(Clone)]
pub struct Function {
    callback: NapiCallback,
    data: *mut c_void,
    properties: HashMap<PropertyKey, NapiValue>,
    _thaw_bridge: Option<Arc<ThawCallbackBridge>>,
    // A reflected native accessor remains callable after its property is
    // replaced or deleted. Its copied descriptor also pins JS callback roots.
    _accessor_owner: Option<Arc<Accessor>>,
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum PropertyKey {
    String(String),
    Utf16(Vec<u16>),
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

#[derive(Clone, Copy)]
enum ThawCallback {
    Event(ThawNativeCallback),
    EventGraph(ThawNativeCallback),
    Value(ThawNativeValueCallback),
    ValueGraph(ThawNativeValueCallback),
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
        retired: bool,
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
    // One independently retained QuickJS handle, owned by this Env.
    QuickJsHandle {
        handle: u64,
        object_like: bool,
    },
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

#[derive(Clone, Copy, Eq, PartialEq)]
enum NapiGraphWrapperKind {
    Map,
    Set,
    RegExp,
}

pub struct Env {
    owner: std::thread::ThreadId,
    values: Vec<NapiValue>,
    // Each allocation carries a generation so a private temporary removed
    // from `values` cannot be mistaken for a later Box at the same address.
    value_generations: HashMap<usize, u64>,
    // Values that outlived their creating scope through an external root are
    // revisited when that root is released. The generation rejects stale slots.
    pending_scope_values: Vec<(NapiValue, u64)>,
    rooted_scope_escapes: HashSet<usize>,
    proxy_live_counts: HashMap<usize, u32>,
    next_value_generation: u64,
    graph_wrappers: HashMap<usize, NapiGraphWrapperKind>,
    global: NapiValue,
    exception: Option<NapiValue>,
    wraps: HashMap<usize, WrapRecord>,
    instances: HashMap<usize, usize>,
    // Explicit object/Array constructor returns are live native objects,
    // without acquiring the constructor's nominal instance identity.
    live_constructor_returns: HashSet<usize>,
    prototypes: HashMap<usize, usize>,
    accessors: HashMap<(usize, PropertyKey), Accessor>,
    finalizers: Vec<FinalizeRecord>,
    object_finalizers: HashMap<usize, Vec<FinalizeRecord>>,
    noenv_finalizers: Vec<NoEnvFinalizeRecord>,
    posted_finalizers: Vec<FinalizeRecord>,
    instance_data: Option<FinalizeRecord>,
    cleanup_hooks: Vec<CleanupHookRecord>,
    async_cleanup_hooks: Vec<*mut AsyncCleanupHookHandle>,
    // Set before native module init so async hooks always have a pinned Host owner.
    host_managed: bool,
    async_cleanup_dispatching: bool,
    shutdown_requested: bool,
    // Exact Host-owned loop returned through napi_get_uv_event_loop to this Env.
    acquired_default_uv_loop: Option<usize>,
    finalizing: bool,
    finalized: bool,
    external_memory: i64,
    sealed_objects: HashSet<usize>,
    nonextensible_objects: HashSet<usize>,
    frozen_objects: HashSet<usize>,
    property_attributes: HashMap<(usize, PropertyKey), u32>,
    property_order: HashMap<usize, Vec<PropertyKey>>,
    host_properties: HashMap<usize, HashMap<PropertyKey, NapiValue>>,
    error_names: HashMap<usize, String>,
    symbols: HashMap<u64, NapiValue>,
    // Native Symbol.for IDs and permanent JS Symbols stay alive without a
    // positive reference. Ordinary napi_create_symbol IDs are weak metadata.
    strong_symbol_ids: HashSet<u64>,
    // Provenance is separate from weak-root policy: Symbol.for and well-known
    // Symbols imported from JS are permanent yet must revive as their exact
    // original JS values, never by description or a fresh local Symbol.
    js_original_symbol_ids: HashSet<u64>,
    // Local symbols imported from the per-addon QuickJS realm have a weak JS
    // cache entry. Their native wrapper is kept only while native roots or
    // the engine's independent WeakRef target remain live.
    js_origin_symbol_ids: HashSet<u64>,
    // A trusted JS finalization notice clears this record. Absence of a
    // notice is conservative: a failed/missing weak query cannot establish
    // that the JS Symbol is unreachable.
    js_live_symbol_ids: HashSet<u64>,
    js_registering_symbol_ids: HashSet<u64>,
    js_pinned_symbol_ids: HashSet<u64>,
    type_tags: HashMap<usize, NapiTypeTag>,
    property_keys: HashMap<String, NapiValue>,
    utf16_property_keys: HashMap<Vec<u16>, NapiValue>,
    // Malformed UTF-16 needs exact code units alongside the ordinary display string.
    utf16_strings: HashMap<usize, Vec<u16>>,
    utf16_symbols: HashMap<usize, Vec<u16>>,
    // Legacy ownerless carrier metadata; managed bridge Symbols use native
    // IDs plus per-owner weak JS identity rather than an Env-owned handle.
    #[cfg(feature = "quickjs")]
    quickjs_symbol_ids: HashMap<u64, u64>,
    quickjs_references: HashMap<u64, NapiValue>,
    quickjs_live_values: HashMap<u64, NapiValue>,
    #[cfg(feature = "quickjs")]
    released_handles: HashSet<usize>,
    // Positive references held by native Json graph nodes postpone proxy
    // finalization and Env cleanup until their last shared Rc owner drops.
    native_graph_pins: usize,
    // Only references created for graph transport may be consumed by a
    // graph wire's lease list. An arbitrary positive N-API reference is not
    // interchangeable with one of these transfer tokens.
    graph_reference_tokens: HashSet<usize>,
    // A raw u64 handle returned to compiled code has no proxy finalizer of
    // its own. Root each escaped instance once until this Env shuts down.
    escaped_native_handles: HashSet<usize>,
    finalized_handles: HashSet<usize>,
    graph_owner_id: u64,
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
    // Shared with the worker pool: a queued or executing work item stays
    // allocated until the worker drops its reference, even if the Env is gone.
    async_works: Vec<Arc<AsyncWork>>,
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
    escaped_value: Option<(NapiValue, u64)>,
    values: Vec<(NapiValue, u64)>,
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
    backing: NapiValue,
}

struct NoEnvFinalizeRecord {
    data: *mut c_void,
    finalize: Option<NodeApiNoEnvFinalize>,
    hint: *mut c_void,
    backing: NapiValue,
}

struct WrapRecord {
    data: *mut c_void,
    finalize: Option<NapiFinalize>,
    hint: *mut c_void,
}

#[derive(Clone)]
struct Accessor {
    getter: Option<NapiCallback>,
    setter: Option<NapiCallback>,
    getter_data: *mut c_void,
    setter_data: *mut c_void,
    getter_reflection: Option<NapiValue>,
    setter_reflection: Option<NapiValue>,
    #[cfg(feature = "quickjs")]
    js_owner: Option<Arc<QuickJsAccessorRoots>>,
}

#[cfg(feature = "quickjs")]
struct QuickJsAccessorRoots {
    getter: u64,
    setter: u64,
    getter_native: NapiValue,
    setter_native: NapiValue,
}

#[cfg(feature = "quickjs")]
impl Drop for QuickJsAccessorRoots {
    fn drop(&mut self) {
        // Releasing the last callback handle may run a JS finalizer that
        // reenters this addon. Keep Env retirement deferred until both
        // independent roots have been returned to the QuickJS registry.
        let _dispatch = ForeignCallbackGuard::new();
        if self.getter != 0 {
            thaw_quickjs::thaw_js_release_handle(self.getter);
        }
        if self.setter != 0 {
            thaw_quickjs::thaw_js_release_handle(self.setter);
        }
    }
}

impl Env {
    fn new() -> Self {
        Self {
            owner: std::thread::current().id(),
            values: Vec::new(),
            value_generations: HashMap::new(),
            pending_scope_values: Vec::new(),
            rooted_scope_escapes: HashSet::new(),
            proxy_live_counts: HashMap::new(),
            next_value_generation: 0,
            graph_wrappers: HashMap::new(),
            global: ptr::null_mut(),
            exception: None,
            wraps: HashMap::new(),
            instances: HashMap::new(),
            live_constructor_returns: HashSet::new(),
            prototypes: HashMap::new(),
            accessors: HashMap::new(),
            finalizers: Vec::new(),
            object_finalizers: HashMap::new(),
            noenv_finalizers: Vec::new(),
            posted_finalizers: Vec::new(),
            instance_data: None,
            cleanup_hooks: Vec::new(),
            async_cleanup_hooks: Vec::new(),
            host_managed: false,
            async_cleanup_dispatching: false,
            shutdown_requested: false,
            acquired_default_uv_loop: None,
            finalizing: false,
            finalized: false,
            external_memory: 0,
            sealed_objects: HashSet::new(),
            nonextensible_objects: HashSet::new(),
            frozen_objects: HashSet::new(),
            property_attributes: HashMap::new(),
            property_order: HashMap::new(),
            host_properties: HashMap::new(),
            error_names: HashMap::new(),
            symbols: HashMap::new(),
            strong_symbol_ids: HashSet::new(),
            js_original_symbol_ids: HashSet::new(),
            js_origin_symbol_ids: HashSet::new(),
            js_live_symbol_ids: HashSet::new(),
            js_registering_symbol_ids: HashSet::new(),
            js_pinned_symbol_ids: HashSet::new(),
            type_tags: HashMap::new(),
            property_keys: HashMap::new(),
            utf16_property_keys: HashMap::new(),
            utf16_strings: HashMap::new(),
            utf16_symbols: HashMap::new(),
            #[cfg(feature = "quickjs")]
            quickjs_symbol_ids: HashMap::new(),
            quickjs_references: HashMap::new(),
            quickjs_live_values: HashMap::new(),
            #[cfg(feature = "quickjs")]
            released_handles: HashSet::new(),
            native_graph_pins: 0,
            graph_reference_tokens: HashSet::new(),
            escaped_native_handles: HashSet::new(),
            finalized_handles: HashSet::new(),
            graph_owner_id: 0,
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
        self.next_value_generation = self
            .next_value_generation
            .checked_add(1)
            .expect("N-API value generation limit exceeded");
        let generation = self.next_value_generation;
        self.value_generations.insert(value as usize, generation);
        self.values.push(value);
        if let Some(scope) = self.active_handle_scopes.last().copied() {
            unsafe {
                (*scope).values.push((value, generation));
            }
        }
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
        .find(|candidate| std::ptr::eq(Arc::as_ptr(candidate), work))
        .map(|candidate| &**candidate)
        .ok_or(NAPI_INVALID_ARG);
    if work.is_err() {
        record_status(env, NAPI_INVALID_ARG);
    }
    work
}

thread_local! {
    // A raw napi_value is a pointer-shaped ABI token. Keep its tiny slot
    // reserved after scope retirement so a stale token can never alias a
    // later allocation; its owned payload is dropped immediately.
    #[allow(clippy::vec_box)]
    static RETIRED_SCOPE_HANDLE_SLOTS: RefCell<Vec<Box<Value>>> = const { RefCell::new(Vec::new()) };
    static RETIRED_SCOPE_HANDLE_IDS: RefCell<HashSet<usize>> = RefCell::new(HashSet::new());
}

impl Env {
    fn scope_reachable_values(
        &self,
        candidates: &HashSet<usize>,
        escaped: Option<NapiValue>,
        external_roots: &[NapiValue],
        include_js_weak: bool,
        include_graph_references: bool,
    ) -> HashSet<usize> {
        fn enqueue(
            value: NapiValue,
            live: &HashSet<usize>,
            seen: &mut HashSet<usize>,
            queue: &mut VecDeque<NapiValue>,
        ) {
            if !value.is_null() && live.contains(&(value as usize)) && seen.insert(value as usize) {
                queue.push_back(value);
            }
        }
        let live: HashSet<_> = self.values.iter().map(|value| *value as usize).collect();
        let mut seen = HashSet::new();
        let mut queue = VecDeque::new();
        for value in &self.values {
            if !candidates.contains(&(*value as usize)) {
                enqueue(*value, &live, &mut seen, &mut queue);
            }
        }
        for scope in &self.active_handle_scopes {
            for (value, generation) in unsafe { &**scope }.values.iter().copied() {
                if self.value_generations.get(&(value as usize)) == Some(&generation) {
                    enqueue(value, &live, &mut seen, &mut queue);
                }
            }
        }
        if let Some(value) = escaped {
            enqueue(value, &live, &mut seen, &mut queue);
        }
        for key in &self.rooted_scope_escapes {
            enqueue(*key as NapiValue, &live, &mut seen, &mut queue);
        }
        enqueue(self.global, &live, &mut seen, &mut queue);
        if let Some(value) = self.exception {
            enqueue(value, &live, &mut seen, &mut queue);
        }
        for value in external_roots {
            enqueue(*value, &live, &mut seen, &mut queue);
        }
        for reference in &self.references {
            if !reference.deleted
                && reference.count > 0
                && (include_graph_references
                    || !self
                        .graph_reference_tokens
                        .contains(&(reference.as_ref() as *const Reference as usize)))
            {
                enqueue(reference.value, &live, &mut seen, &mut queue);
            }
        }
        for value in self
            .property_keys
            .values()
            .chain(self.utf16_property_keys.values())
            .chain(self.quickjs_references.values())
        {
            enqueue(*value, &live, &mut seen, &mut queue);
        }
        for (id, value) in &self.symbols {
            // Native/global IDs and provisional JS registrations have their
            // own roots. A QuickJsHandle entry in this metadata table is not
            // itself an owner; its actual property-key edges are traversed
            // below with every native property map.
            if self.strong_symbol_ids.contains(id)
                || self.js_registering_symbol_ids.contains(id)
                || (self.graph_owner_id == 0
                    && matches!(
                        unsafe { value_ref(*value) },
                        Ok(Value::QuickJsHandle { .. })
                    ))
            {
                enqueue(*value, &live, &mut seen, &mut queue);
            }
            #[cfg(feature = "quickjs")]
            if include_js_weak && self.js_live_symbol_ids.contains(id) {
                enqueue(*value, &live, &mut seen, &mut queue);
            }
        }
        for handle in &self.escaped_native_handles {
            enqueue(*handle as NapiValue, &live, &mut seen, &mut queue);
        }
        for handle in self.proxy_live_counts.keys() {
            enqueue(*handle as NapiValue, &live, &mut seen, &mut queue);
        }
        for context in &self.async_contexts {
            if !context.destroyed {
                enqueue(context.resource, &live, &mut seen, &mut queue);
            }
        }
        for deferred in &self.deferreds {
            let Ok(state) = deferred.state.try_borrow() else {
                return candidates.clone();
            };
            match &*state {
                PromiseState::Resolved(value) | PromiseState::Rejected(value) => {
                    enqueue(*value, &live, &mut seen, &mut queue)
                }
                PromiseState::Pending => {}
            }
            // A Deferred independently owns the state of the Promise value.
            // Its C handle can settle after the creating handle scope closes.
            for value in &self.values {
                if let Value::Promise(promise) = unsafe { &**value } {
                    if Rc::ptr_eq(promise, &deferred.state) {
                        enqueue(*value, &live, &mut seen, &mut queue);
                    }
                }
            }
        }
        let Ok(functions) = threadsafe_functions().try_lock() else {
            return candidates.clone();
        };
        for function in functions.iter() {
            if function.env == self as *const Env as usize {
                let Ok(state) = function.state.try_lock() else {
                    return candidates.clone();
                };
                if !state.live_released {
                    enqueue(function.function as NapiValue, &live, &mut seen, &mut queue);
                }
            }
        }
        // External backing and callback owners can run finalizers when their
        // Value drops. Keep them until the existing Env/proxy retirement path;
        // this scope collector only retires payloads with no user callback.
        for value in &self.values {
            let key = *value as usize;
            // Instance data and posted finalizers belong to the Env itself;
            // object/wrap/external callbacks are collected with their owner.
            let env_owned = self
                .posted_finalizers
                .iter()
                .any(|record| record.backing as usize == key)
                || self
                    .instance_data
                    .as_ref()
                    .is_some_and(|record| record.backing as usize == key);
            if env_owned {
                enqueue(*value, &live, &mut seen, &mut queue);
            }
        }
        while let Some(value) = queue.pop_front() {
            let key = value as usize;
            let mut children = Vec::new();
            match unsafe { &*value } {
                Value::Object(fields) => {
                    for (name, child) in fields {
                        children.push(*child);
                        if let PropertyKey::Symbol(id) = name {
                            if let Some(symbol) = self.symbols.get(id) {
                                children.push(*symbol);
                            }
                        }
                    }
                }
                Value::Array(items) => children.extend(items.iter().flatten().copied()),
                Value::Function(function) => {
                    for (name, child) in &function.properties {
                        children.push(*child);
                        if let PropertyKey::Symbol(id) = name {
                            if let Some(symbol) = self.symbols.get(id) {
                                children.push(*symbol);
                            }
                        }
                    }
                    #[cfg(feature = "quickjs")]
                    if let Some(owner) = &function._accessor_owner {
                        if let Some(roots) = &owner.js_owner {
                            children.push(roots.getter_native);
                            children.push(roots.setter_native);
                        }
                    }
                }
                Value::BufferView { array_buffer, .. }
                | Value::TypedArray { array_buffer, .. }
                | Value::DataView { array_buffer, .. } => children.push(*array_buffer),
                Value::Promise(state) => {
                    let Ok(state) = state.try_borrow() else {
                        return candidates.clone();
                    };
                    match &*state {
                        PromiseState::Resolved(child) | PromiseState::Rejected(child) => {
                            children.push(*child)
                        }
                        PromiseState::Pending => {}
                    }
                }
                _ => {}
            }
            if let Some(prototype) = self.prototypes.get(&key) {
                children.push(*prototype as NapiValue);
            }
            if let Some(properties) = self.host_properties.get(&key) {
                for (name, child) in properties {
                    children.push(*child);
                    if let PropertyKey::Symbol(id) = name {
                        if let Some(symbol) = self.symbols.get(id) {
                            children.push(*symbol);
                        }
                    }
                }
            }
            for ((owner, name), accessor) in &self.accessors {
                if *owner == key {
                    if let Some(value) = accessor.getter_reflection {
                        children.push(value);
                    }
                    if let Some(value) = accessor.setter_reflection {
                        children.push(value);
                    }
                    #[cfg(feature = "quickjs")]
                    if let Some(roots) = &accessor.js_owner {
                        children.push(roots.getter_native);
                        children.push(roots.setter_native);
                    }
                    if let PropertyKey::Symbol(id) = name {
                        if let Some(symbol) = self.symbols.get(id) {
                            children.push(*symbol);
                        }
                    }
                }
            }
            if let Some(class) = self.instances.get(&key) {
                children.push(*class as NapiValue);
            }
            for child in children {
                enqueue(child, &live, &mut seen, &mut queue);
            }
        }
        seen
    }
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
        escaped_value: None,
        values: Vec::new(),
    });
    let scope_ptr = (&mut *scope) as *mut HandleScope;
    env_ref.handle_scopes.push(scope);
    env_ref.active_handle_scopes.push(scope_ptr);
    *out = scope_ptr.cast();
    NAPI_OK
}

fn scope_external_roots(env: NapiEnv, host_managed: bool) -> Option<Vec<NapiValue>> {
    if !host_managed {
        return Some(Vec::new());
    }
    HOST.try_with(|host| {
        let host = host.try_borrow().ok()?;
        let mut roots = Vec::new();
        for (candidate_env, value) in host.exports.values() {
            if *candidate_env == env as usize {
                roots.push(*value);
            }
        }
        roots.extend(host.compiled_callbacks.values().copied());
        let symbols = unsafe { env.as_ref()? };
        for function in host.functions.values() {
            for (name, value) in &function.properties {
                roots.push(*value);
                if let PropertyKey::Symbol(id) = name {
                    if let Some(symbol) = symbols.symbols.get(id) {
                        roots.push(*symbol);
                    }
                }
            }
        }
        Some(roots)
    })
    .ok()
    .flatten()
}

#[cfg(feature = "quickjs")]
unsafe fn sync_js_origin_symbol_pins(env: NapiEnv) {
    let _dispatch = ForeignCallbackGuard::new();
    let Some(host_managed) = env.as_ref().and_then(|owner| {
        (owner.graph_owner_id != 0 && !owner.finalizing && !owner.finalized)
            .then_some(owner.host_managed)
    }) else {
        return;
    };
    let Some(external_roots) = scope_external_roots(env, host_managed) else {
        return;
    };
    let Some((graph_owner_id, changes)) = env.as_ref().map(|owner| {
        let candidates: HashSet<_> = owner
            .js_origin_symbol_ids
            .iter()
            .filter_map(|id| owner.symbols.get(id).copied())
            .filter(|value| owner.values.contains(value))
            .map(|value| value as usize)
            .collect();
        // A wire's positive graph reference must pin the original JS Symbol
        // until its decoder consumes the transfer. Do not include the JS
        // weak-live table itself: that would make the pin self-sustaining.
        let native = owner.scope_reachable_values(&candidates, None, &external_roots, false, true);
        let changes = owner
            .js_origin_symbol_ids
            .iter()
            .filter_map(|id| {
                let value = *owner.symbols.get(id)?;
                let generation = *owner.value_generations.get(&(value as usize))?;
                let pin = native.contains(&(value as usize));
                (pin != owner.js_pinned_symbol_ids.contains(id))
                    .then_some((*id, value, generation, pin))
            })
            .collect::<Vec<_>>();
        (owner.graph_owner_id, changes)
    }) else {
        return;
    };
    for (id, value, generation, pin) in changes {
        if thaw_quickjs::thaw_js_napi_symbol_set_pin(graph_owner_id, id, u8::from(pin)) == 0 {
            continue;
        }
        let Some(owner) = env.as_mut() else {
            return;
        };
        if owner.value_generations.get(&(value as usize)) != Some(&generation)
            || owner.symbols.get(&id) != Some(&value)
        {
            continue;
        }
        if pin {
            owner.js_pinned_symbol_ids.insert(id);
        } else {
            owner.js_pinned_symbol_ids.remove(&id);
        }
    }
}

#[derive(Default)]
struct ScopedFinalizeCallbacks {
    wraps: Vec<WrapRecord>,
    objects: Vec<FinalizeRecord>,
    externals: Vec<FinalizeRecord>,
    noenv: Vec<NoEnvFinalizeRecord>,
}

// A scope collector can run a user finalizer after the initiating N-API call
// has already recorded its status. Keep that call's exception live through
// reentrant finalizers, report each finalizer exception through the existing
// shutdown-error queue, and restore the initiating call's error channels.
struct ScopedFinalizerErrorGuard {
    env: NapiEnv,
    original_exception: Option<NapiValue>,
    temporary_reference: Option<*mut Reference>,
    original_error: Option<(*const c_char, *mut c_void, u32, NapiStatus)>,
}

impl ScopedFinalizerErrorGuard {
    unsafe fn new(env: NapiEnv) -> Self {
        let Ok(owner) = env_mut(env) else {
            return Self {
                env,
                original_exception: None,
                temporary_reference: None,
                original_error: None,
            };
        };
        let original_exception = owner.exception.take();
        // A separate positive Reference remains a root even if a reentrant
        // finalizer also escapes this Value. A temporary set entry could not
        // distinguish those two owners when it was removed.
        let temporary_reference = original_exception.map(|value| alloc_reference(owner, value, 1));
        let info = &owner.last_error_info;
        Self {
            env,
            original_exception,
            temporary_reference,
            original_error: Some((
                info.error_message,
                info.engine_reserved,
                info.engine_error_code,
                info.error_code,
            )),
        }
    }
}

impl Drop for ScopedFinalizerErrorGuard {
    fn drop(&mut self) {
        let Some((message, reserved, engine_code, code)) = self.original_error else {
            return;
        };
        unsafe {
            let Ok(owner) = env_mut(self.env) else {
                return;
            };
            owner.exception = self.original_exception;
            if let Some(reference) = self.temporary_reference {
                if let Some(index) = owner
                    .references
                    .iter()
                    .position(|candidate| std::ptr::eq(candidate.as_ref(), reference))
                {
                    owner.references.swap_remove(index);
                }
            }
            owner.last_error_info.error_message = message;
            owner.last_error_info.engine_reserved = reserved;
            owner.last_error_info.engine_error_code = engine_code;
            owner.last_error_info.error_code = code;
        }
    }
}

impl ScopedFinalizeCallbacks {
    unsafe fn run(self, env: NapiEnv) {
        for wrap in self.wraps {
            if let Some(finalize) = wrap.finalize {
                finalize(env, wrap.data, wrap.hint);
                capture_shutdown_exception(env);
            }
        }
        for record in self.objects.into_iter().chain(self.externals) {
            if let Some(finalize) = record.finalize {
                finalize(env, record.data, record.hint);
                capture_shutdown_exception(env);
            }
        }
        for record in self.noenv {
            if let Some(finalize) = record.finalize {
                finalize(record.data, record.hint);
                capture_shutdown_exception(env);
            }
        }
    }
}

unsafe fn retire_scoped_value(
    env_ref: &mut Env,
    value: NapiValue,
    generation: u64,
    old_payloads: &mut Vec<Value>,
    old_accessors: &mut Vec<Accessor>,
    quickjs_releases: &mut Vec<u64>,
    callbacks: &mut ScopedFinalizeCallbacks,
) {
    let key = value as usize;
    if env_ref.value_generations.get(&key) != Some(&generation) || !env_ref.values.contains(&value)
    {
        return;
    }
    if let Some(index) = env_ref.values.iter().position(|entry| *entry == value) {
        env_ref.values.swap_remove(index);
    }
    env_ref.value_generations.remove(&key);
    // Mark before invoking any callback so reentrant graph/proxy lookups
    // cannot renew an owner whose scoped payload is being retired.
    env_ref.finalized_handles.insert(key);
    if let Some(wrap) = env_ref.wraps.remove(&key) {
        callbacks.wraps.push(wrap);
    }
    if let Some(records) = env_ref.object_finalizers.remove(&key) {
        callbacks.objects.extend(records);
    }
    let mut retained = Vec::new();
    for record in std::mem::take(&mut env_ref.finalizers) {
        if record.backing == value {
            callbacks.externals.push(record);
        } else {
            retained.push(record);
        }
    }
    env_ref.finalizers = retained;
    let mut retained = Vec::new();
    for record in std::mem::take(&mut env_ref.noenv_finalizers) {
        if record.backing == value {
            callbacks.noenv.push(record);
        } else {
            retained.push(record);
        }
    }
    env_ref.noenv_finalizers = retained;
    env_ref.utf16_strings.remove(&key);
    env_ref.utf16_symbols.remove(&key);
    let removed_symbols: HashSet<u64> = env_ref
        .symbols
        .iter()
        .filter_map(|(id, symbol)| (*symbol == value).then_some(*id))
        .collect();
    for id in &removed_symbols {
        env_ref.symbols.remove(id);
        env_ref.strong_symbol_ids.remove(id);
        env_ref.js_original_symbol_ids.remove(id);
        env_ref.js_origin_symbol_ids.remove(id);
        env_ref.js_live_symbol_ids.remove(id);
        env_ref.js_registering_symbol_ids.remove(id);
        env_ref.js_pinned_symbol_ids.remove(id);
    }
    #[cfg(feature = "quickjs")]
    env_ref
        .quickjs_symbol_ids
        .retain(|_, id| !removed_symbols.contains(id));
    env_ref.error_names.remove(&key);
    // These tables describe the Value; they do not independently own it.
    env_ref.graph_wrappers.remove(&key);
    env_ref.host_properties.remove(&key);
    // Remove owner descriptors before callbacks run. Their Arc<QuickJsAccessorRoots>
    // destructors may reenter this Env, so defer the actual Drop until after
    // the entire unreachable set has been detached.
    let accessor_keys: Vec<_> = env_ref
        .accessors
        .keys()
        .filter(|(owner, _)| *owner == key)
        .cloned()
        .collect();
    for accessor_key in accessor_keys {
        if let Some(accessor) = env_ref.accessors.remove(&accessor_key) {
            old_accessors.push(accessor);
        }
    }
    #[cfg(feature = "quickjs")]
    if let Value::QuickJsHandle { handle, .. } = &*value {
        env_ref.quickjs_symbol_ids.remove(handle);
        if env_ref.quickjs_live_values.get(handle) == Some(&value) {
            env_ref.quickjs_live_values.remove(handle);
            quickjs_releases.push(*handle);
        }
    }
    #[cfg(not(feature = "quickjs"))]
    let _ = quickjs_releases;
    env_ref.instances.remove(&key);
    env_ref.live_constructor_returns.remove(&key);
    env_ref.prototypes.remove(&key);
    env_ref.type_tags.remove(&key);
    env_ref
        .property_attributes
        .retain(|(owner, _), _| *owner != key);
    env_ref.property_order.remove(&key);
    env_ref.sealed_objects.remove(&key);
    env_ref.nonextensible_objects.remove(&key);
    env_ref.frozen_objects.remove(&key);
    for reference in &mut env_ref.references {
        if !reference.deleted && reference.count == 0 && reference.value == value {
            reference.value = ptr::null_mut();
        }
    }
    let mut tombstone = Box::from_raw(value);
    old_payloads.push(std::mem::replace(&mut *tombstone, Value::Undefined));
    RETIRED_SCOPE_HANDLE_IDS.with(|ids| {
        ids.borrow_mut().insert(key);
    });
    RETIRED_SCOPE_HANDLE_SLOTS.with(|slots| slots.borrow_mut().push(tombstone));
}

// Revisit values which survived a scope solely because of an Env/Host root.
// Called after reference/graph-owner release, when user finalizers have run.
unsafe fn sweep_pending_scope_values(env: NapiEnv) {
    let _dispatch = ForeignCallbackGuard::new();
    #[cfg(feature = "quickjs")]
    sync_js_origin_symbol_pins(env);
    let Ok(env_ref) = env_mut(env) else {
        return;
    };
    if env_ref.pending_scope_values.is_empty() {
        return;
    }
    let Some(external_roots) = scope_external_roots(env, env_ref.host_managed) else {
        return;
    };
    let values = std::mem::take(&mut env_ref.pending_scope_values);
    let candidates: HashSet<_> = values
        .iter()
        .filter(|(value, generation)| {
            env_ref.value_generations.get(&(*value as usize)) == Some(generation)
                && env_ref.values.contains(value)
        })
        .map(|(value, _)| *value as usize)
        .collect();
    let reachable = env_ref.scope_reachable_values(&candidates, None, &external_roots, true, true);
    let mut old_payloads = Vec::new();
    let mut old_accessors = Vec::new();
    let mut quickjs_releases = Vec::new();
    let mut callbacks = ScopedFinalizeCallbacks::default();
    for (value, generation) in values {
        if reachable.contains(&(value as usize)) {
            env_ref.pending_scope_values.push((value, generation));
        } else {
            retire_scoped_value(
                env_ref,
                value,
                generation,
                &mut old_payloads,
                &mut old_accessors,
                &mut quickjs_releases,
                &mut callbacks,
            );
        }
    }
    let _error_guard = ScopedFinalizerErrorGuard::new(env);
    for payload in old_payloads {
        drop(payload);
        capture_shutdown_exception(env);
    }
    for accessor in old_accessors {
        drop(accessor);
        capture_shutdown_exception(env);
    }
    #[cfg(feature = "quickjs")]
    for handle in quickjs_releases {
        thaw_quickjs::thaw_js_release_handle(handle);
        capture_shutdown_exception(env);
    }
    callbacks.run(env);
    #[cfg(feature = "quickjs")]
    sync_js_origin_symbol_pins(env);
}

// Property mutations may remove the last native edge to a scoped value.
// Run the collector only after the public mutation has published all of its
// descriptor state and released any replaced accessor owner.
struct ScopeMutationSweep {
    env: NapiEnv,
    changed: bool,
}

impl ScopeMutationSweep {
    fn new(env: NapiEnv) -> Self {
        Self {
            env,
            changed: false,
        }
    }
    fn changed(&mut self) {
        self.changed = true;
    }
}

impl Drop for ScopeMutationSweep {
    fn drop(&mut self) {
        if self.changed {
            unsafe {
                sweep_pending_scope_values(self.env);
            }
        }
    }
}

unsafe fn close_handle_scope(
    env: NapiEnv,
    scope: *mut c_void,
    kind: HandleScopeKind,
) -> NapiStatus {
    let _dispatch = ForeignCallbackGuard::new();
    let Ok(env_ref) = env_mut(env) else {
        return NAPI_INVALID_ARG;
    };
    let scope_ptr = scope.cast::<HandleScope>();
    let Some(index) = env_ref
        .handle_scopes
        .iter()
        .position(|candidate| std::ptr::eq(candidate.as_ref(), scope_ptr))
    else {
        return record_status(env, NAPI_INVALID_ARG);
    };
    let scope_ref = &env_ref.handle_scopes[index];
    if scope_ref.env != env as usize {
        return record_status(env, NAPI_INVALID_ARG);
    }
    if scope_ref.closed
        || scope_ref.kind != kind
        || env_ref.active_handle_scopes.last().copied() != Some(scope_ptr)
    {
        return record_status(env, NAPI_HANDLE_SCOPE_MISMATCH);
    }
    let mut values = std::mem::take(&mut env_ref.handle_scopes[index].values);
    values.extend(std::mem::take(&mut env_ref.pending_scope_values));
    let escaped = env_ref.handle_scopes[index].escaped_value.take();
    env_ref.handle_scopes[index].closed = true;
    env_ref.active_handle_scopes.pop();
    let escaped = escaped.and_then(|(value, generation)| {
        (env_ref.value_generations.get(&(value as usize)) == Some(&generation)).then_some(value)
    });
    if env_ref.active_handle_scopes.is_empty() {
        if let Some(value) = escaped {
            env_ref.rooted_scope_escapes.insert(value as usize);
        }
    }
    let candidates: HashSet<_> = values
        .iter()
        .filter(|(value, generation)| {
            env_ref.value_generations.get(&(*value as usize)) == Some(generation)
                && env_ref.values.contains(value)
        })
        .map(|(value, _)| *value as usize)
        .collect();
    #[cfg(feature = "quickjs")]
    sync_js_origin_symbol_pins(env);
    if candidates.is_empty() {
        return NAPI_OK;
    }
    let Some(external_roots) = scope_external_roots(env, env_ref.host_managed) else {
        if let Some(parent) = env_ref.active_handle_scopes.last().copied() {
            (*parent).values.extend(values);
        } else {
            env_ref.pending_scope_values.extend(values);
        }
        return NAPI_OK;
    };
    let reachable =
        env_ref.scope_reachable_values(&candidates, escaped, &external_roots, true, true);
    let mut old_payloads = Vec::new();
    let mut old_accessors = Vec::new();
    let mut quickjs_releases = Vec::new();
    let mut callbacks = ScopedFinalizeCallbacks::default();
    for (value, generation) in values {
        let key = value as usize;
        if env_ref.value_generations.get(&key) != Some(&generation)
            || !env_ref.values.contains(&value)
        {
            continue;
        }
        if reachable.contains(&key) {
            if let Some(parent) = env_ref.active_handle_scopes.last().copied() {
                (*parent).values.push((value, generation));
            } else {
                env_ref.pending_scope_values.push((value, generation));
            }
        } else {
            retire_scoped_value(
                env_ref,
                value,
                generation,
                &mut old_payloads,
                &mut old_accessors,
                &mut quickjs_releases,
                &mut callbacks,
            );
        }
    }
    let _error_guard = ScopedFinalizerErrorGuard::new(env);
    for payload in old_payloads {
        drop(payload);
        capture_shutdown_exception(env);
    }
    for accessor in old_accessors {
        drop(accessor);
        capture_shutdown_exception(env);
    }
    #[cfg(feature = "quickjs")]
    for handle in quickjs_releases {
        thaw_quickjs::thaw_js_release_handle(handle);
        capture_shutdown_exception(env);
    }
    callbacks.run(env);
    #[cfg(feature = "quickjs")]
    sync_js_origin_symbol_pins(env);
    NAPI_OK
}

impl Env {
    unsafe fn disable_dispatch(env: NapiEnv, host_owned: bool) {
        // Cleanup code may release callback data. Keep the Env discoverable,
        // but remove every route which could execute that data again.
        if host_owned {
            HOST.with(|host| {
                let mut host = host.borrow_mut();
                let names = host
                    .exports
                    .iter()
                    .filter(|(_, (owner, _))| *owner == env as usize)
                    .map(|(name, _)| name.clone())
                    .collect::<Vec<_>>();
                for name in names {
                    host.exports.remove(&name);
                    host.functions.remove(&name);
                }
                host.compiled_callbacks
                    .retain(|_, value| !(*env).values.contains(value));
            });
        }
    }

    unsafe fn begin_async_cleanup(env: NapiEnv, host_owned: bool) {
        if (*env).async_cleanup_dispatching {
            return;
        }
        Self::disable_dispatch(env, host_owned);
        (*env).async_cleanup_dispatching = true;
        loop {
            let pending = {
                // Registration and draining use the handle lock. Removal only
                // tombstones handles and never mutates this owner-thread Vec.
                let _handles = async_cleanup_handles()
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                std::mem::take(&mut (*env).async_cleanup_hooks)
            };
            if pending.is_empty() {
                break;
            }
            for handle in pending.into_iter().rev() {
                // Serialize the state transition and counter with removal. A hook
                // may finish on another thread immediately after it starts.
                let callback = {
                    let handles = async_cleanup_handles()
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner());
                    handles
                        .iter()
                        .find(|entry| std::ptr::eq(entry.as_ref(), handle))
                        .and_then(|entry| {
                            if entry
                                .state
                                .compare_exchange(0, 1, Ordering::AcqRel, Ordering::Acquire)
                                .is_ok()
                            {
                                ACTIVE_ASYNC_CLEANUP_HOOKS.fetch_add(1, Ordering::AcqRel);
                                Some((entry.hook, entry.data))
                            } else {
                                None
                            }
                        })
                };
                if let Some((hook, data)) = callback {
                    let _dispatch = ForeignCallbackGuard::new();
                    unsafe { hook(handle, data as *mut c_void) };
                    unsafe { capture_shutdown_exception(env) };
                }
            }
        }
        (*env).async_cleanup_dispatching = false;
    }

    fn async_cleanup_pending(&self) -> bool {
        let env = self as *const Env as usize;
        async_cleanup_handles()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .iter()
            .any(|handle| handle.env == env && handle.state.load(Ordering::Acquire) == 1)
    }
}

impl Env {
    fn invalidate_external_backing(&mut self, backing: NapiValue) {
        if backing.is_null() || !self.values.contains(&backing) {
            return;
        }
        match unsafe { &mut *backing } {
            Value::ExternalBuffer { data, length } => {
                *data = ptr::null_mut();
                *length = 0;
            }
            Value::ExternalSharedArrayBuffer {
                data,
                length,
                retired,
            } => {
                *data = ptr::null_mut();
                *length = 0;
                *retired = true;
            }
            Value::ExternalArrayBuffer {
                data,
                length,
                detached,
            } => {
                *data = ptr::null_mut();
                *length = 0;
                *detached = true;
            }
            Value::External(data) => *data = ptr::null_mut(),
            _ => {}
        }
    }

    // Keep the owning Box in HOST while invoking addon hooks/finalizers. Its
    // NapiEnv address and owner lookups remain valid across reentrant calls.
    unsafe fn finish_cleanup(env: NapiEnv, host_owned: bool) {
        if (*env).finalizing || (*env).finalized {
            return;
        }
        Self::disable_dispatch(env, host_owned);
        (*env).finalizing = true;
        let _dispatch = ForeignCallbackGuard::new();
        for hook in std::mem::take(&mut (*env).cleanup_hooks).into_iter().rev() {
            (hook.hook)(hook.data);
            capture_shutdown_exception(env);
        }
        if let Some(record) = (*env).instance_data.take() {
            if let Some(finalize) = record.finalize {
                (*env).invalidate_external_backing(record.backing);
                finalize(env, record.data, record.hint);
                capture_shutdown_exception(env);
            }
        }
        for record in std::mem::take(&mut (*env).finalizers) {
            if let Some(finalize) = record.finalize {
                (*env).invalidate_external_backing(record.backing);
                finalize(env, record.data, record.hint);
                capture_shutdown_exception(env);
            }
        }
        for record in std::mem::take(&mut (*env).object_finalizers)
            .into_values()
            .flatten()
        {
            if let Some(finalize) = record.finalize {
                (*env).invalidate_external_backing(record.backing);
                finalize(env, record.data, record.hint);
                capture_shutdown_exception(env);
            }
        }
        for record in std::mem::take(&mut (*env).noenv_finalizers) {
            if let Some(finalize) = record.finalize {
                (*env).invalidate_external_backing(record.backing);
                finalize(record.data, record.hint);
                capture_shutdown_exception(env);
            }
        }
        for wrap in std::mem::take(&mut (*env).wraps).into_values() {
            if let Some(finalize) = wrap.finalize {
                finalize(env, wrap.data, wrap.hint);
                capture_shutdown_exception(env);
            }
        }
        while !(*env).posted_finalizers.is_empty() {
            for record in std::mem::take(&mut (*env).posted_finalizers) {
                if let Some(finalize) = record.finalize {
                    (*env).invalidate_external_backing(record.backing);
                    finalize(env, record.data, record.hint);
                    capture_shutdown_exception(env);
                }
            }
        }
        // Other Envs may still hold these stable handles while their own
        // cleanup callbacks run. The external-memory finalizers above may
        // have freed the backing stores, so make retained Values inert before
        // another callback can inspect or serialize a child handle.
        for value in &(*env).values {
            match &mut *(*value) {
                Value::ExternalBuffer { data, length } => {
                    *data = ptr::null_mut();
                    *length = 0;
                }
                Value::ExternalSharedArrayBuffer {
                    data,
                    length,
                    retired,
                } => {
                    *data = ptr::null_mut();
                    *length = 0;
                    *retired = true;
                }
                Value::ExternalArrayBuffer {
                    data,
                    length,
                    detached,
                } => {
                    *data = ptr::null_mut();
                    *length = 0;
                    *detached = true;
                }
                Value::External(data) => *data = ptr::null_mut(),
                _ => {}
            }
        }
        // Keep the owner and Values address-stable until unload; public
        // lookups reject this finalized owner below.
        (*env).finalized = true;
        (*env).finalizing = false;
        // Accessor callbacks can own independent JS handles. Invalidate the
        // native descriptor table before their final release can reenter.
        let accessors = std::mem::take(&mut (*env).accessors);
        drop(accessors);
        // A QuickJS release can invoke a JS finalizer and reenter N-API.
        // Invalidate this Env's cache first, then release outside any HOST
        // borrow or reference into an Env value. The Box remains pinned.
        #[cfg(feature = "quickjs")]
        let quickjs_handles = std::mem::take(&mut (*env).quickjs_live_values)
            .into_keys()
            .collect::<Vec<_>>();
        #[cfg(feature = "quickjs")]
        for handle in quickjs_handles {
            thaw_quickjs::thaw_js_release_handle(handle);
        }
    }
}

impl Drop for Env {
    fn drop(&mut self) {
        forget_standalone_graph_owner(self.graph_owner_id);
        if !self.finalized {
            // Direct internal Env owners still use the synchronous Drop path.
            // Host-owned Envs explicitly start hooks while their Box is pinned.
            unsafe {
                Self::begin_async_cleanup(self, false);
                Self::finish_cleanup(self, false);
            }
        }
        for value in std::mem::take(&mut self.values) {
            unsafe { drop(Box::from_raw(value)) };
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
    qualified_packages: HashSet<String>,
    compiled_callbacks: HashMap<(usize, usize, usize, bool), NapiValue>,
    libraries: Vec<*mut c_void>,
    // Each thread-local Host owns its own loop; Worker's callback context must
    // never run callbacks from the process-global uv_default_loop.
    owned_uv_loop: Option<usize>,
    // Recorded by the process-main loader while its libuv handle is retained.
    main_default_uv_loop: Option<usize>,
    embedded_files: Vec<std::fs::File>,
    // Addons retain `napi_env` pointers, so moving an Env during Vec growth
    // would invalidate foreign pointers. The Box provides stable addresses.
    #[allow(clippy::vec_box)]
    module_envs: Vec<Box<Env>>,
    // Async addons retain the `napi_env`; boxes keep those addresses stable
    // while unrelated calls grow the pending vector.
    #[allow(clippy::vec_box)]
    pending_call_envs: Vec<Box<Env>>,
    unloading: bool,
    shutdown_errors: VecDeque<String>,
    last_error: String,
}

impl Host {
    /// The registered key for `name`: the exact key, or (for a bare name) the single
    /// package-qualified `pkg::name` that `present` accepts. Ambiguous names resolve to nothing.
    fn qualified_key(&self, name: &str, present: impl Fn(&str) -> bool) -> Option<String> {
        if present(name) {
            return Some(name.to_string());
        }
        if name.contains("::") {
            return None;
        }
        let mut found = None;
        for package in &self.qualified_packages {
            let key = format!("{package}::{name}");
            if present(&key) {
                if found.is_some() {
                    return None;
                }
                found = Some(key);
            }
        }
        found
    }

    fn new() -> Self {
        Self {
            functions: HashMap::new(),
            exports: HashMap::new(),
            qualified_packages: HashSet::new(),
            compiled_callbacks: HashMap::new(),
            libraries: Vec::new(),
            owned_uv_loop: None,
            main_default_uv_loop: None,
            embedded_files: Vec::new(),
            module_envs: Vec::new(),
            pending_call_envs: Vec::new(),
            unloading: false,
            shutdown_errors: VecDeque::new(),
            last_error: String::new(),
        }
    }

    fn has_active_async_work(&self) -> bool {
        self.module_envs
            .iter()
            .chain(&self.pending_call_envs)
            .flat_map(|env| &env.async_works)
            .any(|work| {
                matches!(
                    work.state.load(Ordering::Acquire),
                    ASYNC_QUEUED | ASYNC_EXECUTING | ASYNC_COMPLETE_PENDING
                )
            })
    }

    fn has_active_cleanup(&self) -> bool {
        self.module_envs
            .iter()
            .chain(&self.pending_call_envs)
            .any(|env| {
                env.async_cleanup_dispatching
                    || env.async_cleanup_pending()
                    || (env.shutdown_requested && !env.async_cleanup_hooks.is_empty())
            })
    }
}

fn host_threadsafe_state(referenced_only: bool) -> bool {
    let owner = std::thread::current().id();
    threadsafe_functions()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .iter()
        .filter(|function| function.creator == owner)
        .any(|function| {
            if referenced_only {
                function.referenced.load(Ordering::Acquire)
            } else {
                !function
                    .state
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .live_released
            }
        })
}

fn host_has_unfinalized_threadsafe() -> bool {
    let owner = std::thread::current().id();
    threadsafe_functions()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .iter()
        .filter(|function| function.creator == owner)
        .any(|function| {
            !function
                .state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .finalizer_completed
        })
}

impl Drop for Host {
    fn drop(&mut self) {
        // Host lives in a thread-local slot, so this runs from the TLS
        // destructor: addon finalizers and cleanup hooks re-enter HOST through
        // N-API calls and `LocalKey::with` panics once the slot is destroyed
        // (a panic inside an extern "C" finalizer aborts the process). Running
        // user code here is therefore never valid; anything not yet finalized
        // is left for the OS to reclaim at process exit.
        // ponytail: leaked, not finalized; an explicit pre-TLS-teardown pass
        // (run on thread exit before the slot dies) would also run finalizers.
        for env in self
            .module_envs
            .drain(..)
            .chain(self.pending_call_envs.drain(..))
        {
            if env.finalized {
                drop(env);
            } else {
                Box::leak(env);
            }
        }
    }
}

// Host vectors own each Box throughout hook dispatch. Do not hold a HOST
// borrow or a Rust Env reference across addon callbacks or finalizers.
fn retire_owned_envs() {
    if ForeignCallbackGuard::active() {
        return;
    }
    loop {
        if HOST.with(|host| host.borrow().has_active_async_work()) || host_threadsafe_state(false) {
            return;
        }
        // Keep boxes in their original Host vectors while callback code runs.
        let retiring = HOST.with(|host| {
            let host = host.borrow();
            host.pending_call_envs
                .iter()
                .chain(host.module_envs.iter().filter(|env| env.shutdown_requested))
                .map(|env| (&**env as *const Env).cast_mut())
                .collect::<Vec<_>>()
        });
        for env in &retiring {
            unsafe { Env::disable_dispatch(*env, true) };
        }
        for env in &retiring {
            if unsafe {
                !(*(*env)).async_cleanup_dispatching && !(*(*env)).async_cleanup_hooks.is_empty()
            } {
                unsafe { Env::begin_async_cleanup(*env, true) };
            }
        }
        if HOST.with(|host| {
            let host = host.borrow();
            host.has_active_async_work() || host.has_active_cleanup()
        }) || host_threadsafe_state(false)
            || !registered_uv_loops().is_empty()
            || unsafe { uv_handles_open() }
        {
            return;
        }
        // External libuv callbacks may still hold Env, addon function data,
        // and QuickJS context. Cleanup hooks must close/unregister their loops
        // before any finalizer can reclaim those resources.
        // A finalizer may register a hook or queue work on another Env. Do
        // one owner at a time, then restart the global hook/work barrier.
        let next = retiring.into_iter().find(|env| unsafe {
            !(*(*env)).finalized
                && !(*(*env)).async_cleanup_dispatching
                && !(*(*env)).finalizing
                && !(*(*env)).async_cleanup_pending()
                && (*(*env)).native_graph_pins == 0
                && (*(*env)).async_cleanup_hooks.is_empty()
        });
        let Some(env) = next else { break };
        unsafe { Env::finish_cleanup(env, true) };
    }
}

thread_local! {
    static HOST: RefCell<Host> = RefCell::new(Host::new());
    static PENDING_MODULE: RefCell<Option<NapiModule>> = const { RefCell::new(None) };
}

unsafe fn capture_shutdown_exception(env: NapiEnv) {
    // Describing a named N-API error consults HOST. During thread-local Host
    // destruction there is no safe reporter; do not reenter that TLS slot.
    if HOST.try_with(|_| ()).is_err() {
        return;
    }
    if let Err(error) = take_env_exception(env) {
        let _ = HOST.try_with(|host| {
            let mut host = host.borrow_mut();
            host.last_error = error.clone();
            host.shutdown_errors.push_back(error);
        });
    }
}

#[no_mangle]
pub extern "C" fn thaw_napi_report_shutdown_errors() -> usize {
    let errors = HOST.with(|host| std::mem::take(&mut host.borrow_mut().shutdown_errors));
    let count = errors.len();
    for error in errors {
        eprintln!("thaw-napi: uncaught shutdown exception: {error}");
    }
    count
}

/// Returns the next callback, cleanup hook, or finalizer error in occurrence
/// order. The caller owns the string and must call thaw_cstring_destroy.
/// Main and Worker can pass it to their uncaught reporter before Env release.
#[no_mangle]
pub extern "C" fn thaw_napi_take_shutdown_error() -> *mut c_char {
    HOST.with(|host| host.borrow_mut().shutdown_errors.pop_front())
        .map(thaw_arena::owned_string)
        .unwrap_or(ptr::null_mut())
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
    Ok(thaw_arena::NativeStr::from_ptr(ptr)
        .to_string_lossy()
        .into_owned())
}

unsafe fn env_mut<'a>(env: NapiEnv) -> Result<&'a mut Env, NapiStatus> {
    match env.as_mut() {
        Some(env) if !env.finalized => Ok(env),
        _ => Err(NAPI_INVALID_ARG),
    }
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
    if value.is_null()
        || RETIRED_SCOPE_HANDLE_IDS.with(|ids| ids.borrow().contains(&(value as usize)))
    {
        return Err(NAPI_INVALID_ARG);
    }
    value.as_ref().ok_or(NAPI_INVALID_ARG)
}

unsafe fn value_belongs_to_environment(env: NapiEnv, value: NapiValue) -> bool {
    if value.is_null() || env.as_ref().is_none_or(|env| env.finalized) {
        record_status(env, NAPI_INVALID_ARG);
        return false;
    }
    let belongs = env
        .as_ref()
        .is_some_and(|env| !env.finalized && env.values.contains(&value))
        || HOST.with(|host| {
            host.borrow()
                .module_envs
                .iter()
                .any(|module_env| !module_env.finalized && module_env.values.contains(&value))
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
            | Value::QuickJsHandle {
                object_like: true,
                ..
            }
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
