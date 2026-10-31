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
    AsValueRef, BasicMetadataValueEnum, BasicValue, BasicValueEnum, FloatValue, FunctionValue,
    InstructionOpcode, IntValue, Operand, PointerValue, StructValue,
};
use inkwell::{AddressSpace, FloatPredicate, IntPredicate, OptimizationLevel};

use thaw_hir::{
    BinOp, DynamicBackend, DynamicSignature, FfiAggregateAbi, FfiAggregateLayout,
    FfiBitFieldLayout, FfiCallingConvention, FfiErrorAbi, FfiOwnership, FfiRegisterClass,
    FfiSignature, FfiStringAbi, FfiVariadicAbi, HirExpr, HirFunction, HirInitStep, HirLit,
    HirParam, HirProgram, HirStmt, HirType, NonArrowThisAccess, SourceLoc,
};

#[derive(Clone)]
struct OwnedJsonDiscardCandidate<'ctx> {
    value: BasicValueEnum<'ctx>,
    transferred: PointerValue<'ctx>,
}

#[derive(Clone)]
struct OwnedJsonDiscardFrame<'ctx> {
    function_name: String,
    candidates: Vec<OwnedJsonDiscardCandidate<'ctx>>,
}

#[derive(Clone)]
struct ExplicitFfiField {
    native_index: u32,
    bitfield: Option<FfiBitFieldLayout>,
}

/// The user's `main`, if any, is compiled under this symbol instead of
/// `main` so we can wrap it in a proper `i32 main(void)` C entry point
/// rather than exposing a `void`-returning function as the process entry.
const USER_MAIN_SYMBOL: &str = "thaw_user_main";
/// Section holding every compiled function (the linker brackets it with `__start_`/`__stop_`).
const STACK_FRAME_SECTION: &str = "thaw_user_fns";
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
/// Lowest stack address compiled code may reach (0 = no guard, e.g. off the main thread); set at
/// process entry by `thaw_stack_guard_limit` and checked in every function prologue.
const STACK_LIMIT_SYMBOL: &str = "__thaw_stack_limit";
const LIBRARY_METADATA_SYMBOL: &str = "__thaw_pkg_metadata";
const PENDING_REJECTION_SYMBOL: &str = "__thaw_pending_rejection";
/// The exact rejection text currently being delivered to a Promise callback.
/// This scoped provenance lets structural casts distinguish their own string
/// input from stale typed exception fields left by another throw.
const ACTIVE_REJECTION_TEXT_SYMBOL: &str = "__thaw_active_rejection_text";
/// Parallel, opt-in companion to `PENDING_EXCEPTION_SYMBOL`: null unless the
/// thrown value was a real (Error-family) class instance, in which case it
/// holds that object's own pointer so a catch site can read fields beyond
/// `.message`/`.name` after an explicit `as` cast (see
/// `docs/design/exceptions.md` section 3). Every other reader of the
/// exception state (console.log, N-API, Lambda error reporting,
/// `Promise.reject`) only ever looks at the string channel and stays
/// completely unaware this one exists.
const PENDING_EXCEPTION_OBJECT_SYMBOL: &str = "__thaw_pending_exception_object";
const PENDING_EXCEPTION_NATIVE_SYMBOL: &str = "__thaw_pending_exception_native";
const PENDING_EXCEPTION_AGGREGATE_SYMBOL: &str = "__thaw_pending_exception_aggregate_errors";
const PENDING_EXCEPTION_NATIVE_TEXT_SYMBOL: &str = "__thaw_pending_exception_native_text";
const PENDING_EXCEPTION_VALUE_TAG_SYMBOL: &str = "__thaw_pending_exception_value_tag";
const PENDING_EXCEPTION_F64_SYMBOL: &str = "__thaw_pending_exception_f64";
const PENDING_EXCEPTION_I64_SYMBOL: &str = "__thaw_pending_exception_i64";
const PENDING_EXCEPTION_BOOL_SYMBOL: &str = "__thaw_pending_exception_bool";

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
    left.checked_add(right)
        .filter(|size| *size <= isize::MAX as u64)
        .ok_or_else(|| "arena layout size overflow".to_string())
}

fn checked_storage_mul(left: u64, right: u64) -> Result<u64, String> {
    left.checked_mul(right)
        .filter(|size| *size <= isize::MAX as u64)
        .ok_or_else(|| "arena layout size overflow".to_string())
}

fn checked_array_offset(stride: u64, index: usize) -> Result<u64, String> {
    checked_storage_add(
        ARRAY_HEADER_BYTES,
        checked_storage_mul(stride, index as u64)?,
    )
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
        HirType::F64
        | HirType::I64
        | HirType::BigInt
        | HirType::Str
        | HirType::Symbol
        | HirType::StrLiteral(_)
        | HirType::Json
        | HirType::Dictionary(_)
        | HirType::JsValue
        | HirType::Promise(_)
        | HirType::Array(_)
        | HirType::Bytes
        | HirType::Map(_, _)
        | HirType::WeakMap(_, _)
        | HirType::Set(_)
        | HirType::WeakSet(_)
        | HirType::Tuple(_)
        | HirType::Object(_)
        | HirType::Recursive(_)
        | HirType::Function(_, _)
        | HirType::CallableFunction(..)
        | HirType::NativeException
        | HirType::NonArrowReceiver(_)
        | HirType::FunctionWithThis(_, _) => Ok((8, 8)),
        HirType::OverloadedFunction(_, physical) => raw_value_layout(physical),
        HirType::Union(members) if !members.is_empty() => Ok((16, 8)),
        HirType::Optional(payload) | HirType::Nullable(payload) | HirType::Nullish(payload) => {
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
            let bytes = (integer.get_bit_width() as u64).div_ceil(8).max(1);
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
    let legacy = if matches!(
        ty,
        HirType::Optional(_) | HirType::Nullable(_) | HirType::Nullish(_) | HirType::Union(_)
    ) {
        16
    } else {
        8
    };
    Ok(legacy.max(align_storage(raw_value_layout(ty)?.0, 8)?))
}

fn object_field_storage_bytes(ty: &HirType) -> Result<u64, String> {
    arena_storage_bytes(ty)
}

fn array_element_storage_bytes(ty: &HirType) -> Result<u64, String> {
    arena_storage_bytes(ty)
}

fn tuple_element_storage_bytes(elements: &[HirType]) -> Result<u64, String> {
    elements
        .iter()
        .try_fold(ARRAY_ELEM_BYTES, |max_width, element| {
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
    thaw_hir::reorder_class_fields_by_definition(
        fields,
        indices.into_iter().map(|(index, _)| index).collect(),
    )
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

#[derive(Clone)]
struct FreshEventKeyHandle<'ctx> {
    handle: inkwell::values::IntValue<'ctx>,
    active: inkwell::values::PointerValue<'ctx>,
}

#[derive(Clone, Copy)]
struct FreshJsonArgument<'ctx> {
    value: inkwell::values::PointerValue<'ctx>,
    active: inkwell::values::PointerValue<'ctx>,
}

#[derive(Clone, Copy)]
pub(crate) struct NativeHandleLedger<'ctx> {
    buffer: inkwell::values::PointerValue<'ctx>,
    capacity: inkwell::values::PointerValue<'ctx>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum TypedJsonArgumentOwner {
    ProvenWrapper(String),
    DirectQuickJs(String),
    NapiConstructor(String),
}

#[derive(Clone, Copy)]
struct FreshJsonArgumentArray<'ctx> {
    value: PointerValue<'ctx>,
    active: PointerValue<'ctx>,
}

type FreshConstructorArguments<'ctx> = (
    Vec<FreshEventKeyHandle<'ctx>>,
    Vec<FreshJsonArgument<'ctx>>,
    Option<FreshJsonArgumentArray<'ctx>>,
);

struct FreshEventKeyArgumentScope<'ctx> {
    catch_depth: usize,
    function_name: String,
    typed_json_owner: Option<TypedJsonArgumentOwner>,
    handles: Vec<FreshEventKeyHandle<'ctx>>,
    json_values: Vec<FreshJsonArgument<'ctx>>,
    argument_array: Option<FreshJsonArgumentArray<'ctx>>,
    native_handle_ledgers: Vec<NativeHandleLedger<'ctx>>,
}

/// A catch target and the physical Promise slots which remain live there.
/// Keeping them in one private value prevents cleanup boundaries from
/// drifting when a compiler context is isolated or suspended.
#[derive(Clone)]
struct CatchContext<'ctx> {
    scopes: Vec<CatchScope<'ctx>>,
}

#[derive(Clone)]
struct CatchScope<'ctx> {
    target: BasicBlock<'ctx>,
    promise_boundary: HashSet<PointerValue<'ctx>>,
}

#[derive(Clone)]
struct LoopScope<'ctx> {
    continue_target: BasicBlock<'ctx>,
    break_target: BasicBlock<'ctx>,
    promise_boundary: HashSet<PointerValue<'ctx>>,
}

#[derive(Clone)]
struct LoopPromotionScope<'ctx> {
    preheader: BasicBlock<'ctx>,
    variables: HashSet<String>,
    catches: CatchContext<'ctx>,
    stack_promise_slots: HashMap<PointerValue<'ctx>, PointerValue<'ctx>>,
    outer_loops: Vec<LoopScope<'ctx>>,
}

#[derive(Clone)]
struct CodegenScopeContext<'ctx> {
    catches: CatchContext<'ctx>,
    stack_promise_slots: HashMap<PointerValue<'ctx>, PointerValue<'ctx>>,
    loops: Vec<LoopScope<'ctx>>,
    loop_promotions: Vec<LoopPromotionScope<'ctx>>,
    active_async_completion: Option<PointerValue<'ctx>>,
    thaw_js_call_arguments: bool,
}

macro_rules! isolated_codegen_scope {
    ($compiler:expr, $body:block) => {{
        let saved_scope_context = $compiler.take_codegen_scope_context();
        #[allow(clippy::redundant_closure_call)]
        let result: Result<_, String> = (|| $body)();
        $compiler.replace_codegen_scope_context(saved_scope_context);
        result
    }};
}

/// One C-callable export of a shared library build (`thaw lib build`): the compiled function
/// `hir_name` (all parameters and the result `number`) becomes `double c_symbol(double, ...)`.
#[derive(Clone, Debug)]
pub struct LibraryExport {
    pub hir_name: String,
    pub c_symbol: String,
}

pub struct HirCompiler<'ctx> {
    context: &'ctx Context,
    module: Module<'ctx>,
    builder: Builder<'ctx>,
    variables: HashMap<String, (PointerValue<'ctx>, BasicTypeEnum<'ctx>)>,
    /// Only compiler-created catch cells enter this map; source-visible names
    /// (including a matching metadata suffix) never establish provenance.
    catch_native_text: HashMap<
        String,
        (
            PointerValue<'ctx>,
            PointerValue<'ctx>,
            PointerValue<'ctx>,
            PointerValue<'ctx>,
            PointerValue<'ctx>,
        ),
    >,
    /// Catch names of the async function being planned whose binding HIR already
    /// declared as the caught-exception carrier (`caught_exception_carrier_prelude`).
    async_carrier_catch_names: std::cell::RefCell<std::collections::HashSet<String>>,
    /// Declared types of the async function being planned (parameters and
    /// every `let`), so a bare `await promiseVariable` is recognised as a
    /// suspension point before any of its locals have been compiled.
    planning_var_types: std::cell::RefCell<HashMap<String, HirType>>,
    variable_hir_types: HashMap<String, HirType>,
    /// Set while `console.log` converts a typed value for inspection: an `Optional` field with no
    /// value is a property that was never set, so it is omitted instead of printed as `undefined`.
    console_omit_absent_fields: bool,
    /// The native object the recursive-layout encoder is converting: registered with the runtime
    /// as soon as its Json object exists, so a revisit becomes a shared reference (a cycle).
    recursive_register: Option<PointerValue<'ctx>>,
    /// Set by the recursive-layout helper for its own object so the entry router lets it through.
    bypass_recursive_route: bool,
    /// Set while encoding the fields of a class instance for console output.
    encoding_class_instance_fields: bool,
    /// Set while encoding a `JSON.stringify` argument: an Error-family record then shows only the
    /// own enumerable properties Node shows (no `message`, `name` only when it was assigned).
    encoding_stringify_errors: bool,
    arena_variables: HashSet<String>,
    /// Only prepromoted JsValue cells have a pending first-capture retain.
    /// Key by the exact LLVM cell value so branch/function map swaps need no mirror.
    pending_js_capture_claims: HashMap<PointerValue<'ctx>, PointerValue<'ctx>>,
    /// Names some closure in the program captures; `None` disables in-place
    /// string appends (unknown program).
    captured_names: Option<std::collections::BTreeSet<String>>,
    /// Loop-local string variable cell -> its `[owned, len, cap]` append state.
    string_append_states: HashMap<PointerValue<'ctx>, PointerValue<'ctx>>,
    async_frame_cells: HashSet<PointerValue<'ctx>>,
    async_frame_slot_owners: HashMap<PointerValue<'ctx>, PointerValue<'ctx>>,
    stack_promise_slots: HashMap<PointerValue<'ctx>, PointerValue<'ctx>>,
    /// Frame slots holding the current fresh cell of captured `for (let ...)`
    /// bindings. A resume reloads the pointer rather than sharing a value slot.
    for_iteration_frame_slots: HashMap<String, PointerValue<'ctx>>,
    global_variables: HashMap<String, (PointerValue<'ctx>, BasicTypeEnum<'ctx>, HirType)>,
    /// Canonical closure globals for capture-free named function references.
    /// Store the emitted pointer because LLVM globals share a symbol namespace
    /// with functions and may be uniquified when a source function collides.
    function_ref_closures: HashMap<String, Vec<(FunctionType<'ctx>, PointerValue<'ctx>)>>,
    module_exception_roots: Vec<PointerValue<'ctx>>,
    function_return_types: HashMap<String, HirType>,
    proven_typed_quickjs_wrappers: HashSet<String>,
    ffi_signatures: HashMap<String, FfiSignature>,
    /// Stack of enclosing `try` targets. `throw` and a failed nested Thaw
    /// call target the innermost entry; an empty stack propagates by returning
    /// from the current function with the pending exception left intact.
    catch_stack: Vec<CatchScope<'ctx>>,
    loop_scopes: Vec<LoopScope<'ctx>>,
    loop_promotion_scopes: Vec<LoopPromotionScope<'ctx>>,
    /// Functions whose async ABI is a Promise-returning ramp rather than the
    /// legacy synchronous V1 ABI. Seeded to a fixed point before declarations
    /// so callers and callees agree on the LLVM signature.
    frame_async_functions: HashMap<String, HirType>,
    async_lambda_captures: HashMap<String, Vec<HirParam>>,
    promise_returning_functions: HashSet<String>,
    active_async_completion: Option<PointerValue<'ctx>>,
    /// Frame and result type of the async segment being emitted, keyed by its
    /// completion handle. A `return` nested in a plain statement (loop body,
    /// `try`) resolves this completion instead of returning from the segment.
    async_return_frames: HashMap<PointerValue<'ctx>, (PointerValue<'ctx>, HirType)>,
    next_lambda: usize,
    /// Definition sites of the source functions (`HirProgram::fn_locs`), by HIR name.
    fn_locs: HashMap<String, SourceLoc>,
    /// Native class instance layouts (`HirProgram::class_layouts`).
    class_layouts: Vec<HirType>,
    /// Where each stack-visible function (by LLVM symbol) is defined, for `Error.stack`.
    stack_locs: HashMap<String, SourceLoc>,
    uses_napi: bool,
    /// Backend optimisation level for `write_object_file` (`Default` unless `set_codegen_opt`).
    codegen_opt: OptimizationLevel,
    uses_thaw_js: bool,
    uses_intl: bool,
    /// The program is a thaw-js routed build (it falls back to an unrouted one if it needs QuickJS), so
    /// a plain callback coerced to a dynamic value is a thaw-js callback.
    thaw_js_routed: bool,
    /// Compiling the arguments of a thaw-js call (reset inside nested closure bodies): a `JsValue`
    /// converted to `Json` there is a thaw-js handle or callback marker.
    thaw_js_call_arguments: bool,
    uses_quickjs: bool,
    /// Entry is a CommonJS program: `process.nextTick` callbacks run before the promise jobs
    /// the main script queued (an ES module's body runs inside a microtask, so they run after).
    commonjs_entry: bool,
    uses_quickjs_handles: bool,
    uses_thaw_js_handles: bool,
    /// Compile full-layout native projection adapters only in modules whose
    /// completed HIR actually crosses a fixed object into QuickJS.
    projects_native_objects: bool,
    /// Nonprefix structural aliases need tracked owner identities even in
    /// modules that never start QuickJS.
    tracks_physical_object_layouts: bool,
    /// The program calls `Object.freeze`/`seal`/`preventExtensions`, so native-array
    /// mutations consult the object-state table (otherwise no array can be locked).
    tracks_object_integrity: bool,
    /// Non-arrow Json receivers and live Json literals create owned boxes
    /// that may escape through generated globals or captured arena cells.
    tracks_owned_json_roots: bool,
    /// Emit the stack-overflow prologue guard; whole-program (`compile_program`) builds only.
    stack_guard: bool,
    /// True while compiling a QuickJS-backed dynamic call's own JSON-shaped
    /// arguments (set/restored around that one argument-marshaling loop in
    /// `compile_typed_dynamic_call`, so a nested dynamic call compiled
    /// while marshaling an outer one's arguments doesn't see this as
    /// already on). Lets `compile_dynamic_value_placeholder` (json_bridge.rs)
    /// tell that context apart from one where embedding a live JS value
    /// handle has no meaning and should be rejected instead (console.log
    /// formatting, a `Dictionary` literal, an N-API call).
    compiling_quickjs_dynamic_arguments: bool,
    validating_native_argument: bool,
    /// Fresh native scalar handles and projection ledgers owned by active typed
    /// dynamic-call argument scopes.
    fresh_event_key_argument_scopes: Vec<FreshEventKeyArgumentScope<'ctx>>,
    /// Function-local native-to-JS projection state; avoids carrying SSA values
    /// across generated callback/adapter functions.
    live_native_projection: Option<(String, NativeHandleLedger<'ctx>)>,
    /// Fresh JSON Box candidates emitted while compiling one whole expression
    /// statement. Only an exact final SSA result in the same LLVM function is
    /// eligible for discard cleanup.
    owned_json_discard_frames: Vec<OwnedJsonDiscardFrame<'ctx>>,
    library_exports: Vec<LibraryExport>,
    pic: bool,
}

// Prove that a generated typed QuickJS wrapper returns a newly retained
// dynamic result on every normal path. A matching prefix alone does not
// authorize releasing its arguments after the call.
fn is_pure_wrapper_argument(expr: &HirExpr) -> bool {
    match expr {
        HirExpr::Var(_) | HirExpr::Lit(_) => true,
        HirExpr::OptionalValue(value, _) => matches!(value.as_ref(), HirExpr::Var(_)),
        _ => false,
    }
}

fn is_pure_optional_wrapper_guard(expr: &HirExpr) -> bool {
    matches!(
        expr,
        HirExpr::BinOp(
            BinOp::EqEqEq,
            absence,
            literal,
        ) if matches!(absence.as_ref(), HirExpr::OptionalIsNone(value, _) if matches!(value.as_ref(), HirExpr::Var(_)))
            && matches!(literal.as_ref(), HirExpr::Lit(HirLit::Bool(false)))
    )
}

fn returns_direct_quickjs_jsvalue(expr: &HirExpr) -> bool {
    match expr {
        HirExpr::DynamicCall(signature, args) => {
            signature.backend == DynamicBackend::QuickJs
                && signature.ret == HirType::JsValue
                && args.iter().all(is_pure_wrapper_argument)
        }
        HirExpr::Conditional(test, yes, no, HirType::JsValue) => {
            is_pure_optional_wrapper_guard(test)
                && returns_direct_quickjs_jsvalue(yes)
                && returns_direct_quickjs_jsvalue(no)
        }
        _ => false,
    }
}

fn is_proven_typed_quickjs_wrapper(function: &HirFunction) -> bool {
    if !function.name.starts_with("__thaw_typed_wrapper_js_") || function.ret != HirType::JsValue {
        return false;
    }

    fn statements_return_quickjs_jsvalue(statements: &[HirStmt]) -> (bool, bool) {
        let mut falls_through = true;
        for statement in statements {
            if !falls_through {
                break;
            }
            match statement {
                HirStmt::Return(Some(value)) => {
                    if !returns_direct_quickjs_jsvalue(value) {
                        return (false, false);
                    }
                    falls_through = false;
                }
                HirStmt::If(test, then_body, else_body) => {
                    if !is_pure_optional_wrapper_guard(test) {
                        return (false, true);
                    }
                    let (then_valid, then_falls) = statements_return_quickjs_jsvalue(then_body);
                    let (else_valid, else_falls) = statements_return_quickjs_jsvalue(else_body);
                    if !then_valid || !else_valid {
                        return (false, true);
                    }
                    falls_through = then_falls || else_falls;
                }
                // Generated wrappers are straight-line pure forwarders. Even
                // an unused expression or local can escape an input value.
                _ => return (false, true),
            }
        }
        (true, falls_through)
    }

    let (valid, falls_through) = statements_return_quickjs_jsvalue(&function.body);
    valid && !falls_through
}

#[derive(Clone, Copy)]
enum HirPreflightQuery<'a> {
    NamedCall(&'a str),
    NonArrowThisValue,
}

fn hir_contains_preflight_query(program: &HirProgram, query: HirPreflightQuery<'_>) -> bool {
    fn stmts(body: &[HirStmt], query: HirPreflightQuery<'_>) -> bool {
        body.iter().any(|stmt| match stmt {
            HirStmt::Expr(expr) | HirStmt::Throw(expr) | HirStmt::Let(_, _, expr) => {
                expr_has_call(expr, query)
            }
            HirStmt::Return(Some(expr)) => expr_has_call(expr, query),
            HirStmt::If(test, yes, no) => {
                expr_has_call(test, query) || stmts(yes, query) || stmts(no, query)
            }
            HirStmt::While(test, body) => expr_has_call(test, query) || stmts(body, query),
            HirStmt::Finally(body, _) => stmts(body, query),
            HirStmt::Try(body, _, catch, _) => stmts(body, query) || stmts(catch, query),
            HirStmt::Return(None)
            | HirStmt::Break
            | HirStmt::Continue
            | HirStmt::BreakDepth(_)
            | HirStmt::ContinueDepth(_) => false,
        })
    }

    fn expr_has_call(expr: &HirExpr, query: HirPreflightQuery<'_>) -> bool {
        if matches!(query, HirPreflightQuery::NonArrowThisValue)
            && matches!(
                expr,
                HirExpr::NonArrowThis {
                    access: NonArrowThisAccess::Value,
                    ..
                }
            )
        {
            return true;
        }
        match expr {
            HirExpr::Call(callee, args) => {
                if matches!(callee.as_ref(), HirExpr::Var(target)
                    if target == "__thaw_register_native_object_projector")
                {
                    // The deferred factory itself contains a projection, but
                    // never runs in a native-only program. Inspect only the
                    // owner expression at this registration site.
                    return args
                        .first()
                        .is_some_and(|owner| expr_has_call(owner, query));
                }
                matches!((callee.as_ref(), query),
                    (HirExpr::Var(target), HirPreflightQuery::NamedCall(name)) if target == name)
                    || expr_has_call(callee, query)
                    || args.iter().any(|arg| expr_has_call(arg, query))
            }
            HirExpr::BinOp(_, left, right)
            | HirExpr::EvalThen(left, right)
            | HirExpr::UnionMemberIsEqual(left, right, _, _)
            | HirExpr::UnionIsEqual(left, right, _)
            | HirExpr::Index(left, right)
            | HirExpr::TypedIndex(left, right, _)
            | HirExpr::ArraySetLen(left, right, _)
            | HirExpr::DynamicPropAccess(left, right, _, _)
            | HirExpr::JsonKey(left, right)
            | HirExpr::JsonDelete(left, right)
            | HirExpr::JsonIndex(left, right) => {
                expr_has_call(left, query) || expr_has_call(right, query)
            }
            HirExpr::JsonSet(object, key, value, _, _)
            | HirExpr::JsonIndexSet(object, key, value)
            | HirExpr::IndexAssign(object, key, value) => {
                expr_has_call(object, query)
                    || expr_has_call(key, query)
                    || expr_has_call(value, query)
            }
            HirExpr::Conditional(test, yes, no, _) => {
                expr_has_call(test, query) || expr_has_call(yes, query) || expr_has_call(no, query)
            }
            HirExpr::FunctionCallWithThis(callee, receiver, args, _, _)
            | HirExpr::FunctionBindThis(callee, receiver, args, _, _) => {
                expr_has_call(callee, query)
                    || expr_has_call(receiver, query)
                    || args.iter().any(|arg| expr_has_call(arg, query))
            }
            HirExpr::DynamicCall(_, args)
            | HirExpr::FfiCall(_, args)
            | HirExpr::ArrayLit(args)
            | HirExpr::ArrayConcat(args, _)
            | HirExpr::PromiseAll(args, _)
            | HirExpr::PromiseAllTuple(args, _)
            | HirExpr::PromiseRace(args, _)
            | HirExpr::PromiseAny(args, _)
            | HirExpr::PromiseAllSettled(args, _) => {
                args.iter().any(|arg| expr_has_call(arg, query))
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
            | HirExpr::EnumReverseLookup(inner, _)
            | HirExpr::JsonAsNumber(inner)
            | HirExpr::JsonAsString(inner)
            | HirExpr::JsonAsBool(inner)
            | HirExpr::JsonAsNative(inner, _)
            | HirExpr::JsValueAsJson(inner)
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
            | HirExpr::NullishValue(inner, _)
            | HirExpr::UnionInject(inner, _, _)
            | HirExpr::UnionTag(inner, _)
            | HirExpr::UnionValue(inner, _, _)
            | HirExpr::RecursiveClosure(_, _, inner)
            | HirExpr::TypedClosure(_, inner)
            | HirExpr::NonArrowFunction(inner)
            | HirExpr::NonArrowThis { payload: inner, .. }
            | HirExpr::NonArrowReceiverPack { value: inner, .. }
            | HirExpr::PromiseNew(inner, _, _, _)
            | HirExpr::PromiseNewMixed(inner, _, _)
            | HirExpr::PropAccess(inner, _, _)
            | HirExpr::JsonGet(inner, _) => expr_has_call(inner, query),
            HirExpr::Lambda(_, _, _, body) => expr_has_call(body, query),
            HirExpr::PromiseThen(source, callback, _, _, _, _)
            | HirExpr::PromiseFinally(source, callback, _, _) => {
                expr_has_call(source, query) || expr_has_call(callback, query)
            }
            HirExpr::PromiseThenBoth(source, fulfilled, rejected, _, _, _, _) => {
                expr_has_call(source, query)
                    || fulfilled
                        .as_ref()
                        .is_some_and(|callback| expr_has_call(callback, query))
                    || expr_has_call(rejected, query)
            }
            HirExpr::ThrowValue(error, fallback) => {
                expr_has_call(error, query) || expr_has_call(fallback, query)
            }
            HirExpr::Block(body) => stmts(body, query),
            HirExpr::ObjectLit(fields) | HirExpr::JsonObjectLit(fields, _) => {
                fields.iter().any(|(_, value)| expr_has_call(value, query))
            }
            HirExpr::PropAssign(object, _, _, value) => {
                expr_has_call(object, query) || expr_has_call(value, query)
            }
            HirExpr::Lit(_)
            | HirExpr::Var(_)
            | HirExpr::OptionalNone(_)
            | HirExpr::NullableNone(_)
            | HirExpr::NullishNull(_)
            | HirExpr::NullishUndefined(_)
            | HirExpr::EnvVar(_)
            | HirExpr::ObjectAlloc(_)
            | HirExpr::ClassAlloc(_)
            | HirExpr::FunctionRef(..)
            | HirExpr::FunctionRefThis(..)
            | HirExpr::MethodRef(..)
            | HirExpr::PostfixUpdate(_, _) => false,
        }
    }

    program
        .globals
        .iter()
        .any(|global| expr_has_call(&global.init, query))
        || program.initializers.iter().any(|step| match step {
            HirInitStep::StoreGlobal(_, expr) => expr_has_call(expr, query),
            HirInitStep::Statement(stmt) => stmts(std::slice::from_ref(stmt), query),
            HirInitStep::ExecutionBoundary(_) | HirInitStep::ModuleBoundary { .. } => false,
        })
        || program
            .functions
            .iter()
            .any(|function| stmts(&function.body, query))
}

fn hir_contains_named_call(program: &HirProgram, name: &str) -> bool {
    hir_contains_preflight_query(program, HirPreflightQuery::NamedCall(name))
}

fn hir_contains_non_arrow_this_value(program: &HirProgram) -> bool {
    // Reuse the full HIR preflight walk; its special registration handling
    // deliberately skips deferred projector factory bodies.
    hir_contains_preflight_query(program, HirPreflightQuery::NonArrowThisValue)
}

impl<'ctx> HirCompiler<'ctx> {
    pub fn new(context: &'ctx Context, module_name: &str) -> Self {
        Self {
            context,
            module: context.create_module(module_name),
            builder: context.create_builder(),
            variables: HashMap::new(),
            catch_native_text: HashMap::new(),
            async_carrier_catch_names: std::cell::RefCell::new(std::collections::HashSet::new()),
            planning_var_types: std::cell::RefCell::new(HashMap::new()),
            variable_hir_types: HashMap::new(),
            console_omit_absent_fields: false,
            recursive_register: None,
            bypass_recursive_route: false,
            encoding_stringify_errors: false,
            encoding_class_instance_fields: false,
            arena_variables: HashSet::new(),
            pending_js_capture_claims: HashMap::new(),
            captured_names: None,
            string_append_states: HashMap::new(),
            async_frame_cells: HashSet::new(),
            async_frame_slot_owners: HashMap::new(),
            stack_promise_slots: HashMap::new(),
            for_iteration_frame_slots: HashMap::new(),
            global_variables: HashMap::new(),
            function_ref_closures: HashMap::new(),
            module_exception_roots: Vec::new(),
            function_return_types: HashMap::new(),
            proven_typed_quickjs_wrappers: HashSet::new(),
            ffi_signatures: HashMap::new(),
            catch_stack: Vec::new(),
            loop_scopes: Vec::new(),
            loop_promotion_scopes: Vec::new(),
            frame_async_functions: HashMap::new(),
            async_lambda_captures: HashMap::new(),
            promise_returning_functions: HashSet::new(),
            active_async_completion: None,
            async_return_frames: HashMap::new(),
            next_lambda: 0,
            fn_locs: HashMap::new(),
            class_layouts: Vec::new(),
            stack_locs: HashMap::new(),
            uses_napi: false,
            uses_thaw_js: false,
            uses_intl: false,
            thaw_js_routed: false,
            thaw_js_call_arguments: false,
            uses_quickjs: false,
            commonjs_entry: false,
            uses_quickjs_handles: false,
            uses_thaw_js_handles: false,
            projects_native_objects: false,
            tracks_physical_object_layouts: false,
            tracks_object_integrity: false,
            tracks_owned_json_roots: false,
            stack_guard: false,
            compiling_quickjs_dynamic_arguments: false,
            validating_native_argument: false,
            fresh_event_key_argument_scopes: Vec::new(),
            live_native_projection: None,
            owned_json_discard_frames: Vec::new(),
            codegen_opt: OptimizationLevel::Default,
            library_exports: Vec::new(),
            pic: false,
        }
    }

    fn is_proven_typed_quickjs_wrapper(&self, name: &str) -> bool {
        self.proven_typed_quickjs_wrappers.contains(name)
    }

    pub fn compile_program(&mut self, program: &HirProgram) -> Result<(), String> {
        self.stack_guard = true;
        self.fn_locs = program.fn_locs.0.clone();
        self.class_layouts = program.class_layouts.clone();
        self.projects_native_objects =
            hir_contains_named_call(program, "__thaw_build_native_object_wrapper")
                || hir_contains_non_arrow_this_value(program);
        self.tracks_physical_object_layouts =
            hir_contains_named_call(program, "__thaw_register_native_object_layout");
        self.tracks_object_integrity =
            hir_contains_named_call(program, "__thaw_object_set_state_and_return");
        self.declare_runtime_builtins();
        self.declare_exception_state();
        self.declare_globals(program)?;
        self.discover_frame_async_functions(program);
        self.function_return_types = program
            .functions
            .iter()
            .map(|function| (function.name.clone(), function.ret.clone()))
            .collect();
        self.proven_typed_quickjs_wrappers = program
            .functions
            .iter()
            .filter(|function| is_proven_typed_quickjs_wrapper(function))
            .map(|function| function.name.clone())
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
        self.captured_names = Some(
            thaw_hir::closure_captured_names_deep(program)
                .into_iter()
                .map(|name| name.to_string())
                .collect(),
        );
        let quickjs_before_init = self.uses_quickjs;
        self.emit_top_level_init(program)?;
        if !quickjs_before_init
            && self.uses_quickjs
            && std::env::var("THAW_JS_RUNTIME_DEBUG").as_deref() == Ok("1")
        {
            eprintln!("thaw-js routing: the top-level initializer needs QuickJS");
        }
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
            let quickjs_before = self.uses_quickjs;
            self.compile_function_body(func)?;
            if !quickjs_before
                && self.uses_quickjs
                && std::env::var("THAW_JS_RUNTIME_DEBUG").as_deref() == Ok("1")
            {
                eprintln!(
                    "thaw-js routing: `{}` is the first function that needs QuickJS",
                    func.name
                );
            }
        }

        let has_main = self.module.get_function(USER_MAIN_SYMBOL).is_some();
        let handler_hir = program.functions.iter().find(|f| f.name == "handler");

        if !self.library_exports.is_empty() {
            self.emit_library_exports(program)?;
        } else {
            match (has_main, handler_hir) {
                (true, Some(_)) => {
                    return Err("a program can define `main` or `handler`, not both".to_string())
                }
                (true, None) => self.emit_c_main_entry(),
                (false, Some(handler)) => self.emit_lambda_entry(handler)?,
                (false, None) => self.emit_c_main_entry(),
            }
        }

        // A routed build holds no QuickJS values: its `JsValue` handles live in the thaw-js heap, whose
        // retain/release the thaw-js archive provides under its own names.
        if self.thaw_js_routed && self.uses_thaw_js && !self.uses_quickjs {
            for (quickjs, thaw_js) in [
                ("thaw_js_retain_handle", "thaw_tjs_retain_handle"),
                ("thaw_js_release_handle", "thaw_tjs_release_handle"),
                (
                    "thaw_js_get_property_result",
                    "thaw_tjs_get_property_result",
                ),
                (
                    "thaw_js_get_property_json_key_result",
                    "thaw_tjs_get_property_json_key_result",
                ),
                (
                    "thaw_js_get_property_exact_result",
                    "thaw_tjs_get_property_exact_result",
                ),
                (
                    "thaw_js_capture_handle_graph_result",
                    "thaw_tjs_capture_handle_graph_result",
                ),
                (
                    "thaw_js_call_selected_method_graph_result",
                    "thaw_tjs_call_selected_method_graph_result",
                ),
                (
                    "thaw_js_call_selected_method_handle_graph_args_result",
                    "thaw_tjs_call_selected_method_handle_graph_args_result",
                ),
                (
                    "thaw_js_resolve_handle_graph_result",
                    "thaw_tjs_resolve_handle_graph_result",
                ),
                ("thaw_js_handle_to_string", "thaw_tjs_handle_to_string"),
                (
                    "thaw_js_handle_to_console_string",
                    "thaw_tjs_handle_to_console_string",
                ),
                (
                    "thaw_js_handle_is_plain_data",
                    "thaw_tjs_handle_is_plain_data",
                ),
                (
                    "thaw_js_handle_typed_array_probe",
                    "thaw_tjs_handle_typed_array_probe",
                ),
                (
                    "thaw_js_exception_native_token",
                    "thaw_tjs_exception_native_token",
                ),
                ("thaw_js_host_query_result", "thaw_tjs_host_query_result"),
                (
                    "thaw_js_host_date_set_result",
                    "thaw_tjs_host_date_set_result",
                ),
                (
                    "thaw_js_host_enumerate_result",
                    "thaw_tjs_host_enumerate_result",
                ),
                (
                    "thaw_js_set_property_graph_result",
                    "thaw_tjs_set_property_graph_result",
                ),
                (
                    "thaw_js_property_predicate_json_key_result",
                    "thaw_tjs_property_predicate_json_key_result",
                ),
                (
                    "thaw_js_error_from_tagged_handle",
                    "thaw_tjs_error_from_tagged_handle",
                ),
                (
                    "thaw_js_lookup_native_object",
                    "thaw_tjs_lookup_native_object",
                ),
                (
                    "thaw_js_intern_native_object",
                    "thaw_tjs_intern_native_object",
                ),
                ("thaw_js_get_global", "thaw_tjs_get_global"),
                (
                    "thaw_js_release_native_handle_array",
                    "thaw_tjs_release_native_handle_array",
                ),
                (
                    "thaw_js_call_handle_value_graph_result",
                    "thaw_tjs_call_handle_value_graph_result",
                ),
                (
                    "thaw_js_construct_handle_graph_args_result",
                    "thaw_tjs_construct_handle_graph_args_result",
                ),
                (
                    "thaw_js_call_handle_handle_graph_args_result",
                    "thaw_tjs_call_handle_handle_graph_args_result",
                ),
                (
                    "thaw_js_call_handle_graph_result",
                    "thaw_tjs_call_handle_graph_result",
                ),
                (
                    "thaw_js_set_property_result",
                    "thaw_tjs_set_property_result",
                ),
                (
                    "thaw_js_delete_property_result",
                    "thaw_tjs_delete_property_result",
                ),
                (
                    "thaw_js_has_property_result",
                    "thaw_tjs_has_property_result",
                ),
                (
                    "thaw_js_resolve_handle_handle_result",
                    "thaw_tjs_resolve_handle_handle_result",
                ),
                (
                    "thaw_js_call_method_graph_result",
                    "thaw_tjs_call_method_graph_result",
                ),
                (
                    "thaw_js_call_handle_mixed_graph_result",
                    "thaw_tjs_call_handle_mixed_graph_result",
                ),
                (
                    "thaw_js_call_handle_mixed_exact_consuming_result",
                    "thaw_tjs_call_handle_mixed_exact_consuming_result",
                ),
                (
                    "thaw_js_call_handle_mixed_exact_resolved_consuming_result",
                    "thaw_tjs_call_handle_mixed_exact_resolved_consuming_result",
                ),
                (
                    "thaw_js_register_native_callback_graph",
                    "thaw_tjs_register_native_callback_graph",
                ),
                ("thaw_js_run_event_loop", "thaw_tjs_run_event_loop"),
                (
                    "thaw_js_terminal_work_pending",
                    "thaw_tjs_terminal_work_pending",
                ),
            ] {
                if let Some(function) = self.module.get_function(quickjs) {
                    function.as_global_value().set_name(thaw_js);
                    // A weak reference would not pull its definition out of the static thaw-js archive.
                    function.set_linkage(Linkage::External);
                }
            }
        }
        self.hoist_static_allocas();
        self.module
            .verify()
            .map_err(|e| format!("module verification failed:\n{e}"))
    }

    /// An `alloca` outside the entry block grows the native stack on every execution, so one
    /// emitted inside a loop body leaks a slot per iteration until the stack limit trips. Move
    /// every fixed-size one to the entry block, where it is allocated once per call.
    fn hoist_static_allocas(&self) {
        for function in self.module.get_functions() {
            let Some(entry) = function.get_first_basic_block() else {
                continue;
            };
            let Some(anchor) = entry.get_first_instruction() else {
                continue;
            };
            // Ahead of the entry's first instruction, so every use is dominated.
            let builder = self.context.create_builder();
            builder.position_before(&anchor);
            for block in function.get_basic_blocks().into_iter().skip(1) {
                let mut next = block.get_first_instruction();
                while let Some(instruction) = next {
                    next = instruction.get_next_instruction();
                    let fixed = instruction.get_opcode() == InstructionOpcode::Alloca
                        && instruction
                            .get_operand(0)
                            .and_then(|operand| match operand {
                                Operand::Value(value) => Some(value),
                                Operand::Block(_) => None,
                            })
                            .is_some_and(|size| {
                                size.is_int_value() && size.into_int_value().is_const()
                            });
                    if fixed {
                        let name = instruction
                            .get_name()
                            .and_then(|name| name.to_str().ok())
                            .map(str::to_owned);
                        instruction.remove_from_basic_block();
                        builder.insert_instruction(&instruction, name.as_deref());
                    }
                }
            }
        }
    }

    /// Selects CommonJS (`true`) or ES module (`false`, the default) `process.nextTick` ordering
    /// for the process entry point.
    pub fn set_commonjs_entry(&mut self, commonjs: bool) {
        self.commonjs_entry = commonjs;
    }

    /// Selects the backend optimisation level used by `write_object_file`. `None` skips
    /// instruction-selection optimisation (fast compile, slower generated code).
    pub fn set_codegen_opt(&mut self, level: OptimizationLevel) {
        self.codegen_opt = level;
    }

    /// Whether the generated module actually calls the QuickJS fallback host.
    /// Callers can omit that archive when every operation was lowered to the
    /// native/JIT paths.
    pub fn uses_quickjs(&self) -> bool {
        self.uses_quickjs
    }

    /// Whether the generated module calls the N-API addon host.
    /// Whether the program calls into the experimental thaw-js runtime (`DynamicBackend::ThawJs`).
    pub fn set_thaw_js_routed(&mut self, routed: bool) {
        self.thaw_js_routed = routed;
    }

    /// Whether the program calls the native locale runtime (thaw-intl), which the
    /// linker must then be given.
    pub fn uses_intl(&self) -> bool {
        self.uses_intl
    }

    pub fn uses_thaw_js(&self) -> bool {
        self.uses_thaw_js
    }

    pub fn uses_napi(&self) -> bool {
        self.uses_napi
    }

    fn declare_exception_state(&self) {
        let ptr_ty = self.context.ptr_type(AddressSpace::default());
        let stack_limit = self
            .module
            .add_global(self.context.i64_type(), None, STACK_LIMIT_SYMBOL);
        stack_limit.set_linkage(Linkage::Internal);
        stack_limit.set_initializer(&self.context.i64_type().const_zero());
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
        let active_rejection_text =
            self.module
                .add_global(ptr_ty, None, ACTIVE_REJECTION_TEXT_SYMBOL);
        active_rejection_text.set_linkage(Linkage::Internal);
        active_rejection_text.set_initializer(&ptr_ty.const_null());
        let native_text =
            self.module
                .add_global(ptr_ty, None, PENDING_EXCEPTION_NATIVE_TEXT_SYMBOL);
        native_text.set_linkage(Linkage::Internal);
        native_text.set_initializer(&ptr_ty.const_null());
        let pending_object = self
            .module
            .add_global(ptr_ty, None, PENDING_EXCEPTION_OBJECT_SYMBOL);
        pending_object.set_linkage(Linkage::Internal);
        pending_object.set_initializer(&ptr_ty.const_null());
        let pending_native = self
            .module
            .add_global(ptr_ty, None, PENDING_EXCEPTION_NATIVE_SYMBOL);
        pending_native.set_linkage(Linkage::Internal);
        pending_native.set_initializer(&ptr_ty.const_null());
        let pending_aggregate =
            self.module
                .add_global(ptr_ty, None, PENDING_EXCEPTION_AGGREGATE_SYMBOL);
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
        #[allow(clippy::type_complexity)]
        let mut groups: Vec<(usize, bool, bool, Vec<usize>, Vec<HirInitStep>)> = Vec::new();
        for step in &program.initializers {
            if let HirInitStep::ModuleBoundary {
                index,
                eager,
                runtime,
                static_dependencies,
            } = step
            {
                groups.push((
                    *index,
                    *eager,
                    *runtime,
                    static_dependencies.clone(),
                    Vec::new(),
                ));
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
        self.stack_promise_slots.clear();
        self.for_iteration_frame_slots.clear();
        self.catch_native_text.clear();
        self.variable_hir_types.clear();
        self.arena_variables.clear();
        self.clear_catch_context();
        self.loop_scopes.clear();
        self.loop_promotion_scopes.clear();
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
                    let value = self.compile_expr_named(name, expression)?;
                    let (pointer, _, _) = self.global_variables[name];
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
            if self
                .builder
                .get_insert_block()
                .is_some_and(|block| block.get_terminator().is_some())
            {
                break;
            }
            if !eager || !runtime {
                continue;
            }
            let function = self
                .module
                .get_function(&format!("__thaw_lazy_module_{index}"))
                .ok_or_else(|| format!("missing module initializer {index}"))?;
            self.builder
                .build_call(function, &[], "initialize_eager_module")
                .map_err(|error| error.to_string())?;
            let pending = self
                .builder
                .build_load(
                    self.context.ptr_type(AddressSpace::default()),
                    self.pending_exception().as_pointer_value(),
                    "eager_module_exception",
                )
                .map_err(|error| error.to_string())?
                .into_pointer_value();
            let failed = self
                .builder
                .build_is_not_null(pending, "eager_module_failed")
                .map_err(|error| error.to_string())?;
            let next = self.context.append_basic_block(init, "next_eager_module");
            self.builder
                .build_conditional_branch(failed, done, next)
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
        let function = self
            .module
            .get_function(&name)
            .ok_or_else(|| format!("missing generated module initializer {index}"))?;
        let state_ty = self.context.i8_type();
        let state = self
            .module
            .add_global(state_ty, None, &format!("{name}_state"));
        state.set_linkage(Linkage::Internal);
        state.set_initializer(&state_ty.const_zero());
        let exception_slots: [(&str, BasicTypeEnum<'ctx>); 9] = [
            (
                PENDING_EXCEPTION_SYMBOL,
                self.context.ptr_type(AddressSpace::default()).into(),
            ),
            (
                PENDING_EXCEPTION_NATIVE_TEXT_SYMBOL,
                self.context.ptr_type(AddressSpace::default()).into(),
            ),
            (
                PENDING_EXCEPTION_OBJECT_SYMBOL,
                self.context.ptr_type(AddressSpace::default()).into(),
            ),
            (
                PENDING_EXCEPTION_NATIVE_SYMBOL,
                self.context.ptr_type(AddressSpace::default()).into(),
            ),
            (
                PENDING_EXCEPTION_AGGREGATE_SYMBOL,
                self.context.ptr_type(AddressSpace::default()).into(),
            ),
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
        ];
        let mut cached = Vec::new();
        for (symbol, ty) in exception_slots {
            let slot = self
                .module
                .add_global(ty, None, &format!("{name}_{symbol}"));
            slot.set_linkage(Linkage::Internal);
            slot.set_initializer(&ty.const_zero());
            if symbol == PENDING_EXCEPTION_SYMBOL
                || symbol == PENDING_EXCEPTION_OBJECT_SYMBOL
                || symbol == PENDING_EXCEPTION_NATIVE_SYMBOL
                || symbol == PENDING_EXCEPTION_AGGREGATE_SYMBOL
            {
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
        let current = self
            .builder
            .build_load(state_ty, state.as_pointer_value(), "module_state")
            .map_err(|error| error.to_string())?
            .into_int_value();
        let uninitialized = self
            .builder
            .build_int_compare(
                IntPredicate::EQ,
                current,
                state_ty.const_zero(),
                "module_uninitialized",
            )
            .map_err(|error| error.to_string())?;
        self.builder
            .build_conditional_branch(uninitialized, first, prior)
            .map_err(|error| error.to_string())?;
        self.builder.position_at_end(prior);
        let had_failed = self
            .builder
            .build_int_compare(
                IntPredicate::EQ,
                current,
                state_ty.const_int(3, false),
                "module_had_failed",
            )
            .map_err(|error| error.to_string())?;
        self.builder
            .build_conditional_branch(had_failed, restore, done)
            .map_err(|error| error.to_string())?;
        self.builder.position_at_end(restore);
        for (pending, saved, ty) in &cached {
            let value = self
                .builder
                .build_load(*ty, saved.as_pointer_value(), "saved_exception")
                .map_err(|error| error.to_string())?;
            self.builder
                .build_store(pending.as_pointer_value(), value)
                .map_err(|error| error.to_string())?;
        }
        self.builder
            .build_unconditional_branch(done)
            .map_err(|error| error.to_string())?;

        self.builder.position_at_end(first);
        self.builder
            .build_store(state.as_pointer_value(), state_ty.const_int(1, false))
            .map_err(|error| error.to_string())?;
        self.variables.clear();
        self.for_iteration_frame_slots.clear();
        self.catch_native_text.clear();
        self.variable_hir_types.clear();
        self.arena_variables.clear();
        self.stack_promise_slots.clear();
        self.clear_catch_context();
        self.loop_scopes.clear();
        self.loop_promotion_scopes.clear();
        self.seed_global_variables();
        self.push_catch_target(failed);
        let initializer_result = (|| -> Result<(), String> {
            for dependency in dependencies {
                let initializer = self
                    .module
                    .get_function(&format!("__thaw_lazy_module_{dependency}"))
                    .ok_or_else(|| format!("missing static dependency initializer {dependency}"))?;
                self.builder
                    .build_call(initializer, &[], "initialize_static_dependency")
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
                        let value = self.compile_expr_named(name, expression)?;
                        let (pointer, _, _) = self.global_variables[name];
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
            Ok(())
        })();
        self.pop_catch_target();
        initializer_result?;
        if self
            .builder
            .get_insert_block()
            .is_some_and(|block| block.get_terminator().is_none())
        {
            self.builder
                .build_store(state.as_pointer_value(), state_ty.const_int(2, false))
                .map_err(|error| error.to_string())?;
            self.builder
                .build_unconditional_branch(done)
                .map_err(|error| error.to_string())?;
        }
        self.builder.position_at_end(failed);
        for (pending, saved, ty) in &cached {
            let value = self
                .builder
                .build_load(*ty, pending.as_pointer_value(), "failed_exception")
                .map_err(|error| error.to_string())?;
            self.builder
                .build_store(saved.as_pointer_value(), value)
                .map_err(|error| error.to_string())?;
        }
        self.builder
            .build_store(state.as_pointer_value(), state_ty.const_int(3, false))
            .map_err(|error| error.to_string())?;
        self.builder
            .build_unconditional_branch(done)
            .map_err(|error| error.to_string())?;
        self.builder.position_at_end(done);
        self.builder
            .build_return(None)
            .map_err(|error| error.to_string())?;
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
        &self,
        stmt: &HirStmt,
        frame_functions: &std::collections::HashSet<String>,
    ) -> bool {
        match stmt {
            HirStmt::Expr(expr) | HirStmt::Throw(expr) | HirStmt::Let(_, _, expr) => {
                self.expr_awaits_frame_source(expr, frame_functions)
            }
            HirStmt::Return(Some(expr)) => self.expr_awaits_frame_source(expr, frame_functions),
            HirStmt::Return(None) => false,
            HirStmt::If(cond, then_body, else_body) => {
                self.expr_awaits_frame_source(cond, frame_functions)
                    || then_body
                        .iter()
                        .any(|stmt| self.stmt_awaits_frame_source(stmt, frame_functions))
                    || else_body
                        .iter()
                        .any(|stmt| self.stmt_awaits_frame_source(stmt, frame_functions))
            }
            HirStmt::While(cond, body) => {
                self.expr_awaits_frame_source(cond, frame_functions)
                    || body
                        .iter()
                        .any(|stmt| self.stmt_awaits_frame_source(stmt, frame_functions))
            }
            HirStmt::Break
            | HirStmt::Continue
            | HirStmt::BreakDepth(_)
            | HirStmt::ContinueDepth(_) => false,
            HirStmt::Finally(body, _) => body
                .iter()
                .any(|stmt| self.stmt_awaits_frame_source(stmt, frame_functions)),
            HirStmt::Try(body, _, catch_body, _) => body
                .iter()
                .chain(catch_body)
                .any(|stmt| self.stmt_awaits_frame_source(stmt, frame_functions)),
        }
    }

    fn expr_awaits_frame_source(
        &self,
        expr: &HirExpr,
        frame_functions: &std::collections::HashSet<String>,
    ) -> bool {
        match expr {
            HirExpr::AwaitPromise(_, _) => true,
            HirExpr::Await(inner) => {
                // A bare promise-typed variable/field (`await p;` with
                // `p: Promise<void>`) suspends like any other source.
                matches!(inner.as_ref(), HirExpr::Var(_))
                    && matches!(self.expr_hir_type(inner), Some(HirType::Promise(_)))
                    || matches!(
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
                    // A call through a function-typed value whose result is a Promise
                    // (`const sleep = (ms) => new Promise(..)`) suspends like a named source.
                    || matches!(inner.as_ref(), HirExpr::Call(callee, _)
                        if matches!(self.expr_hir_type(callee),
                            Some(HirType::Function(_, ret) | HirType::CallableFunction(_, _, _, ret))
                                if matches!(ret.as_ref(), HirType::Promise(_))))
                    // An argument-binding wrapper `(lambda)(args)` that returns a Promise.
                    || matches!(inner.as_ref(), HirExpr::Call(callee, _)
                        if matches!(callee.as_ref(), HirExpr::Lambda(_, _, HirType::Promise(_), body)
                            if matches!(body.as_ref(), HirExpr::PromiseNew(..))))
                    || self.expr_awaits_frame_source(inner, frame_functions)
            }
            HirExpr::BinOp(_, left, right)
            | HirExpr::EvalThen(left, right)
            | HirExpr::Index(left, right)
            | HirExpr::TypedIndex(left, right, _)
            | HirExpr::DynamicPropAccess(left, right, _, _) => {
                self.expr_awaits_frame_source(left, frame_functions)
                    || self.expr_awaits_frame_source(right, frame_functions)
            }
            HirExpr::Assign(_, value) => self.expr_awaits_frame_source(value, frame_functions),
            HirExpr::Call(callee, args) => {
                self.expr_awaits_frame_source(callee, frame_functions)
                    || args
                        .iter()
                        .any(|arg| self.expr_awaits_frame_source(arg, frame_functions))
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
                .any(|arg| self.expr_awaits_frame_source(arg, frame_functions)),
            HirExpr::IndexAssign(a, b, c) => {
                self.expr_awaits_frame_source(a, frame_functions)
                    || self.expr_awaits_frame_source(b, frame_functions)
                    || self.expr_awaits_frame_source(c, frame_functions)
            }
            HirExpr::JsonSet(object, key, value, _, _)
            | HirExpr::JsonIndexSet(object, key, value) => [object, key, value]
                .iter()
                .any(|value| self.expr_awaits_frame_source(value, frame_functions)),
            HirExpr::ObjectLit(fields) | HirExpr::JsonObjectLit(fields, _) => fields
                .iter()
                .any(|(_, value)| self.expr_awaits_frame_source(value, frame_functions)),
            HirExpr::PropAccess(obj, _, _)
            | HirExpr::ArrayLen(obj)
            | HirExpr::EnumReverseLookup(obj, _)
            | HirExpr::JsonAsNumber(obj)
            | HirExpr::JsonAsString(obj)
            | HirExpr::JsonAsBool(obj)
            | HirExpr::NonArrowThis { payload: obj, .. }
            | HirExpr::NonArrowReceiverPack { value: obj, .. } => {
                self.expr_awaits_frame_source(obj, frame_functions)
            }
            HirExpr::PropAssign(obj, _, _, value)
            | HirExpr::JsonIndex(obj, value)
            | HirExpr::JsonKey(obj, value)
            | HirExpr::JsonDelete(obj, value) => {
                self.expr_awaits_frame_source(obj, frame_functions)
                    || self.expr_awaits_frame_source(value, frame_functions)
            }
            HirExpr::JsonGet(obj, _) => self.expr_awaits_frame_source(obj, frame_functions),
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

    fn active_rejection_text(&self) -> inkwell::values::GlobalValue<'ctx> {
        self.module
            .get_global(ACTIVE_REJECTION_TEXT_SYMBOL)
            .expect("active rejection text is declared with exception state")
    }

    fn clear_pending_native_text(&self) -> Result<(), String> {
        let null = self.context.ptr_type(AddressSpace::default()).const_null();
        self.builder
            .build_store(self.pending_exception_native().as_pointer_value(), null)
            .map_err(|error| error.to_string())?;
        self.builder
            .build_store(
                self.pending_exception_native_text().as_pointer_value(),
                null,
            )
            .map_err(|error| error.to_string())?;
        // Every fresh native/FFI/host error already clears text provenance.
        // The Promise.any companion belongs to the same exception tuple.
        self.builder
            .build_store(
                self.pending_exception_aggregate_errors().as_pointer_value(),
                null,
            )
            .map_err(|error| error.to_string())?;
        Ok(())
    }

    fn mark_pending_native_text(
        &self,
        value: impl Into<BasicValueEnum<'ctx>>,
    ) -> Result<(), String> {
        self.builder
            .build_store(
                self.pending_exception_native_text().as_pointer_value(),
                value.into(),
            )
            .map_err(|error| error.to_string())?;
        Ok(())
    }

    fn async_catch_valid_name(name: &str) -> String {
        format!("@@thaw_catch_original_valid:{name}")
    }

    fn async_catch_original_name(name: &str) -> String {
        format!("@@thaw_catch_original_value:{name}")
    }

    /// Frame cell for the caught text when `name` itself is the carrier local.
    fn async_catch_text_name(name: &str) -> String {
        format!("@@thaw_catch_text:{name}")
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
        let HirExpr::Var(binding) = expression else {
            return Ok(false);
        };
        let Some((catch_slot, native_slot, original_slot, valid_slot, exception_native_slot)) =
            self.catch_native_text.get(binding)
        else {
            return Ok(false);
        };
        if self.variables.get(binding).map(|(slot, _)| slot) != Some(catch_slot) {
            return Ok(false);
        }
        let ptr = self.context.ptr_type(AddressSpace::default());
        let original = self
            .builder
            .build_load(ptr, *original_slot, "caught_original_value")
            .map_err(|error| error.to_string())?
            .into_pointer_value();
        let same = self
            .builder
            .build_int_compare(
                IntPredicate::EQ,
                self.builder
                    .build_ptr_to_int(error, self.context.i64_type(), "rethrow_error_bits")
                    .map_err(|error| error.to_string())?,
                self.builder
                    .build_ptr_to_int(original, self.context.i64_type(), "rethrow_original_bits")
                    .map_err(|error| error.to_string())?,
                "rethrow_original_matches",
            )
            .map_err(|error| error.to_string())?;
        let original_valid = self
            .builder
            .build_load(
                self.context.bool_type(),
                *valid_slot,
                "rethrow_original_valid",
            )
            .map_err(|error| error.to_string())?
            .into_int_value();
        let same = self
            .builder
            .build_and(same, original_valid, "rethrow_unchanged_catch")
            .map_err(|error| error.to_string())?;
        let native = self
            .builder
            .build_load(ptr, *native_slot, "caught_native_text")
            .map_err(|error| error.to_string())?
            .into_pointer_value();
        let matching_text = self
            .builder
            .build_int_compare(
                IntPredicate::EQ,
                self.builder
                    .build_ptr_to_int(error, self.context.i64_type(), "caught_error_bits")
                    .map_err(|error| error.to_string())?,
                self.builder
                    .build_ptr_to_int(native, self.context.i64_type(), "caught_native_bits")
                    .map_err(|error| error.to_string())?,
                "caught_native_matches",
            )
            .map_err(|error| error.to_string())?;
        let native_present = self
            .builder
            .build_is_not_null(native, "caught_native_present")
            .map_err(|error| error.to_string())?;
        let trusted = self
            .builder
            .build_and(
                same,
                self.builder
                    .build_and(
                        matching_text,
                        native_present,
                        "caught_native_matches_present",
                    )
                    .map_err(|error| error.to_string())?,
                "caught_native_trusted",
            )
            .map_err(|error| error.to_string())?;
        let native = self
            .builder
            .build_select(trusted, native, ptr.const_null(), "rethrow_native_text")
            .map_err(|error| error.to_string())?;
        self.builder
            .build_store(
                self.pending_exception_native_text().as_pointer_value(),
                native,
            )
            .map_err(|error| error.to_string())?;
        let exception_native = self
            .builder
            .build_load(
                ptr,
                *exception_native_slot,
                "caught_native_exception_descriptor",
            )
            .map_err(|error| error.to_string())?
            .into_pointer_value();
        let exception_native = self
            .builder
            .build_select(
                same,
                exception_native,
                ptr.const_null(),
                "rethrow_native_descriptor",
            )
            .map_err(|error| error.to_string())?;
        self.builder
            .build_store(
                self.pending_exception_native().as_pointer_value(),
                exception_native,
            )
            .map_err(|error| error.to_string())?;
        let metadata: [(
            &str,
            PointerValue<'ctx>,
            BasicTypeEnum<'ctx>,
            BasicValueEnum<'ctx>,
            bool,
        ); 6] = [
            (
                "aggregate",
                self.pending_exception_aggregate_errors().as_pointer_value(),
                BasicTypeEnum::from(ptr),
                ptr.const_null().into(),
                true,
            ),
            (
                "object",
                self.pending_exception_object().as_pointer_value(),
                BasicTypeEnum::from(ptr),
                ptr.const_null().into(),
                false,
            ),
            (
                "tag",
                self.pending_exception_value(PENDING_EXCEPTION_VALUE_TAG_SYMBOL)
                    .as_pointer_value(),
                self.context.i64_type().into(),
                self.context.i64_type().const_int(4, false).into(),
                false,
            ),
            (
                "f64",
                self.pending_exception_value(PENDING_EXCEPTION_F64_SYMBOL)
                    .as_pointer_value(),
                self.context.f64_type().into(),
                self.context.f64_type().const_zero().into(),
                false,
            ),
            (
                "i64",
                self.pending_exception_value(PENDING_EXCEPTION_I64_SYMBOL)
                    .as_pointer_value(),
                self.context.i64_type().into(),
                self.context.i64_type().const_zero().into(),
                false,
            ),
            (
                "bool",
                self.pending_exception_value(PENDING_EXCEPTION_BOOL_SYMBOL)
                    .as_pointer_value(),
                self.context.bool_type().into(),
                self.context.bool_type().const_zero().into(),
                false,
            ),
        ];
        for (suffix, global, ty, zero, require_trusted) in metadata {
            let name = format!("{binding}__thaw_exception_{suffix}");
            let (slot, _) = self
                .variables
                .get(&name)
                .ok_or_else(|| format!("missing caught exception metadata `{name}`"))?;
            let value = self
                .builder
                .build_load(ty, *slot, "caught_rethrow_metadata")
                .map_err(|error| error.to_string())?;
            let value = self
                .builder
                .build_select(
                    if require_trusted { trusted } else { same },
                    value,
                    zero,
                    "selected_rethrow_metadata",
                )
                .map_err(|error| error.to_string())?;
            self.builder
                .build_store(global, value)
                .map_err(|error| error.to_string())?;
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
        } else {
            self.reject_opaque_value(completion, error, name)
        }
    }

    fn reject_opaque_value(
        &self,
        completion: PointerValue<'ctx>,
        error: PointerValue<'ctx>,
        name: &str,
    ) -> Result<(), String> {
        self.builder
            .build_call(
                self.module.get_function("thaw_promise_reject").unwrap(),
                &[completion.into(), error.into()],
                name,
            )
            .map_err(|error| error.to_string())?;
        Ok(())
    }

    fn pending_exception_native_text(&self) -> inkwell::values::GlobalValue<'ctx> {
        self.module
            .get_global(PENDING_EXCEPTION_NATIVE_TEXT_SYMBOL)
            .expect("native text provenance is declared before code generation")
    }

    fn pending_exception_aggregate_errors(&self) -> inkwell::values::GlobalValue<'ctx> {
        self.module
            .get_global(PENDING_EXCEPTION_AGGREGATE_SYMBOL)
            .expect("aggregate exception state is declared before code generation")
    }

    fn pending_exception_object(&self) -> inkwell::values::GlobalValue<'ctx> {
        self.module
            .get_global(PENDING_EXCEPTION_OBJECT_SYMBOL)
            .expect("exception state is declared before code generation")
    }

    fn pending_exception_native(&self) -> inkwell::values::GlobalValue<'ctx> {
        self.module
            .get_global(PENDING_EXCEPTION_NATIVE_SYMBOL)
            .expect("native exception state is declared before code generation")
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
        let native_text = self
            .builder
            .build_load(
                self.context.ptr_type(AddressSpace::default()),
                self.pending_exception_native_text().as_pointer_value(),
                "pending_native_text_provenance",
            )
            .map_err(|error| error.to_string())?;
        args.push(native_text.into());
        let aggregate_errors = self
            .builder
            .build_load(
                self.context.ptr_type(AddressSpace::default()),
                self.pending_exception_aggregate_errors().as_pointer_value(),
                "pending_aggregate_errors",
            )
            .map_err(|error| error.to_string())?;
        let native = self
            .builder
            .build_load(
                self.context.ptr_type(AddressSpace::default()),
                self.pending_exception_native().as_pointer_value(),
                "pending_native_exception_descriptor",
            )
            .map_err(|error| error.to_string())?;
        let ptr_int = self.context.i64_type();
        let error_bits = self
            .builder
            .build_ptr_to_int(error, ptr_int, "pending_error_bits")
            .map_err(|error| error.to_string())?;
        let native_bits = self
            .builder
            .build_ptr_to_int(
                native_text.into_pointer_value(),
                ptr_int,
                "pending_native_text_bits",
            )
            .map_err(|error| error.to_string())?;
        let matching_text = self
            .builder
            .build_int_compare(
                IntPredicate::EQ,
                error_bits,
                native_bits,
                "aggregate_text_matches_error",
            )
            .map_err(|error| error.to_string())?;
        let nonnull_text = self
            .builder
            .build_is_not_null(
                native_text.into_pointer_value(),
                "aggregate_has_trusted_text",
            )
            .map_err(|error| error.to_string())?;
        let trusted = self
            .builder
            .build_and(matching_text, nonnull_text, "aggregate_trusted_pair")
            .map_err(|error| error.to_string())?;
        let aggregate_errors = self
            .builder
            .build_select(
                trusted,
                aggregate_errors.into_pointer_value(),
                self.context.ptr_type(AddressSpace::default()).const_null(),
                "paired_aggregate_errors",
            )
            .map_err(|error| error.to_string())?;
        args.push(aggregate_errors.into());
        args.push(native.into());
        self.builder
            .build_call(
                self.module
                    .get_function("thaw_promise_reject_typed_with_native_provenance")
                    .unwrap(),
                &args,
                name,
            )
            .map_err(|error| error.to_string())?;
        Ok(())
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
                self.module
                    .get_function("thaw_promise_forward_rejection")
                    .unwrap(),
                &[promise.into(), source.into(), error.into()],
                name,
            )
            .map_err(|error| error.to_string())?;
        Ok(())
    }

    fn build_default_return(&mut self) -> Result<(), String> {
        // This helper is the common escaping-exception/failed-adapter exit.
        // Runtime ownership flags make it safe to use on every return path.
        self.emit_stack_promise_cleanup()?;
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

    /// Function prologue: when the stack is within the guard margin of its end, throw a catchable
    /// `RangeError` instead of faulting. Costs one load, one compare and a cold branch per call.
    fn emit_stack_overflow_guard(&mut self) -> Result<(), String> {
        if !self.stack_guard {
            return Ok(());
        }
        let function = self.current_function();
        let i64_type = self.context.i64_type();
        let probe = self.allocate_variable_cell(self.context.i8_type().into(), "stack_probe")?;
        let sp = self
            .builder
            .build_ptr_to_int(probe, i64_type, "stack_pointer")
            .map_err(|e| e.to_string())?;
        let limit_slot = self
            .module
            .get_global(STACK_LIMIT_SYMBOL)
            .expect("stack limit is declared with the exception state");
        let limit = self
            .builder
            .build_load(i64_type, limit_slot.as_pointer_value(), "stack_limit")
            .map_err(|e| e.to_string())?
            .into_int_value();
        let overflow = self
            .builder
            .build_int_compare(inkwell::IntPredicate::ULT, sp, limit, "stack_overflow")
            .map_err(|e| e.to_string())?;
        let cold = self.context.append_basic_block(function, "stack_overflow");
        let ok = self.context.append_basic_block(function, "stack_ok");
        let weights = self.context.metadata_node(&[
            self.context.metadata_string("branch_weights").into(),
            self.context.i32_type().const_int(1, false).into(),
            self.context.i32_type().const_int(1 << 20, false).into(),
        ]);
        let branch = self
            .builder
            .build_conditional_branch(overflow, cold, ok)
            .map_err(|e| e.to_string())?;
        branch
            .set_metadata(weights, self.context.get_kind_id("prof"))
            .map_err(|e| e.to_string())?;
        self.builder.position_at_end(cold);
        // The trusted-text marker keeps this cold path free of runtime calls (JIT-mapped modules
        // must not gain external symbols) and of the arena-owned literal frame.
        self.compile_stmt(&HirStmt::Throw(HirExpr::Call(
            Box::new(HirExpr::Var("@@thaw_trusted_exception_text".to_string())),
            vec![HirExpr::Lit(HirLit::Str(
                "\u{1}RangeError\u{1}Maximum call stack size exceeded".to_string(),
            ))],
        )))?;
        self.builder.position_at_end(ok);
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

        if let Some(catch) = self.catch_stack.last().cloned() {
            let propagate_bb = self
                .context
                .append_basic_block(function, "cleanup_before_catch");
            self.builder
                .build_conditional_branch(has_exception, propagate_bb, continue_bb)
                .map_err(|e| e.to_string())?;
            self.builder.position_at_end(propagate_bb);
            self.emit_fresh_event_key_cleanup_before_catch(Some(self.catch_stack.len() - 1))?;
            self.emit_stack_promise_cleanup_except(&catch.promise_boundary)?;
            self.builder
                .build_unconditional_branch(catch.target)
                .map_err(|e| e.to_string())?;
        } else {
            let propagate_bb = self
                .context
                .append_basic_block(function, "propagate_exception");
            self.builder
                .build_conditional_branch(has_exception, propagate_bb, continue_bb)
                .map_err(|e| e.to_string())?;
            self.builder.position_at_end(propagate_bb);
            self.emit_fresh_event_key_cleanup_before_catch(None)?;
            if let Some(completion) = self.active_async_completion {
                self.reject_promise_with_pending_exception(
                    completion,
                    pending,
                    "reject_frame_exception",
                )?;
                self.builder
                    .build_store(
                        self.pending_exception().as_pointer_value(),
                        ptr_ty.const_null(),
                    )
                    .map_err(|e| e.to_string())?;
                self.clear_pending_native_text()?;
                self.builder
                    .build_store(
                        self.pending_exception_object().as_pointer_value(),
                        ptr_ty.const_null(),
                    )
                    .map_err(|e| e.to_string())?;
                self.builder
                    .build_store(
                        self.pending_exception_value(PENDING_EXCEPTION_VALUE_TAG_SYMBOL)
                            .as_pointer_value(),
                        self.context.i64_type().const_zero(),
                    )
                    .map_err(|e| e.to_string())?;
                // The rejection helper has taken ownership of the reason;
                // now local synchronous Promise tokens can be released.
                self.emit_stack_promise_cleanup()?;
                if function.get_type().get_return_type().is_some() {
                    self.builder
                        .build_return(Some(&completion))
                        .map_err(|e| e.to_string())?;
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
        self.stack_promise_slots.clear();
        self.for_iteration_frame_slots.clear();
        self.catch_native_text.clear();
        self.variable_hir_types.clear();
        self.arena_variables.clear();
        self.clear_catch_context();
        self.loop_scopes.clear();
        self.loop_promotion_scopes.clear();
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
            if matches!(hir_param.ty, HirType::Promise(_)) {
                self.register_stack_promise_slot(slot, &hir_param.name)?;
            }
        }

        self.emit_stack_overflow_guard()?;
        let terminated = self.compile_block(&func.body)?;

        if !terminated {
            if func.ret == HirType::Void {
                self.emit_stack_promise_cleanup()?;
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
include!("hir_codegen/json_discard.rs");

include!("hir_codegen/values/expressions.rs");
include!("hir_codegen/values/unions.rs");
include!("hir_codegen/values/closures.rs");

include!("hir_codegen/collections.rs");

include!("hir_codegen/builtins.rs");

include!("hir_codegen/json_values.rs");

include!("hir_codegen/js_engine.rs");

include!("hir_codegen/dynamic_host.rs");

include!("hir_codegen/json_bridge.rs");

include!("hir_codegen/operators.rs");

include!("hir_codegen/bigint.rs");

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

    /// Library mode (`thaw lib build`): no C `main`; each export gets a C wrapper, everything else
    /// the program defines turns internal. Must be called before `compile_program`. Also selects
    /// position-independent code.
    pub fn set_library_mode(&mut self, exports: Vec<LibraryExport>) {
        self.library_exports = exports;
        self.pic = true;
    }

    /// `embed_artifact_metadata` for a library: symbol `__thaw_pkg_metadata`, section `.thaw.pkg`.
    pub fn embed_package_metadata(&self, metadata: &[u8]) -> Result<(), String> {
        let value = self.context.const_string(metadata, true);
        let global = self
            .module
            .add_global(value.get_type(), None, LIBRARY_METADATA_SYMBOL);
        global.set_initializer(&value);
        global.set_constant(true);
        global.set_section(Some(".thaw.pkg"));
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
                self.codegen_opt,
                if self.pic {
                    RelocMode::PIC
                } else {
                    RelocMode::Default
                },
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
