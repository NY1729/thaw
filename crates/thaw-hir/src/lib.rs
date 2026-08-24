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

pub use lower::{lower_module, lower_module_with_source_map, LowerDiagnostic, SourceRange};

pub type Symbol = String;

#[derive(Debug, Clone, PartialEq)]
pub enum HirType {
    F64,
    I64,
    Bool,
    Undefined,
    Null,
    Void,
    Str,
    Json,
    /// Opaque callable/object value retained by the embedded JavaScript realm.
    JsValue,
    Promise(Box<HirType>),
    Array(Box<HirType>),
    Tuple(Vec<HirType>),
    Object(Vec<(Symbol, HirType)>),
    Function(Vec<HirType>, Box<HirType>),
    Union(Vec<HirType>),
    /// A native `T | undefined` value represented as an explicit presence
    /// tag plus a payload. This avoids sentinel collisions with NaN/pointers.
    Optional(Box<HirType>),
    /// A native `T | null` value with the same tagged physical layout as an
    /// optional, but a distinct absence kind for equality/typeof/display.
    Nullable(Box<HirType>),
    /// A native `T | null | undefined` value. Its tag distinguishes a
    /// payload, `null`, and `undefined` without relying on sentinels.
    Nullish(Box<HirType>),
    /// Type inference failed / not yet supported for this expression ->
    /// falls back to QuickJS-NG at runtime (see design doc section 3.1).
    Dynamic,
}

#[derive(Debug, Clone, PartialEq)]
pub enum HirLit {
    F64(f64),
    Str(String),
    Bool(bool),
    Undefined,
    Null,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Exp,
    BitOr,
    BitXor,
    BitAnd,
    LShift,
    RShift,
    ZeroFillRShift,
    Lt,
    Gt,
    LtEq,
    GtEq,
    EqEqEq,
}

#[derive(Debug, Clone, PartialEq)]
pub struct HirParam {
    pub name: Symbol,
    pub ty: HirType,
}

/// A native function signature, used by `HirExpr::FfiCall`. Built today
/// from ambient `declare function` statements (see docs/design/bridge.md);
/// thaw-bridge will eventually also generate these from `.d.ts` files for
/// whole npm packages, via the same type-classification rules.
#[derive(Debug, Clone, PartialEq)]
pub enum FfiErrorAbi {
    Direct,
    ThawResult,
}

#[derive(Debug, Clone, PartialEq)]
pub enum FfiOwnership {
    Borrowed,
    Owned { destroy: Symbol },
    ArenaCopy { destroy: Option<Symbol> },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FfiStringAbi {
    NullTerminated,
    PointerLength,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FfiCallingConvention {
    C,
    Fast,
    Cold,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FfiAggregateAbi {
    Internal,
    Portable,
    Packed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FfiAggregateLayout {
    pub field_offsets: Vec<u64>,
    pub field_layouts: Vec<Option<Box<FfiAggregateLayout>>>,
    pub size: u64,
    pub alignment: u32,
    pub indirect: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FfiSignature {
    pub symbol: Symbol,
    pub params: Vec<HirType>,
    /// Element type of a trailing C varargs sequence. Lowering accepts
    /// TypeScript rest declarations of number, boolean, or string arrays.
    pub variadic: Option<HirType>,
    pub ret: HirType,
    pub error_abi: FfiErrorAbi,
    pub return_ownership: FfiOwnership,
    pub error_ownership: FfiOwnership,
    pub param_string_abis: Vec<FfiStringAbi>,
    pub return_string_abi: FfiStringAbi,
    pub calling_convention: FfiCallingConvention,
    pub aggregate_return_abi: FfiAggregateAbi,
    pub aggregate_return_layout: Option<FfiAggregateLayout>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DynamicBackend {
    QuickJs,
    Napi,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DynamicSignature {
    pub backend: DynamicBackend,
    pub symbol: Symbol,
    pub params: Vec<HirType>,
    pub ret: HirType,
}

#[derive(Debug, Clone, PartialEq)]
pub enum HirExpr {
    Lit(HirLit),
    Var(Symbol),
    BinOp(BinOp, Box<HirExpr>, Box<HirExpr>),
    OptionalSome(Box<HirExpr>, HirType),
    OptionalNone(HirType),
    OptionalIsNone(Box<HirExpr>, HirType),
    OptionalValue(Box<HirExpr>, HirType),
    NullableSome(Box<HirExpr>, HirType),
    NullableNone(HirType),
    NullableIsNone(Box<HirExpr>, HirType),
    NullableValue(Box<HirExpr>, HirType),
    NullishSome(Box<HirExpr>, HirType),
    NullishNull(HirType),
    NullishUndefined(HirType),
    NullishIsNull(Box<HirExpr>, HirType),
    NullishIsUndefined(Box<HirExpr>, HirType),
    NullishIsNone(Box<HirExpr>, HirType),
    NullishValue(Box<HirExpr>, HirType),
    Call(Box<HirExpr>, Vec<HirExpr>),
    /// A homogeneous `Promise.all` join. The element type is retained so
    /// codegen can copy and later load non-number result slots correctly.
    PromiseAll(Vec<HirExpr>, HirType),
    /// A homogeneous `Promise.all` whose promises are supplied as an array.
    PromiseAllArray(Box<HirExpr>, HirType),
    /// A heterogeneous literal `Promise.all`, preserving each result type.
    PromiseAllTuple(Vec<HirExpr>, Vec<HirType>),
    /// A homogeneous `Promise.race` over a literal list.
    PromiseRace(Vec<HirExpr>, HirType),
    /// A homogeneous `Promise.race` supplied through an array value.
    PromiseRaceArray(Box<HirExpr>, HirType),
    /// A homogeneous `Promise.any` over a literal list.
    PromiseAny(Vec<HirExpr>, HirType),
    /// A homogeneous `Promise.any` supplied through an array value.
    PromiseAnyArray(Box<HirExpr>, HirType),
    /// A homogeneous `Promise.allSettled` over a literal list.
    PromiseAllSettled(Vec<HirExpr>, HirType),
    /// A homogeneous `Promise.allSettled` supplied through an array value.
    PromiseAllSettledArray(Box<HirExpr>, HirType),
    /// `new Promise<T>((resolve, reject) => ...)`.
    PromiseNew(Box<HirExpr>, HirType, bool),
    /// A typed `.then`/`.catch` continuation. `on_rejected` distinguishes
    /// catch from then while retaining the input and output native layouts.
    PromiseThen(Box<HirExpr>, Box<HirExpr>, HirType, HirType, bool, bool),
    /// A `.finally` continuation. The callback return type records whether
    /// codegen must wait for a returned Promise before forwarding settlement.
    PromiseFinally(Box<HirExpr>, Box<HirExpr>, HirType, HirType),
    /// A legacy/direct await form, retained for void event sources and the
    /// synchronous fallback path. Frame splitting extracts real suspension
    /// points before ordinary expression code generation.
    Await(Box<HirExpr>),
    /// Awaiting a Promise value loaded from a variable, parameter, or field.
    /// The resolved type is retained for coroutine-frame resume loads.
    AwaitPromise(Box<HirExpr>, HirType),
    /// A typed closure. `captures` contains the outer bindings referenced by
    /// the body in deterministic name order, followed by ordinary parameters,
    /// the return type, and the body itself.
    Lambda(Vec<HirParam>, Vec<HirParam>, HirType, Box<HirExpr>),
    /// A top-level function adapted to the closure ABI when used as a value.
    FunctionRef(String, Vec<HirType>, HirType),
    Block(Vec<HirStmt>),
    FfiCall(FfiSignature, Vec<HirExpr>),
    DynamicCall(DynamicSignature, Vec<HirExpr>),
    /// `name = value` (and desugared compound assignments / `++`/`--`).
    /// Evaluates to `value`.
    Assign(Symbol, Box<HirExpr>),
    /// Phase 1 arrays are number-only at codegen time (see hir_codegen);
    /// the HIR shape itself doesn't enforce that.
    ArrayLit(Vec<HirExpr>),
    /// Array literal containing one or more spreads. Every part is itself a
    /// homogeneous typed array and is evaluated once in source order.
    ArrayConcat(Vec<HirExpr>, HirType),
    /// Allocates an uninitialized homogeneous array with a runtime length.
    /// Lowering fills every slot before exposing the value.
    ArrayAlloc(Box<HirExpr>, HirType),
    /// Rewrites the visible length of an arena-owned array after lowering has
    /// initialized a prefix of a larger allocation.
    ArraySetLen(Box<HirExpr>, Box<HirExpr>, HirType),
    /// `array[index]`
    Index(Box<HirExpr>, Box<HirExpr>),
    /// `array[index]` with the statically resolved element type.
    TypedIndex(Box<HirExpr>, Box<HirExpr>, HirType),
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
    /// An object literal, fields in the order codegen should lay them out
    /// in memory. thaw-hir's lowering reorders a literal's fields to match
    /// its declared type (see `lower.rs`) before this node is built, so
    /// codegen never has to reconcile two different field orderings.
    /// Phase 2 codegen only supports `f64`-valued fields.
    ObjectLit(Vec<(Symbol, HirExpr)>),
    /// `object.field`. Unlike `ArrayLen`/`EnvVar`, this *is* a general
    /// member-access node -- but it still isn't resolved by a real type
    /// checker at codegen time, so lowering bakes in the object's full
    /// `HirType::Object(fields)` shape (field order = the field's byte
    /// offset in the arena-allocated buffer) so codegen doesn't need to
    /// re-derive it.
    PropAccess(Box<HirExpr>, HirType, Symbol),
    /// `object.field = value` (and desugared compound forms). Evaluates to
    /// `value`. Same `HirType` bookkeeping as `PropAccess`.
    PropAssign(Box<HirExpr>, HirType, Symbol, Box<HirExpr>),
    /// `json.field` where `json: HirType::Json` (a *dynamic* value, e.g.
    /// from `JSON.parse`) -- unlike `PropAccess`, there's no static field
    /// list to validate against or bake in; any name is accepted and
    /// resolved at runtime by thaw-std's `thaw_json_get`, returning
    /// another `Json` value (never a concrete type).
    JsonGet(Box<HirExpr>, Symbol),
    /// `json[index]` where `json: HirType::Json`. Same story as
    /// `JsonGet`, via `thaw_json_index`.
    JsonIndex(Box<HirExpr>, Box<HirExpr>),
    /// `Number(json)`: converts a `Json` leaf to `f64` (`0.0` if it isn't
    /// numeric -- there's no exception channel wired to this yet). Only
    /// valid on a `Json`-typed argument; lowering rejects anything else,
    /// since `Str`/`Array`/`Object`/`Json` all share the same pointer
    /// representation and codegen can't otherwise tell them apart.
    JsonAsNumber(Box<HirExpr>),
    /// `String(json)`: converts a `Json` leaf to `Str`. Same restriction
    /// as `JsonAsNumber`.
    JsonAsString(Box<HirExpr>),
    /// `Boolean(json)`: converts a `Json` leaf to `Bool`. Same restriction
    /// as `JsonAsNumber`.
    JsonAsBool(Box<HirExpr>),
}

#[derive(Debug, Clone, PartialEq)]
pub enum HirStmt {
    Expr(HirExpr),
    Return(Option<HirExpr>),
    Let(Symbol, HirType, HirExpr),
    If(HirExpr, Vec<HirStmt>, Vec<HirStmt>),
    While(HirExpr, Vec<HirStmt>),
    Break,
    Continue,
    /// Break/continue an enclosing loop, where zero is the innermost loop.
    BreakDepth(usize),
    ContinueDepth(usize),
    Throw(HirExpr),
    /// `throw` unwinds through generated Thaw function calls to the nearest
    /// lexical `try`. Codegen implements this with a pending-exception slot,
    /// preserving the ordinary function and C FFI ABIs. `catch` binds the
    /// thrown value as a string. `finally` is expanded around normal and
    /// abrupt exits during lowering, so it does not require a separate HIR
    /// variant.
    Try(Vec<HirStmt>, Symbol, Vec<HirStmt>),
}

#[derive(Debug, Clone, PartialEq)]
pub struct HirFunction {
    pub name: Symbol,
    pub params: Vec<HirParam>,
    /// The function's *unwrapped* return type: for an `async function`
    /// declared as `Promise<T>`, this is `T`, not `Promise<T>` -- see the
    /// V1 async/await design (docs/design/async-await.md). `is_async`
    /// records that the unwrap happened, for tooling/future-V2 use; V1
    /// codegen doesn't need to branch on it (`await` is an identity
    /// transform, see hir_codegen).
    pub ret: HirType,
    pub is_async: bool,
    pub body: Vec<HirStmt>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct HirProgram {
    pub functions: Vec<HirFunction>,
    /// Ambient function declarations (`declare function foo(...): T;`, no
    /// body) -- see docs/design/bridge.md section 6. Calls to one of these
    /// names lower to `HirExpr::FfiCall` instead of `HirExpr::Call`;
    /// codegen declares each as an `extern "C"` symbol that the final link
    /// step must resolve from elsewhere (a real native library today; a
    /// thaw-registry-fetched one eventually).
    pub extern_functions: Vec<FfiSignature>,
}

/// Applies an explicitly configured error ABI to an ambient symbol and every
/// call site that carries its copied [`FfiSignature`]. Direct ABI remains the
/// default so existing native libraries are unaffected.
pub fn set_ffi_error_abi(
    program: &mut HirProgram,
    symbol: &str,
    error_abi: FfiErrorAbi,
) -> Result<(), String> {
    let mut found = false;
    for signature in &mut program.extern_functions {
        if signature.symbol == symbol {
            signature.error_abi = error_abi.clone();
            found = true;
        }
    }

    fn visit_expr(expr: &mut HirExpr, symbol: &str, abi: &FfiErrorAbi, found: &mut bool) {
        match expr {
            HirExpr::FfiCall(signature, args) => {
                if signature.symbol == symbol {
                    signature.error_abi = abi.clone();
                    *found = true;
                }
                for arg in args {
                    visit_expr(arg, symbol, abi, found);
                }
            }
            HirExpr::BinOp(_, left, right)
            | HirExpr::Index(left, right)
            | HirExpr::TypedIndex(left, right, _)
            | HirExpr::ArraySetLen(left, right, _) => {
                visit_expr(left, symbol, abi, found);
                visit_expr(right, symbol, abi, found);
            }
            HirExpr::Call(callee, args) => {
                visit_expr(callee, symbol, abi, found);
                for arg in args {
                    visit_expr(arg, symbol, abi, found);
                }
            }
            HirExpr::DynamicCall(_, args) => {
                for arg in args {
                    visit_expr(arg, symbol, abi, found);
                }
            }
            HirExpr::Await(inner)
            | HirExpr::AwaitPromise(inner, _)
            | HirExpr::PromiseAllArray(inner, _)
            | HirExpr::PromiseRaceArray(inner, _)
            | HirExpr::PromiseAnyArray(inner, _)
            | HirExpr::PromiseAllSettledArray(inner, _)
            | HirExpr::Assign(_, inner)
            | HirExpr::ArrayAlloc(inner, _)
            | HirExpr::ArrayLen(inner)
            | HirExpr::JsonAsNumber(inner)
            | HirExpr::JsonAsString(inner)
            | HirExpr::JsonAsBool(inner)
            | HirExpr::OptionalSome(inner, _)
            | HirExpr::OptionalIsNone(inner, _)
            | HirExpr::OptionalValue(inner, _)
            | HirExpr::NullableSome(inner, _)
            | HirExpr::NullableIsNone(inner, _)
            | HirExpr::NullableValue(inner, _)
            | HirExpr::NullishSome(inner, _)
            | HirExpr::NullishIsNull(inner, _)
            | HirExpr::NullishIsUndefined(inner, _)
            | HirExpr::NullishIsNone(inner, _)
            | HirExpr::NullishValue(inner, _) => visit_expr(inner, symbol, abi, found),
            HirExpr::Lambda(_, _, _, body) => visit_expr(body, symbol, abi, found),
            HirExpr::PromiseNew(executor, _, _) => visit_expr(executor, symbol, abi, found),
            HirExpr::PromiseThen(source, callback, _, _, _, _) => {
                visit_expr(source, symbol, abi, found);
                visit_expr(callback, symbol, abi, found);
            }
            HirExpr::PromiseFinally(source, callback, _, _) => {
                visit_expr(source, symbol, abi, found);
                visit_expr(callback, symbol, abi, found);
            }
            HirExpr::Block(stmts) => visit_stmts(stmts, symbol, abi, found),
            HirExpr::ArrayLit(values)
            | HirExpr::ArrayConcat(values, _)
            | HirExpr::PromiseAll(values, _)
            | HirExpr::PromiseAllTuple(values, _)
            | HirExpr::PromiseRace(values, _)
            | HirExpr::PromiseAny(values, _)
            | HirExpr::PromiseAllSettled(values, _) => {
                for value in values {
                    visit_expr(value, symbol, abi, found);
                }
            }
            HirExpr::IndexAssign(array, index, value) => {
                visit_expr(array, symbol, abi, found);
                visit_expr(index, symbol, abi, found);
                visit_expr(value, symbol, abi, found);
            }
            HirExpr::ObjectLit(fields) => {
                for (_, value) in fields {
                    visit_expr(value, symbol, abi, found);
                }
            }
            HirExpr::PropAccess(object, _, _) | HirExpr::JsonGet(object, _) => {
                visit_expr(object, symbol, abi, found);
            }
            HirExpr::PropAssign(object, _, _, value) | HirExpr::JsonIndex(object, value) => {
                visit_expr(object, symbol, abi, found);
                visit_expr(value, symbol, abi, found);
            }
            HirExpr::Lit(_)
            | HirExpr::OptionalNone(_)
            | HirExpr::NullableNone(_)
            | HirExpr::NullishNull(_)
            | HirExpr::NullishUndefined(_)
            | HirExpr::Var(_)
            | HirExpr::EnvVar(_)
            | HirExpr::FunctionRef(..) => {}
        }
    }

    fn visit_stmts(stmts: &mut [HirStmt], symbol: &str, abi: &FfiErrorAbi, found: &mut bool) {
        for stmt in stmts {
            match stmt {
                HirStmt::Expr(expr) | HirStmt::Throw(expr) | HirStmt::Let(_, _, expr) => {
                    visit_expr(expr, symbol, abi, found)
                }
                HirStmt::Return(Some(expr)) => visit_expr(expr, symbol, abi, found),
                HirStmt::If(condition, then_body, else_body) => {
                    visit_expr(condition, symbol, abi, found);
                    visit_stmts(then_body, symbol, abi, found);
                    visit_stmts(else_body, symbol, abi, found);
                }
                HirStmt::While(condition, body) => {
                    visit_expr(condition, symbol, abi, found);
                    visit_stmts(body, symbol, abi, found);
                }
                HirStmt::Try(body, _, catch_body) => {
                    visit_stmts(body, symbol, abi, found);
                    visit_stmts(catch_body, symbol, abi, found);
                }
                HirStmt::Return(None)
                | HirStmt::Break
                | HirStmt::Continue
                | HirStmt::BreakDepth(_)
                | HirStmt::ContinueDepth(_) => {}
            }
        }
    }

    for function in &mut program.functions {
        visit_stmts(&mut function.body, symbol, &error_abi, &mut found);
    }
    if found {
        Ok(())
    } else {
        Err(format!(
            "FFI metadata references unknown ambient function `{symbol}`"
        ))
    }
}

pub fn set_ffi_ownership(
    program: &mut HirProgram,
    symbol: &str,
    return_ownership: FfiOwnership,
    error_ownership: FfiOwnership,
) -> Result<(), String> {
    fn update_expr(
        expr: &mut HirExpr,
        symbol: &str,
        returns: &FfiOwnership,
        errors: &FfiOwnership,
        found: &mut bool,
    ) {
        if let HirExpr::FfiCall(signature, _) = expr {
            if signature.symbol == symbol {
                signature.return_ownership = returns.clone();
                signature.error_ownership = errors.clone();
                *found = true;
            }
        }
        match expr {
            HirExpr::FfiCall(_, args)
            | HirExpr::ArrayLit(args)
            | HirExpr::ArrayConcat(args, _)
            | HirExpr::PromiseAll(args, _)
            | HirExpr::PromiseAllTuple(args, _)
            | HirExpr::PromiseRace(args, _)
            | HirExpr::PromiseAny(args, _)
            | HirExpr::PromiseAllSettled(args, _) => {
                for arg in args {
                    update_expr(arg, symbol, returns, errors, found);
                }
            }
            HirExpr::Call(callee, args) => {
                update_expr(callee, symbol, returns, errors, found);
                for arg in args {
                    update_expr(arg, symbol, returns, errors, found);
                }
            }
            HirExpr::DynamicCall(_, args) => {
                for arg in args {
                    update_expr(arg, symbol, returns, errors, found);
                }
            }
            HirExpr::BinOp(_, left, right)
            | HirExpr::Index(left, right)
            | HirExpr::TypedIndex(left, right, _)
            | HirExpr::ArraySetLen(left, right, _) => {
                update_expr(left, symbol, returns, errors, found);
                update_expr(right, symbol, returns, errors, found);
            }
            HirExpr::Await(inner)
            | HirExpr::AwaitPromise(inner, _)
            | HirExpr::PromiseAllArray(inner, _)
            | HirExpr::PromiseRaceArray(inner, _)
            | HirExpr::PromiseAnyArray(inner, _)
            | HirExpr::PromiseAllSettledArray(inner, _)
            | HirExpr::Assign(_, inner)
            | HirExpr::ArrayAlloc(inner, _)
            | HirExpr::ArrayLen(inner)
            | HirExpr::JsonAsNumber(inner)
            | HirExpr::JsonAsString(inner)
            | HirExpr::JsonAsBool(inner)
            | HirExpr::OptionalSome(inner, _)
            | HirExpr::OptionalIsNone(inner, _)
            | HirExpr::OptionalValue(inner, _)
            | HirExpr::NullableSome(inner, _)
            | HirExpr::NullableIsNone(inner, _)
            | HirExpr::NullableValue(inner, _)
            | HirExpr::NullishSome(inner, _)
            | HirExpr::NullishIsNull(inner, _)
            | HirExpr::NullishIsUndefined(inner, _)
            | HirExpr::NullishIsNone(inner, _)
            | HirExpr::NullishValue(inner, _) => update_expr(inner, symbol, returns, errors, found),
            HirExpr::Lambda(_, _, _, body) => update_expr(body, symbol, returns, errors, found),
            HirExpr::PromiseNew(executor, _, _) => {
                update_expr(executor, symbol, returns, errors, found)
            }
            HirExpr::PromiseThen(source, callback, _, _, _, _) => {
                update_expr(source, symbol, returns, errors, found);
                update_expr(callback, symbol, returns, errors, found);
            }
            HirExpr::PromiseFinally(source, callback, _, _) => {
                update_expr(source, symbol, returns, errors, found);
                update_expr(callback, symbol, returns, errors, found);
            }
            HirExpr::Block(stmts) => update_stmts(stmts, symbol, returns, errors, found),
            HirExpr::IndexAssign(a, b, c) => {
                update_expr(a, symbol, returns, errors, found);
                update_expr(b, symbol, returns, errors, found);
                update_expr(c, symbol, returns, errors, found);
            }
            HirExpr::ObjectLit(fields) => {
                for (_, value) in fields {
                    update_expr(value, symbol, returns, errors, found);
                }
            }
            HirExpr::PropAccess(object, _, _) | HirExpr::JsonGet(object, _) => {
                update_expr(object, symbol, returns, errors, found)
            }
            HirExpr::PropAssign(object, _, _, value) | HirExpr::JsonIndex(object, value) => {
                update_expr(object, symbol, returns, errors, found);
                update_expr(value, symbol, returns, errors, found);
            }
            HirExpr::Lit(_)
            | HirExpr::OptionalNone(_)
            | HirExpr::NullableNone(_)
            | HirExpr::NullishNull(_)
            | HirExpr::NullishUndefined(_)
            | HirExpr::Var(_)
            | HirExpr::EnvVar(_)
            | HirExpr::FunctionRef(..) => {}
        }
    }
    fn update_stmts(
        stmts: &mut [HirStmt],
        symbol: &str,
        returns: &FfiOwnership,
        errors: &FfiOwnership,
        found: &mut bool,
    ) {
        for stmt in stmts {
            match stmt {
                HirStmt::Expr(expr) | HirStmt::Throw(expr) | HirStmt::Let(_, _, expr) => {
                    update_expr(expr, symbol, returns, errors, found)
                }
                HirStmt::Return(Some(expr)) => update_expr(expr, symbol, returns, errors, found),
                HirStmt::If(condition, then_body, else_body) => {
                    update_expr(condition, symbol, returns, errors, found);
                    update_stmts(then_body, symbol, returns, errors, found);
                    update_stmts(else_body, symbol, returns, errors, found);
                }
                HirStmt::While(condition, body) => {
                    update_expr(condition, symbol, returns, errors, found);
                    update_stmts(body, symbol, returns, errors, found);
                }
                HirStmt::Try(body, _, catch_body) => {
                    update_stmts(body, symbol, returns, errors, found);
                    update_stmts(catch_body, symbol, returns, errors, found);
                }
                HirStmt::Return(None)
                | HirStmt::Break
                | HirStmt::Continue
                | HirStmt::BreakDepth(_)
                | HirStmt::ContinueDepth(_) => {}
            }
        }
    }
    let mut found = false;
    for signature in &mut program.extern_functions {
        if signature.symbol == symbol {
            signature.return_ownership = return_ownership.clone();
            signature.error_ownership = error_ownership.clone();
            found = true;
        }
    }
    for function in &mut program.functions {
        update_stmts(
            &mut function.body,
            symbol,
            &return_ownership,
            &error_ownership,
            &mut found,
        );
    }
    if found {
        Ok(())
    } else {
        Err(format!(
            "FFI ownership metadata references unknown ambient function `{symbol}`"
        ))
    }
}

pub fn set_ffi_string_abi(
    program: &mut HirProgram,
    symbol: &str,
    param_abis: Vec<FfiStringAbi>,
    return_abi: FfiStringAbi,
    calling_convention: FfiCallingConvention,
    aggregate_return_abi: FfiAggregateAbi,
) -> Result<(), String> {
    fn update_expr(
        expr: &mut HirExpr,
        symbol: &str,
        params: &[FfiStringAbi],
        returns: FfiStringAbi,
        calling_convention: FfiCallingConvention,
        aggregate_return_abi: FfiAggregateAbi,
        found: &mut bool,
    ) {
        if let HirExpr::FfiCall(signature, _) = expr {
            if signature.symbol == symbol {
                signature.param_string_abis = params.to_vec();
                signature.return_string_abi = returns;
                signature.calling_convention = calling_convention;
                signature.aggregate_return_abi = aggregate_return_abi;
                *found = true;
            }
        }
        match expr {
            HirExpr::FfiCall(_, values)
            | HirExpr::ArrayLit(values)
            | HirExpr::ArrayConcat(values, _)
            | HirExpr::PromiseAll(values, _)
            | HirExpr::PromiseAllTuple(values, _)
            | HirExpr::PromiseRace(values, _)
            | HirExpr::PromiseAny(values, _)
            | HirExpr::PromiseAllSettled(values, _) => {
                for value in values {
                    update_expr(
                        value,
                        symbol,
                        params,
                        returns,
                        calling_convention,
                        aggregate_return_abi,
                        found,
                    );
                }
            }
            HirExpr::Call(callee, values) => {
                update_expr(
                    callee,
                    symbol,
                    params,
                    returns,
                    calling_convention,
                    aggregate_return_abi,
                    found,
                );
                for value in values {
                    update_expr(
                        value,
                        symbol,
                        params,
                        returns,
                        calling_convention,
                        aggregate_return_abi,
                        found,
                    );
                }
            }
            HirExpr::DynamicCall(_, values) => {
                for value in values {
                    update_expr(
                        value,
                        symbol,
                        params,
                        returns,
                        calling_convention,
                        aggregate_return_abi,
                        found,
                    );
                }
            }
            HirExpr::BinOp(_, left, right)
            | HirExpr::Index(left, right)
            | HirExpr::TypedIndex(left, right, _)
            | HirExpr::ArraySetLen(left, right, _) => {
                update_expr(
                    left,
                    symbol,
                    params,
                    returns,
                    calling_convention,
                    aggregate_return_abi,
                    found,
                );
                update_expr(
                    right,
                    symbol,
                    params,
                    returns,
                    calling_convention,
                    aggregate_return_abi,
                    found,
                );
            }
            HirExpr::Await(inner)
            | HirExpr::AwaitPromise(inner, _)
            | HirExpr::PromiseAllArray(inner, _)
            | HirExpr::PromiseRaceArray(inner, _)
            | HirExpr::PromiseAnyArray(inner, _)
            | HirExpr::PromiseAllSettledArray(inner, _)
            | HirExpr::Assign(_, inner)
            | HirExpr::ArrayAlloc(inner, _)
            | HirExpr::ArrayLen(inner)
            | HirExpr::JsonAsNumber(inner)
            | HirExpr::JsonAsString(inner)
            | HirExpr::JsonAsBool(inner)
            | HirExpr::OptionalSome(inner, _)
            | HirExpr::OptionalIsNone(inner, _)
            | HirExpr::OptionalValue(inner, _)
            | HirExpr::NullableSome(inner, _)
            | HirExpr::NullableIsNone(inner, _)
            | HirExpr::NullableValue(inner, _)
            | HirExpr::NullishSome(inner, _)
            | HirExpr::NullishIsNull(inner, _)
            | HirExpr::NullishIsUndefined(inner, _)
            | HirExpr::NullishIsNone(inner, _)
            | HirExpr::NullishValue(inner, _) => update_expr(
                inner,
                symbol,
                params,
                returns,
                calling_convention,
                aggregate_return_abi,
                found,
            ),
            HirExpr::Lambda(_, _, _, body) => update_expr(
                body,
                symbol,
                params,
                returns,
                calling_convention,
                aggregate_return_abi,
                found,
            ),
            HirExpr::PromiseNew(executor, _, _) => update_expr(
                executor,
                symbol,
                params,
                returns,
                calling_convention,
                aggregate_return_abi,
                found,
            ),
            HirExpr::PromiseThen(source, callback, _, _, _, _) => {
                update_expr(
                    source,
                    symbol,
                    params,
                    returns,
                    calling_convention,
                    aggregate_return_abi,
                    found,
                );
                update_expr(
                    callback,
                    symbol,
                    params,
                    returns,
                    calling_convention,
                    aggregate_return_abi,
                    found,
                );
            }
            HirExpr::PromiseFinally(source, callback, _, _) => {
                update_expr(
                    source,
                    symbol,
                    params,
                    returns,
                    calling_convention,
                    aggregate_return_abi,
                    found,
                );
                update_expr(
                    callback,
                    symbol,
                    params,
                    returns,
                    calling_convention,
                    aggregate_return_abi,
                    found,
                );
            }
            HirExpr::Block(stmts) => update_stmts(
                stmts,
                symbol,
                params,
                returns,
                calling_convention,
                aggregate_return_abi,
                found,
            ),
            HirExpr::IndexAssign(a, b, c) => {
                update_expr(
                    a,
                    symbol,
                    params,
                    returns,
                    calling_convention,
                    aggregate_return_abi,
                    found,
                );
                update_expr(
                    b,
                    symbol,
                    params,
                    returns,
                    calling_convention,
                    aggregate_return_abi,
                    found,
                );
                update_expr(
                    c,
                    symbol,
                    params,
                    returns,
                    calling_convention,
                    aggregate_return_abi,
                    found,
                );
            }
            HirExpr::ObjectLit(fields) => {
                for (_, value) in fields {
                    update_expr(
                        value,
                        symbol,
                        params,
                        returns,
                        calling_convention,
                        aggregate_return_abi,
                        found,
                    );
                }
            }
            HirExpr::PropAccess(object, _, _) | HirExpr::JsonGet(object, _) => update_expr(
                object,
                symbol,
                params,
                returns,
                calling_convention,
                aggregate_return_abi,
                found,
            ),
            HirExpr::PropAssign(object, _, _, value) | HirExpr::JsonIndex(object, value) => {
                update_expr(
                    object,
                    symbol,
                    params,
                    returns,
                    calling_convention,
                    aggregate_return_abi,
                    found,
                );
                update_expr(
                    value,
                    symbol,
                    params,
                    returns,
                    calling_convention,
                    aggregate_return_abi,
                    found,
                );
            }
            HirExpr::Lit(_)
            | HirExpr::OptionalNone(_)
            | HirExpr::NullableNone(_)
            | HirExpr::NullishNull(_)
            | HirExpr::NullishUndefined(_)
            | HirExpr::Var(_)
            | HirExpr::EnvVar(_)
            | HirExpr::FunctionRef(..) => {}
        }
    }
    fn update_stmts(
        stmts: &mut [HirStmt],
        symbol: &str,
        params: &[FfiStringAbi],
        returns: FfiStringAbi,
        calling_convention: FfiCallingConvention,
        aggregate_return_abi: FfiAggregateAbi,
        found: &mut bool,
    ) {
        for stmt in stmts {
            match stmt {
                HirStmt::Expr(expr) | HirStmt::Throw(expr) | HirStmt::Let(_, _, expr) => {
                    update_expr(
                        expr,
                        symbol,
                        params,
                        returns,
                        calling_convention,
                        aggregate_return_abi,
                        found,
                    )
                }
                HirStmt::Return(Some(expr)) => update_expr(
                    expr,
                    symbol,
                    params,
                    returns,
                    calling_convention,
                    aggregate_return_abi,
                    found,
                ),
                HirStmt::If(condition, then_body, else_body) => {
                    update_expr(
                        condition,
                        symbol,
                        params,
                        returns,
                        calling_convention,
                        aggregate_return_abi,
                        found,
                    );
                    update_stmts(
                        then_body,
                        symbol,
                        params,
                        returns,
                        calling_convention,
                        aggregate_return_abi,
                        found,
                    );
                    update_stmts(
                        else_body,
                        symbol,
                        params,
                        returns,
                        calling_convention,
                        aggregate_return_abi,
                        found,
                    );
                }
                HirStmt::While(condition, body) => {
                    update_expr(
                        condition,
                        symbol,
                        params,
                        returns,
                        calling_convention,
                        aggregate_return_abi,
                        found,
                    );
                    update_stmts(
                        body,
                        symbol,
                        params,
                        returns,
                        calling_convention,
                        aggregate_return_abi,
                        found,
                    );
                }
                HirStmt::Try(body, _, catch_body) => {
                    update_stmts(
                        body,
                        symbol,
                        params,
                        returns,
                        calling_convention,
                        aggregate_return_abi,
                        found,
                    );
                    update_stmts(
                        catch_body,
                        symbol,
                        params,
                        returns,
                        calling_convention,
                        aggregate_return_abi,
                        found,
                    );
                }
                HirStmt::Return(None)
                | HirStmt::Break
                | HirStmt::Continue
                | HirStmt::BreakDepth(_)
                | HirStmt::ContinueDepth(_) => {}
            }
        }
    }

    let mut found = false;
    for signature in &mut program.extern_functions {
        if signature.symbol == symbol {
            if param_abis.len() != signature.params.len() {
                return Err(format!(
                    "FFI string ABI metadata for `{symbol}` has {} parameter layouts, expected {}",
                    param_abis.len(),
                    signature.params.len()
                ));
            }
            for (index, (ty, abi)) in signature.params.iter().zip(&param_abis).enumerate() {
                if *abi == FfiStringAbi::PointerLength && *ty != HirType::Str {
                    return Err(format!(
                        "FFI parameter {index} of `{symbol}` uses string pointer-length ABI but is not a string"
                    ));
                }
            }
            if return_abi == FfiStringAbi::PointerLength && signature.ret != HirType::Str {
                return Err(format!(
                    "FFI return of `{symbol}` uses string pointer-length ABI but is not a string"
                ));
            }
            signature.param_string_abis = param_abis.clone();
            signature.return_string_abi = return_abi;
            signature.calling_convention = calling_convention;
            signature.aggregate_return_abi = aggregate_return_abi;
            found = true;
        }
    }
    for function in &mut program.functions {
        update_stmts(
            &mut function.body,
            symbol,
            &param_abis,
            return_abi,
            calling_convention,
            aggregate_return_abi,
            &mut found,
        );
    }
    if found {
        Ok(())
    } else {
        Err(format!(
            "FFI string ABI metadata references unknown ambient function `{symbol}`"
        ))
    }
}

pub fn set_ffi_aggregate_layout(
    program: &mut HirProgram,
    symbol: &str,
    layout: FfiAggregateLayout,
) -> Result<(), String> {
    let Some(signature) = program
        .extern_functions
        .iter_mut()
        .find(|signature| signature.symbol == symbol)
    else {
        return Err(format!(
            "FFI aggregate layout metadata references unknown ambient function `{symbol}`"
        ));
    };
    let HirType::Object(fields) = &signature.ret else {
        return Err(format!(
            "FFI aggregate layout for `{symbol}` requires a fixed object return"
        ));
    };
    if signature.aggregate_return_abi == FfiAggregateAbi::Internal {
        return Err(format!(
            "FFI aggregate layout for `{symbol}` requires a portable or packed aggregate return ABI"
        ));
    }
    fn validate_layout(
        symbol: &str,
        path: &str,
        fields: &[(Symbol, HirType)],
        layout: &FfiAggregateLayout,
        root: bool,
    ) -> Result<(), String> {
        if layout.field_offsets.len() != fields.len() || layout.field_layouts.len() != fields.len()
        {
            return Err(format!(
                "FFI aggregate layout for `{symbol}` at `{path}` has {} field offsets and {} field layouts, expected {} of each",
                layout.field_offsets.len(),
                layout.field_layouts.len(),
                fields.len()
            ));
        }
        if layout.alignment == 0 || !layout.alignment.is_power_of_two() {
            return Err(format!(
                "FFI aggregate layout for `{symbol}` at `{path}` needs a non-zero power-of-two alignment"
            ));
        }
        if layout.size == 0 || !layout.size.is_multiple_of(u64::from(layout.alignment)) {
            return Err(format!(
                "FFI aggregate layout for `{symbol}` at `{path}` needs a non-zero size divisible by its alignment"
            ));
        }
        if root != layout.indirect {
            return Err(format!(
                "FFI aggregate layout for `{symbol}` at `{path}` requires `indirect: {root}`"
            ));
        }
        let mut previous_end = 0;
        for (index, ((name, ty), offset)) in fields.iter().zip(&layout.field_offsets).enumerate() {
            let child = layout.field_layouts[index].as_deref();
            let field_size = match (ty, child) {
                (HirType::Bool, None) => 1,
                (HirType::F64 | HirType::I64 | HirType::Str, None) => 8,
                (HirType::Object(child_fields), Some(child_layout)) => {
                    validate_layout(
                        symbol,
                        &format!("{path}.{name}"),
                        child_fields,
                        child_layout,
                        false,
                    )?;
                    child_layout.size
                }
                (HirType::Object(_), None) => {
                    return Err(format!(
                        "FFI aggregate layout field `{path}.{name}` of `{symbol}` needs a nested field layout"
                    ))
                }
                (_, Some(_)) => {
                    return Err(format!(
                        "FFI aggregate layout field `{path}.{name}` of `{symbol}` has a nested layout for a non-object type"
                    ))
                }
                (other, None) => {
                    return Err(format!(
                        "FFI aggregate layout field `{path}.{name}` of `{symbol}` has unsupported explicit-layout type {other:?}"
                    ))
                }
            };
            if *offset < previous_end || offset.saturating_add(field_size) > layout.size {
                return Err(format!(
                    "FFI aggregate layout field `{path}.{name}` of `{symbol}` is outside or overlaps the declared size"
                ));
            }
            previous_end = offset + field_size;
        }
        Ok(())
    }

    if layout.field_offsets.len() != fields.len() {
        return Err(format!(
            "FFI aggregate layout for `{symbol}` has {} field offsets, expected {}",
            layout.field_offsets.len(),
            fields.len()
        ));
    }
    validate_layout(symbol, "return", fields, &layout, true)?;
    signature.aggregate_return_layout = Some(layout);
    Ok(())
}
