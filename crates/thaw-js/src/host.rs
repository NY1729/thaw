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
            require,
            Value::Undefined,
            file,
            Value::str("/"),
            Value::Undefined,
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
        let mut interp = Box::new(Interp::new());
        interp.run_id += 1; // factory spans come from this one parse
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
                let args: Vec<Value> = args.iter().map(|a| to_value(it, a)).collect();
                let r = it.call(&f, &this, &args).map_err(|c| classify(it, c))?;
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
        from_value(it, &r, 0)
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
        Value::Undefined => Native::Undefined,
        Value::Null => Native::Null,
        Value::Bool(b) => Native::Bool(*b),
        Value::Number(n) => Native::Number(*n),
        Value::Str(s) => Native::Str(s.to_string()),
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

    /// Parse JSON text through the evaluator's own `JSON.parse`.
    pub fn from_json(text: &str) -> Result<Native, HostError> {
        let mut it = Interp::new();
        let mut quoted = String::new();
        crate::builtins::json_quote(text, &mut quoted);
        match it.run(&format!("JSON.parse({quoted})")) {
            Ok(v) => from_value(&mut it, &v, 0),
            Err(c) => Err(classify(&mut it, c)),
        }
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
        with_package(handle, |p| p.call_export(export, &args))
            .map(|v| v.to_json())
            .map_err(|e| e.to_string())
    })())
}
