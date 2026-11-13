//! `__thaw_wasm_*` natives over `thaw-wasm` (cargo feature `wasm`). The JS-visible `WebAssembly`
//! object is built on these by wasm.js (W2b). Handles are the engine's ids (small integers), one
//! table set per Interp. Engine failures throw `{__wasmErrorKind, message}` (CompileError,
//! LinkError, RuntimeError, or Exit with `code`); TypeError and RangeError are thrown as real errors.
//! Values: i32 -> Number, i64 -> BigInt, f32/f64 -> Number, funcref -> Number handle or null,
//! externref -> the JS value itself (or null). Multi-value results are arrays.
//!
//! Re-entry: while a call runs, the engine is borrowed. A host import that calls back into a
//! `__thaw_wasm_*` native gets a TypeError instead (ponytail: re-entrant store access, add when needed).

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use num_bigint::BigInt;
use num_traits::ToPrimitive;
#[cfg(feature = "wasi")]
use thaw_wasm::wasi::WasiOptions;
use thaw_wasm::{
    CallError, CompileError, Engine, Extern, ExternKind, FuncId, GlobalId, HostCallbacks, Import,
    ImportDesc, InstanceId, InstantiateError, LinkError, MemoryId, Module as WModule,
    Store as WStore, TableId, Trap, Val, ValType,
};

use crate::typed::{same_shared, shared_bytes, Bytes};
use crate::{arg, put, to_int32, Ctl, Interp, Obj, Value, R};

const BUSY: &str = "WebAssembly is busy: re-entering it from a host import is not supported";
const INCOMPAT: &str = "type incompatibility when transforming from/to JS";

/// A JS function imported by a module (`Import::Host(index)`).
struct Host {
    func: Value,
    results: Vec<ValType>,
}

/// Interp-side tables. Kept apart from the engine store so a host call can use them while the
/// engine is borrowed.
#[derive(Default)]
struct Rt {
    modules: HashMap<u32, Rc<WModule>>,
    instances: HashMap<u32, Rc<WModule>>,
    hosts: Vec<Host>,
    /// Externref values by handle; `None` once released.
    externs: Vec<Option<Value>>,
    /// Exceptions thrown by host imports, indexed by the id handed to the engine. Cleared per call.
    pending: Vec<Ctl>,
    global_types: HashMap<u32, (ValType, bool)>,
    table_types: HashMap<u32, ValType>,
    /// Last ArrayBuffer handed out for each memory, with its storage (identity stable until grow).
    buffers: HashMap<u32, (Obj, Bytes)>,
    next_module: u32,
}

struct Wasm {
    engine: Engine,
    store: WStore,
    rt: Rt,
}

/// Engine calls back into JS through this, with the interpreter lent for the duration of the call.
struct Callbacks<'a> {
    it: &'a mut Interp,
    rt: &'a mut Rt,
}

impl HostCallbacks for Callbacks<'_> {
    fn call_import(&mut self, id: u32, args: &[Val]) -> Result<Vec<Val>, u32> {
        match self.invoke(id, args) {
            Ok(values) => Ok(values),
            Err(ctl) => {
                self.rt.pending.push(ctl);
                Err((self.rt.pending.len() - 1) as u32)
            }
        }
    }
}

impl Callbacks<'_> {
    fn invoke(&mut self, id: u32, args: &[Val]) -> R<Vec<Val>> {
        let (func, results) = {
            let host = &self.rt.hosts[id as usize];
            (host.func.clone(), host.results.clone())
        };
        let mut js = Vec::with_capacity(args.len());
        for v in args {
            js.push(to_js(self.it, self.rt, v)?);
        }
        let ret = self.it.call(&func, &Value::Undefined, &js)?;
        match results.len() {
            0 => Ok(Vec::new()),
            1 => Ok(vec![to_wasm(self.it, self.rt, &ret, results[0])?]),
            n => {
                if !matches!(ret, Value::Object(_)) {
                    return self
                        .it
                        .throw_error("TypeError", "multi-return value must be iterable");
                }
                let mut out = Vec::with_capacity(n);
                for (i, ty) in results.iter().enumerate() {
                    let item = self.it.get(&ret, &i.to_string())?;
                    out.push(to_wasm(self.it, self.rt, &item, *ty)?);
                }
                Ok(out)
            }
        }
    }
}

/// Installs the natives on `it`'s global object.
pub(crate) fn install(it: &mut Interp) {
    let engine = Engine::default();
    let store = WStore::new(&engine);
    let state = Rc::new(RefCell::new(Wasm {
        engine,
        store,
        rt: Rt::default(),
    }));
    let def = |it: &mut Interp, name: &str, f: fn(&mut Interp, &mut Wasm, &[Value]) -> R<Value>| {
        let st = state.clone();
        let native = it.native(move |it, _, a| {
            let Ok(mut w) = st.try_borrow_mut() else {
                return it.throw_error("TypeError", BUSY);
            };
            f(it, &mut w, a)
        });
        it.global.clone().declare(name, native, true);
    };
    def(it, "__thaw_wasm_validate", n_validate);
    def(it, "__thaw_wasm_compile", n_compile);
    def(it, "__thaw_wasm_module_imports", n_module_imports);
    def(it, "__thaw_wasm_module_exports", n_module_exports);
    def(it, "__thaw_wasm_module_custom_sections", n_custom_sections);
    def(it, "__thaw_wasm_instantiate", n_instantiate);
    def(it, "__thaw_wasm_instance_exports", n_instance_exports);
    def(it, "__thaw_wasm_call", n_call);
    def(it, "__thaw_wasm_call_func", n_call_func);
    def(it, "__thaw_wasm_func_type", n_func_type);
    def(it, "__thaw_wasm_memory_create", n_memory_create);
    def(it, "__thaw_wasm_memory_buffer", n_memory_buffer);
    def(it, "__thaw_wasm_memory_grow", n_memory_grow);
    def(it, "__thaw_wasm_memory_size", n_memory_size);
    def(it, "__thaw_wasm_table_create", n_table_create);
    def(it, "__thaw_wasm_table_length", n_table_length);
    def(it, "__thaw_wasm_table_get", n_table_get);
    def(it, "__thaw_wasm_table_set", n_table_set);
    def(it, "__thaw_wasm_table_grow", n_table_grow);
    def(it, "__thaw_wasm_global_create", n_global_create);
    def(it, "__thaw_wasm_global_get", n_global_get);
    def(it, "__thaw_wasm_global_set", n_global_set);
    def(it, "__thaw_wasm_release", n_release);
    #[cfg(feature = "wasi")]
    def(it, "__thaw_wasm_instantiate_wasi", n_instantiate_wasi);
}

// ---- argument and value helpers ------------------------------------------

fn num(it: &mut Interp, a: &[Value], i: usize) -> f64 {
    it.num(&arg(a, i))
}

fn handle(it: &mut Interp, a: &[Value], i: usize) -> u32 {
    num(it, a, i) as u32
}

fn text(it: &mut Interp, a: &[Value], i: usize) -> String {
    it.to_str(&arg(a, i)).to_string()
}

/// The optional API prefix (e.g. `WebAssembly.Module()`) that V8 puts before every message.
fn prefix(it: &mut Interp, a: &[Value], i: usize, default: &str) -> String {
    match arg(a, i) {
        Value::Undefined => default.to_string(),
        v => it.to_str(&v).to_string(),
    }
}

fn bytes_of(it: &mut Interp, a: &[Value], prefix: &str) -> R<Vec<u8>> {
    match Interp::view_bytes(&arg(a, 0)) {
        Some(bytes) => Ok(bytes),
        None => it.throw_error(
            "TypeError",
            &format!("{prefix}: Argument 0 must be a buffer source"),
        ),
    }
}

/// `{__wasmErrorKind, message}`: the shim turns it into the matching WebAssembly error class.
fn plain_error(it: &mut Interp, kind: &str, message: &str) -> Ctl {
    let obj = it.plain();
    put(&obj, "__wasmErrorKind", Value::str(kind));
    put(&obj, "message", Value::str(message));
    Ctl::Throw(Value::Object(obj))
}

/// The Ctl of a real error thrown through the interpreter.
fn error_ctl(it: &mut Interp, class: &str, message: &str) -> Ctl {
    match it.throw_error::<()>(class, message) {
        Err(ctl) => ctl,
        Ok(()) => unreachable!("throw_error always fails"),
    }
}

fn type_name(ty: ValType) -> &'static str {
    match ty {
        ValType::I32 => "i32",
        ValType::I64 => "i64",
        ValType::F32 => "f32",
        ValType::F64 => "f64",
        ValType::V128 => "v128",
        ValType::FuncRef => "anyfunc",
        ValType::ExternRef => "externref",
    }
}

fn parse_type(name: &str) -> Option<ValType> {
    Some(match name {
        "i32" => ValType::I32,
        "i64" => ValType::I64,
        "f32" => ValType::F32,
        "f64" => ValType::F64,
        "anyfunc" | "funcref" => ValType::FuncRef,
        "externref" => ValType::ExternRef,
        _ => return None,
    })
}

fn kind_name(kind: &ExternKind) -> &'static str {
    match kind {
        ExternKind::Func { .. } => "function",
        ExternKind::Memory { .. } => "memory",
        ExternKind::Table { .. } => "table",
        ExternKind::Global { .. } => "global",
    }
}

/// Low 64 bits of a BigInt, two's complement (ToBigInt64).
fn wrap_i64(big: &BigInt) -> i64 {
    let mask = BigInt::from(u64::MAX);
    (big & &mask).to_u64().unwrap_or(0) as i64
}

/// JS -> wasm (ToWebAssemblyValue). Numeric coercions can run user code (valueOf), as in V8.
fn to_wasm(it: &mut Interp, rt: &mut Rt, v: &Value, ty: ValType) -> R<Val> {
    Ok(match ty {
        ValType::I32 => Val::I32(to_int32(it.num(v))),
        ValType::I64 => Val::I64(wrap_i64(&it.bigint_element_of(v)?)),
        ValType::F32 => Val::F32(it.num(v) as f32),
        ValType::F64 => Val::F64(it.num(v)),
        ValType::V128 => return it.throw_error("TypeError", INCOMPAT),
        ValType::FuncRef => match v {
            Value::Null => Val::FuncRef(None),
            Value::Number(n) => Val::FuncRef(Some(FuncId::from_index(*n as u32))),
            _ => return it.throw_error("TypeError", INCOMPAT),
        },
        ValType::ExternRef => match v {
            Value::Null => Val::ExternRef(None),
            _ => {
                rt.externs.push(Some(v.clone()));
                Val::ExternRef(Some((rt.externs.len() - 1) as u32))
            }
        },
    })
}

/// wasm -> JS (ToJSValue).
fn to_js(it: &mut Interp, rt: &Rt, v: &Val) -> R<Value> {
    Ok(match v {
        Val::I32(n) => Value::Number(f64::from(*n)),
        Val::I64(n) => crate::bigint::big(BigInt::from(*n)),
        Val::F32(f) => Value::Number(f64::from(*f)),
        Val::F64(f) => Value::Number(*f),
        Val::V128(_) => return it.throw_error("TypeError", INCOMPAT),
        Val::FuncRef(None) | Val::ExternRef(None) => Value::Null,
        Val::FuncRef(Some(id)) => Value::Number(f64::from(id.index())),
        Val::ExternRef(Some(h)) => rt
            .externs
            .get(*h as usize)
            .cloned()
            .flatten()
            .unwrap_or(Value::Undefined),
    })
}

/// Elements of an array-like argument (anything else is empty).
fn list_of(it: &mut Interp, v: &Value) -> R<Vec<Value>> {
    if !matches!(v, Value::Object(_)) {
        return Ok(Vec::new());
    }
    let len = it.get(v, "length")?;
    let len = it.num(&len) as usize;
    let mut out = Vec::with_capacity(len);
    for i in 0..len {
        out.push(it.get(v, &i.to_string())?);
    }
    Ok(out)
}

fn put_str(o: &Obj, key: &str, value: &str) {
    put(o, key, Value::str(value));
}

// ---- errors from the engine ----------------------------------------------

fn compile_error(it: &mut Interp, prefix: &str, error: CompileError) -> Ctl {
    plain_error(it, "CompileError", &format!("{prefix}: {}", error.message))
}

fn call_error(it: &mut Interp, rt: &mut Rt, error: CallError) -> Ctl {
    match error {
        CallError::Trap(Trap::StackOverflow) => {
            error_ctl(it, "RangeError", Trap::StackOverflow.message())
        }
        CallError::Trap(trap) => plain_error(it, "RuntimeError", trap.message()),
        CallError::Exception(id) if (id as usize) < rt.pending.len() => {
            rt.pending.swap_remove(id as usize)
        }
        CallError::Exception(_) => plain_error(it, "RuntimeError", "host exception was lost"),
        CallError::Exit(code) => {
            let obj = it.plain();
            put_str(&obj, "__wasmErrorKind", "Exit");
            put(&obj, "code", Value::Number(f64::from(code)));
            Ctl::Throw(Value::Object(obj))
        }
        CallError::Argument(message) => error_ctl(it, "TypeError", &message),
    }
}

fn instantiate_error(it: &mut Interp, rt: &mut Rt, prefix: &str, error: InstantiateError) -> Ctl {
    match error {
        InstantiateError::Link(LinkError {
            type_error,
            message,
        }) => {
            let message = format!("{prefix}: {message}");
            if type_error {
                error_ctl(it, "TypeError", &message)
            } else {
                plain_error(it, "LinkError", &message)
            }
        }
        InstantiateError::Call(error) => call_error(it, rt, error),
    }
}

// ---- modules --------------------------------------------------------------

fn compile_module(it: &mut Interp, w: &mut Wasm, a: &[Value]) -> R<u32> {
    let prefix = prefix(it, a, 1, "WebAssembly.Module()");
    let bytes = bytes_of(it, a, &prefix)?;
    match WModule::compile(&w.engine, &bytes) {
        Ok(module) => {
            let id = w.rt.next_module;
            w.rt.next_module += 1;
            w.rt.modules.insert(id, Rc::new(module));
            Ok(id)
        }
        Err(error) => Err(compile_error(it, &prefix, error)),
    }
}

fn module_of(it: &mut Interp, w: &Wasm, a: &[Value]) -> R<Rc<WModule>> {
    let id = handle(it, a, 0);
    match w.rt.modules.get(&id) {
        Some(module) => Ok(module.clone()),
        None => it.throw_error("TypeError", "WebAssembly.Module: module was released"),
    }
}

fn n_validate(it: &mut Interp, w: &mut Wasm, a: &[Value]) -> R<Value> {
    let bytes = bytes_of(it, a, "WebAssembly.validate()")?;
    Ok(Value::Bool(WModule::validate(&w.engine, &bytes).is_ok()))
}

fn n_compile(it: &mut Interp, w: &mut Wasm, a: &[Value]) -> R<Value> {
    Ok(Value::Number(f64::from(compile_module(it, w, a)?)))
}

fn n_module_imports(it: &mut Interp, w: &mut Wasm, a: &[Value]) -> R<Value> {
    let module = module_of(it, w, a)?;
    let mut items = Vec::new();
    for import in module.imports() {
        let o = it.plain();
        put_str(&o, "module", &import.module);
        put_str(&o, "name", &import.name);
        put_str(&o, "kind", kind_name(&import.kind));
        items.push(Value::Object(o));
    }
    Ok(it.array(items))
}

fn n_module_exports(it: &mut Interp, w: &mut Wasm, a: &[Value]) -> R<Value> {
    let module = module_of(it, w, a)?;
    let mut items = Vec::new();
    for export in module.exports() {
        let o = it.plain();
        put_str(&o, "name", &export.name);
        put_str(&o, "kind", kind_name(&export.kind));
        items.push(Value::Object(o));
    }
    Ok(it.array(items))
}

fn n_custom_sections(it: &mut Interp, w: &mut Wasm, a: &[Value]) -> R<Value> {
    let module = module_of(it, w, a)?;
    let name = text(it, a, 1);
    let sections: Vec<Vec<u8>> = module
        .custom_sections(&name)
        .into_iter()
        .map(<[u8]>::to_vec)
        .collect();
    let items = sections
        .into_iter()
        .map(|bytes| it.array_buffer_from_vec(bytes))
        .collect();
    Ok(it.array(items))
}

// ---- instances ------------------------------------------------------------

/// One `Import` per module import, from the linkage array `[module, field, kind, value]` (the
/// names are only for messages: the module's own names decide). Kinds: `function` (a JS callable;
/// anything else is a missing import), `wasmFunction` / `memory` / `table` / `global` (engine handles),
/// `globalValue` (a plain value for a new global), `missing` and `missingModule`.
fn build_imports(
    it: &mut Interp,
    rt: &mut Rt,
    descs: &[ImportDesc],
    linkage: &Value,
) -> R<Vec<Import>> {
    let mut imports = Vec::with_capacity(descs.len());
    for (i, desc) in descs.iter().enumerate() {
        let entry = it.get(linkage, &i.to_string())?;
        let kind = {
            let v = it.get(&entry, "2")?;
            it.to_str(&v).to_string()
        };
        let value = it.get(&entry, "3")?;
        let id = || match value {
            Value::Number(n) => n as u32,
            _ => 0,
        };
        let import = match kind.as_str() {
            "missing" => Import::Missing {
                module_exists: true,
            },
            "missingModule" => Import::Missing {
                module_exists: false,
            },
            "function" if value.is_callable() => {
                let results = match &desc.kind {
                    ExternKind::Func { results, .. } => results.clone(),
                    _ => Vec::new(),
                };
                rt.hosts.push(Host {
                    func: value,
                    results,
                });
                Import::Host((rt.hosts.len() - 1) as u32)
            }
            "function" => Import::Missing {
                module_exists: true,
            },
            "wasmFunction" => Import::Func(FuncId::from_index(id())),
            "memory" => Import::Memory(MemoryId::from_index(id())),
            "table" => Import::Table(TableId::from_index(id())),
            "global" => Import::Global(GlobalId::from_index(id())),
            "globalValue" => {
                let ty = match &desc.kind {
                    ExternKind::Global { ty, .. } => *ty,
                    _ => ValType::I32,
                };
                Import::GlobalValue(to_wasm(it, rt, &value, ty)?)
            }
            _ => return it.throw_error("TypeError", "unknown import kind"),
        };
        imports.push(import);
    }
    Ok(imports)
}

fn instantiate_module(
    it: &mut Interp,
    w: &mut Wasm,
    a: &[Value],
    #[cfg(feature = "wasi")] wasi: Option<WasiOptions>,
) -> R<Value> {
    let prefix = prefix(it, a, 2, "WebAssembly.Instance()");
    let module = module_of(it, w, a)?;
    let Wasm { store, rt, .. } = w;
    let imports = build_imports(it, rt, module.imports(), &arg(a, 1))?;
    rt.pending.clear();
    let result = {
        let mut cb = Callbacks {
            it: &mut *it,
            rt: &mut *rt,
        };
        #[cfg(feature = "wasi")]
        match &wasi {
            Some(options) => store.instantiate_with_wasi(&module, &imports, &mut cb, options),
            None => store.instantiate(&module, &imports, &mut cb),
        }
        #[cfg(not(feature = "wasi"))]
        store.instantiate(&module, &imports, &mut cb)
    };
    match result {
        Ok(instance) => {
            let id = instance.index();
            rt.instances.insert(id, module);
            Ok(Value::Number(f64::from(id)))
        }
        Err(error) => Err(instantiate_error(it, rt, &prefix, error)),
    }
}

fn n_instantiate(it: &mut Interp, w: &mut Wasm, a: &[Value]) -> R<Value> {
    #[cfg(feature = "wasi")]
    return instantiate_module(it, w, a, None);
    #[cfg(not(feature = "wasi"))]
    instantiate_module(it, w, a)
}

#[cfg(feature = "wasi")]
fn n_instantiate_wasi(it: &mut Interp, w: &mut Wasm, a: &[Value]) -> R<Value> {
    // options: {args: [string], env: [[k, v]], preopens: [[guestPath, hostPath]]}
    let options = arg(a, 2);
    let mut wasi = WasiOptions {
        args: Vec::new(),
        env: Vec::new(),
        preopens: Vec::new(),
    };
    let args = it.get(&options, "args")?;
    for v in list_of(it, &args)? {
        wasi.args.push(it.to_str(&v).to_string());
    }
    let env = it.get(&options, "env")?;
    for pair in list_of(it, &env)? {
        let k = it.get(&pair, "0")?;
        let v = it.get(&pair, "1")?;
        wasi.env
            .push((it.to_str(&k).to_string(), it.to_str(&v).to_string()));
    }
    let preopens = it.get(&options, "preopens")?;
    for pair in list_of(it, &preopens)? {
        let guest = it.get(&pair, "0")?;
        let host = it.get(&pair, "1")?;
        wasi.preopens.push((
            it.to_str(&guest).to_string(),
            std::path::PathBuf::from(it.to_str(&host).to_string()),
        ));
    }
    instantiate_module(
        it,
        w,
        &[arg(a, 0), arg(a, 1), Value::str("WebAssembly.Instance()")],
        Some(wasi),
    )
}

/// `[{name, kind, handle, ...}]` for the instance's exports, in declaration order. Functions carry
/// `params`/`results`; globals `type`/`mutable`; tables `element`.
fn n_instance_exports(it: &mut Interp, w: &mut Wasm, a: &[Value]) -> R<Value> {
    let id = handle(it, a, 0);
    let Some(module) = w.rt.instances.get(&id).cloned() else {
        return it.throw_error("TypeError", "WebAssembly.Instance: instance was released");
    };
    let instance = InstanceId::from_index(id);
    let mut items = Vec::new();
    for export in module.exports() {
        let Some(ext) = w.store.export(instance, &export.name) else {
            continue;
        };
        let o = it.plain();
        put_str(&o, "name", &export.name);
        put_str(&o, "kind", kind_name(&export.kind));
        let handle_value = match ext {
            Extern::Func(f) => f.index(),
            Extern::Memory(m) => m.index(),
            Extern::Table(t) => t.index(),
            Extern::Global(g) => g.index(),
        };
        put(&o, "handle", Value::Number(f64::from(handle_value)));
        match &export.kind {
            ExternKind::Func { params, results } => {
                let p = params.iter().map(|t| Value::str(type_name(*t))).collect();
                let r = results.iter().map(|t| Value::str(type_name(*t))).collect();
                let (p, r) = (it.array(p), it.array(r));
                put(&o, "params", p);
                put(&o, "results", r);
            }
            ExternKind::Global { ty, mutable } => {
                w.rt.global_types.insert(handle_value, (*ty, *mutable));
                put_str(&o, "type", type_name(*ty));
                put(&o, "mutable", Value::Bool(*mutable));
            }
            ExternKind::Table { element, .. } => {
                w.rt.table_types.insert(handle_value, *element);
                put_str(&o, "element", type_name(*element));
            }
            ExternKind::Memory { .. } => {}
        }
        items.push(Value::Object(o));
    }
    Ok(it.array(items))
}

/// Runs a call with JS arguments coerced to the parameter types (missing ones are undefined).
fn invoke(it: &mut Interp, w: &mut Wasm, func: FuncId, args: &Value) -> R<Value> {
    let (params, _) = w.store.func_type(func);
    let mut values = list_of(it, args)?;
    values.resize(params.len(), Value::Undefined);
    let mut inputs = Vec::with_capacity(params.len());
    for (v, ty) in values.iter().zip(&params) {
        inputs.push(to_wasm(it, &mut w.rt, v, *ty)?);
    }
    let Wasm { store, rt, .. } = w;
    rt.pending.clear();
    let result = {
        let mut cb = Callbacks {
            it: &mut *it,
            rt: &mut *rt,
        };
        store.call_func(func, &inputs, &mut cb)
    };
    match result {
        Ok(outputs) => {
            let mut out = Vec::with_capacity(outputs.len());
            for v in &outputs {
                out.push(to_js(it, rt, v)?);
            }
            Ok(match out.len() {
                0 => Value::Undefined,
                1 => out.remove(0),
                _ => it.array(out),
            })
        }
        Err(error) => Err(call_error(it, rt, error)),
    }
}

fn n_call(it: &mut Interp, w: &mut Wasm, a: &[Value]) -> R<Value> {
    let id = handle(it, a, 0);
    let name = text(it, a, 1);
    match w.store.export(InstanceId::from_index(id), &name) {
        Some(Extern::Func(func)) => invoke(it, w, func, &arg(a, 2)),
        Some(_) => it.throw_error("TypeError", &format!("export '{name}' is not a function")),
        None => it.throw_error("TypeError", &format!("no export named '{name}'")),
    }
}

fn n_call_func(it: &mut Interp, w: &mut Wasm, a: &[Value]) -> R<Value> {
    let func = FuncId::from_index(handle(it, a, 0));
    invoke(it, w, func, &arg(a, 1))
}

fn n_func_type(it: &mut Interp, w: &mut Wasm, a: &[Value]) -> R<Value> {
    let func = FuncId::from_index(handle(it, a, 0));
    let (params, results) = w.store.func_type(func);
    let o = it.plain();
    let p = params.iter().map(|t| Value::str(type_name(*t))).collect();
    let r = results.iter().map(|t| Value::str(type_name(*t))).collect();
    let (p, r) = (it.array(p), it.array(r));
    put(&o, "params", p);
    put(&o, "results", r);
    Ok(Value::Object(o))
}

// ---- memory ---------------------------------------------------------------

fn n_memory_create(it: &mut Interp, w: &mut Wasm, a: &[Value]) -> R<Value> {
    let initial = handle(it, a, 0);
    let maximum = match arg(a, 1) {
        Value::Undefined => None,
        _ => Some(handle(it, a, 1)),
    };
    match w.store.new_memory(initial, maximum) {
        Ok(id) => Ok(Value::Number(f64::from(id.index()))),
        Err(_) => it.throw_error(
            "RangeError",
            "WebAssembly.Memory(): could not allocate memory",
        ),
    }
}

/// The memory's ArrayBuffer. The same object until the memory grows; after that a new one, and the
/// old one reads as detached (its storage is emptied by the engine).
fn n_memory_buffer(it: &mut Interp, w: &mut Wasm, a: &[Value]) -> R<Value> {
    let id = handle(it, a, 0);
    let Some(shared) = w.store.memory_buffer(MemoryId::from_index(id)) else {
        return it.throw_error(
            "TypeError",
            "WebAssembly.Memory: memory is not shared with the embedder",
        );
    };
    if let Some((obj, data)) = w.rt.buffers.get(&id) {
        if same_shared(data, &shared) {
            return Ok(Value::Object(obj.clone()));
        }
    }
    let data: Bytes = shared_bytes(shared);
    let obj = it.new_array_buffer(data.clone());
    w.rt.buffers.insert(id, (obj.clone(), data));
    Ok(Value::Object(obj))
}

fn n_memory_grow(it: &mut Interp, w: &mut Wasm, a: &[Value]) -> R<Value> {
    let id = handle(it, a, 0);
    let delta = handle(it, a, 1);
    match w
        .store
        .memory_grow(MemoryId::from_index(id), u64::from(delta))
    {
        Ok(old) => Ok(Value::Number(old as f64)),
        Err(_) => it.throw_error(
            "RangeError",
            "WebAssembly.Memory.grow(): Maximum memory size exceeded",
        ),
    }
}

fn n_memory_size(it: &mut Interp, w: &mut Wasm, a: &[Value]) -> R<Value> {
    let id = handle(it, a, 0);
    Ok(Value::Number(
        w.store.memory_pages(MemoryId::from_index(id)) as f64,
    ))
}

// ---- table ----------------------------------------------------------------

fn n_table_create(it: &mut Interp, w: &mut Wasm, a: &[Value]) -> R<Value> {
    let element = {
        let name = text(it, a, 0);
        match parse_type(&name) {
            Some(ty @ (ValType::FuncRef | ValType::ExternRef)) => ty,
            _ => return it.throw_error("TypeError", "WebAssembly.Table(): Descriptor property 'element' must be a WebAssembly reference type"),
        }
    };
    let initial = handle(it, a, 1);
    let maximum = match arg(a, 2) {
        Value::Undefined => None,
        _ => Some(handle(it, a, 2)),
    };
    let init = to_wasm(it, &mut w.rt, &arg(a, 3), element)?;
    match w.store.new_table(element, initial, maximum, &init) {
        Ok(id) => {
            w.rt.table_types.insert(id.index(), element);
            Ok(Value::Number(f64::from(id.index())))
        }
        Err(_) => it.throw_error(
            "RangeError",
            "WebAssembly.Table(): could not allocate table",
        ),
    }
}

fn table_kind(w: &Wasm, id: TableId) -> ValType {
    w.rt.table_types
        .get(&id.index())
        .copied()
        .unwrap_or(ValType::FuncRef)
}

fn n_table_length(it: &mut Interp, w: &mut Wasm, a: &[Value]) -> R<Value> {
    let id = TableId::from_index(handle(it, a, 0));
    Ok(Value::Number(w.store.table_size(id) as f64))
}

fn n_table_get(it: &mut Interp, w: &mut Wasm, a: &[Value]) -> R<Value> {
    let id = TableId::from_index(handle(it, a, 0));
    let index = num(it, a, 1) as u64;
    match w.store.table_get(id, index) {
        Some(v) => to_js(it, &w.rt, &v),
        None => {
            let (size, kind) = (w.store.table_size(id), table_kind(w, id));
            let what = if kind == ValType::ExternRef {
                "externref"
            } else {
                "funcref"
            };
            it.throw_error(
                "RangeError",
                &format!("WebAssembly.Table.get(): invalid index {index} into {what} table of size {size}"),
            )
        }
    }
}

fn n_table_set(it: &mut Interp, w: &mut Wasm, a: &[Value]) -> R<Value> {
    let id = TableId::from_index(handle(it, a, 0));
    let index = num(it, a, 1) as u64;
    let kind = table_kind(w, id);
    let value = to_wasm(it, &mut w.rt, &arg(a, 2), kind)?;
    match w.store.table_set(id, index, &value) {
        Ok(()) => Ok(Value::Undefined),
        Err(_) => {
            let size = w.store.table_size(id);
            let what = if kind == ValType::ExternRef {
                "externref"
            } else {
                "funcref"
            };
            it.throw_error(
                "RangeError",
                &format!("WebAssembly.Table.set(): invalid index {index} into {what} table of size {size}"),
            )
        }
    }
}

fn n_table_grow(it: &mut Interp, w: &mut Wasm, a: &[Value]) -> R<Value> {
    let id = TableId::from_index(handle(it, a, 0));
    let delta = num(it, a, 1) as u64;
    let kind = table_kind(w, id);
    let init = to_wasm(it, &mut w.rt, &arg(a, 2), kind)?;
    match w.store.table_grow(id, delta, &init) {
        Ok(old) => Ok(Value::Number(old as f64)),
        Err(_) => it.throw_error(
            "RangeError",
            &format!("WebAssembly.Table.grow(): failed to grow table by {delta}"),
        ),
    }
}

// ---- global ---------------------------------------------------------------

fn n_global_create(it: &mut Interp, w: &mut Wasm, a: &[Value]) -> R<Value> {
    let name = text(it, a, 0);
    let Some(ty) = parse_type(&name) else {
        return it.throw_error(
            "TypeError",
            "WebAssembly.Global(): Descriptor property 'value' must be a WebAssembly type",
        );
    };
    let mutable = arg(a, 1).truthy();
    let value = match (arg(a, 2), ty) {
        (Value::Undefined, ValType::ExternRef) => Val::ExternRef(Some({
            w.rt.externs.push(Some(Value::Undefined));
            (w.rt.externs.len() - 1) as u32
        })),
        (Value::Undefined, ValType::FuncRef) => Val::FuncRef(None),
        (Value::Undefined, ValType::I64) => Val::I64(0),
        (Value::Undefined, ValType::F32) => Val::F32(0.0),
        (Value::Undefined, ValType::F64) => Val::F64(0.0),
        (Value::Undefined, _) => Val::I32(0),
        (v, ty) => to_wasm(it, &mut w.rt, &v, ty)?,
    };
    let id = w.store.new_global(mutable, &value);
    w.rt.global_types.insert(id.index(), (ty, mutable));
    Ok(Value::Number(f64::from(id.index())))
}

fn n_global_get(it: &mut Interp, w: &mut Wasm, a: &[Value]) -> R<Value> {
    let id = GlobalId::from_index(handle(it, a, 0));
    let v = w.store.global_get(id);
    to_js(it, &w.rt, &v)
}

fn n_global_set(it: &mut Interp, w: &mut Wasm, a: &[Value]) -> R<Value> {
    let id = GlobalId::from_index(handle(it, a, 0));
    let (ty, mutable) =
        w.rt.global_types
            .get(&id.index())
            .copied()
            .unwrap_or((ValType::I32, true));
    if !mutable {
        return it.throw_error(
            "TypeError",
            "set WebAssembly.Global.value: Can't set the value of an immutable global.",
        );
    }
    let value = to_wasm(it, &mut w.rt, &arg(a, 1), ty)?;
    match w.store.global_set(id, &value) {
        Ok(()) => Ok(Value::Undefined),
        Err(message) => it.throw_error(
            "TypeError",
            &format!("set WebAssembly.Global.value: {message}"),
        ),
    }
}

// ---- release ----------------------------------------------------------------

/// Drops the Interp-side record for a handle. The engine itself frees nothing until the store
/// is dropped (wasmi keeps entities alive for the store's lifetime).
fn n_release(it: &mut Interp, w: &mut Wasm, a: &[Value]) -> R<Value> {
    let kind = text(it, a, 0);
    let id = handle(it, a, 1);
    match kind.as_str() {
        "module" => {
            w.rt.modules.remove(&id);
        }
        "externref" => {
            if let Some(slot) = w.rt.externs.get_mut(id as usize) {
                *slot = None;
            }
        }
        "memory" => {
            w.rt.buffers.remove(&id);
        }
        _ => {}
    }
    Ok(Value::Undefined)
}
