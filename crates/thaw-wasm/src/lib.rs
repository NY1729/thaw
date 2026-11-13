//! Engine-neutral WebAssembly runtime on wasmi 1.1.0 (interpreter, no JIT).
//!
//! Entities live in one `Store`, addressed by small copyable ids, so the
//! embedder (thaw-js) can keep ids in its own tables without holding wasmi
//! types. Host imports call back through [`HostCallbacks`].

mod error;
mod memory;
#[cfg(feature = "wasi")]
pub mod wasi;

use std::cell::Cell;
use std::ptr::NonNull;

pub use error::{CallError, CompileError, InstantiateError, LinkError, Trap};
pub use memory::{alloc_zeroed_vec, reserve_pages, Bytes, PAGE_BYTES, RESERVE_CEILING_PAGES};
use wasmi::{AsContext, AsContextMut, Ref};

/// True when this build can run SIMD (`v128`) modules.
pub const SIMD_AVAILABLE: bool = cfg!(feature = "simd");

macro_rules! id_type {
    ($($name:ident),*) => {$(
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
        pub struct $name(u32);
        impl $name {
            pub fn index(self) -> u32 {
                self.0
            }
        }
    )*};
}
id_type!(FuncId, MemoryId, TableId, GlobalId, InstanceId);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ValType {
    I32,
    I64,
    F32,
    F64,
    V128,
    FuncRef,
    ExternRef,
}

/// A wasm value at the embedder boundary. `ExternRef(Some(h))` is an opaque
/// embedder handle; `FuncRef(Some(id))` is a function owned by the store.
#[derive(Clone, Debug, PartialEq)]
pub enum Val {
    I32(i32),
    I64(i64),
    F32(f32),
    F64(f64),
    V128(u128),
    FuncRef(Option<FuncId>),
    ExternRef(Option<u32>),
}

/// Proposal switches. `simd` is only honored when the `simd` feature is on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Features {
    pub multi_value: bool,
    pub bulk_memory: bool,
    pub reference_types: bool,
    pub tail_call: bool,
    pub simd: bool,
}

impl Default for Features {
    fn default() -> Self {
        Self {
            multi_value: true,
            bulk_memory: true,
            reference_types: true,
            tail_call: true,
            simd: SIMD_AVAILABLE,
        }
    }
}

#[derive(Clone)]
pub struct Engine {
    inner: wasmi::Engine,
}

impl Engine {
    pub fn new(features: Features) -> Self {
        let mut config = wasmi::Config::default();
        config
            .wasm_multi_value(features.multi_value)
            .wasm_bulk_memory(features.bulk_memory)
            .wasm_reference_types(features.reference_types)
            .wasm_tail_call(features.tail_call);
        #[cfg(feature = "simd")]
        config.wasm_simd(features.simd);
        Self {
            inner: wasmi::Engine::new(&config),
        }
    }
}

impl Default for Engine {
    fn default() -> Self {
        Self::new(Features::default())
    }
}

/// Type of an import or export, as Node's `WebAssembly.Module.imports()` shows it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ExternKind {
    Func {
        params: Vec<ValType>,
        results: Vec<ValType>,
    },
    Memory {
        min: u64,
        max: Option<u64>,
    },
    Table {
        element: ValType,
        min: u64,
        max: Option<u64>,
    },
    Global {
        ty: ValType,
        mutable: bool,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImportDesc {
    pub module: String,
    pub name: String,
    pub kind: ExternKind,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExportDesc {
    pub name: String,
    pub kind: ExternKind,
}

pub struct Module {
    inner: wasmi::Module,
    imports: Vec<ImportDesc>,
    exports: Vec<ExportDesc>,
    custom: Vec<(String, Vec<u8>)>,
}

impl Module {
    /// Validates and compiles binary wasm. Text input is rejected like Node does.
    pub fn compile(engine: &Engine, bytes: &[u8]) -> Result<Self, CompileError> {
        check_header(bytes)?;
        let inner = wasmi::Module::new(&engine.inner, bytes)
            .map_err(|e| CompileError::new(e.to_string()))?;
        let imports = inner
            .imports()
            .map(|import| ImportDesc {
                module: import.module().to_string(),
                name: import.name().to_string(),
                kind: kind_of(import.ty()),
            })
            .collect();
        let mut exports: Vec<ExportDesc> = inner
            .exports()
            .map(|export| ExportDesc {
                name: export.name().to_string(),
                kind: kind_of(export.ty()),
            })
            .collect();
        // wasmi lists exports sorted by name; Node lists them in declaration order.
        let order = declared_export_names(bytes);
        exports.sort_by_key(|export| {
            order
                .iter()
                .position(|n| *n == export.name)
                .unwrap_or(usize::MAX)
        });
        let custom = inner
            .custom_sections()
            .map(|section| (section.name().to_string(), section.data().to_vec()))
            .collect();
        Ok(Self {
            inner,
            imports,
            exports,
            custom,
        })
    }

    pub fn validate(engine: &Engine, bytes: &[u8]) -> Result<(), CompileError> {
        Self::compile(engine, bytes).map(|_| ())
    }

    pub fn imports(&self) -> &[ImportDesc] {
        &self.imports
    }

    pub fn exports(&self) -> &[ExportDesc] {
        &self.exports
    }

    /// Payloads of every custom section with this name, in order.
    pub fn custom_sections(&self, name: &str) -> Vec<&[u8]> {
        self.custom
            .iter()
            .filter(|(n, _)| n == name)
            .map(|(_, data)| data.as_slice())
            .collect()
    }
}

/// Export names in the order the export section declares them (empty if unparsable).
fn declared_export_names(bytes: &[u8]) -> Vec<String> {
    fn uleb(bytes: &[u8], mut pos: usize) -> Option<(usize, usize)> {
        let (mut value, mut shift) = (0usize, 0u32);
        loop {
            let byte = *bytes.get(pos)?;
            pos += 1;
            value |= ((byte & 0x7f) as usize).checked_shl(shift)?;
            if byte & 0x80 == 0 {
                return Some((value, pos));
            }
            shift += 7;
        }
    }
    let mut names = Vec::new();
    let mut pos = 8;
    while let Some(&id) = bytes.get(pos) {
        let Some((size, body)) = uleb(bytes, pos + 1) else {
            break;
        };
        let Some(end) = body.checked_add(size) else {
            break;
        };
        if id == 7 {
            let Some((count, mut p)) = uleb(bytes, body) else {
                break;
            };
            for _ in 0..count {
                let Some((len, after)) = uleb(bytes, p) else {
                    break;
                };
                let Some(text) = bytes.get(after..after + len) else {
                    break;
                };
                names.push(String::from_utf8_lossy(text).into_owned());
                // Skip the kind byte, then the index.
                let Some((_, next)) = uleb(bytes, after + len + 1) else {
                    break;
                };
                p = next;
            }
        }
        pos = end;
    }
    names
}

/// The V8 header diagnostics that wasmi does not produce in the same words.
fn check_header(bytes: &[u8]) -> Result<(), CompileError> {
    fn hex(bytes: &[u8]) -> String {
        bytes
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<Vec<_>>()
            .join(" ")
    }
    if bytes.is_empty() {
        return Err(CompileError::new("BufferSource argument is empty"));
    }
    if bytes.len() < 4 {
        return Err(CompileError::new("expected 4 bytes, fell off end @+0"));
    }
    if bytes[..4] != [0x00, 0x61, 0x73, 0x6d] {
        return Err(CompileError::new(format!(
            "expected magic word 00 61 73 6d, found {} @+0",
            hex(&bytes[..4])
        )));
    }
    if bytes.len() < 8 {
        return Err(CompileError::new("expected 4 bytes, fell off end @+4"));
    }
    if bytes[4..8] != [0x01, 0x00, 0x00, 0x00] {
        return Err(CompileError::new(format!(
            "expected version 01 00 00 00, found {} @+4",
            hex(&bytes[4..8])
        )));
    }
    Ok(())
}

fn val_type(ty: wasmi::ValType) -> ValType {
    match ty {
        wasmi::ValType::I32 => ValType::I32,
        wasmi::ValType::I64 => ValType::I64,
        wasmi::ValType::F32 => ValType::F32,
        wasmi::ValType::F64 => ValType::F64,
        wasmi::ValType::V128 => ValType::V128,
        wasmi::ValType::FuncRef => ValType::FuncRef,
        wasmi::ValType::ExternRef => ValType::ExternRef,
    }
}

fn wasmi_val_type(ty: ValType) -> wasmi::ValType {
    match ty {
        ValType::I32 => wasmi::ValType::I32,
        ValType::I64 => wasmi::ValType::I64,
        ValType::F32 => wasmi::ValType::F32,
        ValType::F64 => wasmi::ValType::F64,
        ValType::V128 => wasmi::ValType::V128,
        ValType::FuncRef => wasmi::ValType::FuncRef,
        ValType::ExternRef => wasmi::ValType::ExternRef,
    }
}

fn kind_of(ty: &wasmi::ExternType) -> ExternKind {
    match ty {
        wasmi::ExternType::Func(func) => ExternKind::Func {
            params: func.params().iter().map(|t| val_type(*t)).collect(),
            results: func.results().iter().map(|t| val_type(*t)).collect(),
        },
        wasmi::ExternType::Memory(memory) => ExternKind::Memory {
            min: memory.minimum(),
            max: memory.maximum(),
        },
        wasmi::ExternType::Table(table) => ExternKind::Table {
            element: val_type(table.element()),
            min: table.minimum(),
            max: table.maximum(),
        },
        wasmi::ExternType::Global(global) => ExternKind::Global {
            ty: val_type(global.content()),
            mutable: matches!(global.mutability(), wasmi::Mutability::Var),
        },
    }
}

/// What the embedder supplies for one module import, in declaration order.
#[derive(Clone, Debug, PartialEq)]
pub enum Import {
    /// Nothing supplied. `module_exists` picks Node's TypeError vs LinkError text.
    Missing {
        module_exists: bool,
    },
    /// A JS-style host function: `HostCallbacks::call_import(id, ..)`.
    Host(u32),
    Func(FuncId),
    Memory(MemoryId),
    Table(TableId),
    Global(GlobalId),
    /// A plain number/reference for a global import (Node creates a new global).
    GlobalValue(Val),
}

/// Embedder side of host imports. `Err(id)` makes the call throw embedder
/// value `id` unchanged.
pub trait HostCallbacks {
    fn call_import(&mut self, id: u32, args: &[Val]) -> Result<Vec<Val>, u32>;
}

/// Export of an instance.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Extern {
    Func(FuncId),
    Memory(MemoryId),
    Table(TableId),
    Global(GlobalId),
}

/// Linear memory with a generation counter. The generation increments whenever
/// the page count changes (grow from JS, from wasm `memory.grow`, or from an
/// embedder call), so an embedder can detach views of the old size. Backing is
/// wasmi's own buffer today; an external-memory wasmi fork would replace this
/// struct's `data` path only.
struct MemEntry {
    key: String,
    mem: wasmi::Memory,
    pages: Cell<u64>,
    generation: Cell<u64>,
    /// Present when the bytes are shared with the embedder (see `memory.rs`).
    external: Option<memory::External>,
}

/// wasmi handles have no public identity, only `Debug` (unique per entity in a store).
/// Ids are interned by that key. ponytail: linear scan; use a map if profiles show it.
struct Slot<T> {
    key: String,
    item: T,
}

/// Per-store state, reachable from host closures through `Caller`.
pub(crate) struct HostState {
    funcs: Vec<Slot<wasmi::Func>>,
    memories: Vec<MemEntry>,
    tables: Vec<Slot<wasmi::Table>>,
    globals: Vec<Slot<wasmi::Global>>,
    instances: Vec<Vec<(String, Extern)>>,
    /// Valid only while `Store::call_*`/`instantiate` run (see `with_host`).
    host: Option<NonNull<dyn HostCallbacks>>,
    #[cfg(feature = "wasi")]
    pub(crate) wasi: wasmi_wasi::WasiCtx,
}

/// Carries an embedder exception id through wasmi's error type.
#[derive(Debug)]
struct HostException(u32);

impl std::fmt::Display for HostException {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "host exception {}", self.0)
    }
}

impl wasmi::errors::HostError for HostException {}

pub struct Store {
    inner: wasmi::Store<HostState>,
}

impl Store {
    pub fn new(engine: &Engine) -> Self {
        let state = HostState {
            funcs: Vec::new(),
            memories: Vec::new(),
            tables: Vec::new(),
            globals: Vec::new(),
            instances: Vec::new(),
            host: None,
            #[cfg(feature = "wasi")]
            wasi: wasmi_wasi::sync::WasiCtxBuilder::new().build(),
        };
        Self {
            inner: wasmi::Store::new(&engine.inner, state),
        }
    }

    // ---- memory -------------------------------------------------------

    /// Creates an embedder-owned memory whose bytes are shared through `memory_buffer`.
    pub fn new_memory(&mut self, min: u32, max: Option<u32>) -> Result<MemoryId, String> {
        let vec = reserve_pages(min, max, &mut alloc_zeroed_vec)
            .ok_or_else(|| "could not allocate memory".to_string())?;
        let (mem, external) = memory::create_external(&mut self.inner, vec, min, max)?;
        Ok(self.intern_memory(mem, Some(external)))
    }

    /// Shared storage of an embedder-owned memory; `None` for memories wasmi owns
    /// (memories defined inside a module). Identity changes when a grow replaces it.
    pub fn memory_buffer(&self, id: MemoryId) -> Option<Bytes> {
        self.state().memories[id.0 as usize]
            .external
            .as_ref()
            .map(|ext| ext.buffer())
    }

    pub fn memory_pages(&self, id: MemoryId) -> u64 {
        self.state().memories[id.0 as usize].pages.get()
    }

    pub fn memory_generation(&self, id: MemoryId) -> u64 {
        self.state().memories[id.0 as usize].generation.get()
    }

    /// Runs `f` over the current bytes of the memory. Do not retain the slice.
    pub fn with_memory_bytes<R>(&mut self, id: MemoryId, f: impl FnOnce(&mut [u8]) -> R) -> R {
        if let Some(buffer) = self.memory_buffer(id) {
            let mut bytes = buffer.borrow_mut();
            return f(&mut bytes[..]);
        }
        let mem = self.mem(id);
        f(mem.data_mut(&mut self.inner))
    }

    /// Grows by `delta` pages; returns the old page count.
    pub fn memory_grow(&mut self, id: MemoryId, delta: u64) -> Result<u64, String> {
        let mem = self.mem(id);
        let old = mem
            .grow(&mut self.inner, delta)
            .map_err(|e| e.to_string())?;
        sync_memories(self.state(), &self.inner);
        Ok(old)
    }

    // ---- table --------------------------------------------------------

    pub fn new_table(
        &mut self,
        element: ValType,
        min: u32,
        max: Option<u32>,
        init: &Val,
    ) -> Result<TableId, String> {
        let init = to_wasmi(&mut self.inner, init);
        let ty = wasmi::TableType::new(wasmi_val_type(element), min, max);
        let table = wasmi::Table::new(&mut self.inner, ty, init).map_err(|e| e.to_string())?;
        Ok(TableId(intern_slot(&mut self.state_mut().tables, table)))
    }

    pub fn table_size(&self, id: TableId) -> u64 {
        self.table(id).size(&self.inner)
    }

    pub fn table_get(&mut self, id: TableId, index: u64) -> Option<Val> {
        let value = self.table(id).get(&self.inner, index)?;
        Some(from_wasmi(&mut self.inner, &value))
    }

    pub fn table_set(&mut self, id: TableId, index: u64, value: &Val) -> Result<(), String> {
        let value = to_wasmi(&mut self.inner, value);
        self.table(id)
            .set(&mut self.inner, index, value)
            .map_err(|e| e.to_string())
    }

    /// Grows by `delta` slots filled with `init`; returns the old length.
    pub fn table_grow(&mut self, id: TableId, delta: u64, init: &Val) -> Result<u64, String> {
        let init = to_wasmi(&mut self.inner, init);
        self.table(id)
            .grow(&mut self.inner, delta, init)
            .map_err(|e| e.to_string())
    }

    // ---- global -------------------------------------------------------

    pub fn new_global(&mut self, mutable: bool, init: &Val) -> GlobalId {
        let mutability = if mutable {
            wasmi::Mutability::Var
        } else {
            wasmi::Mutability::Const
        };
        let init = to_wasmi(&mut self.inner, init);
        let global = wasmi::Global::new(&mut self.inner, init, mutability);
        GlobalId(intern_slot(&mut self.state_mut().globals, global))
    }

    pub fn global_get(&mut self, id: GlobalId) -> Val {
        let value = self.global(id).get(&self.inner);
        from_wasmi(&mut self.inner, &value)
    }

    pub fn global_set(&mut self, id: GlobalId, value: &Val) -> Result<(), String> {
        let global = self.global(id);
        if !matches!(global.ty(&self.inner).mutability(), wasmi::Mutability::Var) {
            return Err("Can't set the value of an immutable global.".to_string());
        }
        let value = to_wasmi(&mut self.inner, value);
        global
            .set(&mut self.inner, value)
            .map_err(|e| e.to_string())
    }

    // ---- functions and instances ---------------------------------------

    /// Parameter and result types of a function.
    pub fn func_type(&self, id: FuncId) -> (Vec<ValType>, Vec<ValType>) {
        let ty = self.func(id).ty(&self.inner);
        (
            ty.params().iter().map(|t| val_type(*t)).collect(),
            ty.results().iter().map(|t| val_type(*t)).collect(),
        )
    }

    /// Calls a function. Host imports invoked on the way use `host`.
    pub fn call_func(
        &mut self,
        id: FuncId,
        args: &[Val],
        host: &mut dyn HostCallbacks,
    ) -> Result<Vec<Val>, CallError> {
        let func = self.func(id);
        let ty = func.ty(&self.inner);
        if args.len() != ty.params().len() {
            return Err(CallError::Argument(format!(
                "expected {} arguments, got {}",
                ty.params().len(),
                args.len()
            )));
        }
        let inputs: Vec<wasmi::Val> = args.iter().map(|a| to_wasmi(&mut self.inner, a)).collect();
        let mut outputs = vec![wasmi::Val::I32(0); ty.results().len()];
        let previous = self.set_host(host);
        let result = func.call(&mut self.inner, &inputs, &mut outputs);
        self.state_mut().host = previous;
        sync_memories(self.state(), &self.inner);
        result.map_err(map_error)?;
        Ok(outputs
            .iter()
            .map(|v| from_wasmi(&mut self.inner, v))
            .collect())
    }

    /// Calls an exported function by name.
    pub fn call_export(
        &mut self,
        instance: InstanceId,
        name: &str,
        args: &[Val],
        host: &mut dyn HostCallbacks,
    ) -> Result<Vec<Val>, CallError> {
        match self.export(instance, name) {
            Some(Extern::Func(id)) => self.call_func(id, args, host),
            Some(_) => Err(CallError::Argument(format!(
                "export '{name}' is not a function"
            ))),
            None => Err(CallError::Argument(format!("no export named '{name}'"))),
        }
    }

    pub fn exports(&self, instance: InstanceId) -> &[(String, Extern)] {
        &self.state().instances[instance.0 as usize]
    }

    pub fn export(&self, instance: InstanceId, name: &str) -> Option<Extern> {
        self.exports(instance)
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, e)| *e)
    }

    /// Instantiates `module` with one entry of `imports` per module import.
    /// The start function (if any) runs before this returns.
    pub fn instantiate(
        &mut self,
        module: &Module,
        imports: &[Import],
        host: &mut dyn HostCallbacks,
    ) -> Result<InstanceId, InstantiateError> {
        self.instantiate_inner(module, imports, host, false)
    }

    pub(crate) fn instantiate_inner(
        &mut self,
        module: &Module,
        imports: &[Import],
        host: &mut dyn HostCallbacks,
        wasi: bool,
    ) -> Result<InstanceId, InstantiateError> {
        if imports.len() != module.imports.len() {
            return Err(InstantiateError::Link(LinkError {
                type_error: true,
                message: format!(
                    "expected {} imports, got {}",
                    module.imports.len(),
                    imports.len()
                ),
            }));
        }
        let mut linker = wasmi::Linker::<HostState>::new(&self.inner.engine().clone());
        #[cfg(feature = "wasi")]
        if wasi {
            wasi::add_to_linker(&mut linker).map_err(|e| link_error(false, e.to_string()))?;
        }
        #[cfg(not(feature = "wasi"))]
        let _ = wasi;

        for (index, (desc, import)) in module.imports.iter().zip(imports).enumerate() {
            let is_wasi = cfg!(feature = "wasi") && wasi && desc.module == "wasi_snapshot_preview1";
            if is_wasi && matches!(import, Import::Missing { .. }) {
                continue;
            }
            if linker.get(&self.inner, &desc.module, &desc.name).is_some() {
                continue;
            }
            let external = self.resolve_import(index, desc, import)?;
            linker
                .define(&desc.module, &desc.name, external)
                .map_err(|e| {
                    link_error(
                        false,
                        format!(
                            "Import #{index} module=\"{}\" function=\"{}\": {e}",
                            desc.module, desc.name
                        ),
                    )
                })?;
        }

        let previous = self.set_host(host);
        let result = linker.instantiate_and_start(&mut self.inner, &module.inner);
        self.state_mut().host = previous;
        let instance = match result {
            Ok(instance) => instance,
            Err(error) => {
                sync_memories(self.state(), &self.inner);
                return Err(InstantiateError::Call(map_error(error)));
            }
        };
        sync_memories(self.state(), &self.inner);

        let mut exports = Vec::with_capacity(module.exports.len());
        for export in &module.exports {
            if let Some(ext) = instance.get_export(&self.inner, &export.name) {
                let value = self.intern_extern(ext);
                exports.push((export.name.clone(), value));
            }
        }
        self.state_mut().instances.push(exports);
        Ok(InstanceId((self.state().instances.len() - 1) as u32))
    }

    // ---- internals ------------------------------------------------------

    fn state(&self) -> &HostState {
        self.inner.data()
    }

    fn state_mut(&mut self) -> &mut HostState {
        self.inner.data_mut()
    }

    /// Installs `host` for the duration of one entry into wasm. Returns the
    /// previous pointer, which the caller restores afterwards.
    fn set_host(&mut self, host: &mut dyn HostCallbacks) -> Option<NonNull<dyn HostCallbacks>> {
        // SAFETY: the pointer is only dereferenced by host closures, which run
        // only while the `&mut` borrow passed here is still alive (the caller
        // restores the previous pointer before returning).
        let erased: &'static mut dyn HostCallbacks = unsafe { std::mem::transmute(host) };
        self.state_mut().host.replace(NonNull::from(erased))
    }

    fn resolve_import(
        &mut self,
        index: usize,
        desc: &ImportDesc,
        import: &Import,
    ) -> Result<wasmi::Extern, InstantiateError> {
        let at = format!("Import #{index} module=\"{}\" ", desc.module);
        let named = format!("{at}function=\"{}\"", desc.name);
        let link = |message: String| {
            InstantiateError::Link(LinkError {
                type_error: false,
                message,
            })
        };
        match (import, &desc.kind) {
            (Import::Missing { module_exists: false }, _) => Err(InstantiateError::Link(LinkError {
                type_error: true,
                message: format!("Import #{index} module=\"{}\": module is not an object or function", desc.module),
            })),
            (Import::Missing { .. }, ExternKind::Func { .. }) => {
                Err(link(format!("{named}: function import requires a callable")))
            }
            (Import::Missing { .. }, ExternKind::Memory { .. }) => {
                Err(link(format!("{at}memory=\"{}\": memory import must be a WebAssembly.Memory object", desc.name)))
            }
            (Import::Missing { .. }, ExternKind::Table { .. }) => {
                Err(link(format!("{at}table=\"{}\": table import requires a WebAssembly.Table", desc.name)))
            }
            (Import::Missing { .. }, ExternKind::Global { .. }) => Err(link(format!(
                "{at}global=\"{}\": global import must be a number, valid Wasm reference, or WebAssembly.Global object",
                desc.name
            ))),
            (Import::Host(id), ExternKind::Func { params, results }) => {
                let ty = wasmi::FuncType::new(
                    params.iter().map(|t| wasmi_val_type(*t)),
                    results.iter().map(|t| wasmi_val_type(*t)),
                );
                Ok(wasmi::Extern::Func(self.host_func(*id, ty)))
            }
            (Import::Func(id), ExternKind::Func { params, results }) => {
                let want = (params.clone(), results.clone());
                if self.func_type(*id) != want {
                    return Err(link(format!("{named}: imported function does not match the expected type")));
                }
                Ok(wasmi::Extern::Func(self.func(*id)))
            }
            (Import::Memory(id), ExternKind::Memory { .. }) => Ok(wasmi::Extern::Memory(self.mem(*id))),
            (Import::Table(id), ExternKind::Table { .. }) => Ok(wasmi::Extern::Table(self.table(*id))),
            (Import::Global(id), ExternKind::Global { .. }) => Ok(wasmi::Extern::Global(self.global(*id))),
            (Import::GlobalValue(value), ExternKind::Global { mutable, .. }) => {
                let value = to_wasmi(&mut self.inner, value);
                let mutability = if *mutable { wasmi::Mutability::Var } else { wasmi::Mutability::Const };
                Ok(wasmi::Extern::Global(wasmi::Global::new(&mut self.inner, value, mutability)))
            }
            _ => Err(link(format!("{named}: import kind does not match the module"))),
        }
    }

    /// A wasmi host function that forwards to the embedder's `HostCallbacks`.
    fn host_func(&mut self, id: u32, ty: wasmi::FuncType) -> wasmi::Func {
        wasmi::Func::new(
            &mut self.inner,
            ty,
            move |mut caller: wasmi::Caller<'_, HostState>,
                  params: &[wasmi::Val],
                  results: &mut [wasmi::Val]| {
                sync_memories(caller.data(), caller.as_context());
                let args: Vec<Val> = params.iter().map(|p| from_wasmi(&mut caller, p)).collect();
                let host = caller
                    .data()
                    .host
                    .ok_or_else(|| wasmi::Error::new("no host callbacks installed"))?;
                // SAFETY: installed by `set_host` for this entry into wasm; the
                // embedder's `&mut` is alive until that entry returns.
                let outcome = unsafe { (*host.as_ptr()).call_import(id, &args) };
                match outcome {
                    Ok(values) => {
                        if values.len() != results.len() {
                            return Err(wasmi::Error::new(format!(
                                "host function returned {} values, expected {}",
                                values.len(),
                                results.len()
                            )));
                        }
                        for (slot, value) in results.iter_mut().zip(&values) {
                            *slot = to_wasmi(&mut caller, value);
                        }
                        Ok(())
                    }
                    Err(exception) => Err(wasmi::Error::host(HostException(exception))),
                }
            },
        )
    }

    fn intern_memory(
        &mut self,
        mem: wasmi::Memory,
        external: Option<memory::External>,
    ) -> MemoryId {
        let key = format!("{mem:?}");
        let pages = mem.size(&self.inner);
        let memories = &mut self.state_mut().memories;
        let index = match memories.iter().position(|e| e.key == key) {
            Some(index) => index,
            None => {
                memories.push(MemEntry {
                    key,
                    mem,
                    pages: Cell::new(pages),
                    generation: Cell::new(0),
                    external,
                });
                memories.len() - 1
            }
        };
        MemoryId(index as u32)
    }

    fn intern_extern(&mut self, ext: wasmi::Extern) -> Extern {
        match ext {
            wasmi::Extern::Func(f) => {
                Extern::Func(FuncId(intern_slot(&mut self.state_mut().funcs, f)))
            }
            wasmi::Extern::Memory(m) => Extern::Memory(self.intern_memory(m, None)),
            wasmi::Extern::Table(t) => {
                Extern::Table(TableId(intern_slot(&mut self.state_mut().tables, t)))
            }
            wasmi::Extern::Global(g) => {
                Extern::Global(GlobalId(intern_slot(&mut self.state_mut().globals, g)))
            }
        }
    }

    fn func(&self, id: FuncId) -> wasmi::Func {
        self.state().funcs[id.0 as usize].item
    }

    fn mem(&self, id: MemoryId) -> wasmi::Memory {
        self.state().memories[id.0 as usize].mem
    }

    fn table(&self, id: TableId) -> wasmi::Table {
        self.state().tables[id.0 as usize].item
    }

    fn global(&self, id: GlobalId) -> wasmi::Global {
        self.state().globals[id.0 as usize].item
    }
}

/// Index of `item` in `slots`, pushing it on first sight.
fn intern_slot<T: Copy + std::fmt::Debug>(slots: &mut Vec<Slot<T>>, item: T) -> u32 {
    let key = format!("{item:?}");
    if let Some(index) = slots.iter().position(|slot| slot.key == key) {
        return index as u32;
    }
    slots.push(Slot { key, item });
    (slots.len() - 1) as u32
}

fn link_error(type_error: bool, message: String) -> InstantiateError {
    InstantiateError::Link(LinkError {
        type_error,
        message,
    })
}

/// Refreshes page counts and bumps generations of memories that changed size.
fn sync_memories(state: &HostState, ctx: impl AsContext<Data = HostState>) {
    for entry in &state.memories {
        let pages = entry.mem.size(&ctx);
        if pages != entry.pages.get() {
            entry.pages.set(pages);
            entry.generation.set(entry.generation.get() + 1);
        }
        if let Some(external) = &entry.external {
            external.sync(entry.mem.data_size(&ctx));
        }
    }
}

fn to_wasmi(ctx: &mut impl AsContextMut<Data = HostState>, value: &Val) -> wasmi::Val {
    match value {
        Val::I32(v) => wasmi::Val::I32(*v),
        Val::I64(v) => wasmi::Val::I64(*v),
        Val::F32(v) => wasmi::Val::F32(wasmi::F32::from_float(*v)),
        Val::F64(v) => wasmi::Val::F64(wasmi::F64::from_float(*v)),
        Val::V128(bits) => wasmi::Val::V128(wasmi::V128::from(*bits)),
        Val::FuncRef(None) => wasmi::Val::FuncRef(Ref::Null),
        Val::FuncRef(Some(id)) => {
            let func = ctx.as_context().data().funcs[id.0 as usize].item;
            wasmi::Val::FuncRef(Ref::Val(func))
        }
        Val::ExternRef(None) => wasmi::Val::ExternRef(Ref::Null),
        Val::ExternRef(Some(handle)) => {
            wasmi::Val::ExternRef(Ref::Val(wasmi::ExternRef::new::<u32>(&mut *ctx, *handle)))
        }
    }
}

fn from_wasmi(ctx: &mut impl AsContextMut<Data = HostState>, value: &wasmi::Val) -> Val {
    match value {
        wasmi::Val::I32(v) => Val::I32(*v),
        wasmi::Val::I64(v) => Val::I64(*v),
        wasmi::Val::F32(v) => Val::F32(v.to_float()),
        wasmi::Val::F64(v) => Val::F64(v.to_float()),
        wasmi::Val::V128(v) => Val::V128(v.as_u128()),
        wasmi::Val::FuncRef(Ref::Null) => Val::FuncRef(None),
        wasmi::Val::FuncRef(Ref::Val(func)) => Val::FuncRef(Some(FuncId(intern_slot(
            &mut ctx.as_context_mut().data_mut().funcs,
            *func,
        )))),
        wasmi::Val::ExternRef(Ref::Null) => Val::ExternRef(None),
        wasmi::Val::ExternRef(Ref::Val(object)) => {
            let handle = object.data(ctx.as_context()).downcast_ref::<u32>().copied();
            Val::ExternRef(handle)
        }
    }
}

fn map_error(error: wasmi::Error) -> CallError {
    if let Some(exception) = error.downcast_ref::<HostException>() {
        return CallError::Exception(exception.0);
    }
    if let Some(code) = error.i32_exit_status() {
        return CallError::Exit(code);
    }
    if let Some(code) = error.as_trap_code() {
        return CallError::Trap(match code {
            wasmi::TrapCode::UnreachableCodeReached => Trap::Unreachable,
            wasmi::TrapCode::MemoryOutOfBounds => Trap::MemoryOutOfBounds,
            wasmi::TrapCode::TableOutOfBounds => Trap::TableOutOfBounds,
            wasmi::TrapCode::IndirectCallToNull | wasmi::TrapCode::BadSignature => {
                Trap::IndirectCallMismatch
            }
            wasmi::TrapCode::IntegerDivisionByZero => Trap::DivideByZero,
            wasmi::TrapCode::IntegerOverflow => Trap::DivideOverflow,
            wasmi::TrapCode::BadConversionToInteger => Trap::FloatUnrepresentable,
            wasmi::TrapCode::StackOverflow => Trap::StackOverflow,
            other => Trap::Other(other.to_string()),
        });
    }
    CallError::Trap(Trap::Other(error.to_string()))
}
