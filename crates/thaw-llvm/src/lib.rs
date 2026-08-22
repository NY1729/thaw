pub mod ast;
pub mod codegen;
pub mod hir_codegen;
pub mod lexer;
pub mod parser;

pub use ast::{Expr, Function, Prototype, TopLevel};
pub use codegen::Compiler;
pub use hir_codegen::HirCompiler;

#[cfg(test)]
mod tests {
    use super::*;
    use inkwell::context::Context;
    use inkwell::execution_engine::JitFunction;
    use inkwell::OptimizationLevel;

    /// Smoke test: build `fn answer() -> f64 { 42 }` directly with inkwell
    /// (bypassing the parser) and JIT-execute it. Confirms the inkwell <->
    /// system LLVM toolchain link works end to end.
    #[test]
    fn jit_smoke_test() {
        let context = Context::create();
        let module = context.create_module("thaw_smoke");
        let builder = context.create_builder();

        let f64_type = context.f64_type();
        let fn_type = f64_type.fn_type(&[], false);
        let function = module.add_function("answer", fn_type, None);
        let entry = context.append_basic_block(function, "entry");
        builder.position_at_end(entry);

        let forty_two = f64_type.const_float(42.0);
        builder.build_return(Some(&forty_two)).unwrap();

        let execution_engine = module
            .create_jit_execution_engine(OptimizationLevel::None)
            .expect("failed to create JIT execution engine");

        type AnswerFn = unsafe extern "C" fn() -> f64;
        unsafe {
            let answer: JitFunction<AnswerFn> = execution_engine
                .get_function("answer")
                .expect("failed to find `answer` in JIT module");
            assert_eq!(answer.call(), 42.0);
        }
    }

    fn jit_eval(source: &str) -> f64 {
        let program = parser::parse(source).expect("parse error");
        let context = Context::create();
        let mut compiler = Compiler::new(&context, "test_module");
        let engine = compiler
            .create_jit_execution_engine(OptimizationLevel::None)
            .expect("failed to create JIT execution engine");

        let mut result = None;
        for (i, item) in program.into_iter().enumerate() {
            match item {
                TopLevel::Extern(proto) => {
                    compiler.compile_prototype(&proto);
                }
                TopLevel::Function(func) => {
                    compiler.compile_function(&func).expect("codegen error");
                }
                TopLevel::Expression(expr) => {
                    let anon_name = format!("__anon_expr_{i}");
                    let func = Function {
                        proto: Prototype {
                            name: anon_name.clone(),
                            args: vec![],
                        },
                        body: Some(expr),
                    };
                    compiler.compile_function(&func).expect("codegen error");
                    type NullaryFn = unsafe extern "C" fn() -> f64;
                    unsafe {
                        let f: JitFunction<NullaryFn> = engine
                            .get_function(&anon_name)
                            .expect("failed to find anon expr in JIT module");
                        result = Some(f.call());
                    }
                }
            }
        }
        result.expect("program contained no top-level expression")
    }

    #[test]
    fn evaluates_arithmetic_with_precedence() {
        assert_eq!(jit_eval("1 + 2 * 3"), 7.0);
    }

    #[test]
    fn calls_a_user_defined_function() {
        assert_eq!(jit_eval("def add(a b) a + b\nadd(3, 4)"), 7.0);
    }

    #[test]
    fn evaluates_recursive_fibonacci_via_if() {
        let src = "def fib(x) if x < 3 then 1 else fib(x - 1) + fib(x - 2)\nfib(10)";
        assert_eq!(jit_eval(src), 55.0);
    }

    #[test]
    fn evaluates_for_loop() {
        // Loop bodies can't mutate outer state (no alloca'd variables yet),
        // so this only proves the loop actually iterates & terminates
        // by returning the tutorial's standard sentinel value.
        assert_eq!(jit_eval("for i = 1, i < 10, 1 in i"), 0.0);
    }
}
