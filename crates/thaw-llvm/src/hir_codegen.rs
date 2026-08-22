//! The real Thaw pipeline: `thaw_hir::HirProgram` -> LLVM IR -> a native
//! object file. This is a separate, unrelated compiler from the
//! `lexer`/`ast`/`parser`/`codegen` modules at the crate root, which are the
//! Kaleidoscope tutorial scaffold used to learn inkwell in the first place.
//!
//! Scope matches thaw-hir's Phase 0 lowering: numbers (f64), strings
//! (`i8*`, C-string style), booleans (`i1`), and functions, plus a
//! `console.log` builtin bridged to libc `puts`/`printf` as a bootstrap
//! (the real `console.log`/std implementation lands in Phase 2).

use std::collections::HashMap;
use std::path::Path;

use inkwell::builder::Builder;
use inkwell::context::Context;
use inkwell::module::{Linkage, Module};
use inkwell::targets::{
    CodeModel, FileType, InitializationConfig, RelocMode, Target, TargetMachine,
};
use inkwell::types::{BasicMetadataTypeEnum, BasicType};
use inkwell::values::{BasicMetadataValueEnum, BasicValueEnum, FunctionValue};
use inkwell::{FloatPredicate, OptimizationLevel};

use thaw_hir::{BinOp, HirExpr, HirFunction, HirLit, HirProgram, HirStmt, HirType};

/// The user's `main`, if any, is compiled under this symbol instead of
/// `main` so we can wrap it in a proper `i32 main(void)` C entry point
/// rather than exposing a `void`-returning function as the process entry.
const USER_MAIN_SYMBOL: &str = "thaw_user_main";

pub struct HirCompiler<'ctx> {
    context: &'ctx Context,
    module: Module<'ctx>,
    builder: Builder<'ctx>,
    variables: HashMap<String, BasicValueEnum<'ctx>>,
}

impl<'ctx> HirCompiler<'ctx> {
    pub fn new(context: &'ctx Context, module_name: &str) -> Self {
        Self {
            context,
            module: context.create_module(module_name),
            builder: context.create_builder(),
            variables: HashMap::new(),
        }
    }

    pub fn compile_program(&mut self, program: &HirProgram) -> Result<(), String> {
        self.declare_libc_builtins();

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

    fn declare_libc_builtins(&self) {
        let i8_ptr = self.context.ptr_type(inkwell::AddressSpace::default());
        let i32_type = self.context.i32_type();

        let puts_type = i32_type.fn_type(&[i8_ptr.into()], false);
        self.module.add_function("puts", puts_type, Some(Linkage::External));

        let printf_type = i32_type.fn_type(&[i8_ptr.into()], true);
        self.module
            .add_function("printf", printf_type, Some(Linkage::External));
    }

    fn llvm_symbol_for(name: &str) -> String {
        if name == "main" {
            USER_MAIN_SYMBOL.to_string()
        } else {
            name.to_string()
        }
    }

    fn basic_type(&self, ty: &HirType) -> Result<inkwell::types::BasicTypeEnum<'ctx>, String> {
        match ty {
            HirType::F64 => Ok(self.context.f64_type().into()),
            HirType::I64 => Ok(self.context.i64_type().into()),
            HirType::Bool => Ok(self.context.bool_type().into()),
            HirType::Str => Ok(self.context.ptr_type(inkwell::AddressSpace::default()).into()),
            other => Err(format!("Phase 0 codegen does not support type {other:?} yet")),
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
        for (param, hir_param) in function.get_param_iter().zip(func.params.iter()) {
            self.variables.insert(hir_param.name.clone(), param);
        }

        let mut returned = false;
        for stmt in &func.body {
            if self.compile_stmt(stmt)? {
                returned = true;
                break;
            }
        }

        if !returned {
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

    /// Returns `Ok(true)` if this statement was a `return` (so the caller
    /// should stop compiling further statements in this block).
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
            HirStmt::Let(name, _ty, expr) => {
                let val = self.compile_expr(expr)?;
                self.variables.insert(name.clone(), val);
                Ok(false)
            }
        }
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

            HirExpr::Var(name) => self
                .variables
                .get(name)
                .copied()
                .ok_or_else(|| format!("unknown variable `{name}`")),

            HirExpr::BinOp(op, lhs, rhs) => self.compile_binop(*op, lhs, rhs),

            HirExpr::Call(callee, args) => self.compile_call(callee, args),

            other => Err(format!("Phase 0 codegen does not support {other:?} yet")),
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
            return Err("call target must be a plain name in Phase 0".to_string());
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

    /// `console.log` is bridged straight to libc for Phase 0: strings go to
    /// `puts`, numbers go through `printf("%g\n", ...)`. The real `console`
    /// implementation belongs in `std/` once Phase 2 gets there.
    fn compile_console_log(&mut self, args: &[HirExpr]) -> Result<BasicValueEnum<'ctx>, String> {
        let [arg] = args else {
            return Err("console.log expects exactly one argument in Phase 0".to_string());
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

        let module = thaw_parser::parse_typescript(source).unwrap();
        let program = thaw_hir::lower_module(&module).unwrap();

        let context = Context::create();
        let mut compiler = HirCompiler::new(&context, "hello_world");
        compiler.compile_program(&program).unwrap();

        let dir = std::env::temp_dir().join(format!("thaw-hir-codegen-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let obj_path = dir.join("hello.o");
        let exe_path = dir.join("hello");

        compiler.write_object_file(&obj_path).unwrap();

        let link_status = Command::new("cc")
            .arg(&obj_path)
            .arg("-o")
            .arg(&exe_path)
            .status()
            .expect("failed to invoke system `cc` linker");
        assert!(link_status.success(), "linking failed");

        let output = Command::new(&exe_path)
            .output()
            .expect("failed to execute compiled binary");
        assert!(output.status.success());
        assert_eq!(
            String::from_utf8_lossy(&output.stdout),
            "Hello, Thaw!\n5\n"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }
}
