//! Package host: loads a registry bundle's CommonJS factories into an [`Interp`] with its own
//! `require` resolution, and exchanges plain values with compiled Thaw code.
//!
//! Boundary values ([`Native`]) mirror Thaw's native layouts for plain data (numbers, strings,
//! booleans, arrays, string-keyed objects). They are *copied* in and out here; the designed live
//! views (`Json` / native class instances, design.md section 2) are milestone M4.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::ffi::{c_char, c_void, CStr, CString};
use std::ptr;
use std::rc::Rc;

use crate::scan::{parse_bundle, BundleParts};
use crate::{array_index, Ctl, Interp, Kind, Obj, Slot, Value, R};

/// Plain data crossing the compiled-code / thaw-js boundary.
#[derive(Debug, Clone, PartialEq)]
pub enum Native {
    Undefined,
    Null,
    Bool(bool),
    Number(f64),
    Str(String),
    Array(Vec<Native>),
    Object(Vec<(String, Native)>),
}

#[derive(Debug)]
pub enum HostError {
    /// The bundle uses a construct outside the evaluator (feature name as in the scan).
    Unsupported(String),
    /// A JS exception, rendered as `Name: message`.
    Thrown(String),
    /// A result that has no plain-data form (function, cycle).
    NotPlainData(String),
    Bundle(String),
}

impl std::fmt::Display for HostError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HostError::Unsupported(x) => write!(f, "unsupported: {x}"),
            HostError::Thrown(x) => write!(f, "{x}"),
            HostError::NotPlainData(x) => write!(f, "not plain data: {x}"),
            HostError::Bundle(x) => write!(f, "bad bundle: {x}"),
        }
    }
}

struct Loader {
    parts: BundleParts,
    factories: HashMap<String, Value>,
    cache: RefCell<HashMap<String, Value>>,
    /// The package's addon object, built on first use (`addon_of`).
    addon: RefCell<Option<Value>>,
    /// The package name the napi bridge keys its exports by.
    package: String,
}

/// Builds the addon object from the napi proxy layer globals: one function per export, routed to
/// `construct` under `new` and to `call` otherwise (the QuickJS prelude's `__thaw_install_native` shape).
const ADDON_BUILDER: &str = r#"function (info, each, call, construct, pkg) {
  var addon = {};
  each(info, pkg + '::', pkg, function (name, bare) {
    addon[bare] = function () { return new.target
      ? construct(name, arguments, new.target.prototype)
      : call(name, arguments); };
  });
  return addon;
}"#;

fn js_error(it: &mut Interp, message: &str) -> Ctl {
    let ctor = it
        .lookup(&it.global.clone(), "Error")
        .unwrap_or(Value::Undefined);
    match it.construct(&ctor, &[Value::str(message)]) {
        Ok(err) => Ctl::Throw(err),
        Err(c) => c,
    }
}

/// The package's native addon object. Built once per package from the napi proxy layer globals (installed
/// on every realm once the bridge is enabled); throws when no bridge is registered.
fn addon_of(loader: &Rc<Loader>, it: &mut Interp) -> R<Value> {
    if let Some(addon) = loader.addon.borrow().clone() {
        return Ok(addon);
    }
    let global = it.global.clone();
    let ready = match it.lookup(&global, "__thaw_json_graph_napi_bridge_available") {
        Some(f) => it.call(&f, &Value::Undefined, &[])?.truthy(),
        None => false,
    };
    if !ready {
        return Err(js_error(it, "native addon bridge is unavailable"));
    }
    let exports = it
        .lookup(&global, "__thaw_json_graph_napi_exports")
        .unwrap_or(Value::Undefined);
    let info = it.call(&exports, &Value::Undefined, &[])?;
    let each = it
        .lookup(&global, "__thaw_json_graph_napi_each_export")
        .unwrap_or(Value::Undefined);
    let call = it
        .lookup(&global, "__thaw_json_graph_napi_call_export")
        .unwrap_or(Value::Undefined);
    let construct = it
        .lookup(&global, "__thaw_json_graph_napi_construct")
        .unwrap_or(Value::Undefined);
    let build = it.eval_function(ADDON_BUILDER)?;
    let addon = it.call(
        &build,
        &Value::Undefined,
        &[info, each, call, construct, Value::str(&loader.package)],
    )?;
    *loader.addon.borrow_mut() = Some(addon.clone());
    Ok(addon)
}

fn require_in(loader: &Rc<Loader>, it: &mut Interp, from: &str, spec: &str) -> R<Value> {
    // the addon loaders return the addon itself, never the bundled code (render.rs shortcuts)
    if spec == "bindings" || spec == "node-gyp-build" {
        let l = loader.clone();
        return Ok(it.native(move |it, _, _| addon_of(&l, it)));
    }
    if spec.ends_with(".node") {
        return addon_of(loader, it);
    }
    let target = loader
        .parts
        .require_maps
        .get(from)
        .and_then(|m| m.get(spec))
        .cloned();
    match target {
        Some(key) if key.ends_with(".node") => addon_of(loader, it),
        Some(key) => load_module(loader, it, &key),
        None => {
            let ctor = it
                .lookup(&it.global.clone(), "Error")
                .unwrap_or(Value::Undefined);
            let err = it.construct(
                &ctor,
                &[Value::str(&format!("Cannot find module '{spec}'"))],
            )?;
            it.set(&err, "code", Value::str("MODULE_NOT_FOUND"))?;
            Err(Ctl::Throw(err))
        }
    }
}

fn load_module(loader: &Rc<Loader>, it: &mut Interp, key: &str) -> R<Value> {
    let cached = loader.cache.borrow().get(key).cloned();
    if let Some(m) = cached {
        return it.get(&m, "exports");
    }
    let Some(factory) = loader.factories.get(key).cloned() else {
        // ponytail: only the crypto stand-in is a builtin module; add others as packages need them
        if matches!(key, "node:crypto" | "crypto") {
            return Ok(it
                .lookup(&it.global.clone(), "__thaw_crypto_module")
                .unwrap_or(Value::Undefined));
        }
        return Err(Ctl::Throw(Value::str(&format!(
            "Cannot find module '{key}'"
        ))));
    };
    let module = Value::Object(it.plain());
    let exports = Value::Object(it.plain());
    it.set(&module, "exports", exports.clone())?;
    loader
        .cache
        .borrow_mut()
        .insert(key.to_string(), module.clone());
    let (l, k) = (loader.clone(), key.to_string());
    let require = it.native(move |it, _, args| {
        let spec = it.to_str(&crate::arg(args, 0));
        require_in(&l, it, &k, &spec)
    });
    let l = loader.clone();
    let addon = it.native(move |it, _, _| addon_of(&l, it));
    it.set(&require, "addon", addon)?;
    // ESM-converted modules are `function (origin) { return function (module, exports, ...) {...}; }`
    let mut f = factory;
    if it.get(&f, "length")?.as_number() == Some(1.0) {
        f = it.call(&f, &Value::Undefined, &[Value::Undefined])?;
    }
    let file = Value::str(key);
    it.call(
        &f,
        &exports,
        &[
            module.clone(),
            exports.clone(),
            require.clone(),
            Value::Undefined,
            file,
            Value::str("/"),
            require,
        ],
    )?;
    it.get(&module, "exports")
}

/// Bundle text that starts the builtin-name JSON array (`render.rs` writes it verbatim).
const BUILTIN_NAMES_MARK: &str = "var __thaw_bundle_builtin_names = Object.freeze(";

/// `node:module` needs the loader globals the registry renders in the bundle IIFE (which thaw-js never
/// runs): `createRequire` reads `__thaw_bundle_create_require`, `isBuiltin` reads the names list.
/// Only that entry gets them, so no other package sees new globals.
fn install_module_loader(it: &mut Interp, loader: &Rc<Loader>, bundle_js: &str) -> R<()> {
    let names = bundle_js
        .find(BUILTIN_NAMES_MARK)
        .and_then(|at| {
            let rest = &bundle_js[at + BUILTIN_NAMES_MARK.len()..];
            rest.find(");").map(|end| rest[..end].to_string())
        })
        .ok_or_else(|| Ctl::Throw(Value::str("bundle has no __thaw_bundle_builtin_names")))?;
    let keys = serde_json::to_string(&loader.factories.keys().collect::<Vec<_>>())
        .expect("string keys serialize");
    let l = loader.clone();
    let load = it.native(move |it, _, args| {
        let key = it.to_str(&crate::arg(args, 0));
        load_module(&l, it, &key)
    });
    // ponytail: only stateless builtins are served through created requires (a separate realm would split
    // identity); any other spec is a MODULE_NOT_FOUND
    let prelude = format!(
        r#"function (load) {{
          var names = Object.freeze({names}), keys = {keys};
          function nameOf(spec) {{ spec = String(spec); return spec.indexOf('node:') === 0 ? spec.slice(5) : spec; }}
          function missing(spec) {{ var e = new Error("Cannot find module '" + spec + "'"); e.code = 'MODULE_NOT_FOUND'; return e; }}
          globalThis.__thaw_bundle_builtin_names = names;
          globalThis.__thaw_bundle_create_require = function (base) {{
            var created = function (spec) {{
              var n = nameOf(spec);
              if (names.indexOf(n) < 0 || keys.indexOf('node:' + n) < 0) throw missing(spec);
              return load('node:' + n);
            }};
            created.resolve = function (spec) {{ if (names.indexOf(nameOf(spec)) < 0) throw missing(spec); return String(spec); }};
            created.cache = {{}};
            return created;
          }};
        }}"#
    );
    let install = it.eval_function(&prelude)?;
    it.call(&install, &Value::Undefined, &[load])?;
    Ok(())
}

/// A loaded package: one interpreter, the bundle's modules, and its entry exports.
pub struct Package {
    /// Boxed: async functions hold its address while suspended.
    pub(crate) interp: Box<Interp>,
    exports: Value,
}

impl Package {
    /// Parse `bundle_js` (registry `bundle.js`) and evaluate its entry module.
    pub fn load(bundle_js: &str) -> Result<Package, HostError> {
        Self::load_with_stack(bundle_js, 0)
    }

    /// As [`Package::load`], limiting recursion by the real stack (`stack_bytes` available from here)
    /// instead of the fixed call depth; 0 keeps the depth cap.
    pub fn load_with_stack(bundle_js: &str, stack_bytes: usize) -> Result<Package, HostError> {
        Self::load_named("", bundle_js, stack_bytes)
    }

    /// As [`Package::load_with_stack`] for the package `name`: its native addon exports are the ones the
    /// napi bridge lists under `name::` (qualified packages) or bare (`addon_of`).
    pub fn load_named(
        name: &str,
        bundle_js: &str,
        stack_bytes: usize,
    ) -> Result<Package, HostError> {
        let mut parts = parse_bundle(bundle_js).map_err(HostError::Bundle)?;
        if let Some(problem) = parts.esm_problems.iter().next() {
            return Err(HostError::Unsupported((*problem).to_string()));
        }
        let mut interp = Box::new(Interp::runtime());
        interp.share_process_state(crate::procstate::host_shared());
        // console output goes straight to stdout: nothing in the host drains `Interp::out`
        interp.set_stream(true);
        if stack_bytes != 0 {
            interp.configure_stack(stack_bytes);
        }
        interp.run_id = crate::next_run_id(); // factory spans come from this one parse
        interp
            .sources
            .insert(interp.run_id, std::rc::Rc::from(parts.source.as_str()));
        let entry = parts.entry.clone();
        for (_, func) in parts.factories.iter_mut() {
            interp.annotate_function(func);
        }
        let mut factories = HashMap::new();
        for (key, func) in &parts.factories {
            let scope = interp.global.clone();
            let f = interp
                .function_value(None, func, &scope)
                .map_err(|c| classify(&mut interp, c))?;
            factories.insert(key.clone(), f);
        }
        let loader = Rc::new(Loader {
            parts,
            factories,
            cache: RefCell::new(HashMap::new()),
            addon: RefCell::new(None),
            package: name.to_string(),
        });
        if entry == "node:module" {
            install_module_loader(&mut interp, &loader, bundle_js)
                .map_err(|c| classify(&mut interp, c))?;
        }
        let exports =
            load_module(&loader, &mut interp, &entry).map_err(|c| classify(&mut interp, c))?;
        Ok(Package { interp, exports })
    }

    /// Call `path` (`"name"` or `"a.b"`) on the package exports with plain-data arguments.
    pub fn call(&mut self, path: &str, args: &[Native]) -> Result<Native, HostError> {
        self.call_or_get(path, Some(args), false)
    }

    /// Call an export addressed the way registry shims name it (the `.d.ts` function name). A package
    /// whose `module.exports` is itself the function (`ms`, `minimist`) has no property of that name,
    /// so an absent property falls back to `exports.default`, then to `exports` when callable.
    pub fn call_export(&mut self, name: &str, args: &[Native]) -> Result<Native, HostError> {
        self.call_or_get(name, Some(args), true)
    }

    /// Read the value at `path` (`"VERSION"`, `"a.b"`) without calling it.
    pub fn get(&mut self, path: &str) -> Result<Native, HostError> {
        self.call_or_get(path, None, false)
    }

    fn call_or_get(
        &mut self,
        path: &str,
        args: Option<&[Native]>,
        export_fallback: bool,
    ) -> Result<Native, HostError> {
        let r = self.call_or_get_value(path, args, export_fallback, false)?;
        from_value(&mut self.interp, &r, 0)
    }

    fn call_or_get_value(
        &mut self,
        path: &str,
        args: Option<&[Native]>,
        export_fallback: bool,
        pending_ok: bool,
    ) -> Result<Value, HostError> {
        let args = args.map(|a| {
            a.iter()
                .map(|a| to_value(&mut self.interp, a))
                .collect::<Vec<_>>()
        });
        self.call_or_get_values(path, args.as_deref(), export_fallback, pending_ok)
    }

    fn call_or_get_values(
        &mut self,
        path: &str,
        args: Option<&[Value]>,
        export_fallback: bool,
        pending_ok: bool,
    ) -> Result<Value, HostError> {
        let it = &mut self.interp;
        let mut this = self.exports.clone();
        let mut f = this.clone();
        for part in path.split('.').filter(|p| !p.is_empty()) {
            this = f.clone();
            f = it.get(&this, part).map_err(|c| classify(it, c))?;
        }
        if export_fallback && matches!(f, Value::Undefined) {
            let default = it
                .get(&self.exports, "default")
                .map_err(|c| classify(it, c))?;
            f = if default.is_callable() {
                default
            } else {
                self.exports.clone()
            };
            this = self.exports.clone();
        }
        let r = match args {
            Some(args) => {
                if !f.is_callable() {
                    return Err(not_callable(it, format!("`{path}` is not a function")));
                }
                let r = it.call(&f, &this, args).map_err(|c| classify(it, c))?;
                // an async export: await only until its promise settles, not until the loop is idle
                if pending_ok {
                    awaited_or_pending(it, r)?
                } else {
                    awaited(it, r)?
                }
            }
            None => f,
        };
        Ok(r)
    }

    /// A member of the entry exports by (possibly dotted) path, with the same fallbacks as
    /// [`Package::call_export`] for a package whose `module.exports` is the value itself.
    fn exports_member(&mut self, path: &str) -> Result<Value, HostError> {
        let it = &mut self.interp;
        let mut value = self.exports.clone();
        let parts = path
            .split('.')
            .filter(|p| !p.is_empty())
            .collect::<Vec<_>>();
        for (i, part) in parts.iter().enumerate() {
            let next = it.get(&value, part).map_err(|c| classify(it, c))?;
            // `export = NS` makes `module.exports` the namespace itself, so a leading `NS.` segment
            // that is not a property there is skipped (`NS.enc.Hex` reads `enc.Hex`)
            value = if i == 0 && parts.len() > 1 && matches!(next, Value::Undefined) {
                value
            } else {
                next
            };
        }
        if parts.len() == 1 && matches!(value, Value::Undefined) {
            let default = it
                .get(&self.exports, "default")
                .map_err(|c| classify(it, c))?;
            value = if default.is_callable() {
                default
            } else {
                self.exports.clone()
            };
        }
        Ok(value)
    }

    /// The entry module's `module.exports`.
    pub fn exports(&self) -> Value {
        self.exports.clone()
    }

    pub fn interp(&mut self) -> &mut Interp {
        &mut self.interp
    }
}

pub(crate) fn classify(it: &mut Interp, c: Ctl) -> HostError {
    match c {
        Ctl::Unsupported(f) => HostError::Unsupported(f),
        Ctl::Throw(v) => {
            if let Some((display, framed)) = frame_error(it, &v) {
                LAST_FRAME.with(|f| *f.borrow_mut() = Some((display.clone(), framed)));
                return HostError::Thrown(display);
            }
            let text = it.to_str(&v);
            let name = it.get(&v, "name").ok().map(|n| it.to_str(&n).to_string());
            HostError::Thrown(match (name, matches!(v, Value::Object(_))) {
                (Some(n), true) if !text.starts_with(&n) => format!("{n}: {text}"),
                _ => text.to_string(),
            })
        }
        Ctl::Exit(code) => {
            terminate_process(it, code);
            HostError::Thrown(format!("process.exit({code})"))
        }
        Ctl::Return(_) | Ctl::Break(_) | Ctl::Continue(_) => {
            HostError::Thrown("stray completion".into())
        }
    }
}

const CALLBACK_KEY: &str = "__thaw_tjs_cb__";
const TIMER_KEY: &str = "__thaw_tjs_timer__";
const HANDLE_KEY: &str = "__thaw_tjs_handle__";
const DATE_KEY: &str = "__thaw_tjs_date__";
/// `{"__thaw_tjs_bigint__": "<decimal>"}`: a compiled BigInt, which a JSON number cannot carry exactly.
const BIGINT_KEY: &str = "__thaw_tjs_bigint__";
/// `{"__thaw_tjs_num__": "-0"}`: a compiled negative zero, which JSON integers lose; also `"NaN"`/`"Infinity"`/`"-Infinity"`.
const NUM_KEY: &str = "__thaw_tjs_num__";
/// `{"__thaw_tjs_bytes__": {"k": "Buffer" | "Uint8Array" | "ArrayBuffer" | "DataView", "b64": "..."}}`:
/// compiled `Bytes` crossing as base64 text. `k` selects the JS kind; a missing or unknown `k` is a Buffer.
/// ArrayBuffer and DataView cross by copy (a DataView over a fresh ArrayBuffer of exactly its window).
const BYTES_KEY: &str = "__thaw_tjs_bytes__";
const MAP_KEY: &str = "__thaw_tjs_map__";
const SET_KEY: &str = "__thaw_tjs_set__";
const UNDEFINED_KEY: &str = "$__thaw_napi_undefined$";
/// Marks an `Object.create(null)` object crossing as plain data; thaw-std's JSON parser
/// turns the marked object back into a null-prototype one (`[Object: null prototype]`).
const NULL_PROTO_KEY: &str = "__thaw_tjs_nullproto__";

/// `(adapter, context)` of a compiled callback: `adapter(context, args_json) -> result_json`.
type Adapter = unsafe extern "C" fn(*const c_void, *const c_char) -> *mut c_char;

struct CompiledCallback {
    adapter: usize,
    context: usize,
    /// The closure's identity (aliases resolved) when the compiler gave one: registrations of the same
    /// closure share one id, so `off(listener)` finds what `on(listener)` registered. 0 when unknown.
    identity: usize,
    params: usize,
    /// Graph-wire callback (`thaw_tjs_register_callback_graph`): arguments cross as `[this, ...args]`
    /// with live objects behind handle leases, the result is a graph wire; `finish` (when not 0)
    /// drains a native Promise result.
    graph: bool,
    finish: usize,
    has_rest: bool,
    /// The compiler's `jsvalue_param_mask` (graph callbacks): low 32 bits = JsValue positions whose
    /// arguments cross as handle leases, high 32 bits = `Str` positions where an `Error` is framed.
    mask: u64,
    /// An object-literal method (`thaw_tjs_register_native_method_callback_graph`): its receiver is
    /// declared parameter 0, so the mask bit `i` is wire item `i`; a plain callback's is `i + 1`.
    method: bool,
    /// Keeps the closure environment alive for as long as JS may call it.
    _root: thaw_arena::ArenaRoot,
}

// SAFETY: the table is only appended to and read, and no entry is ever dropped, so its `ArenaRoot` pins
// are never released on a thread other than the one that took them. Coroutine threads dispatch these
// callbacks, so the table is process-wide rather than thread-local.
unsafe impl Send for CompiledCallback {}

static CALLBACKS: std::sync::Mutex<Vec<CompiledCallback>> = std::sync::Mutex::new(Vec::new());

thread_local! {
    /// Values compiled code holds by handle (class instances): `(package handle, value)`; id = index + 1.
    /// A released handle keeps its slot (ids are never reused, so a stale id is `undefined`).
    pub(crate) static HANDLES: RefCell<Vec<(usize, Value)>> = const { RefCell::new(Vec::new()) };
    /// Outstanding references per handle id (index + 1): compiled code retains and releases handles
    /// the way it does QuickJS ones.
    static HANDLE_REFS: RefCell<Vec<u32>> = const { RefCell::new(Vec::new()) };
    /// Live handle ids by the object they hold (`Rc` address): a plain result that is such an object
    /// leases the existing handle, so compiled code sees the same identity it holds.
    static LIVE_BY_OBJECT: RefCell<HashMap<usize, Vec<u64>>> = RefCell::new(HashMap::new());
}

thread_local! {
    /// The package last entered by `with_package`.
    static LAST_PACKAGE: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// `THAW_JS_TRACE_WIRE` is read once; off, a trace point costs one load.
pub(crate) fn trace_wire() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("THAW_JS_TRACE_WIRE").is_some())
}

/// Trace one handle operation with its outstanding reference count (no-op unless tracing).
/// Must not be called while `HANDLE_REFS` is borrowed.
fn trace_h(op: &str, handle: i64) {
    if trace_wire() {
        let refs = HANDLE_REFS.with(|r| r.borrow().get((handle as usize).wrapping_sub(1)).copied());
        eprintln!("tjs {op} id={handle} refs={refs:?}");
    }
}

/// Take a lease on `value` in `package`. An object (or Symbol, which is an object here) that already has a
/// live handle in that package leases the same id again, so every read of one object names one id;
/// a primitive always gets a fresh slot. Ids are never reused, and the id dies with its last lease.
pub(crate) fn store_handle(package: usize, value: Value) -> i64 {
    let object = object_key(&value);
    if let Some(key) = object {
        let live = LIVE_BY_OBJECT.with(|m| {
            m.borrow().get(&key).and_then(|ids| {
                ids.iter().copied().find(|&id| {
                    HANDLES.with(|h| {
                        h.borrow()
                            .get((id as usize).wrapping_sub(1))
                            .is_some_and(|e| e.0 == package)
                    })
                })
            })
        });
        if let Some(id) = live {
            HANDLE_REFS.with(|r| {
                if let Some(refs) = r.borrow_mut().get_mut((id as usize).wrapping_sub(1)) {
                    *refs += 1;
                }
            });
            trace_h("store", id as i64);
            return id as i64;
        }
    }
    HANDLE_REFS.with(|r| r.borrow_mut().push(1));
    let id = HANDLES.with(|h| {
        let mut h = h.borrow_mut();
        h.push((package, value));
        h.len() as i64
    });
    if let Some(key) = object {
        LIVE_BY_OBJECT.with(|m| m.borrow_mut().entry(key).or_default().push(id as u64));
    }
    trace_h("store", id);
    id
}

/// The identity of an object value (its `Rc` address), if it is one.
fn object_key(v: &Value) -> Option<usize> {
    match v {
        Value::Object(o) => Some(Rc::as_ptr(o) as usize),
        _ => None,
    }
}

/// A live handle id that holds exactly this object, if compiled code has one.
pub(crate) fn live_handle_of(v: &Value) -> Option<u64> {
    let key = object_key(v)?;
    LIVE_BY_OBJECT.with(|m| m.borrow().get(&key).and_then(|ids| ids.first().copied()))
}

/// Take another reference on a handle; returns 1 when it is live.
#[no_mangle]
pub extern "C" fn thaw_tjs_retain_handle(handle: i64) -> u8 {
    let ok =
        HANDLE_REFS.with(
            |r| match r.borrow_mut().get_mut((handle as usize).wrapping_sub(1)) {
                Some(refs) if *refs > 0 => {
                    *refs += 1;
                    1
                }
                _ => 0,
            },
        );
    if ok == 1 {
        trace_h("retain", handle);
    } else if trace_wire() {
        eprintln!("RETAIN-FAILED id={handle}");
    }
    ok
}

/// Whether `handle` names a live handle (one with a reference left).
pub(crate) fn handle_is_live(handle: i64) -> bool {
    HANDLE_REFS.with(|r| {
        r.borrow()
            .get((handle as usize).wrapping_sub(1))
            .is_some_and(|refs| *refs > 0)
    })
}

/// Drop a reference on a handle (the value is released with the last one); returns 1 when it was live.
#[no_mangle]
pub extern "C" fn thaw_tjs_release_handle(handle: i64) -> u8 {
    let index = (handle as usize).wrapping_sub(1);
    let last = HANDLE_REFS.with(|r| match r.borrow_mut().get_mut(index) {
        Some(refs) if *refs > 0 => {
            *refs -= 1;
            Some(*refs == 0)
        }
        _ => None,
    });
    match last {
        Some(true) => {
            trace_h("release", handle);
            if trace_wire() {
                eprintln!(
                    "release->0 id={handle}\n{}",
                    std::backtrace::Backtrace::force_capture()
                );
            }
            // Take the value out before dropping it: its drop may release other handles.
            let value = HANDLES.with(|h| {
                h.borrow_mut()
                    .get_mut(index)
                    .map(|e| std::mem::replace(&mut e.1, Value::Undefined))
            });
            if let Some(key) = value.as_ref().and_then(object_key) {
                LIVE_BY_OBJECT.with(|m| {
                    let mut m = m.borrow_mut();
                    if let Some(ids) = m.get_mut(&key) {
                        ids.retain(|&id| id != handle as u64);
                        if ids.is_empty() {
                            m.remove(&key);
                        }
                    }
                });
            }
            drop(value);
            1
        }
        Some(false) => {
            trace_h("release", handle);
            1
        }
        None => {
            if trace_wire() {
                eprintln!("RELEASE-OF-DEAD id={handle}");
            }
            0
        }
    }
}

/// Release every live handle and every bridge reference kept until shutdown (terminal shutdown: napi
/// graph pins are dropped with their handles). Releasing can re-enter napi finalizers, which may retain
/// or create handles, so the scan repeats until none is live. Returns the number released.
#[no_mangle]
pub extern "C" fn thaw_tjs_release_all_handles() -> u64 {
    // one thaw-js Error per native id is a live reference too
    PACKAGES.with(|ps| {
        for package in ps.borrow_mut().iter_mut().flatten() {
            package.interp.native_errors.clear();
        }
    });
    let mut released = 0;
    loop {
        let live: Vec<i64> = HANDLE_REFS.with(|r| {
            r.borrow()
                .iter()
                .enumerate()
                .filter(|(_, refs)| **refs > 0)
                .map(|(index, _)| index as i64 + 1)
                .collect()
        });
        let retained = crate::napi_bridge::release_retained();
        if live.is_empty() && retained == 0 {
            return released;
        }
        for handle in &live {
            while handle_is_live(*handle) {
                thaw_tjs_release_handle(*handle);
            }
        }
        released += live.len() as u64 + retained;
    }
}

/// What a class operation does (the symbol prefix compiled code passes).
enum ClassOp<'a> {
    New {
        class: &'a str,
    },
    /// `$value$pkg::name$arity0`: the export itself, kept as a handle
    Value {
        path: &'a str,
    },
    Method {
        receiver: i64,
        name: &'a str,
    },
    /// `$setter$pkg::Class$name$overloadK$arity1`: `receiver[name] = value` under strict mode; the
    /// result is the assigned value
    Setter {
        receiver: i64,
        name: &'a str,
    },
    /// `$getter$pkg::Class$name`: `receiver[name]`, read through the instance handle
    Getter {
        receiver: i64,
        name: &'a str,
    },
    /// `$staticgetter$pkg::Class$name`: `Class[name]`, read from the class constructor
    StaticGetter {
        class: &'a str,
        name: &'a str,
    },
    /// `$staticsetter$pkg::Class$name`: `Class[name] = value` under strict mode; the result is the value
    StaticSetter {
        class: &'a str,
        name: &'a str,
    },
    Static {
        class: &'a str,
        name: &'a str,
    },
}

/// `$new$pkg::Class$arityN[...]` / `$method$pkg::Class$name$overloadK$arityN` /
/// `$staticmethod$pkg::Class$name$...` (the `void` variants alike) -> (op kind, package, class path, member).
fn parse_class_symbol(symbol: &str) -> Option<(&str, &str, &str, Option<&str>)> {
    let mut parts = symbol.strip_prefix('$')?.splitn(3, '$');
    let kind = parts.next()?;
    let class = parts.next()?;
    let rest = parts.next()?;
    let (package, class_path) = class.split_once("::")?;
    let member = match kind {
        "new" => None,
        // the member may itself start with `$` (`$disconnect`): cut at the trailing `$overloadK$arityN`
        _ => Some(
            rest.rsplit_once("$overload")
                .map_or(rest, |(member, _)| member),
        ),
    };
    Some((kind, package, class_path, member))
}

/// `flags`: bit 0 asks for a handle id, bit 1 awaits a returned Promise (its settled value is the result),
/// bit 2 awaits a returned Promise only when the declared return type is a union with a Promise member
/// (`Response | Promise<Response>`); the same settle as bit 1 when the result is a Promise.
fn class_call(
    symbol: &str,
    args_json: &str,
    receiver: i64,
    flags: u8,
) -> Result<String, HostError> {
    let (want_handle, await_promise, await_union) =
        (flags & 1 != 0, flags & 2 != 0, flags & 4 != 0);
    let (kind, package, class_path, member) = parse_class_symbol(symbol)
        .ok_or_else(|| HostError::Bundle(format!("malformed class symbol `{symbol}`")))?;
    let op = match kind {
        "new" => ClassOp::New { class: class_path },
        "value" => ClassOp::Value { path: class_path },
        "method" | "methodvoid" => ClassOp::Method {
            receiver,
            name: member.unwrap_or(""),
        },
        "setter" => ClassOp::Setter {
            receiver,
            name: member.unwrap_or(""),
        },
        "getter" => ClassOp::Getter {
            receiver,
            name: member.unwrap_or(""),
        },
        "staticgetter" => ClassOp::StaticGetter {
            class: class_path,
            name: member.unwrap_or(""),
        },
        "staticsetter" => ClassOp::StaticSetter {
            class: class_path,
            name: member.unwrap_or(""),
        },
        "staticmethod" | "staticmethodvoid" => ClassOp::Static {
            class: class_path,
            name: member.unwrap_or(""),
        },
        other => {
            return Err(HostError::Bundle(format!(
                "unsupported class operation `{other}`"
            )))
        }
    };
    let package_handle = match &op {
        ClassOp::Method { receiver, .. }
        | ClassOp::Setter { receiver, .. }
        | ClassOp::Getter { receiver, .. } => slot(*receiver as u64)
            .map(|(p, _)| p)
            .ok_or_else(|| HostError::Bundle(stale_handle(*receiver as u64)))?,
        _ => BY_NAME
            .with(|m| m.borrow().get(package).copied())
            .ok_or_else(|| HostError::Bundle(not_loaded(package)))?,
    };
    let Native::Array(args) = Native::from_json(args_json)? else {
        return Err(HostError::Bundle("arguments must be a JSON array".into()));
    };
    with_package(package_handle, |p| {
        let it = &mut p.interp;
        let args: Vec<Value> = args.iter().map(|a| to_value(it, a)).collect();
        let result = match op {
            ClassOp::New { class } => {
                let ctor = p.exports_member(class)?;
                let it = &mut p.interp;
                it.construct(&ctor, &args).map_err(|c| classify(it, c))?
            }
            ClassOp::Value { path } => p.exports_member(path)?,
            ClassOp::Method { receiver, name } => {
                let this = slot(receiver as u64).map_or(Value::Undefined, |(_, v)| v);
                let it = &mut p.interp;
                let f = it.get(&this, name).map_err(|c| classify(it, c))?;
                if !f.is_callable() {
                    let message =
                        format!("`{name}` is not a function (class operation `{symbol}`)");
                    return Err(not_callable(it, message));
                }
                it.call(&f, &this, &args).map_err(|c| classify(it, c))?
            }
            ClassOp::Setter { receiver, name } => {
                let this = slot(receiver as u64).map_or(Value::Undefined, |(_, v)| v);
                let it = &mut p.interp;
                let value = args.first().cloned().unwrap_or(Value::Undefined);
                strict_host_set(it, &this, name, value.clone()).map_err(|c| classify(it, c))?;
                value
            }
            ClassOp::Getter { receiver, name } => {
                let this = slot(receiver as u64).map_or(Value::Undefined, |(_, v)| v);
                let it = &mut p.interp;
                it.get(&this, name).map_err(|c| classify(it, c))?
            }
            ClassOp::StaticGetter { class, name } => {
                let ctor = p.exports_member(class)?;
                let it = &mut p.interp;
                it.get(&ctor, name).map_err(|c| classify(it, c))?
            }
            ClassOp::StaticSetter { class, name } => {
                let ctor = p.exports_member(class)?;
                let it = &mut p.interp;
                let value = args.first().cloned().unwrap_or(Value::Undefined);
                strict_host_set(it, &ctor, name, value.clone()).map_err(|c| classify(it, c))?;
                value
            }
            ClassOp::Static { class, name } => {
                let ctor = p.exports_member(class)?;
                let it = &mut p.interp;
                let f = it.get(&ctor, name).map_err(|c| classify(it, c))?;
                if !f.is_callable() {
                    let message =
                        format!("`{name}` is not a function (class operation `{symbol}`)");
                    return Err(not_callable(it, message));
                }
                it.call(&f, &ctor, &args).map_err(|c| classify(it, c))?
            }
        };
        let it = &mut p.interp;
        let result = if await_promise || (await_union && it.is_promise(&result)) {
            awaited(it, result)?
        } else {
            result
        };
        if want_handle {
            return Ok(store_handle(package_handle, result).to_string());
        }
        from_value(it, &result, 0).map(|n| n.to_json())
    })
}

/// Run a class operation (see `parse_class_symbol`): construct (`receiver` 0), call or read/write a member
/// of an instance handle, or call or read/write a static member of the class. The result is JSON text, or a decimal handle id when
/// bit 0 of `flags` is set (the class instance stays in the thaw-js heap); bit 1 awaits a Promise result,
/// bit 2 awaits one only for a union return type (see `class_call`).
#[no_mangle]
pub extern "C" fn thaw_tjs_class_call(
    symbol: *const c_char,
    args_json: *const c_char,
    receiver: i64,
    flags: u8,
) -> ThawResult {
    text_result((|| {
        let (Some(symbol), Some(args)) = (cstr(symbol), cstr(args_json)) else {
            return Err("invalid call text".to_string());
        };
        if let Some(message) = symbol.strip_prefix('\u{1}') {
            return Err(message.to_string());
        }
        let out = class_call(&symbol, &args, receiver, flags).map_err(|e| e.to_string());
        pump_watches();
        out
    })())
}

pub(crate) fn graph_field<'a>(fields: &'a [(String, Native)], key: &str) -> Option<&'a Native> {
    fields.iter().find(|(k, _)| k == key).map(|(_, v)| v)
}

/// Decoder state of one graph wire. `napi` selects the N-API wire rules (`su`, property attributes,
/// `p`, cycles); `release_in_node` is false when the wire lists its leases, which `decode_napi_wire`
/// releases once, after decoding.
pub(crate) struct GraphCx {
    pub(crate) memo: Vec<Option<Value>>,
    pub(crate) busy: Vec<bool>,
    pub(crate) napi: bool,
    pub(crate) release_in_node: bool,
    /// The package the wire's native names (Symbol registry, pair proofs) belong to.
    pub(crate) owner: usize,
}

impl GraphCx {
    pub(crate) fn new(nodes: usize, napi: bool, release_in_node: bool, owner: usize) -> Self {
        GraphCx {
            memo: vec![None; nodes],
            busy: vec![false; nodes],
            napi,
            release_in_node,
            owner,
        }
    }
}

/// One graph-wire token (`{"v":..}`, `{"r":i}`, `{"nf":..}`, `{"u":1}`) as a value.
pub(crate) fn graph_token(
    it: &mut Interp,
    token: &Native,
    nodes: &[Native],
    cx: &mut GraphCx,
) -> Result<Value, String> {
    let Native::Object(fields) = token else {
        return Err("malformed graph token".into());
    };
    if cx.napi && fields.len() <= 2 && graph_field(fields, "nsy").is_some() {
        return native_symbol_pair(it, fields, cx);
    }
    let [(key, payload)] = fields.as_slice() else {
        return Err("malformed graph token".into());
    };
    match (key.as_str(), payload) {
        ("v", v) => Ok(to_value(it, v)),
        ("u", _) => Ok(Value::Undefined),
        ("nf", Native::Str(s)) => Ok(Value::Number(match s.as_str() {
            "-0" => -0.0,
            "NaN" => f64::NAN,
            "Infinity" => f64::INFINITY,
            "-Infinity" => f64::NEG_INFINITY,
            _ => return Err("malformed graph number".into()),
        })),
        ("su", Native::Array(units)) if cx.napi => {
            let units = units
                .iter()
                .map(|u| match u {
                    Native::Number(n) if (0.0..=65535.0).contains(n) && n.fract() == 0.0 => {
                        Ok(*n as u16)
                    }
                    _ => Err("malformed graph string".to_string()),
                })
                .collect::<Result<Vec<u16>, String>>()?;
            Ok(Value::str(&crate::builtins::units_str(&units)))
        }
        ("r", Native::Number(i)) => graph_node(it, *i as usize, nodes, cx),
        ("bi", Native::Str(decimal)) => Ok(Value::BigInt(Rc::new(
            crate::bigint::parse_bigint(decimal).ok_or("malformed graph bigint")?,
        ))),
        _ => Err(format!("unsupported graph token `{key}`")),
    }
}

/// `new <global ctor>(args)`.
fn construct_global(it: &mut Interp, name: &str, args: &[Value]) -> Result<Value, String> {
    let global = it.global.clone();
    let ctor = it
        .lookup(&global, name)
        .ok_or_else(|| format!("{name} is not defined"))?;
    it.construct(&ctor, args)
        .map_err(|c| classify(it, c).to_string())
}

/// A `Date` of time value `ms` (`null`, how `NaN` crosses as JSON, is an Invalid Date).
fn new_date(it: &mut Interp, ms: &Native) -> Result<Value, String> {
    let ms = match ms {
        Native::Number(n) => *n,
        _ => f64::NAN,
    };
    construct_global(it, "Date", &[Value::Number(ms)])
}

/// The Symbol of a `{"nsy","hdl"}` token: the handle it names (the wire's reference is released, as for `hdl`).
/// Under the N-API wire the pair is proven first, and a lone `{"nsy"}` names a Symbol of the owner.
fn native_symbol_pair(
    it: &mut Interp,
    fields: &[(String, Native)],
    cx: &GraphCx,
) -> Result<Value, String> {
    if cx.napi {
        return match (graph_field(fields, "nsy"), graph_field(fields, "hdl")) {
            (Some(Native::Str(native)), None) => {
                crate::napi_graph::lone_symbol(it, cx.owner, native)
            }
            _ => crate::napi_graph::native_pair(cx.owner, fields, 2, cx.release_in_node),
        };
    }
    let release = cx.release_in_node;
    let Some(Native::Number(id)) = graph_field(fields, "hdl") else {
        return Err("invalid paired native Symbol graph token".into());
    };
    let value = slot(*id as u64).map(|(_, v)| v);
    let Some(value) = value.filter(crate::napi_symbols::is_symbol) else {
        return Err("invalid paired native Symbol graph token".into());
    };
    if release {
        thaw_tjs_release_handle(*id as i64);
    }
    Ok(value)
}

/// A property key of a graph object: a string, or a Symbol token (`{"nsy","hdl"}`, a node reference, or a handle).
fn graph_key(
    it: &mut Interp,
    key: &Native,
    nodes: &[Native],
    cx: &mut GraphCx,
) -> Result<String, String> {
    if let Native::Str(name) = key {
        return Ok(name.clone());
    }
    let value = match key {
        Native::Object(fields) if graph_field(fields, "nsy").is_some() => {
            native_symbol_pair(it, fields, cx)?
        }
        _ => graph_token(it, key, nodes, cx)?,
    };
    if let Some(name) = crate::napi_symbols::symbol_key(&value) {
        return Ok(name);
    }
    if matches!(value, Value::Str(_)) {
        return Ok(it.to_str(&value).to_string());
    }
    Err("malformed graph property key".into())
}

/// The attribute flags of an `o` or `p` entry (`1` writable, `2` enumerable, `4` configurable): its third
/// element under the N-API wire, all attributes when absent. The plain wire ignores the element.
fn graph_attributes(rest: &[Native], napi: bool) -> Result<u8, String> {
    match rest.first() {
        Some(Native::Number(n)) if napi && n.fract() == 0.0 && (0.0..=7.0).contains(n) => {
            Ok(*n as u8)
        }
        None => Ok(7),
        Some(_) if !napi => Ok(7),
        _ => Err("Invalid native JSON graph property attributes".into()),
    }
}

/// Set `name` on the object `target` with `flags`. All attributes is a plain put; the others go through
/// `define_slot`.
fn graph_put(it: &mut Interp, target: &Obj, name: &str, v: Value, flags: u8) -> Result<(), String> {
    if flags == 7 {
        crate::put(target, name, v);
        return Ok(());
    }
    crate::napi_props::define_slot(
        it,
        &Value::Object(target.clone()),
        name,
        Slot::Data(v),
        flags & 2 != 0,
        flags & 1 != 0,
        flags & 4 != 0,
    )
    .map_err(|c| classify(it, c).to_string())
}

/// An `a` node: its elements, then (N-API wire) the own properties `p` as `[key, token, flags]`. Under the
/// N-API wire the array is memoized before its elements decode, so a cycle back to it resolves.
fn graph_array(
    it: &mut Interp,
    index: usize,
    items: &[Native],
    props: &[Native],
    nodes: &[Native],
    cx: &mut GraphCx,
) -> Result<Value, String> {
    let arr = it.array(Vec::new());
    let Value::Object(obj) = arr.clone() else {
        return Err("malformed graph array".into());
    };
    if cx.napi {
        cx.memo[index] = Some(arr.clone());
    }
    let mut out = Vec::with_capacity(items.len());
    for item in items {
        out.push(match item {
            Native::Object(f) if graph_field(f, "h").is_some() => Value::Hole,
            _ => graph_token(it, item, nodes, cx)?,
        });
    }
    *obj.kind.borrow_mut() = Kind::Array(out);
    for pair in props {
        let Native::Array(kv) = pair else {
            return Err("malformed graph property".into());
        };
        let [key, token, rest @ ..] = kv.as_slice() else {
            return Err("malformed graph property".into());
        };
        let name = graph_key(it, key, nodes, cx)?;
        // Elements came with `a`; `length` is the element count. Neither has its own attributes here.
        if name == "length" || array_index(&name).is_some() {
            continue;
        }
        let flags = graph_attributes(rest, true)?;
        let v = graph_token(it, token, nodes, cx)?;
        graph_put(it, &obj, &name, v, flags)?;
    }
    Ok(arr)
}

/// Node `index` of a graph wire; shared nodes decode to one value (identity kept).
fn graph_node(
    it: &mut Interp,
    index: usize,
    nodes: &[Native],
    cx: &mut GraphCx,
) -> Result<Value, String> {
    if let Some(Some(v)) = cx.memo.get(index) {
        return Ok(v.clone());
    }
    let node = nodes.get(index).ok_or("graph node out of range")?;
    // ponytail: cyclic graphs are rejected (containers are built whole); fill-after-create if a package needs them.
    if std::mem::replace(&mut cx.busy[index], true) {
        return Err("cyclic graph is not supported".into());
    }
    let Native::Object(fields) = node else {
        return Err("malformed graph node".into());
    };
    let value = match fields.as_slice() {
        [(k, Native::Array(items))] if k == "a" => graph_array(it, index, items, &[], nodes, cx)?,
        [(k, Native::Array(items)), (pk, Native::Array(props))]
            if cx.napi && k == "a" && pk == "p" =>
        {
            graph_array(it, index, items, props, nodes, cx)?
        }
        [(k, Native::Array(pairs))] if k == "o" => {
            // The one-key markers compiled code puts in a `Json` (a live thaw-js value, a compiled callback,
            // a timer, `undefined`), as `to_value` reads them from plain data.
            if let [Native::Array(kv)] = pairs.as_slice() {
                if let [Native::Str(name), Native::Object(v), ..] = kv.as_slice() {
                    let found = match (name.as_str(), graph_field(v, "v")) {
                        (HANDLE_KEY, Some(Native::Number(id))) => slot(*id as u64).map(|(_, v)| v),
                        (CALLBACK_KEY, Some(Native::Number(id))) => {
                            Some(callback_value(it, *id as usize))
                        }
                        (TIMER_KEY, Some(Native::Number(id))) => Some(it.timer_object(*id as u64)),
                        (UNDEFINED_KEY, Some(Native::Bool(true))) => Some(Value::Undefined),
                        _ => None,
                    };
                    if let Some(found) = found {
                        cx.memo[index] = Some(found.clone());
                        return Ok(found);
                    }
                }
            }
            let o = it.plain();
            if cx.napi {
                cx.memo[index] = Some(Value::Object(o.clone()));
            }
            for pair in pairs {
                let Native::Array(kv) = pair else {
                    return Err("malformed graph property".into());
                };
                let [key, token, rest @ ..] = kv.as_slice() else {
                    return Err("malformed graph property".into());
                };
                let flags = graph_attributes(rest, cx.napi)?;
                let name = graph_key(it, key, nodes, cx)?;
                let v = graph_token(it, token, nodes, cx)?;
                graph_put(it, &o, &name, v, flags)?;
            }
            Value::Object(o)
        }
        [_, _] if graph_field(fields, "nsy").is_some() => native_symbol_pair(it, fields, cx)?,
        [_, _] if cx.napi && graph_field(fields, "nfn").is_some() => {
            crate::napi_graph::native_pair(cx.owner, fields, 1, cx.release_in_node)?
        }
        [(k, Native::Str(native))]
            if cx.napi && (k == "nh" || k == "nfn") && crate::napi_graph::is_decimal(native) =>
        {
            crate::napi_graph::lone_handle(it, k, native)?
        }
        [(k, Native::Number(id))] if k == "hdl" => {
            let v = slot(*id as u64).map_or(Value::Undefined, |(_, v)| v);
            // The encoder retained this reference for the wire; the decoded value holds its own.
            if cx.release_in_node {
                thaw_tjs_release_handle(*id as i64);
            }
            v
        }
        [(k, ms)] if k == "d" => new_date(it, ms)?,
        [(k, Native::Str(decimal))] if k == "bi" => Value::BigInt(Rc::new(
            crate::bigint::parse_bigint(decimal).ok_or("malformed graph bigint")?,
        )),
        [(k, Native::Array(parts))] if k == "re" => {
            let (Some(Native::Str(source)), Some(Native::Str(flags))) =
                (parts.first(), parts.get(1))
            else {
                return Err("malformed graph regexp".into());
            };
            it.new_regexp(source, flags)
                .map_err(|c| classify(it, c).to_string())?
        }
        [(k, token)] if k == "m" || k == "s" => {
            let items = graph_token(it, token, nodes, cx)?;
            construct_global(it, if k == "m" { "Map" } else { "Set" }, &[items])?
        }
        // `{"b":[0..255,...]}`: a branded Buffer/ArrayBuffer wrapper from compiled code, rebuilt as a `Buffer` exactly
        // like the `__thaw_tjs_bytes__` arm of `to_value`.
        [(k, Native::Array(items))] if k == "b" => {
            let bytes: Vec<u8> = items
                .iter()
                .map(|item| match item {
                    Native::Number(n) => *n as u8,
                    _ => 0,
                })
                .collect();
            let view = it.u8_from_vec(bytes);
            let global = it.global.clone();
            let proto = it
                .lookup(&global, "Buffer")
                .and_then(|b| it.get(&b, "prototype").ok());
            if let (Value::Object(o), Some(Value::Object(p))) = (&view, proto) {
                *o.proto.borrow_mut() = Some(p);
            }
            view
        }
        _ => return Err("unsupported graph node (Buffer)".into()),
    };
    cx.memo[index] = Some(value.clone());
    Ok(value)
}

#[repr(C)]
pub struct HandleResult {
    pub value: u64,
    pub error: *mut c_char,
}

/// The realm user code without a package runs in (`new Function`, `eval`, `any` literals): the
/// package last entered, or, in a program that loaded none, a bare interpreter created on first use.
/// ponytail: a program mixing packages and dynamic code shares the last-entered package's realm.
pub(crate) fn default_realm() -> usize {
    if PACKAGES.with(|ps| ps.borrow().is_empty()) {
        let mut interp = Box::new(Interp::runtime());
        interp.share_process_state(crate::procstate::host_shared());
        // console output goes straight to stdout: nothing in the host drains `Interp::out`
        interp.set_stream(true);
        let stack = process_stack_bytes();
        if stack != 0 {
            interp.configure_stack(stack);
        }
        let package = Package {
            interp,
            exports: Value::Undefined,
        };
        PACKAGES.with(|ps| ps.borrow_mut().push(Some(package)));
        LAST_PACKAGE.with(|l| l.set(0));
    }
    LAST_PACKAGE.with(|l| l.get())
}

/// `new Function(...args)` from the JSON array of the constructor's string arguments (the last is the
/// body); the function is kept as a handle. The counterpart of QuickJS's `thaw_js_new_function`.
#[no_mangle]
pub extern "C" fn thaw_tjs_new_function(args_json: *const c_char) -> HandleResult {
    handle_result((|| {
        let args = cstr(args_json).ok_or("invalid `new Function` arguments")?;
        let Native::Array(args) = Native::from_json(&args).map_err(|e| e.to_string())? else {
            return Err("invalid `new Function` arguments".to_string());
        };
        let owner = default_realm();
        with_package(owner, |p| {
            let it = &mut p.interp;
            let args = args.iter().map(|a| to_value(it, a)).collect::<Vec<_>>();
            let ctor = it
                .lookup(&it.global.clone(), "Function")
                .unwrap_or(Value::Undefined);
            let f = it.construct(&ctor, &args).map_err(|c| classify(it, c))?;
            Ok(store_handle(owner, f))
        })
        .map_err(error_text)
    })())
}

/// Call the global function `name` (a dotted path, `eval`) with the graph-wire argument array; the
/// result is a plain graph wire. The counterpart of QuickJS's `thaw_js_call_graph_result`.
#[no_mangle]
pub extern "C" fn thaw_tjs_call_graph_result(
    name: *const c_char,
    args_json: *const c_char,
) -> ThawResult {
    graph_result((|| {
        let name = cstr(name).ok_or("invalid function name")?;
        let args_json = cstr(args_json).ok_or("invalid arguments")?;
        let (owner, args) = decode_wire(&args_json)?;
        with_package(owner, |p| {
            let it = &mut p.interp;
            let args = it.array_items(&args);
            let mut parts = name.split('.');
            let first = parts.next().unwrap_or("");
            let mut this = Value::Undefined;
            let mut f = it
                .lookup(&it.global.clone(), first)
                .unwrap_or(Value::Undefined);
            for part in parts {
                this = f.clone();
                f = it.get(&this, part).map_err(|c| classify(it, c))?;
            }
            let result = it.call(&f, &this, &args).map_err(|c| classify(it, c))?;
            let result = awaited(it, result)?;
            let mut out = WireOut {
                owner,
                plain: true,
                live_functions: true,
                copy_root: false,
                receiver: None,
                nodes: Vec::new(),
                seen: Vec::new(),
                leases: Vec::new(),
            };
            let root = out.token(it, &result)?;
            Ok(out.finish(root))
        })
        .map_err(error_text)
    })())
}

/// Decode a graph wire (`thaw_json_graph_encode`) into `(owning package, value)`. The package is the one
/// that owns a handle the wire references, else the package last entered (a value only needs an
/// interpreter to be built in and a package for later method calls on it).
pub(crate) fn decode_wire(wire: &str) -> Result<(usize, Value), String> {
    if std::env::var_os("THAW_JS_TRACE_WIRE").is_some() {
        eprintln!("thaw-js decode: {wire}");
    }
    let Native::Object(top) = Native::from_json(wire).map_err(|e| e.to_string())? else {
        return Err("malformed graph wire".into());
    };
    let root = graph_field(&top, "root").ok_or("graph wire has no root")?;
    let nodes = match graph_field(&top, "nodes") {
        Some(Native::Array(nodes)) => nodes.as_slice(),
        _ => &[],
    };
    let owner = wire_owner(nodes);
    with_package(owner, |p| {
        let mut cx = GraphCx::new(nodes.len(), false, true, owner);
        graph_token(&mut p.interp, root, nodes, &mut cx).map_err(HostError::Bundle)
    })
    .map(|v| (owner, v))
    .map_err(|e| e.to_string())
}

/// The package that owns a handle a graph wire's nodes reference, else the package last entered.
pub(crate) fn wire_owner(nodes: &[Native]) -> usize {
    nodes
        .iter()
        .find_map(|n| match n {
            Native::Object(f) => match graph_field(f, "hdl") {
                Some(Native::Number(id)) => slot(*id as u64).map(|(p, _)| p),
                _ => None,
            },
            _ => None,
        })
        .unwrap_or_else(default_realm)
}

pub(crate) fn handle_result(result: Result<i64, String>) -> HandleResult {
    match result {
        Ok(handle) => HandleResult {
            value: handle as u64,
            error: ptr::null_mut(),
        },
        Err(e) => HandleResult {
            value: 0,
            error: thaw_arena::owned_string(e),
        },
    }
}

/// Retain the value a graph wire (`thaw_json_graph_encode`) describes as a thaw-js handle. The
/// counterpart of QuickJS's `thaw_js_retain_graph_result`.
#[no_mangle]
pub extern "C" fn thaw_tjs_retain_graph_result(wire: *const c_char) -> HandleResult {
    handle_result((|| {
        let wire = cstr(wire).ok_or("invalid graph wire")?;
        let (owner, value) = decode_wire(&wire)?;
        Ok(store_handle(owner, value))
    })())
}

/// The error text a failed handle operation reports: the framed form of the last `Error` object thrown.
pub(crate) fn error_text(e: HostError) -> String {
    let display = e.to_string();
    LAST_FRAME.with(|f| match f.borrow_mut().take() {
        Some((d, framed)) if d == display => framed,
        _ => display,
    })
}

/// What handle `id` names: `(owner package, value)`. Id 0 is `undefined`, owned by the default realm, at
/// every thaw-js boundary. A released id keeps its slot holding `undefined` (see `release`). None only
/// for an id past the table, which was never allocated.
pub(crate) fn slot(id: u64) -> Option<(usize, Value)> {
    if id == 0 {
        return Some((default_realm(), Value::Undefined));
    }
    HANDLES.with(|h| h.borrow().get((id as usize).wrapping_sub(1)).cloned())
}

/// The error for an id past the handle table.
pub(crate) fn stale_handle(id: u64) -> String {
    format!("invalid JavaScript handle {id}: released or never allocated")
}

/// The N-API seam's resolver (decision D1): Node-API's NULL `napi_value` is invalid there, so id 0 is an
/// error, not `undefined`. Its text is the bare "invalid JavaScript handle" the N-API tests pin.
pub(crate) fn slot_strict(id: u64) -> Result<(usize, Value), String> {
    match slot(id) {
        Some(entry) if id != 0 => Ok(entry),
        _ => Err("invalid JavaScript handle".to_string()),
    }
}

/// Run `f(interpreter, package, value)` on the value behind `handle`, in the package that owns it.
pub(crate) fn on_handle<T>(
    handle: u64,
    f: impl FnOnce(&mut Package, usize, Value) -> Result<T, HostError>,
) -> Result<T, String> {
    let (owner, value) = slot(handle).ok_or_else(|| stale_handle(handle))?;
    with_package(owner, |p| f(p, owner, value)).map_err(error_text)
}

/// `on_handle` for the N-API seam: id 0 is an error (see `slot_strict`).
pub(crate) fn on_handle_strict<T>(
    handle: u64,
    f: impl FnOnce(&mut Package, usize, Value) -> Result<T, HostError>,
) -> Result<T, String> {
    let (owner, value) = slot_strict(handle)?;
    with_package(owner, |p| f(p, owner, value)).map_err(error_text)
}

fn property(p: &mut Package, owner: usize, value: &Value, key: &str) -> Result<i64, HostError> {
    let it = &mut p.interp;
    let v = it.get(value, key).map_err(|c| classify(it, c))?;
    Ok(store_handle(owner, v))
}

/// Read property `name` (plain C string) of the value behind `handle`; the result is a new handle.
#[no_mangle]
pub extern "C" fn thaw_tjs_get_property_result(handle: u64, name: *const c_char) -> HandleResult {
    handle_result((|| {
        let name = cstr(name).ok_or("invalid property name")?;
        on_handle(handle, |p, owner, v| property(p, owner, &v, &name))
    })())
}

/// As [`thaw_tjs_get_property_result`], the key being a JSON string literal.
#[no_mangle]
pub extern "C" fn thaw_tjs_get_property_json_key_result(
    handle: u64,
    key_json: *const c_char,
) -> HandleResult {
    handle_result((|| {
        let key_json = cstr(key_json).ok_or("invalid property key")?;
        let Native::Str(key) = Native::from_json(&key_json).map_err(|e| e.to_string())? else {
            return Err("Invalid host property key".to_string());
        };
        if std::env::var_os("THAW_JS_TRACE_WIRE").is_some() {
            eprintln!("thaw-js get({handle}, {key})");
        }
        on_handle(handle, |p, owner, v| property(p, owner, &v, &key))
    })())
}

#[repr(C)]
pub struct ExactResult {
    pub value: u64,
    pub exception_handle: u64,
    pub exception_object_like: u8,
    pub error: *mut c_char,
}

/// Property read whose thrown value survives as a handle (`exception_handle`) instead of text.
#[no_mangle]
pub extern "C" fn thaw_tjs_get_property_exact_result(
    handle: u64,
    name: *const c_char,
) -> ExactResult {
    let none = ExactResult {
        value: 0,
        exception_handle: 0,
        exception_object_like: 0,
        error: ptr::null_mut(),
    };
    let Some(name) = cstr(name) else {
        return ExactResult {
            error: thaw_arena::owned_string("invalid property name"),
            ..none
        };
    };
    let result = on_handle(handle, |p, owner, v| {
        let it = &mut p.interp;
        Ok(match it.get(&v, &name) {
            Ok(v) => Ok(store_handle(owner, v)),
            Err(Ctl::Throw(t)) => {
                let like = matches!(t, Value::Object(_));
                Err((store_handle(owner, t), like))
            }
            Err(c) => return Err(classify(it, c)),
        })
    });
    match result {
        Ok(Ok(value)) => ExactResult {
            value: value as u64,
            ..none
        },
        Ok(Err((thrown, like))) => ExactResult {
            exception_handle: thrown as u64,
            exception_object_like: u8::from(like),
            ..none
        },
        Err(e) => ExactResult {
            error: thaw_arena::owned_string(e),
            ..none
        },
    }
}

/// The native packet a thrown thaw-js Error was built from: its hidden native id, while that packet is
/// live; 0 for any other value (a plain Error, or a packet already released).
#[no_mangle]
pub extern "C" fn thaw_tjs_exception_native_token(handle: u64) -> u64 {
    on_handle(handle, |p, _owner, v| {
        let id = match p.interp.get(&v, NATIVE_ID_KEY) {
            Ok(Value::Number(n)) if n >= 1.0 => n as usize,
            _ => 0,
        };
        Ok(thaw_arena::exception::packet_for_identity(id).unwrap_or(0))
    })
    .unwrap_or(0)
}

/// Graph-wire builder for a result value. `plain` copies arrays and objects deeply (symbols become
/// `undefined`, and functions too unless `live_functions`; `toJSON` is honored) like the QuickJS
/// result lane; `receiver` is the value that stays a live handle wherever it recurs.
pub(crate) struct WireOut {
    pub(crate) owner: usize,
    pub(crate) plain: bool,
    /// Plain mode only: functions stay live handles (`{"hdl":id}`) instead of becoming `undefined`.
    pub(crate) live_functions: bool,
    /// Not plain mode: the root container is copied, only what it holds stays live.
    pub(crate) copy_root: bool,
    pub(crate) receiver: Option<(u64, Value)>,
    pub(crate) nodes: Vec<String>,
    pub(crate) seen: Vec<(*const crate::ObjData, usize)>,
    pub(crate) leases: Vec<u64>,
}

impl WireOut {
    /// A host-handle node for `id` (the wire owns one reference, released by the decoder); its token.
    fn lease(&mut self, id: u64) -> String {
        self.leases.push(id);
        self.nodes.push(format!("{{\"hdl\":{id}}}"));
        format!("{{\"r\":{}}}", self.nodes.len() - 1)
    }

    /// A registered native Symbol: its native id and a handle to it (`{"nsy","hdl"}`, as the QuickJS wire).
    fn lease_symbol(&mut self, id: u64, native: u64) -> String {
        self.leases.push(id);
        self.nodes
            .push(format!("{{\"nsy\":\"{native}\",\"hdl\":{id}}}"));
        format!("{{\"r\":{}}}", self.nodes.len() - 1)
    }

    pub(crate) fn token(&mut self, it: &mut Interp, v: &Value) -> Result<String, HostError> {
        let mut v = v.clone();
        if let Some((id, r)) = &self.receiver {
            if matches!((&v, r), (Value::Object(a), Value::Object(b)) if Rc::ptr_eq(a, b)) {
                let id = *id;
                thaw_tjs_retain_handle(id as i64);
                return Ok(self.lease(id));
            }
        }
        if let Some(native) = crate::napi_symbols::native_id_of_value(self.owner as u64, &v) {
            let id = store_handle(self.owner, v.clone()) as u64;
            return Ok(self.lease_symbol(id, native));
        }
        // A Buffer/Uint8Array result crosses as the branded `{"b":[bytes]}` node, which thaw-std rebuilds
        // as a Buffer-shaped wrapper (json.rs:4378) instead of a graph-plain {type,data} copy it will not read as bytes.
        if self.plain {
            let bytes = match &v {
                Value::Object(o) => match &*o.kind.borrow() {
                    Kind::Typed(t) if matches!(t.ty, crate::typed::Ty::U8) => {
                        Some(t.window().iter().map(u8::to_string).collect::<Vec<_>>())
                    }
                    _ => None,
                },
                _ => None,
            };
            if let Some(items) = bytes {
                self.nodes.push(format!("{{\"b\":[{}]}}", items.join(",")));
                return Ok(format!("{{\"r\":{}}}", self.nodes.len() - 1));
            }
        }
        // (A class instance keeps its identity and methods even when it defines `toJSON`.)
        if self.plain && matches!(v, Value::Object(_)) && !v.is_callable() && !is_live_value(it, &v)
        {
            let to_json = it.get(&v, "toJSON").map_err(|c| classify(it, c))?;
            if to_json.is_callable() {
                v = it
                    .call(&to_json, &v, &[Value::str("")])
                    .map_err(|c| classify(it, c))?;
            }
        }
        // A plain result that compiled code already holds by handle stays that handle (same identity).
        if self.plain && (!v.is_callable() || self.live_functions) {
            if let Some(id) = live_handle_of(&v) {
                thaw_tjs_retain_handle(id as i64);
                return Ok(self.lease(id));
            }
        }
        Ok(match &v {
            Value::Undefined | Value::Hole => "{\"u\":1}".into(),
            Value::Null => "{\"v\":null}".into(),
            Value::Bool(b) => format!("{{\"v\":{b}}}"),
            Value::Number(n) if *n == 0.0 && n.is_sign_negative() => "{\"nf\":\"-0\"}".into(),
            Value::Number(n) if n.is_nan() => "{\"nf\":\"NaN\"}".into(),
            Value::Number(n) if n.is_infinite() => {
                format!("{{\"nf\":\"{}Infinity\"}}", if *n < 0.0 { "-" } else { "" })
            }
            Value::Number(n) => format!("{{\"v\":{}}}", crate::number_to_string(*n)),
            Value::Str(s) => {
                let mut q = String::new();
                crate::builtins::json_quote(s, &mut q);
                format!("{{\"v\":{q}}}")
            }
            Value::BigInt(_) if !self.plain => {
                format!("{{\"bi\":\"{}\"}}", it.to_str(&v))
            }
            Value::BigInt(_) => {
                return Err(HostError::Thrown(
                    "TypeError: Do not know how to serialize a BigInt".into(),
                ))
            }
            Value::Object(o) => {
                if self.plain && v.is_callable() && !self.live_functions {
                    return Ok("{\"u\":1}".into());
                }
                // A class instance (prototype other than Object.prototype) keeps its identity and methods.
                let instance = !v.is_callable() && is_live_value(it, &v);
                let first = self.copy_root && self.nodes.is_empty();
                let live = (!self.plain && !first)
                    || v.is_callable()
                    || instance
                    || !matches!(&*o.kind.borrow(), Kind::Plain | Kind::Array(_));
                if live {
                    // ponytail: Date/Map/Set/RegExp/typed-array results stay live handles; give them wire nodes if a package needs the copy
                    let id = store_handle(self.owner, v.clone()) as u64;
                    return Ok(self.lease(id));
                }
                let key = Rc::as_ptr(o);
                if let Some((_, i)) = self.seen.iter().find(|(p, _)| *p == key) {
                    return Ok(format!("{{\"r\":{i}}}"));
                }
                let index = self.nodes.len();
                self.nodes.push(String::new());
                self.seen.push((key, index));
                let node = if matches!(&*o.kind.borrow(), Kind::Array(_)) {
                    let mut parts = Vec::new();
                    for item in it.array_items_raw(&v) {
                        parts.push(match item {
                            Value::Hole => "{\"h\":1}".to_string(),
                            item => self.token(it, &item)?,
                        });
                    }
                    format!("{{\"a\":[{}]}}", parts.join(","))
                } else {
                    let mut parts = Vec::new();
                    for k in it.own_keys(&v) {
                        let val = it.get(&v, &k).map_err(|c| classify(it, c))?;
                        let mut q = String::new();
                        crate::builtins::json_quote(&k, &mut q);
                        parts.push(format!("[{q},{},7]", self.token(it, &val)?));
                    }
                    format!("{{\"o\":[{}]}}", parts.join(","))
                };
                self.nodes[index] = node;
                format!("{{\"r\":{index}}}")
            }
        })
    }

    pub(crate) fn finish(self, root: String) -> String {
        let leases: Vec<String> = self.leases.iter().map(u64::to_string).collect();
        format!(
            "{{\"root\":{root},\"nodes\":[{}],\"leases\":[{}],\"napiLeases\":[]}}",
            self.nodes.join(","),
            leases.join(",")
        )
    }
}

/// Register `wire` with the arena so the decoder accepts its leases (the producer-side grant).
fn graph_ok(wire: String) -> ThawResult {
    if std::env::var_os("THAW_JS_TRACE_WIRE").is_some() {
        eprintln!("thaw-js wire: {wire}");
    }
    let ptr = thaw_arena::owned_string(wire);
    // SAFETY: `ptr` is a fresh arena-owned string.
    if unsafe { thaw_arena::register_owned_graph_wire(ptr) } {
        ThawResult {
            value: ptr,
            error: ptr::null_mut(),
        }
    } else {
        ThawResult {
            value: ptr::null_mut(),
            error: thaw_arena::owned_string("Unable to register JavaScript graph result"),
        }
    }
}

pub(crate) fn graph_result(r: Result<String, String>) -> ThawResult {
    match r {
        Ok(wire) => graph_ok(wire),
        Err(e) => ThawResult {
            value: ptr::null_mut(),
            error: thaw_arena::owned_string(e),
        },
    }
}

/// The exact value behind `handle` as a graph wire: an object stays a live handle (a thrown Promise is
/// not assimilated), a primitive is copied.
#[no_mangle]
pub extern "C" fn thaw_tjs_capture_handle_graph_result(handle: u64) -> ThawResult {
    if std::env::var_os("THAW_JS_TRACE_WIRE").is_some() {
        eprintln!(
            "thaw-js capture handle={handle} thread={:?}",
            std::thread::current().id()
        );
    }
    graph_result(on_handle(handle, |p, owner, v| {
        let mut out = WireOut {
            owner,
            plain: false,
            live_functions: false,
            copy_root: false,
            receiver: Some((handle, v.clone())),
            nodes: Vec::new(),
            seen: Vec::new(),
            leases: Vec::new(),
        };
        let root = out.token(&mut p.interp, &v)?;
        Ok(out.finish(root))
    }))
}

/// Node's overloaded native `http.Server` methods: a dynamic call picks the field by method name and
/// argument shape, as the compiler does for a typed receiver (QuickJS: `native_server_overload`).
fn native_server_overload(
    it: &mut Interp,
    this: &Value,
    name: &str,
    args: &[Value],
) -> Option<Value> {
    let is_function = |index: usize| matches!(args.get(index), Some(v) if v.is_callable());
    let target = match name {
        "listen" if args.len() >= 3 && is_function(2) => "__listenWithHostCallback",
        "listen" if args.len() == 2 && is_function(1) => "__listenWithCallback",
        "close" if args.len() == 1 && is_function(0) => "__closeWithCallback",
        "on" if args.len() == 2
            && matches!(&args[0], Value::Str(_))
            && it.to_str(&args[0]).to_string() == "error" =>
        {
            "__onError"
        }
        _ => return None,
    };
    let field = it.get(this, target).ok()?;
    field.is_callable().then_some(field)
}

/// Call the already-selected method `selected` on `receiver` with the graph-wire argument array; the
/// result is a plain graph wire, the receiver itself staying a live handle.
#[no_mangle]
pub extern "C" fn thaw_tjs_call_selected_method_graph_result(
    receiver: u64,
    selected: u64,
    name: *const c_char,
    args_json: *const c_char,
) -> ThawResult {
    graph_result((|| {
        let args_json = cstr(args_json).ok_or("invalid arguments")?;
        let name = cstr(name).unwrap_or_default();
        let (_, args) = decode_wire(&args_json)?;
        let method = slot(selected)
            .map(|(_, v)| v)
            .ok_or("invalid JavaScript method")?;
        on_handle(receiver, |p, owner, this| {
            let it = &mut p.interp;
            let args = it.array_items(&args);
            let method =
                native_server_overload(it, &this, &name, &args).unwrap_or_else(|| method.clone());
            let result = it
                .call(&method, &this, &args)
                .map_err(|c| classify(it, c))?;
            let result = awaited(it, result)?;
            let mut out = WireOut {
                owner,
                plain: true,
                live_functions: true,
                copy_root: false,
                receiver: Some((receiver, this)),
                nodes: Vec::new(),
                seen: Vec::new(),
                leases: Vec::new(),
            };
            let root = out.token(it, &result)?;
            Ok(out.finish(root))
        })
    })())
}

/// The dynamic prelude's helpers live in the realm that serves compiled code; a package realm that never
/// ran `default_realm` lacks them.
fn default_realm_prelude_check(it: &mut Interp) -> Result<(), HostError> {
    if it
        .lookup(&it.global.clone(), "__thaw_json_host_enumerate")
        .is_none()
    {
        it.run_sloppy(include_str!("dynamic_prelude.js"))
            .map_err(|c| classify(it, c))?;
    }
    Ok(())
}

/// Call the prelude helper `name(value, operation)`; its string result is the query answer.
fn prelude_call(
    it: &mut Interp,
    name: &str,
    v: &Value,
    operation: u8,
) -> Result<String, HostError> {
    default_realm_prelude_check(it)?;
    let helper = it
        .lookup(&it.global.clone(), name)
        .unwrap_or(Value::Undefined);
    let answer = it
        .call(
            &helper,
            &Value::Undefined,
            &[v.clone(), Value::Number(f64::from(operation))],
        )
        .map_err(|c| classify(it, c))?;
    Ok(it.to_str(&answer).to_string())
}

fn unsupported_text(what: &str) -> ThawResult {
    ThawResult {
        value: ptr::null_mut(),
        error: thaw_arena::owned_string(format!(
            "thaw-js does not support the live host operation `{what}`"
        )),
    }
}

// The Json host-operation table (`thaw_json_register_host_operations`) names these; the ones a thaw-js
// package has not needed yet report an error instead of linking the QuickJS runtime.
/// A Buffer: a Uint8Array with `__isBuffer` in its prototype chain (what op 11 and op 29 test).
fn is_buffer_value(v: &Value) -> bool {
    matches!(v, Value::Object(o) if matches!(&*o.kind.borrow(), Kind::Typed(t) if matches!(t.ty, crate::typed::Ty::U8))
        && crate::find_in_chain(o, "__isBuffer").is_some())
}

/// Operation 27: a host typed array other than a Buffer, as `{"n":name,"v":[text,..]}` (the probe
/// thaw-std's `typed_probe_node` reads for console inspection); `""` for anything else. A BigInt item
/// carries an `n` suffix and `-0` keeps its sign, as in QuickJS.
fn typed_probe_text(it: &mut Interp, v: &Value) -> String {
    let Value::Object(o) = v else {
        return String::new();
    };
    if is_buffer_value(v) {
        return String::new();
    }
    let (name, len) = match &*o.kind.borrow() {
        Kind::Typed(t) => (t.ty.name(), t.cur_len()),
        _ => return String::new(),
    };
    let mut items = Vec::with_capacity(len);
    for i in 0..len {
        let item = match &*o.kind.borrow() {
            Kind::Typed(t) => t.get_value(i).unwrap_or(Value::Undefined),
            _ => Value::Undefined,
        };
        items.push(match item {
            Value::Number(n) if n == 0.0 && n.is_sign_negative() => "\"-0\"".to_string(),
            Value::Number(n) => format!("\"{}\"", crate::number_to_string(n)),
            other => format!("\"{}n\"", it.to_str(&other)),
        });
    }
    format!(r#"{{"n":"{name}","v":[{}]}}"#, items.join(","))
}

/// Operation 28: an Error's own extra properties as JSON (`message`, `name` and `stack` excluded), via the
/// realm's `__thaw_error_properties_json`; `"{}"` when there are none.
fn error_properties_json(it: &mut Interp, v: &Value) -> String {
    let g = Value::Object(it.global_object.clone());
    let Ok(helper) = it.get(&g, "__thaw_error_properties_json") else {
        return "{}".to_string();
    };
    match it.call(&helper, &g, std::slice::from_ref(v)) {
        Ok(Value::Str(json)) if !json.is_empty() => json.to_string(),
        _ => "{}".to_string(),
    }
}

/// Introspection of the value behind `handle` for the Json host bridge (the numbering is the QuickJS
/// `__thaw_json_host_query`'s). Kinds thaw-js has no value for (Date, Buffer, Map, Set, native
/// wrappers) answer "no" to the copy queries; operation 29 says whether the value must stay a live handle,
/// and operation 30 whether a plain object or array must (a call argument keeps its identity).
#[no_mangle]
pub extern "C" fn thaw_tjs_host_query_result(handle: u64, operation: u8) -> ThawResult {
    if std::env::var_os("THAW_JS_TRACE_WIRE").is_some() {
        eprintln!("thaw-js host_query({handle}, {operation})");
    }
    text_result(on_handle(handle, |p, _, v| {
        let it = &mut p.interp;
        let flag = |b: bool| if b { "1" } else { "0" }.to_string();
        // a proxy over an array is an array (IsArray looks through the proxy; a revoked one answers "no")
        let is_array = matches!(
            &it.proxy_unwrap(&v, "isArray").unwrap_or(Value::Undefined),
            Value::Object(o) if matches!(&*o.kind.borrow(), Kind::Array(_))
        );
        let is_object = matches!(v, Value::Object(_));
        let number = |it: &mut Interp, v: &Value| {
            let n = it.num(v);
            if n == 0.0 && n.is_sign_negative() {
                "-0".to_string()
            } else {
                crate::number_to_string(n)
            }
        };
        Ok(match operation {
            0 => match &v {
                Value::Undefined | Value::Hole => "undefined",
                Value::Null => "object",
                Value::Bool(_) => "boolean",
                Value::Number(_) => "number",
                Value::Str(_) => "string",
                Value::BigInt(_) => "bigint",
                Value::Object(_) if v.is_callable() => "function",
                Value::Object(o) if matches!(&*o.kind.borrow(), Kind::Symbol { .. }) => "symbol",
                Value::Object(_) => "object",
            }
            .to_string(),
            1 => flag(v.truthy()),
            2 => it.to_str(&v).to_string(),
            3 => {
                let mut q = String::new();
                crate::builtins::json_quote(&it.to_str(&v), &mut q);
                q
            }
            4 => flag(is_array),
            5 => flag(matches!(v, Value::Undefined | Value::Hole)),
            6 => flag(matches!(v, Value::Undefined | Value::Hole | Value::Null)),
            7 => flag(matches!(v, Value::Null)),
            // a function or class instance stays a handle in the text, so it survives as an argument
            8 if is_live_value(it, &v) => handle_marker_json(handle as i64),
            8 => from_value(it, &v, 0)
                .map(|n| n.to_json())
                .unwrap_or_else(|_| "null".into()),
            9 => flag(is_object),
            10 => number(it, &v),
            14 => flag(v.is_callable()),
            16 => number(it, &v),
            // the UTF-16 code units of a string, as a JSON array of numbers
            15 => match &v {
                Value::Str(s) => {
                    let units = crate::builtins::str_units(s);
                    let items: Vec<String> = units.iter().map(u16::to_string).collect();
                    format!("[{}]", items.join(","))
                }
                _ => return Err(HostError::Thrown("TypeError: Expected string".into())),
            },
            26 => flag(matches!(&v, Value::Object(o) if o.proto.borrow().is_none())),
            11 => flag(is_buffer_value(&v)),
            25 => "0".to_string(),
            12 => flag(it.date_target(&v)),
            13 => crate::number_to_string(it.date_time(&v).map_err(|c| classify(it, c))?),
            17..=20 | 22..=24 => prelude_call(it, "__thaw_json_host_query", &v, operation)?,
            21 => native_owner_pointer_hex(&v),
            27 => typed_probe_text(it, &v),
            28 => error_properties_json(it, &v),
            // a value that must cross a thaw-js call argument as a live handle: a typed array other
            // than Uint8Array, an ArrayBuffer, a Map, a Set or a Proxy (the native array, collection and
            // class views) would otherwise be flattened to a plain object. A plain Uint8Array is live
            // too (so `util.inspect` keeps it), but a Buffer (`__isBuffer` in its chain) still answers 0
            29 => flag(matches!(&v, Value::Object(o) if matches!(&*o.kind.borrow(),
                Kind::ArrayBuffer(_) | Kind::Map(_) | Kind::Set(_) | Kind::Proxy(_) | Kind::RegExp(_))
                || matches!(&*o.kind.borrow(), Kind::Typed(t) if !matches!(t.ty, crate::typed::Ty::U8))
                || (matches!(&*o.kind.borrow(), Kind::Typed(t) if matches!(t.ty, crate::typed::Ty::U8))
                    && !is_buffer_value(&v)))),
            // 30: a plain object or array (a promoted object literal, a native-class view) passed as a
            // call argument keeps its identity as a live handle instead of a JSON copy. Kept apart from 29
            // so `Json` slots that read 29 (structured clone) see no change.
            30 => flag(
                matches!(&v, Value::Object(o) if matches!(&*o.kind.borrow(), Kind::Plain | Kind::Array(_))),
            ),
            _ => {
                return Err(HostError::Thrown(format!(
                    "TypeError: thaw-js does not support the host query {operation}"
                )))
            }
        })
    }))
}

#[no_mangle]
pub extern "C" fn thaw_tjs_host_date_set_result(_handle: u64, _timestamp: f64) -> ThawResult {
    unsupported_text("date_set")
}

/// Own keys / values / entries of the value behind `handle` as a graph wire (operations 0 entries,
/// 1 and 2 keys, 3 values); the elements of the returned array stay live handles.
#[no_mangle]
pub extern "C" fn thaw_tjs_host_enumerate_result(handle: u64, operation: u8) -> ThawResult {
    if std::env::var_os("THAW_JS_TRACE_WIRE").is_some() {
        eprintln!("thaw-js host_enumerate({handle}, {operation})");
    }
    graph_result(on_handle(handle, |p, owner, v| {
        let it = &mut p.interp;
        default_realm_prelude_check(it)?;
        let list = {
            let helper = it
                .lookup(&it.global.clone(), "__thaw_json_host_enumerate")
                .unwrap_or(Value::Undefined);
            it.call(
                &helper,
                &Value::Undefined,
                &[v, Value::Number(f64::from(operation))],
            )
            .map_err(|c| classify(it, c))?
        };
        let mut out = WireOut {
            owner,
            plain: false,
            live_functions: false,
            copy_root: true,
            receiver: None,
            nodes: Vec::new(),
            seen: Vec::new(),
            leases: Vec::new(),
        };
        let root = out.token(it, &list)?;
        Ok(out.finish(root))
    }))
}

/// A property key from the compiler's JSON key text (a JSON string or number).
pub(crate) fn json_key(key_json: *const c_char) -> Result<String, String> {
    match Native::from_json(&cstr(key_json).ok_or("invalid property key")?) {
        Ok(Native::Str(s)) => Ok(s),
        Ok(Native::Number(n)) => Ok(crate::number_to_string(n)),
        _ => Err("thaw-js supports string and number property keys only".into()),
    }
}

/// Host-side `target[key] = value` under strict mode (compiled code is strict): a write to a frozen object
/// or a new key on a non-extensible one throws, where `Interp::set` silently ignores it.
fn strict_host_set(it: &mut Interp, target: &Value, key: &str, value: Value) -> R<()> {
    if let Value::Object(o) = target {
        if o.frozen.get() {
            return it.throw_error(
                "TypeError",
                &format!("Cannot assign to read only property '{key}' of object"),
            );
        }
        if o.ext.get() != 0 && crate::find_in_chain(o, key).is_none() {
            return it.throw_error(
                "TypeError",
                &format!("Cannot add property {key}, object is not extensible"),
            );
        }
    }
    it.set(target, key, value)
}

/// `handle[key] = value` with the key as JSON text and the value a graph wire; the result value is 1.
#[no_mangle]
pub extern "C" fn thaw_tjs_set_property_graph_result(
    handle: u64,
    key_json: *const c_char,
    graph_json: *const c_char,
) -> HandleResult {
    handle_result((|| {
        let key = json_key(key_json)?;
        let (_, value) = decode_wire(&cstr(graph_json).ok_or("invalid graph")?)?;
        on_handle(handle, |p, _, target| {
            let it = &mut p.interp;
            strict_host_set(it, &target, &key, value).map_err(|c| classify(it, c))?;
            Ok(1)
        })
    })())
}

/// Property predicate with a JSON key: `operation` 0 has, 1 own-has, 2 delete; enumerability queries
/// (3, 4) are not supported.
#[no_mangle]
pub extern "C" fn thaw_tjs_property_predicate_json_key_result(
    handle: u64,
    key_json: *const c_char,
    operation: u8,
) -> HandleResult {
    handle_result((|| {
        let key = json_key(key_json)?;
        on_handle(handle, |p, _, target| {
            property_predicate_value(&mut p.interp, &target, &key, operation)
        })
    })())
}

/// Shared body of `thaw_tjs_property_predicate_json_key_result`: operations 0 (has), 1 (own),
/// 2 (delete), 3 (own and enumerable) and 4 (hide: make the own property non-enumerable).
pub(crate) fn property_predicate_value(
    it: &mut Interp,
    target: &Value,
    key: &str,
    operation: u8,
) -> Result<i64, HostError> {
    let result = match operation {
        0 => it.has_property(target, key).map_err(|c| classify(it, c))?,
        1 => matches!(target, Value::Object(o) if crate::find_own(o, key).is_some()),
        2 => match target {
            Value::Object(o) => {
                crate::reflect::delete_own(it, o, key).map_err(|c| classify(it, c))?
            }
            _ => true,
        },
        3 => {
            matches!(target, Value::Object(o) if crate::find_own(o, key).is_some_and(|(_, enumerable)| enumerable))
        }
        4 => match target {
            Value::Object(o) => {
                if crate::find_own(o, key).is_none() {
                    return Ok(0);
                }
                let desc = it.plain();
                crate::put(&desc, "enumerable", Value::Bool(false));
                it.try_define(o, key, &Value::Object(desc))
                    .map_err(|c| classify(it, c))?
            }
            _ => false,
        },
        _ => {
            return Err(HostError::Unsupported(
                "property enumerability queries".into(),
            ))
        }
    };
    Ok(i64::from(result))
}

/// `handle[name](...args)`; a returned Promise is awaited and the result is a plain graph wire.
#[no_mangle]
pub extern "C" fn thaw_tjs_call_method_graph_result(
    handle: u64,
    name: *const c_char,
    args_json: *const c_char,
) -> ThawResult {
    graph_result((|| {
        let name = cstr(name).ok_or("invalid method name")?;
        let (_, args) = decode_wire(&cstr(args_json).ok_or("invalid arguments")?)?;
        on_handle(handle, |p, owner, this| {
            let it = &mut p.interp;
            let method = it.get(&this, &name).map_err(|c| classify(it, c))?;
            let args = it.array_items(&args);
            let method = native_server_overload(it, &this, &name, &args).unwrap_or(method);
            let result = it
                .call(&method, &this, &args)
                .map_err(|c| classify(it, c))?;
            let result = awaited(it, result)?;
            let mut out = WireOut {
                owner,
                plain: true,
                live_functions: true,
                copy_root: false,
                receiver: Some((handle, this)),
                nodes: Vec::new(),
                seen: Vec::new(),
                leases: Vec::new(),
            };
            let root = out.token(it, &result)?;
            Ok(out.finish(root))
        })
    })())
}

/// `handle[name] = value` (`value_handle` a retained handle); the result value is 1.
#[no_mangle]
pub extern "C" fn thaw_tjs_set_property_result(
    handle: u64,
    name: *const c_char,
    value_handle: u64,
) -> HandleResult {
    handle_result((|| {
        let name = cstr(name).ok_or("invalid property name")?;
        let value = on_handle(value_handle, |_, _, v| Ok(v))?;
        on_handle(handle, |p, _, target| {
            let it = &mut p.interp;
            strict_host_set(it, &target, &name, value).map_err(|c| classify(it, c))?;
            Ok(1)
        })
    })())
}

fn property_predicate(handle: u64, name: *const c_char, delete: bool) -> HandleResult {
    handle_result((|| {
        let name = cstr(name).ok_or("invalid property name")?;
        on_handle(handle, |p, _, target| {
            let it = &mut p.interp;
            let result = if delete {
                match &target {
                    Value::Object(o) => {
                        crate::reflect::delete_own(it, o, &name).map_err(|c| classify(it, c))?
                    }
                    _ => true,
                }
            } else {
                it.has_property(&target, &name)
                    .map_err(|c| classify(it, c))?
            };
            Ok(i64::from(result))
        })
    })())
}

/// `Reflect.deleteProperty(handle, name)`.
#[no_mangle]
pub extern "C" fn thaw_tjs_delete_property_result(
    handle: u64,
    name: *const c_char,
) -> HandleResult {
    property_predicate(handle, name, true)
}

/// `Reflect.has(handle, name)`.
#[no_mangle]
pub extern "C" fn thaw_tjs_has_property_result(handle: u64, name: *const c_char) -> HandleResult {
    property_predicate(handle, name, false)
}

/// The value behind `handle` with a Promise awaited (the event loop runs until it settles), as a new handle.
#[no_mangle]
pub extern "C" fn thaw_tjs_resolve_handle_handle_result(handle: u64) -> HandleResult {
    handle_result(on_handle(handle, |p, owner, v| {
        let it = &mut p.interp;
        let v = adopt_thenable(it, v)?;
        Ok(store_handle(owner, awaited(it, v)?))
    }))
}

/// Emit a native uncaught exception or rejection on the first loaded realm whose `process` listens to one of
/// `events`. The result is the emit's handled flag, or the listener's thrown text; no listener is `0`.
// ponytail: first realm with a matching listener, not the realm the report came from (QuickJS has one `process`).
fn native_process_report(
    events: &[&str],
    emit: impl FnOnce(&mut Interp) -> R<bool>,
) -> HandleResult {
    let count = PACKAGES.with(|ps| ps.borrow().len());
    let owner = (0..count).find(|&handle| {
        with_package(handle, |p| {
            Ok(events.iter().any(|e| p.interp.has_process_listeners(e)))
        })
        .unwrap_or(false)
    });
    let Some(owner) = owner else {
        return HandleResult {
            value: 0,
            error: ptr::null_mut(),
        };
    };
    match with_package(owner, |p| {
        Ok(emit(&mut p.interp).map_err(|c| classify(&mut p.interp, c)))
    }) {
        Ok(Ok(handled)) => HandleResult {
            value: u64::from(handled),
            error: ptr::null_mut(),
        },
        Ok(Err(e)) | Err(e) => HandleResult {
            value: 0,
            error: thaw_arena::owned_string(error_text(e)),
        },
    }
}

/// The `Error` a native report carries: framed (`\u{1}Name\u{1}message`) text, or a plain message.
fn native_report_error(it: &mut Interp, raw: &str) -> R<Value> {
    match tagged_error(it, raw) {
        Ctl::Throw(v) => Ok(v),
        other => Err(other),
    }
}

fn native_report_text(message: *const c_char) -> Option<String> {
    if message.is_null() {
        return None;
    }
    // SAFETY: the compiled side passes a NUL-terminated string.
    Some(
        unsafe { CStr::from_ptr(message) }
            .to_string_lossy()
            .into_owned(),
    )
}

/// A native uncaught exception: `process.emit('uncaughtExceptionMonitor')`, then `uncaughtException`.
#[no_mangle]
pub extern "C" fn thaw_tjs_emit_uncaught_result(message: *const c_char) -> HandleResult {
    let Some(raw) = native_report_text(message) else {
        return HandleResult {
            value: 0,
            error: ptr::null_mut(),
        };
    };
    native_process_report(&["uncaughtExceptionMonitor", "uncaughtException"], |it| {
        let error = native_report_error(it, &raw)?;
        it.emit(
            "uncaughtExceptionMonitor",
            &[error.clone(), Value::str("uncaughtException")],
        )?;
        it.emit(
            "uncaughtException",
            &[error, Value::str("uncaughtException")],
        )
    })
}

/// A native unhandled rejection: an untagged reason is a rejected string, a framed one an `Error`.
#[no_mangle]
pub extern "C" fn thaw_tjs_emit_unhandled_rejection_result(message: *const c_char) -> HandleResult {
    let Some(raw) = native_report_text(message) else {
        return HandleResult {
            value: 0,
            error: ptr::null_mut(),
        };
    };
    native_process_report(&["unhandledRejection"], |it| {
        let reason = if raw.starts_with('\u{1}') {
            native_report_error(it, &raw)?
        } else {
            Value::str(&raw)
        };
        it.emit("unhandledRejection", &[reason, Value::Undefined])
    })
}

/// A previously unhandled Promise gained a handler: `process.emit('rejectionHandled')`.
#[no_mangle]
pub extern "C" fn thaw_tjs_emit_rejection_handled_result() -> HandleResult {
    native_process_report(&["rejectionHandled"], |it| {
        it.emit("rejectionHandled", &[Value::Undefined])
    })
}

/// A genuine `Error` object from the compiler's framed error text, as a new handle.
#[no_mangle]
pub extern "C" fn thaw_tjs_error_from_tagged_handle(value: *const c_char) -> HandleResult {
    handle_result((|| {
        let text = cstr(value).ok_or("null framed Error text")?;
        let owner = default_realm();
        with_package(owner, |p| {
            let it = &mut p.interp;
            let Ctl::Throw(err) = tagged_error(it, &text) else {
                return Err(HostError::Bundle("cannot build an Error".into()));
            };
            Ok(store_handle(owner, err))
        })
        .map_err(error_text)
    })())
}

/// `(owning package, wrapper, pin on the native allocation)`.
type NativeWrapper = (usize, Value, thaw_arena::ArenaRoot);

thread_local! {
    /// The canonical wrapper of each projected native object: `(pointer, layout)` -> `(package, wrapper)`.
    /// The `ArenaRoot` keeps the native allocation alive, so a pointer is never reused for another object.
    static NATIVE_OBJECTS: RefCell<HashMap<(usize, String), NativeWrapper>> =
        RefCell::new(HashMap::new());
    /// The reverse of `NATIVE_OBJECTS` for compiled code: wrapper address (`Rc::as_ptr`) -> the
    /// `(pointer, layout)` pairs it was interned with. The wrappers stay alive in `NATIVE_OBJECTS`,
    /// so an address is never reused for another object.
    static NATIVE_OWNERS: RefCell<HashMap<usize, Vec<(usize, String)>>> =
        RefCell::new(HashMap::new());
    /// Native pointer -> the layouts it has canonical wrappers for, in interning order. A lookup
    /// that misses the exact `(pointer, layout)` key falls back to these, as QuickJS's
    /// `layoutContains` does.
    static NATIVE_LAYOUTS: RefCell<HashMap<usize, Vec<String>>> =
        RefCell::new(HashMap::new());
}

/// The prefix of a class identity name (the chain of class names follows it).
const CLASS_IDENTITY_MARKER: &[u8] = b"__thaw_class_identity_\x1e";

/// Splits a layout into its fields: each field is two `<bytes>:<hex>` pairs, kept as their source text.
/// `None` for a malformed layout.
fn layout_fields(layout: &str) -> Option<Vec<&str>> {
    let bytes = layout.as_bytes();
    let mut fields = Vec::new();
    let mut cursor = 0;
    while cursor < bytes.len() {
        let start = cursor;
        for _ in 0..2 {
            let colon = cursor + bytes.get(cursor..)?.iter().position(|&b| b == b':')?;
            let len: usize = layout.get(cursor..colon)?.parse().ok()?;
            cursor = colon.checked_add(1 + len.checked_mul(2)?)?;
            if cursor > bytes.len() {
                return None;
            }
        }
        fields.push(layout.get(start..cursor)?);
    }
    Some(fields)
}

/// The name a field segment carries: the hex bytes of its first pair.
fn segment_name(segment: &str) -> Option<Vec<u8>> {
    let colon = segment.find(':')?;
    let len: usize = segment.get(..colon)?.parse().ok()?;
    (0..len)
        .map(|index| {
            let at = colon + 1 + index * 2;
            u8::from_str_radix(segment.get(at..at + 2)?, 16).ok()
        })
        .collect()
}

/// A field of the actual layout satisfies a wanted one when they are equal, or when both are class
/// identities and the actual chain ends with `\u{1f}` + the wanted chain (a subclass satisfies its base).
fn segment_matches(have: &str, want: &str) -> bool {
    if have == want {
        return true;
    }
    let (Some(want_name), Some(have_name)) = (segment_name(want), segment_name(have)) else {
        return false;
    };
    let (Some(want_chain), Some(have_chain)) = (
        want_name.strip_prefix(CLASS_IDENTITY_MARKER),
        have_name.strip_prefix(CLASS_IDENTITY_MARKER),
    ) else {
        return false;
    };
    let mut suffix = vec![0x1f];
    suffix.extend_from_slice(want_chain);
    have_chain.len() > want_chain.len() && have_chain.ends_with(&suffix)
}

/// Whether an object interned with layout `actual` satisfies a request for layout `expected`: every
/// wanted field matches a distinct actual field. The empty `expected` matches any well-formed layout.
/// Port of QuickJS's `layoutContains`.
fn layout_contains(actual: &str, expected: &str) -> bool {
    if actual == expected {
        return true;
    }
    let (Some(have), Some(want)) = (layout_fields(actual), layout_fields(expected)) else {
        return false;
    };
    let mut used = vec![false; have.len()];
    for wanted in want {
        let mut found = false;
        for (index, field) in have.iter().enumerate() {
            if !used[index] && segment_matches(field, wanted) {
                used[index] = true;
                found = true;
                break;
            }
        }
        if !found {
            return false;
        }
    }
    true
}

/// The retained canonical wrapper already made for this native object and layout, else 0.
#[no_mangle]
pub extern "C" fn thaw_tjs_lookup_native_object(
    pointer: *const c_void,
    layout: *const c_char,
) -> HandleResult {
    handle_result((|| {
        let layout = cstr(layout).ok_or("native object lookup requires a layout")?;
        if pointer.is_null() {
            return Err("native object lookup requires pointer and layout".to_string());
        }
        let pointer = pointer as usize;
        Ok(NATIVE_OBJECTS.with(|m| {
            let m = m.borrow();
            let entry = m.get(&(pointer, layout.clone())).or_else(|| {
                NATIVE_LAYOUTS.with(|l| {
                    l.borrow()
                        .get(&pointer)?
                        .iter()
                        .find(|actual| layout_contains(actual, &layout))
                        .and_then(|actual| m.get(&(pointer, actual.clone())))
                })
            });
            if std::env::var_os("THAW_JS_TRACE_WIRE").is_some() {
                let outcome = if entry.is_some() { "hit" } else { "miss" };
                eprintln!(
                    "[tjs] lookup_native_object ptr={pointer:?} layout={layout:?} -> {outcome}"
                );
            }
            entry.map_or(0, |(owner, value, _)| store_handle(*owner, value.clone()))
        }))
    })())
}

/// Consumes a candidate wrapper handle and returns one retained canonical wrapper for the native object.
#[no_mangle]
pub extern "C" fn thaw_tjs_intern_native_object(
    pointer: *const c_void,
    layout: *const c_char,
    candidate_handle: u64,
) -> HandleResult {
    let result = handle_result((|| {
        let layout = cstr(layout).ok_or("native object identity requires a layout")?;
        if pointer.is_null() {
            return Err("native object identity requires pointer and layout".to_string());
        }
        let (owner, candidate) =
            slot(candidate_handle).ok_or_else(|| stale_handle(candidate_handle))?;
        Ok(NATIVE_OBJECTS.with(|m| {
            let mut m = m.borrow_mut();
            if std::env::var_os("THAW_JS_TRACE_WIRE").is_some() {
                eprintln!("[tjs] intern_native_object ptr={pointer:?} layout={layout:?}");
            }
            let key = (pointer as usize, layout);
            if !m.contains_key(&key) {
                NATIVE_LAYOUTS.with(|l| {
                    l.borrow_mut().entry(key.0).or_default().push(key.1.clone());
                });
                if let Value::Object(obj) = &candidate {
                    NATIVE_OWNERS.with(|o| {
                        o.borrow_mut()
                            .entry(Rc::as_ptr(obj) as usize)
                            .or_default()
                            .push((key.0, key.1.clone()));
                    });
                }
                // SAFETY: the compiler passes a live arena allocation; the root pins it.
                let root = thaw_arena::ArenaRoot::new(pointer as usize);
                m.insert(key.clone(), (owner, candidate, root));
            }
            let entry = &m[&key];
            store_handle(entry.0, entry.1.clone())
        }))
    })());
    thaw_tjs_release_handle(candidate_handle as i64);
    result
}

/// Host query 21: the native owner pointer (lowercase hex) of an interned native wrapper, under any
/// layout, or "" for anything else. The counterpart of QuickJS's `object_pointer` bridge with an empty layout.
fn native_owner_pointer_hex(v: &Value) -> String {
    let Value::Object(obj) = v else {
        return String::new();
    };
    NATIVE_OWNERS.with(|o| {
        o.borrow()
            .get(&(Rc::as_ptr(obj) as usize))
            .and_then(|owners| {
                owners
                    .iter()
                    .find(|(_, layout)| layout_contains(layout, ""))
            })
            .map_or_else(String::new, |(pointer, _)| format!("{pointer:x}"))
    })
}

/// The native pointer of the owner an interned wrapper (`handle`) was made for with a layout that
/// satisfies `expected_layout`, else 0. A handle that is not such a wrapper is not an error: 0 says no.
#[no_mangle]
pub extern "C" fn thaw_tjs_native_object_pointer(
    handle: u64,
    expected_layout: *const c_char,
) -> HandleResult {
    handle_result((|| {
        let expected = cstr(expected_layout).ok_or("native object pointer requires a layout")?;
        let value = slot(handle).map(|(_, v)| v);
        let trace = std::env::var_os("THAW_JS_TRACE_WIRE").is_some();
        if trace {
            eprintln!("[tjs] native_object_pointer handle={handle} expected_layout={expected:?}");
        }
        let Some(Value::Object(obj)) = value else {
            return Ok(0);
        };
        Ok(NATIVE_OWNERS.with(|o| {
            let o = o.borrow();
            let owners = o.get(&(Rc::as_ptr(&obj) as usize));
            if trace {
                for (pointer, layout) in owners.into_iter().flatten() {
                    eprintln!("[tjs]   owner ptr={pointer:?} layout={layout:?}");
                }
            }
            let result = owners
                .and_then(|owners| {
                    owners
                        .iter()
                        .find(|(_, layout)| layout_contains(layout, &expected))
                })
                .map_or(0, |(pointer, _)| *pointer as i64);
            if trace {
                eprintln!("[tjs]   result={result:?}");
            }
            result
        }))
    })())
}

/// As [`thaw_tjs_call_selected_method_graph_result`], the result kept as a new handle.
#[no_mangle]
pub extern "C" fn thaw_tjs_call_selected_method_handle_graph_args_result(
    receiver: u64,
    selected: u64,
    name: *const c_char,
    args_json: *const c_char,
    defer_resolution: bool,
) -> HandleResult {
    handle_result((|| {
        let args_json = cstr(args_json).ok_or("invalid arguments")?;
        let name = cstr(name).unwrap_or_default();
        let (_, args) = decode_wire(&args_json)?;
        let method = slot(selected)
            .map(|(_, v)| v)
            .ok_or("invalid JavaScript method")?;
        on_handle(receiver, |p, owner, this| {
            let it = &mut p.interp;
            let args = it.array_items(&args);
            let method =
                native_server_overload(it, &this, &name, &args).unwrap_or_else(|| method.clone());
            let result = it
                .call(&method, &this, &args)
                .map_err(|c| classify(it, c))?;
            let result = if defer_resolution {
                result
            } else {
                awaited(it, result)?
            };
            Ok(store_handle(owner, result))
        })
    })())
}

/// A handle on the global `name`: `pkg::Class.path` is a member of that package's exports (the
/// qualified names the class shims use), anything else a property of the global object (dotted paths
/// walk nested properties). 0 when it does not exist.
#[no_mangle]
pub extern "C" fn thaw_tjs_get_global(name: *const c_char) -> i64 {
    let Some(name) = cstr(name) else { return 0 };
    if std::env::var_os("THAW_JS_TRACE_WIRE").is_some() {
        eprintln!("thaw-js get_global({name})");
    }
    let found = (|| -> Result<Option<(usize, Value)>, HostError> {
        if let Some((package, path)) = name.split_once("::") {
            let Some(handle) = BY_NAME.with(|m| m.borrow().get(package).copied()) else {
                return Ok(None);
            };
            return with_package(handle, |p| p.exports_member(path)).map(|v| Some((handle, v)));
        }
        let handle = default_realm();
        with_package(handle, |p| {
            let it = &mut p.interp;
            if name.starts_with("__thaw_") {
                default_realm_prelude_check(it)?;
            }
            // A global whose literal name holds a `.` (a package name) wins over the nested walk.
            if let Some(value) = it
                .lookup(&it.global.clone(), &name)
                .filter(|v| !matches!(v, Value::Undefined))
            {
                return Ok(value);
            }
            let mut parts = name.split('.');
            let first = parts.next().unwrap_or("");
            let mut value = it
                .lookup(&it.global.clone(), first)
                .unwrap_or(Value::Undefined);
            for part in parts {
                value = it.get(&value, part).map_err(|c| classify(it, c))?;
            }
            Ok(value)
        })
        .map(|v| Some((handle, v)))
    })();
    match found {
        Ok(Some((package, value))) if !matches!(value, Value::Undefined) || name == "undefined" => {
            store_handle(package, value)
        }
        _ => {
            if std::env::var_os("THAW_JS_TRACE_WIRE").is_some() {
                eprintln!("thaw-js get_global: `{name}` is not defined");
            }
            0
        }
    }
}

/// `new handle(...args)`; the instance is kept as a new handle.
#[no_mangle]
pub extern "C" fn thaw_tjs_construct_handle_graph_args_result(
    handle: u64,
    args_json: *const c_char,
) -> HandleResult {
    handle_result((|| {
        let args_json = cstr(args_json).ok_or("invalid arguments")?;
        let (_, args) = decode_wire(&args_json)?;
        on_handle(handle, |p, owner, ctor| {
            let it = &mut p.interp;
            let args = it.array_items(&args);
            let value = it.construct(&ctor, &args).map_err(|c| classify(it, c))?;
            Ok(store_handle(owner, value))
        })
    })())
}

/// `handle(...args)` with an `undefined` receiver; the result is kept as a new handle.
#[no_mangle]
pub extern "C" fn thaw_tjs_call_handle_handle_graph_args_result(
    handle: u64,
    args_json: *const c_char,
    defer_resolution: bool,
) -> HandleResult {
    handle_result((|| {
        let args_json = cstr(args_json).ok_or("invalid arguments")?;
        let (_, args) = decode_wire(&args_json)?;
        on_handle(handle, |p, owner, f| {
            let it = &mut p.interp;
            check_handle_callable(it, &f)?;
            let args = it.array_items(&args);
            let value = it
                .call(&f, &Value::Undefined, &args)
                .map_err(|c| classify(it, c))?;
            let value = if defer_resolution {
                value
            } else {
                awaited(it, value)?
            };
            Ok(store_handle(owner, value))
        })
    })())
}

/// `handle(...args)` with an `undefined` receiver; the result is a plain graph wire (a Promise result is awaited).
#[no_mangle]
pub extern "C" fn thaw_tjs_call_handle_graph_result(
    handle: u64,
    args_json: *const c_char,
) -> ThawResult {
    call_handle_graph(handle, args_json, false)
}

/// As [`thaw_tjs_call_handle_graph_result`], the first element of the argument array being the receiver
/// (`this`) of the call (a native callback invoked as a method).
#[no_mangle]
pub extern "C" fn thaw_tjs_call_handle_with_this_graph_wire_result(
    handle: u64,
    args_json: *const c_char,
) -> ThawResult {
    call_handle_graph(handle, args_json, true)
}

fn call_handle_graph(handle: u64, args_json: *const c_char, with_this: bool) -> ThawResult {
    graph_result((|| {
        let args_json = cstr(args_json).ok_or("invalid arguments")?;
        let (_, args) = decode_wire(&args_json)?;
        on_handle(handle, |p, owner, f| {
            let it = &mut p.interp;
            check_handle_callable(it, &f)?;
            let mut args = it.array_items(&args);
            let this = if with_this {
                if args.is_empty() {
                    return Err(HostError::Bundle(
                        "JS callback receiver payload is empty".into(),
                    ));
                }
                args.remove(0)
            } else {
                Value::Undefined
            };
            let result = it.call(&f, &this, &args).map_err(|c| classify(it, c))?;
            let result = awaited(it, result)?;
            let mut out = WireOut {
                owner,
                plain: true,
                live_functions: false,
                copy_root: false,
                receiver: None,
                nodes: Vec::new(),
                seen: Vec::new(),
                leases: Vec::new(),
            };
            let root = out.token(it, &result)?;
            Ok(out.finish(root))
        })
    })())
}

/// A native string (or `null`) as a thaw-js handle: the counterpart of QuickJS's
/// `thaw_js_event_key_handle_result`. `value_json` is a one-element JSON array; `is_symbol` marks a
/// native Symbol token: its string is a wire key text, resolved by `napi_symbols::key_text_symbol`.
#[no_mangle]
pub extern "C" fn thaw_tjs_event_key_handle_result(
    value_json: *const c_char,
    is_symbol: u8,
) -> HandleResult {
    handle_result((|| {
        if is_symbol > 1 {
            return Err("invalid native scalar kind".to_string());
        }
        let wire = cstr(value_json).ok_or("invalid native scalar wire")?;
        let Native::Array(items) = Native::from_json(&wire).map_err(|e| e.to_string())? else {
            return Err("native scalar wire value must be an array".to_string());
        };
        if is_symbol == 1 {
            let [Native::Str(text)] = items.as_slice() else {
                return Err("native Symbol wire array must contain one string".to_string());
            };
            return crate::napi_symbols::key_text_symbol(default_realm(), text);
        }
        let value = match items.as_slice() {
            [Native::Str(s)] => Value::str(s),
            [Native::Null] => Value::Null,
            [_] => return Err("native scalar wire element must be a string or null".to_string()),
            _ => return Err("native scalar wire array must contain one string".to_string()),
        };
        Ok(store_handle(default_realm(), value))
    })())
}

/// `handle(...args, ...direct handles)` with an `undefined` receiver (`handles` is `[count, h1, ...]`,
/// not consumed); a returned Promise is awaited and the result is a plain graph wire.
///
/// # Safety
/// `handles` is a live native handle array (`[u64 count][u64; count]`).
#[no_mangle]
pub unsafe extern "C" fn thaw_tjs_call_handle_mixed_graph_result(
    handle: u64,
    args_json: *const c_char,
    handles: *const u8,
) -> ThawResult {
    let direct: Option<Vec<u64>> = (!handles.is_null()).then(|| {
        let count = unsafe { *(handles.cast::<u64>()) } as usize;
        unsafe { std::slice::from_raw_parts(handles.add(8).cast::<u64>(), count) }.to_vec()
    });
    graph_result((|| {
        let direct = direct.as_ref().ok_or("null JsValue array")?;
        let args_json = cstr(args_json).ok_or("invalid arguments")?;
        let (_, args) = decode_wire(&args_json)?;
        let mut values = Vec::new();
        for h in direct {
            values.push(on_handle(*h, |_, _, v| Ok(v))?);
        }
        on_handle(handle, |p, owner, f| {
            let it = &mut p.interp;
            check_handle_callable(it, &f)?;
            let mut args = it.array_items(&args);
            args.extend(values);
            let called = it
                .call(&f, &Value::Undefined, &args)
                .map_err(|c| classify(it, c))?;
            let result = match it.await_settled(&called).map_err(|c| classify(it, c))? {
                Some(Ok(v)) => v,
                Some(Err(e)) => return Err(classify(it, Ctl::Throw(e))),
                None => return Err(HostError::Thrown("promise never settled".into())),
            };
            let mut out = WireOut {
                owner,
                plain: true,
                live_functions: false,
                copy_root: false,
                receiver: None,
                nodes: Vec::new(),
                seen: Vec::new(),
                leases: Vec::new(),
            };
            let root = out.token(it, &result)?;
            Ok(out.finish(root))
        })
    })())
}

/// Release every handle of a native handle array (`[u64 count][u64; count]`) that was never passed to a consuming call.
///
/// # Safety
/// `handles` is null or a live native handle array.
#[no_mangle]
pub unsafe extern "C" fn thaw_tjs_release_native_handle_array(handles: *const u8) {
    if handles.is_null() {
        return;
    }
    let count = unsafe { *(handles.cast::<u64>()) } as usize;
    for h in unsafe { std::slice::from_raw_parts(handles.add(8).cast::<u64>(), count) } {
        thaw_tjs_release_handle(*h as i64);
    }
}

/// `handle(argument)` where `argument` is itself a live handle (not consumed); the result is a plain graph wire.
/// A returned Promise is awaited (a rejection throws).
#[no_mangle]
pub extern "C" fn thaw_tjs_call_handle_value_graph_result(
    handle: u64,
    argument: u64,
) -> ThawResult {
    graph_result((|| {
        let argument = on_handle(argument, |_, _, v| Ok(v))?;
        on_handle(handle, |p, owner, f| {
            let it = &mut p.interp;
            check_handle_callable(it, &f)?;
            let result = it
                .call(&f, &Value::Undefined, &[argument])
                .map_err(|c| classify(it, c))?;
            let result = awaited(it, result)?;
            let mut out = WireOut {
                owner,
                plain: true,
                live_functions: false,
                copy_root: false,
                receiver: None,
                nodes: Vec::new(),
                seen: Vec::new(),
                leases: Vec::new(),
            };
            let root = out.token(it, &result)?;
            Ok(out.finish(root))
        })
    })())
}

/// The compiler-private exact call: `handle(...args, ...direct handles)`, consuming the callee and every
/// direct handle (`handles` is `[count, h1, ...]`); a returned value or a thrown value gets its own handle.
///
/// # Safety
/// `handles` is a live native handle array (`[u64 count][u64; count]`).
#[no_mangle]
pub unsafe extern "C" fn thaw_tjs_call_handle_mixed_exact_consuming_result(
    handle: u64,
    args_json: *const c_char,
    handles: *const u8,
) -> ExactResult {
    unsafe { exact_call(handle, args_json, handles, false) }
}

/// Like [`thaw_tjs_call_handle_mixed_exact_consuming_result`], but a returned Promise is awaited (the
/// event loop runs until it settles): the result is its value, a rejection is the exception.
///
/// # Safety
/// `handles` is a live native handle array (`[u64 count][u64; count]`).
#[no_mangle]
pub unsafe extern "C" fn thaw_tjs_call_handle_mixed_exact_resolved_consuming_result(
    handle: u64,
    args_json: *const c_char,
    handles: *const u8,
) -> ExactResult {
    unsafe { exact_call(handle, args_json, handles, true) }
}

unsafe fn exact_call(
    handle: u64,
    args_json: *const c_char,
    handles: *const u8,
    resolve: bool,
) -> ExactResult {
    let none = ExactResult {
        value: 0,
        exception_handle: 0,
        exception_object_like: 0,
        error: ptr::null_mut(),
    };
    let direct: Option<Vec<u64>> = (!handles.is_null()).then(|| {
        let count = unsafe { *(handles.cast::<u64>()) } as usize;
        unsafe { std::slice::from_raw_parts(handles.add(8).cast::<u64>(), count) }.to_vec()
    });
    let outcome: Result<Result<i64, (i64, bool)>, String> = (|| {
        let direct = direct.as_ref().ok_or("null JsValue array")?;
        let args_json = cstr(args_json).ok_or("invalid arguments")?;
        let (_, args) = decode_wire(&args_json)?;
        let mut values = Vec::new();
        for h in direct {
            values.push(on_handle(*h, |_, _, v| Ok(v))?);
        }
        on_handle(handle, |p, owner, f| {
            let it = &mut p.interp;
            check_handle_callable(it, &f)?;
            let mut args = it.array_items(&args);
            args.extend(values);
            let called = it.call(&f, &Value::Undefined, &args);
            let called = match called {
                Ok(v) if resolve => match it.await_settled(&v).map_err(|c| classify(it, c))? {
                    Some(Ok(v)) => Ok(v),
                    Some(Err(e)) => Err(Ctl::Throw(e)),
                    None => return Err(HostError::Thrown("promise never settled".into())),
                },
                other => other,
            };
            Ok(match called {
                Ok(v) => Ok(store_handle(owner, v)),
                Err(Ctl::Throw(t)) => {
                    let like = matches!(t, Value::Object(_));
                    Err((store_handle(owner, t), like))
                }
                Err(c) => return Err(classify(it, c)),
            })
        })
    })();
    for h in direct.into_iter().flatten() {
        thaw_tjs_release_handle(h as i64);
    }
    thaw_tjs_release_handle(handle as i64);
    match outcome {
        Ok(Ok(value)) => ExactResult {
            value: value as u64,
            ..none
        },
        Ok(Err((thrown, like))) => ExactResult {
            exception_handle: thrown as u64,
            exception_object_like: u8::from(like),
            ..none
        },
        Err(e) => ExactResult {
            error: thaw_arena::owned_string(e),
            ..none
        },
    }
}

/// `[u64 count][u64; count]` native handle array -> ids (empty for null).
///
/// # Safety
/// `handles` is null or a live native handle array.
unsafe fn direct_handle_ids(handles: *const u8) -> Vec<u64> {
    if handles.is_null() {
        return Vec::new();
    }
    let count = unsafe { *(handles.cast::<u64>()) } as usize;
    unsafe { std::slice::from_raw_parts(handles.add(8).cast::<u64>(), count) }.to_vec()
}

/// `handle(...args, ...direct handles)` with an `undefined` receiver; the result is kept as a new
/// handle and a returned Promise is not awaited. `consuming` releases the callee and every direct handle.
unsafe fn call_mixed_handle(
    handle: u64,
    args_json: *const c_char,
    handles: *const u8,
    consuming: bool,
) -> HandleResult {
    let direct = unsafe { direct_handle_ids(handles) };
    let result = handle_result((|| {
        let args_json = cstr(args_json).ok_or("invalid arguments")?;
        let (_, args) = decode_wire(&args_json)?;
        let mut values = Vec::new();
        for h in &direct {
            values.push(on_handle(*h, |_, _, v| Ok(v))?);
        }
        on_handle(handle, |p, owner, f| {
            let it = &mut p.interp;
            check_handle_callable(it, &f)?;
            let mut args = it.array_items(&args);
            args.extend(values);
            let value = it
                .call(&f, &Value::Undefined, &args)
                .map_err(|c| classify(it, c))?;
            Ok(store_handle(owner, value))
        })
    })());
    if consuming {
        for h in direct {
            thaw_tjs_release_handle(h as i64);
        }
        thaw_tjs_release_handle(handle as i64);
    }
    result
}

/// # Safety
/// `handles` is null or a live native handle array.
#[no_mangle]
pub unsafe extern "C" fn thaw_tjs_call_handle_mixed_handle_graph_args_result(
    handle: u64,
    args_json: *const c_char,
    handles: *const u8,
) -> HandleResult {
    unsafe { call_mixed_handle(handle, args_json, handles, false) }
}

/// # Safety
/// `handles` is null or a live native handle array.
#[no_mangle]
pub unsafe extern "C" fn thaw_tjs_call_handle_mixed_handle_graph_args_consuming_result(
    handle: u64,
    args_json: *const c_char,
    handles: *const u8,
) -> HandleResult {
    unsafe { call_mixed_handle(handle, args_json, handles, true) }
}

/// `handle[name](...args)`; the result is kept as a new handle (a Promise is awaited unless
/// `chain_intermediate`).
#[no_mangle]
pub extern "C" fn thaw_tjs_call_method_handle_graph_args_result(
    handle: u64,
    name: *const c_char,
    args_json: *const c_char,
    chain_intermediate: bool,
) -> HandleResult {
    handle_result((|| {
        let name = cstr(name).ok_or("invalid method name")?;
        let (_, args) = decode_wire(&cstr(args_json).ok_or("invalid arguments")?)?;
        on_handle(handle, |p, owner, this| {
            let it = &mut p.interp;
            let method = it.get(&this, &name).map_err(|c| classify(it, c))?;
            let args = it.array_items(&args);
            let method = native_server_overload(it, &this, &name, &args).unwrap_or(method);
            let result = it
                .call(&method, &this, &args)
                .map_err(|c| classify(it, c))?;
            let result = if chain_intermediate {
                result
            } else {
                awaited(it, result)?
            };
            Ok(store_handle(owner, result))
        })
    })())
}

/// `handle[name] = args[0]` (`args_json` a graph wire array); the result is the assigned value as a graph wire.
#[no_mangle]
pub extern "C" fn thaw_tjs_set_property_json_graph_result(
    handle: u64,
    name: *const c_char,
    args_json: *const c_char,
) -> ThawResult {
    graph_result((|| {
        let name = cstr(name).ok_or("invalid property name")?;
        let (_, args) = decode_wire(&cstr(args_json).ok_or("invalid arguments")?)?;
        on_handle(handle, |p, owner, target| {
            let it = &mut p.interp;
            let value = it
                .array_items(&args)
                .into_iter()
                .next()
                .unwrap_or(Value::Undefined);
            strict_host_set(it, &target, &name, value.clone()).map_err(|c| classify(it, c))?;
            let mut out = WireOut {
                owner,
                plain: false,
                live_functions: false,
                copy_root: false,
                receiver: None,
                nodes: Vec::new(),
                seen: Vec::new(),
                leases: Vec::new(),
            };
            let root = out.token(it, &value)?;
            Ok(out.finish(root))
        })
    })())
}

/// `GetIterator` / `GetAsyncIterator` as JS (the QuickJS bootstrap helpers `__thaw_to_iterator` /
/// `__thaw_to_async_iterator`): a sync result is wrapped so `next`/`return`/`throw` are plain calls.
const ITERATOR_HELPERS: &str = r#"({
  sync: (value) => {
    if (value == null) return value;
    const method = value[Symbol.iterator];
    const iterator = typeof method === 'function' ? method.call(value) : value;
    if (iterator == null) return iterator;
    const next = iterator.next;
    if (typeof next !== 'function') return iterator;
    return {
      next: () => next.call(iterator),
      return: () => { const close = iterator.return; return typeof close === 'function' ? close.call(iterator) : { value: undefined, done: true }; },
      throw: (error) => { const resume = iterator.throw; return typeof resume === 'function' ? resume.call(iterator, error) : { value: undefined, done: true }; },
    };
  },
  async: (value) => {
    if (value == null) throw new TypeError('value is not async iterable');
    const asyncMethod = value[Symbol.asyncIterator];
    const object = (x) => x !== null && (typeof x === 'object' || typeof x === 'function');
    if (asyncMethod != null) {
      if (typeof asyncMethod !== 'function') throw new TypeError('value is not async iterable');
      const iterator = asyncMethod.call(value);
      if (!object(iterator)) throw new TypeError('Result of the Symbol.asyncIterator method is not an object');
      return iterator;
    }
    const method = value[Symbol.iterator];
    if (typeof method !== 'function') throw new TypeError('value is not async iterable');
    const iterator = method.call(value);
    if (!object(iterator)) throw new TypeError('Result of the Symbol.iterator method is not an object');
    const settle = (result) => {
      if (!object(result)) throw new TypeError('Iterator result is not an object');
      const done = !!result.done;
      return Promise.resolve(result.value).then((settled) => ({ done, value: settled }));
    };
    return {
      next: () => { const next = iterator.next; if (typeof next !== 'function') throw new TypeError('Iterator method is not callable'); return settle(next.call(iterator)); },
      return: () => { const close = iterator.return; return typeof close === 'function' ? settle(close.call(iterator)) : { value: undefined, done: true }; },
      throw: (error) => { const resume = iterator.throw; return typeof resume === 'function' ? settle(resume.call(iterator, error)) : { value: undefined, done: true }; },
    };
  },
})"#;

fn to_iterator_exact(source: u64, asynchronous: bool) -> ExactResult {
    let none = ExactResult {
        value: 0,
        exception_handle: 0,
        exception_object_like: 0,
        error: ptr::null_mut(),
    };
    let outcome: Result<Result<i64, (i64, bool)>, String> = on_handle(source, |p, owner, value| {
        let it = &mut p.interp;
        let step = (|it: &mut Interp| -> R<Value> {
            let helpers = it.run_sloppy(ITERATOR_HELPERS)?;
            let helper = it.get(&helpers, if asynchronous { "async" } else { "sync" })?;
            it.call(&helper, &Value::Undefined, &[value])
        })(it);
        Ok(match step {
            Ok(v) => Ok(store_handle(owner, v)),
            Err(Ctl::Throw(t)) => {
                let like = matches!(t, Value::Object(_));
                Err((store_handle(owner, t), like))
            }
            Err(c) => return Err(classify(it, c)),
        })
    });
    match outcome {
        Ok(Ok(value)) => ExactResult {
            value: value as u64,
            ..none
        },
        Ok(Err((thrown, like))) => ExactResult {
            exception_handle: thrown as u64,
            exception_object_like: u8::from(like),
            ..none
        },
        Err(e) => ExactResult {
            error: thaw_arena::owned_string(e),
            ..none
        },
    }
}

/// `for...of` iterator acquisition on the value behind `source`.
#[no_mangle]
pub extern "C" fn thaw_tjs_to_iterator_exact_result(source: u64) -> ExactResult {
    to_iterator_exact(source, false)
}

/// `for await...of` acquisition: `Symbol.asyncIterator`, then the sync iterator.
#[no_mangle]
pub extern "C" fn thaw_tjs_to_async_iterator_exact_result(source: u64) -> ExactResult {
    to_iterator_exact(source, true)
}

/// One iterator step (`mode` 0 next, 1 return, 2 throw(`thrown_text`); +3 captures the completion value;
/// +6 awaits the step: an async iterator). The `{done, value}` record is kept as a handle; a thrown
/// value becomes the exception handle.
#[no_mangle]
pub extern "C" fn thaw_tjs_iterator_step_exact_result(
    iterator: u64,
    mode: u8,
    thrown_text: *const c_char,
) -> ExactResult {
    let none = ExactResult {
        value: 0,
        exception_handle: 0,
        exception_object_like: 0,
        error: ptr::null_mut(),
    };
    if std::env::var_os("THAW_JS_TRACE_WIRE").is_some() {
        eprintln!("thaw-js iterator_step({iterator}, {mode})");
    }
    let outcome: Result<Result<i64, (i64, bool)>, String> = (|| {
        if mode > 11 {
            return Err("Invalid iterator step mode".to_string());
        }
        let (asynchronous, mode) = (mode >= 6, if mode >= 6 { mode - 6 } else { mode });
        let (capture, mode) = (mode >= 3, if mode >= 3 { mode - 3 } else { mode });
        let error = cstr(thrown_text);
        on_handle(iterator, |p, owner, iter| {
            let it = &mut p.interp;
            let step = (|it: &mut Interp| -> R<Value> {
                let key = ["next", "return", "throw"][mode as usize];
                let method = it.get(&iter, key)?;
                if mode == 1 && matches!(method, Value::Undefined | Value::Null) {
                    let o = Value::Object(it.plain());
                    it.set(&o, "done", Value::Bool(true))?;
                    if capture {
                        it.set(&o, "value", Value::Undefined)?;
                    }
                    return Ok(o);
                }
                if !method.is_callable() {
                    return it.throw_error("TypeError", "Iterator method is not callable");
                }
                let args = if mode == 2 {
                    vec![Value::str(error.as_deref().unwrap_or(""))]
                } else {
                    vec![]
                };
                let mut step = it.call(&method, &iter, &args)?;
                if asynchronous {
                    step = match it.await_settled(&step)? {
                        Some(Ok(v)) => v,
                        Some(Err(e)) => return Err(Ctl::Throw(e)),
                        None => return Err(Ctl::Throw(Value::str("promise never settled"))),
                    };
                }
                if !matches!(step, Value::Object(_)) {
                    return it.throw_error("TypeError", "Iterator result is not an object");
                }
                let out = Value::Object(it.plain());
                if mode == 1 && !capture {
                    it.set(&out, "done", Value::Bool(true))?;
                    return Ok(out);
                }
                let done = it.get(&step, "done")?;
                let done = done.truthy();
                it.set(&out, "done", Value::Bool(done))?;
                if !done || capture {
                    let value = it.get(&step, "value")?;
                    it.set(&out, "value", value)?;
                }
                Ok(out)
            })(it);
            Ok(match step {
                Ok(v) => Ok(store_handle(owner, v)),
                Err(Ctl::Throw(t)) => {
                    let like = matches!(t, Value::Object(_));
                    Err((store_handle(owner, t), like))
                }
                Err(c) => return Err(classify(it, c)),
            })
        })
    })();
    match outcome {
        Ok(Ok(value)) => ExactResult {
            value: value as u64,
            ..none
        },
        Ok(Err((thrown, like))) => ExactResult {
            exception_handle: thrown as u64,
            exception_object_like: u8::from(like),
            ..none
        },
        Err(e) => ExactResult {
            error: thaw_arena::owned_string(e),
            ..none
        },
    }
}

/// The value behind `handle` as a plain graph wire (a JSON-like copy).
#[no_mangle]
pub extern "C" fn thaw_tjs_resolve_handle_graph_result(handle: u64) -> ThawResult {
    graph_result(on_handle(handle, |p, owner, v| {
        // a Promise is awaited first, like QuickJS's resolve
        let v = adopt_thenable(&mut p.interp, v)?;
        let v = match p
            .interp
            .await_settled(&v)
            .map_err(|c| classify(&mut p.interp, c))?
        {
            Some(Ok(v)) => v,
            Some(Err(e)) => return Err(classify(&mut p.interp, Ctl::Throw(e))),
            None => return Err(HostError::Thrown("promise never settled".into())),
        };
        let mut out = WireOut {
            owner,
            plain: true,
            live_functions: true,
            copy_root: false,
            receiver: None,
            nodes: Vec::new(),
            seen: Vec::new(),
            leases: Vec::new(),
        };
        let root = out.token(&mut p.interp, &v)?;
        Ok(out.finish(root))
    }))
}

/// A function or class instance: it cannot cross as plain data, so compiled code keeps it by handle.
fn is_live_value(it: &Interp, v: &Value) -> bool {
    match v {
        Value::Object(o) => {
            v.is_callable()
                || (matches!(&*o.kind.borrow(), Kind::Plain)
                    && o.proto
                        .borrow()
                        .as_ref()
                        .is_some_and(|p| !Rc::ptr_eq(p, &it.object_proto)))
        }
        _ => false,
    }
}

/// A `TypeError` for a call of something that is not a function, classified like any thrown error.
fn not_callable(it: &mut Interp, message: String) -> HostError {
    let Err(c) = it.throw_error::<()>("TypeError", &message) else {
        unreachable!("throw_error always returns Err")
    };
    classify(it, c)
}

/// A host-originated call of a handle: a value that is not a function fails naming its kind.
pub(crate) fn check_handle_callable(it: &mut Interp, f: &Value) -> Result<(), HostError> {
    if f.is_callable() {
        return Ok(());
    }
    let message = format!("handle call: {}", it.not_callable_message(f));
    Err(not_callable(it, message))
}

/// `await v` on a live value: a non-Promise object with a callable `then` is adopted like
/// `Promise.resolve(v)` (so the thenable's `then` runs); everything else is unchanged.
fn adopt_thenable(it: &mut Interp, v: Value) -> Result<Value, HostError> {
    if it.is_promise(&v) || !matches!(v, Value::Object(_)) {
        return Ok(v);
    }
    let then = it.get(&v, "then").map_err(|c| classify(it, c))?;
    if !then.is_callable() {
        return Ok(v);
    }
    let promise = it.new_promise();
    it.resolve_promise(&promise, v)
        .map_err(|c| classify(it, c))?;
    Ok(Value::Object(promise))
}

/// A call result with a returned Promise awaited (the event loop runs until it settles); a rejection throws.
pub(crate) fn awaited(it: &mut Interp, v: Value) -> Result<Value, HostError> {
    match it.await_settled(&v).map_err(|c| classify(it, c))? {
        Some(Ok(v)) => Ok(v),
        Some(Err(e)) => Err(classify(it, Ctl::Throw(e))),
        None => Err(HostError::Thrown("promise never settled".into())),
    }
}

/// `awaited` for a `\u{4}` call: a promise still pending when the loop runs out of work is returned as it is.
fn awaited_or_pending(it: &mut Interp, v: Value) -> Result<Value, HostError> {
    match it.await_settled(&v).map_err(|c| classify(it, c))? {
        Some(Ok(v)) => Ok(v),
        Some(Err(e)) => Err(classify(it, Ctl::Throw(e))),
        None => Ok(v),
    }
}

fn handle_marker_json(id: i64) -> String {
    format!("{{\"{HANDLE_KEY}\":{id}}}")
}

fn handle_text(handle: u64, bigint_suffix: bool) -> *const c_char {
    let text = on_handle(handle, |p, _, v| {
        let it = &mut p.interp;
        if bigint_suffix {
            return Ok(console_text(it, &v));
        }
        Ok(it.to_str(&v).to_string())
    })
    .unwrap_or_else(|_| "[invalid JsValue]".to_string());
    thaw_arena::owned_string(text)
}

fn is_function_value(v: &Value) -> bool {
    matches!(v, Value::Object(o) if matches!(&*o.kind.borrow(), Kind::Closure { .. } | Kind::Native(_)))
}

/// `console.log` text of `v`: a bigint with its `n`, a tagged plain object as `Object [Tag] {}`, a
/// function or class as Node prints it, and an object's `Symbol.for('nodejs.util.inspect.custom')`
/// result when it is a string. A failing getter or custom inspector falls back to `String(v)`.
fn console_text(it: &mut Interp, v: &Value) -> String {
    if let Value::Object(o) = v {
        if it.tagged_plain(o).is_some() {
            return it.inspect(v, false);
        }
        if let Some(text) = console_function_text(it, v).ok().flatten() {
            return text;
        }
        if let Some(text) = console_custom_text(it, v).ok().flatten() {
            return text;
        }
    }
    match v {
        Value::BigInt(_) => format!("{}n", it.to_str(v)),
        _ => it.to_str(v).to_string(),
    }
}

/// `[class NAME extends BASE] { k: v }`, `[Function: NAME]` or `[Function (anonymous)]`; `None` when
/// `v` is not callable.
fn console_function_text(it: &mut Interp, v: &Value) -> R<Option<String>> {
    let is_class = match v {
        Value::Object(o) => match &*o.kind.borrow() {
            Kind::Closure { def, .. } => def.class.is_some(),
            Kind::Native(_) => false,
            _ => return Ok(None),
        },
        _ => return Ok(None),
    };
    let name = match it.get(v, "name")? {
        Value::Str(s) => s.to_string(),
        _ => String::new(),
    };
    if !is_class {
        return Ok(Some(if name.is_empty() {
            "[Function (anonymous)]".to_string()
        } else {
            format!("[Function: {name}]")
        }));
    }
    let mut text = format!(
        "[class {}",
        if name.is_empty() {
            "(anonymous)"
        } else {
            &name
        }
    );
    if let Value::Object(o) = v {
        let base = o.proto.borrow().clone();
        if let Some(base) = base.filter(|b| !Rc::ptr_eq(b, &it.function_proto)) {
            if let Value::Str(base_name) = it.get(&Value::Object(base), "name")? {
                if !base_name.is_empty() {
                    text.push_str(&format!(" extends {base_name}"));
                }
            }
        }
    }
    text.push(']');
    let keys: Vec<Rc<str>> = it
        .own_keys(v)
        .into_iter()
        .filter(|k| !k.starts_with("__thaw"))
        .collect();
    if !keys.is_empty() {
        let mut parts = Vec::with_capacity(keys.len());
        for key in keys {
            let val = it.get(v, &key)?;
            let shown = match &val {
                Value::Str(s) => format!("'{s}'"),
                f if is_function_value(f) => {
                    let fname = it.get(f, "name")?;
                    format!("[Function: {}]", it.to_str(&fname))
                }
                other => it.to_str(other).to_string(),
            };
            parts.push(format!("{key}: {shown}"));
        }
        text.push_str(&format!(" {{ {} }}", parts.join(", ")));
    }
    Ok(Some(text))
}

/// The string an object's `nodejs.util.inspect.custom` method returns; `None` otherwise.
fn console_custom_text(it: &mut Interp, v: &Value) -> R<Option<String>> {
    let sym = it.symbol_for("nodejs.util.inspect.custom".into())?;
    let key = it.key_of(&sym);
    let custom = it.get(v, &key)?;
    if !is_function_value(&custom) {
        return Ok(None);
    }
    let opts = Value::Object(it.plain());
    it.set(&opts, "breakLength", Value::Number(80.0))?;
    it.set(&opts, "depth", Value::Number(2.0))?;
    let result = it.call(&custom, v, &[Value::Number(2.0), opts, Value::Undefined])?;
    Ok(match result {
        Value::Str(s) => Some(s.to_string()),
        _ => None,
    })
}

/// `String(value)` of the value behind `handle`.
#[no_mangle]
pub extern "C" fn thaw_tjs_handle_to_string(handle: u64) -> *const c_char {
    handle_text(handle, false)
}

/// `String(value)` as `console.log` prints it (a bigint with its `n`).
#[no_mangle]
pub extern "C" fn thaw_tjs_handle_to_console_string(handle: u64) -> *const c_char {
    handle_text(handle, true)
}

/// Whether every element of array `o` is a primitive, a plain object, or a plain array. A nested
/// array already on `ancestors` (a cycle) counts as plain, keeping the old Json path for it.
fn array_elements_are_plain(it: &mut Interp, o: &Obj, ancestors: &mut Vec<usize>) -> bool {
    let ptr = Rc::as_ptr(o) as usize;
    if ancestors.contains(&ptr) {
        return true;
    }
    let items = match &*o.kind.borrow() {
        Kind::Array(items) => items.clone(),
        _ => return false,
    };
    ancestors.push(ptr);
    let plain = items.iter().all(|x| match x {
        Value::Object(e) if matches!(&*e.kind.borrow(), Kind::Array(_)) => {
            array_elements_are_plain(it, e, ancestors)
        }
        Value::Object(e) => {
            matches!(&*e.kind.borrow(), Kind::Plain)
                && e.proto
                    .borrow()
                    .as_ref()
                    .is_none_or(|q| Rc::ptr_eq(q, &it.object_proto))
                && it.tagged_plain(e).is_none()
        }
        _ => true,
    });
    ancestors.pop();
    plain
}

/// 1 for a plain array or plain object (`console.log` inspects those like a native `Json`).
#[no_mangle]
pub extern "C" fn thaw_tjs_handle_is_plain_data(handle: u64) -> u8 {
    on_handle(handle, |p, _, v| {
        Ok(match &v {
            // an array is data only if no element is a non-plain object (it would print as a handle)
            Value::Object(o) if matches!(&*o.kind.borrow(), Kind::Array(_)) => {
                u8::from(array_elements_are_plain(&mut p.interp, o, &mut Vec::new()))
            }
            Value::Object(o) if matches!(&*o.kind.borrow(), Kind::Plain) => u8::from(
                o.proto
                    .borrow()
                    .as_ref()
                    .is_none_or(|q| Rc::ptr_eq(q, &p.interp.object_proto))
                    // a tagged object (`Math`) is inspected by the interpreter, not as data
                    && p.interp.tagged_plain(o).is_none(),
            ),
            _ => 0,
        })
    })
    .unwrap_or(0)
}

/// `console.log` of a typed array: `{"n":name,"v":[text, ..],"e":[]}` for the std inspector
/// (`thaw_console_typed_or`), or null for any other value (and for a `Buffer`, which prints itself).
#[no_mangle]
pub extern "C" fn thaw_tjs_handle_typed_array_probe(handle: u64) -> *const c_char {
    let probe = on_handle(handle, |_, _, v| {
        let Value::Object(o) = &v else {
            return Ok(None);
        };
        let Kind::Typed(t) = &*o.kind.borrow() else {
            return Ok(None);
        };
        if crate::find_in_chain(o, "__isBuffer").is_some() {
            return Ok(None);
        }
        // BigInt elements print with the `n` suffix, as Node's inspect does
        let items: Vec<String> = t
            .to_values()
            .iter()
            .map(|v| {
                let text = match v {
                    Value::BigInt(b) => format!("{b}n"),
                    Value::Number(x) if *x == 0.0 && x.is_sign_negative() => "-0".to_string(),
                    Value::Number(x) => crate::number_to_string(*x),
                    _ => String::new(),
                };
                format!("\"{text}\"")
            })
            .collect();
        // ponytail: no extra own-property list ("e"); add if a package prints a typed array with extra keys
        Ok(Some(format!(
            "{{\"n\":\"{}\",\"v\":[{}],\"e\":[]}}",
            t.ty.name(),
            items.join(",")
        )))
    });
    match probe {
        Ok(Some(text)) => thaw_arena::owned_string(text),
        _ => ptr::null(),
    }
}

/// Register a compiled closure callable from JS; returns its id. The same closure registered twice
/// keeps one id, so the function identity JS sees is stable.
#[no_mangle]
pub extern "C" fn thaw_tjs_register_callback(
    adapter: *const c_void,
    context: *const c_void,
    params: i64,
) -> i64 {
    let identity = PENDING_IDENTITY.with(|p| p.replace(0));
    register_callback(
        adapter,
        context,
        identity,
        params as usize,
        false,
        0,
        false,
        0,
        false,
    )
}

thread_local! {
    /// Identity announced by [`thaw_tjs_callback_identity`] for the next `thaw_tjs_register_callback`.
    static PENDING_IDENTITY: Cell<usize> = const { Cell::new(0) };
}

/// Announce the identity (the compiler's alias-resolved closure pointer) of the closure the next
/// `thaw_tjs_register_callback` registers: registering the same closure again returns the same id.
#[no_mangle]
pub extern "C" fn thaw_tjs_callback_identity(identity: *const c_void) {
    PENDING_IDENTITY.with(|p| p.set(identity as usize));
}

#[allow(clippy::too_many_arguments)]
fn register_callback(
    adapter: *const c_void,
    context: *const c_void,
    identity: usize,
    params: usize,
    graph: bool,
    finish: usize,
    has_rest: bool,
    mask: u64,
    method: bool,
) -> i64 {
    let mut callbacks = CALLBACKS.lock().unwrap();
    let (adapter, context) = (adapter as usize, context as usize);
    if let Some(id) = callbacks.iter().position(|c| {
        (if identity != 0 && c.identity != 0 {
            c.identity == identity
        } else {
            c.adapter == adapter && c.context == context
        }) && c.params == params
            && c.graph == graph
            && c.finish == finish
            && c.has_rest == has_rest
            && c.mask == mask
            && c.method == method
    }) {
        return id as i64;
    }
    callbacks.push(CompiledCallback {
        adapter,
        context,
        identity,
        params,
        graph,
        finish,
        has_rest,
        mask,
        method,
        _root: thaw_arena::ArenaRoot::new(context),
    });
    callbacks.len() as i64 - 1
}

/// Register a compiled graph-wire callback (the counterpart of QuickJS's
/// `thaw_js_register_native_callback_graph`; the void flag is not needed) and return it as a JS function
/// kept by handle. `jsvalue_param_mask` says which arguments cross as handles (see `CompiledCallback`).
#[no_mangle]
pub extern "C" fn thaw_tjs_register_native_callback_graph(
    adapter: *const c_void,
    context: *const c_void,
    jsvalue_param_mask: u64,
    param_count: u64,
    _void_result: u8,
    finish: *const c_void,
    has_rest: u8,
) -> HandleResult {
    register_graph_callback(
        adapter,
        context,
        jsvalue_param_mask,
        param_count,
        finish,
        has_rest,
        false,
    )
}

fn register_graph_callback(
    adapter: *const c_void,
    context: *const c_void,
    jsvalue_param_mask: u64,
    param_count: u64,
    finish: *const c_void,
    has_rest: u8,
    method: bool,
) -> HandleResult {
    // The compiler announces the closure identity right before this call (see `thaw_tjs_callback_identity`).
    let identity = PENDING_IDENTITY.with(|p| p.replace(0));
    if std::env::var_os("THAW_JS_TRACE_WIRE").is_some() {
        eprintln!(
            "thaw-js register: params {param_count} mask {jsvalue_param_mask:#x} method {method}"
        );
    }
    let id = register_callback(
        adapter,
        context,
        identity,
        param_count as usize,
        true,
        finish as usize,
        has_rest != 0,
        jsvalue_param_mask,
        method,
    );
    let owner = default_realm();
    handle_result(
        with_package(owner, |p| Ok(callback_value(&mut p.interp, id as usize)))
            .map(|f| store_handle(owner, f))
            .map_err(|e| e.to_string()),
    )
}

/// As [`thaw_tjs_register_native_callback_graph`], for an object-literal method (the receiver already
/// crosses as the leading `this` of every graph callback).
#[no_mangle]
pub extern "C" fn thaw_tjs_register_native_method_callback_graph(
    adapter: *const c_void,
    context: *const c_void,
    jsvalue_param_mask: u64,
    param_count: u64,
    _void_result: u8,
    finish: *const c_void,
    has_rest: u8,
) -> HandleResult {
    register_graph_callback(
        adapter,
        context,
        jsvalue_param_mask,
        param_count,
        finish,
        has_rest,
        true,
    )
}

fn callback_value(it: &mut Interp, id: usize) -> Value {
    if let Some(v) = it.callback_values.get(&id) {
        return v.clone();
    }
    let function = it.native(move |it, this, args| call_compiled(it, id, this, args));
    // the declared arity (a rest parameter does not count), as a JS function's `length`
    let arity = CALLBACKS.lock().unwrap().get(id).map(|c| {
        if c.has_rest {
            c.params.saturating_sub(1)
        } else {
            c.params
        }
    });
    if let (Value::Object(o), Some(n)) = (&function, arity) {
        crate::put_hidden(o, "length", Value::Number(n as f64));
    }
    it.callback_values.insert(id, function.clone());
    function
}

/// Rebuild a thrown error from the compiled side's `\u{1}Name\u{1}message` framing.
pub(crate) fn tagged_error(it: &mut Interp, tagged: &str) -> Ctl {
    match error_object(it, tagged) {
        Ok(err) => Ctl::Throw(err),
        Err(c) => c,
    }
}

/// Reserved bag key of a native Error's identity (`thaw_error_json_text`); kept as a hidden property.
const NATIVE_ID_KEY: &str = "\u{0}id";

/// The error object for a framed error string (`\u{1}Name\u{1}message`, or the tagged `\u{1}Name\u{1}\u{1e}E1:..`
/// form), with its `\u{3}code`, `\u{5}<json>` own properties and `\u{2}` cause; `Err` is what the construction
/// raised. One object per native id: a repeat crossing of the same native Error returns the cached object.
fn error_object(it: &mut Interp, tagged: &str) -> Result<Value, Ctl> {
    let tagged = tagged.strip_prefix('\u{2}').unwrap_or(tagged);
    // a native error token (`\u{1}Name\u{1}\u{1e}E1:<units>:-:<message>`): the name, the message and the suffix it frames
    let framed = thaw_arena::error_wire::parse_tagged(tagged.as_bytes()).and_then(|f| {
        Some((
            std::str::from_utf8(f.chain).ok()?,
            std::str::from_utf8(f.display).ok()?,
            std::str::from_utf8(f.suffix).ok()?,
        ))
    });
    let (name, message, suffix) = match framed.or_else(|| {
        let (name, rest) = tagged.strip_prefix('\u{1}')?.split_once('\u{1}')?;
        // the message runs up to the first marker (`\u{2}`..=`\u{5}`); the rest is the suffix
        let end = rest
            .find(|c| ('\u{2}'..='\u{5}').contains(&c))
            .unwrap_or(rest.len());
        Some((name, &rest[..end], &rest[end..]))
    }) {
        Some(parts) => parts,
        None => ("Error", tagged, ""),
    };
    // the `\u{5}<json>` bag of the error's own properties (`isAxiosError`, `response`, ...); its key order is the error's own order
    let fields = match suffix_segment(suffix, '\u{5}').map(crate::json::parse_native) {
        Some(Ok(Native::Object(fields))) => fields,
        _ => Vec::new(),
    };
    let native_id = fields.iter().find_map(|(k, v)| match v {
        Native::Number(n) if k == NATIVE_ID_KEY && *n >= 1.0 => Some(*n as u64),
        _ => None,
    });
    // a cached error keeps its identity only while its name and message still match (a reused arena id
    // of another Error is not this one)
    let cached = native_id.and_then(|id| {
        it.native_errors
            .get(&id)
            .filter(|(n, m, _)| n.as_str() == name && m.as_str() == message)
            .map(|(_, _, err)| err.clone())
    });
    let err = match cached {
        Some(err) => err,
        None => {
            let global = it.global.clone();
            let ctor = it
                .lookup(&global, name)
                .filter(|c| c.is_callable())
                .or_else(|| it.lookup(&global, "Error"))
                .unwrap_or(Value::Undefined);
            let err = it.construct(&ctor, &[Value::str(message)])?;
            if let (Some(id), Value::Object(o)) = (native_id, &err) {
                crate::put_hidden(o, NATIVE_ID_KEY, Value::Number(id as f64));
                it.native_errors
                    .insert(id, (name.to_string(), message.to_string(), err.clone()));
            }
            // the native `\u{2}<cause>` segment (up to the code marker) is `.cause`: a tagged text is an Error
            if let (Value::Object(o), Some(cause)) = (&err, suffix_segment(suffix, '\u{2}')) {
                if !cause.is_empty() {
                    let cause = if cause.starts_with('\u{1}') {
                        framed_error(it, cause).unwrap_or_else(|| Value::str(cause))
                    } else {
                        Value::str(cause)
                    };
                    crate::put_hidden(o, "cause", cause);
                }
            }
            err
        }
    };
    let mut has_code = false;
    for (k, v) in fields {
        if k != "name" && k != "message" && k != "stack" && !k.starts_with('\u{0}') {
            has_code |= k == "code";
            let v = to_value(it, &v);
            if k == "\u{2}cause" {
                if let Value::Object(o) = &err {
                    crate::put_hidden(o, "cause", v);
                }
            } else {
                let _ = it.set(&err, &k, v);
            }
        }
    }
    // `code` only when the bag did not carry it (a frame without a bag): after the bag so bag order wins
    if !has_code {
        if let Some(code) = suffix_segment(suffix, '\u{3}') {
            let _ = it.set(&err, "code", Value::str(code));
        }
    }
    Ok(err)
}

/// The text after `marker` in a framed error suffix, up to the next marker (`\u{2}`..=`\u{6}`).
fn suffix_segment(suffix: &str, marker: char) -> Option<&str> {
    let (_, rest) = suffix.split_once(marker)?;
    Some(
        rest.split(|c| ('\u{2}'..='\u{6}').contains(&c))
            .next()
            .unwrap_or(""),
    )
}

/// A compiled `Error` that crossed as its framed string (the test of json.rs `is_error_frame_text`): the
/// Error object. Any other string, or a frame that does not parse, stays a string.
fn framed_error(it: &mut Interp, s: &str) -> Option<Value> {
    let bytes = s.as_bytes();
    if bytes.first() != Some(&1) || !bytes[1..].contains(&1) {
        return None;
    }
    error_object(it, s).ok()
}

fn throw_type_error<T>(it: &mut Interp, message: &str) -> R<T> {
    it.throw_error("TypeError", message)
}

/// The live handle id of a tracked native Json root (`__thaw_js_handle_id__`), or 0. Looked up by
/// symbol like `native_promise_state`; thaw-js takes no Cargo dependency on thaw-std.
#[cfg(unix)]
fn tracked_handle_id(object: usize) -> u64 {
    // SAFETY: the symbol, when present, is thaw-std's `thaw_json_borrowed_tracked_handle_id`.
    unsafe {
        let sym = libc::dlsym(
            libc::RTLD_DEFAULT,
            c"thaw_json_borrowed_tracked_handle_id".as_ptr(),
        );
        if sym.is_null() {
            return 0;
        }
        std::mem::transmute::<*mut c_void, unsafe extern "C" fn(*const c_void) -> u64>(sym)(
            object as *const c_void,
        )
    }
}

#[cfg(not(unix))]
fn tracked_handle_id(_object: usize) -> u64 {
    0
}

/// The live value behind a handle id (index + 1 into `HANDLES`); 0 or a released id gives None.
pub(crate) fn handle_value(id: u64) -> Option<Value> {
    if id == 0 {
        return None;
    }
    slot(id).map(|(_, v)| v)
}

/// A compiled throw of a live JS object rethrows that exact object: tag 7 with a non-zero `object`
/// that is a tracked native Json root. The guard does not require `pending == object` or
/// `native_text == 0`, because async-callback rejections publish a display text in `pending` and
/// `native_text` while the object stays in `object`; the tracked-root lookup is what authenticates
/// the object. An id of 0 (untracked, or not a live handle) keeps the framed-Error path. The
/// QuickJS side (`thaw-quickjs` `reconstitute_exception_packet`) is unchanged.
pub(crate) fn original_js_exception_with(
    packet: &thaw_arena::exception::ExceptionPacket,
    lookup: impl FnOnce(usize) -> u64,
) -> Option<Value> {
    let looked_up = if packet.tag == 7 && packet.object != 0 {
        lookup(packet.object)
    } else {
        0
    };
    // The throwing thread recorded the handle id in `bigint` (the tag-7 i64 slot). The tracked-root
    // lookup can miss on another thread, so fall back to it.
    let id = if looked_up != 0 {
        looked_up
    } else {
        packet.bigint
    };
    if std::env::var_os("THAW_JS_TRACE_WIRE").is_some() {
        eprintln!(
            "thaw-js exc packet: tag={:#x} object={:#x} pending={:#x} native={:#x} aggregate={:#x} native_text={:#x} lookup={:#x} bigint={:#x} thread={:?}",
            packet.tag, packet.object, packet.pending, packet.native, packet.aggregate, packet.native_text, id, packet.bigint, std::thread::current().id()
        );
    }
    if !(packet.tag == 7 && packet.object != 0 && packet.native == 0 && packet.aggregate == 0) {
        return None;
    }
    handle_value(id)
}

fn original_js_exception(packet: &thaw_arena::exception::ExceptionPacket) -> Option<Value> {
    original_js_exception_with(packet, tracked_handle_id)
}

/// The exact value of a thrown or rejected primitive (`throw 0`, `throw false`, `reject(undefined)`).
/// It has no object, native or aggregate identity, so its type is the packet's value tag. The tags
/// are the compiled runtime's (`hir_codegen/statements.rs` unions 1 number, 2 bigint, 3 bool, 5
/// undefined, 6 null) and match thaw-quickjs `reconstitute_exception_packet`. Tag 4 (text) is not
/// mapped here: a string throw keeps the existing Error path.
fn primitive_reason(packet: &thaw_arena::exception::ExceptionPacket) -> Option<Value> {
    if packet.object != 0 || packet.native != 0 || packet.aggregate != 0 {
        return None;
    }
    Some(match packet.tag {
        1 => Value::Number(f64::from_bits(packet.number)),
        2 => Value::BigInt(Rc::new(num_bigint::BigInt::from(packet.bigint as i64))),
        3 => Value::Bool(packet.boolean != 0),
        5 => Value::Undefined,
        6 => Value::Null,
        _ => return None,
    })
}

/// Unwrap a callback result string: a thrown error (arena packet or `\u{2}` framing) becomes a JS throw.
pub(crate) fn take_callback_result(
    it: &mut Interp,
    result: *mut c_char,
) -> Result<Option<String>, Ctl> {
    let packet = thaw_arena::take_owned_exception(result);
    let out = if result.is_null() {
        String::new()
    } else {
        let s = unsafe { CStr::from_ptr(result) }
            .to_string_lossy()
            .into_owned();
        unsafe { thaw_arena::destroy_string(result) };
        s
    };
    if std::env::var_os("THAW_JS_TRACE_WIRE").is_some() {
        eprintln!("thaw-js callback result: {out:?}");
    }
    if let Some(original) = packet.as_deref().and_then(original_js_exception) {
        return Err(Ctl::Throw(original));
    }
    if let Some(reason) = packet.as_deref().and_then(primitive_reason) {
        return Err(Ctl::Throw(reason));
    }
    if packet.is_some() || out.starts_with('\u{2}') {
        return Err(tagged_error(it, &out));
    }
    Ok((!out.is_empty()).then_some(out))
}

/// One argument of a graph callback's wire. A JsValue position (`jsvalue`) keeps the value behind a
/// handle lease, which the compiled side decodes; a `Str` position (`error`) holding an `Error` crosses
/// as its framed text, the way QuickJS's `__thaw_error_to_tagged` does.
fn callback_arg_token(
    it: &mut Interp,
    out: &mut WireOut,
    item: &Value,
    jsvalue: bool,
    error: bool,
) -> Result<String, HostError> {
    if error {
        if let Value::Object(o) = item {
            if crate::builtins::is_error_instance(it, o) {
                if let Some((_, framed)) = frame_error(it, item) {
                    let mut q = String::new();
                    crate::builtins::json_quote(&framed, &mut q);
                    return Ok(format!("{{\"v\":{q}}}"));
                }
            }
        }
    }
    if jsvalue {
        let id = store_handle(out.owner, item.clone()) as u64;
        return Ok(out.lease(id));
    }
    out.token(it, item)
}

/// A graph callback: `[this, ...args]` goes over as a graph wire (objects stay live behind handle
/// leases), the result is a graph wire (or a native Promise polled through `finish`).
#[allow(clippy::too_many_arguments)]
fn call_compiled_graph(
    it: &mut Interp,
    adapter: usize,
    context: usize,
    params: usize,
    finish: usize,
    has_rest: bool,
    mask: u64,
    method: bool,
    this: &Value,
    args: &[Value],
) -> R<Value> {
    let mut items = vec![this.clone()];
    if has_rest && params > 0 {
        items.extend(args.iter().take(params - 1).cloned());
        let rest = args.get(params - 1..).unwrap_or(&[]).to_vec();
        items.push(it.array(rest));
    } else {
        items.extend(args.iter().take(params).cloned());
    }
    // A masked declared position the caller did not pass (`cb(err)` for a `(err, data)` callback) crosses
    // as an `undefined` handle, not as the absent sentinel that decodes to handle 0.
    let (low, errors) = (mask & 0xffff_ffff, mask >> 32);
    if let Some(last) = (0..params.min(32))
        .filter(|p| (low >> p) & 1 == 1)
        .map(|p| if method { p } else { p + 1 })
        .max()
    {
        items.resize(items.len().max(last + 1), Value::Undefined);
    }
    let mut out = WireOut {
        owner: LAST_PACKAGE.with(|l| l.get()),
        plain: false,
        live_functions: false,
        copy_root: true,
        receiver: None,
        nodes: Vec::new(),
        seen: Vec::new(),
        leases: Vec::new(),
    };
    // The argument array is the wire's root (as a copied container, so only what it holds goes live).
    let root = out.nodes.len();
    out.nodes.push(String::new());
    let mut parts = Vec::with_capacity(items.len());
    for (j, item) in items.iter().enumerate() {
        // The declared parameter of wire item `j` (`this` is item 0 of a plain callback).
        let param = if method { Some(j) } else { j.checked_sub(1) }.filter(|p| *p < 32);
        let jsvalue = param.is_some_and(|p| (low >> p) & 1 == 1);
        let error = param.is_some_and(|p| (errors >> p) & 1 == 1);
        match callback_arg_token(it, &mut out, item, jsvalue, error) {
            Ok(part) => parts.push(part),
            Err(e) => {
                for id in &out.leases {
                    thaw_tjs_release_handle(*id as i64);
                }
                return throw_type_error(it, &e.to_string());
            }
        }
    }
    out.nodes[root] = format!("{{\"a\":[{}]}}", parts.join(","));
    let token = format!("{{\"r\":{root}}}");
    let leases = out.leases.clone();
    let wire = thaw_arena::owned_string(out.finish(token));
    if std::env::var_os("THAW_JS_TRACE_WIRE").is_some() {
        // SAFETY: `wire` is a fresh arena-owned string, read before it is handed to the adapter.
        let text = unsafe { std::ffi::CStr::from_ptr(wire) }.to_string_lossy();
        eprintln!("thaw-js callback wire: {text}");
    }
    // SAFETY: `wire` is a fresh arena-owned string.
    let granted = unsafe { thaw_arena::register_owned_graph_wire(wire) };
    // SAFETY: `adapter` is a generated `(context, wire) -> result` function and `context` its environment.
    let adapter: Adapter = unsafe { std::mem::transmute(adapter) };
    let result =
        thaw_arena::with_thread_stack_guard(|| unsafe { adapter(context as *const c_void, wire) });
    // An unconsumed grant means the adapter never decoded the wire: return its leases.
    if granted && unsafe { thaw_arena::take_owned_graph_wire_snapshot(wire) }.is_some() {
        for id in &leases {
            thaw_tjs_release_handle(*id as i64);
        }
    }
    unsafe { thaw_arena::destroy_string(wire) };
    if finish != 0 {
        return native_promise_value(it, result as usize, finish);
    }
    match take_callback_result(it, result)? {
        None => Ok(Value::Undefined),
        Some(text) => decode_callback_wire(it, &text),
    }
}

fn decode_callback_wire(it: &mut Interp, text: &str) -> R<Value> {
    match decode_wire(text) {
        Ok((_, v)) => Ok(v),
        Err(e) => throw_type_error(it, &e),
    }
}

/// A JS Promise that settles when the compiled side's native Promise `promise` does.
fn native_promise_value(it: &mut Interp, promise: usize, finish: usize) -> R<Value> {
    let p = it.new_promise();
    native_promise_mark_handled(promise);
    poll_native_promise(it, p.clone(), promise, finish)?;
    Ok(Value::Object(p))
}

fn poll_native_promise(it: &mut Interp, p: Obj, promise: usize, finish: usize) -> R<()> {
    let check = it.native(move |it, _, _| {
        let state = native_promise_state(promise);
        if state == 0 {
            return poll_native_promise(it, p.clone(), promise, finish).map(|()| Value::Undefined);
        }
        let finish_fn: Adapter = unsafe { std::mem::transmute(finish) };
        let result = thaw_arena::with_thread_stack_guard(|| unsafe {
            finish_fn(promise as *const c_void, ptr::null())
        });
        match take_callback_result(it, result) {
            Err(Ctl::Throw(e)) => it.reject_promise(&p, e),
            Err(other) => return Err(other),
            Ok(_) if state == 2 => {
                it.reject_promise(&p, Value::str("native Promise rejected"));
            }
            Ok(None) => it.resolve_promise(&p, Value::Undefined)?,
            Ok(Some(text)) => match decode_callback_wire(it, &text) {
                Ok(v) => it.resolve_promise(&p, v)?,
                Err(Ctl::Throw(e)) => it.reject_promise(&p, e),
                Err(other) => return Err(other),
            },
        }
        Ok(Value::Undefined)
    });
    it.defer_tick(check)
}

/// Test stand-in for the compiled runtime's `thaw_http_poll_servers` (process-wide, like the symbol).
#[cfg(test)]
pub(crate) static TEST_SERVER_POLL: std::sync::Mutex<Option<extern "C" fn() -> u8>> =
    std::sync::Mutex::new(None);

/// One drain of the compiled runtime's native continuations (`thaw_runtime_run_until_idle`).
/// True when a continuation ran. Missing symbol (no compiled runtime, e.g. tests): no progress.
#[cfg(unix)]
fn drain_native() -> bool {
    // SAFETY: the symbol is the compiled runtime's `extern "C" fn() -> usize`, looked up like the other natives.
    unsafe {
        let sym = libc::dlsym(libc::RTLD_DEFAULT, c"thaw_runtime_run_until_idle".as_ptr());
        if sym.is_null() {
            return false;
        }
        std::mem::transmute::<*mut c_void, extern "C" fn() -> usize>(sym)() != 0
    }
}

#[cfg(not(unix))]
fn drain_native() -> bool {
    false
}

thread_local! {
    /// Whether the last `thaw_http_poll_servers` pass saw a live server (bit 0). Host thread only.
    static NATIVE_SERVERS_LIVE: Cell<bool> = const { Cell::new(false) };
}

/// Whether a compiled http server is still live, as of the last poll on this thread.
pub(crate) fn native_servers_live() -> bool {
    NATIVE_SERVERS_LIVE.with(Cell::get)
}

/// `thaw_http_poll_servers` (compiled thaw-std): bit 0 = a server is live, bit 1 = a listening/error/close
/// callback ran. Looked up like `drain_native`; 0 without thaw-std. Any thread may call it (process-wide state).
fn poll_native_servers() -> u8 {
    #[cfg(test)]
    let test_poll = *TEST_SERVER_POLL.lock().unwrap();
    #[cfg(test)]
    let r = match test_poll {
        Some(poll) => poll(),
        None => poll_native_servers_symbol(),
    };
    #[cfg(not(test))]
    let r = poll_native_servers_symbol();
    NATIVE_SERVERS_LIVE.with(|live| live.set(r & 1 != 0));
    r
}

#[cfg(unix)]
fn poll_native_servers_symbol() -> u8 {
    // SAFETY: the symbol is the compiled thaw-std's `extern "C" fn() -> u8`, exported by `thaw_js_extra_export_args`.
    unsafe {
        let sym = libc::dlsym(libc::RTLD_DEFAULT, c"thaw_http_poll_servers".as_ptr());
        if sym.is_null() {
            return 0;
        }
        std::mem::transmute::<*mut c_void, extern "C" fn() -> u8>(sym)()
    }
}

#[cfg(not(unix))]
fn poll_native_servers_symbol() -> u8 {
    0
}

/// `drain_native` as a loop turn hook. A listen callback may queue continuations, so servers are polled first.
fn drain_native_continuations(_: &mut Interp) -> bool {
    let http = poll_native_servers();
    let ran = drain_native();
    ran || (http & 2) != 0
}

#[cfg(unix)]
fn native_promise_state(promise: usize) -> u8 {
    // SAFETY: both symbols are the compiled runtime's, looked up like the QuickJS host does.
    unsafe {
        let poll = libc::dlsym(libc::RTLD_DEFAULT, c"thaw_runtime_poll_one".as_ptr());
        let state = libc::dlsym(libc::RTLD_DEFAULT, c"thaw_promise_state".as_ptr());
        if poll.is_null() || state.is_null() {
            return 1;
        }
        std::mem::transmute::<*mut c_void, extern "C" fn() -> u8>(poll)();
        std::mem::transmute::<*mut c_void, unsafe extern "C" fn(*const c_void) -> u8>(state)(
            promise as *const c_void,
        )
    }
}

#[cfg(unix)]
fn native_promise_mark_handled(promise: usize) {
    // SAFETY: as above.
    unsafe {
        let mark = libc::dlsym(libc::RTLD_DEFAULT, c"thaw_promise_mark_handled".as_ptr());
        if !mark.is_null() {
            std::mem::transmute::<*mut c_void, unsafe extern "C" fn(*mut c_void) -> u8>(mark)(
                promise as *mut c_void,
            );
        }
    }
}

#[cfg(not(unix))]
fn native_promise_state(_promise: usize) -> u8 {
    1
}

#[cfg(not(unix))]
fn native_promise_mark_handled(_promise: usize) {}

fn call_compiled(it: &mut Interp, id: usize, this: &Value, args: &[Value]) -> R<Value> {
    let Some((adapter, context, params, graph, finish, has_rest, mask, method)) =
        CALLBACKS.lock().unwrap().get(id).map(|c| {
            (
                c.adapter, c.context, c.params, c.graph, c.finish, c.has_rest, c.mask, c.method,
            )
        })
    else {
        return it.throw_error("Error", "unknown compiled callback");
    };
    if graph {
        return call_compiled_graph(
            it, adapter, context, params, finish, has_rest, mask, method, this, args,
        );
    }
    let mut items = Vec::new();
    for arg in args.iter().take(params) {
        match from_value(it, arg, 0) {
            Ok(n) => items.push(n),
            Err(e) => {
                return it.throw_error(
                    "TypeError",
                    &format!("callback argument cannot cross into compiled code: {e}"),
                )
            }
        }
    }
    let mut text = String::from("[");
    for (i, item) in items.iter().enumerate() {
        if i > 0 {
            text.push(',');
        }
        text.push_str(&item.to_callback_json());
    }
    text.push(']');
    let Ok(text) = CString::new(text) else {
        return it.throw_error("TypeError", "callback argument contains a NUL");
    };
    // SAFETY: `adapter` is a generated `(context, args_json) -> result_json` function and `context`
    // its matching environment, both kept alive by the compiled program.
    let adapter: Adapter = unsafe { std::mem::transmute(adapter) };
    let result = thaw_arena::with_thread_stack_guard(|| unsafe {
        adapter(context as *const c_void, text.as_ptr())
    });
    let packet = thaw_arena::take_owned_exception(result);
    let out = if result.is_null() {
        String::new()
    } else {
        let s = unsafe { CStr::from_ptr(result) }
            .to_string_lossy()
            .into_owned();
        unsafe { thaw_arena::destroy_string(result) };
        s
    };
    if std::env::var_os("THAW_JS_TRACE_WIRE").is_some() {
        eprintln!("thaw-js callback({id}) result: {out:?} args={}", args.len());
    }
    if let Some(original) = packet.as_deref().and_then(original_js_exception) {
        return Err(Ctl::Throw(original));
    }
    if packet.is_some() || out.starts_with('\u{2}') {
        return Err(tagged_error(it, &out));
    }
    if out.is_empty() {
        return Ok(Value::Undefined);
    }
    let global = it.global.clone();
    let json = it.lookup(&global, "JSON").unwrap_or(Value::Undefined);
    let parse = it.get(&json, "parse")?;
    let parsed = it.call(&parse, &json, &[Value::str(&out)])?;
    let native = from_value(it, &parsed, 0).map_err(|e| {
        match it.throw_error::<()>("TypeError", &e.to_string()) {
            Err(c) => c,
            Ok(()) => Ctl::Throw(Value::Undefined),
        }
    })?;
    Ok(to_value(it, &native))
}

thread_local! {
    /// `(display, framed)` of the last `Error` object a package call threw: compiled code asks for the
    /// framed form of an error string (`thaw_tjs_frame_error`) when it raises it as a TypeScript error.
    static LAST_FRAME: RefCell<Option<(String, String)>> = const { RefCell::new(None) };
}

/// The wire framing (`\u{1}Name\u{1}message...`, name / message / `code` / own properties) of the
/// error string `error` returned by a thaw-js call, when it is the last `Error` object a package threw;
/// any other text is returned unchanged. Consumes `error`; the result is owned like the call results.
///
/// # Safety
/// `error` must be null or a string from `thaw_arena::owned_string`, not used afterwards.
#[no_mangle]
pub unsafe extern "C" fn thaw_tjs_frame_error(error: *mut c_char) -> *mut c_char {
    let Some(text) = cstr(error) else {
        return error;
    };
    // SAFETY: `error` came from `thaw_arena::owned_string` in a thaw-js call result.
    unsafe { thaw_arena::destroy_string(error) };
    let framed = LAST_FRAME.with(|f| match f.borrow_mut().take() {
        Some((display, framed)) if display == text => framed,
        _ => text,
    });
    thaw_arena::owned_string(framed)
}

/// The wire framing of a thrown `Error`-like object (`message` is a string): `\u{1}name\u{1}message`,
/// then `\u{3}code` and `\u{5}{own properties as JSON}` like the QuickJS bridge.
pub(crate) fn frame_error(it: &mut Interp, v: &Value) -> Option<(String, String)> {
    if !matches!(v, Value::Object(_)) || v.is_callable() {
        return None;
    }
    let Value::Str(message) = it.get(v, "message").ok()? else {
        return None;
    };
    let name = match it.get(v, "name").ok()? {
        Value::Str(n) => n.to_string(),
        _ => "Error".to_string(),
    };
    // An error of another realm (a node:vm context) is not an instance of this realm's Error: its chain
    // ends in the `\u{18}` sentinel, which `thaw_error_is_instance` never matches. Same-realm errors keep the
    // legacy frame. A null-prototype object is its own root, so it is not foreign.
    let foreign = {
        let mut cur = match v {
            Value::Object(o) => Some(o.clone()),
            _ => None,
        };
        let mut root = None;
        while let Some(o) = cur {
            cur = o.proto.borrow().clone();
            root = Some(o);
        }
        match (&root, v) {
            (Some(r), Value::Object(o)) => !Rc::ptr_eq(r, &it.object_proto) && !Rc::ptr_eq(r, o),
            _ => false,
        }
    };
    let mut framed = if foreign {
        format!("\u{1}\u{1e}{name}\u{1f}\u{18}\u{1}{message}")
    } else {
        format!("\u{1}{name}\u{1}{message}")
    };
    if let Ok(Value::Str(code)) = it.get(v, "code") {
        framed.push('\u{3}');
        framed.push_str(&code);
    }
    // The QuickJS bridge's property bag: strings, numbers, booleans, null and objects of the error (own and
    // inherited enumerable), serialized with the realm's JSON.stringify so nested objects (axios `response`) survive.
    let g = Value::Object(it.global_object.clone());
    if let Ok(helper) = it.get(&g, "__thaw_error_properties_json") {
        if let Ok(Value::Str(json)) = it.call(&helper, &g, std::slice::from_ref(v)) {
            if !json.is_empty() && &*json != "{}" {
                framed.push('\u{5}');
                framed.push_str(&json);
            }
        }
    }
    let text = it.to_str(v);
    let display = if text.starts_with(&name) {
        text.to_string()
    } else {
        format!("{name}: {text}")
    };
    Some((display, framed))
}

pub fn to_value(it: &mut Interp, n: &Native) -> Value {
    match n {
        Native::Undefined => Value::Undefined,
        Native::Null => Value::Null,
        Native::Bool(b) => Value::Bool(*b),
        Native::Number(x) => Value::Number(*x),
        Native::Str(s) => framed_error(it, s).unwrap_or_else(|| Value::str(s)),
        Native::Array(items) => {
            let v = items.iter().map(|i| to_value(it, i)).collect();
            it.array(v)
        }
        // `{"__thaw_tjs_cb__": id}`: a compiled callback registered with `thaw_tjs_register_callback`.
        Native::Object(fields) if matches!(fields.as_slice(), [(k, Native::Number(_))] if k == CALLBACK_KEY) =>
        {
            let Some((_, Native::Number(id))) = fields.first() else {
                return Value::Undefined;
            };
            callback_value(it, *id as usize)
        }
        // `{"__thaw_tjs_handle__": id}`: a class instance (or other value) kept in the thaw-js heap.
        Native::Object(fields) if matches!(fields.as_slice(), [(k, Native::Number(_))] if k == HANDLE_KEY) =>
        {
            let Some((_, Native::Number(id))) = fields.first() else {
                return Value::Undefined;
            };
            slot(*id as u64).map_or(Value::Undefined, |(_, v)| v)
        }
        // `{"__thaw_tjs_timer__": id}`: a Timeout/Immediate returned earlier (still live, else its id).
        Native::Object(fields) if matches!(fields.as_slice(), [(k, Native::Number(_))] if k == TIMER_KEY) =>
        {
            let Some((_, Native::Number(id))) = fields.first() else {
                return Value::Undefined;
            };
            it.timer_object(*id as u64)
        }
        // `{"__thaw_tjs_date__": ms}`: a compiled `Date` (`null` for an Invalid Date).
        Native::Object(fields) if matches!(fields.as_slice(), [(k, _)] if k == DATE_KEY) => {
            new_date(it, &fields[0].1).unwrap_or(Value::Undefined)
        }
        // `{"__thaw_tjs_bigint__": "<decimal>"}`: a compiled BigInt.
        Native::Object(fields) if matches!(fields.as_slice(), [(k, Native::Str(_))] if k == BIGINT_KEY) =>
        {
            let Native::Str(decimal) = &fields[0].1 else {
                unreachable!()
            };
            crate::bigint::parse_bigint(decimal)
                .map(|b| Value::BigInt(Rc::new(b)))
                .unwrap_or(Value::Undefined)
        }
        // `{"__thaw_tjs_num__": "-0" | "NaN" | "Infinity" | "-Infinity"}`: a compiled negative zero or non-finite number.
        Native::Object(fields) if matches!(fields.as_slice(), [(k, Native::Str(s))] if k == NUM_KEY && matches!(s.as_str(), "-0" | "NaN" | "Infinity" | "-Infinity")) =>
        {
            let Native::Str(tag) = &fields[0].1 else {
                unreachable!()
            };
            Value::Number(match tag.as_str() {
                "NaN" => f64::NAN,
                "Infinity" => f64::INFINITY,
                "-Infinity" => f64::NEG_INFINITY,
                _ => -0.0,
            })
        }
        // `{"__thaw_tjs_bytes__": {"k", "b64"}}`: compiled byte data rebuilt as a `Buffer`, or as a
        // plain `Uint8Array` when `k` says so. A branded Json wrapper (`{"__thaw_tjs_bytes__":
        // {"k":"Buffer","data":[...]}}`) carries its bytes as a number array instead of base64.
        Native::Object(fields) if matches!(fields.as_slice(), [(k, Native::Object(_))] if k == BYTES_KEY) =>
        {
            let Native::Object(parts) = &fields[0].1 else {
                unreachable!()
            };
            let bytes = match parts.iter().find(|(k, _)| k == "b64") {
                Some((_, Native::Str(s))) => {
                    base64::Engine::decode(&base64::engine::general_purpose::STANDARD, s.as_str())
                        .unwrap_or_default()
                }
                _ => match parts.iter().find(|(k, _)| k == "data") {
                    Some((_, Native::Array(items))) => items
                        .iter()
                        .map(|item| match item {
                            Native::Number(n) => *n as u8,
                            _ => 0,
                        })
                        .collect(),
                    _ => Vec::new(),
                },
            };
            let kind = match parts.iter().find(|(k, _)| k == "k") {
                Some((_, Native::Str(s))) => s.as_str(),
                _ => "Buffer",
            };
            match kind {
                "ArrayBuffer" => return it.array_buffer_from_vec(bytes),
                "DataView" => {
                    let ab = it.array_buffer_from_vec(bytes);
                    return construct_global(it, "DataView", &[ab]).unwrap_or(Value::Undefined);
                }
                _ => {}
            }
            let is_u8 = kind == "Uint8Array";
            let view = it.u8_from_vec(bytes);
            // Buffer is a Uint8Array whose prototype is `Buffer.prototype` (see buffer.rs `wrap`)
            if !is_u8 {
                let global = it.global.clone();
                let proto = it
                    .lookup(&global, "Buffer")
                    .and_then(|b| it.get(&b, "prototype").ok());
                if let (Value::Object(o), Some(Value::Object(p))) = (&view, proto) {
                    *o.proto.borrow_mut() = Some(p);
                }
            }
            view
        }
        // `{"__thaw_tjs_map__": [[k, v], ...]}` / `{"__thaw_tjs_set__": [v, ...]}`: a compiled
        // `Map`/`Set` crossing as its entries, rebuilt as a real collection.
        Native::Object(fields) if matches!(fields.as_slice(), [(k, Native::Array(_))] if k == MAP_KEY || k == SET_KEY) =>
        {
            let ctor = if fields[0].0 == MAP_KEY { "Map" } else { "Set" };
            let entries = to_value(it, &fields[0].1);
            construct_global(it, ctor, &[entries]).unwrap_or(Value::Undefined)
        }
        // `{"__thaw_regexp__": {source, flags, lastIndex}}`: a compiled `RegExp` crossing as plain data.
        Native::Object(fields) if matches!(fields.as_slice(), [(k, Native::Object(_))] if k == "__thaw_regexp__") =>
        {
            let Native::Object(parts) = &fields[0].1 else {
                unreachable!()
            };
            let text = |name: &str| match parts.iter().find(|(k, _)| k == name) {
                Some((_, Native::Str(s))) => s.as_str(),
                _ => "",
            };
            let made = construct_global(
                it,
                "RegExp",
                &[Value::str(text("source")), Value::str(text("flags"))],
            );
            made.unwrap_or(Value::Undefined)
        }
        // The compiled side's `undefined` marker (absent optional argument, undefined result).
        Native::Object(fields) if matches!(fields.as_slice(), [(k, Native::Bool(true))] if k == UNDEFINED_KEY) => {
            Value::Undefined
        }
        Native::Object(fields) => {
            let o = it.plain();
            for (k, v) in fields {
                let v = to_value(it, v);
                crate::put(&o, k, v);
            }
            Value::Object(o)
        }
    }
}

pub fn from_value(it: &mut Interp, v: &Value, depth: usize) -> Result<Native, HostError> {
    if depth > 64 {
        return Err(HostError::NotPlainData("cyclic or too deep".into()));
    }
    Ok(match v {
        Value::Undefined | Value::Hole => Native::Undefined,
        Value::Null => Native::Null,
        Value::Bool(b) => Native::Bool(*b),
        Value::Number(n) => Native::Number(*n),
        Value::Str(s) => Native::Str(s.to_string()),
        Value::BigInt(_) => return Err(HostError::NotPlainData("bigint".into())),
        Value::Object(o) => {
            if v.is_callable() {
                return Err(HostError::NotPlainData("function".into()));
            }
            if matches!(&*o.kind.borrow(), Kind::RegExp(_)) {
                return Err(HostError::NotPlainData("RegExp".into()));
            }
            if matches!(&*o.kind.borrow(), Kind::Array(_)) {
                let items = it.array_items(v);
                return Ok(Native::Array(
                    items
                        .iter()
                        .map(|i| {
                            if i.is_callable() {
                                Ok(Native::Null)
                            } else {
                                from_value(it, i, depth + 1)
                            }
                        })
                        .collect::<Result<_, _>>()?,
                ));
            }
            // an Error object crosses as its framed string, the form a compiled Error takes, so compiled
            // code revives it as an Error again (a plain object with a `message` stays an object)
            if matches!(&*o.kind.borrow(), Kind::Plain) && crate::builtins::is_error_instance(it, o)
            {
                if let Some((_, framed)) = frame_error(it, v) {
                    return Ok(Native::Str(framed));
                }
            }
            // a Date crosses as its time value
            if matches!(&*o.kind.borrow(), Kind::Plain) && it.date_target(v) {
                let ms = it.date_time(v).map_err(|c| classify(it, c))?;
                // a non-finite time value (Invalid Date) crosses as "NaN": JSON has no NaN
                let ms = if ms.is_finite() {
                    Native::Number(ms)
                } else {
                    Native::Str("NaN".into())
                };
                return Ok(Native::Object(vec![(DATE_KEY.to_string(), ms)]));
            }
            // a Buffer or Uint8Array crosses as `{"type":"Buffer","data":[...]}`, the shape the compiled
            // side decodes as `Bytes` (both flavours share it: the compiled side has one byte type)
            let bytes = match &*o.kind.borrow() {
                Kind::Typed(t) if matches!(t.ty, crate::typed::Ty::U8) => Some(t.window()),
                _ => None,
            };
            if let Some(bytes) = bytes {
                let data = bytes
                    .into_iter()
                    .map(|b| Native::Number(f64::from(b)))
                    .collect();
                return Ok(Native::Object(vec![
                    ("type".to_string(), Native::Str("Buffer".to_string())),
                    ("data".to_string(), Native::Array(data)),
                ]));
            }
            // a Timeout/Immediate crosses as its id and is resolved back to the live timer
            if let Some(id) = it.timer_handle_id(v) {
                return Ok(Native::Object(vec![(
                    TIMER_KEY.to_string(),
                    Native::Number(id as f64),
                )]));
            }
            let mut fields = Vec::new();
            for k in it.own_keys(v) {
                let val = it.get(v, &k).map_err(|c| classify(it, c))?;
                if val.is_callable() {
                    continue;
                }
                fields.push((k.to_string(), from_value(it, &val, depth + 1)?));
            }
            if matches!(&*o.kind.borrow(), Kind::Plain) && o.proto.borrow().is_none() {
                fields.push((NULL_PROTO_KEY.to_string(), Native::Bool(true)));
            }
            Native::Object(fields)
        }
    })
}

impl Native {
    /// JSON text of a callback argument: `undefined` keeps the compiled side's marker.
    fn to_callback_json(&self) -> String {
        match self {
            Native::Undefined => format!("{{\"{UNDEFINED_KEY}\":true}}"),
            Native::Array(a) => format!(
                "[{}]",
                a.iter()
                    .map(Native::to_callback_json)
                    .collect::<Vec<_>>()
                    .join(",")
            ),
            other => other.to_json(),
        }
    }

    /// JSON text of a typed-ABI call result: a top-level `undefined` keeps the compiled side's marker
    /// (so `any`/optional results can tell it from `null`); nested values still use `to_json`.
    fn to_result_json(&self) -> String {
        match self {
            Native::Undefined => format!("{{\"{UNDEFINED_KEY}\":true}}"),
            Native::Number(n) if !n.is_finite() => Native::non_finite_sentinel(*n),
            other => other.to_json_keeping_undefined(),
        }
    }

    /// NaN and the infinities cannot be JSON numbers: std decodes this sentinel back to the number at any depth.
    fn non_finite_sentinel(n: f64) -> String {
        let word = if n.is_nan() {
            "NaN"
        } else if n > 0.0 {
            "Infinity"
        } else {
            "-Infinity"
        };
        format!("{{\"$__thaw_non_finite$\":\"{word}\"}}")
    }

    /// `to_json` for a typed-call result: an `undefined` OWN property of an object keeps the compiled side's
    /// `undefined` marker instead of vanishing (Object.keys / console.log see it; JSON.stringify still omits it).
    /// Array items and the top level are unchanged (`undefined` stays `null` there).
    pub(crate) fn to_json_keeping_undefined(&self) -> String {
        match self {
            Native::Number(n) if !n.is_finite() => Native::non_finite_sentinel(*n),
            Native::Array(a) => format!(
                "[{}]",
                a.iter()
                    .map(|v| {
                        if *v == Native::Undefined {
                            "null".to_string()
                        } else {
                            v.to_json_keeping_undefined()
                        }
                    })
                    .collect::<Vec<_>>()
                    .join(",")
            ),
            Native::Object(o) => format!(
                "{{{}}}",
                o.iter()
                    .map(|(k, v)| {
                        let mut key = String::new();
                        crate::builtins::json_quote(k, &mut key);
                        let value = if *v == Native::Undefined {
                            format!("{{\"{UNDEFINED_KEY}\":true}}")
                        } else {
                            v.to_json_keeping_undefined()
                        };
                        format!("{key}:{value}")
                    })
                    .collect::<Vec<_>>()
                    .join(",")
            ),
            other => other.to_json(),
        }
    }

    /// JSON text (`undefined` becomes `null`; object keys keep order).
    pub fn to_json(&self) -> String {
        match self {
            Native::Undefined | Native::Null => "null".into(),
            Native::Bool(b) => b.to_string(),
            Native::Number(n) if n.is_finite() => crate::number_to_string(*n),
            Native::Number(_) => "null".into(),
            Native::Str(s) => {
                let mut out = String::new();
                crate::builtins::json_quote(s, &mut out);
                out
            }
            Native::Array(a) => format!(
                "[{}]",
                a.iter().map(Native::to_json).collect::<Vec<_>>().join(",")
            ),
            Native::Object(o) => format!(
                "{{{}}}",
                o.iter()
                    .filter(|(_, v)| *v != Native::Undefined)
                    .map(|(k, v)| {
                        let mut key = String::new();
                        crate::builtins::json_quote(k, &mut key);
                        format!("{key}:{}", v.to_json())
                    })
                    .collect::<Vec<_>>()
                    .join(",")
            ),
        }
    }

    /// Parse JSON text (the grammar and error messages of the evaluator's `JSON.parse`, without
    /// building an interpreter: the typed-call ABI parses its arguments on every call).
    pub fn from_json(text: &str) -> Result<Native, HostError> {
        crate::json::parse_native(text).map_err(|m| HostError::Thrown(format!("SyntaxError: {m}")))
    }
}

// ----- C ABI (JSON text boundary; the live-view ABI is milestone M4) -----

thread_local! {
    static PACKAGES: RefCell<Vec<Option<Package>>> = const { RefCell::new(Vec::new()) };
    static LAST_ERROR: RefCell<String> = const { RefCell::new(String::new()) };
}

thread_local! {
    /// Packages currently running a call, by handle: a compiled callback may call back into the
    /// package that invoked it (nested `map` callbacks), which re-uses the running instance.
    static RUNNING: RefCell<HashMap<usize, *mut Package>> = RefCell::new(HashMap::new());
}

/// Run `f` on package `handle`. The package is taken out of its slot meanwhile so a compiled
/// callback may call other packages; a re-entrant call into the same package runs on the same
/// instance (ponytail: the suspended outer call holds a `&mut` to it, formally aliasing; the outer
/// evaluator is parked inside the callback and touches nothing until it returns).
pub(crate) fn with_package<T>(
    handle: usize,
    f: impl FnOnce(&mut Package) -> Result<T, HostError>,
) -> Result<T, HostError> {
    let taken = PACKAGES.with(|ps| ps.borrow_mut().get_mut(handle).map(Option::take));
    let mut package = match taken {
        Some(Some(package)) => package,
        Some(None) => {
            let running = RUNNING.with(|r| r.borrow().get(&handle).copied());
            return match running {
                // SAFETY: the pointer is a live stack local of an enclosing `with_package`.
                Some(ptr) => f(unsafe { &mut *ptr }),
                None => Err(HostError::Bundle("thaw-js package is unloaded".into())),
            };
        }
        None => return Err(HostError::Bundle("bad handle".into())),
    };
    LAST_PACKAGE.with(|l| l.set(handle));
    RUNNING.with(|r| r.borrow_mut().insert(handle, &mut package as *mut Package));
    let result = f(&mut package);
    RUNNING.with(|r| r.borrow_mut().remove(&handle));
    PACKAGES.with(|ps| ps.borrow_mut()[handle] = Some(package));
    result
}

/// The thread-local host tables of one realm. They are moved, not copied, between the host thread
/// and the coroutine thread that runs its async bodies, so a compiled callback that runs there sees
/// the same packages, handles and callbacks as on the host thread.
struct HostTls {
    packages: Vec<Option<Package>>,
    running: HashMap<usize, *mut Package>,
    by_name: HashMap<String, usize>,
    last_package: usize,
    handles: Vec<(usize, Value)>,
    handle_refs: Vec<u32>,
    live_by_object: HashMap<usize, Vec<u64>>,
    native_objects: HashMap<(usize, String), NativeWrapper>,
    native_owners: HashMap<usize, Vec<(usize, String)>>,
    last_frame: Option<(String, String)>,
    watches: Vec<Watch>,
}

// SAFETY: the host thread and the coroutine thread strictly alternate (each hands control to the
// other through a channel), so the `Rc`s and raw pointers inside are never touched by two threads at once.
struct SendHostTls(HostTls);
unsafe impl Send for SendHostTls {}

/// Tables given up by the thread that is handing control away, keyed by the realm (`Interp` address).
/// The thread receiving control takes its realm's entry; a realm on another host thread has its own key.
static PARKED: std::sync::Mutex<Vec<(usize, SendHostTls)>> = std::sync::Mutex::new(Vec::new());

/// Give this thread's host tables for realm `key` to whichever thread takes control next.
pub(crate) fn park_host_tls(key: usize) {
    let tls = HostTls {
        packages: PACKAGES.with(|c| std::mem::take(&mut *c.borrow_mut())),
        running: RUNNING.with(|c| std::mem::take(&mut *c.borrow_mut())),
        by_name: BY_NAME.with(|c| std::mem::take(&mut *c.borrow_mut())),
        last_package: LAST_PACKAGE.with(|c| c.take()),
        handles: HANDLES.with(|c| std::mem::take(&mut *c.borrow_mut())),
        handle_refs: HANDLE_REFS.with(|c| std::mem::take(&mut *c.borrow_mut())),
        live_by_object: LIVE_BY_OBJECT.with(|c| std::mem::take(&mut *c.borrow_mut())),
        native_objects: NATIVE_OBJECTS.with(|c| std::mem::take(&mut *c.borrow_mut())),
        native_owners: NATIVE_OWNERS.with(|c| std::mem::take(&mut *c.borrow_mut())),
        last_frame: LAST_FRAME.with(|c| std::mem::take(&mut *c.borrow_mut())),
        watches: WATCHES.with(|c| std::mem::take(&mut *c.borrow_mut())),
    };
    PARKED.lock().unwrap().push((key, SendHostTls(tls)));
}

/// Take over realm `key`'s parked host tables on this thread. A no-op when nothing is parked for it.
pub(crate) fn adopt_host_tls(key: usize) {
    let parked = {
        let mut parked = PARKED.lock().unwrap();
        parked
            .iter()
            .position(|(k, _)| *k == key)
            .map(|i| parked.remove(i))
    };
    let Some((_, SendHostTls(tls))) = parked else {
        return;
    };
    let HostTls {
        packages,
        running,
        by_name,
        last_package,
        handles,
        handle_refs,
        live_by_object,
        native_objects,
        native_owners,
        last_frame,
        watches,
    } = tls;
    // the replaced values are empty (the other thread parked them), so dropping them runs no code
    drop(PACKAGES.with(|c| c.replace(packages)));
    drop(RUNNING.with(|c| c.replace(running)));
    drop(BY_NAME.with(|c| c.replace(by_name)));
    LAST_PACKAGE.with(|c| c.set(last_package));
    drop(HANDLES.with(|c| c.replace(handles)));
    drop(HANDLE_REFS.with(|c| c.replace(handle_refs)));
    drop(LIVE_BY_OBJECT.with(|c| c.replace(live_by_object)));
    drop(NATIVE_OBJECTS.with(|c| c.replace(native_objects)));
    drop(NATIVE_OWNERS.with(|c| c.replace(native_owners)));
    drop(LAST_FRAME.with(|c| c.replace(last_frame)));
    drop(WATCHES.with(|c| c.replace(watches)));
}

/// Where a "not loaded" package lookup failed: the thread, and what is loaded there.
fn not_loaded(package: &str) -> String {
    let mut names: Vec<String> = BY_NAME.with(|m| m.borrow().keys().cloned().collect());
    names.sort();
    format!(
        "thaw-js package `{package}` is not loaded (thread {:?}, loaded: [{}], packages: {})",
        std::thread::current().id(),
        names.join(", "),
        names.len()
    )
}

thread_local! {
    /// Tests only: called with the status instead of `_exit` in `terminate_process`.
    static EXIT_HOOK: Cell<Option<fn(i32)>> = const { Cell::new(None) };
}

/// Tests only: replaces the `_exit` that ends the process on `process.exit` with `hook`.
#[doc(hidden)]
pub fn set_exit_hook(hook: Option<fn(i32)>) {
    EXIT_HOOK.with(|h| h.set(hook));
}

/// The other loaded realms, as pointers (a realm up-stack of this call is reached through `RUNNING`).
fn other_realms(me: *mut Interp) -> Vec<*mut Interp> {
    let count = PACKAGES.with(|ps| ps.borrow().len());
    let mut realms = Vec::new();
    for handle in 0..count {
        let loaded = PACKAGES.with(|ps| {
            ps.borrow_mut()
                .get_mut(handle)
                .and_then(|slot| slot.as_mut().map(|p| &mut *p.interp as *mut Interp))
        });
        let ptr = loaded.or_else(|| {
            RUNNING.with(|r| {
                // SAFETY: a running package's pointer is a live stack local of its `with_package` frame.
                r.borrow()
                    .get(&handle)
                    .map(|pk| unsafe { &mut *(**pk).interp as *mut Interp })
            })
        });
        if let Some(ptr) = ptr.filter(|ptr| *ptr != me) {
            realms.push(ptr);
        }
    }
    realms
}

/// Run `f` on every other loaded realm (same safety argument as `terminate_process`).
pub(crate) fn for_other_realms(me: &mut Interp, mut f: impl FnMut(&mut Interp)) {
    for ptr in other_realms(me as *mut Interp) {
        // SAFETY: a loaded realm, or a running one up-stack through `RUNNING`; `f` only edits its timer tables.
        f(unsafe { &mut *ptr });
    }
}

/// Ends the process with `code` as `process.exit` does: every realm's `exit` listeners run, output is flushed
/// and the terminal restored, then the process ends at once (no unwinding, so no JS `catch`/`finally` runs).
/// A worker's exit stays local to the worker, so this returns for one.
fn terminate_process(it: &mut Interp, code: i32) {
    if crate::worker::is_worker(it) || it.interrupt.is_some() {
        return;
    }
    it.emit_exit_once(code);
    for ptr in other_realms(&mut *it as *mut Interp) {
        // SAFETY: the realm is loaded or up-stack of this call, and the process ends below; only its `exit`
        // listeners run here.
        let other = unsafe { &mut *ptr };
        other.emit_exit_once(code);
        other.flush_partial();
    }
    // the napi side retires before the process ends; its callbacks may run JS on this realm
    crate::napi_bridge::shutdown_at_exit(it);
    // exit listeners may have changed `process.exitCode`; every realm shares it
    let status = it.rt.shared.exit_code.get().map_or(code, |n| n as i32);
    it.flush_partial();
    let _ = std::io::Write::flush(&mut std::io::stdout());
    let _ = std::io::Write::flush(&mut std::io::stderr());
    // SAFETY: flushes every C stdio stream; no pointer is passed.
    unsafe {
        libc::fflush(std::ptr::null_mut());
    }
    crate::sysio::restore_stdin_termios();
    if let Some(hook) = EXIT_HOOK.with(Cell::get) {
        hook(status);
        return;
    }
    // SAFETY: `_exit` takes no pointers. Used instead of `std::process::exit` so TLS destructors do not tear down
    // the packages while up-stack frames still hold them (as the QuickJS bridge does).
    unsafe { libc::_exit(status) }
}

pub(crate) fn cstr(p: *const c_char) -> Option<String> {
    if p.is_null() {
        return None;
    }
    // SAFETY: the caller passes a NUL-terminated string.
    unsafe { CStr::from_ptr(p) }
        .to_str()
        .ok()
        .map(str::to_string)
}

fn leak(s: String) -> *mut c_char {
    CString::new(s.replace('\0', "\\u0000")).map_or(std::ptr::null_mut(), CString::into_raw)
}

/// Load a registry bundle. Returns a handle (>= 0) or -1 on failure / unsupported construct
/// (the reason is available from [`thaw_tjs_last_error`]).
#[no_mangle]
pub extern "C" fn thaw_tjs_load(bundle: *const c_char) -> i64 {
    let Some(src) = cstr(bundle) else { return -1 };
    match Package::load(&src) {
        Ok(p) => PACKAGES.with(|ps| {
            let mut ps = ps.borrow_mut();
            ps.push(Some(p));
            ps.len() as i64 - 1
        }),
        Err(e) => {
            LAST_ERROR.with(|l| *l.borrow_mut() = e.to_string());
            -1
        }
    }
}

/// The last error message (caller frees with [`thaw_tjs_free`]).
#[no_mangle]
pub extern "C" fn thaw_tjs_last_error() -> *mut c_char {
    leak(LAST_ERROR.with(|l| l.borrow().clone()))
}

/// Call `path` on package `handle` with a JSON array of arguments; returns the JSON result
/// (caller frees with [`thaw_tjs_free`]) or NULL on error.
#[no_mangle]
pub extern "C" fn thaw_tjs_call(
    handle: i64,
    path: *const c_char,
    args_json: *const c_char,
) -> *mut c_char {
    let (Some(path), Some(args)) = (cstr(path), cstr(args_json)) else {
        return std::ptr::null_mut();
    };
    let result = (|| {
        let Native::Array(args) = Native::from_json(&args)? else {
            return Err(HostError::Bundle("arguments must be a JSON array".into()));
        };
        with_package(handle as usize, |p| p.call(&path, &args))
    })();
    match result {
        Ok(v) => leak(v.to_json()),
        Err(e) => {
            LAST_ERROR.with(|l| *l.borrow_mut() = e.to_string());
            std::ptr::null_mut()
        }
    }
}

#[no_mangle]
pub extern "C" fn thaw_tjs_unload(handle: i64) {
    // The napi host is thread-local: it retires with the last package of this thread, before that realm is dropped.
    let last = PACKAGES.with(|ps| {
        ps.borrow()
            .iter()
            .enumerate()
            .all(|(i, slot)| i as i64 == handle || slot.is_none())
    });
    if last && crate::napi_bridge::registered() {
        let _ = with_package(handle as usize, |p| {
            crate::napi_bridge::shutdown_at_exit(&mut p.interp);
            Ok(())
        });
    }
    PACKAGES.with(|ps| {
        if let Some(slot) = ps.borrow_mut().get_mut(handle as usize) {
            *slot = None;
        }
    });
}

/// Node's `-e SRC` for a compiled app that re-runs itself through `process.execPath` (the codegen
/// renames `thaw_js_run_cli` to this symbol for thaw-js builds). Returns -1 for an ordinary start,
/// otherwise the exit status. Script files are not handled here, so they still run as `main`.
#[no_mangle]
pub extern "C" fn thaw_tjs_run_cli() -> i32 {
    let arguments: Vec<String> = std::env::args().collect();
    if let Some(index) = crate::eventloop::cli_eval_index(&arguments) {
        let Some(source) = arguments.get(index + 1) else {
            eprintln!("{}: -e requires an argument", arguments[0]);
            return 1;
        };
        return run_eval_source(source);
    }
    match crate::eventloop::cli_script_index(&arguments) {
        Some(index) => run_script_file(&arguments[index]),
        None => -1,
    }
}

/// Runs `source` in the default realm the way `node -e` does, then drains the event loop; returns
/// the exit status (an uncaught error is 1).
pub(crate) fn run_eval_source(source: &str) -> i32 {
    run_default_main(|it| it.run_main(source))
}

/// Runs a CommonJS script file as the main module (`node file.js`), then drains the event loop.
fn run_script_file(path: &str) -> i32 {
    run_default_main(|it| it.run_main_file(path))
}

fn run_default_main(main: impl FnOnce(&mut Interp) -> R<i32>) -> i32 {
    let handle = default_realm();
    let ran = with_package(handle, |p| {
        let code = main(&mut p.interp).map_err(|c| classify(&mut p.interp, c));
        p.interp.flush_partial();
        code
    });
    let status = match ran {
        Ok(code) => code,
        Err(e) => {
            eprintln!("{e}");
            1
        }
    };
    let loop_status = thaw_tjs_run_event_loop();
    // Output written during the loop (timers, then-callbacks) may end in a partial line too.
    let _ = with_package(handle, |p| {
        p.interp.flush_partial();
        Ok(())
    });
    if status != 0 {
        status
    } else {
        loop_status
    }
}

/// Wall-clock time slice of one package's turn in the terminal loop (see `other_packages_have_work`).
const SLICE_MS: u64 = 5;

/// Another loaded package (not the one running; that one is taken out of its slot) has work: the running
/// package's loop yields at its slice end so every package makes progress.
pub(crate) fn other_packages_have_work() -> bool {
    PACKAGES.with(|ps| {
        ps.borrow()
            .iter()
            .flatten()
            .any(|p| p.interp.work_pending())
    })
}

/// Terminal event loop for a routed build that holds no QuickJS context: runs every loaded
/// package's timers/immediates/jobs until idle. Returns 1 if a package ended with an uncaught
/// error. (The codegen renames `thaw_js_run_event_loop` to this symbol for such builds.)
#[no_mangle]
pub extern "C" fn thaw_tjs_run_event_loop() -> i32 {
    let mut status = 0;
    let mut exited = false;
    loop {
        // the compiled http server (if any) is serviced here too, so a routed build with no JS timers still answers
        let http = poll_native_servers();
        let servers_live = (http & 1) != 0;
        let http_ran = (http & 2) != 0;
        let settled = pump_watches();
        // a settled watch resumes the compiled async code queued on the native side, which may watch new promises;
        // a live server's accept watchers progress only through this drain
        let drained = ((settled > 0 || servers_live) && drain_native()) || http_ran;
        let count = PACKAGES.with(|ps| ps.borrow().len());
        let mut ran_any = false;
        let mut yielded_any = false;
        for handle in 0..count {
            let ran = with_package(handle, |p| {
                // work that native continuations queue after the idle pass still runs (exit is emitted once)
                if !p.interp.work_pending() {
                    return Ok(0);
                }
                ran_any = true;
                // native continuations (compiled async code) run inside the loop, so a worker that keeps the loop
                // alive does not starve a Promise a timer just resolved; re-entry into this package is `with_package`'s
                p.interp.turn_hook = Some(drain_native_continuations);
                p.interp.slice_end =
                    Some(std::time::Instant::now() + std::time::Duration::from_millis(SLICE_MS));
                let r = p
                    .interp
                    .run_loop_only()
                    .map_err(|c| classify(&mut p.interp, c));
                p.interp.slice_end = None;
                p.interp.turn_hook = None;
                // the slice ended with work left: not idle, so no exit handling; the next round re-enters this package
                if std::mem::take(&mut p.interp.yielded) {
                    yielded_any = true;
                    // process.exit never yields, but stay safe if the flag is set
                    if p.interp.rt.shared.exit_requested.get() {
                        let code = crate::procstate::host_shared()
                            .exit_code
                            .get()
                            .map_or(0, |n| n as i32);
                        terminate_process(&mut p.interp, code);
                        exited = true;
                    }
                    return Ok(0);
                }
                // `process.exit` from a timer, immediate, tick or stdin handler ends the process here
                if let (true, Ok(code)) = (p.interp.rt.shared.exit_requested.get(), &r) {
                    terminate_process(&mut p.interp, *code);
                    exited = true;
                }
                r
            });
            // `process.exit(n)` / `process.exitCode = n` inside a package (e.g. commander after a sub-command) is the program's status.
            match ran {
                Err(e) => {
                    eprintln!("{e}");
                    status = 1;
                }
                Ok(code) if code != 0 => status = code,
                Ok(_) => {}
            }
        }
        // ponytail: a long timer keeps a package's loop alive, so a watch whose promise settles only through jobs
        // waits until that loop returns; pumping from inside the turn hook would re-enter `with_package`. Upgrade: pump there.
        if exited {
            break;
        }
        if servers_live && !ran_any && !yielded_any && settled == 0 && !drained {
            // ponytail: 1 ms poll like QuickJS api.rs:1821; upgrade = block in the runtime fd poll
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        let watching = WATCHES.with(|w| !w.borrow().is_empty());
        if settled == 0 && !drained && !(ran_any && watching) && !yielded_any && !servers_live {
            break;
        }
    }
    // Idle for real (no thaw-js work, no native watch, no live server, no process.exit): each realm runs its
    // `beforeExit` re-arm loop and then `exit`, once. A realm with listeners but no work still gets them.
    if !exited && !WATCHES.with(|w| !w.borrow().is_empty()) && !crate::host::native_servers_live() {
        let count = PACKAGES.with(|ps| ps.borrow().len());
        for handle in 0..count {
            let ran = with_package(handle, |p| {
                if p.interp.exit_emitted() || !p.interp.exit_listeners() {
                    return Ok(0);
                }
                p.interp.turn_hook = Some(drain_native_continuations);
                let r = p
                    .interp
                    .run_event_loop()
                    .map_err(|c| classify(&mut p.interp, c));
                p.interp.turn_hook = None;
                r
            });
            match ran {
                Err(e) => {
                    eprintln!("{e}");
                    status = 1;
                }
                Ok(code) if code != 0 => status = code,
                Ok(_) => {}
            }
        }
    }
    // a natural end with `process.exitCode = n` and no pending work is the program's status too
    if status == 0 {
        status = crate::procstate::host_shared()
            .exit_code
            .get()
            .map_or(0, |n| n as i32);
    }
    status
}

/// Drains the `process.nextTick` queue of every loaded package. A routed entry calls this before the compiled
/// promise queue, so ticks run before promise jobs (Node order). A tick that throws ends the process.
#[no_mangle]
pub extern "C" fn thaw_tjs_drain_next_ticks() {
    let count = PACKAGES.with(|ps| ps.borrow().len());
    for handle in 0..count {
        let _ = with_package(handle, |p| {
            let r = p.interp.drain_ticks();
            if p.interp.rt.shared.exit_requested.get() {
                let code = crate::procstate::host_shared()
                    .exit_code
                    .get()
                    .map_or(0, |n| n as i32);
                terminate_process(&mut p.interp, code);
            }
            if let Err(c) = r {
                eprintln!("{}", classify(&mut p.interp, c));
                terminate_process(&mut p.interp, 1);
            }
            Ok::<(), HostError>(())
        });
    }
}

/// Fatal uncaught exception in a routed build with no handler: every realm drops its queued work and emits
/// `exit` with code 1 (the thaw-js half of the QuickJS `thaw_js_fatal_exit`).
#[no_mangle]
pub extern "C" fn thaw_tjs_fatal_exit() {
    let count = PACKAGES.with(|ps| ps.borrow().len());
    for handle in 0..count {
        let _ = with_package(handle, |p| {
            p.interp.drop_pending_work();
            p.interp.set_exit_code(Some(1.0));
            p.interp.emit_exit_once(1);
            Ok::<(), HostError>(())
        });
    }
}

/// 1 while any loaded package still has referenced timers/immediates or queued jobs.
#[no_mangle]
pub extern "C" fn thaw_tjs_terminal_work_pending() -> u8 {
    let count = PACKAGES.with(|ps| ps.borrow().len());
    ((0..count).any(|h| {
        PACKAGES.with(|ps| {
            ps.borrow()
                .get(h)
                .is_some_and(|p| p.as_ref().is_some_and(|p| p.interp.work_pending()))
        })
    }) || crate::napi_bridge::pending()) as u8
}

/// Free a string returned by this module.
///
/// # Safety
/// `p` must come from `thaw_tjs_call` / `thaw_tjs_last_error` and not be freed twice.
#[no_mangle]
pub unsafe extern "C" fn thaw_tjs_free(p: *mut c_char) {
    if !p.is_null() {
        drop(unsafe { CString::from_raw(p) });
    }
}

// ----- typed-call ABI used by compiled programs (`DynamicBackend::ThawJs`) -----

/// Same layout as thaw-napi's `ThawResult`: JSON text or an error string, both `thaw_arena`-owned.
#[repr(C)]
pub struct ThawResult {
    pub value: *mut c_char,
    pub error: *mut c_char,
}

thread_local! {
    static BY_NAME: RefCell<HashMap<String, usize>> = RefCell::new(HashMap::new());
}

fn text_result(r: Result<String, String>) -> ThawResult {
    match r {
        Ok(v) => ThawResult {
            value: thaw_arena::owned_string(v),
            error: ptr::null_mut(),
        },
        Err(e) => ThawResult {
            value: ptr::null_mut(),
            error: thaw_arena::owned_string(e),
        },
    }
}

/// Load `bundle` (registry `bundle.js`, uncompressed) under `name`. Returns 1 on success.
#[no_mangle]
pub extern "C" fn thaw_tjs_register(name: *const c_char, bundle: *const c_char) -> u8 {
    let (Some(name), Some(src)) = (cstr(name), cstr(bundle)) else {
        return 0;
    };
    if std::env::var_os("THAW_JS_TRACE").is_some() {
        eprintln!("thaw-js: loading package `{name}`");
    }
    match Package::load_named(&name, &src, process_stack_bytes()) {
        Ok(p) => {
            PACKAGES.with(|ps| {
                let mut ps = ps.borrow_mut();
                ps.push(Some(p));
                BY_NAME.with(|m| m.borrow_mut().insert(name, ps.len() - 1));
            });
            1
        }
        Err(e) => {
            eprintln!("thaw-js: failed to load package `{name}`: {e}");
            LAST_ERROR.with(|l| *l.borrow_mut() = e.to_string());
            0
        }
    }
}

/// Run a script (sloppy, global scope) in the default realm; 1 on success. The counterpart of
/// QuickJS's `thaw_js_load` for the class values compiled code defines.
#[no_mangle]
pub extern "C" fn thaw_tjs_load_script(source: *const c_char) -> u8 {
    let Some(source) = cstr(source) else { return 0 };
    let source = crate::napi_eval::unpack_script(source);
    let owner = default_realm();
    let result = with_package(owner, |p| {
        p.interp
            .run_sloppy(&source)
            .map_err(|c| classify(&mut p.interp, c))
    });
    match result {
        Ok(_) => 1,
        Err(e) => {
            eprintln!("thaw-js: failed to load script: {e}");
            0
        }
    }
}

/// The main thread's stack limit (`RLIMIT_STACK`), capped at 1 GiB (256 MiB when unlimited); 0 when unknown.
fn process_stack_bytes() -> usize {
    #[repr(C)]
    struct RLimit {
        cur: u64,
        max: u64,
    }
    extern "C" {
        fn getrlimit(resource: i32, rlim: *mut RLimit) -> i32;
    }
    let mut r = RLimit { cur: 0, max: 0 };
    // SAFETY: `r` is a valid out-pointer; RLIMIT_STACK is 3 on Linux and macOS.
    let ok = unsafe { getrlimit(3, &mut r) } == 0;
    match (ok, r.cur) {
        (true, u64::MAX) => 256 << 20,
        (true, cur) => (cur as usize).min(1 << 30),
        _ => 0,
    }
}

/// `{"__thaw_tjs_pending__": <handle>}`: the result of a `\u{4}` call whose Promise is still pending.
const PENDING_KEY: &str = "__thaw_tjs_pending__";

fn pending_marker_json(id: i64) -> String {
    format!("{{\"{PENDING_KEY}\":{id}}}")
}

/// Settles a native promise compiled code holds: `(context, result JSON or NULL, framed error or NULL)`.
pub type SettleFn = extern "C" fn(*mut c_void, *const c_char, *const c_char);

struct Watch {
    package: usize,
    promise: Value,
    native: *mut c_void,
    settle: SettleFn,
}

thread_local! {
    /// Pending promises compiled code watches (`thaw_tjs_watch_promise`); `pump_watches` settles them.
    static WATCHES: RefCell<Vec<Watch>> = const { RefCell::new(Vec::new()) };
}

/// Watch the Promise behind a `\u{4}` call's pending handle: `settle` runs once it leaves Pending, after the
/// queued jobs of its package have drained (see `pump_watches`).
#[no_mangle]
pub extern "C" fn thaw_tjs_watch_promise(handle: i64, native: *mut c_void, settle: SettleFn) {
    // 0 is `undefined`, which is never a pending promise: it takes the failure path below.
    let entry = if handle == 0 {
        None
    } else {
        slot(handle as u64)
    };
    match entry {
        Some((package, promise)) => WATCHES.with(|w| {
            w.borrow_mut().push(Watch {
                package,
                promise,
                native,
                settle,
            })
        }),
        None => settle_native(
            native,
            settle,
            Err("thaw-js: invalid pending promise handle".into()),
        ),
    }
}

fn settle_native(native: *mut c_void, settle: SettleFn, result: Result<String, String>) {
    let text = |s: String| CString::new(s.replace('\0', "\u{FFFD}")).unwrap();
    let (json, error) = match result {
        Ok(json) => (Some(text(json)), None),
        Err(error) => (None, Some(text(error))),
    };
    settle(
        native,
        json.as_ref().map_or(ptr::null(), |s| s.as_ptr()),
        error.as_ref().map_or(ptr::null(), |s| s.as_ptr()),
    );
}

/// Drain the jobs of every package with a watched promise, then settle the watches that left Pending.
/// A no-op while nothing is watched, so plain calls behave as before. Returns how many watches settled.
fn pump_watches() -> usize {
    let mut packages: Vec<usize> = WATCHES.with(|w| w.borrow().iter().map(|x| x.package).collect());
    if packages.is_empty() {
        return 0;
    }
    packages.sort_unstable();
    packages.dedup();
    for package in packages {
        if let Err(e) = with_package(package, |p| {
            p.interp.run_jobs().map_err(|c| classify(&mut p.interp, c))
        }) {
            eprintln!("{e}");
        }
    }
    let watches = WATCHES.with(|w| std::mem::take(&mut *w.borrow_mut()));
    let mut waiting = Vec::new();
    let mut settled = Vec::new();
    for w in watches {
        let state = with_package(w.package, |p| {
            let Some(state) = p.interp.promise_state(&w.promise) else {
                return Ok(None);
            };
            Ok(Some(match state {
                Ok(v) => from_value(&mut p.interp, &v, 0)
                    .map(|n| n.to_json_keeping_undefined())
                    .map_err(|e| e.to_string()),
                Err(e) => Err(match frame_error(&mut p.interp, &e) {
                    Some((_, framed)) => framed,
                    None => from_value(&mut p.interp, &e, 0)
                        .map(|n| n.to_json())
                        .unwrap_or_default(),
                }),
            }))
        });
        match state {
            Ok(None) => waiting.push(w),
            Ok(Some(result)) => settled.push((w, result)),
            Err(e) => settled.push((w, Err(e.to_string()))),
        }
    }
    // keep the still-pending watches (in order) and any added while settling; settle outside the borrow
    WATCHES.with(|w| {
        let mut cur = w.borrow_mut();
        waiting.append(&mut cur);
        *cur = waiting;
    });
    let count = settled.len();
    for (w, result) in settled {
        settle_native(w.native, w.settle, result);
    }
    count
}

/// Call `"<package>::<export>"` with a JSON array of arguments; the result is JSON text.
#[no_mangle]
pub extern "C" fn thaw_tjs_call_typed_result(
    symbol: *const c_char,
    args_json: *const c_char,
) -> ThawResult {
    text_result((|| {
        let (Some(symbol), Some(args)) = (cstr(symbol), cstr(args_json)) else {
            return Err("invalid call text".to_string());
        };
        // The compiler routes a call whose signature cannot cross the boundary here as an error.
        if let Some(message) = symbol.strip_prefix('\u{1}') {
            return Err(message.to_string());
        }
        // `\u{2}` prefix: the result is wanted as a handle id (a `JsValue` result); `\u{3}`: as a Date node;
        // `\u{4}`: a Promise result, which may still be pending (`{"__thaw_tjs_pending__":<handle>}`).
        let (want_handle, want_date, want_pending, symbol) = match symbol.as_str() {
            s if s.starts_with('\u{2}') => (true, false, false, &s[1..]),
            s if s.starts_with('\u{3}') => (false, true, false, &s[1..]),
            s if s.starts_with('\u{4}') => (false, false, true, &s[1..]),
            s => (false, false, false, s),
        };
        if std::env::var_os("THAW_JS_TRACE_WIRE").is_some() {
            eprintln!("thaw-js call: {symbol} {args}");
        }
        let (package, export) = symbol.split_once("::").ok_or("malformed thaw-js symbol")?;
        let handle = BY_NAME
            .with(|m| m.borrow().get(package).copied())
            .ok_or_else(|| not_loaded(package))?;
        let Native::Array(args) = Native::from_json(&args).map_err(|e| e.to_string())? else {
            return Err("arguments must be a JSON array".into());
        };
        let out = with_package(handle, |p| {
            let r = p.call_or_get_value(export, Some(&args), true, want_pending)?;
            if want_pending && p.interp.is_promise(&r) && p.interp.promise_state(&r).is_none() {
                return Ok(pending_marker_json(store_handle(handle, r)));
            }
            if want_handle {
                return Ok(store_handle(handle, r).to_string());
            }
            if is_live_value(&p.interp, &r) && !(want_date && p.interp.date_target(&r)) {
                return Ok(handle_marker_json(store_handle(handle, r)));
            }
            // A plain result that compiled code already holds by handle returns that handle (same identity).
            if !(want_date && p.interp.date_target(&r)) {
                if let Some(id) = live_handle_of(&r) {
                    thaw_tjs_retain_handle(id as i64);
                    return Ok(handle_marker_json(id as i64));
                }
            }
            from_value(&mut p.interp, &r, 0).map(|n| n.to_result_json())
        })
        .map_err(|e| e.to_string());
        pump_watches();
        out
    })())
}

#[cfg(test)]
mod native_owner_tests {
    use super::*;
    use std::ffi::CString;

    /// One layout field: the field name and a second pair, each `<bytes>:<hex>`.
    fn field(name: &str) -> String {
        let seg = |text: &[u8]| {
            format!(
                "{}:{}",
                text.len(),
                text.iter().map(|b| format!("{b:02x}")).collect::<String>()
            )
        };
        format!("{}{}", seg(name.as_bytes()), seg(b"T"))
    }

    fn class_field(chain: &str) -> String {
        field(&format!("__thaw_class_identity_\u{1e}{chain}"))
    }

    fn new_handle() -> u64 {
        let args = CString::new(r#"["return 1"]"#).unwrap();
        let result = thaw_tjs_new_function(args.as_ptr());
        assert!(result.error.is_null());
        result.value
    }

    /// Interns a fresh candidate object for `(pointer, layout)` and returns the handle.
    fn intern(pointer: usize, layout: &str) -> u64 {
        let layout = CString::new(layout).unwrap();
        let result =
            thaw_tjs_intern_native_object(pointer as *const c_void, layout.as_ptr(), new_handle());
        assert!(result.error.is_null());
        result.value
    }

    fn pointer_of(handle: u64, layout: &str) -> u64 {
        let layout = CString::new(layout).unwrap();
        let result = thaw_tjs_native_object_pointer(handle, layout.as_ptr());
        assert!(result.error.is_null());
        result.value
    }

    #[test]
    fn interned_wrapper_reports_its_owner_pointer() {
        let layout = field("a");
        let wrapper = intern(0x1000, &layout);
        assert_eq!(pointer_of(wrapper, &layout), 0x1000);
        // interning the same (pointer, layout) again hands back the canonical wrapper
        let again = intern(0x1000, &layout);
        assert_eq!(pointer_of(again, &layout), 0x1000);
    }

    #[test]
    fn layout_mismatch_and_plain_handles_report_no_owner() {
        let wrapper = intern(0x2000, &field("a"));
        assert_eq!(pointer_of(wrapper, &field("b")), 0);
        assert_eq!(pointer_of(new_handle(), &field("a")), 0);
        assert_eq!(pointer_of(0, &field("a")), 0);
    }

    #[test]
    fn empty_expected_layout_matches_any_owner() {
        let wrapper = intern(0x3000, &field("a"));
        assert_eq!(pointer_of(wrapper, ""), 0x3000);
    }

    #[test]
    fn subclass_wrapper_satisfies_base_layout_only() {
        let derived = intern(0x4000, &class_field("Derived\u{1f}Base"));
        assert_eq!(pointer_of(derived, &class_field("Base")), 0x4000);
        let base = intern(0x5000, &class_field("Base"));
        assert_eq!(pointer_of(base, &class_field("Derived\u{1f}Base")), 0);
    }

    /// The wrapper address behind a handle, else 0 for a miss (0 is never a live address).
    fn wrapper_addr(handle: u64) -> usize {
        if handle == 0 {
            return 0;
        }
        match slot(handle) {
            Some((_, Value::Object(obj))) => Rc::as_ptr(&obj) as usize,
            _ => panic!("handle is not an object"),
        }
    }

    fn lookup(pointer: usize, layout: &str) -> usize {
        let layout = CString::new(layout).unwrap();
        let result = thaw_tjs_lookup_native_object(pointer as *const c_void, layout.as_ptr());
        assert!(result.error.is_null());
        wrapper_addr(result.value)
    }

    #[test]
    fn lookup_serves_base_layout_from_interned_derived_wrapper() {
        let derived = intern(0x7000, &class_field("Derived\u{1f}Base"));
        let wrapper = lookup(0x7000, &class_field("Base"));
        assert_ne!(wrapper, 0);
        assert_eq!(wrapper, wrapper_addr(derived));
    }

    #[test]
    fn lookup_does_not_serve_derived_layout_from_base_wrapper() {
        intern(0x8000, &class_field("Base"));
        assert_eq!(lookup(0x8000, &class_field("Derived\u{1f}Base")), 0);
    }

    #[test]
    fn lookup_misses_unrelated_layout_and_other_pointers() {
        intern(0x9000, &field("a"));
        assert_eq!(lookup(0x9000, &field("b")), 0);
        assert_eq!(lookup(0x9001, &field("a")), 0);
    }

    #[test]
    fn lookup_exact_layout_hits_before_contained_layout() {
        let derived = intern(0xa000, &class_field("Derived\u{1f}Base"));
        let base = intern(0xa000, &class_field("Base"));
        assert_ne!(wrapper_addr(derived), wrapper_addr(base));
        assert_eq!(lookup(0xa000, &class_field("Base")), wrapper_addr(base));
        assert_eq!(
            lookup(0xa000, &class_field("Derived\u{1f}Base")),
            wrapper_addr(derived)
        );
    }

    #[test]
    fn null_layout_is_an_error() {
        let wrapper = intern(0x6000, &field("a"));
        let result = thaw_tjs_native_object_pointer(wrapper, std::ptr::null());
        assert!(!result.error.is_null());
        // SAFETY: the error string was just returned by the call above and is freed once.
        unsafe { thaw_arena::destroy_string(result.error) };
    }

    #[test]
    fn layout_contains_matches_quickjs_rules() {
        let a = field("a");
        let b = field("b");
        assert!(layout_contains(&a, &a));
        assert!(layout_contains("", ""));
        assert!(layout_contains(&a, ""));
        assert!(!layout_contains("", &a));
        // malformed actual layout (the field runs past the end)
        assert!(!layout_contains("3:zz", ""));
        // a wanted subset of the actual fields
        assert!(layout_contains(&format!("{a}{b}"), &a));
        assert!(!layout_contains(&a, &format!("{a}{b}")));
        assert!(!layout_contains(
            &format!("{a}{b}"),
            &format!("{a}{}", field("c"))
        ));
        // each wanted field needs its own actual field
        assert!(!layout_contains(&a, &format!("{a}{a}")));
        assert!(layout_contains(&format!("{a}{a}"), &format!("{a}{a}")));
        // a class identity only satisfies a class identity
        assert!(!layout_contains(&field("Base"), &class_field("Base")));
        assert!(layout_contains(
            &class_field("Derived\u{1f}Base"),
            &class_field("Base")
        ));
        assert!(!layout_contains(
            &class_field("Base"),
            &class_field("Derived\u{1f}Base")
        ));
    }
}

#[cfg(test)]
mod live_result_lease_tests {
    use super::*;

    fn plain_wire() -> WireOut {
        WireOut {
            owner: 0,
            plain: true,
            live_functions: false,
            copy_root: false,
            receiver: None,
            nodes: Vec::new(),
            seen: Vec::new(),
            leases: Vec::new(),
        }
    }

    fn token_of(it: &mut Interp, v: &Value) -> (String, WireOut) {
        let mut out = plain_wire();
        let Ok(token) = out.token(it, v) else {
            panic!("token failed")
        };
        (token, out)
    }

    fn object(it: &mut Interp, src: &str) -> Value {
        let Ok(v) = it.run(src) else {
            panic!("script failed")
        };
        v
    }

    #[test]
    fn plain_wire_emits_bytes_as_b_node_and_keeps_buffer_shaped_literal_as_object() {
        let mut it = Interp::new();
        let buf = object(&mut it, "Buffer.from([1, 2])");
        let (token, out) = token_of(&mut it, &buf);
        assert_eq!(token, "{\"r\":0}");
        assert_eq!(out.nodes, vec!["{\"b\":[1,2]}".to_string()]);

        let u8 = object(&mut it, "new Uint8Array([1, 2])");
        let (token, out) = token_of(&mut it, &u8);
        assert_eq!(token, "{\"r\":0}");
        assert_eq!(out.nodes, vec!["{\"b\":[1,2]}".to_string()]);

        // a plain user object with the Buffer shape is still an object node, not bytes
        let literal = object(&mut it, "({type: 'Buffer', data: [1, 2]})");
        let (_, out) = token_of(&mut it, &literal);
        assert!(out.nodes[0].starts_with("{\"o\":"), "{:?}", out.nodes);
    }

    #[test]
    fn live_object_leases_its_handle_and_equal_shape_copies() {
        let mut it = Interp::new();
        let live = object(&mut it, "({x: 7})");
        let id = store_handle(0, live.clone()) as u64;

        let (token, out) = token_of(&mut it, &live);
        assert_eq!(token, "{\"r\":0}");
        assert_eq!(out.nodes, vec![format!("{{\"hdl\":{id}}}")]);
        assert_eq!(out.leases, vec![id]);
        assert_eq!(HANDLE_REFS.with(|r| r.borrow()[id as usize - 1]), 2);

        // an equal-shaped but different object is still copied as data
        let other = object(&mut it, "({x: 7})");
        let (token, out) = token_of(&mut it, &other);
        assert!(token.starts_with("{\"r\":0}"), "{token}");
        assert!(out.nodes[0].starts_with("{\"o\":"), "{:?}", out.nodes);
        assert!(out.leases.is_empty());
    }

    #[test]
    fn released_object_is_copied_again() {
        let mut it = Interp::new();
        let live = object(&mut it, "[1, 2]");
        let id = store_handle(0, live.clone()) as u64;
        let (_, out) = token_of(&mut it, &live);
        assert_eq!(out.leases, vec![id]);
        // drop the lease the wire took, then the handle itself: the object is no longer held
        assert_eq!(thaw_tjs_release_handle(id as i64), 1);
        assert_eq!(thaw_tjs_release_handle(id as i64), 1);
        let (_, out) = token_of(&mut it, &live);
        assert!(out.leases.is_empty());
        assert!(out.nodes[0].starts_with("{\"a\":"), "{:?}", out.nodes);
    }

    fn plain_wire_live_functions() -> WireOut {
        WireOut {
            live_functions: true,
            ..plain_wire()
        }
    }

    #[test]
    fn live_functions_keep_function_members_as_handles() {
        let mut it = Interp::new();
        let v = object(&mut it, "({f(){}, n: 1})");
        let mut out = plain_wire_live_functions();
        let Ok(token) = out.token(&mut it, &v) else {
            panic!("token failed")
        };
        assert_eq!(token, "{\"r\":0}");
        assert!(
            out.nodes[0].contains("[\"f\",{\"r\":1},7]"),
            "{:?}",
            out.nodes
        );
        assert!(
            out.nodes[0].contains("[\"n\",{\"v\":1},7]"),
            "{:?}",
            out.nodes
        );
        assert_eq!(out.leases.len(), 1, "{:?}", out.leases);
        assert_eq!(out.nodes[1], format!("{{\"hdl\":{}}}", out.leases[0]));
        thaw_tjs_release_handle(out.leases[0] as i64);
    }

    #[test]
    fn plain_wire_still_turns_function_members_into_undefined() {
        let mut it = Interp::new();
        let v = object(&mut it, "({f(){}, n: 1})");
        let (token, out) = token_of(&mut it, &v);
        assert_eq!(token, "{\"r\":0}");
        assert!(
            out.nodes[0].contains("[\"f\",{\"u\":1},7]"),
            "{:?}",
            out.nodes
        );
        assert!(out.leases.is_empty());
    }

    #[test]
    fn live_function_already_held_by_handle_reuses_it() {
        let mut it = Interp::new();
        let f = object(&mut it, "(function named() {})");
        let id = store_handle(0, f.clone()) as u64;
        let mut out = plain_wire_live_functions();
        let Ok(token) = out.token(&mut it, &f) else {
            panic!("token failed")
        };
        assert_eq!(token, "{\"r\":0}");
        assert_eq!(out.leases, vec![id]);
        // drop the wire's lease, then the handle itself
        thaw_tjs_release_handle(id as i64);
        thaw_tjs_release_handle(id as i64);
    }
}

#[cfg(test)]
mod handle_identity_tests {
    use super::*;

    fn eval(it: &mut Interp, src: &str) -> Value {
        it.run(src).unwrap_or_else(|_| panic!("script failed"))
    }

    fn refs_of(id: i64) -> u32 {
        HANDLE_REFS.with(|r| r.borrow()[id as usize - 1])
    }

    #[test]
    fn one_object_read_twice_names_one_id() {
        let mut it = Interp::new();
        let a = store_handle(0, eval(&mut it, "Buffer"));
        let b = store_handle(0, eval(&mut it, "globalThis.Buffer"));
        assert_eq!(a, b);
        assert_eq!(refs_of(a), 2);
        assert!(handle_is_live(a));
    }

    #[test]
    fn releasing_one_of_two_leases_keeps_the_value() {
        let mut it = Interp::new();
        let v = eval(&mut it, "({k: 1})");
        let id = store_handle(0, v.clone());
        assert_eq!(store_handle(0, v), id);
        assert_eq!(thaw_tjs_release_handle(id), 1);
        assert_eq!(refs_of(id), 1);
        assert!(matches!(slot(id as u64), Some((0, Value::Object(_)))));
        // the last release frees the slot and the object binding
        assert_eq!(thaw_tjs_release_handle(id), 1);
        assert!(!handle_is_live(id));
        assert!(matches!(slot(id as u64), Some((0, Value::Undefined))));
        assert_eq!(thaw_tjs_release_handle(id), 0);
    }

    #[test]
    fn retain_counts_as_one_more_lease_on_the_same_id() {
        let mut it = Interp::new();
        let v = eval(&mut it, "[1, 2]");
        let id = store_handle(0, v.clone());
        assert_eq!(thaw_tjs_retain_handle(id), 1);
        assert_eq!(store_handle(0, v), id);
        assert_eq!(refs_of(id), 3);
        for _ in 0..3 {
            assert_eq!(thaw_tjs_release_handle(id), 1);
        }
        assert!(!handle_is_live(id));
    }

    #[test]
    fn released_then_recreated_object_gets_a_fresh_id() {
        let mut it = Interp::new();
        let v = eval(&mut it, "({k: 2})");
        let id = store_handle(0, v.clone());
        assert_eq!(thaw_tjs_release_handle(id), 1);
        let again = store_handle(0, v.clone());
        assert_ne!(again, id);
        assert!(handle_is_live(again));
        assert!(!handle_is_live(id));
        assert_eq!(thaw_tjs_release_handle(again), 1);
    }

    #[test]
    fn primitives_never_intern() {
        let a = store_handle(0, Value::Number(1.0));
        let b = store_handle(0, Value::Number(1.0));
        assert_ne!(a, b);
        assert_eq!(refs_of(a), 1);
        assert_eq!(refs_of(b), 1);
        thaw_tjs_release_handle(a);
        thaw_tjs_release_handle(b);
    }

    #[test]
    fn symbols_intern_by_identity() {
        let mut it = Interp::new();
        let sym = eval(&mut it, "Symbol('s')");
        let a = store_handle(0, sym.clone());
        assert_eq!(store_handle(0, sym), a);
        // a distinct symbol with the same description is a different identity
        let other = store_handle(0, eval(&mut it, "Symbol('s')"));
        assert_ne!(other, a);
        thaw_tjs_release_handle(a);
        thaw_tjs_release_handle(a);
        thaw_tjs_release_handle(other);
    }

    #[test]
    fn same_object_in_another_package_keeps_its_own_lease() {
        let mut it = Interp::new();
        let v = eval(&mut it, "({k: 3})");
        let a = store_handle(0, v.clone());
        let b = store_handle(7, v);
        assert_ne!(a, b);
        assert!(matches!(slot(b as u64), Some((7, _))));
        thaw_tjs_release_handle(a);
        thaw_tjs_release_handle(b);
    }
}

#[cfg(test)]
mod namespace_tag_handle_tests {
    use super::*;
    use std::ffi::{CStr, CString};

    #[test]
    fn math_handle_is_not_plain_data_and_prints_as_a_tagged_object() {
        let math = thaw_tjs_get_global(CString::new("Math").unwrap().as_ptr());
        assert_ne!(math, 0, "Math is not defined");
        assert_eq!(thaw_tjs_handle_is_plain_data(math as u64), 0);
        let text = thaw_tjs_handle_to_console_string(math as u64);
        assert_eq!(
            unsafe { CStr::from_ptr(text) }.to_str().unwrap(),
            "Object [Math] {}"
        );
    }

    #[test]
    fn array_holding_a_non_plain_object_is_not_plain_data() {
        let mut it = Interp::new();
        let array = |it: &mut Interp, src: &str| match it.run(src).unwrap() {
            Value::Object(o) => o,
            other => panic!("not an object: {}", it.to_str(&other)),
        };
        let with_math = array(&mut it, "[Math]");
        assert!(!array_elements_are_plain(
            &mut it,
            &with_math,
            &mut Vec::new()
        ));
        let nested = array(&mut it, "[1, 'a', [2, { b: 3 }]]");
        assert!(array_elements_are_plain(&mut it, &nested, &mut Vec::new()));
    }
}

#[cfg(test)]
mod console_function_text_tests {
    use super::*;

    fn console(it: &mut Interp, src: &str) -> String {
        let v = it.run(src).unwrap();
        console_text(it, &v)
    }

    #[test]
    fn classes_and_functions_print_like_node() {
        let mut it = Interp::new();
        assert_eq!(
            console(&mut it, "(class A { static x = 1 })"),
            "[class A] { x: 1 }"
        );
        assert_eq!(console(&mut it, "Number"), "[Function: Number]");
        assert_eq!(console(&mut it, "String"), "[Function: String]");
        assert_eq!(
            console(&mut it, "(function () {})"),
            "[Function (anonymous)]"
        );
        assert_eq!(
            console(&mut it, "(class B extends (class A {}) {})"),
            "[class B extends A]"
        );
        assert_eq!(console(&mut it, "12n"), "12n");
    }
}

#[cfg(test)]
mod host_query_live_tests {
    use super::*;
    use std::ffi::{CStr, CString};

    /// `new <global>(...args)` as a handle; `args` is a JSON array of graph argument nodes.
    fn construct(global: &str, args: &str) -> u64 {
        let ctor = thaw_tjs_get_global(CString::new(global).unwrap().as_ptr());
        assert_ne!(ctor, 0, "{global} is not defined");
        let wire =
            CString::new(format!(r#"{{"root":{{"r":0}},"nodes":[{{"a":{args}}}]}}"#)).unwrap();
        let result = thaw_tjs_construct_handle_graph_args_result(ctor as u64, wire.as_ptr());
        assert!(result.error.is_null());
        result.value
    }

    fn query(handle: u64, operation: u8) -> String {
        let result = thaw_tjs_host_query_result(handle, operation);
        assert!(result.error.is_null());
        let text = unsafe { CStr::from_ptr(result.value) }
            .to_str()
            .unwrap()
            .to_owned();
        unsafe { thaw_arena::destroy_string(result.value) };
        text
    }

    #[test]
    fn host_query_keep_live_answers_one_for_typed_buffers_maps_and_sets() {
        for (global, args) in [
            ("Float64Array", "[{\"v\":2}]"),
            ("Uint16Array", "[{\"v\":2}]"),
            ("ArrayBuffer", "[{\"v\":8}]"),
            ("Map", "[]"),
            ("Set", "[]"),
        ] {
            assert_eq!(query(construct(global, args), 29), "1", "{global}");
        }
    }

    #[test]
    fn host_query_is_buffer_answers_one_only_for_buffers() {
        assert_eq!(query(construct("Buffer", "[{\"v\":3}]"), 11), "1");
        assert_eq!(query(construct("Uint8Array", "[{\"v\":3}]"), 11), "0");
        assert_eq!(query(construct("Float64Array", "[{\"v\":2}]"), 11), "0");
        assert_eq!(query(construct("Object", "[]"), 11), "0");
        assert_eq!(query(construct("Buffer", "[{\"v\":3}]"), 25), "0");
    }

    #[test]
    fn host_query_keep_live_answers_zero_for_copyable_values() {
        assert_eq!(query(construct("Uint8Array", "[{\"v\":2}]"), 29), "1");
        assert_eq!(query(construct("Object", "[]"), 29), "0");
        assert_eq!(query(construct("Array", "[]"), 29), "0");
        assert_eq!(query(construct("Date", "[{\"v\":0}]"), 29), "0");
        assert_eq!(query(thaw_tjs_new_function_handle(), 29), "0");
    }

    #[test]
    fn host_query_keep_live_splits_plain_uint8array_from_buffer() {
        assert_eq!(query(construct("Uint8Array", "[{\"v\":3}]"), 29), "1");
        assert_eq!(query(construct("Uint8Array", "[{\"v\":0}]"), 29), "1");
        assert_eq!(query(construct("Buffer", "[{\"v\":3}]"), 29), "0");
    }

    #[test]
    fn host_query_keep_live_answers_one_for_proxies() {
        // a native array / collection / class view is a Proxy, so it must cross as a live handle too
        assert_eq!(
            query(construct("Proxy", "[{\"v\":{}},{\"v\":{}}]"), 29),
            "1"
        );
    }

    #[test]
    fn host_query_is_array_looks_through_a_proxy_over_an_array() {
        let array = construct("Array", "[{\"v\":1},{\"v\":2}]");
        assert_eq!(query(array, 4), "1");
        let ctor = thaw_tjs_get_global(CString::new("Proxy").unwrap().as_ptr());
        let wire = CString::new(format!(
            r#"{{"root":{{"r":0}},"nodes":[{{"a":[{{"r":1}},{{"v":{{}}}}]}},{{"hdl":{array}}}]}}"#
        ))
        .unwrap();
        let result = thaw_tjs_construct_handle_graph_args_result(ctor as u64, wire.as_ptr());
        assert!(result.error.is_null());
        let proxy = result.value;
        assert_eq!(query(proxy, 4), "1");
        assert_eq!(query(construct("Proxy", "[{\"v\":{}},{\"v\":{}}]"), 4), "0");
    }

    #[test]
    fn host_query_plain_answers_one_for_plain_objects_and_arrays_only() {
        assert_eq!(query(construct("Object", "[]"), 30), "1");
        assert_eq!(query(construct("Array", "[]"), 30), "1");
        assert_eq!(query(construct("Map", "[]"), 30), "0");
        assert_eq!(query(construct("Float64Array", "[{\"v\":2}]"), 30), "0");
        assert_eq!(query(construct("Buffer", "[{\"v\":3}]"), 30), "0");
        assert_eq!(query(thaw_tjs_new_function_handle(), 30), "0");
        // operation 29 keeps its meaning: a plain object is still copied by it
        assert_eq!(query(construct("Object", "[]"), 29), "0");
    }

    #[test]
    fn host_query_keep_live_leaves_operation_eight_unchanged() {
        // op 8 still flattens a typed array for a real JSON.stringify
        assert_eq!(
            query(construct("Float64Array", "[{\"v\":2}]"), 8),
            r#"{"0":0,"1":0}"#
        );
        assert_eq!(query(construct("Map", "[]"), 8), "{}");
    }

    #[test]
    fn host_query_typed_probe_lists_typed_array_items_like_node() {
        // node 22: new Uint8Array([3,4]) and new Float64Array([1.5]); Symbol.toStringTag gives the name
        assert_eq!(
            query(construct_array("Uint8Array", &[3, 4]), 27),
            r#"{"n":"Uint8Array","v":["3","4"]}"#
        );
        assert_eq!(
            query(construct_array("Float64Array", &[1]), 27),
            r#"{"n":"Float64Array","v":["1"]}"#
        );
    }

    #[test]
    fn host_query_typed_probe_is_empty_for_non_typed_values() {
        // node 22: a Buffer and a DataView are not probed; a plain object, a Map and a RegExp are not typed arrays
        assert_eq!(query(construct("Buffer", "[{\"v\":3}]"), 27), "");
        assert_eq!(query(construct("Object", "[]"), 27), "");
        assert_eq!(query(construct("Map", "[]"), 27), "");
        assert_eq!(query(construct("RegExp", "[{\"v\":\"y\"}]"), 27), "");
    }

    #[test]
    fn host_query_keep_live_keeps_regexp_whole_for_structured_clone() {
        // node 22: structuredClone(/y/m).flags === "m"; a RegExp lease must stay whole, not an empty object
        assert_eq!(
            query(construct("RegExp", "[{\"v\":\"y\"},{\"v\":\"m\"}]"), 29),
            "1"
        );
    }

    #[test]
    fn host_query_error_properties_are_own_extra_properties_json() {
        // node 22: JSON.stringify(Object.assign(new Error("x"), {code: "C"})) is {"code":"C"}; message/name/stack excluded
        let error = construct("Error", "[{\"v\":\"x\"}]");
        let wire = CString::new(r#"{"root":{"v":"C"},"nodes":[]}"#).unwrap();
        let key = CString::new("\"code\"").unwrap();
        let result = thaw_tjs_set_property_graph_result(error, key.as_ptr(), wire.as_ptr());
        assert!(result.error.is_null());
        assert_eq!(query(error, 28), r#"{"code":"C"}"#);
        assert_eq!(query(construct("Error", "[{\"v\":\"x\"}]"), 28), "{}");
    }

    /// `new <global>([items])` as a handle: node 1 is the item array, node 0 the argument list holding it.
    fn construct_array(global: &str, items: &[i64]) -> u64 {
        let ctor = thaw_tjs_get_global(CString::new(global).unwrap().as_ptr());
        assert_ne!(ctor, 0, "{global} is not defined");
        let values = items
            .iter()
            .map(|n| format!(r#"{{"v":{n}}}"#))
            .collect::<Vec<_>>()
            .join(",");
        let wire = CString::new(format!(
            r#"{{"root":{{"r":0}},"nodes":[{{"a":[{{"r":1}}]}},{{"a":[{values}]}}]}}"#
        ))
        .unwrap();
        let result = thaw_tjs_construct_handle_graph_args_result(ctor as u64, wire.as_ptr());
        assert!(result.error.is_null());
        result.value
    }

    #[test]
    fn host_query_typed_probe_marks_bigint_items_like_node() {
        // node 22: new BigInt64Array([1n, -1n]) probes as BigInt items with the `n` suffix
        let mut it = Interp::new();
        let v = it.run("new BigInt64Array([1n, -1n])").unwrap();
        assert_eq!(
            typed_probe_text(&mut it, &v),
            r#"{"n":"BigInt64Array","v":["1n","-1n"]}"#
        );
    }

    #[test]
    fn host_query_lease_kinds_for_date_map_set_and_error_match_node_clone() {
        // node 22: structuredClone(new Date(0)) is a Date, Map and Set clone as collections, Error as an Error
        assert_eq!(query(construct("Date", "[{\"v\":0}]"), 12), "1");
        assert_eq!(query(construct("Map", "[]"), 24), "map");
        assert_eq!(query(construct("Set", "[]"), 24), "set");
        assert_eq!(query(construct("Error", "[{\"v\":\"x\"}]"), 22), "1");
        // a Date lease is non-plain, so structuredClone keeps it live (op 12 = "1")
        assert_eq!(query(construct("Date", "[{\"v\":0}]"), 29), "0");
    }

    fn thaw_tjs_new_function_handle() -> u64 {
        let args = CString::new(r#"["return 1"]"#).unwrap();
        let result = thaw_tjs_new_function(args.as_ptr());
        assert!(result.error.is_null());
        result.value
    }
}

#[cfg(test)]
mod class_symbol_thenable_tests {
    use super::*;
    use std::ffi::CStr;

    #[test]
    fn class_symbol_member_may_start_with_a_dollar() {
        assert_eq!(
            parse_class_symbol("$method$p::C$$disconnect$overload0$arity0"),
            Some(("method", "p", "C", Some("$disconnect")))
        );
        assert_eq!(
            parse_class_symbol("$staticmethodvoid$p::C$of$overload0$arity1"),
            Some(("staticmethodvoid", "p", "C", Some("of")))
        );
        assert_eq!(
            parse_class_symbol("$method$p::C$describe$overload0$arity0"),
            Some(("method", "p", "C", Some("describe")))
        );
        assert_eq!(
            parse_class_symbol("$new$p::C$arity0"),
            Some(("new", "p", "C", None))
        );
    }

    #[test]
    fn await_of_a_handle_to_a_thenable_runs_its_then() {
        let owner = default_realm();
        let handle = with_package(owner, |p| {
            let it = &mut p.interp;
            let v = it
                .run("({ then: function (resolve) { resolve(7); } })")
                .map_err(|c| classify(it, c))?;
            // the call-result path (`awaited`) still hands the object back untouched
            let same = awaited(it, v.clone())?;
            assert_eq!(object_key(&same), object_key(&v));
            Ok(store_handle(owner, v))
        })
        .unwrap();
        let result = thaw_tjs_resolve_handle_handle_result(handle as u64);
        assert!(result.error.is_null());
        let text = unsafe { CStr::from_ptr(thaw_tjs_handle_to_string(result.value)) }
            .to_str()
            .unwrap()
            .to_owned();
        assert_eq!(text, "7");
    }
}

#[cfg(test)]
mod handle_call_kind_tests {
    use super::*;

    #[test]
    fn handle_call_of_a_non_function_names_the_handle_and_its_kind() {
        let mut it = Interp::new();
        let value = it.run("({a: 1})").unwrap();
        match check_handle_callable(&mut it, &value) {
            Err(HostError::Thrown(msg)) => assert!(
                msg.contains("handle call: object is not a function (constructor Object"),
                "{msg}"
            ),
            other => panic!("expected a thrown TypeError, got {}", other.is_ok()),
        }
        let undefined = Value::Undefined;
        assert!(check_handle_callable(&mut it, &undefined).is_err());
    }
}

#[cfg(test)]
mod strict_host_set_tests {
    use super::*;

    #[test]
    fn host_set_on_a_frozen_object_throws_and_keeps_the_value() {
        let mut it = Interp::new();
        let obj = it.run("Object.freeze({value: 1})").unwrap();
        assert!(strict_host_set(&mut it, &obj, "value", Value::Number(99.0)).is_err());
        assert!(matches!(it.get(&obj, "value"), Ok(Value::Number(n)) if n == 1.0));
    }

    #[test]
    fn typeof_function_prototype_is_function() {
        let mut it = Interp::new();
        let kind = it.run("typeof Function.prototype").unwrap();
        assert_eq!(it.to_str(&kind).to_string(), "function");
    }
}

#[cfg(test)]
mod handle_value_graph_await_tests {
    use super::*;
    use std::ffi::CStr;

    /// `function (a) { <body> }` as a handle.
    fn function_handle(body: &str) -> u64 {
        let args = std::ffi::CString::new(format!(r#"["a","{body}"]"#)).unwrap();
        let result = thaw_tjs_new_function(args.as_ptr());
        assert!(result.error.is_null());
        result.value
    }

    #[test]
    fn call_handle_value_graph_result_awaits_a_returned_promise() {
        let f = function_handle("return Promise.resolve(5)");
        let argument = function_handle("return 1");
        let result = thaw_tjs_call_handle_value_graph_result(f, argument);
        assert!(result.error.is_null());
        let wire = unsafe { CStr::from_ptr(result.value) }
            .to_str()
            .unwrap()
            .to_owned();
        assert!(wire.contains(r#""root":{"v":5}"#), "{wire}");
    }

    #[test]
    fn call_handle_value_graph_result_reports_a_rejected_promise_as_an_error() {
        let f = function_handle("return Promise.reject(new Error('x'))");
        let argument = function_handle("return 1");
        let result = thaw_tjs_call_handle_value_graph_result(f, argument);
        assert!(!result.error.is_null());
    }
}

#[cfg(test)]
mod graph_bytes_node_tests {
    use super::*;

    #[test]
    fn graph_b_node_decodes_to_a_buffer_over_its_bytes() {
        let (_, v) = decode_wire(r#"{"root":{"r":0},"nodes":[{"b":[104,105]}]}"#).unwrap();
        assert_eq!(Interp::view_bytes(&v), Some(vec![104, 105]));
        assert!(Interp::typed_of(&v).is_some());
    }
}
