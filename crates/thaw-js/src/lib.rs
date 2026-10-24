//! EXPERIMENTAL: prototype of milestone M1 of the Thaw-native JavaScript runtime
//! (design: `/home/fedora/work/thaw-tmp/designs/js-runtime/design.md`).
//!
//! A small tree-walking evaluator over the SWC AST produced by `thaw-parser`, for an ES5-ish
//! subset (functions/closures, objects, arrays, prototypes, `new`, exceptions, loops, a few
//! builtins, CommonJS `exports`). Anything outside the subset yields [`Ctl::Unsupported`] with
//! the feature name, which is the shape the future package-routing scan reports to
//! `thaw inspect`. Nothing here is wired into the compiler; it is not used by default.
//!
//! Simplifications (each is a known ceiling, upgrade path in the design doc):
//! * strings are `Rc<str>` (UTF-8), not Thaw's arena WTF-8 strings;
//! * properties are an insertion-ordered `Vec` with linear lookup (no shapes/atoms/ICs);
//! * scopes are `HashMap` chains (no slot resolution), no TDZ;
//! * integer-keyed object properties are not ordered first; arrays have no holes.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use thaw_parser::ast as a;

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
    props: RefCell<Vec<(Rc<str>, Value)>>,
    proto: RefCell<Option<Obj>>,
    kind: RefCell<Kind>,
}

enum Kind {
    Plain,
    Array(Vec<Value>),
    Closure { def: Rc<FuncDef>, scope: Rc<Scope> },
    Native(NativeFn),
}

type NativeFn = Rc<dyn Fn(&mut Interp, &Value, &[Value]) -> R<Value>>;

struct FuncDef {
    name: Option<Rc<str>>,
    params: Vec<a::Pat>,
    body: Body,
    arrow: bool,
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
}

pub type R<T> = Result<T, Ctl>;

struct Binding {
    value: Value,
    mutable: bool,
}

pub struct Scope {
    vars: RefCell<HashMap<Rc<str>, Binding>>,
    parent: Option<Rc<Scope>>,
}

impl Scope {
    fn new(parent: Option<Rc<Scope>>) -> Rc<Scope> {
        Rc::new(Scope {
            vars: RefCell::new(HashMap::new()),
            parent,
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
        Rc::new(Scope {
            vars: RefCell::new(vars),
            parent: Some(parent),
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
}

fn new_obj(proto: Option<Obj>, kind: Kind) -> Obj {
    Rc::new(ObjData {
        props: RefCell::new(Vec::new()),
        proto: RefCell::new(proto),
        kind: RefCell::new(kind),
    })
}

fn number_to_string(n: f64) -> String {
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

fn string_to_number(s: &str) -> f64 {
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

fn to_int32(n: f64) -> i32 {
    if !n.is_finite() {
        return 0;
    }
    (n.trunc().rem_euclid(4294967296.0) as u64 as u32) as i32
}

fn strict_equals(x: &Value, y: &Value) -> bool {
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
    fn_cache: HashMap<(u32, u32), Rc<FuncDef>>,
    /// Distinguishes source positions of different `run` calls (spans restart at 1 per parse).
    run_id: u32,
    depth: usize,
    /// Lines written by `console.log` (kept for tests and embedders).
    pub out: Vec<String>,
}

// Self-hosted pieces of the standard library: written in the supported subset on purpose.
const PRELUDE: &str = r#"
function Error(message) { this.message = message === undefined ? "" : String(message); }
Error.prototype.name = "Error";
Error.prototype.toString = function () { return this.message ? this.name + ": " + this.message : this.name; };
function TypeError(message) { this.message = String(message); }
TypeError.prototype = Object.create(Error.prototype); TypeError.prototype.name = "TypeError";
function RangeError(message) { this.message = String(message); }
RangeError.prototype = Object.create(Error.prototype); RangeError.prototype.name = "RangeError";
function ReferenceError(message) { this.message = String(message); }
ReferenceError.prototype = Object.create(Error.prototype); ReferenceError.prototype.name = "ReferenceError";
Array.prototype.forEach = function (f) { for (var i = 0; i < this.length; i++) f(this[i], i, this); };
Array.prototype.map = function (f) { var r = []; for (var i = 0; i < this.length; i++) r.push(f(this[i], i, this)); return r; };
Array.prototype.filter = function (f) { var r = []; for (var i = 0; i < this.length; i++) if (f(this[i], i, this)) r.push(this[i]); return r; };
Array.prototype.reduce = function (f, init) {
  var i = 0, acc = init;
  if (init === undefined && this.length > 0) { acc = this[0]; i = 1; } // ponytail: explicit undefined init is treated as absent
  for (; i < this.length; i++) acc = f(acc, this[i], i, this);
  return acc;
};
"#;

impl Interp {
    pub fn new() -> Interp {
        let object_proto = new_obj(None, Kind::Plain);
        let function_proto = new_obj(Some(object_proto.clone()), Kind::Plain);
        let array_proto = new_obj(Some(object_proto.clone()), Kind::Plain);
        let string_proto = new_obj(Some(object_proto.clone()), Kind::Plain);
        let mut it = Interp {
            global: Scope::new(None),
            object_proto,
            function_proto,
            array_proto,
            string_proto,
            fn_cache: HashMap::new(),
            run_id: 0,
            depth: 0,
            out: Vec::new(),
        };
        it.install_builtins();
        it.run(PRELUDE)
            .unwrap_or_else(|_| panic!("thaw-js prelude must evaluate"));
        it
    }

    fn native(&self, f: impl Fn(&mut Interp, &Value, &[Value]) -> R<Value> + 'static) -> Value {
        Value::Object(new_obj(
            Some(self.function_proto.clone()),
            Kind::Native(Rc::new(f)),
        ))
    }

    fn set_native(
        &self,
        target: &Obj,
        name: &str,
        f: impl Fn(&mut Interp, &Value, &[Value]) -> R<Value> + 'static,
    ) {
        let v = self.native(f);
        put(target, name, v);
    }

    fn install_builtins(&mut self) {
        let g = self.global.clone();
        let console = new_obj(Some(self.object_proto.clone()), Kind::Plain);
        self.set_native(&console, "log", |it, _, args| {
            let line = args
                .iter()
                .map(|v| it.inspect(v, true))
                .collect::<Vec<_>>()
                .join(" ");
            it.out.push(line);
            Ok(Value::Undefined)
        });
        g.declare("console", Value::Object(console), true);

        let math = new_obj(Some(self.object_proto.clone()), Kind::Plain);
        macro_rules! math1 {
            ($name:literal, $f:expr) => {
                self.set_native(&math, $name, |it, _, args| {
                    Ok(Value::Number($f(
                        it.num(args.first().unwrap_or(&Value::Undefined))
                    )))
                });
            };
        }
        math1!("floor", f64::floor);
        math1!("ceil", f64::ceil);
        math1!("abs", f64::abs);
        math1!("sqrt", f64::sqrt);
        math1!("trunc", f64::trunc);
        math1!("round", |x: f64| (x + 0.5).floor());
        self.set_native(&math, "pow", |it, _, args| {
            Ok(Value::Number(
                it.num(&arg(args, 0)).powf(it.num(&arg(args, 1))),
            ))
        });
        self.set_native(&math, "max", |it, _, args| {
            Ok(Value::Number(args.iter().fold(
                f64::NEG_INFINITY,
                |m, v| {
                    let n = it.num(v);
                    if n.is_nan() || m.is_nan() {
                        f64::NAN
                    } else {
                        m.max(n)
                    }
                },
            )))
        });
        self.set_native(&math, "min", |it, _, args| {
            Ok(Value::Number(args.iter().fold(f64::INFINITY, |m, v| {
                let n = it.num(v);
                if n.is_nan() || m.is_nan() {
                    f64::NAN
                } else {
                    m.min(n)
                }
            })))
        });
        put(&math, "PI", Value::Number(std::f64::consts::PI));
        g.declare("Math", Value::Object(math), true);

        let object = self.native(|_, _, args| Ok(arg(args, 0)));
        if let Value::Object(o) = &object {
            put(o, "prototype", Value::Object(self.object_proto.clone()));
            self.set_native(o, "keys", |it, _, args| {
                let keys = it.own_keys(&arg(args, 0));
                Ok(it.array(keys.into_iter().map(Value::Str).collect()))
            });
            self.set_native(o, "create", |_, _, args| {
                let proto = match arg(args, 0) {
                    Value::Object(p) => Some(p),
                    _ => None,
                };
                Ok(Value::Object(new_obj(proto, Kind::Plain)))
            });
        }
        g.declare("Object", object, true);

        let array = self.native(|it, _, args| Ok(it.array(args.to_vec())));
        if let Value::Object(o) = &array {
            put(o, "prototype", Value::Object(self.array_proto.clone()));
            self.set_native(o, "isArray", |_, _, args| Ok(Value::Bool(matches!(arg(args, 0), Value::Object(o) if matches!(&*o.kind.borrow(), Kind::Array(_))))));
        }
        g.declare("Array", array, true);

        g.declare(
            "String",
            self.native(|it, _, args| {
                Ok(Value::Str(if args.is_empty() {
                    "".into()
                } else {
                    it.to_str(&args[0])
                }))
            }),
            true,
        );
        g.declare(
            "Number",
            self.native(|it, _, args| {
                Ok(Value::Number(if args.is_empty() {
                    0.0
                } else {
                    it.num(&args[0])
                }))
            }),
            true,
        );
        g.declare(
            "isNaN",
            self.native(|it, _, args| Ok(Value::Bool(it.num(&arg(args, 0)).is_nan()))),
            true,
        );
        g.declare(
            "parseInt",
            self.native(|it, _, args| {
                let s = it.to_str(&arg(args, 0));
                let radix = match arg(args, 1) {
                    Value::Undefined => 10,
                    v => it.num(&v) as u32,
                };
                let t = s.trim();
                let (neg, t) = match t.strip_prefix('-') {
                    Some(r) => (true, r),
                    None => (false, t.strip_prefix('+').unwrap_or(t)),
                };
                let digits: String = t
                    .chars()
                    .take_while(|c| c.is_digit(radix.clamp(2, 36)))
                    .collect();
                Ok(Value::Number(
                    match i64::from_str_radix(&digits, radix.clamp(2, 36)) {
                        Ok(v) => {
                            if neg {
                                -(v as f64)
                            } else {
                                v as f64
                            }
                        }
                        Err(_) => f64::NAN,
                    },
                ))
            }),
            true,
        );
        g.declare("undefined", Value::Undefined, false);
        g.declare("NaN", Value::Number(f64::NAN), false);
        g.declare("Infinity", Value::Number(f64::INFINITY), false);

        // Array.prototype natives.
        let ap = self.array_proto.clone();
        self.set_native(&ap, "push", |it, this, args| {
            let Value::Object(o) = this else {
                return it.throw_error("TypeError", "push on non-array");
            };
            let mut kind = o.kind.borrow_mut();
            let Kind::Array(items) = &mut *kind else {
                drop(kind);
                return it.throw_error("TypeError", "push on non-array");
            };
            items.extend(args.iter().cloned());
            Ok(Value::Number(items.len() as f64))
        });
        self.set_native(&ap, "pop", |_, this, _| {
            if let Value::Object(o) = this {
                if let Kind::Array(items) = &mut *o.kind.borrow_mut() {
                    return Ok(items.pop().unwrap_or(Value::Undefined));
                }
            }
            Ok(Value::Undefined)
        });
        self.set_native(&ap, "join", |it, this, args| {
            let sep = match arg(args, 0) {
                Value::Undefined => ",".into(),
                v => it.to_str(&v),
            };
            Ok(Value::Str(it.join(this, &sep)))
        });
        self.set_native(&ap, "indexOf", |_, this, args| {
            let needle = arg(args, 0);
            if let Value::Object(o) = this {
                if let Kind::Array(items) = &*o.kind.borrow() {
                    return Ok(Value::Number(
                        items
                            .iter()
                            .position(|v| strict_equals(v, &needle))
                            .map_or(-1.0, |i| i as f64),
                    ));
                }
            }
            Ok(Value::Number(-1.0))
        });
        self.set_native(&ap, "slice", |it, this, args| {
            let items = match this {
                Value::Object(o) => match &*o.kind.borrow() {
                    Kind::Array(v) => v.clone(),
                    _ => Vec::new(),
                },
                _ => Vec::new(),
            };
            let (from, to) = it.range(args, items.len());
            Ok(it.array(items[from..to].to_vec()))
        });
        self.set_native(&ap, "concat", |it, this, args| {
            let mut items = match this {
                Value::Object(o) => match &*o.kind.borrow() {
                    Kind::Array(v) => v.clone(),
                    _ => Vec::new(),
                },
                _ => Vec::new(),
            };
            for v in args {
                match v {
                    Value::Object(o) if matches!(&*o.kind.borrow(), Kind::Array(_)) => {
                        if let Kind::Array(more) = &*o.kind.borrow() {
                            items.extend(more.iter().cloned())
                        }
                    }
                    other => items.push(other.clone()),
                }
            }
            Ok(it.array(items))
        });

        // String.prototype natives (UTF-16 semantics are an M2 item; indices here are chars).
        let sp = self.string_proto.clone();
        self.set_native(&sp, "charAt", |it, this, args| {
            let s = it.to_str(this);
            let i = it.num(&arg(args, 0));
            Ok(Value::Str(if i >= 0.0 {
                s.chars()
                    .nth(i as usize)
                    .map(String::from)
                    .unwrap_or_default()
                    .into()
            } else {
                "".into()
            }))
        });
        self.set_native(&sp, "charCodeAt", |it, this, args| {
            let s = it.to_str(this);
            let i = it.num(&arg(args, 0));
            Ok(Value::Number(if i >= 0.0 {
                s.chars()
                    .nth(i as usize)
                    .map_or(f64::NAN, |c| c as u32 as f64)
            } else {
                f64::NAN
            }))
        });
        self.set_native(&sp, "indexOf", |it, this, args| {
            let (s, n) = (it.to_str(this), it.to_str(&arg(args, 0)));
            Ok(Value::Number(
                s.find(&*n).map_or(-1.0, |b| s[..b].chars().count() as f64),
            ))
        });
        self.set_native(&sp, "toUpperCase", |it, this, _| {
            Ok(Value::Str(it.to_str(this).to_uppercase().into()))
        });
        self.set_native(&sp, "toLowerCase", |it, this, _| {
            Ok(Value::Str(it.to_str(this).to_lowercase().into()))
        });
        self.set_native(&sp, "trim", |it, this, _| {
            Ok(Value::Str(it.to_str(this).trim().into()))
        });
        self.set_native(&sp, "slice", |it, this, args| {
            let chars: Vec<char> = it.to_str(this).chars().collect();
            let (from, to) = it.range(args, chars.len());
            Ok(Value::Str(
                chars[from..to].iter().collect::<String>().into(),
            ))
        });
        self.set_native(&sp, "split", |it, this, args| {
            let s = it.to_str(this);
            let parts: Vec<Value> = match arg(args, 0) {
                Value::Undefined => vec![Value::Str(s)],
                sep => {
                    let sep = it.to_str(&sep);
                    if sep.is_empty() {
                        s.chars()
                            .map(|c| Value::Str(c.to_string().into()))
                            .collect()
                    } else {
                        s.split(&*sep).map(Value::str).collect()
                    }
                }
            };
            Ok(it.array(parts))
        });
    }

    /// Resolve `slice(start, end)`-style relative arguments.
    fn range(&mut self, args: &[Value], len: usize) -> (usize, usize) {
        let rel = |it: &mut Interp, v: Value, default: usize| -> usize {
            match v {
                Value::Undefined => default,
                v => {
                    let n = it.num(&v);
                    let n = if n.is_nan() { 0.0 } else { n.trunc() };
                    if n < 0.0 {
                        (len as f64 + n).max(0.0) as usize
                    } else {
                        (n as usize).min(len)
                    }
                }
            }
        };
        let from = rel(self, arg(args, 0), 0);
        let to = rel(self, arg(args, 1), len);
        (from, to.max(from))
    }

    fn array(&self, items: Vec<Value>) -> Value {
        Value::Object(new_obj(Some(self.array_proto.clone()), Kind::Array(items)))
    }

    fn join(&mut self, this: &Value, sep: &str) -> Rc<str> {
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
                if let Ok(Value::Object(f)) = self.get(v, "toString") {
                    if !matches!(&*f.kind.borrow(), Kind::Plain) {
                        if let Ok(Value::Str(s)) = self.call(&Value::Object(f.clone()), v, &[]) {
                            return s;
                        }
                    }
                }
                if matches!(&*o.kind.borrow(), Kind::Closure { .. } | Kind::Native(_)) {
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
            Value::Object(_) => {
                let s = self.to_str(v);
                string_to_number(&s)
            }
        }
    }

    /// `console.log`-style rendering (a small subset of Node's util.inspect).
    pub fn inspect(&mut self, v: &Value, top: bool) -> String {
        match v {
            Value::Str(s) if top => s.to_string(),
            Value::Str(s) => format!("'{s}'"),
            Value::Object(o) => {
                let kind = match &*o.kind.borrow() {
                    Kind::Array(items) => Some(Ok(items.clone())),
                    Kind::Closure { .. } | Kind::Native(_) => Some(Err(())),
                    Kind::Plain => None,
                };
                match kind {
                    Some(Ok(items)) if items.is_empty() => "[]".into(),
                    Some(Ok(items)) => format!(
                        "[ {} ]",
                        items
                            .iter()
                            .map(|i| self.inspect(i, false))
                            .collect::<Vec<_>>()
                            .join(", ")
                    ),
                    Some(Err(())) => match self.get(v, "name") {
                        Ok(Value::Str(n)) if !n.is_empty() => format!("[Function: {n}]"),
                        _ => "[Function (anonymous)]".into(),
                    },
                    None => {
                        let props = o.props.borrow().clone();
                        if props.is_empty() {
                            "{}".into()
                        } else {
                            format!(
                                "{{ {} }}",
                                props
                                    .iter()
                                    .map(|(k, val)| format!("{k}: {}", self.inspect(val, false)))
                                    .collect::<Vec<_>>()
                                    .join(", ")
                            )
                        }
                    }
                }
            }
            Value::Number(n) if *n == 0.0 && n.is_sign_negative() => "-0".into(),
            other => (*self.to_str(other)).to_string(),
        }
    }

    fn own_keys(&self, v: &Value) -> Vec<Rc<str>> {
        match v {
            Value::Object(o) => {
                let mut keys: Vec<Rc<str>> = Vec::new();
                if let Kind::Array(items) = &*o.kind.borrow() {
                    keys.extend((0..items.len()).map(|i| Rc::from(i.to_string())));
                }
                keys.extend(o.props.borrow().iter().map(|(k, _)| k.clone()));
                keys
            }
            Value::Str(s) => (0..s.chars().count())
                .map(|i| Rc::from(i.to_string()))
                .collect(),
            _ => Vec::new(),
        }
    }

    fn throw_error<T>(&mut self, class: &str, message: &str) -> R<T> {
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
        match target {
            Value::Undefined | Value::Null => {
                let what = if matches!(target, Value::Null) {
                    "null"
                } else {
                    "undefined"
                };
                self.throw_error(
                    "TypeError",
                    &format!("Cannot read properties of {what} (reading '{key}')"),
                )
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
                Ok(lookup_chain(&self.string_proto, key))
            }
            Value::Number(_) | Value::Bool(_) => Ok(lookup_chain(&self.object_proto, key)),
            Value::Object(o) => {
                if let Kind::Array(items) = &*o.kind.borrow() {
                    if key == "length" {
                        return Ok(Value::Number(items.len() as f64));
                    }
                    if let Ok(i) = key.parse::<usize>() {
                        return Ok(items.get(i).cloned().unwrap_or(Value::Undefined));
                    }
                }
                if key == "name" {
                    if let Kind::Closure { def, .. } = &*o.kind.borrow() {
                        if find_own(o, "name").is_none() {
                            return Ok(def.name.clone().map_or(Value::str(""), Value::Str));
                        }
                    }
                }
                Ok(lookup_chain(o, key))
            }
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
        put(o, key, value);
        Ok(())
    }

    fn key_of(&mut self, v: &Value) -> Rc<str> {
        self.to_str(v)
    }

    // ----- calls -----

    pub fn call(&mut self, f: &Value, this: &Value, args: &[Value]) -> R<Value> {
        let Value::Object(fo) = f else {
            return self.throw_error("TypeError", "value is not a function");
        };
        enum Callee {
            Native(NativeFn),
            Closure(Rc<FuncDef>, Rc<Scope>),
        }
        let callee = match &*fo.kind.borrow() {
            Kind::Native(n) => Callee::Native(n.clone()),
            Kind::Closure { def, scope } => Callee::Closure(def.clone(), scope.clone()),
            _ => return self.throw_error("TypeError", "value is not a function"),
        };
        if self.depth >= MAX_DEPTH {
            return self.throw_error("RangeError", "Maximum call stack size exceeded");
        }
        self.depth += 1;
        let result = match callee {
            Callee::Native(n) => n(self, this, args),
            Callee::Closure(def, scope) => self.call_closure(f, &def, &scope, this, args),
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
        let frame = Scope::new(Some(scope.clone()));
        if !def.arrow {
            frame.declare("this", this.clone(), false);
            if let Some(name) = &def.name {
                if self.lookup_local(&frame, name).is_none() {
                    frame.declare(name, f.clone(), true);
                }
            }
        }
        for (i, p) in def.params.iter().enumerate() {
            match p {
                a::Pat::Ident(id) => frame.declare(
                    &id.id.sym,
                    args.get(i).cloned().unwrap_or(Value::Undefined),
                    true,
                ),
                a::Pat::Assign(d) => {
                    let a::Pat::Ident(id) = &*d.left else {
                        return unsupported("destructuring parameter");
                    };
                    let v = match args.get(i) {
                        Some(Value::Undefined) | None => self.eval(&d.right, &frame)?,
                        Some(v) => v.clone(),
                    };
                    frame.declare(&id.id.sym, v, true);
                }
                a::Pat::Rest(r) => {
                    let a::Pat::Ident(id) = &*r.arg else {
                        return unsupported("destructuring rest parameter");
                    };
                    frame.declare(
                        &id.id.sym,
                        self.array(args.get(i..).unwrap_or(&[]).to_vec()),
                        true,
                    );
                }
                _ => return unsupported("destructuring parameter"),
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
        let is_ctor = matches!(&*fo.kind.borrow(), Kind::Closure { def, .. } if !def.arrow);
        if !is_ctor {
            return self.throw_error("TypeError", "value is not a constructor");
        }
        let proto = match self.get(f, "prototype")? {
            Value::Object(p) => p,
            _ => self.object_proto.clone(),
        };
        let this = Value::Object(new_obj(Some(proto), Kind::Plain));
        match self.call(f, &this, args)? {
            r @ Value::Object(_) => Ok(r),
            _ => Ok(this),
        }
    }

    fn make_function(
        &mut self,
        name: Option<Rc<str>>,
        params: Vec<a::Pat>,
        body: Body,
        arrow: bool,
        lo: u32,
        scope: &Rc<Scope>,
    ) -> Value {
        let def = self
            .fn_cache
            .entry((self.run_id, lo))
            .or_insert_with(|| {
                Rc::new(FuncDef {
                    name,
                    params,
                    body,
                    arrow,
                })
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
            put(&proto, "constructor", Value::Object(f.clone()));
            put(&f, "prototype", Value::Object(proto));
        }
        Value::Object(f)
    }

    // ----- scopes -----

    fn lookup_local(&self, scope: &Rc<Scope>, name: &str) -> Option<Value> {
        scope.vars.borrow().get(name).map(|b| b.value.clone())
    }

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
                a::ModuleItem::ModuleDecl(_) => return unsupported("ES module syntax"),
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

    /// Run a CommonJS-style bundle and return `module.exports`.
    pub fn run_commonjs(&mut self, source: &str) -> R<Value> {
        let module = Value::Object(new_obj(Some(self.object_proto.clone()), Kind::Plain));
        let exports = Value::Object(new_obj(Some(self.object_proto.clone()), Kind::Plain));
        self.set(&module, "exports", exports.clone())?;
        self.global.declare("module", module.clone(), true);
        self.global.declare("exports", exports, true);
        self.run(source)?;
        self.get(&module, "exports")
    }

    /// Call `exports[name](...args)`.
    pub fn call_export(&mut self, exports: &Value, name: &str, args: &[Value]) -> R<Value> {
        let f = self.get(exports, name)?;
        self.call(&f, exports, args)
    }

    // ----- statements -----

    fn hoist_functions(&mut self, stmts: &[a::Stmt], scope: &Rc<Scope>) -> R<()> {
        for s in stmts {
            if let a::Stmt::Decl(a::Decl::Fn(f)) = s {
                if f.function.is_async || f.function.is_generator {
                    return unsupported("async/generator function");
                }
                let name: Rc<str> = f.ident.sym.as_ref().into();
                let func = self.make_function(
                    Some(name.clone()),
                    fn_pats(&f.function.params),
                    Body::Block(
                        f.function
                            .body
                            .as_ref()
                            .map(|b| b.stmts.clone())
                            .unwrap_or_default(),
                    ),
                    false,
                    f.function.span.lo.0,
                    scope,
                );
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
            a::Stmt::Empty(_) => Ok(()),
            a::Stmt::Block(b) => self.exec_block(&b.stmts, scope),
            a::Stmt::Decl(a::Decl::Fn(_)) => Ok(()), // hoisted
            a::Stmt::Decl(a::Decl::Var(v)) => self.exec_var(v, scope),
            a::Stmt::Decl(_) => unsupported("class declaration"),
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
                    if let Some(a::Pat::Ident(id)) = &h.param {
                        catch_scope.declare(&id.id.sym, v.clone(), true);
                    } else if h.param.is_some() {
                        return unsupported("destructuring catch parameter");
                    }
                    result = self.exec_stmts(&h.body.stmts, &catch_scope);
                }
                if let Some(f) = &t.finalizer {
                    self.exec_block(&f.stmts, scope)?; // an abrupt finalizer overrides, as in JS
                }
                result
            }
            _ => unsupported("statement kind"),
        }
    }

    fn exec_var(&mut self, v: &a::VarDecl, scope: &Rc<Scope>) -> R<()> {
        let mutable = !matches!(v.kind, a::VarDeclKind::Const);
        for d in &v.decls {
            let a::Pat::Ident(id) = &d.name else {
                return unsupported("destructuring declaration");
            };
            let value = match &d.init {
                Some(e) => self.eval(e, scope)?,
                None if matches!(v.kind, a::VarDeclKind::Var) => continue, // hoisted already
                None => Value::Undefined,
            };
            if matches!(v.kind, a::VarDeclKind::Var) {
                self.assign_var(scope, &id.id.sym, value)?;
            } else {
                scope.declare(&id.id.sym, value, mutable);
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
                let items: Vec<Value> = match &iterable {
                    Value::Object(o) => match &*o.kind.borrow() {
                        Kind::Array(v) => v.clone(),
                        _ => return unsupported("for-of over non-array iterable"),
                    },
                    Value::Str(s) => s
                        .chars()
                        .map(|c| Value::Str(c.to_string().into()))
                        .collect(),
                    _ => return unsupported("for-of over non-array iterable"),
                };
                for item in items {
                    let iter = Scope::new(Some(scope.clone()));
                    self.bind_for_head(&f.left, item, &iter)?;
                    if step(self.exec(&f.body, &iter))? {
                        break;
                    }
                }
                Ok(())
            }
            a::Stmt::ForIn(f) => {
                let target = self.eval(&f.right, scope)?;
                for key in self.own_keys(&target) {
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
                Some(a::Pat::Ident(id)) => {
                    scope.declare(&id.id.sym, value, !matches!(v.kind, a::VarDeclKind::Const));
                    Ok(())
                }
                _ => unsupported("destructuring for-head"),
            },
            _ => unsupported("for-head without declaration"),
        }
    }

    // ----- expressions -----

    fn eval_args(&mut self, args: &[a::ExprOrSpread], scope: &Rc<Scope>) -> R<Vec<Value>> {
        let mut out = Vec::with_capacity(args.len());
        for arg in args {
            let v = self.eval(&arg.expr, scope)?;
            if arg.spread.is_some() {
                match &v {
                    Value::Object(o) => match &*o.kind.borrow() {
                        Kind::Array(items) => out.extend(items.iter().cloned()),
                        _ => return unsupported("spread of non-array"),
                    },
                    _ => return unsupported("spread of non-array"),
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
            a::MemberProp::PrivateName(_) => unsupported("private names"),
        }
    }

    fn eval(&mut self, e: &a::Expr, scope: &Rc<Scope>) -> R<Value> {
        match e {
            a::Expr::Lit(l) => Ok(match l {
                a::Lit::Num(n) => Value::Number(n.value),
                a::Lit::Str(s) => Value::str(s.value.as_str().unwrap_or("")), // ponytail: lone surrogates lost
                a::Lit::Bool(b) => Value::Bool(b.value),
                a::Lit::Null(_) => Value::Null,
                _ => return unsupported("literal kind (regexp/bigint)"),
            }),
            a::Expr::Ident(i) => match self.lookup(scope, &i.sym) {
                Some(v) => Ok(v),
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
                let obj = new_obj(Some(self.object_proto.clone()), Kind::Plain);
                for p in &o.props {
                    let a::PropOrSpread::Prop(p) = p else {
                        return unsupported("object spread");
                    };
                    match &**p {
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
                            let f = self.make_function(
                                Some(k.clone()),
                                fn_pats(&m.function.params),
                                Body::Block(
                                    m.function
                                        .body
                                        .as_ref()
                                        .map(|b| b.stmts.clone())
                                        .unwrap_or_default(),
                                ),
                                false,
                                m.function.span.lo.0,
                                scope,
                            );
                            put(&obj, &k, f);
                        }
                        _ => return unsupported("accessor property"),
                    }
                }
                Ok(Value::Object(obj))
            }
            a::Expr::Fn(f) => {
                if f.function.is_async || f.function.is_generator {
                    return unsupported("async/generator function");
                }
                let name = f.ident.as_ref().map(|i| Rc::<str>::from(i.sym.as_ref()));
                Ok(self.make_function(
                    name,
                    fn_pats(&f.function.params),
                    Body::Block(
                        f.function
                            .body
                            .as_ref()
                            .map(|b| b.stmts.clone())
                            .unwrap_or_default(),
                    ),
                    false,
                    f.function.span.lo.0,
                    scope,
                ))
            }
            a::Expr::Arrow(f) => {
                if f.is_async || f.is_generator {
                    return unsupported("async/generator function");
                }
                let body = match &*f.body {
                    a::ArrowFunctionBody::FunctionBody(b) => Body::Block(b.stmts.clone()),
                    a::ArrowFunctionBody::Expr(e) => Body::Expr(e.clone()),
                };
                Ok(self.make_function(None, f.params.clone(), body, true, f.span.lo.0, scope))
            }
            a::Expr::Member(m) => {
                let obj = self.eval(&m.obj, scope)?;
                let key = self.member_key(&m.prop, scope)?;
                self.get(&obj, &key)
            }
            a::Expr::Call(c) => {
                let a::Callee::Expr(callee) = &c.callee else {
                    return unsupported("super/import call");
                };
                let (f, this) = match &**callee {
                    a::Expr::Member(m) => {
                        let obj = self.eval(&m.obj, scope)?;
                        let key = self.member_key(&m.prop, scope)?;
                        (self.get(&obj, &key)?, obj)
                    }
                    other => (self.eval(other, scope)?, Value::Undefined),
                };
                let args = self.eval_args(&c.args, scope)?;
                if !matches!(&f, Value::Object(o) if matches!(&*o.kind.borrow(), Kind::Closure { .. } | Kind::Native(_)))
                {
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
            _ => unsupported("expression kind"),
        }
    }

    fn prop_name(&mut self, key: &a::PropName, scope: &Rc<Scope>) -> R<Rc<str>> {
        Ok(match key {
            a::PropName::Ident(i) => i.sym.as_ref().into(),
            a::PropName::Str(s) => s.value.as_str().unwrap_or("").into(),
            a::PropName::Num(n) => number_to_string(n.value).into(),
            a::PropName::Computed(c) => {
                let v = self.eval(&c.expr, scope)?;
                self.key_of(&v)
            }
            _ => return unsupported("property name kind"),
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
                    Value::Object(o) => {
                        if matches!(&*o.kind.borrow(), Kind::Closure { .. } | Kind::Native(_)) {
                            "function"
                        } else {
                            "object"
                        }
                    }
                }))
            }
            a::UnaryOp::Delete => {
                let a::Expr::Member(m) = &*u.arg else {
                    return Ok(Value::Bool(true));
                };
                let obj = self.eval(&m.obj, scope)?;
                let key = self.member_key(&m.prop, scope)?;
                if let Value::Object(o) = obj {
                    o.props.borrow_mut().retain(|(k, _)| **k != *key);
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
            a::AssignTarget::Pat(_) => return unsupported("destructuring assignment"),
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
        let num = |it: &mut Interp| (it.num(l), it.num(r));
        Ok(match op {
            B::Add => {
                let prim = |it: &mut Interp, v: &Value| {
                    if matches!(v, Value::Object(_)) {
                        Value::Str(it.to_str(v))
                    } else {
                        v.clone()
                    }
                };
                let (lp, rp) = (prim(self, l), prim(self, r));
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
                let key = self.to_str(l);
                let Value::Object(o) = r else {
                    return self
                        .throw_error("TypeError", "Cannot use 'in' operator on a non-object");
                };
                let is_index = matches!(&*o.kind.borrow(), Kind::Array(items) if key.parse::<usize>().is_ok_and(|i| i < items.len()));
                Value::Bool(
                    is_index
                        || !matches!(lookup_chain(o, &key), Value::Undefined)
                        || find_own(o, &key).is_some(),
                )
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

fn arg(args: &[Value], i: usize) -> Value {
    args.get(i).cloned().unwrap_or(Value::Undefined)
}

fn find_own(o: &Obj, key: &str) -> Option<Value> {
    o.props
        .borrow()
        .iter()
        .find(|(k, _)| &**k == key)
        .map(|(_, v)| v.clone())
}

fn put(o: &Obj, key: &str, value: Value) {
    let mut props = o.props.borrow_mut();
    match props.iter_mut().find(|(k, _)| &**k == key) {
        Some((_, v)) => *v = value,
        None => props.push((key.into(), value)),
    }
}

fn lookup_chain(o: &Obj, key: &str) -> Value {
    let mut cur = Some(o.clone());
    while let Some(c) = cur {
        if let Some(v) = find_own(&c, key) {
            return v;
        }
        cur = c.proto.borrow().clone();
    }
    Value::Undefined
}

/// Declare every `var` in `stmts` (not crossing function boundaries) as `undefined` in `scope`.
fn hoist_vars(stmts: &[a::Stmt], scope: &Rc<Scope>) {
    fn pat(p: &a::Pat, scope: &Rc<Scope>) {
        if let a::Pat::Ident(id) = p {
            if !scope.vars.borrow().contains_key(&*id.id.sym) {
                scope.declare(&id.id.sym, Value::Undefined, true);
            }
        }
    }
    fn walk(s: &a::Stmt, scope: &Rc<Scope>) {
        match s {
            a::Stmt::Decl(a::Decl::Var(v)) if matches!(v.kind, a::VarDeclKind::Var) => {
                v.decls.iter().for_each(|d| pat(&d.name, scope))
            }
            a::Stmt::Block(b) => b.stmts.iter().for_each(|s| walk(s, scope)),
            a::Stmt::If(i) => {
                walk(&i.cons, scope);
                if let Some(alt) = &i.alt {
                    walk(alt, scope);
                }
            }
            a::Stmt::For(f) => {
                if let Some(a::VarDeclOrExpr::VarDecl(v)) = &f.init {
                    if matches!(v.kind, a::VarDeclKind::Var) {
                        v.decls.iter().for_each(|d| pat(&d.name, scope));
                    }
                }
                walk(&f.body, scope);
            }
            a::Stmt::ForIn(f) => {
                hoist_head(&f.left, scope);
                walk(&f.body, scope)
            }
            a::Stmt::ForOf(f) => {
                hoist_head(&f.left, scope);
                walk(&f.body, scope)
            }
            a::Stmt::While(w) => walk(&w.body, scope),
            a::Stmt::DoWhile(w) => walk(&w.body, scope),
            a::Stmt::Labeled(l) => walk(&l.body, scope),
            a::Stmt::Try(t) => {
                t.block.stmts.iter().for_each(|s| walk(s, scope));
                if let Some(h) = &t.handler {
                    h.body.stmts.iter().for_each(|s| walk(s, scope));
                }
                if let Some(f) = &t.finalizer {
                    f.stmts.iter().for_each(|s| walk(s, scope));
                }
            }
            a::Stmt::Switch(sw) => sw
                .cases
                .iter()
                .for_each(|c| c.cons.iter().for_each(|s| walk(s, scope))),
            _ => {}
        }
    }
    fn hoist_head(h: &a::ForHead, scope: &Rc<Scope>) {
        if let a::ForHead::VarDecl(v) = h {
            if matches!(v.kind, a::VarDeclKind::Var) {
                v.decls.iter().for_each(|d| pat(&d.name, scope));
            }
        }
    }
    stmts.iter().for_each(|s| walk(s, scope));
}

#[cfg(test)]
mod tests;
