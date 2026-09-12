use super::*;

#[derive(Debug, Clone, PartialEq)]
pub enum HirExpr {
    Lit(HirLit),
    Var(Symbol),
    BinOp(BinOp, Box<HirExpr>, Box<HirExpr>),
    Conditional(Box<HirExpr>, Box<HirExpr>, Box<HirExpr>, HirType),
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
    /// Injects one native value into a heterogeneous tagged union. The
    /// zero-based member index is the runtime tag.
    UnionInject(Box<HirExpr>, usize, Vec<HirType>),
    /// Reads the runtime member tag of a heterogeneous union.
    UnionTag(Box<HirExpr>, Vec<HirType>),
    /// Extracts a member after lowering has established the matching tag.
    UnionValue(Box<HirExpr>, usize, Vec<HirType>),
    /// Strict equality between a tagged union and one concrete member value.
    UnionMemberIsEqual(Box<HirExpr>, Box<HirExpr>, usize, Vec<HirType>),
    /// Strict equality between two values with the same tagged union type.
    UnionIsEqual(Box<HirExpr>, Box<HirExpr>, Vec<HirType>),
    Call(Box<HirExpr>, Vec<HirExpr>),
    // Invokes a closure through its second entry with an explicit JavaScript `thisArg`.
    FunctionCallWithThis(
        Box<HirExpr>,
        Box<HirExpr>,
        Vec<HirExpr>,
        Vec<HirType>,
        HirType,
    ),
    // Binds an explicit receiver and zero or more leading arguments to a closure.
    FunctionBindThis(
        Box<HirExpr>,
        Box<HirExpr>,
        Vec<HirExpr>,
        Vec<HirType>,
        HirType,
    ),
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
    // A closure initializer whose named function expression captures its own preallocated cell.
    RecursiveClosure(Symbol, HirType, Box<HirExpr>),
    // Preserves a logical callable shape around a physically ordinary closure.
    TypedClosure(HirType, Box<HirExpr>),
    /// A top-level function adapted to the closure ABI when used as a value.
    FunctionRef(String, Vec<HirType>, HirType),
    /// A native method value with separate unbound and explicit-receiver entries.
    MethodRef(String, String, Vec<HirType>, HirType, bool),
    Block(Vec<HirStmt>),
    /// Raises `error` from an expression position. `fallback` supplies the
    /// unreachable native value required by the surrounding typed expression.
    ThrowValue(Box<HirExpr>, Box<HirExpr>),
    FfiCall(Box<FfiSignature>, Vec<HirExpr>),
    DynamicCall(DynamicSignature, Vec<HirExpr>),
    /// `name = value` (and desugared compound assignments / `++`/`--`).
    /// Evaluates to `value`.
    Assign(Symbol, Box<HirExpr>),
    PostfixUpdate(Symbol, BinOp),
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
    /// A runtime-keyed homogeneous object backed by thaw-std's JSON value.
    JsonObjectLit(Vec<(Symbol, HirExpr)>, HirType),
    /// Wraps a live `JsValue` (an opaque, permanent id into QuickJS's own
    /// retained-value table) as a genuine `Json` value -- there is no
    /// direct `Json` representation of "a live JS object", so this
    /// encodes it as the same `{"__thaw_js_handle_id__": <id>}`
    /// placeholder object `compile_dynamic_value_placeholder` already
    /// builds for a `JsValue` crossing into a dynamic call's own JSON
    /// argument array; the QuickJS-side JSON reviver (`dates.js`)
    /// splices the real live value back in the moment this placeholder
    /// gets JSON-parsed there, the same way it already does for a
    /// `Date`. Unlike that call-argument-only path (gated on
    /// `compiling_quickjs_dynamic_arguments`, since nothing reads the
    /// placeholder back on the far end anywhere else), this node exists
    /// specifically for `coerce_to_declared`'s `Json`-declared branch:
    /// a `JsValue`-actual value reaching a plain `Json`-typed slot
    /// (a local variable, an array element, an object field, ...) needs
    /// a real, valid `Json`-typed *value* -- `HirType::Json` and
    /// `HirType::JsValue` have different native layouts (an opaque
    /// pointer to a boxed `serde_json::Value` vs. a plain `i64` handle),
    /// so simply passing the raw handle through unchanged (as the call-
    /// argument path's own placeholder builder can, since the LLVM-level
    /// encoding happens right there before anything treats the operand
    /// as a real `Json` pointer) produced a real value whose own
    /// inferred type stayed `JsValue` while the slot's declared type
    /// said `Json` -- undetected by a `let`/`const` declaration (nothing
    /// re-validates a coerced initializer's own type against the
    /// variable's declared one), and a segfault the moment anything
    /// later dereferenced the raw handle bits as if they were a real
    /// `Json` pointer. This node's own `infer_expr_type` is `Json`
    /// (matching what it actually, genuinely produces), so both the
    /// re-validation gap and the segfault are closed at once.
    JsValueAsJson(Box<HirExpr>),
    /// Allocates a fixed-shape object and initializes every field to its
    /// native zero value before the reference escapes. Constructors use this
    /// to establish instance identity before executing `this.field = ...`.
    ObjectAlloc(HirType),
    /// `object.field`. Unlike `ArrayLen`/`EnvVar`, this *is* a general
    /// member-access node -- but it still isn't resolved by a real type
    /// checker at codegen time, so lowering bakes in the object's full
    /// `HirType::Object(fields)` shape (field order = the field's byte
    /// offset in the arena-allocated buffer) so codegen doesn't need to
    /// re-derive it.
    PropAccess(Box<HirExpr>, HirType, Symbol),
    DynamicPropAccess(Box<HirExpr>, Box<HirExpr>, Vec<(Symbol, HirType)>, HirType),
    /// Runtime numeric-enum reverse lookup. Known values produce a present
    /// string and unknown values produce `undefined`.
    EnumReverseLookup(Box<HirExpr>, Vec<(f64, Symbol)>),
    /// `object.field = value` (and desugared compound forms). Evaluates to
    /// `value`. Same `HirType` bookkeeping as `PropAccess`.
    PropAssign(Box<HirExpr>, HirType, Symbol, Box<HirExpr>),
    /// `json.field` where `json: HirType::Json` (a *dynamic* value, e.g.
    /// from `JSON.parse`) -- unlike `PropAccess`, there's no static field
    /// list to validate against or bake in; any name is accepted and
    /// resolved at runtime by thaw-std's `thaw_json_get`, returning
    /// another `Json` value (never a concrete type).
    JsonGet(Box<HirExpr>, Symbol),
    /// A JSON object lookup with a runtime string key.
    JsonKey(Box<HirExpr>, Box<HirExpr>),
    /// Mutates a homogeneous runtime-keyed object and evaluates to the
    /// assigned value. The trailing `bool` is `preserve_undefined`,
    /// mirroring `compile_json_object_set_native_with_undefined`'s own
    /// flag: when the value's declared type is `Optional`/`Nullable`/
    /// `Nullish` and it's actually absent/undefined at runtime, `false`
    /// (the ordinary case -- a real object-literal field legitimately
    /// omits itself from the resulting JSON, matching `JSON.stringify`'s
    /// own behavior for an `undefined`-valued property) omits the key
    /// entirely, while `true` writes the same `{"$__thaw_napi_undefined$":
    /// true}` sentinel a bare `Undefined` argument uses (see
    /// `coerce_to_declared`) instead of losing the distinction -- needed
    /// by `wrap_native_value_as_json`, which uses this node to encode a
    /// *standalone* value (not a real object literal's own field, where
    /// omission would be correct) as its own temporary object's one
    /// field, then reads that same field straight back out. Historically
    /// this was the *only* way to preserve a real `undefined`'s identity
    /// through a `Json` round-trip -- `thaw_json_get` (thaw-std) used to
    /// report a plain missing key as bare JSON `null` too, so omitting
    /// the key here would have silently turned a real `undefined`
    /// argument into `null`, wrong for something like zod's own
    /// `ZodUndefined`, which rejects `null`. `thaw_json_get` now
    /// synthesizes the same sentinel for a genuinely missing key on its
    /// own, so the two `preserve_undefined` branches converge for this
    /// particular round-trip either way -- this flag (and the plumbing
    /// behind it) stays, since it's still what makes the *native*-value
    /// encoding step choose to write the sentinel in the first place,
    /// rather than omitting the field, before `thaw_json_get` is ever
    /// involved at all.
    JsonSet(Box<HirExpr>, Box<HirExpr>, Box<HirExpr>, HirType, bool),
    JsonIndexSet(Box<HirExpr>, Box<HirExpr>, Box<HirExpr>),
    /// Deletes a runtime-keyed JSON/dictionary property and returns `true`.
    JsonDelete(Box<HirExpr>, Box<HirExpr>),
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
    /// Restores a structured native value from its JSON-backed dictionary
    /// representation. Lowering only emits target layouts supported by LLVM.
    JsonAsNative(Box<HirExpr>, HirType),
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

/// A module-scoped binding. Its initializer is evaluated by the generated
/// module initializer in source order before either `main` or `handler` runs.
#[derive(Debug, Clone, PartialEq)]
pub struct HirGlobal {
    pub name: Symbol,
    pub ty: HirType,
    pub init: HirExpr,
    pub mutable: bool,
}

/// One source-ordered action performed by the guarded module initializer.
#[derive(Debug, Clone, PartialEq)]
pub enum HirInitStep {
    StoreGlobal(Symbol, HirExpr),
    Statement(HirStmt),
}
