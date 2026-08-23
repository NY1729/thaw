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
//! Phase 2 additionally adds `handler(event: string): string` as an
//! alternate process entry point (see `emit_lambda_entry`), `process.env`,
//! and object types: `{ x: number; y: number }`-style records, `f64`
//! fields only, arena-allocated as a flat `[f64 field0]...[f64 fieldN-1]`
//! buffer with no length header (field order is static, part of the type
//! -- see `thaw_hir::HirExpr::PropAccess`, which bakes in the resolved
//! field layout so codegen never needs its own type inference pass).
//! async/await is designed but not implemented yet -- see
//! `docs/design/async-await.md`.

use std::collections::HashMap;
use std::path::Path;

use inkwell::basic_block::BasicBlock;
use inkwell::builder::Builder;
use inkwell::context::Context;
use inkwell::module::{Linkage, Module};
use inkwell::passes::PassBuilderOptions;
use inkwell::targets::{
    CodeModel, FileType, InitializationConfig, RelocMode, Target, TargetMachine,
};
use inkwell::types::{BasicMetadataTypeEnum, BasicType, BasicTypeEnum};
use inkwell::values::{BasicMetadataValueEnum, BasicValueEnum, FunctionValue, PointerValue};
use inkwell::{AddressSpace, FloatPredicate, IntPredicate, OptimizationLevel};

use thaw_hir::{BinOp, FfiSignature, HirExpr, HirFunction, HirLit, HirProgram, HirStmt, HirType};

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
/// Phase 2 objects only ever hold `f64` fields (see module doc); no header
/// (field count/order is static, part of the type, not a runtime value).
const OBJECT_FIELD_BYTES: u64 = 8;
const ASYNC_FRAME_BYTES: u64 = 24;
const ASYNC_COMPLETION_OFFSET: u64 = 0;
const ASYNC_STATE_OFFSET: u64 = 8;
const ASYNC_WAITING_OFFSET: u64 = 16;
const ASYNC_RESULT_OFFSET: u64 = ASYNC_FRAME_BYTES;

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
    /// Stack of enclosing `try` targets. `throw` and a failed nested Thaw
    /// call target the innermost entry; an empty stack propagates by returning
    /// from the current function with the pending exception left intact.
    catch_stack: Vec<BasicBlock<'ctx>>,
    loop_stack: Vec<(BasicBlock<'ctx>, BasicBlock<'ctx>)>,
    /// Functions whose async ABI is a Promise-returning ramp rather than the
    /// legacy synchronous V1 ABI. Seeded to a fixed point before declarations
    /// so callers and callees agree on the LLVM signature.
    frame_async_functions: HashMap<String, HirType>,
}

impl<'ctx> HirCompiler<'ctx> {
    pub fn new(context: &'ctx Context, module_name: &str) -> Self {
        Self {
            context,
            module: context.create_module(module_name),
            builder: context.create_builder(),
            variables: HashMap::new(),
            catch_stack: Vec::new(),
            loop_stack: Vec::new(),
            frame_async_functions: HashMap::new(),
        }
    }

    pub fn compile_program(&mut self, program: &HirProgram) -> Result<(), String> {
        self.declare_runtime_builtins();
        self.declare_exception_state();
        self.discover_frame_async_functions(program);

        for sig in &program.extern_functions {
            self.declare_extern_function(sig)?;
        }
        for func in &program.functions {
            self.declare_function(func)?;
        }
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
                    "no `function main(): void { ... }` or `function handler(event: string): string { ... }` found"
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

    fn discover_frame_async_functions(&mut self, program: &HirProgram) {
        let mut selected = HashMap::new();
        loop {
            let names = selected
                .keys()
                .cloned()
                .collect::<std::collections::HashSet<_>>();
            let mut changed = false;
            for func in &program.functions {
                if !func.is_async || selected.contains_key(&func.name) {
                    continue;
                }
                if func
                    .body
                    .iter()
                    .any(|stmt| Self::stmt_awaits_frame_source(stmt, &names))
                {
                    selected.insert(func.name.clone(), func.ret.clone());
                    changed = true;
                }
            }
            if !changed {
                break;
            }
        }
        self.frame_async_functions = selected;
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
            HirStmt::Break | HirStmt::Continue => false,
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
            HirExpr::Await(inner) => {
                matches!(inner.as_ref(), HirExpr::Call(callee, _)
                    if matches!(callee.as_ref(), HirExpr::Var(name)
                        if name == "sleep" || name == "fetch" || frame_functions.contains(name)))
                    || Self::expr_awaits_frame_source(inner, frame_functions)
            }
            HirExpr::BinOp(_, left, right) | HirExpr::Index(left, right) => {
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
            HirExpr::FfiCall(_, args) | HirExpr::ArrayLit(args) => args
                .iter()
                .any(|arg| Self::expr_awaits_frame_source(arg, frame_functions)),
            HirExpr::IndexAssign(a, b, c) => {
                Self::expr_awaits_frame_source(a, frame_functions)
                    || Self::expr_awaits_frame_source(b, frame_functions)
                    || Self::expr_awaits_frame_source(c, frame_functions)
            }
            HirExpr::ObjectLit(fields) => fields
                .iter()
                .any(|(_, value)| Self::expr_awaits_frame_source(value, frame_functions)),
            HirExpr::PropAccess(obj, _, _)
            | HirExpr::ArrayLen(obj)
            | HirExpr::JsonAsNumber(obj)
            | HirExpr::JsonAsString(obj)
            | HirExpr::JsonAsBool(obj) => Self::expr_awaits_frame_source(obj, frame_functions),
            HirExpr::PropAssign(obj, _, _, value) | HirExpr::JsonIndex(obj, value) => {
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
        let i32_type = self.context.i32_type();
        let i64_type = self.context.i64_type();

        let puts_type = i32_type.fn_type(&[i8_ptr.into()], false);
        self.module
            .add_function("puts", puts_type, Some(Linkage::External));

        let printf_type = i32_type.fn_type(&[i8_ptr.into()], true);
        self.module
            .add_function("printf", printf_type, Some(Linkage::External));

        let arena_alloc_type = i8_ptr.fn_type(&[i64_type.into(), i64_type.into()], false);
        self.module.add_function(
            "thaw_arena_alloc",
            arena_alloc_type,
            Some(Linkage::External),
        );

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

        // thaw-quickjs: the QuickJS-NG fallback path (docs/design/bridge.md
        // section 7). Same i8-not-i1 reasoning as `thaw_json_as_bool`.
        let js_load_type = self.context.i8_type().fn_type(&[i8_ptr.into()], false);
        self.module
            .add_function("thaw_js_load", js_load_type, Some(Linkage::External));

        let js_call_type = i8_ptr.fn_type(&[i8_ptr.into(), i8_ptr.into()], false);
        self.module
            .add_function("thaw_js_call", js_call_type, Some(Linkage::External));

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
            // Strings and arrays are both represented as a single opaque
            // pointer at the LLVM level; what they point to differs (a
            // C string vs. a [len][elements...] buffer).
            HirType::Str => Ok(self.context.ptr_type(AddressSpace::default()).into()),
            HirType::Array(elem) => {
                if **elem != HirType::F64 {
                    return Err(format!(
                        "Phase 1 only supports number[] arrays, not {elem:?}[]"
                    ));
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
            HirType::Json => Ok(self.context.ptr_type(AddressSpace::default()).into()),
            other => Err(format!(
                "Phase 1/2 codegen does not support type {other:?} yet"
            )),
        }
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
    /// site. Only parameters are adapted this way -- a return value still
    /// uses Thaw's own internal representation (`basic_type`) unchanged;
    /// marshaling an `Array`/`Object` *return* into a real C convention is
    /// still out of scope (docs/design/registry.md section 18).
    fn declare_extern_function(
        &mut self,
        sig: &FfiSignature,
    ) -> Result<FunctionValue<'ctx>, String> {
        let param_types = self.ffi_param_types(&sig.params)?;

        let fn_type = match &sig.ret {
            HirType::Void => self.context.void_type().fn_type(&param_types, false),
            ret => self.basic_type(ret)?.fn_type(&param_types, false),
        };

        Ok(self
            .module
            .add_function(&sig.symbol, fn_type, Some(Linkage::External)))
    }

    /// The real C ABI parameter list a Fast path native symbol is declared
    /// with, for the given `.d.ts`-derived HIR parameter types --
    /// docs/design/bridge.md section 5's "Marshal コード生成", the part
    /// left explicitly unimplemented when Fast path FFI calls were first
    /// added (see docs/design/registry.md section 18, "実際の C ABI...
    /// に合わせた Marshal アダプタ生成"). Thaw's own internal
    /// representation for `Array`/`Object` (a single opaque pointer to an
    /// arena buffer with Thaw-specific layout) is almost never what a real
    /// native library's C signature expects, so those two are expanded
    /// here into the shape a real C API actually tends to use -- this
    /// list, and `compile_ffi_call`'s argument-building loop, must stay in
    /// lockstep (each match arm here has a matching arm there).
    ///
    /// - `number[]` -> two C parameters, `(const double*, int64_t len)` --
    ///   the near-universal C convention for passing an array, as opposed
    ///   to Thaw's own single-pointer `[i64 len][f64 elements...]` buffer.
    /// - `{ x: number; ... }` -> one C parameter *per field*, in
    ///   declaration order (`distance(x1, y1, x2, y2)` rather than a
    ///   single struct pointer) -- Fast path classification only accepts
    ///   `number` object fields (docs/design/bridge.md section 4.1), so
    ///   this is always one `double` per field. A real by-value C struct
    ///   parameter would need to replicate the target's own struct-passing
    ///   ABI (register-class rules, padding, ...), which isn't attempted
    ///   here; a native library that genuinely wants a struct by value
    ///   needs its own flat-field C wrapper, the same way many real C APIs
    ///   already choose to expose one.
    /// - everything else -- unchanged, one C parameter each.
    fn ffi_param_types(
        &self,
        params: &[HirType],
    ) -> Result<Vec<BasicMetadataTypeEnum<'ctx>>, String> {
        let mut out = Vec::with_capacity(params.len());
        for ty in params {
            match ty {
                HirType::Array(elem) if **elem == HirType::F64 => {
                    out.push(self.context.ptr_type(AddressSpace::default()).into());
                    out.push(self.context.i64_type().into());
                }
                HirType::Object(fields) => {
                    for (field_name, field_ty) in fields {
                        let field_llvm_ty = self
                            .basic_type(field_ty)
                            .map_err(|e| format!("FFI object field `{field_name}`: {e}"))?;
                        out.push(field_llvm_ty.into());
                    }
                }
                other => out.push(self.basic_type(other).map(BasicMetadataTypeEnum::from)?),
            }
        }
        Ok(out)
    }

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
        self.catch_stack.clear();
        for (param_val, hir_param) in function.get_param_iter().zip(func.params.iter()) {
            let ty = self.basic_type(&hir_param.ty)?;
            let slot = self
                .builder
                .build_alloca(ty, &hir_param.name)
                .map_err(|e| e.to_string())?;
            self.builder
                .build_store(slot, param_val)
                .map_err(|e| e.to_string())?;
            self.variables.insert(hir_param.name.clone(), (slot, ty));
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

    fn frame_await_plan(&self, func: &HirFunction) -> Result<Option<FrameAsyncPlan>, String> {
        if !self.frame_async_functions.contains_key(&func.name) {
            return Ok(None);
        }
        let normalized_body = self.flatten_async_finally_only_tries(&func.body)?;
        let mut segments = vec![AsyncSegment {
            stmts: Vec::new(),
            awaited: None,
            await_guard: None,
            await_next: None,
            resume_target: None,
            rejection_handler: None,
        }];
        let mut found = false;
        let mut extra_locals = Vec::new();
        let mut guarded_rethrow_handlers = HashMap::new();
        let mut next_temporary = 0usize;
        let mut next_guard = 0usize;
        for stmt in &normalized_body {
            if let HirStmt::Try(try_body, catch_name, catch_body) = stmt {
                let frame_names = self
                    .frame_async_functions
                    .keys()
                    .cloned()
                    .collect::<std::collections::HashSet<_>>();
                if try_body
                    .iter()
                    .chain(catch_body)
                    .any(|nested| Self::stmt_awaits_frame_source(nested, &frame_names))
                {
                    found = true;
                    let try_guard = format!("__thaw_try_{next_guard}");
                    let catch_guard = format!("__thaw_catch_{next_guard}");
                    next_guard += 1;
                    extra_locals.push((catch_name.clone(), HirType::Str));
                    let current = segments.last_mut().unwrap();
                    current.stmts.push(HirStmt::Let(
                        try_guard.clone(),
                        HirType::Bool,
                        HirExpr::Lit(HirLit::Bool(true)),
                    ));
                    current.stmts.push(HirStmt::Let(
                        catch_guard.clone(),
                        HirType::Bool,
                        HirExpr::Lit(HirLit::Bool(false)),
                    ));
                    let outer_handler = AsyncRejectionHandler {
                        try_guard: try_guard.clone(),
                        catch_guard: catch_guard.clone(),
                        catch_binding: catch_name.clone(),
                        disable_guards: Vec::new(),
                    };
                    for nested in try_body {
                        if let HirStmt::Try(inner_body, inner_catch_name, inner_catch_body) = nested
                        {
                            let inner_try_guard = format!("__thaw_try_{next_guard}");
                            let inner_catch_guard = format!("__thaw_catch_{next_guard}");
                            next_guard += 1;
                            extra_locals.push((inner_catch_name.clone(), HirType::Str));
                            let current = segments.last_mut().unwrap();
                            current.stmts.push(HirStmt::Let(
                                inner_try_guard.clone(),
                                HirType::Bool,
                                HirExpr::Lit(HirLit::Bool(false)),
                            ));
                            current.stmts.push(HirStmt::Let(
                                inner_catch_guard.clone(),
                                HirType::Bool,
                                HirExpr::Lit(HirLit::Bool(false)),
                            ));
                            current.stmts.push(HirStmt::If(
                                HirExpr::Var(try_guard.clone()),
                                vec![HirStmt::Expr(HirExpr::Assign(
                                    inner_try_guard.clone(),
                                    Box::new(HirExpr::Lit(HirLit::Bool(true))),
                                ))],
                                Vec::new(),
                            ));
                            let inner_handler = AsyncRejectionHandler {
                                try_guard: inner_try_guard.clone(),
                                catch_guard: inner_catch_guard.clone(),
                                catch_binding: inner_catch_name.clone(),
                                disable_guards: Vec::new(),
                            };
                            for inner_stmt in inner_body {
                                if let HirStmt::Try(
                                    nested_try_body,
                                    nested_catch_name,
                                    nested_catch_body,
                                ) = inner_stmt
                                {
                                    self.append_nested_async_try(
                                        &mut segments,
                                        nested_try_body,
                                        nested_catch_name,
                                        nested_catch_body,
                                        &inner_try_guard,
                                        Some(inner_handler.clone()),
                                        &frame_names,
                                        &mut extra_locals,
                                        &mut guarded_rethrow_handlers,
                                        &mut next_temporary,
                                        &mut next_guard,
                                    )?;
                                    continue;
                                }
                                if let HirStmt::Throw(error) = inner_stmt {
                                    segments.last_mut().unwrap().stmts.push(HirStmt::If(
                                        HirExpr::Var(inner_try_guard.clone()),
                                        vec![
                                            HirStmt::Expr(HirExpr::Assign(
                                                inner_catch_name.clone(),
                                                Box::new(error.clone()),
                                            )),
                                            HirStmt::Expr(HirExpr::Assign(
                                                inner_try_guard.clone(),
                                                Box::new(HirExpr::Lit(HirLit::Bool(false))),
                                            )),
                                            HirStmt::Expr(HirExpr::Assign(
                                                inner_catch_guard.clone(),
                                                Box::new(HirExpr::Lit(HirLit::Bool(true))),
                                            )),
                                        ],
                                        Vec::new(),
                                    ));
                                } else {
                                    let first_new = segments.len();
                                    self.append_guarded_async_stmt(
                                        &mut segments,
                                        inner_stmt,
                                        &inner_try_guard,
                                        true,
                                        None,
                                        &frame_names,
                                        &mut extra_locals,
                                        &mut guarded_rethrow_handlers,
                                        Some(inner_handler.clone()),
                                        &mut next_temporary,
                                        &mut next_guard,
                                    )?;
                                    for segment in &mut segments[first_new..] {
                                        segment.rejection_handler = Some(inner_handler.clone());
                                    }
                                }
                            }
                            for inner_stmt in inner_catch_body {
                                if let HirStmt::Try(
                                    nested_try_body,
                                    nested_catch_name,
                                    nested_catch_body,
                                ) = inner_stmt
                                {
                                    let mut enclosing = outer_handler.clone();
                                    enclosing.disable_guards.push(inner_catch_guard.clone());
                                    self.append_nested_async_try(
                                        &mut segments,
                                        nested_try_body,
                                        nested_catch_name,
                                        nested_catch_body,
                                        &inner_catch_guard,
                                        Some(enclosing),
                                        &frame_names,
                                        &mut extra_locals,
                                        &mut guarded_rethrow_handlers,
                                        &mut next_temporary,
                                        &mut next_guard,
                                    )?;
                                    continue;
                                }
                                let first_new = segments.len();
                                if let HirStmt::Throw(error) = inner_stmt {
                                    let mut handler = outer_handler.clone();
                                    handler.disable_guards.push(inner_catch_guard.clone());
                                    guarded_rethrow_handlers
                                        .insert(inner_catch_guard.clone(), handler);
                                    segments.last_mut().unwrap().stmts.push(HirStmt::If(
                                        HirExpr::Var(inner_catch_guard.clone()),
                                        vec![HirStmt::Throw(error.clone())],
                                        Vec::new(),
                                    ));
                                } else {
                                    self.append_guarded_async_stmt(
                                        &mut segments,
                                        inner_stmt,
                                        &inner_catch_guard,
                                        true,
                                        None,
                                        &frame_names,
                                        &mut extra_locals,
                                        &mut guarded_rethrow_handlers,
                                        Some({
                                            let mut handler = outer_handler.clone();
                                            handler.disable_guards.push(inner_catch_guard.clone());
                                            handler
                                        }),
                                        &mut next_temporary,
                                        &mut next_guard,
                                    )?;
                                }
                                for segment in &mut segments[first_new..] {
                                    let mut handler = outer_handler.clone();
                                    handler.disable_guards.push(inner_catch_guard.clone());
                                    segment.rejection_handler = Some(handler);
                                }
                            }
                            continue;
                        }
                        if let HirStmt::Throw(error) = nested {
                            segments.last_mut().unwrap().stmts.push(HirStmt::If(
                                HirExpr::Var(try_guard.clone()),
                                vec![
                                    HirStmt::Expr(HirExpr::Assign(
                                        catch_name.clone(),
                                        Box::new(error.clone()),
                                    )),
                                    HirStmt::Expr(HirExpr::Assign(
                                        try_guard.clone(),
                                        Box::new(HirExpr::Lit(HirLit::Bool(false))),
                                    )),
                                    HirStmt::Expr(HirExpr::Assign(
                                        catch_guard.clone(),
                                        Box::new(HirExpr::Lit(HirLit::Bool(true))),
                                    )),
                                ],
                                Vec::new(),
                            ));
                        } else {
                            let first_new_segment = segments.len();
                            self.append_guarded_async_stmt(
                                &mut segments,
                                nested,
                                &try_guard,
                                true,
                                None,
                                &frame_names,
                                &mut extra_locals,
                                &mut guarded_rethrow_handlers,
                                Some(outer_handler.clone()),
                                &mut next_temporary,
                                &mut next_guard,
                            )?;
                            for segment in &mut segments[first_new_segment..] {
                                segment.rejection_handler = Some(outer_handler.clone());
                            }
                        }
                    }
                    for nested in catch_body {
                        if let HirStmt::Throw(error) = nested {
                            segments.last_mut().unwrap().stmts.push(HirStmt::If(
                                HirExpr::Var(catch_guard.clone()),
                                vec![HirStmt::Throw(error.clone())],
                                Vec::new(),
                            ));
                        } else {
                            self.append_guarded_async_stmt(
                                &mut segments,
                                nested,
                                &catch_guard,
                                true,
                                None,
                                &frame_names,
                                &mut extra_locals,
                                &mut guarded_rethrow_handlers,
                                None,
                                &mut next_temporary,
                                &mut next_guard,
                            )?;
                        }
                    }
                    continue;
                }
            }
            if let HirStmt::While(cond, body) = stmt {
                let frame_names = self
                    .frame_async_functions
                    .keys()
                    .cloned()
                    .collect::<std::collections::HashSet<_>>();
                let condition_awaits = Self::expr_awaits_frame_source(cond, &frame_names);
                let body_awaits = body
                    .iter()
                    .any(|nested| Self::stmt_awaits_frame_source(nested, &frame_names));
                if condition_awaits || body_awaits {
                    let sleep_zero = || {
                        HirExpr::Call(
                            Box::new(HirExpr::Var("sleep".to_string())),
                            vec![HirExpr::Lit(HirLit::F64(0.0))],
                        )
                    };
                    found = true;
                    segments.last_mut().unwrap().awaited = Some(sleep_zero());
                    let condition_state = segments.len();
                    let guard = format!("__thaw_loop_{next_guard}");
                    let body_guard = format!("__thaw_loop_body_{next_guard}");
                    next_guard += 1;
                    segments.push(AsyncSegment {
                        stmts: Vec::new(),
                        awaited: None,
                        await_guard: None,
                        await_next: None,
                        resume_target: None,
                        rejection_handler: None,
                    });
                    let mut rewritten_condition = cond.clone();
                    loop {
                        let temporary = format!("__thaw_await_{next_temporary}");
                        let Some((awaited, ty)) =
                            self.extract_first_frame_await(&mut rewritten_condition, &temporary)?
                        else {
                            break;
                        };
                        next_temporary += 1;
                        segments.last_mut().unwrap().awaited = Some(awaited);
                        segments.push(AsyncSegment {
                            stmts: Vec::new(),
                            awaited: None,
                            await_guard: None,
                            await_next: None,
                            resume_target: Some((temporary, ty)),
                            rejection_handler: None,
                        });
                    }
                    segments.last_mut().unwrap().stmts.push(HirStmt::Let(
                        guard.clone(),
                        HirType::Bool,
                        rewritten_condition,
                    ));
                    segments.last_mut().unwrap().stmts.push(HirStmt::Let(
                        body_guard.clone(),
                        HirType::Bool,
                        HirExpr::Var(guard.clone()),
                    ));

                    for nested in body {
                        if matches!(nested, HirStmt::Break | HirStmt::Continue) {
                            let mut exits = vec![HirStmt::Expr(HirExpr::Assign(
                                body_guard.clone(),
                                Box::new(HirExpr::Lit(HirLit::Bool(false))),
                            ))];
                            if matches!(nested, HirStmt::Break) {
                                exits.push(HirStmt::Expr(HirExpr::Assign(
                                    guard.clone(),
                                    Box::new(HirExpr::Lit(HirLit::Bool(false))),
                                )));
                            }
                            segments.last_mut().unwrap().stmts.push(HirStmt::If(
                                HirExpr::Var(body_guard.clone()),
                                exits,
                                Vec::new(),
                            ));
                            continue;
                        }
                        if let HirStmt::If(cond, then_body, else_body) = nested {
                            self.append_nested_async_if(
                                &mut segments,
                                cond,
                                then_body,
                                else_body,
                                &body_guard,
                                true,
                                Some((&body_guard, &guard)),
                                &frame_names,
                                &mut extra_locals,
                                &mut guarded_rethrow_handlers,
                                None,
                                &mut next_temporary,
                                &mut next_guard,
                            )?;
                            continue;
                        }
                        if let HirStmt::While(cond, nested_body) = nested {
                            self.append_nested_async_while(
                                &mut segments,
                                cond,
                                nested_body,
                                &body_guard,
                                true,
                                &frame_names,
                                &mut extra_locals,
                                &mut guarded_rethrow_handlers,
                                None,
                                &mut next_temporary,
                                &mut next_guard,
                            )?;
                            continue;
                        }
                        if matches!(nested, HirStmt::Try(..))
                            || Self::stmt_contains_frame_unsupported(nested)
                        {
                            self.append_guarded_async_stmt(
                                &mut segments,
                                nested,
                                &body_guard,
                                true,
                                Some((&body_guard, &guard)),
                                &frame_names,
                                &mut extra_locals,
                                &mut guarded_rethrow_handlers,
                                None,
                                &mut next_temporary,
                                &mut next_guard,
                            )?;
                            continue;
                        }
                        if let HirStmt::Expr(HirExpr::Await(inner)) = nested {
                            if self.is_frame_await_source(inner) {
                                segments.last_mut().unwrap().awaited = Some(inner.as_ref().clone());
                                segments.last_mut().unwrap().await_guard =
                                    Some((body_guard.clone(), true));
                                segments.push(AsyncSegment {
                                    stmts: Vec::new(),
                                    awaited: None,
                                    await_guard: None,
                                    await_next: None,
                                    resume_target: None,
                                    rejection_handler: None,
                                });
                                continue;
                            }
                        }
                        let mut rewritten = nested.clone();
                        loop {
                            let temporary = format!("__thaw_await_{next_temporary}");
                            let Some((awaited, ty)) = self
                                .extract_first_frame_await_from_stmt(&mut rewritten, &temporary)?
                            else {
                                break;
                            };
                            next_temporary += 1;
                            segments.last_mut().unwrap().awaited = Some(awaited);
                            segments.last_mut().unwrap().await_guard =
                                Some((body_guard.clone(), true));
                            segments.push(AsyncSegment {
                                stmts: Vec::new(),
                                awaited: None,
                                await_guard: None,
                                await_next: None,
                                resume_target: Some((temporary, ty)),
                                rejection_handler: None,
                            });
                        }
                        segments.last_mut().unwrap().stmts.push(HirStmt::If(
                            HirExpr::Var(body_guard.clone()),
                            vec![rewritten],
                            Vec::new(),
                        ));
                    }

                    segments.last_mut().unwrap().awaited = Some(sleep_zero());
                    segments.last_mut().unwrap().await_guard = Some((guard, true));
                    segments.last_mut().unwrap().await_next = Some(condition_state);
                    segments.push(AsyncSegment {
                        stmts: Vec::new(),
                        awaited: None,
                        await_guard: None,
                        await_next: None,
                        resume_target: None,
                        rejection_handler: None,
                    });
                    continue;
                }
            }
            if let HirStmt::If(cond, then_body, else_body) = stmt {
                let frame_names = self
                    .frame_async_functions
                    .keys()
                    .cloned()
                    .collect::<std::collections::HashSet<_>>();
                let branches_await = then_body
                    .iter()
                    .chain(else_body)
                    .any(|nested| Self::stmt_awaits_frame_source(nested, &frame_names));
                if branches_await {
                    found = true;
                    let mut rewritten_condition = cond.clone();
                    loop {
                        let temporary = format!("__thaw_await_{next_temporary}");
                        let Some((awaited, ty)) =
                            self.extract_first_frame_await(&mut rewritten_condition, &temporary)?
                        else {
                            break;
                        };
                        next_temporary += 1;
                        segments.last_mut().unwrap().awaited = Some(awaited);
                        segments.push(AsyncSegment {
                            stmts: Vec::new(),
                            awaited: None,
                            await_guard: None,
                            await_next: None,
                            resume_target: Some((temporary, ty)),
                            rejection_handler: None,
                        });
                    }
                    let guard = format!("__thaw_branch_{next_guard}");
                    next_guard += 1;
                    segments.last_mut().unwrap().stmts.push(HirStmt::Let(
                        guard.clone(),
                        HirType::Bool,
                        rewritten_condition,
                    ));
                    for (body, expected) in [(then_body, true), (else_body, false)] {
                        for nested in body {
                            if let HirStmt::If(cond, then_body, else_body) = nested {
                                self.append_nested_async_if(
                                    &mut segments,
                                    cond,
                                    then_body,
                                    else_body,
                                    &guard,
                                    expected,
                                    None,
                                    &frame_names,
                                    &mut extra_locals,
                                    &mut guarded_rethrow_handlers,
                                    None,
                                    &mut next_temporary,
                                    &mut next_guard,
                                )?;
                                continue;
                            }
                            if matches!(nested, HirStmt::While(..) | HirStmt::Try(..))
                                || Self::stmt_contains_frame_unsupported(nested)
                            {
                                self.append_guarded_async_stmt(
                                    &mut segments,
                                    nested,
                                    &guard,
                                    expected,
                                    None,
                                    &frame_names,
                                    &mut extra_locals,
                                    &mut guarded_rethrow_handlers,
                                    None,
                                    &mut next_temporary,
                                    &mut next_guard,
                                )?;
                                continue;
                            }
                            if let HirStmt::Expr(HirExpr::Await(inner)) = nested {
                                if self.is_frame_await_source(inner) {
                                    found = true;
                                    segments.last_mut().unwrap().awaited =
                                        Some(inner.as_ref().clone());
                                    segments.last_mut().unwrap().await_guard =
                                        Some((guard.clone(), expected));
                                    segments.push(AsyncSegment {
                                        stmts: Vec::new(),
                                        awaited: None,
                                        await_guard: None,
                                        await_next: None,
                                        resume_target: None,
                                        rejection_handler: None,
                                    });
                                    continue;
                                }
                            }
                            let mut rewritten = nested.clone();
                            loop {
                                let temporary = format!("__thaw_await_{next_temporary}");
                                let Some((awaited, ty)) = self
                                    .extract_first_frame_await_from_stmt(
                                        &mut rewritten,
                                        &temporary,
                                    )?
                                else {
                                    break;
                                };
                                next_temporary += 1;
                                found = true;
                                segments.last_mut().unwrap().awaited = Some(awaited);
                                segments.last_mut().unwrap().await_guard =
                                    Some((guard.clone(), expected));
                                segments.push(AsyncSegment {
                                    stmts: Vec::new(),
                                    awaited: None,
                                    await_guard: None,
                                    await_next: None,
                                    resume_target: Some((temporary, ty)),
                                    rejection_handler: None,
                                });
                            }
                            segments.last_mut().unwrap().stmts.push(HirStmt::If(
                                HirExpr::Var(guard.clone()),
                                if expected {
                                    vec![rewritten.clone()]
                                } else {
                                    Vec::new()
                                },
                                if expected {
                                    Vec::new()
                                } else {
                                    vec![rewritten]
                                },
                            ));
                        }
                    }
                    continue;
                }
            }
            let boundary = match stmt {
                HirStmt::Expr(HirExpr::Await(inner)) if self.is_frame_await_source(inner) => {
                    Some((inner.as_ref().clone(), None))
                }
                HirStmt::Let(name, ty, HirExpr::Await(inner))
                    if self.is_frame_await_source(inner) =>
                {
                    Some((inner.as_ref().clone(), Some((name.clone(), ty.clone()))))
                }
                _ => None,
            };
            if let Some((awaited, target)) = boundary {
                found = true;
                segments.last_mut().unwrap().awaited = Some(awaited);
                segments.push(AsyncSegment {
                    stmts: Vec::new(),
                    awaited: None,
                    await_guard: None,
                    await_next: None,
                    resume_target: target,
                    rejection_handler: None,
                });
            } else {
                let frame_names = self
                    .frame_async_functions
                    .keys()
                    .cloned()
                    .collect::<std::collections::HashSet<_>>();
                if Self::stmt_awaits_frame_source(stmt, &frame_names) {
                    match stmt {
                        HirStmt::If(_, then_body, else_body)
                            if then_body.iter().chain(else_body).any(|nested| {
                                Self::stmt_awaits_frame_source(nested, &frame_names)
                            }) =>
                        {
                            return Err(
                                "frame-split `await` inside an if branch is not supported yet"
                                    .to_string(),
                            );
                        }
                        HirStmt::While(..) => {
                            return Err("frame-split `await` inside a loop is not supported yet"
                                .to_string());
                        }
                        HirStmt::Try(..) => {
                            return Err(
                                "frame-split `await` inside try/catch is not supported yet"
                                    .to_string(),
                            );
                        }
                        _ => {}
                    }
                    let mut rewritten = stmt.clone();
                    loop {
                        let temporary = format!("__thaw_await_{next_temporary}");
                        let Some((awaited, ty)) =
                            self.extract_first_frame_await_from_stmt(&mut rewritten, &temporary)?
                        else {
                            break;
                        };
                        next_temporary += 1;
                        found = true;
                        segments.last_mut().unwrap().awaited = Some(awaited);
                        segments.push(AsyncSegment {
                            stmts: Vec::new(),
                            awaited: None,
                            await_guard: None,
                            await_next: None,
                            resume_target: Some((temporary, ty)),
                            rejection_handler: None,
                        });
                    }
                    segments.last_mut().unwrap().stmts.push(rewritten);
                    continue;
                }
                segments.last_mut().unwrap().stmts.push(stmt.clone());
            }
        }
        if !found {
            return Ok(None);
        }
        if !func.is_async {
            return Err("frame-split await requires an async function".to_string());
        }
        if func.ret != HirType::Void {
            self.basic_type(&func.ret)
                .map_err(|e| format!("async frame result: {e}"))?;
        }
        let mut locals = func
            .params
            .iter()
            .map(|param| {
                self.basic_type(&param.ty)
                    .map_err(|e| format!("async frame parameter `{}`: {e}", param.name))?;
                Ok((param.name.clone(), param.ty.clone()))
            })
            .collect::<Result<Vec<_>, String>>()?;
        locals.extend(extra_locals);
        for stmt in segments.iter().flat_map(|segment| &segment.stmts) {
            self.collect_async_frame_locals(stmt, &mut locals)?;
        }
        for (name, ty) in segments
            .iter()
            .filter_map(|segment| segment.resume_target.as_ref())
        {
            if locals.iter().any(|(existing, _)| existing == name) {
                return Err(format!("duplicate async frame local `{name}`"));
            }
            self.basic_type(ty)
                .map_err(|e| format!("async frame local `{name}`: {e}"))?;
            locals.push((name.clone(), ty.clone()));
        }
        Ok(Some(FrameAsyncPlan {
            segments,
            locals,
            ret: func.ret.clone(),
            guarded_rethrow_handlers,
        }))
    }

    fn collect_async_frame_locals(
        &self,
        stmt: &HirStmt,
        locals: &mut Vec<(String, HirType)>,
    ) -> Result<(), String> {
        match stmt {
            HirStmt::Let(name, ty, _) => {
                if locals.iter().any(|(existing, _)| existing == name) {
                    return Err(format!("duplicate async frame local `{name}`"));
                }
                self.basic_type(ty)
                    .map_err(|e| format!("async frame local `{name}`: {e}"))?;
                locals.push((name.clone(), ty.clone()));
            }
            HirStmt::If(_, then_body, else_body) => {
                for nested in then_body.iter().chain(else_body) {
                    self.collect_async_frame_locals(nested, locals)?;
                }
            }
            HirStmt::While(_, body) => {
                for nested in body {
                    self.collect_async_frame_locals(nested, locals)?;
                }
            }
            HirStmt::Try(body, _, catch_body) => {
                for nested in body.iter().chain(catch_body) {
                    self.collect_async_frame_locals(nested, locals)?;
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn flatten_async_finally_only_tries(&self, body: &[HirStmt]) -> Result<Vec<HirStmt>, String> {
        let mut flattened = Vec::new();
        for stmt in body {
            let HirStmt::Try(try_body, catch_name, catch_body) = stmt else {
                flattened.push(stmt.clone());
                continue;
            };
            let synthetic_finally_catch = catch_name.starts_with("__thaw_finally_exception")
                && matches!(
                    catch_body.last(),
                    Some(HirStmt::Throw(HirExpr::Var(name))) if name == catch_name
                );
            let contains_frame_await = try_body.iter().any(|stmt| {
                let names = self
                    .frame_async_functions
                    .keys()
                    .cloned()
                    .collect::<std::collections::HashSet<_>>();
                Self::stmt_awaits_frame_source(stmt, &names)
            });
            if !contains_frame_await {
                flattened.push(stmt.clone());
                continue;
            }
            if !synthetic_finally_catch {
                flattened.push(stmt.clone());
                continue;
            }
            let frame_names = self
                .frame_async_functions
                .keys()
                .cloned()
                .collect::<std::collections::HashSet<_>>();
            let may_reject = try_body.iter().any(Self::stmt_contains_throw)
                || try_body
                    .iter()
                    .any(|stmt| Self::stmt_awaits_named_async(stmt, &frame_names));
            if may_reject {
                flattened.push(stmt.clone());
                continue;
            }
            flattened.extend(try_body.iter().cloned());
        }
        Ok(flattened)
    }

    fn stmt_contains_throw(stmt: &HirStmt) -> bool {
        match stmt {
            HirStmt::Throw(_) => true,
            HirStmt::If(_, then_body, else_body) => then_body
                .iter()
                .chain(else_body)
                .any(Self::stmt_contains_throw),
            HirStmt::While(_, body) => body.iter().any(Self::stmt_contains_throw),
            HirStmt::Try(body, _, catch_body) => {
                body.iter().chain(catch_body).any(Self::stmt_contains_throw)
            }
            _ => false,
        }
    }

    fn stmt_awaits_named_async(
        stmt: &HirStmt,
        frame_names: &std::collections::HashSet<String>,
    ) -> bool {
        match stmt {
            HirStmt::Expr(expr) | HirStmt::Throw(expr) | HirStmt::Let(_, _, expr) => {
                Self::expr_awaits_named_async(expr, frame_names)
            }
            HirStmt::Return(Some(expr)) => Self::expr_awaits_named_async(expr, frame_names),
            HirStmt::If(cond, then_body, else_body) => {
                Self::expr_awaits_named_async(cond, frame_names)
                    || then_body
                        .iter()
                        .chain(else_body)
                        .any(|stmt| Self::stmt_awaits_named_async(stmt, frame_names))
            }
            HirStmt::While(cond, body) => {
                Self::expr_awaits_named_async(cond, frame_names)
                    || body
                        .iter()
                        .any(|stmt| Self::stmt_awaits_named_async(stmt, frame_names))
            }
            HirStmt::Try(body, _, catch_body) => body
                .iter()
                .chain(catch_body)
                .any(|stmt| Self::stmt_awaits_named_async(stmt, frame_names)),
            _ => false,
        }
    }

    fn expr_awaits_named_async(
        expr: &HirExpr,
        frame_names: &std::collections::HashSet<String>,
    ) -> bool {
        match expr {
            HirExpr::Await(inner) => {
                matches!(inner.as_ref(), HirExpr::Call(callee, _)
                    if matches!(callee.as_ref(), HirExpr::Var(name)
                        if name == "fetch" || frame_names.contains(name)))
                    || Self::expr_awaits_named_async(inner, frame_names)
            }
            HirExpr::BinOp(_, left, right) | HirExpr::Index(left, right) => {
                Self::expr_awaits_named_async(left, frame_names)
                    || Self::expr_awaits_named_async(right, frame_names)
            }
            HirExpr::Assign(_, value) => Self::expr_awaits_named_async(value, frame_names),
            HirExpr::Call(callee, args) => {
                Self::expr_awaits_named_async(callee, frame_names)
                    || args
                        .iter()
                        .any(|arg| Self::expr_awaits_named_async(arg, frame_names))
            }
            _ => false,
        }
    }

    fn is_frame_await_source(&self, expr: &HirExpr) -> bool {
        matches!(expr, HirExpr::Call(callee, _)
            if matches!(callee.as_ref(), HirExpr::Var(name)
                if name == "sleep" || name == "fetch" || self.frame_async_functions.contains_key(name)))
    }

    fn extract_first_frame_await_from_stmt(
        &self,
        stmt: &mut HirStmt,
        temporary: &str,
    ) -> Result<Option<(HirExpr, HirType)>, String> {
        match stmt {
            HirStmt::Expr(expr)
            | HirStmt::Return(Some(expr))
            | HirStmt::Let(_, _, expr)
            | HirStmt::Throw(expr)
            | HirStmt::If(expr, _, _) => self.extract_first_frame_await(expr, temporary),
            _ => Ok(None),
        }
    }

    fn extract_first_frame_await(
        &self,
        expr: &mut HirExpr,
        temporary: &str,
    ) -> Result<Option<(HirExpr, HirType)>, String> {
        if let HirExpr::Await(inner) = expr {
            if self.is_frame_await_source(inner) {
                let awaited = inner.as_ref().clone();
                let ty = match inner.as_ref() {
                    HirExpr::Call(callee, _) if matches!(callee.as_ref(), HirExpr::Var(name) if name == "sleep") =>
                    {
                        return Err("the void result of `await sleep(...)` cannot be used inside an expression".to_string());
                    }
                    HirExpr::Call(callee, _) if matches!(callee.as_ref(), HirExpr::Var(name) if name == "fetch") => {
                        HirType::Str
                    }
                    HirExpr::Call(callee, _) => {
                        let HirExpr::Var(name) = callee.as_ref() else {
                            unreachable!()
                        };
                        self.frame_async_functions
                            .get(name)
                            .cloned()
                            .ok_or_else(|| format!("missing async result type for `{name}`"))?
                    }
                    _ => unreachable!(),
                };
                *expr = HirExpr::Var(temporary.to_string());
                return Ok(Some((awaited, ty)));
            }
        }

        match expr {
            HirExpr::BinOp(_, left, right) | HirExpr::Index(left, right) => {
                if let Some(found) = self.extract_first_frame_await(left, temporary)? {
                    Ok(Some(found))
                } else {
                    self.extract_first_frame_await(right, temporary)
                }
            }
            HirExpr::Call(callee, args) | HirExpr::DynamicCall(callee, args) => {
                if let Some(found) = self.extract_first_frame_await(callee, temporary)? {
                    return Ok(Some(found));
                }
                for arg in args {
                    if let Some(found) = self.extract_first_frame_await(arg, temporary)? {
                        return Ok(Some(found));
                    }
                }
                Ok(None)
            }
            HirExpr::FfiCall(_, args) | HirExpr::ArrayLit(args) => {
                for arg in args {
                    if let Some(found) = self.extract_first_frame_await(arg, temporary)? {
                        return Ok(Some(found));
                    }
                }
                Ok(None)
            }
            HirExpr::Assign(_, value)
            | HirExpr::ArrayLen(value)
            | HirExpr::JsonGet(value, _)
            | HirExpr::JsonAsNumber(value)
            | HirExpr::JsonAsString(value)
            | HirExpr::JsonAsBool(value) => self.extract_first_frame_await(value, temporary),
            HirExpr::IndexAssign(a, b, c) => {
                for value in [a, b, c] {
                    if let Some(found) = self.extract_first_frame_await(value, temporary)? {
                        return Ok(Some(found));
                    }
                }
                Ok(None)
            }
            HirExpr::ObjectLit(fields) => {
                for (_, value) in fields {
                    if let Some(found) = self.extract_first_frame_await(value, temporary)? {
                        return Ok(Some(found));
                    }
                }
                Ok(None)
            }
            HirExpr::PropAccess(obj, _, _) => self.extract_first_frame_await(obj, temporary),
            HirExpr::PropAssign(obj, _, _, value) | HirExpr::JsonIndex(obj, value) => {
                if let Some(found) = self.extract_first_frame_await(obj, temporary)? {
                    Ok(Some(found))
                } else {
                    self.extract_first_frame_await(value, temporary)
                }
            }
            HirExpr::Await(inner) => self.extract_first_frame_await(inner, temporary),
            _ => Ok(None),
        }
    }

    #[allow(clippy::too_many_arguments)]
    #[allow(clippy::too_many_arguments)]
    fn append_nested_async_try(
        &self,
        segments: &mut Vec<AsyncSegment>,
        try_body: &[HirStmt],
        catch_name: &str,
        catch_body: &[HirStmt],
        activation_guard: &str,
        enclosing_handler: Option<AsyncRejectionHandler>,
        frame_names: &std::collections::HashSet<String>,
        extra_locals: &mut Vec<(String, HirType)>,
        guarded_rethrow_handlers: &mut HashMap<String, AsyncRejectionHandler>,
        next_temporary: &mut usize,
        next_guard: &mut usize,
    ) -> Result<(), String> {
        let try_guard = format!("__thaw_try_{}", *next_guard);
        let catch_guard = format!("__thaw_catch_{}", *next_guard);
        *next_guard += 1;
        extra_locals.push((catch_name.to_string(), HirType::Str));

        let current = segments.last_mut().unwrap();
        current.stmts.push(HirStmt::Let(
            try_guard.clone(),
            HirType::Bool,
            HirExpr::Lit(HirLit::Bool(false)),
        ));
        current.stmts.push(HirStmt::Let(
            catch_guard.clone(),
            HirType::Bool,
            HirExpr::Lit(HirLit::Bool(false)),
        ));
        current.stmts.push(HirStmt::If(
            HirExpr::Var(activation_guard.to_string()),
            vec![HirStmt::Expr(HirExpr::Assign(
                try_guard.clone(),
                Box::new(HirExpr::Lit(HirLit::Bool(true))),
            ))],
            Vec::new(),
        ));

        let handler = AsyncRejectionHandler {
            try_guard: try_guard.clone(),
            catch_guard: catch_guard.clone(),
            catch_binding: catch_name.to_string(),
            disable_guards: Vec::new(),
        };
        for stmt in try_body {
            if let HirStmt::Try(nested_try, nested_name, nested_catch) = stmt {
                self.append_nested_async_try(
                    segments,
                    nested_try,
                    nested_name,
                    nested_catch,
                    &try_guard,
                    Some(handler.clone()),
                    frame_names,
                    extra_locals,
                    guarded_rethrow_handlers,
                    next_temporary,
                    next_guard,
                )?;
                continue;
            }
            if let HirStmt::Throw(error) = stmt {
                segments.last_mut().unwrap().stmts.push(HirStmt::If(
                    HirExpr::Var(try_guard.clone()),
                    vec![
                        HirStmt::Expr(HirExpr::Assign(
                            catch_name.to_string(),
                            Box::new(error.clone()),
                        )),
                        HirStmt::Expr(HirExpr::Assign(
                            try_guard.clone(),
                            Box::new(HirExpr::Lit(HirLit::Bool(false))),
                        )),
                        HirStmt::Expr(HirExpr::Assign(
                            catch_guard.clone(),
                            Box::new(HirExpr::Lit(HirLit::Bool(true))),
                        )),
                    ],
                    Vec::new(),
                ));
                continue;
            }
            let first_new = segments.len();
            self.append_guarded_async_stmt(
                segments,
                stmt,
                &try_guard,
                true,
                None,
                frame_names,
                extra_locals,
                guarded_rethrow_handlers,
                Some(handler.clone()),
                next_temporary,
                next_guard,
            )?;
            for segment in &mut segments[first_new..] {
                segment.rejection_handler = Some(handler.clone());
            }
        }

        let catch_enclosing = enclosing_handler.map(|mut outer| {
            outer.disable_guards.push(catch_guard.clone());
            outer
        });
        for stmt in catch_body {
            if let HirStmt::Try(nested_try, nested_name, nested_catch) = stmt {
                self.append_nested_async_try(
                    segments,
                    nested_try,
                    nested_name,
                    nested_catch,
                    &catch_guard,
                    catch_enclosing.clone(),
                    frame_names,
                    extra_locals,
                    guarded_rethrow_handlers,
                    next_temporary,
                    next_guard,
                )?;
                continue;
            }
            let first_new = segments.len();
            if let HirStmt::Throw(error) = stmt {
                if let Some(outer) = &catch_enclosing {
                    guarded_rethrow_handlers.insert(catch_guard.clone(), outer.clone());
                }
                segments.last_mut().unwrap().stmts.push(HirStmt::If(
                    HirExpr::Var(catch_guard.clone()),
                    vec![HirStmt::Throw(error.clone())],
                    Vec::new(),
                ));
            } else {
                self.append_guarded_async_stmt(
                    segments,
                    stmt,
                    &catch_guard,
                    true,
                    None,
                    frame_names,
                    extra_locals,
                    guarded_rethrow_handlers,
                    catch_enclosing.clone(),
                    next_temporary,
                    next_guard,
                )?;
            }
            if let Some(outer) = &catch_enclosing {
                for segment in &mut segments[first_new..] {
                    segment.rejection_handler = Some(outer.clone());
                }
            }
        }
        Ok(())
    }

    fn append_nested_async_while(
        &self,
        segments: &mut Vec<AsyncSegment>,
        cond: &HirExpr,
        body: &[HirStmt],
        parent_guard: &str,
        parent_expected: bool,
        frame_names: &std::collections::HashSet<String>,
        extra_locals: &mut Vec<(String, HirType)>,
        guarded_rethrow_handlers: &mut HashMap<String, AsyncRejectionHandler>,
        rejection_handler: Option<AsyncRejectionHandler>,
        next_temporary: &mut usize,
        next_guard: &mut usize,
    ) -> Result<(), String> {
        let suffix = *next_guard;
        *next_guard += 1;
        let enabled_guard = format!("__thaw_nested_loop_enabled_{suffix}");
        let loop_guard = format!("__thaw_nested_loop_{suffix}");
        let body_guard = format!("__thaw_nested_loop_body_{suffix}");
        let current = segments.last_mut().unwrap();
        current.stmts.push(HirStmt::Let(
            enabled_guard.clone(),
            HirType::Bool,
            HirExpr::Lit(HirLit::Bool(false)),
        ));
        let enable = HirStmt::Expr(HirExpr::Assign(
            enabled_guard.clone(),
            Box::new(HirExpr::Lit(HirLit::Bool(true))),
        ));
        current.stmts.push(HirStmt::If(
            HirExpr::Var(parent_guard.to_string()),
            if parent_expected {
                vec![enable.clone()]
            } else {
                Vec::new()
            },
            if parent_expected {
                Vec::new()
            } else {
                vec![enable]
            },
        ));

        let sleep_zero = || {
            HirExpr::Call(
                Box::new(HirExpr::Var("sleep".to_string())),
                vec![HirExpr::Lit(HirLit::F64(0.0))],
            )
        };
        current.awaited = Some(sleep_zero());
        current.await_guard = Some((enabled_guard.clone(), true));
        let condition_state = segments.len();
        segments.push(AsyncSegment {
            stmts: Vec::new(),
            awaited: None,
            await_guard: None,
            await_next: None,
            resume_target: None,
            rejection_handler: None,
        });

        let mut rewritten_condition = cond.clone();
        loop {
            let temporary = format!("__thaw_await_{}", *next_temporary);
            let Some((awaited, ty)) =
                self.extract_first_frame_await(&mut rewritten_condition, &temporary)?
            else {
                break;
            };
            *next_temporary += 1;
            let current = segments.last_mut().unwrap();
            current.awaited = Some(awaited);
            current.await_guard = Some((enabled_guard.clone(), true));
            segments.push(AsyncSegment {
                stmts: Vec::new(),
                awaited: None,
                await_guard: None,
                await_next: None,
                resume_target: Some((temporary, ty)),
                rejection_handler: None,
            });
        }

        let current = segments.last_mut().unwrap();
        current.stmts.push(HirStmt::Let(
            loop_guard.clone(),
            HirType::Bool,
            HirExpr::Lit(HirLit::Bool(false)),
        ));
        current.stmts.push(HirStmt::If(
            HirExpr::Var(enabled_guard.clone()),
            vec![HirStmt::Expr(HirExpr::Assign(
                loop_guard.clone(),
                Box::new(rewritten_condition),
            ))],
            Vec::new(),
        ));
        current.stmts.push(HirStmt::Let(
            body_guard.clone(),
            HirType::Bool,
            HirExpr::Var(loop_guard.clone()),
        ));

        for stmt in body {
            self.append_guarded_async_stmt(
                segments,
                stmt,
                &body_guard,
                true,
                Some((&body_guard, &loop_guard)),
                frame_names,
                extra_locals,
                guarded_rethrow_handlers,
                rejection_handler.clone(),
                next_temporary,
                next_guard,
            )?;
        }

        let current = segments.last_mut().unwrap();
        current.awaited = Some(sleep_zero());
        current.await_guard = Some((loop_guard, true));
        current.await_next = Some(condition_state);
        segments.push(AsyncSegment {
            stmts: Vec::new(),
            awaited: None,
            await_guard: None,
            await_next: None,
            resume_target: None,
            rejection_handler: None,
        });
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn append_nested_async_if(
        &self,
        segments: &mut Vec<AsyncSegment>,
        cond: &HirExpr,
        then_body: &[HirStmt],
        else_body: &[HirStmt],
        parent_guard: &str,
        parent_expected: bool,
        loop_guards: Option<(&str, &str)>,
        frame_names: &std::collections::HashSet<String>,
        extra_locals: &mut Vec<(String, HirType)>,
        guarded_rethrow_handlers: &mut HashMap<String, AsyncRejectionHandler>,
        rejection_handler: Option<AsyncRejectionHandler>,
        next_temporary: &mut usize,
        next_guard: &mut usize,
    ) -> Result<(), String> {
        let mut rewritten_condition = cond.clone();
        loop {
            let temporary = format!("__thaw_await_{}", *next_temporary);
            let Some((awaited, ty)) =
                self.extract_first_frame_await(&mut rewritten_condition, &temporary)?
            else {
                break;
            };
            *next_temporary += 1;
            let current = segments.last_mut().unwrap();
            current.awaited = Some(awaited);
            current.await_guard = Some((parent_guard.to_string(), parent_expected));
            segments.push(AsyncSegment {
                stmts: Vec::new(),
                awaited: None,
                await_guard: None,
                await_next: None,
                resume_target: Some((temporary, ty)),
                rejection_handler: None,
            });
        }
        let then_guard = format!("__thaw_nested_then_{}", *next_guard);
        let else_guard = format!("__thaw_nested_else_{}", *next_guard);
        *next_guard += 1;
        let current = segments.last_mut().unwrap();
        current.stmts.push(HirStmt::Let(
            then_guard.clone(),
            HirType::Bool,
            HirExpr::Lit(HirLit::Bool(false)),
        ));
        current.stmts.push(HirStmt::Let(
            else_guard.clone(),
            HirType::Bool,
            HirExpr::Lit(HirLit::Bool(false)),
        ));
        let choose = HirStmt::If(
            rewritten_condition,
            vec![HirStmt::Expr(HirExpr::Assign(
                then_guard.clone(),
                Box::new(HirExpr::Lit(HirLit::Bool(true))),
            ))],
            vec![HirStmt::Expr(HirExpr::Assign(
                else_guard.clone(),
                Box::new(HirExpr::Lit(HirLit::Bool(true))),
            ))],
        );
        current.stmts.push(HirStmt::If(
            HirExpr::Var(parent_guard.to_string()),
            if parent_expected {
                vec![choose.clone()]
            } else {
                Vec::new()
            },
            if parent_expected {
                Vec::new()
            } else {
                vec![choose]
            },
        ));

        for stmt in then_body {
            self.append_guarded_async_stmt(
                segments,
                stmt,
                &then_guard,
                true,
                loop_guards,
                frame_names,
                extra_locals,
                guarded_rethrow_handlers,
                rejection_handler.clone(),
                next_temporary,
                next_guard,
            )?;
        }
        for stmt in else_body {
            self.append_guarded_async_stmt(
                segments,
                stmt,
                &else_guard,
                true,
                loop_guards,
                frame_names,
                extra_locals,
                guarded_rethrow_handlers,
                rejection_handler.clone(),
                next_temporary,
                next_guard,
            )?;
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn append_guarded_async_stmt(
        &self,
        segments: &mut Vec<AsyncSegment>,
        stmt: &HirStmt,
        guard: &str,
        expected: bool,
        loop_guards: Option<(&str, &str)>,
        frame_names: &std::collections::HashSet<String>,
        extra_locals: &mut Vec<(String, HirType)>,
        guarded_rethrow_handlers: &mut HashMap<String, AsyncRejectionHandler>,
        rejection_handler: Option<AsyncRejectionHandler>,
        next_temporary: &mut usize,
        next_guard: &mut usize,
    ) -> Result<(), String> {
        let first_new_segment = segments.len();
        if let HirStmt::If(cond, then_body, else_body) = stmt {
            return self.append_nested_async_if(
                segments,
                cond,
                then_body,
                else_body,
                guard,
                expected,
                loop_guards,
                frame_names,
                extra_locals,
                guarded_rethrow_handlers,
                rejection_handler.clone(),
                next_temporary,
                next_guard,
            );
        }
        if matches!(stmt, HirStmt::Break | HirStmt::Continue) {
            let Some((body_guard, loop_guard)) = loop_guards else {
                return Err(
                    "`break`/`continue` in an async branch requires an enclosing async loop"
                        .to_string(),
                );
            };
            let mut exits = vec![HirStmt::Expr(HirExpr::Assign(
                body_guard.to_string(),
                Box::new(HirExpr::Lit(HirLit::Bool(false))),
            ))];
            if matches!(stmt, HirStmt::Break) {
                exits.push(HirStmt::Expr(HirExpr::Assign(
                    loop_guard.to_string(),
                    Box::new(HirExpr::Lit(HirLit::Bool(false))),
                )));
            }
            segments.last_mut().unwrap().stmts.push(HirStmt::If(
                HirExpr::Var(guard.to_string()),
                if expected { exits.clone() } else { Vec::new() },
                if expected { Vec::new() } else { exits },
            ));
            return Ok(());
        }
        if let HirStmt::While(cond, body) = stmt {
            return self.append_nested_async_while(
                segments,
                cond,
                body,
                guard,
                expected,
                frame_names,
                extra_locals,
                guarded_rethrow_handlers,
                rejection_handler.clone(),
                next_temporary,
                next_guard,
            );
        }
        if let HirStmt::Try(try_body, catch_name, catch_body) = stmt {
            let activation_guard = if expected {
                guard.to_string()
            } else {
                let active = format!("__thaw_active_{}", *next_guard);
                *next_guard += 1;
                let current = segments.last_mut().unwrap();
                current.stmts.push(HirStmt::Let(
                    active.clone(),
                    HirType::Bool,
                    HirExpr::Lit(HirLit::Bool(false)),
                ));
                current.stmts.push(HirStmt::If(
                    HirExpr::Var(guard.to_string()),
                    Vec::new(),
                    vec![HirStmt::Expr(HirExpr::Assign(
                        active.clone(),
                        Box::new(HirExpr::Lit(HirLit::Bool(true))),
                    ))],
                ));
                active
            };
            return self.append_nested_async_try(
                segments,
                try_body,
                catch_name,
                catch_body,
                &activation_guard,
                rejection_handler,
                frame_names,
                extra_locals,
                guarded_rethrow_handlers,
                next_temporary,
                next_guard,
            );
        }
        if matches!(stmt, HirStmt::Throw(_)) {
            if let Some(handler) = &rejection_handler {
                guarded_rethrow_handlers.insert(guard.to_string(), handler.clone());
            }
        }
        if let HirStmt::Expr(HirExpr::Await(inner)) = stmt {
            if self.is_frame_await_source(inner) {
                let current = segments.last_mut().unwrap();
                current.awaited = Some(inner.as_ref().clone());
                current.await_guard = Some((guard.to_string(), expected));
                segments.push(AsyncSegment {
                    stmts: Vec::new(),
                    awaited: None,
                    await_guard: None,
                    await_next: None,
                    resume_target: None,
                    rejection_handler: None,
                });
                if let Some(handler) = rejection_handler {
                    for segment in &mut segments[first_new_segment..] {
                        segment.rejection_handler = Some(handler.clone());
                    }
                }
                return Ok(());
            }
        }
        let mut rewritten = stmt.clone();
        loop {
            let temporary = format!("__thaw_await_{}", *next_temporary);
            let Some((awaited, ty)) =
                self.extract_first_frame_await_from_stmt(&mut rewritten, &temporary)?
            else {
                break;
            };
            *next_temporary += 1;
            let current = segments.last_mut().unwrap();
            current.awaited = Some(awaited);
            current.await_guard = Some((guard.to_string(), expected));
            segments.push(AsyncSegment {
                stmts: Vec::new(),
                awaited: None,
                await_guard: None,
                await_next: None,
                resume_target: Some((temporary, ty)),
                rejection_handler: None,
            });
        }
        segments.last_mut().unwrap().stmts.push(HirStmt::If(
            HirExpr::Var(guard.to_string()),
            if expected {
                vec![rewritten.clone()]
            } else {
                Vec::new()
            },
            if expected {
                Vec::new()
            } else {
                vec![rewritten]
            },
        ));
        if let Some(handler) = rejection_handler {
            for segment in &mut segments[first_new_segment..] {
                segment.rejection_handler = Some(handler.clone());
            }
        }
        Ok(())
    }

    fn stmt_contains_frame_unsupported(stmt: &HirStmt) -> bool {
        match stmt {
            HirStmt::Let(..) | HirStmt::Return(..) | HirStmt::Throw(..) | HirStmt::Try(..) => true,
            HirStmt::If(_, then_body, else_body) => {
                if matches!(
                    (then_body.as_slice(), else_body.as_slice()),
                    ([HirStmt::Throw(_)], [])
                ) {
                    return false;
                }
                then_body.iter().any(Self::stmt_contains_frame_unsupported)
                    || else_body.iter().any(Self::stmt_contains_frame_unsupported)
            }
            HirStmt::While(_, body) => body.iter().any(Self::stmt_contains_frame_unsupported),
            _ => false,
        }
    }

    fn async_frame_field(
        &self,
        frame: PointerValue<'ctx>,
        offset: u64,
        name: &str,
    ) -> Result<PointerValue<'ctx>, String> {
        let offset = self.context.i64_type().const_int(offset, false);
        unsafe {
            self.builder
                .build_in_bounds_gep(self.context.i8_type(), frame, &[offset], name)
                .map_err(|e| e.to_string())
        }
    }

    fn compile_frame_async_function(
        &mut self,
        func: &HirFunction,
        plan: &FrameAsyncPlan,
    ) -> Result<(), String> {
        let segments = &plan.segments;
        let symbol = Self::llvm_symbol_for(&func.name);
        let ramp = self.module.get_function(&symbol).unwrap();
        let ptr_ty = self.context.ptr_type(AddressSpace::default());
        let resume_ty = self
            .context
            .void_type()
            .fn_type(&[ptr_ty.into(), ptr_ty.into()], false);
        let resume = self.module.add_function(
            &format!("{symbol}.resume"),
            resume_ty,
            Some(Linkage::Internal),
        );

        let entry = self.context.append_basic_block(ramp, "entry");
        self.builder.position_at_end(entry);
        self.variables.clear();
        self.catch_stack.clear();

        let alloc = self.module.get_function("thaw_arena_alloc").unwrap();
        let frame = self
            .builder
            .build_call(
                alloc,
                &[
                    self.context
                        .i64_type()
                        .const_int(self.async_frame_size(plan), false)
                        .into(),
                    self.context.i64_type().const_int(8, false).into(),
                ],
                "async_frame",
            )
            .map_err(|e| e.to_string())?
            .try_as_basic_value()
            .basic()
            .ok_or("arena allocator returned no frame")?
            .into_pointer_value();
        let completion = self
            .builder
            .build_call(
                self.module.get_function("thaw_promise_new").unwrap(),
                &[],
                "async_completion",
            )
            .map_err(|e| e.to_string())?
            .try_as_basic_value()
            .basic()
            .ok_or("promise allocator returned no value")?
            .into_pointer_value();
        let completion_slot =
            self.async_frame_field(frame, ASYNC_COMPLETION_OFFSET, "completion_slot")?;
        self.builder
            .build_store(completion_slot, completion)
            .map_err(|e| e.to_string())?;
        let waiting_slot = self.async_frame_field(frame, ASYNC_WAITING_OFFSET, "waiting_slot")?;
        self.builder
            .build_store(waiting_slot, ptr_ty.const_null())
            .map_err(|e| e.to_string())?;
        self.bind_async_frame_locals(frame, plan)?;
        for (param_value, param) in ramp.get_param_iter().zip(&func.params) {
            let (slot, _) = self.variables.get(&param.name).copied().ok_or_else(|| {
                format!("missing async frame parameter slot for `{}`", param.name)
            })?;
            self.builder
                .build_store(slot, param_value)
                .map_err(|e| e.to_string())?;
        }
        self.emit_async_segment(&segments[0], frame, completion, resume, 1, true, plan, None)?;

        let resume_entry = self.context.append_basic_block(resume, "entry");
        self.builder.position_at_end(resume_entry);
        self.variables.clear();
        self.catch_stack.clear();
        let resume_frame = resume.get_nth_param(0).unwrap().into_pointer_value();
        let resume_result = resume.get_nth_param(1).unwrap().into_pointer_value();
        let waiting_slot =
            self.async_frame_field(resume_frame, ASYNC_WAITING_OFFSET, "waiting_slot")?;
        let waiting = self
            .builder
            .build_load(ptr_ty, waiting_slot, "waiting_promise")
            .map_err(|e| e.to_string())?
            .into_pointer_value();
        let completion_slot =
            self.async_frame_field(resume_frame, ASYNC_COMPLETION_OFFSET, "completion_slot")?;
        let resume_completion = self
            .builder
            .build_load(ptr_ty, completion_slot, "completion")
            .map_err(|e| e.to_string())?
            .into_pointer_value();
        let state_slot = self.async_frame_field(resume_frame, ASYNC_STATE_OFFSET, "state_slot")?;
        let state = self
            .builder
            .build_load(self.context.i64_type(), state_slot, "async_state")
            .map_err(|e| e.to_string())?
            .into_int_value();
        let promise_state = self
            .builder
            .build_call(
                self.module.get_function("thaw_promise_state").unwrap(),
                &[waiting.into()],
                "waiting_state",
            )
            .map_err(|e| e.to_string())?
            .try_as_basic_value()
            .basic()
            .ok_or("thaw_promise_state returned no value")?
            .into_int_value();
        let is_rejected = self
            .builder
            .build_int_compare(
                IntPredicate::EQ,
                promise_state,
                self.context.i8_type().const_int(2, false),
                "waiting_rejected",
            )
            .map_err(|e| e.to_string())?;
        let rejected = self.context.append_basic_block(resume, "handle_rejection");
        let resume_ok = self.context.append_basic_block(resume, "resume_fulfilled");
        self.builder
            .build_conditional_branch(is_rejected, rejected, resume_ok)
            .map_err(|e| e.to_string())?;

        self.builder.position_at_end(rejected);
        let propagate_rejection = self
            .context
            .append_basic_block(resume, "propagate_rejection");
        let rejection_handlers = segments
            .iter()
            .enumerate()
            .skip(1)
            .filter_map(|(index, segment)| {
                segment.rejection_handler.as_ref().map(|handler| {
                    (
                        index,
                        handler.clone(),
                        self.context
                            .append_basic_block(resume, &format!("catch_rejection_{index}")),
                    )
                })
            })
            .collect::<Vec<_>>();
        let rejection_cases = rejection_handlers
            .iter()
            .map(|(index, _, block)| {
                (
                    self.context.i64_type().const_int(*index as u64, false),
                    *block,
                )
            })
            .collect::<Vec<_>>();
        self.builder
            .build_switch(state, propagate_rejection, &rejection_cases)
            .map_err(|e| e.to_string())?;

        for (_, handler, block) in rejection_handlers {
            self.builder.position_at_end(block);
            for name in [handler.try_guard.as_str(), handler.catch_guard.as_str()] {
                let index = plan
                    .locals
                    .iter()
                    .position(|(local, _)| local == name)
                    .ok_or_else(|| format!("missing async rejection guard `{name}`"))?;
                let slot = self.async_frame_field(
                    resume_frame,
                    self.async_locals_offset(plan) + OBJECT_FIELD_BYTES * index as u64,
                    &format!("frame_{name}"),
                )?;
                let enabled = name == handler.catch_guard;
                self.builder
                    .build_store(
                        slot,
                        self.context.bool_type().const_int(enabled as u64, false),
                    )
                    .map_err(|e| e.to_string())?;
            }
            for name in &handler.disable_guards {
                let index = plan
                    .locals
                    .iter()
                    .position(|(local, _)| local == name)
                    .ok_or_else(|| format!("missing async disabled guard `{name}`"))?;
                let slot = self.async_frame_field(
                    resume_frame,
                    self.async_locals_offset(plan) + OBJECT_FIELD_BYTES * index as u64,
                    &format!("frame_{name}"),
                )?;
                self.builder
                    .build_store(slot, self.context.bool_type().const_zero())
                    .map_err(|e| e.to_string())?;
            }
            let binding_index = plan
                .locals
                .iter()
                .position(|(local, _)| local == &handler.catch_binding)
                .ok_or_else(|| {
                    format!("missing async catch binding `{}`", handler.catch_binding)
                })?;
            let binding_slot = self.async_frame_field(
                resume_frame,
                self.async_locals_offset(plan) + OBJECT_FIELD_BYTES * binding_index as u64,
                &format!("frame_{}", handler.catch_binding),
            )?;
            self.builder
                .build_store(binding_slot, resume_result)
                .map_err(|e| e.to_string())?;
            self.builder
                .build_call(
                    self.module.get_function("thaw_promise_destroy").unwrap(),
                    &[waiting.into()],
                    "destroy_caught_waiting",
                )
                .map_err(|e| e.to_string())?;
            self.builder
                .build_store(waiting_slot, ptr_ty.const_null())
                .map_err(|e| e.to_string())?;
            self.builder
                .build_call(
                    resume,
                    &[resume_frame.into(), resume_frame.into()],
                    "resume_catch",
                )
                .map_err(|e| e.to_string())?;
            self.builder.build_return(None).map_err(|e| e.to_string())?;
        }

        self.builder.position_at_end(propagate_rejection);
        self.builder
            .build_call(
                self.module.get_function("thaw_promise_reject").unwrap(),
                &[resume_completion.into(), resume_result.into()],
                "reject_completion",
            )
            .map_err(|e| e.to_string())?;
        self.builder
            .build_call(
                self.module.get_function("thaw_promise_destroy").unwrap(),
                &[waiting.into()],
                "destroy_rejected_waiting",
            )
            .map_err(|e| e.to_string())?;
        self.builder
            .build_store(waiting_slot, ptr_ty.const_null())
            .map_err(|e| e.to_string())?;
        self.builder.build_return(None).map_err(|e| e.to_string())?;

        self.builder.position_at_end(resume_ok);
        self.builder
            .build_call(
                self.module.get_function("thaw_promise_destroy").unwrap(),
                &[waiting.into()],
                "destroy_waiting",
            )
            .map_err(|e| e.to_string())?;
        self.builder
            .build_store(waiting_slot, ptr_ty.const_null())
            .map_err(|e| e.to_string())?;
        let invalid = self.context.append_basic_block(resume, "invalid_state");
        let case_blocks = (1..segments.len())
            .map(|index| {
                self.context
                    .append_basic_block(resume, &format!("state_{index}"))
            })
            .collect::<Vec<_>>();
        let cases = case_blocks
            .iter()
            .enumerate()
            .map(|(index, block)| {
                (
                    self.context.i64_type().const_int((index + 1) as u64, false),
                    *block,
                )
            })
            .collect::<Vec<_>>();
        self.builder
            .build_switch(state, invalid, &cases)
            .map_err(|e| e.to_string())?;
        self.builder.position_at_end(invalid);
        self.builder.build_return(None).map_err(|e| e.to_string())?;

        for (index, block) in case_blocks.into_iter().enumerate() {
            self.builder.position_at_end(block);
            self.variables.clear();
            self.catch_stack.clear();
            self.bind_async_frame_locals(resume_frame, plan)?;
            self.emit_async_segment(
                &segments[index + 1],
                resume_frame,
                resume_completion,
                resume,
                index + 2,
                false,
                plan,
                Some(resume_result),
            )?;
        }
        Ok(())
    }

    fn emit_async_segment(
        &mut self,
        segment: &AsyncSegment,
        frame: PointerValue<'ctx>,
        completion: PointerValue<'ctx>,
        resume: FunctionValue<'ctx>,
        next_state: usize,
        ramp: bool,
        plan: &FrameAsyncPlan,
        resume_result: Option<PointerValue<'ctx>>,
    ) -> Result<(), String> {
        if let Some((name, ty)) = &segment.resume_target {
            let result_ptr = resume_result.ok_or("async resume result is unavailable")?;
            let llvm_ty = self.basic_type(ty)?;
            let value = self
                .builder
                .build_load(llvm_ty, result_ptr, &format!("awaited_{name}"))
                .map_err(|e| e.to_string())?;
            let (slot, _) = self
                .variables
                .get(name)
                .copied()
                .ok_or_else(|| format!("missing async frame slot for `{name}`"))?;
            self.builder
                .build_store(slot, value)
                .map_err(|e| e.to_string())?;
        }
        match self.compile_async_segment_block(&segment.stmts, frame, completion, plan)? {
            AsyncBlockExit::Returned => {
                self.resolve_async_completion(
                    completion,
                    self.async_completion_result(frame, plan)?,
                    ramp,
                )?;
                return Ok(());
            }
            AsyncBlockExit::Rejected => {
                if ramp {
                    self.builder
                        .build_return(Some(&completion))
                        .map_err(|e| e.to_string())?;
                } else {
                    self.builder.build_return(None).map_err(|e| e.to_string())?;
                }
                return Ok(());
            }
            AsyncBlockExit::Continue => {}
        }
        if let Some(awaited) = &segment.awaited {
            if let Some((guard_name, expected)) = &segment.await_guard {
                let function = self.current_function();
                let schedule = self
                    .context
                    .append_basic_block(function, "await_guard_true");
                let skip = self
                    .context
                    .append_basic_block(function, "await_guard_skip");
                let (guard_slot, guard_ty) = self
                    .variables
                    .get(guard_name)
                    .copied()
                    .ok_or_else(|| format!("missing async branch guard `{guard_name}`"))?;
                let mut condition = self
                    .builder
                    .build_load(guard_ty, guard_slot, guard_name)
                    .map_err(|e| e.to_string())?
                    .into_int_value();
                if !expected {
                    condition = self
                        .builder
                        .build_not(condition, "inverse_branch_guard")
                        .map_err(|e| e.to_string())?;
                }
                self.builder
                    .build_conditional_branch(condition, schedule, skip)
                    .map_err(|e| e.to_string())?;

                self.builder.position_at_end(skip);
                let state_slot = self.async_frame_field(frame, ASYNC_STATE_OFFSET, "state_slot")?;
                self.builder
                    .build_store(
                        state_slot,
                        self.context.i64_type().const_int(next_state as u64, false),
                    )
                    .map_err(|e| e.to_string())?;
                self.builder
                    .build_call(resume, &[frame.into(), frame.into()], "skip_guarded_await")
                    .map_err(|e| e.to_string())?;
                if ramp {
                    self.builder
                        .build_return(Some(&completion))
                        .map_err(|e| e.to_string())?;
                } else {
                    self.builder.build_return(None).map_err(|e| e.to_string())?;
                }
                self.builder.position_at_end(schedule);
            }
            let waiting = match awaited {
                HirExpr::Call(callee, args) if matches!(callee.as_ref(), HirExpr::Var(name) if name == "fetch") => {
                    self.compile_single_arg_call("thaw_http_get_async", args, "async_fetch")?
                        .into_pointer_value()
                }
                _ => self.compile_expr(awaited)?.into_pointer_value(),
            };
            let waiting_slot =
                self.async_frame_field(frame, ASYNC_WAITING_OFFSET, "waiting_slot")?;
            self.builder
                .build_store(waiting_slot, waiting)
                .map_err(|e| e.to_string())?;
            let state_slot = self.async_frame_field(frame, ASYNC_STATE_OFFSET, "state_slot")?;
            let scheduled_state = segment.await_next.unwrap_or(next_state);
            self.builder
                .build_store(
                    state_slot,
                    self.context
                        .i64_type()
                        .const_int(scheduled_state as u64, false),
                )
                .map_err(|e| e.to_string())?;
            self.builder
                .build_call(
                    self.module.get_function("thaw_promise_subscribe").unwrap(),
                    &[
                        waiting.into(),
                        resume.as_global_value().as_pointer_value().into(),
                        frame.into(),
                    ],
                    "subscribe_resume",
                )
                .map_err(|e| e.to_string())?;
            if ramp {
                self.builder
                    .build_return(Some(&completion))
                    .map_err(|e| e.to_string())?;
            } else {
                self.builder.build_return(None).map_err(|e| e.to_string())?;
            }
        } else {
            if plan.ret != HirType::Void {
                return Err("value-returning frame-split async function does not return a value on all paths".to_string());
            }
            self.resolve_async_completion(completion, frame, ramp)?;
        }
        Ok(())
    }

    fn resolve_async_completion(
        &self,
        completion: PointerValue<'ctx>,
        result: PointerValue<'ctx>,
        ramp: bool,
    ) -> Result<(), String> {
        self.builder
            .build_call(
                self.module.get_function("thaw_promise_resolve").unwrap(),
                &[completion.into(), result.into()],
                "resolve_completion",
            )
            .map_err(|e| e.to_string())?;
        if ramp {
            self.builder
                .build_return(Some(&completion))
                .map_err(|e| e.to_string())?;
        } else {
            self.builder.build_return(None).map_err(|e| e.to_string())?;
        }
        Ok(())
    }

    fn bind_async_frame_locals(
        &mut self,
        frame: PointerValue<'ctx>,
        plan: &FrameAsyncPlan,
    ) -> Result<(), String> {
        for (index, (name, ty)) in plan.locals.iter().enumerate() {
            let slot = self.async_frame_field(
                frame,
                self.async_locals_offset(plan) + OBJECT_FIELD_BYTES * index as u64,
                &format!("frame_{name}"),
            )?;
            self.variables
                .insert(name.clone(), (slot, self.basic_type(ty)?));
        }
        Ok(())
    }

    fn compile_async_segment_block(
        &mut self,
        stmts: &[HirStmt],
        frame: PointerValue<'ctx>,
        completion: PointerValue<'ctx>,
        plan: &FrameAsyncPlan,
    ) -> Result<AsyncBlockExit, String> {
        for stmt in stmts {
            if let HirStmt::If(guard, then_body, else_body) = stmt {
                let guarded = match (then_body.as_slice(), else_body.as_slice()) {
                    ([only], []) => Some((only, true)),
                    ([], [only]) => Some((only, false)),
                    _ => None,
                };
                if let Some((HirStmt::Let(name, ty, expr), expected)) = guarded {
                    let function = self.current_function();
                    let initialize = self.context.append_basic_block(function, "guarded_let");
                    let continue_block = self.context.append_basic_block(function, "let_skipped");
                    let mut condition = self.compile_expr(guard)?.into_int_value();
                    if !expected {
                        condition = self
                            .builder
                            .build_not(condition, "inverse_let_guard")
                            .map_err(|e| e.to_string())?;
                    }
                    self.builder
                        .build_conditional_branch(condition, initialize, continue_block)
                        .map_err(|e| e.to_string())?;
                    self.builder.position_at_end(initialize);
                    let value = self.compile_expr(expr)?;
                    let (slot, slot_ty) = self
                        .variables
                        .get(name)
                        .copied()
                        .ok_or_else(|| format!("missing async frame slot for `{name}`"))?;
                    if slot_ty != self.basic_type(ty)? {
                        return Err(format!("async frame local `{name}` changed type"));
                    }
                    self.builder
                        .build_store(slot, value)
                        .map_err(|e| e.to_string())?;
                    self.builder
                        .build_unconditional_branch(continue_block)
                        .map_err(|e| e.to_string())?;
                    self.builder.position_at_end(continue_block);
                    continue;
                }
                if let Some((HirStmt::Return(value), expected)) = guarded {
                    let function = self.current_function();
                    let return_block = self.context.append_basic_block(function, "guarded_return");
                    let continue_block =
                        self.context.append_basic_block(function, "return_skipped");
                    let mut condition = self.compile_expr(guard)?.into_int_value();
                    if !expected {
                        condition = self
                            .builder
                            .build_not(condition, "inverse_return_guard")
                            .map_err(|e| e.to_string())?;
                    }
                    self.builder
                        .build_conditional_branch(condition, return_block, continue_block)
                        .map_err(|e| e.to_string())?;
                    self.builder.position_at_end(return_block);
                    match value {
                        Some(expr) if plan.ret != HirType::Void => {
                            let result = self.compile_expr(expr)?;
                            let result_slot =
                                self.async_frame_field(frame, ASYNC_RESULT_OFFSET, "async_result")?;
                            self.builder
                                .build_store(result_slot, result)
                                .map_err(|e| e.to_string())?;
                        }
                        None if plan.ret == HirType::Void => {}
                        Some(_) => {
                            return Err(
                                "void frame-split async function cannot return a value".to_string()
                            )
                        }
                        None => {
                            return Err(
                                "value-returning frame-split async function cannot use `return;`"
                                    .to_string(),
                            )
                        }
                    }
                    self.resolve_async_completion(
                        completion,
                        self.async_completion_result(frame, plan)?,
                        function.get_type().get_return_type().is_some(),
                    )?;
                    self.builder.position_at_end(continue_block);
                    continue;
                }
                if let Some((HirStmt::Throw(error), expected)) = guarded {
                    let enclosing_handler = match guard {
                        HirExpr::Var(name) => plan.guarded_rethrow_handlers.get(name).cloned(),
                        _ => None,
                    };
                    let function = self.current_function();
                    let reject = self.context.append_basic_block(function, "guarded_rethrow");
                    let continue_block =
                        self.context.append_basic_block(function, "rethrow_skipped");
                    let mut condition = self.compile_expr(guard)?.into_int_value();
                    if !expected {
                        condition = self
                            .builder
                            .build_not(condition, "inverse_throw_guard")
                            .map_err(|e| e.to_string())?;
                    }
                    self.builder
                        .build_conditional_branch(condition, reject, continue_block)
                        .map_err(|e| e.to_string())?;
                    self.builder.position_at_end(reject);
                    let error = self.compile_expr(error)?.into_pointer_value();
                    if let Some(handler) = enclosing_handler {
                        let catch_index = plan
                            .locals
                            .iter()
                            .position(|(name, _)| name == &handler.catch_binding)
                            .ok_or_else(|| {
                                format!("missing async catch binding `{}`", handler.catch_binding)
                            })?;
                        let catch_slot = self.async_frame_field(
                            frame,
                            self.async_locals_offset(plan)
                                + OBJECT_FIELD_BYTES * catch_index as u64,
                            "outer_catch_binding",
                        )?;
                        self.builder
                            .build_store(catch_slot, error)
                            .map_err(|e| e.to_string())?;
                        for (guard_name, value) in
                            [(&handler.try_guard, false), (&handler.catch_guard, true)]
                                .into_iter()
                                .chain(handler.disable_guards.iter().map(|name| (name, false)))
                        {
                            let guard_index = plan
                                .locals
                                .iter()
                                .position(|(name, _)| name == guard_name)
                                .ok_or_else(|| format!("missing async guard `{guard_name}`"))?;
                            let guard_slot = self.async_frame_field(
                                frame,
                                self.async_locals_offset(plan)
                                    + OBJECT_FIELD_BYTES * guard_index as u64,
                                "outer_catch_guard",
                            )?;
                            self.builder
                                .build_store(
                                    guard_slot,
                                    self.context.bool_type().const_int(value as u64, false),
                                )
                                .map_err(|e| e.to_string())?;
                        }
                        self.builder
                            .build_unconditional_branch(continue_block)
                            .map_err(|e| e.to_string())?;
                    } else {
                        self.builder
                            .build_call(
                                self.module.get_function("thaw_promise_reject").unwrap(),
                                &[completion.into(), error.into()],
                                "rethrow_rejection",
                            )
                            .map_err(|e| e.to_string())?;
                        if function.get_type().get_return_type().is_some() {
                            self.builder
                                .build_return(Some(&completion))
                                .map_err(|e| e.to_string())?;
                        } else {
                            self.builder.build_return(None).map_err(|e| e.to_string())?;
                        }
                    }
                    self.builder.position_at_end(continue_block);
                    continue;
                }
            }
            if matches!(stmt, HirStmt::Return(None)) {
                if plan.ret != HirType::Void {
                    return Err(
                        "value-returning frame-split async function cannot use `return;`"
                            .to_string(),
                    );
                }
                return Ok(AsyncBlockExit::Returned);
            }
            if let HirStmt::Return(Some(expr)) = stmt {
                if plan.ret == HirType::Void {
                    return Err("void frame-split async function cannot return a value".to_string());
                }
                let value = self.compile_expr(expr)?;
                let result_slot =
                    self.async_frame_field(frame, ASYNC_RESULT_OFFSET, "async_result")?;
                self.builder
                    .build_store(result_slot, value)
                    .map_err(|e| e.to_string())?;
                return Ok(AsyncBlockExit::Returned);
            }
            if let HirStmt::Throw(expr) = stmt {
                let error = self.compile_expr(expr)?.into_pointer_value();
                self.builder
                    .build_call(
                        self.module.get_function("thaw_promise_reject").unwrap(),
                        &[completion.into(), error.into()],
                        "reject_completion",
                    )
                    .map_err(|e| e.to_string())?;
                return Ok(AsyncBlockExit::Rejected);
            }
            if let HirStmt::Let(name, ty, expr) = stmt {
                let value = self.compile_expr(expr)?;
                let index = plan
                    .locals
                    .iter()
                    .position(|(local, _)| local == name)
                    .ok_or_else(|| format!("missing async frame slot for `{name}`"))?;
                let slot = self.async_frame_field(
                    frame,
                    self.async_locals_offset(plan) + OBJECT_FIELD_BYTES * index as u64,
                    &format!("frame_{name}"),
                )?;
                self.builder
                    .build_store(slot, value)
                    .map_err(|e| e.to_string())?;
                self.variables
                    .insert(name.clone(), (slot, self.basic_type(ty)?));
            } else if self.compile_stmt(stmt)? {
                return Ok(AsyncBlockExit::Returned);
            }
        }
        Ok(AsyncBlockExit::Continue)
    }

    fn async_locals_offset(&self, plan: &FrameAsyncPlan) -> u64 {
        ASYNC_FRAME_BYTES
            + if plan.ret == HirType::Void {
                0
            } else {
                OBJECT_FIELD_BYTES
            }
    }

    fn async_frame_size(&self, plan: &FrameAsyncPlan) -> u64 {
        self.async_locals_offset(plan) + OBJECT_FIELD_BYTES * plan.locals.len() as u64
    }

    fn async_completion_result(
        &self,
        frame: PointerValue<'ctx>,
        plan: &FrameAsyncPlan,
    ) -> Result<PointerValue<'ctx>, String> {
        if plan.ret == HirType::Void {
            Ok(frame)
        } else {
            self.async_frame_field(frame, ASYNC_RESULT_OFFSET, "async_result")
        }
    }

    /// Compiles a statement list, stopping early if one of them
    /// unconditionally leaves the block (`return`/`throw`, or an `if` whose
    /// branches all do). Returns `true` when that happened, so callers know
    /// not to fall through past this block.
    fn compile_block(&mut self, stmts: &[HirStmt]) -> Result<bool, String> {
        for stmt in stmts {
            if self.compile_stmt(stmt)? {
                return Ok(true);
            }
        }
        Ok(false)
    }

    fn compile_stmt(&mut self, stmt: &HirStmt) -> Result<bool, String> {
        match stmt {
            HirStmt::Expr(expr) => {
                self.compile_expr(expr)?;
                Ok(false)
            }

            HirStmt::Return(value) => {
                match value {
                    Some(expr) => {
                        let val = self.compile_expr(expr)?;
                        self.builder
                            .build_return(Some(&val))
                            .map_err(|e| e.to_string())?;
                    }
                    None => {
                        self.builder.build_return(None).map_err(|e| e.to_string())?;
                    }
                }
                Ok(true)
            }

            HirStmt::Let(name, ty, expr) => {
                let val = self.compile_expr(expr)?;
                let llvm_ty = self.basic_type(ty)?;
                let slot = self
                    .builder
                    .build_alloca(llvm_ty, name)
                    .map_err(|e| e.to_string())?;
                self.builder
                    .build_store(slot, val)
                    .map_err(|e| e.to_string())?;
                self.variables.insert(name.clone(), (slot, llvm_ty));
                Ok(false)
            }

            HirStmt::If(cond, then_branch, else_branch) => {
                self.compile_if(cond, then_branch, else_branch)
            }

            HirStmt::While(cond, body) => self.compile_while(cond, body),

            HirStmt::Break => {
                let (_, break_target) = self
                    .loop_stack
                    .last()
                    .copied()
                    .ok_or("`break` used outside a loop")?;
                self.builder
                    .build_unconditional_branch(break_target)
                    .map_err(|e| e.to_string())?;
                Ok(true)
            }

            HirStmt::Continue => {
                let (continue_target, _) = self
                    .loop_stack
                    .last()
                    .copied()
                    .ok_or("`continue` used outside a loop")?;
                self.builder
                    .build_unconditional_branch(continue_target)
                    .map_err(|e| e.to_string())?;
                Ok(true)
            }

            HirStmt::Throw(expr) => {
                let val = self.compile_expr(expr)?;
                self.builder
                    .build_store(self.pending_exception().as_pointer_value(), val)
                    .map_err(|e| e.to_string())?;
                if let Some(catch_bb) = self.catch_stack.last().copied() {
                    self.builder
                        .build_unconditional_branch(catch_bb)
                        .map_err(|e| e.to_string())?;
                } else {
                    self.build_default_return()?;
                }
                Ok(true)
            }

            HirStmt::Try(body, catch_name, catch_body) => {
                self.compile_try(body, catch_name, catch_body)
            }
        }
    }

    fn compile_if(
        &mut self,
        cond: &HirExpr,
        then_branch: &[HirStmt],
        else_branch: &[HirStmt],
    ) -> Result<bool, String> {
        let function = self.current_function();
        let cond_val = self.compile_expr(cond)?.into_int_value();

        let then_bb = self.context.append_basic_block(function, "then");
        let else_bb = self.context.append_basic_block(function, "else");
        let merge_bb = self.context.append_basic_block(function, "ifcont");

        self.builder
            .build_conditional_branch(cond_val, then_bb, else_bb)
            .map_err(|e| e.to_string())?;

        self.builder.position_at_end(then_bb);
        let then_terminated = self.compile_block(then_branch)?;
        if !then_terminated {
            self.builder
                .build_unconditional_branch(merge_bb)
                .map_err(|e| e.to_string())?;
        }

        self.builder.position_at_end(else_bb);
        let else_terminated = self.compile_block(else_branch)?;
        if !else_terminated {
            self.builder
                .build_unconditional_branch(merge_bb)
                .map_err(|e| e.to_string())?;
        }

        self.builder.position_at_end(merge_bb);
        if then_terminated && else_terminated {
            // merge_bb has no predecessors; give it a terminator so the
            // module stays valid IR, then report that this whole
            // if-statement terminates its enclosing block.
            self.builder
                .build_unreachable()
                .map_err(|e| e.to_string())?;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    fn compile_while(&mut self, cond: &HirExpr, body: &[HirStmt]) -> Result<bool, String> {
        let function = self.current_function();

        let header_bb = self.context.append_basic_block(function, "whilecond");
        let body_bb = self.context.append_basic_block(function, "whilebody");
        let after_bb = self.context.append_basic_block(function, "whileend");

        self.builder
            .build_unconditional_branch(header_bb)
            .map_err(|e| e.to_string())?;

        self.builder.position_at_end(header_bb);
        let cond_val = self.compile_expr(cond)?.into_int_value();
        self.builder
            .build_conditional_branch(cond_val, body_bb, after_bb)
            .map_err(|e| e.to_string())?;

        self.builder.position_at_end(body_bb);
        self.loop_stack.push((header_bb, after_bb));
        let body_terminated = self.compile_block(body)?;
        self.loop_stack.pop();
        if !body_terminated {
            self.builder
                .build_unconditional_branch(header_bb)
                .map_err(|e| e.to_string())?;
        }

        self.builder.position_at_end(after_bb);
        // `after_bb` is always reachable (the condition can be false on the
        // first check), so a `while` never terminates its enclosing block.
        Ok(false)
    }

    fn compile_try(
        &mut self,
        body: &[HirStmt],
        catch_name: &str,
        catch_body: &[HirStmt],
    ) -> Result<bool, String> {
        let function = self.current_function();

        let try_bb = self.context.append_basic_block(function, "try");
        let catch_bb = self.context.append_basic_block(function, "catch");
        let merge_bb = self.context.append_basic_block(function, "trycont");

        let ptr_ty = self.context.ptr_type(AddressSpace::default());
        let str_ty: BasicTypeEnum = ptr_ty.into();
        let catch_slot = self
            .builder
            .build_alloca(str_ty, "catch_slot")
            .map_err(|e| e.to_string())?;

        self.builder
            .build_unconditional_branch(try_bb)
            .map_err(|e| e.to_string())?;

        self.builder.position_at_end(try_bb);
        self.catch_stack.push(catch_bb);
        let try_terminated = self.compile_block(body)?;
        self.catch_stack.pop();
        if !try_terminated {
            self.builder
                .build_unconditional_branch(merge_bb)
                .map_err(|e| e.to_string())?;
        }

        self.builder.position_at_end(catch_bb);
        let thrown = self
            .builder
            .build_load(
                ptr_ty,
                self.pending_exception().as_pointer_value(),
                "caught_exception",
            )
            .map_err(|e| e.to_string())?;
        self.builder
            .build_store(catch_slot, thrown)
            .map_err(|e| e.to_string())?;
        self.builder
            .build_store(
                self.pending_exception().as_pointer_value(),
                ptr_ty.const_null(),
            )
            .map_err(|e| e.to_string())?;
        self.variables
            .insert(catch_name.to_string(), (catch_slot, str_ty));
        let catch_terminated = self.compile_block(catch_body)?;
        if !catch_terminated {
            self.builder
                .build_unconditional_branch(merge_bb)
                .map_err(|e| e.to_string())?;
        }

        self.builder.position_at_end(merge_bb);
        if try_terminated && catch_terminated {
            self.builder
                .build_unreachable()
                .map_err(|e| e.to_string())?;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    fn current_function(&self) -> FunctionValue<'ctx> {
        self.builder
            .get_insert_block()
            .and_then(|bb| bb.get_parent())
            .expect("builder must be positioned inside a function")
    }

    fn compile_expr(&mut self, expr: &HirExpr) -> Result<BasicValueEnum<'ctx>, String> {
        match expr {
            HirExpr::Lit(HirLit::F64(n)) => Ok(self.context.f64_type().const_float(*n).into()),
            HirExpr::Lit(HirLit::Bool(b)) => {
                Ok(self.context.bool_type().const_int(*b as u64, false).into())
            }
            HirExpr::Lit(HirLit::Str(s)) => {
                let global = self
                    .builder
                    .build_global_string_ptr(s, "strlit")
                    .map_err(|e| e.to_string())?;
                Ok(global.as_pointer_value().into())
            }

            HirExpr::Var(name) => {
                let (ptr, ty) = *self
                    .variables
                    .get(name)
                    .ok_or_else(|| format!("unknown variable `{name}`"))?;
                self.builder
                    .build_load(ty, ptr, name)
                    .map_err(|e| e.to_string())
            }

            HirExpr::Assign(name, value) => {
                let val = self.compile_expr(value)?;
                let (ptr, _ty) = *self
                    .variables
                    .get(name)
                    .ok_or_else(|| format!("assignment to undeclared variable `{name}`"))?;
                self.builder
                    .build_store(ptr, val)
                    .map_err(|e| e.to_string())?;
                Ok(val)
            }

            HirExpr::BinOp(op, lhs, rhs) => self.compile_binop(*op, lhs, rhs),

            HirExpr::Call(callee, args) => self.compile_call(callee, args),
            HirExpr::FfiCall(sig, args) => self.compile_ffi_call(sig, args),

            HirExpr::ArrayLit(elems) => self.compile_array_lit(elems),
            HirExpr::Index(arr, idx) => {
                let elem_ptr = self.compile_element_ptr(arr, idx)?;
                self.builder
                    .build_load(self.context.f64_type(), elem_ptr, "elem")
                    .map_err(|e| e.to_string())
            }
            HirExpr::IndexAssign(arr, idx, value) => {
                let elem_ptr = self.compile_element_ptr(arr, idx)?;
                let val = self.compile_expr(value)?;
                self.builder
                    .build_store(elem_ptr, val)
                    .map_err(|e| e.to_string())?;
                Ok(val)
            }
            HirExpr::ArrayLen(arr) => {
                let arr_ptr = self.compile_expr(arr)?.into_pointer_value();
                let len = self
                    .builder
                    .build_load(self.context.i64_type(), arr_ptr, "arrlen_i64")
                    .map_err(|e| e.to_string())?
                    .into_int_value();
                self.builder
                    .build_signed_int_to_float(len, self.context.f64_type(), "arrlen")
                    .map(Into::into)
                    .map_err(|e| e.to_string())
            }

            HirExpr::EnvVar(name) => self.compile_env_var(name),

            HirExpr::JsonGet(obj, field) => self.compile_json_get(obj, field),
            HirExpr::JsonIndex(obj, idx) => self.compile_json_index(obj, idx),
            HirExpr::JsonAsNumber(inner) => self.compile_json_as(inner, "thaw_json_as_number"),
            HirExpr::JsonAsString(inner) => self.compile_json_as(inner, "thaw_json_as_string"),
            HirExpr::JsonAsBool(inner) => self.compile_json_as_bool(inner),

            // V1 async/await (docs/design/async-await.md): `await` is an
            // identity transform -- `Promise` was already erased at
            // lowering time (async functions' `ret` is the unwrapped `T`),
            // so there's nothing left to suspend on here.
            HirExpr::Await(inner) => self.compile_await(inner),

            HirExpr::ObjectLit(fields) => self.compile_object_lit(fields),
            HirExpr::PropAccess(obj, object_ty, field) => {
                let field_ty = self.field_type(object_ty, field)?;
                let llvm_ty = self.basic_type(&field_ty)?;
                let field_ptr = self.compile_field_ptr(obj, object_ty, field)?;
                self.builder
                    .build_load(llvm_ty, field_ptr, "field")
                    .map_err(|e| e.to_string())
            }
            HirExpr::PropAssign(obj, object_ty, field, value) => {
                let field_ptr = self.compile_field_ptr(obj, object_ty, field)?;
                let val = self.compile_expr(value)?;
                self.builder
                    .build_store(field_ptr, val)
                    .map_err(|e| e.to_string())?;
                Ok(val)
            }

            other => Err(format!("Phase 1/2 codegen does not support {other:?} yet")),
        }
    }

    /// Allocates `[i64 length][f64 elem0]...[f64 elemN-1]` from the arena
    /// and returns a pointer to the start of the buffer (the array value).
    fn compile_array_lit(&mut self, elems: &[HirExpr]) -> Result<BasicValueEnum<'ctx>, String> {
        let elem_vals = elems
            .iter()
            .map(|e| self.compile_expr(e))
            .collect::<Result<Vec<_>, _>>()?;

        let size = ARRAY_HEADER_BYTES + ARRAY_ELEM_BYTES * elem_vals.len() as u64;
        let i64_type = self.context.i64_type();
        let size_val = i64_type.const_int(size, false);
        let align_val = i64_type.const_int(ARRAY_ELEM_BYTES, false);

        let alloc_fn = self.module.get_function("thaw_arena_alloc").unwrap();
        let call = self
            .builder
            .build_call(alloc_fn, &[size_val.into(), align_val.into()], "arr_alloc")
            .map_err(|e| e.to_string())?;
        let base_ptr = call
            .try_as_basic_value()
            .basic()
            .ok_or("thaw_arena_alloc did not return a value")?
            .into_pointer_value();

        self.builder
            .build_store(base_ptr, i64_type.const_int(elem_vals.len() as u64, false))
            .map_err(|e| e.to_string())?;

        for (i, val) in elem_vals.into_iter().enumerate() {
            let offset =
                i64_type.const_int(ARRAY_HEADER_BYTES + ARRAY_ELEM_BYTES * i as u64, false);
            let elem_ptr = unsafe {
                self.builder
                    .build_in_bounds_gep(self.context.i8_type(), base_ptr, &[offset], "elem_ptr")
                    .map_err(|e| e.to_string())?
            };
            self.builder
                .build_store(elem_ptr, val)
                .map_err(|e| e.to_string())?;
        }

        Ok(base_ptr.into())
    }

    /// Computes the address of `array[index]` (past the length header).
    fn compile_element_ptr(
        &mut self,
        array: &HirExpr,
        index: &HirExpr,
    ) -> Result<PointerValue<'ctx>, String> {
        let arr_ptr = self.compile_expr(array)?.into_pointer_value();
        let idx_val = self.compile_expr(index)?.into_float_value();

        let i64_type = self.context.i64_type();
        let idx_int = self
            .builder
            .build_float_to_signed_int(idx_val, i64_type, "idx")
            .map_err(|e| e.to_string())?;
        let elem_size = i64_type.const_int(ARRAY_ELEM_BYTES, false);
        let byte_offset = self
            .builder
            .build_int_mul(idx_int, elem_size, "byteoff")
            .map_err(|e| e.to_string())?;
        let byte_offset = self
            .builder
            .build_int_add(
                byte_offset,
                i64_type.const_int(ARRAY_HEADER_BYTES, false),
                "byteoff_hdr",
            )
            .map_err(|e| e.to_string())?;

        unsafe {
            self.builder
                .build_in_bounds_gep(self.context.i8_type(), arr_ptr, &[byte_offset], "elem_ptr")
                .map_err(|e| e.to_string())
        }
    }

    /// Allocates `[f64 field0]...[f64 fieldN-1]` from the arena, in the
    /// order `fields` lists them (thaw-hir's lowering already reordered
    /// the literal to match its declared type, so this order is always the
    /// declared one, not whatever order the user happened to write). No
    /// length header: field count/order is static, part of the type, so
    /// unlike arrays there's nothing to record at runtime.
    fn compile_object_lit(
        &mut self,
        fields: &[(String, HirExpr)],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let field_vals = fields
            .iter()
            .map(|(_, expr)| self.compile_expr(expr))
            .collect::<Result<Vec<_>, _>>()?;

        let size = OBJECT_FIELD_BYTES * field_vals.len() as u64;
        let i64_type = self.context.i64_type();
        let size_val = i64_type.const_int(size.max(1), false);
        let align_val = i64_type.const_int(OBJECT_FIELD_BYTES, false);

        let alloc_fn = self.module.get_function("thaw_arena_alloc").unwrap();
        let call = self
            .builder
            .build_call(alloc_fn, &[size_val.into(), align_val.into()], "obj_alloc")
            .map_err(|e| e.to_string())?;
        let base_ptr = call
            .try_as_basic_value()
            .basic()
            .ok_or("thaw_arena_alloc did not return a value")?
            .into_pointer_value();

        for (i, val) in field_vals.into_iter().enumerate() {
            let offset = i64_type.const_int(OBJECT_FIELD_BYTES * i as u64, false);
            let field_ptr = unsafe {
                self.builder
                    .build_in_bounds_gep(self.context.i8_type(), base_ptr, &[offset], "field_ptr")
                    .map_err(|e| e.to_string())?
            };
            self.builder
                .build_store(field_ptr, val)
                .map_err(|e| e.to_string())?;
        }

        Ok(base_ptr.into())
    }

    /// Looks up `field`'s declared type within `object_ty`, so a read
    /// knows whether to load an `f64`, a pointer (nested object/array/
    /// string/json), etc. -- fields are no longer assumed to all be `f64`
    /// now that nested objects are supported.
    fn field_type(&self, object_ty: &HirType, field: &str) -> Result<HirType, String> {
        let HirType::Object(fields) = object_ty else {
            return Err(format!(
                "`.{field}` used on a non-object type {object_ty:?}"
            ));
        };
        fields
            .iter()
            .find(|(name, _)| name == field)
            .map(|(_, ty)| ty.clone())
            .ok_or_else(|| format!("object has no field `{field}`"))
    }

    /// Computes the address of `object.field`, from `object_ty`'s
    /// (statically known, per `HirExpr::PropAccess`'s payload) field order.
    fn compile_field_ptr(
        &mut self,
        object: &HirExpr,
        object_ty: &HirType,
        field: &str,
    ) -> Result<PointerValue<'ctx>, String> {
        let HirType::Object(fields) = object_ty else {
            return Err(format!(
                "`.{field}` used on a non-object type {object_ty:?}"
            ));
        };
        let index = fields
            .iter()
            .position(|(name, _)| name == field)
            .ok_or_else(|| format!("object has no field `{field}`"))?;

        let obj_ptr = self.compile_expr(object)?.into_pointer_value();
        let offset = self
            .context
            .i64_type()
            .const_int(OBJECT_FIELD_BYTES * index as u64, false);

        unsafe {
            self.builder
                .build_in_bounds_gep(self.context.i8_type(), obj_ptr, &[offset], "field_ptr")
                .map_err(|e| e.to_string())
        }
    }

    /// `process.env.NAME`, via libc `getenv`. Returns an empty string
    /// instead of a null pointer when the variable is unset, since Str
    /// values elsewhere (puts/printf %s) assume a valid C string.
    /// Calls a declared extern function of shape `ptr (ptr)` with a single
    /// compiled argument -- the shared shape of `fetch`/`JSON.parse`/
    /// `JSON.stringify`.
    fn compile_single_arg_call(
        &mut self,
        fn_name: &str,
        args: &[HirExpr],
        source_name: &str,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let [arg] = args else {
            return Err(format!("`{source_name}` expects exactly one argument"));
        };
        let arg_val = self.compile_expr(arg)?;
        let function = self.module.get_function(fn_name).unwrap();
        let call = self
            .builder
            .build_call(function, &[arg_val.into()], "call")
            .map_err(|e| e.to_string())?;
        call.try_as_basic_value()
            .basic()
            .ok_or_else(|| format!("`{fn_name}` did not return a value"))
    }

    /// `json.field`, via thaw-std's `thaw_json_get`.
    fn compile_json_get(
        &mut self,
        obj: &HirExpr,
        field: &str,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let obj_val = self.compile_expr(obj)?;
        let key_global = self
            .builder
            .build_global_string_ptr(field, "jsonkey")
            .map_err(|e| e.to_string())?;
        let get_fn = self.module.get_function("thaw_json_get").unwrap();
        let call = self
            .builder
            .build_call(
                get_fn,
                &[obj_val.into(), key_global.as_pointer_value().into()],
                "json_get",
            )
            .map_err(|e| e.to_string())?;
        call.try_as_basic_value()
            .basic()
            .ok_or_else(|| "thaw_json_get did not return a value".to_string())
    }

    /// `json[index]`, via thaw-std's `thaw_json_index`.
    fn compile_json_index(
        &mut self,
        obj: &HirExpr,
        index: &HirExpr,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let obj_val = self.compile_expr(obj)?;
        let idx_val = self.compile_expr(index)?.into_float_value();
        let idx_i64 = self
            .builder
            .build_float_to_signed_int(idx_val, self.context.i64_type(), "jsonidx")
            .map_err(|e| e.to_string())?;
        let index_fn = self.module.get_function("thaw_json_index").unwrap();
        let call = self
            .builder
            .build_call(index_fn, &[obj_val.into(), idx_i64.into()], "json_index")
            .map_err(|e| e.to_string())?;
        call.try_as_basic_value()
            .basic()
            .ok_or_else(|| "thaw_json_index did not return a value".to_string())
    }

    /// `Number(json)`/`String(json)`/`Boolean(json)`.
    fn compile_json_as(
        &mut self,
        inner: &HirExpr,
        fn_name: &str,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let val = self.compile_expr(inner)?;
        let function = self.module.get_function(fn_name).unwrap();
        let call = self
            .builder
            .build_call(function, &[val.into()], "json_as")
            .map_err(|e| e.to_string())?;
        call.try_as_basic_value()
            .basic()
            .ok_or_else(|| format!("`{fn_name}` did not return a value"))
    }

    /// `Boolean(json)`. Separate from `compile_json_as`: `thaw_json_as_bool`
    /// returns `i8` (0/1), not `i1`, so the result needs converting to
    /// match how `HirType::Bool` is represented everywhere else.
    fn compile_json_as_bool(&mut self, inner: &HirExpr) -> Result<BasicValueEnum<'ctx>, String> {
        let val = self.compile_expr(inner)?;
        let function = self.module.get_function("thaw_json_as_bool").unwrap();
        let call = self
            .builder
            .build_call(function, &[val.into()], "json_as_bool_u8")
            .map_err(|e| e.to_string())?;
        let u8_val = call
            .try_as_basic_value()
            .basic()
            .ok_or("thaw_json_as_bool did not return a value")?
            .into_int_value();
        let zero = self.context.i8_type().const_int(0, false);
        self.builder
            .build_int_compare(inkwell::IntPredicate::NE, u8_val, zero, "json_as_bool")
            .map(Into::into)
            .map_err(|e| e.to_string())
    }

    /// `loadScript(source): boolean`, via thaw-quickjs's `thaw_js_load`.
    /// Same `i8` -> `i1` conversion as `compile_json_as_bool` and for the
    /// same reason (the extern function avoids relying on `bool`'s C ABI
    /// shape).
    fn compile_load_script(&mut self, args: &[HirExpr]) -> Result<BasicValueEnum<'ctx>, String> {
        let [source] = args else {
            return Err("loadScript expects exactly one argument".to_string());
        };
        let source_val = self.compile_expr(source)?;
        let function = self.module.get_function("thaw_js_load").unwrap();
        let call = self
            .builder
            .build_call(function, &[source_val.into()], "load_script_u8")
            .map_err(|e| e.to_string())?;
        let u8_val = call
            .try_as_basic_value()
            .basic()
            .ok_or("thaw_js_load did not return a value")?
            .into_int_value();
        let zero = self.context.i8_type().const_int(0, false);
        self.builder
            .build_int_compare(inkwell::IntPredicate::NE, u8_val, zero, "load_script_ok")
            .map(Into::into)
            .map_err(|e| e.to_string())
    }

    /// `callDynamic(name, args): Json` -- the QuickJS-NG fallback path
    /// (docs/design/bridge.md section 7). Composes thaw-std's
    /// `thaw_json_stringify`/`thaw_json_parse` with thaw-quickjs's
    /// `thaw_js_call` so a `Json` value flows in and out without this
    /// module needing to know thaw-quickjs's internals (or vice versa).
    fn compile_call_dynamic(&mut self, args: &[HirExpr]) -> Result<BasicValueEnum<'ctx>, String> {
        let [name, call_args] = args else {
            return Err("callDynamic expects exactly two arguments (name, args)".to_string());
        };
        let name_val = self.compile_expr(name)?;
        let args_json_val = self.compile_expr(call_args)?;

        let stringify_fn = self.module.get_function("thaw_json_stringify").unwrap();
        let args_json_str = self
            .builder
            .build_call(
                stringify_fn,
                &[args_json_val.into()],
                "call_dynamic_args_json",
            )
            .map_err(|e| e.to_string())?
            .try_as_basic_value()
            .basic()
            .ok_or("thaw_json_stringify did not return a value")?;

        let call_fn = self.module.get_function("thaw_js_call").unwrap();
        let result_json_str = self
            .builder
            .build_call(
                call_fn,
                &[name_val.into(), args_json_str.into()],
                "call_dynamic_result_json",
            )
            .map_err(|e| e.to_string())?
            .try_as_basic_value()
            .basic()
            .ok_or("thaw_js_call did not return a value")?;

        let parse_fn = self.module.get_function("thaw_json_parse").unwrap();
        self.builder
            .build_call(parse_fn, &[result_json_str.into()], "call_dynamic_result")
            .map_err(|e| e.to_string())?
            .try_as_basic_value()
            .basic()
            .ok_or_else(|| "thaw_json_parse did not return a value".to_string())
    }

    fn compile_env_var(&mut self, name: &str) -> Result<BasicValueEnum<'ctx>, String> {
        let name_global = self
            .builder
            .build_global_string_ptr(name, "envname")
            .map_err(|e| e.to_string())?;
        let getenv_fn = self.module.get_function("getenv").unwrap();
        let call = self
            .builder
            .build_call(
                getenv_fn,
                &[name_global.as_pointer_value().into()],
                "getenv_call",
            )
            .map_err(|e| e.to_string())?;
        let raw_ptr = call
            .try_as_basic_value()
            .basic()
            .ok_or("getenv did not return a value")?
            .into_pointer_value();

        let function = self.current_function();
        let null_bb = self.context.append_basic_block(function, "envnull");
        let notnull_bb = self.context.append_basic_block(function, "envnotnull");
        let merge_bb = self.context.append_basic_block(function, "envmerge");

        let is_null = self
            .builder
            .build_is_null(raw_ptr, "is_null")
            .map_err(|e| e.to_string())?;
        self.builder
            .build_conditional_branch(is_null, null_bb, notnull_bb)
            .map_err(|e| e.to_string())?;

        self.builder.position_at_end(null_bb);
        let empty = self
            .builder
            .build_global_string_ptr("", "envempty")
            .map_err(|e| e.to_string())?;
        self.builder
            .build_unconditional_branch(merge_bb)
            .map_err(|e| e.to_string())?;

        self.builder.position_at_end(notnull_bb);
        self.builder
            .build_unconditional_branch(merge_bb)
            .map_err(|e| e.to_string())?;

        self.builder.position_at_end(merge_bb);
        let ptr_ty = self.context.ptr_type(AddressSpace::default());
        let phi = self
            .builder
            .build_phi(ptr_ty, "envval")
            .map_err(|e| e.to_string())?;
        phi.add_incoming(&[(&empty.as_pointer_value(), null_bb), (&raw_ptr, notnull_bb)]);
        Ok(phi.as_basic_value())
    }

    fn compile_binop(
        &mut self,
        op: BinOp,
        lhs: &HirExpr,
        rhs: &HirExpr,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let lhs_val = self.compile_expr(lhs)?.into_float_value();
        let rhs_val = self.compile_expr(rhs)?.into_float_value();

        match op {
            BinOp::Add => self
                .builder
                .build_float_add(lhs_val, rhs_val, "addtmp")
                .map(Into::into)
                .map_err(|e| e.to_string()),
            BinOp::Sub => self
                .builder
                .build_float_sub(lhs_val, rhs_val, "subtmp")
                .map(Into::into)
                .map_err(|e| e.to_string()),
            BinOp::Mul => self
                .builder
                .build_float_mul(lhs_val, rhs_val, "multmp")
                .map(Into::into)
                .map_err(|e| e.to_string()),
            BinOp::Div => self
                .builder
                .build_float_div(lhs_val, rhs_val, "divtmp")
                .map(Into::into)
                .map_err(|e| e.to_string()),
            BinOp::Lt => self
                .builder
                .build_float_compare(FloatPredicate::OLT, lhs_val, rhs_val, "lttmp")
                .map(Into::into)
                .map_err(|e| e.to_string()),
            BinOp::Gt => self
                .builder
                .build_float_compare(FloatPredicate::OGT, lhs_val, rhs_val, "gttmp")
                .map(Into::into)
                .map_err(|e| e.to_string()),
            BinOp::EqEqEq => self
                .builder
                .build_float_compare(FloatPredicate::OEQ, lhs_val, rhs_val, "eqtmp")
                .map(Into::into)
                .map_err(|e| e.to_string()),
        }
    }

    fn compile_call(
        &mut self,
        callee: &HirExpr,
        args: &[HirExpr],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let HirExpr::Var(name) = callee else {
            return Err("call target must be a plain name in Phase 0/1".to_string());
        };

        match name.as_str() {
            "console.log" => return self.compile_console_log(args),
            "fetch" => return self.compile_single_arg_call("thaw_fetch_get", args, "fetch"),
            "sleep" => return self.compile_sleep(args),
            "JSON.parse" => {
                return self.compile_single_arg_call("thaw_json_parse", args, "JSON.parse")
            }
            "JSON.stringify" => {
                return self.compile_single_arg_call("thaw_json_stringify", args, "JSON.stringify")
            }
            "loadScript" => return self.compile_load_script(args),
            "callDynamic" => return self.compile_call_dynamic(args),
            _ => {}
        }

        let symbol = Self::llvm_symbol_for(name);
        let function = self
            .module
            .get_function(&symbol)
            .ok_or_else(|| format!("call to undeclared function `{name}`"))?;

        self.build_call_with(function, args, name)
    }

    fn compile_sleep(&mut self, args: &[HirExpr]) -> Result<BasicValueEnum<'ctx>, String> {
        let [milliseconds] = args else {
            return Err("sleep expects exactly one millisecond argument".to_string());
        };
        let milliseconds = self.compile_expr(milliseconds)?.into_float_value();
        let milliseconds = self
            .builder
            .build_float_to_unsigned_int(milliseconds, self.context.i64_type(), "sleep_ms")
            .map_err(|e| e.to_string())?;
        let sleep = self.module.get_function("thaw_sleep_ms").unwrap();
        self.builder
            .build_call(sleep, &[milliseconds.into()], "sleep_promise")
            .map_err(|e| e.to_string())?
            .try_as_basic_value()
            .basic()
            .ok_or_else(|| "thaw_sleep_ms did not return a promise".to_string())
    }

    fn compile_await(&mut self, inner: &HirExpr) -> Result<BasicValueEnum<'ctx>, String> {
        let is_sleep = matches!(
            inner,
            HirExpr::Call(callee, _) if matches!(callee.as_ref(), HirExpr::Var(name) if name == "sleep")
        );
        let value = self.compile_expr(inner)?;
        if !is_sleep {
            // User-defined async functions still use the V1 synchronous ABI.
            return Ok(value);
        }

        let promise = value.into_pointer_value();
        let run_until = self
            .module
            .get_function("thaw_runtime_run_until_resolved")
            .unwrap();
        let result = self
            .builder
            .build_call(run_until, &[promise.into()], "await_result")
            .map_err(|e| e.to_string())?
            .try_as_basic_value()
            .basic()
            .ok_or("thaw_runtime_run_until_resolved did not return a value")?
            .into_pointer_value();
        let resolved = self
            .builder
            .build_is_not_null(result, "await_resolved")
            .map_err(|e| e.to_string())?;
        Ok(resolved.into())
    }

    /// `HirExpr::FfiCall` -- an ambient `declare function` call (see
    /// docs/design/bridge.md). By the time codegen sees this, the symbol
    /// is already declared (`declare_extern_function` ran in the
    /// `compile_program` pre-pass) with the *adapted* C ABI parameter list
    /// `ffi_param_types` computed, so unlike a normal Thaw-to-Thaw call
    /// (`build_call_with`), an `Array`/`Object` argument here must be
    /// unpacked into that same adapted shape before the call -- each match
    /// arm below has a matching arm in `ffi_param_types`'s doc comment.
    fn compile_ffi_call(
        &mut self,
        sig: &FfiSignature,
        args: &[HirExpr],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let function = self
            .module
            .get_function(&sig.symbol)
            .expect("extern function was declared in compile_program's pre-pass");

        let mut compiled_args: Vec<BasicMetadataValueEnum<'ctx>> = Vec::with_capacity(args.len());
        for (param_ty, arg) in sig.params.iter().zip(args) {
            let value = self.compile_expr(arg)?;
            match param_ty {
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
                        let offset = i64_type.const_int(OBJECT_FIELD_BYTES * i as u64, false);
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

        let call_site = self
            .builder
            .build_call(function, &compiled_args, "ffi_calltmp")
            .map_err(|e| e.to_string())?;
        call_site
            .try_as_basic_value()
            .basic()
            .ok_or_else(|| format!("`{}` does not return a value", sig.symbol))
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

        call_site
            .try_as_basic_value()
            .basic()
            .ok_or_else(|| format!("`{name}` does not return a value"))
    }

    /// `console.log` is bridged straight to libc for Phase 0/1: strings go
    /// to `puts`, numbers go through `printf("%g\n", ...)`. The real
    /// `console` implementation belongs in `std/` once Phase 2 gets there.
    fn compile_console_log(&mut self, args: &[HirExpr]) -> Result<BasicValueEnum<'ctx>, String> {
        let [arg] = args else {
            return Err("console.log expects exactly one argument in Phase 0/1".to_string());
        };
        let value = self.compile_expr(arg)?;

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

        // Flush immediately -- see the comment on `fflush`'s declaration.
        let fflush_fn = self.module.get_function("fflush").unwrap();
        let null_ptr = self.context.ptr_type(AddressSpace::default()).const_null();
        self.builder
            .build_call(fflush_fn, &[null_ptr.into()], "fflush_call")
            .map_err(|e| e.to_string())?;

        Ok(self.context.i32_type().const_int(0, false).into())
    }

    /// Emits `int main(void) { thaw_user_main(); return 0; }`, the real
    /// process entry point that the system linker/CRT expects, for
    /// ordinary (non-Lambda) programs that define `main`.
    fn emit_c_main_entry(&mut self) {
        let user_main = self.module.get_function(USER_MAIN_SYMBOL).unwrap();

        let (_main_fn, entry) = self.new_c_main();
        self.builder.position_at_end(entry);
        self.call_module_init_if_present();
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
        }
        self.finish_c_main();
    }

    /// Calls `__thaw_module_init` before user code, if the program defines
    /// one (see `MODULE_INIT_SYMBOL`). A no-op for programs with no
    /// registry packages that ship a `bundle.js`.
    fn call_module_init_if_present(&self) {
        if let Some(init_fn) = self.module.get_function(MODULE_INIT_SYMBOL) {
            self.builder
                .build_call(init_fn, &[], "call_thaw_module_init")
                .unwrap();
        }
    }

    /// Emits `int main(void) { thaw_runtime_run(&handler); return 0; }` for
    /// programs that define `handler` instead of `main` -- `thaw_runtime_run`
    /// (see thaw-runtime) never actually returns; it polls the Lambda
    /// Runtime API forever.
    fn emit_lambda_entry(&mut self, handler_hir: &HirFunction) -> Result<(), String> {
        let valid_signature = handler_hir.params.len() == 1
            && handler_hir.params[0].ty == HirType::Str
            && handler_hir.ret == HirType::Str;
        if !valid_signature {
            return Err("`handler` must have the signature `(event: string): string`".to_string());
        }
        let handler_fn = self.module.get_function("handler").unwrap();

        let i8_ptr = self.context.ptr_type(AddressSpace::default());
        let run_type = self.context.void_type().fn_type(&[i8_ptr.into()], false);
        let run_fn =
            self.module
                .add_function("thaw_runtime_run", run_type, Some(Linkage::External));

        let (_main_fn, entry) = self.new_c_main();
        self.builder.position_at_end(entry);
        self.call_module_init_if_present();
        let handler_ptr = handler_fn.as_global_value().as_pointer_value();
        self.builder
            .build_call(run_fn, &[handler_ptr.into()], "call_thaw_runtime_run")
            .map_err(|e| e.to_string())?;
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
        self.builder
            .build_return(Some(&i32_type.const_int(0, false)))
            .unwrap();
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
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::process::{Command, Stdio};
    use std::sync::mpsc;
    use std::time::Duration;

    /// Builds the given TS source into a standalone native binary (linking
    /// thaw-arena's staticlib too, since array-using programs call into
    /// it), runs it with the given extra environment variables, and
    /// returns its captured stdout. This is the same pipeline thaw-cli
    /// drives, just inlined for testing.
    fn compile_and_run(source: &str, test_name: &str) -> String {
        compile_and_run_with_env(source, test_name, &[])
    }

    fn compile_and_run_with_env(source: &str, test_name: &str, envs: &[(&str, &str)]) -> String {
        let module = thaw_parser::parse_typescript(source).unwrap();
        let program = thaw_hir::lower_module(&module).unwrap();

        let context = Context::create();
        let mut compiler = HirCompiler::new(&context, test_name);
        compiler.compile_program(&program).unwrap();

        let dir = std::env::temp_dir().join(format!(
            "thaw-hir-codegen-test-{test_name}-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let obj_path = dir.join("out.o");
        let exe_path = dir.join("out");

        compiler.write_object_file(&obj_path).unwrap();

        // Match thaw-cli and always link every runtime archive -- an
        // unreferenced static archive member is
        // simply never pulled in, same reasoning as thaw-cli's build().
        let arena_lib = build_staticlib("thaw-arena");
        let runtime_lib = build_staticlib("thaw-runtime");
        let std_lib = build_staticlib("thaw-std");
        let quickjs_lib = build_staticlib("thaw-quickjs");

        let link_status = Command::new("cc")
            .arg(&obj_path)
            .arg(&arena_lib)
            .arg(&runtime_lib)
            .arg(&std_lib)
            .arg(&quickjs_lib)
            // QuickJS-NG's C code calls libm math functions directly;
            // `rustc` normally adds `-lm` automatically when it does the
            // final link, but this is a manual `cc` invocation instead.
            .arg("-lm")
            .arg("-o")
            .arg(&exe_path)
            .status()
            .expect("failed to invoke system `cc` linker");
        assert!(link_status.success(), "linking failed");

        let output = Command::new(&exe_path)
            .envs(envs.iter().copied())
            .output()
            .expect("failed to execute compiled binary");
        assert!(
            output.status.success(),
            "binary exited {:?}: {}",
            output.status.code(),
            String::from_utf8_lossy(&output.stderr)
        );

        let _ = std::fs::remove_dir_all(&dir);
        String::from_utf8_lossy(&output.stdout).into_owned()
    }

    /// Builds `pkg` as a staticlib (if not already built) and returns the
    /// path to the resulting `.a` file, by parsing `cargo build`'s JSON
    /// artifact output -- robust to `CARGO_TARGET_DIR` overrides, unlike
    /// guessing a relative path.
    fn build_staticlib(pkg: &str) -> std::path::PathBuf {
        let output = Command::new("cargo")
            .args(["build", "--release", "-p", pkg, "--message-format=json"])
            .output()
            .unwrap_or_else(|e| panic!("failed to invoke `cargo build -p {pkg}`: {e}"));
        assert!(output.status.success(), "building {pkg} failed");

        let stdout = String::from_utf8_lossy(&output.stdout);
        let target_name = pkg.replace('-', "_");
        for line in stdout.lines() {
            if !line.contains(&format!("\"name\":\"{target_name}\"")) {
                continue;
            }
            if let Some(idx) = line.find("\"filenames\":[\"") {
                let rest = &line[idx + "\"filenames\":[\"".len()..];
                if let Some(end) = rest.find(".a\"") {
                    let path = &rest[..end + 2];
                    if path.ends_with(".a") {
                        return std::path::PathBuf::from(path);
                    }
                }
            }
        }
        panic!("could not find a staticlib for `{pkg}` in `cargo build` output:\n{stdout}");
    }

    /// Full pipeline smoke test: TS source -> thaw-parser -> thaw-hir ->
    /// this module -> a real object file -> linked into a native binary via
    /// the system `cc` -> executed as a subprocess. This is the literal
    /// Phase 0 "numbers, strings, functions -> LLVM IR, Hello World binary"
    /// deliverable, not a mock of it.
    #[test]
    fn compiles_and_runs_a_native_hello_world_binary() {
        let source = r#"
            function add(a: number, b: number): number {
                return a + b;
            }

            function main(): void {
                console.log("Hello, Thaw!");
                console.log(add(2, 3));
            }
        "#;

        assert_eq!(compile_and_run(source, "hello"), "Hello, Thaw!\n5\n");
    }

    #[test]
    fn compiles_inferred_returns_and_locals_to_native_layouts() {
        let source = r#"
            interface Box<T> {
                value: T;
            }
            interface Wrapper<T> {
                boxed: Box<T>;
            }
            interface Pair<T, U> {
                first: T;
                second: U;
            }

            function first() {
                return second();
            }

            function second() {
                const base = 40;
                return identity(base) + 2;
            }

            function identity<T>(value: T): T {
                return value;
            }

            function sameArray<T>(value: T[]): T[] {
                return value;
            }

            function sameBox<T>(value: Box<T>): Box<T> {
                return value;
            }

            function sameWrapper<T>(value: Wrapper<T>): Wrapper<T> {
                return value;
            }

            function samePair<T, U>(value: Pair<T, U>): Pair<T, U> {
                return value;
            }

            function firstOf<T, U>(value: Pair<T, U>): T {
                return value.first;
            }

            function unbox<T>(value: Wrapper<T>): T {
                return value.boxed.value;
            }

            function main(): void {
                const answer = first();
                console.log(answer);
                console.log(identity("ok"));
                const values = sameArray([7, 8]);
                console.log(values[1]);
                const boxed = sameBox({ value: 9 });
                console.log(boxed.value);
                const wrapped = sameWrapper({ boxed: { value: 10 } });
                console.log(wrapped.boxed.value);
                const pair = samePair({ first: 11, second: 12 });
                console.log(pair.first);
                console.log(pair.second);
                console.log(firstOf({ first: 13, second: 14 }));
                console.log(unbox({ boxed: { value: 15 } }));
            }
        "#;

        assert_eq!(
            compile_and_run(source, "inferred_types"),
            "42\nok\n8\n9\n10\n11\n12\n13\n15\n"
        );
    }

    #[test]
    fn compiles_multi_argument_generic_specializations_end_to_end() {
        let source = r#"
            interface Pair<T, U> {
                first: T;
                second: U;
            }

            function main(): void {
                console.log(chooseFirst(42, "unused"));
                console.log(chooseFirst("chosen", 7));
                console.log(chooseFirst(43, "deduplicated"));
                const pair = makePair("answer", 44);
                console.log(pair.first);
                console.log(pair.second);
                console.log(forward(45, "through generic"));
            }

            function chooseFirst<T, U>(first: T, second: U): T {
                return first;
            }

            function makePair<T, U>(first: T, second: U): Pair<T, U> {
                return { first: first, second: second };
            }

            function forward<T, U>(first: T, second: U): T {
                return chooseFirst(first, second);
            }
        "#;

        assert_eq!(
            compile_and_run(source, "multi_argument_generics"),
            "42\nchosen\n43\nanswer\n44\n45\n"
        );
    }

    #[test]
    fn compiles_if_else_and_recursion() {
        let source = r#"
            function fib(n: number): number {
                if (n < 2) {
                    return n;
                } else {
                    return fib(n - 1) + fib(n - 2);
                }
            }

            function main(): void {
                console.log(fib(10));
            }
        "#;
        assert_eq!(compile_and_run(source, "fib"), "55\n");
    }

    #[test]
    fn compiles_classic_for_loop_over_a_number_array() {
        let source = r#"
            function main(): void {
                const xs: number[] = [10, 20, 30, 40];
                let sum: number = 0;
                for (let i = 0; i < xs.length; i = i + 1) {
                    sum = sum + xs[i];
                }
                console.log(sum);
            }
        "#;
        assert_eq!(compile_and_run(source, "forloop"), "100\n");
    }

    #[test]
    fn compiles_while_loop_with_array_mutation() {
        let source = r#"
            function main(): void {
                const xs: number[] = [0, 0, 0];
                let i: number = 0;
                while (i < 3) {
                    xs[i] = i * i;
                    i = i + 1;
                }
                console.log(xs[0]);
                console.log(xs[1]);
                console.log(xs[2]);
            }
        "#;
        assert_eq!(compile_and_run(source, "whileloop"), "0\n1\n4\n");
    }

    #[test]
    fn compiles_try_catch_within_a_single_function() {
        let source = r#"
            function main(): void {
                try {
                    console.log("before");
                    throw "boom";
                    console.log("unreachable");
                } catch (e) {
                    console.log(e);
                }
                console.log("after");
            }
        "#;
        assert_eq!(compile_and_run(source, "trycatch"), "before\nboom\nafter\n");
    }

    #[test]
    fn propagates_throw_across_function_calls() {
        let source = r#"
            function deepest(): string {
                throw "cross-function boom";
            }

            function middle(): string {
                return deepest();
            }

            function main(): void {
                try {
                    console.log(middle());
                    console.log("unreachable");
                } catch (e) {
                    console.log(e);
                }
                console.log("after");
            }
        "#;
        assert_eq!(
            compile_and_run(source, "cross_function_trycatch"),
            "cross-function boom\nafter\n"
        );
    }

    #[test]
    fn a_catch_can_rethrow_to_an_outer_function() {
        let source = r#"
            function inner(): string {
                try {
                    throw "first";
                } catch (e) {
                    console.log(e);
                    throw "second";
                }
            }

            function main(): void {
                try {
                    console.log(inner());
                } catch (e) {
                    console.log(e);
                }
            }
        "#;
        assert_eq!(
            compile_and_run(source, "cross_function_rethrow"),
            "first\nsecond\n"
        );
    }

    #[test]
    fn finally_runs_on_normal_return_and_caught_throw() {
        let source = r#"
            function returns(): string {
                try {
                    return "value";
                } finally {
                    console.log("finally-return");
                }
            }

            function catches(): string {
                try {
                    throw "boom";
                } catch (e) {
                    console.log(e);
                    return "caught";
                } finally {
                    console.log("finally-catch");
                }
            }

            function main(): void {
                console.log(returns());
                console.log(catches());
            }
        "#;
        assert_eq!(
            compile_and_run(source, "finally_return_and_catch"),
            "finally-return\nvalue\nboom\nfinally-catch\ncaught\n"
        );
    }

    #[test]
    fn try_finally_without_catch_rethrows_after_cleanup() {
        let source = r#"
            function fails(): string {
                try {
                    throw "boom";
                } finally {
                    console.log("cleanup");
                }
            }

            function main(): void {
                try {
                    console.log(fails());
                } catch (e) {
                    console.log(e);
                }
            }
        "#;
        assert_eq!(
            compile_and_run(source, "finally_rethrow"),
            "cleanup\nboom\n"
        );
    }

    #[test]
    fn nested_finally_blocks_run_inside_out() {
        let source = r#"
            function nested(): string {
                try {
                    try {
                        return "done";
                    } finally {
                        console.log("inner");
                    }
                } finally {
                    console.log("outer");
                }
            }

            function main(): void {
                console.log(nested());
            }
        "#;
        assert_eq!(
            compile_and_run(source, "nested_finally"),
            "inner\nouter\ndone\n"
        );
    }

    #[test]
    fn a_throw_from_finally_bypasses_the_same_try_catch() {
        let source = r#"
            function fails(): string {
                try {
                    console.log("try");
                } catch (e) {
                    console.log("wrong-catch");
                } finally {
                    throw "from-finally";
                }
                return "unreachable";
            }

            function main(): void {
                try {
                    console.log(fails());
                } catch (e) {
                    console.log(e);
                }
            }
        "#;
        assert_eq!(
            compile_and_run(source, "finally_throw_bypasses_catch"),
            "try\nfrom-finally\n"
        );
    }

    #[test]
    fn return_from_finally_overrides_an_earlier_return() {
        let source = r#"
            function value(): string {
                try {
                    return "try";
                } finally {
                    return "finally";
                }
            }

            function main(): void {
                console.log(value());
            }
        "#;
        assert_eq!(
            compile_and_run(source, "finally_return_override"),
            "finally\n"
        );
    }

    #[test]
    fn compiles_object_literals_field_access_and_mutation() {
        let source = r#"
            function dist(p: { x: number; y: number }): number {
                return p.x + p.y;
            }

            function main(): void {
                const p: { x: number; y: number } = { y: 2, x: 1 };
                console.log(p.x);
                console.log(dist(p));

                p.x = p.x + 10;
                console.log(p.x);

                console.log(dist({ x: 3, y: 4 }));
            }
        "#;
        assert_eq!(compile_and_run(source, "objects"), "1\n3\n11\n7\n");
    }

    #[test]
    fn compiles_interface_typed_object() {
        let source = r#"
            interface Point {
                x: number;
                y: number;
            }

            function dist(p: Point): number {
                return p.x + p.y;
            }

            function main(): void {
                const p: Point = { y: 2, x: 1 };
                console.log(dist(p));
                p.x = p.x + 10;
                console.log(p.x);
            }
        "#;
        assert_eq!(compile_and_run(source, "interfaces"), "3\n11\n");
    }

    #[test]
    fn compiles_nested_object_fields() {
        let source = r#"
            interface Point {
                x: number;
                y: number;
            }
            interface Line {
                start: Point;
                length: number;
            }

            function main(): void {
                const l: Line = { length: 5, start: { x: 1, y: 2 } };
                console.log(l.start.x);
                console.log(l.start.y);
                console.log(l.length);

                l.start.x = l.start.x + 100;
                console.log(l.start.x);

                l.start = { x: 9, y: 9 };
                console.log(l.start.x);
            }
        "#;
        assert_eq!(
            compile_and_run(source, "nested_objects"),
            "1\n2\n5\n101\n9\n"
        );
    }

    #[test]
    fn compiles_interface_extends() {
        let source = r#"
            interface Shape {
                color: number;
            }
            interface Circle extends Shape {
                radius: number;
            }

            function area(c: Circle): number {
                return c.radius * c.radius;
            }

            function main(): void {
                const c: Circle = { radius: 3, color: 1 };
                console.log(c.color);
                console.log(area(c));
            }
        "#;
        assert_eq!(compile_and_run(source, "interface_extends"), "1\n9\n");
    }

    #[test]
    fn compiles_generic_interface_instantiation() {
        let source = r#"
            interface Box<T> {
                value: T;
            }

            function unwrapNumber(b: Box<number>): number {
                return b.value;
            }

            function main(): void {
                const b: Box<number> = { value: 42 };
                console.log(unwrapNumber(b));

                const s: Box<string> = { value: "hi" };
                console.log(s.value);
            }
        "#;
        assert_eq!(compile_and_run(source, "generic_interfaces"), "42\nhi\n");
    }

    /// The QuickJS-NG fallback path (docs/design/bridge.md section 7): a
    /// compiled Thaw program loads real JS source and calls into it,
    /// round-tripping arguments/results through `Json`.
    #[test]
    fn compiles_quickjs_fallback_path() {
        let source = r#"
            function main(): void {
                const ok: boolean = loadScript(
                    "function add(a, b) { return a + b; } function greet(name) { return 'hello, ' + name; } function later(x) { return Promise.resolve(x * 2); }"
                );
                console.log(ok);

                const sum = callDynamic("add", JSON.parse("[2, 3]"));
                console.log(Number(sum));

                const greeting = callDynamic("greet", JSON.parse("[\"thaw\"]"));
                console.log(String(greeting));

                const doubled = callDynamic("later", JSON.parse("[21]"));
                console.log(Number(doubled));
            }
        "#;
        assert_eq!(
            compile_and_run(source, "quickjs_fallback"),
            "true\n5\nhello, thaw\n42\n"
        );
    }

    /// Module auto-initialization: a registry-generated `__thaw_module_init`
    /// (thaw-bridge's `generate_module_init`, wired in via thaw-cli's
    /// `--use`) must run before `main`'s body, with no `loadScript` call
    /// written by the user -- that's the whole point of automating it.
    #[test]
    fn runs_module_init_before_main_body() {
        let source = r#"
            function __thaw_module_init(): void {
                loadScript("function greet() { return 'hi from registry'; }");
            }

            function main(): void {
                const result = callDynamic("greet", JSON.parse("[]"));
                console.log(String(result));
            }
        "#;
        assert_eq!(
            compile_and_run(source, "module_init_main"),
            "hi from registry\n"
        );
    }

    /// Same mechanism, but through the Lambda `handler` entry point instead
    /// of `main` (`emit_lambda_entry` has its own copy of the
    /// `call_module_init_if_present` call, see hir_codegen.rs).
    #[test]
    fn runs_module_init_before_lambda_handler() {
        let source = r#"
            function __thaw_module_init(): void {
                loadScript("function greet() { return 'hi from registry'; }");
            }

            function handler(event: string): string {
                const result = callDynamic("greet", JSON.parse("[]"));
                return String(result);
            }
        "#;
        let module = thaw_parser::parse_typescript(source).unwrap();
        let program = thaw_hir::lower_module(&module).unwrap();

        let context = Context::create();
        let mut compiler = HirCompiler::new(&context, "module_init_lambda");
        compiler.compile_program(&program).unwrap();

        let dir = std::env::temp_dir().join(format!(
            "thaw-hir-codegen-test-module_init_lambda-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let obj_path = dir.join("out.o");
        let exe_path = dir.join("out");
        compiler.write_object_file(&obj_path).unwrap();

        let arena_lib = build_staticlib("thaw-arena");
        let runtime_lib = build_staticlib("thaw-runtime");
        let std_lib = build_staticlib("thaw-std");
        let quickjs_lib = build_staticlib("thaw-quickjs");

        let link_status = Command::new("cc")
            .arg(&obj_path)
            .arg(&arena_lib)
            .arg(&runtime_lib)
            .arg(&std_lib)
            .arg(&quickjs_lib)
            .arg("-lm")
            .arg("-o")
            .arg(&exe_path)
            .status()
            .expect("failed to invoke system `cc` linker");
        assert!(link_status.success(), "linking failed");

        // Same mock Lambda Runtime API protocol as
        // `compiles_and_runs_a_lambda_handler_against_a_mock_runtime_api`:
        // bind a real TCP listener, tell the binary about it via
        // `AWS_LAMBDA_RUNTIME_API`, and check the response it posts back.
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap().to_string();

        let (tx, rx) = mpsc::channel();
        let server = std::thread::spawn(move || {
            let (mut conn, _) = listener.accept().unwrap();
            let mut buf = [0u8; 4096];
            let _ = conn.read(&mut buf).unwrap();
            let body = "\"ping\"";
            let response = format!(
                "HTTP/1.1 200 OK\r\nLambda-Runtime-Aws-Request-Id: test-req-1\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            conn.write_all(response.as_bytes()).unwrap();
            drop(conn);

            let (mut conn, _) = listener.accept().unwrap();
            let mut buf = Vec::new();
            conn.read_to_end(&mut buf).unwrap();
            tx.send(String::from_utf8_lossy(&buf).into_owned()).unwrap();

            let response =
                "HTTP/1.1 202 Accepted\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
            conn.write_all(response.as_bytes()).unwrap();
        });

        let mut child = Command::new(&exe_path)
            .env("AWS_LAMBDA_RUNTIME_API", &addr)
            .stdout(Stdio::piped())
            .spawn()
            .expect("failed to spawn compiled Lambda handler binary");

        let post_request = rx
            .recv_timeout(Duration::from_secs(10))
            .expect("handler never posted a response to the mock runtime API");
        server.join().unwrap();

        let _ = child.kill();
        let _ = child.wait_with_output();
        let _ = std::fs::remove_dir_all(&dir);

        assert!(post_request.starts_with("POST /2018-06-01/runtime/invocation/test-req-1/response"));
        assert!(
            post_request.ends_with("hi from registry"),
            "response body should be the module-init-loaded function's result, got: {post_request}"
        );
    }

    /// V1 async/await (docs/design/async-await.md): `async`/`await` are
    /// pure sugar over synchronous calls, including `main` itself being
    /// `async` -- the entry-point detection in `compile_program` doesn't
    /// care about `is_async` at all, since by the time codegen sees it the
    /// function's `ret` is already the unwrapped, non-Promise type.
    #[test]
    fn compiles_async_functions_as_synchronous_calls() {
        let source = r#"
            async function computeStage(): Promise<string> {
                const s: string = process.env.STAGE;
                return s;
            }

            async function addAsync(a: number, b: number): Promise<number> {
                return a + b;
            }

            async function main(): Promise<void> {
                const stage: string = await computeStage();
                console.log(stage);
                const sum: number = await addAsync(2, 3);
                console.log(sum);
            }
        "#;
        assert_eq!(
            compile_and_run_with_env(source, "async_v1", &[("STAGE", "prod")]),
            "prod\n5\n"
        );
    }

    #[test]
    fn await_sleep_is_driven_by_the_runtime_event_loop() {
        let source = r#"
            async function main(): Promise<void> {
                console.log("before");
                await sleep(1);
                console.log("after");
            }
        "#;
        assert_eq!(compile_and_run(source, "await_sleep"), "before\nafter\n");
    }

    #[test]
    fn frame_split_async_main_has_resume_function_and_multiple_states() {
        let source = r#"
            async function main(): Promise<void> {
                console.log("state-0");
                await sleep(1);
                console.log("state-1");
                await sleep(1);
                console.log("state-2");
            }
        "#;

        let module = thaw_parser::parse_typescript(source).unwrap();
        let program = thaw_hir::lower_module(&module).unwrap();
        let context = Context::create();
        let mut compiler = HirCompiler::new(&context, "frame_split_ir");
        compiler.compile_program(&program).unwrap();
        let ir = compiler.print_to_string();
        assert!(ir.contains("define ptr @thaw_user_main()"));
        assert!(ir.contains("define internal void @thaw_user_main.resume"));
        assert!(ir.contains("state_1"));
        assert!(ir.contains("state_2"));

        assert_eq!(
            compile_and_run(source, "frame_split_multiple_awaits"),
            "state-0\nstate-1\nstate-2\n"
        );
    }

    #[test]
    fn frame_split_preserves_and_mutates_locals_across_awaits() {
        let source = r#"
            async function main(): Promise<void> {
                let value: number = 40;
                await sleep(1);
                value = value + 2;
                await sleep(1);
                console.log(value);
            }
        "#;
        let module = thaw_parser::parse_typescript(source).unwrap();
        let program = thaw_hir::lower_module(&module).unwrap();
        let context = Context::create();
        let mut compiler = HirCompiler::new(&context, "frame_local_slots");
        compiler.compile_program(&program).unwrap();
        let ir = compiler.print_to_string();
        assert!(ir.contains("frame_value"));
        assert_eq!(compile_and_run(source, "frame_local_slots"), "42\n");
    }

    #[test]
    fn frame_split_void_return_resolves_completion_and_stops() {
        let source = r#"
            async function main(): Promise<void> {
                console.log("before");
                await sleep(1);
                console.log("resumed");
                return;
                console.log("unreachable");
            }
        "#;
        assert_eq!(
            compile_and_run(source, "frame_void_return"),
            "before\nresumed\n"
        );
    }

    #[test]
    fn frame_split_value_return_is_stored_in_completion_result_slot() {
        let source = r#"
            async function main(): Promise<number> {
                console.log("before");
                await sleep(1);
                return 42;
                console.log("unreachable");
            }
        "#;
        let module = thaw_parser::parse_typescript(source).unwrap();
        let program = thaw_hir::lower_module(&module).unwrap();
        let context = Context::create();
        let mut compiler = HirCompiler::new(&context, "frame_value_return");
        compiler.compile_program(&program).unwrap();
        let ir = compiler.print_to_string();
        assert!(ir.contains("async_result"));
        assert!(ir.contains("store double 4.200000e+01"));
        assert_eq!(compile_and_run(source, "frame_value_return"), "before\n");
    }

    #[test]
    fn frame_splits_non_main_async_function() {
        let source = r#"
            async function compute(): Promise<number> {
                await sleep(1);
                return 42;
            }

            function main(): void {
                console.log("main");
            }
        "#;
        let module = thaw_parser::parse_typescript(source).unwrap();
        let program = thaw_hir::lower_module(&module).unwrap();
        let context = Context::create();
        let mut compiler = HirCompiler::new(&context, "general_async_frame");
        compiler.compile_program(&program).unwrap();
        let ir = compiler.print_to_string();
        assert!(ir.contains("define ptr @compute()"));
        assert!(ir.contains("define internal void @compute.resume"));
        assert_eq!(compile_and_run(source, "general_async_frame"), "main\n");
    }

    #[test]
    fn frame_split_awaits_user_async_function_and_reads_result() {
        let source = r#"
            async function compute(): Promise<number> {
                await sleep(1);
                return 42;
            }

            async function main(): Promise<void> {
                const value: number = await compute();
                console.log(value);
            }
        "#;
        let module = thaw_parser::parse_typescript(source).unwrap();
        let program = thaw_hir::lower_module(&module).unwrap();
        let context = Context::create();
        let mut compiler = HirCompiler::new(&context, "await_async_result");
        compiler.compile_program(&program).unwrap();
        let ir = compiler.print_to_string();
        assert!(ir.contains("call ptr @compute()"));
        assert!(ir.contains("awaited_value"));
        assert_eq!(compile_and_run(source, "await_async_result"), "42\n");
    }

    #[test]
    fn frame_split_async_function_preserves_arguments_across_await() {
        let source = r#"
            async function compute(a: number, b: number): Promise<number> {
                await sleep(1);
                return a + b;
            }

            async function main(): Promise<void> {
                const value: number = await compute(20, 22);
                console.log(value);
            }
        "#;
        let module = thaw_parser::parse_typescript(source).unwrap();
        let program = thaw_hir::lower_module(&module).unwrap();
        let context = Context::create();
        let mut compiler = HirCompiler::new(&context, "async_arguments");
        compiler.compile_program(&program).unwrap();
        let ir = compiler.print_to_string();
        assert!(ir.contains("define ptr @compute(double"));
        assert!(ir.contains("frame_a"));
        assert!(ir.contains("frame_b"));
        assert_eq!(compile_and_run(source, "async_arguments"), "42\n");
    }

    #[test]
    fn frame_split_supports_await_inside_expressions() {
        let source = r#"
            async function compute(value: number): Promise<number> {
                await sleep(1);
                return value;
            }

            async function main(): Promise<void> {
                const value: number = (await compute(20)) + (await compute(21)) + 1;
                console.log(value);
                console.log((await compute(6)) * 7);
            }
        "#;
        let module = thaw_parser::parse_typescript(source).unwrap();
        let program = thaw_hir::lower_module(&module).unwrap();
        let context = Context::create();
        let mut compiler = HirCompiler::new(&context, "await_in_expression");
        compiler.compile_program(&program).unwrap();
        let ir = compiler.print_to_string();
        assert!(ir.contains("frame___thaw_await_0"));
        assert!(ir.contains("frame___thaw_await_1"));
        assert_eq!(compile_and_run(source, "await_in_expression"), "42\n42\n");
    }

    #[test]
    fn frame_split_supports_await_in_if_condition() {
        let source = r#"
            async function isReady(value: number): Promise<boolean> {
                await sleep(1);
                return value > 0;
            }

            async function main(): Promise<void> {
                if (await isReady(1)) {
                    console.log("ready");
                } else {
                    console.log("not-ready");
                }
                if (await isReady(0)) {
                    console.log("unexpected");
                } else {
                    console.log("false-branch");
                }
            }
        "#;
        assert_eq!(
            compile_and_run(source, "await_if_condition"),
            "ready\nfalse-branch\n"
        );
    }

    #[test]
    fn frame_split_supports_await_inside_if_branches() {
        let source = r#"
            async function compute(value: number): Promise<number> {
                await sleep(1);
                return value;
            }

            async function main(): Promise<void> {
                let value: number = 0;
                if (true) {
                    value = await compute(20);
                } else {
                    console.log("wrong-else");
                    value = await compute(100);
                }
                if (false) {
                    console.log("wrong-then");
                    await sleep(1);
                } else {
                    value = value + (await compute(22));
                }
                console.log(value);
            }
        "#;
        assert_eq!(compile_and_run(source, "await_if_branches"), "42\n");
    }

    #[test]
    fn frame_split_supports_await_inside_while_loop() {
        let source = r#"
            async function compute(value: number): Promise<number> {
                await sleep(1);
                return value;
            }

            async function main(): Promise<void> {
                let index: number = 0;
                let total: number = 0;
                while (index < 3) {
                    await sleep(1);
                    total = total + (await compute(index));
                    index = index + 1;
                }
                console.log(total);
                console.log(index);
            }
        "#;
        assert_eq!(compile_and_run(source, "await_while_loop"), "3\n3\n");
    }

    #[test]
    fn frame_split_supports_await_in_while_condition() {
        let source = r#"
            async function shouldContinue(value: number): Promise<boolean> {
                await sleep(1);
                return value < 3;
            }

            async function main(): Promise<void> {
                let index: number = 0;
                while (await shouldContinue(index)) {
                    console.log(index);
                    index = index + 1;
                }
                console.log("done");
            }
        "#;
        assert_eq!(
            compile_and_run(source, "await_while_condition"),
            "0\n1\n2\ndone\n"
        );
    }

    #[test]
    fn compiles_synchronous_break_and_continue() {
        let source = r#"
            function main(): void {
                let index: number = 0;
                let total: number = 0;
                while (index < 5) {
                    index = index + 1;
                    if (index < 3) {
                        continue;
                    }
                    total = total + index;
                    if (index > 3) {
                        break;
                    }
                }
                console.log(total);
            }
        "#;
        assert_eq!(compile_and_run(source, "sync_break_continue"), "7\n");
    }

    #[test]
    fn frame_split_supports_break_and_continue_in_while_loop() {
        let source = r#"
            async function main(): Promise<void> {
                let continued: number = 0;
                while (continued < 3) {
                    continued = continued + 1;
                    await sleep(1);
                    continue;
                    continued = 100;
                }
                console.log(continued);

                let broken: number = 0;
                while (true) {
                    await sleep(1);
                    broken = broken + 1;
                    break;
                    broken = 100;
                }
                console.log(broken);
            }
        "#;
        assert_eq!(compile_and_run(source, "async_break_continue"), "3\n1\n");
    }

    #[test]
    fn frame_split_supports_deeply_nested_if_awaits() {
        let source = r#"
            async function compute(value: number): Promise<number> {
                await sleep(1);
                return value;
            }

            async function main(): Promise<void> {
                let value: number = 0;
                if (true) {
                    if (false) {
                        console.log("wrong-inner-then");
                        value = 100;
                    } else {
                        if (true) {
                            value = await compute(42);
                        }
                    }
                } else {
                    console.log("wrong-outer-else");
                }
                console.log(value);
            }
        "#;
        assert_eq!(compile_and_run(source, "nested_async_if"), "42\n");
    }

    #[test]
    fn frame_split_supports_nested_break_and_continue() {
        let source = r#"
            async function main(): Promise<void> {
                let index: number = 0;
                let total: number = 0;
                while (index < 5) {
                    index = index + 1;
                    await sleep(1);
                    if (index < 3) {
                        continue;
                    }
                    if (index > 3) {
                        break;
                    }
                    total = total + index;
                }
                console.log(total);
                console.log(index);
            }
        "#;
        assert_eq!(
            compile_and_run(source, "nested_async_loop_control"),
            "3\n4\n"
        );
    }

    #[test]
    fn frame_split_supports_await_in_condition_and_branch_body() {
        let source = r#"
            async function ready(value: boolean): Promise<boolean> {
                await sleep(1);
                return value;
            }

            async function compute(value: number): Promise<number> {
                await sleep(1);
                return value;
            }

            async function main(): Promise<void> {
                let value: number = 0;
                if (await ready(true)) {
                    value = await compute(42);
                } else {
                    value = await compute(100);
                }
                console.log(value);
            }
        "#;
        assert_eq!(compile_and_run(source, "async_if_condition_body"), "42\n");
    }

    #[test]
    fn frame_split_supports_await_in_deeply_nested_if_condition() {
        let source = r#"
            async function ready(value: boolean): Promise<boolean> {
                await sleep(1);
                return value;
            }

            async function compute(value: number): Promise<number> {
                await sleep(1);
                return value;
            }

            async function main(): Promise<void> {
                let value: number = 0;
                if (true) {
                    if (await ready(false)) {
                        value = await compute(100);
                    } else {
                        if (await ready(true)) {
                            value = await compute(42);
                        }
                    }
                }
                console.log(value);
            }
        "#;
        assert_eq!(compile_and_run(source, "nested_async_if_condition"), "42\n");
    }

    #[test]
    fn frame_split_supports_nested_async_while_loops() {
        let source = r#"
            async function below(value: number, limit: number): Promise<boolean> {
                await sleep(1);
                return value < limit;
            }

            async function main(): Promise<void> {
                let outer: number = 0;
                let inner: number = 0;
                let total: number = 0;
                while (outer < 2) {
                    await sleep(1);
                    inner = 0;
                    while (await below(inner, 3)) {
                        total = total + 1;
                        inner = inner + 1;
                    }
                    outer = outer + 1;
                }
                console.log(total);
                console.log(outer);
            }
        "#;
        assert_eq!(compile_and_run(source, "nested_async_while"), "6\n2\n");
    }

    #[test]
    fn frame_split_supports_async_try_finally_normal_completion() {
        let source = r#"
            async function main(): Promise<void> {
                try {
                    console.log("try");
                    await sleep(1);
                    console.log("resumed");
                } finally {
                    console.log("finally");
                }
                console.log("done");
            }
        "#;
        assert_eq!(
            compile_and_run(source, "async_try_finally_normal"),
            "try\nresumed\nfinally\ndone\n"
        );
    }

    #[test]
    fn frame_split_runs_finally_before_async_return() {
        let source = r#"
            async function main(): Promise<void> {
                try {
                    await sleep(1);
                    console.log("returning");
                    return;
                } finally {
                    console.log("finally");
                }
                console.log("unreachable");
            }
        "#;
        assert_eq!(
            compile_and_run(source, "async_try_finally_return"),
            "returning\nfinally\n"
        );
    }

    #[test]
    fn frame_split_propagates_async_rejection_through_await_chain() {
        let source = r#"
            async function fail(): Promise<number> {
                await sleep(1);
                throw "boom";
            }

            async function relay(): Promise<number> {
                const value: number = await fail();
                return value;
            }

            async function main(): Promise<void> {
                console.log("before");
                await relay();
                console.log("unreachable");
            }
        "#;
        let module = thaw_parser::parse_typescript(source).unwrap();
        let program = thaw_hir::lower_module(&module).unwrap();
        let context = Context::create();
        let mut compiler = HirCompiler::new(&context, "async_rejection_ir");
        compiler.compile_program(&program).unwrap();
        let ir = compiler.print_to_string();
        assert!(ir.contains("call i8 @thaw_promise_reject"));
        assert!(ir.contains("propagate_rejection"));
        assert_eq!(compile_and_run(source, "async_rejection_chain"), "before\n");
    }

    #[test]
    fn frame_split_catches_throw_after_await_in_same_function() {
        let source = r#"
            async function main(): Promise<void> {
                try {
                    console.log("try");
                    await sleep(1);
                    throw "boom";
                    console.log("unreachable");
                } catch (error) {
                    console.log(error);
                    await sleep(1);
                    console.log("caught");
                }
                console.log("done");
            }
        "#;
        assert_eq!(
            compile_and_run(source, "async_direct_catch"),
            "try\nboom\ncaught\ndone\n"
        );
    }

    #[test]
    fn frame_split_catches_rejected_child_promise() {
        let source = r#"
            async function fail(): Promise<void> {
                await sleep(1);
                throw "child-boom";
            }

            async function main(): Promise<void> {
                try {
                    console.log("before");
                    await fail();
                    console.log("unreachable");
                } catch (error) {
                    console.log(error);
                    await sleep(1);
                    console.log("caught");
                }
                console.log("done");
            }
        "#;
        assert_eq!(
            compile_and_run(source, "async_rejection_catch"),
            "before\nchild-boom\ncaught\ndone\n"
        );
    }

    #[test]
    fn frame_split_runs_finally_then_rethrows_child_rejection() {
        let source = r#"
            async function fail(): Promise<void> {
                await sleep(1);
                throw "boom";
            }

            async function main(): Promise<void> {
                console.log("before");
                try {
                    await fail();
                    console.log("unreachable");
                } finally {
                    console.log("finally");
                }
                console.log("also-unreachable");
            }
        "#;
        assert_eq!(
            compile_and_run(source, "async_rejection_finally"),
            "before\nfinally\n"
        );
    }

    #[test]
    fn frame_split_runs_catch_then_finally_for_child_rejection() {
        let source = r#"
            async function fail(): Promise<void> {
                await sleep(1);
                throw "boom";
            }

            async function main(): Promise<void> {
                try {
                    await fail();
                } catch (error) {
                    console.log(error);
                    await sleep(1);
                    console.log("caught");
                } finally {
                    console.log("finally");
                }
                console.log("done");
            }
        "#;
        assert_eq!(
            compile_and_run(source, "async_catch_finally_rejection"),
            "boom\ncaught\nfinally\ndone\n"
        );
    }

    #[test]
    fn frame_split_rethrows_after_await_in_catch() {
        let source = r#"
            async function fail(): Promise<void> {
                await sleep(1);
                throw "original";
            }

            async function relay(): Promise<void> {
                try {
                    await fail();
                } catch (error) {
                    console.log(error);
                    await sleep(1);
                    console.log("rethrowing");
                    throw "wrapped";
                }
                console.log("relay-unreachable");
            }

            async function main(): Promise<void> {
                console.log("before");
                await relay();
                console.log("main-unreachable");
            }
        "#;
        assert_eq!(
            compile_and_run(source, "async_catch_rethrow"),
            "before\noriginal\nrethrowing\n"
        );
    }

    #[test]
    fn frame_split_runs_finally_before_catch_rethrow() {
        let source = r#"
            async function fail(): Promise<void> {
                await sleep(1);
                throw "original";
            }

            async function relay(): Promise<void> {
                try {
                    await fail();
                } catch (error) {
                    console.log(error);
                    await sleep(1);
                    throw "wrapped";
                } finally {
                    console.log("finally");
                }
            }

            async function main(): Promise<void> {
                await relay();
                console.log("unreachable");
            }
        "#;
        assert_eq!(
            compile_and_run(source, "async_catch_rethrow_finally"),
            "original\nfinally\n"
        );
    }

    #[test]
    fn frame_split_selects_nearest_catch_in_nested_async_try() {
        let source = r#"
            async function failInner(): Promise<void> {
                await sleep(1);
                throw "inner-error";
            }

            async function failOuter(): Promise<void> {
                await sleep(1);
                throw "outer-error";
            }

            async function main(): Promise<void> {
                try {
                    try {
                        await failInner();
                    } catch (inner) {
                        console.log(inner);
                        await failOuter();
                        console.log("inner-unreachable");
                    }
                    console.log("outer-try-unreachable");
                } catch (outer) {
                    console.log(outer);
                    await sleep(1);
                    console.log("outer-caught");
                }
                console.log("done");
            }
        "#;
        assert_eq!(
            compile_and_run(source, "nested_async_try_catch"),
            "inner-error\nouter-error\nouter-caught\ndone\n"
        );
    }

    #[test]
    fn frame_split_routes_inner_catch_rethrow_to_outer_catch() {
        let source = r#"
            async function main(): Promise<void> {
                try {
                    try {
                        await sleep(1);
                        throw "inner";
                    } catch (inner) {
                        console.log(inner);
                        await sleep(1);
                        throw "wrapped";
                        console.log("inner-unreachable");
                    }
                    console.log("outer-try-unreachable");
                } catch (outer) {
                    console.log(outer);
                    await sleep(1);
                    console.log("outer-caught");
                }
                console.log("done");
            }
        "#;
        assert_eq!(
            compile_and_run(source, "nested_async_catch_rethrow"),
            "inner\nwrapped\nouter-caught\ndone\n"
        );
    }

    #[test]
    fn frame_split_supports_arbitrarily_deep_async_try_catch() {
        let source = r#"
            async function failDeep(): Promise<void> {
                await sleep(1);
                throw "deep";
            }

            async function main(): Promise<void> {
                try {
                    try {
                        try {
                            await failDeep();
                        } catch (deep) {
                            console.log(deep);
                            await sleep(1);
                            throw "middle";
                        }
                    } catch (middle) {
                        console.log(middle);
                        await sleep(1);
                        throw "outer";
                    }
                } catch (outer) {
                    console.log(outer);
                    await sleep(1);
                    console.log("caught");
                }
                console.log("done");
            }
        "#;
        assert_eq!(
            compile_and_run(source, "deep_async_try_catch"),
            "deep\nmiddle\nouter\ncaught\ndone\n"
        );
    }

    #[test]
    fn frame_split_supports_locals_and_return_inside_async_branch() {
        let source = r#"
            async function value(): Promise<number> {
                await sleep(1);
                return 42;
            }

            async function choose(): Promise<number> {
                if (1 < 2) {
                    const answer = await value();
                    return answer;
                }
                return 0;
            }

            async function main(): Promise<void> {
                const result = await choose();
                console.log(result);
            }
        "#;
        assert_eq!(compile_and_run(source, "async_branch_local_return"), "42\n");
    }

    #[test]
    fn frame_split_supports_try_inside_async_if_and_while() {
        let source = r#"
            async function main(): Promise<void> {
                let i = 0;
                if (1 < 2) {
                    try {
                        const label = "if-error";
                        await sleep(1);
                        throw label;
                    } catch (error) {
                        console.log(error);
                    }
                }
                while (i < 1) {
                    try {
                        const label = "while-error";
                        await sleep(1);
                        throw label;
                    } catch (error) {
                        console.log(error);
                    }
                    i = i + 1;
                }
                console.log("done");
            }
        "#;
        assert_eq!(
            compile_and_run(source, "async_try_in_if_while"),
            "if-error\nwhile-error\ndone\n"
        );
    }

    #[test]
    fn frame_split_supports_async_if_and_while_inside_try() {
        let source = r#"
            async function main(): Promise<void> {
                let i = 0;
                try {
                    if (1 < 2) {
                        const prefix = "branch";
                        await sleep(1);
                        console.log(prefix);
                    }
                    while (i < 2) {
                        const current = i;
                        await sleep(1);
                        i = current + 1;
                        if (i > 1) {
                            throw "loop-error";
                        }
                    }
                    console.log("unreachable");
                } catch (error) {
                    console.log(error);
                    await sleep(1);
                    console.log("caught");
                }
                console.log("done");
            }
        "#;
        assert_eq!(
            compile_and_run(source, "async_if_while_in_try"),
            "branch\nloop-error\ncaught\ndone\n"
        );
    }

    #[test]
    fn frame_split_await_fetch_uses_nonblocking_http_promise() {
        let source = r#"
            async function main(): Promise<void> {
                const body = await fetch("http://127.0.0.1:8080/data");
                console.log(body);
            }
        "#;
        let module = thaw_parser::parse_typescript(source).unwrap();
        let program = thaw_hir::lower_module(&module).unwrap();
        let context = Context::create();
        let mut compiler = HirCompiler::new(&context, "async_fetch");
        compiler.compile_program(&program).unwrap();
        let ir = compiler.print_to_string();
        assert!(ir.contains("call ptr @thaw_http_get_async"));
        assert!(ir.contains("define internal void @thaw_user_main.resume"));
        assert!(!ir.contains("call ptr @thaw_fetch_get"));
    }

    /// Full pipeline: `fetch` a JSON body from a real (mock) HTTP server,
    /// `JSON.parse` it, read fields (both `.field` and `[i]`), and convert
    /// them to concrete types with `Number`/`String`/`Boolean`.
    #[test]
    fn compiles_fetch_and_json_parsing() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();

        let server = std::thread::spawn(move || {
            let (mut conn, _) = listener.accept().unwrap();
            let mut buf = [0u8; 4096];
            let _ = conn.read(&mut buf).unwrap();
            let body = r#"{"name": "thaw", "active": true, "tags": ["fast", "native"]}"#;
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            conn.write_all(response.as_bytes()).unwrap();
        });

        let source = format!(
            r#"
            async function main(): Promise<void> {{
                const text: string = await fetch("http://{addr}/");
                const data = JSON.parse(text);
                console.log(String(data.name));
                console.log(Boolean(data.active));
                console.log(Number(data.tags.length));
                console.log(String(data.tags[0]));
                console.log(JSON.stringify(data));
            }}
        "#
        );

        let output = compile_and_run(&source, "fetch_json");
        server.join().unwrap();

        let mut lines = output.lines();
        assert_eq!(lines.next(), Some("thaw"));
        assert_eq!(lines.next(), Some("true"));
        assert_eq!(lines.next(), Some("2"));
        assert_eq!(lines.next(), Some("fast"));
        let reparsed: serde_json::Value = serde_json::from_str(lines.next().unwrap()).unwrap();
        assert_eq!(
            reparsed,
            serde_json::json!({"name": "thaw", "active": true, "tags": ["fast", "native"]})
        );
    }

    /// docs/design/bridge.md's first vertical slice: an ambient `declare
    /// function` call actually links against and calls a real native
    /// implementation -- here a tiny hand-written C function, standing in
    /// for what would eventually be a thaw-registry-fetched library.
    #[test]
    fn compiles_ambient_declaration_and_links_a_real_native_function() {
        let source = r#"
            declare function native_add(a: number, b: number): number;

            function main(): void {
                console.log(native_add(2, 3));
            }
        "#;

        let module = thaw_parser::parse_typescript(source).unwrap();
        let program = thaw_hir::lower_module(&module).unwrap();
        assert_eq!(
            program.extern_functions.len(),
            1,
            "sanity: this is really an FFI call"
        );

        let context = Context::create();
        let mut compiler = HirCompiler::new(&context, "ffi_ambient");
        compiler.compile_program(&program).unwrap();

        let dir = std::env::temp_dir().join(format!(
            "thaw-hir-codegen-test-ffi_ambient-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let obj_path = dir.join("out.o");
        let exe_path = dir.join("out");
        let native_c_path = dir.join("native.c");
        let native_obj_path = dir.join("native.o");

        compiler.write_object_file(&obj_path).unwrap();

        std::fs::write(
            &native_c_path,
            "double native_add(double a, double b) { return a + b; }\n",
        )
        .unwrap();
        let cc_status = Command::new("cc")
            .arg("-c")
            .arg(&native_c_path)
            .arg("-o")
            .arg(&native_obj_path)
            .status()
            .expect("failed to invoke `cc` to build the native stand-in library");
        assert!(cc_status.success(), "compiling native.c failed");

        let arena_lib = build_staticlib("thaw-arena");
        let link_status = Command::new("cc")
            .arg(&obj_path)
            .arg(&native_obj_path)
            .arg(&arena_lib)
            .arg("-o")
            .arg(&exe_path)
            .status()
            .expect("failed to invoke system `cc` linker");
        assert!(link_status.success(), "linking failed");

        let output = Command::new(&exe_path)
            .output()
            .expect("failed to execute compiled binary");
        assert!(output.status.success(), "binary exited non-zero");
        assert_eq!(String::from_utf8_lossy(&output.stdout), "5\n");

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// docs/design/bridge.md section 5's marshal-adapter gap, closed:
    /// unlike the plain-`number` case above, a real C function taking an
    /// array almost never expects Thaw's own internal `[i64 len][f64
    /// elements...]` buffer -- it expects the near-universal `(const
    /// double*, int64_t len)` two-argument convention instead, exactly
    /// what this real (not Thaw-authored) C function below declares. If
    /// `ffi_param_types`/`compile_ffi_call` still passed Thaw's raw buffer
    /// pointer as a single argument, this would either fail to link
    /// (arity mismatch caught by `cc`) or silently misread memory as a
    /// `(double*, int64_t)` pair -- it does neither: the sum comes back
    /// correct.
    #[test]
    fn ffi_call_marshals_a_number_array_into_pointer_plus_length() {
        let source = r#"
            declare function native_sum(xs: number[]): number;

            function main(): void {
                const xs: number[] = [1, 2, 3, 4];
                console.log(native_sum(xs));
            }
        "#;

        let module = thaw_parser::parse_typescript(source).unwrap();
        let program = thaw_hir::lower_module(&module).unwrap();
        assert_eq!(
            program.extern_functions.len(),
            1,
            "sanity: this is really an FFI call"
        );

        let context = Context::create();
        let mut compiler = HirCompiler::new(&context, "ffi_array_marshal");
        compiler.compile_program(&program).unwrap();

        let dir = std::env::temp_dir().join(format!(
            "thaw-hir-codegen-test-ffi_array_marshal-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let obj_path = dir.join("out.o");
        let exe_path = dir.join("out");
        let native_c_path = dir.join("native.c");
        let native_obj_path = dir.join("native.o");

        compiler.write_object_file(&obj_path).unwrap();

        // A real, independently-written C function -- not one shaped
        // around Thaw's own array layout. `len` is genuinely used (not
        // just accepted and ignored), so a wrong length would also
        // produce a wrong sum, not just "happen to work".
        std::fs::write(
            &native_c_path,
            "#include <stdint.h>\n\
             double native_sum(const double* xs, int64_t len) {\n\
             \x20\x20double total = 0;\n\
             \x20\x20for (int64_t i = 0; i < len; i++) { total += xs[i]; }\n\
             \x20\x20return total;\n\
             }\n",
        )
        .unwrap();
        let cc_status = Command::new("cc")
            .arg("-c")
            .arg(&native_c_path)
            .arg("-o")
            .arg(&native_obj_path)
            .status()
            .expect("failed to invoke `cc` to build the native stand-in library");
        assert!(cc_status.success(), "compiling native.c failed");

        let arena_lib = build_staticlib("thaw-arena");
        let link_status = Command::new("cc")
            .arg(&obj_path)
            .arg(&native_obj_path)
            .arg(&arena_lib)
            .arg("-o")
            .arg(&exe_path)
            .status()
            .expect("failed to invoke system `cc` linker");
        assert!(link_status.success(), "linking failed");

        let output = Command::new(&exe_path)
            .output()
            .expect("failed to execute compiled binary");
        assert!(output.status.success(), "binary exited non-zero");
        assert_eq!(String::from_utf8_lossy(&output.stdout), "10\n");

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Same gap, for `Object` params: a real C function is far more
    /// likely to be a flat multi-argument function than to agree with
    /// Thaw's own arena struct layout, so an object-typed FFI parameter
    /// is flattened into one scalar C argument per field, in declared
    /// order. `x*10 + y` (rather than something field-order-symmetric
    /// like `x + y`) specifically catches a field-order bug: if `x`/`y`
    /// were swapped, `{x: 3, y: 4}` would produce `43` instead of `34`.
    #[test]
    fn ffi_call_marshals_an_object_into_one_scalar_argument_per_field() {
        let source = r#"
            declare function native_combine(p: { x: number; y: number }): number;

            function main(): void {
                console.log(native_combine({ x: 3, y: 4 }));
            }
        "#;

        let module = thaw_parser::parse_typescript(source).unwrap();
        let program = thaw_hir::lower_module(&module).unwrap();
        assert_eq!(
            program.extern_functions.len(),
            1,
            "sanity: this is really an FFI call"
        );

        let context = Context::create();
        let mut compiler = HirCompiler::new(&context, "ffi_object_marshal");
        compiler.compile_program(&program).unwrap();

        let dir = std::env::temp_dir().join(format!(
            "thaw-hir-codegen-test-ffi_object_marshal-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let obj_path = dir.join("out.o");
        let exe_path = dir.join("out");
        let native_c_path = dir.join("native.c");
        let native_obj_path = dir.join("native.o");

        compiler.write_object_file(&obj_path).unwrap();

        std::fs::write(
            &native_c_path,
            "double native_combine(double x, double y) { return x * 10 + y; }\n",
        )
        .unwrap();
        let cc_status = Command::new("cc")
            .arg("-c")
            .arg(&native_c_path)
            .arg("-o")
            .arg(&native_obj_path)
            .status()
            .expect("failed to invoke `cc` to build the native stand-in library");
        assert!(cc_status.success(), "compiling native.c failed");

        let arena_lib = build_staticlib("thaw-arena");
        let link_status = Command::new("cc")
            .arg(&obj_path)
            .arg(&native_obj_path)
            .arg(&arena_lib)
            .arg("-o")
            .arg(&exe_path)
            .status()
            .expect("failed to invoke system `cc` linker");
        assert!(link_status.success(), "linking failed");

        let output = Command::new(&exe_path)
            .output()
            .expect("failed to execute compiled binary");
        assert!(output.status.success(), "binary exited non-zero");
        assert_eq!(String::from_utf8_lossy(&output.stdout), "34\n");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn reads_process_env() {
        let source = r#"
            function main(): void {
                console.log(process.env.THAW_TEST_VAR);
            }
        "#;
        assert_eq!(
            compile_and_run_with_env(source, "envvar", &[("THAW_TEST_VAR", "hello-env")]),
            "hello-env\n"
        );
    }

    #[test]
    fn unset_env_var_reads_as_empty_string_not_a_crash() {
        let source = r#"
            function main(): void {
                console.log(process.env.THAW_DEFINITELY_UNSET_VAR_XYZ);
                console.log("still alive");
            }
        "#;
        assert_eq!(compile_and_run(source, "envvar_unset"), "\nstill alive\n");
    }

    /// Full pipeline test for the Lambda entry point: a `handler(event:
    /// string): string` program is compiled, linked against both
    /// thaw-arena and thaw-runtime, run as a real subprocess against a
    /// mock Lambda Runtime API server (the same protocol thaw-runtime
    /// itself is tested against), and its actual HTTP interaction is
    /// verified end to end.
    #[test]
    fn compiles_and_runs_a_lambda_handler_against_a_mock_runtime_api() {
        let source = r#"
            function handler(event: string): string {
                console.log(event);
                return "{\"ok\":true}";
            }
        "#;

        let module = thaw_parser::parse_typescript(source).unwrap();
        let program = thaw_hir::lower_module(&module).unwrap();

        let context = Context::create();
        let mut compiler = HirCompiler::new(&context, "lambda_handler");
        compiler.compile_program(&program).unwrap();

        let dir = std::env::temp_dir().join(format!(
            "thaw-hir-codegen-test-lambda-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let obj_path = dir.join("out.o");
        let exe_path = dir.join("out");
        compiler.write_object_file(&obj_path).unwrap();

        let arena_lib = build_staticlib("thaw-arena");
        let runtime_lib = build_staticlib("thaw-runtime");

        let link_status = Command::new("cc")
            .arg(&obj_path)
            .arg(&arena_lib)
            .arg(&runtime_lib)
            .arg("-o")
            .arg(&exe_path)
            .status()
            .expect("failed to invoke system `cc` linker");
        assert!(link_status.success(), "linking failed");

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap().to_string();

        let (tx, rx) = mpsc::channel();
        let server = std::thread::spawn(move || {
            let (mut conn, _) = listener.accept().unwrap();
            let mut buf = [0u8; 4096];
            let _ = conn.read(&mut buf).unwrap();
            let body = "\"ping\"";
            let response = format!(
                "HTTP/1.1 200 OK\r\nLambda-Runtime-Aws-Request-Id: test-req-1\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            conn.write_all(response.as_bytes()).unwrap();
            drop(conn);

            let (mut conn, _) = listener.accept().unwrap();
            let mut buf = Vec::new();
            conn.read_to_end(&mut buf).unwrap();
            tx.send(String::from_utf8_lossy(&buf).into_owned()).unwrap();

            let response =
                "HTTP/1.1 202 Accepted\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
            conn.write_all(response.as_bytes()).unwrap();
        });

        let mut child = Command::new(&exe_path)
            .env("AWS_LAMBDA_RUNTIME_API", &addr)
            .stdout(Stdio::piped())
            .spawn()
            .expect("failed to spawn compiled Lambda handler binary");

        let post_request = rx
            .recv_timeout(Duration::from_secs(10))
            .expect("handler never posted a response to the mock runtime API");
        server.join().unwrap();

        // `thaw_runtime_run` loops forever by design (it's the actual
        // Lambda execution model) -- kill the process after we've observed
        // one full round trip rather than waiting for an exit that never
        // comes on its own.
        let _ = child.kill();
        let output = child.wait_with_output().unwrap();

        assert!(post_request.starts_with("POST /2018-06-01/runtime/invocation/test-req-1/response"));
        assert!(post_request.ends_with("{\"ok\":true}"));
        // Also confirms the fflush-after-console.log fix: stdout is a pipe
        // here (fully buffered by default in libc), and the process is
        // killed rather than exited normally, so without an explicit flush
        // this assertion would flake/fail.
        assert!(String::from_utf8_lossy(&output.stdout).contains("\"ping\""));

        let _ = std::fs::remove_dir_all(&dir);
    }
}
