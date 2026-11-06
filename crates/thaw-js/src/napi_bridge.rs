//! Bridge host of the N-API engine seam (PR 5 of the stage 2 plan): the seven callbacks the napi side
//! registers, the four shutdown callbacks, the per-realm `__thaw_napi_bridge_*` and
//! `__thaw_napi_graph_{owner,release}` globals, and the loop hooks (poll, pending, shutdown).
//! Mirrors `register_napi_bridge`, `register_napi_shutdown_bridge` and `install_napi_bridge` of thaw-quickjs.
//! Registration is by function pointer, so no link-time export is needed.
//! ponytail: the callbacks are process-wide while the napi host is thread-local; a worker thread's end has no shutdown hook yet.

use std::cell::Cell;
use std::ffi::{c_char, CStr, CString};
use std::sync::Mutex;

use crate::host::classify;
use crate::napi_graph;
use crate::{arg, Interp, Value, R};

pub type BridgeCallback = unsafe extern "C" fn(*const c_char, *const c_char) -> *const c_char;
pub type BridgeExports = unsafe extern "C" fn() -> *const c_char;
pub type BridgePoll = extern "C" fn() -> usize;
pub type BridgePending = extern "C" fn() -> u8;
pub type BridgeOwner = unsafe extern "C" fn(u64) -> *mut c_char;
pub type BridgeRelease = extern "C" fn(u64) -> u8;
pub type BridgeHandle = unsafe extern "C" fn(
    *const c_char,
    *const c_char,
    *const c_char,
    *const c_char,
) -> *const c_char;

#[derive(Clone, Copy)]
struct Bridge {
    exports: BridgeExports,
    call: BridgeCallback,
    handle: BridgeHandle,
    poll: BridgePoll,
    pending: BridgePending,
    owner: BridgeOwner,
    release: BridgeRelease,
}

#[derive(Clone, Copy)]
struct Shutdown {
    begin: extern "C" fn() -> u8,
    poll: extern "C" fn() -> u8,
    take_error: extern "C" fn() -> *mut c_char,
    finish: extern "C" fn() -> u8,
}

static BRIDGE: Mutex<Option<Bridge>> = Mutex::new(None);
static SHUTDOWN: Mutex<Option<Shutdown>> = Mutex::new(None);

thread_local! {
    /// Set when `shutdown` has begun on this thread; not reset (a second shutdown is a no-op).
    static SHUTDOWN_DONE: Cell<bool> = const { Cell::new(false) };
}

/// Registers the napi side's callbacks. Registering again replaces them.
#[no_mangle]
pub extern "C" fn thaw_tjs_register_napi_bridge(
    exports: BridgeExports,
    call: BridgeCallback,
    handle: BridgeHandle,
    poll: BridgePoll,
    pending: BridgePending,
    owner: BridgeOwner,
    release: BridgeRelease,
) {
    *BRIDGE.lock().unwrap() = Some(Bridge {
        exports,
        call,
        handle,
        poll,
        pending,
        owner,
        release,
    });
}

/// Registers the napi side's shutdown callbacks. Registering again replaces them.
#[no_mangle]
pub extern "C" fn thaw_tjs_register_napi_shutdown_bridge(
    begin: extern "C" fn() -> u8,
    poll: extern "C" fn() -> u8,
    take_error: extern "C" fn() -> *mut c_char,
    finish: extern "C" fn() -> u8,
) {
    *SHUTDOWN.lock().unwrap() = Some(Shutdown {
        begin,
        poll,
        take_error,
        finish,
    });
}

/// Whether the napi side has registered its bridge. The copy is taken under the lock, so a
/// callback that re-enters the bridge never waits on it.
fn bridge() -> Option<Bridge> {
    *BRIDGE.lock().unwrap()
}

pub(crate) fn registered() -> bool {
    bridge().is_some()
}

/// Releases one reference the napi side retained for a graph wire (what `__thaw_napi_graph_release` does);
/// false when no bridge is registered.
pub(crate) fn release_reference(token: u64) -> bool {
    match bridge() {
        Some(b) => (b.release)(token) != 0,
        None => false,
    }
}

/// One `handle` operation of the bridge (`retain_graph_handle`, `symbol_handle`, `symbol_metadata`): its
/// JSON reply, or the error text the reply carries.
pub(crate) fn handle_op(
    op: &str,
    target: &str,
    extra: &str,
    graph: &str,
) -> Result<serde_json::Value, String> {
    let Some(b) = bridge() else {
        return Err("native addon bridge is not registered".into());
    };
    let parts = [op, target, extra, graph].map(cstring);
    // SAFETY: the parts are live NUL-terminated strings for the duration of the call.
    let reply = unsafe {
        take_text((b.handle)(
            parts[0].as_ptr(),
            parts[1].as_ptr(),
            parts[2].as_ptr(),
            parts[3].as_ptr(),
        ))
    };
    let reply: serde_json::Value =
        serde_json::from_str(&reply).map_err(|_| "malformed native bridge reply".to_string())?;
    match reply.get("__thaw_error__") {
        Some(error) => Err(error
            .as_str()
            .unwrap_or("native bridge operation failed")
            .to_string()),
        None => Ok(reply),
    }
}

/// Copies a string the napi side returned and frees it; null is the empty string.
///
/// # Safety
/// `ptr` must be null or a string this bridge owns (paired with `destroy_string`).
unsafe fn take_text(ptr: *const c_char) -> String {
    if ptr.is_null() {
        return String::new();
    }
    let text = unsafe { CStr::from_ptr(ptr) }
        .to_string_lossy()
        .into_owned();
    unsafe { thaw_arena::destroy_string(ptr) };
    text
}

/// The text of argument `i`; anything but a string is empty.
fn text(args: &[Value], i: usize) -> String {
    match arg(args, i) {
        Value::Str(s) => s.to_string(),
        _ => String::new(),
    }
}

fn cstring(text: &str) -> CString {
    CString::new(text).unwrap_or_default()
}

/// Throws `text` as a TypeError (a host failure of a graph operation).
fn throw_text<T>(it: &mut Interp, text: &str) -> R<T> {
    it.throw_error("TypeError", text.trim_start_matches("TypeError: "))
}

/// `__thaw_json_graph_encode_js(root, handleMask, live, plainResult, forceHandle)`: the JSON text of the graph
/// wire. `plainResult` is the copy lane (`WireOut` plain mode): its handle mask and forced root are not
/// applied (ponytail: the napi encoder has no plain mode of its own).
fn graph_encode(it: &mut Interp, args: &[Value]) -> R<Value> {
    let owner = crate::host::default_realm();
    let root = arg(args, 0);
    let mask = match arg(args, 1) {
        Value::Number(n) if n >= 0.0 => n as u32,
        _ => 0,
    };
    let live = arg(args, 2).truthy();
    let force = match arg(args, 4) {
        Value::Undefined => None,
        value => Some(value),
    };
    let encoded = if arg(args, 3).truthy() {
        let mut out = crate::host::WireOut {
            owner,
            plain: true,
            live_functions: false,
            copy_root: false,
            receiver: None,
            nodes: Vec::new(),
            seen: Vec::new(),
            leases: Vec::new(),
        };
        out.token(it, &root).map(|token| out.finish(token))
    } else {
        napi_graph::encode_napi_graph(it, owner, &root, mask, live, force)
    };
    match encoded {
        Ok(text) => Ok(Value::str(&text)),
        Err(e) => throw_text(it, &e.to_string()),
    }
}

/// Installs the `__thaw_napi_bridge_*` and `__thaw_napi_graph_*` globals of a realm as immutable
/// bindings: non-enumerable, and an assignment from user code throws.
pub(crate) fn install(it: &mut Interp) {
    let available = it.native(|_, _, _| Ok(Value::Bool(registered())));
    let exports = it.native(|_, _, _| {
        let text = match bridge() {
            // SAFETY: the registered callback returns a string this bridge owns.
            Some(b) => unsafe { take_text((b.exports)()) },
            None => String::new(),
        };
        Ok(Value::str(&text))
    });
    let call = it.native(|_, _, args| {
        let Some(b) = bridge() else {
            return Ok(Value::str(""));
        };
        let (name, payload) = (cstring(&text(args, 0)), cstring(&text(args, 1)));
        // SAFETY: both arguments are live NUL-terminated strings for the duration of the call.
        let reply = unsafe { take_text((b.call)(name.as_ptr(), payload.as_ptr())) };
        Ok(Value::str(&reply))
    });
    let handle = it.native(|_, _, args| {
        let Some(b) = bridge() else {
            return Ok(Value::str(""));
        };
        let parts =
            [text(args, 0), text(args, 1), text(args, 2), text(args, 3)].map(|t| cstring(&t));
        // SAFETY: as above.
        let reply = unsafe {
            take_text((b.handle)(
                parts[0].as_ptr(),
                parts[1].as_ptr(),
                parts[2].as_ptr(),
                parts[3].as_ptr(),
            ))
        };
        Ok(Value::str(&reply))
    });
    let owner = it.native(|_, _, args| {
        let text = match (bridge(), text(args, 0).parse::<u64>()) {
            // SAFETY: the registered callback returns a string this bridge owns.
            (Some(b), Ok(handle)) => unsafe { take_text((b.owner)(handle)) },
            _ => String::new(),
        };
        Ok(Value::str(&text))
    });
    let release = it.native(|_, _, args| {
        let released = match (bridge(), text(args, 0).parse::<u64>()) {
            (Some(b), Ok(token)) => (b.release)(token) != 0,
            _ => false,
        };
        Ok(Value::Bool(released))
    });
    let tag = it.native(|it, _, args| {
        let (Value::Object(object), native) = (arg(args, 0), text(args, 2)) else {
            return it.throw_error("TypeError", "Invalid native graph tag");
        };
        let kind = match arg(args, 1) {
            Value::Number(1.0) => napi_graph::Tag::Function,
            Value::Number(2.0) => napi_graph::Tag::Handle,
            _ => return it.throw_error("TypeError", "Invalid native graph tag"),
        };
        if !napi_graph::is_decimal(&native) {
            return it.throw_error("TypeError", "Invalid native graph tag");
        }
        napi_graph::tag(&object, kind, &native);
        Ok(Value::Undefined)
    });
    let encode = it.native(|it, _, args| graph_encode(it, args));
    let decode = it.native(|it, _, args| {
        let Some(text) = crate::napi_eval::json_stringify(it, &arg(args, 0))
            .ok()
            .flatten()
        else {
            return throw_text(it, "Invalid native JSON graph");
        };
        match napi_graph::decode_napi_in(it, &text, false) {
            Ok(value) => Ok(value),
            Err(e) => throw_text(it, &e),
        }
    });
    let decode_owned =
        it.native(
            |it, _, args| match napi_graph::decode_napi_in(it, &text(args, 0), true) {
                Ok(value) => Ok(value),
                Err(e) => throw_text(it, &e),
            },
        );
    for (name, value) in [
        ("__thaw_napi_bridge_available", available),
        ("__thaw_napi_bridge_exports", exports),
        ("__thaw_napi_bridge_call", call),
        ("__thaw_napi_bridge_handle", handle),
        ("__thaw_napi_graph_owner", owner.clone()),
        ("__thaw_napi_graph_release", release),
        ("__thaw_json_graph_owner_of_handle", owner),
        ("__thaw_napi_graph_tag", tag),
        ("__thaw_json_graph_encode_js", encode),
        ("__thaw_json_graph_decode", decode),
        ("__thaw_json_graph_decode_owned", decode_owned),
    ] {
        // Declared in the global scope, as the other host globals are: not an own property of
        // `globalThis`'s object, so `Object.keys` skips it, and an immutable binding.
        it.global.declare(name, value, false);
    }
}

/// One poll of the napi side's pending completions (run on every loop turn).
pub(crate) fn poll() {
    if let Some(b) = bridge() {
        (b.poll)();
    }
}

/// Whether the napi side still has work that keeps the loop alive.
pub(crate) fn pending() -> bool {
    bridge().is_some_and(|b| (b.pending)() != 0)
}

/// Retires the napi side: begin, then poll until ready, collecting the errors it reports, then
/// finish (repeated while finish does not succeed). Jobs the callbacks queue run on `it`.
/// Runs once per thread; a call without a registered shutdown bridge does nothing.
pub(crate) fn shutdown(it: &mut Interp) -> Result<(), String> {
    let Some(s) = *SHUTDOWN.lock().unwrap() else {
        return Ok(());
    };
    if SHUTDOWN_DONE.with(|done| done.replace(true)) {
        return Ok(());
    }
    if (s.begin)() == 0 {
        return Err("native addon shutdown could not start".into());
    }
    let mut errors = Vec::new();
    loop {
        let ready = (s.poll)();
        loop {
            let error = (s.take_error)();
            if error.is_null() {
                break;
            }
            // SAFETY: take_error hands over a string this bridge owns.
            errors.push(unsafe { take_text(error) });
        }
        if let Err(c) = it.run_jobs() {
            errors.push(classify(it, c).to_string());
        }
        if ready != 0 && (s.finish)() == 0 {
            break;
        }
        if ready == 0 {
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join("\n"))
    }
}

/// Test support: forget every registration and the shutdown latch of this thread.
#[cfg(test)]
pub(crate) fn clear_for_tests() {
    *BRIDGE.lock().unwrap() = None;
    *SHUTDOWN.lock().unwrap() = None;
    SHUTDOWN_DONE.with(|done| done.set(false));
}

/// Shutdown for the process end: `terminate_process` calls it before the process exits.
pub(crate) fn shutdown_at_exit(it: &mut Interp) {
    if let Err(message) = shutdown(it) {
        eprintln!("{message}");
    }
}
