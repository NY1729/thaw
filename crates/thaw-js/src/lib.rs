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

mod bigint;
mod buffer;
mod builtins;
mod classes;
mod collections;
mod date;
mod esm;
mod eventloop;
mod extras;
mod generators;
pub mod host;
#[cfg(feature = "intl")]
mod intl;
mod json;
mod numfmt;
mod proxy;
pub mod scan;
mod sysio;
mod typed;
mod url;
#[path = "web/abort.rs"]
mod web_abort;
#[path = "web/blob.rs"]
mod web_blob;
#[path = "web/crypto.rs"]
mod web_crypto;
#[path = "web/dom_exception.rs"]
mod web_dom_exception;
#[path = "web/event_target.rs"]
mod web_event_target;
#[path = "web/fetch.rs"]
mod web_fetch;
#[path = "web/streams.rs"]
mod web_streams;
#[path = "web/text_codec.rs"]
mod web_text_codec;

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
    "generator",
    "yield",
    "bigint",
    "private-name",
    "new.target",
];

const MAX_DEPTH: usize = 100;

#[derive(Clone)]
pub enum Value {
    Undefined,
    Null,
    Bool(bool),
    Number(f64),
    Str(Rc<str>),
    BigInt(Rc<num_bigint::BigInt>),
    Object(Obj),
    /// An empty array slot (`[1,,3]`, `new Array(3)`, `delete a[i]`). Only ever stored inside
    /// `Kind::Array`; every read path maps it to `undefined`.
    Hole,
}

pub type Obj = Rc<ObjData>;

/// An array slot as seen by user code: holes read as `undefined`.
pub(crate) fn unhole(v: Option<&Value>) -> Value {
    match v {
        None | Some(Value::Hole) => Value::Undefined,
        Some(v) => v.clone(),
    }
}

pub struct ObjData {
    props: RefCell<Vec<Prop>>,
    proto: RefCell<Option<Obj>>,
    kind: RefCell<Kind>,
    frozen: Cell<bool>,
    /// 0 extensible, 1 non-extensible (`preventExtensions`), 2 sealed.
    ext: Cell<u8>,
    /// Key -> position in `props`, built lazily for objects with many properties (`None` = stale).
    index: RefCell<Option<FxMap<Rc<str>, usize>>>,
}

/// Objects with fewer properties than this are searched linearly.
const INDEX_MIN: usize = 10;

impl ObjData {
    /// Position of own property `key`.
    fn position(&self, key: &str) -> Option<usize> {
        let props = self.props.borrow();
        if props.len() < INDEX_MIN {
            return props.iter().position(|p| &*p.key == key);
        }
        let mut idx = self.index.borrow_mut();
        let map = idx.get_or_insert_with(|| {
            props
                .iter()
                .enumerate()
                .map(|(i, p)| (p.key.clone(), i))
                .collect()
        });
        map.get(key).copied()
    }

    /// Remove own property `key`, if present.
    pub(crate) fn remove_prop(&self, key: &str) {
        self.props.borrow_mut().retain(|p| *p.key != *key);
        *self.index.borrow_mut() = None;
    }
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
    Map(collections::MapData),
    Set(collections::SetData),
    Symbol {
        id: u64,
        desc: Option<Rc<str>>,
    },
    Promise(eventloop::PromiseData),
    Generator(Rc<generators::GenData>),
    Proxy(Rc<proxy::ProxyData>),
    ArrayBuffer(typed::Bytes),
    Typed(typed::TypedArr),
    /// `new Number(1)` / `new String('a')` / `new Boolean(true)`.
    Wrapper(Value),
}

type NativeFn = Rc<dyn Fn(&mut Interp, &Value, &[Value]) -> R<Value>>;

struct FuncDef {
    name: Option<Rc<str>>,
    params: Vec<a::Pat>,
    body: Body,
    arrow: bool,
    is_async: bool,
    is_gen: bool,
    /// Named function expressions see their own name; methods do not.
    bind_self: bool,
    uses_arguments: bool,
    uses_new_target: bool,
    /// The body (or an arrow inside it) reads `this`; other calls skip the binding.
    uses_this: bool,
    /// `(run_id, start, end)` of the function's text, for `Function.prototype.toString`.
    src: (u32, u32, u32),
    /// `var` names declared in the body (hoisted once, not rediscovered on every call).
    hoisted: Vec<Rc<str>>,
    /// Name of each plain-identifier parameter (`None` for patterns / rest), allocated once.
    param_names: Vec<Option<Rc<str>>>,
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

/// A resolved assignment / update / read-modify-write target.
enum Loc<'a> {
    Ident(&'a a::Ident),
    Prop(Value, Key<'a>),
    /// `obj[i]` with a non-negative integer `i`.
    Index(Value, usize),
}

/// A property key: borrowed from the syntax tree for `obj.name`, owned when computed.
pub(crate) enum Key<'a> {
    Borrowed(&'a str),
    Owned(Rc<str>),
}

impl std::ops::Deref for Key<'_> {
    type Target = str;
    fn deref(&self) -> &str {
        match self {
            Key::Borrowed(s) => s,
            Key::Owned(r) => r,
        }
    }
}

/// FxHash: the identifier keys are short and not attacker-chosen, SipHash is pure overhead here.
#[derive(Default, Clone, Copy)]
pub(crate) struct FxHasher(u64);

impl std::hash::Hasher for FxHasher {
    fn write(&mut self, bytes: &[u8]) {
        const K: u64 = 0x517c_c1b7_2722_0a95;
        let (chunks, remainder) = bytes.as_chunks::<8>();
        for c in chunks {
            let w = u64::from_le_bytes(*c);
            self.0 = (self.0.rotate_left(5) ^ w).wrapping_mul(K);
        }
        for &b in remainder {
            self.0 = (self.0.rotate_left(5) ^ b as u64).wrapping_mul(K);
        }
    }
    fn finish(&self) -> u64 {
        self.0
    }
}

pub(crate) type FxMap<K, V> = HashMap<K, V, std::hash::BuildHasherDefault<FxHasher>>;

/// The variables of one scope: a short list searched linearly, indexed once it grows.
#[derive(Default)]
struct Vars {
    list: Vec<(Rc<str>, Binding)>,
    index: Option<FxMap<Rc<str>, usize>>,
}

/// Scopes with at least this many variables get a hash index.
const VARS_INDEX_MIN: usize = 16;

impl Vars {
    fn position(&self, key: &str) -> Option<usize> {
        match &self.index {
            Some(map) => map.get(key).copied(),
            None => self.list.iter().position(|(name, _)| &**name == key),
        }
    }
    fn get(&self, key: &str) -> Option<&Binding> {
        self.position(key).map(|i| &self.list[i].1)
    }
    fn get_mut(&mut self, key: &str) -> Option<&mut Binding> {
        let i = self.position(key)?;
        Some(&mut self.list[i].1)
    }
    fn contains_key(&self, key: &str) -> bool {
        self.position(key).is_some()
    }
    fn insert(&mut self, key: Rc<str>, binding: Binding) {
        if let Some(i) = self.position(&key) {
            self.list[i].1 = binding;
            return;
        }
        self.push_new(key, binding);
    }
    /// Append a key known to be absent.
    fn push_new(&mut self, key: Rc<str>, binding: Binding) {
        let i = self.list.len();
        if let Some(map) = &mut self.index {
            map.insert(key.clone(), i);
        }
        self.list.push((key, binding));
        if self.index.is_none() && self.list.len() >= VARS_INDEX_MIN {
            self.index = Some(
                self.list
                    .iter()
                    .enumerate()
                    .map(|(i, (name, _))| (name.clone(), i))
                    .collect(),
            );
        }
    }
    fn keys(&self) -> impl Iterator<Item = &Rc<str>> {
        self.list.iter().map(|(name, _)| name)
    }
    fn values(&self) -> impl Iterator<Item = &Binding> {
        self.list.iter().map(|(_, binding)| binding)
    }
    fn iter(&self) -> impl Iterator<Item = (&Rc<str>, &Binding)> {
        self.list.iter().map(|(name, binding)| (name, binding))
    }
}

/// One bit of a scope's name filter: a lookup skips every scope whose filter lacks the name's bit.
fn name_bit(name: &str) -> u64 {
    use std::hash::Hasher;
    let mut h = FxHasher::default();
    h.write(name.as_bytes());
    1u64 << (h.finish() >> 58)
}

pub struct Scope {
    vars: RefCell<Vars>,
    /// Union of `name_bit` over the declared names (no false negatives).
    mask: Cell<u64>,
    parent: Option<Rc<Scope>>,
    strict: Cell<bool>,
}

impl Scope {
    fn new(parent: Option<Rc<Scope>>) -> Rc<Scope> {
        let strict = parent.as_ref().is_some_and(|p| p.strict.get());
        Rc::new(Scope {
            vars: RefCell::new(Vars::default()),
            mask: Cell::new(0),
            parent,
            strict: Cell::new(strict),
        })
    }
    fn declare(&self, name: &str, value: Value, mutable: bool) {
        self.declare_rc(name.into(), value, mutable);
    }
    fn declare_rc(&self, name: Rc<str>, value: Value, mutable: bool) {
        let bit = name_bit(&name);
        let maybe_present = self.mask.get() & bit != 0;
        self.mask.set(self.mask.get() | bit);
        let binding = Binding { value, mutable };
        let mut vars = self.vars.borrow_mut();
        if maybe_present {
            vars.insert(name, binding);
        } else {
            vars.push_new(name, binding);
        }
    }
    /// Declare `name` as `undefined` unless this scope already has it (a hoisted `var`).
    fn declare_if_absent(&self, name: &Rc<str>) {
        let bit = name_bit(name);
        if self.mask.get() & bit != 0 && self.vars.borrow().contains_key(name) {
            return;
        }
        self.mask.set(self.mask.get() | bit);
        self.vars.borrow_mut().push_new(
            name.clone(),
            Binding {
                value: Value::Undefined,
                mutable: true,
            },
        );
    }
    fn copy_of(&self, parent: Rc<Scope>) -> Rc<Scope> {
        let mut vars = Vars::default();
        for (k, b) in self.vars.borrow().iter() {
            vars.insert(
                k.clone(),
                Binding {
                    value: b.value.clone(),
                    mutable: b.mutable,
                },
            );
        }
        Rc::new(Scope {
            vars: RefCell::new(vars),
            mask: Cell::new(self.mask.get()),
            parent: Some(parent),
            strict: Cell::new(self.strict.get()),
        })
    }
}

thread_local! {
    /// The name of the per-call `this` binding, allocated once.
    static THIS_NAME: Rc<str> = "this".into();
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
            Value::Undefined | Value::Hole | Value::Null => false,
            Value::Bool(b) => *b,
            Value::Number(n) => !(*n == 0.0 || n.is_nan()),
            Value::Str(s) => !s.is_empty(),
            Value::BigInt(b) => !num_traits::Zero::is_zero(&**b),
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
        matches!(self, Value::Object(o) if match &*o.kind.borrow() {
            Kind::Closure { .. } | Kind::Native(_) => true,
            Kind::Proxy(p) => p.callable,
            _ => false,
        })
    }
}

fn new_obj(proto: Option<Obj>, kind: Kind) -> Obj {
    Rc::new(ObjData {
        props: RefCell::new(Vec::new()),
        proto: RefCell::new(proto),
        kind: RefCell::new(kind),
        frozen: Cell::new(false),
        ext: Cell::new(0),
        index: RefCell::new(None),
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
    } else if n.fract() == 0.0 && n.abs() < 9007199254740992.0 {
        format!("{n:.0}")
    } else if n.fract() == 0.0 {
        // large integers: the shortest round-trip digits, zero padded (not the exact binary value)
        let s = format!("{:e}", n.abs());
        let (m, e) = s.split_once('e').unwrap_or((&s, "0"));
        let digits: String = m.chars().filter(|c| *c != '.').collect();
        let exp: usize = e.parse().unwrap_or(0);
        let pad = (exp + 1).saturating_sub(digits.len());
        format!(
            "{}{digits}{}",
            if n < 0.0 { "-" } else { "" },
            "0".repeat(pad)
        )
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
        (Value::BigInt(p), Value::BigInt(q)) => p == q,
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
    fn_cache: FxMap<(u32, u32), Rc<FuncDef>>,
    tpl_cache: FxMap<(u32, u32), Value>,
    /// Script text by `run_id` (function source for `toString`).
    pub(crate) sources: FxMap<u32, Rc<str>>,
    /// For statements (by position) whose body creates closures, so each iteration needs its own bindings.
    loop_captures: FxMap<(u32, u32), bool>,
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
    new_target: bool,
    /// `this` (or `super`) appears in the body, arrows included.
    uses_this: bool,
}

impl Visit for ArgumentsFinder {
    fn visit_ident(&mut self, n: &a::Ident) {
        if &*n.sym == "arguments" {
            self.found = true;
        }
    }
    fn visit_function(&mut self, _: &a::Function) {}
    fn visit_this_expr(&mut self, _: &a::ThisExpr) {
        self.uses_this = true;
    }
    fn visit_super(&mut self, _: &a::Super) {
        self.uses_this = true;
    }
    fn visit_meta_prop_expr(&mut self, n: &a::MetaPropExpr) {
        if matches!(n.kind, a::MetaPropKind::NewTarget) {
            self.new_target = true;
        }
    }
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
            fn_cache: FxMap::default(),
            tpl_cache: FxMap::default(),
            sources: FxMap::default(),
            loop_captures: FxMap::default(),
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
        it.install_generators();
        it.install_bigint();
        it.install_numfmt();
        it.install_proxy();
        it.run(builtins::PRELUDE)
            .unwrap_or_else(|_| panic!("thaw-js prelude must evaluate"));
        it.run_promise_prelude();
        it.run(extras::EXTRAS).unwrap_or_else(|e| {
            let why = match &e {
                Ctl::Throw(v) => {
                    let m = it.get(v, "message").unwrap_or(Value::Undefined);
                    it.inspect(&m, true)
                }
                other => format!("{other:?}"),
            };
            panic!("thaw-js extras prelude must evaluate: {why}")
        });
        it.install_array_iteration();
        it.install_typed();
        it.install_buffer();
        it.install_sysio();
        it.install_fs_host();
        web_dom_exception::install(&mut it);
        web_text_codec::install(&mut it);
        web_event_target::install(&mut it);
        web_abort::install(&mut it);
        web_blob::install(&mut it);
        web_fetch::install(&mut it);
        web_streams::install(&mut it);
        web_crypto::install(&mut it);
        it.install_url();
        #[cfg(feature = "intl")]
        {
            // timers are scheduled against `rt.now`, which only the event loop advances: do not let the
            // polyfill's evaluation time make a timer created by the program overdue at the first turn
            let started = std::time::Instant::now();
            it.install_intl();
            it.skip_timer_clock(started.elapsed().as_secs_f64() * 1000.0);
        }
        it.run(extras::EXTRAS_LATE)
            .unwrap_or_else(|_| panic!("thaw-js late extras prelude must evaluate"));
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
        names.extend(
            self.global_object
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
                self.lookup(&self.global, member).is_some() || find_in_chain(&o, member).is_some()
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
                // namespace members such as `Intl.DateTimeFormat.prototype`
                let members: Vec<Value> = o
                    .props
                    .borrow()
                    .iter()
                    .filter_map(|p| match &p.slot {
                        Slot::Data(Value::Object(m)) => Some(Value::Object(m.clone())),
                        _ => None,
                    })
                    .collect();
                for m in members {
                    if let Value::Object(m) = m {
                        if let Some((Slot::Data(Value::Object(p)), _)) = find_own(&m, "prototype") {
                            objs.push(p);
                        }
                    }
                }
            }
        }
        for o in objs {
            // inherited methods count too (`%TypedArray%.prototype.subarray` on `Uint8Array.prototype`)
            let mut cur = Some(o);
            while let Some(o) = cur {
                names.extend(o.props.borrow().iter().map(|p| p.key.to_string()));
                cur = o.proto.borrow().clone();
            }
        }
        names.extend(
            [
                "next",
                "return",
                "throw",
                "require",
                "enqueue",
                "releaseLock",
            ]
            .map(String::from),
        ); // iterator protocol / `module.require` (user-defined) / stream controller and reader methods
        names
    }

    // ----- conversions -----

    pub fn to_str(&mut self, v: &Value) -> Rc<str> {
        match v {
            Value::Undefined | Value::Hole => "undefined".into(),
            Value::Null => "null".into(),
            Value::Bool(b) => if *b { "true" } else { "false" }.into(),
            Value::Number(n) => number_to_string(*n).into(),
            Value::Str(s) => s.clone(),
            Value::BigInt(b) => b.to_string().into(),
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
            Value::Undefined | Value::Hole => f64::NAN,
            Value::Null => 0.0,
            Value::Bool(b) => *b as u8 as f64,
            Value::Number(n) => *n,
            Value::Str(s) => string_to_number(s),
            Value::BigInt(b) => num_traits::ToPrimitive::to_f64(&**b).unwrap_or(f64::NAN),
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
        let mut out = String::new();
        for (i, v) in items.iter().enumerate() {
            if i > 0 {
                builtins::push_joined(&mut out, sep);
            }
            if !matches!(v, Value::Undefined | Value::Null | Value::Hole) {
                builtins::push_joined(&mut out, &self.to_str(v));
            }
        }
        out.into()
    }

    /// `console.log`-style rendering (a small subset of Node's util.inspect).
    pub fn inspect(&mut self, v: &Value, top: bool) -> String {
        match v {
            Value::Str(s) if top => s.to_string(),
            Value::Str(s) => format!("'{s}'"),
            Value::Object(o) => {
                let proxied = match &*o.kind.borrow() {
                    Kind::Proxy(p) => Some(p.target.clone()),
                    _ => None,
                };
                if let Some(target) = proxied {
                    return self.inspect(&target, top);
                }
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
                    | Kind::Generator(_)
                    | Kind::Proxy(_)
                    | Kind::Symbol { .. } => K::Plain,
                    Kind::Promise(_) => K::Prom,
                    Kind::Wrapper(p) => K::Bytes(match p {
                        Value::Number(n) => format!("[Number: {}]", number_to_string(*n)),
                        Value::Str(s) => format!("[String: '{s}']"),
                        Value::Bool(b) => format!("[Boolean: {b}]"),
                        _ => "[Object]".into(),
                    }),
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
                    K::Arr(items) => {
                        let mut parts: Vec<String> = Vec::new();
                        let mut i = 0;
                        while i < items.len() {
                            if matches!(items[i], Value::Hole) {
                                let n = items[i..]
                                    .iter()
                                    .take_while(|v| matches!(v, Value::Hole))
                                    .count();
                                parts.push(format!(
                                    "<{n} empty item{}>",
                                    if n == 1 { "" } else { "s" }
                                ));
                                i += n;
                            } else {
                                parts.push(self.inspect(&items[i], false));
                                i += 1;
                            }
                        }
                        format!("[ {} ]", parts.join(", "))
                    }
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
                        let (mut indexed, rest): (Vec<Prop>, Vec<Prop>) = props
                            .into_iter()
                            .partition(|p| array_index(&p.key).is_some());
                        indexed.sort_by_key(|p| array_index(&p.key));
                        indexed.extend(rest);
                        let props = indexed;
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
            Value::BigInt(b) => format!("{b}n"),
            other => (*self.to_str(other)).to_string(),
        }
    }

    /// Own enumerable string keys (array indices first).
    pub(crate) fn own_keys(&mut self, v: &Value) -> Vec<Rc<str>> {
        match v {
            Value::Object(o) => {
                if matches!(&*o.kind.borrow(), Kind::Proxy(_)) {
                    return match self.proxy_own_keys(o, true) {
                        Ok(Some(keys)) => keys
                            .into_iter()
                            .filter_map(|k| match k {
                                Value::Str(s) => Some(s),
                                _ => None,
                            })
                            .collect(),
                        _ => Vec::new(),
                    };
                }
                let mut keys: Vec<Rc<str>> = Vec::new();
                if let Kind::Array(items) = &*o.kind.borrow() {
                    keys.extend(
                        (0..items.len())
                            .filter(|&i| !matches!(items[i], Value::Hole))
                            .map(|i| Rc::from(i.to_string())),
                    );
                }
                if let Kind::Typed(t) = &*o.kind.borrow() {
                    keys.extend((0..t.len).map(|i| Rc::from(i.to_string())));
                }
                if let Kind::Wrapper(Value::Str(s)) = &*o.kind.borrow() {
                    keys.extend((0..builtins::str_len(s)).map(|i| Rc::from(i.to_string())));
                }
                let own: Vec<Rc<str>> = o
                    .props
                    .borrow()
                    .iter()
                    .filter(|p| p.enumerable && !p.key.starts_with("@@sym:"))
                    .map(|p| p.key.clone())
                    .collect();
                keys.extend(order_keys(own));
                keys
            }
            Value::Str(s) => (0..builtins::str_len(s))
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
            Value::Undefined | Value::Hole | Value::Null => {
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
                    return Ok(Value::Number(builtins::str_len(s) as f64));
                }
                if let Ok(i) = key.parse::<usize>() {
                    return Ok(builtins::str_unit_at(s, i).map_or(Value::Undefined, |u| {
                        Value::Str(builtins::units_str(&[u]).into())
                    }));
                }
                self.string_proto.clone()
            }
            Value::Number(_) => self.number_proto.clone(),
            Value::Bool(_) => self.bool_proto.clone(),
            Value::BigInt(_) => self
                .rt
                .bigint_proto
                .clone()
                .unwrap_or_else(|| self.object_proto.clone()),
            Value::Object(o) => {
                if matches!(&*o.kind.borrow(), Kind::Global) {
                    if let Some(v) = self.lookup(&self.global.clone(), key) {
                        return Ok(v);
                    }
                }
                if let Some(v) = self.proxy_get(o, key, target)? {
                    return Ok(v);
                }
                if let Kind::Array(items) = &*o.kind.borrow() {
                    if key == "length" {
                        return Ok(Value::Number(items.len() as f64));
                    }
                    if let Ok(i) = key.parse::<usize>() {
                        return Ok(unhole(items.get(i)));
                    }
                }
                if let Kind::Wrapper(Value::Str(s)) = &*o.kind.borrow() {
                    if key == "length" {
                        return Ok(Value::Number(builtins::str_len(s) as f64));
                    }
                    if let Some(u) = key
                        .parse::<usize>()
                        .ok()
                        .and_then(|i| builtins::str_unit_at(s, i))
                    {
                        return Ok(Value::Str(builtins::units_str(&[u]).into()));
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
        if self.proxy_set(o, key, &value, target)? {
            return Ok(());
        }
        if matches!(&*o.kind.borrow(), Kind::Global) {
            return self.assign_var(&self.global.clone(), key, value);
        }
        if let Kind::Array(items) = &mut *o.kind.borrow_mut() {
            if let Ok(i) = key.parse::<usize>() {
                if i >= items.len() {
                    items.resize(i + 1, Value::Hole);
                }
                items[i] = value;
                return Ok(());
            }
            if key == "length" {
                let n = match value {
                    Value::Number(n) if n >= 0.0 => n as usize,
                    _ => 0,
                };
                items.resize(n, Value::Hole);
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
        if o.ext.get() != 0 && find_own(o, key).is_none() {
            return Ok(()); // sloppy mode: silently ignored
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
        if matches!(&*fo.kind.borrow(), Kind::Proxy(_)) {
            return self.proxy_call(fo, this, args);
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
            Callee::Native(n) => {
                // natives on a wrapper object's prototype chain see the wrapped primitive
                let prim = match this {
                    Value::Object(o) => match &*o.kind.borrow() {
                        Kind::Wrapper(p) => Some(p.clone()),
                        _ => None,
                    },
                    _ => None,
                };
                n(self, prim.as_ref().unwrap_or(this), args)
            }
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
        if def.is_gen {
            return self.make_generator(f, def, scope, this, args);
        }
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
        let new_target = self.rt.pending_nt.take();
        let frame = Scope::new(Some(scope.clone()));
        if def.strict {
            frame.strict.set(true);
        }
        if def.uses_new_target {
            frame.declare("%newtarget", new_target.unwrap_or(Value::Undefined), false);
        }
        if !def.arrow {
            let this = if !frame.strict.get() && matches!(this, Value::Undefined | Value::Null) {
                Value::Object(self.global_object.clone())
            } else {
                this.clone()
            };
            if def.uses_this {
                frame.declare_rc(THIS_NAME.with(Rc::clone), this, false);
            }
            if let Some(name) = def.name.as_ref().filter(|_| def.bind_self) {
                frame.declare(name, f.clone(), true);
            }
            if def.uses_arguments {
                frame.declare("arguments", self.array(args.to_vec()), true);
            }
        }
        for (i, p) in def.params.iter().enumerate() {
            if let Some(Some(name)) = def.param_names.get(i) {
                let value = args.get(i).cloned().unwrap_or(Value::Undefined);
                frame.declare_rc(name.clone(), value, true);
            } else if let a::Pat::Rest(r) = p {
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
                declare_hoisted(&def.hoisted, &frame);
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
        if matches!(&*fo.kind.borrow(), Kind::Proxy(p) if p.callable) {
            return self.proxy_construct(fo, f, args);
        }
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
            if !matches!(r, Value::Object(_)) && find_own(fo, "@@wrap").is_some() {
                return Ok(Value::Object(new_obj(Some(proto), Kind::Wrapper(r))));
            }
            return Ok(r);
        }
        let this = Value::Object(new_obj(Some(proto), Kind::Plain));
        self.rt.pending_nt = Some(f.clone());
        let result = if is_class {
            self.init_instance(f, &this, args)
        } else {
            self.call(f, &this, args)
        };
        self.rt.pending_nt = None;
        let result = result?;
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
        is_gen: bool,
        bind_self: bool,
        lo: u32,
        hi: u32,
        scope: &Rc<Scope>,
    ) -> Value {
        let self_run_id = self.run_id;
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
                let hoisted = match &body {
                    Body::Block(s) => hoisted_names(s),
                    Body::Expr(_) => Vec::new(),
                };
                let param_names = simple_param_names(&params);
                Rc::new(FuncDef { name, params, body, arrow, is_async, is_gen, bind_self, uses_arguments: finder.found && !arrow, uses_new_target: finder.new_target && !arrow, uses_this: finder.uses_this, src: (self_run_id, lo, hi), hoisted, param_names, strict, class: None })
            })
            .clone();
        self.closure_from_def(def, scope)
    }

    /// A fresh function object for an already analysed definition.
    fn closure_from_def(&mut self, def: Rc<FuncDef>, scope: &Rc<Scope>) -> Value {
        let (arrow, is_async, is_gen) = (def.arrow, def.is_async, def.is_gen);
        let f = new_obj(
            Some(self.function_proto.clone()),
            Kind::Closure {
                def,
                scope: scope.clone(),
            },
        );
        if is_gen {
            let parent = if is_async {
                self.rt.async_generator_proto.clone()
            } else {
                self.rt.generator_proto.clone()
            };
            put_hidden(&f, "prototype", Value::Object(new_obj(parent, Kind::Plain)));
        } else if !arrow {
            let proto = new_obj(Some(self.object_proto.clone()), Kind::Plain);
            put_hidden(&proto, "constructor", Value::Object(f.clone()));
            put_hidden(&f, "prototype", Value::Object(proto));
        }
        Value::Object(f)
    }

    // ----- scopes -----

    fn lookup(&self, scope: &Rc<Scope>, name: &str) -> Option<Value> {
        let bit = name_bit(name);
        let mut cur: &Scope = scope;
        loop {
            if cur.mask.get() & bit != 0 {
                if let Some(b) = cur.vars.borrow().get(name) {
                    return Some(b.value.clone());
                }
            }
            cur = cur.parent.as_deref()?;
        }
    }

    fn assign_var(&mut self, scope: &Rc<Scope>, name: &str, value: Value) -> R<()> {
        let bit = name_bit(name);
        let mut cur: Option<&Scope> = Some(scope);
        while let Some(s) = cur {
            if s.mask.get() & bit == 0 {
                cur = s.parent.as_deref();
                continue;
            }
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
                None => cur = s.parent.as_deref(),
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
        self.sources.insert(self.run_id, Rc::from(source));
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
        if let Some(def) = self.fn_cache.get(&(self.run_id, f.span.lo.0)).cloned() {
            return Ok(self.closure_from_def(def, scope));
        }
        let body = Body::Block(f.body.as_ref().map(|b| b.stmts.clone()).unwrap_or_default());
        Ok(self.make_function(
            name,
            fn_pats(&f.params),
            body,
            false,
            f.is_async,
            f.is_generator,
            bind_self,
            f.span.lo.0,
            f.span.hi.0,
            scope,
        ))
    }

    fn hoist_functions(&mut self, stmts: &[a::Stmt], scope: &Rc<Scope>) -> R<()> {
        self.hoist_functions_in(stmts, scope, false)
    }

    /// `in_block`: sloppy-mode Annex B also assigns a block-level function to the `var` of the
    /// same name in the enclosing function.
    fn hoist_functions_in(
        &mut self,
        stmts: &[a::Stmt],
        scope: &Rc<Scope>,
        in_block: bool,
    ) -> R<()> {
        for s in stmts {
            if let a::Stmt::Decl(a::Decl::Fn(f)) = s {
                let name: Rc<str> = f.ident.sym.as_ref().into();
                let func = self.function_value(Some(name.clone()), &f.function, scope)?;
                if in_block && !scope.strict.get() {
                    let mut cur = scope.parent.as_deref();
                    while let Some(outer) = cur {
                        if let Some(b) = outer.vars.borrow_mut().get_mut(&name) {
                            if b.mutable {
                                b.value = func.clone();
                            }
                            break;
                        }
                        cur = outer.parent.as_deref();
                    }
                }
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
        // A block that declares nothing lexically (`let`/`const`/class/function) needs no scope of its own.
        let lexical = stmts.iter().any(|s| match s {
            a::Stmt::Decl(a::Decl::Var(v)) => !matches!(v.kind, a::VarDeclKind::Var),
            a::Stmt::Decl(a::Decl::Class(_) | a::Decl::Fn(_)) => true,
            _ => false,
        });
        if !lexical {
            for s in stmts {
                self.exec(s, scope)?;
            }
            return Ok(());
        }
        let inner = Scope::new(Some(scope.clone()));
        self.hoist_functions_in(stmts, &inner, true)?;
        for s in stmts {
            self.exec(s, &inner)?;
        }
        Ok(())
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
                let key = (self.run_id, f.span.lo.0);
                let captures = match self.loop_captures.get(&key) {
                    Some(c) => *c,
                    None => {
                        #[derive(Default)]
                        struct Closures(bool);
                        impl Visit for Closures {
                            fn visit_function(&mut self, _: &a::Function) {
                                self.0 = true;
                            }
                            fn visit_arrow_expr(&mut self, _: &a::ArrowExpr) {
                                self.0 = true;
                            }
                            fn visit_class(&mut self, _: &a::Class) {
                                self.0 = true;
                            }
                        }
                        let mut finder = Closures::default();
                        f.visit_with(&mut finder);
                        self.loop_captures.insert(key, finder.0);
                        finder.0
                    }
                };
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
                    // (only observable when the loop creates a closure)
                    if captures {
                        cur = cur.copy_of(scope.clone());
                    }
                    if let Some(u) = &f.update {
                        self.eval(u, &cur)?;
                    }
                }
                Ok(())
            }
            a::Stmt::ForOf(f) => {
                let iterable = self.eval(&f.right, scope)?;
                if f.is_await {
                    if let Some((iterator, next)) = self.get_async_iterator(&iterable)? {
                        loop {
                            let result = self.call(&next, &iterator, &[])?;
                            let result = self.await_value(result)?;
                            if !matches!(result, Value::Object(_)) {
                                return self
                                    .throw_error("TypeError", "Iterator result is not an object");
                            }
                            if self.get(&result, "done")?.truthy() {
                                break;
                            }
                            let item = self.get(&result, "value")?;
                            let iter = Scope::new(Some(scope.clone()));
                            let outcome = match self.bind_for_head(&f.left, item, &iter) {
                                Ok(()) => step(self.exec(&f.body, &iter)),
                                Err(e) => Err(e),
                            };
                            match outcome {
                                Ok(false) => {}
                                Ok(true) | Err(_) => {
                                    let early = outcome;
                                    if let Ok(ret) = self.get(&iterator, "return") {
                                        if ret.is_callable() {
                                            if let Ok(r) = self.call(&ret, &iterator, &[]) {
                                                let _ = self.await_value(r);
                                            }
                                        }
                                    }
                                    early?;
                                    break;
                                }
                            }
                        }
                        return Ok(());
                    }
                }
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

    fn member_key<'a>(&mut self, prop: &'a a::MemberProp, scope: &Rc<Scope>) -> R<Key<'a>> {
        match prop {
            // `obj.name`: the name is borrowed from the syntax tree, no allocation per access
            a::MemberProp::Ident(i) => Ok(Key::Borrowed(&i.sym)),
            a::MemberProp::Computed(c) => {
                let v = self.eval(&c.expr, scope)?;
                Ok(Key::Owned(self.key_of(&v)))
            }
            a::MemberProp::PrivateName(p) => Ok(Key::Owned(format!("@@priv:{}", &*p.name).into())),
        }
    }

    fn eval(&mut self, e: &a::Expr, scope: &Rc<Scope>) -> R<Value> {
        match e {
            a::Expr::Lit(l) => Ok(match l {
                a::Lit::Num(n) => Value::Number(n.value),
                a::Lit::Str(s) => match s.value.as_str() {
                    Some(x) => Value::str(x),
                    None => Value::Str(
                        builtins::units_str(&s.value.to_ill_formed_utf16().collect::<Vec<u16>>())
                            .into(),
                    ),
                },
                a::Lit::Bool(b) => Value::Bool(b.value),
                a::Lit::Null(_) => Value::Null,
                a::Lit::Regex(r) => return self.new_regexp(&r.exp, &r.flags),
                a::Lit::BigInt(b) => return Ok(bigint::big((*b.value).clone())),
                _ => return unsupported("bigint"),
            }),
            a::Expr::Ident(i) => self.eval_ident(i, scope),
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
                    builtins::push_joined(
                        &mut s,
                        q.cooked.as_ref().and_then(|c| c.as_str()).unwrap_or(""),
                    );
                    if let Some(e) = t.exprs.get(i) {
                        let v = self.eval(e, scope)?;
                        builtins::push_joined(&mut s, &self.to_str(&v));
                    }
                }
                Ok(Value::Str(s.into()))
            }
            a::Expr::Array(arr) => {
                let mut items = Vec::new();
                for el in &arr.elems {
                    match el {
                        Some(el) => items.extend(self.eval_args(std::slice::from_ref(el), scope)?),
                        None => items.push(Value::Hole),
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
                if let Some(def) = self.fn_cache.get(&(self.run_id, f.span.lo.0)).cloned() {
                    return Ok(self.closure_from_def(def, scope));
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
                    false,
                    f.span.lo.0,
                    f.span.hi.0,
                    scope,
                ))
            }
            a::Expr::Member(m) => {
                let obj = self.eval(&m.obj, scope)?;
                match self.member_loc(obj, &m.prop, scope)? {
                    Loc::Index(obj, i) => self.get_index(&obj, i),
                    Loc::Prop(obj, key) => self.get(&obj, &key),
                    Loc::Ident(_) => unreachable!(),
                }
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
                let loc = self.loc_of(&u.arg, scope)?;
                let old = self.read_loc(&loc, scope)?;
                if let Value::BigInt(b) = &old {
                    let one = num_bigint::BigInt::from(1);
                    let new = bigint::big(if matches!(u.op, a::UpdateOp::PlusPlus) {
                        &**b + one
                    } else {
                        &**b - one
                    });
                    self.write_loc(&loc, new.clone(), scope)?;
                    return Ok(if u.prefix { new } else { old });
                }
                let old = self.num(&old);
                let new = if matches!(u.op, a::UpdateOp::PlusPlus) {
                    old + 1.0
                } else {
                    old - 1.0
                };
                self.write_loc(&loc, Value::Number(new), scope)?;
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
                a::BinaryOp::In if matches!(&*b.left, a::Expr::PrivateName(_)) => {
                    let a::Expr::PrivateName(p) = &*b.left else {
                        unreachable!()
                    };
                    let key = format!("@@priv:{}", &*p.name);
                    match self.eval(&b.right, scope)? {
                        Value::Object(o) => Ok(Value::Bool(find_in_chain(&o, &key).is_some())),
                        _ => self.throw_error("TypeError", "Cannot use 'in' operator to search for a private field in a non-object"),
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
                // one frozen strings array per call site (its identity is observable)
                let site = (self.run_id, t.tpl.span.lo.0);
                let strings = match self.tpl_cache.get(&site) {
                    Some(s) => s.clone(),
                    None => {
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
                            if let Value::Object(r) = &raw {
                                r.frozen.set(true);
                            }
                            put_hidden(o, "raw", raw);
                            o.frozen.set(true);
                        }
                        self.tpl_cache.insert(site, strings.clone());
                        strings
                    }
                };
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
            a::Expr::MetaProp(m) if matches!(m.kind, a::MetaPropKind::NewTarget) => {
                Ok(self.lookup(scope, "%newtarget").unwrap_or(Value::Undefined))
            }
            a::Expr::MetaProp(_) => unsupported("import.meta"),
            a::Expr::Yield(y) => {
                let v = match &y.arg {
                    Some(e) => self.eval(e, scope)?,
                    None => Value::Undefined,
                };
                if y.delegate {
                    self.yield_star(v)
                } else {
                    self.yield_value(v)
                }
            }
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
                    Value::Undefined | Value::Hole => "undefined",
                    Value::Null => "object",
                    Value::Bool(_) => "boolean",
                    Value::Number(_) => "number",
                    Value::Str(_) => "string",
                    Value::BigInt(_) => "bigint",
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
                    if let Some(r) = self.proxy_delete(&o, &key)? {
                        return Ok(Value::Bool(r));
                    }
                    if o.frozen.get() || o.ext.get() == 2 {
                        return Ok(Value::Bool(false));
                    }
                    if let Kind::Array(items) = &mut *o.kind.borrow_mut() {
                        if let Some(slot) = key.parse::<usize>().ok().and_then(|i| items.get_mut(i))
                        {
                            *slot = Value::Hole;
                        }
                    }
                    o.remove_prop(&key);
                }
                Ok(Value::Bool(true))
            }
            _ => {
                let v = self.eval(&u.arg, scope)?;
                Ok(match u.op {
                    a::UnaryOp::Bang => Value::Bool(!v.truthy()),
                    a::UnaryOp::Minus => match &v {
                        Value::BigInt(b) => bigint::big(-(**b).clone()),
                        _ => Value::Number(-self.num(&v)),
                    },
                    a::UnaryOp::Plus if matches!(v, Value::BigInt(_)) => {
                        return self
                            .throw_error("TypeError", "Cannot convert a BigInt value to a number")
                    }
                    a::UnaryOp::Plus => Value::Number(self.num(&v)),
                    a::UnaryOp::Tilde => match &v {
                        Value::BigInt(b) => bigint::big(-(**b).clone() - 1),
                        _ => Value::Number(!to_int32(self.num(&v)) as f64),
                    },
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
        // the target's object and key are evaluated once, before the right-hand side
        let loc = match target {
            a::SimpleAssignTarget::Ident(i) => Loc::Ident(&i.id),
            a::SimpleAssignTarget::Member(m) => {
                let obj = self.eval(&m.obj, scope)?;
                self.member_loc(obj, &m.prop, scope)?
            }
            _ => return unsupported("assignment target"),
        };
        let value = match asg.op {
            A::Assign => self.eval(&asg.right, scope)?,
            A::AndAssign | A::OrAssign | A::NullishAssign => {
                let cur = self.read_loc(&loc, scope)?;
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
                let cur = self.read_loc(&loc, scope)?;
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
        self.write_loc(&loc, value.clone(), scope)?;
        Ok(value)
    }

    fn eval_ident(&mut self, i: &a::Ident, scope: &Rc<Scope>) -> R<Value> {
        match self.lookup(scope, &i.sym) {
            Some(v) => Ok(v),
            // the global object inherits from Object.prototype (`toString`, `hasOwnProperty`, ...)
            None => match find_in_chain(&self.object_proto, &i.sym) {
                Some(Slot::Data(v)) => Ok(v),
                _ => self.throw_error("ReferenceError", &format!("{} is not defined", &*i.sym)),
            },
        }
    }

    /// The (object, key) of a member access, with a non-negative integer key kept numeric so that
    /// `arr[i]` never builds the key string.
    fn member_loc<'a>(
        &mut self,
        obj: Value,
        prop: &'a a::MemberProp,
        scope: &Rc<Scope>,
    ) -> R<Loc<'a>> {
        if let a::MemberProp::Computed(c) = prop {
            let kv = self.eval(&c.expr, scope)?;
            if let Value::Number(n) = &kv {
                if *n >= 0.0 && n.fract() == 0.0 && *n < 4294967295.0 {
                    return Ok(Loc::Index(obj, *n as usize));
                }
            }
            return Ok(Loc::Prop(obj, Key::Owned(self.key_of(&kv))));
        }
        Ok(Loc::Prop(obj, self.member_key(prop, scope)?))
    }

    fn loc_of<'a>(&mut self, target: &'a a::Expr, scope: &Rc<Scope>) -> R<Loc<'a>> {
        match target {
            a::Expr::Ident(i) => Ok(Loc::Ident(i)),
            a::Expr::Member(m) => {
                let obj = self.eval(&m.obj, scope)?;
                self.member_loc(obj, &m.prop, scope)
            }
            a::Expr::Paren(p) => self.loc_of(&p.expr, scope),
            _ => unsupported("assignment target"),
        }
    }

    fn read_loc(&mut self, loc: &Loc<'_>, scope: &Rc<Scope>) -> R<Value> {
        match loc {
            Loc::Ident(i) => self.eval_ident(i, scope),
            Loc::Prop(obj, key) => self.get(obj, key),
            Loc::Index(obj, i) => self.get_index(obj, *i),
        }
    }

    fn write_loc(&mut self, loc: &Loc<'_>, value: Value, scope: &Rc<Scope>) -> R<()> {
        match loc {
            Loc::Ident(i) => self.assign_var(scope, &i.sym, value),
            Loc::Prop(obj, key) => self.set(obj, key, value),
            Loc::Index(obj, i) => self.set_index(obj, *i, value),
        }
    }

    /// `obj[i]` for an integer `i`: arrays and typed arrays are indexed directly.
    fn get_index(&mut self, obj: &Value, i: usize) -> R<Value> {
        if let Value::Object(o) = obj {
            match &*o.kind.borrow() {
                Kind::Array(items) => return Ok(unhole(items.get(i))),
                Kind::Typed(t) => return Ok(t.get(i).map_or(Value::Undefined, Value::Number)),
                _ => {}
            }
        }
        self.get(obj, &i.to_string())
    }

    fn set_index(&mut self, obj: &Value, i: usize, value: Value) -> R<()> {
        if let Value::Object(o) = obj {
            if !o.frozen.get() && o.ext.get() == 0 {
                if let Kind::Array(items) = &mut *o.kind.borrow_mut() {
                    if i < items.len() {
                        items[i] = value;
                        return Ok(());
                    }
                    if i == items.len() {
                        items.push(value);
                        return Ok(());
                    }
                }
            }
        }
        self.set(obj, &i.to_string(), value)
    }

    fn store(&mut self, target: &a::Expr, value: Value, scope: &Rc<Scope>) -> R<()> {
        let loc = self.loc_of(target, scope)?;
        self.write_loc(&loc, value, scope)
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
        if matches!(l, Value::BigInt(_)) || matches!(r, Value::BigInt(_)) {
            if let Some(v) = self.big_binary(op, l, r)? {
                return Ok(v);
            }
        }
        let num = |it: &mut Interp| (it.num(l), it.num(r));
        Ok(match op {
            B::Add => {
                let (lp, rp) = (l.clone(), r.clone());
                if matches!(lp, Value::Str(_)) || matches!(rp, Value::Str(_)) {
                    let mut s = self.to_str(&lp).to_string();
                    builtins::push_joined(&mut s, &self.to_str(&rp));
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
                if matches!(r, Value::Object(_)) {
                    let key = match self.rt.has_instance_key.clone() {
                        Some(k) => k,
                        None => {
                            let k = self.well_known_key("hasInstance");
                            self.rt.has_instance_key = Some(k.clone());
                            k
                        }
                    };
                    let custom = self.get(r, &key)?;
                    if custom.is_callable() {
                        let res = self.call(&custom, r, std::slice::from_ref(l))?;
                        return Ok(Value::Bool(res.truthy()));
                    }
                }
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
                Value::Bool(self.has_property(r, &key)?)
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
            (Value::BigInt(x), Value::BigInt(y)) => x == y,
            (Value::BigInt(x), Value::Str(s)) | (Value::Str(s), Value::BigInt(x)) => {
                bigint::parse_bigint(s).is_some_and(|y| **x == y)
            }
            (Value::BigInt(x), o) | (o, Value::BigInt(x)) if !matches!(o, Value::Object(_)) => {
                let n = self.num(o);
                bigint::cmp_big_num(x, n) == Some(std::cmp::Ordering::Equal)
            }
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
    let i = o.position(key)?;
    let props = o.props.borrow();
    let p = &props[i];
    Some((p.slot.clone(), p.enumerable))
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

/// Index of the first property an array-index key `key` must precede, so integer keys stay
/// first in ascending order (`None`: not an array index, append).
pub(crate) fn push_prop(o: &ObjData, props: &mut Vec<Prop>, p: Prop) {
    // Insertion order is kept; the integer-keys-first rule is applied when keys are enumerated
    // (`order_keys`), so appending stays O(1) and the hash index stays valid.
    let key = p.key.clone();
    props.push(p);
    if let Some(map) = o.index.borrow_mut().as_mut() {
        map.insert(key, props.len() - 1);
    }
}

/// An array-index property key (`"0"`, `"17"`, not `"01"`).
fn array_index(k: &str) -> Option<u32> {
    let b = k.as_bytes();
    if b.is_empty() || !b[0].is_ascii_digit() || (b[0] == b'0' && b.len() > 1) || b.len() > 10 {
        return None;
    }
    k.parse::<u32>().ok().filter(|n| *n != u32::MAX)
}

/// Enumeration order of own keys: array indices ascending, then the rest in insertion order.
pub(crate) fn order_keys(keys: Vec<Rc<str>>) -> Vec<Rc<str>> {
    if !keys.iter().any(|k| array_index(k).is_some()) {
        return keys;
    }
    let (mut indices, rest): (Vec<_>, Vec<_>) =
        keys.into_iter().partition(|k| array_index(k).is_some());
    indices.sort_by_key(|k| array_index(k));
    indices.extend(rest);
    indices
}

/// Create or overwrite an enumerable, writable data property.
pub(crate) fn put(o: &Obj, key: &str, value: Value) {
    let pos = o.position(key);
    let mut props = o.props.borrow_mut();
    match pos.map(|i| &mut props[i]) {
        Some(p) => p.slot = Slot::Data(value),
        None => push_prop(
            o,
            &mut props,
            Prop {
                key: key.into(),
                slot: Slot::Data(value),
                enumerable: true,
                writable: true,
            },
        ),
    }
}

/// Create or overwrite a non-enumerable, writable data property (builtins).
pub(crate) fn put_hidden(o: &Obj, key: &str, value: Value) {
    let pos = o.position(key);
    let mut props = o.props.borrow_mut();
    match pos.map(|i| &mut props[i]) {
        Some(p) => {
            p.slot = Slot::Data(value);
            p.enumerable = false;
        }
        None => push_prop(
            o,
            &mut props,
            Prop {
                key: key.into(),
                slot: Slot::Data(value),
                enumerable: false,
                writable: true,
            },
        ),
    }
}

pub(crate) fn define_accessor(
    o: &Obj,
    key: &str,
    get: Option<Value>,
    set: Option<Value>,
    enumerable: bool,
) {
    let pos = o.position(key);
    let mut props = o.props.borrow_mut();
    match pos.map(|i| &mut props[i]) {
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
        None => push_prop(
            o,
            &mut props,
            Prop {
                key: key.into(),
                slot: Slot::Accessor { get, set },
                enumerable,
                writable: true,
            },
        ),
    }
}

fn simple_param_names(params: &[a::Pat]) -> Vec<Option<Rc<str>>> {
    params
        .iter()
        .map(|p| match p {
            a::Pat::Ident(id) => Some(Rc::from(&*id.id.sym)),
            _ => None,
        })
        .collect()
}

fn hoisted_names(stmts: &[a::Stmt]) -> Vec<Rc<str>> {
    struct Hoister(Vec<Rc<str>>);
    impl Visit for Hoister {
        fn visit_function(&mut self, _: &a::Function) {}
        fn visit_arrow_expr(&mut self, _: &a::ArrowExpr) {}
        fn visit_class(&mut self, _: &a::Class) {}
        // Annex B: a function declared in a nested block is also a `var` of the enclosing function.
        fn visit_fn_decl(&mut self, n: &a::FnDecl) {
            if !self.0.iter().any(|known| **known == *n.ident.sym) {
                self.0.push(n.ident.sym.as_ref().into());
            }
        }
        fn visit_var_decl(&mut self, n: &a::VarDecl) {
            if matches!(n.kind, a::VarDeclKind::Var) {
                for d in &n.decls {
                    let mut names = Names(Vec::new());
                    d.name.visit_with(&mut names);
                    for name in names.0 {
                        if !self.0.iter().any(|known| **known == *name) {
                            self.0.push(name.as_str().into());
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
    let mut h = Hoister(Vec::new());
    stmts.visit_with(&mut h);
    h.0
}

/// Declare every `var` in `stmts` (not crossing function boundaries) as `undefined` in `scope`.
fn hoist_vars(stmts: &[a::Stmt], scope: &Rc<Scope>) {
    declare_hoisted(&hoisted_names(stmts), scope);
}

fn declare_hoisted(names: &[Rc<str>], scope: &Rc<Scope>) {
    for name in names {
        scope.declare_if_absent(name);
    }
}

#[cfg(test)]
mod tests;
