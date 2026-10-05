//! The real Thaw pipeline: `thaw_hir::HirProgram` -> LLVM IR -> a native
//! object file. This is a separate, unrelated compiler from the
//! `lexer`/`ast`/`parser`/`codegen` modules at the crate root, which are the
//! Kaleidoscope tutorial scaffold used to learn inkwell in the first place.
//!
//! Generates native functions, closures, collections, tagged values, exception
//! propagation, and resumable async frames from typed HIR. Runtime calls provide
//! strings, promises, platform APIs, and dynamic QuickJS/N-API interoperation.
//! Entrypoints support both `main` programs and Lambda `handler` programs.
//! Generated globals are registered as arena roots before Lambda initialization.

use std::collections::{HashMap, HashSet};
use std::num::NonZeroU32;
use std::path::Path;

use inkwell::basic_block::BasicBlock;
use inkwell::builder::Builder;
use inkwell::context::Context;
use inkwell::module::{Linkage, Module};
use inkwell::passes::PassBuilderOptions;
use inkwell::targets::{
    CodeModel, FileType, InitializationConfig, RelocMode, Target, TargetMachine,
};
use inkwell::types::{BasicMetadataTypeEnum, BasicType, BasicTypeEnum, FunctionType};
use inkwell::values::{
    BasicMetadataValueEnum, BasicValue, BasicValueEnum, FloatValue, FunctionValue, IntValue,
    PointerValue, StructValue,
};
use inkwell::{AddressSpace, FloatPredicate, IntPredicate, OptimizationLevel};

use thaw_hir::{
    BinOp, DynamicBackend, DynamicSignature, FfiAggregateAbi, FfiAggregateLayout,
    FfiBitFieldLayout, FfiCallingConvention, FfiErrorAbi, FfiOwnership, FfiRegisterClass,
    FfiSignature, FfiStringAbi, FfiVariadicAbi, HirExpr, HirFunction, HirInitStep, HirLit,
    HirParam, HirProgram, HirStmt, HirType,
};

#[derive(Clone)]
struct ExplicitFfiField {
    native_index: u32,
    bitfield: Option<FfiBitFieldLayout>,
}

/// The user's `main`, if any, is compiled under this symbol instead of
/// `main` so we can wrap it in a proper `i32 main(void)` C entry point
/// rather than exposing a `void`-returning function as the process entry.
const USER_MAIN_SYMBOL: &str = "thaw_user_main";
/// Magic function name a registry-generated shim can define (see
/// thaw-bridge's `generate_module_init` and thaw-cli's `--use`) to run
/// code once before user code -- currently just `loadScript` calls for
/// each package's bundled JS, so Fallback-path functions are callable
/// without the user writing `loadScript` themselves. An ordinary
/// `HirFunction` like any other; the only special treatment is that
/// `compile_program` calls it first, if present, from both possible
/// entry points (`main` and `handler`).
const MODULE_INIT_SYMBOL: &str = "__thaw_module_init";
const NATIVE_MODULE_INIT_SYMBOL: &str = "__thaw_native_module_init";
const TOP_LEVEL_INIT_SYMBOL: &str = "__thaw_top_level_init";
const TOP_LEVEL_INIT_GUARD_SYMBOL: &str = "__thaw_top_level_initialized";
/// A null pointer means normal execution; a non-null pointer is the string
/// value currently unwinding through generated Thaw calls. Keeping this in
/// generated-module state preserves the existing function ABI (important for
/// C FFI) while allowing callers to branch to their nearest lexical catch.
const PENDING_EXCEPTION_SYMBOL: &str = "__thaw_pending_exception";
const PENDING_REJECTION_SYMBOL: &str = "__thaw_pending_rejection";
/// Parallel, opt-in companion to `PENDING_EXCEPTION_SYMBOL`: null unless the
/// thrown value was a real (Error-family) class instance, in which case it
/// holds that object's own pointer so a catch site can read fields beyond
/// `.message`/`.name` after an explicit `as` cast (see
/// `docs/design/exceptions.md` section 3). Every other reader of the
/// exception state (console.log, N-API, Lambda error reporting,
/// `Promise.reject`) only ever looks at the string channel and stays
/// completely unaware this one exists.
const PENDING_EXCEPTION_OBJECT_SYMBOL: &str = "__thaw_pending_exception_object";
const PENDING_EXCEPTION_AGGREGATE_SYMBOL: &str = "__thaw_pending_exception_aggregate_errors";
const PENDING_EXCEPTION_NATIVE_TEXT_SYMBOL: &str = "__thaw_pending_exception_native_text";
// Null for borrowed/static NativeStr. An owned FFI error stores the exact
// allocation here until a consumer has taken or explicitly discarded it.
const PENDING_EXCEPTION_NATIVE_TEXT_OWNER_SYMBOL: &str = "__thaw_pending_exception_native_text_owner";
const PENDING_EXCEPTION_VALUE_TAG_SYMBOL: &str = "__thaw_pending_exception_value_tag";
const PENDING_EXCEPTION_F64_SYMBOL: &str = "__thaw_pending_exception_f64";
const PENDING_EXCEPTION_I64_SYMBOL: &str = "__thaw_pending_exception_i64";
const PENDING_EXCEPTION_BOOL_SYMBOL: &str = "__thaw_pending_exception_bool";
// Null for borrowed Json. A fresh captured exception stores its exact Box
// here until a consumer has acquired an independent owner.
const PENDING_EXCEPTION_JSON_OWNER_SYMBOL: &str = "__thaw_pending_exception_json_owner";
const HTTP_SAVED_EXCEPTION_ROOT_SYMBOL: &str = "__thaw_http_saved_exception_root";

/// Byte size of an array's length header (a single `i64`) that precedes its
/// elements in the arena-allocated buffer. See the module doc for the layout.
const ARRAY_HEADER_BYTES: u64 = 8;
/// Phase 1 arrays only ever hold `f64` elements (see module doc).
const ARRAY_ELEM_BYTES: u64 = 8;
/// Minimum storage/alignment unit for native object fields. Tagged fields
/// occupy at least two units and wider nested values use checked strides.
const OBJECT_FIELD_BYTES: u64 = 8;
const ASYNC_FRAME_BYTES: u64 = 24;
const ASYNC_SLOT_BYTES: u64 = 16;

fn checked_storage_add(left: u64, right: u64) -> Result<u64, String> {
    left.checked_add(right).filter(|size| *size <= isize::MAX as u64)
        .ok_or_else(|| "arena layout size overflow".to_string())
}

fn checked_storage_mul(left: u64, right: u64) -> Result<u64, String> {
    left.checked_mul(right).filter(|size| *size <= isize::MAX as u64)
        .ok_or_else(|| "arena layout size overflow".to_string())
}

fn checked_array_offset(stride: u64, index: usize) -> Result<u64, String> {
    checked_storage_add(ARRAY_HEADER_BYTES, checked_storage_mul(stride, index as u64)?)
}

fn align_storage(size: u64, align: u64) -> Result<u64, String> {
    checked_storage_add(size, align - 1).map(|value| value & !(align - 1))
}

// Match the nonpacked 64-bit LLVM types in HirCompiler::basic_type. The
// recursive raw width is distinct from the legacy 8/16-byte arena stride.
fn raw_value_layout(ty: &HirType) -> Result<(u64, u64), String> {
    match ty {
        HirType::Bool | HirType::Undefined | HirType::Null => Ok((1, 1)),
        HirType::Void => Ok((4, 4)),
        HirType::F64 | HirType::I64 | HirType::Str | HirType::Symbol
        | HirType::StrLiteral(_) | HirType::Json | HirType::Dictionary(_)
        | HirType::JsValue | HirType::Promise(_) | HirType::Array(_)
        | HirType::Bytes | HirType::Map(_, _) | HirType::WeakMap(_, _)
        | HirType::Set(_) | HirType::WeakSet(_) | HirType::Tuple(_)
        | HirType::Object(_) | HirType::Function(_, _)
        | HirType::CallableFunction(..) => Ok((8, 8)),
        HirType::Union(members) if !members.is_empty() => Ok((16, 8)),
        HirType::Optional(payload) | HirType::Nullable(payload)
        | HirType::Nullish(payload) => {
            let (payload_size, payload_align) = raw_value_layout(payload)?;
            let start = align_storage(1, payload_align)?;
            let size = checked_storage_add(start, payload_size)?;
            Ok((align_storage(size, payload_align)?, payload_align))
        }
        _ => Err(format!("unsupported arena field type {ty:?}")),
    }
}

// Validate compiled literal values against the same nonpacked layout. HIR
// inference can be absent for synthetic expressions, so their actual LLVM
// struct width must still participate in the arena stride.
fn compiled_value_layout(ty: BasicTypeEnum<'_>) -> Result<(u64, u64), String> {
    match ty {
        BasicTypeEnum::IntType(integer) => {
            let bytes = ((integer.get_bit_width() as u64 + 7) / 8).max(1);
            Ok((bytes, bytes.min(8)))
        }
        BasicTypeEnum::FloatType(_) | BasicTypeEnum::PointerType(_) => Ok((8, 8)),
        BasicTypeEnum::StructType(structure) if !structure.is_packed() => {
            let mut offset = 0;
            let mut alignment = 1;
            for field in structure.get_field_types() {
                let (size, align) = compiled_value_layout(field)?;
                alignment = alignment.max(align);
                offset = checked_storage_add(align_storage(offset, align)?, size)?;
            }
            Ok((align_storage(offset, alignment)?, alignment))
        }
        _ => Err("unsupported compiled arena value type".into()),
    }
}

fn arena_storage_bytes(ty: &HirType) -> Result<u64, String> {
    let legacy = if matches!(ty, HirType::Optional(_) | HirType::Nullable(_)
        | HirType::Nullish(_) | HirType::Union(_)) { 16 } else { 8 };
    Ok(legacy.max(align_storage(raw_value_layout(ty)?.0, 8)?))
}

fn object_field_storage_bytes(ty: &HirType) -> Result<u64, String> {
    arena_storage_bytes(ty)
}

fn array_element_storage_bytes(ty: &HirType) -> Result<u64, String> {
    arena_storage_bytes(ty)
}

fn tuple_element_storage_bytes(elements: &[HirType]) -> Result<u64, String> {
    elements.iter().try_fold(ARRAY_ELEM_BYTES, |max_width, element| {
        Ok::<_, String>(max_width.max(array_element_storage_bytes(element)?))
    })
}

fn object_field_offset(fields: &[(String, HirType)], index: usize) -> Result<u64, String> {
    fields[..index].iter().try_fold(0, |offset, (_, ty)| {
        checked_storage_add(offset, object_field_storage_bytes(ty)?)
    })
}

fn is_hidden_accessor_field(name: &str) -> bool {
    name.starts_with("__thaw_getter_") || name.starts_with("__thaw_setter_")
}

fn ecmascript_object_field_order(fields: &[(String, HirType)]) -> Vec<usize> {
    let mut indices = fields
        .iter()
        .enumerate()
        .filter_map(|(index, (name, _))| {
            (!name.starts_with("__thaw_getter_") && !name.starts_with("__thaw_setter_"))
                .then_some((index, object_array_index_key(name)))
        })
        .collect::<Vec<_>>();
    indices.sort_by_key(|(index, array_index)| {
        (array_index.is_none(), array_index.unwrap_or(*index as u32))
    });
    indices.into_iter().map(|(index, _)| index).collect()
}

fn object_array_index_key(name: &str) -> Option<u32> {
    let index = name.parse::<u32>().ok()?;
    (index != u32::MAX && index.to_string() == name).then_some(index)
}

fn object_storage_bytes(fields: &[(String, HirType)]) -> Result<u64, String> {
    object_field_offset(fields, fields.len())
}
const ASYNC_COMPLETION_OFFSET: u64 = 0;
const ASYNC_STATE_OFFSET: u64 = 8;
const ASYNC_WAITING_OFFSET: u64 = 16;
const ASYNC_RESULT_OFFSET: u64 = ASYNC_FRAME_BYTES;
const CLOSURE_THIS_ENTRY_OFFSET: u64 = 8;
const CLOSURE_CAPTURE_BASE: u64 = 16;
const BOUND_CLOSURE_SOURCE_OFFSET: u64 = CLOSURE_CAPTURE_BASE;
const BOUND_CLOSURE_THIS_OFFSET: u64 = CLOSURE_CAPTURE_BASE + 8;
const BOUND_CLOSURE_ARGUMENT_BASE: u64 = CLOSURE_CAPTURE_BASE + 24;

struct AsyncSegment {
    stmts: Vec<HirStmt>,
    awaited: Option<HirExpr>,
    await_guard: Option<(String, bool)>,
    await_next: Option<usize>,
    resume_target: Option<(String, HirType)>,
    rejection_handler: Option<AsyncRejectionHandler>,
    // `None` may be an intentional finalizer route to the promise rejection,
    // not an unassigned handler for an enclosing Try to fill later.
    rejection_handler_authoritative: bool,
}

#[derive(Clone)]
struct AsyncRejectionHandler {
    try_guard: String,
    catch_guard: String,
    catch_binding: String,
    /// Set only on a generated catch-to-outer-catch rethrow. The inner
    /// catch's owner must retire after throw provenance reaches pending.
    rethrow_source_binding: Option<String>,
    disable_guards: Vec<String>,
    parent: Option<Box<AsyncRejectionHandler>>,
}

struct FrameAsyncPlan {
    segments: Vec<AsyncSegment>,
    captures: Vec<(String, HirType)>,
    locals: Vec<(String, HirType)>,
    generated_catch_bindings: Vec<String>,
    ret: HirType,
    guarded_rethrow_handlers: HashMap<String, AsyncRejectionHandler>,
    returns_on_all_paths: bool,
}

enum AsyncBlockExit {
    Continue,
    Returned,
    Rejected,
}

#[derive(Clone, Copy)]
struct CatchOwnerRoot<'ctx> {
    cell: PointerValue<'ctx>,
    catch_depth: usize,
    loop_depth: usize,
}

pub struct HirCompiler<'ctx> {
    context: &'ctx Context,
    module: Module<'ctx>,
    builder: Builder<'ctx>,
    variables: HashMap<String, (PointerValue<'ctx>, BasicTypeEnum<'ctx>)>,
    /// Only compiler-created catch cells enter this map; source-visible names
    /// (including a matching metadata suffix) never establish provenance.
    catch_native_text: HashMap<String, (PointerValue<'ctx>, PointerValue<'ctx>, PointerValue<'ctx>, PointerValue<'ctx>)>,
    variable_hir_types: HashMap<String, HirType>,
    arena_variables: HashSet<String>,
    /// Only prepromoted JsValue cells have a pending first-capture retain.
    /// Key by the exact LLVM cell value so branch/function map swaps need no mirror.
    pending_js_capture_claims: HashMap<PointerValue<'ctx>, PointerValue<'ctx>>,
    async_frame_cells: HashSet<PointerValue<'ctx>>,
    /// Frame slots holding the current fresh cell of captured `for (let ...)`
    /// bindings. A resume reloads the pointer rather than sharing a value slot.
    for_iteration_frame_slots: HashMap<String, PointerValue<'ctx>>,
    global_variables: HashMap<String, (PointerValue<'ctx>, BasicTypeEnum<'ctx>, HirType)>,
    /// A pointer-sized mirror of the active member in each wrapped native
    /// Promise global. The runtime's existing reset reconciler reads these
    /// slots without interpreting HIR wrapper layouts.
    global_promise_shadows: HashMap<String, PointerValue<'ctx>>,
    module_exception_roots: Vec<PointerValue<'ctx>>,
    function_return_types: HashMap<String, HirType>,
    ffi_signatures: HashMap<String, FfiSignature>,
    /// Stack of enclosing `try` targets. `throw` and a failed nested Thaw
    /// call target the innermost entry; an empty stack propagates by returning
    /// from the current function with the pending exception left intact.
    catch_stack: Vec<BasicBlock<'ctx>>,
    catch_owner_roots: Vec<CatchOwnerRoot<'ctx>>,
    /// During HTTP's exact graph conversion, a pre-reserved packet owns any
    /// freshly taken std HostError NativeStr before synthetic catch transfer.
    active_graph_host_error_owner: Option<PointerValue<'ctx>>,
    loop_stack: Vec<(BasicBlock<'ctx>, BasicBlock<'ctx>)>,
    loop_promotion_scopes: Vec<(BasicBlock<'ctx>, HashSet<String>)>,
    /// Functions whose async ABI is a Promise-returning ramp rather than the
    /// legacy synchronous V1 ABI. Seeded to a fixed point before declarations
    /// so callers and callees agree on the LLVM signature.
    frame_async_functions: HashMap<String, HirType>,
    async_lambda_captures: HashMap<String, Vec<HirParam>>,
    promise_returning_functions: HashSet<String>,
    active_async_completion: Option<PointerValue<'ctx>>,
    next_lambda: usize,
    /// One graph finisher per resolved Promise HIR type. Repeated reflection
    /// of the same native Promise must register the same native adapter.
    graph_promise_finishers: Vec<(HirType, PointerValue<'ctx>)>,
    uses_napi: bool,
    uses_quickjs: bool,
    uses_quickjs_handles: bool,
    /// Compile full-layout native projection adapters only in modules whose
    /// completed HIR actually crosses a fixed object into QuickJS.
    projects_native_objects: bool,
    /// Nonprefix structural aliases need tracked owner identities even in
    /// modules that never start QuickJS.
    tracks_physical_object_layouts: bool,
    /// Non-arrow Json receivers and live Json literals create owned boxes
    /// that may escape through generated globals or captured arena cells.
    tracks_owned_json_roots: bool,
    /// A suspended generator can own a QuickJS handle through an arena cell.
    tracks_arena_handle_owners: bool,
    /// Promise-bearing native values can outlive one arena reset through a
    /// module slot, stable array handle, or callback owner share.
    tracks_native_promise_owners: bool,
    /// True while compiling a QuickJS-backed dynamic call's own JSON-shaped
    /// arguments (set/restored around that one argument-marshaling loop in
    /// `compile_typed_dynamic_call`, so a nested dynamic call compiled
    /// while marshaling an outer one's arguments doesn't see this as
    /// already on). Lets `compile_dynamic_value_placeholder` (json_bridge.rs)
    /// tell that context apart from one where embedding a live JS value
    /// handle has no meaning and should be rejected instead (console.log
    /// formatting, a `Dictionary` literal, an N-API call).
    compiling_quickjs_dynamic_arguments: bool,
}

// Inspect completed HIR, including nested closures and module initializers,
// before deciding whether native allocations need a deferred JS projection.
// This is deliberately a call-site check: ordinary native objects never pull
// the QuickJS adapter into modules that do not project an object to JS.
fn hir_contains_named_call(program: &HirProgram, name: &str) -> bool {
    fn stmts(body: &[HirStmt], name: &str) -> bool {
        body.iter().any(|stmt| match stmt {
            HirStmt::Expr(expr) | HirStmt::Throw(expr) | HirStmt::Let(_, _, expr) => expr_has_call(expr, name),
            HirStmt::Return(Some(expr)) => expr_has_call(expr, name),
            HirStmt::If(test, yes, no) => expr_has_call(test, name) || stmts(yes, name) || stmts(no, name),
            HirStmt::While(test, body) => expr_has_call(test, name) || stmts(body, name),
            HirStmt::Finally(body, _) => stmts(body, name),
            HirStmt::Try(body, _, catch, _) => stmts(body, name) || stmts(catch, name),
            HirStmt::Return(None) | HirStmt::Break | HirStmt::Continue
            | HirStmt::BreakDepth(_) | HirStmt::ContinueDepth(_) => false,
        })
    }

    fn expr_has_call(expr: &HirExpr, name: &str) -> bool {
        // allSettled exposes an arbitrary original rejection owner as a
        // JsValue, so its callback may invoke a producer's deferred factory.
        if name == "__thaw_build_native_object_wrapper"
            && matches!(expr, HirExpr::PromiseAllSettled(_, _)
                | HirExpr::PromiseAllSettledArray(_, _)) {
            return true;
        }
        match expr {
            HirExpr::Call(callee, args) => {
                if matches!(callee.as_ref(), HirExpr::Var(target)
                    if target == "__thaw_register_native_object_projector") {
                    // The deferred factory itself contains a projection, but
                    // never runs in a native-only program. Inspect only the
                    // owner expression at this registration site.
                    return args.first().is_some_and(|owner| expr_has_call(owner, name));
                }
                matches!(callee.as_ref(), HirExpr::Var(target) if target == name)
                    || expr_has_call(callee, name) || args.iter().any(|arg| expr_has_call(arg, name))
            }
            HirExpr::BinOp(_, left, right) | HirExpr::EvalThen(left, right)
            | HirExpr::UnionMemberIsEqual(left, right, _, _)
            | HirExpr::UnionIsEqual(left, right, _)
            | HirExpr::Index(left, right) | HirExpr::TypedIndex(left, right, _)
            | HirExpr::ArraySetLen(left, right, _)
            | HirExpr::DynamicPropAccess(left, right, _, _)
            | HirExpr::JsonKey(left, right) | HirExpr::JsonDelete(left, right)
            | HirExpr::JsonIndex(left, right) =>
                expr_has_call(left, name) || expr_has_call(right, name),
            HirExpr::JsonSet(object, key, value, _, _)
            | HirExpr::JsonIndexSet(object, key, value)
            | HirExpr::IndexAssign(object, key, value) =>
                expr_has_call(object, name) || expr_has_call(key, name) || expr_has_call(value, name),
            HirExpr::Conditional(test, yes, no, _) =>
                expr_has_call(test, name) || expr_has_call(yes, name) || expr_has_call(no, name),
            HirExpr::FunctionCallWithThis(callee, receiver, args, _, _)
            | HirExpr::FunctionBindThis(callee, receiver, args, _, _) =>
                expr_has_call(callee, name) || expr_has_call(receiver, name)
                    || args.iter().any(|arg| expr_has_call(arg, name)),
            HirExpr::DynamicCall(_, args) | HirExpr::FfiCall(_, args)
            | HirExpr::ArrayLit(args) | HirExpr::ArrayConcat(args, _)
            | HirExpr::PromiseAll(args, _) | HirExpr::PromiseAllTuple(args, _)
            | HirExpr::PromiseRace(args, _) | HirExpr::PromiseAny(args, _)
            | HirExpr::PromiseAllSettled(args, _) => args.iter().any(|arg| expr_has_call(arg, name)),
            HirExpr::Await(inner) | HirExpr::AwaitPromise(inner, _)
            | HirExpr::PromiseAllArray(inner, _) | HirExpr::PromiseRaceArray(inner, _)
            | HirExpr::PromiseAnyArray(inner, _) | HirExpr::PromiseAllSettledArray(inner, _)
            | HirExpr::Assign(_, inner) | HirExpr::ArrayAlloc(inner, _)
            | HirExpr::ArrayLen(inner) | HirExpr::EnumReverseLookup(inner, _)
            | HirExpr::JsonAsNumber(inner) | HirExpr::JsonAsString(inner)
            | HirExpr::JsonAsBool(inner) | HirExpr::JsonAsNative(inner, _)
            | HirExpr::JsValueAsJson(inner) | HirExpr::OptionalSome(inner, _)
            | HirExpr::OptionalIsNone(inner, _) | HirExpr::OptionalValue(inner, _)
            | HirExpr::NullableSome(inner, _) | HirExpr::NullableIsNone(inner, _)
            | HirExpr::NullableValue(inner, _) | HirExpr::NullishSome(inner, _)
            | HirExpr::NullishIsNull(inner, _) | HirExpr::NullishIsUndefined(inner, _)
            | HirExpr::NullishIsNone(inner, _) | HirExpr::NullishValue(inner, _)
            | HirExpr::UnionInject(inner, _, _) | HirExpr::UnionTag(inner, _)
            | HirExpr::UnionValue(inner, _, _) | HirExpr::RecursiveClosure(_, _, inner)
            | HirExpr::TypedClosure(_, inner) | HirExpr::NonArrowFunction(inner)
            | HirExpr::PromiseNew(inner, _, _, _) | HirExpr::PromiseNewMixed(inner, _, _)
            | HirExpr::PropAccess(inner, _, _) | HirExpr::JsonGet(inner, _) => expr_has_call(inner, name),
            HirExpr::Lambda(_, _, _, body) => expr_has_call(body, name),
            HirExpr::PromiseThen(source, callback, _, _, _, _)
            | HirExpr::PromiseFinally(source, callback, _, _) =>
                expr_has_call(source, name) || expr_has_call(callback, name),
            HirExpr::PromiseThenBoth(source, fulfilled, rejected, _, _, _, _) =>
                expr_has_call(source, name)
                    || fulfilled.as_ref().is_some_and(|callback| expr_has_call(callback, name))
                    || expr_has_call(rejected, name),
            HirExpr::ThrowValue(error, fallback) =>
                expr_has_call(error, name) || expr_has_call(fallback, name),
            HirExpr::Block(body) => stmts(body, name),
            HirExpr::ObjectLit(fields) | HirExpr::JsonObjectLit(fields, _) =>
                fields.iter().any(|(_, value)| expr_has_call(value, name)),
            HirExpr::PropAssign(object, _, _, value) =>
                expr_has_call(object, name) || expr_has_call(value, name),
            HirExpr::Lit(_) | HirExpr::Var(_) | HirExpr::OptionalNone(_)
            | HirExpr::NullableNone(_) | HirExpr::NullishNull(_)
            | HirExpr::NullishUndefined(_) | HirExpr::EnvVar(_)
            | HirExpr::ObjectAlloc(_) | HirExpr::ClassAlloc(_)
            | HirExpr::FunctionRef(..) | HirExpr::MethodRef(..)
            | HirExpr::PostfixUpdate(_, _) => false,
        }
    }

    program.globals.iter().any(|global| expr_has_call(&global.init, name))
        || program.initializers.iter().any(|step| match step {
            HirInitStep::StoreGlobal(_, expr) => expr_has_call(expr, name),
            HirInitStep::Statement(stmt) => stmts(std::slice::from_ref(stmt), name),
            HirInitStep::ExecutionBoundary(_) | HirInitStep::ModuleBoundary { .. } => false,
        })
        || program.functions.iter().any(|function| stmts(&function.body, name))
}

impl<'ctx> HirCompiler<'ctx> {
    pub fn new(context: &'ctx Context, module_name: &str) -> Self {
        Self {
            context,
            module: context.create_module(module_name),
            builder: context.create_builder(),
            variables: HashMap::new(),
            catch_native_text: HashMap::new(),
            variable_hir_types: HashMap::new(),
            arena_variables: HashSet::new(),
            pending_js_capture_claims: HashMap::new(),
            async_frame_cells: HashSet::new(),
            for_iteration_frame_slots: HashMap::new(),
            global_variables: HashMap::new(),
            global_promise_shadows: HashMap::new(),
            module_exception_roots: Vec::new(),
            function_return_types: HashMap::new(),
            ffi_signatures: HashMap::new(),
            catch_stack: Vec::new(),
            catch_owner_roots: Vec::new(),
            active_graph_host_error_owner: None,
            loop_stack: Vec::new(),
            loop_promotion_scopes: Vec::new(),
            frame_async_functions: HashMap::new(),
            async_lambda_captures: HashMap::new(),
            promise_returning_functions: HashSet::new(),
            active_async_completion: None,
            next_lambda: 0,
            graph_promise_finishers: Vec::new(),
            uses_napi: false,
            uses_quickjs: false,
            uses_quickjs_handles: false,
            projects_native_objects: false,
            tracks_physical_object_layouts: false,
            tracks_owned_json_roots: false,
            tracks_arena_handle_owners: false,
            tracks_native_promise_owners: false,
            compiling_quickjs_dynamic_arguments: false,
        }
    }

    pub fn compile_program(&mut self, program: &HirProgram) -> Result<(), String> {
        self.projects_native_objects =
            hir_contains_named_call(program, "__thaw_build_native_object_wrapper")
            || hir_contains_named_call(program, "__thaw_lookup_native_projector")
            // The HTTP cleanup reporter constructs the caught-value adapter
            // after ordinary HIR lowering. Its native Object arm calls the
            // projector even when no source expression requested a JS
            // wrapper, so register native factories before module init.
            || program.extern_functions.iter().any(|signature| signature.symbol == "createServer");
        self.tracks_physical_object_layouts =
            hir_contains_named_call(program, "__thaw_register_native_object_layout");
        self.tracks_arena_handle_owners =
            hir_contains_named_call(program, "__thaw_track_generator_super_base");
        self.tracks_native_promise_owners = program.globals.iter().any(|global| {
            Self::type_contains_native_promise(&global.ty)
                || matches!(&global.ty, HirType::Array(element)
                    if Self::type_contains_native_promise(element))
        });
        self.declare_runtime_builtins();
        self.declare_exception_state();
        self.declare_globals(program)?;
        self.discover_frame_async_functions(program);
        self.function_return_types = program
            .functions
            .iter()
            .map(|function| (function.name.clone(), function.ret.clone()))
            .collect();

        self.ffi_signatures = program
            .extern_functions
            .iter()
            .map(|signature| (signature.symbol.clone(), signature.clone()))
            .collect();
        for sig in &program.extern_functions {
            self.declare_extern_function(sig)?;
        }
        for func in &program.functions {
            self.declare_function(func)?;
        }
        self.emit_top_level_init(program)?;
        for func in &program.functions {
            if func.name.starts_with("__thaw_lazy_module_")
                && func.name["__thaw_lazy_module_".len()..]
                    .parse::<usize>()
                    .ok()
                    .is_some_and(|index| program.initializers.iter().any(|step| {
                        matches!(step, HirInitStep::ModuleBoundary { index: owner, runtime: true, .. } if *owner == index)
                    }))
            {
                continue;
            }
            self.compile_function_body(func)?;
        }

        let has_main = self.module.get_function(USER_MAIN_SYMBOL).is_some();
        let handler_hir = program.functions.iter().find(|f| f.name == "handler");

        match (has_main, handler_hir) {
            (true, Some(_)) => {
                return Err("a program can define `main` or `handler`, not both".to_string())
            }
            (true, None) => self.emit_c_main_entry(),
            (false, Some(handler)) => self.emit_lambda_entry(handler)?,
            (false, None) => self.emit_c_main_entry(),
        }

        self.module
            .verify()
            .map_err(|e| format!("module verification failed:\n{e}"))
    }

    /// Whether the generated module actually calls the QuickJS fallback host.
    /// Callers can omit that archive when every operation was lowered to the
    /// native/JIT paths.
    pub fn uses_quickjs(&self) -> bool {
        self.uses_quickjs
    }

    /// Whether the generated module calls the N-API addon host.
    pub fn uses_napi(&self) -> bool {
        self.uses_napi
    }

    fn declare_exception_state(&mut self) {
        let ptr_ty = self.context.ptr_type(AddressSpace::default());
        let pending = self
            .module
            .add_global(ptr_ty, None, PENDING_EXCEPTION_SYMBOL);
        pending.set_linkage(Linkage::Internal);
        pending.set_initializer(&ptr_ty.const_null());
        let rejection = self
            .module
            .add_global(ptr_ty, None, PENDING_REJECTION_SYMBOL);
        rejection.set_linkage(Linkage::Internal);
        rejection.set_initializer(&ptr_ty.const_null());
        let native_text = self.module.add_global(ptr_ty, None, PENDING_EXCEPTION_NATIVE_TEXT_SYMBOL);
        native_text.set_linkage(Linkage::Internal);
        native_text.set_initializer(&ptr_ty.const_null());
        let native_text_owner = self.module.add_global(ptr_ty, None, PENDING_EXCEPTION_NATIVE_TEXT_OWNER_SYMBOL);
        native_text_owner.set_linkage(Linkage::Internal);
        native_text_owner.set_initializer(&ptr_ty.const_null());
        let pending_object = self
            .module
            .add_global(ptr_ty, None, PENDING_EXCEPTION_OBJECT_SYMBOL);
        pending_object.set_linkage(Linkage::Internal);
        pending_object.set_initializer(&ptr_ty.const_null());
        let pending_json_owner = self.module.add_global(ptr_ty, None, PENDING_EXCEPTION_JSON_OWNER_SYMBOL);
        pending_json_owner.set_linkage(Linkage::Internal);
        pending_json_owner.set_initializer(&ptr_ty.const_null());
        let active_exception_root = self.module.add_global(ptr_ty, None, HTTP_SAVED_EXCEPTION_ROOT_SYMBOL);
        active_exception_root.set_linkage(Linkage::Internal);
        active_exception_root.set_initializer(&ptr_ty.const_null());
        self.module_exception_roots.push(active_exception_root.as_pointer_value());
        let pending_aggregate = self.module.add_global(ptr_ty, None, PENDING_EXCEPTION_AGGREGATE_SYMBOL);
        pending_aggregate.set_linkage(Linkage::Internal);
        pending_aggregate.set_initializer(&ptr_ty.const_null());
        let typed_slots: [(&str, BasicTypeEnum); 4] = [
            (
                PENDING_EXCEPTION_VALUE_TAG_SYMBOL,
                BasicTypeEnum::from(self.context.i64_type()),
            ),
            (
                PENDING_EXCEPTION_F64_SYMBOL,
                BasicTypeEnum::from(self.context.f64_type()),
            ),
            (
                PENDING_EXCEPTION_I64_SYMBOL,
                BasicTypeEnum::from(self.context.i64_type()),
            ),
            (
                PENDING_EXCEPTION_BOOL_SYMBOL,
                BasicTypeEnum::from(self.context.bool_type()),
            ),
        ];
        for (name, ty) in typed_slots {
            let value = self.module.add_global(ty, None, name);
            value.set_linkage(Linkage::Internal);
            value.set_initializer(&ty.const_zero());
        }
    }

    fn declare_globals(&mut self, program: &HirProgram) -> Result<(), String> {
        for global in &program.globals {
            let ty = self.basic_type(&global.ty)?;
            let symbol = format!("__thaw_global_{}", global.name);
            let value = self.module.add_global(ty, None, &symbol);
            value.set_linkage(Linkage::Internal);
            value.set_initializer(&ty.const_zero());
            self.global_variables.insert(
                global.name.clone(),
                (value.as_pointer_value(), ty, global.ty.clone()),
            );
            if Self::type_contains_native_promise(&global.ty)
                && !matches!(&global.ty, HirType::Promise(_))
            {
                let pointer = self.context.ptr_type(AddressSpace::default());
                let shadow = self.module.add_global(pointer, None,
                    &format!("__thaw_promise_global_shadow_{}", global.name));
                shadow.set_linkage(Linkage::Internal);
                shadow.set_initializer(&pointer.const_null());
                self.global_promise_shadows.insert(global.name.clone(), shadow.as_pointer_value());
            }
        }
        Ok(())
    }

    fn seed_global_variables(&mut self) {
        for (name, (pointer, llvm_type, hir_type)) in &self.global_variables {
            self.variables.insert(name.clone(), (*pointer, *llvm_type));
            self.variable_hir_types
                .insert(name.clone(), hir_type.clone());
        }
    }

    fn emit_top_level_init(&mut self, program: &HirProgram) -> Result<(), String> {
        if program.initializers.is_empty() {
            return Ok(());
        }
        let mut common = Vec::new();
        let mut groups: Vec<(usize, bool, bool, Vec<usize>, Vec<HirInitStep>)> = Vec::new();
        for step in &program.initializers {
            if let HirInitStep::ModuleBoundary { index, eager, runtime, static_dependencies } = step {
                groups.push((*index, *eager, *runtime, static_dependencies.clone(), Vec::new()));
            } else if let Some((_, _, _, _, steps)) = groups.last_mut() {
                steps.push(step.clone());
            } else {
                common.push(step.clone());
            }
        }
        for (index, _, runtime, dependencies, steps) in &groups {
            if *runtime {
                self.emit_guarded_module_init(*index, dependencies, steps)?;
            }
        }
        let bool_type = self.context.bool_type();
        let guard = self
            .module
            .add_global(bool_type, None, TOP_LEVEL_INIT_GUARD_SYMBOL);
        guard.set_linkage(Linkage::Internal);
        guard.set_initializer(&bool_type.const_zero());

        let init = self.module.add_function(
            TOP_LEVEL_INIT_SYMBOL,
            self.context.void_type().fn_type(&[], false),
            Some(Linkage::Internal),
        );
        let entry = self.context.append_basic_block(init, "entry");
        let initialize = self.context.append_basic_block(init, "initialize");
        let done = self.context.append_basic_block(init, "done");
        self.builder.position_at_end(entry);
        let initialized = self
            .builder
            .build_load(bool_type, guard.as_pointer_value(), "top_level_initialized")
            .map_err(|error| error.to_string())?
            .into_int_value();
        self.builder
            .build_conditional_branch(initialized, done, initialize)
            .map_err(|error| error.to_string())?;

        self.builder.position_at_end(initialize);
        self.builder
            .build_store(guard.as_pointer_value(), bool_type.const_int(1, false))
            .map_err(|error| error.to_string())?;
        self.variables.clear();
        self.for_iteration_frame_slots.clear();
        self.catch_native_text.clear();
        self.variable_hir_types.clear();
        self.arena_variables.clear();
        self.catch_stack.clear();
        self.catch_owner_roots.clear();
        self.active_graph_host_error_owner = None;
        self.seed_global_variables();
        let mut execute = true;
        for step in &common {
            match step {
                HirInitStep::ExecutionBoundary(enabled) => {
                    execute = *enabled;
                }
                HirInitStep::ModuleBoundary { .. } => unreachable!(),
                _ if !execute => {}
                HirInitStep::StoreGlobal(name, expression) => {
                    let value = self.compile_expr(expression)?;
                    let (pointer, _, ty) = self.global_variables[name].clone();
                    self.store_native_promise_global_shadow(name, &ty, value)?;
                    self.builder
                        .build_store(pointer, value)
                        .map_err(|error| error.to_string())?;
                }
                HirInitStep::Statement(statement) => {
                    if self.compile_stmt(statement)? {
                        break;
                    }
                }
            }
        }
        for (index, eager, runtime, _, _) in &groups {
            if self.builder.get_insert_block().is_some_and(|block| block.get_terminator().is_some()) {
                break;
            }
            if !eager || !runtime {
                continue;
            }
            let function = self.module.get_function(&format!("__thaw_lazy_module_{index}"))
                .ok_or_else(|| format!("missing module initializer {index}"))?;
            self.builder.build_call(function, &[], "initialize_eager_module")
                .map_err(|error| error.to_string())?;
            let pending = self.builder.build_load(
                self.context.ptr_type(AddressSpace::default()),
                self.pending_exception().as_pointer_value(),
                "eager_module_exception",
            ).map_err(|error| error.to_string())?.into_pointer_value();
            let failed = self.builder.build_is_not_null(pending, "eager_module_failed")
                .map_err(|error| error.to_string())?;
            let next = self.context.append_basic_block(init, "next_eager_module");
            self.builder.build_conditional_branch(failed, done, next)
                .map_err(|error| error.to_string())?;
            self.builder.position_at_end(next);
        }
        if self
            .builder
            .get_insert_block()
            .is_some_and(|block| block.get_terminator().is_none())
        {
            self.builder
                .build_unconditional_branch(done)
                .map_err(|error| error.to_string())?;
        }
        self.builder.position_at_end(done);
        self.builder
            .build_return(None)
            .map_err(|error| error.to_string())?;
        Ok(())
    }

    fn emit_guarded_module_init(
        &mut self,
        index: usize,
        dependencies: &[usize],
        steps: &[HirInitStep],
    ) -> Result<(), String> {
        let name = format!("__thaw_lazy_module_{index}");
        let function = self.module.get_function(&name)
            .ok_or_else(|| format!("missing generated module initializer {index}"))?;
        let state_ty = self.context.i8_type();
        let state = self.module.add_global(state_ty, None, &format!("{name}_state"));
        state.set_linkage(Linkage::Internal);
        state.set_initializer(&state_ty.const_zero());
        let exception_slots = [
            (PENDING_EXCEPTION_SYMBOL, self.context.ptr_type(AddressSpace::default()).into()),
            (PENDING_EXCEPTION_NATIVE_TEXT_SYMBOL, self.context.ptr_type(AddressSpace::default()).into()),
            (PENDING_EXCEPTION_NATIVE_TEXT_OWNER_SYMBOL, self.context.ptr_type(AddressSpace::default()).into()),
            (PENDING_EXCEPTION_OBJECT_SYMBOL, self.context.ptr_type(AddressSpace::default()).into()),
            (PENDING_EXCEPTION_JSON_OWNER_SYMBOL, self.context.ptr_type(AddressSpace::default()).into()),
            (PENDING_EXCEPTION_AGGREGATE_SYMBOL, self.context.ptr_type(AddressSpace::default()).into()),
            (PENDING_EXCEPTION_VALUE_TAG_SYMBOL, self.context.i64_type().into()),
            (PENDING_EXCEPTION_F64_SYMBOL, self.context.f64_type().into()),
            (PENDING_EXCEPTION_I64_SYMBOL, self.context.i64_type().into()),
            (PENDING_EXCEPTION_BOOL_SYMBOL, self.context.bool_type().into()),
        ];
        let mut cached = Vec::new();
        for (symbol, ty) in exception_slots {
            let slot = self.module.add_global(ty, None, &format!("{name}_{symbol}"));
            slot.set_linkage(Linkage::Internal);
            slot.set_initializer(&ty.const_zero());
            if symbol == PENDING_EXCEPTION_SYMBOL || symbol == PENDING_EXCEPTION_OBJECT_SYMBOL || symbol == PENDING_EXCEPTION_JSON_OWNER_SYMBOL || symbol == PENDING_EXCEPTION_AGGREGATE_SYMBOL {
                self.module_exception_roots.push(slot.as_pointer_value());
            }
            cached.push((self.module.get_global(symbol).unwrap(), slot, ty));
        }
        let entry = self.context.append_basic_block(function, "entry");
        let first = self.context.append_basic_block(function, "first_import");
        let prior = self.context.append_basic_block(function, "prior_import");
        let restore = self.context.append_basic_block(function, "restore_failure");
        let failed = self.context.append_basic_block(function, "cache_failure");
        let done = self.context.append_basic_block(function, "done");
        self.builder.position_at_end(entry);
        let current = self.builder.build_load(state_ty, state.as_pointer_value(), "module_state")
            .map_err(|error| error.to_string())?.into_int_value();
        let uninitialized = self.builder.build_int_compare(IntPredicate::EQ, current, state_ty.const_zero(), "module_uninitialized")
            .map_err(|error| error.to_string())?;
        self.builder.build_conditional_branch(uninitialized, first, prior)
            .map_err(|error| error.to_string())?;
        self.builder.position_at_end(prior);
        let had_failed = self.builder.build_int_compare(IntPredicate::EQ, current, state_ty.const_int(3, false), "module_had_failed")
            .map_err(|error| error.to_string())?;
        self.builder.build_conditional_branch(had_failed, restore, done)
            .map_err(|error| error.to_string())?;
        self.builder.position_at_end(restore);
        for (pending, saved, ty) in &cached {
            let value = self.builder.build_load(*ty, saved.as_pointer_value(), "saved_exception")
                .map_err(|error| error.to_string())?;
            self.builder.build_store(pending.as_pointer_value(), value)
                .map_err(|error| error.to_string())?;
        }
        self.builder.build_unconditional_branch(done).map_err(|error| error.to_string())?;

        self.builder.position_at_end(first);
        self.builder.build_store(state.as_pointer_value(), state_ty.const_int(1, false))
            .map_err(|error| error.to_string())?;
        self.variables.clear();
        self.for_iteration_frame_slots.clear();
        self.catch_native_text.clear();
        self.variable_hir_types.clear();
        self.arena_variables.clear();
        self.catch_stack.clear();
        self.catch_owner_roots.clear();
        self.active_graph_host_error_owner = None;
        self.seed_global_variables();
        self.catch_stack.push(failed);
        for dependency in dependencies {
            let initializer = self.module.get_function(&format!("__thaw_lazy_module_{dependency}"))
                .ok_or_else(|| format!("missing static dependency initializer {dependency}"))?;
            self.builder.build_call(initializer, &[], "initialize_static_dependency")
                .map_err(|error| error.to_string())?;
            self.branch_on_pending_exception()?;
        }
        let mut execute = true;
        for step in steps {
            match step {
                HirInitStep::ExecutionBoundary(enabled) => execute = *enabled,
                HirInitStep::ModuleBoundary { .. } => unreachable!(),
                _ if !execute => {}
                HirInitStep::StoreGlobal(name, expression) => {
                    let value = self.compile_expr(expression)?;
                    let (pointer, _, ty) = self.global_variables[name].clone();
                    self.store_native_promise_global_shadow(name, &ty, value)?;
                    self.builder.build_store(pointer, value).map_err(|error| error.to_string())?;
                }
                HirInitStep::Statement(statement) => {
                    if self.compile_stmt(statement)? { break; }
                }
            }
        }
        self.catch_stack.pop();
        if self.builder.get_insert_block().is_some_and(|block| block.get_terminator().is_none()) {
            self.builder.build_store(state.as_pointer_value(), state_ty.const_int(2, false))
                .map_err(|error| error.to_string())?;
            self.builder.build_unconditional_branch(done).map_err(|error| error.to_string())?;
        }
        self.builder.position_at_end(failed);
        for (pending, saved, ty) in &cached {
            let value = self.builder.build_load(*ty, pending.as_pointer_value(), "failed_exception")
                .map_err(|error| error.to_string())?;
            self.builder.build_store(saved.as_pointer_value(), value)
                .map_err(|error| error.to_string())?;
        }
        self.builder.build_store(state.as_pointer_value(), state_ty.const_int(3, false))
            .map_err(|error| error.to_string())?;
        self.builder.build_unconditional_branch(done).map_err(|error| error.to_string())?;
        self.builder.position_at_end(done);
        self.builder.build_return(None).map_err(|error| error.to_string())?;
        Ok(())
    }

    fn discover_frame_async_functions(&mut self, program: &HirProgram) {
        // Every async function must expose the same Promise-handle ABI, even
        // when its body happens not to suspend. Otherwise storing or passing
        // its result as `Promise<T>` would depend on an implementation detail
        // of that function's current body.
        self.frame_async_functions = program
            .functions
            .iter()
            .filter(|func| func.is_async)
            .map(|func| (func.name.clone(), func.ret.clone()))
            .collect();
        self.promise_returning_functions = program
            .functions
            .iter()
            .filter(|func| !func.is_async && matches!(func.ret, HirType::Promise(_)))
            .map(|func| func.name.clone())
            .collect();
    }

    fn stmt_awaits_frame_source(
        stmt: &HirStmt,
        frame_functions: &std::collections::HashSet<String>,
    ) -> bool {
        match stmt {
            HirStmt::Expr(expr) | HirStmt::Throw(expr) | HirStmt::Let(_, _, expr) => {
                Self::expr_awaits_frame_source(expr, frame_functions)
            }
            HirStmt::Return(Some(expr)) => Self::expr_awaits_frame_source(expr, frame_functions),
            HirStmt::Return(None) => false,
            HirStmt::If(cond, then_body, else_body) => {
                Self::expr_awaits_frame_source(cond, frame_functions)
                    || then_body
                        .iter()
                        .any(|stmt| Self::stmt_awaits_frame_source(stmt, frame_functions))
                    || else_body
                        .iter()
                        .any(|stmt| Self::stmt_awaits_frame_source(stmt, frame_functions))
            }
            HirStmt::While(cond, body) => {
                Self::expr_awaits_frame_source(cond, frame_functions)
                    || body
                        .iter()
                        .any(|stmt| Self::stmt_awaits_frame_source(stmt, frame_functions))
            }
            HirStmt::Break
            | HirStmt::Continue
            | HirStmt::BreakDepth(_)
            | HirStmt::ContinueDepth(_) => false,
            HirStmt::Finally(body, _) => body
                .iter()
                .any(|stmt| Self::stmt_awaits_frame_source(stmt, frame_functions)),
            HirStmt::Try(body, _, catch_body, _) => body
                .iter()
                .chain(catch_body)
                .any(|stmt| Self::stmt_awaits_frame_source(stmt, frame_functions)),
        }
    }

    fn expr_awaits_frame_source(
        expr: &HirExpr,
        frame_functions: &std::collections::HashSet<String>,
    ) -> bool {
        match expr {
            HirExpr::AwaitPromise(_, _) => true,
            HirExpr::Await(inner) => {
                matches!(
                    inner.as_ref(),
                    HirExpr::PromiseNew(_, _, _, _)
                        | HirExpr::PromiseNewMixed(_, _, _)
                        | HirExpr::PromiseThen(_, _, _, _, _, _)
                        | HirExpr::PromiseThenBoth(_, _, _, _, _, _, _)
                        | HirExpr::PromiseFinally(_, _, _, _)
                        | HirExpr::PromiseAll(_, _)
                        | HirExpr::PromiseAllArray(_, _)
                        | HirExpr::PromiseAllTuple(_, _)
                        | HirExpr::PromiseRace(_, _)
                        | HirExpr::PromiseRaceArray(_, _)
                        | HirExpr::PromiseAny(_, _)
                        | HirExpr::PromiseAnyArray(_, _)
                        | HirExpr::PromiseAllSettled(_, _)
                        | HirExpr::PromiseAllSettledArray(_, _)
                ) || matches!(inner.as_ref(), HirExpr::Call(callee, _)
                    if matches!(callee.as_ref(), HirExpr::Var(name)
                        if name == "sleep" || name == "fetch" || name == "Promise.all" || frame_functions.contains(name)))
                    || Self::expr_awaits_frame_source(inner, frame_functions)
            }
            HirExpr::BinOp(_, left, right)
            | HirExpr::EvalThen(left, right)
            | HirExpr::Index(left, right)
            | HirExpr::TypedIndex(left, right, _)
            | HirExpr::DynamicPropAccess(left, right, _, _) => {
                Self::expr_awaits_frame_source(left, frame_functions)
                    || Self::expr_awaits_frame_source(right, frame_functions)
            }
            HirExpr::Assign(_, value) => Self::expr_awaits_frame_source(value, frame_functions),
            HirExpr::Call(callee, args) => {
                Self::expr_awaits_frame_source(callee, frame_functions)
                    || args
                        .iter()
                        .any(|arg| Self::expr_awaits_frame_source(arg, frame_functions))
            }
            HirExpr::FfiCall(_, args)
            | HirExpr::DynamicCall(_, args)
            | HirExpr::ArrayLit(args)
            | HirExpr::ArrayConcat(args, _)
            | HirExpr::PromiseAll(args, _)
            | HirExpr::PromiseAllTuple(args, _)
            | HirExpr::PromiseRace(args, _)
            | HirExpr::PromiseAny(args, _)
            | HirExpr::PromiseAllSettled(args, _) => args
                .iter()
                .any(|arg| Self::expr_awaits_frame_source(arg, frame_functions)),
            HirExpr::IndexAssign(a, b, c) => {
                Self::expr_awaits_frame_source(a, frame_functions)
                    || Self::expr_awaits_frame_source(b, frame_functions)
                    || Self::expr_awaits_frame_source(c, frame_functions)
            }
            HirExpr::JsonSet(object, key, value, _, _)
            | HirExpr::JsonIndexSet(object, key, value) => [object, key, value]
                .iter()
                .any(|value| Self::expr_awaits_frame_source(value, frame_functions)),
            HirExpr::ObjectLit(fields) | HirExpr::JsonObjectLit(fields, _) => fields
                .iter()
                .any(|(_, value)| Self::expr_awaits_frame_source(value, frame_functions)),
            HirExpr::PropAccess(obj, _, _)
            | HirExpr::ArrayLen(obj)
            | HirExpr::EnumReverseLookup(obj, _)
            | HirExpr::JsonAsNumber(obj)
            | HirExpr::JsonAsString(obj)
            | HirExpr::JsonAsBool(obj) => Self::expr_awaits_frame_source(obj, frame_functions),
            HirExpr::PropAssign(obj, _, _, value)
            | HirExpr::JsonIndex(obj, value)
            | HirExpr::JsonKey(obj, value)
            | HirExpr::JsonDelete(obj, value) => {
                Self::expr_awaits_frame_source(obj, frame_functions)
                    || Self::expr_awaits_frame_source(value, frame_functions)
            }
            HirExpr::JsonGet(obj, _) => Self::expr_awaits_frame_source(obj, frame_functions),
            _ => false,
        }
    }

    fn pending_exception(&self) -> inkwell::values::GlobalValue<'ctx> {
        self.module
            .get_global(PENDING_EXCEPTION_SYMBOL)
            .expect("exception state is declared before code generation")
    }

    fn pending_rejection(&self) -> inkwell::values::GlobalValue<'ctx> {
        self.module
            .get_global(PENDING_REJECTION_SYMBOL)
            .expect("rejection state is declared before code generation")
    }

    fn clear_pending_native_text(&self) -> Result<(), String> {
        // This clears provenance only. A heap owner can be in transit to a
        // catch, async frame, Promise, or deferred packet; each consumer must
        // detach/discard that token at its own proven handoff point.
        let null = self.context.ptr_type(AddressSpace::default()).const_null();
        self.builder.build_store(self.pending_exception_native_text().as_pointer_value(), null)
            .map_err(|error| error.to_string())?;
        // Every fresh native/FFI/host error already clears text provenance.
        // The Promise.any companion belongs to the same exception tuple.
        self.builder.build_store(self.pending_exception_aggregate_errors().as_pointer_value(), null)
            .map_err(|error| error.to_string())?;
        Ok(())
    }

    fn mark_pending_native_text(&self, value: impl Into<BasicValueEnum<'ctx>>) -> Result<(), String> {
        let value = value.into().into_pointer_value();
        // A new text producer replaces the previous pending owner. Retain
        // that owner only when this is the same pointer being propagated;
        // otherwise detach it before freeing its non-reentrant NativeStr
        // allocation. This also covers a new borrowed literal replacing a
        // previous FFI/QuickJS error without treating the literal as owned.
        let ptr = self.context.ptr_type(AddressSpace::default());
        let owner_slot = self.pending_exception_native_text_owner().as_pointer_value();
        let old_owner = self.builder.build_load(ptr, owner_slot, "replaced_native_text_owner")
            .map_err(|error| error.to_string())?.into_pointer_value();
        let same = self.builder.build_int_compare(IntPredicate::EQ,
            self.builder.build_ptr_to_int(old_owner, self.context.i64_type(), "old_text_owner_bits")
                .map_err(|error| error.to_string())?,
            self.builder.build_ptr_to_int(value, self.context.i64_type(), "new_text_bits")
                .map_err(|error| error.to_string())?, "same_native_text_owner")
            .map_err(|error| error.to_string())?;
        let replaced = self.builder.build_select(same, ptr.const_null(), old_owner,
            "replaced_owned_native_text").map_err(|error| error.to_string())?.into_pointer_value();
        let retained = self.builder.build_select(same, old_owner, ptr.const_null(),
            "retained_owned_native_text").map_err(|error| error.to_string())?;
        self.builder.build_store(owner_slot, retained).map_err(|error| error.to_string())?;
        self.builder.build_call(self.module.get_function("thaw_cstring_destroy").unwrap(),
            &[replaced.into()], "destroy_replaced_native_text")
            .map_err(|error| error.to_string())?;
        self.builder.build_store(self.pending_exception_native_text().as_pointer_value(), value)
            .map_err(|error| error.to_string())?;
        Ok(())
    }

    /// Transfer one `owned_string`/`thaw_json_take_host_error` allocation to
    /// the pending tuple. This is distinct from a borrowed native literal.
    /// Callers must first dispose or transfer any previous owner token.
    fn mark_pending_owned_native_text(&self, value: impl Into<BasicValueEnum<'ctx>>) -> Result<(), String> {
        let value = value.into().into_pointer_value();
        self.mark_pending_native_text(value)?;
        self.builder.build_store(self.pending_exception_native_text_owner().as_pointer_value(), value)
            .map_err(|error| error.to_string())?;
        Ok(())
    }

    /// Detach the owned allocation before a caller invokes any destructor or
    /// callback. The borrowed text channel is left intact so a status-0
    /// caller can still propagate the original value before disposition.
    fn take_pending_owned_native_text(&self) -> Result<PointerValue<'ctx>, String> {
        let slot = self.pending_exception_native_text_owner().as_pointer_value();
        let owner = self.builder.build_load(self.context.ptr_type(AddressSpace::default()),
            slot, "taken_pending_native_text_owner")
            .map_err(|error| error.to_string())?.into_pointer_value();
        self.builder.build_store(slot, self.context.ptr_type(AddressSpace::default()).const_null())
            .map_err(|error| error.to_string())?;
        Ok(owner)
    }

    /// Consume a pending NativeStr only after the caller has finished using
    /// the original tuple (for example, after a successful Promise rejection
    /// has copied its text). Status-zero callers must first choose their
    /// propagation path. No borrowed string is released by this operation.
    fn discard_pending_owned_native_text(&self) -> Result<(), String> {
        let ptr = self.context.ptr_type(AddressSpace::default());
        let owner = self.take_pending_owned_native_text()?;
        let owner_bits = self.builder.build_ptr_to_int(owner, self.context.i64_type(),
            "discard_native_owner_bits").map_err(|error| error.to_string())?;
        for slot in [self.pending_exception(), self.pending_exception_native_text()] {
            let value = self.builder.build_load(ptr, slot.as_pointer_value(),
                "discard_native_owner_alias").map_err(|error| error.to_string())?.into_pointer_value();
            let same = self.builder.build_int_compare(IntPredicate::EQ, owner_bits,
                self.builder.build_ptr_to_int(value, self.context.i64_type(),
                    "discard_native_alias_bits").map_err(|error| error.to_string())?,
                "discard_native_alias_matches").map_err(|error| error.to_string())?;
            let cleared = self.builder.build_select(same, ptr.const_null(), value,
                "discard_native_alias_cleared").map_err(|error| error.to_string())?;
            self.builder.build_store(slot.as_pointer_value(), cleared)
                .map_err(|error| error.to_string())?;
        }
        self.builder.build_call(self.module.get_function("thaw_cstring_destroy").unwrap(),
            &[owner.into()], "discard_pending_owned_native_text")
            .map_err(|error| error.to_string())?;
        Ok(())
    }

    /// The runtime has copied trusted native text only when it reports a
    /// successful settlement. On status zero, the original pending tuple is
    /// still the sole exact owner and the caller must choose a propagation
    /// path before clearing it. This primitive deliberately does not decide
    /// how a failed Promise producer returns or reports its error.
    fn discard_pending_owned_native_text_after_settlement(
        &self,
        status: IntValue<'ctx>,
    ) -> Result<(), String> {
        let function = self.current_function();
        let settled = self.builder.build_int_compare(
            IntPredicate::NE, status, status.get_type().const_zero(),
            "native_text_settlement_succeeded",
        ).map_err(|error| error.to_string())?;
        let discard = self.context.append_basic_block(function, "settled_native_text_discard");
        let continue_block = self.context.append_basic_block(function, "native_text_status_checked");
        self.builder.build_conditional_branch(settled, discard, continue_block)
            .map_err(|error| error.to_string())?;
        self.builder.position_at_end(discard);
        self.discard_pending_owned_native_text()?;
        self.builder.build_unconditional_branch(continue_block)
            .map_err(|error| error.to_string())?;
        self.builder.position_at_end(continue_block);
        Ok(())
    }

    fn async_catch_valid_name(name: &str) -> String {
        format!("@@thaw_catch_original_valid:{name}")
    }

    fn async_catch_original_name(name: &str) -> String {
        format!("@@thaw_catch_original_value:{name}")
    }

    fn async_catch_native_name(name: &str) -> String {
        // `@` is not valid in a TypeScript binding. The frame cell is reserved
        // for compiler-generated catch state and cannot be forged by a user Var.
        format!("@@thaw_catch_native_text:{name}")
    }

    /// Restore the tuple captured at the catch boundary only while its visible
    /// binding still holds the original value. A source assignment invalidates
    /// the original slot, so an overwritten catch is thrown as an opaque value.
    fn restore_caught_exception_tuple(
        &self,
        error: PointerValue<'ctx>,
        expression: &HirExpr,
    ) -> Result<bool, String> {
        let HirExpr::Var(binding) = expression else { return Ok(false) };
        let Some((catch_slot, native_slot, original_slot, valid_slot)) = self.catch_native_text.get(binding) else {
            return Ok(false);
        };
        if self.variables.get(binding).map(|(slot, _)| slot) != Some(catch_slot) {
            return Ok(false);
        }
        let ptr = self.context.ptr_type(AddressSpace::default());
        let original = self.builder.build_load(ptr, *original_slot, "caught_original_value")
            .map_err(|error| error.to_string())?.into_pointer_value();
        let same = self.builder.build_int_compare(IntPredicate::EQ,
            self.builder.build_ptr_to_int(error, self.context.i64_type(), "rethrow_error_bits")
                .map_err(|error| error.to_string())?,
            self.builder.build_ptr_to_int(original, self.context.i64_type(), "rethrow_original_bits")
                .map_err(|error| error.to_string())?,
            "rethrow_original_matches",
        ).map_err(|error| error.to_string())?;
        let original_valid = self.builder.build_load(self.context.bool_type(), *valid_slot,
            "rethrow_original_valid").map_err(|error| error.to_string())?.into_int_value();
        let same = self.builder.build_and(same, original_valid, "rethrow_unchanged_catch")
            .map_err(|error| error.to_string())?;
        let native = self.builder.build_load(ptr, *native_slot, "caught_native_text")
            .map_err(|error| error.to_string())?.into_pointer_value();
        let matching_text = self.builder.build_int_compare(IntPredicate::EQ,
            self.builder.build_ptr_to_int(error, self.context.i64_type(), "caught_error_bits")
                .map_err(|error| error.to_string())?,
            self.builder.build_ptr_to_int(native, self.context.i64_type(), "caught_native_bits")
                .map_err(|error| error.to_string())?,
            "caught_native_matches",
        ).map_err(|error| error.to_string())?;
        let native_present = self.builder.build_is_not_null(native, "caught_native_present")
            .map_err(|error| error.to_string())?;
        let trusted = self.builder.build_and(same,
            self.builder.build_and(matching_text, native_present, "caught_native_matches_present")
                .map_err(|error| error.to_string())?,
            "caught_native_trusted",
        ).map_err(|error| error.to_string())?;
        let native = self.builder.build_select(trusted, native, ptr.const_null(), "rethrow_native_text")
            .map_err(|error| error.to_string())?;
        self.mark_pending_native_text(native.into_pointer_value())?;
        let metadata: [(&str, PointerValue<'ctx>, BasicTypeEnum<'ctx>, BasicValueEnum<'ctx>, bool); 8] = [
            ("aggregate", self.pending_exception_aggregate_errors().as_pointer_value(),
                BasicTypeEnum::from(ptr), ptr.const_null().into(), true),
            ("object", self.pending_exception_object().as_pointer_value(),
                BasicTypeEnum::from(ptr), ptr.const_null().into(), false),
            ("json_owner", self.pending_exception_json_owner().as_pointer_value(),
                BasicTypeEnum::from(ptr), ptr.const_null().into(), false),
            ("native_text_owner", self.pending_exception_native_text_owner().as_pointer_value(),
                BasicTypeEnum::from(ptr), ptr.const_null().into(), false),
            ("tag", self.pending_exception_value(PENDING_EXCEPTION_VALUE_TAG_SYMBOL).as_pointer_value(),
                self.context.i64_type().into(), self.context.i64_type().const_int(4, false).into(), false),
            ("f64", self.pending_exception_value(PENDING_EXCEPTION_F64_SYMBOL).as_pointer_value(),
                self.context.f64_type().into(), self.context.f64_type().const_zero().into(), false),
            ("i64", self.pending_exception_value(PENDING_EXCEPTION_I64_SYMBOL).as_pointer_value(),
                self.context.i64_type().into(), self.context.i64_type().const_zero().into(), false),
            ("bool", self.pending_exception_value(PENDING_EXCEPTION_BOOL_SYMBOL).as_pointer_value(),
                self.context.bool_type().into(), self.context.bool_type().const_zero().into(), false),
        ];
        for (suffix, global, ty, zero, require_trusted) in metadata {
            let name = format!("{binding}__thaw_exception_{suffix}");
            let (slot, _) = self.variables.get(&name)
                .ok_or_else(|| format!("missing caught exception metadata `{name}`"))?;
            let value = self.builder.build_load(ty, *slot, "caught_rethrow_metadata")
                .map_err(|error| error.to_string())?;
            let value = self.builder.build_select(if require_trusted { trusted } else { same },
                value, zero, "selected_rethrow_metadata")
                .map_err(|error| error.to_string())?;
            self.builder.build_store(global, value).map_err(|error| error.to_string())?;
        }
        Ok(true)
    }

    fn reject_caught_or_opaque_value(
        &mut self,
        completion: PointerValue<'ctx>,
        error: PointerValue<'ctx>,
        expression: &HirExpr,
        name: &str,
    ) -> Result<(), String> {
        if self.restore_caught_exception_tuple(error, expression)? {
            self.reject_promise_with_pending_exception(completion, error, name)
        } else if let HirExpr::Lit(HirLit::Str(literal)) = expression {
            // ThrowValue is compiler-generated. A framed literal denotes a
            // native Error producer; the ordinary literal is an exact String.
            // NativeStr's length metadata preserves embedded NUL in either
            // branch. The raw two-argument ABI has neither type authority.
            if literal.starts_with('\u{1}') {
                self.builder.build_call(
                    self.module.get_function("thaw_promise_reject_native_text").unwrap(),
                    &[completion.into(), error.into()], name,
                ).map_err(|error| error.to_string())?;
            } else {
                self.builder.build_call(
                    self.module.get_function("thaw_promise_reject_typed_with_native_text").unwrap(),
                    &[completion.into(), error.into(),
                        self.context.i64_type().const_int(4, false).into(),
                        self.context.f64_type().const_zero().into(),
                        self.context.i64_type().const_zero().into(),
                        self.context.bool_type().const_zero().into(),
                        self.context.ptr_type(AddressSpace::default()).const_null().into(),
                        error.into()], name,
                ).map_err(|error| error.to_string())?;
            }
            Ok(())
        } else {
            self.reject_opaque_value(completion, error, name)
        }
    }

    fn reject_opaque_value(&self, completion: PointerValue<'ctx>, error: PointerValue<'ctx>, name: &str) -> Result<(), String> {
        self.builder.build_call(
            self.module.get_function("thaw_promise_reject").unwrap(),
            &[completion.into(), error.into()], name,
        ).map_err(|error| error.to_string())?;
        Ok(())
    }

    fn pending_exception_native_text(&self) -> inkwell::values::GlobalValue<'ctx> {
        self.module.get_global(PENDING_EXCEPTION_NATIVE_TEXT_SYMBOL)
            .expect("native text provenance is declared before code generation")
    }

    fn pending_exception_native_text_owner(&self) -> inkwell::values::GlobalValue<'ctx> {
        self.module.get_global(PENDING_EXCEPTION_NATIVE_TEXT_OWNER_SYMBOL)
            .expect("owned native text provenance is declared before code generation")
    }

    fn pending_exception_aggregate_errors(&self) -> inkwell::values::GlobalValue<'ctx> {
        self.module.get_global(PENDING_EXCEPTION_AGGREGATE_SYMBOL)
            .expect("aggregate exception state is declared before code generation")
    }

    fn pending_exception_object(&self) -> inkwell::values::GlobalValue<'ctx> {
        self.module
            .get_global(PENDING_EXCEPTION_OBJECT_SYMBOL)
            .expect("exception state is declared before code generation")
    }

    fn pending_exception_json_owner(&self) -> inkwell::values::GlobalValue<'ctx> {
        self.module.get_global(PENDING_EXCEPTION_JSON_OWNER_SYMBOL)
            .expect("JSON exception owner is declared before code generation")
    }

    fn http_saved_exception_root(&self) -> inkwell::values::GlobalValue<'ctx> {
        self.module.get_global(HTTP_SAVED_EXCEPTION_ROOT_SYMBOL)
            .expect("HTTP exception snapshot root is declared before code generation")
    }

    fn clear_http_snapshot_spare_fields(&self, cell: PointerValue<'ctx>) -> Result<(), String> {
        let ptr = self.context.ptr_type(AddressSpace::default());
        for offset in [64, 72, 80] {
            let field = unsafe { self.builder.build_in_bounds_gep(self.context.i8_type(), cell,
                &[self.context.i64_type().const_int(offset, false)], "http_snapshot_spare_field")
                .map_err(|error| error.to_string())? };
            self.builder.build_store(field, ptr.const_null()).map_err(|error| error.to_string())?;
        }
        Ok(())
    }

    fn destroy_catch_json_preserving_pending(
        &self, owner: PointerValue<'ctx>, owner_field: PointerValue<'ctx>,
    ) -> Result<IntValue<'ctx>, String> {
        // A Host lease destructor can call back into compiled code. Snapshot
        // all pending channels in an arena-visible cell before it runs; SSA
        // copies alone would be reclaimed by a nested arena reset.
        let ptr = self.context.ptr_type(AddressSpace::default());
        let slots = [
            (self.pending_exception(), BasicTypeEnum::from(ptr)),
            (self.pending_exception_native_text(), BasicTypeEnum::from(ptr)),
            (self.pending_exception_native_text_owner(), BasicTypeEnum::from(ptr)),
            (self.pending_exception_object(), BasicTypeEnum::from(ptr)),
            (self.pending_exception_json_owner(), BasicTypeEnum::from(ptr)),
            (self.pending_exception_aggregate_errors(), BasicTypeEnum::from(ptr)),
            (self.pending_exception_value(PENDING_EXCEPTION_VALUE_TAG_SYMBOL), BasicTypeEnum::from(self.context.i64_type())),
            (self.pending_exception_value(PENDING_EXCEPTION_F64_SYMBOL), BasicTypeEnum::from(self.context.f64_type())),
            (self.pending_exception_value(PENDING_EXCEPTION_I64_SYMBOL), BasicTypeEnum::from(self.context.i64_type())),
            (self.pending_exception_value(PENDING_EXCEPTION_BOOL_SYMBOL), BasicTypeEnum::from(self.context.bool_type())),
        ];
        let mut saved = Vec::with_capacity(slots.len());
        for (slot, ty) in slots.iter().copied() {
            saved.push((slot, self.builder.build_load(ty, slot.as_pointer_value(), "catch_cleanup_saved_field")
                .map_err(|error| error.to_string())?));
        }
        let cell = self.builder.build_call(self.module.get_function("thaw_arena_alloc").unwrap(),
            &[self.context.i64_type().const_int(88, false).into(),
              self.context.i64_type().const_int(8, false).into()], "catch_cleanup_snapshot_cell")
            .map_err(|error| error.to_string())?.try_as_basic_value().basic()
            .ok_or("catch cleanup snapshot allocation returned no pointer")?.into_pointer_value();
        let function = self.current_function();
        let ready = self.context.append_basic_block(function, "catch_cleanup_snapshot_ready");
        let done = self.context.append_basic_block(function, "catch_cleanup_snapshot_done");
        let missing = self.builder.build_is_null(cell, "catch_cleanup_snapshot_missing")
            .map_err(|error| error.to_string())?;
        let before_snapshot = self.builder.get_insert_block()
            .ok_or("catch cleanup has no allocation block")?;
        // Allocation failure leaves the original Box in the existing tracked
        // arena-root ledger. A later reset reclaims it; never call a reentrant
        // destructor while the active exception cannot be rooted.
        self.builder.build_conditional_branch(missing, done, ready)
            .map_err(|error| error.to_string())?;
        self.builder.position_at_end(ready);
        // Reserve the next owned-Json root before the original destructor can
        // reenter. Tracking its replacement after reentry must not allocate
        // or itself destroy a value on an OOM path.
        let reserved = self.builder.build_call(
            self.module.get_function("thaw_json_reserve_arena_owned_root").unwrap(),
            &[], "catch_cleanup_reserved_root")
            .map_err(|error| error.to_string())?.try_as_basic_value().basic()
            .ok_or("catch cleanup root reservation returned no pointer")?.into_pointer_value();
        let reserved_ready = self.context.append_basic_block(function, "catch_cleanup_root_reserved");
        let no_reservation = self.builder.build_is_null(reserved, "catch_cleanup_reservation_missing")
            .map_err(|error| error.to_string())?;
        self.builder.build_conditional_branch(no_reservation, done, reserved_ready)
            .map_err(|error| error.to_string())?;
        self.builder.position_at_end(reserved_ready);
        let root = self.http_saved_exception_root();
        let previous = self.builder.build_load(ptr, root.as_pointer_value(), "catch_cleanup_previous_root")
            .map_err(|error| error.to_string())?;
        self.builder.build_store(cell, previous).map_err(|error| error.to_string())?;
        for (offset, index) in [(8, 0), (16, 1), (24, 2), (32, 3), (40, 4), (48, 5)] {
            let field = unsafe { self.builder.build_in_bounds_gep(self.context.i8_type(), cell,
                &[self.context.i64_type().const_int(offset, false)], "catch_cleanup_pointer_field")
                .map_err(|error| error.to_string())? };
            self.builder.build_store(field, saved[index].1).map_err(|error| error.to_string())?;
        }
        let reservation_field = unsafe { self.builder.build_in_bounds_gep(self.context.i8_type(), cell,
            &[self.context.i64_type().const_int(56, false)], "catch_cleanup_reservation_field")
            .map_err(|error| error.to_string())? };
        self.builder.build_store(reservation_field, reserved).map_err(|error| error.to_string())?;
        self.clear_http_snapshot_spare_fields(cell)?;
        self.builder.build_store(root.as_pointer_value(), cell).map_err(|error| error.to_string())?;
        for (slot, ty) in slots.iter().copied() {
            self.builder.build_store(slot.as_pointer_value(), ty.const_zero())
                .map_err(|error| error.to_string())?;
        }
        // The lexical catch cell remains linked until this point. Only now
        // remove its owner pointer: an arena reset during the destructor must
        // not find and destroy the Box a second time.
        self.builder.build_store(owner_field, ptr.const_null())
            .map_err(|error| error.to_string())?;
        self.builder.build_call(self.module.get_function("thaw_json_destroy").unwrap(),
            &[owner.into()], "destroy_retired_catch_json")
            .map_err(|error| error.to_string())?;
        // Cleanup may itself invoke compiled code. Preserve the original
        // pending tuple and defer the new owned Box to the existing arena
        // ledger with the preallocated token. Destroying it here could create
        // an unbounded chain of fresh exceptions from finalizers.
        let deferred = self.builder.build_load(ptr,
            self.pending_exception_json_owner().as_pointer_value(), "deferred_cleanup_json_owner")
            .map_err(|error| error.to_string())?.into_pointer_value();
        let has_deferred = self.builder.build_is_not_null(deferred, "has_deferred_cleanup_owner")
            .map_err(|error| error.to_string())?;
        let defer = self.context.append_basic_block(function, "defer_reentrant_cleanup_owner");
        let clear_cleanup = self.context.append_basic_block(function, "clear_reentrant_cleanup_packet");
        self.builder.build_conditional_branch(has_deferred, defer, clear_cleanup)
            .map_err(|error| error.to_string())?;
        self.builder.position_at_end(defer);
        self.builder.build_call(self.module.get_function("thaw_json_enqueue_reserved_cleanup").unwrap(),
            &[deferred.into(), reserved.into()], "transfer_reentrant_cleanup_to_ledger")
            .map_err(|error| error.to_string())?;
        self.builder.build_unconditional_branch(clear_cleanup).map_err(|error| error.to_string())?;
        self.builder.position_at_end(clear_cleanup);
        for (slot, ty) in slots.iter().copied() {
            self.builder.build_store(slot.as_pointer_value(), ty.const_zero())
                .map_err(|error| error.to_string())?;
        }
        for (slot, value) in saved {
            self.builder.build_store(slot.as_pointer_value(), value)
                .map_err(|error| error.to_string())?;
        }
        let head = self.builder.build_load(ptr, root.as_pointer_value(), "catch_cleanup_current_root")
            .map_err(|error| error.to_string())?.into_pointer_value();
        let is_head = self.builder.build_int_compare(IntPredicate::EQ,
            self.builder.build_ptr_to_int(head, self.context.i64_type(), "catch_cleanup_head_bits")
                .map_err(|error| error.to_string())?,
            self.builder.build_ptr_to_int(cell, self.context.i64_type(), "catch_cleanup_cell_bits")
                .map_err(|error| error.to_string())?, "catch_cleanup_still_head")
            .map_err(|error| error.to_string())?;
        let pop_snapshot = self.context.append_basic_block(function, "pop_catch_cleanup_snapshot");
        let snapshot_retained = self.context.append_basic_block(function, "catch_cleanup_snapshot_retained");
        self.builder.build_conditional_branch(is_head, pop_snapshot, snapshot_retained)
            .map_err(|error| error.to_string())?;
        self.builder.position_at_end(pop_snapshot);
        self.builder.build_store(root.as_pointer_value(), previous)
            .map_err(|error| error.to_string())?;
        self.builder.build_unconditional_branch(snapshot_retained).map_err(|error| error.to_string())?;
        self.builder.position_at_end(snapshot_retained);
        let completed = self.builder.get_insert_block().ok_or("catch cleanup has no completion block")?;
        self.builder.build_unconditional_branch(done).map_err(|error| error.to_string())?;
        self.builder.position_at_end(done);
        let released = self.builder.build_phi(self.context.bool_type(), "catch_cleanup_released")
            .map_err(|error| error.to_string())?;
        released.add_incoming(&[
            (&self.context.bool_type().const_zero(), before_snapshot),
            (&self.context.bool_type().const_zero(), ready),
            (&self.context.bool_type().const_all_ones(), completed),
        ]);
        Ok(released.as_basic_value().into_int_value())
    }

    fn pop_catch_owner_root(&self, cell: PointerValue<'ctx>, transfer_to_pending: bool) -> Result<(), String> {
        let ptr = self.context.ptr_type(AddressSpace::default());
        let owner_field = unsafe { self.builder.build_in_bounds_gep(self.context.i8_type(), cell,
            &[self.context.i64_type().const_int(8, false)], "retiring_catch_owner_field")
            .map_err(|error| error.to_string())? };
        let owner = self.builder.build_load(ptr, owner_field, "retiring_catch_json_owner")
            .map_err(|error| error.to_string())?.into_pointer_value();
        let previous = self.builder.build_load(
            ptr, cell, "previous_catch_owner_root",
        ).map_err(|error| error.to_string())?;
        let owned = self.builder.build_is_not_null(owner, "retiring_catch_has_owner")
            .map_err(|error| error.to_string())?;
        let destroy = if transfer_to_pending {
            let pending = self.builder.build_load(ptr,
                self.pending_exception_json_owner().as_pointer_value(), "retiring_catch_pending_owner")
                .map_err(|error| error.to_string())?.into_pointer_value();
            let same = self.builder.build_int_compare(IntPredicate::EQ,
                self.builder.build_ptr_to_int(owner, self.context.i64_type(), "retiring_owner_bits")
                    .map_err(|error| error.to_string())?,
                self.builder.build_ptr_to_int(pending, self.context.i64_type(), "retiring_pending_bits")
                    .map_err(|error| error.to_string())?, "catch_owner_transferred_to_pending")
                .map_err(|error| error.to_string())?;
            self.builder.build_and(owned, self.builder.build_not(same, "catch_owner_not_transferred")
                .map_err(|error| error.to_string())?, "retiring_catch_destroy")
                .map_err(|error| error.to_string())?
        } else { owned };
        let function = self.current_function();
        let release = self.context.append_basic_block(function, "release_catch_json_owner");
        let no_release = self.context.append_basic_block(function, "transfer_catch_json_owner");
        let decide_unlink = self.context.append_basic_block(function, "catch_json_owner_retired");
        self.builder.build_conditional_branch(destroy, release, no_release)
            .map_err(|error| error.to_string())?;
        self.builder.position_at_end(release);
        let released = self.destroy_catch_json_preserving_pending(owner, owner_field)?;
        let released_block = self.builder.get_insert_block().ok_or("catch release has no block")?;
        self.builder.build_unconditional_branch(decide_unlink).map_err(|error| error.to_string())?;
        self.builder.position_at_end(no_release);
        self.builder.build_unconditional_branch(decide_unlink).map_err(|error| error.to_string())?;
        self.builder.position_at_end(decide_unlink);
        let can_unlink = self.builder.build_phi(self.context.bool_type(), "catch_owner_can_unlink")
            .map_err(|error| error.to_string())?;
        can_unlink.add_incoming(&[
            (&released, released_block),
            (&self.context.bool_type().const_all_ones(), no_release),
        ]);
        let unlink = self.context.append_basic_block(function, "unlink_retired_catch_owner");
        let done = self.context.append_basic_block(function, "catch_json_owner_released");
        self.builder.build_conditional_branch(can_unlink.as_basic_value().into_int_value(), unlink, done)
            .map_err(|error| error.to_string())?;
        self.builder.position_at_end(unlink);
        self.builder.build_store(owner_field, ptr.const_null()).map_err(|error| error.to_string())?;
        // An inner catch can retain its link after snapshot OOM. Walk link
        // slots instead of assigning the head unconditionally, so removing
        // an outer catch never unroots that still-owned inner Box.
        let root_slot = self.http_saved_exception_root().as_pointer_value();
        let walk_start = self.builder.get_insert_block().ok_or("catch unlink has no start block")?;
        let walk = self.context.append_basic_block(function, "walk_catch_owner_links");
        let inspect = self.context.append_basic_block(function, "inspect_catch_owner_link");
        let found = self.context.append_basic_block(function, "splice_catch_owner_link");
        let advance = self.context.append_basic_block(function, "advance_catch_owner_link");
        self.builder.build_unconditional_branch(walk).map_err(|error| error.to_string())?;
        self.builder.position_at_end(walk);
        let link_slot = self.builder.build_phi(ptr, "current_catch_owner_link_slot")
            .map_err(|error| error.to_string())?;
        link_slot.add_incoming(&[(&root_slot, walk_start)]);
        let slot = link_slot.as_basic_value().into_pointer_value();
        let current = self.builder.build_load(ptr, slot, "current_catch_owner_link")
            .map_err(|error| error.to_string())?.into_pointer_value();
        let missing = self.builder.build_is_null(current, "catch_owner_link_missing")
            .map_err(|error| error.to_string())?;
        self.builder.build_conditional_branch(missing, done, inspect)
            .map_err(|error| error.to_string())?;
        self.builder.position_at_end(inspect);
        let matches = self.builder.build_int_compare(IntPredicate::EQ,
            self.builder.build_ptr_to_int(current, self.context.i64_type(), "current_catch_link_bits")
                .map_err(|error| error.to_string())?,
            self.builder.build_ptr_to_int(cell, self.context.i64_type(), "retired_catch_link_bits")
                .map_err(|error| error.to_string())?, "catch_owner_link_matches")
            .map_err(|error| error.to_string())?;
        self.builder.build_conditional_branch(matches, found, advance)
            .map_err(|error| error.to_string())?;
        self.builder.position_at_end(found);
        self.builder.build_store(slot, previous).map_err(|error| error.to_string())?;
        self.builder.build_unconditional_branch(done).map_err(|error| error.to_string())?;
        self.builder.position_at_end(advance);
        self.builder.build_unconditional_branch(walk).map_err(|error| error.to_string())?;
        link_slot.add_incoming(&[(&current, advance)]);
        self.builder.position_at_end(done);
        Ok(())
    }

    fn unwind_catch_owner_roots_for_return(&self) -> Result<(), String> {
        for root in self.catch_owner_roots.iter().rev() {
            self.pop_catch_owner_root(root.cell, false)?;
        }
        Ok(())
    }

    fn unwind_catch_owner_roots_for_throw(&self, handler_depth: usize) -> Result<(), String> {
        for root in self.catch_owner_roots.iter().rev() {
            if root.catch_depth >= handler_depth {
                self.pop_catch_owner_root(root.cell, true)?;
            }
        }
        Ok(())
    }

    fn unwind_catch_owner_roots_for_loop(&self, target_index: usize) -> Result<(), String> {
        for root in self.catch_owner_roots.iter().rev() {
            if target_index < root.loop_depth {
                self.pop_catch_owner_root(root.cell, false)?;
            }
        }
        Ok(())
    }

    /// Read the complete active tuple for a branch-local restore. The
    /// registered snapshot cell, not these SSA values, keeps pointers alive
    /// while a Promise destructor or other user callback can run.
    fn current_pending_exception_tuple_values(
        &self,
    ) -> Result<Vec<(inkwell::values::GlobalValue<'ctx>, BasicValueEnum<'ctx>)>, String> {
        let ptr = self.context.ptr_type(AddressSpace::default());
        let slots = [
            (self.pending_exception(), BasicTypeEnum::from(ptr)),
            (self.pending_exception_native_text(), BasicTypeEnum::from(ptr)),
            (self.pending_exception_native_text_owner(), BasicTypeEnum::from(ptr)),
            (self.pending_exception_object(), BasicTypeEnum::from(ptr)),
            (self.pending_exception_json_owner(), BasicTypeEnum::from(ptr)),
            (self.pending_exception_aggregate_errors(), BasicTypeEnum::from(ptr)),
            (self.pending_exception_value(PENDING_EXCEPTION_VALUE_TAG_SYMBOL), BasicTypeEnum::from(self.context.i64_type())),
            (self.pending_exception_value(PENDING_EXCEPTION_F64_SYMBOL), BasicTypeEnum::from(self.context.f64_type())),
            (self.pending_exception_value(PENDING_EXCEPTION_I64_SYMBOL), BasicTypeEnum::from(self.context.i64_type())),
            (self.pending_exception_value(PENDING_EXCEPTION_BOOL_SYMBOL), BasicTypeEnum::from(self.context.bool_type())),
        ];
        slots.into_iter().map(|(slot, ty)| Ok((slot,
            self.builder.build_load(ty, slot.as_pointer_value(), "current_exception_field")
                .map_err(|error| error.to_string())?,
        ))).collect()
    }

    /// Reserve the same registered snapshot for cleanup of a newly created
    /// Promise. Call before allocating that Promise: on OOM the caller may
    /// branch to `on_oom` without leaking a creator/base share. This helper
    /// intentionally leaves the active tuple published for the rejection
    /// producer; only the failure cleanup branch clears it before releasing
    /// a Promise that can run Host destructors.
    fn reserve_pending_exception_cleanup_snapshot(
        &self, on_oom: BasicBlock<'ctx>,
    ) -> Result<(Vec<(inkwell::values::GlobalValue<'ctx>, BasicValueEnum<'ctx>)>, PointerValue<'ctx>), String> {
        let ptr = self.context.ptr_type(AddressSpace::default());
        let slots = [
            (self.pending_exception(), BasicTypeEnum::from(ptr)),
            (self.pending_exception_native_text(), BasicTypeEnum::from(ptr)),
            (self.pending_exception_native_text_owner(), BasicTypeEnum::from(ptr)),
            (self.pending_exception_object(), BasicTypeEnum::from(ptr)),
            (self.pending_exception_json_owner(), BasicTypeEnum::from(ptr)),
            (self.pending_exception_aggregate_errors(), BasicTypeEnum::from(ptr)),
            (self.pending_exception_value(PENDING_EXCEPTION_VALUE_TAG_SYMBOL), BasicTypeEnum::from(self.context.i64_type())),
            (self.pending_exception_value(PENDING_EXCEPTION_F64_SYMBOL), BasicTypeEnum::from(self.context.f64_type())),
            (self.pending_exception_value(PENDING_EXCEPTION_I64_SYMBOL), BasicTypeEnum::from(self.context.i64_type())),
            (self.pending_exception_value(PENDING_EXCEPTION_BOOL_SYMBOL), BasicTypeEnum::from(self.context.bool_type())),
        ];
        let mut saved = Vec::with_capacity(slots.len());
        for (slot, ty) in slots.iter().copied() {
            saved.push((slot, self.builder.build_load(ty, slot.as_pointer_value(), "pre_promise_exception_field")
                .map_err(|error| error.to_string())?));
        }
        let cell = self.builder.build_call(self.module.get_function("thaw_arena_alloc").unwrap(),
            &[self.context.i64_type().const_int(88, false).into(),
              self.context.i64_type().const_int(8, false).into()], "pre_promise_exception_snapshot")
            .map_err(|error| error.to_string())?.try_as_basic_value().basic()
            .ok_or("pre-Promise exception snapshot returned no pointer")?.into_pointer_value();
        let function = self.current_function();
        let snapshot_ready = self.context.append_basic_block(function, "pre_promise_snapshot_ready");
        let missing = self.builder.build_is_null(cell, "pre_promise_snapshot_missing")
            .map_err(|error| error.to_string())?;
        self.builder.build_conditional_branch(missing, on_oom, snapshot_ready)
            .map_err(|error| error.to_string())?;
        self.builder.position_at_end(snapshot_ready);
        let reserved = self.builder.build_call(
            self.module.get_function("thaw_json_reserve_arena_owned_root").unwrap(),
            &[], "pre_promise_exception_reserved_root")
            .map_err(|error| error.to_string())?.try_as_basic_value().basic()
            .ok_or("pre-Promise cleanup root reservation returned no pointer")?.into_pointer_value();
        let ready = self.context.append_basic_block(function, "pre_promise_cleanup_ready");
        let no_reservation = self.builder.build_is_null(reserved, "pre_promise_reservation_missing")
            .map_err(|error| error.to_string())?;
        self.builder.build_conditional_branch(no_reservation, on_oom, ready)
            .map_err(|error| error.to_string())?;
        self.builder.position_at_end(ready);
        let root = self.http_saved_exception_root();
        let previous = self.builder.build_load(ptr, root.as_pointer_value(), "pre_promise_previous_root")
            .map_err(|error| error.to_string())?;
        self.builder.build_store(cell, previous).map_err(|error| error.to_string())?;
        for (offset, index) in [(8, 0), (16, 1), (24, 2), (32, 3), (40, 4), (48, 5)] {
            let field = unsafe { self.builder.build_in_bounds_gep(self.context.i8_type(), cell,
                &[self.context.i64_type().const_int(offset, false)], "pre_promise_saved_pointer")
                .map_err(|error| error.to_string())? };
            self.builder.build_store(field, saved[index].1).map_err(|error| error.to_string())?;
        }
        let reserved_field = unsafe { self.builder.build_in_bounds_gep(self.context.i8_type(), cell,
            &[self.context.i64_type().const_int(56, false)], "pre_promise_reserved_field")
            .map_err(|error| error.to_string())? };
        self.builder.build_store(reserved_field, reserved).map_err(|error| error.to_string())?;
        self.clear_http_snapshot_spare_fields(cell)?;
        // Both packets must exist before a Promise is created. Its release
        // can reenter and publish a second exact exception even when the
        // original registration failed because allocation was exhausted.
        for offset in [64_u64, 72_u64] {
            let packet = self.builder.build_call(
                self.module.get_function("thaw_json_reserve_deferred_exception_packet")
                    .ok_or("missing deferred exception packet reservation")?,
                &[], "pre_promise_exception_packet",
            ).map_err(|error| error.to_string())?.try_as_basic_value().basic()
                .ok_or("pre-Promise packet reservation returned no pointer")?
                .into_pointer_value();
            let packet_ready = self.context.append_basic_block(function, "pre_promise_packet_ready");
            let packet_missing = self.builder.build_is_null(packet, "pre_promise_packet_missing")
                .map_err(|error| error.to_string())?;
            self.builder.build_conditional_branch(packet_missing, on_oom, packet_ready)
                .map_err(|error| error.to_string())?;
            self.builder.position_at_end(packet_ready);
            let field = unsafe { self.builder.build_in_bounds_gep(self.context.i8_type(), cell,
                &[self.context.i64_type().const_int(offset, false)], "pre_promise_packet_field")
                .map_err(|error| error.to_string())? };
            self.builder.build_store(field, packet).map_err(|error| error.to_string())?;
        }
        self.builder.build_store(root.as_pointer_value(), cell).map_err(|error| error.to_string())?;
        Ok((saved, cell))
    }

    /// Copy the current pending tuple into a pre-reserved packet, detaching
    /// explicit NativeStr ownership before any Promise release can reenter.
    /// The registered snapshot keeps the packet reachable until it is either
    /// published back into pending globals or enqueued for later reporting.
    fn isolate_pending_exception_in_reserved_packet(
        &self, snapshot: PointerValue<'ctx>, offset: u64,
    ) -> Result<PointerValue<'ctx>, String> {
        let ptr = self.context.ptr_type(AddressSpace::default());
        let field = unsafe { self.builder.build_in_bounds_gep(self.context.i8_type(), snapshot,
            &[self.context.i64_type().const_int(offset, false)], "reserved_exception_packet_field")
            .map_err(|error| error.to_string())? };
        let packet = self.builder.build_load(ptr, field, "reserved_exception_packet")
            .map_err(|error| error.to_string())?.into_pointer_value();
        self.copy_pending_exception_to_deferred_packet(packet)?;
        self.clear_pending_exception_tuple_for_cleanup()?;
        Ok(packet)
    }

    /// Isolate an outbound callback from the current exception. A callback
    /// can reenter an arena reset, so SSA copies alone are not GC roots.
    /// Link the six pointer fields to a registered process-lifetime root
    /// before clearing their pending globals. The caller pops this link only
    /// after restoring the outer tuple.
    fn snapshot_and_clear_pending_exception_tuple(
        &self,
        output: PointerValue<'ctx>,
    ) -> Result<(Vec<(inkwell::values::GlobalValue<'ctx>, BasicValueEnum<'ctx>)>, PointerValue<'ctx>), String> {
        let ptr = self.context.ptr_type(AddressSpace::default());
        let slots = [
            (self.pending_exception(), BasicTypeEnum::from(ptr)),
            (self.pending_exception_native_text(), BasicTypeEnum::from(ptr)),
            (self.pending_exception_native_text_owner(), BasicTypeEnum::from(ptr)),
            (self.pending_exception_object(), BasicTypeEnum::from(ptr)),
            (self.pending_exception_json_owner(), BasicTypeEnum::from(ptr)),
            (self.pending_exception_aggregate_errors(), BasicTypeEnum::from(ptr)),
            (self.pending_exception_value(PENDING_EXCEPTION_VALUE_TAG_SYMBOL), BasicTypeEnum::from(self.context.i64_type())),
            (self.pending_exception_value(PENDING_EXCEPTION_F64_SYMBOL), BasicTypeEnum::from(self.context.f64_type())),
            (self.pending_exception_value(PENDING_EXCEPTION_I64_SYMBOL), BasicTypeEnum::from(self.context.i64_type())),
            (self.pending_exception_value(PENDING_EXCEPTION_BOOL_SYMBOL), BasicTypeEnum::from(self.context.bool_type())),
        ];
        let mut saved = Vec::with_capacity(slots.len());
        for (slot, ty) in slots.iter().copied() {
            let value = self.builder.build_load(ty, slot.as_pointer_value(), "saved_exception_field")
                .map_err(|error| error.to_string())?;
            saved.push((slot, value));
        }
        let cell = self.builder.build_call(self.module.get_function("thaw_arena_alloc").unwrap(),
            &[self.context.i64_type().const_int(96, false).into(),
              self.context.i64_type().const_int(8, false).into()], "http_saved_exception_cell")
            .map_err(|error| error.to_string())?.try_as_basic_value().basic()
            .ok_or("HTTP exception root allocation returned no value")?.into_pointer_value();
        let function = self.current_function();
        let oom = self.context.append_basic_block(function, "http_exception_snapshot_oom");
        let ready = self.context.append_basic_block(function, "http_exception_snapshot_ready");
        let missing = self.builder.build_is_null(cell, "http_exception_snapshot_missing")
            .map_err(|error| error.to_string())?;
        self.builder.build_conditional_branch(missing, oom, ready)
            .map_err(|error| error.to_string())?;
        self.builder.position_at_end(oom);
        self.builder.build_store(output, ptr.const_null()).map_err(|error| error.to_string())?;
        let ownership = unsafe { self.builder.build_in_bounds_gep(self.context.i8_type(), output,
            &[self.context.i64_type().const_int(8, false)], "http_oom_ownership_slot")
            .map_err(|error| error.to_string())? };
        self.builder.build_store(ownership, self.context.i8_type().const_int(3, false))
            .map_err(|error| error.to_string())?;
        self.builder.build_return(None).map_err(|error| error.to_string())?;
        self.builder.position_at_end(ready);

        let reserved = self.builder.build_call(
            self.module.get_function("thaw_json_reserve_arena_owned_root").unwrap(),
            &[], "http_discard_reserved_root")
            .map_err(|error| error.to_string())?.try_as_basic_value().basic()
            .ok_or("HTTP discard root reservation returned no pointer")?.into_pointer_value();
        let reserved_ready = self.context.append_basic_block(function, "http_discard_root_reserved");
        let no_reservation = self.builder.build_is_null(reserved, "http_discard_reservation_missing")
            .map_err(|error| error.to_string())?;
        self.builder.build_conditional_branch(no_reservation, oom, reserved_ready)
            .map_err(|error| error.to_string())?;
        self.builder.position_at_end(reserved_ready);

        let root = self.http_saved_exception_root();
        let previous = self.builder.build_load(ptr, root.as_pointer_value(), "http_previous_snapshot")
            .map_err(|error| error.to_string())?;
        self.builder.build_store(cell, previous).map_err(|error| error.to_string())?;
        for (offset, index) in [(8, 0), (16, 1), (24, 2), (32, 3), (40, 4), (48, 5)] {
            let field = unsafe { self.builder.build_in_bounds_gep(self.context.i8_type(), cell,
                &[self.context.i64_type().const_int(offset, false)], "http_saved_exception_pointer")
                .map_err(|error| error.to_string())? };
            self.builder.build_store(field, saved[index].1).map_err(|error| error.to_string())?;
        }
        let reserved_field = unsafe { self.builder.build_in_bounds_gep(self.context.i8_type(), cell,
            &[self.context.i64_type().const_int(56, false)], "http_discard_reserved_field")
            .map_err(|error| error.to_string())? };
        self.builder.build_store(reserved_field, reserved).map_err(|error| error.to_string())?;
        self.clear_http_snapshot_spare_fields(cell)?;
        let terminal_field = unsafe { self.builder.build_in_bounds_gep(self.context.i8_type(), cell,
            &[self.context.i64_type().const_int(88, false)], "http_terminal_spare_field")
            .map_err(|error| error.to_string())? };
        self.builder.build_store(terminal_field, ptr.const_null()).map_err(|error| error.to_string())?;
        self.builder.build_store(root.as_pointer_value(), cell).map_err(|error| error.to_string())?;
        for (slot, ty) in slots.iter().copied() {
            self.builder.build_store(slot.as_pointer_value(), ty.const_zero())
                .map_err(|error| error.to_string())?;
        }
        Ok((saved, cell))
    }

    fn restore_pending_exception_tuple(
        &self,
        saved: &[(inkwell::values::GlobalValue<'ctx>, BasicValueEnum<'ctx>)],
        cell: PointerValue<'ctx>,
    ) -> Result<(), String> {
        for (slot, value) in saved {
            self.builder.build_store(slot.as_pointer_value(), *value)
                .map_err(|error| error.to_string())?;
        }
        let previous = self.builder.build_load(self.context.ptr_type(AddressSpace::default()), cell,
            "http_previous_exception_snapshot").map_err(|error| error.to_string())?;
        let root = self.http_saved_exception_root();
        let ptr = self.context.ptr_type(AddressSpace::default());
        let head = self.builder.build_load(ptr, root.as_pointer_value(), "http_current_exception_snapshot")
            .map_err(|error| error.to_string())?.into_pointer_value();
        let is_head = self.builder.build_int_compare(IntPredicate::EQ,
            self.builder.build_ptr_to_int(head, self.context.i64_type(), "http_snapshot_head_bits")
                .map_err(|error| error.to_string())?,
            self.builder.build_ptr_to_int(cell, self.context.i64_type(), "http_snapshot_cell_bits")
                .map_err(|error| error.to_string())?, "http_snapshot_still_head")
            .map_err(|error| error.to_string())?;
        let function = self.current_function();
        let pop = self.context.append_basic_block(function, "pop_http_exception_snapshot");
        let done = self.context.append_basic_block(function, "http_exception_snapshot_restored");
        self.builder.build_conditional_branch(is_head, pop, done)
            .map_err(|error| error.to_string())?;
        self.builder.position_at_end(pop);
        self.builder.build_store(root.as_pointer_value(), previous)
            .map_err(|error| error.to_string())?;
        self.builder.build_unconditional_branch(done).map_err(|error| error.to_string())?;
        self.builder.position_at_end(done);
        Ok(())
    }

    /// Clear a packet only while its original pointer fields remain in a
    /// registered snapshot cell. The caller may then run a Promise release
    /// that invokes Host destructors without exposing stale pending ownership.
    fn clear_pending_exception_tuple_for_cleanup(&self) -> Result<(), String> {
        let ptr = self.context.ptr_type(AddressSpace::default());
        for (slot, ty) in [
            (self.pending_exception(), BasicTypeEnum::from(ptr)),
            (self.pending_exception_native_text(), BasicTypeEnum::from(ptr)),
            (self.pending_exception_native_text_owner(), BasicTypeEnum::from(ptr)),
            (self.pending_exception_object(), BasicTypeEnum::from(ptr)),
            (self.pending_exception_json_owner(), BasicTypeEnum::from(ptr)),
            (self.pending_exception_aggregate_errors(), BasicTypeEnum::from(ptr)),
            (self.pending_exception_value(PENDING_EXCEPTION_VALUE_TAG_SYMBOL), BasicTypeEnum::from(self.context.i64_type())),
            (self.pending_exception_value(PENDING_EXCEPTION_F64_SYMBOL), BasicTypeEnum::from(self.context.f64_type())),
            (self.pending_exception_value(PENDING_EXCEPTION_I64_SYMBOL), BasicTypeEnum::from(self.context.i64_type())),
            (self.pending_exception_value(PENDING_EXCEPTION_BOOL_SYMBOL), BasicTypeEnum::from(self.context.bool_type())),
        ] {
            self.builder.build_store(slot.as_pointer_value(), ty.const_zero())
                .map_err(|error| error.to_string())?;
        }
        Ok(())
    }

    /// After status one, the Promise owns an independent Json share. Queue
    /// the producer's original Box in the token reserved before the Promise
    /// was created; its Host leases are released at the bounded HTTP cleanup
    /// boundary, never inside this callback stack.
    fn defer_pending_owned_json_after_settlement(
        &self, snapshot: PointerValue<'ctx>,
    ) -> Result<(), String> {
        let ptr = self.context.ptr_type(AddressSpace::default());
        let owner_slot = self.pending_exception_json_owner().as_pointer_value();
        let owner = self.builder.build_load(ptr, owner_slot, "settled_json_original_owner")
            .map_err(|error| error.to_string())?.into_pointer_value();
        let field = unsafe { self.builder.build_in_bounds_gep(self.context.i8_type(), snapshot,
            &[self.context.i64_type().const_int(56, false)], "settled_json_reserved_field")
            .map_err(|error| error.to_string())? };
        let reserved = self.builder.build_load(ptr, field, "settled_json_reserved_cleanup")
            .map_err(|error| error.to_string())?.into_pointer_value();
        self.builder.build_store(owner_slot, ptr.const_null()).map_err(|error| error.to_string())?;
        for slot in [self.pending_exception(), self.pending_exception_object()] {
            let value = self.builder.build_load(ptr, slot.as_pointer_value(), "settled_json_alias")
                .map_err(|error| error.to_string())?.into_pointer_value();
            let same = self.builder.build_int_compare(IntPredicate::EQ,
                self.builder.build_ptr_to_int(owner, self.context.i64_type(), "json_owner_bits")
                    .map_err(|error| error.to_string())?,
                self.builder.build_ptr_to_int(value, self.context.i64_type(), "json_alias_bits")
                    .map_err(|error| error.to_string())?, "settled_json_alias_matches")
                .map_err(|error| error.to_string())?;
            let retained = self.builder.build_select(same, ptr.const_null(), value,
                "settled_json_nonowner_alias").map_err(|error| error.to_string())?;
            self.builder.build_store(slot.as_pointer_value(), retained)
                .map_err(|error| error.to_string())?;
        }
        self.builder.build_call(self.module.get_function("thaw_json_enqueue_reserved_cleanup").unwrap(),
            &[owner.into(), reserved.into()], "defer_settled_json_original")
            .map_err(|error| error.to_string())?;
        Ok(())
    }

    /// Copy the complete active exception into a preallocated arena node.
    /// Its caller must keep the node reachable from the registered HTTP
    /// snapshot until it either enqueues the node or finishes reporting it.
    /// The pending NativeStr owner moves into byte 88; subsequent pending
    /// operations borrow the packet's text until that packet is retired.
    fn copy_pending_exception_to_deferred_packet(
        &self, packet: PointerValue<'ctx>,
    ) -> Result<(), String> {
        let ptr = self.context.ptr_type(AddressSpace::default());
        for (offset, slot, ty) in [
            (8, self.pending_exception(), BasicTypeEnum::from(ptr)),
            (16, self.pending_exception_native_text(), BasicTypeEnum::from(ptr)),
            (24, self.pending_exception_object(), BasicTypeEnum::from(ptr)),
            (32, self.pending_exception_json_owner(), BasicTypeEnum::from(ptr)),
            (40, self.pending_exception_aggregate_errors(), BasicTypeEnum::from(ptr)),
            (48, self.pending_exception_value(PENDING_EXCEPTION_VALUE_TAG_SYMBOL), BasicTypeEnum::from(self.context.i64_type())),
            (56, self.pending_exception_value(PENDING_EXCEPTION_F64_SYMBOL), BasicTypeEnum::from(self.context.f64_type())),
            (64, self.pending_exception_value(PENDING_EXCEPTION_I64_SYMBOL), BasicTypeEnum::from(self.context.i64_type())),
            (72, self.pending_exception_value(PENDING_EXCEPTION_BOOL_SYMBOL), BasicTypeEnum::from(self.context.bool_type())),
            (88, self.pending_exception_native_text_owner(), BasicTypeEnum::from(ptr)),
        ] {
            let value = self.builder.build_load(ty, slot.as_pointer_value(), "deferred_packet_field")
                .map_err(|error| error.to_string())?;
            let field = unsafe { self.builder.build_in_bounds_gep(self.context.i8_type(), packet,
                &[self.context.i64_type().const_int(offset, false)], "deferred_packet_slot")
                .map_err(|error| error.to_string())? };
            self.builder.build_store(field, value).map_err(|error| error.to_string())?;
        }
        // This is a transfer, not a second owner. Graph conversion and
        // listener dispatch can publish a replacement exception; they must
        // not free the original NativeStr still held by this packet.
        self.builder.build_store(self.pending_exception_native_text_owner().as_pointer_value(),
            ptr.const_null()).map_err(|error| error.to_string())?;
        Ok(())
    }

    fn publish_deferred_exception_packet(
        &self, packet: PointerValue<'ctx>,
    ) -> Result<(), String> {
        let ptr = self.context.ptr_type(AddressSpace::default());
        for (offset, slot, ty) in [
            (8, self.pending_exception(), BasicTypeEnum::from(ptr)),
            (16, self.pending_exception_native_text(), BasicTypeEnum::from(ptr)),
            (24, self.pending_exception_object(), BasicTypeEnum::from(ptr)),
            (32, self.pending_exception_json_owner(), BasicTypeEnum::from(ptr)),
            (40, self.pending_exception_aggregate_errors(), BasicTypeEnum::from(ptr)),
            (48, self.pending_exception_value(PENDING_EXCEPTION_VALUE_TAG_SYMBOL), BasicTypeEnum::from(self.context.i64_type())),
            (56, self.pending_exception_value(PENDING_EXCEPTION_F64_SYMBOL), BasicTypeEnum::from(self.context.f64_type())),
            (64, self.pending_exception_value(PENDING_EXCEPTION_I64_SYMBOL), BasicTypeEnum::from(self.context.i64_type())),
            (72, self.pending_exception_value(PENDING_EXCEPTION_BOOL_SYMBOL), BasicTypeEnum::from(self.context.bool_type())),
        ] {
            let field = unsafe { self.builder.build_in_bounds_gep(self.context.i8_type(), packet,
                &[self.context.i64_type().const_int(offset, false)], "restore_deferred_packet_slot")
                .map_err(|error| error.to_string())? };
            let value = self.builder.build_load(ty, field, "restore_deferred_packet_field")
                .map_err(|error| error.to_string())?;
            self.builder.build_store(slot.as_pointer_value(), value)
                .map_err(|error| error.to_string())?;
        }
        // The popped packet leaves the queue after this call. Transfer its
        // string owner to the active tuple; the report path immediately
        // transfers it again to its pre-reserved causal packet before any
        // graph conversion or user listener can replace pending state.
        let owner_field = unsafe { self.builder.build_in_bounds_gep(self.context.i8_type(), packet,
            &[self.context.i64_type().const_int(88, false)], "published_packet_text_owner")
            .map_err(|error| error.to_string())? };
        let owner = self.builder.build_load(ptr, owner_field, "published_native_text_owner")
            .map_err(|error| error.to_string())?;
        self.builder.build_store(owner_field, ptr.const_null())
            .map_err(|error| error.to_string())?;
        self.builder.build_store(self.pending_exception_native_text_owner().as_pointer_value(), owner)
            .map_err(|error| error.to_string())?;
        Ok(())
    }

    fn enqueue_reported_exception_owner(
        &self, packet: PointerValue<'ctx>, snapshot: PointerValue<'ctx>,
    ) -> Result<(), String> {
        let ptr = self.context.ptr_type(AddressSpace::default());
        let owner_field = unsafe { self.builder.build_in_bounds_gep(self.context.i8_type(), packet,
            &[self.context.i64_type().const_int(32, false)], "reported_exception_owner_field")
            .map_err(|error| error.to_string())? };
        let owner = self.builder.build_load(ptr, owner_field, "reported_exception_owner")
            .map_err(|error| error.to_string())?.into_pointer_value();
        let reserved_field = unsafe { self.builder.build_in_bounds_gep(self.context.i8_type(), snapshot,
            &[self.context.i64_type().const_int(56, false)], "reported_exception_reserved_field")
            .map_err(|error| error.to_string())? };
        let reserved = self.builder.build_load(ptr, reserved_field, "reported_exception_reserved")
            .map_err(|error| error.to_string())?.into_pointer_value();
        self.builder.build_call(self.module.get_function("thaw_json_enqueue_reserved_cleanup").unwrap(),
            &[owner.into(), reserved.into()], "retire_reported_exception_owner")
            .map_err(|error| error.to_string())?;
        Ok(())
    }

    /// The packet carries separate owned NativeStr slots for a reporter
    /// infrastructure failure and its original producer. Its visible native
    /// text may also be borrowed; only these owner slots authorize release.
    fn destroy_reporter_owned_native_text(
        &self, packet: PointerValue<'ctx>,
    ) -> Result<(), String> {
        let ptr = self.context.ptr_type(AddressSpace::default());
        let field = unsafe { self.builder.build_in_bounds_gep(self.context.i8_type(), packet,
            &[self.context.i64_type().const_int(80, false)], "reporter_owned_text_field")
            .map_err(|error| error.to_string())? };
        let owned = self.builder.build_load(ptr, field, "reporter_owned_native_text")
            .map_err(|error| error.to_string())?.into_pointer_value();
        let has_owned = self.builder.build_is_not_null(owned, "has_reporter_owned_text")
            .map_err(|error| error.to_string())?;
        let function = self.current_function();
        let release = self.context.append_basic_block(function, "release_reporter_owned_text");
        let done = self.context.append_basic_block(function, "reporter_owned_text_done");
        self.builder.build_conditional_branch(has_owned, release, done)
            .map_err(|error| error.to_string())?;
        self.builder.position_at_end(release);
        self.builder.build_store(field, ptr.const_null()).map_err(|error| error.to_string())?;
        self.builder.build_call(self.module.get_function("thaw_cstring_destroy").unwrap(),
            &[owned.into()], "destroy_reporter_owned_text")
            .map_err(|error| error.to_string())?;
        self.builder.build_unconditional_branch(done).map_err(|error| error.to_string())?;
        self.builder.position_at_end(done);
        let producer_field = unsafe { self.builder.build_in_bounds_gep(self.context.i8_type(), packet,
            &[self.context.i64_type().const_int(88, false)], "producer_owned_text_field")
            .map_err(|error| error.to_string())? };
        let producer = self.builder.build_load(ptr, producer_field, "producer_owned_native_text")
            .map_err(|error| error.to_string())?.into_pointer_value();
        self.builder.build_store(producer_field, ptr.const_null()).map_err(|error| error.to_string())?;
        let present = self.builder.build_is_not_null(producer, "has_producer_owned_text")
            .map_err(|error| error.to_string())?;
        let distinct = self.builder.build_int_compare(IntPredicate::NE,
            self.builder.build_ptr_to_int(producer, self.context.i64_type(), "producer_text_bits")
                .map_err(|error| error.to_string())?,
            self.builder.build_ptr_to_int(owned, self.context.i64_type(), "reporter_text_bits")
                .map_err(|error| error.to_string())?, "producer_text_distinct")
            .map_err(|error| error.to_string())?;
        let release_producer = self.builder.build_and(present, distinct,
            "release_distinct_producer_text").map_err(|error| error.to_string())?;
        let producer_release = self.context.append_basic_block(function, "release_producer_owned_text");
        let finished = self.context.append_basic_block(function, "packet_owned_text_done");
        self.builder.build_conditional_branch(release_producer, producer_release, finished)
            .map_err(|error| error.to_string())?;
        self.builder.position_at_end(producer_release);
        self.builder.build_call(self.module.get_function("thaw_cstring_destroy").unwrap(),
            &[producer.into()], "destroy_producer_owned_text")
            .map_err(|error| error.to_string())?;
        self.builder.build_unconditional_branch(finished).map_err(|error| error.to_string())?;
        self.builder.position_at_end(finished);
        Ok(())
    }

    /// After a reentrant Promise cleanup, hand an explicitly owned new Box
    /// to the token reserved before the Promise was created. This never
    /// destroys that Box in the current callback stack. The caller then
    /// restores its saved original tuple; borrowed Json is untouched.
    fn defer_reentrant_pending_exception_tuple(
        &self, snapshot_cell: PointerValue<'ctx>,
    ) -> Result<(), String> {
        let ptr = self.context.ptr_type(AddressSpace::default());
        let function = self.current_function();
        let pending = self.builder.build_load(ptr,
            self.pending_exception().as_pointer_value(), "promise_cleanup_reentrant_pending")
            .map_err(|error| error.to_string())?.into_pointer_value();
        let has_pending = self.builder.build_is_not_null(pending,
            "promise_cleanup_has_reentrant_exception")
            .map_err(|error| error.to_string())?;
        let packet_path = self.context.append_basic_block(function, "queue_reentrant_exception_packet");
        let orphan_path = self.context.append_basic_block(function, "defer_orphan_cleanup_owner");
        let done = self.context.append_basic_block(function, "reentrant_cleanup_deferred");
        self.builder.build_conditional_branch(has_pending, packet_path, orphan_path)
            .map_err(|error| error.to_string())?;
        self.builder.position_at_end(packet_path);
        let packet = self.isolate_pending_exception_in_reserved_packet(snapshot_cell, 72)?;
        self.builder.build_call(
            self.module.get_function("thaw_json_enqueue_deferred_exception_packet")
                .ok_or("missing exact deferred exception queue")?,
            &[packet.into()], "queue_promise_cleanup_reentrant_exception",
        ).map_err(|error| error.to_string())?;
        self.builder.build_unconditional_branch(done).map_err(|error| error.to_string())?;
        self.builder.position_at_end(orphan_path);
        // A destructor can publish an owner before it publishes a primary
        // exception. Such an orphan is cleanup work, not an exception packet.
        let reserved_field = unsafe { self.builder.build_in_bounds_gep(self.context.i8_type(), snapshot_cell,
            &[self.context.i64_type().const_int(56, false)], "promise_cleanup_reserved_field")
            .map_err(|error| error.to_string())? };
        let reserved = self.builder.build_load(ptr, reserved_field, "promise_cleanup_reserved_root")
            .map_err(|error| error.to_string())?.into_pointer_value();
        let reentrant = self.builder.build_load(ptr,
            self.pending_exception_json_owner().as_pointer_value(), "promise_cleanup_orphan_json_owner")
            .map_err(|error| error.to_string())?.into_pointer_value();
        self.builder.build_call(self.module.get_function("thaw_json_enqueue_reserved_cleanup").unwrap(),
            &[reentrant.into(), reserved.into()], "transfer_orphan_json_to_ledger")
            .map_err(|error| error.to_string())?;
        self.discard_pending_owned_native_text()?;
        self.clear_pending_exception_tuple_for_cleanup()?;
        self.builder.build_unconditional_branch(done).map_err(|error| error.to_string())?;
        self.builder.position_at_end(done);
        Ok(())
    }

    /// Drain at most sixteen deferred Host/NAPI lease boxes per HTTP loop
    /// turn. The callback runs after the previous handler has unwound, but
    /// still in generated code so every reentrant exception channel can be
    /// isolated and restored. A newly produced owned reason goes back to the
    /// queue for the next turn instead of recursively running its destructor.
    fn compile_http_deferred_cleanup_driver(&mut self) -> Result<FunctionValue<'ctx>, String> {
        let parent = self.builder.get_insert_block().ok_or("HTTP cleanup driver has no parent")?;
        let ptr = self.context.ptr_type(AddressSpace::default());
        let function = self.module.add_function("__thaw_http_drain_deferred_json",
            self.context.void_type().fn_type(&[], false), Some(Linkage::Internal));
        let entry = self.context.append_basic_block(function, "entry");
        let check = self.context.append_basic_block(function, "check_cleanup_budget");
        let peek = self.context.append_basic_block(function, "peek_deferred_cleanup");
        let work = self.context.append_basic_block(function, "drain_deferred_cleanup");
        let release = self.context.append_basic_block(function, "release_deferred_json");
        let finish = self.context.append_basic_block(function, "finish_deferred_cleanup_item");
        let done = self.context.append_basic_block(function, "done");
        self.builder.position_at_end(entry);
        let count = self.builder.build_alloca(self.context.i32_type(), "deferred_cleanup_count")
            .map_err(|error| error.to_string())?;
        self.builder.build_store(count, self.context.i32_type().const_zero())
            .map_err(|error| error.to_string())?;
        let output = self.builder.build_alloca(self.context.i128_type(), "deferred_cleanup_oom_output")
            .map_err(|error| error.to_string())?;
        self.builder.build_unconditional_branch(check).map_err(|error| error.to_string())?;
        self.builder.position_at_end(check);
        let used = self.builder.build_load(self.context.i32_type(), count, "deferred_cleanup_used")
            .map_err(|error| error.to_string())?.into_int_value();
        let has_budget = self.builder.build_int_compare(IntPredicate::ULT, used,
            self.context.i32_type().const_int(16, false), "deferred_cleanup_has_budget")
            .map_err(|error| error.to_string())?;
        let fatal = self.builder.build_call(
            self.module.get_function("thaw_http_unhandled_error_pending").unwrap(),
            &[], "deferred_cleanup_fatal_pending",
        ).map_err(|error| error.to_string())?.try_as_basic_value().basic()
            .ok_or("HTTP fatal query returned no status")?.into_int_value();
        let fatal = self.builder.build_int_compare(IntPredicate::NE, fatal,
            fatal.get_type().const_zero(), "deferred_cleanup_must_stop")
            .map_err(|error| error.to_string())?;
        let continue_work = self.builder.build_and(has_budget,
            self.builder.build_not(fatal, "deferred_cleanup_no_fatal")
                .map_err(|error| error.to_string())?, "deferred_cleanup_can_continue")
            .map_err(|error| error.to_string())?;
        self.builder.build_conditional_branch(continue_work, peek, done)
            .map_err(|error| error.to_string())?;
        self.builder.position_at_end(peek);
        let packet_node = self.builder.build_call(
            self.module.get_function("thaw_json_deferred_exception_packet_head").unwrap(),
            &[], "deferred_exception_packet_node")
            .map_err(|error| error.to_string())?.try_as_basic_value().basic()
            .ok_or("deferred exception head returned no pointer")?.into_pointer_value();
        let has_packet = self.builder.build_is_not_null(packet_node, "has_deferred_exception_packet")
            .map_err(|error| error.to_string())?;
        let cleanup_node = self.builder.build_call(
            self.module.get_function("thaw_json_deferred_cleanup_head").unwrap(),
            &[], "deferred_cleanup_node")
            .map_err(|error| error.to_string())?.try_as_basic_value().basic()
            .ok_or("deferred cleanup head returned no pointer")?.into_pointer_value();
        let node = self.builder.build_select(has_packet, packet_node, cleanup_node,
            "deferred_work_node").map_err(|error| error.to_string())?.into_pointer_value();
        let empty = self.builder.build_is_null(node, "deferred_cleanup_empty")
            .map_err(|error| error.to_string())?;
        self.builder.build_conditional_branch(empty, done, work)
            .map_err(|error| error.to_string())?;
        self.builder.position_at_end(work);
        let (saved, snapshot) = self.snapshot_and_clear_pending_exception_tuple(output)?;
        let node_field = unsafe { self.builder.build_in_bounds_gep(self.context.i8_type(), snapshot,
            &[self.context.i64_type().const_int(56, false)], "deferred_snapshot_node_field")
            .map_err(|error| error.to_string())? };
        self.builder.build_store(node_field, node).map_err(|error| error.to_string())?;
        // Both packet nodes must exist before the destructor can reenter.
        // The first retains its exact pending tuple if graph conversion
        // fails; the second can own an exact listener-thrown carrier.
        let original_packet = self.builder.build_call(
            self.module.get_function("thaw_json_reserve_deferred_exception_packet").unwrap(),
            &[], "reserve_original_cleanup_packet")
            .map_err(|error| error.to_string())?.try_as_basic_value().basic()
            .ok_or("cleanup packet reservation returned no pointer")?.into_pointer_value();
        let listener_packet = self.builder.build_call(
            self.module.get_function("thaw_json_reserve_deferred_exception_packet").unwrap(),
            &[], "reserve_listener_cleanup_packet")
            .map_err(|error| error.to_string())?.try_as_basic_value().basic()
            .ok_or("listener packet reservation returned no pointer")?.into_pointer_value();
        let terminal_packet = self.builder.build_call(
            self.module.get_function("thaw_json_reserve_deferred_exception_packet").unwrap(),
            &[], "reserve_terminal_reentry_packet")
            .map_err(|error| error.to_string())?.try_as_basic_value().basic()
            .ok_or("terminal packet reservation returned no pointer")?.into_pointer_value();
        let original_field = unsafe { self.builder.build_in_bounds_gep(self.context.i8_type(), snapshot,
            &[self.context.i64_type().const_int(72, false)], "deferred_original_packet_field")
            .map_err(|error| error.to_string())? };
        let listener_field = unsafe { self.builder.build_in_bounds_gep(self.context.i8_type(), snapshot,
            &[self.context.i64_type().const_int(80, false)], "deferred_listener_packet_field")
            .map_err(|error| error.to_string())? };
        self.builder.build_store(original_field, original_packet).map_err(|error| error.to_string())?;
        self.builder.build_store(listener_field, listener_packet).map_err(|error| error.to_string())?;
        let terminal_field = unsafe { self.builder.build_in_bounds_gep(self.context.i8_type(), snapshot,
            &[self.context.i64_type().const_int(88, false)], "deferred_terminal_packet_field")
            .map_err(|error| error.to_string())? };
        self.builder.build_store(terminal_field, terminal_packet).map_err(|error| error.to_string())?;
        let original_missing = self.builder.build_is_null(original_packet, "original_packet_oom")
            .map_err(|error| error.to_string())?;
        let listener_missing = self.builder.build_is_null(listener_packet, "listener_packet_oom")
            .map_err(|error| error.to_string())?;
        let terminal_missing = self.builder.build_is_null(terminal_packet, "terminal_packet_oom")
            .map_err(|error| error.to_string())?;
        let missing_pair = self.builder.build_or(original_missing, listener_missing,
            "cleanup_packet_allocation_failed").map_err(|error| error.to_string())?;
        let allocation_failed = self.builder.build_or(missing_pair, terminal_missing,
            "cleanup_terminal_packet_allocation_failed").map_err(|error| error.to_string())?;
        let reservations_ready = self.context.append_basic_block(function, "cleanup_packets_reserved");
        let reservations_failed = self.context.append_basic_block(function, "cleanup_packet_oom_restore");
        let load_queued_packet = self.context.append_basic_block(function, "load_queued_exception_packet");
        let load_cleanup_box = self.context.append_basic_block(function, "load_cleanup_box");
        let report = self.context.append_basic_block(function, "report_cleanup_exception_graph");
        let no_report = self.context.append_basic_block(function, "cleanup_without_exception");
        let graph_failed = self.context.append_basic_block(function, "cleanup_graph_encode_failed");
        self.builder.build_conditional_branch(allocation_failed, reservations_failed, reservations_ready)
            .map_err(|error| error.to_string())?;
        self.builder.position_at_end(reservations_failed);
        self.restore_pending_exception_tuple(&saved, snapshot)?;
        self.builder.build_unconditional_branch(done).map_err(|error| error.to_string())?;
        self.builder.position_at_end(reservations_ready);
        self.builder.build_conditional_branch(has_packet, load_queued_packet, load_cleanup_box)
            .map_err(|error| error.to_string())?;
        self.builder.position_at_end(load_queued_packet);
        let queued = self.builder.build_call(
            self.module.get_function("thaw_json_pop_deferred_exception_packet").unwrap(),
            &[node.into()], "pop_deferred_exception_packet")
            .map_err(|error| error.to_string())?.try_as_basic_value().basic()
            .ok_or("deferred exception pop returned no pointer")?.into_pointer_value();
        let queued_valid = self.builder.build_is_not_null(queued, "queued_exception_valid")
            .map_err(|error| error.to_string())?;
        let queued_ready = self.context.append_basic_block(function, "queued_exception_ready");
        self.builder.build_conditional_branch(queued_valid, queued_ready, finish)
            .map_err(|error| error.to_string())?;
        self.builder.position_at_end(queued_ready);
        self.builder.build_store(original_field, queued).map_err(|error| error.to_string())?;
        self.publish_deferred_exception_packet(queued)?;
        self.builder.build_unconditional_branch(report).map_err(|error| error.to_string())?;
        self.builder.position_at_end(load_cleanup_box);
        let owner = self.builder.build_call(
            self.module.get_function("thaw_json_pop_deferred_cleanup").unwrap(),
            &[node.into()], "deferred_cleanup_owned_json")
            .map_err(|error| error.to_string())?.try_as_basic_value().basic()
            .ok_or("deferred cleanup pop returned no value")?.into_pointer_value();
        let owner_field = unsafe { self.builder.build_in_bounds_gep(self.context.i8_type(), snapshot,
            &[self.context.i64_type().const_int(64, false)], "deferred_snapshot_owner_field")
            .map_err(|error| error.to_string())? };
        self.builder.build_store(owner_field, owner).map_err(|error| error.to_string())?;
        let has_owner = self.builder.build_is_not_null(owner, "deferred_cleanup_has_owner")
            .map_err(|error| error.to_string())?;
        self.builder.build_conditional_branch(has_owner, release, finish)
            .map_err(|error| error.to_string())?;
        self.builder.position_at_end(release);
        self.builder.build_call(self.module.get_function("thaw_json_destroy").unwrap(),
            &[owner.into()], "destroy_deferred_cleanup_json")
            .map_err(|error| error.to_string())?;
        self.builder.build_store(owner_field, ptr.const_null()).map_err(|error| error.to_string())?;
        // Preserve the exact reentrant tuple before graph conversion can
        // replace pending state with its own failure. The packet cell is
        // reachable through the outer snapshot's registered root.
        self.copy_pending_exception_to_deferred_packet(original_packet)?;
        let thrown = self.builder.build_load(ptr, self.pending_exception().as_pointer_value(),
            "cleanup_reentrant_pending").map_err(|error| error.to_string())?.into_pointer_value();
        let has_thrown = self.builder.build_is_not_null(thrown, "cleanup_reentrant_threw")
            .map_err(|error| error.to_string())?;
        self.builder.build_conditional_branch(has_thrown, report, no_report)
            .map_err(|error| error.to_string())?;
        self.builder.position_at_end(no_report);
        self.builder.build_unconditional_branch(finish).map_err(|error| error.to_string())?;
        self.builder.position_at_end(report);
        let reported_packet = self.builder.build_load(ptr, original_field,
            "active_deferred_exception_packet").map_err(|error| error.to_string())?.into_pointer_value();
        self.catch_stack.push(graph_failed);
        let graph_packet = self.compile_pending_exception_graph_packet_with_error_owner(
            false, Some(listener_packet));
        self.catch_stack.pop();
        let graph_packet = graph_packet?.into_pointer_value();
        // The graph owns transferred leases. The original pending Box
        // remains rooted in reported_packet until the emitter consumes it.
        self.clear_pending_exception_tuple_for_cleanup()?;
        let emitted = self.builder.build_call(
            self.module.get_function("thaw_js_emit_uncaught_graph_result").unwrap(),
            &[graph_packet.into()], "emit_exact_cleanup_exception")
            .map_err(|error| error.to_string())?.try_as_basic_value().basic()
            .ok_or("exact cleanup reporter returned no value")?.into_struct_value();
        self.builder.build_call(self.module.get_function("thaw_cstring_destroy").unwrap(),
            &[graph_packet.into()], "release_cleanup_graph_packet")
            .map_err(|error| error.to_string())?;
        let report_error = self.builder.build_extract_value(emitted, 3, "cleanup_report_error")
            .map_err(|error| error.to_string())?.into_pointer_value();
        let listener_handle = self.builder.build_extract_value(emitted, 1, "cleanup_listener_handle")
            .map_err(|error| error.to_string())?.into_int_value();
        let outcome_kind = self.builder.build_extract_value(emitted, 2, "cleanup_outcome_kind")
            .map_err(|error| error.to_string())?.into_int_value();
        let report_failed = self.builder.build_is_not_null(report_error, "cleanup_report_failed")
            .map_err(|error| error.to_string())?;
        let reporter_error = self.context.append_basic_block(function, "cleanup_reporter_error");
        let reporter_ok = self.context.append_basic_block(function, "cleanup_reporter_ok");
        self.builder.build_conditional_branch(report_failed, reporter_error, reporter_ok)
            .map_err(|error| error.to_string())?;
        self.builder.position_at_end(reporter_error);
        self.builder.build_store(self.pending_exception().as_pointer_value(), report_error)
            .map_err(|error| error.to_string())?;
        self.mark_pending_native_text(report_error)?;
        self.copy_pending_exception_to_deferred_packet(listener_packet)?;
        self.clear_pending_exception_tuple_for_cleanup()?;
        // The ninth typed channel is borrowed by convention. Carry this
        // reporter-created allocation in its own explicit packet owner slot.
        let reporter_text_field = unsafe { self.builder.build_in_bounds_gep(self.context.i8_type(),
            listener_packet, &[self.context.i64_type().const_int(80, false)],
            "reporter_failure_owned_text_field").map_err(|error| error.to_string())? };
        self.builder.build_store(reporter_text_field, report_error)
            .map_err(|error| error.to_string())?;
        for packet in [reported_packet, listener_packet] {
            self.builder.build_call(self.module.get_function("thaw_json_enqueue_deferred_exception_packet").unwrap(),
                &[packet.into()], "queue_exact_cleanup_packet")
                .map_err(|error| error.to_string())?;
        }
        self.builder.build_call(self.module.get_function("thaw_http_mark_unhandled_error").unwrap(),
            &[], "fatal_cleanup_reporter_failure").map_err(|error| error.to_string())?;
        self.clear_pending_exception_tuple_for_cleanup()?;
        self.builder.build_unconditional_branch(finish).map_err(|error| error.to_string())?;
        self.builder.position_at_end(reporter_ok);
        let terminal_kind = self.builder.build_int_compare(IntPredicate::EQ, outcome_kind,
            self.context.i8_type().const_int(2, false), "cleanup_original_unhandled")
            .map_err(|error| error.to_string())?;
        let terminal_unhandled = self.context.append_basic_block(function, "terminal_unhandled_cleanup_exception");
        let listener_dispatch = self.context.append_basic_block(function, "cleanup_listener_dispatch");
        self.builder.build_conditional_branch(terminal_kind, terminal_unhandled, listener_dispatch)
            .map_err(|error| error.to_string())?;
        self.builder.position_at_end(terminal_unhandled);
        // The retained QuickJS handle carries the exact original through
        // process listeners into the existing fatal exit policy once.
        self.builder.build_call(self.module.get_function("thaw_http_mark_unhandled_error").unwrap(),
            &[], "mark_fatal_cleanup_exception").map_err(|error| error.to_string())?;
        self.builder.build_call(self.module.get_function("thaw_js_report_terminal_handle").unwrap(),
            &[listener_handle.into()], "report_exact_fatal_cleanup_exception")
            .map_err(|error| error.to_string())?;
        self.destroy_reporter_owned_native_text(reported_packet)?;
        self.clear_pending_exception_tuple_for_cleanup()?;
        self.builder.build_unconditional_branch(finish).map_err(|error| error.to_string())?;
        self.builder.position_at_end(listener_dispatch);
        let listener_threw = self.builder.build_int_compare(IntPredicate::NE, listener_handle,
            self.context.i64_type().const_zero(), "cleanup_listener_threw")
            .map_err(|error| error.to_string())?;
        let listener_throw = self.context.append_basic_block(function, "capture_cleanup_listener_throw");
        let listener_done = self.context.append_basic_block(function, "cleanup_listener_done");
        self.builder.build_conditional_branch(listener_threw, listener_throw, listener_done)
            .map_err(|error| error.to_string())?;
        self.builder.position_at_end(listener_throw);
        // A process uncaught listener's own throw is terminal under the
        // existing QuickJS event-loop policy. Its retained exact handle can
        // be formatted and released directly; retrying the listener would
        // report recursively without bound.
        self.builder.build_call(self.module.get_function("thaw_http_mark_unhandled_error").unwrap(),
            &[], "fatal_cleanup_listener_throw").map_err(|error| error.to_string())?;
        self.builder.build_call(self.module.get_function("thaw_js_report_terminal_handle").unwrap(),
            &[listener_handle.into()], "report_exact_cleanup_listener_throw")
            .map_err(|error| error.to_string())?;
        self.destroy_reporter_owned_native_text(reported_packet)?;
        self.clear_pending_exception_tuple_for_cleanup()?;
        self.builder.build_unconditional_branch(finish).map_err(|error| error.to_string())?;
        self.builder.position_at_end(listener_done);
        // Encoder/reporter failure still uses the second pre-reserved packet.
        // No process listener throw is requeued after terminal handoff.
        let listener_pending = self.builder.build_load(ptr, self.pending_exception().as_pointer_value(),
            "cleanup_listener_pending").map_err(|error| error.to_string())?.into_pointer_value();
        let listener_pending_flag = self.builder.build_is_not_null(listener_pending,
            "cleanup_listener_has_pending").map_err(|error| error.to_string())?;
        let queue_listener = self.context.append_basic_block(function, "queue_cleanup_listener_packet");
        let complete_report = self.context.append_basic_block(function, "complete_cleanup_report");
        self.builder.build_conditional_branch(listener_pending_flag, queue_listener, complete_report)
            .map_err(|error| error.to_string())?;
        self.builder.position_at_end(queue_listener);
        self.copy_pending_exception_to_deferred_packet(listener_packet)?;
        self.enqueue_reported_exception_owner(reported_packet, snapshot)?;
        self.destroy_reporter_owned_native_text(reported_packet)?;
        self.builder.build_call(self.module.get_function("thaw_json_enqueue_deferred_exception_packet").unwrap(),
            &[listener_packet.into()], "queue_exact_listener_throw")
            .map_err(|error| error.to_string())?;
        self.builder.build_call(self.module.get_function("thaw_http_mark_unhandled_error").unwrap(),
            &[], "fatal_cleanup_listener_packet")
            .map_err(|error| error.to_string())?;
        self.clear_pending_exception_tuple_for_cleanup()?;
        self.builder.build_unconditional_branch(finish).map_err(|error| error.to_string())?;
        self.builder.position_at_end(complete_report);
        let handled = self.builder.build_extract_value(emitted, 0, "cleanup_report_handled")
            .map_err(|error| error.to_string())?.into_int_value();
        let is_handled = self.builder.build_int_compare(IntPredicate::NE, handled,
            self.context.i64_type().const_zero(), "cleanup_exception_handled")
            .map_err(|error| error.to_string())?;
        let accepted = self.context.append_basic_block(function, "cleanup_report_accepted");
        let unhandled = self.context.append_basic_block(function, "cleanup_report_unhandled");
        self.builder.build_conditional_branch(is_handled, accepted, unhandled)
            .map_err(|error| error.to_string())?;
        self.builder.position_at_end(unhandled);
        // The QuickJS emitter may have already rendered the exact value in
        // its active Ctx if retaining a terminal handle failed (private
        // discriminator 3). Older emitters may also lack discriminator 2.
        // Either way, do not retry a listener or lose the rooted packet.
        self.builder.build_call(self.module.get_function("thaw_http_mark_unhandled_error").unwrap(),
            &[], "fatal_unhandled_cleanup_packet")
            .map_err(|error| error.to_string())?;
        // The emitter has finished its one-shot use of the native text.
        // This popped packet is not requeued on the fatal path, so retire
        // its explicit NativeStr owners before the snapshot is unlinked.
        self.destroy_reporter_owned_native_text(reported_packet)?;
        self.builder.build_unconditional_branch(finish).map_err(|error| error.to_string())?;
        self.builder.position_at_end(accepted);
        self.enqueue_reported_exception_owner(reported_packet, snapshot)?;
        self.destroy_reporter_owned_native_text(reported_packet)?;
        self.builder.build_unconditional_branch(finish).map_err(|error| error.to_string())?;
        self.builder.position_at_end(graph_failed);
        // A graph encoder/capture failure is also an exact typed packet.
        // Keep both the original reason and failure rooted before restoring
        // the caller's outer nine fields. Enqueue the original first and
        // the failure second: the LIFO head is the causal failure, whose
        // next link retains the exact reason it failed to project. The
        // fatal flag prevents another delivery attempt or recursive
        // uncaughtException emission.
        self.copy_pending_exception_to_deferred_packet(listener_packet)?;
        for packet in [reported_packet, listener_packet] {
            self.builder.build_call(self.module.get_function("thaw_json_enqueue_deferred_exception_packet").unwrap(),
                &[packet.into()], "queue_failed_cleanup_graph")
                .map_err(|error| error.to_string())?;
        }
        self.clear_pending_exception_tuple_for_cleanup()?;
        // A tag-7 Json Host already owns the original QuickJS value. The
        // graph encoder can fail while traversing it, but terminal reporting
        // need not traverse: borrow its exact engine handle, acquire one
        // independent retain, and consume that retain at the one-shot sink.
        // The source packet remains arena-rooted until this call returns.
        let tag_field = unsafe { self.builder.build_in_bounds_gep(self.context.i8_type(),
            reported_packet, &[self.context.i64_type().const_int(48, false)],
            "terminal_original_tag_field").map_err(|error| error.to_string())? };
        let original_tag = self.builder.build_load(self.context.i64_type(), tag_field,
            "terminal_original_tag").map_err(|error| error.to_string())?.into_int_value();
        let original_json = unsafe { self.builder.build_in_bounds_gep(self.context.i8_type(),
            reported_packet, &[self.context.i64_type().const_int(24, false)],
            "terminal_original_json_field").map_err(|error| error.to_string())? };
        let original_json = self.builder.build_load(ptr, original_json,
            "terminal_original_json").map_err(|error| error.to_string())?.into_pointer_value();
        let is_json = self.builder.build_int_compare(IntPredicate::EQ, original_tag,
            self.context.i64_type().const_int(7, false), "terminal_original_is_json")
            .map_err(|error| error.to_string())?;
        let has_json = self.builder.build_is_not_null(original_json, "terminal_original_has_json")
            .map_err(|error| error.to_string())?;
        let may_borrow = self.builder.build_and(is_json, has_json,
            "terminal_original_may_borrow_handle").map_err(|error| error.to_string())?;
        let borrow_handle = self.context.append_basic_block(function, "terminal_borrow_original_handle");
        let report_scalar = self.context.append_basic_block(function, "terminal_report_scalar");
        let queue_failed = self.context.append_basic_block(function, "queue_failed_cleanup_graph");
        self.builder.build_conditional_branch(may_borrow, borrow_handle, report_scalar)
            .map_err(|error| error.to_string())?;
        self.builder.position_at_end(borrow_handle);
        let borrowed = self.builder.build_call(
            self.module.get_function("thaw_json_borrowed_handle_id").unwrap(),
            &[original_json.into()], "terminal_original_borrowed_handle")
            .map_err(|error| error.to_string())?.try_as_basic_value().basic()
            .ok_or("borrowed original handle returned no value")?.into_int_value();
        let has_handle = self.builder.build_int_compare(IntPredicate::NE, borrowed,
            borrowed.get_type().const_zero(), "terminal_original_has_handle")
            .map_err(|error| error.to_string())?;
        let retain_handle = self.context.append_basic_block(function, "terminal_retain_original_handle");
        self.builder.build_conditional_branch(has_handle, retain_handle, report_scalar)
            .map_err(|error| error.to_string())?;
        self.builder.position_at_end(retain_handle);
        let retained = self.builder.build_call(
            self.module.get_function("thaw_js_retain_handle").unwrap(),
            &[borrowed.into()], "terminal_original_retain")
            .map_err(|error| error.to_string())?.try_as_basic_value().basic()
            .ok_or("terminal retain returned no status")?.into_int_value();
        let retained = self.builder.build_int_compare(IntPredicate::NE, retained,
            retained.get_type().const_zero(), "terminal_original_retained")
            .map_err(|error| error.to_string())?;
        let report_handle = self.context.append_basic_block(function, "terminal_report_original_handle");
        self.builder.build_conditional_branch(retained, report_handle, report_scalar)
            .map_err(|error| error.to_string())?;
        self.builder.position_at_end(report_handle);
        self.builder.build_call(self.module.get_function("thaw_js_report_terminal_handle").unwrap(),
            &[borrowed.into()], "terminal_report_graph_failed_original")
            .map_err(|error| error.to_string())?;
        self.builder.build_unconditional_branch(queue_failed).map_err(|error| error.to_string())?;
        self.builder.position_at_end(report_scalar);
        let scalar_field = |this: &Self, offset: u64, ty: BasicTypeEnum<'ctx>, name: &str| {
            let field = unsafe { this.builder.build_in_bounds_gep(this.context.i8_type(),
                reported_packet, &[this.context.i64_type().const_int(offset, false)], name)
                .map_err(|error| error.to_string())? };
            this.builder.build_load(ty, field, name).map_err(|error| error.to_string())
        };
        let native_text = scalar_field(self, 16, ptr.into(), "terminal_original_text")?;
        let number = scalar_field(self, 56, self.context.f64_type().into(), "terminal_original_number")?;
        let bigint = scalar_field(self, 64, self.context.i64_type().into(), "terminal_original_bigint")?;
        let boolean = scalar_field(self, 72, self.context.bool_type().into(), "terminal_original_boolean")?;
        let boolean = self.builder.build_int_z_extend(boolean.into_int_value(),
            self.context.i8_type(), "terminal_original_boolean_byte")
            .map_err(|error| error.to_string())?;
        self.builder.build_call(self.module.get_function("thaw_js_report_terminal_scalar").unwrap(),
            &[original_tag.into(), number, bigint, boolean.into(), native_text],
            "terminal_report_graph_failed_scalar")
            .map_err(|error| error.to_string())?;
        self.builder.build_unconditional_branch(queue_failed).map_err(|error| error.to_string())?;
        self.builder.position_at_end(queue_failed);
        let terminal_reentry = self.builder.build_load(ptr,
            self.pending_exception().as_pointer_value(), "terminal_reentry_pending")
            .map_err(|error| error.to_string())?.into_pointer_value();
        let has_terminal_reentry = self.builder.build_is_not_null(terminal_reentry,
            "terminal_reentry_has_reason").map_err(|error| error.to_string())?;
        let queue_terminal = self.context.append_basic_block(function, "queue_terminal_reentry");
        let mark_graph_fatal = self.context.append_basic_block(function, "mark_graph_failure_fatal");
        self.builder.build_conditional_branch(has_terminal_reentry, queue_terminal, mark_graph_fatal)
            .map_err(|error| error.to_string())?;
        self.builder.position_at_end(queue_terminal);
        self.copy_pending_exception_to_deferred_packet(terminal_packet)?;
        self.builder.build_call(self.module.get_function("thaw_json_enqueue_deferred_exception_packet").unwrap(),
            &[terminal_packet.into()], "queue_terminal_reentry_packet")
            .map_err(|error| error.to_string())?;
        self.builder.build_unconditional_branch(mark_graph_fatal).map_err(|error| error.to_string())?;
        self.builder.position_at_end(mark_graph_fatal);
        self.builder.build_call(self.module.get_function("thaw_http_mark_unhandled_error").unwrap(),
            &[], "fatal_cleanup_graph_failure").map_err(|error| error.to_string())?;
        self.clear_pending_exception_tuple_for_cleanup()?;
        self.builder.build_unconditional_branch(finish).map_err(|error| error.to_string())?;
        self.builder.position_at_end(finish);
        self.restore_pending_exception_tuple(&saved, snapshot)?;
        let next = self.builder.build_int_add(used, self.context.i32_type().const_int(1, false),
            "next_deferred_cleanup_count").map_err(|error| error.to_string())?;
        self.builder.build_store(count, next).map_err(|error| error.to_string())?;
        self.builder.build_unconditional_branch(check).map_err(|error| error.to_string())?;
        self.builder.position_at_end(done);
        self.builder.build_return(None).map_err(|error| error.to_string())?;
        self.builder.position_at_end(parent);
        Ok(function)
    }

    /// Call only after the consumer has decided the isolated tuple is no
    /// longer needed. A successful Promise rejection has its own Json share;
    /// a failed one must first select its caller-specific fallback. The
    /// explicit owner slot distinguishes a fresh Box from borrowed Json.
    fn discard_isolated_pending_exception_tuple(&self, snapshot_cell: PointerValue<'ctx>) -> Result<(), String> {
        let ptr = self.context.ptr_type(AddressSpace::default());
        let function = self.current_function();
        let reserved_field = unsafe { self.builder.build_in_bounds_gep(self.context.i8_type(), snapshot_cell,
            &[self.context.i64_type().const_int(56, false)], "isolated_discard_reserved_field")
            .map_err(|error| error.to_string())? };
        let reserved = self.builder.build_load(ptr, reserved_field, "isolated_discard_reserved_root")
            .map_err(|error| error.to_string())?.into_pointer_value();
        let owned = self.builder.build_load(ptr, self.pending_exception_json_owner().as_pointer_value(),
            "isolated_owned_exception_json")
            .map_err(|error| error.to_string())?.into_pointer_value();
        for (slot, ty) in [
            (self.pending_exception(), BasicTypeEnum::from(ptr)),
            (self.pending_exception_native_text(), BasicTypeEnum::from(ptr)),
            (self.pending_exception_native_text_owner(), BasicTypeEnum::from(ptr)),
            (self.pending_exception_object(), BasicTypeEnum::from(ptr)),
            (self.pending_exception_json_owner(), BasicTypeEnum::from(ptr)),
            (self.pending_exception_aggregate_errors(), BasicTypeEnum::from(ptr)),
            (self.pending_exception_value(PENDING_EXCEPTION_VALUE_TAG_SYMBOL), BasicTypeEnum::from(self.context.i64_type())),
            (self.pending_exception_value(PENDING_EXCEPTION_F64_SYMBOL), BasicTypeEnum::from(self.context.f64_type())),
            (self.pending_exception_value(PENDING_EXCEPTION_I64_SYMBOL), BasicTypeEnum::from(self.context.i64_type())),
            (self.pending_exception_value(PENDING_EXCEPTION_BOOL_SYMBOL), BasicTypeEnum::from(self.context.bool_type())),
        ] {
            self.builder.build_store(slot.as_pointer_value(), ty.const_zero())
                .map_err(|error| error.to_string())?;
        }
        let destroy = self.context.append_basic_block(function, "discard_owned_exception_json");
        let done = self.context.append_basic_block(function, "discard_exception_done");
        let has_owner = self.builder.build_is_not_null(owned, "isolated_json_has_owner")
            .map_err(|error| error.to_string())?;
        self.builder.build_conditional_branch(has_owner, destroy, done)
            .map_err(|error| error.to_string())?;
        self.builder.position_at_end(destroy);
        self.builder.build_call(self.module.get_function("thaw_json_destroy").unwrap(),
            &[owned.into()], "destroy_isolated_exception_json")
            .map_err(|error| error.to_string())?;
        self.builder.build_unconditional_branch(done).map_err(|error| error.to_string())?;
        self.builder.position_at_end(done);
        let reentrant = self.builder.build_load(ptr,
            self.pending_exception_json_owner().as_pointer_value(), "isolated_discard_reentrant_owner")
            .map_err(|error| error.to_string())?.into_pointer_value();
        let has_reentrant = self.builder.build_is_not_null(reentrant, "isolated_discard_has_reentrant_owner")
            .map_err(|error| error.to_string())?;
        let defer = self.context.append_basic_block(function, "defer_isolated_discard_owner");
        let clear = self.context.append_basic_block(function, "clear_isolated_discard_packet");
        self.builder.build_conditional_branch(has_reentrant, defer, clear)
            .map_err(|error| error.to_string())?;
        self.builder.position_at_end(defer);
        self.builder.build_call(self.module.get_function("thaw_json_enqueue_reserved_cleanup").unwrap(),
            &[reentrant.into(), reserved.into()], "transfer_isolated_discard_to_ledger")
            .map_err(|error| error.to_string())?;
        self.builder.build_unconditional_branch(clear).map_err(|error| error.to_string())?;
        self.builder.position_at_end(clear);
        for (slot, ty) in [
            (self.pending_exception(), BasicTypeEnum::from(ptr)),
            (self.pending_exception_native_text(), BasicTypeEnum::from(ptr)),
            (self.pending_exception_native_text_owner(), BasicTypeEnum::from(ptr)),
            (self.pending_exception_object(), BasicTypeEnum::from(ptr)),
            (self.pending_exception_json_owner(), BasicTypeEnum::from(ptr)),
            (self.pending_exception_aggregate_errors(), BasicTypeEnum::from(ptr)),
            (self.pending_exception_value(PENDING_EXCEPTION_VALUE_TAG_SYMBOL), BasicTypeEnum::from(self.context.i64_type())),
            (self.pending_exception_value(PENDING_EXCEPTION_F64_SYMBOL), BasicTypeEnum::from(self.context.f64_type())),
            (self.pending_exception_value(PENDING_EXCEPTION_I64_SYMBOL), BasicTypeEnum::from(self.context.i64_type())),
            (self.pending_exception_value(PENDING_EXCEPTION_BOOL_SYMBOL), BasicTypeEnum::from(self.context.bool_type())),
        ] {
            self.builder.build_store(slot.as_pointer_value(), ty.const_zero())
                .map_err(|error| error.to_string())?;
        }
        Ok(())
    }

    fn pending_exception_value(&self, name: &str) -> inkwell::values::GlobalValue<'ctx> {
        self.module
            .get_global(name)
            .expect("typed exception state is declared before code generation")
    }

    fn reject_promise_with_pending_exception(
        &mut self,
        promise: PointerValue<'ctx>,
        error: PointerValue<'ctx>,
        name: &str,
    ) -> Result<(), String> {
        let status = self.reject_promise_with_pending_exception_status(promise, error, name)?;
        // A successful rejection copied trusted NativeStr bytes into the
        // Promise. Status zero must leave the original tuple and its owner
        // untouched until the caller chooses its failure disposition.
        self.discard_pending_owned_native_text_after_settlement(status)
    }

    /// Preserve the runtime's settlement status for callers that transfer a
    /// newly captured JSON reason only after the Promise owns its own share.
    fn reject_promise_with_pending_exception_status(
        &mut self,
        promise: PointerValue<'ctx>,
        error: PointerValue<'ctx>,
        name: &str,
    ) -> Result<IntValue<'ctx>, String> {
        let mut args: Vec<BasicMetadataValueEnum<'ctx>> = vec![promise.into(), error.into()];
        for (symbol, ty) in [
            (
                PENDING_EXCEPTION_VALUE_TAG_SYMBOL,
                self.context.i64_type().into(),
            ),
            (PENDING_EXCEPTION_F64_SYMBOL, self.context.f64_type().into()),
            (PENDING_EXCEPTION_I64_SYMBOL, self.context.i64_type().into()),
            (
                PENDING_EXCEPTION_BOOL_SYMBOL,
                self.context.bool_type().into(),
            ),
            (
                PENDING_EXCEPTION_OBJECT_SYMBOL,
                BasicTypeEnum::from(self.context.ptr_type(AddressSpace::default())),
            ),
        ] {
            let global = if symbol == PENDING_EXCEPTION_OBJECT_SYMBOL {
                self.pending_exception_object()
            } else {
                self.pending_exception_value(symbol)
            };
            args.push(
                self.builder
                    .build_load(ty, global.as_pointer_value(), "promise_exception_value")
                    .map_err(|error| error.to_string())?
                    .into(),
            );
        }
        let native_text = self.builder.build_load(
            self.context.ptr_type(AddressSpace::default()),
            self.pending_exception_native_text().as_pointer_value(),
            "pending_native_text_provenance",
        ).map_err(|error| error.to_string())?;
        args.push(native_text.into());
        let aggregate_errors = self.builder.build_load(
            self.context.ptr_type(AddressSpace::default()),
            self.pending_exception_aggregate_errors().as_pointer_value(),
            "pending_aggregate_errors",
        ).map_err(|error| error.to_string())?;
        let ptr_int = self.context.i64_type();
        let error_bits = self.builder.build_ptr_to_int(error, ptr_int, "pending_error_bits")
            .map_err(|error| error.to_string())?;
        let native_bits = self.builder.build_ptr_to_int(native_text.into_pointer_value(), ptr_int,
            "pending_native_text_bits").map_err(|error| error.to_string())?;
        let matching_text = self.builder.build_int_compare(IntPredicate::EQ, error_bits, native_bits,
            "aggregate_text_matches_error").map_err(|error| error.to_string())?;
        let nonnull_text = self.builder.build_is_not_null(native_text.into_pointer_value(),
            "aggregate_has_trusted_text").map_err(|error| error.to_string())?;
        let trusted = self.builder.build_and(matching_text, nonnull_text, "aggregate_trusted_pair")
            .map_err(|error| error.to_string())?;
        let aggregate_errors = self.builder.build_select(trusted, aggregate_errors.into_pointer_value(),
            self.context.ptr_type(AddressSpace::default()).const_null(), "paired_aggregate_errors")
            .map_err(|error| error.to_string())?;
        args.push(aggregate_errors.into());
        self.builder
            .build_call(
                self.module
                    .get_function("thaw_promise_reject_typed_with_aggregate")
                    .unwrap(),
                &args,
                name,
            )
            .map_err(|error| error.to_string())?
            .try_as_basic_value().basic()
            .ok_or("typed Promise rejection returned no settlement status")
            .map(|value| value.into_int_value())
    }

    fn reject_promise_with_source_exception(
        &mut self,
        promise: PointerValue<'ctx>,
        error: PointerValue<'ctx>,
        source: PointerValue<'ctx>,
        name: &str,
    ) -> Result<(), String> {
        self.builder
            .build_call(
                self.module.get_function("thaw_promise_forward_rejection").unwrap(),
                &[promise.into(), source.into(), error.into()],
                name,
            )
            .map_err(|error| error.to_string())?;
        Ok(())
    }

    fn build_default_return(&self) -> Result<(), String> {
        let function = self.current_function();
        let return_type = function.get_type().get_return_type();
        match return_type {
            None => {
                self.builder.build_return(None).map_err(|e| e.to_string())?;
            }
            Some(ty) => {
                let value = ty.const_zero();
                self.builder
                    .build_return(Some(&value))
                    .map_err(|e| e.to_string())?;
            }
        }
        Ok(())
    }

    /// Checks the module exception slot immediately after a generated Thaw
    /// function call. FFI and runtime calls do not participate in this ABI.
    fn branch_on_pending_exception(&mut self) -> Result<(), String> {
        let function = self.current_function();
        let ptr_ty = self.context.ptr_type(AddressSpace::default());
        let pending = self
            .builder
            .build_load(
                ptr_ty,
                self.pending_exception().as_pointer_value(),
                "pending_exception",
            )
            .map_err(|e| e.to_string())?
            .into_pointer_value();
        let has_exception = self
            .builder
            .build_is_not_null(pending, "has_pending_exception")
            .map_err(|e| e.to_string())?;
        let continue_bb = self.context.append_basic_block(function, "call_ok");

        if let Some(catch_bb) = self.catch_stack.last().copied() {
            let exited = self.catch_owner_roots.iter()
                .any(|root| root.catch_depth >= self.catch_stack.len());
            let failure_bb = if exited {
                self.context.append_basic_block(function, "call_unwind_catch_owner")
            } else { catch_bb };
            self.builder
                .build_conditional_branch(has_exception, failure_bb, continue_bb)
                .map_err(|e| e.to_string())?;
            if exited {
                self.builder.position_at_end(failure_bb);
                self.unwind_catch_owner_roots_for_throw(self.catch_stack.len())?;
                self.builder.build_unconditional_branch(catch_bb)
                    .map_err(|e| e.to_string())?;
            }
        } else {
            let propagate_bb = self
                .context
                .append_basic_block(function, "propagate_exception");
            self.builder
                .build_conditional_branch(has_exception, propagate_bb, continue_bb)
                .map_err(|e| e.to_string())?;
            self.builder.position_at_end(propagate_bb);
            self.unwind_catch_owner_roots_for_throw(0)?;
            if let Some(completion) = self.active_async_completion {
                self.reject_promise_with_pending_exception(
                    completion,
                    pending,
                    "reject_frame_exception",
                )?;
                self.builder
                    .build_store(self.pending_exception().as_pointer_value(), ptr_ty.const_null())
                    .map_err(|e| e.to_string())?;
                self.clear_pending_native_text()?;
                self.builder
                    .build_store(self.pending_exception_object().as_pointer_value(), ptr_ty.const_null())
                    .map_err(|e| e.to_string())?;
                self.builder
                    .build_store(
                        self.pending_exception_value(PENDING_EXCEPTION_VALUE_TAG_SYMBOL).as_pointer_value(),
                        self.context.i64_type().const_zero(),
                    )
                    .map_err(|e| e.to_string())?;
                if function.get_type().get_return_type().is_some() {
                    self.builder.build_return(Some(&completion)).map_err(|e| e.to_string())?;
                } else {
                    self.builder.build_return(None).map_err(|e| e.to_string())?;
                }
            } else {
                self.build_default_return()?;
            }
        }

        self.builder.position_at_end(continue_bb);
        Ok(())
    }
}

include!("hir_codegen/runtime_declarations.rs");

include!("hir_codegen/functions.rs");

include!("hir_codegen/ffi_types.rs");

impl<'ctx> HirCompiler<'ctx> {
    fn compile_function_body(&mut self, func: &HirFunction) -> Result<(), String> {
        if let Some(plan) = self.frame_await_plan(func)? {
            return self.compile_frame_async_function(func, &plan);
        }
        let symbol = Self::llvm_symbol_for(&func.name);
        let function = self
            .module
            .get_function(&symbol)
            .expect("function was declared in the prior pass");

        let entry = self.context.append_basic_block(function, "entry");
        self.builder.position_at_end(entry);

        self.variables.clear();
        self.for_iteration_frame_slots.clear();
        self.catch_native_text.clear();
        self.variable_hir_types.clear();
        self.arena_variables.clear();
        self.catch_stack.clear();
        self.catch_owner_roots.clear();
        self.seed_global_variables();
        for (param_val, hir_param) in function.get_param_iter().zip(func.params.iter()) {
            let ty = self.basic_type(&hir_param.ty)?;
            let slot = self.allocate_variable_cell(ty, &hir_param.name)?;
            self.builder
                .build_store(slot, param_val)
                .map_err(|e| e.to_string())?;
            self.variables.insert(hir_param.name.clone(), (slot, ty));
            self.variable_hir_types
                .insert(hir_param.name.clone(), hir_param.ty.clone());
        }

        let terminated = self.compile_block(&func.body)?;

        if !terminated {
            if func.ret == HirType::Void {
                self.builder.build_return(None).map_err(|e| e.to_string())?;
            } else {
                return Err(format!(
                    "function `{}` does not return a value on all paths",
                    func.name
                ));
            }
        }

        Ok(())
    }
}

include!("hir_codegen/async_frames/planning/plan.rs");
include!("hir_codegen/async_frames/planning/analysis.rs");
include!("hir_codegen/async_frames/planning/extraction.rs");
include!("hir_codegen/async_frames/planning/try.rs");
include!("hir_codegen/async_frames/planning/loops.rs");
include!("hir_codegen/async_frames/planning/branches.rs");
include!("hir_codegen/async_frames/planning/validation.rs");
include!("hir_codegen/async_frames/codegen.rs");

include!("hir_codegen/statements.rs");

include!("hir_codegen/values/expressions.rs");
include!("hir_codegen/values/unions.rs");
include!("hir_codegen/values/closures.rs");

include!("hir_codegen/collections.rs");

include!("hir_codegen/builtins.rs");

include!("hir_codegen/json_values.rs");

include!("hir_codegen/dynamic_host.rs");

include!("hir_codegen/json_bridge.rs");

include!("hir_codegen/operators.rs");

include!("hir_codegen/invocations.rs");

include!("hir_codegen/promises.rs");

include!("hir_codegen/ffi_calls.rs");

impl<'ctx> HirCompiler<'ctx> {
    /// Compiles `args`, calls `function` with them, and extracts the
    /// return value. Used by `compile_call`'s own default (plain named
    /// function) fallthrough -- every other call shape (a dynamic method
    /// call, a typed ambient declaration, an FFI call, ...) has its own
    /// dedicated compilation path instead.
    ///
    /// A `JsValue`-typed argument reaching a `Json`-declared parameter
    /// here needs the same `{"__thaw_js_handle_id__": N}` placeholder
    /// encoding `compile_json_object_set_native`'s own field setter
    /// already applies (see `compile_dynamic_value_placeholder`'s doc
    /// comment) -- `coerce_to_declared` (thaw-hir) passes a `JsValue`
    /// through *unchanged* into a `Json`-declared slot, relying on
    /// exactly this kind of downstream detection to thread the real
    /// handle through instead of a raw, mistyped integer. Previously
    /// only the JSON-args-array-construction call sites did this
    /// (guarded by `compiling_quickjs_dynamic_arguments`); an *ordinary*
    /// function call reaching this path never did, so a real npm
    /// function's own generated Fallback wrapper receiving another such
    /// wrapper's `JsValue`-ish return value as an argument (real
    /// example: uuid's `stringify(parse(id))`, `parse`'s `NonSharedArrayBuffer`
    /// return classifying as `JsValue` and `stringify`'s `Uint8Array`
    /// parameter classifying as `Json`, both via the same "unclassified
    /// npm type" fallback) crashed LLVM's own module verifier outright
    /// ("Call parameter type does not match function signature!") --
    /// passing a raw `i64` where the callee's declared parameter is a
    /// pointer. Uses the *unchecked* placeholder builder (no
    /// `compiling_quickjs_dynamic_arguments` gate) since this path is
    /// never reached for an N-API/native-addon target (those have their
    /// own separate, dedicated call-compilation code, not this one) --
    /// safe to assume a QuickJS-side reviver always exists downstream
    /// wherever this specific mismatch can occur at all.
    fn build_call_with(
        &mut self,
        function: FunctionValue<'ctx>,
        args: &[HirExpr],
        name: &str,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let param_types = function.get_type().get_param_types();
        let compiled_args = args
            .iter()
            .enumerate()
            .map(|(index, arg)| {
                let outer = self.compiling_quickjs_dynamic_arguments;
                // A bare pointer-typed parameter slot (`Json`/`Dictionary`/
                // `Object`/`Array`/`Tuple`) can carry a nested `JsValue`
                // needing this same placeholder mechanism, so those
                // already enable it -- but so can a *struct*-typed slot
                // (`Optional`/`Nullable`/`Nullish`/`Union`, this codegen's
                // own tagged representation for all of them), whose
                // payload is exactly one of those pointer-typed shapes
                // once unwrapped. Real example: joi's `object(schema?:
                // T)` (`schema`'s declared type is `Optional(Json)` once
                // its unresolved generic `T` falls back), called with an
                // object literal that itself nests a `JsValue` field
                // (`Joi.object({ name: Joi.string() })`) -- the argument's
                // own top-level LLVM type is a struct (the `Optional` tag
                // + payload), never a bare pointer, so this flag never
                // used to get enabled for it at all, and the nested field
                // failed outright. Enabling it a little more broadly than
                // strictly necessary here is harmless: it only permits
                // `compile_dynamic_value_placeholder` to run if actually
                // reached, never changes what a value that doesn't need
                // it compiles to.
                if param_types
                    .get(index)
                    .is_some_and(|ty| ty.is_pointer_type() || ty.is_struct_type())
                {
                    self.compiling_quickjs_dynamic_arguments = true;
                }
                let value = self.compile_expr(arg);
                self.compiling_quickjs_dynamic_arguments = outer;
                let value = value?;
                let value = if value.is_int_value()
                    && param_types
                        .get(index)
                        .is_some_and(|ty| ty.is_pointer_type())
                {
                    self.compile_dynamic_value_placeholder_unchecked(value)?
                } else {
                    value
                };
                Ok(BasicMetadataValueEnum::from(value))
            })
            .collect::<Result<Vec<_>, String>>()?;

        let call_site = self
            .builder
            .build_call(function, &compiled_args, "calltmp")
            .map_err(|e| e.to_string())?;

        self.branch_on_pending_exception()?;

        if function.get_type().get_return_type().is_none() {
            Ok(self.context.f64_type().const_zero().into())
        } else {
            call_site
                .try_as_basic_value()
                .basic()
                .ok_or_else(|| format!("`{name}` does not return a value"))
        }
    }
}

include!("hir_codegen/console.rs");

include!("hir_codegen/entrypoints.rs");

impl<'ctx> HirCompiler<'ctx> {
    /// Renders the module as textual LLVM IR (handy for debugging / `thaw
    /// build --emit-llvm`).
    pub fn print_to_string(&self) -> String {
        self.module.print_to_string().to_string()
    }

    /// Emits build metadata outside user source so an artifact inspector can
    /// identify it by ELF section rather than by an arbitrary byte sequence.
    pub fn embed_artifact_metadata(&self, metadata: &[u8]) -> Result<(), String> {
        const SYMBOL: &str = "__thaw_artifact_metadata";
        if self.module.get_global(SYMBOL).is_some() || self.module.get_function(SYMBOL).is_some() {
            return Err("artifact metadata has already been emitted".into());
        }
        let value = self.context.const_string(metadata, true);
        let global = self.module.add_global(value.get_type(), None, SYMBOL);
        global.set_initializer(&value);
        global.set_constant(true);
        global.set_section(Some(".thaw.artifact"));
        Ok(())
    }

    /// Compiles this module to a native object file for the host target.
    pub fn write_object_file(&self, path: &Path) -> Result<(), String> {
        Target::initialize_native(&InitializationConfig::default())?;

        let triple = TargetMachine::get_default_triple();
        let target = Target::from_triple(&triple).map_err(|e| e.to_string())?;

        let cpu = TargetMachine::get_host_cpu_name();
        let features = TargetMachine::get_host_cpu_features();

        let target_machine = target
            .create_target_machine(
                &triple,
                cpu.to_str().unwrap(),
                features.to_str().unwrap(),
                OptimizationLevel::Default,
                RelocMode::Default,
                CodeModel::Default,
            )
            .ok_or("failed to create a target machine for the host triple")?;

        // V2 async functions emit `llvm.coro.*` intrinsics. These passes are
        // what turn that ramp function into resume/destroy functions before
        // instruction selection. They are no-ops for today's synchronous
        // functions, so keeping the pipeline always enabled gives both modes
        // one deterministic object-generation path.
        self.module
            .run_passes(
                "coro-early,coro-split,coro-cleanup",
                &target_machine,
                PassBuilderOptions::create(),
            )
            .map_err(|e| format!("coroutine pass pipeline failed: {e}"))?;

        target_machine
            .write_to_file(&self.module, FileType::Object, path)
            .map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests;
