//! Thaw's typed intermediate representation. Definitions here mirror the
//! design doc (section 6) plus the small amount of scaffolding (`HirStmt`,
//! `HirParam`, `BinOp`, `FfiSignature`) that section only sketched.
//!
//! Phase 0 only *populates* a thin slice of this shape (see `lower.rs`):
//! top-level functions with number/string/boolean/void params and returns,
//! literals, identifiers, binary ops, and calls. The rest of the enum
//! (`Promise`, `Union`, `FfiCall`, `DynamicCall`, ...) exists so the shape
//! matches the target design, but nothing constructs those variants yet.

mod lower;

pub use lower::lower_module;

pub type Symbol = String;

#[derive(Debug, Clone, PartialEq)]
pub enum HirType {
    F64,
    I64,
    Bool,
    Void,
    Str,
    Json,
    Promise(Box<HirType>),
    Array(Box<HirType>),
    Object(Vec<(Symbol, HirType)>),
    Union(Vec<HirType>),
    /// Type inference failed / not yet supported for this expression ->
    /// falls back to QuickJS-NG at runtime (see design doc section 3.1).
    Dynamic,
}

#[derive(Debug, Clone, PartialEq)]
pub enum HirLit {
    F64(f64),
    Str(String),
    Bool(bool),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Lt,
    Gt,
    EqEqEq,
}

#[derive(Debug, Clone, PartialEq)]
pub struct HirParam {
    pub name: Symbol,
    pub ty: HirType,
}

/// A native function signature, used by `HirExpr::FfiCall` once thaw-bridge
/// starts generating these from `.d.ts` files. Unused by Phase 0's lowering.
#[derive(Debug, Clone, PartialEq)]
pub struct FfiSignature {
    pub symbol: Symbol,
    pub params: Vec<HirType>,
    pub ret: HirType,
}

#[derive(Debug, Clone, PartialEq)]
pub enum HirExpr {
    Lit(HirLit),
    Var(Symbol),
    BinOp(BinOp, Box<HirExpr>, Box<HirExpr>),
    Call(Box<HirExpr>, Vec<HirExpr>),
    Await(Box<HirExpr>),
    Lambda(Vec<HirParam>, Box<HirExpr>),
    Block(Vec<HirStmt>),
    FfiCall(FfiSignature, Vec<HirExpr>),
    DynamicCall(Box<HirExpr>, Vec<HirExpr>),
    /// `name = value` (and desugared compound assignments / `++`/`--`).
    /// Evaluates to `value`.
    Assign(Symbol, Box<HirExpr>),
    /// Phase 1 arrays are number-only at codegen time (see hir_codegen);
    /// the HIR shape itself doesn't enforce that.
    ArrayLit(Vec<HirExpr>),
    /// `array[index]`
    Index(Box<HirExpr>, Box<HirExpr>),
    /// `array[index] = value` (and desugared compound forms). Evaluates to
    /// `value`.
    IndexAssign(Box<HirExpr>, Box<HirExpr>, Box<HirExpr>),
    /// `array.length`. Not a general property-access node -- Phase 1 has no
    /// object/member model yet, so this is special-cased at lowering time
    /// the same way `console.log` is.
    ArrayLen(Box<HirExpr>),
    /// `process.env.NAME`. Same story as `ArrayLen`: no general member
    /// model yet, just this one special-cased builtin (Phase 2).
    EnvVar(Symbol),
}

#[derive(Debug, Clone, PartialEq)]
pub enum HirStmt {
    Expr(HirExpr),
    Return(Option<HirExpr>),
    Let(Symbol, HirType, HirExpr),
    If(HirExpr, Vec<HirStmt>, Vec<HirStmt>),
    While(HirExpr, Vec<HirStmt>),
    Throw(HirExpr),
    /// Phase 1 try/catch is intentionally limited: a `throw` only unwinds to
    /// the nearest lexically-enclosing `try` *in the same HIR function* --
    /// there is no real stack unwinding across function calls yet (that
    /// needs either LLVM's invoke/landingpad machinery or a setjmp/longjmp
    /// runtime, both deferred). `catch` binds the thrown value as a string.
    Try(Vec<HirStmt>, Symbol, Vec<HirStmt>),
}

#[derive(Debug, Clone, PartialEq)]
pub struct HirFunction {
    pub name: Symbol,
    pub params: Vec<HirParam>,
    pub ret: HirType,
    pub body: Vec<HirStmt>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct HirProgram {
    pub functions: Vec<HirFunction>,
}
