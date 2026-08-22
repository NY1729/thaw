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

use thaw_hir::{BinOp, HirExpr, HirFunction, HirLit, HirProgram, HirStmt, HirType};

/// The user's `main`, if any, is compiled under this symbol instead of
/// `main` so we can wrap it in a proper `i32 main(void)` C entry point
/// rather than exposing a `void`-returning function as the process entry.
const USER_MAIN_SYMBOL: &str = "thaw_user_main";

/// Byte size of an array's length header (a single `i64`) that precedes its
/// elements in the arena-allocated buffer. See the module doc for the layout.
const ARRAY_HEADER_BYTES: u64 = 8;
/// Phase 1 arrays only ever hold `f64` elements (see module doc).
const ARRAY_ELEM_BYTES: u64 = 8;

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

        for func in &program.functions {
            self.declare_function(func)?;
        }
        for func in &program.functions {
            self.compile_function_body(func)?;
        }

        if self.module.get_function(USER_MAIN_SYMBOL).is_none() {
            return Err("no `function main(): void { ... }` found".to_string());
        }
        self.emit_c_main();

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
            other => Err(format!("Phase 1 codegen does not support type {other:?} yet")),
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

            other => Err(format!("Phase 1 codegen does not support {other:?} yet")),
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

        if name == "console.log" {
            return self.compile_console_log(args);
        }

        let symbol = Self::llvm_symbol_for(name);
        let function = self
            .module
            .get_function(&symbol)
            .ok_or_else(|| format!("call to undeclared function `{name}`"))?;

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
            other => {
                return Err(format!(
                    "console.log does not support values of this kind yet: {other:?}"
                ))
            }
        }

        Ok(self.context.i32_type().const_int(0, false).into())
    }

    /// Emits `int main(void) { thaw_user_main(); return 0; }`, the real
    /// process entry point that the system linker/CRT expects.
    fn emit_c_main(&mut self) {
        let user_main = self.module.get_function(USER_MAIN_SYMBOL).unwrap();

        let i32_type = self.context.i32_type();
        let main_type = i32_type.fn_type(&[], false);
        let main_fn = self.module.add_function("main", main_type, None);
        let entry = self.context.append_basic_block(main_fn, "entry");
        self.builder.position_at_end(entry);

        self.builder
            .build_call(user_main, &[], "call_thaw_user_main")
            .unwrap();
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
    use std::process::Command;

    /// Builds the given TS source into a standalone native binary (linking
    /// thaw-arena's staticlib too, since array-using programs call into
    /// it), runs it, and returns its captured stdout. This is the same
    /// pipeline thaw-cli drives, just inlined for testing.
    fn compile_and_run(source: &str, test_name: &str) -> String {
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

        let arena_lib = build_thaw_arena_staticlib();

        let link_status = Command::new("cc")
            .arg(&obj_path)
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

        let _ = std::fs::remove_dir_all(&dir);
        String::from_utf8_lossy(&output.stdout).into_owned()
    }

    /// Builds thaw-arena as a staticlib (if not already built) and returns
    /// the path to the resulting `.a` file, by parsing `cargo build`'s JSON
    /// artifact output -- robust to `CARGO_TARGET_DIR` overrides, unlike
    /// guessing a relative path.
    fn build_thaw_arena_staticlib() -> std::path::PathBuf {
        let output = Command::new("cargo")
            .args([
                "build",
                "--release",
                "-p",
                "thaw-arena",
                "--message-format=json",
            ])
            .output()
            .expect("failed to invoke `cargo build -p thaw-arena`");
        assert!(output.status.success(), "building thaw-arena failed");

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
        panic!("could not find libthaw_arena.a in `cargo build` output:\n{stdout}");
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
}
