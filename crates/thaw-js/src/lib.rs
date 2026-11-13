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
use std::rc::{Rc, Weak};

use classes::ClassInfo;
use swc_ecma_visit::{Visit, VisitWith};
use thaw_parser::ast as a;

mod bigint;
mod buffer;
mod builtins;
mod child;
mod classes;
mod jsstr;
pub use jsstr::JsStr;
mod collections;
mod compile;
mod date;
mod dns;
mod esm;
mod eventloop;
mod extras;
mod generators;
mod hints;
pub mod host;
#[cfg(feature = "intl")]
mod intl;
mod json;
mod napi_bridge;
mod napi_eval;
mod napi_graph;
mod napi_ops;
mod napi_props;
mod napi_symbols;
mod net;
mod node_crypto;
mod numfmt;
mod procstate;
mod proxy;
mod reflect;
pub mod scan;
mod sysio;
mod tls;
mod typed;
mod url;
mod vm;
#[path = "web/abort.rs"]
mod web_abort;
#[path = "web/async_hooks.rs"]
mod web_async_hooks;
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
#[path = "web/messaging.rs"]
mod web_messaging;
#[path = "web/napi_proxy.rs"]
mod web_napi_proxy;
#[path = "web/navigator.rs"]
mod web_navigator;
#[path = "web/performance.rs"]
mod web_performance;
#[path = "web/streams.rs"]
mod web_streams;
#[path = "web/text_codec.rs"]
mod web_text_codec;
#[path = "web/wasm.rs"]
mod web_wasm;
#[path = "web/worker_stream.rs"]
mod web_worker_stream;
#[path = "web/worker_threads.rs"]
mod web_worker_threads;
#[path = "web/worker_wire.rs"]
mod web_worker_wire;
mod worker;
mod zlib;

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
    Str(JsStr),
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
    /// Own properties in insertion order. Read it through `props()` when a slot's value matters;
    /// `raw_props` is for key, position and flag readers (a `LazyProto` placeholder has the flags
    /// of the real `prototype`).
    raw_props: RefCell<Vec<Prop>>,
    /// Set while a `Slot::LazyProto` placeholder may be in `raw_props`.
    lazy_proto: Cell<bool>,
    proto: RefCell<Option<Obj>>,
    kind: RefCell<Kind>,
    frozen: Cell<bool>,
    /// An array's `length` made non-writable (`defineProperty(a, 'length', {writable: false})`).
    len_ro: Cell<bool>,
    /// 0 extensible, 1 non-extensible (`preventExtensions`), 2 sealed.
    ext: Cell<u8>,
    /// Key -> position in `props`, built lazily for objects with many properties (`None` = stale).
    index: RefCell<Option<FxMap<Rc<str>, usize>>>,
}

/// Objects with fewer properties than this are searched linearly.
const INDEX_MIN: usize = 10;

impl ObjData {
    /// Writability of an array's `length`: frozen or made read-only both clear it.
    fn len_writable(&self) -> bool {
        !self.frozen.get() && !self.len_ro.get()
    }

    /// The own property table with a pending lazy `prototype` materialised first.
    pub(crate) fn props(&self) -> &RefCell<Vec<Prop>> {
        self.force_lazy();
        &self.raw_props
    }

    /// Materialise the lazy `prototype` when `key` is that key (the only key a placeholder sits on).
    pub(crate) fn force_key(&self, key: &str) {
        if key == "prototype" {
            self.force_lazy();
        }
    }

    /// Replace a pending `LazyProto` with the object `closure_from_def` would have built eagerly:
    /// `{constructor: f}` inheriting from the realm's Object.prototype. The key stays at its index.
    pub(crate) fn force_lazy(&self) {
        if !self.lazy_proto.get() {
            return;
        }
        self.lazy_proto.set(false);
        let found = self
            .raw_props
            .borrow()
            .iter()
            .enumerate()
            .find_map(|(i, p)| match &p.slot {
                Slot::LazyProto { owner, parent } => Some((i, owner.clone(), parent.clone())),
                _ => None,
            });
        let Some((i, owner, parent)) = found else {
            return;
        };
        let f = owner
            .upgrade()
            .expect("a lazy prototype is only reached through its live function");
        let proto = new_obj(Some(parent), Kind::Plain);
        put_hidden(&proto, "constructor", Value::Object(f));
        self.raw_props.borrow_mut()[i].slot = Slot::Data(Value::Object(proto));
    }

    /// Position of own property `key`.
    fn position(&self, key: &str) -> Option<usize> {
        let props = self.raw_props.borrow();
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
        self.raw_props.borrow_mut().retain(|p| *p.key != *key);
        *self.index.borrow_mut() = None;
    }
}

#[derive(Clone)]
struct Prop {
    key: Rc<str>,
    slot: Slot,
    enumerable: bool,
    writable: bool,
    configurable: bool,
}

#[derive(Clone)]
enum Slot {
    Data(Value),
    Accessor {
        get: Option<Value>,
        set: Option<Value>,
    },
    /// A plain function's `prototype` before its first use. `owner` is the function (a weak link, so
    /// no cycle) and `parent` the Object.prototype the materialised object inherits from.
    LazyProto {
        owner: Weak<ObjData>,
        parent: Obj,
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
    /// A `Map`/`Set` iterator (`keys()`, `values()`, `entries()`), live over its collection.
    CollIter(collections::CollIterData),
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
    /// `JSON.rawJSON(text)`: serializes as `text` verbatim.
    RawJson(Rc<str>),
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
    /// No `prototype` object: methods, accessors and async (non-generator) functions. Generators keep one.
    no_prototype: bool,
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
    /// Union of `name_bit` over `param_names`, so a call frame never re-hashes its parameters.
    param_mask: u64,
    strict: bool,
    class: Option<ClassInfo>,
    /// Lazy compilation state (`compile/mod.rs`); `Cold` until the function has been called enough.
    cstate: std::cell::RefCell<compile::CState>,
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
    origin: Origin,
}

/// How a global-scope binding relates to the `globalThis` object, for `delete`.
#[derive(Clone, Copy, PartialEq)]
enum Origin {
    /// let/const/class, parameters, builtins: never removed by `delete`.
    Decl,
    /// top-level `var` or function of a script: non-configurable, `delete` returns false.
    Var,
    /// implicit global (assigned without a declaration): configurable, `delete` removes it.
    Prop,
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
        // Rotate so the high (well-mixed) bits reach the low bits hashbrown uses for buckets;
        // f64 bits of small integers have zero low words, which otherwise collide.
        self.0.rotate_left(26)
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
            // a name interned for this key matches by pointer; any other name compares by string
            None => self.list.iter().position(|(name, _)| {
                (name.as_ptr() == key.as_ptr() && name.len() == key.len()) || &**name == key
            }),
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
    fn remove(&mut self, key: &str) {
        let Some(i) = self.position(key) else { return };
        self.list.remove(i);
        if self.index.is_some() {
            self.index = Some(
                self.list
                    .iter()
                    .enumerate()
                    .map(|(i, (k, _))| (k.clone(), i))
                    .collect(),
            );
        }
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
/// Every byte feeds an Fx-style fold and the top 6 bits pick the bit, so names that differ only in
/// their middle still spread out. Collisions just cost a walk.
fn name_bit(name: &str) -> u64 {
    let mut h: u64 = 0;
    for &b in name.as_bytes() {
        h = (h.rotate_left(5) ^ u64::from(b)).wrapping_mul(0x517c_c1b7_2722_0a95);
    }
    1u64 << (h >> 58)
}

/// One scope walk to the binding of `name` that runs `f` on it in place. `None` (with nothing
/// changed) when `f` declines, when a `with` record comes first, or when no scope binds the name;
/// callers then take the generic path.
fn with_binding_mut<T>(
    scope: &Rc<Scope>,
    name: &str,
    f: impl FnOnce(&mut Binding) -> Option<T>,
) -> Option<T> {
    let bit = name_bit(name);
    let mut cur: &Scope = scope;
    loop {
        if cur.mask.get() & bit != 0 {
            if cur.with.is_some() {
                return None;
            }
            if let Some(b) = cur.vars.borrow_mut().get_mut(name) {
                return f(b);
            }
        }
        cur = cur.parent.as_deref()?;
    }
}

/// `name += rhs` for a string binding, appended in place. `cur` is the caller's snapshot of the
/// binding, taken before the right-hand side ran. It is released only after the binding is known
/// to still hold that same string, so a binding that is the sole owner can grow without a copy.
/// Returns `None` with `cur` untouched when the generic path must run (const, `with`, missing
/// binding, or the right-hand side reassigned the binding).
fn append_in_place(scope: &Rc<Scope>, name: &str, cur: &mut Value, rhs: &JsStr) -> Option<Value> {
    with_binding_mut(scope, name, |b| {
        if !b.mutable {
            return None;
        }
        let (Value::Str(js), Value::Str(snap)) = (&mut b.value, &*cur) else {
            return None;
        };
        if !JsStr::ptr_eq(js, snap) {
            return None;
        }
        *cur = Value::Undefined;
        js.push_joined(rhs);
        Some(b.value.clone())
    })
}

/// Arithmetic and relational operators on two numbers, without `binary()`'s coercion chain.
fn number_binop(op: a::BinaryOp, x: f64, y: f64) -> Option<Value> {
    use a::BinaryOp as B;
    Some(match op {
        B::Add => Value::Number(x + y),
        B::Sub => Value::Number(x - y),
        B::Mul => Value::Number(x * y),
        B::Div => Value::Number(x / y),
        B::Mod => Value::Number(x % y),
        B::Lt => Value::Bool(x < y),
        B::LtEq => Value::Bool(x <= y),
        B::Gt => Value::Bool(x > y),
        B::GtEq => Value::Bool(x >= y),
        B::BitAnd => Value::Number((to_int32(x) & to_int32(y)) as f64),
        B::BitOr => Value::Number((to_int32(x) | to_int32(y)) as f64),
        B::BitXor => Value::Number((to_int32(x) ^ to_int32(y)) as f64),
        B::LShift => Value::Number(to_int32(x).wrapping_shl(to_int32(y) as u32 & 31) as f64),
        B::RShift => Value::Number((to_int32(x) >> (to_int32(y) as u32 & 31)) as f64),
        B::ZeroFillRShift => {
            Value::Number(((to_int32(x) as u32) >> (to_int32(y) as u32 & 31)) as f64)
        }
        _ => return None,
    })
}

/// The binary operator behind a compound assignment (`None` for `=`, `&&=`, `||=`, `??=`).
fn compound_binop(op: a::AssignOp) -> Option<a::BinaryOp> {
    use a::AssignOp as A;
    use a::BinaryOp as B;
    Some(match op {
        A::AddAssign => B::Add,
        A::SubAssign => B::Sub,
        A::MulAssign => B::Mul,
        A::DivAssign => B::Div,
        A::ModAssign => B::Mod,
        A::ExpAssign => B::Exp,
        A::LShiftAssign => B::LShift,
        A::RShiftAssign => B::RShift,
        A::ZeroFillRShiftAssign => B::ZeroFillRShift,
        A::BitOrAssign => B::BitOr,
        A::BitXorAssign => B::BitXor,
        A::BitAndAssign => B::BitAnd,
        _ => return None,
    })
}

pub struct Scope {
    vars: RefCell<Vars>,
    /// Union of `name_bit` over the declared names (no false negatives).
    mask: Cell<u64>,
    parent: Option<Rc<Scope>>,
    strict: Cell<bool>,
    /// Object environment record of a `with` statement (its `mask` matches every name).
    with: Option<Obj>,
    /// Some scope on the chain (self included) is a `with` record.
    in_with: bool,
    /// The `this` bound by a non-arrow call frame, the global scope, or a class scope.
    this_val: RefCell<Option<Value>>,
}

impl Scope {
    fn new(parent: Option<Rc<Scope>>) -> Rc<Scope> {
        let strict = parent.as_ref().is_some_and(|p| p.strict.get());
        let in_with = parent.as_ref().is_some_and(|p| p.in_with);
        Rc::new(Scope {
            vars: RefCell::new(Vars::default()),
            mask: Cell::new(0),
            parent,
            strict: Cell::new(strict),
            with: None,
            in_with,
            this_val: RefCell::new(None),
        })
    }
    fn new_with(parent: Rc<Scope>, obj: Obj) -> Rc<Scope> {
        Rc::new(Scope {
            vars: RefCell::new(Vars::default()),
            mask: Cell::new(u64::MAX),
            strict: Cell::new(parent.strict.get()),
            parent: Some(parent),
            with: Some(obj),
            in_with: true,
            this_val: RefCell::new(None),
        })
    }
    fn declare(&self, name: &str, value: Value, mutable: bool) {
        self.declare_rc(name.into(), value, mutable);
    }
    fn set_this(&self, value: Value) {
        *self.this_val.borrow_mut() = Some(value);
    }
    fn declare_rc(&self, name: Rc<str>, value: Value, mutable: bool) {
        let bit = name_bit(&name);
        let maybe_present = self.mask.get() & bit != 0;
        self.mask.set(self.mask.get() | bit);
        let binding = Binding {
            value,
            mutable,
            origin: Origin::Decl,
        };
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
                origin: Origin::Decl,
            },
        );
    }
    /// A hoisted global `var` with `origin`: `undefined` unless the global already has `name`
    /// (an existing global keeps its origin).
    fn declare_global_var(&self, name: &Rc<str>, origin: Origin) {
        if self.vars.borrow().contains_key(name) {
            return;
        }
        self.declare_if_absent(name);
        self.set_origin(name, origin);
    }
    fn set_origin(&self, name: &str, origin: Origin) {
        if let Some(b) = self.vars.borrow_mut().get_mut(name) {
            b.origin = origin;
        }
    }
    fn copy_of(&self, parent: Rc<Scope>) -> Rc<Scope> {
        let mut vars = Vars::default();
        for (k, b) in self.vars.borrow().iter() {
            vars.insert(
                k.clone(),
                Binding {
                    value: b.value.clone(),
                    mutable: b.mutable,
                    origin: b.origin,
                },
            );
        }
        Rc::new(Scope {
            vars: RefCell::new(vars),
            mask: Cell::new(self.mask.get()),
            in_with: self.in_with || parent.in_with,
            parent: Some(parent),
            strict: Cell::new(self.strict.get()),
            with: self.with.clone(),
            this_val: RefCell::new(self.this_val.borrow().clone()),
        })
    }
}

/// Process-wide run counter: every realm draws from it, so `(run_id, span)` cache keys never collide.
static RUN_IDS: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

pub(crate) fn next_run_id() -> u32 {
    RUN_IDS.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1
}

/// Realm ids are process-wide (worker threads share the platform tables), starting at 1.
static REALM_IDS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(1);

fn next_realm_id() -> usize {
    REALM_IDS.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
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
        raw_props: RefCell::new(Vec::new()),
        lazy_proto: Cell::new(false),
        proto: RefCell::new(proto),
        kind: RefCell::new(kind),
        frozen: Cell::new(false),
        len_ro: Cell::new(false),
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
    // |n| < 2^63: `as i64` truncates toward zero (no libm `trunc` call) and `as i32` keeps the
    // low 32 bits, which is ToInt32. NaN fails this comparison too, so it reaches the check below.
    if n.abs() < 9.2e18 {
        return n as i64 as i32;
    }
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
    /// Shared `Rc<str>` for every name declared by `let`/`const`/class/hoisted function/catch/destructuring,
    /// so re-executing a declaration reuses one allocation. Per Interp: never shared across threads.
    // ponytail: grows with distinct declared names; eval-generated names are unbounded, cap it if that shows.
    names: std::collections::HashSet<Rc<str>, std::hash::BuildHasherDefault<FxHasher>>,
    /// Distinguishes source positions of different `run` calls (spans restart at 1 per parse).
    run_id: u32,
    /// Read-hint slots, one per annotated identifier (`hints.rs`). Slot 0 is never used.
    ident_hints: Vec<hints::HintSlot>,
    /// Hint counters (`THAW_JS_HINT_STATS=1`).
    hint_stats: hints::HintStats,
    /// Oracle mode: every hinted read also runs the by-name lookup and asserts they agree.
    check_hints: bool,
    /// `THAW_JS_COMPILE` mode, `THAW_JS_COMPILE_AFTER` call threshold, `THAW_JS_COMPILE_STATS` report flag.
    compile_mode: compile::CompileMode,
    compile_after: u32,
    compile_stats: bool,
    cstats: compile::Stats,
    /// Recycled slot buffers of compiled calls (`compile/run.rs`); at most `FRAME_POOL_CAP` entries, all empty.
    frame_pool: Vec<Vec<Value>>,
    /// Recycled argument buffers of compiled calls (`compile/call.rs`); at most `ARG_POOL_CAP` entries, all empty.
    arg_pool: Vec<Vec<Value>>,
    /// Process-unique id of this realm: tags the zlib jobs it starts so only it collects their results.
    pub(crate) realm_id: usize,
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
    /// `node:vm` contexts by id (vm.rs).
    pub(crate) vm: HashMap<u32, vm::VmSlot>,
    pub(crate) vm_next: u32,
    /// The sandbox of the `node:vm` run in progress, so `globalThis.x` / `this.x` reach it (vm.rs).
    pub(crate) vm_sandbox: Option<Obj>,
    /// When a `node:vm` run with a timeout must stop (vm.rs `arm`), and the timeout in ms.
    pub(crate) deadline: Option<std::time::Instant>,
    pub(crate) dl_ms: u64,
    /// The deadline has passed: sticky, so later checks and `catch` keep seeing the timeout.
    pub(crate) dl_hit: bool,
    pub(crate) dl_tick: u32,
    /// Set by a worker's parent thread (worker.rs `terminate`): checked by `check_interrupt`.
    pub(crate) interrupt: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
    /// Worker threads this Interp spawned, and its own parent channel on a worker thread (worker.rs).
    pub(crate) worker: worker::WorkerState,
    /// Run once per loop turn after the timers (host.rs sets it for the compiled terminal loop, which also
    /// drains native continuations). Returns true when it made progress, so the loop re-checks without sleeping.
    pub(crate) turn_hook: Option<fn(&mut Interp) -> bool>,
    /// Host time slice for the terminal multi-package loop (host.rs): past this instant the loop yields when
    /// another package has work. None (default) means run to quiescence.
    pub(crate) slice_end: Option<std::time::Instant>,
    /// Set by `loop_inner` when it returned because the slice ended, not because the loop went idle.
    pub(crate) yielded: bool,
    /// Function values made for compiled callbacks, by callback id (host.rs `callback_value`): stable identity.
    pub(crate) callback_values: HashMap<usize, Value>,
    /// The intrinsic `Symbol` constructor, its `Symbol.for` registry and the well-known symbols, captured at
    /// realm setup (collections.rs `install_collections`), so Rust never looks `Symbol` up in the global scope.
    pub(crate) symbol_ctor: Option<Obj>,
    pub(crate) symbol_registry: Option<Obj>,
    pub(crate) well_known: Vec<Obj>,
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
        Self::build(true)
    }

    /// A `node:vm` context realm: the ECMAScript globals only. The host-only installers (process, sysio,
    /// node/web modules, crypto, URL, Intl) are skipped; `vm.rs` would hide or discard them anyway.
    pub(crate) fn new_vm_realm() -> Interp {
        Self::build(false)
    }

    /// `host == false` builds a bare ECMAScript realm (see `new_vm_realm`).
    fn build(host: bool) -> Interp {
        let object_proto = new_obj(None, Kind::Plain);
        let mk = |op: &Obj| new_obj(Some(op.clone()), Kind::Plain);
        // Function.prototype is itself callable (typeof "function"), as in JS.
        let function_proto = new_obj(
            Some(object_proto.clone()),
            Kind::Native(Rc::new(
                |_: &mut Interp, _: &Value, _: &[Value]| -> R<Value> { Ok(Value::Undefined) },
            )),
        );
        let (array_proto, string_proto, regexp_proto) =
            (mk(&object_proto), mk(&object_proto), mk(&object_proto));
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
            names: Default::default(),
            run_id: 0,
            ident_hints: vec![hints::HintSlot::default()],
            hint_stats: hints::HintStats::from_env(),
            check_hints: cfg!(test) || std::env::var_os("THAW_JS_CHECK_HINTS").is_some(),
            compile_mode: compile::CompileMode::parse(
                std::env::var("THAW_JS_COMPILE").ok().as_deref(),
            ),
            compile_after: compile::parse_after(
                std::env::var("THAW_JS_COMPILE_AFTER").ok().as_deref(),
            ),
            compile_stats: compile::stats_enabled(
                std::env::var("THAW_JS_COMPILE_STATS").ok().as_deref(),
            ),
            cstats: compile::Stats::with_level(compile::stats_level(
                std::env::var("THAW_JS_COMPILE_STATS").ok().as_deref(),
            )),
            frame_pool: Vec::new(),
            arg_pool: Vec::new(),
            realm_id: next_realm_id(),
            depth: 0,
            max_depth: MAX_DEPTH,
            stack_limit: 0,
            out: Vec::new(),
            err: Vec::new(),
            rt: Default::default(),
            vm: HashMap::new(),
            vm_next: 0,
            vm_sandbox: None,
            deadline: None,
            dl_ms: 0,
            dl_hit: false,
            dl_tick: 0,
            interrupt: None,
            worker: Default::default(),
            turn_hook: None,
            slice_end: None,
            yielded: false,
            callback_values: HashMap::new(),
            symbol_ctor: None,
            symbol_registry: None,
            well_known: Vec::new(),
        };
        it.install_builtins();
        it.install_collections();
        it.install_eventloop();
        it.install_generators();
        it.install_bigint();
        it.install_numfmt();
        it.install_proxy();
        it.install_reflect();
        it.run(builtins::PRELUDE)
            .unwrap_or_else(|_| panic!("thaw-js prelude must evaluate"));
        it.run_promise_prelude();
        if host {
            it.install_main_file_loader();
        }
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
        if host {
            it.install_sysio();
            it.install_process_stdin();
            napi_bridge::install(&mut it);
            net::install(&mut it);
            worker::install(&mut it);
            dns::install(&mut it);
            vm::install(&mut it);
            zlib::install(&mut it);
            tls::install(&mut it);
            it.install_fs_host();
            web_dom_exception::install(&mut it);
            web_text_codec::install(&mut it);
            web_event_target::install(&mut it);
            web_messaging::install(&mut it);
            web_performance::install(&mut it);
            web_worker_stream::install(&mut it);
            web_worker_wire::install(&mut it);
            web_worker_threads::install(&mut it);
            web_abort::install(&mut it);
            web_blob::install(&mut it);
            web_fetch::install(&mut it);
            web_streams::install(&mut it);
            web_async_hooks::install(&mut it);
            web_wasm::install(&mut it);
            web_crypto::install(&mut it);
            web_navigator::install(&mut it);
            node_crypto::install(&mut it);
            it.install_url();
            #[cfg(feature = "intl")]
            it.install_intl();
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

    /// Errors with `Ctl::Exit(1)` once the parent of this worker has asked it to terminate. Not catchable by JS.
    #[inline]
    pub(crate) fn check_interrupt(&self) -> R<()> {
        match &self.interrupt {
            Some(flag) if flag.load(std::sync::atomic::Ordering::Relaxed) => Err(Ctl::Exit(1)),
            _ => Ok(()),
        }
    }

    /// Errors with `ERR_SCRIPT_EXECUTION_TIMEOUT` once the `node:vm` deadline has passed. Clock reads
    /// are sampled every 1024 checks. Also the point where a worker's interrupt is observed.
    #[inline]
    pub(crate) fn check_deadline(&mut self) -> R<()> {
        self.check_interrupt()?;
        let Some(d) = self.deadline else {
            return Ok(());
        };
        self.dl_tick = self.dl_tick.wrapping_add(1);
        if !self.dl_hit && self.dl_tick & 1023 != 0 {
            return Ok(());
        }
        if !self.dl_hit && std::time::Instant::now() < d {
            return Ok(());
        }
        self.dl_hit = true;
        // building the Error calls `call`, which would come back here: take the deadline meanwhile
        self.deadline = None;
        let r = self.throw_error::<()>(
            "Error",
            &format!("Script execution timed out after {}ms", self.dl_ms),
        );
        self.deadline = Some(d);
        if let Err(Ctl::Throw(Value::Object(o))) = &r {
            put(o, "code", Value::str("ERR_SCRIPT_EXECUTION_TIMEOUT"));
        }
        r
    }

    #[inline]
    pub(crate) fn too_deep(&self) -> bool {
        if self.depth >= self.max_depth {
            return true;
        }
        let marker = 0u8;
        let sp = &marker as *const u8 as usize;
        // below the configured limit is only deep if this thread's own stack agrees: compiled code may
        // enter from a thread other than the one that configured the limit (an async continuation)
        self.stack_limit != 0 && sp < self.stack_limit && !thread_stack_has_room(sp)
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
                .raw_props
                .borrow()
                .iter()
                .map(|p| p.key.to_string()),
        );
        names.extend(
            self.global_object
                .raw_props
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
                    .props()
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
                names.extend(o.raw_props.borrow().iter().map(|p| p.key.to_string()));
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
                "ref",
                "unref",
                "hasRef",
                "refresh",
            ]
            .map(String::from),
        ); // iterator protocol / `module.require` (user-defined) / stream controller and reader methods / timer handles
        names
    }

    // ----- conversions -----

    pub fn to_str(&mut self, v: &Value) -> JsStr {
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

    pub(crate) fn join(&mut self, this: &Value, sep: &str) -> JsStr {
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

    /// The string `Symbol.toStringTag` of a plain object on `Object.prototype` (Node prints it as
    /// `Object [Tag] {..}`); `None` for anything else.
    pub(crate) fn tagged_plain(&mut self, o: &Obj) -> Option<Rc<str>> {
        if !matches!(&*o.kind.borrow(), Kind::Plain) {
            return None;
        }
        let on_object_proto = o
            .proto
            .borrow()
            .as_ref()
            .is_none_or(|q| Rc::ptr_eq(q, &self.object_proto));
        if !on_object_proto {
            return None;
        }
        let key = self.well_known_key("toStringTag");
        match find_in_chain(o, &key) {
            Some(Slot::Data(Value::Str(tag))) => Some(tag.into_rc()),
            _ => None,
        }
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
                    | Kind::CollIter(_)
                    | Kind::Generator(_)
                    | Kind::Proxy(_)
                    | Kind::RawJson(_)
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
                            let items: Vec<String> = t
                                .to_values()
                                .iter()
                                .map(|v| match v {
                                    Value::BigInt(b) => format!("{b}n"),
                                    Value::Number(n) => number_to_string(*n),
                                    _ => String::new(),
                                })
                                .collect();
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
                            .raw_props
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
                        let head = match self.tagged_plain(o) {
                            Some(tag) => format!("Object [{tag}] "),
                            // a class instance shows its constructor name, as in Node (`Navigator {}`)
                            None => match class_instance_name(self, o) {
                                Some(name) => format!("{name} "),
                                None => String::new(),
                            },
                        };
                        if props.is_empty() {
                            format!("{head}{{}}")
                        } else {
                            let parts: Vec<String> = props
                                .iter()
                                .map(|p| match &p.slot {
                                    Slot::Data(val) => {
                                        format!("{}: {}", p.key, self.inspect(val, false))
                                    }
                                    Slot::Accessor { .. } => format!("{}: [Getter/Setter]", p.key),
                                    // non-enumerable, so filtered out above
                                    Slot::LazyProto { .. } => {
                                        unreachable!("placeholder is not enumerable")
                                    }
                                })
                                .collect();
                            format!("{head}{{ {} }}", parts.join(", "))
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
                                Value::Str(s) => Some(s.into_rc()),
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
                    .raw_props
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
        let holder: Obj;
        let start: &ObjData = match target {
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
                    return Ok(Value::Number(s.len16() as f64));
                }
                if let Some(i) = array_index(key) {
                    return Ok(s.unit_at(i).map_or(Value::Undefined, |u| {
                        Value::Str(builtins::units_str(&[u]).into())
                    }));
                }
                holder = self.string_proto.clone();
                &holder
            }
            Value::Number(_) => {
                holder = self.number_proto.clone();
                &holder
            }
            Value::Bool(_) => {
                holder = self.bool_proto.clone();
                &holder
            }
            Value::BigInt(_) => {
                holder = self
                    .rt
                    .bigint_proto
                    .clone()
                    .unwrap_or_else(|| self.object_proto.clone());
                &holder
            }
            Value::Object(o) => {
                // One kind inspection picks the special case; the borrow is dropped before any
                // JS (proxy traps, global lookups) runs.
                let fast = match &*o.kind.borrow() {
                    Kind::Global => Fast::Global,
                    Kind::Proxy(_) => Fast::Proxy,
                    Kind::Array(items) => {
                        if key == "length" {
                            Fast::Hit(Value::Number(items.len() as f64))
                        } else {
                            array_index(key)
                                .map_or(Fast::Chain, |i| Fast::Hit(unhole(items.get(i))))
                        }
                    }
                    Kind::Wrapper(Value::Str(s)) => {
                        if key == "length" {
                            Fast::Hit(Value::Number(s.len16() as f64))
                        } else {
                            array_index(key)
                                .and_then(|i| s.unit_at(i))
                                .map_or(Fast::Chain, |u| {
                                    Fast::Hit(Value::Str(builtins::units_str(&[u]).into()))
                                })
                        }
                    }
                    Kind::Typed(t) => match key {
                        "length" => Fast::Hit(Value::Number(t.len as f64)),
                        "byteLength" => Fast::Hit(Value::Number((t.len * t.ty.size()) as f64)),
                        "byteOffset" => Fast::Hit(Value::Number(t.off as f64)),
                        "buffer" => Fast::Hit(Value::Object(t.abuf.clone())),
                        _ => array_index(key).map_or(Fast::Chain, |i| {
                            Fast::Hit(t.get_value(i).unwrap_or(Value::Undefined))
                        }),
                    },
                    Kind::ArrayBuffer(b) if key == "byteLength" => {
                        Fast::Hit(Value::Number(b.borrow().len() as f64))
                    }
                    Kind::Closure { def, .. }
                        if (key == "name" || key == "length") && find_own(o, key).is_none() =>
                    {
                        Fast::Hit(closure_name_or_length(def, key))
                    }
                    _ => Fast::Chain,
                };
                match fast {
                    Fast::Hit(v) => return Ok(v),
                    Fast::Global => {
                        if let Some(v) = self.lookup(&self.global.clone(), key) {
                            return Ok(v);
                        }
                        if let Some(sb) = &self.vm_sandbox {
                            if let Some(Slot::Data(v)) = find_in_chain(sb, key) {
                                return Ok(v);
                            }
                        }
                    }
                    Fast::Proxy => {
                        if let Some(v) = self.proxy_get(o, key, target)? {
                            return Ok(v);
                        }
                    }
                    Fast::Chain => {}
                }
                o
            }
        };
        match read_in_chain(start, key) {
            None => Ok(Value::Undefined),
            Some(Read::Data(v)) => Ok(v),
            Some(Read::Getter(g)) => self.call(&g, target, &[]),
            Some(Read::NoGetter) => Ok(Value::Undefined),
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
            if let Some(sb) = &self.vm_sandbox {
                if find_in_chain(sb, key).is_some() {
                    put(sb, key, value);
                    return Ok(());
                }
            }
            return self.assign_var(&self.global.clone(), key, value);
        }
        let len_ro = o.len_ro.get();
        if let Kind::Array(items) = &mut *o.kind.borrow_mut() {
            if let Some(i) = array_index(key) {
                if len_ro && i >= items.len() {
                    return Ok(()); // sloppy mode: silently ignored (read-only `length` blocks growth)
                }
                if i >= items.len() {
                    items.resize(i + 1, Value::Hole);
                }
                items[i] = value;
                return Ok(());
            }
            if key == "length" {
                if len_ro {
                    return Ok(()); // sloppy mode: silently ignored
                }
                let n = match value {
                    Value::Number(n) if n >= 0.0 => n as usize,
                    _ => 0,
                };
                items.resize(n, Value::Hole);
                return Ok(());
            }
        }
        let typed = match &*o.kind.borrow() {
            Kind::Typed(t) => Some(t.clone()),
            _ => None,
        };
        if let (Some(t), Some(i)) = (typed, array_index(key)) {
            t.set_value(self, i, &value)?;
            return Ok(());
        }
        match find_in_chain(o, key) {
            Some(Slot::Accessor { set, .. }) => {
                if let Some(s) = set {
                    self.call(&s, target, &[value])?;
                }
                return Ok(());
            }
            Some(Slot::LazyProto { .. }) => unreachable!("`find_own` forces the key"),
            Some(Slot::Data(_)) => {
                // an inherited or own read-only data property ignores the write
                let mut cur = Some(o.clone());
                while let Some(c) = cur {
                    if let Some(p) = c.raw_props.borrow().iter().find(|p| &*p.key == key) {
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

    /// The text of a "not a function" error for `v`: its `typeof`, then the constructor name and
    /// a short preview when they add something, e.g. `object is not a function (constructor Foo)`.
    pub(crate) fn not_callable_message(&mut self, v: &Value) -> String {
        let kind = match v {
            Value::Undefined | Value::Hole => "undefined",
            Value::Null => "object",
            Value::Bool(_) => "boolean",
            Value::Number(_) => "number",
            Value::Str(_) => "string",
            Value::BigInt(_) => "bigint",
            Value::Object(o) if matches!(&*o.kind.borrow(), Kind::Symbol { .. }) => "symbol",
            Value::Object(_) => "object",
        };
        let mut detail = Vec::new();
        if let Value::Object(o) = v {
            if let Some(Slot::Data(Value::Object(c))) = find_in_chain(o, "constructor") {
                if let Some((Slot::Data(Value::Str(name)), _)) = find_own(&c, "name") {
                    if !name.is_empty() {
                        detail.push(format!("constructor {name}"));
                    }
                }
            }
        }
        let shown = self.inspect(v, false);
        let shown = if shown.chars().count() > 60 {
            format!("{}...", shown.chars().take(60).collect::<String>())
        } else {
            shown
        };
        if shown != kind {
            detail.push(format!("value {shown}"));
        }
        if detail.is_empty() {
            format!("{kind} is not a function")
        } else {
            format!("{kind} is not a function ({})", detail.join(", "))
        }
    }

    pub fn call(&mut self, f: &Value, this: &Value, args: &[Value]) -> R<Value> {
        let Value::Object(fo) = f else {
            let msg = self.not_callable_message(f);
            return self.throw_error("TypeError", &msg);
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
            _ => {
                let msg = self.not_callable_message(f);
                return self.throw_error("TypeError", &msg);
            }
        };
        if self.too_deep() {
            return self.throw_error("RangeError", "Maximum call stack size exceeded");
        }
        self.check_deadline()?;
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
        // the body's spans belong to the script it was parsed from, which may be another realm's
        let caller_run = std::mem::replace(&mut self.run_id, def.src.0);
        let r = self.call_closure_body(f, def, scope, this, args);
        self.run_id = caller_run;
        r
    }

    fn call_closure_body(
        &mut self,
        f: &Value,
        def: &Rc<FuncDef>,
        scope: &Rc<Scope>,
        this: &Value,
        args: &[Value],
    ) -> R<Value> {
        let new_target = self.rt.pending_nt.take();
        if self.compile_mode != compile::CompileMode::Off {
            if let Some(r) = self.try_compiled(f, def, scope, this, args, &new_target) {
                return r;
            }
        }
        self.call_closure_ast(f, def, scope, this, args, new_target)
    }

    /// The AST body of a call: the path of every function that is not (or not yet) compiled.
    fn call_closure_ast(
        &mut self,
        f: &Value,
        def: &Rc<FuncDef>,
        scope: &Rc<Scope>,
        this: &Value,
        args: &[Value],
        new_target: Option<Value>,
    ) -> R<Value> {
        let frame = Scope::new(Some(scope.clone()));
        // the parameters are the bulk of a frame's bindings: size the list once and seed the filter
        frame.mask.set(def.param_mask);
        let n = def.params.len() + def.hoisted.len();
        if n > 0 {
            frame.vars.borrow_mut().list.reserve(n);
        }
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
                frame.set_this(this);
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
                // the filter already holds every parameter bit: no hash, and a duplicate name
                // (sloppy `function f(a, a)`) still overwrites its slot through `insert`
                frame.vars.borrow_mut().insert(
                    name.clone(),
                    Binding {
                        value,
                        mutable: true,
                        origin: Origin::Decl,
                    },
                );
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
        self.construct_with(f, args, f)
    }

    /// [[Construct]](f, args, newTarget): the instance takes its prototype from `new_target.prototype`.
    pub(crate) fn construct_with(
        &mut self,
        f: &Value,
        args: &[Value],
        new_target: &Value,
    ) -> R<Value> {
        let Value::Object(fo) = f else {
            return self.throw_error("TypeError", "value is not a constructor");
        };
        if matches!(&*fo.kind.borrow(), Kind::Proxy(p) if p.callable) {
            return self.proxy_construct(fo, args, new_target);
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
        let proto = match self.get(new_target, "prototype")? {
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
            // a subclass or Reflect.construct(native, args, other): the fresh object takes the new target's
            // prototype. ponytail: an object passed in by the caller is returned as-is and left alone.
            if let Value::Object(ro) = &r {
                let is_arg = args
                    .iter()
                    .any(|a| matches!(a, Value::Object(x) if Rc::ptr_eq(x, ro)));
                if !is_arg && !matches!(new_target, Value::Object(n) if Rc::ptr_eq(n, fo)) {
                    *ro.proto.borrow_mut() = Some(proto);
                }
            }
            return Ok(r);
        }
        let this = Value::Object(new_obj(Some(proto), Kind::Plain));
        self.rt.pending_nt = Some(new_target.clone());
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
        method: bool,
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
                let param_mask = param_mask(&param_names);
                Rc::new(FuncDef { name, params, body, arrow, is_async, is_gen, bind_self, no_prototype: !is_gen && (method || is_async), uses_arguments: finder.found && !arrow, uses_new_target: finder.new_target && !arrow, uses_this: finder.uses_this, src: (self_run_id, lo, hi), hoisted, param_names, param_mask, strict, class: None, cstate: Default::default() })
            })
            .clone();
        self.closure_from_def(def, scope)
    }

    /// A fresh function object for an already analysed definition.
    fn closure_from_def(&mut self, def: Rc<FuncDef>, scope: &Rc<Scope>) -> Value {
        let (arrow, is_async, is_gen, no_prototype) =
            (def.arrow, def.is_async, def.is_gen, def.no_prototype);
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
            put_with(
                &f,
                "prototype",
                Value::Object(new_obj(parent, Kind::Plain)),
                true,
                false,
                false,
            );
        } else if !arrow && !no_prototype {
            // ponytail: the object is built on first use (`ObjData::force_lazy`); the placeholder keeps
            // the key at this index with the same flags, so key order and descriptors do not change.
            let slot = Slot::LazyProto {
                owner: Rc::downgrade(&f),
                parent: self.object_proto.clone(),
            };
            push_prop(
                &f,
                &mut f.raw_props.borrow_mut(),
                Prop {
                    key: "prototype".into(),
                    slot,
                    enumerable: false,
                    writable: true,
                    configurable: false,
                },
            );
            f.lazy_proto.set(true);
        }
        Value::Object(f)
    }

    // ----- scopes -----

    /// The `this` of the nearest scope that binds one. `with` records never supply it.
    fn this_of(&self, scope: &Rc<Scope>) -> Value {
        let mut cur: &Scope = scope;
        loop {
            if let Some(v) = cur.this_val.borrow().as_ref() {
                return v.clone();
            }
            match cur.parent.as_deref() {
                Some(p) => cur = p,
                None => return Value::Undefined,
            }
        }
    }

    fn lookup(&self, scope: &Rc<Scope>, name: &str) -> Option<Value> {
        let bit = name_bit(name);
        let mut cur: &Scope = scope;
        loop {
            if cur.mask.get() & bit != 0 {
                if let Some(o) = &cur.with {
                    if let Some(v) = self.with_get(o, name) {
                        return Some(v);
                    }
                } else if let Some(b) = cur.vars.borrow().get(name) {
                    return Some(b.value.clone());
                }
            }
            cur = cur.parent.as_deref()?;
        }
    }

    /// `HasBinding` of an object environment record: the property exists and is not blocked by
    /// `Symbol.unscopables`. ponytail: accessors are skipped (no getter calls from `&self`).
    fn with_get(&self, o: &Obj, name: &str) -> Option<Value> {
        let Some(Slot::Data(v)) = find_in_chain(o, name) else {
            return None;
        };
        let id = match self.lookup(&self.global, "Symbol") {
            Some(Value::Object(sym)) => match find_in_chain(&sym, "unscopables") {
                Some(Slot::Data(Value::Object(u))) => match &*u.kind.borrow() {
                    Kind::Symbol { id, .. } => Some(*id),
                    _ => None,
                },
                _ => None,
            },
            _ => None,
        };
        if let Some(id) = id {
            if let Some(Slot::Data(Value::Object(u))) = find_in_chain(o, &format!("@@sym:{id}")) {
                if matches!(find_in_chain(&u, name), Some(Slot::Data(b)) if b.truthy()) {
                    return None;
                }
            }
        }
        Some(v)
    }

    /// The `with` object that resolves `name` (the `this` of a call through it), if any.
    fn with_base(&self, scope: &Rc<Scope>, name: &str) -> Option<Obj> {
        let bit = name_bit(name);
        let mut cur: &Scope = scope;
        loop {
            if cur.mask.get() & bit != 0 {
                match &cur.with {
                    Some(o) if self.with_get(o, name).is_some() => return Some(o.clone()),
                    Some(_) => {}
                    None if cur.vars.borrow().contains_key(name) => return None,
                    None => {}
                }
            }
            cur = cur.parent.as_deref()?;
        }
    }

    /// Global-scope binding `name` for `delete`: `Some(false)` for a var or function, `Some(true)`
    /// after removing an implicit global, `None` when it is not one of those (lexical or absent).
    pub(crate) fn delete_global_binding(&mut self, name: &str) -> Option<bool> {
        let g = self.global.clone();
        let origin = g.vars.borrow().get(name)?.origin;
        match origin {
            Origin::Decl => None,
            Origin::Var => Some(false),
            Origin::Prop => {
                g.vars.borrow_mut().remove(name);
                Some(true)
            }
        }
    }

    /// `delete name` (sloppy): only a global binding can be refused or removed.
    fn delete_ident(&mut self, scope: &Rc<Scope>, name: &str) -> bool {
        let bit = name_bit(name);
        let mut cur: Option<&Scope> = Some(scope);
        while let Some(s) = cur {
            if s.with.is_some() {
                return true;
            }
            if s.mask.get() & bit != 0 && s.vars.borrow().contains_key(name) {
                if !std::ptr::eq(s, &*self.global) {
                    return true;
                }
                return self.delete_global_binding(name).unwrap_or(true);
            }
            cur = s.parent.as_deref();
        }
        true
    }

    fn assign_var(&mut self, scope: &Rc<Scope>, name: &str, value: Value) -> R<()> {
        let bit = name_bit(name);
        let mut cur: Option<&Scope> = Some(scope);
        while let Some(s) = cur {
            if s.mask.get() & bit == 0 {
                cur = s.parent.as_deref();
                continue;
            }
            if let Some(o) = &s.with {
                if self.with_get(o, name).is_some() {
                    put(o, name, value);
                    return Ok(());
                }
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
        self.global.set_origin(name, Origin::Prop);
        Ok(())
    }

    /// The shared `Rc<str>` for a declared name (see `Interp::names`).
    fn intern(&mut self, name: &str) -> Rc<str> {
        if let Some(n) = self.names.get(name) {
            return n.clone();
        }
        let n: Rc<str> = name.into();
        self.names.insert(n.clone());
        n
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
                    let name = self.intern(&id.id.sym);
                    scope.declare_rc(name, value, true);
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
                    return self.throw_error("TypeError", "value is not iterable");
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
        self.run_parsed(source, thaw_parser::parse_javascript(source), true)
    }

    /// Like [`run`](Self::run) for `eval` / `new Function` text: parsed as a sloppy-mode script.
    pub fn run_sloppy(&mut self, source: &str) -> R<Value> {
        let parsed = thaw_parser::parse_javascript_script(source);
        if let Err(msg) = &parsed {
            return self.throw_error("SyntaxError", msg);
        }
        self.run_parsed(source, parsed, false)
    }

    /// `script`: a top-level script, whose vars and functions are non-configurable globals; an
    /// indirect `eval` declares configurable ones.
    fn run_parsed(
        &mut self,
        source: &str,
        parsed: Result<a::Module, String>,
        script: bool,
    ) -> R<Value> {
        let mut module = parsed.map_err(|e| Ctl::Throw(Value::str(&e)))?;
        hints::annotate(&mut module, &mut self.ident_hints);
        self.run_id = next_run_id();
        self.sources.insert(self.run_id, Rc::from(source));
        let mut stmts = Vec::new();
        for item in module.body {
            match item {
                a::ModuleItem::Stmt(s) => stmts.push(s),
                a::ModuleItem::ModuleDecl(_) => return unsupported("esm-syntax"),
            }
        }
        let global = self.global.clone();
        let origin = if script { Origin::Var } else { Origin::Prop };
        for name in &hoisted_names(&stmts) {
            global.declare_global_var(name, origin);
        }
        self.hoist_functions(&stmts, &global)?;
        for s in &stmts {
            if let a::Stmt::Decl(a::Decl::Fn(f)) = s {
                global.set_origin(&f.ident.sym, origin);
            }
        }
        self.run_hoisted(&stmts, &global)
    }

    /// Hoists the functions of `stmts` into `scope`, then runs them; the value is that of the last
    /// expression statement.
    fn run_stmts(&mut self, stmts: &[a::Stmt], scope: &Rc<Scope>) -> R<Value> {
        self.hoist_functions(stmts, scope)?;
        self.run_hoisted(stmts, scope)
    }

    /// Runs `stmts` whose functions are already hoisted; the value is that of the last expression
    /// statement.
    fn run_hoisted(&mut self, stmts: &[a::Stmt], scope: &Rc<Scope>) -> R<Value> {
        let mut last = Value::Undefined;
        for s in stmts {
            if let a::Stmt::Expr(e) = s {
                last = self.eval(&e.expr, scope)?;
            } else {
                self.exec(s, scope)?;
            }
        }
        Ok(last)
    }

    /// A `node:vm` script run in a context whose scope chain is `scope -> with(sandbox) -> global`
    /// (see vm.rs). Vars and functions are hoisted into `scope`, except names `sandbox` already has,
    /// so those stay bound to the sandbox property. Returns the value and the hoisted names.
    pub(crate) fn run_vm_script(
        &mut self,
        source: &str,
        scope: &Rc<Scope>,
        sandbox: &Obj,
    ) -> (R<Value>, Vec<Rc<str>>) {
        let mut module = match thaw_parser::parse_javascript_script(source) {
            Ok(m) => m,
            Err(_) => {
                return (
                    self.throw_error("SyntaxError", "Invalid or unexpected token"),
                    Vec::new(),
                )
            }
        };
        hints::annotate(&mut module, &mut self.ident_hints);
        self.run_id = next_run_id();
        self.sources.insert(self.run_id, Rc::from(source));
        let mut stmts = Vec::new();
        for item in module.body {
            match item {
                a::ModuleItem::Stmt(s) => stmts.push(s),
                a::ModuleItem::ModuleDecl(_) => return (unsupported("esm-syntax"), Vec::new()),
            }
        }
        let names = hoisted_names(&stmts);
        for name in &names {
            if find_in_chain(sandbox, name).is_none() {
                scope.declare_if_absent(name);
            }
        }
        (self.run_stmts(&stmts, scope), names)
    }

    /// Compile a function literal (`function (a, b) { ... }` expression source) in the global scope.
    pub fn eval_function(&mut self, source: &str) -> R<Value> {
        self.run_sloppy(&format!("({source})"))
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
        self.function_value_ex(name, f, scope, false)
    }

    /// A method / accessor: named, but its name is not a binding inside its own body.
    pub(crate) fn method_value(
        &mut self,
        name: Option<Rc<str>>,
        f: &a::Function,
        scope: &Rc<Scope>,
    ) -> R<Value> {
        self.function_value_ex(name, f, scope, true)
    }

    fn function_value_ex(
        &mut self,
        name: Option<Rc<str>>,
        f: &a::Function,
        scope: &Rc<Scope>,
        method: bool,
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
            !method,
            method,
            f.span.lo.0,
            f.span.hi.0,
            scope,
        ))
    }

    /// A function expression's value (`Expr::Fn`; compiled closures call it too).
    fn fn_expr_value(&mut self, f: &a::FnExpr, scope: &Rc<Scope>) -> R<Value> {
        let name = f.ident.as_ref().map(|i| Rc::<str>::from(i.sym.as_ref()));
        self.function_value(name, &f.function, scope)
    }

    /// An arrow function's value (`Expr::Arrow`; compiled closures call it too).
    fn arrow_value(&mut self, f: &a::ArrowExpr, scope: &Rc<Scope>) -> R<Value> {
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
            false,
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
                let name = self.intern(&f.ident.sym);
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
                scope.declare_rc(name, func, true);
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
                let name = self.intern(&c.ident.sym);
                scope.declare_rc(name, v, true);
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
                // a `node:vm` timeout cannot be caught by the script
                if let (Err(Ctl::Throw(v)), Some(h), false) = (&result, &t.handler, self.dl_hit) {
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
            a::Stmt::With(w) => {
                let Value::Object(o) = self.eval(&w.obj, scope)? else {
                    return self.throw_error("TypeError", "with: object expected");
                };
                self.exec(&w.body, &Scope::new_with(scope.clone(), o))
            }
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
                a::Pat::Ident(id) if !is_var => {
                    let name = self.intern(&id.id.sym);
                    scope.declare_rc(name, value, mutable);
                }
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
                    self.check_deadline()?;
                    if step(self.exec(&w.body, scope))? {
                        break;
                    }
                }
                Ok(())
            }
            a::Stmt::DoWhile(w) => {
                loop {
                    self.check_deadline()?;
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
                    self.check_deadline()?;
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
                            self.check_deadline()?;
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
                        self.check_deadline()?;
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
                    return self.throw_error("TypeError", "value is not iterable");
                };
                loop {
                    self.check_deadline()?;
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
                        for p in c.raw_props.borrow().iter().filter(|p| p.enumerable) {
                            if !keys.contains(&p.key) {
                                keys.push(p.key.clone());
                            }
                        }
                        cur = c.proto.borrow().clone();
                    }
                }
                for key in keys {
                    self.check_deadline()?;
                    let iter = Scope::new(Some(scope.clone()));
                    self.bind_for_head(&f.left, Value::Str(key.into()), &iter)?;
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
                    None => {
                        return self.throw_error(
                            "TypeError",
                            "Spread syntax requires ...iterable[Symbol.iterator] to be a function",
                        )
                    }
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
            a::Expr::This(_) => Ok(self.this_of(scope)),
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
                        Some(el) if el.spread.is_some() => {
                            let v = self.eval(&el.expr, scope)?;
                            match self.iter_protocol(&v)? {
                                Some(vals) => items.extend(vals),
                                None => {
                                    return self.throw_error("TypeError", "value is not iterable")
                                }
                            }
                        }
                        Some(el) => items.push(self.eval(&el.expr, scope)?),
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
                                if is_anonymous_fn_def(&kv.value) {
                                    name_anonymous_fn(&v, &k);
                                }
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
                                let f = self.method_value(
                                    Some(format!("get {k}").into()),
                                    &g.function,
                                    scope,
                                )?;
                                define_accessor(&obj, &k, Some(f), None, true);
                            }
                            a::Prop::Setter(s) => {
                                let k = self.prop_name(&s.key, scope)?;
                                let f = self.method_value(
                                    Some(format!("set {k}").into()),
                                    &s.function,
                                    scope,
                                )?;
                                define_accessor(&obj, &k, None, Some(f), true);
                            }
                            a::Prop::Assign(_) => return unsupported("shorthand default"),
                        },
                    }
                }
                Ok(Value::Object(obj))
            }
            a::Expr::Fn(f) => self.fn_expr_value(f, scope),
            a::Expr::Arrow(f) => self.arrow_value(f, scope),
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
                    a::Expr::SuperProp(_) => (self.eval(callee, scope)?, self.this_of(scope)),
                    a::Expr::Member(m) => {
                        let obj = self.eval(&m.obj, scope)?;
                        let key = self.member_key(&m.prop, scope)?;
                        (self.get(&obj, &key)?, obj)
                    }
                    a::Expr::Ident(i) if scope.in_with => (
                        self.eval(callee, scope)?,
                        self.with_base(scope, &i.sym)
                            .map_or(Value::Undefined, Value::Object),
                    ),
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
                if let a::Expr::Ident(i) = &*u.arg {
                    let inc = matches!(u.op, a::UpdateOp::PlusPlus);
                    let fast = with_binding_mut(scope, &i.sym, |b| {
                        if !b.mutable {
                            return None;
                        }
                        let Value::Number(old) = b.value else {
                            return None;
                        };
                        let new = if inc { old + 1.0 } else { old - 1.0 };
                        b.value = Value::Number(new);
                        Some(if u.prefix { new } else { old })
                    });
                    if let Some(v) = fast {
                        return Ok(Value::Number(v));
                    }
                }
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
                    if let (Value::Number(x), Value::Number(y)) = (&l, &r) {
                        if let Some(v) = number_binop(op, *x, *y) {
                            return Ok(v);
                        }
                    }
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
                let this = self.this_of(scope);
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
                            put_hidden(o, "raw", raw.clone());
                            if let Value::Object(r) = &raw {
                                crate::builtins::freeze_obj(r);
                            }
                            crate::builtins::freeze_obj(o);
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
                    a::Expr::Ident(i) => self.lookup_ident(scope, i).unwrap_or(Value::Undefined),
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
            a::UnaryOp::Delete => match &*u.arg {
                a::Expr::Member(m) => {
                    let obj = self.eval(&m.obj, scope)?;
                    let key = self.member_key(&m.prop, scope)?;
                    if let Value::Object(o) = obj {
                        return Ok(Value::Bool(crate::reflect::delete_own(self, &o, &key)?));
                    }
                    Ok(Value::Bool(true))
                }
                a::Expr::Ident(i) => Ok(Value::Bool(self.delete_ident(scope, &i.sym))),
                _ => Ok(Value::Bool(true)),
            },
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

    /// The value of a right-hand side that is a number literal or a variable holding a number.
    /// Reading it has no side effects, so it may be read before the left operand.
    fn pure_number(&self, scope: &Rc<Scope>, e: &a::Expr) -> Option<f64> {
        match e {
            a::Expr::Lit(a::Lit::Num(n)) => Some(n.value),
            a::Expr::Ident(i) => match self.lookup_ident(scope, i)? {
                Value::Number(n) => Some(n),
                _ => None,
            },
            _ => None,
        }
    }

    /// `x op= n` and `x = x op n` on a mutable number binding with a pure numeric `n`: one scope
    /// walk, no `binary()`. `None` leaves everything untouched for the generic path.
    fn ident_rmw_fast(&self, scope: &Rc<Scope>, name: &str, asg: &a::AssignExpr) -> Option<Value> {
        use a::AssignOp as A;
        use a::BinaryOp as B;
        let (op, rhs) = match asg.op {
            A::AddAssign => (B::Add, &*asg.right),
            A::SubAssign => (B::Sub, &*asg.right),
            A::MulAssign => (B::Mul, &*asg.right),
            A::DivAssign => (B::Div, &*asg.right),
            A::ModAssign => (B::Mod, &*asg.right),
            A::Assign => match &*asg.right {
                a::Expr::Bin(b)
                    if matches!(b.op, B::Add | B::Sub | B::Mul | B::Div | B::Mod)
                        && matches!(&*b.left, a::Expr::Ident(i) if &*i.sym == name) =>
                {
                    (b.op, &*b.right)
                }
                _ => return None,
            },
            _ => return None,
        };
        let r = self.pure_number(scope, rhs)?;
        with_binding_mut(scope, name, |b| {
            if !b.mutable {
                return None;
            }
            let Value::Number(l) = b.value else {
                return None;
            };
            let v = number_binop(op, l, r)?;
            b.value = v.clone();
            Some(v)
        })
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
        if let a::SimpleAssignTarget::Ident(i) = target {
            if let Some(v) = self.ident_rmw_fast(scope, &i.id.sym, asg) {
                return Ok(v);
            }
        }
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
                let mut cur = self.read_loc(&loc, scope)?;
                let rhs = self.eval(&asg.right, scope)?;
                // `s += x` with a primitive `x` (no user code can run in ToPrimitive)
                let append = matches!(op, A::AddAssign)
                    && matches!(cur, Value::Str(_))
                    && matches!(
                        rhs,
                        Value::Str(_)
                            | Value::Number(_)
                            | Value::Bool(_)
                            | Value::Null
                            | Value::Undefined
                            | Value::BigInt(_)
                    );
                if let (true, Loc::Ident(id)) = (append, &loc) {
                    let rs = self.to_str(&rhs);
                    if let Some(v) = append_in_place(scope, &id.sym, &mut cur, &rs) {
                        return Ok(v);
                    }
                }
                // both Numbers: no coercion, no user code (same operators as `number_binop`)
                if let (Value::Number(x), Value::Number(y)) = (&cur, &rhs) {
                    if let Some(v) = compound_binop(op).and_then(|b| number_binop(b, *x, *y)) {
                        self.write_loc(&loc, v.clone(), scope)?;
                        return Ok(v);
                    }
                }
                let Some(bop) = compound_binop(op) else {
                    return unsupported("assignment operator");
                };
                self.binary(bop, &cur, &rhs)?
            }
        };
        self.write_loc(&loc, value.clone(), scope)?;
        Ok(value)
    }

    fn eval_ident(&mut self, i: &a::Ident, scope: &Rc<Scope>) -> R<Value> {
        match self.lookup_ident(scope, i) {
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
                // Range first (rejects NaN), then an exact-integer round trip: no libm `fract` call.
                if *n >= 0.0 && *n < 4294967295.0 && (*n as u32) as f64 == *n {
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
                Kind::Typed(t) => return Ok(t.get_value(i).unwrap_or(Value::Undefined)),
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
                    // the operand is borrowed, so this is one copy into a buffer with slack
                    let mut s = self.to_str(&lp);
                    s.push_joined(&self.to_str(&rp));
                    Value::Str(s)
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
    o.force_key(key);
    let i = o.position(key)?;
    let props = o.raw_props.borrow();
    let p = &props[i];
    Some((p.slot.clone(), p.enumerable))
}

/// `name` or `length` of a closure that has no real `Prop` for it, derived from its definition.
fn closure_name_or_length(def: &FuncDef, key: &str) -> Value {
    if key == "name" {
        def.name
            .clone()
            .map_or(Value::str(""), |n| Value::Str(n.into()))
    } else {
        Value::Number(
            def.params
                .iter()
                .take_while(|p| matches!(p, a::Pat::Ident(_)))
                .count() as f64,
        )
    }
}

/// The synthesized own `name`/`length` of closure `o` (`None` for any other key or object).
/// Natives report neither (ceiling: no synthesized name/length for natives yet).
pub(crate) fn synth_fn_own(o: &Obj, key: &str) -> Option<Value> {
    match &*o.kind.borrow() {
        Kind::Closure { def, .. }
            if (key == "name" || key == "length") && find_own(o, key).is_none() =>
        {
            Some(closure_name_or_length(def, key))
        }
        _ => None,
    }
}

/// What a property read needs from the first holder that has the key. Data clones only the
/// value and an accessor clones only its getter.
enum Read {
    Data(Value),
    Getter(Value),
    NoGetter,
}

/// Outcome of a `get` special case decided under one kind borrow.
enum Fast {
    Hit(Value),
    Global,
    Proxy,
    Chain,
}

fn read_own(o: &ObjData, key: &str) -> Option<Read> {
    o.force_key(key);
    let i = o.position(key)?;
    let props = o.raw_props.borrow();
    Some(match &props[i].slot {
        Slot::Data(v) => Read::Data(v.clone()),
        Slot::Accessor { get: Some(g), .. } => Read::Getter(g.clone()),
        Slot::Accessor { .. } => Read::NoGetter,
        Slot::LazyProto { .. } => unreachable!("`force_key` runs before the slot is read"),
    })
}

/// Read `key` along the prototype chain of `o`, cloning one hop's Rc only when it moves up.
fn read_in_chain(o: &ObjData, key: &str) -> Option<Read> {
    if let Some(r) = read_own(o, key) {
        return Some(r);
    }
    let mut cur: Obj = o.proto.borrow().clone()?;
    loop {
        if let Some(r) = read_own(&cur, key) {
            return Some(r);
        }
        let next = cur.proto.borrow().clone()?;
        cur = next;
    }
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

/// Canonical array-index property key (`"0"`, `"17"`; not `"01"`, `"+1"`, `"1.0"`, `" 1"`, `"-0"`),
/// below 2^32 - 1. Every index-keyed site uses this one function.
#[inline]
pub(crate) fn array_index(k: &str) -> Option<usize> {
    let b = k.as_bytes();
    if b.is_empty() || !b[0].is_ascii_digit() || (b[0] == b'0' && b.len() > 1) || b.len() > 10 {
        return None;
    }
    k.parse::<u32>()
        .ok()
        .filter(|n| *n != u32::MAX)
        .map(|n| n as usize)
}

/// Enumeration order of own keys: array indices ascending, then the rest in insertion order.
/// The name of the class whose prototype `o` directly inherits from, if it is a named class other than
/// `Object` (`class A {}; new A()` -> "A"). Null-prototype and plain objects give None.
fn class_instance_name(it: &mut Interp, o: &Obj) -> Option<Rc<str>> {
    let proto = o.proto.borrow().clone()?;
    let Some((Slot::Data(c @ Value::Object(_)), _)) = find_own(&proto, "constructor") else {
        return None;
    };
    match it.get(&c, "name") {
        Ok(Value::Str(name)) if !name.is_empty() && &*name != "Object" => Some(name.into_rc()),
        _ => None,
    }
}

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
/// Whether the calling thread's own stack still has room at `sp` (same reserve as `configure_stack`).
#[cfg(target_os = "linux")]
fn thread_stack_has_room(sp: usize) -> bool {
    // SAFETY: plain pthread queries on the current thread with a zeroed attribute object.
    unsafe {
        let mut attr: libc::pthread_attr_t = std::mem::zeroed();
        if libc::pthread_getattr_np(libc::pthread_self(), &mut attr) != 0 {
            return false;
        }
        let mut addr: *mut libc::c_void = std::ptr::null_mut();
        let mut size: libc::size_t = 0;
        let ok = libc::pthread_attr_getstack(&attr, &mut addr, &mut size) == 0;
        libc::pthread_attr_destroy(&mut attr);
        ok && size != 0 && sp >= addr as usize + (size / 8 + (4 << 20)).min(size / 2)
    }
}

#[cfg(not(target_os = "linux"))]
fn thread_stack_has_room(_sp: usize) -> bool {
    false
}

pub(crate) fn put(o: &Obj, key: &str, value: Value) {
    let pos = o.position(key);
    let mut props = o.raw_props.borrow_mut();
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
                configurable: true,
            },
        ),
    }
}

/// ES NamedEvaluation: an anonymous function, arrow or class (possibly parenthesized)
/// takes the property key it is assigned to as its name.
fn is_anonymous_fn_def(e: &a::Expr) -> bool {
    match e {
        a::Expr::Paren(p) => is_anonymous_fn_def(&p.expr),
        a::Expr::Arrow(_) => true,
        a::Expr::Fn(f) => f.ident.is_none(),
        a::Expr::Class(c) => c.ident.is_none(),
        _ => false,
    }
}

/// Give an anonymous closure an own `name` (a function's `def.name` is shared by every
/// closure from the same source, so the name lives on the object). Named functions and
/// functions that already have an own `name` are left alone.
// ponytail: symbol keys are not named "[desc]" (key_of stringifies them); add when needed.
fn name_anonymous_fn(v: &Value, key: &str) {
    if let Value::Object(o) = v {
        let anonymous =
            matches!(&*o.kind.borrow(), Kind::Closure { def, .. } if def.name.is_none());
        if anonymous && find_own(o, "name").is_none() {
            put_with(o, "name", Value::str(key), false, false, true);
        }
    }
}

/// Create or overwrite a data property with explicit attributes (the general form of `put` and
/// `put_hidden`; overwrites every attribute of an existing property).
pub(crate) fn put_with(
    o: &Obj,
    key: &str,
    value: Value,
    writable: bool,
    enumerable: bool,
    configurable: bool,
) {
    let pos = o.position(key);
    let mut props = o.raw_props.borrow_mut();
    match pos.map(|i| &mut props[i]) {
        Some(p) => {
            p.slot = Slot::Data(value);
            p.writable = writable;
            p.enumerable = enumerable;
            p.configurable = configurable;
        }
        None => push_prop(
            o,
            &mut props,
            Prop {
                key: key.into(),
                slot: Slot::Data(value),
                enumerable,
                writable,
                configurable,
            },
        ),
    }
}

/// Create or overwrite a non-enumerable, writable data property (builtins).
pub(crate) fn put_hidden(o: &Obj, key: &str, value: Value) {
    let pos = o.position(key);
    let mut props = o.raw_props.borrow_mut();
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
                configurable: true,
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
    let mut props = o.raw_props.borrow_mut();
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
                configurable: true,
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

fn param_mask(names: &[Option<Rc<str>>]) -> u64 {
    names.iter().flatten().fold(0, |m, n| m | name_bit(n))
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

#[cfg(all(test, unix))]
mod fs_tests;
#[cfg(test)]
mod handle_json_argument_tests;
#[cfg(test)]
mod handle_zero_tests;
#[cfg(test)]
mod napi_bridge_tests;
#[cfg(test)]
mod napi_eval_tests;
#[cfg(test)]
mod napi_globals_tests;
#[cfg(test)]
mod napi_graph_encode_tests;
#[cfg(test)]
mod napi_graph_tests;
#[cfg(test)]
mod napi_ops_tests;
#[cfg(test)]
mod napi_props_tests;
#[cfg(test)]
mod napi_proxy_tests;
#[cfg(test)]
mod napi_symbol_protocol_tests;
#[cfg(test)]
mod napi_symbols_tests;
#[cfg(test)]
mod net_tests;
#[cfg(test)]
mod prop_array_length_tests;
#[cfg(test)]
mod prop_configurable_tests;
#[cfg(test)]
mod prop_get_tests;
#[cfg(test)]
mod proxy_trap_tests;
#[cfg(test)]
mod str_cursor_tests;
#[cfg(test)]
mod str_unit_tests;
#[cfg(test)]
mod sysio_stdin_tests;
#[cfg(test)]
mod tests;

#[cfg(test)]
mod coroutine_teardown_tests;
#[cfg(test)]
mod vm_realm_tests;
