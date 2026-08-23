use std::collections::HashMap;

use inkwell::builder::Builder;
use inkwell::context::Context;
use inkwell::execution_engine::ExecutionEngine;
use inkwell::module::Module;
use inkwell::values::{FloatValue, FunctionValue};
use inkwell::{FloatPredicate, OptimizationLevel};

use crate::ast::{Expr, Function, Prototype};

/// Every Kaleidoscope value is an `f64`, exactly like the upstream tutorial.
/// This is deliberately not the real Thaw HIR type system (see thaw-hir) --
/// it's scoped to "learn inkwell/LLVM", not to be production codegen.
pub struct Compiler<'ctx> {
    pub context: &'ctx Context,
    pub module: Module<'ctx>,
    builder: Builder<'ctx>,
    variables: HashMap<String, FloatValue<'ctx>>,
    current_fn: Option<FunctionValue<'ctx>>,
}

impl<'ctx> Compiler<'ctx> {
    pub fn new(context: &'ctx Context, module_name: &str) -> Self {
        Self {
            context,
            module: context.create_module(module_name),
            builder: context.create_builder(),
            variables: HashMap::new(),
            current_fn: None,
        }
    }

    pub fn create_jit_execution_engine(
        &self,
        opt_level: OptimizationLevel,
    ) -> Result<ExecutionEngine<'ctx>, String> {
        self.module
            .create_jit_execution_engine(opt_level)
            .map_err(|e| e.to_string())
    }

    pub fn compile_prototype(&self, proto: &Prototype) -> FunctionValue<'ctx> {
        if let Some(existing) = self.module.get_function(&proto.name) {
            return existing;
        }
        let f64_type = self.context.f64_type();
        let arg_types = vec![f64_type.into(); proto.args.len()];
        let fn_type = f64_type.fn_type(&arg_types, false);
        let function = self.module.add_function(&proto.name, fn_type, None);

        for (param, name) in function.get_param_iter().zip(proto.args.iter()) {
            param.into_float_value().set_name(name);
        }

        function
    }

    pub fn compile_function(&mut self, func: &Function) -> Result<FunctionValue<'ctx>, String> {
        let function = self.compile_prototype(&func.proto);
        let Some(body) = &func.body else {
            return Ok(function);
        };

        let entry = self.context.append_basic_block(function, "entry");
        self.builder.position_at_end(entry);

        self.variables.clear();
        for (param, name) in function.get_param_iter().zip(func.proto.args.iter()) {
            self.variables
                .insert(name.clone(), param.into_float_value());
        }
        self.current_fn = Some(function);

        let body_value = self.compile_expr(body)?;
        self.builder
            .build_return(Some(&body_value))
            .map_err(|e| e.to_string())?;

        if function.verify(true) {
            Ok(function)
        } else {
            let name = func.proto.name.clone();
            unsafe { function.delete() };
            Err(format!("LLVM verification failed for function `{name}`"))
        }
    }

    fn compile_expr(&mut self, expr: &Expr) -> Result<FloatValue<'ctx>, String> {
        match expr {
            Expr::Number(n) => Ok(self.context.f64_type().const_float(*n)),

            Expr::Variable(name) => self
                .variables
                .get(name)
                .copied()
                .ok_or_else(|| format!("unknown variable `{name}`")),

            Expr::Binary { op, lhs, rhs } => self.compile_binary(*op, lhs, rhs),

            Expr::Call { callee, args } => self.compile_call(callee, args),

            Expr::If {
                cond,
                then_branch,
                else_branch,
            } => self.compile_if(cond, then_branch, else_branch),

            Expr::For {
                var_name,
                start,
                end,
                step,
                body,
            } => self.compile_for(var_name, start, end, step.as_deref(), body),
        }
    }

    fn compile_binary(
        &mut self,
        op: char,
        lhs: &Expr,
        rhs: &Expr,
    ) -> Result<FloatValue<'ctx>, String> {
        let lhs_val = self.compile_expr(lhs)?;
        let rhs_val = self.compile_expr(rhs)?;

        let result = match op {
            '+' => self.builder.build_float_add(lhs_val, rhs_val, "addtmp"),
            '-' => self.builder.build_float_sub(lhs_val, rhs_val, "subtmp"),
            '*' => self.builder.build_float_mul(lhs_val, rhs_val, "multmp"),
            '/' => self.builder.build_float_div(lhs_val, rhs_val, "divtmp"),
            '<' | '>' => {
                let predicate = if op == '<' {
                    FloatPredicate::ULT
                } else {
                    FloatPredicate::UGT
                };
                let cmp = self
                    .builder
                    .build_float_compare(predicate, lhs_val, rhs_val, "cmptmp")
                    .map_err(|e| e.to_string())?;
                return self
                    .builder
                    .build_unsigned_int_to_float(cmp, self.context.f64_type(), "booltmp")
                    .map_err(|e| e.to_string());
            }
            other => return Err(format!("unknown binary operator `{other}`")),
        };
        result.map_err(|e| e.to_string())
    }

    fn compile_call(&mut self, callee: &str, args: &[Expr]) -> Result<FloatValue<'ctx>, String> {
        let function = self
            .module
            .get_function(callee)
            .ok_or_else(|| format!("call to undeclared function `{callee}`"))?;

        if function.count_params() as usize != args.len() {
            return Err(format!(
                "`{callee}` expects {} argument(s), got {}",
                function.count_params(),
                args.len()
            ));
        }

        let mut compiled_args = Vec::with_capacity(args.len());
        for arg in args {
            compiled_args.push(self.compile_expr(arg)?.into());
        }

        let call_site = self
            .builder
            .build_call(function, &compiled_args, "calltmp")
            .map_err(|e| e.to_string())?;

        call_site
            .try_as_basic_value()
            .basic()
            .map(|v| v.into_float_value())
            .ok_or_else(|| format!("`{callee}` did not return a value"))
    }

    fn compile_if(
        &mut self,
        cond: &Expr,
        then_branch: &Expr,
        else_branch: &Expr,
    ) -> Result<FloatValue<'ctx>, String> {
        let function = self.current_fn.expect("if-expr outside of a function body");

        let cond_val = self.compile_expr(cond)?;
        let zero = self.context.f64_type().const_float(0.0);
        let cond_bool = self
            .builder
            .build_float_compare(FloatPredicate::ONE, cond_val, zero, "ifcond")
            .map_err(|e| e.to_string())?;

        let then_bb = self.context.append_basic_block(function, "then");
        let else_bb = self.context.append_basic_block(function, "else");
        let merge_bb = self.context.append_basic_block(function, "ifcont");

        self.builder
            .build_conditional_branch(cond_bool, then_bb, else_bb)
            .map_err(|e| e.to_string())?;

        self.builder.position_at_end(then_bb);
        let then_val = self.compile_expr(then_branch)?;
        self.builder
            .build_unconditional_branch(merge_bb)
            .map_err(|e| e.to_string())?;
        let then_end_bb = self.builder.get_insert_block().unwrap();

        self.builder.position_at_end(else_bb);
        let else_val = self.compile_expr(else_branch)?;
        self.builder
            .build_unconditional_branch(merge_bb)
            .map_err(|e| e.to_string())?;
        let else_end_bb = self.builder.get_insert_block().unwrap();

        self.builder.position_at_end(merge_bb);
        let phi = self
            .builder
            .build_phi(self.context.f64_type(), "iftmp")
            .map_err(|e| e.to_string())?;
        phi.add_incoming(&[(&then_val, then_end_bb), (&else_val, else_end_bb)]);

        Ok(phi.as_basic_value().into_float_value())
    }

    fn compile_for(
        &mut self,
        var_name: &str,
        start: &Expr,
        end: &Expr,
        step: Option<&Expr>,
        body: &Expr,
    ) -> Result<FloatValue<'ctx>, String> {
        let function = self
            .current_fn
            .expect("for-expr outside of a function body");

        let start_val = self.compile_expr(start)?;
        let preheader_bb = self.builder.get_insert_block().unwrap();
        let loop_bb = self.context.append_basic_block(function, "loop");

        self.builder
            .build_unconditional_branch(loop_bb)
            .map_err(|e| e.to_string())?;
        self.builder.position_at_end(loop_bb);

        let phi = self
            .builder
            .build_phi(self.context.f64_type(), var_name)
            .map_err(|e| e.to_string())?;
        phi.add_incoming(&[(&start_val, preheader_bb)]);

        let previous_binding = self.variables.remove(var_name);
        self.variables.insert(
            var_name.to_string(),
            phi.as_basic_value().into_float_value(),
        );

        self.compile_expr(body)?;

        let step_val = match step {
            Some(step) => self.compile_expr(step)?,
            None => self.context.f64_type().const_float(1.0),
        };
        let current = phi.as_basic_value().into_float_value();
        let next_val = self
            .builder
            .build_float_add(current, step_val, "nextvar")
            .map_err(|e| e.to_string())?;

        // `end` is itself the continue-condition expression (e.g. `i < n`),
        // evaluated fresh each iteration against the current loop variable --
        // not a target value to compare `next_val` against.
        let end_val = self.compile_expr(end)?;
        let zero = self.context.f64_type().const_float(0.0);
        let cond = self
            .builder
            .build_float_compare(FloatPredicate::ONE, end_val, zero, "loopcond")
            .map_err(|e| e.to_string())?;

        let loop_end_bb = self.builder.get_insert_block().unwrap();
        let after_bb = self.context.append_basic_block(function, "afterloop");

        self.builder
            .build_conditional_branch(cond, loop_bb, after_bb)
            .map_err(|e| e.to_string())?;
        phi.add_incoming(&[(&next_val, loop_end_bb)]);

        self.builder.position_at_end(after_bb);

        self.variables.remove(var_name);
        if let Some(previous) = previous_binding {
            self.variables.insert(var_name.to_string(), previous);
        }

        Ok(self.context.f64_type().const_float(0.0))
    }
}
