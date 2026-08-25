//! QuickJS-NG integration for the Fallback path (docs/design/bridge.md
//! section 7): runs arbitrary JS (e.g. an npm package's actual source, once
//! thaw-registry can fetch one) in an embedded engine, exchanging values
//! with compiled Thaw code as JSON text.
//!
//! `rquickjs` (which bundles QuickJS-NG -- the bnoordhuis/saghul fork the
//! design doc names, not Bellard's original) does the real work; this
//! crate is a thin C ABI shim around it, in the same spirit as
//! thaw-std/thaw-runtime.
//!
//! Argument/result marshaling goes through JSON both at the Rust/QuickJS
//! boundary (`thaw_js_call`'s `args_json`/return) and, one level up, at the
//! HIR/codegen boundary (`callDynamic`, see hir_codegen.rs), which
//! composes this crate's `thaw_js_call` with thaw-std's
//! `thaw_json_stringify`/`thaw_json_parse` so a `Json` value flows in and
//! out without this crate needing to know thaw-std's internal
//! representation. A Promise-returning call is driven to completion by
//! polling QuickJS's job queue (`Promise::finish`) rather than true
//! non-blocking integration -- the same "poll until resolved" shortcut V1
//! async/await already takes (docs/design/async-await.md), consistent
//! rather than a special case.
//!
//! Generated code uses `thaw_js_call_result` to route unknown functions,
//! thrown JS exceptions, malformed arguments, and Promise rejections through
//! Thaw's `try`/`catch`. The original `thaw_js_call` JSON-error-object API is
//! retained for C ABI compatibility with older callers.

use std::cell::RefCell;
use std::ffi::{CStr, CString};
use std::os::raw::c_char;
use std::time::Duration;

use rquickjs::function::Args;
use rquickjs::{Array, Context, Ctx, Function, Object, Runtime, Value};

thread_local! {
    static JS: RefCell<Option<(Runtime, Context)>> = const { RefCell::new(None) };
}

fn to_str(ptr: *const c_char) -> String {
    unsafe { CStr::from_ptr(ptr) }
        .to_string_lossy()
        .into_owned()
}

fn with_context<R>(f: impl FnOnce(Ctx<'_>) -> R) -> R {
    JS.with(|cell| {
        let mut slot = cell.borrow_mut();
        let (_, context) = slot.get_or_insert_with(|| {
            let runtime = Runtime::new().expect("failed to create a QuickJS runtime");
            let context = Context::full(&runtime).expect("failed to create a QuickJS context");
            context.with(|ctx| {
                ctx.eval::<(), _>(PLATFORM_GLOBALS)
                    .expect("failed to install JavaScript platform globals");
            });
            (runtime, context)
        });
        context.with(f)
    })
}

const PLATFORM_GLOBALS: &str = r#"
(() => {
  let nextTimerId = 1;
  const timers = new Map();
  const normalizeDelay = value => {
    const number = Number(value);
    if (!Number.isFinite(number) || number < 0) return 0;
    return Math.min(Math.trunc(number), 2147483647);
  };
  const schedule = (callback, delay, repeat, args) => {
    if (typeof callback !== 'function') {
      throw new TypeError('timer callback must be a function');
    }
    const id = nextTimerId++;
    const milliseconds = normalizeDelay(delay);
    timers.set(id, { callback, args, repeat, milliseconds,
                     due: Date.now() + milliseconds });
    return id;
  };
  globalThis.setTimeout = (callback, delay = 0, ...args) =>
    schedule(callback, delay, false, args);
  globalThis.clearTimeout = id => { timers.delete(Number(id)); };
  globalThis.setInterval = (callback, delay = 0, ...args) =>
    schedule(callback, delay, true, args);
  globalThis.clearInterval = globalThis.clearTimeout;
  globalThis.setImmediate = (callback, ...args) =>
    schedule(callback, 0, false, args);
  globalThis.clearImmediate = globalThis.clearTimeout;
  globalThis.queueMicrotask = callback => {
    if (typeof callback !== 'function') {
      throw new TypeError('microtask callback must be a function');
    }
    Promise.resolve().then(callback);
  };
  const nextTick = (callback, ...args) => {
    if (typeof callback !== 'function') {
      throw new TypeError('process.nextTick callback must be a function');
    }
    queueMicrotask(() => callback(...args));
  };
  if (typeof globalThis.process === 'undefined') globalThis.process = {};
  Object.assign(globalThis.process, {
    argv: globalThis.process.argv || [],
    env: globalThis.process.env || {},
    platform: globalThis.process.platform || 'linux',
    version: globalThis.process.version || '',
    execPath: globalThis.process.execPath || '/usr/bin/node',
    config: globalThis.process.config || { variables: {} },
    versions: Object.assign({ node: '', modules: '', uv: '' },
                            globalThis.process.versions || {}),
    cwd: globalThis.process.cwd || (() => '/'),
    nextTick
  });
  globalThis.__thaw_next_timer_delay = () => {
    let due = Infinity;
    for (const timer of timers.values()) due = Math.min(due, timer.due);
    return due === Infinity ? -1 : Math.max(0, due - Date.now());
  };
  globalThis.__thaw_run_due_timers = () => {
    const now = Date.now();
    const due = [...timers.entries()]
      .filter(([, timer]) => timer.due <= now)
      .sort((a, b) => a[1].due - b[1].due || a[0] - b[0]);
    for (const [id, timer] of due) {
      if (!timers.has(id)) continue;
      if (timer.repeat) timer.due = Date.now() + timer.milliseconds;
      else timers.delete(id);
      timer.callback(...timer.args);
    }
    return due.length;
  };

  const encodeUtf8 = input => {
    const bytes = [];
    for (const character of String(input)) {
      const code = character.codePointAt(0);
      if (code <= 0x7f) bytes.push(code);
      else if (code <= 0x7ff) bytes.push(0xc0 | code >> 6, 0x80 | code & 0x3f);
      else if (code <= 0xffff) bytes.push(0xe0 | code >> 12,
        0x80 | code >> 6 & 0x3f, 0x80 | code & 0x3f);
      else bytes.push(0xf0 | code >> 18, 0x80 | code >> 12 & 0x3f,
        0x80 | code >> 6 & 0x3f, 0x80 | code & 0x3f);
    }
    return bytes;
  };
  globalThis.TextEncoder = class TextEncoder {
    get encoding() { return 'utf-8'; }
    encode(input = '') { return Uint8Array.from(encodeUtf8(input)); }
    encodeInto(input, destination) {
      if (!(destination instanceof Uint8Array)) {
        throw new TypeError('destination must be a Uint8Array');
      }
      let read = 0;
      let written = 0;
      for (const character of String(input)) {
        const bytes = encodeUtf8(character);
        if (written + bytes.length > destination.length) break;
        destination.set(bytes, written);
        written += bytes.length;
        read += character.length;
      }
      return { read, written };
    }
  };

  const replacement = '\ufffd';
  globalThis.TextDecoder = class TextDecoder {
    constructor(label = 'utf-8', options = {}) {
      const normalized = String(label).trim().toLowerCase().replace(/[_\s]/g, '-');
      if (!['utf-8', 'utf8', 'unicode-1-1-utf-8'].includes(normalized)) {
        throw new RangeError(`unsupported encoding: ${label}`);
      }
      this.fatal = Boolean(options.fatal);
      this.ignoreBOM = Boolean(options.ignoreBOM);
      this.encoding = 'utf-8';
    }
    decode(input = new Uint8Array(), options = {}) {
      if (options.stream) throw new TypeError('streaming TextDecoder is not supported');
      const bytes = input instanceof ArrayBuffer
        ? new Uint8Array(input)
        : new Uint8Array(input.buffer, input.byteOffset, input.byteLength);
      let output = '';
      let index = 0;
      const invalid = () => {
        if (this.fatal) throw new TypeError('invalid UTF-8 data');
        output += replacement;
      };
      while (index < bytes.length) {
        const first = bytes[index++];
        if (first <= 0x7f) { output += String.fromCodePoint(first); continue; }
        let length, code, minimum;
        if (first >= 0xc2 && first <= 0xdf) { length = 1; code = first & 0x1f; minimum = 0x80; }
        else if (first >= 0xe0 && first <= 0xef) { length = 2; code = first & 0x0f; minimum = 0x800; }
        else if (first >= 0xf0 && first <= 0xf4) { length = 3; code = first & 7; minimum = 0x10000; }
        else { invalid(); continue; }
        if (index + length > bytes.length) { invalid(); break; }
        let valid = true;
        for (let offset = 0; offset < length; offset++) {
          const continuation = bytes[index + offset];
          if ((continuation & 0xc0) !== 0x80) { valid = false; break; }
          code = code << 6 | continuation & 0x3f;
        }
        if (!valid || code < minimum || code > 0x10ffff ||
            (code >= 0xd800 && code <= 0xdfff)) { invalid(); continue; }
        index += length;
        output += String.fromCodePoint(code);
      }
      if (!this.ignoreBOM && output.charCodeAt(0) === 0xfeff) output = output.slice(1);
      return output;
    }
  };
})();
"#;

/// Evaluates `source` in the (per-thread) global QuickJS context. Top-level
/// function declarations become callable afterwards via `thaw_js_call`.
/// Returns `1` on success, `0` on failure (syntax error, thrown exception).
#[no_mangle]
pub extern "C" fn thaw_js_load(source: *const c_char) -> u8 {
    let source = to_str(source);
    with_context(|ctx| match load_impl(ctx, &source) {
        Ok(()) => 1,
        Err(reason) => {
            eprintln!("thaw-quickjs: failed to load script: {reason}");
            0
        }
    })
}

/// Real error message for a failed `eval`, extracted the same way
/// `call_impl` already does for a thrown exception -- without this,
/// `rquickjs::Error::Exception`'s own `Display` is just a generic
/// placeholder ("Exception generated by QuickJS"), which made a real
/// npm package's load-time failure (e.g. a top-level `require(...)` call
/// this crate's stub rejects, see thaw-bridge's `wrap_as_commonjs_module`)
/// undiagnosable.
fn load_impl(ctx: Ctx<'_>, source: &str) -> Result<(), String> {
    ctx.eval::<(), _>(source).map_err(|e| match e {
        rquickjs::Error::Exception => describe_exception(&ctx),
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
        while ctx.execute_pending_job() {}
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
    match with_context(|ctx| call_impl(ctx, &func_name, &args_json)) {
        Ok(text) => ThawResult {
            value: CString::new(text).unwrap_or_default().into_raw(),
            error: std::ptr::null(),
        },
        Err(reason) => ThawResult {
            value: std::ptr::null(),
            error: CString::new(reason).unwrap_or_default().into_raw(),
        },
    }
}

fn call_impl(ctx: Ctx<'_>, func_name: &str, args_json: &str) -> Result<String, String> {
    let target: Function = ctx
        .globals()
        .get(func_name)
        .map_err(|_| format!("no such function `{func_name}` (was it loaded via loadScript?)"))?;
    invoke_impl(ctx, target, func_name, args_json)
}

fn invoke_impl<'js>(
    ctx: Ctx<'js>,
    target: Function<'js>,
    label: &str,
    args_json: &str,
) -> Result<String, String> {
    let to_string_err = |e: rquickjs::Error| e.to_string();

    let json: Object = ctx.globals().get("JSON").map_err(to_string_err)?;
    let parse: Function = json.get("parse").map_err(to_string_err)?;

    let args_array: Array = parse
        .call((args_json,))
        .map_err(|e| format!("args_json is not a valid JSON array: {e}"))?;

    let mut call_args = Args::new_unsized(ctx.clone());
    for i in 0..args_array.len() {
        let arg: Value = args_array.get(i).map_err(to_string_err)?;
        call_args.push_arg(arg).map_err(to_string_err)?;
    }

    let result: Value = target.call_arg(call_args).map_err(|e| match e {
        rquickjs::Error::Exception => format!("`{label}` threw: {}", describe_exception(&ctx)),
        e => format!("`{label}` threw: {e}"),
    })?;

    resolve_value_impl(ctx, result, label)
}

fn resolve_value_impl<'js>(
    ctx: Ctx<'js>,
    result: Value<'js>,
    label: &str,
) -> Result<String, String> {
    let to_string_err = |e: rquickjs::Error| e.to_string();
    let json: Object = ctx.globals().get("JSON").map_err(to_string_err)?;
    let stringify: Function = json.get("stringify").map_err(to_string_err)?;

    // Always pass the result through the realm's Promise resolution
    // procedure. `Value::as_promise` only recognizes native Promise objects;
    // `Promise.resolve` also assimilates arbitrary foreign thenables, handles
    // throwing `then` accessors/calls, and obeys the first-settlement-wins
    // rule required by JavaScript.
    let assimilate: Function = ctx
        .eval("(value) => Promise.resolve(value)")
        .map_err(to_string_err)?;
    let promise: rquickjs::Promise = assimilate.call((result,)).map_err(|e| match e {
        rquickjs::Error::Exception => {
            format!(
                "`{label}` could not resolve its result: {}",
                describe_exception(&ctx)
            )
        }
        e => format!("`{label}` could not resolve its result: {e}"),
    })?;
    let result = finish_with_platform_events(&ctx, &promise).map_err(|e| match e {
        rquickjs::Error::Exception => {
            format!("`{label}`'s promise rejected: {}", describe_exception(&ctx))
        }
        e => format!("`{label}`'s promise rejected or stalled: {e}"),
    })?;

    stringify
        .call((result,))
        .map_err(|e| format!("failed to JSON-encode the result: {e}"))
}

fn finish_with_platform_events<'js>(
    ctx: &Ctx<'js>,
    promise: &rquickjs::Promise<'js>,
) -> rquickjs::Result<Value<'js>> {
    loop {
        if let Some(result) = promise.result() {
            return result;
        }
        if ctx.execute_pending_job() {
            continue;
        }

        let next_delay: Function = ctx.globals().get("__thaw_next_timer_delay")?;
        let delay: i64 = next_delay.call(())?;
        if delay < 0 {
            return Err(rquickjs::Error::WouldBlock);
        }
        if delay > 0 {
            std::thread::sleep(Duration::from_millis(delay as u64));
        }
        let run_due: Function = ctx.globals().get("__thaw_run_due_timers")?;
        run_due.call::<_, usize>(())?;
    }
}

/// Retains a global JavaScript value in the realm and returns a stable opaque
/// handle. Zero denotes failure or a missing value.
#[no_mangle]
pub extern "C" fn thaw_js_get_global(name: *const c_char) -> u64 {
    let name = to_str(name);
    with_context(|ctx| {
        let Ok(value) = ctx.globals().get::<_, Value>(name.as_str()) else {
            return 0;
        };
        let globals = ctx.globals();
        let handles: Array = match globals.get("__thaw_value_handles") {
            Ok(handles) => handles,
            Err(_) => {
                let Ok(handles) = Array::new(ctx.clone()) else {
                    return 0;
                };
                if globals
                    .set("__thaw_value_handles", handles.clone())
                    .is_err()
                {
                    return 0;
                }
                let Ok(live) = Array::new(ctx.clone()) else {
                    return 0;
                };
                if globals.set("__thaw_value_handle_live", live).is_err() {
                    return 0;
                }
                handles
            }
        };
        let index = handles.len();
        if handles.set(index, value).is_err() {
            return 0;
        }
        let Ok(live) = ctx.globals().get::<_, Array>("__thaw_value_handle_live") else {
            return 0;
        };
        if live.set(index, true).is_err() {
            return 0;
        }
        index as u64 + 1
    })
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

fn retain_value<'js>(ctx: &Ctx<'js>, value: Value<'js>) -> Result<u64, String> {
    let handles = handle_array(ctx)?;
    let index = handles.len();
    handles
        .set(index, value)
        .map_err(|error| error.to_string())?;
    let live: Array = ctx
        .globals()
        .get("__thaw_value_handle_live")
        .map_err(|_| "JavaScript value handle liveness registry is empty".to_string())?;
    live.set(index, true).map_err(|error| error.to_string())?;
    Ok(index as u64 + 1)
}

fn invoke_raw<'js>(
    ctx: Ctx<'js>,
    target: Function<'js>,
    args_json: &str,
) -> Result<Value<'js>, String> {
    let json: Object = ctx
        .globals()
        .get("JSON")
        .map_err(|error| error.to_string())?;
    let parse: Function = json.get("parse").map_err(|error| error.to_string())?;
    let args_array: Array = parse
        .call((args_json,))
        .map_err(|error| format!("args_json is not a valid JSON array: {error}"))?;
    let mut call_args = Args::new_unsized(ctx.clone());
    for index in 0..args_array.len() {
        let arg: Value = args_array.get(index).map_err(|error| error.to_string())?;
        call_args.push_arg(arg).map_err(|error| error.to_string())?;
    }
    target.call_arg(call_args).map_err(|error| match error {
        rquickjs::Error::Exception => describe_exception(&ctx),
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
) -> Result<Value<'js>, String> {
    let json: Object = ctx
        .globals()
        .get("JSON")
        .map_err(|error| error.to_string())?;
    let parse: Function = json.get("parse").map_err(|error| error.to_string())?;
    let arguments: Array = parse
        .call((args_json,))
        .map_err(|error| error.to_string())?;
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
    target.call_arg(call_args).map_err(|error| match error {
        rquickjs::Error::Exception => describe_exception(&ctx),
        error => error.to_string(),
    })
}

#[no_mangle]
/// Calls a retained function with JSON arguments followed by retained values.
///
/// # Safety
///
/// `handles` must point to a live Thaw array buffer containing an initial
/// `u64` length followed by that many aligned `u64` handle identifiers.
pub unsafe extern "C" fn thaw_js_call_handle_mixed_result(
    handle: u64,
    args_json: *const c_char,
    handles: *const u8,
) -> ThawResult {
    let args_json = to_str(args_json);
    let result = with_context(|ctx| {
        let target = Function::from_value(value_for_handle(&ctx, handle)?)
            .map_err(|_| format!("JavaScript value handle {handle} is not callable"))?;
        let value = unsafe { invoke_mixed(ctx.clone(), target, &args_json, handles)? };
        resolve_value_impl(ctx, value, &format!("JavaScript value #{handle}"))
    });
    match result {
        Ok(value) => ThawResult {
            value: CString::new(value).unwrap_or_default().into_raw(),
            error: std::ptr::null(),
        },
        Err(error) => ThawResult {
            value: std::ptr::null(),
            error: CString::new(error).unwrap_or_default().into_raw(),
        },
    }
}

#[no_mangle]
pub extern "C" fn thaw_js_construct_handle_result(
    handle: u64,
    args_json: *const c_char,
) -> ThawHandleResult {
    let args_json = to_str(args_json);
    match with_context(|ctx| {
        let constructor = value_for_handle(&ctx, handle)?
            .into_constructor()
            .ok_or_else(|| format!("JavaScript value handle {handle} is not a constructor"))?;
        let json: Object = ctx
            .globals()
            .get("JSON")
            .map_err(|error| error.to_string())?;
        let parse: Function = json.get("parse").map_err(|error| error.to_string())?;
        let arguments: Array = parse
            .call((args_json,))
            .map_err(|error| error.to_string())?;
        let mut args = Args::new_unsized(ctx.clone());
        for index in 0..arguments.len() {
            args.push_arg(
                arguments
                    .get::<Value>(index)
                    .map_err(|error| error.to_string())?,
            )
            .map_err(|error| error.to_string())?;
        }
        let value: Value = constructor
            .construct_args(args)
            .map_err(|error| match error {
                rquickjs::Error::Exception => describe_exception(&ctx),
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
            error: CString::new(error).unwrap_or_default().into_raw(),
        },
    }
}

#[no_mangle]
pub extern "C" fn thaw_js_call_handle_handle_result(
    handle: u64,
    args_json: *const c_char,
) -> ThawHandleResult {
    let args_json = to_str(args_json);
    let result: Result<u64, String> = with_context(|ctx| {
        let target = Function::from_value(value_for_handle(&ctx, handle)?)
            .map_err(|_| format!("JavaScript value handle {handle} is not callable"))?;
        let result = invoke_raw(ctx.clone(), target, &args_json)?;
        retain_value(&ctx, result)
    });
    match result {
        Ok(value) => ThawHandleResult {
            value,
            error: std::ptr::null(),
        },
        Err(error) => ThawHandleResult {
            value: 0,
            error: CString::new(error).unwrap_or_default().into_raw(),
        },
    }
}

#[no_mangle]
pub extern "C" fn thaw_js_call_handle_value_result(handle: u64, argument: u64) -> ThawResult {
    let result = with_context(|ctx| {
        let target = Function::from_value(value_for_handle(&ctx, handle)?)
            .map_err(|_| format!("JavaScript value handle {handle} is not callable"))?;
        let argument = value_for_handle(&ctx, argument)?;
        let value: Value = target.call((argument,)).map_err(|error| match error {
            rquickjs::Error::Exception => describe_exception(&ctx),
            error => error.to_string(),
        })?;
        resolve_value_impl(ctx, value, &format!("JavaScript value #{handle}"))
    });
    match result {
        Ok(text) => ThawResult {
            value: CString::new(text).unwrap_or_default().into_raw(),
            error: std::ptr::null(),
        },
        Err(error) => ThawResult {
            value: std::ptr::null(),
            error: CString::new(error).unwrap_or_default().into_raw(),
        },
    }
}

#[no_mangle]
pub extern "C" fn thaw_js_release_handle(handle: u64) -> u8 {
    with_context(|ctx| {
        if handle == 0 {
            return 0;
        }
        let Ok(handles) = handle_array(&ctx) else {
            return 0;
        };
        let index = (handle - 1) as usize;
        let Ok(live) = ctx.globals().get::<_, Array>("__thaw_value_handle_live") else {
            return 0;
        };
        if !live.get::<bool>(index).unwrap_or(false) {
            return 0;
        }
        if live.set(index, false).is_err() {
            return 0;
        }
        u8::from(handles.set(index, Value::new_undefined(ctx)).is_ok())
    })
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
        if ctx.globals().set("__thaw_value_handles", handles).is_err()
            || ctx.globals().set("__thaw_value_handle_live", live).is_err()
        {
            return 0;
        }
        count as u64
    })
}

#[no_mangle]
pub extern "C" fn thaw_js_get_property_result(
    handle: u64,
    name: *const c_char,
) -> ThawHandleResult {
    let name = to_str(name);
    let result: Result<u64, String> = with_context(|ctx| {
        let object = value_for_handle(&ctx, handle)?
            .into_object()
            .ok_or_else(|| format!("JavaScript value handle {handle} is not an object"))?;
        let value = object.get(name.as_str()).map_err(|error| match error {
            rquickjs::Error::Exception => describe_exception(&ctx),
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
            error: CString::new(error).unwrap_or_default().into_raw(),
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
    let result: Result<u64, String> = with_context(|ctx| {
        let object = value_for_handle(&ctx, handle)?
            .into_object()
            .ok_or_else(|| format!("JavaScript value handle {handle} is not an object"))?;
        let value = value_for_handle(&ctx, value_handle)?;
        object
            .set(name.as_str(), value)
            .map_err(|error| match error {
                rquickjs::Error::Exception => describe_exception(&ctx),
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
            error: CString::new(error).unwrap_or_default().into_raw(),
        },
    }
}

#[no_mangle]
pub extern "C" fn thaw_js_call_method_result(
    handle: u64,
    name: *const c_char,
    args_json: *const c_char,
) -> ThawResult {
    let name = to_str(name);
    let args_json = to_str(args_json);
    let result = with_context(|ctx| {
        let object = value_for_handle(&ctx, handle)?
            .into_object()
            .ok_or_else(|| format!("JavaScript value handle {handle} is not an object"))?;
        let method: Function = object.get(name.as_str()).map_err(|error| match error {
            rquickjs::Error::Exception => describe_exception(&ctx),
            error => error.to_string(),
        })?;
        let json: Object = ctx
            .globals()
            .get("JSON")
            .map_err(|error| error.to_string())?;
        let parse: Function = json.get("parse").map_err(|error| error.to_string())?;
        let arguments: Array = parse
            .call((args_json.as_str(),))
            .map_err(|error| error.to_string())?;
        let mut call_args = Args::new_unsized(ctx.clone());
        call_args.this(object).map_err(|error| error.to_string())?;
        for index in 0..arguments.len() {
            let argument: Value = arguments.get(index).map_err(|error| error.to_string())?;
            call_args
                .push_arg(argument)
                .map_err(|error| error.to_string())?;
        }
        let value: Value = method.call_arg(call_args).map_err(|error| match error {
            rquickjs::Error::Exception => describe_exception(&ctx),
            error => error.to_string(),
        })?;
        resolve_value_impl(ctx, value, &format!("JavaScript method `{name}`"))
    });
    match result {
        Ok(value) => ThawResult {
            value: CString::new(value).unwrap_or_default().into_raw(),
            error: std::ptr::null(),
        },
        Err(error) => ThawResult {
            value: std::ptr::null(),
            error: CString::new(error).unwrap_or_default().into_raw(),
        },
    }
}

#[no_mangle]
pub extern "C" fn thaw_js_resolve_handle_result(handle: u64) -> ThawResult {
    let result = with_context(|ctx| {
        let value = value_for_handle(&ctx, handle)?;
        resolve_value_impl(ctx, value, &format!("JavaScript value #{handle}"))
    });
    match result {
        Ok(value) => ThawResult {
            value: CString::new(value).unwrap_or_default().into_raw(),
            error: std::ptr::null(),
        },
        Err(error) => ThawResult {
            value: std::ptr::null(),
            error: CString::new(error).unwrap_or_default().into_raw(),
        },
    }
}

/// Calls a callable value retained by [`thaw_js_get_global`]. Arguments and
/// results use the existing JSON bridge while the callable itself preserves
/// identity and closures inside QuickJS.
#[no_mangle]
pub extern "C" fn thaw_js_call_handle_result(handle: u64, args_json: *const c_char) -> ThawResult {
    let args_json = to_str(args_json);
    let result = with_context(|ctx| {
        let target = Function::from_value(value_for_handle(&ctx, handle)?)
            .map_err(|_| format!("JavaScript value handle {handle} is not callable"))?;
        invoke_impl(
            ctx,
            target,
            &format!("JavaScript value #{handle}"),
            &args_json,
        )
    });
    match result {
        Ok(text) => ThawResult {
            value: CString::new(text).unwrap_or_default().into_raw(),
            error: std::ptr::null(),
        },
        Err(reason) => ThawResult {
            value: std::ptr::null(),
            error: CString::new(reason).unwrap_or_default().into_raw(),
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

#[cfg(test)]
#[allow(clippy::items_after_test_module)]
mod tests {
    use super::*;

    fn call(func_name: &str, args_json: &str) -> String {
        let func_name = CString::new(func_name).unwrap();
        let args_json = CString::new(args_json).unwrap();
        let result_ptr = thaw_js_call(func_name.as_ptr(), args_json.as_ptr());
        unsafe { CStr::from_ptr(result_ptr) }
            .to_string_lossy()
            .into_owned()
    }

    fn load(source: &str) -> u8 {
        let source = CString::new(source).unwrap();
        thaw_js_load(source.as_ptr())
    }

    #[test]
    fn loads_and_calls_a_simple_function() {
        assert_eq!(load("function add(a, b) { return a + b; }"), 1);
        assert_eq!(call("add", "[2, 3]"), "5");
    }

    #[test]
    fn round_trips_objects_and_arrays() {
        assert_eq!(load("function identity(x) { return x; }"), 1);
        assert_eq!(
            call("identity", r#"[{"a": 1, "b": [true, "x"]}]"#),
            r#"{"a":1,"b":[true,"x"]}"#
        );
    }

    #[test]
    fn resolves_a_returned_promise() {
        assert_eq!(
            load("function later(x) { return Promise.resolve(x * 2); }"),
            1
        );
        assert_eq!(call("later", "[21]"), "42");
    }

    #[test]
    fn timeout_resolves_promises_and_forwards_arguments() {
        assert_eq!(
            load(
                "function timed(value) { return new Promise(resolve => {\n\
                   setTimeout((left, right) => resolve(left + right), 2, value, 2);\n\
                 }); }"
            ),
            1
        );
        assert_eq!(call("timed", "[40]"), "42");
    }

    #[test]
    fn timeout_can_be_cancelled() {
        assert_eq!(
            load(
                "function cancelled() { return new Promise(resolve => {\n\
                   const id = setTimeout(() => resolve('wrong'), 0);\n\
                   clearTimeout(id);\n\
                   setTimeout(() => resolve('right'), 1);\n\
                 }); }"
            ),
            1
        );
        assert_eq!(call("cancelled", "[]"), r#""right""#);
    }

    #[test]
    fn immediate_forwards_arguments_and_can_be_cancelled() {
        assert_eq!(
            load(
                "function immediate(value) { return new Promise(resolve => {\n\
                   const cancelled = setImmediate(() => resolve('wrong'));\n\
                   clearImmediate(cancelled);\n\
                   setImmediate((left, right) => resolve(left + right), value, 2);\n\
                 }); }"
            ),
            1
        );
        assert_eq!(call("immediate", "[40]"), "42");
    }

    #[test]
    fn interval_repeats_and_can_cancel_itself() {
        assert_eq!(
            load(
                "function countIntervals() { return new Promise(resolve => {\n\
                   let count = 0;\n\
                   const id = setInterval(() => {\n\
                     if (++count === 3) { clearInterval(id); resolve(count); }\n\
                   }, 1);\n\
                 }); }"
            ),
            1
        );
        assert_eq!(call("countIntervals", "[]"), "3");
    }

    #[test]
    fn microtasks_run_before_zero_delay_timers() {
        assert_eq!(
            load(
                "function eventOrder() { return new Promise(resolve => {\n\
                   const events = [];\n\
                   setTimeout(() => { events.push('timer'); resolve(events); }, 0);\n\
                   queueMicrotask(() => events.push('microtask'));\n\
                   events.push('sync');\n\
                 }); }"
            ),
            1
        );
        assert_eq!(call("eventOrder", "[]"), r#"["sync","microtask","timer"]"#);
    }

    #[test]
    fn process_next_tick_is_async_and_forwards_arguments() {
        assert_eq!(
            load(
                "function tickOrder() { return new Promise(resolve => {\n\
                   const events = ['sync'];\n\
                   process.nextTick((left, right) => { events.push(left + right); resolve(events); }, 'next', 'Tick');\n\
                   events.push('after');\n\
                 }); }"
            ),
            1
        );
        assert_eq!(call("tickOrder", "[]"), r#"["sync","after","nextTick"]"#);
    }

    #[test]
    fn text_encoder_and_decoder_support_utf8() {
        assert_eq!(
            load(
                "function utf8RoundTrip(value) {\n\
                   const encoder = new TextEncoder();\n\
                   const encoded = encoder.encode(value);\n\
                   return { bytes: Array.from(encoded), text: new TextDecoder().decode(encoded) };\n\
                 }\n\
                 function encodeInto() {\n\
                   const output = new Uint8Array(5);\n\
                   const result = new TextEncoder().encodeInto('A😀B', output);\n\
                   return { result, bytes: Array.from(output) };\n\
                 }\n\
                 function decodeInvalid() {\n\
                   return new TextDecoder().decode(Uint8Array.from([0x61, 0xff, 0x62]));\n\
                 }"
            ),
            1
        );
        assert_eq!(
            call("utf8RoundTrip", r#"["A😀"]"#),
            r#"{"bytes":[65,240,159,152,128],"text":"A😀"}"#
        );
        assert_eq!(
            call("encodeInto", "[]"),
            r#"{"result":{"read":3,"written":5},"bytes":[65,240,159,152,128]}"#
        );
        assert_eq!(call("decodeInvalid", "[]"), r#""a�b""#);
    }

    #[test]
    fn retains_and_calls_a_callable_javascript_value() {
        assert_eq!(
            load("globalThis.times = factor => value => Promise.resolve(value * factor); globalThis.twice = times(2);"),
            1
        );
        let name = CString::new("twice").unwrap();
        let handle = thaw_js_get_global(name.as_ptr());
        assert_ne!(handle, 0);
        let args = CString::new("[21]").unwrap();
        let result = thaw_js_call_handle_result(handle, args.as_ptr());
        assert!(result.error.is_null());
        assert_eq!(
            unsafe { CStr::from_ptr(result.value) }.to_str().unwrap(),
            "42"
        );
    }

    #[test]
    fn assimilates_foreign_thenables_once_and_reports_then_errors() {
        assert_eq!(
            load(
                "function thenable() {\n\
                   return { then(resolve, reject) { resolve(42); reject('late'); resolve(99); } };\n\
                 }\n\
                 function throwingThen() {\n\
                   return Object.create(null, { then: { get() { throw new Error('bad then'); } } });\n\
                 }"
            ),
            1
        );
        assert_eq!(call("thenable", "[]"), "42");
        let failed = call("throwingThen", "[]");
        assert!(failed.contains("__thaw_error__"), "{failed}");
        assert!(failed.contains("bad then"), "{failed}");
    }

    #[test]
    fn unknown_function_reports_an_error_object_instead_of_crashing() {
        let result = call("doesNotExist", "[]");
        assert!(
            result.contains("__thaw_error__"),
            "unexpected result: {result}"
        );
    }

    #[test]
    fn thrown_exception_reports_an_error_object_instead_of_crashing() {
        assert_eq!(load("function boom() { throw new Error('kaboom'); }"), 1);
        let result = call("boom", "[]");
        assert!(
            result.contains("__thaw_error__"),
            "unexpected result: {result}"
        );
        assert!(result.contains("kaboom"), "unexpected result: {result}");
    }

    #[test]
    fn result_abi_separates_success_from_javascript_exceptions() {
        assert_eq!(
            load("function ok() { return 42; } function boom() { throw new Error('kaboom'); }"),
            1
        );
        let ok_name = CString::new("ok").unwrap();
        let boom_name = CString::new("boom").unwrap();
        let args = CString::new("[]").unwrap();
        let ok = thaw_js_call_result(ok_name.as_ptr(), args.as_ptr());
        assert!(ok.error.is_null());
        assert_eq!(unsafe { CStr::from_ptr(ok.value) }.to_str().unwrap(), "42");

        let failed = thaw_js_call_result(boom_name.as_ptr(), args.as_ptr());
        assert!(failed.value.is_null());
        let error = unsafe { CStr::from_ptr(failed.error) }.to_string_lossy();
        assert!(error.contains("kaboom"), "{error}");
    }

    #[test]
    fn syntax_error_fails_to_load_instead_of_crashing() {
        assert_eq!(load("function( this is not valid js"), 0);
    }

    /// Regression test for the pre-fix behavior: a thrown exception during
    /// `loadScript` used to report only `rquickjs::Error::Exception`'s
    /// generic placeholder message on stderr, not the real thrown message
    /// -- undiagnosable for e.g. a real npm package's top-level
    /// `require(...)` call. `load_impl` is the testable inner function
    /// (`thaw_js_load` itself only returns 0/1, with the message going to
    /// stderr).
    #[test]
    fn load_failure_reports_the_real_thrown_message() {
        with_context(|ctx| {
            let err = load_impl(ctx, "throw new Error('boom');").unwrap_err();
            assert_eq!(err, "boom");
        });
    }
}
/// Common C ABI result for external operations that can either return a
/// pointer-shaped value or throw. Both pointers are owned by the callee for
/// the current request; exactly one is non-null.
#[repr(C)]
pub struct ThawResult {
    pub value: *const c_char,
    pub error: *const c_char,
}

#[repr(C)]
pub struct ThawHandleResult {
    pub value: u64,
    pub error: *const c_char,
}
