//! The lattice: `Bottom` (no information yet) < `Set` (a union of concrete atoms) < `Any(reason)`.
//! Arrays and fixed-shape objects are abstract allocation sites (union-find), so aliasing through
//! variables, arguments and returns is tracked soundly; a site that dynamic code can reach is `Dead`.

use std::collections::BTreeMap;

/// Why a value is `any`. The histogram of these is the work list for the next milestone.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Reason {
    Eval,
    This,
    Arguments,
    Async,
    Generator,
    Exotic,
    UnknownGlobal,
    UnknownCall,
    UnmodeledMethod,
    DynamicKey,
    Proto,
    Closure,
    Escaped,
    Tainted,
    Regex,
    New,
    Destructure,
    OptChain,
    Spread,
    Class,
    Coercion,
    Catch,
    BigInt,
    Iterate,
    Require,
    ShapeChange,
    /// member / call of a Node built-in that has no table yet
    Builtin,
    /// the `.d.ts` itself says `any` / generic / function / unsupported
    Declared,
    Other,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Atom {
    Undef,
    Null,
    Num,
    Str,
    Bool,
    /// string literal type (interned); widened to `Str` when stored in a mutable binding
    SLit(u32),
    /// integer literal type
    NLit(i32),
    Arr(usize),
    Obj(usize),
    /// a specific function (declaration / literal / arrow): lets calls through values be resolved
    Fn(u32),
    /// a Node built-in namespace (`require('path')`, `console`, `process`, ...), see `builtins`
    Ns(u8),
}

#[derive(Clone, Debug, Default, PartialEq)]
pub enum Ty {
    #[default]
    Bottom,
    Set(Vec<Atom>),
    Any(Reason),
}

impl Ty {
    pub fn one(a: Atom) -> Ty {
        Ty::Set(vec![a])
    }
    pub fn num() -> Ty {
        Ty::one(Atom::Num)
    }
    pub fn str() -> Ty {
        Ty::one(Atom::Str)
    }
    pub fn bool() -> Ty {
        Ty::one(Atom::Bool)
    }
    pub fn undef() -> Ty {
        Ty::one(Atom::Undef)
    }
    /// Non-empty and made only of primitives (never an object, array or `any`).
    pub fn prim_only(&self) -> bool {
        matches!(self, Ty::Set(s) if s.iter().all(|a| !matches!(a, Atom::Arr(_) | Atom::Obj(_) | Atom::Fn(_) | Atom::Ns(_))))
    }
    /// Every atom is a string (or string literal).
    pub fn is_str(&self) -> bool {
        matches!(self, Ty::Set(s) if s.iter().all(|a| matches!(a, Atom::Str | Atom::SLit(_))))
    }
    pub fn is_num(&self) -> bool {
        matches!(self, Ty::Set(s) if s.iter().all(|a| matches!(a, Atom::Num | Atom::NLit(_))))
    }
    pub fn only(&self, a: Atom) -> bool {
        matches!(self, Ty::Set(s) if s.len() == 1 && s[0] == a)
    }
}

#[derive(Clone, Debug)]
pub enum Site {
    Arr(Ty),
    Obj(BTreeMap<String, Ty>),
    /// Reachable by dynamic code or of unknown shape: nothing may be assumed about it.
    Dead(Reason),
    /// Merged into another site (union-find parent).
    Link(usize),
}

/// How well a slot is typed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Class {
    /// One concrete type (or never assigned).
    Concrete,
    /// `T | undefined`, `T | null`, `T | null | undefined`.
    Nullable,
    /// `number | string`, `T[] | U`, ... (a real union).
    Union,
    /// Never called and never escaping.
    Dead,
    Any,
}
