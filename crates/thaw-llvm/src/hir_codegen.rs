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

use std::collections::HashMap;
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
    BasicMetadataValueEnum, BasicValue, BasicValueEnum, FunctionValue, IntValue, PointerValue,
    StructValue,
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
    active_async_completion: Option<PointerValue<'ctx>>,
    next_lambda: usize,
    uses_napi: bool,
    uses_quickjs_handles: bool,
}

impl<'ctx> HirCompiler<'ctx> {
    pub fn new(context: &'ctx Context, module_name: &str) -> Self {
        Self {
            context,
            module: context.create_module(module_name),
            builder: context.create_builder(),
            variables: HashMap::new(),
            variable_hir_types: HashMap::new(),
            global_variables: HashMap::new(),
            function_return_types: HashMap::new(),
            ffi_signatures: HashMap::new(),
            catch_stack: Vec::new(),
            loop_stack: Vec::new(),
            frame_async_functions: HashMap::new(),
            active_async_completion: None,
            next_lambda: 0,
            uses_napi: false,
            uses_quickjs_handles: false,
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
            (false, None) => {
                return Err(
                    "no `function main(): void { ... }`, `function handler(event: string): string { ... }`, or `function handler(event: Json): Json { ... }` found"
                        .to_string(),
                )
            }
        }

        self.module
            .verify()
            .map_err(|e| format!("module verification failed:\n{e}"))
    }

    fn declare_exception_state(&self) {
        let ptr_ty = self.context.ptr_type(AddressSpace::default());
        let pending = self
            .module
            .add_global(ptr_ty, None, PENDING_EXCEPTION_SYMBOL);
        pending.set_linkage(Linkage::Internal);
        pending.set_initializer(&ptr_ty.const_null());
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
            HirExpr::JsonSet(object, key, value, _) => [object, key, value]
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
            | HirExpr::JsonKey(obj, value) => {
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

    /// Declares libc's `puts`/`printf` (the Phase 0 `console.log` bootstrap)
    /// and thaw-arena's `thaw_arena_alloc` (backing Phase 1 arrays).
    fn declare_runtime_builtins(&self) {
        let i8_ptr = self.context.ptr_type(AddressSpace::default());
        let i8_type = self.context.i8_type();
        let i32_type = self.context.i32_type();
        let i64_type = self.context.i64_type();

        let puts_type = i32_type.fn_type(&[i8_ptr.into()], false);
        self.module
            .add_function("puts", puts_type, Some(Linkage::External));

        let printf_type = i32_type.fn_type(&[i8_ptr.into()], true);
        self.module
            .add_function("printf", printf_type, Some(Linkage::External));
        let pow_type = self.context.f64_type().fn_type(
            &[
                self.context.f64_type().into(),
                self.context.f64_type().into(),
            ],
            false,
        );
        self.module
            .add_function("pow", pow_type, Some(Linkage::External));
        let unary_f64_type = self
            .context
            .f64_type()
            .fn_type(&[self.context.f64_type().into()], false);
        for name in [
            "tan", "asin", "acos", "atan", "sinh", "cosh", "tanh", "cbrt", "acosh", "asinh",
            "atanh", "expm1", "log1p",
        ] {
            self.module
                .add_function(name, unary_f64_type, Some(Linkage::External));
        }
        for name in ["atan2", "hypot"] {
            self.module
                .add_function(name, pow_type, Some(Linkage::External));
        }
        for name in ["thaw_math_fround", "thaw_math_clz32"] {
            self.module
                .add_function(name, unary_f64_type, Some(Linkage::External));
        }
        self.module
            .add_function("thaw_math_imul", pow_type, Some(Linkage::External));
        let math_random_type = self.context.f64_type().fn_type(&[], false);
        self.module.add_function(
            "thaw_math_random",
            math_random_type,
            Some(Linkage::External),
        );
        for name in [
            "llvm.fabs.f64",
            "llvm.floor.f64",
            "llvm.ceil.f64",
            "llvm.trunc.f64",
            "llvm.sqrt.f64",
            "llvm.exp.f64",
            "llvm.log.f64",
            "llvm.log2.f64",
            "llvm.log10.f64",
            "llvm.sin.f64",
            "llvm.cos.f64",
        ] {
            self.module
                .add_function(name, unary_f64_type, Some(Linkage::External));
        }
        let binary_f64_type = self.context.f64_type().fn_type(
            &[
                self.context.f64_type().into(),
                self.context.f64_type().into(),
            ],
            false,
        );
        for name in ["llvm.minimum.f64", "llvm.maximum.f64"] {
            self.module
                .add_function(name, binary_f64_type, Some(Linkage::External));
        }

        let arena_alloc_type = i8_ptr.fn_type(&[i64_type.into(), i64_type.into()], false);
        self.module.add_function(
            "thaw_arena_alloc",
            arena_alloc_type,
            Some(Linkage::External),
        );

        let strlen_type = i64_type.fn_type(&[i8_ptr.into()], false);
        self.module
            .add_function("strlen", strlen_type, Some(Linkage::External));
        let strcmp_type = i32_type.fn_type(&[i8_ptr.into(), i8_ptr.into()], false);
        self.module
            .add_function("strcmp", strcmp_type, Some(Linkage::External));
        self.module
            .add_function("thaw_string_compare", strcmp_type, Some(Linkage::External));
        let memcpy_type = i8_ptr.fn_type(&[i8_ptr.into(), i8_ptr.into(), i64_type.into()], false);
        self.module
            .add_function("memcpy", memcpy_type, Some(Linkage::External));

        let getenv_type = i8_ptr.fn_type(&[i8_ptr.into()], false);
        self.module
            .add_function("getenv", getenv_type, Some(Linkage::External));

        // Lambda captures stdout via a pipe, not a TTY, so libc's stdio
        // fully-buffers it by default -- output could sit in the buffer
        // and never reach CloudWatch if the process is frozen/killed
        // between invocations. `console.log` flushes after every call to
        // avoid that (see `compile_console_log`).
        let fflush_type = i32_type.fn_type(&[i8_ptr.into()], false);
        self.module
            .add_function("fflush", fflush_type, Some(Linkage::External));

        let run_http_servers_type = self.context.void_type().fn_type(&[], false);
        self.module.add_function(
            "thaw_http_run_servers",
            run_http_servers_type,
            Some(Linkage::External),
        );
        self.module.add_function(
            "thaw_http_take_unhandled_error",
            self.context.i8_type().fn_type(&[], false),
            Some(Linkage::External),
        );

        // thaw-std: fetch + JSON (see docs/design/async-await.md for why
        // `fetch` is a plain blocking call under the hood).
        let f64_type = self.context.f64_type();

        let fetch_type = i8_ptr.fn_type(&[i8_ptr.into()], false);
        self.module
            .add_function("thaw_fetch_get", fetch_type, Some(Linkage::External));

        let json_parse_type = i8_ptr.fn_type(&[i8_ptr.into()], false);
        self.module
            .add_function("thaw_json_parse", json_parse_type, Some(Linkage::External));

        let json_stringify_type = i8_ptr.fn_type(&[i8_ptr.into()], false);
        self.module.add_function(
            "thaw_json_stringify",
            json_stringify_type,
            Some(Linkage::External),
        );

        let json_get_type = i8_ptr.fn_type(&[i8_ptr.into(), i8_ptr.into()], false);
        self.module
            .add_function("thaw_json_get", json_get_type, Some(Linkage::External));

        let json_index_type = i8_ptr.fn_type(&[i8_ptr.into(), i64_type.into()], false);
        self.module
            .add_function("thaw_json_index", json_index_type, Some(Linkage::External));

        let json_as_number_type = f64_type.fn_type(&[i8_ptr.into()], false);
        self.module.add_function(
            "thaw_json_as_number",
            json_as_number_type,
            Some(Linkage::External),
        );
        let number_to_string_type = i8_ptr.fn_type(&[f64_type.into()], false);
        self.module.add_function(
            "thaw_number_object_is",
            i8_type.fn_type(&[f64_type.into(), f64_type.into()], false),
            Some(Linkage::External),
        );
        self.module.add_function(
            "thaw_number_to_string",
            number_to_string_type,
            Some(Linkage::External),
        );
        let string_to_number_type = f64_type.fn_type(&[i8_ptr.into()], false);
        self.module.add_function(
            "thaw_string_to_number",
            string_to_number_type,
            Some(Linkage::External),
        );
        self.module.add_function(
            "thaw_parse_float",
            string_to_number_type,
            Some(Linkage::External),
        );
        let parse_int_type = f64_type.fn_type(&[i8_ptr.into(), f64_type.into()], false);
        self.module
            .add_function("thaw_parse_int", parse_int_type, Some(Linkage::External));
        let array_to_string_type = i8_ptr.fn_type(&[i8_ptr.into()], false);
        for name in [
            "thaw_number_array_to_string",
            "thaw_string_array_to_string",
            "thaw_bool_array_to_string",
            "thaw_object_array_to_string",
        ] {
            self.module
                .add_function(name, array_to_string_type, Some(Linkage::External));
        }
        let array_join_type = i8_ptr.fn_type(&[i8_ptr.into(), i8_ptr.into()], false);
        for name in [
            "thaw_number_array_join",
            "thaw_string_array_join",
            "thaw_bool_array_join",
            "thaw_object_array_join",
        ] {
            self.module
                .add_function(name, array_join_type, Some(Linkage::External));
        }
        let array_reverse_type = i8_ptr.fn_type(&[i8_ptr.into(), i64_type.into()], false);
        self.module.add_function(
            "thaw_array_reverse",
            array_reverse_type,
            Some(Linkage::External),
        );
        let array_copy_within_type = i8_ptr.fn_type(
            &[
                i8_ptr.into(),
                i64_type.into(),
                f64_type.into(),
                f64_type.into(),
                f64_type.into(),
            ],
            false,
        );
        self.module.add_function(
            "thaw_array_copy_within",
            array_copy_within_type,
            Some(Linkage::External),
        );
        for (name, value_type) in [
            ("thaw_number_array_fill", f64_type.into()),
            ("thaw_pointer_array_fill", i8_ptr.into()),
            ("thaw_bool_array_fill", i8_type.into()),
        ] {
            let ty = i8_ptr.fn_type(
                &[i8_ptr.into(), value_type, f64_type.into(), f64_type.into()],
                false,
            );
            self.module.add_function(name, ty, Some(Linkage::External));
        }
        let array_fill_type = i8_ptr.fn_type(
            &[
                i8_ptr.into(),
                i8_ptr.into(),
                i64_type.into(),
                f64_type.into(),
                f64_type.into(),
            ],
            false,
        );
        self.module
            .add_function("thaw_array_fill", array_fill_type, Some(Linkage::External));
        let array_slice_type = i8_ptr.fn_type(
            &[
                i8_ptr.into(),
                i64_type.into(),
                f64_type.into(),
                f64_type.into(),
            ],
            false,
        );
        self.module.add_function(
            "thaw_array_slice",
            array_slice_type,
            Some(Linkage::External),
        );
        self.module.add_function(
            "thaw_array_to_reversed",
            array_reverse_type,
            Some(Linkage::External),
        );
        let array_sort_type = i8_ptr.fn_type(&[i8_ptr.into()], false);
        for name in [
            "thaw_number_array_sort",
            "thaw_string_array_sort",
            "thaw_bool_array_sort",
            "thaw_object_array_sort",
            "thaw_number_array_to_sorted",
            "thaw_string_array_to_sorted",
            "thaw_bool_array_to_sorted",
            "thaw_object_array_to_sorted",
        ] {
            self.module
                .add_function(name, array_sort_type, Some(Linkage::External));
        }
        for (name, needle_type, return_type) in [
            (
                "thaw_number_array_index_of",
                f64_type.into(),
                f64_type.into(),
            ),
            (
                "thaw_number_array_includes",
                f64_type.into(),
                i8_type.into(),
            ),
            ("thaw_string_array_index_of", i8_ptr.into(), f64_type.into()),
            ("thaw_string_array_includes", i8_ptr.into(), i8_type.into()),
            ("thaw_bool_array_index_of", i8_type.into(), f64_type.into()),
            ("thaw_bool_array_includes", i8_type.into(), i8_type.into()),
            ("thaw_object_array_index_of", i8_ptr.into(), f64_type.into()),
            ("thaw_object_array_includes", i8_ptr.into(), i8_type.into()),
            (
                "thaw_number_array_last_index_of",
                f64_type.into(),
                f64_type.into(),
            ),
            (
                "thaw_string_array_last_index_of",
                i8_ptr.into(),
                f64_type.into(),
            ),
            (
                "thaw_bool_array_last_index_of",
                i8_type.into(),
                f64_type.into(),
            ),
            (
                "thaw_object_array_last_index_of",
                i8_ptr.into(),
                f64_type.into(),
            ),
        ] {
            let function_type = match return_type {
                BasicTypeEnum::FloatType(return_type) => {
                    return_type.fn_type(&[i8_ptr.into(), needle_type, f64_type.into()], false)
                }
                BasicTypeEnum::IntType(return_type) => {
                    return_type.fn_type(&[i8_ptr.into(), needle_type, f64_type.into()], false)
                }
                _ => unreachable!(),
            };
            self.module
                .add_function(name, function_type, Some(Linkage::External));
        }
        for name in [
            "thaw_string_includes",
            "thaw_string_starts_with",
            "thaw_string_ends_with",
        ] {
            let function_type =
                i8_type.fn_type(&[i8_ptr.into(), i8_ptr.into(), f64_type.into()], false);
            self.module
                .add_function(name, function_type, Some(Linkage::External));
        }
        let string_index_of_type =
            f64_type.fn_type(&[i8_ptr.into(), i8_ptr.into(), f64_type.into()], false);
        self.module.add_function(
            "thaw_string_index_of",
            string_index_of_type,
            Some(Linkage::External),
        );
        self.module.add_function(
            "thaw_string_last_index_of",
            string_index_of_type,
            Some(Linkage::External),
        );
        let string_transform_type = i8_ptr.fn_type(&[i8_ptr.into()], false);
        for name in [
            "thaw_string_trim",
            "thaw_string_trim_start",
            "thaw_string_trim_end",
            "thaw_string_to_lower_case",
            "thaw_string_to_upper_case",
        ] {
            self.module
                .add_function(name, string_transform_type, Some(Linkage::External));
        }
        self.module.add_function(
            "thaw_string_to_array",
            string_transform_type,
            Some(Linkage::External),
        );
        self.module.add_function(
            "thaw_string_repeat",
            i8_ptr.fn_type(&[i8_ptr.into(), f64_type.into()], false),
            Some(Linkage::External),
        );
        let string_length_type = f64_type.fn_type(&[i8_ptr.into()], false);
        self.module.add_function(
            "thaw_string_length",
            string_length_type,
            Some(Linkage::External),
        );
        let string_char_code_type = f64_type.fn_type(&[i8_ptr.into(), f64_type.into()], false);
        self.module.add_function(
            "thaw_string_char_code_at",
            string_char_code_type,
            Some(Linkage::External),
        );

        let json_as_string_type = i8_ptr.fn_type(&[i8_ptr.into()], false);
        self.module.add_function(
            "thaw_json_as_string",
            json_as_string_type,
            Some(Linkage::External),
        );

        // Returns i8 (0/1), not i1 -- see thaw-std's `thaw_json_as_bool`
        // doc comment on why it avoids relying on `bool`'s C ABI shape.
        let json_as_bool_type = self.context.i8_type().fn_type(&[i8_ptr.into()], false);
        self.module.add_function(
            "thaw_json_as_bool",
            json_as_bool_type,
            Some(Linkage::External),
        );
        self.module.add_function(
            "thaw_json_array_new",
            i8_ptr.fn_type(&[], false),
            Some(Linkage::External),
        );
        self.module.add_function(
            "thaw_json_is_array",
            self.context.i8_type().fn_type(&[i8_ptr.into()], false),
            Some(Linkage::External),
        );
        for (name, value_type) in [
            ("thaw_json_array_push_number", f64_type.into()),
            ("thaw_json_array_push_string", i8_ptr.into()),
            ("thaw_json_array_push_bool", self.context.i8_type().into()),
            ("thaw_json_array_push_json", i8_ptr.into()),
        ] {
            self.module.add_function(
                name,
                self.context
                    .void_type()
                    .fn_type(&[i8_ptr.into(), value_type], false),
                Some(Linkage::External),
            );
        }
        for name in ["thaw_json_from_number_array", "thaw_json_to_number_array"] {
            self.module.add_function(
                name,
                i8_ptr.fn_type(&[i8_ptr.into()], false),
                Some(Linkage::External),
            );
        }
        self.module.add_function(
            "thaw_json_object_new",
            i8_ptr.fn_type(&[], false),
            Some(Linkage::External),
        );
        for (name, value_type) in [
            ("thaw_json_object_set_number", f64_type.into()),
            ("thaw_json_object_set_string", i8_ptr.into()),
            ("thaw_json_object_set_bool", self.context.i8_type().into()),
            ("thaw_json_object_set_json", i8_ptr.into()),
        ] {
            self.module.add_function(
                name,
                self.context
                    .void_type()
                    .fn_type(&[i8_ptr.into(), i8_ptr.into(), value_type], false),
                Some(Linkage::External),
            );
        }

        // thaw-quickjs: the QuickJS-NG fallback path (docs/design/bridge.md
        // section 7). Same i8-not-i1 reasoning as `thaw_json_as_bool`.
        let js_load_type = self.context.i8_type().fn_type(&[i8_ptr.into()], false);
        self.module
            .add_function("thaw_js_load", js_load_type, Some(Linkage::External));

        let js_call_type = i8_ptr.fn_type(&[i8_ptr.into(), i8_ptr.into()], false);
        self.module
            .add_function("thaw_js_call", js_call_type, Some(Linkage::External));
        let result_type = self
            .context
            .struct_type(&[i8_ptr.into(), i8_ptr.into()], false);
        let js_call_result_type = result_type.fn_type(&[i8_ptr.into(), i8_ptr.into()], false);
        self.module.add_function(
            "thaw_js_call_result",
            js_call_result_type,
            Some(Linkage::External),
        );
        self.module.add_function(
            "thaw_js_get_global",
            self.context.i64_type().fn_type(&[i8_ptr.into()], false),
            Some(Linkage::External),
        );
        self.module.add_function(
            "thaw_js_call_handle_result",
            result_type.fn_type(&[self.context.i64_type().into(), i8_ptr.into()], false),
            Some(Linkage::External),
        );
        self.module.add_function(
            "thaw_js_call_handle_handle_result",
            self.context
                .struct_type(&[self.context.i64_type().into(), i8_ptr.into()], false)
                .fn_type(&[self.context.i64_type().into(), i8_ptr.into()], false),
            Some(Linkage::External),
        );
        self.module.add_function(
            "thaw_js_call_handle_value_result",
            result_type.fn_type(
                &[
                    self.context.i64_type().into(),
                    self.context.i64_type().into(),
                ],
                false,
            ),
            Some(Linkage::External),
        );
        self.module.add_function(
            "thaw_js_release_handle",
            self.context
                .i8_type()
                .fn_type(&[self.context.i64_type().into()], false),
            Some(Linkage::External),
        );
        self.module.add_function(
            "thaw_js_release_all_handles",
            self.context.i64_type().fn_type(&[], false),
            Some(Linkage::External),
        );
        let handle_result_type = self
            .context
            .struct_type(&[self.context.i64_type().into(), i8_ptr.into()], false);
        self.module.add_function(
            "thaw_js_get_property_result",
            handle_result_type.fn_type(&[self.context.i64_type().into(), i8_ptr.into()], false),
            Some(Linkage::External),
        );
        self.module.add_function(
            "thaw_js_set_property_result",
            handle_result_type.fn_type(
                &[
                    self.context.i64_type().into(),
                    i8_ptr.into(),
                    self.context.i64_type().into(),
                ],
                false,
            ),
            Some(Linkage::External),
        );
        self.module.add_function(
            "thaw_js_call_method_result",
            result_type.fn_type(
                &[self.context.i64_type().into(), i8_ptr.into(), i8_ptr.into()],
                false,
            ),
            Some(Linkage::External),
        );
        self.module.add_function(
            "thaw_js_resolve_handle_result",
            result_type.fn_type(&[self.context.i64_type().into()], false),
            Some(Linkage::External),
        );
        self.module.add_function(
            "thaw_js_call_handle_mixed_result",
            result_type.fn_type(
                &[self.context.i64_type().into(), i8_ptr.into(), i8_ptr.into()],
                false,
            ),
            Some(Linkage::External),
        );
        self.module.add_function(
            "thaw_js_construct_handle_result",
            handle_result_type.fn_type(&[self.context.i64_type().into(), i8_ptr.into()], false),
            Some(Linkage::External),
        );
        self.module
            .add_function("thaw_napi_load", js_load_type, Some(Linkage::External));
        self.module.add_function(
            "thaw_napi_load_named",
            self.context
                .i8_type()
                .fn_type(&[i8_ptr.into(), i8_ptr.into()], false),
            Some(Linkage::External),
        );
        self.module.add_function(
            "thaw_napi_load_embedded_hex",
            self.context
                .i8_type()
                .fn_type(&[i8_ptr.into(), i8_ptr.into()], false),
            Some(Linkage::External),
        );
        self.module.add_function(
            "thaw_napi_call_result",
            js_call_result_type,
            Some(Linkage::External),
        );
        self.module.add_function(
            "thaw_napi_get_export",
            self.context.i64_type().fn_type(&[i8_ptr.into()], false),
            Some(Linkage::External),
        );
        self.module.add_function(
            "thaw_napi_construct_handle_result",
            handle_result_type.fn_type(&[self.context.i64_type().into(), i8_ptr.into()], false),
            Some(Linkage::External),
        );
        self.module.add_function(
            "thaw_napi_call_method_result",
            result_type.fn_type(
                &[self.context.i64_type().into(), i8_ptr.into(), i8_ptr.into()],
                false,
            ),
            Some(Linkage::External),
        );
        self.module.add_function(
            "thaw_napi_get_property_result",
            result_type.fn_type(&[self.context.i64_type().into(), i8_ptr.into()], false),
            Some(Linkage::External),
        );
        self.module.add_function(
            "thaw_napi_set_property_result",
            result_type.fn_type(
                &[self.context.i64_type().into(), i8_ptr.into(), i8_ptr.into()],
                false,
            ),
            Some(Linkage::External),
        );
        self.module.add_function(
            "thaw_napi_call_method_with_callback_result",
            result_type.fn_type(
                &[
                    self.context.i64_type().into(),
                    i8_ptr.into(),
                    i8_ptr.into(),
                    i8_ptr.into(),
                    i8_ptr.into(),
                    self.context.i8_type().into(),
                ],
                false,
            ),
            Some(Linkage::External),
        );
        self.module.add_function(
            "thaw_napi_call_with_callback_result",
            result_type.fn_type(
                &[i8_ptr.into(), i8_ptr.into(), i8_ptr.into(), i8_ptr.into()],
                false,
            ),
            Some(Linkage::External),
        );
        self.module.add_function(
            "thaw_napi_run_async_work",
            self.context.i64_type().fn_type(&[], false),
            Some(Linkage::External),
        );
        self.module.add_function(
            "thaw_napi_poll_async_work",
            self.context.i64_type().fn_type(&[], false),
            Some(Linkage::External),
        );
        self.module.add_function(
            "thaw_napi_unload_all",
            self.context.i8_type().fn_type(&[], false),
            Some(Linkage::External),
        );
        self.module.add_function(
            "thaw_napi_take_fatal_exception",
            self.context.i8_type().fn_type(&[], false),
            Some(Linkage::External),
        );

        let sleep_type = i8_ptr.fn_type(&[i64_type.into()], false);
        self.module
            .add_function("thaw_sleep_ms", sleep_type, Some(Linkage::External));
        self.module.add_function(
            "thaw_http_get_async",
            i8_ptr.fn_type(&[i8_ptr.into()], false),
            Some(Linkage::External),
        );
        let run_until_type = i8_ptr.fn_type(&[i8_ptr.into()], false);
        self.module.add_function(
            "thaw_runtime_run_until_resolved",
            run_until_type,
            Some(Linkage::External),
        );
        self.module.add_function(
            "thaw_runtime_drain_detached",
            i64_type.fn_type(&[], false),
            Some(Linkage::External),
        );
        self.module.add_function(
            "thaw_promise_new",
            i8_ptr.fn_type(&[], false),
            Some(Linkage::External),
        );
        self.module.add_function(
            "thaw_promise_subscribe",
            self.context
                .i8_type()
                .fn_type(&[i8_ptr.into(), i8_ptr.into(), i8_ptr.into()], false),
            Some(Linkage::External),
        );
        self.module.add_function(
            "thaw_promise_resolve",
            self.context
                .i8_type()
                .fn_type(&[i8_ptr.into(), i8_ptr.into()], false),
            Some(Linkage::External),
        );
        self.module.add_function(
            "thaw_promise_reject",
            self.context
                .i8_type()
                .fn_type(&[i8_ptr.into(), i8_ptr.into()], false),
            Some(Linkage::External),
        );
        self.module.add_function(
            "thaw_promise_state",
            self.context.i8_type().fn_type(&[i8_ptr.into()], false),
            Some(Linkage::External),
        );
        self.module.add_function(
            "thaw_promise_destroy",
            self.context.void_type().fn_type(&[i8_ptr.into()], false),
            Some(Linkage::External),
        );
        self.module.add_function(
            "thaw_promise_chain",
            i8_ptr.fn_type(
                &[
                    i8_ptr.into(),
                    i8_ptr.into(),
                    i8_ptr.into(),
                    self.context.i8_type().into(),
                ],
                false,
            ),
            Some(Linkage::External),
        );
        self.module.add_function(
            "thaw_promise_adopt",
            self.context
                .i8_type()
                .fn_type(&[i8_ptr.into(), i8_ptr.into()], false),
            Some(Linkage::External),
        );
        self.module.add_function(
            "thaw_promise_finally",
            i8_ptr.fn_type(&[i8_ptr.into(), i8_ptr.into(), i8_ptr.into()], false),
            Some(Linkage::External),
        );
        self.module.add_function(
            "thaw_promise_finally_adopt",
            self.context.i8_type().fn_type(
                &[
                    i8_ptr.into(),
                    i8_ptr.into(),
                    i8_ptr.into(),
                    self.context.i8_type().into(),
                ],
                false,
            ),
            Some(Linkage::External),
        );
        self.module.add_function(
            "thaw_promise_all_slots",
            i8_ptr.fn_type(&[i8_ptr.into(), i64_type.into(), i64_type.into()], false),
            Some(Linkage::External),
        );
        self.module.add_function(
            "thaw_promise_all_typed",
            i8_ptr.fn_type(&[i8_ptr.into(), i8_ptr.into(), i64_type.into()], false),
            Some(Linkage::External),
        );
        self.module.add_function(
            "thaw_promise_race",
            i8_ptr.fn_type(&[i8_ptr.into(), i64_type.into()], false),
            Some(Linkage::External),
        );
        self.module.add_function(
            "thaw_promise_any",
            i8_ptr.fn_type(&[i8_ptr.into(), i64_type.into()], false),
            Some(Linkage::External),
        );
        self.module.add_function(
            "thaw_promise_all_settled",
            i8_ptr.fn_type(&[i8_ptr.into(), i64_type.into(), i64_type.into()], false),
            Some(Linkage::External),
        );
    }

    fn llvm_symbol_for(name: &str) -> String {
        if name == "main" {
            USER_MAIN_SYMBOL.to_string()
        } else {
            name.to_string()
        }
    }

    fn basic_type(&self, ty: &HirType) -> Result<BasicTypeEnum<'ctx>, String> {
        match ty {
            HirType::F64 => Ok(self.context.f64_type().into()),
            HirType::I64 => Ok(self.context.i64_type().into()),
            HirType::Bool => Ok(self.context.bool_type().into()),
            HirType::Undefined => Ok(self.context.bool_type().into()),
            // Strings and arrays are both represented as a single opaque
            // pointer at the LLVM level; what they point to differs (a
            // C string vs. a [len][elements...] buffer).
            HirType::Str => Ok(self.context.ptr_type(AddressSpace::default()).into()),
            HirType::Array(elem) => {
                self.basic_type(elem)?;
                Ok(self.context.ptr_type(AddressSpace::default()).into())
            }
            HirType::Tuple(elements) => {
                for element in elements {
                    self.basic_type(element)?;
                }
                Ok(self.context.ptr_type(AddressSpace::default()).into())
            }
            // Objects are represented the same way: a single opaque pointer
            // to an arena-allocated buffer of contiguous `f64` fields (see
            // `compile_object_lit`). Unlike arrays there's no length
            // header -- field count/order is static, part of the type.
            // Every field is one word (8 bytes on this target) regardless
            // of its own type: `f64`/`bool`/pointer values (Str/Array/
            // Object/Json) are all word-sized, so a field can be any type
            // `basic_type` itself accepts -- including another `Object`,
            // recursively. This call validates each field is representable
            // at all (erroring on e.g. `Promise`); it doesn't need to
            // guard against cycles itself since a self-referential
            // interface is already rejected at lowering time
            // (`thaw_hir::lower::resolve_interface`), so a genuinely
            // cyclic `HirType::Object` should never reach codegen.
            HirType::Object(fields) => {
                for (name, ty) in fields {
                    self.basic_type(ty)
                        .map_err(|e| format!("object field `{name}`: {e}"))?;
                }
                Ok(self.context.ptr_type(AddressSpace::default()).into())
            }
            // A `Json` value is an opaque pointer to a boxed, dynamically-
            // typed `serde_json::Value` (thaw-std), same representation
            // family as everything else -- only `thaw_json_*` (thaw-std)
            // ever dereferences it.
            HirType::Json | HirType::Dictionary(_) => {
                Ok(self.context.ptr_type(AddressSpace::default()).into())
            }
            HirType::JsValue => Ok(self.context.i64_type().into()),
            HirType::Null => Ok(self.context.bool_type().into()),
            HirType::Function(_, _) | HirType::CallableFunction(..) => {
                Ok(self.context.ptr_type(AddressSpace::default()).into())
            }
            HirType::Promise(_) => Ok(self.context.ptr_type(AddressSpace::default()).into()),
            HirType::Optional(payload) => {
                let payload = self.basic_type(payload)?;
                Ok(self
                    .context
                    .struct_type(&[self.context.bool_type().into(), payload], false)
                    .into())
            }
            HirType::Nullable(payload) => {
                let payload = self.basic_type(payload)?;
                Ok(self
                    .context
                    .struct_type(&[self.context.bool_type().into(), payload], false)
                    .into())
            }
            HirType::Nullish(payload) => {
                let payload = self.basic_type(payload)?;
                Ok(self
                    .context
                    .struct_type(&[self.context.i8_type().into(), payload], false)
                    .into())
            }
            HirType::Union(elements) => {
                if elements.is_empty() {
                    return Err("empty union type is not supported".into());
                }
                for element in elements {
                    self.basic_type(element)?;
                }
                Ok(self
                    .context
                    .struct_type(
                        &[
                            self.context.i8_type().into(),
                            self.context.i64_type().into(),
                        ],
                        false,
                    )
                    .into())
            }
            other => Err(format!(
                "Phase 1/2 codegen does not support type {other:?} yet"
            )),
        }
    }

    fn function_type(
        &self,
        params: &[HirType],
        ret: &HirType,
    ) -> Result<FunctionType<'ctx>, String> {
        let mut lowered = vec![BasicMetadataTypeEnum::from(
            self.context.ptr_type(AddressSpace::default()),
        )];
        lowered.extend(
            params
                .iter()
                .map(|ty| self.basic_type(ty).map(BasicMetadataTypeEnum::from))
                .collect::<Result<Vec<_>, _>>()?,
        );
        if *ret == HirType::Void {
            Ok(self.context.void_type().fn_type(&lowered, false))
        } else {
            Ok(self.basic_type(ret)?.fn_type(&lowered, false))
        }
    }

    fn compile_ignored_this_adapter(
        &mut self,
        target: FunctionValue<'ctx>,
        params: &[HirType],
        ret: &HirType,
        name: &str,
    ) -> Result<FunctionValue<'ctx>, String> {
        let mut this_params = Vec::with_capacity(params.len() + 1);
        this_params.push(HirType::I64);
        this_params.extend_from_slice(params);
        let adapter = self.module.add_function(
            name,
            self.function_type(&this_params, ret)?,
            Some(Linkage::Internal),
        );
        let parent = self
            .builder
            .get_insert_block()
            .ok_or("this-aware adapter must be emitted inside a function")?;
        let entry = self.context.append_basic_block(adapter, "entry");
        self.builder.position_at_end(entry);
        let mut arguments = vec![BasicMetadataValueEnum::from(
            adapter.get_nth_param(0).unwrap(),
        )];
        arguments.extend(
            adapter
                .get_param_iter()
                .skip(2)
                .map(BasicMetadataValueEnum::from),
        );
        let call = self
            .builder
            .build_call(target, &arguments, "invoke_ignoring_this")
            .map_err(|error| error.to_string())?;
        if *ret == HirType::Void {
            self.builder
                .build_return(None)
                .map_err(|error| error.to_string())?;
        } else {
            let value = call
                .try_as_basic_value()
                .basic()
                .ok_or("this-aware adapter target returned no value")?;
            self.builder
                .build_return(Some(&value))
                .map_err(|error| error.to_string())?;
        }
        self.builder.position_at_end(parent);
        Ok(adapter)
    }

    fn allocate_variable_cell(
        &self,
        ty: BasicTypeEnum<'ctx>,
        name: &str,
    ) -> Result<PointerValue<'ctx>, String> {
        let i64_type = self.context.i64_type();
        let alloc = self.module.get_function("thaw_arena_alloc").unwrap();
        let size = ty
            .size_of()
            .ok_or_else(|| format!("variable `{name}` has an unsized LLVM type"))?;
        let cell = self
            .builder
            .build_call(
                alloc,
                &[size.into(), i64_type.const_int(8, false).into()],
                &format!("{name}_cell"),
            )
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .ok_or("thaw_arena_alloc did not return a variable cell")?
            .into_pointer_value();
        Ok(cell)
    }

    fn declare_function(&mut self, func: &HirFunction) -> Result<FunctionValue<'ctx>, String> {
        let param_types = func
            .params
            .iter()
            .map(|p| self.basic_type(&p.ty).map(BasicMetadataTypeEnum::from))
            .collect::<Result<Vec<_>, _>>()?;

        let frame_split = self.frame_await_plan(func)?.is_some();
        let fn_type = if frame_split {
            self.context
                .ptr_type(AddressSpace::default())
                .fn_type(&param_types, false)
        } else {
            match &func.ret {
                HirType::Void => self.context.void_type().fn_type(&param_types, false),
                ret => self.basic_type(ret)?.fn_type(&param_types, false),
            }
        };

        let symbol = Self::llvm_symbol_for(&func.name);
        Ok(self.module.add_function(&symbol, fn_type, None))
    }

    /// Declares an ambient (`declare function`) signature as an `extern
    /// "C"` symbol -- see docs/design/bridge.md sections 5/6. The final
    /// link step must resolve it from somewhere else (a real native
    /// library today; thaw-registry eventually).
    ///
    /// The declared LLVM parameter list is *not* simply one slot per HIR
    /// parameter: `Array`/`Object` params are expanded into the real C ABI
    /// shape a native library actually expects (see `ffi_param_types`),
    /// matching the argument list `compile_ffi_call` builds at each call
    /// site. Aggregate returns use an explicit, portable C shape and are
    /// copied into Thaw's arena representation after the call:
    /// `number[]` is `{ const double *data; int64_t len; }`, while an object
    /// is a value struct whose fields remain in declaration order.
    fn declare_extern_function(
        &mut self,
        sig: &FfiSignature,
    ) -> Result<FunctionValue<'ctx>, String> {
        let supports_owned_return =
            Self::supports_owned_ffi_return(&sig.ret, sig.aggregate_return_abi);
        if sig.return_ownership != FfiOwnership::Borrowed && !supports_owned_return {
            return Err(format!(
                "FFI function `{}` uses non-borrowed return ownership, which is currently supported only for string, number[], and portable/packed object returns containing supported string or number[] leaves",
                sig.symbol
            ));
        }
        if sig.error_abi == FfiErrorAbi::Direct && sig.error_ownership != FfiOwnership::Borrowed {
            return Err(format!(
                "FFI function `{}` cannot configure error ownership with the direct error ABI",
                sig.symbol
            ));
        }
        for ownership in [&sig.return_ownership, &sig.error_ownership] {
            let destroy = match ownership {
                FfiOwnership::Owned { destroy } => Some(destroy),
                FfiOwnership::ArenaCopy {
                    destroy: Some(destroy),
                } => Some(destroy),
                _ => None,
            };
            if let Some(destroy) = destroy {
                if self.module.get_function(destroy).is_none() {
                    let ptr_ty = self.context.ptr_type(AddressSpace::default());
                    let fn_ty = self.context.void_type().fn_type(&[ptr_ty.into()], false);
                    self.module
                        .add_function(destroy, fn_ty, Some(Linkage::External));
                }
            }
        }
        let param_types = self.ffi_param_types(&sig.params, &sig.param_string_abis)?;

        let return_type = self.ffi_call_return_type(sig)?;
        let fn_type = if Self::uses_indirect_ffi_return(sig) {
            let mut indirect_params = Vec::with_capacity(param_types.len() + 1);
            indirect_params.push(
                self.context
                    .ptr_type(AddressSpace::default())
                    .as_basic_type_enum()
                    .into(),
            );
            indirect_params.extend(param_types);
            self.context
                .void_type()
                .fn_type(&indirect_params, sig.variadic.is_some())
        } else if let Some(return_type) = return_type {
            return_type.fn_type(&param_types, sig.variadic.is_some())
        } else {
            self.context
                .void_type()
                .fn_type(&param_types, sig.variadic.is_some())
        };

        let function = self
            .module
            .add_function(&sig.symbol, fn_type, Some(Linkage::External));
        function.set_call_conventions(Self::ffi_calling_convention(sig.calling_convention));
        Ok(function)
    }
}

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

include!("hir_codegen/async_frames.rs");

impl<'ctx> HirCompiler<'ctx> {}

include!("hir_codegen/statements.rs");

impl<'ctx> HirCompiler<'ctx> {}

include!("hir_codegen/values.rs");

impl<'ctx> HirCompiler<'ctx> {}

include!("hir_codegen/collections.rs");

impl<'ctx> HirCompiler<'ctx> {}

include!("hir_codegen/builtins.rs");

impl<'ctx> HirCompiler<'ctx> {}

include!("hir_codegen/json_values.rs");

impl<'ctx> HirCompiler<'ctx> {}

include!("hir_codegen/dynamic_host.rs");

impl<'ctx> HirCompiler<'ctx> {}

include!("hir_codegen/json_bridge.rs");

impl<'ctx> HirCompiler<'ctx> {}

include!("hir_codegen/operators.rs");

impl<'ctx> HirCompiler<'ctx> {}

include!("hir_codegen/invocations.rs");

impl<'ctx> HirCompiler<'ctx> {}

include!("hir_codegen/promises.rs");

impl<'ctx> HirCompiler<'ctx> {
    fn apply_ffi_string_ownership(
        &mut self,
        source: PointerValue<'ctx>,
        ownership: &FfiOwnership,
        name: &str,
    ) -> Result<PointerValue<'ctx>, String> {
        if *ownership == FfiOwnership::Borrowed {
            return Ok(source);
        }

        let function = self.current_function();
        let null_bb = self
            .context
            .append_basic_block(function, &format!("{name}_null"));
        let copy_bb = self
            .context
            .append_basic_block(function, &format!("{name}_copy"));
        let merge_bb = self
            .context
            .append_basic_block(function, &format!("{name}_merge"));
        let is_null = self
            .builder
            .build_is_null(source, &format!("{name}_is_null"))
            .map_err(|error| error.to_string())?;
        self.builder
            .build_conditional_branch(is_null, null_bb, copy_bb)
            .map_err(|error| error.to_string())?;

        self.builder.position_at_end(null_bb);
        self.builder
            .build_unconditional_branch(merge_bb)
            .map_err(|error| error.to_string())?;

        self.builder.position_at_end(copy_bb);
        let strlen = self.module.get_function("strlen").unwrap();
        let length = self
            .builder
            .build_call(strlen, &[source.into()], &format!("{name}_length"))
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .unwrap()
            .into_int_value();
        let size = self
            .builder
            .build_int_add(
                length,
                self.context.i64_type().const_int(1, false),
                &format!("{name}_size"),
            )
            .map_err(|error| error.to_string())?;
        let arena_alloc = self.module.get_function("thaw_arena_alloc").unwrap();
        let copied = self
            .builder
            .build_call(
                arena_alloc,
                &[
                    size.into(),
                    self.context.i64_type().const_int(1, false).into(),
                ],
                &format!("{name}_alloc"),
            )
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .unwrap()
            .into_pointer_value();
        let memcpy = self.module.get_function("memcpy").unwrap();
        self.builder
            .build_call(
                memcpy,
                &[copied.into(), source.into(), size.into()],
                &format!("{name}_memcpy"),
            )
            .map_err(|error| error.to_string())?;
        let destroy = match ownership {
            FfiOwnership::Owned { destroy } => Some(destroy),
            FfiOwnership::ArenaCopy { destroy } => destroy.as_ref(),
            FfiOwnership::Borrowed => None,
        };
        if let Some(destroy) = destroy {
            let destroy_fn = self
                .module
                .get_function(destroy)
                .ok_or_else(|| format!("FFI ownership destructor `{destroy}` was not declared"))?;
            self.builder
                .build_call(destroy_fn, &[source.into()], &format!("{name}_destroy"))
                .map_err(|error| error.to_string())?;
        }
        self.builder
            .build_unconditional_branch(merge_bb)
            .map_err(|error| error.to_string())?;

        self.builder.position_at_end(merge_bb);
        let ptr_ty = self.context.ptr_type(AddressSpace::default());
        let phi = self
            .builder
            .build_phi(ptr_ty, &format!("{name}_owned"))
            .map_err(|error| error.to_string())?;
        let null = ptr_ty.const_null();
        phi.add_incoming(&[(&null, null_bb), (&copied, copy_bb)]);
        Ok(phi.as_basic_value().into_pointer_value())
    }

    fn marshal_ffi_return(
        &mut self,
        value: BasicValueEnum<'ctx>,
        ty: &HirType,
        string_abi: FfiStringAbi,
        ownership: &FfiOwnership,
        aggregate_abi: FfiAggregateAbi,
        aggregate_layout: Option<&FfiAggregateLayout>,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        match ty {
            HirType::Str if string_abi == FfiStringAbi::PointerLength => {
                let native = value.into_struct_value();
                let source = self
                    .builder
                    .build_extract_value(native, 0, "ffi_string_data")
                    .map_err(|error| error.to_string())?
                    .into_pointer_value();
                let length = self
                    .builder
                    .build_extract_value(native, 1, "ffi_string_length")
                    .map_err(|error| error.to_string())?
                    .into_int_value();
                let i64_type = self.context.i64_type();
                let size = self
                    .builder
                    .build_int_add(length, i64_type.const_int(1, false), "ffi_string_size")
                    .map_err(|error| error.to_string())?;
                let arena_alloc = self.module.get_function("thaw_arena_alloc").unwrap();
                let result = self
                    .builder
                    .build_call(
                        arena_alloc,
                        &[size.into(), i64_type.const_int(1, false).into()],
                        "ffi_string_alloc",
                    )
                    .map_err(|error| error.to_string())?
                    .try_as_basic_value()
                    .basic()
                    .unwrap()
                    .into_pointer_value();
                let memcpy = self.module.get_function("memcpy").unwrap();
                self.builder
                    .build_call(
                        memcpy,
                        &[result.into(), source.into(), length.into()],
                        "ffi_string_copy",
                    )
                    .map_err(|error| error.to_string())?;
                let terminator = unsafe {
                    self.builder
                        .build_in_bounds_gep(
                            self.context.i8_type(),
                            result,
                            &[length],
                            "ffi_string_terminator",
                        )
                        .map_err(|error| error.to_string())?
                };
                self.builder
                    .build_store(terminator, self.context.i8_type().const_zero())
                    .map_err(|error| error.to_string())?;
                let destroy = match ownership {
                    FfiOwnership::Owned { destroy } => Some(destroy),
                    FfiOwnership::ArenaCopy { destroy } => destroy.as_ref(),
                    FfiOwnership::Borrowed => None,
                };
                if let Some(destroy) = destroy {
                    let destroy_fn = self.module.get_function(destroy).ok_or_else(|| {
                        format!("FFI ownership destructor `{destroy}` was not declared")
                    })?;
                    self.builder
                        .build_call(destroy_fn, &[source.into()], "ffi_string_destroy")
                        .map_err(|error| error.to_string())?;
                }
                Ok(result.into())
            }
            HirType::Array(element) if **element == HirType::F64 && value.is_struct_value() => {
                let native = value.into_struct_value();
                let data = self
                    .builder
                    .build_extract_value(native, 0, "ffi_array_data")
                    .map_err(|error| error.to_string())?
                    .into_pointer_value();
                let length = self
                    .builder
                    .build_extract_value(native, 1, "ffi_array_length")
                    .map_err(|error| error.to_string())?
                    .into_int_value();
                let i64_type = self.context.i64_type();
                let bytes = self
                    .builder
                    .build_int_mul(
                        length,
                        i64_type.const_int(ARRAY_ELEM_BYTES, false),
                        "ffi_array_bytes",
                    )
                    .map_err(|error| error.to_string())?;
                let size = self
                    .builder
                    .build_int_add(
                        bytes,
                        i64_type.const_int(ARRAY_HEADER_BYTES, false),
                        "ffi_array_size",
                    )
                    .map_err(|error| error.to_string())?;
                let arena_alloc = self.module.get_function("thaw_arena_alloc").unwrap();
                let result = self
                    .builder
                    .build_call(
                        arena_alloc,
                        &[size.into(), i64_type.const_int(8, false).into()],
                        "ffi_array_alloc",
                    )
                    .map_err(|error| error.to_string())?
                    .try_as_basic_value()
                    .basic()
                    .unwrap()
                    .into_pointer_value();
                self.builder
                    .build_store(result, length)
                    .map_err(|error| error.to_string())?;
                let elements = unsafe {
                    self.builder
                        .build_in_bounds_gep(
                            self.context.i8_type(),
                            result,
                            &[i64_type.const_int(ARRAY_HEADER_BYTES, false)],
                            "ffi_array_elements",
                        )
                        .map_err(|error| error.to_string())?
                };
                let memcpy = self.module.get_function("memcpy").unwrap();
                self.builder
                    .build_call(
                        memcpy,
                        &[elements.into(), data.into(), bytes.into()],
                        "ffi_array_copy",
                    )
                    .map_err(|error| error.to_string())?;
                let destroy = match ownership {
                    FfiOwnership::Owned { destroy } => Some(destroy),
                    FfiOwnership::ArenaCopy { destroy } => destroy.as_ref(),
                    FfiOwnership::Borrowed => None,
                };
                if let Some(destroy) = destroy {
                    let destroy_fn = self.module.get_function(destroy).ok_or_else(|| {
                        format!("FFI ownership destructor `{destroy}` was not declared")
                    })?;
                    self.builder
                        .build_call(destroy_fn, &[data.into()], "ffi_array_destroy")
                        .map_err(|error| error.to_string())?;
                }
                Ok(result.into())
            }
            HirType::Object(fields)
                if value.is_struct_value() && aggregate_abi != FfiAggregateAbi::Internal =>
            {
                let native = value.into_struct_value();
                let field_indices = aggregate_layout
                    .map(|layout| {
                        self.ffi_explicit_object_type(fields, layout)
                            .map(|(_, indices)| indices)
                    })
                    .transpose()?;
                let i64_type = self.context.i64_type();
                let arena_alloc = self.module.get_function("thaw_arena_alloc").unwrap();
                let result = self
                    .builder
                    .build_call(
                        arena_alloc,
                        &[
                            i64_type
                                .const_int(object_storage_bytes(fields), false)
                                .into(),
                            i64_type.const_int(OBJECT_FIELD_BYTES, false).into(),
                        ],
                        "ffi_object_alloc",
                    )
                    .map_err(|error| error.to_string())?
                    .try_as_basic_value()
                    .basic()
                    .unwrap()
                    .into_pointer_value();
                for (index, (name, field_ty)) in fields.iter().enumerate() {
                    let explicit_field = field_indices.as_ref().map(|indices| &indices[index]);
                    let mut field = self
                        .builder
                        .build_extract_value(
                            native,
                            explicit_field.map_or(index as u32, |field| field.native_index),
                            &format!("ffi_{name}"),
                        )
                        .map_err(|error| error.to_string())?;
                    if let Some(bitfield) = explicit_field.and_then(|field| field.bitfield.as_ref())
                    {
                        let storage = field.into_int_value();
                        let storage_type = storage.get_type();
                        let shifted = self
                            .builder
                            .build_right_shift(
                                storage,
                                storage_type.const_int(u64::from(bitfield.bit_offset), false),
                                false,
                                &format!("ffi_{name}_shift"),
                            )
                            .map_err(|error| error.to_string())?;
                        let storage_bits = storage_type.get_bit_width();
                        let width = u32::from(bitfield.bit_width);
                        let mask = if width == 64 {
                            u64::MAX
                        } else {
                            (1_u64 << width) - 1
                        };
                        let masked = self
                            .builder
                            .build_and(
                                shifted,
                                storage_type.const_int(mask, false),
                                &format!("ffi_{name}_mask"),
                            )
                            .map_err(|error| error.to_string())?;
                        field = match field_ty {
                            HirType::Bool => self
                                .builder
                                .build_int_compare(
                                    IntPredicate::NE,
                                    masked,
                                    storage_type.const_zero(),
                                    &format!("ffi_{name}_bool"),
                                )
                                .map_err(|error| error.to_string())?
                                .into(),
                            HirType::F64 => {
                                let integer = if bitfield.signed && width < storage_bits {
                                    let extend = storage_type
                                        .const_int(u64::from(storage_bits - width), false);
                                    let left = self
                                        .builder
                                        .build_left_shift(
                                            masked,
                                            extend,
                                            &format!("ffi_{name}_sign_left"),
                                        )
                                        .map_err(|error| error.to_string())?;
                                    self.builder
                                        .build_right_shift(
                                            left,
                                            extend,
                                            true,
                                            &format!("ffi_{name}_sign_right"),
                                        )
                                        .map_err(|error| error.to_string())?
                                } else {
                                    masked
                                };
                                if bitfield.signed {
                                    self.builder.build_signed_int_to_float(
                                        integer,
                                        self.context.f64_type(),
                                        &format!("ffi_{name}_number"),
                                    )
                                } else {
                                    self.builder.build_unsigned_int_to_float(
                                        integer,
                                        self.context.f64_type(),
                                        &format!("ffi_{name}_number"),
                                    )
                                }
                                .map_err(|error| error.to_string())?
                                .into()
                            }
                            _ => unreachable!("HIR validates bitfield field types"),
                        };
                    } else if explicit_field.is_some() && *field_ty == HirType::Bool {
                        let native_bool = field.into_int_value();
                        field = self
                            .builder
                            .build_int_compare(
                                IntPredicate::NE,
                                native_bool,
                                native_bool.get_type().const_zero(),
                                &format!("ffi_{name}_bool"),
                            )
                            .map_err(|error| error.to_string())?
                            .into();
                    }
                    field = match field_ty {
                        HirType::Str if *ownership != FfiOwnership::Borrowed => self
                            .apply_ffi_string_ownership(
                                field.into_pointer_value(),
                                ownership,
                                &format!("ffi_{name}"),
                            )?
                            .into(),
                        HirType::Object(_) | HirType::Array(_) if field.is_struct_value() => self
                            .marshal_ffi_return(
                            field,
                            field_ty,
                            FfiStringAbi::NullTerminated,
                            ownership,
                            aggregate_abi,
                            aggregate_layout
                                .and_then(|layout| layout.field_layouts[index].as_deref()),
                        )?,
                        _ => field,
                    };
                    let slot = unsafe {
                        self.builder
                            .build_in_bounds_gep(
                                self.context.i8_type(),
                                result,
                                &[i64_type.const_int(object_field_offset(fields, index), false)],
                                "ffi_object_field",
                            )
                            .map_err(|error| error.to_string())?
                    };
                    self.builder
                        .build_store(slot, field)
                        .map_err(|error| error.to_string())?;
                }
                Ok(result.into())
            }
            _ => Ok(value),
        }
    }

    fn unpack_ffi_register_return(
        &mut self,
        value: BasicValueEnum<'ctx>,
        sig: &FfiSignature,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let layout = sig
            .aggregate_return_layout
            .as_ref()
            .expect("register return has an explicit layout");
        let native_type = self.ffi_return_type(
            &sig.ret,
            sig.return_string_abi,
            sig.aggregate_return_abi,
            Some(layout),
        )?;
        let slot = self
            .builder
            .build_alloca(native_type, "ffi_register_result_storage")
            .map_err(|error| error.to_string())?;
        slot.as_instruction_value()
            .expect("alloca is an instruction")
            .set_alignment(layout.alignment)
            .map_err(|error| error.to_string())?;
        let registers = value.into_struct_value();
        for (index, _) in layout.register_classes.iter().enumerate() {
            let register = self
                .builder
                .build_extract_value(registers, index as u32, "ffi_result_register")
                .map_err(|error| error.to_string())?;
            let target = unsafe {
                self.builder
                    .build_in_bounds_gep(
                        self.context.i8_type(),
                        slot,
                        &[self.context.i64_type().const_int((index * 8) as u64, false)],
                        "ffi_result_register_slot",
                    )
                    .map_err(|error| error.to_string())?
            };
            self.builder
                .build_store(target, register)
                .map_err(|error| error.to_string())?;
        }
        self.builder
            .build_load(native_type, slot, "ffi_register_result_value")
            .map_err(|error| error.to_string())
    }

    /// `HirExpr::FfiCall` -- an ambient `declare function` call (see
    /// docs/design/bridge.md). By the time codegen sees this, the symbol
    /// is already declared (`declare_extern_function` ran in the
    /// `compile_program` pre-pass) with the *adapted* C ABI parameter list
    /// `ffi_param_types` computed, so unlike a normal Thaw-to-Thaw call
    /// (`build_call_with`), an `Array`/`Object` argument here must be
    /// unpacked into that same adapted shape before the call -- each match
    /// arm below has a matching arm in `ffi_param_types`'s doc comment.
    fn append_ffi_aggregate_vararg(
        &mut self,
        value: BasicValueEnum<'ctx>,
        ty: &HirType,
        output: &mut Vec<BasicMetadataValueEnum<'ctx>>,
    ) -> Result<(), String> {
        match ty {
            HirType::F64 | HirType::Str | HirType::JsValue => output.push(value.into()),
            HirType::Bool => {
                let promoted = self
                    .builder
                    .build_int_z_extend(
                        value.into_int_value(),
                        self.context.i32_type(),
                        "ffi_aggregate_vararg_bool",
                    )
                    .map_err(|error| error.to_string())?;
                output.push(promoted.into());
            }
            HirType::Array(element)
                if matches!(
                    element.as_ref(),
                    HirType::F64 | HirType::Str | HirType::JsValue
                ) =>
            {
                let base = value.into_pointer_value();
                let i64_type = self.context.i64_type();
                let length = self
                    .builder
                    .build_load(i64_type, base, "ffi_aggregate_vararg_array_length")
                    .map_err(|error| error.to_string())?;
                let data = unsafe {
                    self.builder
                        .build_in_bounds_gep(
                            self.context.i8_type(),
                            base,
                            &[i64_type.const_int(ARRAY_HEADER_BYTES, false)],
                            "ffi_aggregate_vararg_array_data",
                        )
                        .map_err(|error| error.to_string())?
                };
                output.push(data.into());
                output.push(length.into());
            }
            HirType::Array(element) if element.as_ref() == &HirType::Bool => {
                let base = value.into_pointer_value();
                let i64_type = self.context.i64_type();
                let length = self
                    .builder
                    .build_load(i64_type, base, "ffi_bool_vararg_array_length")
                    .map_err(|error| error.to_string())?
                    .into_int_value();
                let byte_count = self
                    .builder
                    .build_int_mul(
                        length,
                        i64_type.const_int(4, false),
                        "ffi_bool_vararg_byte_count",
                    )
                    .map_err(|error| error.to_string())?;
                let empty = self
                    .builder
                    .build_int_compare(
                        IntPredicate::EQ,
                        length,
                        i64_type.const_zero(),
                        "ffi_bool_vararg_empty",
                    )
                    .map_err(|error| error.to_string())?;
                let allocation_size = self
                    .builder
                    .build_select(
                        empty,
                        i64_type.const_int(4, false),
                        byte_count,
                        "ffi_bool_vararg_allocation_size",
                    )
                    .map_err(|error| error.to_string())?;
                let packed = self
                    .builder
                    .build_call(
                        self.module.get_function("thaw_arena_alloc").unwrap(),
                        &[allocation_size.into(), i64_type.const_int(4, false).into()],
                        "ffi_bool_vararg_alloc",
                    )
                    .map_err(|error| error.to_string())?
                    .try_as_basic_value()
                    .basic()
                    .ok_or("thaw_arena_alloc returned no boolean-array pointer")?
                    .into_pointer_value();
                let index_slot = self
                    .builder
                    .build_alloca(i64_type, "ffi_bool_vararg_index")
                    .map_err(|error| error.to_string())?;
                self.builder
                    .build_store(index_slot, i64_type.const_zero())
                    .map_err(|error| error.to_string())?;
                let function = self
                    .builder
                    .get_insert_block()
                    .and_then(|block| block.get_parent())
                    .ok_or("boolean vararg marshalling is outside a function")?;
                let condition = self
                    .context
                    .append_basic_block(function, "ffi_bool_vararg_condition");
                let body = self
                    .context
                    .append_basic_block(function, "ffi_bool_vararg_body");
                let done = self
                    .context
                    .append_basic_block(function, "ffi_bool_vararg_done");
                self.builder
                    .build_unconditional_branch(condition)
                    .map_err(|error| error.to_string())?;
                self.builder.position_at_end(condition);
                let index = self
                    .builder
                    .build_load(i64_type, index_slot, "ffi_bool_vararg_current")
                    .map_err(|error| error.to_string())?
                    .into_int_value();
                let has_element = self
                    .builder
                    .build_int_compare(
                        IntPredicate::ULT,
                        index,
                        length,
                        "ffi_bool_vararg_has_element",
                    )
                    .map_err(|error| error.to_string())?;
                self.builder
                    .build_conditional_branch(has_element, body, done)
                    .map_err(|error| error.to_string())?;
                self.builder.position_at_end(body);
                let source_offset = self
                    .builder
                    .build_int_mul(
                        index,
                        i64_type.const_int(ARRAY_ELEM_BYTES, false),
                        "ffi_bool_vararg_source_offset",
                    )
                    .map_err(|error| error.to_string())?;
                let source_offset = self
                    .builder
                    .build_int_add(
                        source_offset,
                        i64_type.const_int(ARRAY_HEADER_BYTES, false),
                        "ffi_bool_vararg_source_data_offset",
                    )
                    .map_err(|error| error.to_string())?;
                let source = unsafe {
                    self.builder
                        .build_in_bounds_gep(
                            self.context.i8_type(),
                            base,
                            &[source_offset],
                            "ffi_bool_vararg_source",
                        )
                        .map_err(|error| error.to_string())?
                };
                let boolean = self
                    .builder
                    .build_load(self.context.bool_type(), source, "ffi_bool_vararg_value")
                    .map_err(|error| error.to_string())?
                    .into_int_value();
                let promoted = self
                    .builder
                    .build_int_z_extend(
                        boolean,
                        self.context.i32_type(),
                        "ffi_bool_vararg_value_i32",
                    )
                    .map_err(|error| error.to_string())?;
                let target_offset = self
                    .builder
                    .build_int_mul(
                        index,
                        i64_type.const_int(4, false),
                        "ffi_bool_vararg_target_offset",
                    )
                    .map_err(|error| error.to_string())?;
                let target = unsafe {
                    self.builder
                        .build_in_bounds_gep(
                            self.context.i8_type(),
                            packed,
                            &[target_offset],
                            "ffi_bool_vararg_target",
                        )
                        .map_err(|error| error.to_string())?
                };
                self.builder
                    .build_store(target, promoted)
                    .map_err(|error| error.to_string())?;
                let next = self
                    .builder
                    .build_int_add(index, i64_type.const_int(1, false), "ffi_bool_vararg_next")
                    .map_err(|error| error.to_string())?;
                self.builder
                    .build_store(index_slot, next)
                    .map_err(|error| error.to_string())?;
                self.builder
                    .build_unconditional_branch(condition)
                    .map_err(|error| error.to_string())?;
                self.builder.position_at_end(done);
                output.push(packed.into());
                output.push(length.into());
            }
            HirType::Optional(payload) | HirType::Nullable(payload) => {
                let tagged = value.into_struct_value();
                let tag = self
                    .builder
                    .build_extract_value(tagged, 0, "ffi_aggregate_vararg_optional_tag")
                    .map_err(|error| error.to_string())?
                    .into_int_value();
                let promoted = self
                    .builder
                    .build_int_z_extend(
                        tag,
                        self.context.i32_type(),
                        "ffi_aggregate_vararg_optional_tag_i32",
                    )
                    .map_err(|error| error.to_string())?;
                output.push(promoted.into());
                let value = self
                    .builder
                    .build_extract_value(tagged, 1, "ffi_aggregate_vararg_optional_value")
                    .map_err(|error| error.to_string())?;
                self.append_ffi_aggregate_vararg(value, payload, output)?;
            }
            HirType::Nullish(payload) => {
                let tagged = value.into_struct_value();
                let tag = self
                    .builder
                    .build_extract_value(tagged, 0, "ffi_aggregate_vararg_nullish_tag")
                    .map_err(|error| error.to_string())?
                    .into_int_value();
                let promoted = self
                    .builder
                    .build_int_z_extend(
                        tag,
                        self.context.i32_type(),
                        "ffi_aggregate_vararg_nullish_tag_i32",
                    )
                    .map_err(|error| error.to_string())?;
                output.push(promoted.into());
                let value = self
                    .builder
                    .build_extract_value(tagged, 1, "ffi_aggregate_vararg_nullish_value")
                    .map_err(|error| error.to_string())?;
                self.append_ffi_aggregate_vararg(value, payload, output)?;
            }
            HirType::Object(fields) => {
                let base = value.into_pointer_value();
                for (index, (_, field_type)) in fields.iter().enumerate() {
                    let field_pointer = unsafe {
                        self.builder
                            .build_in_bounds_gep(
                                self.context.i8_type(),
                                base,
                                &[self
                                    .context
                                    .i64_type()
                                    .const_int(object_field_offset(fields, index), false)],
                                "ffi_aggregate_vararg_field",
                            )
                            .map_err(|error| error.to_string())?
                    };
                    let field = self
                        .builder
                        .build_load(
                            self.basic_type(field_type)?,
                            field_pointer,
                            "ffi_aggregate_vararg_value",
                        )
                        .map_err(|error| error.to_string())?;
                    self.append_ffi_aggregate_vararg(field, field_type, output)?;
                }
            }
            other => return Err(format!("unsupported aggregate variadic type {other:?}")),
        }
        Ok(())
    }

    fn compile_ffi_call(
        &mut self,
        sig: &FfiSignature,
        args: &[HirExpr],
    ) -> Result<Option<BasicValueEnum<'ctx>>, String> {
        let sig = self
            .ffi_signatures
            .get(&sig.symbol)
            .cloned()
            .unwrap_or_else(|| sig.clone());
        let sig = &sig;
        let function = self
            .module
            .get_function(&sig.symbol)
            .expect("extern function was declared in compile_program's pre-pass");

        let mut compiled_args: Vec<BasicMetadataValueEnum<'ctx>> = Vec::with_capacity(args.len());
        for (index, (param_ty, arg)) in sig.params.iter().zip(args).enumerate() {
            let value = self.compile_expr(arg)?;
            match param_ty {
                HirType::Str
                    if sig.param_string_abis.get(index) == Some(&FfiStringAbi::PointerLength) =>
                {
                    let pointer = value.into_pointer_value();
                    let strlen = self.module.get_function("strlen").unwrap();
                    let length = self
                        .builder
                        .build_call(strlen, &[pointer.into()], "ffi_string_length")
                        .map_err(|error| error.to_string())?
                        .try_as_basic_value()
                        .basic()
                        .unwrap();
                    compiled_args.push(pointer.into());
                    compiled_args.push(length.into());
                }
                // Thaw's own array value is a pointer to `[i64
                // len][f64 elements...]` (`compile_array_lit`) --
                // read the length back out of that header and pass
                // `(elements pointer, len)` instead of the header
                // pointer itself.
                HirType::Array(elem) if **elem == HirType::F64 => {
                    let base_ptr = value.into_pointer_value();
                    let i64_type = self.context.i64_type();
                    let len_val = self
                        .builder
                        .build_load(i64_type, base_ptr, "ffi_arr_len")
                        .map_err(|e| e.to_string())?;
                    let header_offset = i64_type.const_int(ARRAY_HEADER_BYTES, false);
                    let elems_ptr = unsafe {
                        self.builder
                            .build_in_bounds_gep(
                                self.context.i8_type(),
                                base_ptr,
                                &[header_offset],
                                "ffi_arr_elems",
                            )
                            .map_err(|e| e.to_string())?
                    };
                    compiled_args.push(elems_ptr.into());
                    compiled_args.push(len_val.into());
                }
                // Thaw's own object value is a pointer to a flat
                // `[f64 field0]...[f64 fieldN-1]` buffer
                // (`compile_object_lit`) in declared order -- read
                // each field back out and pass it as its own scalar
                // argument, in that same order.
                HirType::Object(fields) => {
                    let base_ptr = value.into_pointer_value();
                    let i64_type = self.context.i64_type();
                    for (i, (field_name, field_ty)) in fields.iter().enumerate() {
                        let field_llvm_ty = self
                            .basic_type(field_ty)
                            .map_err(|e| format!("FFI object field `{field_name}`: {e}"))?;
                        let offset = i64_type.const_int(object_field_offset(fields, i), false);
                        let field_ptr = unsafe {
                            self.builder
                                .build_in_bounds_gep(
                                    self.context.i8_type(),
                                    base_ptr,
                                    &[offset],
                                    "ffi_field_ptr",
                                )
                                .map_err(|e| e.to_string())?
                        };
                        let field_val = self
                            .builder
                            .build_load(field_llvm_ty, field_ptr, "ffi_field")
                            .map_err(|e| e.to_string())?;
                        compiled_args.push(field_val.into());
                    }
                }
                _ => compiled_args.push(value.into()),
            }
        }
        if let Some(variadic) = &sig.variadic {
            for arg in &args[sig.params.len()..] {
                let value = self.compile_expr(arg)?;
                match variadic {
                    HirType::F64 => {
                        let value = value.into_float_value();
                        let converted: BasicValueEnum<'ctx> = match sig.variadic_abi {
                            FfiVariadicAbi::Native => value.into(),
                            FfiVariadicAbi::I32 => self
                                .builder
                                .build_float_to_signed_int(
                                    value,
                                    self.context.i32_type(),
                                    "ffi_vararg_i32",
                                )
                                .map_err(|error| error.to_string())?
                                .into(),
                            FfiVariadicAbi::I64 => self
                                .builder
                                .build_float_to_signed_int(
                                    value,
                                    self.context.i64_type(),
                                    "ffi_vararg_i64",
                                )
                                .map_err(|error| error.to_string())?
                                .into(),
                            FfiVariadicAbi::U32 => self
                                .builder
                                .build_float_to_unsigned_int(
                                    value,
                                    self.context.i32_type(),
                                    "ffi_vararg_u32",
                                )
                                .map_err(|error| error.to_string())?
                                .into(),
                            FfiVariadicAbi::U64 => self
                                .builder
                                .build_float_to_unsigned_int(
                                    value,
                                    self.context.i64_type(),
                                    "ffi_vararg_u64",
                                )
                                .map_err(|error| error.to_string())?
                                .into(),
                        };
                        compiled_args.push(converted.into());
                    }
                    HirType::Bool => {
                        let promoted = self
                            .builder
                            .build_int_z_extend(
                                value.into_int_value(),
                                self.context.i32_type(),
                                "ffi_vararg_bool",
                            )
                            .map_err(|error| error.to_string())?;
                        compiled_args.push(promoted.into());
                    }
                    HirType::Str | HirType::JsValue => compiled_args.push(value.into()),
                    HirType::Array(_)
                    | HirType::Object(_)
                    | HirType::Optional(_)
                    | HirType::Nullable(_)
                    | HirType::Nullish(_) => self
                        .append_ffi_aggregate_vararg(value, variadic, &mut compiled_args)
                        .map_err(|error| format!("FFI function `{}`: {error}", sig.symbol))?,
                    other => {
                        return Err(format!(
                            "FFI function `{}` has unsupported variadic element type {other:?}",
                            sig.symbol
                        ))
                    }
                }
            }
        }

        let indirect_return = if Self::uses_indirect_ffi_return(sig) {
            let return_type = self
                .ffi_call_return_type(sig)?
                .expect("packed aggregate returns always have a value type");
            let slot = self
                .builder
                .build_alloca(return_type, "ffi_indirect_result")
                .map_err(|error| error.to_string())?;
            if let Some(layout) = &sig.aggregate_return_layout {
                slot.as_instruction_value()
                    .expect("alloca is an instruction")
                    .set_alignment(layout.alignment)
                    .map_err(|error| error.to_string())?;
            }
            compiled_args.insert(0, slot.into());
            Some((slot, return_type))
        } else {
            None
        };
        let call_site = self
            .builder
            .build_call(function, &compiled_args, "ffi_calltmp")
            .map_err(|e| e.to_string())?;
        call_site.set_call_convention(Self::ffi_calling_convention(sig.calling_convention));
        let mut returned = if let Some((slot, return_type)) = indirect_return {
            Some(
                self.builder
                    .build_load(return_type, slot, "ffi_indirect_result_value")
                    .map_err(|error| error.to_string())?,
            )
        } else {
            call_site.try_as_basic_value().basic()
        };
        if sig.error_abi == FfiErrorAbi::Direct
            && sig
                .aggregate_return_layout
                .as_ref()
                .is_some_and(|layout| !layout.register_classes.is_empty())
        {
            returned = returned
                .map(|value| self.unpack_ffi_register_return(value, sig))
                .transpose()?;
        }
        if sig.ret == HirType::Void {
            if sig.error_abi == FfiErrorAbi::Direct {
                return Ok(None);
            }
            let result = returned
                .ok_or_else(|| format!("`{}` did not return its error result", sig.symbol))?
                .into_struct_value();
            let error = self
                .builder
                .build_extract_value(result, 0, "ffi_result_error")
                .map_err(|e| e.to_string())?
                .into_pointer_value();
            let error =
                self.apply_ffi_string_ownership(error, &sig.error_ownership, "ffi_error")?;
            self.builder
                .build_store(self.pending_exception().as_pointer_value(), error)
                .map_err(|e| e.to_string())?;
            self.branch_on_pending_exception()?;
            return Ok(None);
        }
        let returned =
            returned.ok_or_else(|| format!("`{}` does not return a value", sig.symbol))?;
        if sig.error_abi == FfiErrorAbi::Direct {
            if sig.ret == HirType::Str && sig.return_string_abi == FfiStringAbi::NullTerminated {
                return self
                    .apply_ffi_string_ownership(
                        returned.into_pointer_value(),
                        &sig.return_ownership,
                        "ffi_return",
                    )
                    .map(BasicValueEnum::from)
                    .map(Some);
            }
            return self
                .marshal_ffi_return(
                    returned,
                    &sig.ret,
                    sig.return_string_abi,
                    &sig.return_ownership,
                    sig.aggregate_return_abi,
                    sig.aggregate_return_layout.as_ref(),
                )
                .map(Some);
        }

        let result = returned.into_struct_value();
        let (value_index, error_index) = self.ffi_result_field_indices(sig)?;
        let value = self
            .builder
            .build_extract_value(result, value_index, "ffi_result_value")
            .map_err(|e| e.to_string())?;
        let error = self
            .builder
            .build_extract_value(result, error_index, "ffi_result_error")
            .map_err(|e| e.to_string())?
            .into_pointer_value();
        let error = self.apply_ffi_string_ownership(error, &sig.error_ownership, "ffi_error")?;
        self.builder
            .build_store(self.pending_exception().as_pointer_value(), error)
            .map_err(|e| e.to_string())?;
        self.branch_on_pending_exception()?;
        if sig.ret == HirType::Str && sig.return_string_abi == FfiStringAbi::NullTerminated {
            return self
                .apply_ffi_string_ownership(
                    value.into_pointer_value(),
                    &sig.return_ownership,
                    "ffi_return",
                )
                .map(BasicValueEnum::from)
                .map(Some);
        }
        self.marshal_ffi_return(
            value,
            &sig.ret,
            sig.return_string_abi,
            &sig.return_ownership,
            sig.aggregate_return_abi,
            sig.aggregate_return_layout.as_ref(),
        )
        .map(Some)
    }

    /// Compiles `args`, calls `function` with them, and extracts the
    /// return value. Shared by `compile_call` and `compile_ffi_call`.
    fn build_call_with(
        &mut self,
        function: FunctionValue<'ctx>,
        args: &[HirExpr],
        name: &str,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let compiled_args = args
            .iter()
            .map(|arg| self.compile_expr(arg).map(BasicMetadataValueEnum::from))
            .collect::<Result<Vec<_>, _>>()?;

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

    /// `console.log` is bridged straight to libc for Phase 0/1: strings go
    /// to `puts`, numbers go through `printf("%g\n", ...)`. The real
    /// `console` implementation belongs in `std/` once Phase 2 gets there.
    fn compile_console_log(&mut self, args: &[HirExpr]) -> Result<BasicValueEnum<'ctx>, String> {
        let [arg] = args else {
            return Err("console.log expects exactly one argument in Phase 0/1".to_string());
        };
        let hir_type = self.expr_hir_type(arg);
        let value = self.compile_expr(arg)?;

        if let Some(HirType::Optional(payload)) = hir_type {
            self.compile_console_tagged(value.into_struct_value(), &payload, "undefined")?;
        } else if let Some(HirType::Nullable(payload)) = hir_type {
            self.compile_console_tagged(value.into_struct_value(), &payload, "null")?;
        } else if let Some(HirType::Nullish(payload)) = hir_type {
            self.compile_console_nullish(value.into_struct_value(), &payload)?;
        } else if let Some(HirType::Union(elements)) = hir_type {
            self.compile_console_union(value.into_struct_value(), &elements)?;
        } else if hir_type == Some(HirType::Undefined) {
            let undefined = self
                .builder
                .build_global_string_ptr("undefined", "undefined_value")
                .map_err(|error| error.to_string())?;
            self.builder
                .build_call(
                    self.module.get_function("puts").unwrap(),
                    &[undefined.as_pointer_value().into()],
                    "puts_undefined_value",
                )
                .map_err(|error| error.to_string())?;
        } else if hir_type == Some(HirType::Null) {
            let null = self
                .builder
                .build_global_string_ptr("null", "null_value")
                .map_err(|error| error.to_string())?;
            self.builder
                .build_call(
                    self.module.get_function("puts").unwrap(),
                    &[null.as_pointer_value().into()],
                    "puts_null_value",
                )
                .map_err(|error| error.to_string())?;
        } else {
            match value {
                BasicValueEnum::PointerValue(ptr) => {
                    let puts = self.module.get_function("puts").unwrap();
                    self.builder
                        .build_call(puts, &[ptr.into()], "putscall")
                        .map_err(|e| e.to_string())?;
                }
                BasicValueEnum::FloatValue(f) => {
                    let format = self
                        .builder
                        .build_global_string_ptr("%g\n", "numfmt")
                        .map_err(|e| e.to_string())?;
                    let printf = self.module.get_function("printf").unwrap();
                    self.builder
                        .build_call(
                            printf,
                            &[format.as_pointer_value().into(), f.into()],
                            "printfcall",
                        )
                        .map_err(|e| e.to_string())?;
                }
                // Our only first-class `IntValue` is `i1` (`HirType::Bool`) --
                // nothing else reaches console.log as a raw `IntValue`.
                BasicValueEnum::IntValue(b) => {
                    let true_str = self
                        .builder
                        .build_global_string_ptr("true", "true_str")
                        .map_err(|e| e.to_string())?;
                    let false_str = self
                        .builder
                        .build_global_string_ptr("false", "false_str")
                        .map_err(|e| e.to_string())?;
                    let selected = self
                        .builder
                        .build_select(
                            b,
                            true_str.as_pointer_value(),
                            false_str.as_pointer_value(),
                            "bool_str",
                        )
                        .map_err(|e| e.to_string())?;
                    let puts = self.module.get_function("puts").unwrap();
                    self.builder
                        .build_call(puts, &[selected.into()], "putscall")
                        .map_err(|e| e.to_string())?;
                }
                other => {
                    return Err(format!(
                        "console.log does not support values of this kind yet: {other:?}"
                    ))
                }
            }
        }

        // Flush immediately -- see the comment on `fflush`'s declaration.
        let fflush_fn = self.module.get_function("fflush").unwrap();
        let null_ptr = self.context.ptr_type(AddressSpace::default()).const_null();
        self.builder
            .build_call(fflush_fn, &[null_ptr.into()], "fflush_call")
            .map_err(|e| e.to_string())?;

        Ok(self.context.i32_type().const_int(0, false).into())
    }

    fn compile_console_union(
        &mut self,
        value: StructValue<'ctx>,
        elements: &[HirType],
    ) -> Result<(), String> {
        if elements.is_empty() {
            return Err("console.log cannot print an empty union".into());
        }
        let tag = self
            .builder
            .build_extract_value(value, 0, "console_union_tag")
            .map_err(|error| error.to_string())?
            .into_int_value();
        let payload = self
            .builder
            .build_extract_value(value, 1, "console_union_payload")
            .map_err(|error| error.to_string())?
            .into_int_value();
        let function = self.current_function();
        let merge = self.context.append_basic_block(function, "union_printed");
        for (index, member) in elements.iter().enumerate() {
            let matched = self
                .context
                .append_basic_block(function, "union_print_member");
            if index + 1 == elements.len() {
                self.builder
                    .build_unconditional_branch(matched)
                    .map_err(|error| error.to_string())?;
            } else {
                let next = self
                    .context
                    .append_basic_block(function, "union_print_next");
                let is_match = self
                    .builder
                    .build_int_compare(
                        IntPredicate::EQ,
                        tag,
                        self.context.i8_type().const_int(index as u64, false),
                        "union_print_tag_match",
                    )
                    .map_err(|error| error.to_string())?;
                self.builder
                    .build_conditional_branch(is_match, matched, next)
                    .map_err(|error| error.to_string())?;
                self.builder.position_at_end(next);
            }
            self.builder.position_at_end(matched);
            let member_value = self.unpack_union_payload(payload, member)?;
            self.compile_console_union_member(member_value, member)?;
            self.builder
                .build_unconditional_branch(merge)
                .map_err(|error| error.to_string())?;
            if index + 1 != elements.len() {
                let next = matched
                    .get_next_basic_block()
                    .ok_or("union console dispatch lost its next comparison block")?;
                self.builder.position_at_end(next);
            }
        }
        self.builder.position_at_end(merge);
        Ok(())
    }

    fn compile_console_union_member(
        &mut self,
        value: BasicValueEnum<'ctx>,
        member: &HirType,
    ) -> Result<(), String> {
        match member {
            HirType::F64 => {
                let format = self
                    .builder
                    .build_global_string_ptr("%g\n", "union_numfmt")
                    .map_err(|error| error.to_string())?;
                self.builder
                    .build_call(
                        self.module.get_function("printf").unwrap(),
                        &[
                            format.as_pointer_value().into(),
                            value.into_float_value().into(),
                        ],
                        "printf_union_number",
                    )
                    .map_err(|error| error.to_string())?;
            }
            HirType::Bool => {
                let yes = self
                    .builder
                    .build_global_string_ptr("true", "union_true")
                    .map_err(|error| error.to_string())?;
                let no = self
                    .builder
                    .build_global_string_ptr("false", "union_false")
                    .map_err(|error| error.to_string())?;
                let selected = self
                    .builder
                    .build_select(
                        value.into_int_value(),
                        yes.as_pointer_value(),
                        no.as_pointer_value(),
                        "union_bool_string",
                    )
                    .map_err(|error| error.to_string())?;
                self.builder
                    .build_call(
                        self.module.get_function("puts").unwrap(),
                        &[selected.into()],
                        "puts_union_bool",
                    )
                    .map_err(|error| error.to_string())?;
            }
            HirType::Str => {
                self.builder
                    .build_call(
                        self.module.get_function("puts").unwrap(),
                        &[value.into_pointer_value().into()],
                        "puts_union_string",
                    )
                    .map_err(|error| error.to_string())?;
            }
            HirType::Undefined => {
                let undefined = self
                    .builder
                    .build_global_string_ptr("undefined", "union_undefined")
                    .map_err(|error| error.to_string())?;
                self.builder
                    .build_call(
                        self.module.get_function("puts").unwrap(),
                        &[undefined.as_pointer_value().into()],
                        "puts_union_undefined",
                    )
                    .map_err(|error| error.to_string())?;
            }
            HirType::Null => {
                let null = self
                    .builder
                    .build_global_string_ptr("null", "union_null")
                    .map_err(|error| error.to_string())?;
                self.builder
                    .build_call(
                        self.module.get_function("puts").unwrap(),
                        &[null.as_pointer_value().into()],
                        "puts_union_null",
                    )
                    .map_err(|error| error.to_string())?;
            }
            HirType::Object(_) | HirType::Json | HirType::Array(_) | HirType::Function(_, _) => {
                let object = self
                    .builder
                    .build_global_string_ptr("[object Object]", "union_object")
                    .map_err(|error| error.to_string())?;
                self.builder
                    .build_call(
                        self.module.get_function("puts").unwrap(),
                        &[object.as_pointer_value().into()],
                        "puts_union_object",
                    )
                    .map_err(|error| error.to_string())?;
            }
            other => return Err(format!("console.log cannot print union member {other:?}")),
        }
        Ok(())
    }

    fn compile_console_tagged(
        &mut self,
        value: StructValue<'ctx>,
        payload_type: &HirType,
        absent_text: &str,
    ) -> Result<(), String> {
        let present = self
            .builder
            .build_extract_value(value, 0, "console_optional_present")
            .map_err(|error| error.to_string())?
            .into_int_value();
        let payload = self
            .builder
            .build_extract_value(value, 1, "console_optional_payload")
            .map_err(|error| error.to_string())?;
        let function = self.current_function();
        let present_block = self.context.append_basic_block(function, "optional_value");
        let absent_block = self
            .context
            .append_basic_block(function, "optional_undefined");
        let merge_block = self
            .context
            .append_basic_block(function, "optional_printed");
        self.builder
            .build_conditional_branch(present, present_block, absent_block)
            .map_err(|error| error.to_string())?;

        self.builder.position_at_end(absent_block);
        let absent = self
            .builder
            .build_global_string_ptr(absent_text, "tagged_absent_string")
            .map_err(|error| error.to_string())?;
        self.builder
            .build_call(
                self.module.get_function("puts").unwrap(),
                &[absent.as_pointer_value().into()],
                "puts_tagged_absent",
            )
            .map_err(|error| error.to_string())?;
        self.builder
            .build_unconditional_branch(merge_block)
            .map_err(|error| error.to_string())?;

        self.builder.position_at_end(present_block);
        match payload_type {
            HirType::F64 => {
                let format = self
                    .builder
                    .build_global_string_ptr("%g\n", "optional_numfmt")
                    .map_err(|error| error.to_string())?;
                self.builder
                    .build_call(
                        self.module.get_function("printf").unwrap(),
                        &[
                            format.as_pointer_value().into(),
                            payload.into_float_value().into(),
                        ],
                        "printf_optional_number",
                    )
                    .map_err(|error| error.to_string())?;
            }
            HirType::Bool => {
                let true_string = self
                    .builder
                    .build_global_string_ptr("true", "optional_true")
                    .map_err(|error| error.to_string())?;
                let false_string = self
                    .builder
                    .build_global_string_ptr("false", "optional_false")
                    .map_err(|error| error.to_string())?;
                let selected = self
                    .builder
                    .build_select(
                        payload.into_int_value(),
                        true_string.as_pointer_value(),
                        false_string.as_pointer_value(),
                        "optional_bool_string",
                    )
                    .map_err(|error| error.to_string())?;
                self.builder
                    .build_call(
                        self.module.get_function("puts").unwrap(),
                        &[selected.into()],
                        "puts_optional_bool",
                    )
                    .map_err(|error| error.to_string())?;
            }
            HirType::Str => {
                self.builder
                    .build_call(
                        self.module.get_function("puts").unwrap(),
                        &[payload.into_pointer_value().into()],
                        "puts_optional_string",
                    )
                    .map_err(|error| error.to_string())?;
            }
            HirType::Optional(inner) => {
                self.compile_console_tagged(payload.into_struct_value(), inner, "undefined")?;
            }
            HirType::Nullable(inner) => {
                self.compile_console_tagged(payload.into_struct_value(), inner, "null")?;
            }
            HirType::Object(_) => {
                let object = self
                    .builder
                    .build_global_string_ptr("[object Object]", "optional_object")
                    .map_err(|error| error.to_string())?;
                self.builder
                    .build_call(
                        self.module.get_function("puts").unwrap(),
                        &[object.as_pointer_value().into()],
                        "puts_optional_object",
                    )
                    .map_err(|error| error.to_string())?;
            }
            other => {
                return Err(format!(
                    "console.log does not support optional payload {other:?} yet"
                ))
            }
        }
        self.builder
            .build_unconditional_branch(merge_block)
            .map_err(|error| error.to_string())?;
        self.builder.position_at_end(merge_block);
        Ok(())
    }

    fn compile_console_nullish(
        &mut self,
        value: StructValue<'ctx>,
        payload_type: &HirType,
    ) -> Result<(), String> {
        let tag = self
            .builder
            .build_extract_value(value, 0, "console_nullish_tag")
            .map_err(|error| error.to_string())?
            .into_int_value();
        let payload = self
            .builder
            .build_extract_value(value, 1, "console_nullish_payload")
            .map_err(|error| error.to_string())?;
        let function = self.current_function();
        let undefined_block = self
            .context
            .append_basic_block(function, "nullish_undefined");
        let value_or_null_block = self
            .context
            .append_basic_block(function, "nullish_value_or_null");
        let merge_block = self.context.append_basic_block(function, "nullish_printed");
        let is_undefined = self
            .builder
            .build_int_compare(
                IntPredicate::EQ,
                tag,
                self.context.i8_type().const_int(2, false),
                "nullish_is_undefined",
            )
            .map_err(|error| error.to_string())?;
        self.builder
            .build_conditional_branch(is_undefined, undefined_block, value_or_null_block)
            .map_err(|error| error.to_string())?;

        self.builder.position_at_end(undefined_block);
        let undefined = self
            .builder
            .build_global_string_ptr("undefined", "nullish_undefined_string")
            .map_err(|error| error.to_string())?;
        self.builder
            .build_call(
                self.module.get_function("puts").unwrap(),
                &[undefined.as_pointer_value().into()],
                "puts_nullish_undefined",
            )
            .map_err(|error| error.to_string())?;
        self.builder
            .build_unconditional_branch(merge_block)
            .map_err(|error| error.to_string())?;

        self.builder.position_at_end(value_or_null_block);
        let present = self
            .builder
            .build_int_compare(
                IntPredicate::EQ,
                tag,
                self.context.i8_type().const_zero(),
                "nullish_has_value",
            )
            .map_err(|error| error.to_string())?;
        let tagged_type = self
            .basic_type(&HirType::Nullable(Box::new(payload_type.clone())))?
            .into_struct_type();
        let tagged = self
            .builder
            .build_insert_value(tagged_type.get_undef(), present, 0, "nullish_console_tag")
            .map_err(|error| error.to_string())?
            .into_struct_value();
        let tagged = self
            .builder
            .build_insert_value(tagged, payload, 1, "nullish_console_payload")
            .map_err(|error| error.to_string())?
            .into_struct_value();
        self.compile_console_tagged(tagged, payload_type, "null")?;
        self.builder
            .build_unconditional_branch(merge_block)
            .map_err(|error| error.to_string())?;
        self.builder.position_at_end(merge_block);
        Ok(())
    }

    /// Emits `int main(void) { thaw_user_main(); return 0; }`, the real
    /// process entry point that the system linker/CRT expects, for
    /// ordinary (non-Lambda) programs that define `main`.
    fn emit_c_main_entry(&mut self) {
        let user_main = self.module.get_function(USER_MAIN_SYMBOL).unwrap();

        let (main_fn, entry) = self.new_c_main();
        let cleanup = self.context.append_basic_block(main_fn, "entry_cleanup");
        self.builder.position_at_end(entry);
        self.call_module_init_if_present(cleanup);
        let call = self
            .builder
            .build_call(user_main, &[], "call_thaw_user_main")
            .unwrap();
        if user_main.get_type().get_return_type().is_some() {
            let completion = call
                .try_as_basic_value()
                .basic()
                .unwrap()
                .into_pointer_value();
            self.builder
                .build_call(
                    self.module
                        .get_function("thaw_runtime_run_until_resolved")
                        .unwrap(),
                    &[completion.into()],
                    "run_async_main",
                )
                .unwrap();
            self.builder
                .build_call(
                    self.module.get_function("thaw_promise_destroy").unwrap(),
                    &[completion.into()],
                    "destroy_async_main",
                )
                .unwrap();
            self.builder
                .build_call(
                    self.module
                        .get_function("thaw_runtime_drain_detached")
                        .unwrap(),
                    &[],
                    "drain_detached_promises",
                )
                .unwrap();
        }
        if self.module.get_function("createServer").is_some() {
            self.builder
                .build_call(
                    self.module.get_function("thaw_http_run_servers").unwrap(),
                    &[],
                    "run_http_servers",
                )
                .unwrap();
        }
        if self.uses_napi {
            self.builder
                .build_call(
                    self.module
                        .get_function("thaw_napi_run_async_work")
                        .unwrap(),
                    &[],
                    "drain_napi_async_work",
                )
                .unwrap();
            self.builder
                .build_call(
                    self.module.get_function("thaw_napi_unload_all").unwrap(),
                    &[],
                    "unload_napi_addons",
                )
                .unwrap();
        }
        self.builder.build_unconditional_branch(cleanup).unwrap();
        self.builder.position_at_end(cleanup);
        self.finish_c_main();
    }

    /// Calls `__thaw_module_init` before user code, if the program defines
    /// one (see `MODULE_INIT_SYMBOL`). A no-op for programs with no
    /// registry packages that ship a `bundle.js`.
    fn call_module_init_if_present(&self, cleanup: BasicBlock<'ctx>) {
        for (symbol, call_name) in [
            (MODULE_INIT_SYMBOL, "call_thaw_module_init"),
            (NATIVE_MODULE_INIT_SYMBOL, "call_thaw_native_module_init"),
            (TOP_LEVEL_INIT_SYMBOL, "call_thaw_top_level_init"),
        ] {
            if let Some(init_fn) = self.module.get_function(symbol) {
                self.builder.build_call(init_fn, &[], call_name).unwrap();
                let function = self
                    .builder
                    .get_insert_block()
                    .and_then(|block| block.get_parent())
                    .unwrap();
                let continue_block = self
                    .context
                    .append_basic_block(function, &format!("{call_name}_ok"));
                let pending = self
                    .builder
                    .build_load(
                        self.context.ptr_type(AddressSpace::default()),
                        self.pending_exception().as_pointer_value(),
                        "init_pending_exception",
                    )
                    .unwrap()
                    .into_pointer_value();
                let failed = self
                    .builder
                    .build_is_not_null(pending, "init_failed")
                    .unwrap();
                self.builder
                    .build_conditional_branch(failed, cleanup, continue_block)
                    .unwrap();
                self.builder.position_at_end(continue_block);
            }
        }
    }

    fn emit_json_handler_adapter(
        &mut self,
        handler_fn: FunctionValue<'ctx>,
        is_frame_async: bool,
    ) -> Result<FunctionValue<'ctx>, String> {
        let ptr_ty = self.context.ptr_type(AddressSpace::default());
        let adapter = self.module.add_function(
            "__thaw_json_handler_adapter",
            ptr_ty.fn_type(&[ptr_ty.into()], false),
            Some(Linkage::Internal),
        );
        let entry = self.context.append_basic_block(adapter, "entry");
        self.builder.position_at_end(entry);
        let event_text = adapter.get_first_param().unwrap().into_pointer_value();
        let parse = self.module.get_function("thaw_json_parse").unwrap();
        let event = self
            .builder
            .build_call(parse, &[event_text.into()], "lambda_event_json")
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .ok_or("thaw_json_parse did not return a value")?;
        let call = self
            .builder
            .build_call(handler_fn, &[event.into()], "call_json_handler")
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .ok_or("JSON handler did not return a value")?;

        let result = if is_frame_async {
            let promise = call.into_pointer_value();
            let result_slot = self
                .builder
                .build_call(
                    self.module
                        .get_function("thaw_runtime_run_until_resolved")
                        .unwrap(),
                    &[promise.into()],
                    "await_json_handler",
                )
                .map_err(|error| error.to_string())?
                .try_as_basic_value()
                .basic()
                .ok_or("async JSON handler did not settle")?
                .into_pointer_value();
            let promise_state = self
                .builder
                .build_call(
                    self.module.get_function("thaw_promise_state").unwrap(),
                    &[promise.into()],
                    "json_handler_state",
                )
                .map_err(|error| error.to_string())?
                .try_as_basic_value()
                .basic()
                .unwrap()
                .into_int_value();
            let rejected = self
                .builder
                .build_int_compare(
                    IntPredicate::EQ,
                    promise_state,
                    self.context.i8_type().const_int(2, false),
                    "json_handler_rejected",
                )
                .map_err(|error| error.to_string())?;
            let failed = self.context.append_basic_block(adapter, "handler_rejected");
            let succeeded = self
                .context
                .append_basic_block(adapter, "handler_fulfilled");
            self.builder
                .build_conditional_branch(rejected, failed, succeeded)
                .map_err(|error| error.to_string())?;
            self.builder.position_at_end(failed);
            self.builder
                .build_store(self.pending_exception().as_pointer_value(), result_slot)
                .map_err(|error| error.to_string())?;
            self.builder
                .build_call(
                    self.module.get_function("thaw_promise_destroy").unwrap(),
                    &[promise.into()],
                    "destroy_rejected_json_handler",
                )
                .map_err(|error| error.to_string())?;
            self.builder
                .build_return(Some(&ptr_ty.const_null()))
                .map_err(|error| error.to_string())?;

            self.builder.position_at_end(succeeded);
            let result = self
                .builder
                .build_load(ptr_ty, result_slot, "json_handler_result")
                .map_err(|error| error.to_string())?;
            self.builder
                .build_call(
                    self.module.get_function("thaw_promise_destroy").unwrap(),
                    &[promise.into()],
                    "destroy_json_handler",
                )
                .map_err(|error| error.to_string())?;
            result
        } else {
            call
        };

        self.branch_on_pending_exception()?;
        let stringify = self.module.get_function("thaw_json_stringify").unwrap();
        let text = self
            .builder
            .build_call(stringify, &[result.into()], "lambda_response_json")
            .map_err(|error| error.to_string())?
            .try_as_basic_value()
            .basic()
            .ok_or("thaw_json_stringify did not return a value")?;
        self.builder
            .build_return(Some(&text))
            .map_err(|error| error.to_string())?;
        Ok(adapter)
    }

    /// Emits `int main(void) { thaw_runtime_run(&handler); return 0; }` for
    /// programs that define `handler` instead of `main` -- `thaw_runtime_run`
    /// (see thaw-runtime) never actually returns; it polls the Lambda
    /// Runtime API forever.
    fn emit_lambda_entry(&mut self, handler_hir: &HirFunction) -> Result<(), String> {
        let string_signature = handler_hir.params.len() == 1
            && handler_hir.params[0].ty == HirType::Str
            && handler_hir.ret == HirType::Str;
        let json_signature = handler_hir.params.len() == 1
            && handler_hir.params[0].ty == HirType::Json
            && handler_hir.ret == HirType::Json;
        if !string_signature && !json_signature {
            return Err("`handler` must have the signature `(event: string): string` or `(event: Json): Json`".to_string());
        }
        let handler_fn = self.module.get_function("handler").unwrap();
        let handler_fn = if json_signature {
            self.emit_json_handler_adapter(
                handler_fn,
                self.frame_async_functions.contains_key("handler"),
            )?
        } else {
            handler_fn
        };

        let i8_ptr = self.context.ptr_type(AddressSpace::default());
        let run_type = self
            .context
            .void_type()
            .fn_type(&[i8_ptr.into(), i8_ptr.into()], false);
        let run_fn =
            self.module
                .add_function("thaw_runtime_run", run_type, Some(Linkage::External));

        let (main_fn, entry) = self.new_c_main();
        let cleanup = self.context.append_basic_block(main_fn, "entry_cleanup");
        self.builder.position_at_end(entry);
        self.call_module_init_if_present(cleanup);
        let handler_ptr = handler_fn.as_global_value().as_pointer_value();
        self.builder
            .build_call(
                run_fn,
                &[
                    handler_ptr.into(),
                    self.pending_exception().as_pointer_value().into(),
                ],
                "call_thaw_runtime_run",
            )
            .map_err(|e| e.to_string())?;
        self.builder
            .build_unconditional_branch(cleanup)
            .map_err(|error| error.to_string())?;
        self.builder.position_at_end(cleanup);
        self.finish_c_main();
        Ok(())
    }

    fn new_c_main(&mut self) -> (FunctionValue<'ctx>, BasicBlock<'ctx>) {
        let i32_type = self.context.i32_type();
        let main_type = i32_type.fn_type(&[], false);
        let main_fn = self.module.add_function("main", main_type, None);
        let entry = self.context.append_basic_block(main_fn, "entry");
        (main_fn, entry)
    }

    fn finish_c_main(&mut self) {
        let i32_type = self.context.i32_type();
        if self.uses_quickjs_handles {
            self.builder
                .build_call(
                    self.module
                        .get_function("thaw_js_release_all_handles")
                        .unwrap(),
                    &[],
                    "release_javascript_handles",
                )
                .unwrap();
        }
        let pending = self
            .builder
            .build_load(
                self.context.ptr_type(AddressSpace::default()),
                self.pending_exception().as_pointer_value(),
                "process_pending_exception",
            )
            .unwrap()
            .into_pointer_value();
        let exception_failure = self
            .builder
            .build_is_not_null(pending, "process_exception_failed")
            .unwrap();
        let http_failure = if self.module.get_function("createServer").is_some() {
            let status = self
                .builder
                .build_call(
                    self.module
                        .get_function("thaw_http_take_unhandled_error")
                        .unwrap(),
                    &[],
                    "take_http_unhandled_error",
                )
                .unwrap()
                .try_as_basic_value()
                .basic()
                .unwrap()
                .into_int_value();
            self.builder
                .build_int_compare(
                    IntPredicate::NE,
                    status,
                    status.get_type().const_zero(),
                    "http_failed",
                )
                .unwrap()
        } else {
            self.context.bool_type().const_zero()
        };
        let process_failure = self
            .builder
            .build_or(exception_failure, http_failure, "process_failed")
            .unwrap();
        if self.uses_napi {
            let fatal_status = self
                .builder
                .build_call(
                    self.module
                        .get_function("thaw_napi_take_fatal_exception")
                        .unwrap(),
                    &[],
                    "take_napi_fatal_exception",
                )
                .unwrap()
                .try_as_basic_value()
                .basic()
                .unwrap()
                .into_int_value();
            let fatal = self
                .builder
                .build_int_compare(
                    IntPredicate::NE,
                    fatal_status,
                    fatal_status.get_type().const_zero(),
                    "napi_fatal",
                )
                .unwrap();
            let failed = self
                .builder
                .build_or(fatal, process_failure, "napi_process_failed")
                .unwrap();
            let status = self
                .builder
                .build_int_z_extend(failed, i32_type, "process_exit_status")
                .unwrap();
            self.builder.build_return(Some(&status)).unwrap();
            return;
        }
        let status = self
            .builder
            .build_int_z_extend(process_failure, i32_type, "process_exit_status")
            .unwrap();
        self.builder.build_return(Some(&status)).unwrap();
    }

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
