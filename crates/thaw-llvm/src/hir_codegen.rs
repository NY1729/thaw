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

impl<'ctx> HirCompiler<'ctx> {}

include!("hir_codegen/ffi_calls.rs");

impl<'ctx> HirCompiler<'ctx> {
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
}

include!("hir_codegen/console.rs");

impl<'ctx> HirCompiler<'ctx> {
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
