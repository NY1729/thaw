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
//! Known limitations, acceptable for what this talks to: no TLS, no
//! chunked transfer-encoding, one request per TCP connection.

use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::ffi::{CStr, CString};
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream, ToSocketAddrs};
use std::os::raw::c_char;
use std::os::unix::io::RawFd;
use std::sync::{Arc, Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant};

use rustls::pki_types::ServerName;
use rustls::{ClientConfig, ClientConnection, RootCertStore};

include!("runtime/native_values.rs");

#[derive(Clone, Copy)]
struct PromiseSubscription {
    resume: PromiseResumeFn,
    frame: *mut u8,
}

thread_local! {
    static READY_CONTINUATIONS: RefCell<VecDeque<(PromiseSubscription, *const u8)>> =
        const { RefCell::new(VecDeque::new()) };
    static TIMERS: RefCell<Vec<PromiseTimer>> = const { RefCell::new(Vec::new()) };
    static FD_WAITS: RefCell<Vec<PromiseFdWait>> = const { RefCell::new(Vec::new()) };
    static FD_WATCHERS: RefCell<Vec<FdWatcher>> = const { RefCell::new(Vec::new()) };
    static ACTIVE_PROMISE_JOINS: Cell<usize> = const { Cell::new(0) };
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

#[derive(Clone, Copy)]
struct FdWatcher {
    id: u64,
    fd: libc::c_int,
    interests: u8,
    callback: FdWatcherFn,
    context: *mut u8,
}

static NEXT_FD_WATCHER_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

static INVALID_FD_ERROR: &[u8] = b"invalid file descriptor\0";
static FD_TIMEOUT_ERROR: &[u8] = b"file descriptor wait timed out\0";
static PROMISE_ALL_INVALID_ERROR: &[u8] = b"Promise.all received an invalid promise\0";
static PROMISE_RACE_EMPTY_ERROR: &[u8] = b"Promise.race requires at least one promise\0";
static PROMISE_RACE_INVALID_ERROR: &[u8] = b"Promise.race received an invalid promise\0";
static PROMISE_CYCLE_ERROR: &[u8] = b"Chaining cycle detected for promise\0";
static PROMISE_ANY_REJECTED_ERROR: &[u8] = b"All promises were rejected\0";
static PROMISE_SETTLED_FULFILLED: &[u8] = b"fulfilled\0";
static PROMISE_SETTLED_REJECTED: &[u8] = b"rejected\0";
static PROMISE_SETTLED_EMPTY_REASON: &[u8] = b"\0";

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
    let timeout_ms = match timeout {
        Some(duration) => duration.as_millis().saturating_add(1).min(i32::MAX as u128) as i32,
        None => -1,
    };
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
            thaw_promise_reject(promise, FD_TIMEOUT_ERROR.as_ptr());
        } else if events & libc::POLLNVAL != 0 {
            thaw_promise_reject(promise, INVALID_FD_ERROR.as_ptr());
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
            .map(|(watcher, pollfd)| (*watcher, pollfd.revents))
            .collect::<Vec<_>>()
    });
    let watcher_count = ready_watchers.len();
    for (watcher, events) in ready_watchers {
        (watcher.callback)(watcher.context, events);
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
    promote_due_timers();
    let io_events = poll_fd_waits(Some(Duration::ZERO));
    let next = READY_CONTINUATIONS.with(|ready| ready.borrow_mut().pop_front());
    let Some((subscription, result)) = next else {
        return u8::from(io_events != 0);
    };
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
    if fd < 0 || interests == 0 || interests & !(THAW_FD_READABLE | THAW_FD_WRITABLE) != 0 {
        thaw_promise_reject(promise, INVALID_FD_ERROR.as_ptr());
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
    TIMERS.with(|timers| {
        timers.borrow_mut().push(PromiseTimer {
            deadline: Instant::now() + Duration::from_millis(milliseconds),
            promise,
        });
    });
    promise
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
        if thaw_runtime_poll_one() != 0 {
            continue;
        }
        if unsafe { thaw_promise_state(promise) } != 0 {
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
            return std::ptr::null();
        }
    }
}

/// Opaque C-ABI promise shared by generated coroutines and runtime bridges.
/// Result ownership remains with the producer and must outlive all resume
/// callbacks. V2 coroutine frames will normally keep results in the request
/// arena, so the Lambda request boundary remains the lifetime boundary.
#[repr(C)]
pub struct ThawPromise {
    result: Option<*const u8>,
    rejected: bool,
    subscribers: Vec<PromiseSubscription>,
}

/// Allocates an unresolved promise. Pair every successful call with
/// `thaw_promise_destroy` after no coroutine can reference the handle.
#[no_mangle]
pub extern "C" fn thaw_promise_new() -> *mut ThawPromise {
    Box::into_raw(Box::new(ThawPromise {
        result: None,
        rejected: false,
        subscribers: Vec::new(),
    }))
}

/// Returns 0 for pending, 1 for fulfilled, 2 for rejected, and 255 for an
/// invalid handle.
///
/// # Safety
///
/// `promise` must be null or point to a live `ThawPromise` that is not being
/// mutated or destroyed concurrently.
#[no_mangle]
pub unsafe extern "C" fn thaw_promise_state(promise: *const ThawPromise) -> u8 {
    let Some(promise) = (unsafe { promise.as_ref() }) else {
        return u8::MAX;
    };
    match (promise.result.is_some(), promise.rejected) {
        (false, _) => 0,
        (true, false) => 1,
        (true, true) => 2,
    }
}

/// Registers a coroutine continuation. If already resolved, the callback is
/// queued immediately, but never invoked reentrantly inside this function.
/// Returns 1 on success and 0 for an invalid handle.
///
/// # Safety
///
/// `promise` must be null or point to a live, exclusively accessible
/// `ThawPromise`. `frame` must remain valid whenever `resume` can be invoked.
#[no_mangle]
pub unsafe extern "C" fn thaw_promise_subscribe(
    promise: *mut ThawPromise,
    resume: PromiseResumeFn,
    frame: *mut u8,
) -> u8 {
    let Some(promise) = (unsafe { promise.as_mut() }) else {
        return 0;
    };
    if let Some(result) = promise.result {
        enqueue_continuation(PromiseSubscription { resume, frame }, result);
    } else {
        promise
            .subscribers
            .push(PromiseSubscription { resume, frame });
    }
    1
}

/// Resolves a promise exactly once and queues every current subscriber in
/// registration order. Returns 0 for a null handle or repeated resolution.
#[no_mangle]
pub extern "C" fn thaw_promise_resolve(promise: *mut ThawPromise, result: *const u8) -> u8 {
    settle_promise(promise, result, false)
}

/// Rejects a promise exactly once and queues all subscribers. The error is an
/// opaque producer-owned pointer, using the same lifetime contract as a
/// fulfilled result. Returns 0 for a null handle or repeated settlement.
#[no_mangle]
pub extern "C" fn thaw_promise_reject(promise: *mut ThawPromise, error: *const u8) -> u8 {
    settle_promise(promise, error, true)
}

struct PromiseChainState {
    output: *mut ThawPromise,
    input: *mut ThawPromise,
    callback: PromiseTransformFn,
    context: *mut u8,
    on_rejected: bool,
}

extern "C" fn resume_promise_chain(frame: *mut u8, result: *const u8) {
    let state = unsafe { Box::from_raw(frame.cast::<PromiseChainState>()) };
    let rejected = unsafe { thaw_promise_state(state.input) } == 2;
    if rejected == state.on_rejected {
        (state.callback)(state.context, state.output, result);
    } else if rejected {
        thaw_promise_reject(state.output, result);
    } else {
        thaw_promise_resolve(state.output, result);
    }
    unsafe { thaw_promise_destroy(state.input) };
}

/// Creates the Promise returned by `.then` or `.catch`. The input handle is
/// consumed. The callback is invoked only for the selected settlement kind;
/// the other kind is forwarded without changing its payload.
///
/// # Safety
///
/// `input` must point to a live `ThawPromise`. `context` must remain valid
/// until `callback` runs.
#[no_mangle]
pub unsafe extern "C" fn thaw_promise_chain(
    input: *mut ThawPromise,
    callback: PromiseTransformFn,
    context: *mut u8,
    on_rejected: u8,
) -> *mut ThawPromise {
    let output = thaw_promise_new();
    if input.is_null() {
        thaw_promise_reject(output, PROMISE_ALL_INVALID_ERROR.as_ptr());
        return output;
    }
    let state = Box::into_raw(Box::new(PromiseChainState {
        output,
        input,
        callback,
        context,
        on_rejected: on_rejected != 0,
    }));
    thaw_promise_subscribe(input, resume_promise_chain, state.cast());
    output
}

struct PromiseAdoptState {
    output: *mut ThawPromise,
    input: *mut ThawPromise,
}

extern "C" fn resume_promise_adopt(frame: *mut u8, result: *const u8) {
    let state = unsafe { Box::from_raw(frame.cast::<PromiseAdoptState>()) };
    if unsafe { thaw_promise_state(state.input) } == 2 {
        thaw_promise_reject(state.output, result);
    } else {
        thaw_promise_resolve(state.output, result);
    }
    unsafe { thaw_promise_destroy(state.input) };
}

/// Makes `output` follow `input`, implementing Promise callback flattening.
/// The input handle is consumed.
///
/// # Safety
///
/// Both arguments must point to distinct live promises.
#[no_mangle]
pub unsafe extern "C" fn thaw_promise_adopt(
    output: *mut ThawPromise,
    input: *mut ThawPromise,
) -> u8 {
    if output.is_null() || input.is_null() {
        return 0;
    }
    if output == input {
        thaw_promise_reject(output, PROMISE_CYCLE_ERROR.as_ptr());
        return 0;
    }
    let state = Box::into_raw(Box::new(PromiseAdoptState { output, input }));
    thaw_promise_subscribe(input, resume_promise_adopt, state.cast());
    1
}

struct PromiseFinallyState {
    output: *mut ThawPromise,
    input: *mut ThawPromise,
    callback: PromiseFinallyFn,
    context: *mut u8,
}

extern "C" fn resume_promise_finally(frame: *mut u8, result: *const u8) {
    let state = unsafe { Box::from_raw(frame.cast::<PromiseFinallyState>()) };
    let rejected = unsafe { thaw_promise_state(state.input) } == 2;
    (state.callback)(state.context, state.output, result, u8::from(rejected));
    unsafe { thaw_promise_destroy(state.input) };
}

/// Runs a `.finally` callback for either settlement kind. The callback owns
/// forwarding or replacing the original settlement. The input is consumed.
///
/// # Safety
///
/// `input` must point to a live Promise and `context` must outlive callback
/// dispatch.
#[no_mangle]
pub unsafe extern "C" fn thaw_promise_finally(
    input: *mut ThawPromise,
    callback: PromiseFinallyFn,
    context: *mut u8,
) -> *mut ThawPromise {
    let output = thaw_promise_new();
    if input.is_null() {
        thaw_promise_reject(output, PROMISE_ALL_INVALID_ERROR.as_ptr());
        return output;
    }
    let state = Box::into_raw(Box::new(PromiseFinallyState {
        output,
        input,
        callback,
        context,
    }));
    thaw_promise_subscribe(input, resume_promise_finally, state.cast());
    output
}

struct PromiseFinallyAdoptState {
    output: *mut ThawPromise,
    input: *mut ThawPromise,
    original: *const u8,
    original_rejected: bool,
}

extern "C" fn resume_promise_finally_adopt(frame: *mut u8, result: *const u8) {
    let state = unsafe { Box::from_raw(frame.cast::<PromiseFinallyAdoptState>()) };
    if unsafe { thaw_promise_state(state.input) } == 2 {
        thaw_promise_reject(state.output, result);
    } else if state.original_rejected {
        thaw_promise_reject(state.output, state.original);
    } else {
        thaw_promise_resolve(state.output, state.original);
    }
    unsafe { thaw_promise_destroy(state.input) };
}

/// Waits for a Promise returned by `.finally`, then forwards the original
/// settlement unless the returned Promise rejects.
///
/// # Safety
///
/// `output` and `input` must be distinct live Promises. `original` must remain
/// valid until `input` settles.
#[no_mangle]
pub unsafe extern "C" fn thaw_promise_finally_adopt(
    output: *mut ThawPromise,
    input: *mut ThawPromise,
    original: *const u8,
    original_rejected: u8,
) -> u8 {
    if output.is_null() || input.is_null() || output == input {
        return 0;
    }
    let state = Box::into_raw(Box::new(PromiseFinallyAdoptState {
        output,
        input,
        original,
        original_rejected: original_rejected != 0,
    }));
    thaw_promise_subscribe(input, resume_promise_finally_adopt, state.cast());
    1
}

struct PromiseAllState {
    output: *mut ThawPromise,
    remaining: usize,
    rejected: bool,
    first_error: *const u8,
    result: *mut u8,
    result_slot: *mut *const u8,
    element_sizes: Vec<usize>,
    element_offsets: Vec<usize>,
}

struct PromiseAllChild {
    state: *mut PromiseAllState,
    promise: *mut ThawPromise,
    indices: Vec<usize>,
}

extern "C" fn resume_promise_all_child(frame: *mut u8, result: *const u8) {
    let child = unsafe { Box::from_raw(frame.cast::<PromiseAllChild>()) };
    let state = unsafe { &mut *child.state };
    let child_state = unsafe { thaw_promise_state(child.promise) };
    if child_state == 2 {
        if !state.rejected {
            state.rejected = true;
            state.first_error = result;
            thaw_promise_reject(state.output, result);
        }
    } else if !state.rejected {
        for index in &child.indices {
            let destination = unsafe { state.result.add(state.element_offsets[*index]) };
            unsafe {
                destination.write_bytes(0, state.element_sizes[*index].max(size_of::<u64>()));
                std::ptr::copy_nonoverlapping(result, destination, state.element_sizes[*index]);
            }
        }
    }
    unsafe { thaw_promise_destroy(child.promise) };
    state.remaining -= 1;
    if state.remaining == 0 {
        if !state.rejected {
            thaw_promise_resolve(state.output, state.result_slot.cast());
        }
        ACTIVE_PROMISE_JOINS.with(|active| active.set(active.get() - 1));
        unsafe { drop(Box::from_raw(child.state)) };
    }
}

/// Joins homogeneous Promise handles without serializing them.
/// The result uses Thaw's `[i64 len][8-byte slots...]` array layout in the
/// request arena and therefore remains valid after the returned promise is
/// destroyed. Child handles are consumed by this call.
///
/// # Safety
///
/// `promises` must reference `len` live handles returned by Thaw async
/// functions. Each handle must be unique and must not be used after this call.
#[no_mangle]
pub unsafe extern "C" fn thaw_promise_all_slots(
    promises: *const *mut ThawPromise,
    len: usize,
    element_size: usize,
) -> *mut ThawPromise {
    let element_sizes = vec![element_size; len];
    unsafe { thaw_promise_all_typed(promises, element_sizes.as_ptr(), len) }
}

/// Joins Promise handles using one result-copy size per input position.
/// Each value occupies at least one eight-byte array slot; wider values use
/// their complete byte width.
///
/// # Safety
///
/// `promises` and `element_sizes` must each reference `len` readable entries.
/// Promise handles are consumed and must not be used after this call.
#[no_mangle]
pub unsafe extern "C" fn thaw_promise_all_typed(
    promises: *const *mut ThawPromise,
    element_sizes: *const usize,
    len: usize,
) -> *mut ThawPromise {
    let output = thaw_promise_new();
    if len != 0 && element_sizes.is_null() {
        thaw_promise_reject(output, PROMISE_ALL_INVALID_ERROR.as_ptr());
        return output;
    }
    let element_sizes = if len == 0 {
        Vec::new()
    } else {
        unsafe { std::slice::from_raw_parts(element_sizes, len) }.to_vec()
    };
    if element_sizes.contains(&0) {
        thaw_promise_reject(output, PROMISE_ALL_INVALID_ERROR.as_ptr());
        return output;
    }
    let mut element_offsets = Vec::with_capacity(len);
    let mut result_bytes = size_of::<u64>();
    for size in &element_sizes {
        element_offsets.push(result_bytes);
        let Some(next) = result_bytes.checked_add((*size).max(size_of::<u64>())) else {
            thaw_promise_reject(output, PROMISE_ALL_INVALID_ERROR.as_ptr());
            return output;
        };
        result_bytes = next;
    }
    let Some(allocation_bytes) = result_bytes.checked_add(size_of::<*const u8>()) else {
        thaw_promise_reject(output, PROMISE_ALL_INVALID_ERROR.as_ptr());
        return output;
    };
    let allocation = thaw_arena::thaw_arena_alloc(allocation_bytes, align_of::<u64>());
    if allocation.is_null() {
        thaw_promise_reject(output, PROMISE_ALL_INVALID_ERROR.as_ptr());
        return output;
    }
    let result_slot = allocation.cast::<*const u8>();
    let result = unsafe { allocation.add(size_of::<*const u8>()) };
    unsafe { result_slot.write(result.cast()) };
    unsafe { result.cast::<u64>().write(len as u64) };
    if len == 0 {
        thaw_promise_resolve(output, result_slot.cast());
        return output;
    }
    if promises.is_null() {
        thaw_promise_reject(output, PROMISE_ALL_INVALID_ERROR.as_ptr());
        return output;
    }
    let mut grouped = Vec::<(*mut ThawPromise, Vec<usize>)>::new();
    let mut invalid = false;
    for index in 0..len {
        let promise = unsafe { *promises.add(index) };
        if promise.is_null() {
            invalid = true;
        } else {
            if let Some((_, indices)) = grouped
                .iter_mut()
                .find(|(existing, _)| *existing == promise)
            {
                indices.push(index);
            } else {
                grouped.push((promise, vec![index]));
            }
        }
    }
    let state = Box::into_raw(Box::new(PromiseAllState {
        output,
        remaining: grouped.len(),
        rejected: invalid,
        first_error: if invalid {
            PROMISE_ALL_INVALID_ERROR.as_ptr()
        } else {
            std::ptr::null()
        },
        result,
        result_slot,
        element_sizes,
        element_offsets,
    }));
    if !grouped.is_empty() {
        ACTIVE_PROMISE_JOINS.with(|active| active.set(active.get() + 1));
    }
    if invalid {
        thaw_promise_reject(output, PROMISE_ALL_INVALID_ERROR.as_ptr());
    }
    for (promise, indices) in grouped {
        let child = Box::into_raw(Box::new(PromiseAllChild {
            state,
            promise,
            indices,
        }));
        unsafe {
            thaw_promise_subscribe(promise, resume_promise_all_child, child.cast());
        }
    }
    if unsafe { (*state).remaining } == 0 {
        let error = unsafe { (*state).first_error };
        thaw_promise_reject(output, error);
        unsafe { drop(Box::from_raw(state)) };
    }
    output
}

/// Backward-compatible number-only entry point.
///
/// # Safety
///
/// The same contract as [`thaw_promise_all_slots`] applies.
#[no_mangle]
pub unsafe extern "C" fn thaw_promise_all_f64(
    promises: *const *mut ThawPromise,
    len: usize,
) -> *mut ThawPromise {
    unsafe { thaw_promise_all_slots(promises, len, size_of::<f64>()) }
}

struct PromiseRaceState {
    output: *mut ThawPromise,
    remaining: usize,
    settled: bool,
}

struct PromiseRaceChild {
    state: *mut PromiseRaceState,
    promise: *mut ThawPromise,
}

extern "C" fn resume_promise_race_child(frame: *mut u8, result: *const u8) {
    let child = unsafe { Box::from_raw(frame.cast::<PromiseRaceChild>()) };
    let state = unsafe { &mut *child.state };
    if !state.settled {
        state.settled = true;
        if unsafe { thaw_promise_state(child.promise) } == 2 {
            thaw_promise_reject(state.output, result);
        } else {
            thaw_promise_resolve(state.output, result);
        }
    }
    unsafe { thaw_promise_destroy(child.promise) };
    state.remaining -= 1;
    if state.remaining == 0 {
        ACTIVE_PROMISE_JOINS.with(|active| active.set(active.get() - 1));
        unsafe { drop(Box::from_raw(child.state)) };
    }
}

/// Settles with the first input Promise to fulfill or reject. Input handles
/// are deduplicated and consumed; slower children continue to be drained.
///
/// # Safety
///
/// `promises` must reference `len` readable Promise handles. Each distinct
/// handle must be live and must not be used after this call.
#[no_mangle]
pub unsafe extern "C" fn thaw_promise_race(
    promises: *const *mut ThawPromise,
    len: usize,
) -> *mut ThawPromise {
    let output = thaw_promise_new();
    if len == 0 {
        thaw_promise_reject(output, PROMISE_RACE_EMPTY_ERROR.as_ptr());
        return output;
    }
    if promises.is_null() {
        thaw_promise_reject(output, PROMISE_RACE_INVALID_ERROR.as_ptr());
        return output;
    }
    let mut unique = Vec::<*mut ThawPromise>::new();
    let mut invalid = false;
    for index in 0..len {
        let promise = unsafe { *promises.add(index) };
        if promise.is_null() {
            invalid = true;
        } else if !unique.contains(&promise) {
            unique.push(promise);
        }
    }
    let state = Box::into_raw(Box::new(PromiseRaceState {
        output,
        remaining: unique.len(),
        settled: invalid,
    }));
    if !unique.is_empty() {
        ACTIVE_PROMISE_JOINS.with(|active| active.set(active.get() + 1));
    }
    if invalid {
        thaw_promise_reject(output, PROMISE_RACE_INVALID_ERROR.as_ptr());
    }
    for promise in unique {
        let child = Box::into_raw(Box::new(PromiseRaceChild { state, promise }));
        unsafe { thaw_promise_subscribe(promise, resume_promise_race_child, child.cast()) };
    }
    if unsafe { (*state).remaining } == 0 {
        unsafe { drop(Box::from_raw(state)) };
    }
    output
}

struct PromiseAnyState {
    output: *mut ThawPromise,
    remaining: usize,
    fulfilled: bool,
}

struct PromiseAnyChild {
    state: *mut PromiseAnyState,
    promise: *mut ThawPromise,
}

extern "C" fn resume_promise_any_child(frame: *mut u8, result: *const u8) {
    let child = unsafe { Box::from_raw(frame.cast::<PromiseAnyChild>()) };
    let state = unsafe { &mut *child.state };
    if !state.fulfilled && unsafe { thaw_promise_state(child.promise) } == 1 {
        state.fulfilled = true;
        thaw_promise_resolve(state.output, result);
    }
    unsafe { thaw_promise_destroy(child.promise) };
    state.remaining -= 1;
    if state.remaining == 0 {
        if !state.fulfilled {
            thaw_promise_reject(state.output, PROMISE_ANY_REJECTED_ERROR.as_ptr());
        }
        ACTIVE_PROMISE_JOINS.with(|active| active.set(active.get() - 1));
        unsafe { drop(Box::from_raw(child.state)) };
    }
}

/// Resolves with the first fulfilled input Promise. Rejections are ignored
/// until every distinct input has rejected, at which point an aggregate error
/// message is used to reject the output. Slower children are still drained.
///
/// # Safety
///
/// `promises` must reference `len` readable Promise handles. Each distinct
/// non-null handle is consumed and must not be used after this call.
#[no_mangle]
pub unsafe extern "C" fn thaw_promise_any(
    promises: *const *mut ThawPromise,
    len: usize,
) -> *mut ThawPromise {
    let output = thaw_promise_new();
    if len == 0 || promises.is_null() {
        thaw_promise_reject(output, PROMISE_ANY_REJECTED_ERROR.as_ptr());
        return output;
    }
    let mut unique = Vec::<*mut ThawPromise>::new();
    for index in 0..len {
        let promise = unsafe { *promises.add(index) };
        if !promise.is_null() && !unique.contains(&promise) {
            unique.push(promise);
        }
    }
    if unique.is_empty() {
        thaw_promise_reject(output, PROMISE_ANY_REJECTED_ERROR.as_ptr());
        return output;
    }
    let state = Box::into_raw(Box::new(PromiseAnyState {
        output,
        remaining: unique.len(),
        fulfilled: false,
    }));
    ACTIVE_PROMISE_JOINS.with(|active| active.set(active.get() + 1));
    for promise in unique {
        let child = Box::into_raw(Box::new(PromiseAnyChild { state, promise }));
        unsafe { thaw_promise_subscribe(promise, resume_promise_any_child, child.cast()) };
    }
    output
}

struct PromiseAllSettledState {
    output: *mut ThawPromise,
    remaining: usize,
    result_slot: *mut *const u8,
    result: *mut u64,
    objects: *mut u8,
    element_size: usize,
    object_stride: usize,
}

struct PromiseAllSettledChild {
    state: *mut PromiseAllSettledState,
    promise: *mut ThawPromise,
    indices: Vec<usize>,
}

extern "C" fn resume_promise_all_settled_child(frame: *mut u8, value: *const u8) {
    let child = unsafe { Box::from_raw(frame.cast::<PromiseAllSettledChild>()) };
    let state = unsafe { &mut *child.state };
    let rejected = unsafe { thaw_promise_state(child.promise) } == 2;
    for index in &child.indices {
        let object = unsafe { state.objects.add(index * state.object_stride) };
        let value_slot = unsafe { object.add(size_of::<u64>()) };
        let reason_slot = unsafe { value_slot.add(state.element_size.max(size_of::<u64>())) };
        unsafe {
            object.cast::<u64>().write(if rejected {
                PROMISE_SETTLED_REJECTED.as_ptr() as u64
            } else {
                PROMISE_SETTLED_FULFILLED.as_ptr() as u64
            });
            value_slot.write_bytes(0, state.element_size.max(size_of::<u64>()));
            if !rejected {
                std::ptr::copy_nonoverlapping(value, value_slot, state.element_size);
            }
            reason_slot.cast::<u64>().write(if rejected {
                value as u64
            } else {
                PROMISE_SETTLED_EMPTY_REASON.as_ptr() as u64
            });
            state.result.add(index + 1).write(object as u64);
        }
    }
    unsafe { thaw_promise_destroy(child.promise) };
    state.remaining -= 1;
    if state.remaining == 0 {
        thaw_promise_resolve(state.output, state.result_slot.cast());
        ACTIVE_PROMISE_JOINS.with(|active| active.set(active.get() - 1));
        unsafe { drop(Box::from_raw(child.state)) };
    }
}

/// Waits for every distinct input and resolves with an input-ordered array of
/// `{ status, value, reason }` object pointers. Rejections become result
/// entries and never reject the output Promise.
///
/// # Safety
///
/// `promises` must reference `len` readable Promise handles. Each distinct
/// handle is consumed. Values wider than one array slot are copied in full.
#[no_mangle]
pub unsafe extern "C" fn thaw_promise_all_settled(
    promises: *const *mut ThawPromise,
    len: usize,
    element_size: usize,
) -> *mut ThawPromise {
    let output = thaw_promise_new();
    if element_size == 0 || (len != 0 && promises.is_null()) {
        thaw_promise_reject(output, PROMISE_ALL_INVALID_ERROR.as_ptr());
        return output;
    }
    let allocation =
        thaw_arena::thaw_arena_alloc((len + 2) * size_of::<u64>(), align_of::<u64>()).cast::<u64>();
    let Some(object_stride) = size_of::<u64>()
        .checked_add(element_size.max(size_of::<u64>()))
        .and_then(|size| size.checked_add(size_of::<u64>()))
    else {
        thaw_promise_reject(output, PROMISE_ALL_INVALID_ERROR.as_ptr());
        return output;
    };
    let Some(objects_size) = len.checked_mul(object_stride) else {
        thaw_promise_reject(output, PROMISE_ALL_INVALID_ERROR.as_ptr());
        return output;
    };
    let objects = thaw_arena::thaw_arena_alloc(objects_size, align_of::<u64>());
    if allocation.is_null() || (len != 0 && objects.is_null()) {
        thaw_promise_reject(output, PROMISE_ALL_INVALID_ERROR.as_ptr());
        return output;
    }
    let result_slot = allocation.cast::<*const u8>();
    let result = unsafe { allocation.add(1) };
    unsafe {
        result_slot.write(result.cast());
        result.write(len as u64);
    }
    if len == 0 {
        thaw_promise_resolve(output, result_slot.cast());
        return output;
    }
    let mut grouped = Vec::<(*mut ThawPromise, Vec<usize>)>::new();
    for index in 0..len {
        let promise = unsafe { *promises.add(index) };
        if promise.is_null() {
            thaw_promise_reject(output, PROMISE_ALL_INVALID_ERROR.as_ptr());
            return output;
        }
        if let Some((_, indices)) = grouped
            .iter_mut()
            .find(|(existing, _)| *existing == promise)
        {
            indices.push(index);
        } else {
            grouped.push((promise, vec![index]));
        }
    }
    let state = Box::into_raw(Box::new(PromiseAllSettledState {
        output,
        remaining: grouped.len(),
        result_slot,
        result,
        objects,
        element_size,
        object_stride,
    }));
    ACTIVE_PROMISE_JOINS.with(|active| active.set(active.get() + 1));
    for (promise, indices) in grouped {
        let child = Box::into_raw(Box::new(PromiseAllSettledChild {
            state,
            promise,
            indices,
        }));
        unsafe { thaw_promise_subscribe(promise, resume_promise_all_settled_child, child.cast()) };
    }
    output
}

fn settle_promise(promise: *mut ThawPromise, result: *const u8, rejected: bool) -> u8 {
    let Some(promise) = (unsafe { promise.as_mut() }) else {
        return 0;
    };
    if promise.result.is_some() {
        return 0;
    }
    promise.result = Some(result);
    promise.rejected = rejected;
    let subscribers = std::mem::take(&mut promise.subscribers);
    for subscriber in subscribers {
        enqueue_continuation(subscriber, result);
    }
    1
}

/// Destroys a promise handle. Passing null is a no-op. The caller must not
/// destroy a promise from inside one of its resume callbacks.
///
/// # Safety
///
/// `promise` must be null or a pointer returned by `thaw_promise_new` that has
/// not already been destroyed and is no longer referenced by any subscriber.
#[no_mangle]
pub unsafe extern "C" fn thaw_promise_destroy(promise: *mut ThawPromise) {
    if !promise.is_null() {
        TIMERS.with(|timers| timers.borrow_mut().retain(|timer| timer.promise != promise));
        FD_WAITS.with(|waits| waits.borrow_mut().retain(|wait| wait.promise != promise));
        drop(Box::from_raw(promise));
    }
}

/// Reclaims request-owned generated values on every exit path. The handler
/// result is copied into a Rust `String` before this guard is dropped, so the
/// response never borrows reclaimed arena memory.
struct InvocationArenaReset;

impl Drop for InvocationArenaReset {
    fn drop(&mut self) {
        thaw_runtime_drain_detached();
        thaw_arena::thaw_arena_reset();
    }
}

/// Runs the event loop forever. Compiled by `thaw-llvm::hir_codegen` as the
/// process entry point whenever a program defines `handler` instead of
/// `main` (see that module for the `int main(void)` wrapper that calls
/// this).
#[no_mangle]
pub extern "C" fn thaw_runtime_run(handler: HandlerFn, error_slot: HandlerErrorSlot) -> ! {
    let runtime_api = std::env::var("AWS_LAMBDA_RUNTIME_API").expect(
        "AWS_LAMBDA_RUNTIME_API is not set -- is this running inside a Lambda execution environment?",
    );

    loop {
        if let Err(err) = handle_one_invocation(&runtime_api, handler, error_slot) {
            eprintln!("thaw-runtime: {err}");
        }
    }
}

fn handle_one_invocation(
    runtime_api: &str,
    handler: HandlerFn,
    error_slot: HandlerErrorSlot,
) -> Result<(), String> {
    let _arena_reset = InvocationArenaReset;
    let next = http_request(
        runtime_api,
        "GET",
        "/2018-06-01/runtime/invocation/next",
        None,
    )?;

    let request_id = next
        .header("lambda-runtime-aws-request-id")
        .ok_or("response from .../invocation/next is missing the request id header")?
        .to_string();

    let event_cstring = CString::new(next.body).map_err(|e| e.to_string())?;
    if !error_slot.is_null() {
        unsafe { *error_slot = std::ptr::null() };
    }
    let result_ptr = handler(event_cstring.as_ptr());
    if result_ptr.is_null() {
        let message = if error_slot.is_null() || unsafe { (*error_slot).is_null() } {
            "handler failed with an uncaught Thaw exception".to_string()
        } else {
            unsafe { CStr::from_ptr(*error_slot) }
                .to_string_lossy()
                .into_owned()
        };
        let error_path = format!("/2018-06-01/runtime/invocation/{request_id}/error");
        let body = format!(
            "{{\"errorMessage\":{},\"errorType\":\"ThawError\"}}",
            json_string(&message)
        );
        http_request(runtime_api, "POST", &error_path, Some(&body))?;
        return Ok(());
    }
    let result = unsafe { CStr::from_ptr(result_ptr) }
        .to_string_lossy()
        .into_owned();

    let response_path = format!("/2018-06-01/runtime/invocation/{request_id}/response");
    http_request(runtime_api, "POST", &response_path, Some(&result))?;
    Ok(())
}

fn json_string(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    for ch in value.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            ch if ch.is_control() => out.push_str(&format!("\\u{:04x}", ch as u32)),
            ch => out.push(ch),
        }
    }
    out.push('"');
    out
}

struct HttpResponse {
    status: u32,
    headers: Vec<(String, String)>,
    body: String,
}

impl HttpResponse {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }
}

fn http_request(
    host: &str,
    method: &str,
    path: &str,
    body: Option<&str>,
) -> Result<HttpResponse, String> {
    let mut stream = TcpStream::connect(host).map_err(|e| format!("connecting to {host}: {e}"))?;

    let mut request = format!("{method} {path} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n");
    if let Some(body) = body {
        request.push_str(&format!(
            "Content-Type: application/json\r\nContent-Length: {}\r\n",
            body.len()
        ));
    }
    request.push_str("\r\n");
    if let Some(body) = body {
        request.push_str(body);
    }

    stream
        .write_all(request.as_bytes())
        .map_err(|e| format!("writing request: {e}"))?;
    // Half-close the write side so the peer's `read_to_end` (used for
    // request bodies on our mock server, and in principle by any HTTP/1.1
    // peer reading an unbounded body) sees EOF instead of blocking forever
    // waiting for more of *our* request -- otherwise both sides can end up
    // blocked in `read_to_end` at once.
    stream
        .shutdown(std::net::Shutdown::Write)
        .map_err(|e| format!("shutting down write half: {e}"))?;

    let mut raw = Vec::new();
    stream
        .read_to_end(&mut raw)
        .map_err(|e| format!("reading response: {e}"))?;
    let raw = String::from_utf8_lossy(&raw);

    let (head, body) = raw
        .split_once("\r\n\r\n")
        .ok_or("malformed HTTP response (no header/body separator)")?;

    let mut lines = head.lines();
    let status_line = lines.next().ok_or("empty HTTP response")?;
    let status = status_line
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse::<u32>().ok())
        .ok_or_else(|| format!("malformed status line: {status_line}"))?;

    let headers = lines
        .filter_map(|line| line.split_once(':'))
        .map(|(k, v)| (k.trim().to_string(), v.trim().to_string()))
        .collect();

    let response = HttpResponse {
        status,
        headers,
        body: body.to_string(),
    };

    if response.status >= 400 {
        return Err(format!(
            "{method} {path} -> HTTP {}: {}",
            response.status, response.body
        ));
    }

    Ok(response)
}

#[cfg(test)]
mod tests;
