// `node:vm` contexts: every contextified sandbox owns an independent QuickJS context (own intrinsics, own global
// lexical environment) inside the runtime, so `var`/`function`/`let`/`const`/`class` persist across runs, unresolved
// identifiers throw ReferenceError, and `timeout` is enforced by the engine's interrupt handler.
//
// The sandbox object is mirrored onto that context's global object (see `VM_CONTEXT_HELPER`): sandbox keys appear on
// the global as live accessors, and globals created by a script are copied back to the sandbox after each run.

use std::time::Instant;

const VM_CONTEXT_HELPER: &str = r#"(function () {
  'use strict';
  var g = globalThis;
  var defineProperty = Object.defineProperty, getDescriptor = Object.getOwnPropertyDescriptor;
  var ownKeys = Reflect.ownKeys, deleteProperty = Reflect.deleteProperty, hasOwn = Object.prototype.hasOwnProperty;
  var noop = function () {};
  var consoleObject = {};
  ['debug', 'error', 'info', 'log', 'warn', 'dir', 'dirxml', 'table', 'trace', 'group', 'groupCollapsed', 'groupEnd',
   'clear', 'count', 'countReset', 'assert', 'profile', 'profileEnd', 'time', 'timeLog', 'timeEnd', 'timeStamp', 'context']
    .forEach(function (name) { defineProperty(consoleObject, name, { value: noop, writable: true, configurable: true, enumerable: true }); });
  defineProperty(g, 'console', { value: consoleObject, writable: true, configurable: true, enumerable: false });
  var initial = new Set(ownKeys(g));
  var tracked = new Map(); // key -> { live: a global accessor mirrors the sandbox, original: the descriptor it replaced }
  function mirror(sandbox, key, enumerable) {
    defineProperty(g, key, {
      get: function () { return sandbox[key]; },
      set: function (value) { Reflect.set(sandbox, key, value); },
      enumerable: enumerable,
      configurable: true,
    });
  }
  function expose(sandbox, key, enumerable) {
    defineProperty(sandbox, key, { get: function () { return g[key]; }, set: function (value) { g[key] = value; }, enumerable: enumerable, configurable: true });
  }
  return {
    pre: function (sandbox, stop) {
      // Own and inherited keys (up to the host's Object.prototype), as V8's interceptors see `sandbox[key]`.
      var keys = [], enumerables = new Map();
      for (var holder = sandbox; holder !== null && holder !== undefined && holder !== stop; holder = Object.getPrototypeOf(holder)) {
        var holderKeys = ownKeys(holder);
        for (var h = 0; h < holderKeys.length; h++) {
          if (enumerables.has(holderKeys[h])) continue;
          keys.push(holderKeys[h]);
          enumerables.set(holderKeys[h], getDescriptor(holder, holderKeys[h]).enumerable);
        }
      }
      var present = new Set(keys);
      tracked.forEach(function (entry, key) {
        if (present.has(key) || !entry.live) return;
        if (entry.original) defineProperty(g, key, entry.original); else deleteProperty(g, key);
        tracked.delete(key);
      });
      for (var i = 0; i < keys.length; i++) {
        var key = keys[i], entry = tracked.get(key), own = getDescriptor(g, key);
        if (entry === undefined) {
          if (own && !own.configurable) { tracked.set(key, { live: false }); try { g[key] = sandbox[key]; } catch (e) {} }
          else { tracked.set(key, { live: true, original: own }); mirror(sandbox, key, enumerables.get(key)); }
        } else if (!entry.live) {
          try { if (g[key] !== sandbox[key]) g[key] = sandbox[key]; } catch (e) {}
        }
      }
    },
    post: function (sandbox) {
      tracked.forEach(function (entry, key) {
        if (entry.live && !hasOwn.call(g, key)) { deleteProperty(sandbox, key); tracked.delete(key); }
      });
      var keys = ownKeys(g);
      for (var i = 0; i < keys.length; i++) {
        var key = keys[i];
        if (initial.has(key) || tracked.has(key)) continue;
        var descriptor = getDescriptor(g, key);
        if (descriptor === undefined) continue;
        if (descriptor.configurable) {
          // Implicit globals and `globalThis.x = ...`: the sandbox owns the value, the global mirrors it.
          if ('value' in descriptor) defineProperty(sandbox, key, { value: descriptor.value, writable: true, enumerable: descriptor.enumerable, configurable: true });
          else defineProperty(sandbox, key, { get: descriptor.get, set: descriptor.set, enumerable: descriptor.enumerable, configurable: true });
          tracked.set(key, { live: true });
          mirror(sandbox, key, descriptor.enumerable);
        } else {
          // `var` / function declarations cannot be redirected; the sandbox reads and writes the global's slot live.
          expose(sandbox, key, descriptor.enumerable);
          tracked.set(key, { live: false });
        }
      }
    },
  };
})()"#;

/// Intrinsics a Node context has but QuickJS does not create per context; they are shared with the host realm.
const VM_SHARED_GLOBALS: [&std::ffi::CStr; 4] = [c"Intl", c"WebAssembly", c"SharedArrayBuffer", c"Atomics"];

struct VmContext {
    ctx: *mut rquickjs::qjs::JSContext,
    pre: rquickjs::qjs::JSValue,
    post: rquickjs::qjs::JSValue,
}

impl Drop for VmContext {
    fn drop(&mut self) {
        // SAFETY: owned context/values created by `host_vm_create`; dropped with the runtime's userdata (before
        // JS_FreeRuntime) or by a finalizer while the runtime is alive.
        unsafe {
            rquickjs::qjs::JS_FreeValue(self.ctx, self.pre);
            rquickjs::qjs::JS_FreeValue(self.ctx, self.post);
            rquickjs::qjs::JS_FreeContext(self.ctx);
        }
    }
}

/// Runtime-scoped registry of vm contexts. Userdata, so the contexts are freed before the runtime.
#[derive(Default)]
struct VmContexts {
    next: Cell<u32>,
    contexts: RefCell<HashMap<u32, VmContext>>,
}

unsafe impl<'js> rquickjs::JsLifetime<'js> for VmContexts {
    type Changed<'to> = VmContexts;
}

thread_local! {
    /// Deadlines of the vm runs currently on this thread's stack; the interrupt handler fires once any has passed.
    static VM_DEADLINES: RefCell<Vec<Instant>> = const { RefCell::new(Vec::new()) };
    static VM_INTERRUPTED: Cell<bool> = const { Cell::new(false) };
}

unsafe extern "C" fn vm_interrupt_handler(_runtime: *mut rquickjs::qjs::JSRuntime, _opaque: *mut std::ffi::c_void) -> i32 {
    let now = Instant::now();
    let expired = VM_DEADLINES.try_with(|deadlines| deadlines.borrow().iter().any(|deadline| now >= *deadline)).unwrap_or(false);
    if expired {
        let _ = VM_INTERRUPTED.try_with(|flag| flag.set(true));
    }
    i32::from(expired)
}

fn vm_throw<'js>(ctx: &Ctx<'js>, raw: rquickjs::qjs::JSValue) -> rquickjs::Error {
    // SAFETY: `raw` is an owned exception taken with JS_GetException; ownership moves into the Value (same runtime).
    let value = unsafe { Value::from_raw(ctx.clone(), raw) };
    ctx.throw(value)
}

fn host_vm_create(ctx: Ctx<'_>) -> rquickjs::Result<i32> {
    let owner = ctx
        .userdata::<VmContexts>()
        .ok_or_else(|| rquickjs::Error::new_from_js_message("vm", "context", "vm contexts are unavailable"))?;
    // SAFETY: the calling context is live; the new context is owned by the `VmContext` stored below.
    unsafe {
        let main = ctx.as_raw().as_ptr();
        let created = rquickjs::qjs::JS_NewContext(rquickjs::qjs::JS_GetRuntime(main));
        if created.is_null() {
            return Err(rquickjs::Error::Allocation);
        }
        let host_global = rquickjs::qjs::JS_GetGlobalObject(main);
        let new_global = rquickjs::qjs::JS_GetGlobalObject(created);
        for name in VM_SHARED_GLOBALS {
            let existing = rquickjs::qjs::JS_GetPropertyStr(created, new_global, name.as_ptr());
            let missing = rquickjs::qjs::JS_IsUndefined(existing);
            rquickjs::qjs::JS_FreeValue(created, existing);
            if !missing {
                continue;
            }
            let value = rquickjs::qjs::JS_GetPropertyStr(main, host_global, name.as_ptr());
            if rquickjs::qjs::JS_IsUndefined(value) {
                continue;
            }
            // Consumes `value`.
            rquickjs::qjs::JS_DefinePropertyValueStr(
                created,
                new_global,
                name.as_ptr(),
                value,
                (rquickjs::qjs::JS_PROP_WRITABLE | rquickjs::qjs::JS_PROP_CONFIGURABLE) as i32,
            );
        }
        rquickjs::qjs::JS_FreeValue(main, host_global);
        rquickjs::qjs::JS_FreeValue(created, new_global);

        let source = std::ffi::CString::new(VM_CONTEXT_HELPER).unwrap_or_default();
        let helper = rquickjs::qjs::JS_Eval(
            created,
            source.as_ptr(),
            source.as_bytes().len() as _,
            c"<thaw-vm-context>".as_ptr(),
            rquickjs::qjs::JS_EVAL_TYPE_GLOBAL as i32,
        );
        if rquickjs::qjs::JS_IsException(helper) {
            rquickjs::qjs::JS_FreeValue(created, rquickjs::qjs::JS_GetException(created));
            rquickjs::qjs::JS_FreeContext(created);
            return Err(rquickjs::Error::Unknown);
        }
        let pre = rquickjs::qjs::JS_GetPropertyStr(created, helper, c"pre".as_ptr());
        let post = rquickjs::qjs::JS_GetPropertyStr(created, helper, c"post".as_ptr());
        rquickjs::qjs::JS_FreeValue(created, helper);
        let id = owner.next.get().wrapping_add(1);
        owner.next.set(id);
        owner.contexts.borrow_mut().insert(id, VmContext { ctx: created, pre, post });
        Ok(id as i32)
    }
}

fn host_vm_free(ctx: Ctx<'_>, id: i32) {
    // Remove first, drop after the registry borrow ends.
    let removed = ctx.userdata::<VmContexts>().and_then(|owner| owner.contexts.borrow_mut().remove(&(id as u32)));
    drop(removed);
}

/// Runs `source` as a classic script in vm context `id` (the calling realm when `id < 0`), mirroring `sandbox`.
/// A script exception is rethrown unchanged; when `timeout_ms > 0` elapses first, an `ERR_SCRIPT_EXECUTION_TIMEOUT`
/// Error created in the target realm (as in Node, it is not an instance of the caller's `Error`) is thrown.
fn host_vm_run<'js>(
    ctx: Ctx<'js>,
    id: i32,
    source: String,
    filename: String,
    timeout_ms: f64,
    sandbox: Value<'js>,
    stop: Value<'js>,
) -> rquickjs::Result<Value<'js>> {
    use rquickjs::qjs;
    let source = std::ffi::CString::new(source)
        .map_err(|_| rquickjs::Error::new_from_js_message("source", "script", "source contains a NUL byte"))?;
    let filename = std::ffi::CString::new(filename.replace('\0', "")).unwrap_or_default();
    let main = ctx.as_raw().as_ptr();
    // (target context, owned references to its pre/post helpers)
    let (target, helpers) = if id < 0 {
        (main, None)
    } else {
        let owner = ctx
            .userdata::<VmContexts>()
            .ok_or_else(|| rquickjs::Error::new_from_js_message("vm", "context", "vm contexts are unavailable"))?;
        let contexts = owner.contexts.borrow();
        let entry = contexts
            .get(&(id as u32))
            .ok_or_else(|| rquickjs::Error::new_from_js_message("vm", "context", "the vm context was released"))?;
        // SAFETY: the helpers are owned by the registry entry; the dup'd references keep them alive for this call
        // even if a script causes the entry to be released.
        let dup = unsafe { (qjs::JS_DupValue(entry.ctx, entry.pre), qjs::JS_DupValue(entry.ctx, entry.post)) };
        (entry.ctx, Some(dup))
    };
    // SAFETY: every pointer belongs to the live runtime of `ctx`; every owned JSValue is freed or moved into a Value.
    unsafe {
        let runtime = qjs::JS_GetRuntime(main);
        let mirror = |helper: qjs::JSValue| -> qjs::JSValue {
            let mut arguments = [sandbox.as_raw(), stop.as_raw()];
            let value = qjs::JS_Call(target, helper, qjs::JS_UNDEFINED, 2, arguments.as_mut_ptr());
            qjs::JS_FreeValue(target, helper);
            value
        };
        let mut post_helper = None;
        if let Some((pre, post)) = helpers {
            let value = mirror(pre);
            if qjs::JS_IsException(value) {
                qjs::JS_FreeValue(target, post);
                return Err(vm_throw(&ctx, qjs::JS_GetException(target)));
            }
            qjs::JS_FreeValue(target, value);
            post_helper = Some(post);
        }
        let deadline = (timeout_ms > 0.0).then(|| Instant::now() + Duration::from_secs_f64(timeout_ms / 1000.0));
        if let Some(deadline) = deadline {
            VM_DEADLINES.with(|deadlines| deadlines.borrow_mut().push(deadline));
            qjs::JS_SetInterruptHandler(runtime, Some(vm_interrupt_handler), std::ptr::null_mut());
        }
        let value = qjs::JS_Eval(target, source.as_ptr(), source.as_bytes().len() as _, filename.as_ptr(), qjs::JS_EVAL_TYPE_GLOBAL as i32);
        let failed = qjs::JS_IsException(value);
        let mut timed_out = false;
        if let Some(deadline) = deadline {
            let remaining = VM_DEADLINES.with(|deadlines| {
                let mut deadlines = deadlines.borrow_mut();
                deadlines.pop();
                deadlines.len()
            });
            if remaining == 0 {
                qjs::JS_SetInterruptHandler(runtime, None, std::ptr::null_mut());
            }
            if failed && Instant::now() >= deadline && VM_INTERRUPTED.with(Cell::get) {
                VM_INTERRUPTED.with(|flag| flag.set(false));
                timed_out = true;
            }
        }
        let exception = failed.then(|| qjs::JS_GetException(target));
        if let Some(post) = post_helper {
            // Mirroring failures must not mask the script's own outcome.
            let mirrored = mirror(post);
            if qjs::JS_IsException(mirrored) {
                qjs::JS_FreeValue(target, qjs::JS_GetException(target));
            } else {
                qjs::JS_FreeValue(target, mirrored);
            }
        }
        if let Some(exception) = exception {
            if timed_out {
                qjs::JS_FreeValue(target, exception);
                let error = qjs::JS_NewError(target);
                let message = format!("Script execution timed out after {}ms", timeout_ms as u64);
                let message = qjs::JS_NewStringLen(target, message.as_ptr().cast(), message.len() as _);
                qjs::JS_DefinePropertyValueStr(target, error, c"message".as_ptr(), message, (qjs::JS_PROP_WRITABLE | qjs::JS_PROP_CONFIGURABLE) as i32);
                let code = c"ERR_SCRIPT_EXECUTION_TIMEOUT";
                let code = qjs::JS_NewStringLen(target, code.as_ptr(), code.to_bytes().len() as _);
                qjs::JS_DefinePropertyValueStr(target, error, c"code".as_ptr(), code, qjs::JS_PROP_C_W_E as i32);
                return Err(vm_throw(&ctx, error));
            }
            return Err(vm_throw(&ctx, exception));
        }
        Ok(Value::from_raw(ctx.clone(), value))
    }
}
