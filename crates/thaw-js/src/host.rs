//! Package host: loads a registry bundle's CommonJS factories into an [`Interp`] with its own
//! `require` resolution, and exchanges plain values with compiled Thaw code.
//!
//! Boundary values ([`Native`]) mirror Thaw's native layouts for plain data (numbers, strings,
//! booleans, arrays, string-keyed objects). They are *copied* in and out here; the designed live
//! views (`Json` / native class instances, design.md section 2) are milestone M4.

use std::cell::RefCell;
use std::collections::HashMap;
use std::ffi::{c_char, CStr, CString};
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
    interp: Interp,
    exports: Value,
}

impl Package {
    /// Parse `bundle_js` (registry `bundle.js`) and evaluate its entry module.
    pub fn load(bundle_js: &str) -> Result<Package, HostError> {
        let parts = parse_bundle(bundle_js).map_err(HostError::Bundle)?;
        let mut interp = Interp::new();
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
        self.call_or_get(path, Some(args))
    }

    /// Read the value at `path` (`"VERSION"`, `"a.b"`) without calling it.
    pub fn get(&mut self, path: &str) -> Result<Native, HostError> {
        self.call_or_get(path, None)
    }

    fn call_or_get(&mut self, path: &str, args: Option<&[Native]>) -> Result<Native, HostError> {
        let it = &mut self.interp;
        let mut this = self.exports.clone();
        let mut f = this.clone();
        for part in path.split('.').filter(|p| !p.is_empty()) {
            this = f.clone();
            f = it.get(&this, part).map_err(|c| classify(it, c))?;
        }
        let r = match args {
            Some(args) => {
                let args: Vec<Value> = args.iter().map(|a| to_value(it, a)).collect();
                it.call(&f, &this, &args).map_err(|c| classify(it, c))?
            }
            None => f,
        };
        from_value(it, &r, 0)
    }

    pub fn interp(&mut self) -> &mut Interp {
        &mut self.interp
    }
}

fn classify(it: &mut Interp, c: Ctl) -> HostError {
    match c {
        Ctl::Unsupported(f) => HostError::Unsupported(f),
        Ctl::Throw(v) => {
            let text = it.to_str(&v);
            let name = it.get(&v, "name").ok().map(|n| it.to_str(&n).to_string());
            HostError::Thrown(match (name, matches!(v, Value::Object(_))) {
                (Some(n), true) if !text.starts_with(&n) => format!("{n}: {text}"),
                _ => text.to_string(),
            })
        }
        Ctl::Return(_) | Ctl::Break(_) | Ctl::Continue(_) => {
            HostError::Thrown("stray completion".into())
        }
    }
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
        PACKAGES.with(|ps| {
            match ps
                .borrow_mut()
                .get_mut(handle as usize)
                .and_then(Option::as_mut)
            {
                Some(p) => p.call(&path, &args),
                None => Err(HostError::Bundle("bad handle".into())),
            }
        })
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
