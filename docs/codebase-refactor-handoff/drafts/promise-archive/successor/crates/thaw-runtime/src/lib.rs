//! A minimal AWS Lambda custom runtime client. This is what turns a
//! compiled Thaw program with a `function handler(event: string): string`
//! into an actual deployable Lambda function: it polls the Runtime API for
//! the next invocation, calls the handler with the raw event JSON, and
//! posts the handler's raw string result back.
//!
//! Deliberately a hand-rolled blocking HTTP/1.1 client over `TcpStream`
//! rather than hyper/tokio: the Runtime API is a fixed, local-only, plain
//! HTTP endpoint with no concurrency to speak of (one invocation at a time
//! per execution environment), and pulling in a full async runtime here
//! would fight the entire point of Thaw -- small binaries, fast cold
//! start. User-code `fetch()` uses the separate fd-driven HTTP/TLS state
//! machine later in this crate; the blocking client here remains specific to
//! the Lambda Runtime API.
//!
//! Known limitations, acceptable for what this talks to: no TLS and one
//! request per TCP connection.

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, VecDeque};
use std::ffi::CString;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream, ToSocketAddrs};
use std::os::raw::c_char;
use std::os::unix::io::RawFd;
use std::sync::{Arc, Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant};
use thaw_arena::NativeStr as CStr;

use rustls::pki_types::ServerName;
use rustls::{ClientConfig, ClientConnection, RootCertStore};

// `arrays.rs`/`maps.rs` call several `Json` helpers (`thaw_json_strict_
// equal`, `thaw_json_typeof`, ...) that are `extern "C"` declarations
// resolved at link time against thaw-std's own staticlib (see their own
// doc comments -- not a real Cargo dependency, so `thaw-std` only exists
// as a `[dev-dependencies]` entry here, the same way thaw-std's own
// `http.rs` mirrors this back with `#[cfg(test)] use thaw_runtime as
// _;`). Without this marker import, `cargo test -p thaw-runtime` in
// isolation never actually links thaw-std's rlib in (nothing here
// references it by path), and those externs are left unresolved.
#[cfg(test)]
use thaw_std as _;

include!("runtime/native_values/numbers.rs");
include!("runtime/native_values/strings.rs");
include!("runtime/native_values/arrays.rs");
include!("runtime/native_values/array_provenance.rs");
include!("runtime/native_values/regex.rs");
include!("runtime/native_values/template_strings.rs");
include!("runtime/native_values/date.rs");
include!("runtime/native_values/temporal.rs");
include!("runtime/native_values/maps.rs");
include!("runtime/native_values/errors.rs");
include!("runtime/native_values/objects.rs");
include!("runtime/native_values/bytes.rs");
include!("runtime/abi.rs");

/// Prints an uncaught compiled exception before the process exits
/// unsuccessfully.
///
/// `error` is the exception channel's own raw tagged string (see
/// `errors.rs`'s doc comment: `\u{1}<Name>\u{1}<message>` or an
/// untagged plain string), never printed as-is before now -- the
/// control-byte markers aren't visible in a terminal, so `\u{1}Error
/// \u{1}boom` rendered as the illegible "Errorboom" (missing real
/// Node's own mandatory "Name: message" separator entirely). Routed
/// through `thaw_error_to_string` (`.toString()`/`String(error)`'s own
/// intrinsic) rather than `thaw_error_stack`: real Node prints a bare
/// `throw "text"`'s uncaught value completely unchanged (no "Error:"
/// prefix at all, confirmed against real Node -- it's not an `Error`),
/// and `thaw_error_to_string` already gives exactly that for an
/// untagged string, while still rendering a real tagged `Error`
/// correctly as "Name: message". `thaw_error_stack` always applies the
/// "defaults to `Error`" convention even to an untagged value (correct
/// for `.stack`, which conceptually only exists on an Error-like
/// object being coerced) -- using it here would have "fixed" the
/// tagged case while silently breaking the untagged one, caught by
/// checking a plain string throw against real Node before settling on
/// this function instead.
///
/// # Safety
/// `error` must be null or point to a valid NUL-terminated string.
#[no_mangle]
pub unsafe extern "C" fn thaw_runtime_report_uncaught(error: *const c_char) {
    if !error.is_null() {
        let rendered = unsafe { thaw_error_to_string(error) };
        let text = if rendered.is_null() {
            unsafe { CStr::from_ptr(error) }.to_string_lossy()
        } else {
            unsafe { CStr::from_ptr(rendered) }.to_string_lossy()
        };
        eprintln!("Uncaught: {text}");
    }
}

/// Terminal native-only sink for the exact deferred exception packet. The
/// packet remains owned by the generated cleanup driver for this call; this
/// routine reads its typed channels without treating an opaque Object/Json
/// pointer as text. NativeStr bytes are written directly, including NUL.
/// A nonzero result means an original value reached this terminal sink, not
/// that every byte was accepted by stderr or that an owner was consumed. An
/// I/O failure still requires a fatal exit: retrying a partially written
/// packet could duplicate output, while its ownership must be retired once.
/// The generated driver retires the packet and its owner separately.
#[no_mangle]
pub unsafe extern "C" fn thaw_runtime_report_uncaught_exact_packet(packet: *const u8) -> u8 {
    use std::io::Write;
    if packet.is_null() { return 0; }
    let pointer = |offset: usize| unsafe { packet.add(offset).cast::<*const u8>().read_unaligned() };
    let original = pointer(8);
    let native_text = pointer(16);
    let object = pointer(24);
    let source = pointer(96).cast::<ThawPromise>();
    let tag = unsafe { packet.add(48).cast::<u64>().read_unaligned() };
    if original.is_null() && !matches!(tag, 5 | 6) { return 0; }
    let mut stderr = std::io::stderr().lock();
    let _ = stderr.write_all(b"Uncaught: ");
    match tag {
        1 => {
            let value = unsafe { packet.add(56).cast::<f64>().read_unaligned() };
            let rendered = javascript_number_string(value);
            let _ = stderr.write_all(rendered.as_bytes());
        }
        2 => {
            let value = unsafe { packet.add(64).cast::<i64>().read_unaligned() };
            let _ = write!(stderr, "{value}");
        }
        3 => {
            let value = unsafe { packet.add(72).cast::<u8>().read_unaligned() };
            let _ = stderr.write_all(if value == 0 { b"false" } else { b"true" });
        }
        5 => { let _ = stderr.write_all(b"undefined"); }
        6 => { let _ = stderr.write_all(b"null"); }
        7 => { let _ = write!(stderr, "Json@{object:p}"); }
        4 if !source.is_null() => {
            if let Some(bytes) = unsafe { &*source }.rejection_text.as_ref() {
                let _ = stderr.write_all(bytes);
            } else {
                let _ = write!(stderr, "String@{original:p}");
            }
        }
        _ if original == native_text => {
            let bytes = unsafe { thaw_arena::NativeStr::from_ptr(native_text.cast()) }.to_bytes();
            let _ = stderr.write_all(bytes);
        }
        _ => { let _ = write!(stderr, "Object@{original:p}"); }
    }
    let _ = stderr.write_all(b"\n");
    1
}

/// Copies a terminal exception into owned report text before a listener can
/// reenter generated code. The public Promise rejection ABI permits opaque
/// pointers, so only an exact producer-supplied native-text pointer is read.
/// Typed primitive payloads remain printable even when that text is absent.
/// The caller must destroy the returned string exactly once.
#[no_mangle]
pub unsafe extern "C" fn thaw_runtime_exception_report_text(
    error: *const c_char,
    native_text: *const c_char,
    tag: u64,
    f64_value: f64,
    i64_value: i64,
    bool_value: bool,
) -> *mut c_char {
    if error.is_null() {
        return std::ptr::null_mut();
    }
    let text: Vec<u8> = if error == native_text {
        unsafe { CStr::from_ptr(error) }.to_bytes().to_vec()
    } else {
        match tag {
            1 => javascript_number_string(f64_value).into_bytes(),
            2 => i64_value.to_string().into_bytes(),
            3 => bool_value.to_string().into_bytes(),
            5 => b"undefined".to_vec(),
            6 => b"null".to_vec(),
            4 => b"Uncaught opaque string exception".to_vec(),
            _ => b"Uncaught opaque exception".to_vec(),
        }
    };
    thaw_arena::owned_string(text)
}

struct PromiseSubscription {
    resume: PromiseResumeFn,
    frame: *mut u8,
    // Runtime-owned join frames need to retire when purge drops a queued or
    // still-pending subscription without invoking its normal resume path.
    cancel: Option<extern "C" fn(*mut u8)>,
    delivered: bool,
    // The subscriber owns one native Promise share until its callback returns.
    // This also keeps the result and rejection payload live during delivery.
    source: *mut ThawPromise,
    // Generated split-async frames carry a raw native completion pointer.
    // Their arena root cannot own this external Promise allocation; retain
    // it independently while the frame is suspended or in its resume call.
    completion: *mut ThawPromise,
    // Reserved before subscription publication. A failed exact share can
    // move the subscription's source claim into this packet without a new
    // allocation or a borrowed Promise pointer escaping `poll_one`.
    terminal_packet: *mut u8,
    terminal_source: *mut ThawPromise,
    _terminal_packet_root: Option<thaw_arena::ArenaRoot>,
    terminal_cause_packet: *mut u8,
    _terminal_cause_root: Option<thaw_arena::ArenaRoot>,
    // A suspended arena frame is otherwise held only by this Rust queue or
    // the source Promise's subscriber Vec, which the arena tracer cannot
    // inspect. Keep it reachable until the resume callback returns.
    _frame_root: thaw_arena::ArenaRoot,
}

impl Drop for PromiseSubscription {
    fn drop(&mut self) {
        if !self.delivered {
            if let Some(cancel) = self.cancel { cancel(self.frame) }
        }
        if !self.completion.is_null() {
            unsafe { thaw_promise_release(self.completion) };
        }
        unsafe { thaw_promise_release(self.source) };
    }
}

type PromiseArrayElement = unsafe extern "C" fn(usize, *const u8) -> *mut ThawPromise;

#[derive(Clone, Copy)]
struct PromiseArrayLayout {
    handle: usize,
    stride: usize,
    element: Option<PromiseArrayElement>,
}

thread_local! {
    // Compiler-owned Promise capture cells are arena allocations, but a
    // closure may share one cell with its outer variable and sibling
    // closures. The reset hook cannot read a reclaimed cell; keep its last
    // explicit ticket here and mark it deferred when the cell dies.
    static PROMISE_CAPTURE_CREATOR_CELLS: RefCell<Vec<(usize, u64, bool)>> =
        const { RefCell::new(Vec::new()) };
    static PROMISE_CAPTURE_CREATOR_DRIVER: Cell<Option<extern "C" fn() -> bool>> =
        const { Cell::new(None) };
    static READY_CONTINUATIONS: RefCell<VecDeque<(PromiseSubscription, *const u8)>> =
        const { RefCell::new(VecDeque::new()) };
    // Only the currently executing resume may transfer its independent
    // source share to a completion after exact Json sharing fails.
    static DELIVERING_SUBSCRIPTION: Cell<*mut PromiseSubscription> =
        const { Cell::new(std::ptr::null_mut()) };
    static PURGING_ASYNC_STATE: Cell<bool> = const { Cell::new(false) };
    static TIMERS: RefCell<Vec<PromiseTimer>> = const { RefCell::new(Vec::new()) };
    static FD_WAITS: RefCell<Vec<PromiseFdWait>> = const { RefCell::new(Vec::new()) };
    static FD_WATCHERS: RefCell<Vec<FdWatcher>> = const { RefCell::new(Vec::new()) };
    static ACTIVE_PROMISE_JOINS: Cell<usize> = const { Cell::new(0) };
    static ACTIVE_PROMISES: RefCell<Vec<*mut ThawPromise>> = const { RefCell::new(Vec::new()) };
    // The allocation's initial share lasts through its request boundary.
    // Consuming subscribers/wrappers take their own shares; persistent
    // fields take another and survive this boundary while their arena owner
    // remains rooted.
    // (identity, arena reset epoch at creation). A reset hook may reenter
    // user code and create a Promise while the previous request is retiring.
    static PROMISE_BASES: RefCell<Vec<(u64, u128)>> = const { RefCell::new(Vec::new()) };
    // Generated module Promise slots are stable outside the arena. At each
    // request boundary their current pointer is reconciled before the
    // request's temporary creator/base shares are released.
    static PROMISE_GLOBAL_SLOTS: RefCell<Vec<(*const *mut ThawPromise, *mut ThawPromise)>> =
        const { RefCell::new(Vec::new()) };
    // Stable native Array handles can replace or mutate their raw buffers.
    // Scan the current buffer at each reset before temporary Promise bases
    // are retired, so in-place and copying mutators share one owner path.
    static PROMISE_ARRAY_HANDLES: RefCell<Vec<PromiseArrayLayout>> = const { RefCell::new(Vec::new()) };
    #[cfg(test)]
    static PROMISE_ARRAY_FORCE_STAGE_FAILURE: Cell<bool> = const { Cell::new(false) };
    #[cfg(test)]
    static PROMISE_ARRAY_FORCE_RETIRE_RESERVE_FAILURE: Cell<bool> = const { Cell::new(false) };
    // If array staging fails, a dead field can be the only share keeping a
    // Promise copied into a live array alive. The later field reset hook must
    // defer its cleanup until a complete array reconciliation succeeds.
    static PROMISE_ARRAY_STAGE_FAILURES: Cell<u128> = const { Cell::new(0) };
    static PROMISE_ARRAY_FIELDS_DEFERRED: Cell<bool> = const { Cell::new(false) };
    // One temporary share per previously-live Promise observed by generated
    // code during this request. Repeated reads are deduplicated by identity.
    static PROMISE_REQUEST_PINS: RefCell<HashMap<u64, *mut ThawPromise>> =
        RefCell::new(HashMap::new());
    // Replaced fields may still have been borrowed into another native
    // aggregate before that aggregate's next ownership reconciliation.
    static PROMISE_RETIRED_FIELDS: RefCell<Vec<*mut ThawPromise>> =
        const { RefCell::new(Vec::new()) };
    // One independently retained share per arena field. The arena reset
    // callback releases these only when the physical owner is reclaimed.
    static PROMISE_FIELDS: RefCell<HashMap<(usize, usize), *mut ThawPromise>> =
        RefCell::new(HashMap::new());
    static UNHANDLED_REPORTER: Cell<Option<PromiseUnhandledFn>> = const { Cell::new(None) };
    static UNHANDLED_REPORTER_RESULT: Cell<Option<PromiseUnhandledResultFn>> = const { Cell::new(None) };
    static UNHANDLED_REPORTER_TEXT_RESULT: Cell<Option<PromiseUnhandledResultFn>> = const { Cell::new(None) };
    static REJECTION_HANDLED_REPORTER: Cell<Option<PromiseRejectionHandledFn>> = const { Cell::new(None) };
    static REJECTION_HANDLED_REPORTER_RESULT: Cell<Option<PromiseRejectionHandledResultFn>> = const { Cell::new(None) };
    static PENDING_REJECTION_HANDLED: Cell<usize> = const { Cell::new(0) };
    static UNHANDLED_FAILURE: Cell<bool> = const { Cell::new(false) };
    static PROMISE_REPORT_ACTIVITY: Cell<usize> = const { Cell::new(0) };
    // The current Lambda invocation's wall-clock deadline, derived from the
    // Runtime API's `Lambda-Runtime-Deadline-Ms` header, paired with the
    // countdown it started from (kept only to phrase the timeout message).
    // `None` outside a Lambda invocation, or when the header was missing.
    static INVOCATION_DEADLINE: Cell<Option<(Instant, Duration)>> = const { Cell::new(None) };
}
// Native creator tickets may be carried through compiled callbacks. A global
// incarnation number prevents a ticket from one runtime thread naming an
// unrelated Promise allocated on another thread.
static NEXT_PROMISE_ID: std::sync::atomic::AtomicU64 =
    std::sync::atomic::AtomicU64::new(1);

static INVOCATION_TIMEOUT_ERROR: &[u8] = b"Task timed out\0";

/// Sets (or clears, when `None`) the current Lambda invocation's timeout
/// countdown. `handle_one_invocation` calls this once per invocation with the
/// remaining milliseconds derived from the Runtime API's deadline header.
fn set_invocation_deadline(remaining_ms: Option<u64>) {
    INVOCATION_DEADLINE.with(|cell| {
        cell.set(remaining_ms.map(|ms| {
            let budget = Duration::from_millis(ms);
            (Instant::now() + budget, budget)
        }))
    });
}

/// `None` when no invocation deadline is active. `Some(Duration::ZERO)` once
/// it has passed, so callers can tell "expired" apart from "no deadline" and
/// still use the value to cap how long they block waiting for other events.
fn invocation_deadline_remaining() -> Option<Duration> {
    INVOCATION_DEADLINE
        .with(Cell::get)
        .map(|(deadline, _)| deadline.saturating_duration_since(Instant::now()))
}

/// Force-settles `promise` with a timeout error once its invocation's
/// deadline has passed, mirroring the real Runtime API's own behavior for a
/// handler that runs past its configured timeout. Safe to call on an
/// already-settled promise -- the Promise ABI accepts only the first settle.
fn reject_for_invocation_deadline(promise: *mut ThawPromise) {
    let seconds = INVOCATION_DEADLINE
        .with(Cell::get)
        .map_or(0.0, |(_, budget)| budget.as_secs_f64());
    let message = format!("Task timed out after {seconds:.2} seconds");
    let error = arena_c_string(&message).unwrap_or(INVOCATION_TIMEOUT_ERROR.as_ptr());
    reject_native_text(promise, error);
}

/// Clears every thread-local queue that could still hold a pointer into the
/// request arena `InvocationArenaReset` is about to reset: an abandoned
/// coroutine (deadline timeout, or genuine deadlock with no possible
/// progress) can leave its own pending timers/fd-waits registered, and a
/// later invocation's event loop must never resume them into reused memory.
/// `FD_WATCHERS` is deliberately untouched -- those back long-lived
/// subscriptions (such as N-API filesystem watchers) that outlive one
/// invocation by design.
struct PurgeAsyncGuard(bool);

impl Drop for PurgeAsyncGuard {
    fn drop(&mut self) {
        PURGING_ASYNC_STATE.with(|purging| purging.set(self.0));
    }
}

fn purge_pending_async_state() {
    let _guard = PurgeAsyncGuard(PURGING_ASYNC_STATE.with(|purging| purging.replace(true)));
    // Abandoned aggregate callback Boxes may outlive cleared queues. Release
    // their temporary pins and make any later callback ignore arena payloads.
    cancel_pending_aggregate_states();
    uncount_pending_promise_all_states_for_invocation();
    // Drop outside the queue borrow: cancelling a join may release a Host
    // payload whose callback reenters this runtime.
    let abandoned_ready = READY_CONTINUATIONS.with(|queue| std::mem::take(&mut *queue.borrow_mut()));
    drop(abandoned_ready);
    // A pending Promise keeps subscriptions in its own Vec. Each of those
    // subscriptions owns a source share, so leaving them installed creates a
    // self-retaining cycle across request boundaries. Use identity snapshots
    // because cancelling one subscription may retire another Promise.
    let pending = ACTIVE_PROMISES.with(|active| active.borrow().iter()
        .map(|promise| unsafe { (**promise).identity }).collect::<Vec<_>>());
    for identity in pending {
        let promise = thaw_promise_from_identity(identity);
        if promise.is_null() { continue; }
        let abandoned = unsafe { std::mem::take(&mut (*promise).subscribers) };
        drop(abandoned);
    }
    TIMERS.with(|timers| timers.borrow_mut().clear());
    FD_WAITS.with(|waits| waits.borrow_mut().clear());
    ACTIVE_PROMISE_JOINS.with(|count| count.set(0));
    PROMISE_REPORT_ACTIVITY.with(|activity| activity.set(0));
    set_invocation_deadline(None);
}

struct PromiseTimer {
    deadline: Instant,
    promise: *mut ThawPromise,
}

struct PromiseFdWait {
    fd: libc::c_int,
    interests: u8,
    promise: *mut ThawPromise,
    deadline: Option<Instant>,
}

#[derive(Clone)]
struct FdWatcher {
    id: u64,
    fd: libc::c_int,
    interests: u8,
    callback: FdWatcherFn,
    context: *mut u8,
    _root: thaw_arena::ArenaRoot,
}

static NEXT_FD_WATCHER_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

static INVALID_FD_ERROR: &[u8] = b"invalid file descriptor\0";
static FD_TIMEOUT_ERROR: &[u8] = b"file descriptor wait timed out\0";
static ASYNC_PURGE_ERROR: &[u8] = b"Task is ending\0";
static PROMISE_ALL_INVALID_ERROR: &[u8] = b"Promise.all received an invalid promise\0";
static PROMISE_RACE_INVALID_ERROR: &[u8] = b"Promise.race received an invalid promise\0";
static PROMISE_CYCLE_ERROR: &[u8] = b"Chaining cycle detected for promise\0";
static PROMISE_ANY_REJECTED_ERROR: &[u8] = b"\x01AggregateError\x01All promises were rejected\0";
static PROMISE_ANY_ALLOCATION_ERROR: &[u8] = b"Promise.any could not allocate rejection reasons\0";
static PROMISE_SETTLED_FULFILLED: &[u8] = b"fulfilled\0";
static PROMISE_SETTLED_REJECTED: &[u8] = b"rejected\0";
static PROMISE_SETTLED_EMPTY_REASON: &[u8] = b"\0";

fn fd_poll_timeout_ms(timeout: Option<Duration>) -> i32 {
    match timeout {
        Some(duration) => duration.as_nanos().div_ceil(1_000_000).min(i32::MAX as u128) as i32,
        None => -1,
    }
}

fn poll_fd_waits(timeout: Option<Duration>) -> usize {
    let wait_count = FD_WAITS.with(|waits| waits.borrow().len());
    let mut pollfds = FD_WAITS.with(|waits| {
        waits
            .borrow()
            .iter()
            .map(|wait| libc::pollfd {
                fd: wait.fd,
                events: (if wait.interests & THAW_FD_READABLE != 0 {
                    libc::POLLIN
                } else {
                    0
                }) | (if wait.interests & THAW_FD_WRITABLE != 0 {
                    libc::POLLOUT
                } else {
                    0
                }),
                revents: 0,
            })
            .collect::<Vec<_>>()
    });
    FD_WATCHERS.with(|watchers| {
        pollfds.extend(watchers.borrow().iter().map(|watcher| libc::pollfd {
            fd: watcher.fd,
            events: poll_events(watcher.interests),
            revents: 0,
        }));
    });
    if pollfds.is_empty() {
        return 0;
    }
    let fd_delay = FD_WAITS.with(|waits| {
        let now = Instant::now();
        waits
            .borrow()
            .iter()
            .filter_map(|wait| wait.deadline)
            .map(|deadline| deadline.saturating_duration_since(now))
            .min()
    });
    let timeout = match (timeout, fd_delay) {
        (Some(left), Some(right)) => Some(left.min(right)),
        (Some(delay), None) | (None, Some(delay)) => Some(delay),
        (None, None) => None,
    };
    let timeout_ms = fd_poll_timeout_ms(timeout);
    let ready = unsafe {
        libc::poll(
            pollfds.as_mut_ptr(),
            pollfds.len() as libc::nfds_t,
            timeout_ms,
        )
    };
    if ready < 0 {
        return 0;
    }

    let completed = FD_WAITS.with(|waits| {
        let mut waits = waits.borrow_mut();
        let mut completed = Vec::new();
        let now = Instant::now();
        for index in (0..wait_count).rev() {
            let timed_out = waits[index]
                .deadline
                .is_some_and(|deadline| deadline <= now);
            if pollfds[index].revents != 0 || timed_out {
                completed.push((
                    waits.swap_remove(index).promise,
                    pollfds[index].revents,
                    timed_out,
                ));
            }
        }
        completed
    });
    let count = completed.len();
    for (promise, events, timed_out) in completed {
        if timed_out && events == 0 {
            reject_native_text(promise, FD_TIMEOUT_ERROR.as_ptr());
        } else if events & libc::POLLNVAL != 0 {
            reject_native_text(promise, INVALID_FD_ERROR.as_ptr());
        } else {
            thaw_promise_resolve(promise, std::ptr::dangling::<u8>());
        }
    }
    let ready_watchers = FD_WATCHERS.with(|watchers| {
        watchers
            .borrow()
            .iter()
            .zip(&pollfds[wait_count..])
            .filter(|(_, pollfd)| pollfd.revents != 0)
            .map(|(watcher, pollfd)| (watcher.clone(), pollfd.revents))
            .collect::<Vec<_>>()
    });
    let mut watcher_count = 0;
    for (watcher, events) in ready_watchers {
        let registered = FD_WATCHERS.with(|watchers| {
            watchers.borrow().iter().any(|current| current.id == watcher.id)
        });
        if registered {
            (watcher.callback)(watcher.context, events);
            watcher_count += 1;
        }
    }
    count + watcher_count
}

fn has_fd_waits() -> bool {
    FD_WAITS.with(|waits| !waits.borrow().is_empty())
        || FD_WATCHERS.with(|watchers| !watchers.borrow().is_empty())
}

fn poll_events(interests: u8) -> i16 {
    (if interests & THAW_FD_READABLE != 0 {
        libc::POLLIN
    } else {
        0
    }) | (if interests & THAW_FD_WRITABLE != 0 {
        libc::POLLOUT
    } else {
        0
    })
}

#[no_mangle]
pub extern "C" fn thaw_runtime_watch_fd(
    fd: libc::c_int,
    interests: u8,
    callback: FdWatcherFn,
    context: *mut u8,
) -> u64 {
    if fd < 0 || interests == 0 || interests & !(THAW_FD_READABLE | THAW_FD_WRITABLE) != 0 {
        return 0;
    }
    let id = NEXT_FD_WATCHER_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    FD_WATCHERS.with(|watchers| {
        watchers.borrow_mut().push(FdWatcher {
            id,
            fd,
            interests,
            callback,
            context,
            _root: thaw_arena::ArenaRoot::new(context as usize),
        });
    });
    id
}

#[no_mangle]
pub extern "C" fn thaw_runtime_unwatch_fd(id: u64) -> bool {
    FD_WATCHERS.with(|watchers| {
        let mut watchers = watchers.borrow_mut();
        let Some(index) = watchers.iter().position(|watcher| watcher.id == id) else {
            return false;
        };
        watchers.swap_remove(index);
        true
    })
}

/// Blocks until the next timer or fd event, dispatches it, and drains one
/// queued continuation. Returns false when no event source remains.
#[no_mangle]
pub extern "C" fn thaw_runtime_run_one_event() -> bool {
    promote_due_timers();
    if thaw_runtime_poll_one() != 0 {
        return true;
    }
    let delay = next_timer_delay();
    if has_fd_waits() {
        poll_fd_waits(delay);
        let _ = thaw_runtime_poll_one();
        true
    } else if let Some(delay) = delay {
        if !delay.is_zero() {
            thread::sleep(delay);
        }
        promote_due_timers();
        let _ = thaw_runtime_poll_one();
        true
    } else {
        false
    }
}

fn enqueue_continuation(subscription: PromiseSubscription, result: *const u8) {
    if PURGING_ASYNC_STATE.with(Cell::get) {
        drop(subscription);
        return;
    }
    READY_CONTINUATIONS.with(|ready| ready.borrow_mut().push_back((subscription, result)));
}

fn promote_due_timers() {
    let now = Instant::now();
    let due = TIMERS.with(|timers| {
        let mut timers = timers.borrow_mut();
        let mut due = Vec::new();
        let mut i = 0;
        while i < timers.len() {
            if timers[i].deadline <= now {
                due.push(timers.swap_remove(i).promise);
            } else {
                i += 1;
            }
        }
        due
    });
    for promise in due {
        // A timer has no payload; a non-null sentinel distinguishes a
        // resolved timer from an invalid/no-progress result at the ABI edge.
        let result = std::ptr::dangling::<u8>();
        thaw_promise_resolve(promise, result);
    }
}

fn next_timer_delay() -> Option<Duration> {
    let now = Instant::now();
    TIMERS.with(|timers| {
        timers
            .borrow()
            .iter()
            .map(|timer| timer.deadline.saturating_duration_since(now))
            .min()
    })
}

/// Runs one ready continuation or dispatches ready I/O and returns 1. Returns
/// 0 when neither source is ready. I/O and QuickJS integrations can alternate
/// their own polling with this function without a multi-threaded executor.
#[no_mangle]
pub extern "C" fn thaw_runtime_poll_one() -> u8 {
    report_pending_rejection_handled();
    promote_due_timers();
    let io_events = poll_fd_waits(Some(Duration::ZERO));
    let next = READY_CONTINUATIONS.with(|ready| ready.borrow_mut().pop_front());
    let Some((mut subscription, result)) = next else {
        report_registered_unhandled_rejections();
        // A rejection listener can enqueue a continuation or another rejection.
        // Recheck after dispatch so an idle drain does not stop before that work.
        let reporting_work = PENDING_REJECTION_HANDLED.with(|pending| pending.get() != 0)
            || ACTIVE_PROMISES.with(|active| {
                active.borrow().iter().any(|promise| {
                    let promise = unsafe { &**promise };
                    promise.rejected && !promise.handled && !promise.reported_unhandled
                })
            });
        return u8::from(io_events != 0
            || READY_CONTINUATIONS.with(|ready| !ready.borrow().is_empty())
            || reporting_work);
    };
    subscription.delivered = true;
    struct RestoreDelivering(*mut PromiseSubscription);
    impl Drop for RestoreDelivering {
        fn drop(&mut self) {
            DELIVERING_SUBSCRIPTION.with(|active| active.set(self.0));
        }
    }
    let previous = DELIVERING_SUBSCRIPTION.with(|active| {
        active.replace(&mut subscription)
    });
    let _restore = RestoreDelivering(previous);
    (subscription.resume)(subscription.frame, result);
    1
}

/// Returns a Promise which settles when `fd` becomes readable or writable.
/// `interests` is a bitmask of `THAW_FD_READABLE`/`THAW_FD_WRITABLE`.
/// The caller retains ownership of the descriptor and must keep it open until
/// settlement or Promise destruction.
#[no_mangle]
pub extern "C" fn thaw_runtime_wait_fd(fd: libc::c_int, interests: u8) -> *mut ThawPromise {
    thaw_runtime_wait_fd_until(fd, interests, None)
}

/// Like `thaw_runtime_wait_fd`, but rejects if readiness is not observed
/// within `milliseconds`.
#[no_mangle]
pub extern "C" fn thaw_runtime_wait_fd_timeout(
    fd: libc::c_int,
    interests: u8,
    milliseconds: u64,
) -> *mut ThawPromise {
    thaw_runtime_wait_fd_until(
        fd,
        interests,
        Some(Instant::now() + Duration::from_millis(milliseconds)),
    )
}

fn thaw_runtime_wait_fd_until(
    fd: libc::c_int,
    interests: u8,
    deadline: Option<Instant>,
) -> *mut ThawPromise {
    let promise = thaw_promise_new();
    if promise.is_null() { return promise; }
    if PURGING_ASYNC_STATE.with(Cell::get) {
        reject_native_text(promise, ASYNC_PURGE_ERROR.as_ptr());
        return promise;
    }
    if fd < 0 || interests == 0 || interests & !(THAW_FD_READABLE | THAW_FD_WRITABLE) != 0 {
        reject_native_text(promise, INVALID_FD_ERROR.as_ptr());
        return promise;
    }
    FD_WAITS.with(|waits| {
        waits.borrow_mut().push(PromiseFdWait {
            fd,
            interests,
            promise,
            deadline,
        });
    });
    promise
}

include!("runtime/http.rs");

/// Drains all continuations which are ready now and returns how many ran.
/// Callbacks may enqueue more work; that work is included in the same drain.
#[no_mangle]
pub extern "C" fn thaw_runtime_run_until_idle() -> usize {
    let mut count = 0;
    while thaw_runtime_poll_one() != 0 {
        count += 1;
    }
    count
}

/// Timers and fd waits can become ready after an idle drain returns zero.
#[no_mangle]
pub extern "C" fn thaw_runtime_async_work_pending() -> u8 {
    u8::from(
        READY_CONTINUATIONS.with(|ready| !ready.borrow().is_empty())
            || TIMERS.with(|timers| !timers.borrow().is_empty())
            || has_fd_waits(),
    )
}

#[no_mangle]
pub extern "C" fn thaw_runtime_shutdown_wait() {
    std::thread::sleep(Duration::from_millis(1));
}

/// Drives detached Promise combinator children to completion. A rejected
/// parent may already have resumed user code, but its child async frames still
/// borrow the current request arena and must finish before that arena resets.
#[no_mangle]
pub extern "C" fn thaw_runtime_drain_detached() -> usize {
    let mut count = 0;
    while ACTIVE_PROMISE_JOINS.with(Cell::get) != 0 {
        if thaw_runtime_poll_one() != 0 {
            count += 1;
            continue;
        }
        let delay = next_timer_delay();
        if has_fd_waits() {
            poll_fd_waits(delay);
        } else if let Some(delay) = delay {
            if !delay.is_zero() {
                std::thread::sleep(delay);
            }
        } else {
            break;
        }
    }
    count
}

/// Creates a promise resolved by the runtime timer queue after at least
/// `milliseconds`. No worker thread is created.
#[no_mangle]
pub extern "C" fn thaw_sleep_ms(milliseconds: u64) -> *mut ThawPromise {
    let promise = thaw_promise_new();
    if promise.is_null() { return promise; }
    if PURGING_ASYNC_STATE.with(Cell::get) {
        reject_native_text(promise, ASYNC_PURGE_ERROR.as_ptr());
        return promise;
    }
    TIMERS.with(|timers| {
        timers.borrow_mut().push(PromiseTimer {
            deadline: Instant::now() + Duration::from_millis(milliseconds),
            promise,
        });
    });
    promise
}

/// Milliseconds elapsed since this process started, matching the Web/Node
/// `performance.now()` contract: a monotonic clock unaffected by wall-clock
/// adjustments, unlike `Date.now()`. The reference point (process start) is
/// implementation-defined by the spec, so any fixed monotonic epoch is
/// spec-compliant; there's no meaningful "request start" to anchor to
/// instead, since a warm Lambda execution environment reuses one process
/// across many invocations.
static PROCESS_START: OnceLock<Instant> = OnceLock::new();

#[no_mangle]
pub extern "C" fn thaw_performance_now() -> f64 {
    PROCESS_START
        .get_or_init(Instant::now)
        .elapsed()
        .as_secs_f64()
        * 1000.0
}

#[no_mangle]
pub extern "C" fn thaw_process_pid() -> f64 {
    f64::from(std::process::id())
}

#[no_mangle]
pub extern "C" fn thaw_process_ppid() -> f64 {
    #[cfg(target_family = "unix")]
    {
        f64::from(unsafe { libc::getppid() })
    }
    #[cfg(not(target_family = "unix"))]
    {
        0.0
    }
}

/// Drives ready continuations and timers until `promise` settles. Returns its
/// result/error pointer; use `thaw_promise_state` to distinguish fulfillment
/// from rejection. Returns null for an invalid handle or no possible progress.
///
/// # Safety
///
/// `promise` must be null or point to a live `ThawPromise` for the duration of
/// this call. No other thread may mutate or destroy it concurrently.
#[no_mangle]
pub unsafe extern "C" fn thaw_runtime_run_until_resolved(promise: *const ThawPromise) -> *const u8 {
    loop {
        let Some(promise_ref) = (unsafe { promise.as_ref() }) else {
            return std::ptr::null();
        };
        if let Some(result) = promise_ref.result {
            return result;
        }
        if invocation_deadline_remaining() == Some(Duration::ZERO) {
            reject_for_invocation_deadline(promise.cast_mut());
            continue;
        }
        if thaw_runtime_poll_one() != 0 {
            continue;
        }
        if unsafe { thaw_promise_state(promise) } != 0 {
            continue;
        }
        let delay = match (next_timer_delay(), invocation_deadline_remaining()) {
            (Some(timer), Some(deadline)) => Some(timer.min(deadline)),
            (Some(timer), None) => Some(timer),
            (None, Some(deadline)) => Some(deadline),
            (None, None) => None,
        };
        if has_fd_waits() {
            poll_fd_waits(delay);
        } else if let Some(delay) = delay {
            if !delay.is_zero() {
                std::thread::sleep(delay);
            }
        } else {
            return std::ptr::null();
        }
    }
}

/// Opaque C-ABI promise shared by generated coroutines and runtime bridges.
/// Producer hooks are intrusive and must remain in their owner allocation
/// until settlement or final Promise retirement.
#[repr(C)]
pub struct PromiseCompletionHook {
    pub next: *mut PromiseCompletionHook,
    pub callback: extern "C" fn(*mut u8),
    pub context: *mut u8,
}

/// Opaque C-ABI promise shared by generated coroutines and runtime bridges.
/// Result ownership remains with the producer and must outlive all resume
/// callbacks. V2 coroutine frames will normally keep results in the request
/// arena, so the Lambda request boundary remains the lifetime boundary.
#[repr(C)]
pub struct ThawPromise {
    owners: usize,
    creator_live: bool,
    base_live: bool,
    // Minted only by the split async ramp. An HTTP observer may release its
    // request base after failed subscription only for this producer kind.
    frame_completion: bool,
    identity: u64,
    // Registered by a compiler-generated, type-specific graph encoder.
    // Never serialized as a JS-visible address.
    graph_finisher: *const u8,
    // Box allocation is outside thaw_arena, but its settled result and
    // exception tuple can point into the arena across request resets.
    root: Option<thaw_arena::ArenaRoot>,
    result: Option<*const u8>,
    // 0: native producer bytes; 1: one canonical JS-origin Json value.
    // A typed consumer must inspect this before loading its expected T.
    result_kind: u8,
    fulfilled_provenance: *const ExceptionProvenance,
    rejection_text: Option<Vec<u8>>,
    rejected: bool,
    exception_tag: u64,
    exception_f64: f64,
    exception_i64: i64,
    exception_bool: bool,
    exception_object: *const u8,
    // Independently owned Json payload for a JS-origin rejection (tag 7).
    // The arena edge alone does not own a Box<Value> or its Host lease.
    rejection_json: Option<*mut u8>,
    // A fallback rejection can borrow the exact tag-7 Box from another
    // settled Promise. Its independent source share owns that Box until this
    // Promise retires; no second Host lease allocation is required.
    rejection_source: Option<*mut ThawPromise>,
    // Shares the *same* owned Json boxes referenced by Promise.any's arena
    // reason buffer across forwarding/finally. Copying boxes without rewriting
    // that buffer would leave its raw payload pointers dangling.
    aggregate_reason_owner: Option<std::rc::Rc<AggregateReasonOwner>>,
    aggregate_errors: *const u8,
    handled: bool,
    reported_unhandled: bool,
    subscribers: Vec<PromiseSubscription>,
    completion_hooks: *mut PromiseCompletionHook,
}

include!("runtime/promises.rs");

include!("runtime/lambda.rs");

#[cfg(test)]
mod tests;
