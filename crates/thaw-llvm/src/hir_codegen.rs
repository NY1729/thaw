//! The real Thaw pipeline: `thaw_hir::HirProgram` -> LLVM IR -> a native
//! object file. This is a separate, unrelated compiler from the
//! `lexer`/`ast`/`parser`/`codegen` modules at the crate root, which are the
//! Kaleidoscope tutorial scaffold used to learn inkwell in the first place.
//!
//! Phase 0 scope: numbers (f64), strings (`i8*`, C-string style), booleans
//! (`i1`), and functions, plus a `console.log` builtin bridged to libc
//! `puts`/`printf` as a bootstrap (the real `console.log`/std implementation
//! lands in Phase 2).
//!
//! Phase 1 adds: `if`/`while` (classic `for` is already desugared to these
//! by thaw-hir's lowering), `throw`/`try`/`catch`, mutable local variables (`let`,
//! assignment, `++`/`--`), and number arrays (`number[]`), heap-allocated
//! from `thaw-arena`'s bump allocator (linked in by thaw-cli as a static
//! archive) rather than `malloc` -- this is the design doc's "no GC, arena
//! allocation" decision actually landing in generated code, not just a
//! standalone crate. Local variables are now alloca'd (loaded/stored on
//! every read/write) instead of tracked as raw SSA values, since mutation
//! needs a memory slot; nothing runs LLVM's mem2reg pass yet; correctness
//! doesn't depend on it, so the extra loads/stores are left for a later
//! optimization pass.
//!
//! Phase 2 additionally adds `handler(event: string): string` and
//! `handler(event: Json): Json` as alternate process entry points (see
//! `emit_lambda_entry`), `process.env`,
//! and object types: `{ x: number; y: number }`-style records, `f64`
//! fields only, arena-allocated as a flat `[f64 field0]...[f64 fieldN-1]`
//! buffer with no length header (field order is static, part of the type
//! -- see `thaw_hir::HirExpr::PropAccess`, which bakes in the resolved
//! field layout so codegen never needs its own type inference pass).
//! async/await is designed but not implemented yet -- see
//! `docs/design/async-await.md`.

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
/// Parallel, opt-in companion to `PENDING_EXCEPTION_SYMBOL`: null unless the
/// thrown value was a real (Error-family) class instance, in which case it
/// holds that object's own pointer so a catch site can read fields beyond
/// `.message`/`.name` after an explicit `as` cast (see
/// `docs/design/exceptions.md` section 3). Every other reader of the
/// exception state (console.log, N-API, Lambda error reporting,
/// `Promise.reject`) only ever looks at the string channel and stays
/// completely unaware this one exists.
const PENDING_EXCEPTION_OBJECT_SYMBOL: &str = "__thaw_pending_exception_object";

/// Byte size of an array's length header (a single `i64`) that precedes its
/// elements in the arena-allocated buffer. See the module doc for the layout.
const ARRAY_HEADER_BYTES: u64 = 8;
/// Phase 1 arrays only ever hold `f64` elements (see module doc).
const ARRAY_ELEM_BYTES: u64 = 8;
/// Minimum storage/alignment unit for native object fields. Tagged optional
/// fields occupy two units; field count/order/layout remain static type data.
const OBJECT_FIELD_BYTES: u64 = 8;
const ASYNC_FRAME_BYTES: u64 = 24;
const ASYNC_SLOT_BYTES: u64 = 16;

fn object_field_storage_bytes(ty: &HirType) -> u64 {
    if matches!(
        ty,
        HirType::Optional(_) | HirType::Nullable(_) | HirType::Nullish(_) | HirType::Union(_)
    ) {
        ASYNC_SLOT_BYTES
    } else {
        OBJECT_FIELD_BYTES
    }
}

fn array_element_storage_bytes(ty: &HirType) -> u64 {
    if matches!(
        ty,
        HirType::Optional(_) | HirType::Nullable(_) | HirType::Nullish(_) | HirType::Union(_)
    ) {
        ASYNC_SLOT_BYTES
    } else {
        ARRAY_ELEM_BYTES
    }
}

fn object_field_offset(fields: &[(String, HirType)], index: usize) -> u64 {
    fields[..index]
        .iter()
        .map(|(_, ty)| object_field_storage_bytes(ty))
        .sum()
}

fn object_storage_bytes(fields: &[(String, HirType)]) -> u64 {
    fields
        .iter()
        .map(|(_, ty)| object_field_storage_bytes(ty))
        .sum()
}
const ASYNC_COMPLETION_OFFSET: u64 = 0;
const ASYNC_STATE_OFFSET: u64 = 8;
const ASYNC_WAITING_OFFSET: u64 = 16;
const ASYNC_RESULT_OFFSET: u64 = ASYNC_FRAME_BYTES;
const CLOSURE_THIS_ENTRY_OFFSET: u64 = 8;
const CLOSURE_CAPTURE_BASE: u64 = 16;
const BOUND_CLOSURE_SOURCE_OFFSET: u64 = CLOSURE_CAPTURE_BASE;
const BOUND_CLOSURE_THIS_OFFSET: u64 = CLOSURE_CAPTURE_BASE + 8;
const BOUND_CLOSURE_ARGUMENT_BASE: u64 = CLOSURE_CAPTURE_BASE + 16;

struct AsyncSegment {
    stmts: Vec<HirStmt>,
    awaited: Option<HirExpr>,
    await_guard: Option<(String, bool)>,
    await_next: Option<usize>,
    resume_target: Option<(String, HirType)>,
    rejection_handler: Option<AsyncRejectionHandler>,
}

#[derive(Clone)]
struct AsyncRejectionHandler {
    try_guard: String,
    catch_guard: String,
    catch_binding: String,
    disable_guards: Vec<String>,
}

struct FrameAsyncPlan {
    segments: Vec<AsyncSegment>,
    locals: Vec<(String, HirType)>,
    ret: HirType,
    guarded_rethrow_handlers: HashMap<String, AsyncRejectionHandler>,
    returns_on_all_paths: bool,
}

enum AsyncBlockExit {
    Continue,
    Returned,
    Rejected,
}

pub struct HirCompiler<'ctx> {
    context: &'ctx Context,
    module: Module<'ctx>,
    builder: Builder<'ctx>,
    variables: HashMap<String, (PointerValue<'ctx>, BasicTypeEnum<'ctx>)>,
    variable_hir_types: HashMap<String, HirType>,
    arena_variables: HashSet<String>,
    global_variables: HashMap<String, (PointerValue<'ctx>, BasicTypeEnum<'ctx>, HirType)>,
    function_return_types: HashMap<String, HirType>,
    ffi_signatures: HashMap<String, FfiSignature>,
    /// Stack of enclosing `try` targets. `throw` and a failed nested Thaw
    /// call target the innermost entry; an empty stack propagates by returning
    /// from the current function with the pending exception left intact.
    catch_stack: Vec<BasicBlock<'ctx>>,
    loop_stack: Vec<(BasicBlock<'ctx>, BasicBlock<'ctx>)>,
    /// Functions whose async ABI is a Promise-returning ramp rather than the
    /// legacy synchronous V1 ABI. Seeded to a fixed point before declarations
    /// so callers and callees agree on the LLVM signature.
    frame_async_functions: HashMap<String, HirType>,
    promise_returning_functions: HashSet<String>,
    active_async_completion: Option<PointerValue<'ctx>>,
    next_lambda: usize,
    uses_napi: bool,
    uses_quickjs: bool,
    uses_quickjs_handles: bool,
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

impl<'ctx> HirCompiler<'ctx> {
    pub fn new(context: &'ctx Context, module_name: &str) -> Self {
        Self {
            context,
            module: context.create_module(module_name),
            builder: context.create_builder(),
            variables: HashMap::new(),
            variable_hir_types: HashMap::new(),
            arena_variables: HashSet::new(),
            global_variables: HashMap::new(),
            function_return_types: HashMap::new(),
            ffi_signatures: HashMap::new(),
            catch_stack: Vec::new(),
            loop_stack: Vec::new(),
            frame_async_functions: HashMap::new(),
            promise_returning_functions: HashSet::new(),
            active_async_completion: None,
            next_lambda: 0,
            uses_napi: false,
            uses_quickjs: false,
            uses_quickjs_handles: false,
            compiling_quickjs_dynamic_arguments: false,
        }
    }

    pub fn compile_program(&mut self, program: &HirProgram) -> Result<(), String> {
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

    fn declare_exception_state(&self) {
        let ptr_ty = self.context.ptr_type(AddressSpace::default());
        let pending = self
            .module
            .add_global(ptr_ty, None, PENDING_EXCEPTION_SYMBOL);
        pending.set_linkage(Linkage::Internal);
        pending.set_initializer(&ptr_ty.const_null());
        let pending_object = self
            .module
            .add_global(ptr_ty, None, PENDING_EXCEPTION_OBJECT_SYMBOL);
        pending_object.set_linkage(Linkage::Internal);
        pending_object.set_initializer(&ptr_ty.const_null());
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
        self.variable_hir_types.clear();
        self.arena_variables.clear();
        self.catch_stack.clear();
        self.seed_global_variables();
        for step in &program.initializers {
            match step {
                HirInitStep::StoreGlobal(name, expression) => {
                    let value = self.compile_expr(expression)?;
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
            HirStmt::Try(body, _, catch_body) => body
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
                    HirExpr::PromiseNew(_, _, _)
                        | HirExpr::PromiseThen(_, _, _, _, _, _)
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

    fn pending_exception_object(&self) -> inkwell::values::GlobalValue<'ctx> {
        self.module
            .get_global(PENDING_EXCEPTION_OBJECT_SYMBOL)
            .expect("exception state is declared before code generation")
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
            self.builder
                .build_conditional_branch(has_exception, catch_bb, continue_bb)
                .map_err(|e| e.to_string())?;
        } else {
            let propagate_bb = self
                .context
                .append_basic_block(function, "propagate_exception");
            self.builder
                .build_conditional_branch(has_exception, propagate_bb, continue_bb)
                .map_err(|e| e.to_string())?;
            self.builder.position_at_end(propagate_bb);
            self.build_default_return()?;
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
        self.variable_hir_types.clear();
        self.catch_stack.clear();
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
                if param_types
                    .get(index)
                    .is_some_and(|ty| ty.is_pointer_type())
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

        // Optimize the IR before instruction selection. LLVM's default O2
        // pipeline includes the coroutine lowering required by async functions.
        self.module
            .run_passes("default<O2>", &target_machine, PassBuilderOptions::create())
            .map_err(|e| format!("optimization pass pipeline failed: {e}"))?;

        target_machine
            .write_to_file(&self.module, FileType::Object, path)
            .map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests;
