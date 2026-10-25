//! EXPERIMENTAL: prototype of milestones M0/M1 of the Thaw-native JavaScript runtime
//! (design: `/home/fedora/work/thaw-tmp/designs/js-runtime/design.md`).
//!
//! A tree-walking evaluator over the SWC AST produced by `thaw-parser`, for an ES5-plus
//! subset (functions/closures, objects with accessors, arrays, prototypes, `new`, exceptions,
//! loops, destructuring, regular expressions via `regress`, `arguments`, a good part of the
//! ES5 builtins) and a CommonJS loader for registry bundles ([`host::Package`]). Anything
//! outside the subset yields [`Ctl::Unsupported`] with the feature name that
//! [`scan`] also reports, so routing can be decided before running. Nothing here is wired
//! into the compiler by default.
//!
//! Simplifications (each is a known ceiling, upgrade path in the design doc):
//! * strings are `Rc<str>` (UTF-8, indices count chars), not Thaw's arena WTF-8 strings;
//! * properties are an insertion-ordered `Vec` with linear lookup (no shapes/atoms/ICs);
//! * scopes are `HashMap` chains (no slot resolution), no TDZ;
//! * integer-keyed object properties are not ordered first; arrays have no holes.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use classes::ClassInfo;
use swc_ecma_visit::{Visit, VisitWith};
use thaw_parser::ast as a;

mod buffer;
mod builtins;
mod classes;
mod collections;
mod date;
mod eventloop;
mod extras;
pub mod host;
pub mod scan;
mod typed;
mod url;

/// Feature names (as reported by [`scan`] and [`Ctl::Unsupported`]) the evaluator runs.
pub const SUPPORTED_FEATURES: &[&str] = &[
    "regexp",
    "destructuring",
    "arguments",
    "accessor",
    "object-spread",
    "class",
    "super",
    "optional-chaining",
    "async",
    "await",
    "tagged-template",
    "static-block",
];

const MAX_DEPTH: usize = 100;

#[derive(Clone)]
pub enum Value {
    Undefined,
    Null,
    Bool(bool),
    Number(f64),
    Str(Rc<str>),
    Object(Obj),
}

pub type Obj = Rc<ObjData>;

pub struct ObjData {
    props: RefCell<Vec<Prop>>,
    proto: RefCell<Option<Obj>>,
    kind: RefCell<Kind>,
    frozen: Cell<bool>,
}

#[derive(Clone)]
struct Prop {
    key: Rc<str>,
    slot: Slot,
    enumerable: bool,
    writable: bool,
}

#[derive(Clone)]
enum Slot {
    Data(Value),
    Accessor {
        get: Option<Value>,
        set: Option<Value>,
    },
}

enum Kind {
    Plain,
    Array(Vec<Value>),
    Closure {
        def: Rc<FuncDef>,
        scope: Rc<Scope>,
    },
    Native(NativeFn),
    RegExp(Rc<regress::Regex>),
    /// The global object: property access is routed to the global scope's bindings.
    Global,
    Map(Vec<(Value, Value)>),
    Set(Vec<Value>),
    Symbol {
        id: u64,
        desc: Option<Rc<str>>,
    },
    Promise(eventloop::PromiseData),
    ArrayBuffer(typed::Bytes),
    Typed(typed::TypedArr),
}

type NativeFn = Rc<dyn Fn(&mut Interp, &Value, &[Value]) -> R<Value>>;

struct FuncDef {
    name: Option<Rc<str>>,
    params: Vec<a::Pat>,
    body: Body,
    arrow: bool,
    is_async: bool,
    /// Named function expressions see their own name; methods do not.
    bind_self: bool,
    uses_arguments: bool,
    strict: bool,
    class: Option<ClassInfo>,
}

enum Body {
    Block(Vec<a::Stmt>),
    Expr(Box<a::Expr>),
}

/// Non-local completion, doubling as the error type of evaluation.
pub enum Ctl {
    Throw(Value),
    Return(Value),
    Break(Option<Rc<str>>),
    Continue(Option<Rc<str>>),
    /// Construct outside the supported subset (feature name for the routing scan).
    Unsupported(String),
    /// `process.exit(code)`: unwinds everything.
    Exit(i32),
}

impl std::fmt::Debug for Ctl {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Ctl::Throw(_) => write!(f, "Throw(..)"),
            Ctl::Return(_) => write!(f, "Return(..)"),
            Ctl::Break(l) => write!(f, "Break({l:?})"),
            Ctl::Continue(l) => write!(f, "Continue({l:?})"),
            Ctl::Unsupported(x) => write!(f, "Unsupported({x})"),
            Ctl::Exit(c) => write!(f, "Exit({c})"),
        }
    }
}

pub type R<T> = Result<T, Ctl>;

struct Binding {
    value: Value,
    mutable: bool,
}

pub struct Scope {
    vars: RefCell<HashMap<Rc<str>, Binding>>,
    parent: Option<Rc<Scope>>,
    strict: Cell<bool>,
}

impl Scope {
    fn new(parent: Option<Rc<Scope>>) -> Rc<Scope> {
        let strict = parent.as_ref().is_some_and(|p| p.strict.get());
        Rc::new(Scope {
            vars: RefCell::new(HashMap::new()),
            parent,
            strict: Cell::new(strict),
        })
    }
    fn declare(&self, name: &str, value: Value, mutable: bool) {
        self.vars
            .borrow_mut()
            .insert(name.into(), Binding { value, mutable });
    }
    fn copy_of(&self, parent: Rc<Scope>) -> Rc<Scope> {
        let vars = self
            .vars
            .borrow()
            .iter()
            .map(|(k, b)| {
                (
                    k.clone(),
                    Binding {
                        value: b.value.clone(),
                        mutable: b.mutable,
                    },
                )
            })
            .collect();
        let strict = Cell::new(self.strict.get());
        Rc::new(Scope {
            vars: RefCell::new(vars),
            parent: Some(parent),
            strict,
        })
    }
}

fn unsupported<T>(what: &str) -> R<T> {
    Err(Ctl::Unsupported(what.to_string()))
}

impl Value {
    pub fn str(s: &str) -> Value {
        Value::Str(s.into())
    }
    fn truthy(&self) -> bool {
        match self {
            Value::Undefined | Value::Null => false,
            Value::Bool(b) => *b,
            Value::Number(n) => !(*n == 0.0 || n.is_nan()),
            Value::Str(s) => !s.is_empty(),
            Value::Object(_) => true,
        }
    }
    pub fn as_number(&self) -> Option<f64> {
        if let Value::Number(n) = self {
            Some(*n)
        } else {
            None
        }
    }
    fn is_callable(&self) -> bool {
        matches!(self, Value::Object(o) if matches!(&*o.kind.borrow(), Kind::Closure { .. } | Kind::Native(_)))
    }
}

fn new_obj(proto: Option<Obj>, kind: Kind) -> Obj {
    Rc::new(ObjData {
        props: RefCell::new(Vec::new()),
        proto: RefCell::new(proto),
        kind: RefCell::new(kind),
        frozen: Cell::new(false),
    })
}

pub(crate) fn number_to_string(n: f64) -> String {
    if n.is_nan() {
        "NaN".into()
    } else if n.is_infinite() {
        if n > 0.0 {
            "Infinity".into()
        } else {
            "-Infinity".into()
        }
    } else if n == 0.0 {
        "0".into()
    } else if n.abs() >= 1e21 || n.abs() < 1e-6 {
        // JS switches to exponent form outside [1e-6, 1e21); Rust's `{:e}` has the shortest digits.
        let s = format!("{n:e}");
        match s.split_once('e') {
            Some((m, e)) if !e.starts_with('-') => format!("{m}e+{e}"),
            _ => s,
        }
    } else if n.fract() == 0.0 {
        format!("{n:.0}")
    } else {
        format!("{n}")
    }
}

pub(crate) fn string_to_number(s: &str) -> f64 {
    let t = s.trim();
    if t.is_empty() {
        return 0.0;
    }
    if let Some(hex) = t.strip_prefix("0x").or_else(|| t.strip_prefix("0X")) {
        return i64::from_str_radix(hex, 16).map_or(f64::NAN, |v| v as f64);
    }
    match t {
        "Infinity" | "+Infinity" => f64::INFINITY,
        "-Infinity" => f64::NEG_INFINITY,
        _ if t.chars().any(|c| c.is_alphabetic() && c != 'e' && c != 'E') => f64::NAN,
        _ => t.parse().unwrap_or(f64::NAN),
    }
}

pub(crate) fn to_int32(n: f64) -> i32 {
    if !n.is_finite() {
        return 0;
    }
    (n.trunc().rem_euclid(4294967296.0) as u64 as u32) as i32
}

pub(crate) fn strict_equals(x: &Value, y: &Value) -> bool {
    match (x, y) {
        (Value::Undefined, Value::Undefined) | (Value::Null, Value::Null) => true,
        (Value::Bool(p), Value::Bool(q)) => p == q,
        (Value::Number(p), Value::Number(q)) => p == q,
        (Value::Str(p), Value::Str(q)) => p == q,
        (Value::Object(p), Value::Object(q)) => Rc::ptr_eq(p, q),
        _ => false,
    }
}

pub struct Interp {
    global: Rc<Scope>,
    object_proto: Obj,
    function_proto: Obj,
    array_proto: Obj,
    string_proto: Obj,
    regexp_proto: Obj,
    number_proto: Obj,
    bool_proto: Obj,
    global_object: Obj,
    next_symbol: u64,
    regex_cache: HashMap<(String, String), Rc<regress::Regex>>,
    fn_cache: HashMap<(u32, u32), Rc<FuncDef>>,
    /// Distinguishes source positions of different `run` calls (spans restart at 1 per parse).
    run_id: u32,
    depth: usize,
    /// Call-depth cap; replaced by the stack-address check once `configure_stack` was called.
    max_depth: usize,
    /// Lowest stack address JS calls may reach (0 = not configured).
    pub(crate) stack_limit: usize,
    /// Lines written by `console.log` (kept for tests and embedders).
    pub out: Vec<String>,
    /// Lines written by `console.error` / `console.warn`.
    pub err: Vec<String>,
    pub(crate) rt: eventloop::Rt,
}

/// Finds a free use of `arguments` in a function body (not crossing non-arrow functions).
#[derive(Default)]
struct ArgumentsFinder {
    found: bool,
}

impl Visit for ArgumentsFinder {
    fn visit_ident(&mut self, n: &a::Ident) {
        if &*n.sym == "arguments" {
            self.found = true;
        }
    }
    fn visit_function(&mut self, _: &a::Function) {}
    fn visit_member_prop(&mut self, n: &a::MemberProp) {
        if let a::MemberProp::Computed(c) = n {
            c.visit_with(self);
        }
    }
}

impl Interp {
    pub fn new() -> Interp {
        let object_proto = new_obj(None, Kind::Plain);
        let mk = |op: &Obj| new_obj(Some(op.clone()), Kind::Plain);
        let (function_proto, array_proto, string_proto, regexp_proto) = (
            mk(&object_proto),
            mk(&object_proto),
            mk(&object_proto),
            mk(&object_proto),
        );
        let (number_proto, bool_proto) = (mk(&object_proto), mk(&object_proto));
        let mut it = Interp {
            global: Scope::new(None),
            object_proto: object_proto.clone(),
            function_proto,
            array_proto,
            string_proto,
            regexp_proto,
            global_object: new_obj(Some(object_proto.clone()), Kind::Global),
            next_symbol: 0,
            regex_cache: HashMap::new(),
            number_proto,
            bool_proto,
            fn_cache: HashMap::new(),
            run_id: 0,
            depth: 0,
            max_depth: MAX_DEPTH,
            stack_limit: 0,
            out: Vec::new(),
            err: Vec::new(),
            rt: Default::default(),
        };
        it.install_builtins();
        it.install_collections();
        it.install_eventloop();
        it.run(builtins::PRELUDE)
            .unwrap_or_else(|_| panic!("thaw-js prelude must evaluate"));
        it.run_promise_prelude();
        it.run(extras::EXTRAS)
            .unwrap_or_else(|_| panic!("thaw-js extras prelude must evaluate"));
        it.install_typed();
        it.install_buffer();
        it.install_url();
        it.name_builtins();
        it
    }

    /// The embedder promises that roughly `bytes` of stack are available from here: recursion is
    /// then limited by actual stack use (a `RangeError`) instead of a fixed call depth.
    pub fn configure_stack(&mut self, bytes: usize) {
        let marker = 0u8;
        let here = &marker as *const u8 as usize;
        self.stack_limit = here.saturating_sub(bytes.saturating_sub(bytes / 8 + (4 << 20)));
        self.max_depth = 200_000;
    }

    pub(crate) fn too_deep(&self) -> bool {
        if self.depth >= self.max_depth {
            return true;
        }
        let marker = 0u8;
        self.stack_limit != 0 && (&marker as *const u8 as usize) < self.stack_limit
    }

    pub(crate) fn native(
        &self,
        f: impl Fn(&mut Interp, &Value, &[Value]) -> R<Value> + 'static,
    ) -> Value {
        Value::Object(new_obj(
            Some(self.function_proto.clone()),
            Kind::Native(Rc::new(f)),
        ))
    }

    pub(crate) fn set_native(
        &self,
        target: &Obj,
        name: &str,
        f: impl Fn(&mut Interp, &Value, &[Value]) -> R<Value> + 'static,
    ) {
        let v = self.native(f);
        put_hidden(target, name, v);
    }

    pub(crate) fn array(&self, items: Vec<Value>) -> Value {
        Value::Object(new_obj(Some(self.array_proto.clone()), Kind::Array(items)))
    }

    pub(crate) fn plain(&self) -> Obj {
        new_obj(Some(self.object_proto.clone()), Kind::Plain)
    }

    // ----- introspection for the feature scan -----

    pub fn global_names(&self) -> Vec<String> {
        let mut names: Vec<String> = self
            .global
            .vars
            .borrow()
            .keys()
            .map(|k| k.to_string())
            .collect();
        names.extend(
            self.object_proto
                .props
                .borrow()
                .iter()
                .map(|p| p.key.to_string()),
        );
        names
    }

    /// True for a global holding an object/function whose members the scan can check.
    pub fn is_builtin_global(&self, name: &str) -> bool {
        matches!(self.lookup(&self.global, name), Some(Value::Object(_)))
    }

    pub fn builtin_member_exists(&self, global: &str, member: &str) -> bool {
        match self.lookup(&self.global, global) {
            Some(Value::Object(o)) if matches!(&*o.kind.borrow(), Kind::Global) => {
                self.lookup(&self.global, member).is_some()
            }
            Some(Value::Object(o)) => {
                find_in_chain(&o, member).is_some()
                    || member == "prototype"
                    || member == "length"
                    || member == "name"
            }
            _ => true,
        }
    }

    /// Every property name reachable on a builtin object or prototype (for `x.m()` heuristics).
    pub fn known_method_names(&self) -> Vec<String> {
        let mut names: Vec<String> = Vec::new();
        let mut objs: Vec<Obj> = vec![
            self.object_proto.clone(),
            self.function_proto.clone(),
            self.array_proto.clone(),
            self.string_proto.clone(),
            self.regexp_proto.clone(),
        ];
        names.extend(self.global.vars.borrow().keys().map(|k| k.to_string()));
        for v in self.global.vars.borrow().values() {
            if let Value::Object(o) = &v.value {
                objs.push(o.clone());
                if let Some((Slot::Data(Value::Object(p)), _)) = find_own(o, "prototype") {
                    objs.push(p);
                }
            }
        }
        for o in objs {
            names.extend(o.props.borrow().iter().map(|p| p.key.to_string()));
        }
        names.extend(["next", "return", "throw", "require"].map(String::from)); // iterator protocol / `module.require` (user-defined)
        names
    }

    // ----- conversions -----

    pub fn to_str(&mut self, v: &Value) -> Rc<str> {
        match v {
            Value::Undefined => "undefined".into(),
            Value::Null => "null".into(),
            Value::Bool(b) => if *b { "true" } else { "false" }.into(),
            Value::Number(n) => number_to_string(*n).into(),
            Value::Str(s) => s.clone(),
            Value::Object(o) => {
                if matches!(&*o.kind.borrow(), Kind::Array(_)) {
                    return self.join(v, ",");
                }
                if let Ok(p) = self.primitive_of(v, extras::Hint::String) {
                    return self.to_str(&p);
                }
                if v.is_callable() {
                    "function".into()
                } else {
                    "[object Object]".into()
                }
            }
        }
    }

    pub fn num(&mut self, v: &Value) -> f64 {
        match v {
            Value::Undefined => f64::NAN,
            Value::Null => 0.0,
            Value::Bool(b) => *b as u8 as f64,
            Value::Number(n) => *n,
            Value::Str(s) => string_to_number(s),
            Value::Object(_) => match self.primitive_of(v, extras::Hint::Number) {
                Ok(p) => self.num(&p),
                Err(_) => f64::NAN,
            },
        }
    }

    pub(crate) fn join(&mut self, this: &Value, sep: &str) -> Rc<str> {
        let items = match this {
            Value::Object(o) => match &*o.kind.borrow() {
                Kind::Array(v) => v.clone(),
                _ => Vec::new(),
            },
            _ => Vec::new(),
        };
        items
            .iter()
            .map(|v| {
                if matches!(v, Value::Undefined | Value::Null) {
                    Rc::from("")
                } else {
                    self.to_str(v)
                }
            })
            .collect::<Vec<_>>()
            .join(sep)
            .into()
    }

    /// `console.log`-style rendering (a small subset of Node's util.inspect).
    pub fn inspect(&mut self, v: &Value, top: bool) -> String {
        match v {
            Value::Str(s) if top => s.to_string(),
            Value::Str(s) => format!("'{s}'"),
            Value::Object(o) => {
                enum K {
                    Arr(Vec<Value>),
                    Fun,
                    Re,
                    Prom,
                    Bytes(String),
                    Plain,
                }
                let kind = match &*o.kind.borrow() {
                    Kind::Array(items) => K::Arr(items.clone()),
                    Kind::Closure { .. } | Kind::Native(_) => K::Fun,
                    Kind::RegExp(_) => K::Re,
                    Kind::Plain
                    | Kind::Global
                    | Kind::Map(_)
                    | Kind::Set(_)
                    | Kind::Symbol { .. } => K::Plain,
                    Kind::Promise(_) => K::Prom,
                    Kind::ArrayBuffer(b) => {
                        let b = b.borrow();
                        let shown: Vec<String> =
                            b.iter().take(50).map(|x| format!("{x:02x}")).collect();
                        let more = if b.len() > 50 {
                            format!(
                                " ... {} more byte{}",
                                b.len() - 50,
                                if b.len() - 50 == 1 { "" } else { "s" }
                            )
                        } else {
                            String::new()
                        };
                        K::Bytes(format!(
                            "ArrayBuffer {{ [Uint8Contents]: <{}{more}>, [byteLength]: {} }}",
                            shown.join(" "),
                            b.len()
                        ))
                    }
                    Kind::Typed(t) => {
                        let is_buffer = find_in_chain(o, "__isBuffer").is_some();
                        if is_buffer {
                            let vals = t.to_vec();
                            let shown: Vec<String> = vals
                                .iter()
                                .take(50)
                                .map(|x| format!("{:02x}", *x as u8))
                                .collect();
                            let more = if vals.len() > 50 {
                                format!(
                                    " ... {} more byte{}",
                                    vals.len() - 50,
                                    if vals.len() - 50 == 1 { "" } else { "s" }
                                )
                            } else {
                                String::new()
                            };
                            K::Bytes(format!(
                                "<Buffer{}{}{more}>",
                                if shown.is_empty() { "" } else { " " },
                                shown.join(" ")
                            ))
                        } else {
                            let items: Vec<String> =
                                t.to_vec().into_iter().map(number_to_string).collect();
                            let name = t.ty.name();
                            K::Bytes(if items.is_empty() {
                                format!("{name}(0) []")
                            } else {
                                format!("{name}({}) [ {} ]", items.len(), items.join(", "))
                            })
                        }
                    }
                };
                match kind {
                    K::Arr(items) if items.is_empty() => "[]".into(),
                    K::Arr(items) => format!(
                        "[ {} ]",
                        items
                            .iter()
                            .map(|i| self.inspect(i, false))
                            .collect::<Vec<_>>()
                            .join(", ")
                    ),
                    K::Fun => match self.get(v, "name") {
                        Ok(Value::Str(n)) if !n.is_empty() => format!("[Function: {n}]"),
                        _ => "[Function (anonymous)]".into(),
                    },
                    K::Re => self.to_str(v).to_string(),
                    K::Bytes(text) => text,
                    K::Prom => match self.promise_state(v) {
                        None => "Promise { <pending> }".into(),
                        Some(Ok(x)) => format!("Promise {{ {} }}", self.inspect(&x, false)),
                        Some(Err(x)) => {
                            format!("Promise {{ <rejected> {} }}", self.inspect(&x, false))
                        }
                    },
                    K::Plain => {
                        let props: Vec<Prop> = o
                            .props
                            .borrow()
                            .iter()
                            .filter(|p| p.enumerable)
                            .cloned()
                            .collect();
                        if props.is_empty() {
                            "{}".into()
                        } else {
                            let parts: Vec<String> = props
                                .iter()
                                .map(|p| match &p.slot {
                                    Slot::Data(val) => {
                                        format!("{}: {}", p.key, self.inspect(val, false))
                                    }
                                    Slot::Accessor { .. } => format!("{}: [Getter/Setter]", p.key),
                                })
                                .collect();
                            format!("{{ {} }}", parts.join(", "))
                        }
                    }
                }
            }
            Value::Number(n) if *n == 0.0 && n.is_sign_negative() => "-0".into(),
            other => (*self.to_str(other)).to_string(),
        }
    }

    /// Own enumerable string keys (array indices first).
    pub(crate) fn own_keys(&self, v: &Value) -> Vec<Rc<str>> {
        match v {
            Value::Object(o) => {
                let mut keys: Vec<Rc<str>> = Vec::new();
                if let Kind::Array(items) = &*o.kind.borrow() {
                    keys.extend((0..items.len()).map(|i| Rc::from(i.to_string())));
                }
                if let Kind::Typed(t) = &*o.kind.borrow() {
                    keys.extend((0..t.len).map(|i| Rc::from(i.to_string())));
                }
                keys.extend(
                    o.props
                        .borrow()
                        .iter()
                        .filter(|p| p.enumerable && !p.key.starts_with("@@sym:"))
                        .map(|p| p.key.clone()),
                );
                keys
            }
            Value::Str(s) => (0..s.chars().count())
                .map(|i| Rc::from(i.to_string()))
                .collect(),
            _ => Vec::new(),
        }
    }

    pub(crate) fn throw_error<T>(&mut self, class: &str, message: &str) -> R<T> {
        let ctor = self
            .lookup(&self.global.clone(), class)
            .unwrap_or(Value::Undefined);
        // the error constructor must be callable even at the depth limit
        let depth = std::mem::replace(&mut self.depth, 0);
        let built = self.construct(&ctor, &[Value::str(message)]);
        self.depth = depth;
        match built {
            Ok(err) => Err(Ctl::Throw(err)),
            Err(e) => Err(e),
        }
    }

    // ----- property access -----

    pub fn get(&mut self, target: &Value, key: &str) -> R<Value> {
        let start = match target {
            Value::Undefined | Value::Null => {
                let what = if matches!(target, Value::Null) {
                    "null"
                } else {
                    "undefined"
                };
                return self.throw_error(
                    "TypeError",
                    &format!("Cannot read properties of {what} (reading '{key}')"),
                );
            }
            Value::Str(s) => {
                if key == "length" {
                    return Ok(Value::Number(s.chars().count() as f64));
                }
                if let Ok(i) = key.parse::<usize>() {
                    return Ok(s
                        .chars()
                        .nth(i)
                        .map_or(Value::Undefined, |c| Value::Str(c.to_string().into())));
                }
                self.string_proto.clone()
            }
            Value::Number(_) => self.number_proto.clone(),
            Value::Bool(_) => self.bool_proto.clone(),
            Value::Object(o) => {
                if matches!(&*o.kind.borrow(), Kind::Global) {
                    if let Some(v) = self.lookup(&self.global.clone(), key) {
                        return Ok(v);
                    }
                }
                if let Kind::Array(items) = &*o.kind.borrow() {
                    if key == "length" {
                        return Ok(Value::Number(items.len() as f64));
                    }
                    if let Ok(i) = key.parse::<usize>() {
                        return Ok(items.get(i).cloned().unwrap_or(Value::Undefined));
                    }
                }
                if let Kind::Typed(t) = &*o.kind.borrow() {
                    match key {
                        "length" => return Ok(Value::Number(t.len as f64)),
                        "byteLength" => return Ok(Value::Number((t.len * t.ty.size()) as f64)),
                        "byteOffset" => return Ok(Value::Number(t.off as f64)),
                        "buffer" => return Ok(Value::Object(t.abuf.clone())),
                        _ => {
                            if let Ok(i) = key.parse::<usize>() {
                                return Ok(t.get(i).map_or(Value::Undefined, Value::Number));
                            }
                        }
                    }
                }
                if key == "byteLength" {
                    if let Kind::ArrayBuffer(b) = &*o.kind.borrow() {
                        return Ok(Value::Number(b.borrow().len() as f64));
                    }
                }
                if key == "name" || key == "length" {
                    if let Kind::Closure { def, .. } = &*o.kind.borrow() {
                        if find_own(o, key).is_none() {
                            return Ok(if key == "name" {
                                def.name.clone().map_or(Value::str(""), Value::Str)
                            } else {
                                Value::Number(
                                    def.params
                                        .iter()
                                        .take_while(|p| matches!(p, a::Pat::Ident(_)))
                                        .count() as f64,
                                )
                            });
                        }
                    }
                }
                o.clone()
            }
        };
        match find_in_chain(&start, key) {
            None => Ok(Value::Undefined),
            Some(Slot::Data(v)) => Ok(v),
            Some(Slot::Accessor { get: Some(g), .. }) => self.call(&g, target, &[]),
            Some(Slot::Accessor { .. }) => Ok(Value::Undefined),
        }
    }

    pub fn set(&mut self, target: &Value, key: &str, value: Value) -> R<()> {
        let Value::Object(o) = target else {
            if matches!(target, Value::Undefined | Value::Null) {
                return self.throw_error(
                    "TypeError",
                    &format!(
                        "Cannot set properties of {} (setting '{key}')",
                        if matches!(target, Value::Null) {
                            "null"
                        } else {
                            "undefined"
                        }
                    ),
                );
            }
            return Ok(());
        };
        if o.frozen.get() {
            return Ok(()); // sloppy mode: silently ignored
        }
        if matches!(&*o.kind.borrow(), Kind::Global) {
            return self.assign_var(&self.global.clone(), key, value);
        }
        if let Kind::Array(items) = &mut *o.kind.borrow_mut() {
            if let Ok(i) = key.parse::<usize>() {
                if i >= items.len() {
                    items.resize(i + 1, Value::Undefined); // ponytail: holes read as undefined
                }
                items[i] = value;
                return Ok(());
            }
            if key == "length" {
                let n = match value {
                    Value::Number(n) if n >= 0.0 => n as usize,
                    _ => 0,
                };
                items.resize(n, Value::Undefined);
                return Ok(());
            }
        }
        if let Kind::Typed(t) = &*o.kind.borrow() {
            if let Ok(i) = key.parse::<usize>() {
                let t = t.clone();
                let n = self.num(&value);
                t.set(i, n);
                return Ok(());
            }
        }
        match find_in_chain(o, key) {
            Some(Slot::Accessor { set, .. }) => {
                if let Some(s) = set {
                    self.call(&s, target, &[value])?;
                }
                return Ok(());
            }
            Some(Slot::Data(_)) => {
                // an inherited or own read-only data property ignores the write
                let mut cur = Some(o.clone());
                while let Some(c) = cur {
                    if let Some(p) = c.props.borrow().iter().find(|p| &*p.key == key) {
                        if !p.writable {
                            return Ok(());
                        }
                        break;
                    }
                    cur = c.proto.borrow().clone();
                }
            }
            None => {}
        }
        put(o, key, value);
        Ok(())
    }

    // ----- calls -----

    pub fn call(&mut self, f: &Value, this: &Value, args: &[Value]) -> R<Value> {
        let Value::Object(fo) = f else {
            return self.throw_error("TypeError", "value is not a function");
        };
        enum Callee {
            Native(NativeFn),
            Closure(Rc<FuncDef>, Rc<Scope>),
            Class(Rc<str>),
        }
        let callee = match &*fo.kind.borrow() {
            Kind::Native(n) => Callee::Native(n.clone()),
            Kind::Closure { def, .. } if def.class.is_some() => {
                Callee::Class(def.name.clone().unwrap_or_else(|| "".into()))
            }
            Kind::Closure { def, scope } => Callee::Closure(def.clone(), scope.clone()),
            _ => return self.throw_error("TypeError", "value is not a function"),
        };
        if self.too_deep() {
            return self.throw_error("RangeError", "Maximum call stack size exceeded");
        }
        self.depth += 1;
        let result = match callee {
            Callee::Native(n) => n(self, this, args),
            Callee::Closure(def, scope) => self.call_closure(f, &def, &scope, this, args),
            Callee::Class(n) => self.throw_error(
                "TypeError",
                &format!("Class constructor {n} cannot be invoked without 'new'"),
            ),
        };
        self.depth -= 1;
        result
    }

    fn call_closure(
        &mut self,
        f: &Value,
        def: &Rc<FuncDef>,
        scope: &Rc<Scope>,
        this: &Value,
        args: &[Value],
    ) -> R<Value> {
        if def.is_async {
            return self.call_async(f, def, scope, this, args);
        }
        self.call_closure_sync(f, def, scope, this, args)
    }

    pub(crate) fn call_closure_sync(
        &mut self,
        f: &Value,
        def: &Rc<FuncDef>,
        scope: &Rc<Scope>,
        this: &Value,
        args: &[Value],
    ) -> R<Value> {
        let frame = Scope::new(Some(scope.clone()));
        if def.strict {
            frame.strict.set(true);
        }
        if !def.arrow {
            let this = if !frame.strict.get() && matches!(this, Value::Undefined | Value::Null) {
                Value::Object(self.global_object.clone())
            } else {
                this.clone()
            };
            frame.declare("this", this, false);
            if let Some(name) = def.name.as_ref().filter(|_| def.bind_self) {
                frame.declare(name, f.clone(), true);
            }
            if def.uses_arguments {
                frame.declare("arguments", self.array(args.to_vec()), true);
            }
        }
        for (i, p) in def.params.iter().enumerate() {
            if let a::Pat::Rest(r) = p {
                let rest = self.array(args.get(i..).unwrap_or(&[]).to_vec());
                self.bind_pattern(&r.arg, rest, &frame, true)?;
            } else {
                self.bind_pattern(
                    p,
                    args.get(i).cloned().unwrap_or(Value::Undefined),
                    &frame,
                    true,
                )?;
            }
        }
        match &def.body {
            Body::Expr(e) => self.eval(e, &frame),
            Body::Block(stmts) => {
                hoist_vars(stmts, &frame);
                match self.exec_stmts(stmts, &frame) {
                    Ok(()) => Ok(Value::Undefined),
                    Err(Ctl::Return(v)) => Ok(v),
                    Err(e) => Err(e),
                }
            }
        }
    }

    pub fn construct(&mut self, f: &Value, args: &[Value]) -> R<Value> {
        let Value::Object(fo) = f else {
            return self.throw_error("TypeError", "value is not a constructor");
        };
        let (is_closure, is_native) = (
            matches!(&*fo.kind.borrow(), Kind::Closure { def, .. } if !def.arrow),
            matches!(&*fo.kind.borrow(), Kind::Native(_)),
        );
        let is_class =
            matches!(&*fo.kind.borrow(), Kind::Closure { def, .. } if def.class.is_some());
        if !is_closure && !is_native {
            return self.throw_error("TypeError", "value is not a constructor");
        }
        let proto = match self.get(f, "prototype")? {
            Value::Object(p) => p,
            _ => self.object_proto.clone(),
        };
        if is_native {
            // natives that act as constructors (Array, RegExp, Object, ...) return their own object
            self.rt.constructing = true;
            let r = self.call(f, &Value::Undefined, args);
            self.rt.constructing = false;
            let r = r?;
            return Ok(r);
        }
        let this = Value::Object(new_obj(Some(proto), Kind::Plain));
        let result = if is_class {
            self.init_instance(f, &this, args)?
        } else {
            self.call(f, &this, args)?
        };
        match result {
            r @ Value::Object(_) => Ok(r),
            _ => Ok(this),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn make_function(
        &mut self,
        name: Option<Rc<str>>,
        params: Vec<a::Pat>,
        body: Body,
        arrow: bool,
        is_async: bool,
        bind_self: bool,
        lo: u32,
        scope: &Rc<Scope>,
    ) -> Value {
        let def = self
            .fn_cache
            .entry((self.run_id, lo))
            .or_insert_with(|| {
                let mut finder = ArgumentsFinder::default();
                match &body {
                    Body::Block(stmts) => stmts.visit_with(&mut finder),
                    Body::Expr(e) => e.visit_with(&mut finder),
                }
                params.visit_with(&mut finder);
                let strict = matches!(&body, Body::Block(s) if matches!(s.first(), Some(a::Stmt::Expr(e)) if matches!(&*e.expr, a::Expr::Lit(a::Lit::Str(x)) if x.value.as_str() == Some("use strict"))));
                Rc::new(FuncDef { name, params, body, arrow, is_async, bind_self, uses_arguments: finder.found && !arrow, strict, class: None })
            })
            .clone();
        let f = new_obj(
            Some(self.function_proto.clone()),
            Kind::Closure {
                def,
                scope: scope.clone(),
            },
        );
        if !arrow {
            let proto = new_obj(Some(self.object_proto.clone()), Kind::Plain);
            put_hidden(&proto, "constructor", Value::Object(f.clone()));
            put_hidden(&f, "prototype", Value::Object(proto));
        }
        Value::Object(f)
    }

    // ----- scopes -----

    fn lookup(&self, scope: &Rc<Scope>, name: &str) -> Option<Value> {
        let mut cur = Some(scope.clone());
        while let Some(s) = cur {
            if let Some(b) = s.vars.borrow().get(name) {
                return Some(b.value.clone());
            }
            cur = s.parent.clone();
        }
        None
    }

    fn assign_var(&mut self, scope: &Rc<Scope>, name: &str, value: Value) -> R<()> {
        let mut cur = Some(scope.clone());
        while let Some(s) = cur {
            let outcome = s.vars.borrow_mut().get_mut(name).map(|b| {
                if b.mutable {
                    b.value = value.clone();
                }
                b.mutable
            });
            match outcome {
                Some(true) => return Ok(()),
                Some(false) => {
                    return self.throw_error("TypeError", "Assignment to constant variable.")
                }
                None => cur = s.parent.clone(),
            }
        }
        // sloppy-mode implicit global
        self.global.declare(name, value, true);
        Ok(())
    }

    /// Bind (`declare`) or assign (`!declare`) `value` to a pattern.
    fn bind_pattern(
        &mut self,
        pat: &a::Pat,
        value: Value,
        scope: &Rc<Scope>,
        declare: bool,
    ) -> R<()> {
        match pat {
            a::Pat::Ident(id) => {
                if declare {
                    scope.declare(&id.id.sym, value, true);
                    Ok(())
                } else {
                    self.assign_var(scope, &id.id.sym, value)
                }
            }
            a::Pat::Assign(d) => {
                let v = if matches!(value, Value::Undefined) {
                    self.eval(&d.right, scope)?
                } else {
                    value
                };
                self.bind_pattern(&d.left, v, scope, declare)
            }
            a::Pat::Expr(e) => self.store(e, value, scope),
            a::Pat::Array(ap) => {
                if !matches!(value, Value::Object(_) | Value::Str(_)) {
                    return self.throw_error("TypeError", "value is not iterable");
                }
                // only as many values as the pattern names are pulled (all of them with a rest element)
                let wanted = if ap
                    .elems
                    .iter()
                    .any(|el| matches!(el, Some(a::Pat::Rest(_))))
                {
                    None
                } else {
                    Some(ap.elems.len())
                };
                let Some(items) = self.iter_protocol_limit(&value, wanted)? else {
                    return unsupported("iterator-protocol");
                };
                for (i, el) in ap.elems.iter().enumerate() {
                    match el {
                        Some(a::Pat::Rest(r)) => {
                            let rest = self.array(items.get(i..).unwrap_or(&[]).to_vec());
                            self.bind_pattern(&r.arg, rest, scope, declare)?;
                        }
                        Some(p) => self.bind_pattern(
                            p,
                            items.get(i).cloned().unwrap_or(Value::Undefined),
                            scope,
                            declare,
                        )?,
                        None => {}
                    }
                }
                Ok(())
            }
            a::Pat::Object(op) => {
                if matches!(value, Value::Undefined | Value::Null) {
                    return self.throw_error("TypeError", "Cannot destructure a nullish value");
                }
                let mut used: Vec<Rc<str>> = Vec::new();
                for p in &op.props {
                    match p {
                        a::ObjectPatProp::KeyValue(kv) => {
                            let k = self.prop_name(&kv.key, scope)?;
                            let v = self.get(&value, &k)?;
                            used.push(k);
                            self.bind_pattern(&kv.value, v, scope, declare)?;
                        }
                        a::ObjectPatProp::Assign(ap) => {
                            let k: Rc<str> = ap.key.id.sym.as_ref().into();
                            let mut v = self.get(&value, &k)?;
                            if matches!(v, Value::Undefined) {
                                if let Some(d) = &ap.value {
                                    v = self.eval(d, scope)?;
                                }
                            }
                            used.push(k.clone());
                            self.bind_pattern(&a::Pat::Ident(ap.key.clone()), v, scope, declare)?;
                        }
                        a::ObjectPatProp::Rest(r) => {
                            let rest = self.plain();
                            for k in self.own_keys(&value) {
                                if !used.contains(&k) {
                                    let v = self.get(&value, &k)?;
                                    put(&rest, &k, v);
                                }
                            }
                            self.bind_pattern(&r.arg, Value::Object(rest), scope, declare)?;
                        }
                    }
                }
                Ok(())
            }
            _ => unsupported("pattern kind"),
        }
    }

    // ----- entry points -----

    /// Evaluate a script in the global scope; the value is that of the last expression statement.
    pub fn run(&mut self, source: &str) -> R<Value> {
        let module =
            thaw_parser::parse_javascript(source).map_err(|e| Ctl::Throw(Value::str(&e)))?;
        self.run_id += 1;
        let mut stmts = Vec::new();
        for item in module.body {
            match item {
                a::ModuleItem::Stmt(s) => stmts.push(s),
                a::ModuleItem::ModuleDecl(_) => return unsupported("esm-syntax"),
            }
        }
        let global = self.global.clone();
        hoist_vars(&stmts, &global);
        let mut last = Value::Undefined;
        self.hoist_functions(&stmts, &global)?;
        for s in &stmts {
            if let a::Stmt::Expr(e) = s {
                last = self.eval(&e.expr, &global)?;
            } else {
                self.exec(s, &global)?;
            }
        }
        Ok(last)
    }

    /// Compile a function literal (`function (a, b) { ... }` expression source) in the global scope.
    pub fn eval_function(&mut self, source: &str) -> R<Value> {
        self.run(&format!("({source})"))
    }

    /// Run a CommonJS-style script and return `module.exports`.
    pub fn run_commonjs(&mut self, source: &str) -> R<Value> {
        let module = Value::Object(self.plain());
        let exports = Value::Object(self.plain());
        self.set(&module, "exports", exports.clone())?;
        self.global.declare("module", module.clone(), true);
        self.global.declare("exports", exports, true);
        self.run(source)?;
        self.get(&module, "exports")
    }

    /// Define a global `require(name)` serving preloaded module exports (for standalone runs).
    pub fn install_require(&mut self, modules: Vec<(String, Value)>) {
        let f = self.native(move |it, _, args| {
            let name = it.to_str(&args.first().cloned().unwrap_or(Value::Undefined));
            match modules.iter().find(|(n, _)| **n == *name) {
                Some((_, v)) => Ok(v.clone()),
                None => it.throw_error("Error", &format!("Cannot find module '{name}'")),
            }
        });
        self.global.declare("require", f, true);
    }

    /// Call `exports[name](...args)`.
    pub fn call_export(&mut self, exports: &Value, name: &str, args: &[Value]) -> R<Value> {
        let f = self.get(exports, name)?;
        if !f.is_callable() {
            return self.throw_error("TypeError", &format!("{name} is not a function"));
        }
        self.call(&f, exports, args)
    }

    // ----- statements -----

    fn function_value(
        &mut self,
        name: Option<Rc<str>>,
        f: &a::Function,
        scope: &Rc<Scope>,
    ) -> R<Value> {
        self.function_value_ex(name, f, scope, true)
    }

    /// A method / accessor: named, but its name is not a binding inside its own body.
    pub(crate) fn method_value(
        &mut self,
        name: Option<Rc<str>>,
        f: &a::Function,
        scope: &Rc<Scope>,
    ) -> R<Value> {
        self.function_value_ex(name, f, scope, false)
    }

    fn function_value_ex(
        &mut self,
        name: Option<Rc<str>>,
        f: &a::Function,
        scope: &Rc<Scope>,
        bind_self: bool,
    ) -> R<Value> {
        if f.is_generator {
            return unsupported("generator");
        }
        let body = Body::Block(f.body.as_ref().map(|b| b.stmts.clone()).unwrap_or_default());
        Ok(self.make_function(
            name,
            fn_pats(&f.params),
            body,
            false,
            f.is_async,
            bind_self,
            f.span.lo.0,
            scope,
        ))
    }

    fn hoist_functions(&mut self, stmts: &[a::Stmt], scope: &Rc<Scope>) -> R<()> {
        for s in stmts {
            if let a::Stmt::Decl(a::Decl::Fn(f)) = s {
                let name: Rc<str> = f.ident.sym.as_ref().into();
                let func = self.function_value(Some(name.clone()), &f.function, scope)?;
                scope.declare(&name, func, true);
            }
        }
        Ok(())
    }

    fn exec_stmts(&mut self, stmts: &[a::Stmt], scope: &Rc<Scope>) -> R<()> {
        self.hoist_functions(stmts, scope)?;
        for s in stmts {
            self.exec(s, scope)?;
        }
        Ok(())
    }

    fn exec_block(&mut self, stmts: &[a::Stmt], scope: &Rc<Scope>) -> R<()> {
        let inner = Scope::new(Some(scope.clone()));
        self.exec_stmts(stmts, &inner)
    }

    fn exec(&mut self, s: &a::Stmt, scope: &Rc<Scope>) -> R<()> {
        match s {
            a::Stmt::Expr(e) => {
                self.eval(&e.expr, scope)?;
                Ok(())
            }
            a::Stmt::Empty(_) | a::Stmt::Debugger(_) => Ok(()),
            a::Stmt::Block(b) => self.exec_block(&b.stmts, scope),
            a::Stmt::Decl(a::Decl::Fn(_)) => Ok(()), // hoisted
            a::Stmt::Decl(a::Decl::Var(v)) => self.exec_var(v, scope),
            a::Stmt::Decl(a::Decl::Class(c)) => {
                let v = self.eval_class(Some(&c.ident), &c.class, scope)?;
                scope.declare(&c.ident.sym, v, true);
                Ok(())
            }
            a::Stmt::Decl(_) => unsupported("declaration kind"),
            a::Stmt::Return(r) => {
                let v = match &r.arg {
                    Some(e) => self.eval(e, scope)?,
                    None => Value::Undefined,
                };
                Err(Ctl::Return(v))
            }
            a::Stmt::If(i) => {
                if self.eval(&i.test, scope)?.truthy() {
                    self.exec(&i.cons, scope)
                } else if let Some(alt) = &i.alt {
                    self.exec(alt, scope)
                } else {
                    Ok(())
                }
            }
            a::Stmt::Throw(t) => Err(Ctl::Throw(self.eval(&t.arg, scope)?)),
            a::Stmt::Break(b) => Err(Ctl::Break(b.label.as_ref().map(|l| l.sym.as_ref().into()))),
            a::Stmt::Continue(c) => Err(Ctl::Continue(
                c.label.as_ref().map(|l| l.sym.as_ref().into()),
            )),
            a::Stmt::Labeled(l) => {
                let label: Rc<str> = l.label.sym.as_ref().into();
                match self.exec_loop_labeled(&l.body, scope, Some(&label)) {
                    Err(Ctl::Break(Some(b))) if b == label => Ok(()),
                    other => other,
                }
            }
            a::Stmt::While(_)
            | a::Stmt::DoWhile(_)
            | a::Stmt::For(_)
            | a::Stmt::ForIn(_)
            | a::Stmt::ForOf(_) => self.exec_loop_labeled(s, scope, None),
            a::Stmt::Switch(sw) => {
                let d = self.eval(&sw.discriminant, scope)?;
                let inner = Scope::new(Some(scope.clone()));
                let mut matched = None;
                for (i, case) in sw.cases.iter().enumerate() {
                    if let Some(t) = &case.test {
                        let tv = self.eval(t, &inner)?;
                        if strict_equals(&d, &tv) {
                            matched = Some(i);
                            break;
                        }
                    }
                }
                let start = matched.or_else(|| sw.cases.iter().position(|c| c.test.is_none()));
                if let Some(start) = start {
                    for case in &sw.cases[start..] {
                        match self.exec_stmts(&case.cons, &inner) {
                            Err(Ctl::Break(None)) => return Ok(()),
                            other => other?,
                        }
                    }
                }
                Ok(())
            }
            a::Stmt::Try(t) => {
                let mut result = self.exec_block(&t.block.stmts, scope);
                if let (Err(Ctl::Throw(v)), Some(h)) = (&result, &t.handler) {
                    let catch_scope = Scope::new(Some(scope.clone()));
                    if let Some(p) = &h.param {
                        self.bind_pattern(p, v.clone(), &catch_scope, true)?;
                    }
                    result = self.exec_stmts(&h.body.stmts, &catch_scope);
                }
                if let Some(f) = &t.finalizer {
                    self.exec_block(&f.stmts, scope)?; // an abrupt finalizer overrides, as in JS
                }
                result
            }
            a::Stmt::With(_) => unsupported("with"),
        }
    }

    fn exec_var(&mut self, v: &a::VarDecl, scope: &Rc<Scope>) -> R<()> {
        let is_var = matches!(v.kind, a::VarDeclKind::Var);
        let mutable = !matches!(v.kind, a::VarDeclKind::Const);
        for d in &v.decls {
            let value = match &d.init {
                Some(e) => self.eval(e, scope)?,
                None if is_var => continue, // hoisted already
                None => Value::Undefined,
            };
            match &d.name {
                a::Pat::Ident(id) if !is_var => scope.declare(&id.id.sym, value, mutable),
                a::Pat::Ident(id) => self.assign_var(scope, &id.id.sym, value)?,
                pat if is_var => self.bind_pattern(pat, value, scope, false)?,
                pat => self.bind_pattern(pat, value, scope, true)?,
            }
        }
        Ok(())
    }

    /// Runs a loop (or a labeled statement) handling `break`/`continue` for `label`.
    fn exec_loop_labeled(
        &mut self,
        s: &a::Stmt,
        scope: &Rc<Scope>,
        label: Option<&Rc<str>>,
    ) -> R<()> {
        // Returns Ok(true) when the loop must stop (break), Ok(false) to go on.
        let step = |r: R<()>| -> R<bool> {
            match r {
                Ok(()) => Ok(false),
                Err(Ctl::Break(None)) => Ok(true),
                Err(Ctl::Continue(None)) => Ok(false),
                Err(Ctl::Continue(Some(l))) if label == Some(&l) => Ok(false),
                Err(e) => Err(e),
            }
        };
        match s {
            a::Stmt::While(w) => {
                while self.eval(&w.test, scope)?.truthy() {
                    if step(self.exec(&w.body, scope))? {
                        break;
                    }
                }
                Ok(())
            }
            a::Stmt::DoWhile(w) => {
                loop {
                    if step(self.exec(&w.body, scope))? || !self.eval(&w.test, scope)?.truthy() {
                        break;
                    }
                }
                Ok(())
            }
            a::Stmt::For(f) => {
                let mut cur = Scope::new(Some(scope.clone()));
                match &f.init {
                    Some(a::VarDeclOrExpr::VarDecl(v)) => self.exec_var(v, &cur)?,
                    Some(a::VarDeclOrExpr::Expr(e)) => {
                        self.eval(e, &cur)?;
                    }
                    None => {}
                }
                loop {
                    if let Some(t) = &f.test {
                        if !self.eval(t, &cur)?.truthy() {
                            break;
                        }
                    }
                    if step(self.exec(&f.body, &cur))? {
                        break;
                    }
                    // fresh per-iteration copy of the loop bindings, so closures capture each iteration
                    cur = cur.copy_of(scope.clone());
                    if let Some(u) = &f.update {
                        self.eval(u, &cur)?;
                    }
                }
                Ok(())
            }
            a::Stmt::ForOf(f) => {
                let iterable = self.eval(&f.right, scope)?;
                if let Some(items) = self.iter_items(&iterable) {
                    for item in items {
                        let item = if f.is_await {
                            self.await_value(item)?
                        } else {
                            item
                        };
                        let iter = Scope::new(Some(scope.clone()));
                        self.bind_for_head(&f.left, item, &iter)?;
                        if step(self.exec(&f.body, &iter))? {
                            break;
                        }
                    }
                    return Ok(());
                }
                // a user-defined iterable or a generator: pulled one value at a time, closed on early exit
                let Some((iterator, next)) = self.get_iterator(&iterable)? else {
                    return unsupported("iterator-protocol");
                };
                loop {
                    let result = self.call(&next, &iterator, &[])?;
                    if !matches!(result, Value::Object(_)) {
                        return self.throw_error("TypeError", "Iterator result is not an object");
                    }
                    if self.get(&result, "done")?.truthy() {
                        break;
                    }
                    let item = self.get(&result, "value")?;
                    let item = if f.is_await {
                        self.await_value(item)?
                    } else {
                        item
                    };
                    let iter = Scope::new(Some(scope.clone()));
                    let outcome = match self.bind_for_head(&f.left, item, &iter) {
                        Ok(()) => step(self.exec(&f.body, &iter)),
                        Err(e) => Err(e),
                    };
                    match outcome {
                        Ok(false) => {}
                        Ok(true) => {
                            self.close_iterator(&iterator);
                            break;
                        }
                        Err(e) => {
                            self.close_iterator(&iterator);
                            return Err(e);
                        }
                    }
                }
                Ok(())
            }
            a::Stmt::ForIn(f) => {
                let target = self.eval(&f.right, scope)?;
                // own enumerable keys, then inherited enumerable ones (shadowed names once)
                let mut keys = self.own_keys(&target);
                if let Value::Object(o) = &target {
                    let mut cur = o.proto.borrow().clone();
                    while let Some(c) = cur {
                        for p in c.props.borrow().iter().filter(|p| p.enumerable) {
                            if !keys.contains(&p.key) {
                                keys.push(p.key.clone());
                            }
                        }
                        cur = c.proto.borrow().clone();
                    }
                }
                for key in keys {
                    let iter = Scope::new(Some(scope.clone()));
                    self.bind_for_head(&f.left, Value::Str(key), &iter)?;
                    if step(self.exec(&f.body, &iter))? {
                        break;
                    }
                }
                Ok(())
            }
            other => self.exec(other, scope),
        }
    }

    fn bind_for_head(&mut self, head: &a::ForHead, value: Value, scope: &Rc<Scope>) -> R<()> {
        match head {
            a::ForHead::VarDecl(v) => match v.decls.first().map(|d| &d.name) {
                Some(p) if matches!(v.kind, a::VarDeclKind::Var) => {
                    self.bind_pattern(p, value, scope, false)
                }
                Some(p) => self.bind_pattern(p, value, scope, true),
                None => unsupported("for-head"),
            },
            a::ForHead::Pat(p) => self.bind_pattern(p, value, scope, false),
            _ => unsupported("for-head"),
        }
    }

    // ----- expressions -----

    fn eval_args(&mut self, args: &[a::ExprOrSpread], scope: &Rc<Scope>) -> R<Vec<Value>> {
        let mut out = Vec::with_capacity(args.len());
        for arg in args {
            let v = self.eval(&arg.expr, scope)?;
            if arg.spread.is_some() {
                match self.iter_protocol(&v)? {
                    Some(items) => out.extend(items),
                    None => return unsupported("iterator-protocol"),
                }
            } else {
                out.push(v);
            }
        }
        Ok(out)
    }

    fn member_key(&mut self, prop: &a::MemberProp, scope: &Rc<Scope>) -> R<Rc<str>> {
        match prop {
            a::MemberProp::Ident(i) => Ok(i.sym.as_ref().into()),
            a::MemberProp::Computed(c) => {
                let v = self.eval(&c.expr, scope)?;
                Ok(self.key_of(&v))
            }
            a::MemberProp::PrivateName(_) => unsupported("private-name"),
        }
    }

    fn eval(&mut self, e: &a::Expr, scope: &Rc<Scope>) -> R<Value> {
        match e {
            a::Expr::Lit(l) => Ok(match l {
                a::Lit::Num(n) => Value::Number(n.value),
                a::Lit::Str(s) => Value::str(s.value.as_str().unwrap_or("")), // ponytail: lone surrogates lost
                a::Lit::Bool(b) => Value::Bool(b.value),
                a::Lit::Null(_) => Value::Null,
                a::Lit::Regex(r) => return self.new_regexp(&r.exp, &r.flags),
                _ => return unsupported("bigint"),
            }),
            a::Expr::Ident(i) => match self.lookup(scope, &i.sym) {
                Some(v) => Ok(v),
                // the global object inherits from Object.prototype (`toString`, `hasOwnProperty`, ...)
                None if matches!(
                    find_in_chain(&self.object_proto, &i.sym),
                    Some(Slot::Data(_))
                ) =>
                {
                    match find_in_chain(&self.object_proto, &i.sym) {
                        Some(Slot::Data(v)) => Ok(v),
                        _ => unreachable!(),
                    }
                }
                None => self.throw_error("ReferenceError", &format!("{} is not defined", &*i.sym)),
            },
            a::Expr::This(_) => Ok(self.lookup(scope, "this").unwrap_or(Value::Undefined)),
            a::Expr::Paren(p) => self.eval(&p.expr, scope),
            a::Expr::Seq(s) => {
                let mut last = Value::Undefined;
                for e in &s.exprs {
                    last = self.eval(e, scope)?;
                }
                Ok(last)
            }
            a::Expr::Tpl(t) => {
                let mut s = String::new();
                for (i, q) in t.quasis.iter().enumerate() {
                    s.push_str(q.cooked.as_ref().and_then(|c| c.as_str()).unwrap_or(""));
                    if let Some(e) = t.exprs.get(i) {
                        let v = self.eval(e, scope)?;
                        s.push_str(&self.to_str(&v));
                    }
                }
                Ok(Value::Str(s.into()))
            }
            a::Expr::Array(arr) => {
                let mut items = Vec::new();
                for el in &arr.elems {
                    match el {
                        Some(el) => items.extend(self.eval_args(std::slice::from_ref(el), scope)?),
                        None => items.push(Value::Undefined),
                    }
                }
                Ok(self.array(items))
            }
            a::Expr::Object(o) => {
                let obj = self.plain();
                for p in &o.props {
                    match p {
                        a::PropOrSpread::Spread(sp) => {
                            let src = self.eval(&sp.expr, scope)?;
                            for k in self.own_keys(&src) {
                                let v = self.get(&src, &k)?;
                                put(&obj, &k, v);
                            }
                        }
                        a::PropOrSpread::Prop(p) => match &**p {
                            a::Prop::KeyValue(kv) => {
                                let k = self.prop_name(&kv.key, scope)?;
                                let v = self.eval(&kv.value, scope)?;
                                put(&obj, &k, v);
                            }
                            a::Prop::Shorthand(i) => {
                                let v = self.eval(&a::Expr::Ident(i.clone()), scope)?;
                                put(&obj, &i.sym, v);
                            }
                            a::Prop::Method(m) => {
                                let k = self.prop_name(&m.key, scope)?;
                                let f = self.method_value(Some(k.clone()), &m.function, scope)?;
                                put(&obj, &k, f);
                            }
                            a::Prop::Getter(g) => {
                                let k = self.prop_name(&g.key, scope)?;
                                let f = self.method_value(Some(k.clone()), &g.function, scope)?;
                                define_accessor(&obj, &k, Some(f), None, true);
                            }
                            a::Prop::Setter(s) => {
                                let k = self.prop_name(&s.key, scope)?;
                                let f = self.method_value(Some(k.clone()), &s.function, scope)?;
                                define_accessor(&obj, &k, None, Some(f), true);
                            }
                            a::Prop::Assign(_) => return unsupported("shorthand default"),
                        },
                    }
                }
                Ok(Value::Object(obj))
            }
            a::Expr::Fn(f) => {
                let name = f.ident.as_ref().map(|i| Rc::<str>::from(i.sym.as_ref()));
                self.function_value(name, &f.function, scope)
            }
            a::Expr::Arrow(f) => {
                if f.is_generator {
                    return unsupported("generator");
                }
                let body = match &*f.body {
                    a::ArrowFunctionBody::FunctionBody(b) => Body::Block(b.stmts.clone()),
                    a::ArrowFunctionBody::Expr(e) => Body::Expr(e.clone()),
                };
                Ok(self.make_function(
                    None,
                    f.params.clone(),
                    body,
                    true,
                    f.is_async,
                    false,
                    f.span.lo.0,
                    scope,
                ))
            }
            a::Expr::Member(m) => {
                let obj = self.eval(&m.obj, scope)?;
                let key = self.member_key(&m.prop, scope)?;
                self.get(&obj, &key)
            }
            a::Expr::Call(c) => {
                let a::Callee::Expr(callee) = &c.callee else {
                    if matches!(c.callee, a::Callee::Super(_)) {
                        let args = self.eval_args(&c.args, scope)?;
                        return self.super_call(&args, scope);
                    }
                    return unsupported("import()");
                };
                let (f, this) = match &**callee {
                    a::Expr::SuperProp(_) => (
                        self.eval(callee, scope)?,
                        self.lookup(scope, "this").unwrap_or(Value::Undefined),
                    ),
                    a::Expr::Member(m) => {
                        let obj = self.eval(&m.obj, scope)?;
                        let key = self.member_key(&m.prop, scope)?;
                        (self.get(&obj, &key)?, obj)
                    }
                    other => (self.eval(other, scope)?, Value::Undefined),
                };
                let args = self.eval_args(&c.args, scope)?;
                if !f.is_callable() {
                    return self.throw_error(
                        "TypeError",
                        &format!("{} is not a function", expr_text(callee)),
                    );
                }
                self.call(&f, &this, &args)
            }
            a::Expr::New(n) => {
                let f = self.eval(&n.callee, scope)?;
                let args = match &n.args {
                    Some(args) => self.eval_args(args, scope)?,
                    None => Vec::new(),
                };
                if !f.is_callable() {
                    return self.throw_error(
                        "TypeError",
                        &format!("{} is not a constructor", expr_text(&n.callee)),
                    );
                }
                self.construct(&f, &args)
            }
            a::Expr::Cond(c) => {
                if self.eval(&c.test, scope)?.truthy() {
                    self.eval(&c.cons, scope)
                } else {
                    self.eval(&c.alt, scope)
                }
            }
            a::Expr::Unary(u) => self.eval_unary(u, scope),
            a::Expr::Update(u) => {
                let old = self.eval(&u.arg, scope)?;
                let old = self.num(&old);
                let new = if matches!(u.op, a::UpdateOp::PlusPlus) {
                    old + 1.0
                } else {
                    old - 1.0
                };
                self.store(&u.arg, Value::Number(new), scope)?;
                Ok(Value::Number(if u.prefix { new } else { old }))
            }
            a::Expr::Bin(b) => match b.op {
                a::BinaryOp::LogicalAnd => {
                    let l = self.eval(&b.left, scope)?;
                    if l.truthy() {
                        self.eval(&b.right, scope)
                    } else {
                        Ok(l)
                    }
                }
                a::BinaryOp::LogicalOr => {
                    let l = self.eval(&b.left, scope)?;
                    if l.truthy() {
                        Ok(l)
                    } else {
                        self.eval(&b.right, scope)
                    }
                }
                a::BinaryOp::NullishCoalescing => {
                    let l = self.eval(&b.left, scope)?;
                    if matches!(l, Value::Undefined | Value::Null) {
                        self.eval(&b.right, scope)
                    } else {
                        Ok(l)
                    }
                }
                op => {
                    let l = self.eval(&b.left, scope)?;
                    let r = self.eval(&b.right, scope)?;
                    self.binary(op, &l, &r)
                }
            },
            a::Expr::Assign(asg) => self.eval_assign(asg, scope),
            a::Expr::Class(c) => self.eval_class(c.ident.as_ref(), &c.class, scope),
            a::Expr::OptChain(oc) => Ok(self.eval_opt(oc, scope)?.unwrap_or(Value::Undefined)),
            a::Expr::SuperProp(sp) => {
                let base = self.super_base(scope)?;
                let key: Rc<str> = match &sp.prop {
                    a::SuperProp::Ident(i) => i.sym.as_ref().into(),
                    a::SuperProp::Computed(c) => {
                        let v = self.eval(&c.expr, scope)?;
                        self.key_of(&v)
                    }
                };
                // accessors on the base run with the current `this`
                let this = self.lookup(scope, "this").unwrap_or(Value::Undefined);
                match &base {
                    Value::Object(o) => match find_in_chain(o, &key) {
                        Some(Slot::Data(v)) => Ok(v),
                        Some(Slot::Accessor { get: Some(g), .. }) => self.call(&g, &this, &[]),
                        _ => Ok(Value::Undefined),
                    },
                    _ => Ok(Value::Undefined),
                }
            }
            a::Expr::TaggedTpl(t) => {
                let (f, this) = match &*t.tag {
                    a::Expr::Member(m) => {
                        let obj = self.eval(&m.obj, scope)?;
                        let key = self.member_key(&m.prop, scope)?;
                        (self.get(&obj, &key)?, obj)
                    }
                    other => (self.eval(other, scope)?, Value::Undefined),
                };
                let cooked = t
                    .tpl
                    .quasis
                    .iter()
                    .map(|q| match q.cooked.as_ref().and_then(|c| c.as_str()) {
                        Some(c) => Value::str(c),
                        None => Value::Undefined,
                    })
                    .collect();
                let raw = t.tpl.quasis.iter().map(|q| Value::str(&q.raw)).collect();
                let strings = self.array(cooked);
                if let Value::Object(o) = &strings {
                    let raw = self.array(raw);
                    put_hidden(o, "raw", raw);
                }
                let mut args = vec![strings];
                for e in &t.tpl.exprs {
                    args.push(self.eval(e, scope)?);
                }
                if !f.is_callable() {
                    return self.throw_error(
                        "TypeError",
                        &format!("{} is not a function", expr_text(&t.tag)),
                    );
                }
                self.call(&f, &this, &args)
            }
            a::Expr::MetaProp(_) => unsupported("new.target"),
            a::Expr::Yield(_) => unsupported("yield"),
            a::Expr::Await(e) => {
                let v = self.eval(&e.arg, scope)?;
                self.await_value(v)
            }
            _ => unsupported("expression"),
        }
    }

    pub(crate) fn prop_name(&mut self, key: &a::PropName, scope: &Rc<Scope>) -> R<Rc<str>> {
        Ok(match key {
            a::PropName::Ident(i) => i.sym.as_ref().into(),
            a::PropName::Str(s) => s.value.as_str().unwrap_or("").into(),
            a::PropName::Num(n) => number_to_string(n.value).into(),
            a::PropName::Computed(c) => {
                let v = self.eval(&c.expr, scope)?;
                self.key_of(&v)
            }
            _ => return unsupported("property name"),
        })
    }

    fn eval_unary(&mut self, u: &a::UnaryExpr, scope: &Rc<Scope>) -> R<Value> {
        match u.op {
            a::UnaryOp::TypeOf => {
                // typeof of an undeclared identifier must not throw
                let v = match &*u.arg {
                    a::Expr::Ident(i) => self.lookup(scope, &i.sym).unwrap_or(Value::Undefined),
                    other => self.eval(other, scope)?,
                };
                Ok(Value::str(match &v {
                    Value::Undefined => "undefined",
                    Value::Null => "object",
                    Value::Bool(_) => "boolean",
                    Value::Number(_) => "number",
                    Value::Str(_) => "string",
                    Value::Object(_) if v.is_callable() => "function",
                    Value::Object(o) if matches!(&*o.kind.borrow(), Kind::Symbol { .. }) => {
                        "symbol"
                    }
                    Value::Object(_) => "object",
                }))
            }
            a::UnaryOp::Delete => {
                let a::Expr::Member(m) = &*u.arg else {
                    return Ok(Value::Bool(true));
                };
                let obj = self.eval(&m.obj, scope)?;
                let key = self.member_key(&m.prop, scope)?;
                if let Value::Object(o) = obj {
                    if !o.frozen.get() {
                        o.props.borrow_mut().retain(|p| *p.key != *key);
                    }
                }
                Ok(Value::Bool(true))
            }
            _ => {
                let v = self.eval(&u.arg, scope)?;
                Ok(match u.op {
                    a::UnaryOp::Bang => Value::Bool(!v.truthy()),
                    a::UnaryOp::Minus => Value::Number(-self.num(&v)),
                    a::UnaryOp::Plus => Value::Number(self.num(&v)),
                    a::UnaryOp::Tilde => Value::Number(!to_int32(self.num(&v)) as f64),
                    a::UnaryOp::Void => Value::Undefined,
                    _ => return unsupported("unary operator"),
                })
            }
        }
    }

    fn eval_assign(&mut self, asg: &a::AssignExpr, scope: &Rc<Scope>) -> R<Value> {
        use a::AssignOp as A;
        let target = match &asg.left {
            a::AssignTarget::Simple(t) => t,
            a::AssignTarget::Pat(p) => {
                let value = self.eval(&asg.right, scope)?;
                let pat = match p {
                    a::AssignTargetPat::Array(ap) => a::Pat::Array(ap.clone()),
                    a::AssignTargetPat::Object(op) => a::Pat::Object(op.clone()),
                    a::AssignTargetPat::Invalid(_) => return unsupported("assignment target"),
                };
                self.bind_pattern(&pat, value.clone(), scope, false)?;
                return Ok(value);
            }
        };
        let expr = match target {
            a::SimpleAssignTarget::Ident(i) => a::Expr::Ident(i.id.clone()),
            a::SimpleAssignTarget::Member(m) => a::Expr::Member(m.clone()),
            _ => return unsupported("assignment target"),
        };
        let value = match asg.op {
            A::Assign => self.eval(&asg.right, scope)?,
            A::AndAssign | A::OrAssign | A::NullishAssign => {
                let cur = self.eval(&expr, scope)?;
                let keep = match asg.op {
                    A::AndAssign => !cur.truthy(),
                    A::OrAssign => cur.truthy(),
                    _ => !matches!(cur, Value::Undefined | Value::Null),
                };
                if keep {
                    return Ok(cur);
                }
                self.eval(&asg.right, scope)?
            }
            op => {
                let cur = self.eval(&expr, scope)?;
                let rhs = self.eval(&asg.right, scope)?;
                let bop = match op {
                    A::AddAssign => a::BinaryOp::Add,
                    A::SubAssign => a::BinaryOp::Sub,
                    A::MulAssign => a::BinaryOp::Mul,
                    A::DivAssign => a::BinaryOp::Div,
                    A::ModAssign => a::BinaryOp::Mod,
                    A::ExpAssign => a::BinaryOp::Exp,
                    A::LShiftAssign => a::BinaryOp::LShift,
                    A::RShiftAssign => a::BinaryOp::RShift,
                    A::ZeroFillRShiftAssign => a::BinaryOp::ZeroFillRShift,
                    A::BitOrAssign => a::BinaryOp::BitOr,
                    A::BitXorAssign => a::BinaryOp::BitXor,
                    A::BitAndAssign => a::BinaryOp::BitAnd,
                    _ => return unsupported("assignment operator"),
                };
                self.binary(bop, &cur, &rhs)?
            }
        };
        self.store(&expr, value.clone(), scope)?;
        Ok(value)
    }

    fn store(&mut self, target: &a::Expr, value: Value, scope: &Rc<Scope>) -> R<()> {
        match target {
            a::Expr::Ident(i) => self.assign_var(scope, &i.sym, value),
            a::Expr::Member(m) => {
                let obj = self.eval(&m.obj, scope)?;
                let key = self.member_key(&m.prop, scope)?;
                self.set(&obj, &key, value)
            }
            a::Expr::Paren(p) => self.store(&p.expr, value, scope),
            _ => unsupported("assignment target"),
        }
    }

    fn binary(&mut self, op: a::BinaryOp, l: &Value, r: &Value) -> R<Value> {
        use a::BinaryOp as B;
        // objects take part in arithmetic / comparison through ToPrimitive
        let (lp0, rp0);
        let (l, r) = if matches!(l, Value::Object(_)) || matches!(r, Value::Object(_)) {
            let hint = if matches!(op, B::Add) {
                extras::Hint::Default
            } else {
                extras::Hint::Number
            };
            if matches!(
                op,
                B::Add
                    | B::Sub
                    | B::Mul
                    | B::Div
                    | B::Mod
                    | B::Exp
                    | B::Lt
                    | B::LtEq
                    | B::Gt
                    | B::GtEq
                    | B::BitAnd
                    | B::BitOr
                    | B::BitXor
                    | B::LShift
                    | B::RShift
                    | B::ZeroFillRShift
            ) {
                lp0 = self.primitive_of(l, hint)?;
                rp0 = self.primitive_of(r, hint)?;
                (&lp0, &rp0)
            } else {
                (l, r)
            }
        } else {
            (l, r)
        };
        let num = |it: &mut Interp| (it.num(l), it.num(r));
        Ok(match op {
            B::Add => {
                let (lp, rp) = (l.clone(), r.clone());
                if matches!(lp, Value::Str(_)) || matches!(rp, Value::Str(_)) {
                    let mut s = self.to_str(&lp).to_string();
                    s.push_str(&self.to_str(&rp));
                    Value::Str(s.into())
                } else {
                    Value::Number(self.num(&lp) + self.num(&rp))
                }
            }
            B::Sub => {
                let (x, y) = num(self);
                Value::Number(x - y)
            }
            B::Mul => {
                let (x, y) = num(self);
                Value::Number(x * y)
            }
            B::Div => {
                let (x, y) = num(self);
                Value::Number(x / y)
            }
            B::Mod => {
                let (x, y) = num(self);
                Value::Number(x % y)
            }
            B::Exp => {
                let (x, y) = num(self);
                Value::Number(x.powf(y))
            }
            B::BitAnd => {
                let (x, y) = num(self);
                Value::Number((to_int32(x) & to_int32(y)) as f64)
            }
            B::BitOr => {
                let (x, y) = num(self);
                Value::Number((to_int32(x) | to_int32(y)) as f64)
            }
            B::BitXor => {
                let (x, y) = num(self);
                Value::Number((to_int32(x) ^ to_int32(y)) as f64)
            }
            B::LShift => {
                let (x, y) = num(self);
                Value::Number(to_int32(x).wrapping_shl(to_int32(y) as u32 & 31) as f64)
            }
            B::RShift => {
                let (x, y) = num(self);
                Value::Number((to_int32(x) >> (to_int32(y) as u32 & 31)) as f64)
            }
            B::ZeroFillRShift => {
                let (x, y) = num(self);
                Value::Number(((to_int32(x) as u32) >> (to_int32(y) as u32 & 31)) as f64)
            }
            B::EqEqEq => Value::Bool(strict_equals(l, r)),
            B::NotEqEq => Value::Bool(!strict_equals(l, r)),
            B::EqEq => Value::Bool(self.loose_equals(l, r)),
            B::NotEq => Value::Bool(!self.loose_equals(l, r)),
            B::Lt | B::LtEq | B::Gt | B::GtEq => {
                if let (Value::Str(x), Value::Str(y)) = (l, r) {
                    // ponytail: compares by code point; JS compares UTF-16 code units
                    let ord = x.cmp(y);
                    return Ok(Value::Bool(match op {
                        B::Lt => ord.is_lt(),
                        B::LtEq => ord.is_le(),
                        B::Gt => ord.is_gt(),
                        _ => ord.is_ge(),
                    }));
                }
                let (x, y) = num(self);
                Value::Bool(match op {
                    B::Lt => x < y,
                    B::LtEq => x <= y,
                    B::Gt => x > y,
                    _ => x >= y,
                })
            }
            B::InstanceOf => {
                let proto = self.get(r, "prototype")?;
                let (Value::Object(o), Value::Object(p)) = (l, &proto) else {
                    return Ok(Value::Bool(false));
                };
                let mut cur = o.proto.borrow().clone();
                while let Some(c) = cur {
                    if Rc::ptr_eq(&c, p) {
                        return Ok(Value::Bool(true));
                    }
                    cur = c.proto.borrow().clone();
                }
                Value::Bool(false)
            }
            B::In => {
                let key = self.key_of(l);
                let Value::Object(o) = r else {
                    return self
                        .throw_error("TypeError", "Cannot use 'in' operator on a non-object");
                };
                let is_index = matches!(&*o.kind.borrow(), Kind::Array(items) if key.parse::<usize>().is_ok_and(|i| i < items.len()));
                Value::Bool(is_index || find_in_chain(o, &key).is_some())
            }
            _ => return unsupported("binary operator"),
        })
    }

    fn loose_equals(&mut self, l: &Value, r: &Value) -> bool {
        match (l, r) {
            (Value::Undefined | Value::Null, Value::Undefined | Value::Null) => true,
            (Value::Undefined | Value::Null, _) | (_, Value::Undefined | Value::Null) => false,
            (Value::Object(_), Value::Object(_)) => strict_equals(l, r),
            (Value::Str(_), Value::Str(_)) => strict_equals(l, r),
            (Value::Object(_), _) | (_, Value::Object(_)) => {
                let (lp, rp) = match (
                    self.primitive_of(l, extras::Hint::Default),
                    self.primitive_of(r, extras::Hint::Default),
                ) {
                    (Ok(x), Ok(y)) => (x, y),
                    _ => return false,
                };
                self.loose_equals(&lp, &rp)
            }
            _ => {
                let (x, y) = (self.num(l), self.num(r));
                x == y
            }
        }
    }
}

impl Default for Interp {
    fn default() -> Self {
        Interp::new()
    }
}

/// Source-like text of a callee for error messages (`a.b.c`), as V8 prints it.
fn expr_text(e: &a::Expr) -> String {
    match e {
        a::Expr::Ident(i) => i.sym.to_string(),
        a::Expr::This(_) => "this".into(),
        a::Expr::Paren(p) => expr_text(&p.expr),
        a::Expr::Member(m) => match &m.prop {
            a::MemberProp::Ident(p) => format!("{}.{}", expr_text(&m.obj), &*p.sym),
            _ => format!("{}[...]", expr_text(&m.obj)),
        },
        _ => "(intermediate value)".into(),
    }
}

fn fn_pats(params: &[a::Param]) -> Vec<a::Pat> {
    params.iter().map(|p| p.pat.clone()).collect()
}

pub(crate) fn arg(args: &[Value], i: usize) -> Value {
    args.get(i).cloned().unwrap_or(Value::Undefined)
}

fn find_own(o: &Obj, key: &str) -> Option<(Slot, bool)> {
    o.props
        .borrow()
        .iter()
        .find(|p| &*p.key == key)
        .map(|p| (p.slot.clone(), p.enumerable))
}

fn find_in_chain(o: &Obj, key: &str) -> Option<Slot> {
    let mut cur = Some(o.clone());
    while let Some(c) = cur {
        if let Some((s, _)) = find_own(&c, key) {
            return Some(s);
        }
        cur = c.proto.borrow().clone();
    }
    None
}

/// Create or overwrite an enumerable, writable data property.
pub(crate) fn put(o: &Obj, key: &str, value: Value) {
    let mut props = o.props.borrow_mut();
    match props.iter_mut().find(|p| &*p.key == key) {
        Some(p) => p.slot = Slot::Data(value),
        None => props.push(Prop {
            key: key.into(),
            slot: Slot::Data(value),
            enumerable: true,
            writable: true,
        }),
    }
}

/// Create or overwrite a non-enumerable, writable data property (builtins).
pub(crate) fn put_hidden(o: &Obj, key: &str, value: Value) {
    let mut props = o.props.borrow_mut();
    match props.iter_mut().find(|p| &*p.key == key) {
        Some(p) => {
            p.slot = Slot::Data(value);
            p.enumerable = false;
        }
        None => props.push(Prop {
            key: key.into(),
            slot: Slot::Data(value),
            enumerable: false,
            writable: true,
        }),
    }
}

pub(crate) fn define_accessor(
    o: &Obj,
    key: &str,
    get: Option<Value>,
    set: Option<Value>,
    enumerable: bool,
) {
    let mut props = o.props.borrow_mut();
    match props.iter_mut().find(|p| &*p.key == key) {
        Some(p) => {
            if let Slot::Accessor { get: g, set: s } = &mut p.slot {
                if get.is_some() {
                    *g = get;
                }
                if set.is_some() {
                    *s = set;
                }
            } else {
                p.slot = Slot::Accessor { get, set };
            }
            p.enumerable = enumerable;
        }
        None => props.push(Prop {
            key: key.into(),
            slot: Slot::Accessor { get, set },
            enumerable,
            writable: true,
        }),
    }
}

/// Declare every `var` in `stmts` (not crossing function boundaries) as `undefined` in `scope`.
fn hoist_vars(stmts: &[a::Stmt], scope: &Rc<Scope>) {
    struct Hoister<'a>(&'a Rc<Scope>);
    impl Visit for Hoister<'_> {
        fn visit_function(&mut self, _: &a::Function) {}
        fn visit_arrow_expr(&mut self, _: &a::ArrowExpr) {}
        fn visit_class(&mut self, _: &a::Class) {}
        fn visit_var_decl(&mut self, n: &a::VarDecl) {
            if matches!(n.kind, a::VarDeclKind::Var) {
                for d in &n.decls {
                    let mut names = Names(Vec::new());
                    d.name.visit_with(&mut names);
                    for name in names.0 {
                        if !self.0.vars.borrow().contains_key(&*name) {
                            self.0.declare(&name, Value::Undefined, true);
                        }
                    }
                }
            }
            n.visit_children_with(self);
        }
    }
    struct Names(Vec<String>);
    impl Visit for Names {
        fn visit_binding_ident(&mut self, n: &a::BindingIdent) {
            self.0.push(n.id.sym.to_string());
        }
        fn visit_assign_pat_prop(&mut self, n: &a::AssignPatProp) {
            self.0.push(n.key.id.sym.to_string());
        }
        fn visit_prop_name(&mut self, _: &a::PropName) {}
        fn visit_expr(&mut self, _: &a::Expr) {}
    }
    stmts.visit_with(&mut Hoister(scope));
}

#[cfg(test)]
mod tests;
