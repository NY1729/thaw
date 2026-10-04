#[derive(Debug)]
struct WasmJsException {
    handle: std::sync::atomic::AtomicU32,
    owner: std::thread::ThreadId,
}
impl Drop for WasmJsException {
    fn drop(&mut self) {
        let handle = self.handle.load(std::sync::atomic::Ordering::Relaxed);
        if handle != 0 && self.owner == std::thread::current().id() {
            let _ = WASM_JS_EXCEPTIONS.try_with(|exceptions| {
                let value = { exceptions.borrow_mut().1.remove(&handle) };
                drop(value);
            });
        }
    }
}
impl std::fmt::Display for WasmJsException {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("JavaScript WebAssembly callback exception")
    }
}
impl wasmi::core::HostError for WasmJsException {}

std::thread_local! {
    static WASM_JS_EXCEPTIONS: std::cell::RefCell<(u32, HashMap<u32, WasmJsValue>)> =
        std::cell::RefCell::new((1, HashMap::new()));
}

fn wasm_js_error(ctx: &Ctx<'_>, error: rquickjs::Error) -> wasmi::Error {
    if !matches!(error, rquickjs::Error::Exception) {
        return wasmi::Error::new(error.to_string());
    }
    let value = ctx.catch();
    let persistent = Persistent::save(ctx, value);
    let context = unsafe { std::mem::transmute::<Ctx<'_>, Ctx<'static>>(ctx.clone()) };
    WASM_JS_EXCEPTIONS.with(|exceptions| {
        let mut exceptions = exceptions.borrow_mut();
        let handle = exceptions.0;
        let Some(next) = handle.checked_add(1) else {
            return wasmi::Error::new("WebAssembly exception handle limit reached");
        };
        exceptions.0 = next;
        exceptions.1.insert(handle, WasmJsValue { context, value: persistent });
        wasmi::Error::host(WasmJsException {
            handle: std::sync::atomic::AtomicU32::new(handle),
            owner: std::thread::current().id(),
        })
    })
}

fn wasm_take_exception<'js>(ctx: Ctx<'js>, handle: u32) -> rquickjs::Result<Value<'js>> {
    let value = WASM_JS_EXCEPTIONS.with(|exceptions| exceptions.borrow_mut().1.remove(&handle));
    let value = value.ok_or_else(|| rquickjs::Error::new_from_js("released WebAssembly exception", "value"))?;
    let _owner = value.context;
    value.value.restore(&ctx)
}

fn wasm_finish_native_result<'js>(ctx: Ctx<'js>, encoded: String) -> rquickjs::Result<String> {
    let exception = serde_json::from_str::<serde_json::Value>(&encoded).ok()
        .and_then(|result| result.get("exception").and_then(serde_json::Value::as_u64))
        .and_then(|handle| u32::try_from(handle).ok());
    if let Some(handle) = exception {
        let value = wasm_take_exception(ctx.clone(), handle)?;
        return Err(ctx.throw(value));
    }
    Ok(encoded)
}

fn wasm_error_result(error: &wasmi::Error) -> String {
    if let Some(exception) = error.downcast_ref::<WasmJsException>() {
        let handle = exception.handle.swap(0, std::sync::atomic::Ordering::Relaxed);
        if handle != 0 {
            return serde_json::json!({ "ok": false, "exception": handle }).to_string();
        }
    }
    if let Some(exit) = error.i32_exit_status() {
        return serde_json::json!({ "ok": false, "exit": exit }).to_string();
    }
    let runtime = error.as_trap_code().is_some()
        || matches!(error.kind(), wasmi::errors::ErrorKind::Host(_) | wasmi::errors::ErrorKind::Message(_));
    serde_json::json!({ "ok": false, "error": error.to_string(), "runtime": runtime }).to_string()
}

#[derive(Clone)]
struct WasmJsImport {
    context: Ctx<'static>,
    function: Persistent<Function<'static>>,
    implementation: Persistent<Function<'static>>,
    owner: Option<u32>,
}

#[derive(Clone)]
struct WasmJsValue {
    context: Ctx<'static>,
    value: Persistent<Value<'static>>,
}

struct WasmInstance {
    store: SharedWasmStore,
    state: std::cell::RefCell<WasmInstanceState>,
}

// Reentrant calls must borrow the active Caller store and instance metadata
// separately. A suspended execution must not reborrow its full Store.
struct WasmInstanceState {
    handle: u32,
    instance: wasmi::Instance,
    imported_memories: HashMap<String, WasmMemory>,
    imported_globals: HashMap<String, wasmi::Global>,
    imported_tables: HashMap<String, wasmi::Table>,
    next_funcref: u32,
    funcrefs: HashMap<u32, wasmi::Func>,
    funcref_ids: HashMap<String, u32>,
    foreign_funcrefs: HashMap<String, (u32, u32)>,
}

// These stacks exist only during synchronous native execution / JS imports.
// RAII removes entries before their stack-owned state or Caller can disappear.
std::thread_local! {
    static WASM_VALUE_SWEEP_PENDING: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    static WASM_RUNNING_STATES: std::cell::RefCell<Vec<Option<*const std::cell::RefCell<WasmInstanceState>>>> = const { std::cell::RefCell::new(Vec::new()) };
    static WASM_ACTIVE_CALLERS: std::cell::RefCell<Vec<WasmActiveCallerEntry>> = const { std::cell::RefCell::new(Vec::new()) };
}

struct WasmRunningState;
impl WasmRunningState {
    fn during_start() -> Self {
        WASM_RUNNING_STATES.with(|stack| stack.borrow_mut().push(None));
        Self
    }
    fn enter(state: &std::cell::RefCell<WasmInstanceState>) -> Self {
        WASM_RUNNING_STATES.with(|stack| stack.borrow_mut().push(Some(state)));
        Self
    }
}
impl Drop for WasmRunningState {
    fn drop(&mut self) { WASM_RUNNING_STATES.with(|stack| { stack.borrow_mut().pop(); }); }
}

struct WasmActiveCallerEntry {
    store_identity: std::rc::Rc<()>,
    state: Option<*const std::cell::RefCell<WasmInstanceState>>,
    caller: *mut std::ffi::c_void,
}

struct WasmActiveCaller;
impl WasmActiveCaller {
    fn enter(caller: &mut WasmCaller<'_, WasmStoreData>) -> Self {
        let state = WASM_RUNNING_STATES.with(|stack| stack.borrow().last().copied().flatten());
        // Start callbacks also own a live Caller, even before instance metadata exists.
        let store_identity = caller.data().identity.clone();
        WASM_ACTIVE_CALLERS.with(|stack| stack.borrow_mut().push(WasmActiveCallerEntry {
            store_identity,
            state,
            caller: (caller as *mut WasmCaller<'_, WasmStoreData>).cast(),
        }));
        Self
    }
}
impl Drop for WasmActiveCaller {
    fn drop(&mut self) {
        WASM_ACTIVE_CALLERS.with(|stack| { stack.borrow_mut().pop(); });
    }
}

fn with_active_wasm_store(
    identity: &std::rc::Rc<()>,
    action: impl FnOnce(&mut WasmCaller<'_, WasmStoreData>) -> String,
) -> Option<String> {
    let caller = WASM_ACTIVE_CALLERS.with(|stack| {
        stack.borrow().iter().rev()
            .find(|entry| std::rc::Rc::ptr_eq(&entry.store_identity, identity))
            .map(|entry| entry.caller)
    })?;
    // The newest matching Caller owns this Store's current synchronous borrow.
    // No registry borrow crosses the action; the stack guard bounds its lifetime.
    Some(unsafe { action(&mut *caller.cast::<WasmCaller<'_, WasmStoreData>>()) })
}

fn with_active_wasm_instance(
    handle: u32,
    action: impl FnOnce(&std::cell::RefCell<WasmInstanceState>, &mut WasmCaller<'_, WasmStoreData>) -> String,
) -> Option<String> {
    let entry = WASM_ACTIVE_CALLERS.with(|stack| {
        stack.borrow().iter().rev().find_map(|entry| {
            let state = entry.state?;
            (unsafe { (&*state).borrow().handle } == handle)
                .then(|| (state, entry.store_identity.clone()))
        })
    })?;
    let (state, identity) = entry;
    with_active_wasm_store(&identity, |caller| unsafe { action(&*state, caller) })
}

#[derive(Clone)]
struct SharedWasmStore {
    identity: std::rc::Rc<()>,
    inner: std::rc::Rc<std::cell::RefCell<WasmStore<WasmStoreData>>>,
}
impl SharedWasmStore {
    fn new(engine: &WasmEngine) -> Self {
        let identity = std::rc::Rc::new(());
        let store = WasmStore::new(engine, WasmStoreData {
            identity: identity.clone(),
            ..Default::default()
        });
        Self { identity, inner: std::rc::Rc::new(std::cell::RefCell::new(store)) }
    }
    fn run<R>(&self, action: impl FnOnce(wasmi::StoreContextMut<'_, WasmStoreData>) -> R) -> R {
        let caller = WASM_ACTIVE_CALLERS.with(|stack| {
            stack.borrow().iter().rev()
                .find(|entry| std::rc::Rc::ptr_eq(&entry.store_identity, &self.identity))
                .map(|entry| entry.caller)
        });
        if let Some(caller) = caller {
            let caller = unsafe { &mut *caller.cast::<WasmCaller<'_, WasmStoreData>>() };
            action(wasmi::AsContextMut::as_context_mut(caller))
        } else {
            let mut store = self.inner.borrow_mut();
            action(wasmi::AsContextMut::as_context_mut(&mut *store))
        }
    }
}

struct StandaloneWasmMemory {
    store: SharedWasmStore,
    memory: WasmMemory,
}

#[derive(Default)]
struct WasmStoreData {
    identity: std::rc::Rc<()>,
    wasi: HashMap<u32, WasiCtx>,
    externrefs: HashMap<u32, wasmi::ExternRef>,
}

struct WasmTable {
    engine: WasmEngine,
    shared_store: SharedWasmStore,
    next_module: u32,
    next_instance: u32,
    next_memory: u32,
    modules: HashMap<u32, WasmModule>,
    instances: HashMap<u32, WasmInstance>,
    memories: HashMap<u32, StandaloneWasmMemory>,
}

impl Default for WasmTable {
    fn default() -> Self {
        let engine = WasmEngine::default();
        let shared_store = SharedWasmStore::new(&engine);
        Self {
            engine,
            shared_store,
            next_module: 1,
            next_instance: 1,
            next_memory: 1,
            modules: HashMap::new(),
            instances: HashMap::new(),
            memories: HashMap::new(),
        }
    }
}

fn wasm_kind(value: &wasmi::ExternType) -> &'static str {
    match value {
        wasmi::ExternType::Func(_) => "function",
        wasmi::ExternType::Global(_) => "global",
        wasmi::ExternType::Memory(_) => "memory",
        wasmi::ExternType::Table(_) => "table",
    }
}

fn wasm_type_name(value: WasmValType) -> &'static str {
    match value {
        WasmValType::I32 => "i32",
        WasmValType::I64 => "i64",
        WasmValType::F32 => "f32",
        WasmValType::F64 => "f64",
        WasmValType::V128 => "v128",
        WasmValType::FuncRef => "funcref",
        WasmValType::ExternRef => "externref",
    }
}

fn wasm_type_from_name(value: &str) -> Result<WasmValType, String> {
    match value {
        "i32" => Ok(WasmValType::I32),
        "i64" => Ok(WasmValType::I64),
        "f32" => Ok(WasmValType::F32),
        "f64" => Ok(WasmValType::F64),
        "v128" => Ok(WasmValType::V128),
        "funcref" => Ok(WasmValType::FuncRef),
        "externref" => Ok(WasmValType::ExternRef),
        _ => Err(format!("invalid WebAssembly value type `{value}`")),
    }
}

fn wasm_compile(value: String) -> String {
    WASM.with(|table| {
        let mut table = table.borrow_mut();
        let bytes = hex_decode(&value);
        let module = match WasmModule::new(&table.engine, bytes) {
            Ok(module) => module,
            Err(error) => return serde_json::json!({ "ok": false, "error": error.to_string() }).to_string(),
        };
        let exports = module
            .exports()
            .map(|export| serde_json::json!({ "name": export.name(), "kind": wasm_kind(export.ty()) }))
            .collect::<Vec<_>>();
        let imports = module
            .imports()
            .map(|import| {
                let result_types = match import.ty() {
                    wasmi::ExternType::Func(function) => Some(function.results().iter().copied().map(wasm_type_name).collect::<Vec<_>>()),
                    _ => None,
                };
                serde_json::json!({ "module": import.module(), "name": import.name(), "kind": wasm_kind(import.ty()), "resultTypes": result_types })
            })
            .collect::<Vec<_>>();
        let handle = table.next_module;
        table.next_module += 1;
        table.modules.insert(handle, module);
        serde_json::json!({ "ok": true, "handle": handle, "exports": exports, "imports": imports }).to_string()
    })
}

fn wasm_custom_sections(module_handle: u32, name: String) -> String {
    WASM.with(|table| {
        let table = table.borrow();
        let Some(module) = table.modules.get(&module_handle) else {
            return serde_json::json!({ "ok": false, "error": "invalid WebAssembly.Module" })
                .to_string();
        };
        let sections = module
            .custom_sections()
            .filter(|section| section.name() == name)
            .map(|section| hex_encode(section.data()))
            .collect::<Vec<_>>();
        serde_json::json!({ "ok": true, "sections": sections }).to_string()
    })
}

fn wasm_retain_import<'js>(ctx: Ctx<'js>, function: Function<'js>, implementation: Option<Function<'js>>) -> u32 {
    let persistent = Persistent::save(&ctx, function);
    let implementation = implementation.map(|function| Persistent::save(&ctx, function)).unwrap_or_else(|| persistent.clone());
    // The cloned context owns a QuickJS context reference and the import table
    // is thread-local. It is dropped before the thread-local JS runtime that
    // created it, so extending only its Rust lifetime brand is sound.
    let context = unsafe { std::mem::transmute::<Ctx<'js>, Ctx<'static>>(ctx.clone()) };
    WASM_JS_IMPORTS.with(|imports| {
        let mut imports = imports.borrow_mut();
        let handle = imports.0;
        imports.0 += 1;
        imports.1.insert(
            handle,
            WasmJsImport {
                context,
                function: persistent,
                implementation,
                owner: None,
            },
        );
        handle
    })
}

fn wasm_retain_funcref<'js>(ctx: Ctx<'js>, function: Function<'js>, owner: u32) -> u32 {
    let handle = wasm_retain_import(ctx, function, None);
    WASM_JS_IMPORTS.with(|imports| {
        if let Some(import) = imports.borrow_mut().1.get_mut(&handle) {
            import.owner = Some(owner);
        }
    });
    handle
}

fn wasm_restore_import<'js>(ctx: Ctx<'js>, handle: u32) -> rquickjs::Result<Function<'js>> {
    let import = WASM_JS_IMPORTS.with(|imports| imports.borrow().1.get(&handle).cloned());
    let import = import
        .ok_or_else(|| rquickjs::Error::new_from_js("released WebAssembly function", "function"))?;
    let _owner = import.context;
    import.function.restore(&ctx)
}

fn wasm_retain_value<'js>(ctx: Ctx<'js>, value: Value<'js>) -> u32 {
    if value.is_null() {
        return 0;
    }
    let persistent = Persistent::save(&ctx, value);
    // See `wasm_retain_import`: the owned context reference keeps this realm
    // alive and only its invariant lifetime brand is extended.
    let context = unsafe { std::mem::transmute::<Ctx<'js>, Ctx<'static>>(ctx.clone()) };
    WASM_JS_VALUES.with(|values| {
        let mut values = values.borrow_mut();
        let handle = values.0;
        values.0 += 1;
        values.1.insert(
            handle,
            WasmJsValue {
                context,
                value: persistent,
            },
        );
        handle
    })
}

fn wasm_retain_value_at<'js>(ctx: Ctx<'js>, value: Value<'js>, handle: u32) -> u32 {
    if value.is_null() {
        return 0;
    }
    let exists = WASM_JS_VALUES.with(|values| values.borrow().1.contains_key(&handle));
    if exists {
        return handle;
    }
    let persistent = Persistent::save(&ctx, value);
    let context = unsafe { std::mem::transmute::<Ctx<'js>, Ctx<'static>>(ctx.clone()) };
    WASM_JS_VALUES.with(|values| {
        let mut values = values.borrow_mut();
        values.0 = values.0.max(handle.saturating_add(1));
        values.1.insert(
            handle,
            WasmJsValue {
                context,
                value: persistent,
            },
        );
    });
    handle
}

fn wasm_restore_value<'js>(ctx: Ctx<'js>, handle: u32) -> rquickjs::Result<Value<'js>> {
    if handle == 0 {
        return Ok(Value::new_null(ctx));
    }
    let value = WASM_JS_VALUES.with(|values| values.borrow().1.get(&handle).cloned());
    let value = value.ok_or_else(|| rquickjs::Error::new_from_js("released externref", "value"))?;
    let _owner = value.context;
    value.value.restore(&ctx)
}

fn wasm_reference_stats() -> String {
    let imports = WASM_JS_IMPORTS.with(|imports| imports.borrow().1.len());
    let values = WASM_JS_VALUES.with(|values| values.borrow().1.len());
    let (modules, instances, store_externrefs) = WASM.with(|table| {
        let table = table.borrow();
        (
            table.modules.len(),
            table.instances.len(),
            table.shared_store.run(|context| context.data().externrefs.len()),
        )
    });
    serde_json::json!({
        "imports": imports,
        "values": values,
        "modules": modules,
        "instances": instances,
        "storeExternrefs": store_externrefs,
    })
    .to_string()
}

fn wasm_sweep_inactive_values() {
    // Executing instances are temporarily outside the registry. Their Store
    // remains live, so sweeping the registry alone would drop live JS values.
    if WASM_RUNNING_STATES.with(|stack| !stack.borrow().is_empty())
        || WASM_ACTIVE_CALLERS.with(|stack| !stack.borrow().is_empty()) {
        WASM_VALUE_SWEEP_PENDING.with(|pending| pending.set(true));
        return;
    }
    WASM_VALUE_SWEEP_PENDING.with(|pending| pending.set(false));
    let retired = WASM.with(|table| {
        let mut table = table.borrow_mut();
        if table.instances.is_empty() && table.memories.is_empty()
            && std::rc::Rc::strong_count(&table.shared_store.inner) == 1 {
            let replacement = SharedWasmStore::new(&table.engine);
            Some(std::mem::replace(&mut table.shared_store, replacement))
        } else {
            None
        }
    });
    // Drop the retired Store after releasing the registry borrow.
    drop(retired);
    let active_values = WASM.with(|table| {
        let store = table.borrow().shared_store.clone();
        store.run(|context| context.data().externrefs.keys().copied().collect::<std::collections::HashSet<_>>())
    });
    WASM_JS_VALUES.with(|values| {
        values
            .borrow_mut()
            .1
            .retain(|handle, _| active_values.contains(handle));
    });
}

fn wasm_flush_deferred_value_sweep() {
    if WASM_VALUE_SWEEP_PENDING.with(|pending| pending.get()) {
        wasm_sweep_inactive_values();
    }
}

struct WasmSweepOnExit;
impl Drop for WasmSweepOnExit {
    fn drop(&mut self) {
        wasm_flush_deferred_value_sweep();
    }
}

fn wasm_release_pending(import_handles: String) {
    let handles = serde_json::from_str::<Vec<u32>>(&import_handles).unwrap_or_default();
    WASM_JS_IMPORTS.with(|imports| {
        let mut imports = imports.borrow_mut();
        for handle in handles {
            if imports
                .1
                .get(&handle)
                .is_some_and(|import| import.owner.is_none())
            {
                imports.1.remove(&handle);
            }
        }
    });
    wasm_sweep_inactive_values();
}

fn wasm_release(kind: String, handle: u32) -> bool {
    if kind == "module" {
        return WASM.with(|table| table.borrow_mut().modules.remove(&handle).is_some());
    }
    if kind == "memory" {
        let removed = WASM.with(|table| table.borrow_mut().memories.remove(&handle).is_some());
        if removed { wasm_sweep_inactive_values(); }
        return removed;
    }
    if kind != "instance" {
        return false;
    }
    let removed = WASM.with(|table| table.borrow_mut().instances.remove(&handle));
    let Some(record) = removed else {
        return false;
    };
    record.store.run(|mut store| { store.data_mut().wasi.remove(&handle); });
    drop(record);
    WASM_JS_IMPORTS.with(|imports| {
        imports
            .borrow_mut()
            .1
            .retain(|_, import| import.owner != Some(handle));
    });
    wasm_sweep_inactive_values();
    true
}

fn wasm_js_value<'js>(
    value: &WasmVal,
    ctx: Ctx<'js>,
    caller: &WasmCaller<'_, WasmStoreData>,
) -> Result<Value<'js>, wasmi::Error> {
    match value {
        WasmVal::I32(value) => Ok(Value::new_int(ctx, *value)),
        WasmVal::I64(value) => {
            Value::new_big_int(ctx, *value).map_err(|error| wasmi::Error::new(error.to_string()))
        }
        WasmVal::F32(value) => Ok(Value::new_float(ctx, f32::from(*value).into())),
        WasmVal::F64(value) => Ok(Value::new_float(ctx, f64::from(*value))),
        WasmVal::ExternRef(reference) => {
            let handle = match reference.val() {
                None => 0,
                Some(reference) => *reference
                    .data(caller)
                    .downcast_ref::<u32>()
                    .ok_or_else(|| wasmi::Error::new("invalid WebAssembly externref payload"))?,
            };
            wasm_restore_value(ctx, handle).map_err(|error| wasmi::Error::new(error.to_string()))
        }
        other => Err(wasmi::Error::new(format!(
            "unsupported JavaScript WebAssembly import value {:?}",
            other.ty()
        ))),
    }
}

enum WasmPreparedValue {
    Scalar(WasmVal),
    ExternRef(u32),
}

fn wasm_finish_js_value(value: WasmPreparedValue, caller: &mut WasmCaller<'_, WasmStoreData>) -> WasmVal {
    match value {
        WasmPreparedValue::Scalar(value) => value,
        WasmPreparedValue::ExternRef(0) => WasmVal::ExternRef(wasmi::Ref::Null),
        WasmPreparedValue::ExternRef(handle) => {
            if let Some(reference) = caller.data().externrefs.get(&handle).copied() {
                return reference.into();
            }
            let reference = wasmi::ExternRef::new(&mut *caller, handle);
            caller.data_mut().externrefs.insert(handle, reference);
            reference.into()
        }
    }
}

fn wasm_prepare_js_value(
    value: Value<'_>,
    ty: WasmValType,
) -> Result<WasmPreparedValue, wasmi::Error> {
    let ctx = value.ctx().clone();
    let conversion_error = |error: rquickjs::Error| wasm_js_error(&ctx, error);
    match ty {
        WasmValType::I32 => <rquickjs::convert::Coerced<i32> as rquickjs::FromJs>::from_js(&ctx, value)
            .map(|value| WasmPreparedValue::Scalar(WasmVal::I32(value.0)))
            .map_err(conversion_error),
        WasmValType::I64 => {
            let mut converted = 0i64;
            // QuickJS applies ToBigInt and signed 64-bit wrapping directly.
            // No Caller borrow is held while user-defined coercion can reenter.
            if unsafe { rquickjs::qjs::JS_ToBigInt64(ctx.as_ptr(), &mut converted, value.as_js_value()) } < 0 {
                return Err(wasm_js_error(&ctx, rquickjs::Error::Exception));
            }
            Ok(WasmPreparedValue::Scalar(WasmVal::I64(converted)))
        }
        WasmValType::F32 | WasmValType::F64 => {
            let value = <rquickjs::convert::Coerced<f64> as rquickjs::FromJs>::from_js(&ctx, value)
                .map_err(conversion_error)?.0;
            Ok(WasmPreparedValue::Scalar(if ty == WasmValType::F32 {
                WasmVal::F32((value as f32).into())
            } else {
                WasmVal::F64(value.into())
            }))
        }
        WasmValType::ExternRef => {
            let ctx = value.ctx().clone();
            Ok(WasmPreparedValue::ExternRef(wasm_retain_value(ctx, value)))
        }
        other => Err(wasmi::Error::new(format!(
            "unsupported JavaScript WebAssembly import result {other:?}"
        ))),
    }
}

fn wasm_call_js_import(
    handle: u32,
    mut caller: WasmCaller<'_, WasmStoreData>,
    inputs: &[WasmVal],
    outputs: &mut [WasmVal],
) -> Result<(), wasmi::Error> {
    let import = WASM_JS_IMPORTS.with(|imports| imports.borrow().1.get(&handle).cloned());
    let import =
        import.ok_or_else(|| wasmi::Error::new("released JavaScript WebAssembly import"))?;
    let ctx = import.context.clone();
    let function = import
        .implementation
        .restore(&ctx)
        .map_err(|error| wasm_js_error(&ctx, error))?;
    let mut arguments = Args::new_unsized(ctx.clone());
    for input in inputs {
        arguments
            .push_arg(wasm_js_value(input, ctx.clone(), &caller)?)
            .map_err(|error| wasm_js_error(&ctx, error))?;
    }
    let active_caller = WasmActiveCaller::enter(&mut caller);
    let result: Value = function.call_arg(arguments).map_err(|error| wasm_js_error(&ctx, error))?;
    let mut prepared = Vec::with_capacity(outputs.len());
    if outputs.len() == 1 {
        prepared.push(wasm_prepare_js_value(result, outputs[0].ty())?);
    } else if outputs.len() > 1 {
        let array = result.into_array().ok_or_else(|| {
            wasmi::Error::new("multi-value WebAssembly import result must be an array")
        })?;
        if array.len() != outputs.len() {
            return Err(wasmi::Error::new(format!(
                "WebAssembly import returned {} values, expected {}",
                array.len(),
                outputs.len()
            )));
        }
        for (index, output) in outputs.iter_mut().enumerate() {
            let value = array
                .get(index)
                .map_err(|error| wasm_js_error(&ctx, error))?;
            prepared.push(wasm_prepare_js_value(value, output.ty())?);
        }
    }
    drop(active_caller);
    for (output, value) in outputs.iter_mut().zip(prepared) {
        *output = wasm_finish_js_value(value, &mut caller);
    }
    Ok(())
}

fn wasm_instantiate(ctx: Ctx<'_>, module_handle: u32, linkage: Option<String>) -> rquickjs::Result<String> {
    let result = wasm_instantiate_inner(module_handle, linkage);
    wasm_finish_native_result(ctx, result)
}

fn wasm_instantiate_inner(module_handle: u32, linkage: Option<String>) -> String {
    // Declared before Store/state locals so they retire before deferred sweep.
    let _sweep_on_exit = WasmSweepOnExit;
    let module_and_engine = WASM.with(|table| {
        let table = table.borrow();
        table.modules.get(&module_handle).cloned().map(|module| (module, table.engine.clone()))
    });
        let Some((module, engine)) = module_and_engine else {
            return serde_json::json!({ "ok": false, "error": "WebAssembly.Module belongs to a released runtime" }).to_string();
        };
        let linkage = match linkage.as_deref().map(serde_json::from_str::<serde_json::Value>).transpose() {
            Ok(linkage) => linkage.unwrap_or_default(),
            Err(error) => return serde_json::json!({ "ok": false, "error": error.to_string() }).to_string(),
        };
        let wasi_options = linkage.get("wasi").and_then(serde_json::Value::as_str);
        let function_imports = linkage
            .get("functions")
            .and_then(serde_json::Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|value| {
                Some(((value.get("module")?.as_str()?.to_string(), value.get("name")?.as_str()?.to_string()), u32::try_from(value.get("handle")?.as_u64()?).ok()?))
            })
            .collect::<HashMap<_, _>>();
        let memory_imports = wasm_linkage_values(&linkage, "memories");
        let global_imports = wasm_linkage_values(&linkage, "globals");
        let table_imports = wasm_linkage_values(&linkage, "tables");
        let wasi = match wasi_options.map(wasm_wasi_context).transpose() {
            Ok(wasi) => wasi,
            Err(error) => return serde_json::json!({ "ok": false, "error": error }).to_string(),
        };
        // Reserve the owner before installing imports. A future shared Store
        // must select WASI by this owner, including during its start callback.
        let handle = WASM.with(|table| {
            let mut table = table.borrow_mut();
            let handle = table.next_instance;
            table.next_instance += 1;
            handle
        });
        let shared_store = WASM.with(|table| table.borrow().shared_store.clone());
        let retained_store = shared_store.clone();
        let published = std::cell::Cell::new(false);
        let result = shared_store.run(|mut store| {
        if let Some(wasi) = wasi {
            store.data_mut().wasi.insert(handle, wasi);
        }
        let mut linker = WasmLinker::new(&engine);
        if store.data().wasi.contains_key(&handle) {
            if let Err(error) = wasmi_wasi::add_to_linker(&mut linker, move |data: &mut WasmStoreData| {
                data.wasi.get_mut(&handle).expect("owning instance WASI context must exist")
            })
            {
                return serde_json::json!({ "ok": false, "error": error.to_string() }).to_string();
            }
        }
        let mut imported_memories = HashMap::new();
        let mut memory_origins: HashMap<String, String> = HashMap::new();
        let mut imported_globals = HashMap::new();
        let mut imported_tables = HashMap::new();
        let mut foreign_funcrefs = HashMap::new();
        for import in module.imports() {
            if import.module() == "wasi_snapshot_preview1" && store.data().wasi.contains_key(&handle) {
                continue;
            }
            let key = (import.module().to_string(), import.name().to_string());
            let encoded_key = wasm_import_key(import.module(), import.name());
            match import.ty() {
                wasmi::ExternType::Func(function_type) => {
                    let Some(handle) = function_imports.get(&key).copied() else {
                        return serde_json::json!({ "ok": false, "error": format!("WebAssembly import {}.{} is not provided", import.module(), import.name()) }).to_string();
                    };
                    if let Err(error) = linker.func_new(
                        import.module(),
                        import.name(),
                        function_type.clone(),
                        move |caller, inputs, outputs| {
                            wasm_call_js_import(handle, caller, inputs, outputs)
                        },
                    ) {
                        return serde_json::json!({ "ok": false, "error": error.to_string() }).to_string();
                    }
                }
                wasmi::ExternType::Memory(_) => {
                    let Some(descriptor) = memory_imports.get(&key) else {
                        return serde_json::json!({ "ok": false, "error": format!("WebAssembly memory import {}.{} is not provided", import.module(), import.name()) }).to_string();
                    };
                    let memory = match wasm_import_memory(descriptor, &store) {
                        Ok(memory) => memory,
                        Err(error) => return serde_json::json!({ "ok": false, "error": error }).to_string(),
                    };
                    memory_origins.entry(format!("{memory:?}")).or_insert_with(|| encoded_key.clone());
                    if let Err(error) = linker.define(import.module(), import.name(), memory) {
                        return serde_json::json!({ "ok": false, "error": error.to_string() }).to_string();
                    }
                    imported_memories.insert(encoded_key, memory);
                }
                wasmi::ExternType::Global(global_type) => {
                    let Some(descriptor) = global_imports.get(&key) else {
                        return serde_json::json!({ "ok": false, "error": format!("WebAssembly global import {}.{} is not provided", import.module(), import.name()) }).to_string();
                    };
                    let value = match wasm_runtime_value(descriptor, global_type.content(), &mut store) {
                        Ok(value) => value,
                        Err(error) => return serde_json::json!({ "ok": false, "error": error }).to_string(),
                    };
                    let global = wasmi::Global::new(&mut store, value, global_type.mutability());
                    if let Err(error) = linker.define(import.module(), import.name(), global) {
                        return serde_json::json!({ "ok": false, "error": error.to_string() }).to_string();
                    }
                    imported_globals.insert(encoded_key, global);
                }
                wasmi::ExternType::Table(expected) => {
                    let Some(descriptor) = table_imports.get(&key) else {
                        return serde_json::json!({ "ok": false, "error": format!("WebAssembly table import {}.{} is not provided", import.module(), import.name()) }).to_string();
                    };
                    let values = descriptor.get("values").and_then(serde_json::Value::as_array).cloned().unwrap_or_default();
                    let minimum = u32::try_from(values.len()).unwrap_or(u32::MAX);
                    let maximum = descriptor.get("maximum").and_then(serde_json::Value::as_u64).and_then(|value| u32::try_from(value).ok());
                    if u64::from(minimum) < expected.minimum() {
                        return serde_json::json!({ "ok": false, "error": format!("WebAssembly table import {}.{} has incompatible limits", import.module(), import.name()) }).to_string();
                    }
                    let initial = WasmVal::default(expected.element());
                    let imported = match wasmi::Table::new(&mut store, wasmi::TableType::new(expected.element(), minimum, maximum), initial) {
                        Ok(table) => table,
                        Err(error) => return serde_json::json!({ "ok": false, "error": error.to_string() }).to_string(),
                    };
                    for (index, value) in values.iter().enumerate() {
                        let value = match if expected.element() == WasmValType::FuncRef
                            && value.get("v").and_then(serde_json::Value::as_u64).unwrap_or(0) != 0
                        {
                            wasm_foreign_funcref(value, &mut store).map(|(value, key, handles)| {
                                foreign_funcrefs.insert(key, handles);
                                value
                            })
                        } else {
                            wasm_runtime_value(value, expected.element(), &mut store)
                        } {
                            Ok(value) => value,
                            Err(error) => return serde_json::json!({ "ok": false, "error": error }).to_string(),
                        };
                        if let Err(error) = imported.set(&mut store, index as u64, value) {
                            return serde_json::json!({ "ok": false, "error": error.to_string() }).to_string();
                        }
                    }
                    if let Err(error) = linker.define(import.module(), import.name(), imported) {
                        return serde_json::json!({ "ok": false, "error": error.to_string() }).to_string();
                    }
                    imported_tables.insert(encoded_key, imported);
                }
            }
        }
        // A start callback has no published instance state yet. Hide an outer
        // running state so this new Store cannot be paired with that state.
        let start_scope = WasmRunningState::during_start();
        let instance = match linker.instantiate_and_start(&mut store, &module) {
            Ok(instance) => instance,
            Err(error) => return wasm_error_result(&error),
        };
        drop(start_scope);
        let exports = instance
            .exports(&store)
            .map(|export| {
                let name = export.name().to_string();
                let export_type = export.ty(&store);
                let parameters = match &export_type {
                    wasmi::ExternType::Func(function) => Some(function.params().len()),
                    _ => None,
                };
                let parameter_types = match &export_type {
                    wasmi::ExternType::Func(function) => Some(
                        function
                            .params()
                            .iter()
                            .copied()
                            .map(wasm_type_name)
                            .collect::<Vec<_>>(),
                    ),
                    _ => None,
                };
                let result_types = match &export_type {
                    wasmi::ExternType::Func(function) => Some(
                        function
                            .results()
                            .iter()
                            .copied()
                            .map(wasm_type_name)
                            .collect::<Vec<_>>(),
                    ),
                    _ => None,
                };
                // ponytail: wasmi 1.1 exposes no memory identity API; its derived
                // handle Debug includes store and index. Replace with an identity API when available.
                let (kind, memory_key, memory_import) = match export.into_extern() {
                    WasmExtern::Func(_) => ("function", None, None),
                    WasmExtern::Global(_) => ("global", None, None),
                    WasmExtern::Table(_) => ("table", None, None),
                    WasmExtern::Memory(memory) => {
                        let identity = format!("{memory:?}");
                        let import = memory_origins.get(&identity).cloned();
                        ("memory", Some(identity), import)
                    },
                };
                serde_json::json!({ "name": name, "kind": kind, "parameters": parameters, "parameterTypes": parameter_types, "resultTypes": result_types, "memoryKey": memory_key, "memoryImport": memory_import })
            })
            .collect::<Vec<_>>();

        WASM_JS_IMPORTS.with(|imports| {
            let mut imports = imports.borrow_mut();
            for import_handle in function_imports.values() {
                if let Some(import) = imports.1.get_mut(import_handle) {
                    import.owner = Some(handle);
                }
            }
        });
        WASM.with(|table| table.borrow_mut().instances.insert(handle, WasmInstance {
            store: retained_store,
            state: std::cell::RefCell::new(WasmInstanceState {
                handle,
                instance,
                imported_memories,
                imported_globals,
                imported_tables,
                next_funcref: 1,
                funcrefs: HashMap::new(),
                funcref_ids: HashMap::new(),
                foreign_funcrefs,
            }),
        }));
        published.set(true);
        serde_json::json!({ "ok": true, "handle": handle, "exports": exports }).to_string()
        });
        if !published.get() {
            shared_store.run(|mut store| { store.data_mut().wasi.remove(&handle); });
            WASM_VALUE_SWEEP_PENDING.with(|pending| pending.set(true));
        }
        result
}

fn wasm_import_memory(
    descriptor: &serde_json::Value,
    context: &impl wasmi::AsContext<Data = WasmStoreData>,
) -> Result<WasmMemory, String> {
    let owner = descriptor.get("instance").and_then(serde_json::Value::as_u64)
        .and_then(|value| u32::try_from(value).ok()).ok_or("invalid WebAssembly.Memory owner")?;
    let name = descriptor.get("name").and_then(serde_json::Value::as_str)
        .ok_or("invalid WebAssembly.Memory name")?;
    let identity = context.as_context().data().identity.clone();
    let resolve = |state: &WasmInstanceState| {
        if let Some(key) = name.strip_prefix("import:") {
            state.imported_memories.get(key).copied()
        } else {
            state.instance.get_memory(context, name)
        }
    };
    if owner != 0 {
        let active = WASM_ACTIVE_CALLERS.with(|stack| {
            stack.borrow().iter().rev().find_map(|entry| {
                let state = entry.state?;
                if !std::rc::Rc::ptr_eq(&entry.store_identity, &identity) { return None; }
                let state = unsafe { (&*state).borrow() };
                (state.handle == owner).then(|| resolve(&state)).flatten()
            })
        });
        if let Some(memory) = active { return Ok(memory); }
    }
    let memory = WASM.with(|table| {
        let table = table.borrow();
        if owner == 0 {
            let record = name.parse::<u32>().ok().and_then(|handle| table.memories.get(&handle))?;
            std::rc::Rc::ptr_eq(&record.store.identity, &identity).then_some(record.memory)
        } else {
            let record = table.instances.get(&owner)?;
            if !std::rc::Rc::ptr_eq(&record.store.identity, &identity) { return None; }
            resolve(&record.state.borrow())
        }
    });
    memory.ok_or_else(|| "WebAssembly.Memory belongs to a released or different Store".to_string())
}

fn wasm_import_key(module: &str, name: &str) -> String {
    format!("{module}\u{1f}{name}")
}

fn wasm_linkage_values(
    linkage: &serde_json::Value,
    field: &str,
) -> HashMap<(String, String), serde_json::Value> {
    linkage
        .get(field)
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|value| {
            Some((
                (
                    value.get("module")?.as_str()?.to_string(),
                    value.get("name")?.as_str()?.to_string(),
                ),
                value.get("value")?.clone(),
            ))
        })
        .collect()
}

fn wasm_wasi_context(options: &str) -> Result<WasiCtx, String> {
    let options: serde_json::Value =
        serde_json::from_str(options).map_err(|error| error.to_string())?;
    let mut builder = WasiCtxBuilder::new();
    builder.inherit_stdio();
    if let Some(arguments) = options.get("args").and_then(serde_json::Value::as_array) {
        let arguments = arguments
            .iter()
            .map(|value| {
                value
                    .as_str()
                    .map(str::to_string)
                    .ok_or_else(|| "WASI arguments must be strings".to_string())
            })
            .collect::<Result<Vec<_>, _>>()?;
        builder
            .args(&arguments)
            .map_err(|error| error.to_string())?;
    }
    if let Some(environment) = options.get("env").and_then(serde_json::Value::as_object) {
        let environment = environment
            .iter()
            .map(|(key, value)| {
                value
                    .as_str()
                    .map(|value| (key.clone(), value.to_string()))
                    .ok_or_else(|| "WASI environment values must be strings".to_string())
            })
            .collect::<Result<Vec<_>, _>>()?;
        builder
            .envs(&environment)
            .map_err(|error| error.to_string())?;
    }
    if let Some(preopens) = options
        .get("preopens")
        .and_then(serde_json::Value::as_object)
    {
        for (guest, host) in preopens {
            let host = host
                .as_str()
                .ok_or_else(|| "WASI preopen paths must be strings".to_string())?;
            let directory = WasiDir::open_ambient_dir(host, ambient_authority())
                .map_err(|error| error.to_string())?;
            builder
                .preopened_dir(directory, guest)
                .map_err(|error| error.to_string())?;
        }
    }
    Ok(builder.build())
}

fn wasm_encoded_number(value: f64) -> serde_json::Value {
    let encoded = if value.is_finite() && !(value == 0.0 && value.is_sign_negative()) {
        serde_json::json!(value)
    } else {
        serde_json::json!(if value.is_nan() { "NaN" } else if value == f64::INFINITY { "Infinity" } else if value == f64::NEG_INFINITY { "-Infinity" } else { "-0" })
    };
    serde_json::json!({ "t": "number", "v": encoded })
}

fn wasm_number(value: &serde_json::Value, ty: WasmValType) -> Result<WasmVal, String> {
    let string = value.get("v").and_then(serde_json::Value::as_str);
    let number = value.get("v").and_then(|v| v.as_f64().or_else(|| match v.as_str() {
        Some("NaN") => Some(f64::NAN), Some("Infinity") => Some(f64::INFINITY),
        Some("-Infinity") => Some(f64::NEG_INFINITY), Some("-0") => Some(-0.0), _ => None,
    }));
    let expected_tag = if ty == WasmValType::I64 { "bigint" } else { "number" };
    if value.get("t").and_then(serde_json::Value::as_str) != Some(expected_tag) {
        return Err("WebAssembly scalar argument has an incompatible encoded type".into());
    }
    match ty {
        WasmValType::I32 => Ok(WasmVal::I32(number.unwrap_or(0.0) as i32)),
        WasmValType::I64 => string
            .ok_or_else(|| "WebAssembly i64 arguments must be BigInt values".to_string())?
            .parse::<i64>()
            .map(WasmVal::I64)
            .map_err(|_| "WebAssembly i64 argument is out of range".to_string()),
        WasmValType::F32 => Ok(WasmVal::F32((number.unwrap_or(f64::NAN) as f32).into())),
        WasmValType::F64 => Ok(WasmVal::F64(number.unwrap_or(f64::NAN).into())),
        other => Err(format!("unsupported WebAssembly argument type {other:?}")),
    }
}

fn wasm_runtime_value(
    value: &serde_json::Value,
    ty: WasmValType,
    context: &mut impl wasmi::AsContextMut<Data = WasmStoreData>,
) -> Result<WasmVal, String> {
    let mut store = context.as_context_mut();
    if ty == WasmValType::FuncRef {
        let handle = value
            .get("v")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0);
        return if handle == 0 {
            Ok(WasmVal::FuncRef(wasmi::Ref::Null))
        } else {
            Err("WebAssembly funcref belongs to a different instance".to_string())
        };
    }
    if ty != WasmValType::ExternRef {
        return wasm_number(value, ty);
    }
    let handle = value
        .get("v")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0);
    if handle == 0 {
        return Ok(WasmVal::ExternRef(wasmi::Ref::Null));
    }
    let handle =
        u32::try_from(handle).map_err(|_| "invalid WebAssembly externref handle".to_string())?;
    if let Some(reference) = store.data().externrefs.get(&handle).copied() {
        return Ok(reference.into());
    }
    let reference = wasmi::ExternRef::new(&mut store, handle);
    store.data_mut().externrefs.insert(handle, reference);
    Ok(reference.into())
}

fn wasm_foreign_funcref(
    value: &serde_json::Value,
    context: &mut impl wasmi::AsContextMut<Data = WasmStoreData>,
) -> Result<(WasmVal, String, (u32, u32)), String> {
    let mut store = context.as_context_mut();
    let bridge = value
        .get("bridge")
        .and_then(serde_json::Value::as_u64)
        .and_then(|value| u32::try_from(value).ok())
        .ok_or_else(|| "WebAssembly funcref has no cross-store bridge".to_string())?;
    let restore = value
        .get("restore")
        .and_then(serde_json::Value::as_u64)
        .and_then(|value| u32::try_from(value).ok())
        .ok_or_else(|| "WebAssembly funcref has no identity handle".to_string())?;
    let parse_types = |field: &str| -> Result<Vec<WasmValType>, String> {
        value
            .get(field)
            .and_then(serde_json::Value::as_array)
            .ok_or_else(|| format!("WebAssembly funcref has no `{field}` signature"))?
            .iter()
            .map(|value| {
                value
                    .as_str()
                    .ok_or_else(|| format!("invalid WebAssembly `{field}` signature"))
                    .and_then(wasm_type_from_name)
            })
            .collect()
    };
    let function_type = wasmi::FuncType::new(parse_types("parameters")?, parse_types("results")?);
    let function = wasmi::Func::new(&mut store, function_type, move |caller, inputs, outputs| {
        wasm_call_js_import(bridge, caller, inputs, outputs)
    });
    let key = format!("{function:?}");
    Ok((WasmVal::from(function), key, (bridge, restore)))
}

fn wasm_instance_runtime_value(
    value: &serde_json::Value,
    ty: WasmValType,
    record: &mut WasmInstanceState,
    context: &mut impl wasmi::AsContextMut<Data = WasmStoreData>,
) -> Result<WasmVal, String> {
    if ty != WasmValType::FuncRef {
        return wasm_runtime_value(value, ty, context);
    }
    let handle = value
        .get("v")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0);
    if handle == 0 {
        return Ok(WasmVal::FuncRef(wasmi::Ref::Null));
    }
    let owner = value
        .get("instance")
        .and_then(serde_json::Value::as_u64)
        .and_then(|value| u32::try_from(value).ok());
    if owner != Some(record.handle) {
        let (function, key, handles) = wasm_foreign_funcref(value, context)?;
        record.foreign_funcrefs.insert(key, handles);
        return Ok(function);
    }
    let handle = u32::try_from(handle).map_err(|_| "invalid WebAssembly funcref handle")?;
    record
        .funcrefs
        .get(&handle)
        .copied()
        .map(WasmVal::from)
        .ok_or_else(|| "WebAssembly funcref belongs to a different instance".to_string())
}

fn wasm_value(value: WasmVal, record: &mut WasmInstanceState, context: &impl wasmi::AsContext<Data = WasmStoreData>) -> Result<serde_json::Value, String> {
    let store = context.as_context();
    match value {
        WasmVal::I32(value) => Ok(serde_json::json!({ "t": "number", "v": value })),
        WasmVal::I64(value) => Ok(serde_json::json!({ "t": "bigint", "v": value.to_string() })),
        WasmVal::F32(value) => Ok(wasm_encoded_number(f32::from(value).into())),
        WasmVal::F64(value) => Ok(wasm_encoded_number(f64::from(value))),
        WasmVal::ExternRef(reference) => {
            let handle = match reference.val() {
                None => 0,
                Some(reference) => *reference
                    .data(&store)
                    .downcast_ref::<u32>()
                    .ok_or_else(|| "invalid WebAssembly externref payload".to_string())?,
            };
            Ok(serde_json::json!({ "t": "externref", "v": handle }))
        }
        WasmVal::FuncRef(reference) => {
            let Some(function) = reference.val().copied() else {
                return Ok(serde_json::json!({ "t": "funcref", "v": 0 }));
            };
            let key = format!("{function:?}");
            if let Some((_, restore)) = record.foreign_funcrefs.get(&key).copied() {
                return Ok(serde_json::json!({ "t": "jsfuncref", "v": restore }));
            }
            let handle = match record.funcref_ids.get(&key).copied() {
                Some(handle) => handle,
                None => {
                    let handle = record.next_funcref;
                    record.next_funcref += 1;
                    record.funcrefs.insert(handle, function);
                    record.funcref_ids.insert(key, handle);
                    handle
                }
            };
            let ty = function.ty(&store);
            let parameters = ty
                .params()
                .iter()
                .copied()
                .map(wasm_type_name)
                .collect::<Vec<_>>();
            let results = ty
                .results()
                .iter()
                .copied()
                .map(wasm_type_name)
                .collect::<Vec<_>>();
            Ok(
                serde_json::json!({ "t": "funcref", "v": handle, "parameters": parameters, "results": results }),
            )
        }
        other => Err(format!(
            "unsupported WebAssembly result type {:?}",
            other.ty()
        )),
    }
}

fn wasm_call_function(record: &std::cell::RefCell<WasmInstanceState>, context: &mut impl wasmi::AsContextMut<Data = WasmStoreData>, function: wasmi::Func, arguments: &str) -> String {
    let mut store = context.as_context_mut();
    let ty = function.ty(&store);
    let raw: Vec<serde_json::Value> = match serde_json::from_str(arguments) {
        Ok(values) => values,
        Err(error) => {
            return serde_json::json!({ "ok": false, "error": error.to_string() }).to_string()
        }
    };
    if raw.len() != ty.params().len() {
        return serde_json::json!({ "ok": false, "error": format!("expected {} arguments, received {}", ty.params().len(), raw.len()) }).to_string();
    }
    let mut inputs = Vec::with_capacity(raw.len());
    for (value, ty) in raw.iter().zip(ty.params()) {
        match wasm_instance_runtime_value(value, *ty, &mut record.borrow_mut(), &mut store) {
            Ok(value) => inputs.push(value),
            Err(error) => return serde_json::json!({ "ok": false, "error": error }).to_string(),
        }
    }
    let mut outputs = ty
        .results()
        .iter()
        .copied()
        .map(WasmVal::default)
        .collect::<Vec<_>>();
    let running_state = WasmRunningState::enter(record);
    let called = function.call(&mut store, &inputs, &mut outputs);
    drop(running_state);
    if let Err(error) = called { return wasm_error_result(&error); }
    let mut values = Vec::with_capacity(outputs.len());
    for output in outputs {
        match wasm_value(output, &mut record.borrow_mut(), &store) {
            Ok(value) => values.push(value),
            Err(error) => return serde_json::json!({ "ok": false, "error": error }).to_string(),
        }
    }
    serde_json::json!({ "ok": true, "values": values }).to_string()
}

fn wasm_call(ctx: Ctx<'_>, instance_handle: u32, name: String, arguments: String) -> rquickjs::Result<String> {
    let result = wasm_call_inner(instance_handle, name, arguments);
    wasm_finish_native_result(ctx, result)
}

fn wasm_call_inner(instance_handle: u32, name: String, arguments: String) -> String {
    // Declared before Store/state locals so they retire before deferred sweep.
    let _sweep_on_exit = WasmSweepOnExit;
    if let Some(result) = with_active_wasm_instance(instance_handle, |state, caller| {
        let function = state.borrow().instance.get_func(&*caller, &name);
        match function {
            Some(function) => wasm_call_function(state, caller, function, &arguments),
            None => serde_json::json!({ "ok": false, "error": format!("WebAssembly export `{name}` is not a function") }).to_string(),
        }
    }) { return result; }
    let record = WASM.with(|table| {
        let mut table = table.borrow_mut();
        table.instances.remove(&instance_handle)
    });
    let Some(mut record) = record else {
        return serde_json::json!({ "ok": false, "error": "invalid or recursively entered WebAssembly.Instance" }).to_string();
    };
    let store = record.store.clone();
    let result = store.run(|mut context| {
        let function = record.state.borrow().instance.get_func(&context, &name);
        match function {
            Some(function) => wasm_call_function(&record.state, &mut context, function, &arguments),
            None => serde_json::json!({ "ok": false, "error": format!("WebAssembly export `{name}` is not a function") }).to_string(),
        }
    });
    WASM.with(|table| {
        table.borrow_mut().instances.insert(instance_handle, record);
    });
    result
}

fn wasm_call_funcref(ctx: Ctx<'_>, instance_handle: u32, handle: u32, arguments: String) -> rquickjs::Result<String> {
    let result = wasm_call_funcref_inner(instance_handle, handle, arguments);
    wasm_finish_native_result(ctx, result)
}

fn wasm_call_funcref_inner(instance_handle: u32, handle: u32, arguments: String) -> String {
    // Declared before Store/state locals so they retire before deferred sweep.
    let _sweep_on_exit = WasmSweepOnExit;
    if let Some(result) = with_active_wasm_instance(instance_handle, |state, caller| {
        let function = state.borrow().funcrefs.get(&handle).copied();
        match function {
            Some(function) => wasm_call_function(state, caller, function, &arguments),
            None => serde_json::json!({ "ok": false, "error": "released WebAssembly function reference" }).to_string(),
        }
    }) { return result; }
    let record = WASM.with(|table| table.borrow_mut().instances.remove(&instance_handle));
    let Some(mut record) = record else {
        return serde_json::json!({ "ok": false, "error": "invalid or recursively entered WebAssembly.Instance" }).to_string();
    };
    let function = record.state.borrow().funcrefs.get(&handle).copied();
    let result = match function {
        Some(function) => record.store.run(|mut context| wasm_call_function(&record.state, &mut context, function, &arguments)),
        None => {
            serde_json::json!({ "ok": false, "error": "released WebAssembly function reference" })
                .to_string()
        }
    };
    WASM.with(|table| {
        table.borrow_mut().instances.insert(instance_handle, record);
    });
    result
}

fn wasm_instance_export_funcref(state: &mut WasmInstanceState, context: &impl wasmi::AsContext<Data = WasmStoreData>, name: &str) -> String {
        let Some(function) = state.instance.get_func(&*context, &name) else {
            return serde_json::json!({ "ok": false, "error": format!("WebAssembly export `{name}` is not a function") }).to_string();
        };
        match wasm_value(WasmVal::from(function), state, &*context) {
            Ok(value) => serde_json::json!({ "ok": true, "value": value }).to_string(),
            Err(error) => serde_json::json!({ "ok": false, "error": error }).to_string(),
        }
}

fn wasm_export_funcref(instance_handle: u32, name: String) -> String {
    if let Some(result) = with_active_wasm_instance(instance_handle, |state, caller| wasm_instance_export_funcref(&mut state.borrow_mut(), caller, &name)) { return result; }
    WASM.with(|table| {
        let mut table = table.borrow_mut();
        let Some(record) = table.instances.get_mut(&instance_handle) else {
            return serde_json::json!({ "ok": false, "error": "invalid WebAssembly.Instance" })
                .to_string();
        };
        record.store.run(|mut context| wasm_instance_export_funcref(&mut record.state.borrow_mut(), &mut context, &name))
    })
}

fn wasm_instance_global(state: &mut WasmInstanceState, context: &mut impl wasmi::AsContextMut<Data = WasmStoreData>, name: &str, value: Option<&str>) -> String {
        let global = if let Some(key) = name.strip_prefix("import:") {
            state.imported_globals.get(key).copied()
        } else {
            state.instance.get_global(&*context, &name)
        };
        let Some(global) = global else {
            return serde_json::json!({ "ok": false, "error": format!("WebAssembly export `{name}` is not a global") }).to_string();
        };
        if let Some(value) = value {
            let encoded: serde_json::Value = match serde_json::from_str(&value) {
                Ok(value) => value,
                Err(error) => return serde_json::json!({ "ok": false, "error": error.to_string() }).to_string(),
            };
            let ty = global.ty(&*context).content();
            let value = match wasm_instance_runtime_value(&encoded, ty, state, &mut *context) {
                Ok(value) => value,
                Err(error) => return serde_json::json!({ "ok": false, "error": error }).to_string(),
            };
            if let Err(error) = global.set(&mut *context, value) {
                return serde_json::json!({ "ok": false, "error": error.to_string() }).to_string();
            }
        }
        let value = global.get(&*context);
        match wasm_value(value, state, &*context) {
            Ok(value) => serde_json::json!({ "ok": true, "value": value, "type": wasm_type_name(global.ty(&*context).content()), "mutable": global.ty(&*context).mutability().is_mut() }).to_string(),
            Err(error) => serde_json::json!({ "ok": false, "error": error }).to_string(),
        }
}

fn wasm_global(instance_handle: u32, name: String, value: Option<String>) -> String {
    if let Some(result) = with_active_wasm_instance(instance_handle, |state, caller| wasm_instance_global(&mut state.borrow_mut(), caller, &name, value.as_deref())) { return result; }
    WASM.with(|table| {
        let mut table = table.borrow_mut();
        let Some(record) = table.instances.get_mut(&instance_handle) else {
            return serde_json::json!({ "ok": false, "error": "invalid WebAssembly.Instance" }).to_string();
        };
        record.store.run(|mut context| wasm_instance_global(&mut record.state.borrow_mut(), &mut context, &name, value.as_deref()))
    })
}

fn wasm_memory_create(initial: u32, maximum: i64) -> String {
    let maximum = u32::try_from(maximum).ok();
    if initial > 65_536 || maximum.is_some_and(|maximum| maximum < initial || maximum > 65_536) {
        return serde_json::json!({ "ok": false, "error": "WebAssembly.Memory page limits are invalid" }).to_string();
    }
    let store = WASM.with(|table| table.borrow().shared_store.clone());
    let retained_store = store.clone();
    store.run(|mut context| {
        let memory = match WasmMemory::new(&mut context, WasmMemoryType::new(initial, maximum)) {
            Ok(memory) => memory,
            Err(error) => return serde_json::json!({ "ok": false, "error": error.to_string() }).to_string(),
        };
        let handle = WASM.with(|table| {
            let mut table = table.borrow_mut();
            let handle = table.next_memory;
            table.next_memory += 1;
            table.memories.insert(handle, StandaloneWasmMemory { store: retained_store, memory });
            handle
        });
        serde_json::json!({ "ok": true, "handle": handle }).to_string()
    })
}

fn wasm_memory(instance_handle: u32, name: String, operation: String, value: String) -> String {
    let store = WASM.with(|table| table.borrow().shared_store.clone());
    store.run(|mut context| {
        let descriptor = serde_json::json!({ "instance": instance_handle, "name": name });
        match wasm_import_memory(&descriptor, &context) {
            Ok(memory) => wasm_memory_operation(&memory, &mut context, &operation, &value),
            Err(error) => serde_json::json!({ "ok": false, "error": error }).to_string(),
        }
    })
}

fn wasm_memory_operation(
    memory: &WasmMemory,
    store: &mut impl wasmi::AsContextMut<Data = WasmStoreData>,
    operation: &str,
    value: &str,
) -> String {
    match operation {
        "retain" => {
            let identity = store.as_context().data().identity.clone();
            WASM.with(|table| {
                let mut table = table.borrow_mut();
                if !std::rc::Rc::ptr_eq(&table.shared_store.identity, &identity) {
                    return serde_json::json!({ "ok": false, "error": "WebAssembly.Memory belongs to a different Store" }).to_string();
                }
                let handle = table.next_memory;
                let Some(next) = handle.checked_add(1) else {
                    return serde_json::json!({ "ok": false, "error": "WebAssembly.Memory handle limit reached" }).to_string();
                };
                table.next_memory = next;
                let retained_store = table.shared_store.clone();
                table.memories.insert(handle, StandaloneWasmMemory { store: retained_store, memory: *memory });
                serde_json::json!({ "ok": true, "handle": handle }).to_string()
            })
        }
        "read" => {
            serde_json::json!({ "ok": true, "value": hex_encode(memory.data(&*store)) }).to_string()
        }
        "write" => {
            let bytes = hex_decode(value);
            if bytes.len() != memory.data_size(&*store) {
                return serde_json::json!({ "ok": false, "error": "WebAssembly.Memory buffer size changed" }).to_string();
            }
            memory.data_mut(store).copy_from_slice(&bytes);
            serde_json::json!({ "ok": true }).to_string()
        }
        "grow" => {
            let pages = value.parse::<u64>().unwrap_or(u64::MAX);
            match memory.grow(store, pages) {
                Ok(previous) => serde_json::json!({ "ok": true, "value": previous }).to_string(),
                Err(error) => {
                    serde_json::json!({ "ok": false, "error": error.to_string() }).to_string()
                }
            }
        }
        _ => serde_json::json!({ "ok": false, "error": "invalid WebAssembly.Memory operation" })
            .to_string(),
    }
}

fn wasm_instance_table(state: &mut WasmInstanceState, context: &mut impl wasmi::AsContextMut<Data = WasmStoreData>, name: &str, operation: &str, index: u64, value: Option<&str>) -> String {
        let table_value = if let Some(key) = name.strip_prefix("import:") {
            state.imported_tables.get(key).copied()
        } else {
            state.instance.get_table(&*context, &name)
        };
        let Some(table_value) = table_value else {
            return serde_json::json!({ "ok": false, "error": format!("WebAssembly export `{name}` is not a table") }).to_string();
        };
        match operation {
            "size" => serde_json::json!({ "ok": true, "value": table_value.size(&*context), "type": wasm_type_name(table_value.ty(&*context).element()) }).to_string(),
            "get" => match table_value.get(&*context, index) {
                Some(value) => match wasm_value(value, state, &*context) {
                    Ok(value) => serde_json::json!({ "ok": true, "value": value }).to_string(),
                    Err(error) => serde_json::json!({ "ok": false, "error": error }).to_string(),
                },
                None => serde_json::json!({ "ok": false, "error": "WebAssembly.Table index is out of bounds" }).to_string(),
            },
            "set" | "grow" => {
                let encoded: serde_json::Value = value
                    .as_deref()
                    .and_then(|value| serde_json::from_str(value).ok())
                    .unwrap_or_else(|| serde_json::json!({ "t": "externref", "v": 0 }));
                let element = table_value.ty(&*context).element();
                let value = match wasm_instance_runtime_value(&encoded, element, state, &mut *context) {
                    Ok(value) => value,
                    Err(error) => return serde_json::json!({ "ok": false, "error": error }).to_string(),
                };
                if operation == "set" {
                    match table_value.set(&mut *context, index, value) {
                        Ok(()) => serde_json::json!({ "ok": true }).to_string(),
                        Err(error) => serde_json::json!({ "ok": false, "error": error.to_string() }).to_string(),
                    }
                } else {
                    match table_value.grow(&mut *context, index, value) {
                        Ok(previous) => serde_json::json!({ "ok": true, "value": previous }).to_string(),
                        Err(error) => serde_json::json!({ "ok": false, "error": error.to_string() }).to_string(),
                    }
                }
            }
            _ => serde_json::json!({ "ok": false, "error": "invalid WebAssembly.Table operation" }).to_string(),
        }
}

fn wasm_table(
    instance_handle: u32,
    name: String,
    operation: String,
    index: u64,
    value: Option<String>,
) -> String {
    if let Some(result) = with_active_wasm_instance(instance_handle, |state, caller| wasm_instance_table(&mut state.borrow_mut(), caller, &name, &operation, index, value.as_deref())) { return result; }
    WASM.with(|table| {
        let mut table = table.borrow_mut();
        let Some(record) = table.instances.get_mut(&instance_handle) else {
            return serde_json::json!({ "ok": false, "error": "invalid WebAssembly.Instance" }).to_string();
        };
        record.store.run(|mut context| wasm_instance_table(&mut record.state.borrow_mut(), &mut context, &name, &operation, index, value.as_deref()))
    })
}
