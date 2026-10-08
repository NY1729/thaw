thread_local! {
    /// Compiled `new Function(...)` results, keyed by the JSON array of
    /// its string arguments (so repeated identical sources reuse the same
    /// JS function object instead of re-parsing/re-compiling each time).
    static DYNAMIC_FUNCTIONS: std::cell::RefCell<std::collections::HashMap<String, u64>> =
        std::cell::RefCell::new(std::collections::HashMap::new());
}

/// `new Function(...args)` via the JS realm's own `Function` constructor,
/// with a source-keyed cache of the retained function handles. `args_json`
/// is the JSON array of the constructor's string arguments (the last is
/// the body). Returns a live function handle (with an error on failure).
#[no_mangle]
pub extern "C" fn thaw_js_new_function(args_json: *const c_char) -> ThawHandleResult {
    let args_json = to_str(args_json);
    let result = with_active_or_context(|ctx| -> Result<u64, String> {
        if let Some(handle) = DYNAMIC_FUNCTIONS.with(|cache| cache.borrow().get(&args_json).copied())
        {
            return Ok(handle);
        }
        let json: Object = ctx.globals().get("JSON").map_err(|e| e.to_string())?;
        let parse: Function = json.get("parse").map_err(|e| e.to_string())?;
        let arguments: Array = parse
            .call((args_json.as_str(),))
            .map_err(|e| format!("invalid `new Function` arguments: {e}"))?;
        let constructor: Function = ctx.globals().get("Function").map_err(|e| e.to_string())?;
        let mut call_args = Args::new_unsized(ctx.clone());
        for index in 0..arguments.len() {
            let argument: Value = arguments.get(index).map_err(|e| e.to_string())?;
            call_args.push_arg(argument).map_err(|e| e.to_string())?;
        }
        let function: Value = constructor.call_arg(call_args).map_err(|error| match error {
            rquickjs::Error::Exception => describe_tagged_exception(&ctx),
            error => error.to_string(),
        })?;
        let handle = retain_value(&ctx, function)?;
        DYNAMIC_FUNCTIONS.with(|cache| cache.borrow_mut().insert(args_json.clone(), handle));
        Ok(handle)
    });
    match result {
        Ok(value) => ThawHandleResult {
            value,
            error: std::ptr::null(),
        },
        Err(error) => ThawHandleResult {
            value: 0,
            error: thaw_arena::owned_string(error),
        },
    }
}

/// A JS string decoded as its WTF-8 bytes. `rquickjs::String::to_string()`
/// rejects a lone UTF-16 surrogate (QuickJS's own `JS_ToCStringLen` emits
/// WTF-8, which is not valid UTF-8), so a native callback that receives
/// user text takes this instead of `String`.
/// Best-effort display of WTF-8 bytes: a lone surrogate becomes one U+FFFD
/// (where `str::from_utf8_lossy` would emit one per invalid byte, i.e. 3).
pub(crate) fn wtf8_display(bytes: &[u8]) -> std::borrow::Cow<'_, str> {
    if let Ok(text) = std::str::from_utf8(bytes) {
        return std::borrow::Cow::Borrowed(text);
    }
    let mut fixed = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == 0xED
            && index + 2 < bytes.len()
            && (0xA0..=0xBF).contains(&bytes[index + 1])
            && bytes[index + 2] & 0xC0 == 0x80
        {
            fixed.extend_from_slice("\u{FFFD}".as_bytes());
            index += 3;
        } else {
            fixed.push(bytes[index]);
            index += 1;
        }
    }
    std::borrow::Cow::Owned(String::from_utf8_lossy(&fixed).into_owned())
}

#[cfg_attr(not(feature = "intl"), allow(dead_code))]
pub(crate) struct Wtf8String(pub(crate) Vec<u8>);

#[cfg_attr(not(feature = "intl"), allow(dead_code))]
impl<'js> FromJs<'js> for Wtf8String {
    fn from_js(ctx: &Ctx<'js>, value: Value<'js>) -> rquickjs::Result<Self> {
        let type_name = value.type_name();
        let string = value
            .into_string()
            .ok_or_else(|| rquickjs::Error::new_from_js(type_name, "string"))?;
        let cstring = string.to_cstring()?;
        let bytes =
            unsafe { std::slice::from_raw_parts(cstring.as_ptr() as *const u8, cstring.len()) }
                .to_vec();
        let _ = ctx;
        Ok(Wtf8String(bytes))
    }
}

#[cfg(unix)]
fn native_promise_state(promise: *const c_void) -> u8 {
    unsafe {
        let poll = libc::dlsym(libc::RTLD_DEFAULT, c"thaw_runtime_poll_one".as_ptr());
        let state = libc::dlsym(libc::RTLD_DEFAULT, c"thaw_promise_state".as_ptr());
        if poll.is_null() || state.is_null() {
            return 0;
        }
        std::mem::transmute::<*mut c_void, extern "C" fn() -> u8>(poll)();
        std::mem::transmute::<*mut c_void, unsafe extern "C" fn(*const c_void) -> u8>(state)(promise)
    }
}

#[cfg(unix)]
fn native_promise_mark_handled(promise: *mut c_void) {
    unsafe {
        let mark = libc::dlsym(libc::RTLD_DEFAULT, c"thaw_promise_mark_handled".as_ptr());
        if !mark.is_null() {
            std::mem::transmute::<*mut c_void, unsafe extern "C" fn(*mut c_void) -> u8>(mark)(
                promise,
            );
        }
    }
}

#[cfg(not(unix))]
fn native_promise_state(_promise: *const c_void) -> u8 {
    0
}

#[cfg(not(unix))]
fn native_promise_mark_handled(_promise: *mut c_void) {}

/// Drains one ready native continuation or I/O event (`thaw_runtime_
/// poll_one`, dlsym'd the same soft-dependency way as `native_promise_
/// state` above -- thaw-quickjs has no hard link on thaw-runtime).
/// Unlike `native_promise_state`, this doesn't watch one specific
/// promise: it's for `thaw_js_run_event_loop`, the generic top-level
/// driver with no single completion to check, which otherwise never
/// polls this queue at all -- a native `Promise<T>` resolved from
/// inside a `setTimeout`-fired callback (the common "wrap a timer in a
/// promise" idiom) settles correctly (pushing its resume callback onto
/// `READY_CONTINUATIONS`) but nothing ever runs that callback for a
/// bare, unawaited top-level async call, which only this driver loop
/// (not `main()`'s own tracked completion) ever drives.
#[cfg(unix)]
fn poll_native_continuations() -> bool {
    unsafe {
        let poll = libc::dlsym(libc::RTLD_DEFAULT, c"thaw_runtime_poll_one".as_ptr());
        if poll.is_null() {
            return false;
        }
        std::mem::transmute::<*mut c_void, extern "C" fn() -> u8>(poll)() != 0
    }
}

#[cfg(not(unix))]
fn poll_native_continuations() -> bool {
    false
}

/// Evaluates `source` in the (per-thread) global QuickJS context. Top-level
/// function declarations become callable afterwards via `thaw_js_call`.
/// Returns `1` on success, `0` on failure (syntax error, thrown exception).
#[no_mangle]
pub extern "C" fn thaw_js_load(source: *const c_char) -> u8 {
    let source = to_str(source);
    let source = match source.strip_prefix("gz:") {
        Some(encoded) => base64::engine::general_purpose::STANDARD
            .decode(encoded)
            .ok()
            .and_then(|compressed| {
                let mut decoded = String::new();
                flate2::read::GzDecoder::new(compressed.as_slice())
                    .read_to_string(&mut decoded)
                    .ok()
                    .map(|_| decoded)
            })
            .unwrap_or(source),
        None => source,
    };
    with_context(|ctx| match load_impl(ctx, &source) {
        Ok(()) => 1,
        Err(reason) => {
            eprintln!("thaw-quickjs: failed to load script: {reason}");
            0
        }
    })
}

/// Runs Node-compatible eval and CommonJS file forms when this compiled
/// executable starts itself through `process.execPath`. Returns `-1` for an
/// ordinary invocation.
#[no_mangle]
pub extern "C" fn thaw_js_run_cli() -> i32 {
    let arguments = std::env::args().collect::<Vec<_>>();
    let source = if let Some((_, source)) = cli_eval(&arguments) {
        let Some(source) = source else {
            eprintln!("{}: -e requires an argument", arguments[0]);
            return 1;
        };
        source.to_string()
    } else if let Some((_, script)) = cli_script(&arguments) {
        format!(
            "globalThis.__thaw_run_main_file({})",
            json_escape_string(script)
        )
    } else {
        return -1;
    };
    let source = CString::new(source).unwrap_or_default();
    if thaw_js_load(source.as_ptr()) == 0 {
        1
    } else {
        thaw_js_run_event_loop()
    }
}

fn process_emit_result(ctx: &Ctx<'_>, result: rquickjs::Result<bool>) -> ThawHandleResult {
    match result {
        Ok(handled) => ThawHandleResult { value: u64::from(handled), error: std::ptr::null() },
        Err(error) => {
            let error = match error {
                rquickjs::Error::Exception => describe_tagged_exception(ctx),
                other => other.to_string(),
            };
            ThawHandleResult {
                value: 0,
                error: thaw_arena::owned_string(error),
            }
        }
    }
}

fn legacy_process_emit(result: ThawHandleResult) -> u8 {
    // Legacy public exports keep their historical scalar ABI. Internal callers
    // use the result variants so they can report a thrown listener separately.
    if !result.error.is_null() {
        unsafe { thaw_arena::destroy_string(result.error.cast_mut()) };
    }
    result.value as u8
}

/// Emits a compiled uncaught exception and preserves a thrown JS listener.
#[no_mangle]
pub extern "C" fn thaw_js_emit_uncaught_result(message: *const c_char) -> ThawHandleResult {
    if message.is_null() {
        return ThawHandleResult { value: 0, error: std::ptr::null() };
    }
    let raw = unsafe { thaw_arena::NativeStr::from_ptr(message) }
        .to_string_lossy()
        .into_owned();
    let (name, message) = tagged_error_parts(&raw);
    let name = json_escape_string(name);
    let message = json_escape_string(message);
    with_active_or_context(|ctx| process_emit_result(&ctx, ctx.eval::<bool, _>(format!(
        "typeof process !== 'undefined' && (() => {{ const error = new Error({message}); error.name = {name}; process.emit('uncaughtExceptionMonitor', error, 'uncaughtException'); return process.emit('uncaughtException', error, 'uncaughtException'); }})()"
    ))))
}

#[no_mangle]
pub extern "C" fn thaw_js_emit_uncaught(message: *const c_char) -> u8 {
    legacy_process_emit(thaw_js_emit_uncaught_result(message))
}

/// Emits a native Promise rejection and preserves a thrown JS listener.
#[no_mangle]
pub extern "C" fn thaw_js_emit_unhandled_rejection_result(message: *const c_char) -> ThawHandleResult {
    if message.is_null() {
        return ThawHandleResult { value: 0, error: std::ptr::null() };
    }
    let raw = unsafe { thaw_arena::NativeStr::from_ptr(message) }
        .to_string_lossy()
        .into_owned();
    let (name, message) = tagged_error_parts(&raw);
    let name = json_escape_string(name);
    let message = json_escape_string(message);
    with_active_or_context(|ctx| process_emit_result(&ctx, ctx.eval::<bool, _>(format!(
        "typeof process !== 'undefined' && (() => {{ const error = new Error({message}); error.name = {name}; return process.emit('unhandledRejection', error, undefined); }})()"
    ))))
}

#[no_mangle]
pub extern "C" fn thaw_js_emit_unhandled_rejection(message: *const c_char) -> u8 {
    legacy_process_emit(thaw_js_emit_unhandled_rejection_result(message))
}

/// Emits Node's notification that a previously-unhandled Promise gained a handler.
#[no_mangle]
pub extern "C" fn thaw_js_emit_rejection_handled_result() -> ThawHandleResult {
    with_active_or_context(|ctx| {
        // ponytail: the Promise argument stays undefined until native Promises have JS wrappers.
        process_emit_result(&ctx, ctx.eval::<bool, _>(
            "typeof process !== 'undefined' && process.emit('rejectionHandled', undefined)",
        ))
    })
}

#[no_mangle]
pub extern "C" fn thaw_js_emit_rejection_handled() {
    let _ = legacy_process_emit(thaw_js_emit_rejection_handled_result());
}

fn framed_error(name: &str, display: &str, original: Option<&str>) -> String {
    let bytes = thaw_arena::error_wire::encode_tagged(
        name.as_bytes(), display.as_bytes(), original.map(str::as_bytes),
    ).expect("Rust strings are valid UTF-8 error text");
    String::from_utf8(bytes).expect("framed Rust error text is UTF-8")
}

fn tagged_error_parts(raw: &str) -> (&str, &str) {
    if let Some(frame) = thaw_arena::error_wire::parse_tagged(raw.as_bytes()) {
        if let (Ok(name), Ok(display)) = (
            std::str::from_utf8(frame.chain), std::str::from_utf8(frame.display),
        ) { return (name, display); }
    }
    raw.strip_prefix('\u{1}')
        .and_then(|tagged| tagged.split_once('\u{1}'))
        .unwrap_or(("Error", raw))
}

/// Real error message for a failed `eval`, extracted the same way
/// `call_impl` already does for a thrown exception -- without this,
/// `rquickjs::Error::Exception`'s own `Display` is just a generic
/// placeholder ("Exception generated by QuickJS"), which made a real
/// npm package's load-time failure (e.g. a top-level `require(...)` call
/// this crate's stub rejects, see thaw-bridge's `wrap_as_commonjs_module`)
/// undiagnosable.
fn load_impl(ctx: Ctx<'_>, source: &str) -> Result<(), String> {
    // Loaded scripts are CommonJS/classic scripts: sloppy unless they opt in with "use strict".
    let mut options = rquickjs::context::EvalOptions::default();
    options.strict = false;
    ctx.eval_with_options::<(), _>(source, options).map_err(|e| match e {
        rquickjs::Error::Exception => describe_exception_with_stack(&ctx),
        e => e.to_string(),
    })?;
    if let Ok(ready) = ctx
        .globals()
        .get::<_, rquickjs::Promise>("__thaw_module_ready")
    {
        finish_with_platform_events(&ctx, &ready).map_err(|error| match error {
            rquickjs::Error::Exception => describe_exception(&ctx),
            error => format!("module initialization failed: {error}"),
        })?;
        loop {
            drain_next_tick_queue(&ctx).map_err(|error| match error {
                rquickjs::Error::Exception => describe_exception_with_stack(&ctx),
                error => error.to_string(),
            })?;
            if !ctx.execute_pending_job() {
                break;
            }
        }
        let _ = ctx
            .globals()
            .set("__thaw_module_ready", Value::new_undefined(ctx.clone()));
    }
    Ok(())
}

/// Evaluates a script in the shared embedded context and returns its result as
/// JSON. JavaScript values for which `JSON.stringify` returns `undefined`
/// produce `None`; exceptions retain their JavaScript message.
pub fn eval_json(source: &str) -> Result<Option<String>, String> {
    with_context(|ctx| {
        let value = ctx
            .eval::<Value<'_>, _>(source)
            .map_err(|error| match error {
                rquickjs::Error::Exception => describe_exception(&ctx),
                error => error.to_string(),
            })?;
        ctx.json_stringify(value)
            .map(|value| value.map(|value| value.to_string().unwrap_or_default()))
            .map_err(|error| match error {
                rquickjs::Error::Exception => describe_exception(&ctx),
                error => error.to_string(),
            })
    })
}

/// Calls a top-level function (previously loaded via `thaw_js_load`) named
/// `func_name`, with `args_json` a JSON-encoded array of arguments.
/// Returns the JSON-encoded result (or an error object -- see the module
/// doc comment).
#[no_mangle]
pub extern "C" fn thaw_js_call(
    func_name: *const c_char,
    args_json: *const c_char,
) -> *const c_char {
    let func_name = to_str(func_name);
    let args_json = to_str(args_json);

    let text = with_context(|ctx| match call_impl(ctx, &func_name, &args_json) {
        Ok(text) => text,
        Err(reason) => format!("{{\"__thaw_error__\":{}}}", json_escape_string(&reason)),
    });

    CString::new(text).unwrap_or_default().into_raw() as *const c_char
}

#[no_mangle]
/// Calls a QuickJS function retained by the N-API argument bridge.
///
/// # Safety
///
/// `context` must contain a reference identifier previously issued by the
/// bridge and `args_json` must point to a live NUL-terminated string.
pub unsafe extern "C" fn thaw_js_call_reference(
    context: *mut std::ffi::c_void,
    args_json: *const c_char,
) -> *const c_char {
    let name = format!("__thaw_napi_reference_{}", context as usize);
    let args_json = to_str(args_json);
    let text = ACTIVE_NAPI_CONTEXT.with(|active| {
        let active = active.get();
        if active.is_null() {
            with_context(|ctx| match call_impl(ctx, &name, &args_json) {
                Ok(text) => text,
                Err(reason) => {
                    format!("\u{2}{}", json_escape_string(&reason))
                }
            })
        } else {
            match call_impl(
                (*(active as *const Ctx<'static>)).clone(),
                &name,
                &args_json,
            ) {
                Ok(text) => text,
                Err(reason) => {
                    format!("\u{2}{}", json_escape_string(&reason))
                }
            }
        }
    });
    CString::new(text).unwrap_or_default().into_raw()
}

/// Result-ABI companion to [`thaw_js_call`]. Unlike the legacy JSON error
/// object API, failures occupy the error channel so generated Thaw code can
/// route JavaScript throws and Promise rejections through `try`/`catch`.
#[no_mangle]
pub extern "C" fn thaw_js_call_result(
    func_name: *const c_char,
    args_json: *const c_char,
) -> ThawResult {
    let func_name = to_str(func_name);
    let args_json = to_str(args_json);
    match with_active_or_context(|ctx| call_impl(ctx, &func_name, &args_json)) {
        Ok(text) => ThawResult {
            value: thaw_arena::owned_string(text),
            error: std::ptr::null(),
        },
        Err(reason) => ThawResult {
            value: std::ptr::null(),
            error: thaw_arena::owned_string(reason),
        },
    }
}

#[no_mangle]
pub extern "C" fn thaw_js_call_graph_result(
    func_name: *const c_char,
    args_json: *const c_char,
) -> ThawResult {
    let func_name = to_str(func_name);
    let args_json = to_str(args_json);
    let result = with_active_or_context(|ctx| {
        let target: Function = before_graph_decode(&ctx, &args_json, true, || {
            ctx.globals().get(func_name.as_str())
                .map_err(|_| format!("no such function `{func_name}` (was it loaded via loadScript?)"))
        })?;
        invoke_impl(ctx, target, &func_name, &args_json, true, true)
    });
    match result {
        Ok(text) => ThawResult { value: thaw_arena::owned_string(text), error: std::ptr::null() },
        Err(reason) => ThawResult {
            value: std::ptr::null(),
            error: thaw_arena::owned_string(reason),
        },
    }
}

fn call_impl(ctx: Ctx<'_>, func_name: &str, args_json: &str) -> Result<String, String> {
    let target: Function = ctx
        .globals()
        .get(func_name)
        .map_err(|_| format!("no such function `{func_name}` (was it loaded via loadScript?)"))?;
    invoke_impl(ctx, target, func_name, args_json, false, false)
}

fn decode_argument_array<'js>(ctx: &Ctx<'js>, text: &str, graph: bool) -> Result<Array<'js>, String> {
    if graph {
        let decode: Function = before_graph_decode(ctx, text, true, || {
            ctx.globals().get("__thaw_json_graph_decode_owned")
                .map_err(|error| error.to_string())
        })?;
        decode.call((text,)).map_err(|error| match error {
            rquickjs::Error::Exception => describe_tagged_exception(ctx),
            error => error.to_string(),
        })
    } else {
        let json: Object = ctx.globals().get("JSON").map_err(|error| error.to_string())?;
        let parse: Function = json.get("parse").map_err(|error| error.to_string())?;
        let reviver: Function = ctx.globals().get("__thaw_json_date_reviver")
            .map_err(|error| error.to_string())?;
        parse.call((text, reviver)).map_err(|error| error.to_string())
    }
}

// Native graph wires transfer one registry reference per lease before the
// callee or property lookup. The normal decoder owns those references once
// reached; a failed lookup must return them without constructing the graph.
fn release_unconsumed_graph_leases(ctx: &Ctx<'_>, text: &str) {
    let Ok(graph) = serde_json::from_str::<serde_json::Value>(text) else {
        return;
    };
    if let Some(leases) = graph.get("leases").and_then(serde_json::Value::as_array) {
        for handle in leases.iter().filter_map(serde_json::Value::as_u64).filter(|handle| *handle != 0) {
            let _ = release_handle_in_context(ctx, handle);
        }
    }
    let release_napi = NAPI_BRIDGE.lock().unwrap().as_ref().map(|bridge| bridge.6);
    if let (Some(release), Some(leases)) = (release_napi,
        graph.get("napiLeases").and_then(serde_json::Value::as_array)) {
        for reference in leases.iter().filter_map(serde_json::Value::as_str)
            .filter_map(|reference| reference.parse::<u64>().ok())
            .filter(|reference| *reference != 0) {
            release(reference);
        }
    }
}

// The JS graph encoder has already retained the leases named by this wire.
// If arena ownership cannot be established, return every producer-owned lease
// before dropping the only bytes that identify them.
fn owned_graph_wire_with_registration(
    text: &str,
    register: impl FnOnce(*const c_char) -> bool,
) -> *mut c_char {
    let wire = thaw_arena::owned_string(text);
    if wire.is_null() || !register(wire) {
        if !wire.is_null() { unsafe { thaw_arena::destroy_string(wire) }; }
        with_active_or_context(|ctx| release_unconsumed_graph_leases(&ctx, text));
        return std::ptr::null_mut();
    }
    wire
}

fn owned_graph_wire(text: &str) -> *mut c_char {
    owned_graph_wire_with_registration(text, |wire| unsafe {
        thaw_arena::register_owned_graph_wire(wire)
    })
}

fn before_graph_decode<'js, T>(
    ctx: &Ctx<'js>, text: &str, graph: bool, lookup: impl FnOnce() -> Result<T, String>,
) -> Result<T, String> {
    lookup().inspect_err(|_error| {
        if graph {
            release_unconsumed_graph_leases(ctx, text);
        }
    })
}

fn before_registered_graph_decode<T>(
    wire: *const c_char,
    lookup: impl FnOnce() -> Result<T, String>,
) -> Result<T, String> {
    lookup().inspect_err(|_error| {
        // Exact mixed graph wires are already registered by their native
        // producer. Consume that grant on lookup failures before dispatch.
        unsafe extern "C" { fn thaw_json_discard_graph_wire(source: *const c_char); }
        unsafe { thaw_json_discard_graph_wire(wire) };
    })
}

fn invoke_impl<'js>(
    ctx: Ctx<'js>,
    target: Function<'js>,
    label: &str,
    args_json: &str,
    preserve_error: bool,
    graph_result: bool,
) -> Result<String, String> {
    let args_array = decode_argument_array(&ctx, args_json, graph_result)?;

    let mut call_args = Args::new_unsized(ctx.clone());
    for i in 0..args_array.len() {
        let arg: Value = args_array.get(i).map_err(|error| error.to_string())?;
        call_args.push_arg(arg).map_err(|error| error.to_string())?;
    }
    let result: Value = target.call_arg(call_args).map_err(|e| match e {
        rquickjs::Error::Exception if preserve_error => describe_tagged_exception(&ctx),
        rquickjs::Error::Exception => describe_host_exception(&ctx, label),
        e => format!("`{label}` threw: {e}"),
    })?;

    if graph_result {
        resolve_value_graph_impl(ctx, result, label, preserve_error)
    } else {
        resolve_value_impl(ctx, result, label, preserve_error)
    }
}

/// Always passes `result` through the realm's Promise resolution
/// procedure, returning the *resolved* value (not JSON-encoded --
/// callers that need JSON should go through `resolve_value_impl`
/// instead, which wraps this). `Value::as_promise` only recognizes
/// native Promise objects; `Promise.resolve` also assimilates arbitrary
/// foreign thenables, handles throwing `then` accessors/calls, and obeys
/// the first-settlement-wins rule required by JavaScript.
///
/// Needed anywhere a result gets **retained as a live handle** rather
/// than JSON-encoded (`callDynamicMethodHandle`'s own native side,
/// `thaw_js_call_method_handle_result`) -- without this, an async
/// method's real result (a still-pending Promise, e.g. real hono's own
/// `app.request(...)`/`.fetch(...)`) would get retained *as the Promise
/// itself*, not the value it resolves to, and reading a property off it
/// afterward (`res.status`) silently reads `undefined` off the wrong
/// object instead of erroring -- confirmed via a direct repro. Thaw's own
/// `await` keyword doesn't help here either: it's a no-op pass-through
/// for anything but `sleep(...)` (`compile_await`'s own doc comment --
/// "User-defined async functions still use the V1 synchronous ABI"),
/// since it doesn't know a `JsValue` might represent a pending QuickJS-
/// side Promise at all.
fn resolve_promise_value<'js>(
    ctx: &Ctx<'js>,
    result: Value<'js>,
    label: &str,
    preserve_error: bool,
) -> Result<Value<'js>, String> {
    let to_string_err = |e: rquickjs::Error| e.to_string();
    let assimilate: Function = ctx
        .eval("(value) => Promise.resolve(value)")
        .map_err(to_string_err)?;
    let promise: rquickjs::Promise = assimilate.call((result,)).map_err(|e| match e {
        rquickjs::Error::Exception => {
            format!(
                "`{label}` could not resolve its result: {}",
                describe_exception(ctx)
            )
        }
        e => format!("`{label}` could not resolve its result: {e}"),
    })?;
    finish_with_platform_events(ctx, &promise).map_err(|e| match e {
        rquickjs::Error::Exception => describe_promise_exception(ctx, label, preserve_error),
        e => format!("`{label}`'s promise rejected or stalled: {e}"),
    })
}

fn resolve_value_impl<'js>(
    ctx: Ctx<'js>,
    result: Value<'js>,
    label: &str,
    preserve_error: bool,
) -> Result<String, String> {
    let result = resolve_promise_value(&ctx, result, label, preserve_error)?;

    // `ctx.json_stringify` (rather than calling the JS `JSON.stringify`
    // function directly and coercing its return value straight to a Rust
    // `String`) surfaces `JSON.stringify`'s "not representable" case as
    // `Ok(None)` instead of a real JS `undefined` value that a direct
    // `String` conversion would just fail on. That case is not rare here:
    // it's exactly what a genuinely `void`-returning (or otherwise
    // undefined-returning) Fallback function's real result looks like.
    //
    // Substituted with the same `$__thaw_napi_undefined$`-tagged sentinel
    // object native-callback argument marshaling already uses for a real
    // `undefined` (`compile_napi_undefined_json`, thaw-llvm), not bare
    // `null` -- every typed decoder that actually cares about the
    // distinction (`Optional`/`Nullable`/`Nullish`/`Union`, and, since
    // this session's earlier `Json`-representation fix, `thaw_json_
    // typeof`/`as_bool`/`as_string` too) already recognizes this exact
    // shape and reports "genuinely undefined" correctly; one that
    // doesn't care (`Void`, which never inspects the JSON it's handed;
    // `Bool`, already falsy for either shape) is unaffected either way.
    // Previously: a Fallback function declared to return `T | undefined`
    // whose real JS implementation actually returned `undefined` was
    // always misread back as `null` (or, worse, as a mis-decoded `T`)
    // on the native side -- real example: any `Optional`-returning
    // package function, found via a synthetic reproduction while fixing
    // validator's union-return crash.
    let stringify: Function = ctx
        .globals()
        .get("__thaw_json_safe_stringify")
        .map_err(|e| format!("failed to JSON-encode the result: {e}"))?;
    stringify
        .call::<_, Option<String>>((result,))
        .map_err(|e| format!("failed to JSON-encode the result: {e}"))
        .map(|value| {
            value.unwrap_or_else(|| r#"{"$__thaw_napi_undefined$":true}"#.to_string())
        })
}

// Private result wire for compiled calls. Graph node tags identify real
// Date/undefined/non-finite values independently of ordinary user fields;
// the public plain-JSON result ABI above remains unchanged.
fn resolve_value_wire_impl<'js>(
    ctx: Ctx<'js>, result: Value<'js>, label: &str,
    preserve_error: bool, graph_result: bool,
) -> Result<String, String> {
    if graph_result { resolve_value_graph_impl(ctx, result, label, preserve_error) }
    else { resolve_value_impl(ctx, result, label, preserve_error) }
}

fn resolve_value_graph_impl<'js>(
    ctx: Ctx<'js>,
    result: Value<'js>,
    label: &str,
    preserve_error: bool,
) -> Result<String, String> {
    let result = resolve_promise_value(&ctx, result, label, preserve_error)?;
    let encode: Function = ctx.globals().get("__thaw_json_graph_encode_js")
        .map_err(|error| error.to_string())?;
    encode.call((result, 0, false, true)).map_err(|error| match error {
        rquickjs::Error::Exception => describe_tagged_exception(&ctx),
        error => error.to_string(),
    })
}

fn finish_with_platform_events<'js>(
    ctx: &Ctx<'js>,
    promise: &rquickjs::Promise<'js>,
) -> rquickjs::Result<Value<'js>> {
    loop {
        dispatch_pending_process_signal(ctx);
        if let Ok(poll_platform_events) = ctx
            .globals()
            .get::<_, Function>("__thaw_poll_platform_events")
        {
            poll_platform_events.call::<_, ()>(())?;
        }
        poll_napi_bridge(ctx);
        if let Some(result) = promise.result() {
            return result;
        }
        // A nextTick callback may be exactly what resolves `promise` --
        // loop back to the `promise.result()` check above immediately
        // rather than falling through to the exhaustion check below,
        // which would otherwise misreport a just-resolved promise as
        // deadlocked (`Error::WouldBlock`) whenever nothing else happens
        // to be scheduled.
        if drain_next_tick_queue(ctx)? {
            continue;
        }
        if ctx.execute_pending_job() {
            continue;
        }

        let next_delay: Function = ctx.globals().get("__thaw_next_timer_delay")?;
        let delay: i64 = next_delay.call(())?;
        if delay < 0 {
            if platform_activity_pending(ctx) {
                std::thread::sleep(Duration::from_millis(1));
                continue;
            }
            return Err(rquickjs::Error::WouldBlock);
        }
        if delay > 0 {
            std::thread::sleep(Duration::from_millis(delay as u64));
        }
        let run_due: Function = ctx.globals().get("__thaw_run_due_timers")?;
        run_due.call::<_, usize>(())?;
    }
}

fn platform_activity_pending(ctx: &Ctx<'_>) -> bool {
    [
        "__thaw_worker_active",
        "__thaw_child_process_active",
        "__thaw_stdin_active",
    ]
        .into_iter()
        .any(|name| {
            ctx.globals()
                .get::<_, Function>(name)
                .ok()
                .and_then(|probe| probe.call::<_, bool>(()).ok())
                .unwrap_or(false)
        })
        || napi_bridge_pending()
}

/// A root node:test run closes only after the host event loop has no work.
/// The registry callback returns null until a root test or hook was declared,
/// and a Promise exactly once when its root after hooks need to finish.
fn finish_root_test_hooks_at_idle(ctx: &Ctx<'_>) -> rquickjs::Result<bool> {
    let Ok(finalize) = ctx.globals().get::<_, Function>("__thaw_test_finalize_root") else {
        return Ok(false);
    };
    let result: Value = finalize.call(())?;
    let Some(promise) = result.as_promise() else { return Ok(false); };
    finish_with_platform_events(ctx, promise)?;
    Ok(true)
}

#[no_mangle]
pub extern "C" fn thaw_js_run_event_loop() -> i32 {
    with_context(|ctx| loop {
        dispatch_pending_process_signal(&ctx);
        if let Ok(poll) = ctx
            .globals()
            .get::<_, Function>("__thaw_poll_platform_events")
        {
            let _ = poll.call::<_, ()>(());
        }
        poll_napi_bridge(&ctx);
        {
            // A resumed continuation may itself need to make a further
            // dynamic call back into this same context (real example:
            // `fs.existsSync`/`fs.readFileSync` after an `@isaacs/fs-
            // minipass` `WriteStream`'s `close` event resolves the
            // native `Promise<T>` this loop just woke up) -- entering
            // `ActiveNapiContext` here (matching `thaw_js_run_until_
            // native_resolved`'s identical, already-working pattern)
            // lets that reentrant call reuse this active `Ctx` via
            // `with_active_or_context` instead of calling `with_context`
            // again, which would panic ("RefCell already borrowed")
            // trying to re-lock a context this same closure already
            // holds -- confirmed via a real crash without this guard.
            let _active = ActiveNapiContext::enter(&ctx);
            while poll_native_continuations() {}
        }
        // Tracks whether the drain below actually ran a next-tick batch
        // or a microtask job -- either can itself resolve a native
        // `Promise<T>` (real example: a `close`/`ready` event fired via
        // `process.nextTick` from a native `fs`-backed stream,
        // `@isaacs/fs-minipass`'s own `WriteStream`), which needs another
        // pass through `poll_native_continuations()` to actually run the
        // now-ready resume callback. Loops back to the *top* of the
        // outer loop instead of polling again from here directly --
        // found, via a real crash, that re-entering `poll_native_
        // continuations`'s own callback dispatch from this inner call
        // frame reenters a `RefCell` this same `with_context` closure
        // already holds; looping back reuses the already-proven-safe
        // poll call above instead of adding a second call site.
        let mut drained_any = false;
        loop {
            match drain_next_tick_queue(&ctx) {
                Ok(true) => drained_any = true,
                Ok(false) => {}
                Err(error) => {
                    let message = if matches!(error, rquickjs::Error::Exception) {
                        describe_exception_with_stack(&ctx)
                    } else {
                        error.to_string()
                    };
                    eprintln!("{message}");
                    return 1;
                }
            }
            let ran_job = ctx.execute_pending_job();
            if ctx.has_exception() {
                let message = describe_exception_with_stack(&ctx);
                let text = thaw_arena::owned_string(message.as_bytes());
                let _active = ActiveNapiContext::enter(&ctx);
                let result = thaw_js_emit_uncaught_result(text);
                unsafe { thaw_arena::destroy_string(text) };
                let listener_failed = !result.error.is_null();
                if listener_failed {
                    eprintln!("{}", unsafe { thaw_arena::NativeStr::from_ptr(result.error) }.to_string_lossy());
                    unsafe { thaw_arena::destroy_string(result.error.cast_mut()) };
                }
                if result.value == 0 || listener_failed {
                    eprintln!("{message}");
                    return 1;
                }
                drained_any = true;
                continue;
            }
            if ran_job {
                drained_any = true;
            } else {
                break;
            }
        }
        if drained_any {
            continue;
        }
        let Ok(next_delay) = ctx.globals().get::<_, Function>("__thaw_next_timer_delay") else {
            return process_exit_code(&ctx);
        };
        let Ok(delay) = next_delay.call::<_, i64>(()) else {
            return process_exit_code(&ctx);
        };
        if delay < 0 {
            if platform_activity_pending(&ctx) {
                std::thread::sleep(Duration::from_millis(1));
                continue;
            }
            match finish_root_test_hooks_at_idle(&ctx) {
                Ok(true) => continue,
                Ok(false) => {}
                Err(error) => {
                    eprintln!("root test finalization failed: {error}");
                    return 1;
                }
            }
            return process_exit_code(&ctx);
        }
        if delay > 0 {
            std::thread::sleep(Duration::from_millis(delay as u64));
        }
        let Ok(run_due) = ctx.globals().get::<_, Function>("__thaw_run_due_timers") else {
            return process_exit_code(&ctx);
        };
        if let Err(error) = run_due.call::<_, usize>(()) {
            let message = if matches!(error, rquickjs::Error::Exception) {
                describe_exception_with_stack(&ctx)
            } else {
                error.to_string()
            };
            eprintln!("{message}");
            return 1;
        }
    })
}

/// Reports queued work independently of `thaw_js_run_event_loop`'s exit
/// status. A failed job can leave later jobs queued, while `exitCode` can
/// remain nonzero after all work is complete.
#[no_mangle]
pub extern "C" fn thaw_js_terminal_work_pending() -> u8 {
    with_context(|ctx| {
        let runtime = unsafe { rquickjs::qjs::JS_GetRuntime(ctx.as_raw().as_ptr()) };
        let jobs = unsafe { rquickjs::qjs::JS_IsJobPending(runtime) };
        let globals = ctx.globals();
        let ticks = globals.get::<_, Function>("__thaw_next_tick_queue_pending")
            .ok().and_then(|pending| pending.call::<_, bool>(()).ok()).unwrap_or(false);
        let timers = globals.get::<_, Function>("__thaw_next_timer_delay")
            .ok().and_then(|delay| delay.call::<_, i64>(()).ok()).is_some_and(|delay| delay >= 0);
        u8::from(jobs || ticks || timers || platform_activity_pending(&ctx))
    })
}

/// Interleaves QuickJS platform work with a native Promise until it settles.
#[no_mangle]
pub extern "C" fn thaw_js_run_until_native_resolved(promise: *const c_void) {
    with_context(|ctx| loop {
        let state = {
            let _active = ActiveNapiContext::enter(&ctx);
            native_promise_state(promise)
        };
        if state != 0 {
            return;
        }
        dispatch_pending_process_signal(&ctx);
        if let Ok(poll) = ctx
            .globals()
            .get::<_, Function>("__thaw_poll_platform_events")
        {
            let _ = poll.call::<_, ()>(());
        }
        poll_napi_bridge(&ctx);
        // This FFI function returns `()` with no error-reporting path
        // of its own -- matches the same silent-ignore convention the
        // neighboring `poll_platform_events` call above already uses
        // for this specific function, unlike `thaw_js_run_event_loop`/
        // `load_impl`, which do have one and use it. Still needs to act
        // on a successful drain, though: a nextTick callback may be
        // exactly what settles `promise` (checked via `native_promise_
        // state` at the top of this same loop), so loop back
        // immediately rather than falling through to the timer/
        // platform-activity exhaustion check below, which would
        // otherwise silently give up on an already-settled promise.
        if drain_next_tick_queue(&ctx).unwrap_or(false) {
            continue;
        }
        if ctx.execute_pending_job() {
            continue;
        }
        let Ok(next_delay) = ctx.globals().get::<_, Function>("__thaw_next_timer_delay") else {
            return;
        };
        let Ok(delay) = next_delay.call::<_, i64>(()) else {
            return;
        };
        if delay < 0 {
            if platform_activity_pending(&ctx) {
                std::thread::sleep(Duration::from_millis(1));
                continue;
            }
            return;
        }
        if delay > 0 {
            std::thread::sleep(Duration::from_millis(delay as u64));
        }
        let Ok(run_due) = ctx.globals().get::<_, Function>("__thaw_run_due_timers") else {
            return;
        };
        let _ = run_due.call::<_, usize>(());
    });
}

/// Retains a global JavaScript value in the realm and returns a stable opaque
/// handle. Zero denotes failure or a missing value.
#[no_mangle]
pub extern "C" fn thaw_js_get_global(name: *const c_char) -> u64 {
    let name = to_str(name);
    with_active_or_context(|ctx| {
        // The whole `name` as a single, literal global property first --
        // this is the *only* lookup that existed before, and must stay
        // that way: some real package names contain a literal `.` of
        // their own (e.g. `socket.io`), so a runtime key derived from one
        // (a qualified alias, `pkg::$value$name`, ...) can itself contain
        // a `.` that was never meant as a namespace separator. Only if
        // no such literal global exists does a `.` in `name` get treated
        // as a nested-property path (`"Intl.DateTimeFormat"` -- needed
        // for `new <Namespace>.<Class>(...)`, thaw-hir's own lowering
        // for this constructs exactly this dotted string) -- `Intl`
        // itself is a real global, but `"Intl.DateTimeFormat"` never is.
        // A plain property lookup for a missing key succeeds with
        // `Value::Undefined` rather than erroring (ordinary JS property
        // access semantics) -- so a name that's genuinely only reachable
        // via the nested-path walk below (`"Intl.DateTimeFormat"`, no
        // literal global by that exact dotted name) must *not* stop here
        // just because this lookup "succeeded".
        if let Ok(value) = ctx.globals().get::<_, Value>(name.as_str()) {
            if !value.is_undefined() {
                return retain_value(&ctx, value).unwrap_or(0);
            }
        }
        let mut segments = name.split('.');
        let Some(first) = segments.next() else {
            return 0;
        };
        let Ok(mut value) = ctx.globals().get::<_, Value>(first) else {
            return 0;
        };
        for segment in segments {
            let Some(object) = value.as_object() else {
                return 0;
            };
            let Ok(next) = object.get::<_, Value>(segment) else {
                return 0;
            };
            value = next;
        }
        retain_value(&ctx, value).unwrap_or(0)
    })
}

/// Synchronizes the one Lambda-owned trace variable with the already-live
/// JavaScript `process.env` snapshot before a compiled handler runs.
#[no_mangle]
pub extern "C" fn thaw_js_sync_lambda_trace_from_env() -> u8 {
    let trace = match std::env::var("_X_AMZN_TRACE_ID") {
        Ok(value) => Some(value),
        Err(std::env::VarError::NotPresent) => None,
        Err(std::env::VarError::NotUnicode(_)) => return 0,
    };
    with_active_or_context(|ctx| {
        let result = ctx
            .globals()
            .get::<_, Object>("process")
            .and_then(|process| process.get::<_, Object>("env"))
            .and_then(|env| match trace.as_deref() {
                Some(value) => env.set("_X_AMZN_TRACE_ID", value),
                None => env.remove("_X_AMZN_TRACE_ID"),
            });
        match result {
            Ok(()) => 1,
            Err(rquickjs::Error::Exception) => {
                // The setter may throw. Consume QuickJS's pending exception
                // before the next warm invocation enters the same context.
                let _ = ctx.catch();
                0
            }
            Err(_) => 0,
        }
    })
}

#[no_mangle]
pub extern "C" fn thaw_js_set_process_env(name: *const c_char, value: *const c_char) -> u8 {
    let name = to_str(name);
    let value = to_str(value);
    with_active_or_context(|ctx| {
        let result = ctx
            .globals()
            .get::<_, Object>("process")
            .and_then(|process| process.get::<_, Object>("env"))
            .and_then(|env| env.set(name.as_str(), value));
        u8::from(result.is_ok())
    })
}

fn process_exit_code(ctx: &Ctx<'_>) -> i32 {
    ctx.eval("process.exitCode == null ? 0 : Number(process.exitCode)")
        .unwrap_or(0)
}

#[no_mangle]
/// Encodes UTF-16 code units as WTF-8 (a lone surrogate kept as its
/// 3-byte sequence) so a JS string can cross into a native string without
/// `rquickjs`'s own Rust-`String` conversion rejecting it.
fn wtf8_from_units(units: &[u16]) -> Vec<u8> {
    fn push(bytes: &mut Vec<u8>, code: u32) {
        if code < 0x80 {
            bytes.push(code as u8);
        } else if code < 0x800 {
            bytes.push(0xC0 | (code >> 6) as u8);
            bytes.push(0x80 | (code & 0x3F) as u8);
        } else if code < 0x10000 {
            bytes.push(0xE0 | (code >> 12) as u8);
            bytes.push(0x80 | ((code >> 6) & 0x3F) as u8);
            bytes.push(0x80 | (code & 0x3F) as u8);
        } else {
            bytes.push(0xF0 | (code >> 18) as u8);
            bytes.push(0x80 | ((code >> 12) & 0x3F) as u8);
            bytes.push(0x80 | ((code >> 6) & 0x3F) as u8);
            bytes.push(0x80 | (code & 0x3F) as u8);
        }
    }
    let mut bytes = Vec::with_capacity(units.len());
    let mut index = 0;
    while index < units.len() {
        let unit = units[index];
        if (0xD800..=0xDBFF).contains(&unit)
            && index + 1 < units.len()
            && (0xDC00..=0xDFFF).contains(&units[index + 1])
        {
            let code = 0x10000
                + ((u32::from(unit) - 0xD800) << 10)
                + (u32::from(units[index + 1]) - 0xDC00);
            push(&mut bytes, code);
            index += 2;
        } else {
            push(&mut bytes, u32::from(unit));
            index += 1;
        }
    }
    bytes
}

/// `String(value)` as WTF-8 bytes: the JS string's code units are read
/// out one at a time (via `charCodeAt`) so a lone surrogate survives --
/// `rquickjs`'s direct `String` -> Rust `String` conversion rejects it.
fn js_handle_string_wtf8<'js>(ctx: &Ctx<'js>, value: rquickjs::Value<'js>) -> Result<Vec<u8>, String> {
    let format: Function = ctx
        .eval(
            "(value) => { const s = String(value); const parts = []; \
             for (let i = 0; i < s.length; i++) parts.push(s.charCodeAt(i)); \
             return parts.join(','); }",
        )
        .map_err(|error| error.to_string())?;
    let units: String = format.call((value,)).map_err(|error| match error {
        rquickjs::Error::Exception => describe_exception(ctx),
        error => error.to_string(),
    })?;
    let units: Vec<u16> = units
        .split(',')
        .filter_map(|part| part.parse::<u16>().ok())
        .collect();
    Ok(wtf8_from_units(&units))
}

#[no_mangle]
pub extern "C" fn thaw_js_handle_to_string(handle: u64) -> *const c_char {
    let bytes = with_active_or_context(|ctx| -> Result<Vec<u8>, String> {
        let value = value_for_handle(&ctx, handle)?;
        js_handle_string_wtf8(&ctx, value)
    })
    .unwrap_or_else(|_| b"[invalid JsValue]".to_vec());
    thaw_arena::owned_string(bytes)
}

/// Like `thaw_js_handle_to_string`, but for `console.log` specifically --
/// real `console.log`/`util.inspect` formatting appends `n` to a `bigint`
/// (`123n`), unlike plain `String(x)` coercion (`"123"`, no suffix, used
/// by every other caller of `thaw_js_handle_to_string`: real `String()`,
/// template-literal interpolation, string concatenation -- none of which
/// should gain a stray `n`). A dedicated function so fixing `console.log`
/// couldn't silently break any of those.
#[no_mangle]
pub extern "C" fn thaw_js_handle_to_console_string(handle: u64) -> *const c_char {
    let bytes = with_active_or_context(|ctx| -> Result<Vec<u8>, String> {
        let value = value_for_handle(&ctx, handle)?;
        let format: Function = ctx
            .eval(
                "(value) => { const s = typeof value === 'bigint' ? String(value) + 'n' : String(value); \
                 const parts = []; for (let i = 0; i < s.length; i++) parts.push(s.charCodeAt(i)); \
                 return parts.join(','); }",
            )
            .map_err(|error| error.to_string())?;
        let units: String = format.call((value,)).map_err(|error| match error {
            rquickjs::Error::Exception => describe_exception(&ctx),
            error => error.to_string(),
        })?;
        let units: Vec<u16> = units
            .split(',')
            .filter_map(|part| part.parse::<u16>().ok())
            .collect();
        Ok(wtf8_from_units(&units))
    })
    .unwrap_or_else(|_| b"[invalid JsValue]".to_vec());
    thaw_arena::owned_string(bytes)
}

/// `console.log` of a typed array (not `Buffer`/`DataView`): `{"n":name,"v":[text, ..]}`
/// for the std inspector (`thaw_console_typed_or`), or null for any other value.
#[no_mangle]
pub extern "C" fn thaw_js_handle_typed_array_probe(handle: u64) -> *const c_char {
    let probe = with_active_or_context(|ctx| -> Result<Option<String>, String> {
        let value = value_for_handle(&ctx, handle)?;
        let probe: Function = ctx
            .eval(
                "(v) => { if (typeof v !== 'object' || v === null || !ArrayBuffer.isView(v) || v instanceof DataView \
                 || (typeof Buffer !== 'undefined' && Buffer.isBuffer(v))) return null; \
                 return JSON.stringify({ n: v[Symbol.toStringTag], \
                 v: Array.from(v, (x) => typeof x === 'bigint' ? x + 'n' : Object.is(x, -0) ? '-0' : String(x)) }); }",
            )
            .map_err(|error| error.to_string())?;
        probe.call::<_, Option<String>>((value,)).map_err(|error| error.to_string())
    });
    match probe {
        Ok(Some(text)) => thaw_arena::owned_string(text),
        _ => std::ptr::null(),
    }
}

/// The residual JIT's `dynamic_object_query` host ABI: operation `0`
/// snapshots a Set-like opaque handle's keys into a JSON object (validating
/// the `size`/`has`/`keys` protocol in the JS realm).
///
/// # Safety
/// `handle` must be a live retained handle (or `0`); `error` must be null or
/// point to a writable pointer slot.
#[no_mangle]
pub unsafe extern "C" fn thaw_js_dynamic_object_query(
    operation: u8,
    handle: u64,
    error: *mut *const c_char,
) -> f64 {
    if operation != 0 {
        if let Some(error) = error.as_mut() {
            *error = CString::new("unsupported dynamic object query")
                .unwrap_or_default()
                .into_raw();
        }
        return 0.0;
    }
    let result = with_active_or_context(|ctx| -> Result<String, String> {
        let value = value_for_handle(&ctx, handle)?;
        let snapshot: Function = ctx
            .eval(
                "(value) => { const size = Number(value.size); const has = value.has; const keys = value.keys; if (Number.isNaN(size) || size < 0 || typeof has !== 'function' || typeof keys !== 'function') throw new TypeError('Set-like object requires non-negative size, has(), and keys()'); const iterator = keys.call(value); if ((typeof iterator !== 'object' && typeof iterator !== 'function') || iterator === null || typeof iterator.next !== 'function') throw new TypeError('Set-like keys() must return an iterator'); const out = Object.create(null); for (;;) { const step = iterator.next(); if ((typeof step !== 'object' && typeof step !== 'function') || step === null) throw new TypeError('Set-like iterator result must be an object'); if (step.done) break; out[String(step.value)] = true; } return JSON.stringify(out); }",
            )
            .map_err(|error| error.to_string())?;
        snapshot.call((value,)).map_err(|error| match error {
            rquickjs::Error::Exception => describe_exception(&ctx),
            error => error.to_string(),
        })
    });
    match result {
        Ok(json) => {
            // Hand the JIT dictionary back as a live thaw-std `Json`
            // value (its own `Value` layout now, not `serde_json::Value`)
            // via thaw-std's exported parser -- the same "resolved at
            // link time" boundary `thaw-runtime`'s `AnyKey` uses.
            unsafe extern "C" {
                fn thaw_json_parse(text: *const c_char) -> *mut u8;
            }
            let Ok(text) = CString::new(json) else {
                if let Some(error) = error.as_mut() {
                    *error = CString::new("dynamic object query produced a NUL")
                        .unwrap_or_default()
                        .into_raw();
                }
                return 0.0;
            };
            let parsed = unsafe { thaw_json_parse(text.as_ptr()) };
            f64::from_bits(parsed as usize as u64)
        }
        Err(message) => {
            if let Some(error) = error.as_mut() {
                *error = CString::new(message).unwrap_or_default().into_raw();
            }
            0.0
        }
    }
}

fn handle_array<'js>(ctx: &Ctx<'js>) -> Result<Array<'js>, String> {
    ctx.globals()
        .get("__thaw_value_handles")
        .map_err(|_| "JavaScript value handle registry is empty".to_string())
}

fn value_for_handle<'js>(ctx: &Ctx<'js>, handle: u64) -> Result<Value<'js>, String> {
    if handle == 0 {
        return Err("invalid JavaScript value handle 0".to_string());
    }
    let live: Array = ctx
        .globals()
        .get("__thaw_value_handle_live")
        .map_err(|_| "JavaScript value handle liveness registry is empty".to_string())?;
    if !live.get::<bool>((handle - 1) as usize).unwrap_or(false) {
        return Err(format!("released JavaScript value handle {handle}"));
    }
    let value: Value = handle_array(ctx)?
        .get((handle - 1) as usize)
        .map_err(|_| format!("invalid JavaScript value handle {handle}"))?;
    Ok(value)
}

fn object_for_handle<'js>(ctx: &Ctx<'js>, handle: u64) -> Result<Object<'js>, String> {
    let value = value_for_handle(ctx, handle)?;
    value
        .as_object()
        .cloned()
        .or_else(|| {
            value
                .as_function()
                .map(|function| AsRef::<Object>::as_ref(function).clone())
        })
        .or_else(|| {
            value
                .as_proxy()
                .map(|proxy| AsRef::<Object>::as_ref(proxy).clone())
        })
        .ok_or_else(|| {
            format!(
                "JavaScript value handle {handle} has non-object type {:?}",
                value.type_of()
            )
        })
}

/// Like [`object_for_handle`], but first auto-boxes a *primitive* handle
/// (string/number/bool/symbol/bigint) via the JS `Object(value)` builtin --
/// the spec's `ToObject`. Both `value.property` and `value.method()` box the
/// receiver in JS, so a dynamic operation on a `JsValue` whose runtime value
/// turned out to be a primitive should behave the same way. Real examples:
/// highlight.js's `highlightAuto(code).value` is a string whose `.length` is
/// then read, and gray-matter's `parsed.content` is a string whose `.trim()`
/// is then called -- both previously failed with "JavaScript value handle N
/// has non-object type String". Note `Reflect.get` is *not* a substitute:
/// QuickJS's own implementation rejects non-object targets too.
fn boxed_object_for_handle<'js>(ctx: &Ctx<'js>, handle: u64) -> Result<Object<'js>, String> {
    let value = value_for_handle(ctx, handle)?;
    if value.is_null() || value.is_undefined() {
        return Err("\u{1}TypeError\u{1}Cannot convert undefined or null to object".into());
    }
    if value.is_object() {
        return object_for_handle(ctx, handle);
    }
    let object_constructor: Function = ctx
        .globals()
        .get("Object")
        .map_err(|error| error.to_string())?;
    let boxed: Value<'js> = object_constructor
        .call((value,))
        .map_err(|error| error.to_string())?;
    boxed.as_object().cloned().ok_or_else(|| {
        format!(
            "JavaScript value handle {handle} has non-object type {:?}",
            boxed.type_of()
        )
    })
}

/// Retains `value` in the realm-global handle registry and returns a
/// stable opaque handle, lazily creating the registry itself (`__thaw_
/// value_handles`/`__thaw_value_handle_live`) on first use -- unlike
/// `handle_array` (a *lookup*, used once something is already known to be
/// registered), this is the sole *write* path into the registry, so it
/// must not assume some earlier call already bootstrapped it. Real
/// example: a program whose first-ever touch of a JsValue handle is
/// registering a native closure as a callback
/// (`thaw_js_register_native_callback`) passed straight into an ordinary
/// top-level Fallback function call, with no prior dynamic-value-
/// returning call (e.g. no `makeHolder(): JsValue`-style call) to have
/// bootstrapped the registry first -- previously failed with "JavaScript
/// value handle registry is empty" even though this was really the very
/// first legitimate retain.
fn retain_value<'js>(ctx: &Ctx<'js>, value: Value<'js>) -> Result<u64, String> {
    let globals = ctx.globals();
    let handles: Array = match globals.get("__thaw_value_handles") {
        Ok(handles) => handles,
        Err(_) => {
            let handles = Array::new(ctx.clone()).map_err(|error| error.to_string())?;
            globals
                .set("__thaw_value_handles", handles.clone())
                .map_err(|error| error.to_string())?;
            let live = Array::new(ctx.clone()).map_err(|error| error.to_string())?;
            globals
                .set("__thaw_value_handle_live", live)
                .map_err(|error| error.to_string())?;
            let refs = Array::new(ctx.clone()).map_err(|error| error.to_string())?;
            globals
                .set("__thaw_value_handle_refs", refs)
                .map_err(|error| error.to_string())?;
            handles
        }
    };
    let live: Array = globals
        .get("__thaw_value_handle_live")
        .map_err(|_| "JavaScript value handle liveness registry is empty".to_string())?;
    let identical = ctx.userdata::<NativeBundleOwner<'_>>()
        .ok_or("intrinsic Object.is is unavailable")?.object_is.clone();
    let refs: Array = globals
        .get("__thaw_value_handle_refs")
        .map_err(|_| "JavaScript value handle reference registry is empty".to_string())?;
    // ponytail: linear identity lookup keeps the registry simple; replace it
    // with a WeakMap only if live handle counts become measurably large.
    let mut free = None;
    for index in 0..handles.len() {
        if live.get::<bool>(index).unwrap_or(false) {
            let existing: Value = handles.get(index).map_err(|error| error.to_string())?;
            if identical
                .call::<_, bool>((existing, value.clone()))
                .map_err(|error| error.to_string())?
            {
                let count = refs.get::<u64>(index).unwrap_or(1);
                refs.set(index, count + 1)
                    .map_err(|error| error.to_string())?;
                return Ok(index as u64 + 1);
            }
        } else if free.is_none() {
            free = Some(index);
        }
    }
    let index = free.unwrap_or_else(|| handles.len());
    handles
        .set(index, value)
        .map_err(|error| error.to_string())?;
    live.set(index, true).map_err(|error| error.to_string())?;
    refs.set(index, 1u64).map_err(|error| error.to_string())?;
    Ok(index as u64 + 1)
}

/// Consumes a candidate JS-object handle and returns one retained canonical
/// wrapper. The private identity map owns a guardian callback holding an
/// ArenaRoot; its pointer cache holds only a WeakRef to the wrapper.
#[no_mangle]
pub extern "C" fn thaw_js_intern_native_object(
    pointer: *const c_void,
    layout: *const c_char,
    candidate_handle: u64,
) -> ThawHandleResult {
    let result = with_active_or_context(|ctx| -> Result<u64, String> {
        if pointer.is_null() || layout.is_null() {
            return Err("native object identity requires pointer and layout".into());
        }
        if !thaw_arena::contains_allocation(pointer as usize) {
            return Err("live native object requires arena-owned storage".into());
        }
        let candidate = object_for_handle(&ctx, candidate_handle)?;
        let bridge = ctx.userdata::<NativeCallbackIdentityBridge<'_>>()
            .ok_or("native object identity bridge is unavailable")?;
        let intern = bridge.intern_object.clone();
        drop(bridge);
        let root = thaw_arena::ArenaRoot::new(pointer as usize);
        let guardian = Function::new(ctx.clone(), move || {
            let _keep_alive = &root;
        }).map_err(|error| error.to_string())?;
        let canonical: Value = intern.call((
            format!("{:x}", pointer as usize), to_str(layout), candidate, guardian,
        )).map_err(|error| error.to_string())?;
        retain_value(&ctx, canonical)
    });
    let _ = thaw_js_release_handle(candidate_handle);
    match result {
        Ok(value) => ThawHandleResult { value, error: std::ptr::null() },
        Err(error) => ThawHandleResult { value: 0, error: thaw_arena::owned_string(error) },
    }
}

/// Returns a retained cached wrapper when this pointer has already been
/// projected with a compatible layout. A miss returns zero without creating
/// any callbacks or transferring ownership.
#[no_mangle]
pub extern "C" fn thaw_js_lookup_native_object(
    pointer: *const c_void,
    layout: *const c_char,
) -> ThawHandleResult {
    let result = with_active_or_context(|ctx| -> Result<u64, String> {
        if pointer.is_null() || layout.is_null() {
            return Err("native object lookup requires pointer and layout".into());
        }
        let bridge = ctx.userdata::<NativeCallbackIdentityBridge<'_>>()
            .ok_or("native object identity bridge is unavailable")?;
        let lookup = bridge.object_lookup.clone();
        drop(bridge);
        let cached: Value = lookup.call((format!("{:x}", pointer as usize), to_str(layout)))
            .map_err(|error| error.to_string())?;
        if cached.is_undefined() { Ok(0) } else { retain_value(&ctx, cached) }
    });
    match result {
        Ok(value) => ThawHandleResult { value, error: std::ptr::null() },
        Err(error) => ThawHandleResult { value: 0, error: thaw_arena::owned_string(error) },
    }
}

/// Returns zero for a non-native or layout-incompatible receiver. No user
/// property can forge the private WeakMap membership used by this query.
#[no_mangle]
pub extern "C" fn thaw_js_native_object_pointer(
    handle: u64,
    expected_layout: *const c_char,
) -> ThawHandleResult {
    let result = with_active_or_context(|ctx| -> Result<u64, String> {
        if expected_layout.is_null() {
            return Err("native object receiver requires a layout".into());
        }
        let value = value_for_handle(&ctx, handle)?;
        let bridge = ctx.userdata::<NativeCallbackIdentityBridge<'_>>()
            .ok_or("native object identity bridge is unavailable")?;
        let pointer_of = bridge.object_pointer.clone();
        drop(bridge);
        let pointer: String = pointer_of.call((value, to_str(expected_layout)))
            .map_err(|error| error.to_string())?;
        if pointer.is_empty() { return Ok(0); }
        u64::from_str_radix(&pointer, 16)
            .map_err(|_| "invalid native object pointer token".into())
    });
    match result {
        Ok(value) => ThawHandleResult { value, error: std::ptr::null() },
        Err(error) => ThawHandleResult { value: 0, error: thaw_arena::owned_string(error) },
    }
}

/// Calls a live, retained function *value* (as opposed to a call by global
/// name, `invoke_impl`, or by method name, `invoke_method`) -- used
/// whenever the call's own *result* is itself a `JsValue`/handle rather
/// than JSON data (`thaw_js_call_handle_handle_result`, `invoke_mixed`).
/// A thrown exception here is just as catchable from compiled Thaw code as
/// one from any other call convention, so it must be tagged
/// (`describe_tagged_exception`) too -- real example: jsonwebtoken's own
/// `verify(token, secret)`, whose 2-argument overload resolves to a
/// `JwtPayload | string` union return type (so its call goes through this
/// handle-result convention), reported a caught `JsonWebTokenError`'s
/// `.name` as plain `"Error"` before this fix, even though the message
/// ("invalid signature") came through correctly -- `describe_exception`
/// (the untagged variant) discards `.name` entirely by design, which is
/// fine for the handful of internal/startup-only call sites that still use
/// it, but wrong for anything a user `try/catch` can observe.
fn invoke_raw<'js>(
    ctx: Ctx<'js>,
    target: Function<'js>,
    args_json: &str,
    graph_args: bool,
) -> Result<Value<'js>, String> {
    let args_array = decode_argument_array(&ctx, args_json, graph_args)?;
    let mut call_args = Args::new_unsized(ctx.clone());
    for index in 0..args_array.len() {
        let arg: Value = args_array.get(index).map_err(|error| error.to_string())?;
        call_args.push_arg(arg).map_err(|error| error.to_string())?;
    }
    target.call_arg(call_args).map_err(|error| match error {
        rquickjs::Error::Exception => describe_tagged_exception(&ctx),
        error => error.to_string(),
    })
}

unsafe fn native_handle_slice<'a>(array: *const u8) -> Result<&'a [u64], String> {
    if array.is_null() {
        return Err("null JsValue array".into());
    }
    let length = unsafe { *(array.cast::<u64>()) } as usize;
    Ok(unsafe { std::slice::from_raw_parts(array.add(8).cast::<u64>(), length) })
}

unsafe fn invoke_mixed<'js>(
    ctx: Ctx<'js>,
    target: Function<'js>,
    args_json: &str,
    handles: *const u8,
    graph_args: bool,
) -> Result<Value<'js>, String> {
    let arguments = decode_argument_array(&ctx, args_json, graph_args)?;
    let mut call_args = Args::new_unsized(ctx.clone());
    for index in 0..arguments.len() {
        call_args
            .push_arg(
                arguments
                    .get::<Value>(index)
                    .map_err(|error| error.to_string())?,
            )
            .map_err(|error| error.to_string())?;
    }
    for handle in unsafe { native_handle_slice(handles)? } {
        call_args
            .push_arg(value_for_handle(&ctx, *handle)?)
            .map_err(|error| error.to_string())?;
    }
    // See `invoke_raw`'s doc comment -- same call convention (a live
    // function value, not a call by name), same fix.
    target.call_arg(call_args).map_err(|error| match error {
        rquickjs::Error::Exception => describe_tagged_exception(&ctx),
        error => error.to_string(),
    })
}

/// Calls a retained function with JSON arguments followed by retained values.
///
/// # Safety
///
/// `handles` must point to a live Thaw array buffer containing an initial
/// `u64` length followed by that many aligned `u64` handle identifiers.
unsafe fn thaw_js_call_handle_mixed_result_impl(
    handle: u64,
    args_json: *const c_char,
    handles: *const u8,
    graph_result: bool,
) -> ThawResult {
    let args_json = to_str(args_json);
    let result = with_active_or_context(|ctx| {
        let target = before_graph_decode(&ctx, &args_json, graph_result, || {
            Function::from_value(value_for_handle(&ctx, handle)?)
                .map_err(|_| format!("JavaScript value handle {handle} is not callable"))
        })?;
        let value = unsafe { invoke_mixed(ctx.clone(), target, &args_json, handles, graph_result)? };
        resolve_value_wire_impl(ctx, value, &format!("JavaScript value #{handle}"), true, graph_result)
    });
    match result {
        Ok(value) => ThawResult {
            value: thaw_arena::owned_string(value),
            error: std::ptr::null(),
        },
        Err(error) => ThawResult {
            value: std::ptr::null(),
            error: thaw_arena::owned_string(error),
        },
    }
}

/// Calls the function-replacer shim with a native Json graph reconstructed
/// before JSON.stringify runs. The input may contain back references.
///
/// # Safety
/// `graph_json` and `space_json` are valid NUL-terminated JSON strings;
/// `handles` is a live native handle array.
unsafe fn thaw_js_call_handle_mixed_native_json_result_impl(
    handle: u64,
    graph_json: *const c_char,
    space_json: *const c_char,
    handles: *const u8,
    graph_result: bool,
) -> ThawResult {
    let graph_json = to_str(graph_json);
    let space_json = to_str(space_json);
    let result = with_active_or_context(|ctx| {
        let (target, parse, decode) = before_graph_decode(&ctx, &graph_json, true, || {
            let target = Function::from_value(value_for_handle(&ctx, handle)?)
                .map_err(|_| format!("JavaScript value handle {handle} is not callable"))?;
            let json: Object = ctx.globals().get("JSON").map_err(|error| error.to_string())?;
            let parse: Function = json.get("parse").map_err(|error| error.to_string())?;
            let decode: Function = ctx.globals().get("__thaw_json_graph_decode_owned")
                .map_err(|error| error.to_string())?;
            Ok((target, parse, decode))
        })?;
        let value: Value = decode.call((graph_json.as_str(),)).map_err(|error| match error {
            rquickjs::Error::Exception => describe_tagged_exception(&ctx),
            error => error.to_string(),
        })?;
        let space: Value = parse.call((space_json.as_str(),))
            .map_err(|error| error.to_string())?;
        let mut call_args = Args::new_unsized(ctx.clone());
        call_args.push_arg(value).map_err(|error| error.to_string())?;
        call_args.push_arg(space).map_err(|error| error.to_string())?;
        for item in unsafe { native_handle_slice(handles)? } {
            call_args.push_arg(value_for_handle(&ctx, *item)?)
                .map_err(|error| error.to_string())?;
        }
        let result = target.call_arg(call_args).map_err(|error| match error {
            rquickjs::Error::Exception => describe_tagged_exception(&ctx),
            error => error.to_string(),
        })?;
        resolve_value_wire_impl(ctx, result, &format!("JavaScript value #{handle}"), true, graph_result)
    });
    match result {
        Ok(value) => ThawResult {
            value: thaw_arena::owned_string(value),
            error: std::ptr::null(),
        },
        Err(error) => ThawResult {
            value: std::ptr::null(),
            error: thaw_arena::owned_string(error),
        },
    }
}

#[no_mangle]
/// Like `thaw_js_call_handle_mixed_result`, but returns the call's result
/// as a live handle instead of a JSON encoding -- needed when the result
/// may not be JSON-representable (a `BigInt` from a dynamic `+`).
///
/// # Safety
/// `args_json` must be null or a valid NUL-terminated UTF-8 string;
/// `handles` must point to `handles_len` handle ids.
pub unsafe extern "C" fn thaw_js_call_handle_mixed_handle_result(
    handle: u64, args_json: *const c_char, handles: *const u8,
) -> ThawHandleResult {
    thaw_js_call_handle_mixed_handle_impl(handle, args_json, handles, false)
}

#[no_mangle]
pub unsafe extern "C" fn thaw_js_call_handle_mixed_handle_graph_args_result(
    handle: u64, args_json: *const c_char, handles: *const u8,
) -> ThawHandleResult {
    thaw_js_call_handle_mixed_handle_impl(handle, args_json, handles, true)
}

/// The native-object projection builder installs its callbacks on the
/// returned JS object. Drop their temporary registry references after the
/// builder call, including when that call throws. The JS object itself keeps
/// the functions and their ArenaRoot captures alive for its own lifetime.
#[no_mangle]
pub unsafe extern "C" fn thaw_js_call_handle_mixed_handle_graph_args_consuming_result(
    handle: u64, args_json: *const c_char, handles: *const u8,
) -> ThawHandleResult {
    // A getter invoked by the builder can re-enter compiled code. Snapshot
    // the exact acquired IDs before that call; never release a mutated
    // native array read back after user code has run.
    let acquired = unsafe { native_handle_slice(handles) }
        .map(|values| values.to_vec());
    let result = thaw_js_call_handle_mixed_handle_impl(handle, args_json, handles, true);
    if let Ok(values) = acquired {
        for value in values {
            let _ = thaw_js_release_handle(value);
        }
    }
    let _ = thaw_js_release_handle(handle);
    result
}

/// Dispose callback registration references if the builder's argument graph
/// cannot be encoded, before the consuming builder ABI is reached.
#[no_mangle]
pub unsafe extern "C" fn thaw_js_release_native_handle_array(handles: *const u8) {
    if let Ok(values) = unsafe { native_handle_slice(handles) } {
        for value in values.iter().copied() {
            let _ = thaw_js_release_handle(value);
        }
    }
}

unsafe fn thaw_js_call_handle_mixed_handle_impl(
    handle: u64, args_json: *const c_char, handles: *const u8, graph_args: bool,
) -> ThawHandleResult {
    let args_json = to_str(args_json);
    match with_active_or_context(|ctx| {
        let target = before_graph_decode(&ctx, &args_json, graph_args, || {
            Function::from_value(value_for_handle(&ctx, handle)?)
                .map_err(|_| format!("JavaScript value handle {handle} is not callable"))
        })?;
        let value = unsafe { invoke_mixed(ctx.clone(), target, &args_json, handles, graph_args)? };
        retain_value(&ctx, value)
    }) {
        Ok(value) => ThawHandleResult {
            value,
            error: std::ptr::null(),
        },
        Err(error) => ThawHandleResult {
            value: 0,
            error: thaw_arena::owned_string(error),
        },
    }
}

#[no_mangle]
pub extern "C" fn thaw_js_construct_handle_result(handle: u64, args_json: *const c_char) -> ThawHandleResult {
    thaw_js_construct_handle_impl(handle, args_json, false)
}

#[no_mangle]
pub extern "C" fn thaw_js_construct_handle_graph_args_result(handle: u64, args_json: *const c_char) -> ThawHandleResult {
    thaw_js_construct_handle_impl(handle, args_json, true)
}

fn thaw_js_construct_handle_impl(handle: u64, args_json: *const c_char, graph_args: bool) -> ThawHandleResult {
    let args_json = to_str(args_json);
    match with_active_or_context(|ctx| {
        let constructor = before_graph_decode(&ctx, &args_json, graph_args, || {
            value_for_handle(&ctx, handle)?
                .into_constructor()
                .ok_or_else(|| format!("JavaScript value handle {handle} is not a constructor"))
        })?;
        let arguments = decode_argument_array(&ctx, &args_json, graph_args)?;
        let mut args = Args::new_unsized(ctx.clone());
        for index in 0..arguments.len() {
            args.push_arg(
                arguments
                    .get::<Value>(index)
                    .map_err(|error| error.to_string())?,
            )
            .map_err(|error| error.to_string())?;
        }
        // See `invoke_raw`'s doc comment -- `new SomeClass(...)` against a
        // live constructor value is just as catchable as any other call.
        let value: Value = constructor
            .construct_args(args)
            .map_err(|error| match error {
                rquickjs::Error::Exception => describe_tagged_exception(&ctx),
                error => error.to_string(),
            })?;
        retain_value(&ctx, value)
    }) {
        Ok(value) => ThawHandleResult {
            value,
            error: std::ptr::null(),
        },
        Err(error) => ThawHandleResult {
            value: 0,
            error: thaw_arena::owned_string(error),
        },
    }
}

/// The generic ABI every adapter thaw-llvm's `compile_napi_value_callback`
/// builds shares, regardless of the real (native) closure's own signature
/// -- the adapter itself already handles all the type-specific arg/return
/// marshaling internally, exposing only this one uniform shape outward.
/// `context` is the closure's own captured-environment pointer (opaque
/// here, passed straight back to the adapter unexamined); the returned
/// pointer is a NUL-terminated JSON string owned by the adapter's own
/// caller-frees-nothing convention (mirrors `thaw_js_call`'s own return),
/// so `to_str` below copies it out immediately and nothing is freed here.
type NativeCallbackAdapter = unsafe extern "C" fn(*const c_void, *const c_char) -> *const c_char;
type NativeCallbackKey = (usize, u64, u64, u8, usize, u8, bool, bool);

thread_local! {
    static NATIVE_CALLBACK_HANDLES: RefCell<HashMap<NativeCallbackKey, u64>> =
        RefCell::new(HashMap::new());
}

/// Wraps a real compiled (native) closure -- already bridged into a
/// generic `(context, args_json) -> result_json` adapter by thaw-llvm's
/// `compile_napi_value_callback` -- as a live, retained QuickJS-NG
/// function value the Fallback dynamic-call path can hand off like any
/// other `JsValue`. Real example: zod's `z.number().refine((n: number) =>
/// n > 0, {...})`, whose predicate has nowhere else to go.
///
/// `adapter`/`closure` are captured by the returned JS function's own
/// closure as plain `usize` (not `Send`-problematic raw pointers) --
/// transmuted back to the real function-pointer type only at call time.
/// Safe because every adapter `compile_napi_value_callback` ever produces
/// shares this exact ABI unconditionally; there is no way to construct a
/// `NativeCallbackAdapter`-typed value here except from that one
/// generator.
///
/// Argument/return marshaling for the *inner* call (JS call args ->
/// `args_json`, `result_json` -> the JS return value) reuses the exact
/// same `JSON.stringify`/`JSON.parse`-with-reviver convention every other
/// dynamic-call boundary here already uses, so a `Date`/`JsValue`/
/// `undefined` argument or return value round-trips the same way it does
/// crossing any other dynamic-call boundary.
/// Retains a JS value and returns its permanent handle id, exposed to JS
/// as `__thaw_retain_dynamic_value` for `thaw_js_register_native_
/// callback`'s own wrapper to call. A plain named function, not a
/// closure -- see the call site's own comment for why.
fn retain_dynamic_value_for_js<'js>(ctx: Ctx<'js>, value: Value<'js>) -> u64 {
    retain_value(&ctx, value).unwrap_or(0)
}

fn release_dynamic_value_for_js(ctx: Ctx<'_>, handle: u64) -> u8 {
    // The graph decoder can call this from ordinary JS, outside a native
    // callback. Reuse its current Ctx when the exported release reenters.
    let _active = ActiveNapiContext::enter(&ctx);
    thaw_js_release_handle(handle)
}

struct NativeSymbolRoots(std::cell::RefCell<std::collections::HashMap<u64, NativeGraphCallbackRoots>>);

struct PrivateNapiSymbolOps {
    register: Persistent<Function<'static>>,
    live: Persistent<Function<'static>>,
    pin: Persistent<Function<'static>>,
}

unsafe impl<'js> rquickjs::JsLifetime<'js> for PrivateNapiSymbolOps {
    type Changed<'to> = PrivateNapiSymbolOps;
}

fn install_private_napi_symbol_ops<'js>(
    ctx: Ctx<'js>, register: Function<'js>, live: Function<'js>, pin: Function<'js>,
) -> rquickjs::Result<()> {
    if ctx.userdata::<PrivateNapiSymbolOps>().is_some() {
        return Err(rquickjs::Error::new_from_js_message(
            "native Symbol bridge", "Function", "Symbol bridge is already installed"));
    }
    let ops = PrivateNapiSymbolOps {
        register: Persistent::save(&ctx, register),
        live: Persistent::save(&ctx, live),
        pin: Persistent::save(&ctx, pin),
    };
    ctx.store_userdata(ops).map(|_| ()).map_err(|_| rquickjs::Error::new_from_js_message(
        "native Symbol bridge", "Function", "Cannot install Symbol bridge"))
}

unsafe impl<'js> rquickjs::JsLifetime<'js> for NativeSymbolRoots {
    // Only numeric native reference tokens are stored; their Drop handlers
    // run when this QuickJS runtime clears userdata before JS_FreeRuntime.
    type Changed<'to> = NativeSymbolRoots;
}

fn pin_native_graph_symbol(ctx: Ctx<'_>, id: String) -> rquickjs::Result<bool> {
    let id = id.parse::<u64>().ok().filter(|id| *id != 0)
        .ok_or_else(|| rquickjs::Error::new_from_js_message(
            "String", "native Symbol ID", "Invalid native Symbol ID"))?;
    if ctx.userdata::<NativeSymbolRoots>()
        .is_some_and(|roots| roots.0.borrow().contains_key(&id)) {
        return Ok(true);
    }
    let callback = NAPI_BRIDGE.lock().unwrap().as_ref().map(|bridge| bridge.2)
        .ok_or_else(|| rquickjs::Error::new_from_js_message(
            "native bridge", "Symbol", "Native addon bridge is unavailable"))?;
    let target = CString::new(id.to_string()).expect("numeric Symbol ID has no NUL");
    let operation = c"retain_graph_handle";
    let empty = c"";
    let _active = ActiveNapiContext::enter(&ctx);
    let reply = unsafe { to_str(callback(operation.as_ptr(), target.as_ptr(),
        empty.as_ptr(), empty.as_ptr())) };
    let parsed: serde_json::Value = serde_json::from_str(&reply)
        .map_err(|_| rquickjs::Error::new_from_js_message(
            "native bridge", "Symbol", "Invalid native Symbol pin response"))?;
    let token = parsed.get("value").and_then(serde_json::Value::as_str)
        .and_then(|token| token.parse::<u64>().ok()).filter(|token| *token != 0)
        .ok_or_else(|| rquickjs::Error::new_from_js_message(
            "native bridge", "Symbol", "Cannot retain native Symbol"))?;
    let root = NativeGraphCallbackRoots([token, 0, 0]);
    let roots = ctx.userdata::<NativeSymbolRoots>()
        .ok_or_else(|| rquickjs::Error::new_from_js_message(
            "native bridge", "Symbol", "Missing Symbol root owner"))?;
    let mut redundant = Some(root);
    {
        let mut entries = roots.0.borrow_mut();
        entries.entry(id).or_insert_with(|| redundant.take().expect("new Symbol root is present"));
    }
    // Releasing an extra positive reference may reenter QuickJS. Never do it
    // while the userdata RefCell is borrowed.
    drop(roots);
    drop(redundant);
    Ok(true)
}

// A native Symbol graph pin belongs to the corresponding live JavaScript
// Symbol, not to its stable numeric ID. A WeakRef finalizer calls this only
// after confirming that the cache still names the same weak entry.
fn unpin_native_graph_symbol(ctx: Ctx<'_>, id: String) -> rquickjs::Result<bool> {
    let id = id.parse::<u64>().ok().filter(|id| *id != 0)
        .ok_or_else(|| rquickjs::Error::new_from_js_message(
            "String", "native Symbol ID", "Invalid native Symbol ID"))?;
    let Some(roots) = ctx.userdata::<NativeSymbolRoots>() else { return Ok(false); };
    let root = roots.0.borrow_mut().remove(&id);
    // The positive-reference destructor can reenter QuickJS/N-API. Keep the
    // userdata RefCell free while it runs.
    drop(roots);
    let removed = root.is_some();
    drop(root);
    Ok(removed)
}

// These queries address the private per-addon Symbol cache captured by the
// platform bootstrap. A native scope collector may ask whether a local JS
// Symbol remains reachable without holding a strong handle to that Symbol.
#[no_mangle]
pub extern "C" fn thaw_js_napi_graph_symbol_register(owner: u64, handle: u64) -> u64 {
    if owner == 0 || handle == 0 { return 0; }
    with_active_or_context(|ctx| {
        let Ok(value) = value_for_handle(&ctx, handle) else { return 0; };
        if !value.is_symbol() { return 0; }
        let Some(ops) = ctx.userdata::<PrivateNapiSymbolOps>() else { return 0; };
        let register = ops.register.clone();
        drop(ops);
        let Ok(register) = register.restore(&ctx) else { return 0; };
        register.call::<_, String>((owner.to_string(), value)).ok()
            .and_then(|id| id.parse::<u64>().ok())
            .filter(|id| *id != 0).unwrap_or(0)
    })
}

// Native Symbol classification for trusted N-API ownership decisions. The
// ordinary host-query operation reads a writable JS global and cannot prove
// that an arbitrary retained handle denotes a Symbol.
#[no_mangle]
pub extern "C" fn thaw_js_napi_handle_is_symbol(handle: u64) -> u8 {
    if handle == 0 { return 2; }
    with_active_or_context(|ctx| match value_for_handle(&ctx, handle) {
        Ok(value) => u8::from(value.is_symbol()),
        Err(_) => 2,
    })
}

#[no_mangle]
pub extern "C" fn thaw_js_napi_symbol_weak_live(owner: u64, id: u64) -> u8 {
    if owner == 0 || id == 0 { return 2; }
    with_active_or_context(|ctx| {
        let Some(ops) = ctx.userdata::<PrivateNapiSymbolOps>() else { return 2; };
        let query = ops.live.clone();
        drop(ops);
        let Ok(query) = query.restore(&ctx) else { return 2; };
        match query.call::<_, bool>((owner.to_string(), id.to_string())) {
            Ok(live) => u8::from(live),
            Err(_) => 2, // Unknown: callers must never treat this as dead.
        }
    })
}

#[no_mangle]
pub extern "C" fn thaw_js_napi_symbol_set_pin(owner: u64, id: u64, pin: u8) -> u8 {
    if owner == 0 || id == 0 { return 0; }
    with_active_or_context(|ctx| {
        let Some(ops) = ctx.userdata::<PrivateNapiSymbolOps>() else { return 0; };
        let update = ops.pin.clone();
        drop(ops);
        let Ok(update) = update.restore(&ctx) else { return 0; };
        u8::from(update.call::<_, bool>((owner.to_string(), id.to_string(), pin != 0))
            .unwrap_or(false))
    })
}

fn make_native_graph_invoker<'js>(
    ctx: Ctx<'js>, target: String,
) -> rquickjs::Result<Function<'js>> {
    let target = CString::new(target).map_err(|_| rquickjs::Error::new_from_js_message(
        "String", "native callback", "Invalid native callback ID"))?;
    // Acquire the native root only after the engine has entered this trusted
    // factory. The returned closure takes it by value; an error while
    // creating that closure drops it exactly once.
    let callback = NAPI_BRIDGE.lock().unwrap().as_ref().map(|bridge| bridge.2)
        .ok_or_else(|| rquickjs::Error::new_from_js_message(
            "native bridge", "Function", "Native addon bridge is unavailable"))?;
    let reply = {
        let _active = ActiveNapiContext::enter(&ctx);
        unsafe { to_str(callback(c"retain_graph_handle".as_ptr(), target.as_ptr(),
            c"".as_ptr(), c"".as_ptr())) }
    };
    let parsed: serde_json::Value = serde_json::from_str(&reply)
        .map_err(|_| rquickjs::Error::new_from_js_message(
            "native bridge", "Function", "Invalid native Function root response"))?;
    let token = parsed.get("value").and_then(serde_json::Value::as_str)
        .and_then(|token| token.parse::<u64>().ok()).filter(|token| *token != 0)
        .ok_or_else(|| rquickjs::Error::new_from_js_message(
            "native bridge", "Function", "Cannot retain native Function"))?;
    let roots = NativeGraphCallbackRoots([token, 0, 0]);
    Function::new(ctx.clone(), move |ctx: Ctx<'_>, graph: String, construct: bool| {
        let _keep_native_function = &roots;
        let Some(callback) = NAPI_BRIDGE.lock().unwrap().as_ref().map(|bridge| bridge.2) else {
            release_unconsumed_graph_leases(&ctx, &graph);
            return "{\"__thaw_error__\":\"Native addon bridge is unavailable\"}".to_string();
        };
        let Ok(graph_c) = CString::new(graph.as_str()) else {
            release_unconsumed_graph_leases(&ctx, &graph);
            return "{\"__thaw_error__\":\"Invalid callback graph\"}".to_string();
        };
        let operation = if construct { c"construct_captured_graph" } else { c"call_captured_graph" };
        let empty = c"";
        let _active = ActiveNapiContext::enter(&ctx);
        unsafe { to_str(callback(operation.as_ptr(), target.as_ptr(),
            empty.as_ptr(), graph_c.as_ptr())) }
    })
}

fn install_graph_handle_functions(ctx: &Ctx<'_>) -> rquickjs::Result<()> {
    ctx.globals().set("__thaw_retain_dynamic_value",
        Function::new(ctx.clone(), retain_dynamic_value_for_js)?)?;
    ctx.globals().set("__thaw_release_dynamic_value",
        Function::new(ctx.clone(), release_dynamic_value_for_js)?)?;
    let first_symbol_install = ctx.userdata::<NativeSymbolRoots>().is_none();
    if first_symbol_install {
        ctx.store_userdata(NativeSymbolRoots(std::cell::RefCell::new(
            std::collections::HashMap::new())))
            .expect("native Symbol roots userdata is already borrowed");
    }
    if !ctx.globals().contains_key("__thaw_napi_graph_invoker")? {
        ctx.globals().prop("__thaw_napi_graph_invoker",
            Function::new(ctx.clone(), make_native_graph_invoker)?)?;
    }
    if first_symbol_install {
        ctx.globals().set("__thaw_napi_graph_symbol_pin",
            Function::new(ctx.clone(), pin_native_graph_symbol)?)?;
        ctx.globals().set("__thaw_napi_graph_symbol_unpin",
            Function::new(ctx.clone(), unpin_native_graph_symbol)?)?;
        ctx.globals().set("__thaw_napi_graph_symbol_install",
            Function::new(ctx.clone(), install_private_napi_symbol_ops)?)?;
    }
    Ok(())
}

#[cfg(test)]
#[test]
fn native_symbol_root_capabilities_are_private_after_platform_bootstrap() {
    // Unrun: user code can guess a numeric Symbol ID, but must not be able
    // to drop its native positive root or desynchronize the JS pin map.
    with_context(|ctx| {
        assert!(ctx.userdata::<PrivateNapiSymbolOps>().is_some());
        install_graph_handle_functions(&ctx).expect("repeated handle registration");
        let hidden: bool = ctx.eval(r#"
          !('__thaw_napi_graph_symbol_unpin' in globalThis)
          && !('__thaw_napi_graph_symbol_pin' in globalThis)
          && !('__thaw_napi_graph_symbol_install' in globalThis)
          && !('__thaw_json_graph_napi_symbol_pin' in globalThis)
          && !('__thaw_json_graph_napi_symbol_register' in globalThis)
          && !('__thaw_json_graph_napi_symbol_live' in globalThis)
        "#).expect("private Symbol bridge surface");
        assert!(hidden);
    });
}

#[cfg(test)]
#[test]
fn handle_registry_identity_ignores_later_object_is_override() {
    let handles = with_context(|ctx| {
        let values: Array = ctx.eval(
            "[{}, {}, Symbol('same'), Symbol('same'), Promise.resolve(1), Promise.resolve(1)]",
        ).expect("distinct live values");
        let object: Object = ctx.globals().get("Object").expect("Object intrinsic");
        let original: Function = object.get("is").expect("Object.is intrinsic");
        let spoof: Function = ctx.eval("() => true").expect("spoof comparator");
        object.set("is", spoof).expect("override Object.is");
        let retained = (|| -> Result<Vec<u64>, String> {
            let mut handles = Vec::new();
            for index in 0..values.len() {
                handles.push(retain_value(&ctx, values.get(index)
                    .map_err(|error| error.to_string())?)?);
            }
            handles.push(retain_value(&ctx, values.get(0)
                .map_err(|error| error.to_string())?)?);
            Ok(handles)
        })();
        object.set("is", original).expect("restore Object.is");
        retained.expect("retain values with captured identity")
    });
    assert_eq!(handles.len(), 7);
    for left in 0..6 {
        for right in left + 1..6 {
            assert_ne!(handles[left], handles[right], "distinct values share a handle");
        }
    }
    assert_eq!(handles[0], handles[6], "same object should reuse its handle");
    for handle in handles {
        assert_eq!(thaw_js_release_handle(handle), 1);
    }
}

#[cfg(test)]
#[test]
fn graph_codec_roundtrip_preserves_negative_zero() {
    let valid: bool = with_context(|ctx| ctx.eval(r#"(() => {
      const source = [-0, 0, NaN, Infinity, -Infinity];
      const result = __thaw_json_graph_decode_owned(__thaw_json_graph_encode_js(source));
      return result.every((value, index) => Object.is(value, source[index]))
        && JSON.stringify(source.slice(0, 2)) === '[0,0]';
    })()"#).expect("negative zero graph roundtrip"));
    assert!(valid);
}

#[cfg(test)]
#[test]
fn property_key_decoder_stays_original_after_replacement_attempt() {
    let valid: bool = with_context(|ctx| ctx.eval(r#"(() => {
      const encoded = '\u0003R19:shared-key';
      const original = globalThis.__thaw_property_key;
      const before = original(encoded);
      const changed = Reflect.set(globalThis, '__thaw_property_key', () => 'wrong');
      const descriptor = Object.getOwnPropertyDescriptor(globalThis, '__thaw_property_key');
      return before === Symbol.for('shared-key') && changed === false
        && globalThis.__thaw_property_key === original
        && globalThis.__thaw_property_key(encoded) === before
        && globalThis.__thaw_json_date_reviver('', encoded) === before
        && descriptor.writable === false && descriptor.configurable === false
        && descriptor.enumerable === true;
    })()"#).expect("property key bootstrap source check"));
    assert!(valid);
}

#[cfg(test)]
#[test]
fn graph_codec_roundtrip_retains_identity_and_releases_live_lease() {
    let valid: bool = with_context(|ctx| ctx.eval(r#"(() => {
      const source = { own: { __thaw_js_handle_id__: 19 }, big: 123n };
      source.self = source;
      source.map = new Map([[source, source.own]]);
      source.date = new Date(1234);
      const result = __thaw_json_graph_decode_owned(
        __thaw_json_graph_encode_js(source));
      if (result.self !== result || result.map.get(result) !== result.own
        || result.own.__thaw_js_handle_id__ !== 19 || result.big !== 123n
        || result.date.getTime() !== 1234) return false;
      const symbol = Symbol('live');
      const graph = JSON.parse(__thaw_json_graph_encode_js([symbol], 0, true));
      const handle = graph.leases[0];
      const values = __thaw_json_graph_decode_owned(JSON.stringify(graph));
      if (values[0] !== symbol || __thaw_value_handle_live[handle - 1] !== false)
        return false;
      const liveArray = [1];
      const arrayGraph = JSON.parse(__thaw_json_graph_encode_js([liveArray], 0, true));
      const arrayHandle = arrayGraph.leases[0];
      const liveResult = __thaw_json_graph_decode_owned(JSON.stringify(arrayGraph));
      if (liveResult[0] !== liveArray
        || __thaw_value_handle_live[arrayHandle - 1] !== false) return false;
      const invalid = JSON.parse(__thaw_json_graph_encode_js([liveArray], 0, true));
      const invalidHandle = invalid.leases[0];
      invalid.root = { r: 999 };
      let failed = false;
      try { __thaw_json_graph_decode_owned(JSON.stringify(invalid)); }
      catch (error) { failed = error instanceof TypeError; }
      return failed && __thaw_value_handle_live[invalidHandle - 1] === false;
    })()"#).expect("graph codec source check"));
    assert!(valid);
}

#[cfg(test)]
#[test]
fn graph_codec_uses_bootstrap_intrinsics_after_global_replacement() {
    let valid: bool = with_context(|ctx| ctx.eval(r#"(() => {
      const source = { date: new Date(11), map: new Map([['x', 2]]),
        set: new Set([3]), regexp: /a/g, big: 123n, bytes: Buffer.from([4]) };
      const saved = {
        Date: globalThis.Date, Map: globalThis.Map, Set: globalThis.Set,
        RegExp: globalThis.RegExp, BigInt: globalThis.BigInt,
        parse: JSON.parse, stringify: JSON.stringify, keys: Object.keys,
        from: Buffer.from, mapSet: Map.prototype.set,
        setAdd: Set.prototype.add, regexpTest: RegExp.prototype.test,
        retain: globalThis.__thaw_retain_dynamic_value,
        release: globalThis.__thaw_release_dynamic_value,
      };
      try {
        const fail = () => { throw new Error('replaced intrinsic'); };
        globalThis.Date = globalThis.Map = globalThis.Set = globalThis.RegExp = fail;
        globalThis.BigInt = fail;
        JSON.parse = JSON.stringify = Object.keys = Buffer.from = fail;
        saved.Map.prototype.set = fail; saved.Set.prototype.add = fail;
        saved.RegExp.prototype.test = fail;
        globalThis.__thaw_retain_dynamic_value = fail;
        globalThis.__thaw_release_dynamic_value = fail;
        const graph = __thaw_json_graph_encode_js(source);
        const result = __thaw_json_graph_decode_owned(graph);
        const symbol = Symbol('live');
        const live = __thaw_json_graph_decode_owned(
          __thaw_json_graph_encode_js([symbol], 0, true));
        return result.date.getTime() === 11 && result.map.get('x') === 2
          && result.set.has(3) && result.big === 123n
          && saved.regexpTest.call(result.regexp, 'a')
          && result.bytes[0] === 4 && live[0] === symbol;
      } finally {
        globalThis.Date = saved.Date; globalThis.Map = saved.Map;
        globalThis.Set = saved.Set; globalThis.RegExp = saved.RegExp;
        globalThis.BigInt = saved.BigInt; JSON.parse = saved.parse;
        JSON.stringify = saved.stringify; Object.keys = saved.keys;
        Buffer.from = saved.from; saved.Map.prototype.set = saved.mapSet;
        saved.Set.prototype.add = saved.setAdd;
        saved.RegExp.prototype.test = saved.regexpTest;
        globalThis.__thaw_retain_dynamic_value = saved.retain;
        globalThis.__thaw_release_dynamic_value = saved.release;
      }
    })()"#).expect("graph codec captured intrinsic source check"));
    assert!(valid);
}

unsafe fn take_owned_string(value: *const c_char) -> String {
    if value.is_null() {
        String::new()
    } else {
        let text = unsafe { CStr::from_ptr(value) }.to_string_lossy().into_owned();
        unsafe { thaw_arena::destroy_string(value.cast_mut()) };
        text
    }
}

#[no_mangle]
pub extern "C" fn thaw_js_register_native_callback(
    adapter: *const c_void,
    closure: *const c_void,
    jsvalue_param_mask: u64,
    param_count: u64,
    void_result: u8,
    finish: *const c_void,
    has_rest: u8,
) -> ThawHandleResult {
    register_native_callback(adapter, closure, jsvalue_param_mask, param_count, void_result, finish, has_rest, false, false)
}

#[no_mangle]
pub extern "C" fn thaw_js_register_native_callback_graph(
    adapter: *const c_void,
    closure: *const c_void,
    jsvalue_param_mask: u64,
    param_count: u64,
    void_result: u8,
    finish: *const c_void,
    has_rest: u8,
) -> ThawHandleResult {
    register_native_callback(adapter, closure, jsvalue_param_mask, param_count, void_result, finish, has_rest, true, false)
}

/// An object-literal method keeps its native closure identity while its
/// explicit leading receiver parameter is supplied from call-time `this`.
#[no_mangle]
pub extern "C" fn thaw_js_register_native_method_callback_graph(
    adapter: *const c_void,
    closure: *const c_void,
    jsvalue_param_mask: u64,
    param_count: u64,
    void_result: u8,
    finish: *const c_void,
    has_rest: u8,
) -> ThawHandleResult {
    register_native_callback(adapter, closure, jsvalue_param_mask, param_count, void_result, finish, has_rest, true, true)
}

#[allow(clippy::too_many_arguments)]
fn register_native_callback(
    adapter: *const c_void,
    closure: *const c_void,
    jsvalue_param_mask: u64,
    param_count: u64,
    void_result: u8,
    finish: *const c_void,
    has_rest: u8,
    graph_mode: bool,
    method_mode: bool,
) -> ThawHandleResult {
    let adapter = adapter as usize;
    let closure = closure as usize;
    let finish = finish as usize;
    let result: Result<u64, String> = with_active_or_context(|ctx| {
        let identity = if closure == 0 { adapter } else { closure };
        let cache_key = (
            identity,
            jsvalue_param_mask,
            param_count,
            void_result,
            finish,
            has_rest,
            graph_mode,
            method_mode,
        );
        // A typed closure decoded from a JS Function carries an original
        // handle. Return that value itself, preserving strict identity and
        // receiver semantics across native static slots and saved aliases.
        unsafe extern "C" {
            fn thaw_json_callback_origin_handle(closure: *const u8) -> u64;
        }
        let origin = unsafe { thaw_json_callback_origin_handle(closure as *const u8) };
        if origin != 0 {
            if let Ok(original) = value_for_handle(&ctx, origin) {
                return retain_value(&ctx, original);
            }
        }
        // The generated adapter fixes the full native parameter/return and
        // receiver-layout ABI. Arity and JsValue mask alone cannot
        // distinguish two typed views of one closure pointer.
        let method_token = format!(
            "{identity:x}:{adapter:x}:{jsvalue_param_mask}:{param_count}:{void_result}:{finish:x}:{has_rest}:{graph_mode}:{method_mode}"
        );
        if method_mode {
            let bridge = ctx.userdata::<NativeCallbackIdentityBridge<'_>>()
                .ok_or("native callback identity bridge is unavailable")?;
            let lookup = bridge.method_lookup.clone();
            drop(bridge);
            let cached: Value = lookup.call((method_token.clone(),))
                .map_err(|error| error.to_string())?;
            if !cached.is_undefined() {
                return retain_value(&ctx, cached);
            }
        }
        // Method identity is keyed by the complete generated adapter ABI in
        // the private weak cache above. The legacy strong callback cache omits
        // that adapter and does not grant a new reference on a hit.
        if !method_mode {
            if let Some(handle) = NATIVE_CALLBACK_HANDLES.with(|handles| {
                handles
                    .borrow()
                    .get(&cache_key)
                    .copied()
                    .filter(|handle| value_for_handle(&ctx, *handle).is_ok())
            }) {
                // Every registration result is an independently owned
                // reference, even when the wrapper identity is cached.
                return retain_value(&ctx, value_for_handle(&ctx, handle)?);
            }
        }
        // The Rust-backed half stays a plain `String -> String` closure --
        // no `Value<'js>` anywhere in its own signature -- deliberately:
        // an `IntoJsFunc` closure returning a value borrowed from `Ctx<'js>`
        // needs every `'_` in its own signature unified with the *call-
        // time* `Ctx<'js>` (not the outer one this function registers it
        // with), which an ordinary closure literal doesn't infer on its
        // own here. A thin JS-side wrapper (below) does the real
        // JSON-encode-args / JSON.parse-with-reviver-result work instead,
        // exactly the same convention `install_napi_bridge`'s own
        // string-only closures already use for the identical reason.
        static NEXT_NATIVE_CALLBACK_ID: std::sync::atomic::AtomicU64 =
            std::sync::atomic::AtomicU64::new(0);
        let id = NEXT_NATIVE_CALLBACK_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let root = thaw_arena::ArenaRoot::new(closure);
        let raw = Function::new(ctx.clone(), move |ctx: Ctx<'_>, args_json: String| -> String {
            let _keep_alive = &root;
            let args_json = CString::new(args_json).unwrap_or_default();
            // Enters the same "currently active `Ctx`" guard `install_
            // napi_bridge`'s own `call`/`handle` closures already use --
            // needed because the native closure this adapter is about to
            // invoke may itself call back into another dynamic operation
            // (a further `callDynamicMethod`/etc., real example: zod's
            // `.superRefine((val, ctx) => { ctx.addIssue(...); })`), which
            // would otherwise try to `with_context` a *second* time while
            // the outer call (the one that invoked *this* callback in the
            // first place) is still on the stack, panicking ("RefCell
            // already borrowed") since `with_context`'s own thread-local
            // slot has no reentrant-borrow support. See `thaw_js_call_
            // method_result`'s own matching `ACTIVE_NAPI_CONTEXT` check,
            // the consuming half of this -- despite the "napi" name, this
            // guard is a generic "currently active Ctx" mechanism, not
            // N-API-specific.
            let _active = ActiveNapiContext::enter(&ctx);
            // SAFETY: `adapter` was produced by `compile_napi_value_
            // callback` and always has exactly this ABI; `closure` is its
            // matching, still-live captured-environment pointer (the
            // compiled program keeps the whole closure alive for as long
            // as this registered callback might be called).
            let adapter_fn: NativeCallbackAdapter = unsafe { std::mem::transmute(adapter) };
            // The JS graph encoder already retained the leases this wire names;
            // grant the adapter's `thaw_json_graph_decode` authority over exactly
            // this buffer, and return the leases if the adapter never consumed it.
            let granted = graph_mode && unsafe { thaw_arena::register_owned_graph_wire(args_json.as_ptr()) };
            let result = unsafe { adapter_fn(closure as *const c_void, args_json.as_ptr()) };
            if granted {
                unsafe extern "C" { fn thaw_json_discard_graph_wire(source: *const c_char); }
                unsafe { thaw_json_discard_graph_wire(args_json.as_ptr()) };
            }
            if finish == 0 {
                unsafe { take_owned_string(result) }
            } else {
                native_promise_mark_handled(result.cast::<c_void>().cast_mut());
                format!("promise:{:x}", result as usize)
            }
        })
        .map_err(|error| error.to_string())?;
        let raw_name = format!("__thaw_native_callback_raw_{id}");
        ctx.globals()
            .set(raw_name.as_str(), raw)
            .map_err(|error| error.to_string())?;
        let poll_name = format!("__thaw_native_callback_poll_{id}");
        if finish != 0 {
            let poll = Function::new(ctx.clone(), move |ctx: Ctx<'_>, ticket: String| -> String {
                let _active = ActiveNapiContext::enter(&ctx);
                let Some(address) = ticket.strip_prefix("promise:") else {
                    return "error:invalid native Promise ticket".to_string();
                };
                let Ok(address) = usize::from_str_radix(address, 16) else {
                    return "error:invalid native Promise address".to_string();
                };
                let promise = address as *const c_void;
                // The finisher consumes settled promises. Check first so a
                // rejection cannot look pending and be consumed repeatedly.
                let state = native_promise_state(promise);
                if state == 0 {
                    return String::new();
                }
                let finish_fn: NativeCallbackAdapter = unsafe { std::mem::transmute(finish) };
                let result = unsafe { finish_fn(promise, std::ptr::null()) };
                if state == 2 {
                    return if result.is_null() {
                        "error:native Promise rejected".to_string()
                    } else {
                        // Keep the *full* tagged exception (`\u{1}name\u{1}
                        // message[\u{5}properties]`), not just the message:
                        // `__thaw_error_from_tagged` on the JS side rebuilds
                        // the original name and custom properties from it
                        // (real trigger: a rejected `ctx.throw(418, ...)`,
                        // whose `.status`/`.expose` koa's error handler
                        // needs).
                        format!("error:{}", to_str(result))
                    };
                }
                if result.is_null() {
                    return String::new();
                }
                unsafe { take_owned_string(result) }
            })
            .map_err(|error| error.to_string())?;
            ctx.globals()
                .set(poll_name.as_str(), poll)
                .map_err(|error| error.to_string())?;
        }
        // Exposes `retain_value` to JS via a plain named function (not a
        // closure literal): `ctx`/`value` need the *same* `'js`
        // (`retain_value` requires it), and a closure's own two
        // independently-elided `'_` params don't unify to one on their
        // own here -- the same lifetime wall the module doc comment
        // above hit for a *returned* `Value<'js>`, this time for two
        // *different* params that must agree. A named function's normal
        // `for<'js>` generic already satisfies `Function::new`'s own
        // bound for any 'js, no closure-lifetime inference involved at
        // all. Reinstalled on every call rather than checked for
        // idempotently first: cheap, and simpler than threading a "is it
        // already there" check through this same function.
        install_graph_handle_functions(&ctx).map_err(|error| error.to_string())?;
        // Marks exactly the argument positions `compile_register_native_
        // callback` (thaw-llvm) declared `JsValue`-typed -- real example:
        // zod's `.superRefine((val, ctx: JsValue) => { ctx.addIssue(...);
        // })`, whose `ctx` is a live object with methods, not JSON-
        // representable data. Retained as a handle and encoded as the
        // same `{"__thaw_js_handle_id__": N}` marker `compile_dynamic_
        // value_placeholder` builds for the opposite direction, which
        // `compile_json_value_to_native`'s own new `HirType::JsValue`
        // case decodes back out on the native side (this marshaling
        // direction has no JS-side reviver to lean on -- the adapter
        // parses `args_json` natively, never through QuickJS's own
        // `JSON.parse`). `1 << i` is a plain 32-bit JS bitwise op --
        // plenty for any real callback's own arity.
        //
        // The replacer additionally retains *any* function argument as a
        // handle, not only the masked positions: a JS function handed to a
        // compiled closure's `Json`/`any` parameter (real example: cors's
        // `(origin: any, callback: any) => callback(null, origin)`) has no
        // JSON encoding, so `JSON.stringify` would silently drop it and the
        // parameter arrived `undefined`. The `{"__thaw_js_handle_id__": N}`
        // shape is exactly what `compile_json_value_to_native`'s `Json`
        // case passes through and what the `Json`-callee dynamic-call path
        // recovers via `JsonAsNative`, so the function stays callable.
        let visible_param_count = param_count - u64::from(method_mode);
        let wrapper_source = format!(
            "(function() {{ \
             var raw = globalThis['{raw_name}']; \
             delete globalThis['{raw_name}']; \
             var poll = globalThis['{poll_name}']; \
             if ({method_mode}) delete globalThis['{poll_name}']; \
             var mask = {jsvalue_param_mask}; \
             var callback = function() {{ \
             var args = {has_rest} \
             ? Array.prototype.slice.call(arguments, 0, {visible_param_count} - 1).concat([Array.prototype.slice.call(arguments, {visible_param_count} - 1)]) \
             : Array.prototype.slice.call(arguments, 0, {visible_param_count}); \
             for (var i = 0; i < args.length; i++) {{ \
             if (!{graph_mode} && (mask & (1 << i)) !== 0) {{ \
             args[i] = {{ __thaw_js_handle_id__: globalThis.__thaw_retain_dynamic_value(args[i]) }}; \
             }} \
             }} \
             var result = raw({graph_mode} ? globalThis.__thaw_json_graph_encode_js([this].concat(args), {method_mode} ? mask : mask << 1, true) : JSON.stringify(args, function(key, value) {{ \
             if (typeof value === 'function') return {{ __thaw_js_handle_id__: globalThis.__thaw_retain_dynamic_value(value) }}; \
             if (value === undefined) return {{ $__thaw_napi_undefined$: true }}; \
             return globalThis.__thaw_json_binary_replacer.call(this, key, value); \
             }})); \
             if (result.charCodeAt(0) === 2) {{ \
             throw globalThis.__thaw_error_from_tagged(result.slice(1)); \
             }} \
             if ({void_result} && result.slice(0, 8) !== 'promise:') return undefined; \
             if (result.slice(0, 8) !== 'promise:') return {graph_mode} ? globalThis.__thaw_json_graph_decode_owned(result) : JSON.parse(result, globalThis.__thaw_json_date_reviver); \
             return new Promise(function(resolve, reject) {{ \
             function check() {{ \
             var settled = ({method_mode} ? poll : globalThis['{poll_name}'])(result); \
             if (!settled) return setTimeout(check, 0); \
             if (settled.slice(0, 6) === 'error:') return reject(globalThis.__thaw_error_from_tagged(settled.slice(6))); \
             if ({void_result}) return resolve(undefined); \
             try {{ resolve({graph_mode} ? globalThis.__thaw_json_graph_decode_owned(settled) : JSON.parse(settled, globalThis.__thaw_json_date_reviver)); }} catch (error) {{ reject(error); }} \
             }} \
             check(); \
             }}); \
             }}; \
             return callback; \
             }})()"
        );
        let wrapper: Value = ctx
            .eval(wrapper_source.as_str())
            .map_err(|error| error.to_string())?;
        let bridge = ctx.userdata::<NativeCallbackIdentityBridge<'_>>()
            .ok_or("native callback identity bridge is unavailable")?;
        let define_length = bridge.define_callback_length.clone();
        drop(bridge);
        define_length.call::<_, ()>((wrapper.clone(), visible_param_count))
            .map_err(|error| error.to_string())?;
        let wrapper = if method_mode {
            let bridge = ctx.userdata::<NativeCallbackIdentityBridge<'_>>()
                .ok_or("native callback identity bridge is unavailable")?;
            let intern = bridge.method_intern.clone();
            drop(bridge);
            intern.call::<_, Value>((method_token.clone(), wrapper))
                .map_err(|error| error.to_string())?
        } else { wrapper };
        if closure != 0 {
            let bridge = ctx.userdata::<NativeCallbackIdentityBridge<'_>>()
                .ok_or("native callback identity bridge is unavailable")?;
            let register = bridge.register.clone();
            drop(bridge);
            register.call::<_, ()>((wrapper.clone(), if method_mode {
                method_token
            } else { format!("{closure:x}") }))
                .map_err(|error| error.to_string())?;
        }
        let handle = retain_value(&ctx, wrapper)?;
        if !method_mode {
            NATIVE_CALLBACK_HANDLES.with(|handles| {
                handles.borrow_mut().insert(cache_key, handle);
            });
        }
        Ok(handle)
    });
    match result {
        Ok(value) => ThawHandleResult {
            value,
            error: std::ptr::null(),
        },
        Err(error) => ThawHandleResult {
            value: 0,
            error: thaw_arena::owned_string(error),
        },
    }
}

#[no_mangle]
pub extern "C" fn thaw_js_call_handle_handle_result(
    handle: u64, args_json: *const c_char, defer_resolution: bool,
) -> ThawHandleResult {
    thaw_js_call_handle_handle_impl(handle, args_json, defer_resolution, false)
}

#[no_mangle]
pub extern "C" fn thaw_js_call_handle_handle_graph_args_result(
    handle: u64, args_json: *const c_char, defer_resolution: bool,
) -> ThawHandleResult {
    thaw_js_call_handle_handle_impl(handle, args_json, defer_resolution, true)
}

/// N-API `napi_call_function` supplies an explicit receiver. The graph root
/// is `[thisArg, ...arguments]` so the receiver and parameters share one
/// identity table and one lease scope. Returns the raw JS result as an owned
/// handle; N-API must not assimilate a returned Promise synchronously.
#[repr(C)]
pub struct ThawCallWithExceptionResult {
    pub value: u64,
    pub exception_handle: u64,
    pub exception_object_like: u8,
    pub error: *const c_char,
}

#[no_mangle]
pub extern "C" fn thaw_js_call_handle_with_this_graph_result(
    handle: u64, args_json: *const c_char,
) -> ThawCallWithExceptionResult {
    enum Outcome { Value(u64), Exception(u64, bool) }
    let args_json = to_str(args_json);
    let result: Result<Outcome, String> = with_active_or_context(|ctx| {
        let target = before_graph_decode(&ctx, &args_json, true, || {
            Function::from_value(value_for_handle(&ctx, handle)?)
                .map_err(|_| format!("JavaScript value handle {handle} is not callable"))
        })?;
        let args_array = decode_argument_array(&ctx, &args_json, true)?;
        if args_array.is_empty() { return Err("missing JavaScript call receiver".into()); }
        let this_value: Value = args_array.get(0).map_err(|error| error.to_string())?;
        let mut args = Args::new_unsized(ctx.clone());
        args.this(this_value).map_err(|error| error.to_string())?;
        for index in 1..args_array.len() {
            let value: Value = args_array.get(index).map_err(|error| error.to_string())?;
            args.push_arg(value).map_err(|error| error.to_string())?;
        }
        let returned: Value = match target.call_arg(args) {
            Ok(value) => value,
            Err(rquickjs::Error::Exception) => {
                let thrown = ctx.catch();
                let object_like = thrown.is_object() || thrown.is_function();
                return retain_value(&ctx, thrown)
                    .map(|handle| Outcome::Exception(handle, object_like));
            }
            Err(error) => return Err(error.to_string()),
        };
        retain_value(&ctx, returned).map(Outcome::Value)
    });
    match result {
        Ok(Outcome::Value(value)) => ThawCallWithExceptionResult {
            value, exception_handle: 0, exception_object_like: 0,
            error: std::ptr::null(),
        },
        Ok(Outcome::Exception(exception_handle, object_like)) => ThawCallWithExceptionResult {
            value: 0, exception_handle, exception_object_like: u8::from(object_like),
            error: std::ptr::null(),
        },
        Err(error) => ThawCallWithExceptionResult {
            value: 0, exception_handle: 0, exception_object_like: 0,
            error: thaw_arena::owned_string(error),
        },
    }
}

fn thaw_js_call_handle_handle_impl(
    handle: u64, args_json: *const c_char, defer_resolution: bool, graph_args: bool,
) -> ThawHandleResult {
    let args_json = to_str(args_json);
    let result: Result<u64, String> = with_active_or_context(|ctx| {
        let target = before_graph_decode(&ctx, &args_json, graph_args, || {
            Function::from_value(value_for_handle(&ctx, handle)?)
                .map_err(|_| format!("JavaScript value handle {handle} is not callable"))
        })?;
        let result = invoke_raw(ctx.clone(), target, &args_json, graph_args)?;
        let result = if defer_resolution {
            result
        } else {
            resolve_promise_value(&ctx, result, &format!("JavaScript value #{handle}"), true)?
        };
        retain_value(&ctx, result)
    });
    match result {
        Ok(value) => ThawHandleResult {
            value,
            error: std::ptr::null(),
        },
        Err(error) => ThawHandleResult {
            value: 0,
            error: thaw_arena::owned_string(error),
        },
    }
}

fn thaw_js_call_handle_value_result_impl(handle: u64, argument: u64, graph_result: bool) -> ThawResult {
    let result = with_active_or_context(|ctx| {
        let target = Function::from_value(value_for_handle(&ctx, handle)?)
            .map_err(|_| format!("JavaScript value handle {handle} is not callable"))?;
        let argument = value_for_handle(&ctx, argument)?;
        // See `invoke_raw`'s doc comment -- same call convention, same fix.
        let value: Value = target.call((argument,)).map_err(|error| match error {
            rquickjs::Error::Exception => describe_tagged_exception(&ctx),
            error => error.to_string(),
        })?;
        resolve_value_wire_impl(ctx, value, &format!("JavaScript value #{handle}"), true, graph_result)
    });
    match result {
        Ok(text) => ThawResult {
            value: thaw_arena::owned_string(text),
            error: std::ptr::null(),
        },
        Err(error) => ThawResult {
            value: std::ptr::null(),
            error: thaw_arena::owned_string(error),
        },
    }
}

#[no_mangle]
pub extern "C" fn thaw_js_retain_handle(handle: u64) -> u8 {
    with_active_or_context(|ctx| {
        if handle == 0 || value_for_handle(&ctx, handle).is_err() {
            return 0;
        }
        let Ok(refs) = ctx.globals().get::<_, Array>("__thaw_value_handle_refs") else {
            return 0;
        };
        let index = (handle - 1) as usize;
        let count = refs.get::<u64>(index).unwrap_or(1);
        u8::from(refs.set(index, count + 1).is_ok())
    })
}

#[no_mangle]
pub extern "C" fn thaw_js_release_handle(handle: u64) -> u8 {
    with_active_or_context(|ctx| release_handle_in_context(&ctx, handle))
}

fn release_handle_in_context(ctx: &Ctx<'_>, handle: u64) -> u8 {
    if handle == 0 {
        return 0;
    }
    let Ok(handles) = handle_array(ctx) else {
        return 0;
    };
    let index = (handle - 1) as usize;
    let Ok(live) = ctx.globals().get::<_, Array>("__thaw_value_handle_live") else {
        return 0;
    };
    let Ok(refs) = ctx.globals().get::<_, Array>("__thaw_value_handle_refs") else {
        return 0;
    };
    if !live.get::<bool>(index).unwrap_or(false) {
        return 0;
    }
    let count = refs.get::<u64>(index).unwrap_or(1);
    if count > 1 {
        return u8::from(refs.set(index, count - 1).is_ok());
    }
    if live.set(index, false).is_err() {
        return 0;
    }
    let _ = refs.set(index, 0u64);
    let cleared = handles.set(index, Value::new_undefined(ctx.clone())).is_ok();
    if cleared {
        NATIVE_CALLBACK_HANDLES.with(|cache| cache.borrow_mut().retain(|_, cached| *cached != handle));
    }
    u8::from(cleared)
}

#[no_mangle]
pub extern "C" fn thaw_js_release_all_handles() -> u64 {
    with_context(|ctx| {
        let count = handle_array(&ctx).map(|handles| handles.len()).unwrap_or(0);
        let Ok(handles) = Array::new(ctx.clone()) else {
            return 0;
        };
        let Ok(live) = Array::new(ctx.clone()) else {
            return 0;
        };
        let Ok(refs) = Array::new(ctx.clone()) else {
            return 0;
        };
        if ctx.globals().set("__thaw_value_handles", handles).is_err()
            || ctx.globals().set("__thaw_value_handle_live", live).is_err()
            || ctx.globals().set("__thaw_value_handle_refs", refs).is_err()
        {
            return 0;
        }
        NATIVE_CALLBACK_HANDLES.with(|cache| cache.borrow_mut().clear());
        count as u64
    })
}

/// Lossless UTF-16 property key predicate for N-API-backed live JS values.
/// `operation`: 0 has, 1 own-has, 2 delete.
#[no_mangle]
pub extern "C" fn thaw_js_property_predicate_json_key_result(
    handle: u64, key_json: *const c_char, operation: u8,
) -> ThawHandleResult {
    let key_json = to_str(key_json);
    let result: Result<u64, String> = with_active_or_context(|ctx| {
        let object = object_for_handle(&ctx, handle)?;
        if operation == 3 {
            let predicate: Function = ctx.globals()
                .get("__thaw_host_property_is_enumerable")
                .map_err(|error| error.to_string())?;
            let result: bool = predicate.call((object, key_json.as_str()))
                .map_err(|error| match error {
                    rquickjs::Error::Exception => describe_tagged_exception(&ctx),
                    error => error.to_string(),
                })?;
            return Ok(u64::from(result));
        }
        let predicate: Function = ctx.globals().get("__thaw_json_host_key_predicate")
            .map_err(|error| error.to_string())?;
        predicate.call::<_, bool>((object, key_json.as_str(), operation, true))
            .map(u64::from).map_err(|error| match error {
                rquickjs::Error::Exception => describe_tagged_exception(&ctx),
                error => error.to_string(),
            })
    });
    match result {
        Ok(value) => ThawHandleResult { value, error: std::ptr::null() },
        Err(error) => ThawHandleResult { value: 0, error: thaw_arena::owned_string(error) },
    }
}

#[no_mangle]
pub extern "C" fn thaw_js_get_property_result(
    handle: u64,
    name: *const c_char,
) -> ThawHandleResult {
    let name = to_str(name);
    let result: Result<u64, String> = with_active_or_context(|ctx| {
        let object = boxed_object_for_handle(&ctx, handle)?;
        // See `invoke_raw`'s doc comment -- a getter can throw too.
        let key = dynamic_property_key(&ctx, &name)?;
        let key = rquickjs::Atom::from_value(ctx.clone(), &key)
            .map_err(|error| error.to_string())?;
        let value = object.get(key).map_err(|error| match error {
            rquickjs::Error::Exception => describe_tagged_exception(&ctx),
            error => error.to_string(),
        })?;
        retain_value(&ctx, value)
    });
    match result {
        Ok(value) => ThawHandleResult {
            value,
            error: std::ptr::null(),
        },
        Err(error) => ThawHandleResult {
            value: 0,
            error: thaw_arena::owned_string(error),
        },
    }
}

/// The live Json host bridge passes a JSON string literal, rather than a raw
/// C string, so a key containing NUL or a lone surrogate retains its exact
/// UTF-16 property identity. The returned handle owns one registry reference.
#[no_mangle]
pub extern "C" fn thaw_js_get_property_json_key_result(
    handle: u64,
    key_json: *const c_char,
) -> ThawHandleResult {
    let key_json = to_str(key_json);
    let result: Result<u64, String> = with_active_or_context(|ctx| {
        let object = boxed_object_for_handle(&ctx, handle)?;
        let json: Object = ctx.globals().get("JSON").map_err(|error| error.to_string())?;
        let parse: Function = json.get("parse").map_err(|error| error.to_string())?;
        let key: Value = parse.call((key_json.as_str(),)).map_err(|error| error.to_string())?;
        if !key.is_string() {
            return Err("Invalid host property key".into());
        }
        let key = rquickjs::Atom::from_value(ctx.clone(), &key)
            .map_err(|error| error.to_string())?;
        let value = object.get(key).map_err(|error| match error {
            rquickjs::Error::Exception => describe_tagged_exception(&ctx),
            error => error.to_string(),
        })?;
        retain_value(&ctx, value)
    });
    match result {
        Ok(value) => ThawHandleResult { value, error: std::ptr::null() },
        Err(error) => ThawHandleResult {
            value: 0,
            error: thaw_arena::owned_string(error),
        },
    }
}

/// Read a property using a retained JS key, preserving symbol identity.
#[no_mangle]
pub extern "C" fn thaw_js_get_property_key_handle_result(
    handle: u64, key_handle: u64,
) -> ThawHandleResult {
    let result: Result<u64, String> = with_active_or_context(|ctx| {
        let object = object_for_handle(&ctx, handle)?;
        let key_value = value_for_handle(&ctx, key_handle)?;
        let key = rquickjs::Atom::from_value(ctx.clone(), &key_value)
            .map_err(|error| error.to_string())?;
        let value = object.get(key).map_err(|error| match error {
            rquickjs::Error::Exception => describe_tagged_exception(&ctx),
            error => error.to_string(),
        })?;
        retain_value(&ctx, value)
    });
    match result {
        Ok(value) => ThawHandleResult { value, error: std::ptr::null() },
        Err(error) => ThawHandleResult { value: 0, error: thaw_arena::owned_string(error) },
    }
}


/// N-API-only property read result. The ordinary HostOperations read ABI above
/// remains handle-or-text; this path retains a thrown JS value before leaving
/// the QuickJS context so napi_get_property can expose the same exception.
fn property_read_with_exception_result(
    handle: u64, key_json: Option<String>, key_handle: u64,
) -> ThawCallWithExceptionResult {
    enum Outcome { Value(u64), Exception(u64, bool) }
    let result: Result<Outcome, String> = with_active_or_context(|ctx| {
        let object = if key_json.is_some() {
            boxed_object_for_handle(&ctx, handle)?
        } else {
            object_for_handle(&ctx, handle)?
        };
        let key_value: Value = if let Some(key_json) = key_json.as_ref() {
            let parse: Function = ctx.globals().get("__thaw_json_host_parse_property_key")
                .map_err(|error| error.to_string())?;
            parse.call((key_json.as_str(),)).map_err(|error| error.to_string())?
        } else {
            value_for_handle(&ctx, key_handle)?
        };
        let key = rquickjs::Atom::from_value(ctx.clone(), &key_value)
            .map_err(|error| error.to_string())?;
        match object.get::<_, Value>(key) {
            Ok(value) => retain_value(&ctx, value).map(Outcome::Value),
            Err(rquickjs::Error::Exception) => {
                let thrown = ctx.catch();
                let object_like = thrown.is_object() || thrown.is_function();
                retain_value(&ctx, thrown).map(|handle| Outcome::Exception(handle, object_like))
            }
            Err(error) => Err(error.to_string()),
        }
    });
    match result {
        Ok(Outcome::Value(value)) => ThawCallWithExceptionResult {
            value, exception_handle: 0, exception_object_like: 0,
            error: std::ptr::null(),
        },
        Ok(Outcome::Exception(exception_handle, object_like)) => ThawCallWithExceptionResult {
            value: 0, exception_handle, exception_object_like: u8::from(object_like),
            error: std::ptr::null(),
        },
        Err(error) => ThawCallWithExceptionResult {
            value: 0, exception_handle: 0, exception_object_like: 0,
            error: thaw_arena::owned_string(error),
        },
    }
}

#[no_mangle]
pub extern "C" fn thaw_js_get_property_json_key_with_exception_result(
    handle: u64, key_json: *const c_char,
) -> ThawCallWithExceptionResult {
    property_read_with_exception_result(handle, Some(to_str(key_json)), 0)
}

#[no_mangle]
pub extern "C" fn thaw_js_get_property_key_handle_with_exception_result(
    handle: u64, key_handle: u64,
) -> ThawCallWithExceptionResult {
    property_read_with_exception_result(handle, None, key_handle)
}

#[no_mangle]
pub extern "C" fn thaw_js_property_predicate_key_handle_result(
    handle: u64, key_handle: u64, operation: u8,
) -> ThawHandleResult {
    let result: Result<u64, String> = with_active_or_context(|ctx| {
        let object = object_for_handle(&ctx, handle)?;
        let key = value_for_handle(&ctx, key_handle)?;
        let predicate: Function = ctx.globals().get("__thaw_json_host_key_predicate")
            .map_err(|error| error.to_string())?;
        let value: bool = predicate.call((object, key, operation, false)).map_err(|error| match error {
            rquickjs::Error::Exception => describe_tagged_exception(&ctx),
            error => error.to_string(),
        })?;
        Ok(u64::from(value))
    });
    match result {
        Ok(value) => ThawHandleResult { value, error: std::ptr::null() },
        Err(error) => ThawHandleResult { value: 0, error: thaw_arena::owned_string(error) },
    }
}

#[no_mangle]
pub extern "C" fn thaw_js_set_property_key_handle_graph_result(
    handle: u64, key_handle: u64, graph_json: *const c_char, receiver_data: u8,
) -> ThawHandleResult {
    let graph_json = to_str(graph_json);
    let result: Result<u64, String> = with_active_or_context(|ctx| {
        let (object, key, decode, set) = before_graph_decode(&ctx, &graph_json, true, || {
            let object = object_for_handle(&ctx, handle)?;
            let key = value_for_handle(&ctx, key_handle)?;
            let decode: Function = ctx.globals().get("__thaw_json_graph_decode_owned")
                .map_err(|error| error.to_string())?;
            let set: Function = ctx.globals().get(if receiver_data != 0 {
                "__thaw_json_host_set_receiver_data"
            } else {
                "__thaw_json_host_set_property"
            })
                .map_err(|error| error.to_string())?;
            Ok((object, key, decode, set))
        })?;
        let value: Value = decode.call((graph_json.as_str(),)).map_err(|error| match error {
            rquickjs::Error::Exception => describe_tagged_exception(&ctx),
            error => error.to_string(),
        })?;
        let written: bool = set.call((object, key, value)).map_err(|error| match error {
            rquickjs::Error::Exception => describe_tagged_exception(&ctx),
            error => error.to_string(),
        })?;
        if !written { return Err("\u{1}TypeError\u{1}Cannot assign to read only property".into()); }
        Ok(1)
    });
    match result {
        Ok(value) => ThawHandleResult { value, error: std::ptr::null() },
        Err(error) => ThawHandleResult { value: 0, error: thaw_arena::owned_string(error) },
    }
}

#[no_mangle]
pub extern "C" fn thaw_js_define_data_property_key_handle_graph_result(
    handle: u64, key_handle: u64, graph_json: *const c_char, attributes: u32,
) -> ThawHandleResult {
    let graph_json = to_str(graph_json);
    let result: Result<u64, String> = with_active_or_context(|ctx| {
        let (object, key, decode, define) = before_graph_decode(&ctx, &graph_json, true, || {
            let object = object_for_handle(&ctx, handle)?;
            let key = value_for_handle(&ctx, key_handle)?;
            let decode: Function = ctx.globals().get("__thaw_json_graph_decode_owned")
                .map_err(|error| error.to_string())?;
            let define: Function = ctx.globals().get("__thaw_json_host_define_data_property")
                .map_err(|error| error.to_string())?;
            Ok((object, key, decode, define))
        })?;
        let value: Value = decode.call((graph_json.as_str(),)).map_err(|error| match error {
            rquickjs::Error::Exception => describe_tagged_exception(&ctx),
            error => error.to_string(),
        })?;
        let defined: bool = define.call((object, key, value, attributes)).map_err(|error| match error {
            rquickjs::Error::Exception => describe_tagged_exception(&ctx),
            error => error.to_string(),
        })?;
        Ok(u64::from(defined))
    });
    match result {
        Ok(value) => ThawHandleResult { value, error: std::ptr::null() },
        Err(error) => ThawHandleResult { value: 0, error: thaw_arena::owned_string(error) },
    }
}

struct NativeGraphCallbackRoots([u64; 3]);

impl Drop for NativeGraphCallbackRoots {
    fn drop(&mut self) {
        let release = NAPI_BRIDGE.lock().unwrap().as_ref().map(|bridge| bridge.6);
        if let Some(release) = release {
            for token in self.0.iter().copied().filter(|token| *token != 0) {
                release(token);
            }
        }
    }
}

#[no_mangle]
pub extern "C" fn thaw_js_define_callback_property_key_handle_result(
    handle: u64, key_handle: u64, getter_id: u64, setter_id: u64,
    method_id: u64, attributes: u32,
    getter_root: u64, setter_root: u64, method_root: u64,
) -> ThawHandleResult {
    let roots = NativeGraphCallbackRoots([getter_root, setter_root, method_root]);
    let result: Result<u64, String> = with_active_or_context(move |ctx| {
        let object = object_for_handle(&ctx, handle)?;
        let key = value_for_handle(&ctx, key_handle)?;
        let define: Function = ctx.globals().get("__thaw_json_host_define_callback_property")
            .map_err(|error| error.to_string())?;
        let invoke = Function::new(ctx.clone(), move |ctx: Ctx<'_>, target: String, graph: String| {
            let _keep_native_functions = &roots;
            let Some(callback) = NAPI_BRIDGE.lock().unwrap().as_ref().map(|bridge| bridge.2) else {
                release_unconsumed_graph_leases(&ctx, &graph);
                return "{\"__thaw_error__\":\"Native addon bridge is unavailable\"}".to_string();
            };
            let Ok(target) = CString::new(target) else {
                release_unconsumed_graph_leases(&ctx, &graph);
                return "{\"__thaw_error__\":\"Invalid native callback\"}".to_string();
            };
            let Ok(graph_c) = CString::new(graph.as_str()) else {
                release_unconsumed_graph_leases(&ctx, &graph);
                return "{\"__thaw_error__\":\"Invalid callback graph\"}".to_string();
            };
            let operation = c"call_captured_graph";
            let empty = c"";
            let _active = ActiveNapiContext::enter(&ctx);
            unsafe { to_str(callback(operation.as_ptr(), target.as_ptr(),
                empty.as_ptr(), graph_c.as_ptr())) }
        }).map_err(|error| error.to_string())?;
        let defined: bool = define.call((object, key, getter_id, setter_id,
            method_id, attributes, invoke)).map_err(|error| match error {
            rquickjs::Error::Exception => describe_tagged_exception(&ctx),
            error => error.to_string(),
        })?;
        Ok(u64::from(defined))
    });
    match result {
        Ok(value) => ThawHandleResult { value, error: std::ptr::null() },
        Err(error) => ThawHandleResult { value: 0, error: thaw_arena::owned_string(error) },
    }
}

#[no_mangle]
pub extern "C" fn thaw_js_strict_equal_handles_result(
    left_handle: u64, right_handle: u64,
) -> ThawHandleResult {
    let result: Result<u64, String> = with_active_or_context(|ctx| {
        let left = value_for_handle(&ctx, left_handle)?;
        let right = value_for_handle(&ctx, right_handle)?;
        let compare: Function = ctx.globals().get("__thaw_json_host_strict_handles_equal")
            .map_err(|error| error.to_string())?;
        let equal: bool = compare.call((left, right)).map_err(|error| match error {
            rquickjs::Error::Exception => describe_tagged_exception(&ctx),
            error => error.to_string(),
        })?;
        Ok(u64::from(equal))
    });
    match result {
        Ok(value) => ThawHandleResult { value, error: std::ptr::null() },
        Err(error) => ThawHandleResult { value: 0, error: thaw_arena::owned_string(error) },
    }
}

// The caller admits only native scalar values. No graph walk or property
// lookup occurs on the compared value.
#[no_mangle]
pub extern "C" fn thaw_js_strict_equal_handle_scalar_result(
    handle: u64, kind: u8, number: f64, text: *const c_char,
) -> ThawHandleResult {
    let text = to_str(text);
    let result: Result<u64, String> = with_active_or_context(|ctx| {
        let left = value_for_handle(&ctx, handle)?;
        let compare: Function = ctx.globals().get("__thaw_json_host_strict_scalar_equal")
            .map_err(|error| error.to_string())?;
        let equal: bool = compare.call((left, kind, number, text.as_str())).map_err(|error| match error {
            rquickjs::Error::Exception => describe_tagged_exception(&ctx),
            error => error.to_string(),
        })?;
        Ok(u64::from(equal))
    });
    match result {
        Ok(value) => ThawHandleResult { value, error: std::ptr::null() },
        Err(error) => ThawHandleResult { value: 0, error: thaw_arena::owned_string(error) },
    }
}

/// Native Promise predicate for the N-API live handle carrier; no JS
/// `instanceof` or user-mutated constructor is consulted.
#[no_mangle]
pub extern "C" fn thaw_js_handle_is_promise_result(handle: u64) -> ThawHandleResult {
    let result = with_active_or_context(|ctx| {
        let value = value_for_handle(&ctx, handle)?;
        Ok::<u64, String>(u64::from(value.as_promise().is_some()))
    });
    match result {
        Ok(value) => ThawHandleResult { value, error: std::ptr::null() },
        Err(error) => ThawHandleResult { value: 0, error: thaw_arena::owned_string(error) },
    }
}

/// Verify a dual native/JS graph token against the private wrapper maps.
/// The wire's numeric native ID alone is not proof that its hdl names the
/// same live Function or Symbol. Called before N-API borrows its Env mutably.
#[no_mangle]
pub extern "C" fn thaw_js_graph_native_pair_matches_result(
    handle: u64, native_handle: u64, kind: u8,
) -> ThawHandleResult {
    let result = with_active_or_context(|ctx| {
        let value = value_for_handle(&ctx, handle)?;
        let verify: Function = ctx.globals()
            .get("__thaw_json_graph_native_pair_matches")
            .map_err(|error| error.to_string())?;
        let matches: bool = verify.call((value, native_handle.to_string(), kind))
            .map_err(|error| match error {
                rquickjs::Error::Exception => describe_tagged_exception(&ctx),
                error => error.to_string(),
            })?;
        Ok::<u64, String>(u64::from(matches))
    });
    match result {
        Ok(value) => ThawHandleResult { value, error: std::ptr::null() },
        Err(error) => ThawHandleResult { value: 0, error: thaw_arena::owned_string(error) },
    }
}

#[no_mangle]
pub extern "C" fn thaw_js_host_query_result(handle: u64, operation: u8) -> ThawResult {
    let result: Result<String, String> = with_active_or_context(|ctx| {
        let value = value_for_handle(&ctx, handle)?;
        // Callback-origin validation must not consult the user-replaceable
        // global Host query function. JS_IsFunction also accepts callable
        // Proxy values and does not invoke their user properties.
        if operation == 14 {
            return Ok(if value.is_function() { "1" } else { "0" }.to_string());
        }
        // 21: the native owner pointer (hex) behind a live native-object wrapper, or "".
        // The private identity WeakMap answers; no user property can forge it.
        if operation == 21 {
            let bridge = ctx.userdata::<NativeCallbackIdentityBridge<'_>>()
                .ok_or("native object identity bridge is unavailable")?;
            let pointer_of = bridge.object_pointer.clone();
            drop(bridge);
            return pointer_of.call((value, String::new())).map_err(|error| error.to_string());
        }
        let query: Function = ctx.globals().get("__thaw_json_host_query")
            .map_err(|error| error.to_string())?;
        query.call((value, operation)).map_err(|error| match error {
            rquickjs::Error::Exception => describe_tagged_exception(&ctx),
            error => error.to_string(),
        })
    });
    match result {
        Ok(value) => ThawResult {
            value: thaw_arena::owned_string(value), error: std::ptr::null(),
        },
        Err(error) => ThawResult {
            value: std::ptr::null(),
            error: thaw_arena::owned_string(error),
        },
    }
}

#[repr(C)]
pub struct ThawStringCoercionResult {
    pub units: *const c_char,
    pub exception_handle: u64,
    pub exception_object_like: u8,
    pub error: *const c_char,
}

#[no_mangle]
pub extern "C" fn thaw_js_abstract_to_string_units_result(
    handle: u64,
    array_element: u8,
) -> ThawStringCoercionResult {
    enum Outcome { Units(String), Exception(u64, bool) }
    let result: Result<Outcome, String> = with_active_or_context(|ctx| {
        let value = value_for_handle(&ctx, handle)?;
        if array_element != 0 && (value.is_null() || value.is_undefined()) {
            return Ok(Outcome::Units("[]".into()));
        }
        let coerce: Function = ctx.globals()
            .get("__thaw_json_host_abstract_to_string_units")
            .map_err(|error| error.to_string())?;
        match coerce.call::<_, String>((value,)) {
            Ok(units) => Ok(Outcome::Units(units)),
            Err(rquickjs::Error::Exception) => {
                let thrown = ctx.catch();
                let object_like = thrown.is_object() || thrown.is_function();
                retain_value(&ctx, thrown).map(|handle| Outcome::Exception(handle, object_like))
            }
            Err(error) => Err(error.to_string()),
        }
    });
    match result {
        Ok(Outcome::Units(units)) => ThawStringCoercionResult {
            units: thaw_arena::owned_string(units),
            exception_handle: 0, exception_object_like: 0,
            error: std::ptr::null(),
        },
        Ok(Outcome::Exception(exception_handle, object_like)) => ThawStringCoercionResult {
            units: std::ptr::null(), exception_handle,
            exception_object_like: u8::from(object_like), error: std::ptr::null(),
        },
        Err(error) => ThawStringCoercionResult {
            units: std::ptr::null(), exception_handle: 0, exception_object_like: 0,
            error: thaw_arena::owned_string(error),
        },
    }
}

#[no_mangle]
pub extern "C" fn thaw_js_host_date_set_result(handle: u64, timestamp: f64) -> ThawResult {
    let result: Result<String, String> = with_active_or_context(|ctx| {
        let value = value_for_handle(&ctx, handle)?;
        let setter: Function = ctx.globals().get("__thaw_json_host_date_set")
            .map_err(|error| error.to_string())?;
        setter.call((value, timestamp)).map_err(|error| match error {
            rquickjs::Error::Exception => describe_tagged_exception(&ctx),
            error => error.to_string(),
        })
    });
    match result {
        Ok(value) => ThawResult {
            value: thaw_arena::owned_string(value), error: std::ptr::null(),
        },
        Err(error) => ThawResult {
            value: std::ptr::null(),
            error: thaw_arena::owned_string(error),
        },
    }
}

fn thaw_js_set_property_graph_impl(
    handle: u64,
    key_json: *const c_char,
    graph_json: *const c_char,
    receiver_data: u8,
) -> ThawHandleResult {
    let key_json = to_str(key_json);
    let graph_json = to_str(graph_json);
    let result: Result<u64, String> = with_active_or_context(|ctx| {
        let (object, key, decode, set) = before_graph_decode(&ctx, &graph_json, true, || {
            let object = object_for_handle(&ctx, handle)?;
            let json: Object = ctx.globals().get("JSON").map_err(|error| error.to_string())?;
            let parse: Function = json.get("parse").map_err(|error| error.to_string())?;
            let key: Value = parse.call((key_json.as_str(),)).map_err(|error| error.to_string())?;
            if !key.is_string() { return Err("Invalid host property key".into()); }
            let decode: Function = ctx.globals().get("__thaw_json_graph_decode_owned")
                .map_err(|error| error.to_string())?;
            let set: Function = ctx.globals().get(if receiver_data != 0 {
                "__thaw_json_host_set_receiver_data"
            } else {
                "__thaw_json_host_set_property"
            })
                .map_err(|error| error.to_string())?;
            Ok((object, key, decode, set))
        })?;
        let value: Value = decode.call((graph_json.as_str(),)).map_err(|error| match error {
            rquickjs::Error::Exception => describe_tagged_exception(&ctx),
            error => error.to_string(),
        })?;
        let written: bool = set.call((object, key, value)).map_err(|error| match error {
            rquickjs::Error::Exception => describe_tagged_exception(&ctx),
            error => error.to_string(),
        })?;
        if !written { return Err("\u{1}TypeError\u{1}Cannot assign to read only property".into()); }
        Ok(1)
    });
    match result {
        Ok(value) => ThawHandleResult { value, error: std::ptr::null() },
        Err(error) => ThawHandleResult {
            value: 0, error: thaw_arena::owned_string(error),
        },
    }
}

// This is also installed in thaw-std's HostOperations table. Keep its three-
// argument C ABI stable; the receiver-data operation has a separate entry.
#[no_mangle]
pub extern "C" fn thaw_js_set_property_graph_result(
    handle: u64, key_json: *const c_char, graph_json: *const c_char,
) -> ThawHandleResult {
    thaw_js_set_property_graph_impl(handle, key_json, graph_json, 0)
}

/// N-API-only graph assignment with a retained thrown JS value. This is
/// intentionally distinct from the three-argument HostOperations setter ABI.
fn property_write_with_exception_result(
    handle: u64, key_json: Option<String>, key_handle: u64,
    graph_json: String, receiver_data: bool,
) -> ThawCallWithExceptionResult {
    enum Outcome { Written, Exception(u64, bool) }
    let result: Result<Outcome, String> = with_active_or_context(|ctx| {
        let (object, key, decode, set) = before_graph_decode(&ctx, &graph_json, true, || {
            let object = object_for_handle(&ctx, handle)?;
            let key: Value = if let Some(key_json) = key_json.as_ref() {
                let parse: Function = ctx.globals().get("__thaw_json_host_parse_property_key")
                    .map_err(|error| error.to_string())?;
                parse.call((key_json.as_str(),)).map_err(|error| error.to_string())?
            } else {
                value_for_handle(&ctx, key_handle)?
            };
            let decode: Function = ctx.globals().get("__thaw_json_graph_decode_owned")
                .map_err(|error| error.to_string())?;
            let set: Function = ctx.globals().get(if receiver_data {
                "__thaw_json_host_set_receiver_data"
            } else {
                "__thaw_json_host_set_property"
            }).map_err(|error| error.to_string())?;
            Ok((object, key, decode, set))
        })?;
        let value: Value = match decode.call((graph_json.as_str(),)) {
            Ok(value) => value,
            Err(rquickjs::Error::Exception) => {
                let thrown = ctx.catch();
                let object_like = thrown.is_object() || thrown.is_function();
                return retain_value(&ctx, thrown)
                    .map(|handle| Outcome::Exception(handle, object_like));
            }
            Err(error) => return Err(error.to_string()),
        };
        match set.call::<_, bool>((object, key, value)) {
            Ok(true) => Ok(Outcome::Written),
            Ok(false) => Err("\u{1}TypeError\u{1}Cannot assign to read only property".into()),
            Err(rquickjs::Error::Exception) => {
                let thrown = ctx.catch();
                let object_like = thrown.is_object() || thrown.is_function();
                retain_value(&ctx, thrown)
                    .map(|handle| Outcome::Exception(handle, object_like))
            }
            Err(error) => Err(error.to_string()),
        }
    });
    match result {
        Ok(Outcome::Written) => ThawCallWithExceptionResult {
            value: 1, exception_handle: 0, exception_object_like: 0,
            error: std::ptr::null(),
        },
        Ok(Outcome::Exception(exception_handle, object_like)) => ThawCallWithExceptionResult {
            value: 0, exception_handle, exception_object_like: u8::from(object_like),
            error: std::ptr::null(),
        },
        Err(error) => ThawCallWithExceptionResult {
            value: 0, exception_handle: 0, exception_object_like: 0,
            error: thaw_arena::owned_string(error),
        },
    }
}

#[no_mangle]
pub extern "C" fn thaw_js_set_property_graph_with_exception_result(
    handle: u64, key_json: *const c_char, graph_json: *const c_char,
) -> ThawCallWithExceptionResult {
    property_write_with_exception_result(handle, Some(to_str(key_json)), 0,
        to_str(graph_json), false)
}

#[no_mangle]
pub extern "C" fn thaw_js_set_receiver_data_graph_with_exception_result(
    handle: u64, key_json: *const c_char, graph_json: *const c_char,
) -> ThawCallWithExceptionResult {
    property_write_with_exception_result(handle, Some(to_str(key_json)), 0,
        to_str(graph_json), true)
}

#[no_mangle]
pub extern "C" fn thaw_js_set_property_key_handle_graph_with_exception_result(
    handle: u64, key_handle: u64, graph_json: *const c_char, receiver_data: u8,
) -> ThawCallWithExceptionResult {
    property_write_with_exception_result(handle, None, key_handle,
        to_str(graph_json), receiver_data != 0)
}

#[no_mangle]
pub extern "C" fn thaw_js_host_enumerate_result(handle: u64, operation: u8) -> ThawResult {
    let result: Result<String, String> = with_active_or_context(|ctx| {
        let value = value_for_handle(&ctx, handle)?;
        let enumerate: Function = ctx.globals().get("__thaw_json_host_enumerate")
            .map_err(|error| error.to_string())?;
        enumerate.call((value, operation)).map_err(|error| match error {
            rquickjs::Error::Exception => describe_tagged_exception(&ctx),
            error => error.to_string(),
        })
    });
    match result {
        // The graph names leases the JS encoder retained; std only decodes a
        // lease-bearing wire that was registered as a grant.
        Ok(value) => {
            let wire = owned_graph_wire(&value);
            if wire.is_null() {
                ThawResult { value: std::ptr::null(),
                    error: thaw_arena::owned_string("Unable to register host enumeration graph") }
            } else {
                ThawResult { value: wire, error: std::ptr::null() }
            }
        }
        Err(error) => ThawResult {
            value: std::ptr::null(),
            error: thaw_arena::owned_string(error),
        },
    }
}

#[no_mangle]
pub extern "C" fn thaw_js_host_property_names_result(
    handle: u64, key_mode: i32, key_filter: u32, key_conversion: i32,
) -> ThawResult {
    let result: Result<String, String> = with_active_or_context(|ctx| {
        let value = value_for_handle(&ctx, handle)?;
        let enumerate: Function = ctx.globals().get("__thaw_json_host_property_names")
            .map_err(|error| error.to_string())?;
        enumerate.call((value, key_mode, key_filter, key_conversion)).map_err(|error| match error {
            rquickjs::Error::Exception => describe_tagged_exception(&ctx),
            error => error.to_string(),
        })
    });
    match result {
        Ok(value) => ThawResult { value: thaw_arena::owned_string(value), error: std::ptr::null() },
        Err(error) => ThawResult { value: std::ptr::null(), error: thaw_arena::owned_string(error) },
    }
}

#[no_mangle]
pub extern "C" fn thaw_js_get_prototype_handle_result(handle: u64) -> ThawHandleResult {
    let result: Result<u64, String> = with_active_or_context(|ctx| {
        let value = value_for_handle(&ctx, handle)?;
        let method: Function = ctx.globals().get("__thaw_json_host_get_prototype")
            .map_err(|error| error.to_string())?;
        let prototype: Value = method.call((value,)).map_err(|error| match error {
            rquickjs::Error::Exception => describe_tagged_exception(&ctx),
            error => error.to_string(),
        })?;
        retain_value(&ctx, prototype)
    });
    match result {
        Ok(value) => ThawHandleResult { value, error: std::ptr::null() },
        Err(error) => ThawHandleResult { value: 0, error: thaw_arena::owned_string(error) },
    }
}

#[no_mangle]
pub extern "C" fn thaw_js_string_from_utf16_units_result(units_json: *const c_char) -> ThawHandleResult {
    let units_json = to_str(units_json);
    let result: Result<u64, String> = with_active_or_context(|ctx| {
        let make: Function = ctx.globals().get("__thaw_json_host_utf16_string")
            .map_err(|error| error.to_string())?;
        let value: Value = make.call((units_json.as_str(),)).map_err(|error| match error {
            rquickjs::Error::Exception => describe_tagged_exception(&ctx),
            error => error.to_string(),
        })?;
        retain_value(&ctx, value)
    });
    match result {
        Ok(value) => ThawHandleResult { value, error: std::ptr::null() },
        Err(error) => ThawHandleResult { value: 0, error: thaw_arena::owned_string(error) },
    }
}

#[no_mangle]
pub extern "C" fn thaw_js_native_symbol_handle_result(
    native_handle: u64,
) -> ThawHandleResult {
    let result: Result<u64, String> = with_active_or_context(|ctx| {
        let make: Function = ctx.globals().get("__thaw_json_host_native_symbol")
            .map_err(|error| error.to_string())?;
        let value: Value = make.call((native_handle.to_string(),))
            .map_err(|error| match error {
                rquickjs::Error::Exception => describe_tagged_exception(&ctx),
                error => error.to_string(),
            })?;
        // Native-created Symbols need an independent native root while a JS
        // Symbol identity is live. A JS-origin Symbol already has a weak
        // per-owner identity and would form a native-ref/JS-pin cycle here.
        let needs_pin: Function = ctx.globals()
            .get("__thaw_json_host_native_symbol_needs_pin")
            .map_err(|error| error.to_string())?;
        let needs_pin: bool = needs_pin.call((native_handle.to_string(),))
            .map_err(|error| match error {
                rquickjs::Error::Exception => describe_tagged_exception(&ctx),
                error => error.to_string(),
            })?;
        if needs_pin {
            pin_native_graph_symbol(ctx.clone(), native_handle.to_string())
                .map_err(|error| match error {
                    rquickjs::Error::Exception => describe_tagged_exception(&ctx),
                    error => error.to_string(),
                })?;
        }
        retain_value(&ctx, value)
    });
    match result {
        Ok(value) => ThawHandleResult { value, error: std::ptr::null() },
        Err(error) => ThawHandleResult { value: 0, error: thaw_arena::owned_string(error) },
    }
}

#[no_mangle]
pub extern "C" fn thaw_js_set_prototype_graph_result(
    handle: u64, prototype_graph: *const c_char,
) -> ThawHandleResult {
    let prototype_graph = to_str(prototype_graph);
    let result: Result<u64, String> = with_active_or_context(|ctx| {
        let object = before_graph_decode(&ctx, &prototype_graph, true,
            || value_for_handle(&ctx, handle))?;
        let arguments = decode_argument_array(&ctx, &prototype_graph, true)?;
        let prototype: Value = arguments.get(0).map_err(|error| error.to_string())?;
        let method: Function = ctx.globals().get("__thaw_json_host_set_prototype")
            .map_err(|error| error.to_string())?;
        let changed: bool = method.call((object, prototype)).map_err(|error| match error {
            rquickjs::Error::Exception => describe_tagged_exception(&ctx),
            error => error.to_string(),
        })?;
        Ok(u64::from(changed))
    });
    match result {
        Ok(value) => ThawHandleResult { value, error: std::ptr::null() },
        Err(error) => ThawHandleResult { value: 0, error: thaw_arena::owned_string(error) },
    }
}

#[no_mangle]
pub extern "C" fn thaw_js_instanceof_graph_result(arguments_graph: *const c_char) -> ThawHandleResult {
    let arguments_graph = to_str(arguments_graph);
    let result: Result<u64, String> = with_active_or_context(|ctx| {
        let arguments = decode_argument_array(&ctx, &arguments_graph, true)?;
        let object: Value = arguments.get(0).map_err(|error| error.to_string())?;
        let constructor: Value = arguments.get(1).map_err(|error| error.to_string())?;
        let method: Function = ctx.globals().get("__thaw_json_host_instanceof")
            .map_err(|error| error.to_string())?;
        let answer: bool = method.call((object, constructor)).map_err(|error| match error {
            rquickjs::Error::Exception => describe_tagged_exception(&ctx),
            error => error.to_string(),
        })?;
        Ok(u64::from(answer))
    });
    match result {
        Ok(value) => ThawHandleResult { value, error: std::ptr::null() },
        Err(error) => ThawHandleResult { value: 0, error: thaw_arena::owned_string(error) },
    }
}

#[no_mangle]
pub extern "C" fn thaw_js_instanceof_handles_result(
    object_handle: u64, constructor_handle: u64,
) -> ThawHandleResult {
    let result: Result<u64, String> = with_active_or_context(|ctx| {
        let object = value_for_handle(&ctx, object_handle)?;
        let constructor = value_for_handle(&ctx, constructor_handle)?;
        let method: Function = ctx.globals().get("__thaw_json_host_instanceof")
            .map_err(|error| error.to_string())?;
        let answer: bool = method.call((object, constructor)).map_err(|error| match error {
            rquickjs::Error::Exception => describe_tagged_exception(&ctx),
            error => error.to_string(),
        })?;
        Ok(u64::from(answer))
    });
    match result {
        Ok(value) => ThawHandleResult { value, error: std::ptr::null() },
        Err(error) => ThawHandleResult { value: 0, error: thaw_arena::owned_string(error) },
    }
}

#[no_mangle]
pub extern "C" fn thaw_js_retain_json_result(value_json: *const c_char) -> ThawHandleResult {
    let value_json = to_str(value_json);
    let result = with_active_or_context(|ctx| -> Result<u64, String> {
        let json: Object = ctx.globals().get("JSON").map_err(|error| error.to_string())?;
        let parse: Function = json.get("parse").map_err(|error| error.to_string())?;
        let reviver: Function = ctx
            .globals()
            .get("__thaw_json_date_reviver")
            .map_err(|error| error.to_string())?;
        let value: Value = parse
            .call((value_json.as_str(), reviver))
            .map_err(|error| error.to_string())?;
        retain_value(&ctx, value)
    });
    match result {
        Ok(value) => ThawHandleResult {
            value,
            error: std::ptr::null(),
        },
        Err(error) => ThawHandleResult {
            value: 0,
            error: thaw_arena::owned_string(error),
        },
    }
}

#[no_mangle]
pub extern "C" fn thaw_js_set_property_result(
    handle: u64,
    name: *const c_char,
    value_handle: u64,
) -> ThawHandleResult {
    let name = to_str(name);
    let result: Result<u64, String> = with_active_or_context(|ctx| {
        let object = object_for_handle(&ctx, handle)?;
        let value = value_for_handle(&ctx, value_handle)?;
        // See `invoke_raw`'s doc comment -- a setter can throw too.
        let key = dynamic_property_key(&ctx, &name)?;
        let key = rquickjs::Atom::from_value(ctx.clone(), &key)
            .map_err(|error| error.to_string())?;
        object
            .set(key, value)
            .map_err(|error| match error {
                rquickjs::Error::Exception => describe_tagged_exception(&ctx),
                error => error.to_string(),
            })?;
        Ok(1)
    });
    match result {
        Ok(value) => ThawHandleResult {
            value,
            error: std::ptr::null(),
        },
        Err(error) => ThawHandleResult {
            value: 0,
            error: thaw_arena::owned_string(error),
        },
    }
}

fn set_property_json_result_impl(
    handle: u64,
    name: *const c_char,
    args_json: *const c_char,
    graph_result: bool,
) -> ThawResult {
    let name = to_str(name);
    let args_json = to_str(args_json);
    let result = with_active_or_context(|ctx| -> Result<String, String> {
        let object = before_graph_decode(&ctx, &args_json, graph_result, || {
            object_for_handle(&ctx, handle)
        })?;
        let args = decode_argument_array(&ctx, &args_json, graph_result)?;
        let value: Value = args.get(0).map_err(|error| error.to_string())?;
        // Resolve and write the reference before encoding the assignment
        // expression result. A throwing setter must not leave a graph wire
        // with transferred Host/NAPI leases unread.
        let key = dynamic_property_key(&ctx, &name)?;
        let key = rquickjs::Atom::from_value(ctx.clone(), &key)
            .map_err(|error| error.to_string())?;
        object.set(key, value.clone()).map_err(|error| match error {
            rquickjs::Error::Exception => describe_tagged_exception(&ctx),
            error => error.to_string(),
        })?;
        // Same substitution as `resolve_value_impl`'s own, and for the
        // same reason: a genuinely `undefined` assigned value (e.g. one
        // just revived from the `$__thaw_napi_undefined$` sentinel by
        // `__thaw_json_date_reviver` when parsing `args_json` above)
        // can't be `JSON.stringify`d at all -- reporting it back as
        // plain `null` would make `obj.prop = someOptionalNone` (an
        // assignment expression, which evaluates to the assigned value)
        // indistinguishable from assigning a real `null`.
        let returned = if graph_result {
            let encode: Function = ctx.globals().get("__thaw_json_graph_encode_js")
                .map_err(|error| error.to_string())?;
            encode.call((value.clone(), 0, false, true))
                .map_err(|error| error.to_string())?
        } else {
            ctx.json_stringify(value.clone())
                .map_err(|error| error.to_string())?
                .map(|value| value.to_string().unwrap_or_default())
                .unwrap_or_else(|| r#"{"$__thaw_napi_undefined$":true}"#.to_string())
        };
        Ok(returned)
    });
    match result {
        Ok(value) => ThawResult {
            value: thaw_arena::owned_string(value),
            error: std::ptr::null(),
        },
        Err(error) => ThawResult {
            value: std::ptr::null(),
            error: thaw_arena::owned_string(error),
        },
    }
}

#[no_mangle]
pub extern "C" fn thaw_js_set_property_json_result(
    handle: u64, name: *const c_char, args_json: *const c_char,
) -> ThawResult {
    set_property_json_result_impl(handle, name, args_json, false)
}

#[no_mangle]
pub extern "C" fn thaw_js_set_property_json_graph_result(
    handle: u64, name: *const c_char, args_json: *const c_char,
) -> ThawResult {
    set_property_json_result_impl(handle, name, args_json, true)
}

#[no_mangle]
pub extern "C" fn thaw_js_delete_property_result(
    handle: u64,
    name: *const c_char,
) -> ThawHandleResult {
    dynamic_property_predicate(handle, name, true)
}

#[no_mangle]
pub extern "C" fn thaw_js_has_property_result(
    handle: u64,
    name: *const c_char,
) -> ThawHandleResult {
    dynamic_property_predicate(handle, name, false)
}

#[no_mangle]
pub extern "C" fn thaw_js_host_object_integrity_result(
    handle: u64, freeze: u8,
) -> ThawHandleResult {
    let result: Result<u64, String> = with_active_or_context(|ctx| {
        let object = object_for_handle(&ctx, handle)?;
        let integrity: Function = ctx.globals().get("__thaw_json_host_object_integrity")
            .map_err(|error| error.to_string())?;
        let done: bool = integrity.call((object, freeze != 0)).map_err(|error| match error {
            rquickjs::Error::Exception => describe_tagged_exception(&ctx),
            error => error.to_string(),
        })?;
        Ok(u64::from(done))
    });
    match result {
        Ok(value) => ThawHandleResult { value, error: std::ptr::null() },
        Err(error) => ThawHandleResult { value: 0, error: thaw_arena::owned_string(error) },
    }
}

fn dynamic_property_predicate(
    handle: u64,
    name: *const c_char,
    delete: bool,
) -> ThawHandleResult {
    let name = to_str(name);
    let result: Result<u64, String> = with_active_or_context(|ctx| {
        let object = object_for_handle(&ctx, handle)?;
        let key = dynamic_property_key(&ctx, &name)?;
        let reflect: Object = ctx
            .globals()
            .get("Reflect")
            .map_err(|error| match error {
                rquickjs::Error::Exception => describe_tagged_exception(&ctx),
                error => error.to_string(),
            })?;
        let operation: Function = reflect
            .get(if delete { "deleteProperty" } else { "has" })
            .map_err(|error| match error {
                rquickjs::Error::Exception => describe_tagged_exception(&ctx),
                error => error.to_string(),
            })?;
        operation
            .call::<_, bool>((object, key))
            .map(u64::from)
            .map_err(|error| match error {
                rquickjs::Error::Exception => describe_tagged_exception(&ctx),
                error => error.to_string(),
            })
    });
    match result {
        Ok(value) => ThawHandleResult {
            value,
            error: std::ptr::null(),
        },
        Err(error) => ThawHandleResult {
            value: 0,
            error: thaw_arena::owned_string(error),
        },
    }
}

fn dynamic_property_key<'js>(ctx: &Ctx<'js>, name: &str) -> Result<Value<'js>, String> {
    ctx.globals()
        .get::<_, Function>("__thaw_property_key")
        .and_then(|resolve| resolve.call((name,)))
        .map_err(|error| error.to_string())
}

// A member call captures its property before arguments and invokes it later.
fn invoke_selected_method<'js>(
    ctx: &Ctx<'js>, receiver: u64, selected: u64, args_json: &str,
) -> Result<Value<'js>, String> {
    let arguments = decode_argument_array(ctx, args_json, true)?;
    // The graph decoder owns transferred leases even if the receiver has
    // since become invalid. Keep Call(this) on the original value.
    let this_value = value_for_handle(ctx, receiver)?;
    // Callability is checked after arguments, but no property is re-read.
    let method = Function::from_value(value_for_handle(ctx, selected)?)
        .map_err(|_| "\u{1}TypeError\u{1}Selected property is not callable".to_string())?;
    let mut call_args = Args::new_unsized(ctx.clone());
    call_args.this(this_value).map_err(|error| error.to_string())?;
    for index in 0..arguments.len() {
        let argument: Value = arguments.get(index).map_err(|error| error.to_string())?;
        call_args.push_arg(argument).map_err(|error| error.to_string())?;
    }
    method.call_arg(call_args).map_err(|error| match error {
        rquickjs::Error::Exception => describe_tagged_exception(ctx),
        error => error.to_string(),
    })
}

#[no_mangle]
pub extern "C" fn thaw_js_call_selected_method_graph_result(
    receiver: u64, selected: u64, name: *const c_char, args_json: *const c_char,
) -> ThawResult {
    let name = to_str(name);
    let args_json = to_str(args_json);
    let result = with_active_or_context(|ctx| {
        let value = invoke_selected_method(&ctx, receiver, selected, &args_json)?;
        resolve_value_wire_impl(ctx, value, &format!("JavaScript method `{name}`"), true, true)
    });
    match result {
        Ok(value) => ThawResult { value: thaw_arena::owned_string(value), error: std::ptr::null() },
        Err(error) => ThawResult { value: std::ptr::null(), error: thaw_arena::owned_string(error) },
    }
}

#[no_mangle]
pub extern "C" fn thaw_js_call_selected_method_handle_graph_args_result(
    receiver: u64, selected: u64, name: *const c_char, args_json: *const c_char, defer_resolution: bool,
) -> ThawHandleResult {
    let name = to_str(name);
    let args_json = to_str(args_json);
    let result: Result<u64, String> = with_active_or_context(|ctx| {
        let value = invoke_selected_method(&ctx, receiver, selected, &args_json)?;
        let value = if defer_resolution { value } else {
            resolve_promise_value(&ctx, value, &format!("JavaScript method `{name}`"), true)?
        };
        retain_value(&ctx, value)
    });
    match result {
        Ok(value) => ThawHandleResult { value, error: std::ptr::null() },
        Err(error) => ThawHandleResult { value: 0, error: thaw_arena::owned_string(error) },
    }
}

fn invoke_method<'js>(
    ctx: &Ctx<'js>,
    handle: u64,
    name: &str,
    args_json: &str,
    graph_args: bool,
) -> Result<Value<'js>, String> {
    // `boxed_object_for_handle`, not `object_for_handle`: a method call on a
    // primitive receiver (`"abc".trim()`, a `JsValue` whose runtime value is
    // a string) needs the same auto-boxing `value.method()` gets in JS.
    let (object, method) = before_graph_decode(ctx, args_json, graph_args, || {
        let object = boxed_object_for_handle(ctx, handle)?;
        // A property getter backing method lookup can throw before the
        // transferred graph is decoded.
        let method: Function = object.get(name).map_err(|error| match error {
            rquickjs::Error::Exception => describe_tagged_exception(ctx),
            error => error.to_string(),
        })?;
        Ok((object, method))
    })?;
    let arguments = decode_argument_array(ctx, args_json, graph_args)?;
    let mut call_args = Args::new_unsized(ctx.clone());
    call_args.this(object).map_err(|error| error.to_string())?;
    for index in 0..arguments.len() {
        let argument: Value = arguments.get(index).map_err(|error| error.to_string())?;
        call_args
            .push_arg(argument)
            .map_err(|error| error.to_string())?;
    }
    method.call_arg(call_args).map_err(|error| match error {
        rquickjs::Error::Exception => describe_tagged_exception(ctx),
        error => error.to_string(),
    })
}

fn thaw_js_call_method_result_impl(
    handle: u64,
    name: *const c_char,
    args_json: *const c_char,
    graph_result: bool,
) -> ThawResult {
    let name = to_str(name);
    let args_json = to_str(args_json);
    let result = with_active_or_context(|ctx| {
        let value = invoke_method(&ctx, handle, &name, &args_json, graph_result)?;
        resolve_value_wire_impl(ctx, value, &format!("JavaScript method `{name}`"), true, graph_result)
    });
    match result {
        Ok(value) => ThawResult {
            value: thaw_arena::owned_string(value),
            error: std::ptr::null(),
        },
        Err(error) => ThawResult {
            value: std::ptr::null(),
            error: thaw_arena::owned_string(error),
        },
    }
}

/// Like [`thaw_js_call_method_result`], but for a method whose own result
/// is itself a `JsValue` -- a live JS value with no JSON representation --
/// rather than plain data: retains the result as a handle instead of
/// JSON-encoding it. Real example: a schema instance's own chained
/// method returning another schema instance, as opposed to
/// `.safeParse(...)`'s plain data result (which stays on the
/// `thaw_js_call_method_result` path).
/// `chain_intermediate` is true when this call's own result is itself
/// the receiver of another chained `.method()` call (e.g. `.from(...)`
/// in `db.select().from(x).where(y)`) -- in that case a pending
/// thenable (a lazy query-builder object, not a genuine async result)
/// must NOT be resolved yet, since `Promise.resolve()` on any thenable
/// eagerly invokes its `.then`, which would execute the chain
/// prematurely. Only the terminal link in a chain (`chain_intermediate
/// == false`) resolves its result, matching real JS semantics where
/// only an explicit `await`/consumption of the final value would ever
/// settle a pending promise or thenable.
#[no_mangle]
pub extern "C" fn thaw_js_call_method_handle_result(
    handle: u64, name: *const c_char, args_json: *const c_char, chain_intermediate: bool,
) -> ThawHandleResult {
    thaw_js_call_method_handle_impl(handle, name, args_json, chain_intermediate, false)
}

#[no_mangle]
pub extern "C" fn thaw_js_call_method_handle_graph_args_result(
    handle: u64, name: *const c_char, args_json: *const c_char, chain_intermediate: bool,
) -> ThawHandleResult {
    thaw_js_call_method_handle_impl(handle, name, args_json, chain_intermediate, true)
}

fn thaw_js_call_method_handle_impl(
    handle: u64, name: *const c_char, args_json: *const c_char, chain_intermediate: bool,
    graph_args: bool,
) -> ThawHandleResult {
    let name = to_str(name);
    let args_json = to_str(args_json);
    let result: Result<u64, String> = with_active_or_context(|ctx| {
        let value = invoke_method(&ctx, handle, &name, &args_json, graph_args)?;
        let value = if chain_intermediate {
            value
        } else {
            resolve_promise_value(
                &ctx,
                value,
                &format!("JavaScript method `{name}`"),
                true,
            )?
        };
        retain_value(&ctx, value)
    });
    match result {
        Ok(value) => ThawHandleResult {
            value,
            error: std::ptr::null(),
        },
        Err(error) => ThawHandleResult {
            value: 0,
            error: thaw_arena::owned_string(error),
        },
    }
}

fn thaw_js_resolve_handle_result_impl(handle: u64, graph_result: bool) -> ThawResult {
    let result = with_active_or_context(|ctx| {
        let value = value_for_handle(&ctx, handle)?;
        resolve_value_wire_impl(ctx, value, &format!("JavaScript value #{handle}"), true, graph_result)
    });
    match result {
        Ok(value) => ThawResult {
            value: thaw_arena::owned_string(value),
            error: std::ptr::null(),
        },
        Err(error) => ThawResult {
            value: std::ptr::null(),
            error: thaw_arena::owned_string(error),
        },
    }
}

#[no_mangle]
pub extern "C" fn thaw_js_resolve_handle_handle_result(handle: u64) -> ThawHandleResult {
    let result = with_active_or_context(|ctx| {
        let value = value_for_handle(&ctx, handle)?;
        let value = resolve_promise_value(
            &ctx,
            value,
            &format!("JavaScript value #{handle}"),
            true,
        )?;
        retain_value(&ctx, value)
    });
    match result {
        Ok(value) => ThawHandleResult {
            value,
            error: std::ptr::null(),
        },
        Err(error) => ThawHandleResult {
            value: 0,
            error: thaw_arena::owned_string(error),
        },
    }
}

/// Calls a callable value retained by [`thaw_js_get_global`]. Arguments and
/// results use the existing JSON bridge while the callable itself preserves
/// identity and closures inside QuickJS.
fn call_handle_result_impl(handle: u64, args_json: *const c_char, graph_result: bool) -> ThawResult {
    let args_json = to_str(args_json);
    let result = with_active_or_context(|ctx| {
        let target = before_graph_decode(&ctx, &args_json, graph_result, || {
            Function::from_value(value_for_handle(&ctx, handle)?)
                .map_err(|_| format!("JavaScript value handle {handle} is not callable"))
        })?;
        invoke_impl(
            ctx,
            target,
            &format!("JavaScript value #{handle}"),
            &args_json,
            true,
            graph_result,
        )
    });
    match result {
        Ok(text) => ThawResult {
            value: thaw_arena::owned_string(text),
            error: std::ptr::null(),
        },
        Err(reason) => ThawResult {
            value: std::ptr::null(),
            error: thaw_arena::owned_string(reason),
        },
    }
}

#[no_mangle]
pub extern "C" fn thaw_js_call_handle_result(handle: u64, args_json: *const c_char) -> ThawResult {
    call_handle_result_impl(handle, args_json, false)
}

#[no_mangle]
pub extern "C" fn thaw_js_call_handle_graph_result(handle: u64, args_json: *const c_char) -> ThawResult {
    call_handle_result_impl(handle, args_json, true)
}

/// Calls the original retained function with a graph-transported receiver.
/// This uses Args::this, not a mutable `fn.call` property.
#[no_mangle]
pub extern "C" fn thaw_js_call_handle_with_this_graph_wire_result(
    handle: u64, args_json: *const c_char,
) -> ThawResult {
    let args_json = to_str(args_json);
    let result = with_active_or_context(|ctx| {
        let target = before_graph_decode(&ctx, &args_json, true, || {
            Function::from_value(value_for_handle(&ctx, handle)?)
                .map_err(|_| format!("JavaScript value handle {handle} is not callable"))
        })?;
        let args_array = decode_argument_array(&ctx, &args_json, true)?;
        if args_array.is_empty() {
            return Err("JS callback receiver payload is empty".into());
        }
        let mut call_args = Args::new_unsized(ctx.clone());
        let receiver: Value = args_array.get(0).map_err(|error| error.to_string())?;
        call_args.this(receiver).map_err(|error| error.to_string())?;
        for index in 1..args_array.len() {
            let arg: Value = args_array.get(index).map_err(|error| error.to_string())?;
            call_args.push_arg(arg).map_err(|error| error.to_string())?;
        }
        let result: Value = target.call_arg(call_args).map_err(|error| match error {
            rquickjs::Error::Exception => describe_tagged_exception(&ctx),
            error => error.to_string(),
        })?;
        resolve_value_graph_impl(ctx, result,
            &format!("JavaScript value #{handle}"), true)
    });
    match result {
        Ok(text) => ThawResult {
            value: thaw_arena::owned_string(text), error: std::ptr::null(),
        },
        Err(reason) => ThawResult {
            value: std::ptr::null(), error: thaw_arena::owned_string(reason),
        },
    }
}

/// `rquickjs::Error::Exception` doesn't carry the thrown value itself
/// (just a generic placeholder message) -- the real value has to be
/// fetched separately via `Ctx::catch`.
fn describe_exception(ctx: &Ctx<'_>) -> String {
    let exc = ctx.catch();
    if let Some(obj) = exc.as_object() {
        if let Ok(msg) = obj.get::<_, String>("message") {
            return msg;
        }
    }
    if let Some(s) = exc.as_string() {
        if let Ok(s) = s.to_string() {
            return s;
        }
    }
    format!("{exc:?}")
}

fn describe_tagged_exception(ctx: &Ctx<'_>) -> String {
    let exc = ctx.catch();
    let properties = error_property_json(ctx, &exc);
    let mut body = if let Some(obj) = exc.as_object() {
        if let Ok(message) = obj.get::<_, String>("message") {
            match obj.get::<_, String>("name") {
                Ok(name) => framed_error(&name, &message, None),
                _ => framed_error("Error", &message, None),
            }
        } else if let Some(value) = exc.as_string() {
            value.to_string().unwrap_or_default()
        } else {
            format!("{exc:?}")
        }
    } else if let Some(value) = exc.as_string() {
        value.to_string().unwrap_or_default()
    } else {
        format!("{exc:?}")
    };
    if let Some(properties) = properties {
        body.push(ERROR_PROPERTIES_MARKER);
        body.push_str(&properties);
    }
    body
}

/// Marker separating an exception's message from its own extra properties
/// (a JSON object), appended by `describe_tagged_exception`. `\u{5}` is
/// unused by `thaw-runtime`'s own tag/cause/code/name-override markers
/// (`\u{1}`..`\u{4}`). Real trigger: an `http-errors` HTTP error's
/// `status`/`statusCode`/`expose`/`headers`, which koa's own error handler
/// reads after the error has crossed native code.
const ERROR_PROPERTIES_MARKER: char = '\u{5}';

/// The caught value's own extra properties -- everything but `name`,
/// `message`, and `stack` -- as a JSON object, so they survive the trip
/// out to native code and back (`__thaw_error_from_tagged`, `runtime.js`).
/// Uses the realm's own `JSON.stringify`, so a nested value (a `headers`
/// object) serializes exactly as it would in JS; a non-serializable or
/// empty property set degrades to `None`.
fn error_property_json<'js>(ctx: &Ctx<'js>, exc: &Value<'js>) -> Option<String> {
    let helper: Function = ctx.globals().get("__thaw_error_properties_json").ok()?;
    let encoded: String = helper.call((exc.clone(),)).ok()?;
    let value: serde_json::Value = serde_json::from_str(&encoded).ok()?;
    if value.as_object()?.is_empty() {
        return None;
    }
    Some(encoded)
}

fn describe_host_exception(ctx: &Ctx<'_>, label: &str) -> String {
    let exc = ctx.catch();
    let properties = error_property_json(ctx, &exc);
    let mut body = if let Some(obj) = exc.as_object() {
        if let Ok(message) = obj.get::<_, String>("message") {
            let display = format!("`{label}` threw: {message}");
            // Preserve the established message → code → name getter order.
            let code = obj.get::<_, String>("code").ok();
            let mut body = match obj.get::<_, String>("name") {
                Ok(name) => framed_error(&name, &display, None),
                _ => framed_error("Error", &display, None),
            };
            if let Some(code) = code {
                body.push('\u{3}');
                body.push_str(&code);
            }
            body
        } else if let Some(value) = exc.as_string() {
            format!("`{label}` threw: {}", value.to_string().unwrap_or_default())
        } else {
            format!("`{label}` threw: {exc:?}")
        }
    } else if let Some(value) = exc.as_string() {
        format!("`{label}` threw: {}", value.to_string().unwrap_or_default())
    } else {
        format!("`{label}` threw: {exc:?}")
    };
    if let Some(properties) = properties {
        body.push(ERROR_PROPERTIES_MARKER);
        body.push_str(&properties);
    }
    body
}

fn describe_promise_exception<'js>(ctx: &Ctx<'js>, label: &str, preserve_error: bool) -> String {
    let exc = ctx.catch();
    let properties = error_property_json(ctx, &exc);
    let mut body = if let Some(obj) = exc.as_object() {
        if let Ok(message) = obj.get::<_, String>("message") {
            let name = obj.get::<_, String>("name").ok();
            if preserve_error {
                framed_error(name.as_deref().unwrap_or("Error"), &message, None)
            } else {
                let display = format!("`{label}`'s promise rejected: {message}");
                match name.as_deref() {
                    Some(name) if name != "Error" => framed_error(name, &display, None),
                    _ => framed_error("Error", &display, Some(&message)),
                }
            }
        } else if let Some(value) = exc.as_string() {
            let value = value.to_string().unwrap_or_default();
            if preserve_error {
                value
            } else {
                format!("`{label}`'s promise rejected: {value}")
            }
        } else {
            format!("`{label}`'s promise rejected: {exc:?}")
        }
    } else if let Some(value) = exc.as_string() {
        let value = value.to_string().unwrap_or_default();
        if preserve_error {
            value
        } else {
            format!("`{label}`'s promise rejected: {value}")
        }
    } else {
        format!("`{label}`'s promise rejected: {exc:?}")
    };
    if let Some(properties) = properties {
        body.push(ERROR_PROPERTIES_MARKER);
        body.push_str(&properties);
    }
    body
}

fn describe_exception_with_stack(ctx: &Ctx<'_>) -> String {
    let exc = ctx.catch();
    if let Some(obj) = exc.as_object() {
        if let Ok(msg) = obj.get::<_, String>("message") {
            if let Ok(stack) = obj.get::<_, String>("stack") {
                if !stack.is_empty() && !stack.ends_with(&msg) {
                    return format!("{msg}\n{stack}");
                }
            }
            return msg;
        }
    }
    if let Some(value) = exc.as_string() {
        if let Ok(value) = value.to_string() {
            return value;
        }
    }
    format!("{exc:?}")
}

fn json_escape_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

#[no_mangle]
pub unsafe extern "C" fn thaw_js_call_handle_mixed_result(handle: u64, args_json: *const c_char, handles: *const u8) -> ThawResult {
    unsafe { thaw_js_call_handle_mixed_result_impl(handle, args_json, handles, false) }
}

#[no_mangle]
pub unsafe extern "C" fn thaw_js_call_handle_mixed_graph_result(handle: u64, args_json: *const c_char, handles: *const u8) -> ThawResult {
    unsafe { thaw_js_call_handle_mixed_result_impl(handle, args_json, handles, true) }
}

/// A compiler-private mixed graph call with exact thrown-value transport.
/// The callable and every staged direct handle are independent temporary
/// references owned by this call, including when decoding or invocation
/// fails. A successful return or thrown value gets its own retained handle.
#[no_mangle]
pub unsafe extern "C" fn thaw_js_call_handle_mixed_exact_consuming_result(
    handle: u64, args_json: *const c_char, handles: *const u8,
) -> ThawCallWithExceptionResult {
    enum Outcome { Value(u64), Exception(u64, bool) }
    // Snapshot before user code can re-enter and mutate the native array.
    let acquired = unsafe { native_handle_slice(handles) }.map(|values| values.to_vec());
    if acquired.is_err() {
        unsafe extern "C" { fn thaw_json_discard_graph_wire(source: *const c_char); }
        // The exact ABI may fail before it reaches the graph decoder. The
        // registered one-shot grant owns any transferred graph leases here.
        unsafe { thaw_json_discard_graph_wire(args_json) };
    }
    let result: Result<Outcome, String> = match &acquired {
        Err(error) => Err(error.clone()),
        Ok(values) => {
            let args_json_ptr = args_json;
            let args_json = to_str(args_json);
            with_active_or_context(|ctx| {
                let target = before_registered_graph_decode(args_json_ptr, || {
                    Function::from_value(value_for_handle(&ctx, handle)?)
                        .map_err(|_| format!("JavaScript value handle {handle} is not callable"))
                })?;
                // The shared graph decoder owns the transferred leases once
                // invoked. Preserve an original JS throw from that decoder
                // through the same exact-result lane as a target-call throw.
                let decode: Function = before_registered_graph_decode(args_json_ptr, || {
                    ctx.globals().get("__thaw_json_graph_decode_owned")
                        .map_err(|error| error.to_string())
                })?;
                let arguments: Array = match decode.call((args_json.as_str(),)) {
                    Ok(arguments) => arguments,
                    Err(rquickjs::Error::Exception) => {
                        let thrown = ctx.catch();
                        let object_like = thrown.is_object() || thrown.is_function();
                        return retain_value(&ctx, thrown)
                            .map(|value| Outcome::Exception(value, object_like));
                    }
                    Err(error) => return Err(error.to_string()),
                };
                let mut call_args = Args::new_unsized(ctx.clone());
                for index in 0..arguments.len() {
                    let argument = match arguments.get::<Value>(index) {
                        Ok(argument) => argument,
                        Err(rquickjs::Error::Exception) => {
                            let thrown = ctx.catch();
                            let object_like = thrown.is_object() || thrown.is_function();
                            return retain_value(&ctx, thrown)
                                .map(|value| Outcome::Exception(value, object_like));
                        }
                        Err(error) => return Err(error.to_string()),
                    };
                    call_args.push_arg(argument)
                        .map_err(|error| error.to_string())?;
                }
                for direct in values {
                    call_args.push_arg(value_for_handle(&ctx, *direct)?)
                        .map_err(|error| error.to_string())?;
                }
                match target.call_arg(call_args) {
                    Ok(value) => retain_value(&ctx, value).map(Outcome::Value),
                    Err(rquickjs::Error::Exception) => {
                        let thrown = ctx.catch();
                        let object_like = thrown.is_object() || thrown.is_function();
                        retain_value(&ctx, thrown)
                            .map(|value| Outcome::Exception(value, object_like))
                    }
                    Err(error) => Err(error.to_string()),
                }
            })
        }
    };
    if let Ok(values) = acquired {
        for direct in values { let _ = thaw_js_release_handle(direct); }
    }
    let _ = thaw_js_release_handle(handle);
    match result {
        Ok(Outcome::Value(value)) => ThawCallWithExceptionResult {
            value, exception_handle: 0, exception_object_like: 0,
            error: std::ptr::null(),
        },
        Ok(Outcome::Exception(exception_handle, object_like)) => ThawCallWithExceptionResult {
            value: 0, exception_handle, exception_object_like: u8::from(object_like),
            error: std::ptr::null(),
        },
        Err(error) => ThawCallWithExceptionResult {
            value: 0, exception_handle: 0, exception_object_like: 0,
            error: thaw_arena::owned_string(error),
        },
    }
}

#[cfg(test)]
thread_local! {
    static EXACT_MIXED_NAPI_RELEASES: std::cell::Cell<u8> = const {
        std::cell::Cell::new(0)
    };
}

#[cfg(test)]
extern "C" fn exact_mixed_napi_retain(reference: u64) -> u64 { reference }

#[cfg(test)]
extern "C" fn exact_mixed_napi_release(reference: u64) -> u8 {
    if reference == 73 {
        EXACT_MIXED_NAPI_RELEASES.with(|releases| releases.set(releases.get() + 1));
        1
    } else { 0 }
}

#[cfg(test)]
#[test]
fn exact_mixed_pre_dispatch_consumes_registered_graph_grant_once() {
    std::thread::spawn(|| {
        unsafe extern "C" {
            fn thaw_json_register_napi_handle_operations(
                retain: extern "C" fn(u64) -> u64,
                release: extern "C" fn(u64) -> u8,
            );
            fn thaw_json_discard_graph_wire(source: *const c_char);
        }
        unsafe {
            thaw_json_register_napi_handle_operations(
                exact_mixed_napi_retain, exact_mixed_napi_release,
            );
        }
        let wire = std::ffi::CString::new(
            r#"{"root":{"v":null},"nodes":[],"leases":[],"napiLeases":["73"]}"#,
        ).unwrap();
        assert!(unsafe { thaw_arena::register_owned_graph_wire(wire.as_ptr()) });

        let result = unsafe {
            thaw_js_call_handle_mixed_exact_consuming_result(
                0, wire.as_ptr(), std::ptr::null(),
            )
        };
        assert!(!result.error.is_null());
        unsafe { thaw_arena::destroy_string(result.error.cast_mut()) };
        EXACT_MIXED_NAPI_RELEASES.with(|releases| assert_eq!(releases.get(), 1));
        unsafe { thaw_json_discard_graph_wire(wire.as_ptr()) };
        EXACT_MIXED_NAPI_RELEASES.with(|releases| assert_eq!(releases.get(), 1));

        // The consumed pointer can be granted again; discard remains a
        // one-shot operation even when called repeatedly or re-entrantly.
        assert!(unsafe { thaw_arena::register_owned_graph_wire(wire.as_ptr()) });
        unsafe { thaw_json_discard_graph_wire(wire.as_ptr()) };
        unsafe { thaw_json_discard_graph_wire(wire.as_ptr()) };
        EXACT_MIXED_NAPI_RELEASES.with(|releases| assert_eq!(releases.get(), 2));

        // A valid empty direct-handle array reaches callable lookup. Its
        // early return must consume the same registered argument grant.
        let handles = [0u64];
        assert!(unsafe { thaw_arena::register_owned_graph_wire(wire.as_ptr()) });
        let result = unsafe {
            thaw_js_call_handle_mixed_exact_consuming_result(
                0, wire.as_ptr(), handles.as_ptr().cast(),
            )
        };
        assert!(!result.error.is_null());
        unsafe { thaw_arena::destroy_string(result.error.cast_mut()) };
        EXACT_MIXED_NAPI_RELEASES.with(|releases| assert_eq!(releases.get(), 3));
        assert!(unsafe { thaw_arena::register_owned_graph_wire(wire.as_ptr()) });
        unsafe { thaw_json_discard_graph_wire(wire.as_ptr()) };
        EXACT_MIXED_NAPI_RELEASES.with(|releases| assert_eq!(releases.get(), 4));
    }).join().unwrap();
}

#[no_mangle]
pub unsafe extern "C" fn thaw_js_call_handle_mixed_native_json_result(handle: u64, graph_json: *const c_char, space_json: *const c_char, handles: *const u8) -> ThawResult {
    unsafe { thaw_js_call_handle_mixed_native_json_result_impl(handle, graph_json, space_json, handles, false) }
}

#[no_mangle]
pub unsafe extern "C" fn thaw_js_call_handle_mixed_native_json_graph_result(handle: u64, graph_json: *const c_char, space_json: *const c_char, handles: *const u8) -> ThawResult {
    unsafe { thaw_js_call_handle_mixed_native_json_result_impl(handle, graph_json, space_json, handles, true) }
}

#[no_mangle]
pub extern "C" fn thaw_js_call_handle_value_result(handle: u64, argument: u64) -> ThawResult {
    thaw_js_call_handle_value_result_impl(handle, argument, false)
}

#[no_mangle]
pub extern "C" fn thaw_js_call_handle_value_graph_result(handle: u64, argument: u64) -> ThawResult {
    thaw_js_call_handle_value_result_impl(handle, argument, true)
}

#[no_mangle]
pub extern "C" fn thaw_js_call_method_result(handle: u64, name: *const c_char, args_json: *const c_char) -> ThawResult {
    thaw_js_call_method_result_impl(handle, name, args_json, false)
}

#[no_mangle]
pub extern "C" fn thaw_js_call_method_graph_result(handle: u64, name: *const c_char, args_json: *const c_char) -> ThawResult {
    thaw_js_call_method_result_impl(handle, name, args_json, true)
}

#[no_mangle]
pub extern "C" fn thaw_js_resolve_handle_result(handle: u64) -> ThawResult {
    thaw_js_resolve_handle_result_impl(handle, false)
}

#[no_mangle]
pub extern "C" fn thaw_js_resolve_handle_graph_result(handle: u64) -> ThawResult {
    thaw_js_resolve_handle_result_impl(handle, true)
}

#[no_mangle]
pub extern "C" fn thaw_js_to_iterator_exact_result(
    source_handle: u64,
) -> ThawCallWithExceptionResult {
    enum Outcome { Value(u64), Exception(u64, bool) }
    let result: Result<Outcome, String> = with_active_or_context(|ctx| {
        let source = value_for_handle(&ctx, source_handle)?;
        let to_iterator: Function = ctx.globals().get("__thaw_to_iterator")
            .map_err(|error| error.to_string())?;
        match to_iterator.call::<_, Value>((source,)) {
            Ok(iterator) => retain_value(&ctx, iterator).map(Outcome::Value),
            Err(rquickjs::Error::Exception) => {
                let thrown = ctx.catch();
                let object_like = thrown.is_object() || thrown.is_function();
                retain_value(&ctx, thrown)
                    .map(|handle| Outcome::Exception(handle, object_like))
            }
            Err(error) => Err(error.to_string()),
        }
    });
    match result {
        Ok(Outcome::Value(value)) => ThawCallWithExceptionResult {
            value, exception_handle: 0, exception_object_like: 0,
            error: std::ptr::null(),
        },
        Ok(Outcome::Exception(exception_handle, object_like)) => ThawCallWithExceptionResult {
            value: 0, exception_handle, exception_object_like: u8::from(object_like),
            error: std::ptr::null(),
        },
        Err(error) => ThawCallWithExceptionResult {
            value: 0, exception_handle: 0, exception_object_like: 0,
            error: thaw_arena::owned_string(error),
        },
    }
}

/// One iterator step, with the exact JS result kept alive for a subsequent
/// non-assimilating graph capture. The bootstrap helper performs ToBoolean
/// on done before reading value and never awaits the method result.
#[no_mangle]
pub extern "C" fn thaw_js_iterator_step_exact_result(
    iterator_handle: u64, mode: u8, error_text: *const c_char,
) -> ThawCallWithExceptionResult {
    enum Outcome { Value(u64), Exception(u64, bool) }
    let result: Result<Outcome, String> = with_active_or_context(|ctx| {
        if mode > 2 { return Err("Invalid iterator step mode".into()); }
        let iterator = value_for_handle(&ctx, iterator_handle)?;
        let step: Function = ctx.globals().get("__thaw_iterator_step_exact")
            .map_err(|error| error.to_string())?;
        let error = if error_text.is_null() { String::new() } else { to_str(error_text) };
        match step.call::<_, Value>((iterator, mode, error)) {
            Ok(record) => retain_value(&ctx, record).map(Outcome::Value),
            Err(rquickjs::Error::Exception) => {
                let thrown = ctx.catch();
                let object_like = thrown.is_object() || thrown.is_function();
                retain_value(&ctx, thrown)
                    .map(|handle| Outcome::Exception(handle, object_like))
            }
            Err(error) => Err(error.to_string()),
        }
    });
    match result {
        Ok(Outcome::Value(value)) => ThawCallWithExceptionResult {
            value, exception_handle: 0, exception_object_like: 0,
            error: std::ptr::null(),
        },
        Ok(Outcome::Exception(exception_handle, object_like)) => ThawCallWithExceptionResult {
            value: 0, exception_handle, exception_object_like: u8::from(object_like),
            error: std::ptr::null(),
        },
        Err(error) => ThawCallWithExceptionResult {
            value: 0, exception_handle: 0, exception_object_like: 0,
            error: thaw_arena::owned_string(error),
        },
    }
}


/// Capture the exact value behind a retained QuickJS handle as an owned graph
/// packet. Unlike resolve/await paths this does not assimilate Promise-like
/// values, so a thrown Promise remains the original rejection value.
#[no_mangle]
pub extern "C" fn thaw_js_capture_handle_graph_result(handle: u64) -> ThawResult {
    let result: Result<String, String> = with_active_or_context(|ctx| {
        let value = value_for_handle(&ctx, handle)?;
        let encode: Function = ctx.globals().get("__thaw_json_graph_encode_js")
            .map_err(|error| error.to_string())?;
        if value.is_object() || value.is_function() || value.is_symbol() {
            encode.call((value.clone(), 0, true, false, value))
        } else {
            encode.call((value, 0, true, false))
        }.map_err(|error| match error {
            rquickjs::Error::Exception => describe_tagged_exception(&ctx),
            error => error.to_string(),
        })
    });
    match result {
        Ok(wire) => {
            let value = owned_graph_wire(&wire);
            if value.is_null() {
                ThawResult {
                    value: std::ptr::null(),
                    error: thaw_arena::owned_string("Unable to register JavaScript exception graph"),
                }
            } else {
                ThawResult { value, error: std::ptr::null() }
            }
        }
        Err(error) => ThawResult { value: std::ptr::null(), error: thaw_arena::owned_string(error) },
    }
}

// Unrun control: failed grant registration retires only the producer's
// retained lease before discarding the owned wire. Each test gets its own
// thread-local QuickJS context and handle registry.
#[cfg(test)]
#[test]
fn failed_exception_graph_grant_retires_producer_handle_lease() {
    std::thread::spawn(|| {
        let handle = with_context(|ctx| {
            let value = ctx.eval::<Value<'_>, _>("(() => 1)").unwrap();
            retain_value(&ctx, value).unwrap()
        });
        let wire = format!(
            r#"{{"root":{{"v":null}},"nodes":[],"leases":[{handle}],"napiLeases":[]}}"#
        );
        assert!(owned_graph_wire_with_registration(&wire, |_| false).is_null());
        assert_eq!(thaw_js_release_handle(handle), 0);
    }).join().unwrap();
}

// Source-authored controls for the forced-root graph wire prerequisite.
// These controls intentionally use the real QuickJS capture API, the real
// std graph decoder, and the production HostOperations callback tuple. They
// are not evidence of execution until the complete checkout is authorized.
#[cfg(test)]
mod forced_root_graph_wire_controls {
    use super::*;
    use std::ffi::{CStr, CString, c_char, c_void};

    thread_local! {
        static INPUT_HANDLES: std::cell::RefCell<Vec<u64>> = const {
            std::cell::RefCell::new(Vec::new())
        };
        static ALL_INPUT_HANDLES: std::cell::RefCell<Vec<u64>> = const {
            std::cell::RefCell::new(Vec::new())
        };
        static TEST_FIXTURE_FAILURES: std::cell::RefCell<Vec<&'static str>> = const {
            std::cell::RefCell::new(Vec::new())
        };
        static HANDLES_SLOT_FAULT_UNRESTORED: std::cell::Cell<bool> = const {
            std::cell::Cell::new(false)
        };
        static FIXTURE_STATE_UNRESTORED: std::cell::Cell<bool> = const {
            std::cell::Cell::new(false)
        };
    }

    fn record_input(handle: u64) {
        INPUT_HANDLES.with(|handles| handles.borrow_mut().push(handle));
        ALL_INPUT_HANDLES.with(|handles| {
            let mut handles = handles.borrow_mut();
            if !handles.contains(&handle) { handles.push(handle); }
        });
    }

    fn forget_input(handle: u64) -> bool {
        INPUT_HANDLES.with(|handles| {
            let mut handles = handles.borrow_mut();
            let Some(index) = handles.iter().rposition(|candidate| *candidate == handle) else {
                return false;
            };
            handles.swap_remove(index);
            true
        })
    }

    struct TestInputScope;

    impl TestInputScope {
        fn new() -> Self {
            INPUT_HANDLES.with(|handles| assert!(handles.borrow().is_empty()));
            ALL_INPUT_HANDLES.with(|handles| handles.borrow_mut().clear());
            TEST_FIXTURE_FAILURES.with(|failures| failures.borrow_mut().clear());
            HANDLES_SLOT_FAULT_UNRESTORED.with(|active| active.set(false));
            FIXTURE_STATE_UNRESTORED.with(|active| active.set(false));
            Self
        }
    }

    impl Drop for TestInputScope {
        fn drop(&mut self) {
            if HANDLES_SLOT_FAULT_UNRESTORED.with(std::cell::Cell::get)
                || FIXTURE_STATE_UNRESTORED.with(std::cell::Cell::get)
            {
                eprintln!("forced-root FAIL/HOLD: fixture restoration is unverified; O lookup/release was withheld");
                if !std::thread::panicking() {
                    panic!("forced-root teardown withheld O because registry/global restoration is unverified");
                }
                return;
            }
            let mut teardown_failed = false;
            let had_leftovers = INPUT_HANDLES.with(|handles| !handles.borrow().is_empty());
            let all_handles = ALL_INPUT_HANDLES.with(|handles| handles.borrow().clone());
            for handle in all_handles {
                let expected = INPUT_HANDLES.with(|handles| {
                    handles.borrow().iter().filter(|candidate| **candidate == handle).count() as u64
                });
                let actual = registry_state(handle);
                if actual != (expected, expected > 0) {
                    teardown_failed = true;
                    eprintln!("forced-root FAIL/HOLD: handle {handle} has registry state {actual:?}, remaining O ledger expects {expected}");
                    // If the registry has extra references after every Box
                    // and wire guard dropped, retire them only as explicit
                    // failing teardown repair. The mismatch remains a test
                    // failure and cannot satisfy a producer cleanup control.
                    if actual.0 > expected {
                        for _ in expected..actual.0 {
                            if thaw_js_release_handle(handle) != 1 { break; }
                        }
                    }
                }
            }
            let leftovers = INPUT_HANDLES.with(|handles| std::mem::take(&mut *handles.borrow_mut()));
            for handle in leftovers.into_iter().rev() {
                if thaw_js_release_handle(handle) != 1 { teardown_failed = true; }
            }
            let restoration_failed = TEST_FIXTURE_FAILURES.with(|failures| !failures.borrow().is_empty());
            if had_leftovers || teardown_failed || restoration_failed {
                eprintln!("forced-root FAIL/HOLD during test teardown: leftover O claims, P/D mismatch, failed release, or unverified restoration");
                if !std::thread::panicking() {
                    panic!("forced-root test teardown did not reach its explicit ownership/restoration assertions");
                }
            }
        }
    }

    struct OwnedJson {
        pointer: *mut c_void,
    }

    impl OwnedJson {
        fn new(pointer: *mut c_void) -> Self { Self { pointer } }
        fn as_ptr(&self) -> *mut c_void { self.pointer }
        fn is_null(&self) -> bool { self.pointer.is_null() }

        fn track_arena_root(mut self) -> Result<Self, &'static str> {
            if self.pointer.is_null() { return Err("decoder returned a null owned Json pointer"); }
            let input = std::mem::replace(&mut self.pointer, std::ptr::null_mut());
            // The real tracker destroys input on a null result. The guard is
            // disarmed before the call, so neither branch can double free.
            let rooted = unsafe { thaw_json_track_arena_owned_root(input) };
            drop(self);
            if rooted.is_null() { Err("root tracker consumed the Box on failure") }
            else { Ok(Self::new(rooted)) }
        }
    }

    impl Drop for OwnedJson {
        fn drop(&mut self) {
            if !self.pointer.is_null() {
                unsafe { thaw_json_destroy(self.pointer) };
                self.pointer = std::ptr::null_mut();
            }
        }
    }

    unsafe extern "C" {
        fn thaw_json_register_host_operations(
            retain: extern "C" fn(u64) -> u8,
            release: extern "C" fn(u64) -> u8,
            get: extern "C" fn(u64, *const c_char) -> ThawHandleResult,
            query: extern "C" fn(u64, u8) -> ThawResult,
            date_set: extern "C" fn(u64, f64) -> ThawResult,
            set: extern "C" fn(u64, *const c_char, *const c_char) -> ThawHandleResult,
            predicate: extern "C" fn(u64, *const c_char, u8) -> ThawHandleResult,
            enumerate: extern "C" fn(u64, u8) -> ThawResult,
        );
        fn thaw_json_graph_decode(source: *const c_char) -> *mut c_void;
        fn thaw_json_discard_graph_wire(source: *const c_char);
        fn thaw_json_take_graph_error() -> u8;
        fn thaw_json_take_host_error() -> *const c_char;
        fn thaw_json_typeof(value: *const c_void) -> *const c_char;
        fn thaw_json_strict_equal(left: *const c_void, right: *const c_void) -> u8;
        fn thaw_json_borrowed_handle_id(value: *const c_void) -> u64;
        fn thaw_json_clone(value: *const c_void) -> *mut c_void;
        fn thaw_json_destroy(value: *mut c_void);
        fn thaw_json_track_arena_owned_root(value: *mut c_void) -> *mut c_void;
        fn thaw_json_get(value: *mut c_void, key: *const c_char) -> *mut c_void;
        fn thaw_json_object_set_number(value: *mut c_void, key: *const c_char, number: f64);
        fn thaw_json_as_number(value: *mut c_void) -> f64;
        fn thaw_cstring_destroy(value: *mut c_char);
    }

    fn install_real_host_callbacks() {
        // The result layouts are the api.rs #[repr(C)] ThawHandleResult
        // (u64 plus error pointer) and ThawResult (text plus error pointer),
        // matching thaw-std's HostHandleResult and HostTextResult exactly.
        unsafe {
            thaw_json_register_host_operations(
                thaw_js_retain_handle,
                thaw_js_release_handle,
                thaw_js_get_property_json_key_result,
                thaw_js_host_query_result,
                thaw_js_host_date_set_result,
                thaw_js_set_property_graph_result,
                thaw_js_property_predicate_json_key_result,
                thaw_js_host_enumerate_result,
            );
        }
    }

    fn retain_source(source: &str) -> u64 {
        let handle = with_context(|ctx| {
            let value = ctx.eval::<Value<'_>, _>(source)
                .expect("create real JavaScript fixture value");
            retain_value(&ctx, value).expect("retain real JavaScript fixture value")
        });
        record_input(handle);
        handle
    }

    fn registry_state(handle: u64) -> (u64, bool) {
        with_context(|ctx| {
            let refs: Array = ctx.globals().get("__thaw_value_handle_refs")
                .expect("real refs registry");
            let live: Array = ctx.globals().get("__thaw_value_handle_live")
                .expect("real live registry");
            let index = (handle - 1) as usize;
            (refs.get::<u64>(index).expect("read primitive registry count"),
             live.get::<bool>(index).expect("read primitive registry liveness"))
        })
    }

    fn eval_bool(source: &str) -> bool {
        with_context(|ctx| ctx.eval::<bool, _>(source).expect("evaluate fixture assertion"))
    }

    fn free_c_string(value: *const c_char) {
        if !value.is_null() { unsafe { thaw_cstring_destroy(value.cast_mut()) }; }
    }

    struct OwnedText { pointer: *const c_char }

    impl OwnedText {
        fn new(pointer: *const c_char) -> Self { Self { pointer } }
        fn is_null(&self) -> bool { self.pointer.is_null() }
        fn to_string_lossy(&self) -> String {
            if self.pointer.is_null() { String::new() }
            else { unsafe { CStr::from_ptr(self.pointer) }.to_string_lossy().into_owned() }
        }
    }

    impl Drop for OwnedText {
        fn drop(&mut self) { free_c_string(self.pointer); }
    }

    struct CaptureResultOwner {
        value: *const c_char,
        error: OwnedText,
        grant_pending: bool,
    }

    impl CaptureResultOwner {
        fn new(result: ThawResult) -> Self {
            Self { value: result.value, error: OwnedText::new(result.error), grant_pending: true }
        }
        fn take_value(&mut self) -> *const c_char {
            self.grant_pending = false;
            std::mem::replace(&mut self.value, std::ptr::null())
        }
    }

    impl Drop for CaptureResultOwner {
        fn drop(&mut self) {
            if !self.value.is_null() {
                if self.grant_pending { unsafe { thaw_json_discard_graph_wire(self.value) }; }
                free_c_string(self.value);
                self.value = std::ptr::null();
            }
        }
    }

    struct RegisteredGraphWire {
        pointer: *const c_char,
        text: String,
        grant_pending: bool,
    }

    impl RegisteredGraphWire {
        fn capture(handle: u64) -> Self {
            let mut result = CaptureResultOwner::new(thaw_js_capture_handle_graph_result(handle));
            assert!(result.error.is_null(), "real capture failed: {}", result.error.to_string_lossy());
            assert!(!result.value.is_null(), "capture returned no registered wire");
            let mut wire = Self { pointer: result.take_value(), text: String::new(), grant_pending: true };
            wire.text = unsafe { CStr::from_ptr(wire.pointer) }.to_str()
                .expect("wire is ASCII/UTF-8").to_owned();
            wire
        }

        fn decode(&mut self) -> OwnedJson {
            // The real decoder consumes the registered one-shot grant even
            // when it returns its documented failure Null.
            let decoded = OwnedJson::new(unsafe { thaw_json_graph_decode(self.pointer) });
            self.grant_pending = false;
            decoded
        }

        fn discard(&mut self) {
            if self.grant_pending {
                unsafe { thaw_json_discard_graph_wire(self.pointer) };
                self.grant_pending = false;
            }
        }
    }

    impl Drop for RegisteredGraphWire {
        fn drop(&mut self) {
            if self.grant_pending { self.discard(); }
            free_c_string(self.pointer);
        }
    }

    fn assert_forced_packet(text: &str, handle: u64) {
        let expected = format!(
            r#"{{"root":{{"r":0}},"nodes":[{{"hdl":{handle}}}],"leases":[{handle}],"napiLeases":[]}}"#
        );
        assert_eq!(text, expected.as_str(), "forced-root producer uses the exact fixed packet bytes");
        let graph: serde_json::Value = serde_json::from_str(text)
            .expect("actual capture wire parses");
        assert_eq!(graph.as_object().map(serde_json::Map::len), Some(4));
        assert_eq!(graph.get("root"), Some(&serde_json::json!({"r": 0})));
        let nodes = graph.get("nodes").and_then(serde_json::Value::as_array)
            .expect("nodes array");
        assert_eq!(nodes.len(), 1);
        let node = nodes[0].as_object().expect("one handle node");
        assert_eq!(node.len(), 1, "forced node has only hdl");
        assert_eq!(node.get("hdl").and_then(serde_json::Value::as_u64), Some(handle));
        assert!(handle > 0 && handle <= 9_007_199_254_740_991);
        assert_eq!(graph.get("leases").and_then(serde_json::Value::as_array)
            .map(|leases| leases.iter().filter_map(serde_json::Value::as_u64).collect::<Vec<_>>()),
            Some(vec![handle]));
        assert_eq!(graph.get("napiLeases").and_then(serde_json::Value::as_array)
            .map(Vec::len), Some(0));
    }

    fn decode_and_assert_type(handle: u64, expected_type: &str) -> OwnedJson {
        let (before_count, before_live) = registry_state(handle);
        assert!(before_live && before_count > 0, "input O is live before capture");
        let mut wire = RegisteredGraphWire::capture(handle);
        assert_forced_packet(&wire.text, handle);
        assert_eq!(registry_state(handle), (before_count + 1, true), "capture adds exactly P to O");
        let decoded = wire.decode();
        assert!(!decoded.is_null(), "real std decoder returned a Box");
        assert_eq!(unsafe { thaw_json_take_graph_error() }, 0);
        let actual = unsafe { CStr::from_ptr(thaw_json_typeof(decoded.as_ptr())) }
            .to_str().expect("static typeof text");
        assert_eq!(actual, expected_type);
        assert_eq!(registry_state(handle), (before_count + 1, true), "D replaces P after decode");
        decoded
    }

    fn destroy_value(value: OwnedJson) { drop(value); }

    fn take_host_error_text() -> Option<String> {
        let value = OwnedText::new(unsafe { thaw_json_take_host_error() });
        if value.is_null() { None } else { Some(value.to_string_lossy()) }
    }

    fn release_input(handle: u64) {
        let (before_count, before_live) = registry_state(handle);
        assert!(before_live && before_count > 0, "input reference is live before retirement");
        let released = thaw_js_release_handle(handle);
        if released == 1 { assert!(forget_input(handle), "retire tracked original O once"); }
        assert_eq!(released, 1, "retire original O");
        assert_eq!(registry_state(handle), (before_count - 1, before_count > 1));
    }

    fn retire_input_in_context(ctx: &Ctx<'_>, handle: u64) -> u8 {
        let released = release_handle_in_context(ctx, handle);
        if released == 1 { assert!(forget_input(handle), "retire tracked original O once"); }
        released
    }

    fn encoder_text_for_handle(handle: u64) -> String {
        with_context(|ctx| {
            let value = value_for_handle(&ctx, handle).expect("live original handle");
            let encode: Function = ctx.globals().get("__thaw_json_graph_encode_js")
                .expect("bootstrap graph encoder");
            encode.call((value.clone(), 0, true, false, value))
                .expect("real forced-root encoder bytes")
        })
    }

    fn release_encoder_leases(text: &str) -> Result<(), String> {
        let graph: serde_json::Value = serde_json::from_str(text)
            .map_err(|error| format!("cannot parse unregistered encoder packet during teardown: {error}"))?;
        for handle in graph.get("leases").and_then(serde_json::Value::as_array)
            .into_iter().flatten().filter_map(serde_json::Value::as_u64)
        {
            if handle != 0 && thaw_js_release_handle(handle) != 1 {
                return Err(format!("producer lease {handle} did not retire once"));
            }
        }
        Ok(())
    }

    struct UnregisteredEncodedPacket {
        text: String,
        transferred: bool,
    }

    impl UnregisteredEncodedPacket {
        fn new(text: String) -> Self { Self { text, transferred: false } }
        fn as_str(&self) -> &str { &self.text }
        fn transfer_to_registration_helper(&mut self) { self.transferred = true; }
    }

    impl Drop for UnregisteredEncodedPacket {
        fn drop(&mut self) {
            if !self.transferred {
                if let Err(error) = release_encoder_leases(&self.text) {
                    eprintln!("forced-root FAIL/HOLD: {error}");
                    TEST_FIXTURE_FAILURES.with(|failures| failures.borrow_mut().push("unregistered producer lease teardown failed"));
                }
                self.transferred = true;
            }
        }
    }

    struct EvalRestore {
        source: &'static str,
        armed: bool,
        faulted_handles_slot: bool,
    }

    impl EvalRestore {
        fn restore(&mut self) -> bool {
            if !self.armed { return true; }
            match with_context(|ctx| ctx.eval::<(), _>(self.source)) {
                Ok(()) => {
                    self.armed = false;
                    FIXTURE_STATE_UNRESTORED.with(|active| active.set(false));
                    if self.faulted_handles_slot {
                        HANDLES_SLOT_FAULT_UNRESTORED.with(|active| active.set(false));
                    }
                    true
                }
                Err(_) => {
                    TEST_FIXTURE_FAILURES.with(|failures| failures.borrow_mut().push(self.source));
                    false
                }
            }
        }
    }

    impl Drop for EvalRestore {
        fn drop(&mut self) {
            if self.armed {
                let restored = with_context(|ctx| {
                    // Clear any test-only pending exception first so the
                    // pre-created descriptor-restoration function can run.
                    drop(ctx.catch());
                    ctx.eval::<(), _>(self.source)
                });
                match restored {
                    Ok(()) => {
                        self.armed = false;
                        FIXTURE_STATE_UNRESTORED.with(|active| active.set(false));
                        if self.faulted_handles_slot {
                            HANDLES_SLOT_FAULT_UNRESTORED.with(|active| active.set(false));
                        }
                    }
                    Err(error) => {
                        TEST_FIXTURE_FAILURES.with(|failures| failures.borrow_mut().push(self.source));
                        FIXTURE_STATE_UNRESTORED.with(|active| active.set(true));
                        if self.faulted_handles_slot {
                            HANDLES_SLOT_FAULT_UNRESTORED.with(|active| active.set(true));
                        }
                        eprintln!("forced-root FAIL/HOLD: could not restore fixture state: {error}");
                    }
                }
            }
        }
    }

    #[test]
    fn forced_root_transfer_failure_rolls_back_once() {
        std::thread::spawn(|| {
            let _inputs = TestInputScope::new();
            install_real_host_callbacks();

            // F4(a): a released input is rejected before any producer P can
            // be acquired or registered.
            let released = retain_source("({released: true})");
            release_input(released);
            let failed = CaptureResultOwner::new(thaw_js_capture_handle_graph_result(released));
            assert!(failed.value.is_null());
            assert!(!failed.error.is_null());
            drop(failed);
            assert_eq!(registry_state(released), (0, false));

            // F4(b): fail only the actual refs-array increment, then restore
            // its exact own data descriptor before reading the count again.
            let restricted = retain_source("({restricted: true})");
            let saved = eval_bool(&format!("(() => {{ const a=globalThis.__thaw_value_handle_refs; const i={}; const d=Object.getOwnPropertyDescriptor(a,i); if(!d || !('value' in d) || !d.writable) return false; globalThis.__fr_saved_refs_descriptor=d; globalThis.__fr_restore_refs_slot=()=>{{Object.defineProperty(a,i,globalThis.__fr_saved_refs_descriptor); delete globalThis.__fr_restore_refs_slot; delete globalThis.__fr_saved_refs_descriptor;}}; Object.defineProperty(a,i,{{...d,writable:false}}); return true; }})()", restricted - 1));
            assert!(saved, "refs slot must be a writable own data property before the fault");
            let mut restore_refs = EvalRestore {
                source: "globalThis.__fr_restore_refs_slot()",
                armed: true,
                faulted_handles_slot: false,
            };
            let failed = CaptureResultOwner::new(thaw_js_capture_handle_graph_result(restricted));
            assert!(failed.value.is_null(), "failed P retain cannot escape as a packet");
            assert!(!failed.error.is_null());
            drop(failed);
            if !restore_refs.restore() { return; }
            assert_eq!(registry_state(restricted), (1, true), "O is unchanged after failed retain");
            release_input(restricted);

            // F4(c): get bytes from the real encoder and let the real
            // registration helper inject its documented false-registration
            // outcome. The helper owns the actual P rollback.
            let registration = retain_source("({registration: true})");
            let mut actual_wire = UnregisteredEncodedPacket::new(encoder_text_for_handle(registration));
            assert_forced_packet(actual_wire.as_str(), registration);
            assert_eq!(registry_state(registration), (2, true));
            actual_wire.transfer_to_registration_helper();
            assert!(owned_graph_wire_with_registration(actual_wire.as_str(), |_| false).is_null());
            assert_eq!(registry_state(registration), (1, true), "only P was retired");
            release_input(registration);

            // F4(d): the registered decoder grant is one-shot even when a
            // discard is repeated; this exercises its actual lease list.
            let discarded = retain_source("({discarded: true})");
            let mut wire = RegisteredGraphWire::capture(discarded);
            assert_forced_packet(&wire.text, discarded);
            assert_eq!(registry_state(discarded), (2, true));
            unsafe { thaw_json_discard_graph_wire(wire.pointer); }
            wire.grant_pending = false;
            assert_eq!(registry_state(discarded), (1, true));
            unsafe { thaw_json_discard_graph_wire(wire.pointer); }
            assert_eq!(registry_state(discarded), (1, true), "second discard is inert");
            drop(wire);
            release_input(discarded);

            // F4(e): mutate only the live bytes. The decoder must reject the
            // snapshot mismatch but clean P from the registered snapshot.
            let mutated = retain_source("({snapshot: true})");
            let mut wire = RegisteredGraphWire::capture(mutated);
            assert_forced_packet(&wire.text, mutated);
            unsafe { *(wire.pointer as *mut c_char) = b'[' as c_char; }
            let failed_box = wire.decode();
            assert!(!failed_box.is_null());
            assert_eq!(unsafe { thaw_json_take_graph_error() }, 1);
            assert_eq!(registry_state(mutated), (1, true), "snapshot cleanup retires P once");
            destroy_value(failed_box);
            assert_eq!(registry_state(mutated), (1, true), "failure Null owns no D");
            drop(wire);
            release_input(mutated);

            // F4(f): the accepted asymmetric negative. Set up the exact
            // restoration function and sentinel before faulting one handles
            // slot. O and P are both live; refs/live remain writable data.
            let asymmetric = retain_source("({asymmetric: true})");
            let mut wire = RegisteredGraphWire::capture(asymmetric);
            assert_forced_packet(&wire.text, asymmetric);
            assert_eq!(registry_state(asymmetric), (2, true));
            assert!(eval_bool(&format!("(() => {{ const i={}; const h=globalThis.__thaw_value_handles; const r=globalThis.__thaw_value_handle_refs; const l=globalThis.__thaw_value_handle_live; const d=Object.getOwnPropertyDescriptor(h,i); const rd=Object.getOwnPropertyDescriptor(r,i); const ld=Object.getOwnPropertyDescriptor(l,i); if(!d || !('value' in d) || !d.configurable || !rd || !('value' in rd) || !rd.writable || !ld || !('value' in ld) || !ld.writable || r[i]!==2 || l[i]!==true) return false; globalThis.__fr_saved_handles_descriptor=d; globalThis.__fr_fault_reads=0; globalThis.__fr_fault_sentinel={{tag:'forced-root-handles-slot-fault'}}; globalThis.__fr_restore_handles_slot=()=>{{Object.defineProperty(h,i,globalThis.__fr_saved_handles_descriptor); delete globalThis.__fr_restore_handles_slot; delete globalThis.__fr_saved_handles_descriptor; delete globalThis.__fr_fault_sentinel; delete globalThis.__fr_fault_reads;}}; Object.defineProperty(h,i,{{configurable:true,enumerable:d.enumerable,get(){{globalThis.__fr_fault_reads++; throw globalThis.__fr_fault_sentinel;}}}}); return true; }})()", asymmetric - 1)), "only a configurable handles data slot may be faulted");
            let mut restore_handles = EvalRestore {
                source: "globalThis.__fr_restore_handles_slot()",
                armed: true,
                faulted_handles_slot: true,
            };
            HANDLES_SLOT_FAULT_UNRESTORED.with(|active| active.set(true));
            let failure_box = wire.decode();
            assert!(!failure_box.is_null());
            let during_fault = registry_state(asymmetric);
            let fault_reads: u64 = with_context(|ctx| ctx.globals().get("__fr_fault_reads")
                .expect("read isolated getter counter without running another JS operation"));
            let graph_error = unsafe { thaw_json_take_graph_error() };
            assert_eq!(during_fault, (1, true), "real decoder cleanup takes refs 2 to 1");
            assert_eq!(fault_reads, 1, "real retain callback hits only the throwing slot");
            assert_eq!(graph_error, 1);
            let sentinel_seen = with_context(|ctx| {
                let caught = ctx.catch();
                let sentinel: Value = ctx.globals().get("__fr_fault_sentinel")
                    .expect("pre-created sentinel");
                let object: Object = ctx.globals().get("Object").expect("Object intrinsic");
                let object_is: Function = object.get("is").expect("Object.is");
                object_is.call::<_, bool>((caught, sentinel)).expect("compare caught identity")
            });
            assert!(sentinel_seen, "the real callback's pending getter exception is drained by identity");
            if !restore_handles.restore() { return; }
            assert_eq!(registry_state(asymmetric), (1, true), "descriptor is restored before O lookup");
            with_context(|ctx| {
                let original = value_for_handle(&ctx, asymmetric)
                    .expect("restored descriptor recovers original O");
                let duplicate = retain_value(&ctx, original).expect("same original identity");
                record_input(duplicate);
                assert_eq!(duplicate, asymmetric);
                assert_eq!(retire_input_in_context(&ctx, duplicate), 1);
            });
            destroy_value(failure_box);
            drop(wire); // its grant was consumed by the decoder
            release_input(asymmetric);

            // F4(g): expose the real writable Host-query boundary. There is
            // deliberately no substitute/default-safe callback in this test.
            let query_override = retain_source("({queryOverride: true})");
            let mut wire = RegisteredGraphWire::capture(query_override);
            assert_forced_packet(&wire.text, query_override);
            assert!(eval_bool("(() => { globalThis.__fr_saved_query_descriptor=Object.getOwnPropertyDescriptor(globalThis,'__thaw_json_host_query'); if(!globalThis.__fr_saved_query_descriptor) return false; globalThis.__fr_restore_query=()=>{Object.defineProperty(globalThis,'__thaw_json_host_query',globalThis.__fr_saved_query_descriptor); delete globalThis.__fr_saved_query_descriptor; delete globalThis.__fr_restore_query;}; Object.defineProperty(globalThis,'__thaw_json_host_query',{...globalThis.__fr_saved_query_descriptor,value(){throw new Error('forced-root query override');}}); return true; })()"));
            let mut restore_query = EvalRestore {
                source: "globalThis.__fr_restore_query()",
                armed: true,
                faulted_handles_slot: false,
            };
            let failed = wire.decode();
            assert!(!failed.is_null());
            assert_eq!(unsafe { thaw_json_take_graph_error() }, 1);
            assert!(take_host_error_text().is_some(), "actual query error is visible");
            assert_eq!(registry_state(query_override), (1, true), "failed D and P are each retired");
            if !restore_query.restore() { return; }
            destroy_value(failed);
            drop(wire);
            release_input(query_override);
        }).join().unwrap();
    }

    #[test]
    fn forced_root_decoded_lease_outlives_consumed_input_and_root() {
        std::thread::spawn(|| {
            let _inputs = TestInputScope::new();
            install_real_host_callbacks();
            // R6 HOLD: this exercises the actual std-to-QuickJS release
            // callback outside with_context, but it does not prove recursive
            // same-grant cleanup/reentry. The pinned production route has no
            // safe source-backed fixture here, so no fake callback is used.
            for consume_original in [false, true] {
                let handle = retain_source("({value: 41})");
                let mut first_wire = RegisteredGraphWire::capture(handle);
                let mut second_wire = RegisteredGraphWire::capture(handle);
                assert_forced_packet(&first_wire.text, handle);
                assert_forced_packet(&second_wire.text, handle);
                assert_eq!(registry_state(handle), (3, true), "O plus independent P1/P2");

                if consume_original {
                    release_input(handle);
                    assert_eq!(registry_state(handle), (2, true), "caller consumes O after capture, before decode");
                }

                let first = first_wire.decode();
                assert!(!first.is_null());
                assert_eq!(unsafe { thaw_json_take_graph_error() }, 0);
                assert_eq!(registry_state(handle), if consume_original { (2, true) } else { (3, true) }, "D1 replaces P1");
                let first = first.track_arena_root().expect("root tracker preserves D1 Box");
                assert!(!first.is_null());
                assert_eq!(registry_state(handle), if consume_original { (2, true) } else { (3, true) },
                    "successful root tracking adds no registry claim");
                let second = second_wire.decode();
                assert!(!second.is_null());
                assert_eq!(unsafe { thaw_json_take_graph_error() }, 0);
                assert_eq!(registry_state(handle), if consume_original { (2, true) } else { (3, true) }, "D2 replaces P2");
                let second = second.track_arena_root().expect("root tracker preserves D2 Box");
                assert!(!second.is_null());
                assert_eq!(registry_state(handle), if consume_original { (2, true) } else { (3, true) },
                    "successful second root tracking adds no registry claim");
                assert_eq!(unsafe { thaw_json_borrowed_handle_id(first.as_ptr()) }, handle);
                assert_eq!(unsafe { thaw_json_borrowed_handle_id(second.as_ptr()) }, handle);

                let first_clone = OwnedJson::new(unsafe { thaw_json_clone(first.as_ptr()) });
                let first_clone = first_clone.track_arena_root().expect("root tracker preserves cloned Box");
                assert!(!first_clone.is_null());
                assert_eq!(registry_state(handle), if consume_original { (2, true) } else { (3, true) },
                    "cloning this Host lease shares its Rc without another registry retain");
                assert_eq!(unsafe { thaw_json_strict_equal(first.as_ptr(), first_clone.as_ptr()) }, 1);
                destroy_value(first);
                let key = CString::new("value").unwrap();
                let still_live = OwnedJson::new(unsafe { thaw_json_get(first_clone.as_ptr(), key.as_ptr()) });
                assert_eq!(unsafe { thaw_json_as_number(still_live.as_ptr()) }, 41.0,
                    "the last surviving alias still reaches the real JS object");
                destroy_value(still_live);
                assert_eq!(registry_state(handle), if consume_original { (2, true) } else { (3, true) },
                    "first Box retirement leaves its cloned Host lease alive");
                destroy_value(first_clone);
                assert_eq!(registry_state(handle), if consume_original { (1, true) } else { (2, true) },
                    "last owner of D1 retires one shared lease");

                let second_value = OwnedJson::new(unsafe { thaw_json_get(second.as_ptr(), key.as_ptr()) });
                assert_eq!(unsafe { thaw_json_as_number(second_value.as_ptr()) }, 41.0,
                    "independent D2 remains live after D1 teardown");
                destroy_value(second_value);
                destroy_value(second);
                assert_eq!(registry_state(handle), if consume_original { (0, false) } else { (1, true) },
                    "final D2 destruction retires its one lease");
                drop(first_wire);
                drop(second_wire);
                if !consume_original { release_input(handle); }
            }
        }).join().unwrap();
    }

    #[test]
    fn forced_root_guard_preserves_other_graph_modes() {
        std::thread::spawn(|| {
            let _inputs = TestInputScope::new();
            install_real_host_callbacks();
            // Packet owners live outside the active QuickJS context borrow.
            // A later encode failure unwinds the context closure first, then
            // retires earlier P claims through the real callback route.
            let mut packets: Vec<UnregisteredEncodedPacket> = Vec::with_capacity(11);
            with_context(|ctx| {
                let encode: Function = ctx.globals().get("__thaw_json_graph_encode_js")
                    .expect("bootstrap graph encoder");
                let root: Value = ctx.eval("({x:1})").expect("object source");
                let other: Value = ctx.eval("({})").expect("nonmatching force value");
                let nested_pair: Array = ctx.eval("(() => { const value={}; return [[value],value]; })()")
                    .expect("nested receiver and separate forced value");
                let nested: Value = nested_pair.get(0).expect("nested receiver array");
                let nested_force: Value = nested_pair.get(1).expect("nested forced value");
                let masked: Value = ctx.eval("[{}]").expect("masked source");
                let scalar: Value = ctx.eval("'scalar'").expect("scalar source");
                let mask_match: Value = ctx.eval("[{}]").expect("matching mask root");
                let nonlive_match: Value = ctx.eval("({nonlive: true})").expect("matching non-live root");
                let plain_match: Value = ctx.eval("({plain: true})").expect("matching plain root");
                packets.push(UnregisteredEncodedPacket::new(encode.call((root.clone(), 0, true, false))
                    .expect("omitted force mode")));
                packets.push(UnregisteredEncodedPacket::new(encode.call((root.clone(), 0, true, false, other))
                    .expect("nonmatching root/force mode")));
                packets.push(UnregisteredEncodedPacket::new(encode.call((nested, 0, true, false, nested_force))
                    .expect("nested force mode")));
                packets.push(UnregisteredEncodedPacket::new(encode.call((masked, 1, false, false))
                    .expect("nonzero mask mode")));
                packets.push(UnregisteredEncodedPacket::new(encode.call((root.clone(), 0, false, false))
                    .expect("non-live mode")));
                packets.push(UnregisteredEncodedPacket::new(encode.call((root, 0, true, true))
                    .expect("plain-result mode")));
                packets.push(UnregisteredEncodedPacket::new(encode.call((scalar.clone(), 0, true, false))
                    .expect("ordinary scalar mode")));
                // Each exact root===force case holds the other guard fields
                // constant so mask, live, plain-result and type exclusions
                // are independently exercised against baseline token output.
                packets.push(UnregisteredEncodedPacket::new(encode.call((
                    mask_match.clone(), 1, true, false, mask_match,
                )).expect("matching root with only nonzero-mask exclusion")));
                packets.push(UnregisteredEncodedPacket::new(encode.call((
                    nonlive_match.clone(), 0, false, false, nonlive_match,
                )).expect("matching root with only non-live exclusion")));
                packets.push(UnregisteredEncodedPacket::new(encode.call((
                    plain_match.clone(), 0, true, true, plain_match,
                )).expect("matching root with only plain-result exclusion")));
                packets.push(UnregisteredEncodedPacket::new(encode.call((
                    scalar.clone(), 0, true, false, scalar,
                )).expect("matching scalar force excluded by root type")));
            });

            let ordinary = r#"{"root":{"r":0},"nodes":[{"o":[["x",{"v":1},7]]}],"leases":[],"napiLeases":[]}"#;
            assert_eq!(packets[0].as_str(), ordinary, "omitted force keeps ordinary bytes");
            assert_eq!(packets[1].as_str(), ordinary, "nonmatching force keeps ordinary bytes");
            assert_eq!(packets[4].as_str(), ordinary, "non-live ordinary object stays unchanged");
            assert_eq!(packets[5].as_str(), ordinary, "plain object mode retains its existing bytes");
            assert_eq!(packets[6].as_str(), r#"{"root":{"v":"scalar"},"nodes":[],"leases":[],"napiLeases":[]}"#);

            let nested: serde_json::Value = serde_json::from_str(packets[2].as_str()).unwrap();
            assert_eq!(nested.get("root"), Some(&serde_json::json!({"r": 0})));
            let nested_leases = nested.get("leases").and_then(serde_json::Value::as_array).unwrap();
            assert_eq!(nested_leases.len(), 2, "entry and own property each transfer a lease");
            let nested_handle = nested_leases[0].as_u64().expect("numeric handle lease");
            assert_eq!(nested_leases[1].as_u64(), Some(nested_handle), "both nested tokens reuse canonical H");
            assert_eq!(packets[2].as_str(), format!(
                r#"{{"root":{{"r":0}},"nodes":[{{"a":[{{"hdl":{nested_handle}}}],"p":[["0",{{"hdl":{nested_handle}}},7],["length",{{"v":1}},1]]}}],"leases":[{nested_handle},{nested_handle}],"napiLeases":[]}}"#
            ));

            let masked: serde_json::Value = serde_json::from_str(packets[3].as_str()).unwrap();
            let masked_leases = masked.get("leases").and_then(serde_json::Value::as_array).unwrap();
            assert_eq!(masked_leases.len(), 1);
            let masked_handle = masked_leases[0].as_u64().expect("masked handle lease");
            assert_eq!(packets[3].as_str(), format!(
                r#"{{"root":{{"r":0}},"nodes":[{{"a":[{{"r":1}}],"p":[["0",{{"r":1}},7],["length",{{"v":1}},1]]}},{{"hdl":{masked_handle}}}],"leases":[{masked_handle}],"napiLeases":[]}}"#
            ));

            for packet in &packets[7..=10] {
                let graph: serde_json::Value = serde_json::from_str(packet.as_str()).unwrap();
                let root_handle = graph.get("root").and_then(|root| root.get("hdl"))
                    .and_then(serde_json::Value::as_u64).expect("matching root remains the baseline direct hdl token");
                assert_eq!(graph.get("nodes").and_then(serde_json::Value::as_array).map(Vec::len), Some(0));
                assert_eq!(graph.get("leases").and_then(serde_json::Value::as_array)
                    .map(|leases| leases.iter().filter_map(serde_json::Value::as_u64).collect::<Vec<_>>()),
                    Some(vec![root_handle]));
                assert_eq!(packet.as_str(), format!(
                    r#"{{"root":{{"hdl":{root_handle}}},"nodes":[],"leases":[{root_handle}],"napiLeases":[]}}"#
                ));
            }

            // Each packet guard releases every real producer claim exactly
            // once, including nested same-H multiplicity, before O cleanup.
            drop(packets);

            // The scalar input route is the real capture API's four-argument
            // ordinary encoder call, not the forced-root fast path.
            let scalar_handle = retain_source("'scalar'");
            let mut scalar_wire = RegisteredGraphWire::capture(scalar_handle);
            assert_eq!(scalar_wire.text,
                r#"{"root":{"v":"scalar"},"nodes":[],"leases":[],"napiLeases":[]}"#);
            let scalar_value = scalar_wire.decode();
            assert!(!scalar_value.is_null());
            assert_eq!(unsafe { thaw_json_take_graph_error() }, 0);
            assert_eq!(unsafe { CStr::from_ptr(thaw_json_typeof(scalar_value.as_ptr())) }.to_bytes(), b"string");
            assert_eq!(registry_state(scalar_handle), (1, true), "scalar path adds no Host lease");
            destroy_value(scalar_value);
            drop(scalar_wire);
            release_input(scalar_handle);
        }).join().unwrap();
    }

    #[test]
    fn forced_root_default_query_boundary_is_explicit() {
        std::thread::spawn(|| {
            let _inputs = TestInputScope::new();
            // Missing HostOperations yields the documented marker-object
            // decoder branch. It is not original-value projection; P needs
            // explicit fixture cleanup because no real release callback is
            // registered in this isolated child thread.
            std::thread::spawn(|| {
                let _inputs = TestInputScope::new();
                let marker_handle = retain_source("({markerOnly: true})");
                let mut marker_wire = RegisteredGraphWire::capture(marker_handle);
                assert_forced_packet(&marker_wire.text, marker_handle);
                let marker = marker_wire.decode();
                assert!(!marker.is_null());
                assert_eq!(unsafe { thaw_json_take_graph_error() }, 0);
                assert_eq!(unsafe { thaw_json_borrowed_handle_id(marker.as_ptr()) }, 0,
                    "marker object is not a successful projection of the original handle");
                assert_eq!(registry_state(marker_handle), (2, true),
                    "without HostOperations the decoder cannot retire P");
                assert_eq!(thaw_js_release_handle(marker_handle), 1,
                    "fixture explicitly retires unconsumed P after demonstrating the boundary");
                destroy_value(marker);
                drop(marker_wire);
                release_input(marker_handle);
            }).join().unwrap();

            install_real_host_callbacks();
            let default_query = retain_source("(() => { globalThis.__fr_default_query_gets=0; const target={}; return new Proxy(target,{get(t,k,r){__fr_default_query_gets++; return Reflect.get(t,k,r);},ownKeys(){__fr_default_query_gets++; return [];},getOwnPropertyDescriptor(){__fr_default_query_gets++; return undefined;},getPrototypeOf(){__fr_default_query_gets++; return null;}}); })()");
            let decoded = decode_and_assert_type(default_query, "object");
            assert!(eval_bool("__fr_default_query_gets === 0"),
                "the actual default operation-0 query is typeof and does not inspect the root");
            destroy_value(decoded);
            release_input(default_query);

            let overridden_query = retain_source("({override: true})");
            let mut wire = RegisteredGraphWire::capture(overridden_query);
            assert_forced_packet(&wire.text, overridden_query);
            assert!(eval_bool("(() => { globalThis.__fr_saved_query_descriptor=Object.getOwnPropertyDescriptor(globalThis,'__thaw_json_host_query'); if(!globalThis.__fr_saved_query_descriptor) return false; globalThis.__fr_restore_query=()=>{Object.defineProperty(globalThis,'__thaw_json_host_query',globalThis.__fr_saved_query_descriptor); delete globalThis.__fr_saved_query_descriptor; delete globalThis.__fr_restore_query;}; Object.defineProperty(globalThis,'__thaw_json_host_query',{...globalThis.__fr_saved_query_descriptor,value(){throw new Error('documented mutable query boundary');}}); return true; })()"));
            let mut restore = EvalRestore {
                source: "globalThis.__fr_restore_query()",
                armed: true,
                faulted_handles_slot: false,
            };
            let failed = wire.decode();
            assert!(!failed.is_null());
            assert_eq!(unsafe { thaw_json_take_graph_error() }, 1);
            assert!(take_host_error_text().is_some());
            assert_eq!(registry_state(overridden_query), (1, true),
                "query failure retires the acquired D and producer P, leaving O");
            if !restore.restore() { return; }
            destroy_value(failed);
            drop(wire);
            release_input(overridden_query);
        }).join().unwrap();
    }
    #[test]
    fn forced_root_capture_packet_uses_existing_node_grammar() {
        std::thread::spawn(|| {
            let _inputs = TestInputScope::new();
            install_real_host_callbacks();
            for (source, expected) in [
                ("({answer: 42})", "object"),
                ("[1, 2]", "object"),
                ("(() => 7)", "function"),
                ("Symbol('forced-root-unique')", "symbol"),
                ("Symbol.for('forced-root-registered')", "symbol"),
                ("new Promise(() => {})", "object"),
            ] {
                let handle = retain_source(source);
                let decoded = decode_and_assert_type(handle, expected);
                destroy_value(decoded);
                assert_eq!(registry_state(handle), (1, true), "destroy D preserves O");
                release_input(handle);
            }
        }).join().unwrap();
    }

    #[test]
    fn forced_root_capture_preserves_identity_aliases_and_liveness() {
        std::thread::spawn(|| {
            let _inputs = TestInputScope::new();
            install_real_host_callbacks();
            let self_handle = retain_source("(() => { const v = {value: 1}; v.self = v; return v; })()");
            let first = decode_and_assert_type(self_handle, "object");
            let second = decode_and_assert_type(self_handle, "object");
            assert_eq!(unsafe { thaw_json_strict_equal(first.as_ptr(), second.as_ptr()) }, 1);
            assert_eq!(unsafe { thaw_json_borrowed_handle_id(first.as_ptr()) }, self_handle);
            assert_eq!(unsafe { thaw_json_borrowed_handle_id(second.as_ptr()) }, self_handle);
            assert_eq!(registry_state(self_handle), (3, true), "O plus independent D1/D2");

            let key_from_json = CString::new("fromJson").unwrap();
            unsafe { thaw_json_object_set_number(first.as_ptr(), key_from_json.as_ptr(), 7.0) };
            let observed_from_original = eval_bool(&format!(
                "globalThis.__thaw_value_handles[{}].fromJson === 7",
                self_handle - 1));
            assert!(observed_from_original, "Json mutation reaches the exact live JS root");
            assert!(eval_bool(&format!(
                "(globalThis.__thaw_value_handles[{}].self === globalThis.__thaw_value_handles[{}])",
                self_handle - 1, self_handle - 1)), "original self alias remains intact");
            assert!(eval_bool(&format!(
                "(globalThis.__thaw_value_handles[{}].fromJs = 9, true)", self_handle - 1)));
            let key_from_js = CString::new("fromJs").unwrap();
            let js_value = OwnedJson::new(unsafe { thaw_json_get(second.as_ptr(), key_from_js.as_ptr()) });
            let first_value = OwnedJson::new(unsafe { thaw_json_get(first.as_ptr(), key_from_js.as_ptr()) });
            assert_eq!(unsafe { thaw_json_strict_equal(js_value.as_ptr(), first_value.as_ptr()) }, 1,
                "original JS mutation is visible through both Json Host aliases");
            destroy_value(js_value);
            destroy_value(first_value);

            let object_a = retain_source("({same: 1})");
            let object_b = retain_source("({same: 1})");
            let function_a = retain_source("(() => 1)");
            let function_b = retain_source("(() => 1)");
            let symbol_a = retain_source("Symbol('same description')");
            let symbol_b = retain_source("Symbol('same description')");
            let registered_a = retain_source("Symbol.for('forced-root-key')");
            let registered_b = retain_source("Symbol.for('forced-root-key')");
            let promise_a = retain_source("new Promise(() => {})");
            let promise_b = retain_source("new Promise(() => {})");
            let decoded = [object_a, object_b, function_a, function_b, symbol_a, symbol_b,
                registered_a, registered_b, promise_a, promise_b]
                .into_iter().map(|handle| decode_and_assert_type(handle,
                    if handle == function_a || handle == function_b { "function" }
                    else if handle == symbol_a || handle == symbol_b
                        || handle == registered_a || handle == registered_b { "symbol" }
                    else { "object" }))
                .collect::<Vec<_>>();
            for pair in [(0, 1), (2, 3), (4, 5), (8, 9)] {
                assert_eq!(unsafe { thaw_json_strict_equal(decoded[pair.0].as_ptr(), decoded[pair.1].as_ptr()) }, 0,
                    "distinct objects/functions/symbols/promises stay distinct");
            }
            assert_eq!(unsafe { thaw_json_strict_equal(decoded[6].as_ptr(), decoded[7].as_ptr()) }, 1,
                "repeated Symbol.for values preserve registered identity");

            let repeated_promise = retain_source("new Promise(() => {})");
            let promise_alias_1 = decode_and_assert_type(repeated_promise, "object");
            let promise_alias_2 = decode_and_assert_type(repeated_promise, "object");
            assert_eq!(unsafe { thaw_json_strict_equal(promise_alias_1.as_ptr(), promise_alias_2.as_ptr()) }, 1,
                "same pending Promise remains the same object without assimilation");
            destroy_value(promise_alias_1);
            assert_eq!(registry_state(repeated_promise), (2, true));
            assert_eq!(unsafe { thaw_json_strict_equal(promise_alias_2.as_ptr(), promise_alias_2.as_ptr()) }, 1);
            destroy_value(promise_alias_2);

            for value in decoded { destroy_value(value); }
            destroy_value(first);
            assert_eq!(registry_state(self_handle), (2, true), "D2 survives D1 retirement");
            assert_eq!(unsafe { thaw_json_borrowed_handle_id(second.as_ptr()) }, self_handle);
            destroy_value(second);
            release_input(self_handle);
            for handle in [object_a, object_b, function_a, function_b, symbol_a, symbol_b,
                registered_a, registered_b, promise_a, promise_b, repeated_promise] {
                release_input(handle);
            }
        }).join().unwrap();
    }

    #[test]
    fn forced_root_capture_performs_no_original_value_hooks() {
        std::thread::spawn(|| {
            let _inputs = TestInputScope::new();
            install_real_host_callbacks();
            let object = retain_source("(() => { globalThis.__fr_hooks = {get:0,own:0,desc:0,proto:0,apply:0,ownGet:0,coerce:0,toJSON:0,then:0}; const target = {}; Object.defineProperty(target, 'x', {get(){__fr_hooks.ownGet++; return 1;}, configurable:true}); target.valueOf = target.toString = () => {__fr_hooks.coerce++; return 1;}; target.toJSON = () => {__fr_hooks.toJSON++; return {};}; target[Symbol.toPrimitive] = () => {__fr_hooks.coerce++; return 1;}; return new Proxy(target, {get(t,k,r){__fr_hooks.get++; return Reflect.get(t,k,r);}, ownKeys(t){__fr_hooks.own++; return Reflect.ownKeys(t);}, getOwnPropertyDescriptor(t,k){__fr_hooks.desc++; return Reflect.getOwnPropertyDescriptor(t,k);}, getPrototypeOf(t){__fr_hooks.proto++; return Reflect.getPrototypeOf(t);}}); })()");
            let decoded_object = decode_and_assert_type(object, "object");
            assert!(eval_bool("Object.values(__fr_hooks).every(n => n === 0)"),
                "capture and default typeof query do not inspect the original Proxy");
            assert!(eval_bool(&format!("globalThis.__thaw_value_handles[{}].x === 1", object - 1)));
            assert!(eval_bool("__fr_hooks.get === 1 && __fr_hooks.ownGet === 1"),
                "only the later explicit property read invokes its expected hooks");
            destroy_value(decoded_object);
            release_input(object);

            let callable = retain_source("(() => { globalThis.__fr_apply_count = 0; const f = function(){}; return new Proxy(f, {get(){__fr_apply_count++; return undefined;}, ownKeys(){__fr_apply_count++; return [];}, getOwnPropertyDescriptor(){__fr_apply_count++; return undefined;}, getPrototypeOf(){__fr_apply_count++; return null;}, apply(){__fr_apply_count++; return undefined;}}); })()");
            let decoded_callable = decode_and_assert_type(callable, "function");
            assert!(eval_bool("__fr_apply_count === 0"), "function Proxy is neither inspected nor called");
            destroy_value(decoded_callable);
            release_input(callable);

            let pending = retain_source("(() => { globalThis.__fr_then_count = 0; const p = new Promise(() => {}); Object.defineProperty(p, 'then', {configurable:true, get(){__fr_then_count++; return function(){__fr_then_count++;};}}); return p; })()");
            let decoded_pending = decode_and_assert_type(pending, "object");
            assert!(eval_bool("__fr_then_count === 0"), "pending Promise is not subscribed or assimilated");
            destroy_value(decoded_pending);
            release_input(pending);

            let revoked = retain_source("(() => { const pair = Proxy.revocable({}, {}); pair.revoke(); return pair.proxy; })()");
            let decoded_revoked = decode_and_assert_type(revoked, "object");
            assert_eq!(unsafe { thaw_json_borrowed_handle_id(decoded_revoked.as_ptr()) }, revoked);
            destroy_value(decoded_revoked);
            release_input(revoked);
        }).join().unwrap();
    }

    #[test]
    fn forced_root_packet_ignores_replaced_intrinsics_and_metadata_hooks() {
        std::thread::spawn(|| {
            let _inputs = TestInputScope::new();
            install_real_host_callbacks();
            let handle = retain_source("({x: 3})");
            let installed = eval_bool("(() => { globalThis.__fr_saved = {String:globalThis.String, stringify:JSON.stringify, push:Array.prototype.push, retain:globalThis.__thaw_retain_dynamic_value, release:globalThis.__thaw_release_dynamic_value, objectToJSON:Object.getOwnPropertyDescriptor(Object.prototype,'toJSON'), arrayToJSON:Object.getOwnPropertyDescriptor(Array.prototype,'toJSON')}; globalThis.__fr_hook_count=0; const fail=()=>{throw new Error('replaced intrinsic used');}; globalThis.String=fail; JSON.stringify=fail; Array.prototype.push=fail; globalThis.__thaw_retain_dynamic_value=fail; globalThis.__thaw_release_dynamic_value=fail; Object.defineProperty(Object.prototype,'toJSON',{configurable:true,get(){__fr_hook_count++; return fail;}}); Object.defineProperty(Array.prototype,'toJSON',{configurable:true,get(){__fr_hook_count++; return fail;}}); return true; })()");
            assert!(installed);
            let mut restore_intrinsics = EvalRestore {
                source: "(() => { const s=globalThis.__fr_saved; globalThis.String=s.String; JSON.stringify=s.stringify; Array.prototype.push=s.push; globalThis.__thaw_retain_dynamic_value=s.retain; globalThis.__thaw_release_dynamic_value=s.release; if (s.objectToJSON) Object.defineProperty(Object.prototype,'toJSON',s.objectToJSON); else delete Object.prototype.toJSON; if (s.arrayToJSON) Object.defineProperty(Array.prototype,'toJSON',s.arrayToJSON); else delete Array.prototype.toJSON; delete globalThis.__fr_saved; })()",
                armed: true,
                faulted_handles_slot: false,
            };
            let mut wire = RegisteredGraphWire::capture(handle);
            assert_forced_packet(&wire.text, handle);
            assert_eq!(registry_state(handle), (2, true), "O+P before default decode");
            let decoded = wire.decode();
            assert_eq!(unsafe { thaw_json_take_graph_error() }, 0);
            assert!(eval_bool("__fr_hook_count === 0"), "prototype JSON hooks stay untouched");
            assert_eq!(registry_state(handle), (2, true), "O+D after default decode");
            destroy_value(decoded);
            assert_eq!(registry_state(handle), (1, true));
            drop(wire);
            if !restore_intrinsics.restore() { return; }
            assert_eq!(registry_state(handle), (1, true), "restoration leaves only O");

            // The registry's canonical index is an existing own data slot.
            // A fresh empty encoder array would hit this inherited index-0
            // setter if the new producer assembled leases through Array.push.
            assert!(eval_bool(&format!("(() => {{ const a=globalThis.__thaw_value_handles; const d=Object.getOwnPropertyDescriptor(a,{}); return !!d && Object.prototype.hasOwnProperty.call(d,'value'); }})()", handle - 1)),
                "canonical registry index must already be an own data property");
            assert!(eval_bool("(() => { globalThis.__fr_index_setter_hits=0; globalThis.__fr_saved_index0_descriptor=Object.getOwnPropertyDescriptor(Array.prototype,'0'); Object.defineProperty(Array.prototype,'0',{configurable:true, set(){__fr_index_setter_hits++;}}); return true; })()"));
            let mut restore_index_setter = EvalRestore {
                source: "(() => { const d=globalThis.__fr_saved_index0_descriptor; if(d) Object.defineProperty(Array.prototype,'0',d); else delete Array.prototype[0]; delete globalThis.__fr_saved_index0_descriptor; delete globalThis.__fr_index_setter_hits; })()",
                armed: true,
                faulted_handles_slot: false,
            };
            assert_eq!(registry_state(handle), (1, true), "O before inherited-setter capture");
            let mut second = RegisteredGraphWire::capture(handle);
            assert_forced_packet(&second.text, handle);
            assert_eq!(registry_state(handle), (2, true), "O+P after setter capture");
            let second_value = second.decode();
            assert_eq!(unsafe { thaw_json_take_graph_error() }, 0);
            assert_eq!(registry_state(handle), (2, true), "O+D after setter decode");
            assert!(eval_bool("__fr_index_setter_hits === 0"), "packet assembly never writes inherited index 0");
            destroy_value(second_value);
            drop(second);
            assert_eq!(registry_state(handle), (1, true), "O remains after decoded D retirement");
            if !restore_index_setter.restore() { return; }
            release_input(handle);
            assert_eq!(registry_state(handle), (0, false), "final O retirement reaches zero");
        }).join().unwrap();
    }
}
