//! Package host: loads a registry bundle's CommonJS factories into an [`Interp`] with its own
//! `require` resolution, and exchanges plain values with compiled Thaw code.
//!
//! Boundary values ([`Native`]) mirror Thaw's native layouts for plain data (numbers, strings,
//! booleans, arrays, string-keyed objects). They are *copied* in and out here; the designed live
//! views (`Json` / native class instances, design.md section 2) are milestone M4.

use std::cell::RefCell;
use std::collections::HashMap;
use std::ffi::{c_char, c_void, CStr, CString};
use std::ptr;
use std::rc::Rc;

use crate::scan::{parse_bundle, BundleParts};
use crate::{Ctl, Interp, Kind, Value, R};

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
}

fn require_in(loader: &Rc<Loader>, it: &mut Interp, from: &str, spec: &str) -> R<Value> {
    let target = loader
        .parts
        .require_maps
        .get(from)
        .and_then(|m| m.get(spec))
        .cloned();
    match target {
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

/// A loaded package: one interpreter, the bundle's modules, and its entry exports.
pub struct Package {
    /// Boxed: async functions hold its address while suspended.
    interp: Box<Interp>,
    exports: Value,
}

impl Package {
    /// Parse `bundle_js` (registry `bundle.js`) and evaluate its entry module.
    pub fn load(bundle_js: &str) -> Result<Package, HostError> {
        let parts = parse_bundle(bundle_js).map_err(HostError::Bundle)?;
        if let Some(problem) = parts.esm_problems.iter().next() {
            return Err(HostError::Unsupported((*problem).to_string()));
        }
        let mut interp = Box::new(Interp::new());
        interp.run_id += 1; // factory spans come from this one parse
        interp
            .sources
            .insert(interp.run_id, std::rc::Rc::from(parts.source.as_str()));
        let entry = parts.entry.clone();
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
        });
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
        let r = self.call_or_get_value(path, args, export_fallback)?;
        from_value(&mut self.interp, &r, 0)
    }

    fn call_or_get_value(
        &mut self,
        path: &str,
        args: Option<&[Native]>,
        export_fallback: bool,
    ) -> Result<Value, HostError> {
        let args = args.map(|a| {
            a.iter()
                .map(|a| to_value(&mut self.interp, a))
                .collect::<Vec<_>>()
        });
        self.call_or_get_values(path, args.as_deref(), export_fallback)
    }

    fn call_or_get_values(
        &mut self,
        path: &str,
        args: Option<&[Value]>,
        export_fallback: bool,
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
                let r = it.call(&f, &this, args).map_err(|c| classify(it, c))?;
                // an async export: run the event loop and unwrap the settled promise
                if it.is_promise(&r) {
                    it.mark_handled(&r);
                    it.run_event_loop().map_err(|c| classify(it, c))?;
                    match it.promise_state(&r) {
                        Some(Ok(v)) => v,
                        Some(Err(e)) => return Err(classify(it, Ctl::Throw(e))),
                        None => return Err(HostError::Thrown("promise never settled".into())),
                    }
                } else {
                    r
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
        for part in &parts {
            value = it.get(&value, part).map_err(|c| classify(it, c))?;
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

fn classify(it: &mut Interp, c: Ctl) -> HostError {
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
        Ctl::Exit(code) => HostError::Thrown(format!("process.exit({code})")),
        Ctl::Return(_) | Ctl::Break(_) | Ctl::Continue(_) => {
            HostError::Thrown("stray completion".into())
        }
    }
}

const CALLBACK_KEY: &str = "__thaw_tjs_cb__";
const TIMER_KEY: &str = "__thaw_tjs_timer__";
const HANDLE_KEY: &str = "__thaw_tjs_handle__";
const UNDEFINED_KEY: &str = "$__thaw_napi_undefined$";

/// `(adapter, context)` of a compiled callback: `adapter(context, args_json) -> result_json`.
type Adapter = unsafe extern "C" fn(*const c_void, *const c_char) -> *mut c_char;

struct CompiledCallback {
    adapter: usize,
    context: usize,
    params: usize,
    /// Keeps the closure environment alive for as long as JS may call it.
    _root: thaw_arena::ArenaRoot,
}

thread_local! {
    static CALLBACKS: RefCell<Vec<CompiledCallback>> = const { RefCell::new(Vec::new()) };
    /// Function values already made for (interpreter address, callback id): stable identity.
    static CALLBACK_VALUES: RefCell<HashMap<(usize, usize), Value>> = RefCell::new(HashMap::new());
}

thread_local! {
    /// Values compiled code holds by handle (class instances): `(package handle, value)`; id = index + 1.
    /// A released handle keeps its slot (ids are never reused, so a stale id is `undefined`).
    static HANDLES: RefCell<Vec<(usize, Value)>> = const { RefCell::new(Vec::new()) };
    /// Outstanding references per handle id (index + 1): compiled code retains and releases handles
    /// the way it does QuickJS ones.
    static HANDLE_REFS: RefCell<Vec<u32>> = const { RefCell::new(Vec::new()) };
}

thread_local! {
    /// The package last entered by `with_package`.
    static LAST_PACKAGE: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

fn store_handle(package: usize, value: Value) -> i64 {
    HANDLE_REFS.with(|r| r.borrow_mut().push(1));
    HANDLES.with(|h| {
        let mut h = h.borrow_mut();
        h.push((package, value));
        h.len() as i64
    })
}

/// Take another reference on a handle; returns 1 when it is live.
#[no_mangle]
pub extern "C" fn thaw_tjs_retain_handle(handle: i64) -> u8 {
    HANDLE_REFS.with(
        |r| match r.borrow_mut().get_mut((handle as usize).wrapping_sub(1)) {
            Some(refs) if *refs > 0 => {
                *refs += 1;
                1
            }
            _ => 0,
        },
    )
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
            // Take the value out before dropping it: its drop may release other handles.
            let value = HANDLES.with(|h| {
                h.borrow_mut()
                    .get_mut(index)
                    .map(|e| std::mem::replace(&mut e.1, Value::Undefined))
            });
            drop(value);
            1
        }
        Some(false) => 1,
        None => 0,
    }
}

/// What a class operation does (the symbol prefix compiled code passes).
enum ClassOp<'a> {
    New { class: &'a str },
    Method { receiver: i64, name: &'a str },
    Static { class: &'a str, name: &'a str },
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
        _ => Some(rest.split('$').next()?),
    };
    Some((kind, package, class_path, member))
}

fn class_call(
    symbol: &str,
    args_json: &str,
    receiver: i64,
    want_handle: bool,
) -> Result<String, HostError> {
    let (kind, package, class_path, member) = parse_class_symbol(symbol)
        .ok_or_else(|| HostError::Bundle(format!("malformed class symbol `{symbol}`")))?;
    let op = match kind {
        "new" => ClassOp::New { class: class_path },
        "method" | "methodvoid" => ClassOp::Method {
            receiver,
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
        ClassOp::Method { receiver, .. } => HANDLES
            .with(|h| {
                h.borrow()
                    .get((*receiver as usize).wrapping_sub(1))
                    .map(|(p, _)| *p)
            })
            .ok_or_else(|| HostError::Bundle("invalid class instance handle".into()))?,
        _ => BY_NAME
            .with(|m| m.borrow().get(package).copied())
            .ok_or_else(|| {
                HostError::Bundle(format!("thaw-js package `{package}` is not loaded"))
            })?,
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
            ClassOp::Method { receiver, name } => {
                let this = HANDLES
                    .with(|h| {
                        h.borrow()
                            .get((receiver as usize).wrapping_sub(1))
                            .map(|(_, v)| v.clone())
                    })
                    .unwrap_or(Value::Undefined);
                let it = &mut p.interp;
                let f = it.get(&this, name).map_err(|c| classify(it, c))?;
                it.call(&f, &this, &args).map_err(|c| classify(it, c))?
            }
            ClassOp::Static { class, name } => {
                let ctor = p.exports_member(class)?;
                let it = &mut p.interp;
                let f = it.get(&ctor, name).map_err(|c| classify(it, c))?;
                it.call(&f, &ctor, &args).map_err(|c| classify(it, c))?
            }
        };
        let it = &mut p.interp;
        if want_handle {
            return Ok(store_handle(package_handle, result).to_string());
        }
        from_value(it, &result, 0).map(|n| n.to_json())
    })
}

/// Run a class operation (see `parse_class_symbol`): construct (`receiver` 0), call a method on an
/// instance handle, or call a static method. The result is JSON text, or a decimal handle id when
/// `want_handle` is set (the class instance stays in the thaw-js heap).
#[no_mangle]
pub extern "C" fn thaw_tjs_class_call(
    symbol: *const c_char,
    args_json: *const c_char,
    receiver: i64,
    want_handle: u8,
) -> ThawResult {
    text_result((|| {
        let (Some(symbol), Some(args)) = (cstr(symbol), cstr(args_json)) else {
            return Err("invalid call text".to_string());
        };
        if let Some(message) = symbol.strip_prefix('\u{1}') {
            return Err(message.to_string());
        }
        class_call(&symbol, &args, receiver, want_handle != 0).map_err(|e| e.to_string())
    })())
}

fn graph_field<'a>(fields: &'a [(String, Native)], key: &str) -> Option<&'a Native> {
    fields.iter().find(|(k, _)| k == key).map(|(_, v)| v)
}

/// One graph-wire token (`{"v":..}`, `{"r":i}`, `{"nf":..}`, `{"u":1}`) as a value.
fn graph_token(
    it: &mut Interp,
    token: &Native,
    nodes: &[Native],
    memo: &mut Vec<Option<Value>>,
    busy: &mut Vec<bool>,
) -> Result<Value, String> {
    let Native::Object(fields) = token else {
        return Err("malformed graph token".into());
    };
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
        ("r", Native::Number(i)) => graph_node(it, *i as usize, nodes, memo, busy),
        _ => Err(format!("unsupported graph token `{key}`")),
    }
}

/// Node `index` of a graph wire; shared nodes decode to one value (identity kept).
fn graph_node(
    it: &mut Interp,
    index: usize,
    nodes: &[Native],
    memo: &mut Vec<Option<Value>>,
    busy: &mut Vec<bool>,
) -> Result<Value, String> {
    if let Some(Some(v)) = memo.get(index) {
        return Ok(v.clone());
    }
    let node = nodes.get(index).ok_or("graph node out of range")?;
    // ponytail: cyclic graphs are rejected (containers are built whole); fill-after-create if a package needs them.
    if std::mem::replace(&mut busy[index], true) {
        return Err("cyclic graph is not supported".into());
    }
    let Native::Object(fields) = node else {
        return Err("malformed graph node".into());
    };
    let value = match fields.as_slice() {
        [(k, Native::Array(items))] if k == "a" => {
            let mut out = Vec::with_capacity(items.len());
            for item in items {
                out.push(match item {
                    Native::Object(f) if graph_field(f, "h").is_some() => Value::Hole,
                    _ => graph_token(it, item, nodes, memo, busy)?,
                });
            }
            it.array(out)
        }
        [(k, Native::Array(pairs))] if k == "o" => {
            // The one-key markers compiled code puts in a `Json` (a live thaw-js value, a compiled callback,
            // a timer, `undefined`), as `to_value` reads them from plain data.
            if let [Native::Array(kv)] = pairs.as_slice() {
                if let [Native::Str(name), Native::Object(v), ..] = kv.as_slice() {
                    let found = match (name.as_str(), graph_field(v, "v")) {
                        (HANDLE_KEY, Some(Native::Number(id))) => HANDLES.with(|h| {
                            h.borrow()
                                .get((*id as usize).wrapping_sub(1))
                                .map(|(_, v)| v.clone())
                        }),
                        (CALLBACK_KEY, Some(Native::Number(id))) => {
                            Some(callback_value(it, *id as usize))
                        }
                        (TIMER_KEY, Some(Native::Number(id))) => Some(it.timer_object(*id as u64)),
                        (UNDEFINED_KEY, Some(Native::Bool(true))) => Some(Value::Undefined),
                        _ => None,
                    };
                    if let Some(found) = found {
                        memo[index] = Some(found.clone());
                        return Ok(found);
                    }
                }
            }
            let o = it.plain();
            for pair in pairs {
                let Native::Array(kv) = pair else {
                    return Err("malformed graph property".into());
                };
                let [Native::Str(name), token, ..] = kv.as_slice() else {
                    return Err("malformed graph property".into());
                };
                let v = graph_token(it, token, nodes, memo, busy)?;
                crate::put(&o, name, v);
            }
            Value::Object(o)
        }
        [(k, Native::Number(id))] if k == "hdl" => {
            let v = HANDLES
                .with(|h| {
                    h.borrow()
                        .get((*id as usize).wrapping_sub(1))
                        .map(|(_, v)| v.clone())
                })
                .unwrap_or(Value::Undefined);
            // The encoder retained this reference for the wire; the decoded value holds its own.
            thaw_tjs_release_handle(*id as i64);
            v
        }
        _ => return Err("unsupported graph node (Date, Buffer, Map, Set, RegExp, BigInt)".into()),
    };
    memo[index] = Some(value.clone());
    Ok(value)
}

#[repr(C)]
pub struct HandleResult {
    pub value: u64,
    pub error: *mut c_char,
}

/// Decode a graph wire (`thaw_json_graph_encode`) into `(owning package, value)`. The package is the one
/// that owns a handle the wire references, else the package last entered (a value only needs an
/// interpreter to be built in and a package for later method calls on it).
fn decode_wire(wire: &str) -> Result<(usize, Value), String> {
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
    let owner = nodes
        .iter()
        .find_map(|n| match n {
            Native::Object(f) => match graph_field(f, "hdl") {
                Some(Native::Number(id)) => {
                    HANDLES.with(|h| h.borrow().get((*id as usize).wrapping_sub(1)).map(|e| e.0))
                }
                _ => None,
            },
            _ => None,
        })
        .unwrap_or_else(|| LAST_PACKAGE.with(|l| l.get()));
    with_package(owner, |p| {
        let mut memo = vec![None; nodes.len()];
        let mut busy = vec![false; nodes.len()];
        graph_token(&mut p.interp, root, nodes, &mut memo, &mut busy).map_err(HostError::Bundle)
    })
    .map(|v| (owner, v))
    .map_err(|e| e.to_string())
}

fn handle_result(result: Result<i64, String>) -> HandleResult {
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
fn error_text(e: HostError) -> String {
    let display = e.to_string();
    LAST_FRAME.with(|f| match f.borrow_mut().take() {
        Some((d, framed)) if d == display => framed,
        _ => display,
    })
}

/// Run `f(interpreter, package, value)` on the value behind `handle`, in the package that owns it.
fn on_handle<T>(
    handle: u64,
    f: impl FnOnce(&mut Package, usize, Value) -> Result<T, HostError>,
) -> Result<T, String> {
    let (owner, value) = HANDLES
        .with(|h| {
            h.borrow()
                .get((handle as usize).wrapping_sub(1))
                .map(|(p, v)| (*p, v.clone()))
        })
        .ok_or("invalid JavaScript handle")?;
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

/// Thrown carrier errors of native packets do not exist in thaw-js: always 0 ("no native token").
#[no_mangle]
pub extern "C" fn thaw_tjs_exception_native_token(_handle: u64) -> u64 {
    0
}

/// Graph-wire builder for a result value. `plain` copies arrays and objects deeply (functions and
/// symbols become `undefined`, `toJSON` is honored) like the QuickJS result lane; `receiver` is the
/// value that stays a live handle wherever it recurs.
struct WireOut {
    owner: usize,
    plain: bool,
    /// Plain mode only: functions stay live handles instead of becoming `undefined`.
    live_functions: bool,
    /// Not plain mode: the root container is copied, only what it holds stays live.
    copy_root: bool,
    receiver: Option<(u64, Value)>,
    nodes: Vec<String>,
    seen: Vec<(*const crate::ObjData, usize)>,
    leases: Vec<u64>,
}

impl WireOut {
    /// A host-handle node for `id` (the wire owns one reference, released by the decoder); its token.
    fn lease(&mut self, id: u64) -> String {
        self.leases.push(id);
        self.nodes.push(format!("{{\"hdl\":{id}}}"));
        format!("{{\"r\":{}}}", self.nodes.len() - 1)
    }

    fn token(&mut self, it: &mut Interp, v: &Value) -> Result<String, HostError> {
        let mut v = v.clone();
        if let Some((id, r)) = &self.receiver {
            if matches!((&v, r), (Value::Object(a), Value::Object(b)) if Rc::ptr_eq(a, b)) {
                let id = *id;
                thaw_tjs_retain_handle(id as i64);
                return Ok(self.lease(id));
            }
        }
        if self.plain && matches!(v, Value::Object(_)) && !v.is_callable() {
            let to_json = it.get(&v, "toJSON").map_err(|c| classify(it, c))?;
            if to_json.is_callable() {
                v = it
                    .call(&to_json, &v, &[Value::str("")])
                    .map_err(|c| classify(it, c))?;
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

    fn finish(self, root: String) -> String {
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

fn graph_result(r: Result<String, String>) -> ThawResult {
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

/// Call the already-selected method `selected` on `receiver` with the graph-wire argument array; the
/// result is a plain graph wire, the receiver itself staying a live handle.
#[no_mangle]
pub extern "C" fn thaw_tjs_call_selected_method_graph_result(
    receiver: u64,
    selected: u64,
    _name: *const c_char,
    args_json: *const c_char,
) -> ThawResult {
    graph_result((|| {
        let args_json = cstr(args_json).ok_or("invalid arguments")?;
        let (_, args) = decode_wire(&args_json)?;
        let method = HANDLES
            .with(|h| {
                h.borrow()
                    .get((selected as usize).wrapping_sub(1))
                    .map(|(_, v)| v.clone())
            })
            .ok_or("invalid JavaScript method")?;
        on_handle(receiver, |p, owner, this| {
            let it = &mut p.interp;
            let args = it.array_items(&args);
            let result = it
                .call(&method, &this, &args)
                .map_err(|c| classify(it, c))?;
            let mut out = WireOut {
                owner,
                plain: true,
                live_functions: false,
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
/// Introspection of the value behind `handle` for the Json host bridge (the numbering is the QuickJS
/// `__thaw_json_host_query`'s). Kinds thaw-js has no value for (Date, Buffer, Map, Set, native
/// wrappers) answer "no".
#[no_mangle]
pub extern "C" fn thaw_tjs_host_query_result(handle: u64, operation: u8) -> ThawResult {
    text_result(on_handle(handle, |p, _, v| {
        let it = &mut p.interp;
        let flag = |b: bool| if b { "1" } else { "0" }.to_string();
        let is_array =
            matches!(&v, Value::Object(o) if matches!(&*o.kind.borrow(), Kind::Array(_)));
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
            26 => flag(matches!(&v, Value::Object(o) if o.proto.borrow().is_none())),
            11 | 12 | 17..=20 | 22 | 25 => "0".to_string(),
            21 | 23 | 24 | 27 => String::new(),
            28 => "{}".to_string(),
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
    graph_result(on_handle(handle, |p, owner, v| {
        let it = &mut p.interp;
        let keys = it.own_keys(&v);
        let mut items = Vec::new();
        for k in keys {
            let value = it.get(&v, &k).map_err(|c| classify(it, c))?;
            items.push(match operation {
                0 => it.array(vec![Value::Str(k), value]),
                1 | 2 => Value::Str(k),
                3 => value,
                _ => {
                    return Err(HostError::Thrown(format!(
                        "TypeError: thaw-js does not support the host enumeration {operation}"
                    )))
                }
            });
        }
        let list = it.array(items);
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

#[no_mangle]
pub extern "C" fn thaw_tjs_set_property_graph_result(
    _handle: u64,
    _key_json: *const c_char,
    _graph_json: *const c_char,
) -> HandleResult {
    handle_result(Err(
        "thaw-js does not support the live host operation `set`".into(),
    ))
}

#[no_mangle]
pub extern "C" fn thaw_tjs_property_predicate_json_key_result(
    _handle: u64,
    _key_json: *const c_char,
    _operation: u8,
) -> HandleResult {
    handle_result(Err(
        "thaw-js does not support the live host operation `predicate`".into(),
    ))
}

/// A genuine `Error` object from the compiler's framed error text, as a new handle.
#[no_mangle]
pub extern "C" fn thaw_tjs_error_from_tagged_handle(value: *const c_char) -> HandleResult {
    handle_result((|| {
        let text = cstr(value).ok_or("null framed Error text")?;
        let owner = LAST_PACKAGE.with(|l| l.get());
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

/// No native-object identity cache in thaw-js: always a miss.
#[no_mangle]
pub extern "C" fn thaw_tjs_lookup_native_object(
    _pointer: *const c_void,
    _layout: *const c_char,
) -> HandleResult {
    handle_result(Ok(0))
}

/// Without an identity cache the candidate is canonical (ownership passes back to the caller).
#[no_mangle]
pub extern "C" fn thaw_tjs_intern_native_object(
    _pointer: *const c_void,
    _layout: *const c_char,
    candidate_handle: u64,
) -> HandleResult {
    handle_result(Ok(candidate_handle as i64))
}

/// As [`thaw_tjs_call_selected_method_graph_result`], the result kept as a new handle.
#[no_mangle]
pub extern "C" fn thaw_tjs_call_selected_method_handle_graph_args_result(
    receiver: u64,
    selected: u64,
    _name: *const c_char,
    args_json: *const c_char,
    _defer_resolution: bool,
) -> HandleResult {
    // ponytail: a returned Promise is not awaited here (QuickJS resolves it when not deferred)
    handle_result((|| {
        let args_json = cstr(args_json).ok_or("invalid arguments")?;
        let (_, args) = decode_wire(&args_json)?;
        let method = HANDLES
            .with(|h| {
                h.borrow()
                    .get((selected as usize).wrapping_sub(1))
                    .map(|(_, v)| v.clone())
            })
            .ok_or("invalid JavaScript method")?;
        on_handle(receiver, |p, owner, this| {
            let it = &mut p.interp;
            let args = it.array_items(&args);
            let result = it
                .call(&method, &this, &args)
                .map_err(|c| classify(it, c))?;
            Ok(store_handle(owner, result))
        })
    })())
}

/// The value behind `handle` as a plain graph wire (a JSON-like copy).
#[no_mangle]
pub extern "C" fn thaw_tjs_resolve_handle_graph_result(handle: u64) -> ThawResult {
    graph_result(on_handle(handle, |p, owner, v| {
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

fn handle_marker_json(id: i64) -> String {
    format!("{{\"{HANDLE_KEY}\":{id}}}")
}

fn handle_text(handle: u64, bigint_suffix: bool) -> *const c_char {
    let text = on_handle(handle, |p, _, v| {
        let it = &mut p.interp;
        let text = it.to_str(&v).to_string();
        Ok(if bigint_suffix && matches!(v, Value::BigInt(_)) {
            format!("{text}n")
        } else {
            text
        })
    })
    .unwrap_or_else(|_| "[invalid JsValue]".to_string());
    thaw_arena::owned_string(text)
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

/// 1 for a plain array or plain object (`console.log` inspects those like a native `Json`).
#[no_mangle]
pub extern "C" fn thaw_tjs_handle_is_plain_data(handle: u64) -> u8 {
    on_handle(handle, |p, _, v| {
        Ok(match &v {
            Value::Object(o) if matches!(&*o.kind.borrow(), Kind::Array(_)) => 1,
            Value::Object(o) if matches!(&*o.kind.borrow(), Kind::Plain) => u8::from(
                o.proto
                    .borrow()
                    .as_ref()
                    .is_none_or(|q| Rc::ptr_eq(q, &p.interp.object_proto)),
            ),
            _ => 0,
        })
    })
    .unwrap_or(0)
}

/// Typed arrays print through `String(value)` here: no probe text.
#[no_mangle]
pub extern "C" fn thaw_tjs_handle_typed_array_probe(_handle: u64) -> *const c_char {
    ptr::null()
}

/// Register a compiled closure callable from JS; returns its id. The same closure registered twice
/// keeps one id, so the function identity JS sees is stable.
#[no_mangle]
pub extern "C" fn thaw_tjs_register_callback(
    adapter: *const c_void,
    context: *const c_void,
    params: i64,
) -> i64 {
    CALLBACKS.with(|callbacks| {
        let mut callbacks = callbacks.borrow_mut();
        let (adapter, context) = (adapter as usize, context as usize);
        if let Some(id) = callbacks.iter().position(|c| {
            c.adapter == adapter && c.context == context && c.params == params as usize
        }) {
            return id as i64;
        }
        callbacks.push(CompiledCallback {
            adapter,
            context,
            params: params as usize,
            _root: thaw_arena::ArenaRoot::new(context),
        });
        callbacks.len() as i64 - 1
    })
}

fn callback_value(it: &mut Interp, id: usize) -> Value {
    let key = (it as *const Interp as usize, id);
    if let Some(v) = CALLBACK_VALUES.with(|m| m.borrow().get(&key).cloned()) {
        return v;
    }
    let function = it.native(move |it, _this, args| call_compiled(it, id, args));
    CALLBACK_VALUES.with(|m| m.borrow_mut().insert(key, function.clone()));
    function
}

/// Rebuild a thrown error from the compiled side's `\u{1}Name\u{1}message` framing.
fn tagged_error(it: &mut Interp, tagged: &str) -> Ctl {
    let tagged = tagged.strip_prefix('\u{2}').unwrap_or(tagged);
    let (name, message) = match tagged
        .strip_prefix('\u{1}')
        .and_then(|t| t.split_once('\u{1}'))
    {
        Some((name, message)) => (name, message.split('\u{5}').next().unwrap_or("")),
        None => ("Error", tagged),
    };
    let global = it.global.clone();
    let ctor = it
        .lookup(&global, name)
        .filter(|c| c.is_callable())
        .or_else(|| it.lookup(&global, "Error"))
        .unwrap_or(Value::Undefined);
    match it.construct(&ctor, &[Value::str(message)]) {
        Ok(err) => Ctl::Throw(err),
        Err(c) => c,
    }
}

fn call_compiled(it: &mut Interp, id: usize, args: &[Value]) -> R<Value> {
    let Some((adapter, context, params)) =
        CALLBACKS.with(|c| c.borrow().get(id).map(|c| (c.adapter, c.context, c.params)))
    else {
        return it.throw_error("Error", "unknown compiled callback");
    };
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
    let result = unsafe { adapter(context as *const c_void, text.as_ptr()) };
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
fn frame_error(it: &mut Interp, v: &Value) -> Option<(String, String)> {
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
    let mut framed = format!("\u{1}{name}\u{1}{message}");
    if let Ok(Value::Str(code)) = it.get(v, "code") {
        framed.push('\u{3}');
        framed.push_str(&code);
    }
    let mut props = Vec::new();
    for key in it.own_keys(v) {
        if matches!(&*key, "name" | "message" | "stack") {
            continue;
        }
        let Ok(value) = it.get(v, &key) else { continue };
        if value.is_callable() {
            continue;
        }
        if let Ok(native) = from_value(it, &value, 0) {
            props.push((key.to_string(), native));
        }
    }
    if !props.is_empty() {
        framed.push('\u{5}');
        framed.push_str(&Native::Object(props).to_json());
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
        Native::Str(s) => Value::str(s),
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
            HANDLES
                .with(|h| {
                    h.borrow()
                        .get((*id as usize).wrapping_sub(1))
                        .map(|(_, v)| v.clone())
                })
                .unwrap_or(Value::Undefined)
        }
        // `{"__thaw_tjs_timer__": id}`: a Timeout/Immediate returned earlier (still live, else its id).
        Native::Object(fields) if matches!(fields.as_slice(), [(k, Native::Number(_))] if k == TIMER_KEY) =>
        {
            let Some((_, Native::Number(id))) = fields.first() else {
                return Value::Undefined;
            };
            it.timer_object(*id as u64)
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
fn with_package<T>(
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

fn cstr(p: *const c_char) -> Option<String> {
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
    PACKAGES.with(|ps| {
        if let Some(slot) = ps.borrow_mut().get_mut(handle as usize) {
            *slot = None;
        }
    });
}

/// Terminal event loop for a routed build that holds no QuickJS context: runs every loaded
/// package's timers/immediates/jobs until idle. Returns 1 if a package ended with an uncaught
/// error. (The codegen renames `thaw_js_run_event_loop` to this symbol for such builds.)
#[no_mangle]
pub extern "C" fn thaw_tjs_run_event_loop() -> i32 {
    let count = PACKAGES.with(|ps| ps.borrow().len());
    let mut status = 0;
    for handle in 0..count {
        let ran = with_package(handle, |p| {
            if !p.interp.work_pending() {
                return Ok(0);
            }
            p.interp
                .run_event_loop()
                .map(|_| 0)
                .map_err(|c| classify(&mut p.interp, c))
        });
        if let Err(e) = ran {
            eprintln!("{e}");
            status = 1;
        }
    }
    status
}

/// 1 while any loaded package still has referenced timers/immediates or queued jobs.
#[no_mangle]
pub extern "C" fn thaw_tjs_terminal_work_pending() -> u8 {
    let count = PACKAGES.with(|ps| ps.borrow().len());
    (0..count).any(|h| {
        PACKAGES.with(|ps| {
            ps.borrow()
                .get(h)
                .is_some_and(|p| p.as_ref().is_some_and(|p| p.interp.work_pending()))
        })
    }) as u8
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
    match Package::load(&src) {
        Ok(p) => {
            PACKAGES.with(|ps| {
                let mut ps = ps.borrow_mut();
                ps.push(Some(p));
                BY_NAME.with(|m| m.borrow_mut().insert(name, ps.len() - 1));
            });
            1
        }
        Err(e) => {
            LAST_ERROR.with(|l| *l.borrow_mut() = e.to_string());
            0
        }
    }
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
        let (package, export) = symbol.split_once("::").ok_or("malformed thaw-js symbol")?;
        let handle = BY_NAME
            .with(|m| m.borrow().get(package).copied())
            .ok_or_else(|| format!("thaw-js package `{package}` is not loaded"))?;
        let Native::Array(args) = Native::from_json(&args).map_err(|e| e.to_string())? else {
            return Err("arguments must be a JSON array".into());
        };
        with_package(handle, |p| {
            let r = p.call_or_get_value(export, Some(&args), true)?;
            if is_live_value(&p.interp, &r) {
                return Ok(handle_marker_json(store_handle(handle, r)));
            }
            from_value(&mut p.interp, &r, 0).map(|n| n.to_json())
        })
        .map_err(|e| e.to_string())
    })())
}
