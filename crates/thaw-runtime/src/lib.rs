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

include!("runtime/native_values/numbers.rs");
include!("runtime/native_values/strings.rs");
include!("runtime/native_values/arrays.rs");
include!("runtime/native_values/regex.rs");
include!("runtime/native_values/date.rs");
include!("runtime/native_values/maps.rs");
include!("runtime/abi.rs");

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

include!("runtime/promises.rs");

include!("runtime/lambda.rs");

#[cfg(test)]
mod tests;
