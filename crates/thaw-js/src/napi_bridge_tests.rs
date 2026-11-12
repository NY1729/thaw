//! Unit tests for the bridge host in `napi_bridge.rs`: the registration natives, the realm globals,
//! the loop hooks and the shutdown order. The callbacks are fakes whose state is per thread, so
//! concurrent tests (and loops of other tests) do not see each other's counts. Registration itself
//! is process-wide, so every test here holds `LOCK`.

use std::cell::{Cell, RefCell};
use std::ffi::{c_char, CStr, CString};
use std::sync::{Mutex, MutexGuard};

use crate::host::thaw_tjs_terminal_work_pending;
use crate::napi_bridge::*;
use crate::Interp;

pub(crate) static LOCK: Mutex<()> = Mutex::new(());

thread_local! {
    static CALLS: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
    static POLLS: Cell<usize> = const { Cell::new(0) };
    static EVENTS: RefCell<Vec<&'static str>> = const { RefCell::new(Vec::new()) };
    static SHUTDOWN_POLLS: Cell<usize> = const { Cell::new(0) };
    static ERROR_PENDING: Cell<bool> = const { Cell::new(false) };
    /// Only the test's own thread drives the fakes. Other threads (the rest of the suite) share the
    /// process-wide registration while it is in place, and must see an idle bridge.
    static ARMED: Cell<bool> = const { Cell::new(false) };
    /// Every token the bridge was asked to release, in order.
    static RELEASED: RefCell<Vec<u64>> = const { RefCell::new(Vec::new()) };
}

/// Holds the test lock. Dropping it (also on panic) clears the registration and the fake state.
struct Guard {
    _lock: MutexGuard<'static, ()>,
}

impl Drop for Guard {
    fn drop(&mut self) {
        ARMED.with(|c| c.set(false));
        clear_for_tests();
    }
}

/// Takes the test lock, forgets every registration and per-thread fake state, and arms the fakes.
fn setup() -> Guard {
    let lock = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    clear_for_tests();
    CALLS.with(|c| c.borrow_mut().clear());
    POLLS.with(|c| c.set(0));
    EVENTS.with(|e| e.borrow_mut().clear());
    SHUTDOWN_POLLS.with(|c| c.set(0));
    ERROR_PENDING.with(|c| c.set(false));
    RELEASED.with(|r| r.borrow_mut().clear());
    ARMED.with(|c| c.set(true));
    Guard { _lock: lock }
}

fn armed() -> bool {
    ARMED.with(Cell::get)
}

fn owned(text: &str) -> *mut c_char {
    CString::new(text).unwrap().into_raw()
}

unsafe fn read(ptr: *const c_char) -> String {
    CStr::from_ptr(ptr).to_string_lossy().into_owned()
}

fn event(name: &'static str) {
    if armed() {
        EVENTS.with(|e| e.borrow_mut().push(name));
    }
}

unsafe extern "C" fn fake_exports() -> *const c_char {
    owned("{\"names\":[]}")
}

unsafe extern "C" fn fake_call(name: *const c_char, args: *const c_char) -> *const c_char {
    let line = format!("call:{}:{}", read(name), read(args));
    CALLS.with(|c| c.borrow_mut().push(line.clone()));
    owned(&line)
}

unsafe extern "C" fn other_call(_: *const c_char, _: *const c_char) -> *const c_char {
    owned("second")
}

unsafe extern "C" fn fake_handle(
    operation: *const c_char,
    target: *const c_char,
    name: *const c_char,
    args: *const c_char,
) -> *const c_char {
    owned(&format!(
        "handle:{}:{}:{}:{}",
        read(operation),
        read(target),
        read(name),
        read(args)
    ))
}

extern "C" fn fake_poll() -> usize {
    POLLS.with(|c| {
        c.set(c.get() + 1);
        c.get()
    })
}

/// Work stays pending until the third poll.
extern "C" fn fake_pending() -> u8 {
    (armed() && POLLS.with(Cell::get) < 3) as u8
}

unsafe extern "C" fn fake_owner(handle: u64) -> *mut c_char {
    owned(&format!("owner:{handle}"))
}

extern "C" fn fake_release(token: u64) -> u8 {
    RELEASED.with(|r| r.borrow_mut().push(token));
    (token == 7) as u8
}

extern "C" fn fake_begin() -> u8 {
    event("begin");
    1
}

extern "C" fn refuse_begin() -> u8 {
    0
}

extern "C" fn fake_shutdown_poll() -> u8 {
    if !armed() {
        return 1;
    }
    event("poll");
    SHUTDOWN_POLLS.with(|c| {
        c.set(c.get() + 1);
        (c.get() >= 2) as u8
    })
}

extern "C" fn fake_take_error() -> *mut c_char {
    event("take_error");
    if ERROR_PENDING.with(|c| c.replace(false)) {
        owned("boom")
    } else {
        std::ptr::null_mut()
    }
}

extern "C" fn fake_finish() -> u8 {
    event("finish");
    0
}

fn register_bridge(call: BridgeCallback) {
    thaw_tjs_register_napi_bridge(
        fake_exports,
        call,
        fake_handle,
        fake_poll,
        fake_pending,
        fake_owner,
        fake_release,
    );
}

fn register_shutdown() {
    thaw_tjs_register_napi_shutdown_bridge(
        fake_begin,
        fake_shutdown_poll,
        fake_take_error,
        fake_finish,
    );
}

/// The output of a script run in a fresh host realm, one line per `console.log`.
fn output(source: &str) -> String {
    let mut it = Interp::new();
    it.run(source)
        .unwrap_or_else(|_| panic!("script must run: {source}"));
    it.out.join("\n").trim_end().to_string()
}

#[test]
fn realm_globals_reach_the_registered_callbacks() {
    let _guard = setup();
    register_bridge(fake_call);
    assert_eq!(
        output(
            "console.log(__thaw_napi_bridge_available(), __thaw_napi_bridge_call('n', '[1]'), \
             __thaw_napi_bridge_handle('op', 't', 'name', '[]'), __thaw_napi_graph_owner('7'), \
             __thaw_napi_graph_release('7'), __thaw_napi_graph_release('8'), __thaw_napi_bridge_exports())"
        ),
        r#"true call:n:[1] handle:op:t:name:[] owner:7 true false {"names":[]}"#
    );
    CALLS.with(|c| assert_eq!(c.borrow().as_slice(), ["call:n:[1]"]));
}

#[test]
fn globals_report_unavailable_before_registration() {
    let _guard = setup();
    assert_eq!(
        output("console.log(__thaw_napi_bridge_available(), __thaw_napi_bridge_call('n', ''))"),
        "false"
    );
}

#[test]
fn registering_again_replaces_the_callbacks() {
    let _guard = setup();
    register_bridge(fake_call);
    register_bridge(other_call);
    assert_eq!(
        output("console.log(__thaw_napi_bridge_call('n', ''))"),
        "second"
    );
}

#[test]
fn realm_globals_are_hidden_and_read_only() {
    let _guard = setup();
    assert_eq!(
        output(
            "try { __thaw_napi_bridge_call = 1; } catch (e) { console.log(e.name); } \
             console.log(typeof __thaw_napi_bridge_call, \
             Object.keys(globalThis).indexOf('__thaw_napi_bridge_call'), \
             Object.keys(globalThis).indexOf('__thaw_napi_graph_owner'))"
        ),
        "TypeError\nfunction -1 -1"
    );
}

#[test]
fn pending_work_keeps_the_loop_polling_until_it_clears() {
    let _guard = setup();
    register_bridge(fake_call);
    let mut it = Interp::new();
    it.run("1").unwrap();
    it.run_event_loop().unwrap();
    assert_eq!(POLLS.with(Cell::get), 3);
}

#[test]
fn terminal_work_counts_pending_napi_work() {
    let _guard = setup();
    assert_eq!(thaw_tjs_terminal_work_pending(), 0);
    register_bridge(fake_call);
    assert_eq!(thaw_tjs_terminal_work_pending(), 1);
}

#[test]
fn release_all_handles_releases_bridge_retained_references() {
    let _guard = setup();
    register_bridge(fake_call);
    keep_until_shutdown(5);
    assert!(crate::host::thaw_tjs_release_all_handles() >= 1);
    RELEASED.with(|r| assert_eq!(r.borrow().as_slice(), [5]));
    assert_eq!(crate::host::thaw_tjs_release_all_handles(), 0);
}

#[test]
fn shutdown_begins_polls_collects_errors_then_finishes() {
    let _guard = setup();
    register_shutdown();
    ERROR_PENDING.with(|c| c.set(true));
    let mut it = Interp::new();
    assert_eq!(shutdown(&mut it), Err("boom".to_string()));
    let expected = [
        "begin",
        "poll",
        "take_error",
        "take_error",
        "poll",
        "take_error",
        "finish",
    ];
    EVENTS.with(|e| assert_eq!(e.borrow().as_slice(), expected));
    // a second shutdown on this thread does nothing
    assert_eq!(shutdown(&mut it), Ok(()));
    EVENTS.with(|e| assert_eq!(e.borrow().len(), expected.len()));
}

#[test]
fn shutdown_without_a_registered_bridge_does_nothing() {
    let _guard = setup();
    let mut it = Interp::new();
    assert_eq!(shutdown(&mut it), Ok(()));
    EVENTS.with(|e| assert!(e.borrow().is_empty()));
}

#[test]
fn shutdown_reports_a_refused_start() {
    let _guard = setup();
    thaw_tjs_register_napi_shutdown_bridge(
        refuse_begin,
        fake_shutdown_poll,
        fake_take_error,
        fake_finish,
    );
    let mut it = Interp::new();
    assert_eq!(
        shutdown(&mut it),
        Err("native addon shutdown could not start".to_string())
    );
}
