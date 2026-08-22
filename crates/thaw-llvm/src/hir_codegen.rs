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
//! by thaw-hir's lowering), `throw`/`try`/`catch` (same-function-scope only
//! -- see `thaw_hir::HirStmt::Try`), mutable local variables (`let`,
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
use inkwell::targets::{
    CodeModel, FileType, InitializationConfig, RelocMode, Target, TargetMachine,
};
use inkwell::types::{BasicMetadataTypeEnum, BasicType, BasicTypeEnum};
use inkwell::values::{BasicMetadataValueEnum, BasicValueEnum, FunctionValue, PointerValue};
use inkwell::{AddressSpace, FloatPredicate, OptimizationLevel};

use thaw_hir::{BinOp, FfiSignature, HirExpr, HirFunction, HirLit, HirProgram, HirStmt, HirType};

/// The user's `main`, if any, is compiled under this symbol instead of
/// `main` so we can wrap it in a proper `i32 main(void)` C entry point
/// rather than exposing a `void`-returning function as the process entry.
const USER_MAIN_SYMBOL: &str = "thaw_user_main";

/// Byte size of an array's length header (a single `i64`) that precedes its
/// elements in the arena-allocated buffer. See the module doc for the layout.
const ARRAY_HEADER_BYTES: u64 = 8;
/// Phase 1 arrays only ever hold `f64` elements (see module doc).
const ARRAY_ELEM_BYTES: u64 = 8;
/// Phase 2 objects only ever hold `f64` fields (see module doc); no header
/// (field count/order is static, part of the type, not a runtime value).
const OBJECT_FIELD_BYTES: u64 = 8;

pub struct HirCompiler<'ctx> {
    context: &'ctx Context,
    module: Module<'ctx>,
    builder: Builder<'ctx>,
    variables: HashMap<String, (PointerValue<'ctx>, BasicTypeEnum<'ctx>)>,
    /// Stack of enclosing `try` targets: the `catch` block to jump to and
    /// the slot to store the thrown value into. `throw` targets the
    /// innermost (last) entry; empty means no enclosing `try`.
    catch_stack: Vec<(BasicBlock<'ctx>, PointerValue<'ctx>)>,
}

impl<'ctx> HirCompiler<'ctx> {
    pub fn new(context: &'ctx Context, module_name: &str) -> Self {
        Self {
            context,
            module: context.create_module(module_name),
            builder: context.create_builder(),
            variables: HashMap::new(),
            catch_stack: Vec::new(),
        }
    }

    pub fn compile_program(&mut self, program: &HirProgram) -> Result<(), String> {
        self.declare_runtime_builtins();

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

    /// Declares libc's `puts`/`printf` (the Phase 0 `console.log` bootstrap)
    /// and thaw-arena's `thaw_arena_alloc` (backing Phase 1 arrays).
    fn declare_runtime_builtins(&self) {
        let i8_ptr = self.context.ptr_type(AddressSpace::default());
        let i32_type = self.context.i32_type();
        let i64_type = self.context.i64_type();

        let puts_type = i32_type.fn_type(&[i8_ptr.into()], false);
        self.module.add_function("puts", puts_type, Some(Linkage::External));

        let printf_type = i32_type.fn_type(&[i8_ptr.into()], true);
        self.module
            .add_function("printf", printf_type, Some(Linkage::External));

        let arena_alloc_type = i8_ptr.fn_type(&[i64_type.into(), i64_type.into()], false);
        self.module
            .add_function("thaw_arena_alloc", arena_alloc_type, Some(Linkage::External));

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
            other => Err(format!("Phase 1/2 codegen does not support type {other:?} yet")),
        }
    }

    fn declare_function(&mut self, func: &HirFunction) -> Result<FunctionValue<'ctx>, String> {
        let param_types = func
            .params
            .iter()
            .map(|p| self.basic_type(&p.ty).map(BasicMetadataTypeEnum::from))
            .collect::<Result<Vec<_>, _>>()?;

        let fn_type = match &func.ret {
            HirType::Void => self.context.void_type().fn_type(&param_types, false),
            ret => self.basic_type(ret)?.fn_type(&param_types, false),
        };

        let symbol = Self::llvm_symbol_for(&func.name);
        Ok(self.module.add_function(&symbol, fn_type, None))
    }

    /// Declares an ambient (`declare function`) signature as an `extern
    /// "C"` symbol -- see docs/design/bridge.md section 6. The final link
    /// step must resolve it from somewhere else (a real native library
    /// today; thaw-registry eventually).
    fn declare_extern_function(&mut self, sig: &FfiSignature) -> Result<FunctionValue<'ctx>, String> {
        let param_types = sig
            .params
            .iter()
            .map(|ty| self.basic_type(ty).map(BasicMetadataTypeEnum::from))
            .collect::<Result<Vec<_>, _>>()?;

        let fn_type = match &sig.ret {
            HirType::Void => self.context.void_type().fn_type(&param_types, false),
            ret => self.basic_type(ret)?.fn_type(&param_types, false),
        };

        Ok(self
            .module
            .add_function(&sig.symbol, fn_type, Some(Linkage::External)))
    }

    fn compile_function_body(&mut self, func: &HirFunction) -> Result<(), String> {
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
                self.builder.build_store(slot, val).map_err(|e| e.to_string())?;
                self.variables.insert(name.clone(), (slot, llvm_ty));
                Ok(false)
            }

            HirStmt::If(cond, then_branch, else_branch) => {
                self.compile_if(cond, then_branch, else_branch)
            }

            HirStmt::While(cond, body) => self.compile_while(cond, body),

            HirStmt::Throw(expr) => {
                let val = self.compile_expr(expr)?;
                let (catch_bb, catch_slot) = *self.catch_stack.last().ok_or(
                    "`throw` outside of any `try` is not supported yet \
                     (Phase 1 has no cross-function unwinding)",
                )?;
                self.builder
                    .build_store(catch_slot, val)
                    .map_err(|e| e.to_string())?;
                self.builder
                    .build_unconditional_branch(catch_bb)
                    .map_err(|e| e.to_string())?;
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
            self.builder.build_unreachable().map_err(|e| e.to_string())?;
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
        let body_terminated = self.compile_block(body)?;
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

        let str_ty: BasicTypeEnum = self.context.ptr_type(AddressSpace::default()).into();
        let catch_slot = self
            .builder
            .build_alloca(str_ty, "catch_slot")
            .map_err(|e| e.to_string())?;

        self.builder
            .build_unconditional_branch(try_bb)
            .map_err(|e| e.to_string())?;

        self.builder.position_at_end(try_bb);
        self.catch_stack.push((catch_bb, catch_slot));
        let try_terminated = self.compile_block(body)?;
        self.catch_stack.pop();
        if !try_terminated {
            self.builder
                .build_unconditional_branch(merge_bb)
                .map_err(|e| e.to_string())?;
        }

        self.builder.position_at_end(catch_bb);
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
            self.builder.build_unreachable().map_err(|e| e.to_string())?;
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
            HirExpr::Lit(HirLit::Bool(b)) => Ok(self
                .context
                .bool_type()
                .const_int(*b as u64, false)
                .into()),
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
                self.builder.build_load(ty, ptr, name).map_err(|e| e.to_string())
            }

            HirExpr::Assign(name, value) => {
                let val = self.compile_expr(value)?;
                let (ptr, _ty) = *self
                    .variables
                    .get(name)
                    .ok_or_else(|| format!("assignment to undeclared variable `{name}`"))?;
                self.builder.build_store(ptr, val).map_err(|e| e.to_string())?;
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
            HirExpr::Await(inner) => self.compile_expr(inner),

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
            let offset = i64_type.const_int(ARRAY_HEADER_BYTES + ARRAY_ELEM_BYTES * i as u64, false);
            let elem_ptr = unsafe {
                self.builder
                    .build_in_bounds_gep(self.context.i8_type(), base_ptr, &[offset], "elem_ptr")
                    .map_err(|e| e.to_string())?
            };
            self.builder.build_store(elem_ptr, val).map_err(|e| e.to_string())?;
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
            .build_int_add(byte_offset, i64_type.const_int(ARRAY_HEADER_BYTES, false), "byteoff_hdr")
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
    fn compile_object_lit(&mut self, fields: &[(String, HirExpr)]) -> Result<BasicValueEnum<'ctx>, String> {
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
            self.builder.build_store(field_ptr, val).map_err(|e| e.to_string())?;
        }

        Ok(base_ptr.into())
    }

    /// Looks up `field`'s declared type within `object_ty`, so a read
    /// knows whether to load an `f64`, a pointer (nested object/array/
    /// string/json), etc. -- fields are no longer assumed to all be `f64`
    /// now that nested objects are supported.
    fn field_type(&self, object_ty: &HirType, field: &str) -> Result<HirType, String> {
        let HirType::Object(fields) = object_ty else {
            return Err(format!("`.{field}` used on a non-object type {object_ty:?}"));
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
            return Err(format!("`.{field}` used on a non-object type {object_ty:?}"));
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
    fn compile_json_get(&mut self, obj: &HirExpr, field: &str) -> Result<BasicValueEnum<'ctx>, String> {
        let obj_val = self.compile_expr(obj)?;
        let key_global = self
            .builder
            .build_global_string_ptr(field, "jsonkey")
            .map_err(|e| e.to_string())?;
        let get_fn = self.module.get_function("thaw_json_get").unwrap();
        let call = self
            .builder
            .build_call(get_fn, &[obj_val.into(), key_global.as_pointer_value().into()], "json_get")
            .map_err(|e| e.to_string())?;
        call.try_as_basic_value()
            .basic()
            .ok_or_else(|| "thaw_json_get did not return a value".to_string())
    }

    /// `json[index]`, via thaw-std's `thaw_json_index`.
    fn compile_json_index(&mut self, obj: &HirExpr, index: &HirExpr) -> Result<BasicValueEnum<'ctx>, String> {
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

    fn compile_env_var(&mut self, name: &str) -> Result<BasicValueEnum<'ctx>, String> {
        let name_global = self
            .builder
            .build_global_string_ptr(name, "envname")
            .map_err(|e| e.to_string())?;
        let getenv_fn = self.module.get_function("getenv").unwrap();
        let call = self
            .builder
            .build_call(getenv_fn, &[name_global.as_pointer_value().into()], "getenv_call")
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

        let is_null = self.builder.build_is_null(raw_ptr, "is_null").map_err(|e| e.to_string())?;
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
        let phi = self.builder.build_phi(ptr_ty, "envval").map_err(|e| e.to_string())?;
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
            "JSON.parse" => {
                return self.compile_single_arg_call("thaw_json_parse", args, "JSON.parse")
            }
            "JSON.stringify" => {
                return self.compile_single_arg_call("thaw_json_stringify", args, "JSON.stringify")
            }
            _ => {}
        }

        let symbol = Self::llvm_symbol_for(name);
        let function = self
            .module
            .get_function(&symbol)
            .ok_or_else(|| format!("call to undeclared function `{name}`"))?;

        self.build_call_with(function, args, name)
    }

    /// `HirExpr::FfiCall` -- an ambient `declare function` call (see
    /// docs/design/bridge.md). By the time codegen sees this, the symbol
    /// is already declared (`declare_extern_function` ran in the
    /// `compile_program` pre-pass), so this is otherwise identical to a
    /// normal call.
    fn compile_ffi_call(
        &mut self,
        sig: &FfiSignature,
        args: &[HirExpr],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let function = self
            .module
            .get_function(&sig.symbol)
            .expect("extern function was declared in compile_program's pre-pass");
        self.build_call_with(function, args, &sig.symbol)
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
                    .build_select(b, true_str.as_pointer_value(), false_str.as_pointer_value(), "bool_str")
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
        self.builder
            .build_call(user_main, &[], "call_thaw_user_main")
            .unwrap();
        self.finish_c_main();
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
            return Err(
                "`handler` must have the signature `(event: string): string`".to_string(),
            );
        }
        let handler_fn = self.module.get_function("handler").unwrap();

        let i8_ptr = self.context.ptr_type(AddressSpace::default());
        let run_type = self.context.void_type().fn_type(&[i8_ptr.into()], false);
        let run_fn = self
            .module
            .add_function("thaw_runtime_run", run_type, Some(Linkage::External));

        let (_main_fn, entry) = self.new_c_main();
        self.builder.position_at_end(entry);
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

        // Always link both -- an unreferenced static archive member is
        // simply never pulled in, same reasoning as thaw-cli's build().
        let arena_lib = build_staticlib("thaw-arena");
        let std_lib = build_staticlib("thaw-std");

        let link_status = Command::new("cc")
            .arg(&obj_path)
            .arg(&arena_lib)
            .arg(&std_lib)
            .arg("-o")
            .arg(&exe_path)
            .status()
            .expect("failed to invoke system `cc` linker");
        assert!(link_status.success(), "linking failed");

        let output = Command::new(&exe_path)
            .envs(envs.iter().copied())
            .output()
            .expect("failed to execute compiled binary");
        assert!(output.status.success(), "binary exited non-zero");

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
        for line in stdout.lines() {
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
        assert_eq!(
            compile_and_run(source, "trycatch"),
            "before\nboom\nafter\n"
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
        assert_eq!(
            compile_and_run(source, "objects"),
            "1\n3\n11\n7\n"
        );
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
        assert_eq!(program.extern_functions.len(), 1, "sanity: this is really an FFI call");

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
        assert_eq!(
            compile_and_run(source, "envvar_unset"),
            "\nstill alive\n"
        );
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

            let response = "HTTP/1.1 202 Accepted\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
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
