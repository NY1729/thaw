#[derive(Clone)]
struct WasmJsImport {
    context: Ctx<'static>,
    function: Persistent<Function<'static>>,
    owner: Option<u32>,
}

#[derive(Clone)]
struct WasmJsValue {
    context: Ctx<'static>,
    value: Persistent<Value<'static>>,
}

struct WasmInstance {
    handle: u32,
    store: WasmStore<WasmStoreData>,
    instance: wasmi::Instance,
    imported_memories: HashMap<String, WasmMemory>,
    imported_globals: HashMap<String, wasmi::Global>,
    imported_tables: HashMap<String, wasmi::Table>,
    next_funcref: u32,
    funcrefs: HashMap<u32, wasmi::Func>,
    funcref_ids: HashMap<String, u32>,
    foreign_funcrefs: HashMap<String, (u32, u32)>,
}

struct StandaloneWasmMemory {
    store: WasmStore<WasmStoreData>,
    memory: WasmMemory,
}

#[derive(Default)]
struct WasmStoreData {
    wasi: Option<WasiCtx>,
    externrefs: HashMap<u32, wasmi::ExternRef>,
}

struct WasmTable {
    engine: WasmEngine,
    next_module: u32,
    next_instance: u32,
    next_memory: u32,
    modules: HashMap<u32, WasmModule>,
    instances: HashMap<u32, WasmInstance>,
    memories: HashMap<u32, StandaloneWasmMemory>,
}

impl Default for WasmTable {
    fn default() -> Self {
        Self {
            engine: WasmEngine::default(),
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
            .map(|import| serde_json::json!({ "module": import.module(), "name": import.name(), "kind": wasm_kind(import.ty()) }))
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

fn wasm_retain_import<'js>(ctx: Ctx<'js>, function: Function<'js>) -> u32 {
    let persistent = Persistent::save(&ctx, function);
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
                owner: None,
            },
        );
        handle
    })
}

fn wasm_retain_funcref<'js>(ctx: Ctx<'js>, function: Function<'js>, owner: u32) -> u32 {
    let handle = wasm_retain_import(ctx, function);
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
            table
                .instances
                .values()
                .map(|instance| instance.store.data().externrefs.len())
                .sum::<usize>(),
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
    let active_values = WASM.with(|table| {
        table
            .borrow()
            .instances
            .values()
            .flat_map(|instance| instance.store.data().externrefs.keys().copied())
            .collect::<std::collections::HashSet<_>>()
    });
    WASM_JS_VALUES.with(|values| {
        values
            .borrow_mut()
            .1
            .retain(|handle, _| active_values.contains(handle));
    });
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
    if kind != "instance" {
        return false;
    }
    let removed = WASM.with(|table| {
        let mut table = table.borrow_mut();
        table.instances.remove(&handle).is_some()
    });
    if !removed {
        return false;
    }
    WASM_JS_IMPORTS.with(|imports| {
        imports
            .borrow_mut()
            .1
            .retain(|_, import| import.owner != Some(handle));
    });
    wasm_sweep_inactive_values();
    true
}

fn run_quickjs_gc(ctx: Ctx<'_>) {
    ctx.run_gc();
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

fn wasm_from_js_value(
    value: Value<'_>,
    ty: WasmValType,
    caller: &mut WasmCaller<'_, WasmStoreData>,
) -> Result<WasmVal, wasmi::Error> {
    match ty {
        WasmValType::I32 => value
            .as_number()
            .map(|value| WasmVal::I32(value as i32))
            .ok_or_else(|| wasmi::Error::new("WebAssembly i32 import result must be a number")),
        WasmValType::I64 => value
            .into_big_int()
            .ok_or_else(|| wasmi::Error::new("WebAssembly i64 import result must be a BigInt"))?
            .to_i64()
            .map(WasmVal::I64)
            .map_err(|error| wasmi::Error::new(error.to_string())),
        WasmValType::F32 => value
            .as_number()
            .map(|value| WasmVal::F32((value as f32).into()))
            .ok_or_else(|| wasmi::Error::new("WebAssembly f32 import result must be a number")),
        WasmValType::F64 => value
            .as_number()
            .map(|value| WasmVal::F64(value.into()))
            .ok_or_else(|| wasmi::Error::new("WebAssembly f64 import result must be a number")),
        WasmValType::ExternRef => {
            let ctx = value.ctx().clone();
            let handle = wasm_retain_value(ctx, value);
            if handle == 0 {
                Ok(WasmVal::ExternRef(wasmi::Ref::Null))
            } else {
                if let Some(reference) = caller.data().externrefs.get(&handle).copied() {
                    return Ok(reference.into());
                }
                let reference = wasmi::ExternRef::new(&mut *caller, handle);
                caller.data_mut().externrefs.insert(handle, reference);
                Ok(reference.into())
            }
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
        .function
        .restore(&ctx)
        .map_err(|error| wasmi::Error::new(error.to_string()))?;
    let mut arguments = Args::new_unsized(ctx.clone());
    for input in inputs {
        arguments
            .push_arg(wasm_js_value(input, ctx.clone(), &caller)?)
            .map_err(|error| wasmi::Error::new(error.to_string()))?;
    }
    let result: Value = function.call_arg(arguments).map_err(|error| {
        let message = match error {
            rquickjs::Error::Exception => describe_exception(&ctx),
            other => other.to_string(),
        };
        wasmi::Error::new(message)
    })?;
    if outputs.len() == 1 {
        outputs[0] = wasm_from_js_value(result, outputs[0].ty(), &mut caller)?;
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
                .map_err(|error| wasmi::Error::new(error.to_string()))?;
            *output = wasm_from_js_value(value, output.ty(), &mut caller)?;
        }
    }
    Ok(())
}

fn wasm_instantiate(module_handle: u32, linkage: Option<String>) -> String {
    WASM.with(|table| {
        let mut table = table.borrow_mut();
        let Some(module) = table.modules.get(&module_handle).cloned() else {
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
        let mut store = WasmStore::new(
            &table.engine,
            WasmStoreData {
                wasi,
                externrefs: HashMap::new(),
            },
        );
        let mut linker = WasmLinker::new(&table.engine);
        if store.data().wasi.is_some() {
            if let Err(error) = wasmi_wasi::add_to_linker(&mut linker, |data: &mut WasmStoreData| {
                data.wasi.as_mut().expect("WASI context must exist")
            })
            {
                return serde_json::json!({ "ok": false, "error": error.to_string() }).to_string();
            }
        }
        let mut imported_memories = HashMap::new();
        let mut imported_globals = HashMap::new();
        let mut imported_tables = HashMap::new();
        let mut foreign_funcrefs = HashMap::new();
        for import in module.imports() {
            if import.module() == "wasi_snapshot_preview1" && store.data().wasi.is_some() {
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
                wasmi::ExternType::Memory(expected) => {
                    let Some(descriptor) = memory_imports.get(&key) else {
                        return serde_json::json!({ "ok": false, "error": format!("WebAssembly memory import {}.{} is not provided", import.module(), import.name()) }).to_string();
                    };
                    let bytes = hex_decode(descriptor.get("data").and_then(serde_json::Value::as_str).unwrap_or(""));
                    let pages = u32::try_from(bytes.len() / 65_536).unwrap_or(u32::MAX);
                    let maximum = descriptor.get("maximum").and_then(serde_json::Value::as_u64).and_then(|value| u32::try_from(value).ok());
                    if !bytes.len().is_multiple_of(65_536)
                        || u64::from(pages) < expected.minimum()
                    {
                        return serde_json::json!({ "ok": false, "error": format!("WebAssembly memory import {}.{} has incompatible limits", import.module(), import.name()) }).to_string();
                    }
                    let memory = match WasmMemory::new(&mut store, WasmMemoryType::new(pages, maximum)) {
                        Ok(memory) => memory,
                        Err(error) => return serde_json::json!({ "ok": false, "error": error.to_string() }).to_string(),
                    };
                    memory.data_mut(&mut store).copy_from_slice(&bytes);
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
        let instance = match linker.instantiate_and_start(&mut store, &module) {
            Ok(instance) => instance,
            Err(error) => return serde_json::json!({ "ok": false, "error": error.to_string() }).to_string(),
        };
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
                let kind = match export.into_extern() {
                    WasmExtern::Func(_) => "function",
                    WasmExtern::Global(_) => "global",
                    WasmExtern::Memory(_) => "memory",
                    WasmExtern::Table(_) => "table",
                };
                serde_json::json!({ "name": name, "kind": kind, "parameters": parameters, "parameterTypes": parameter_types, "resultTypes": result_types })
            })
            .collect::<Vec<_>>();
        let handle = table.next_instance;
        table.next_instance += 1;
        WASM_JS_IMPORTS.with(|imports| {
            let mut imports = imports.borrow_mut();
            for import_handle in function_imports.values() {
                if let Some(import) = imports.1.get_mut(import_handle) {
                    import.owner = Some(handle);
                }
            }
        });
        table.instances.insert(handle, WasmInstance {
            handle,
            store,
            instance,
            imported_memories,
            imported_globals,
            imported_tables,
            next_funcref: 1,
            funcrefs: HashMap::new(),
            funcref_ids: HashMap::new(),
            foreign_funcrefs,
        });
        serde_json::json!({ "ok": true, "handle": handle, "exports": exports }).to_string()
    })
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

fn wasm_number(value: &serde_json::Value, ty: WasmValType) -> Result<WasmVal, String> {
    let string = value.get("v").and_then(serde_json::Value::as_str);
    let number = value.get("v").and_then(serde_json::Value::as_f64);
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
    store: &mut WasmStore<WasmStoreData>,
) -> Result<WasmVal, String> {
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
    let reference = wasmi::ExternRef::new(&mut *store, handle);
    store.data_mut().externrefs.insert(handle, reference);
    Ok(reference.into())
}

fn wasm_foreign_funcref(
    value: &serde_json::Value,
    store: &mut WasmStore<WasmStoreData>,
) -> Result<(WasmVal, String, (u32, u32)), String> {
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
    let function = wasmi::Func::new(store, function_type, move |caller, inputs, outputs| {
        wasm_call_js_import(bridge, caller, inputs, outputs)
    });
    let key = format!("{function:?}");
    Ok((WasmVal::from(function), key, (bridge, restore)))
}

fn wasm_instance_runtime_value(
    value: &serde_json::Value,
    ty: WasmValType,
    record: &mut WasmInstance,
) -> Result<WasmVal, String> {
    if ty != WasmValType::FuncRef {
        return wasm_runtime_value(value, ty, &mut record.store);
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
        let (function, key, handles) = wasm_foreign_funcref(value, &mut record.store)?;
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

fn wasm_value(value: WasmVal, record: &mut WasmInstance) -> Result<serde_json::Value, String> {
    match value {
        WasmVal::I32(value) => Ok(serde_json::json!({ "t": "number", "v": value })),
        WasmVal::I64(value) => Ok(serde_json::json!({ "t": "bigint", "v": value.to_string() })),
        WasmVal::F32(value) => Ok(serde_json::json!({ "t": "number", "v": f32::from(value) })),
        WasmVal::F64(value) => Ok(serde_json::json!({ "t": "number", "v": f64::from(value) })),
        WasmVal::ExternRef(reference) => {
            let handle = match reference.val() {
                None => 0,
                Some(reference) => *reference
                    .data(&record.store)
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
            let ty = function.ty(&record.store);
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

fn wasm_call_function(record: &mut WasmInstance, function: wasmi::Func, arguments: &str) -> String {
    let ty = function.ty(&record.store);
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
        match wasm_instance_runtime_value(value, *ty, record) {
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
    if let Err(error) = function.call(&mut record.store, &inputs, &mut outputs) {
        if let Some(exit) = error.i32_exit_status() {
            return serde_json::json!({ "ok": false, "exit": exit }).to_string();
        }
        return serde_json::json!({ "ok": false, "error": error.to_string() }).to_string();
    }
    let mut values = Vec::with_capacity(outputs.len());
    for output in outputs {
        match wasm_value(output, record) {
            Ok(value) => values.push(value),
            Err(error) => return serde_json::json!({ "ok": false, "error": error }).to_string(),
        }
    }
    serde_json::json!({ "ok": true, "values": values }).to_string()
}

fn wasm_call(instance_handle: u32, name: String, arguments: String) -> String {
    let record = WASM.with(|table| {
        let mut table = table.borrow_mut();
        table.instances.remove(&instance_handle)
    });
    let Some(mut record) = record else {
        return serde_json::json!({ "ok": false, "error": "invalid or recursively entered WebAssembly.Instance" }).to_string();
    };
    let result = match record.instance.get_func(&record.store, &name) {
        Some(function) => wasm_call_function(&mut record, function, &arguments),
        None => serde_json::json!({ "ok": false, "error": format!("WebAssembly export `{name}` is not a function") }).to_string(),
    };
    WASM.with(|table| {
        table.borrow_mut().instances.insert(instance_handle, record);
    });
    result
}

fn wasm_call_funcref(instance_handle: u32, handle: u32, arguments: String) -> String {
    let record = WASM.with(|table| table.borrow_mut().instances.remove(&instance_handle));
    let Some(mut record) = record else {
        return serde_json::json!({ "ok": false, "error": "invalid or recursively entered WebAssembly.Instance" }).to_string();
    };
    let result = match record.funcrefs.get(&handle).copied() {
        Some(function) => wasm_call_function(&mut record, function, &arguments),
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

fn wasm_export_funcref(instance_handle: u32, name: String) -> String {
    WASM.with(|table| {
        let mut table = table.borrow_mut();
        let Some(record) = table.instances.get_mut(&instance_handle) else {
            return serde_json::json!({ "ok": false, "error": "invalid WebAssembly.Instance" })
                .to_string();
        };
        let Some(function) = record.instance.get_func(&record.store, &name) else {
            return serde_json::json!({ "ok": false, "error": format!("WebAssembly export `{name}` is not a function") }).to_string();
        };
        match wasm_value(WasmVal::from(function), record) {
            Ok(value) => serde_json::json!({ "ok": true, "value": value }).to_string(),
            Err(error) => serde_json::json!({ "ok": false, "error": error }).to_string(),
        }
    })
}

fn wasm_global(instance_handle: u32, name: String, value: Option<String>) -> String {
    WASM.with(|table| {
        let mut table = table.borrow_mut();
        let Some(record) = table.instances.get_mut(&instance_handle) else {
            return serde_json::json!({ "ok": false, "error": "invalid WebAssembly.Instance" }).to_string();
        };
        let global = if let Some(key) = name.strip_prefix("import:") {
            record.imported_globals.get(key).copied()
        } else {
            record.instance.get_global(&record.store, &name)
        };
        let Some(global) = global else {
            return serde_json::json!({ "ok": false, "error": format!("WebAssembly export `{name}` is not a global") }).to_string();
        };
        if let Some(value) = value {
            let encoded: serde_json::Value = match serde_json::from_str(&value) {
                Ok(value) => value,
                Err(error) => return serde_json::json!({ "ok": false, "error": error.to_string() }).to_string(),
            };
            let ty = global.ty(&record.store).content();
            let value = match wasm_instance_runtime_value(&encoded, ty, record) {
                Ok(value) => value,
                Err(error) => return serde_json::json!({ "ok": false, "error": error }).to_string(),
            };
            if let Err(error) = global.set(&mut record.store, value) {
                return serde_json::json!({ "ok": false, "error": error.to_string() }).to_string();
            }
        }
        let value = global.get(&record.store);
        match wasm_value(value, record) {
            Ok(value) => serde_json::json!({ "ok": true, "value": value }).to_string(),
            Err(error) => serde_json::json!({ "ok": false, "error": error }).to_string(),
        }
    })
}

fn wasm_memory_create(initial: u32, maximum: i64) -> String {
    WASM.with(|table| {
        let mut table = table.borrow_mut();
        let maximum = u32::try_from(maximum).ok();
        if initial > 65_536 || maximum.is_some_and(|maximum| maximum < initial || maximum > 65_536) {
            return serde_json::json!({ "ok": false, "error": "WebAssembly.Memory page limits are invalid" }).to_string();
        }
        let ty = WasmMemoryType::new(initial, maximum);
        let mut store = WasmStore::new(&table.engine, WasmStoreData::default());
        let memory = match WasmMemory::new(&mut store, ty) {
            Ok(memory) => memory,
            Err(error) => return serde_json::json!({ "ok": false, "error": error.to_string() }).to_string(),
        };
        let handle = table.next_memory;
        table.next_memory += 1;
        table.memories.insert(handle, StandaloneWasmMemory { store, memory });
        serde_json::json!({ "ok": true, "handle": handle }).to_string()
    })
}

fn wasm_memory(instance_handle: u32, name: String, operation: String, value: String) -> String {
    WASM.with(|table| {
        let mut table = table.borrow_mut();
        if instance_handle == 0 {
            let Some(record) = name.parse::<u32>().ok().and_then(|handle| table.memories.get_mut(&handle)) else {
                return serde_json::json!({ "ok": false, "error": "invalid WebAssembly.Memory" }).to_string();
            };
            return wasm_memory_operation(&record.memory, &mut record.store, &operation, &value);
        }
        let Some(record) = table.instances.get_mut(&instance_handle) else {
            return serde_json::json!({ "ok": false, "error": "invalid WebAssembly.Instance" }).to_string();
        };
        let memory = if let Some(key) = name.strip_prefix("import:") {
            record.imported_memories.get(key).copied()
        } else {
            record.instance.get_memory(&record.store, &name)
        };
        let Some(memory) = memory else {
            return serde_json::json!({ "ok": false, "error": format!("WebAssembly export `{name}` is not a memory") }).to_string();
        };
        wasm_memory_operation(&memory, &mut record.store, &operation, &value)
    })
}

fn wasm_memory_operation(
    memory: &WasmMemory,
    store: &mut WasmStore<WasmStoreData>,
    operation: &str,
    value: &str,
) -> String {
    match operation {
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

fn wasm_table(
    instance_handle: u32,
    name: String,
    operation: String,
    index: u64,
    value: Option<String>,
) -> String {
    WASM.with(|table| {
        let mut table = table.borrow_mut();
        let Some(record) = table.instances.get_mut(&instance_handle) else {
            return serde_json::json!({ "ok": false, "error": "invalid WebAssembly.Instance" }).to_string();
        };
        let table_value = if let Some(key) = name.strip_prefix("import:") {
            record.imported_tables.get(key).copied()
        } else {
            record.instance.get_table(&record.store, &name)
        };
        let Some(table_value) = table_value else {
            return serde_json::json!({ "ok": false, "error": format!("WebAssembly export `{name}` is not a table") }).to_string();
        };
        match operation.as_str() {
            "size" => serde_json::json!({ "ok": true, "value": table_value.size(&record.store) }).to_string(),
            "get" => match table_value.get(&record.store, index) {
                Some(value) => match wasm_value(value, record) {
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
                let element = table_value.ty(&record.store).element();
                let value = match wasm_instance_runtime_value(&encoded, element, record) {
                    Ok(value) => value,
                    Err(error) => return serde_json::json!({ "ok": false, "error": error }).to_string(),
                };
                if operation == "set" {
                    match table_value.set(&mut record.store, index, value) {
                        Ok(()) => serde_json::json!({ "ok": true }).to_string(),
                        Err(error) => serde_json::json!({ "ok": false, "error": error.to_string() }).to_string(),
                    }
                } else {
                    match table_value.grow(&mut record.store, index, value) {
                        Ok(previous) => serde_json::json!({ "ok": true, "value": previous }).to_string(),
                        Err(error) => serde_json::json!({ "ok": false, "error": error.to_string() }).to_string(),
                    }
                }
            }
            _ => serde_json::json!({ "ok": false, "error": "invalid WebAssembly.Table operation" }).to_string(),
        }
    })
}
