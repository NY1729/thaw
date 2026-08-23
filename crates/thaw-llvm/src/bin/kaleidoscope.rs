use std::io::{self, Write};

use inkwell::context::Context;
use inkwell::OptimizationLevel;

use thaw_llvm::ast::{Function, Prototype, TopLevel};
use thaw_llvm::{parser, Compiler};

/// Host function exposed to Kaleidoscope source via `extern printd(x)`.
/// This is the same shape thaw-bridge will eventually generate en masse for
/// npm package FFI: a native Rust function, mapped into the JIT'd module by
/// name so compiled code can call straight into the host.
extern "C" fn printd(x: f64) -> f64 {
    println!("{x}");
    x
}

/// Builds a fresh module containing every extern/def seen so far in the
/// session. Each REPL line gets a brand new Context + module + JIT engine
/// rather than reusing one across the session: legacy MCJIT (what
/// `create_jit_execution_engine` binds to) fully finalizes a module the
/// first time you resolve a function address in it, so functions added
/// afterward are never picked up. Kaleidoscope programs are tiny, so
/// replaying everything into a new module per line is cheap and sidesteps
/// that limitation entirely instead of building a multi-module ORC linker.
fn build_world<'ctx>(
    context: &'ctx Context,
    module_name: &str,
    externs: &[Prototype],
    functions: &[Function],
) -> Compiler<'ctx> {
    let mut compiler = Compiler::new(context, module_name);
    for proto in externs {
        compiler.compile_prototype(proto);
    }
    for func in functions {
        // Already validated when it was first `def`'d; safe to ignore here.
        let _ = compiler.compile_function(func);
    }
    compiler
}

fn main() {
    println!("Thaw Kaleidoscope REPL (Phase 0 sandbox). def / extern / expression per line. Ctrl-D to exit.");

    let mut externs: Vec<Prototype> = Vec::new();
    let mut functions: Vec<Function> = Vec::new();
    let mut anon_counter = 0usize;

    let stdin = io::stdin();
    loop {
        print!("ks> ");
        io::stdout().flush().unwrap();

        let mut line = String::new();
        if stdin.read_line(&mut line).unwrap_or(0) == 0 {
            println!();
            break;
        }
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        let program = match parser::parse(line) {
            Ok(program) => program,
            Err(err) => {
                eprintln!("parse error: {err}");
                continue;
            }
        };

        for item in program {
            match item {
                TopLevel::Extern(proto) => {
                    println!("declared `{}`", proto.name);
                    externs.push(proto);
                }

                TopLevel::Function(func) => {
                    let scratch_ctx = Context::create();
                    let mut scratch = build_world(&scratch_ctx, "scratch", &externs, &functions);
                    match scratch.compile_function(&func) {
                        Ok(_) => {
                            println!("defined `{}`", func.proto.name);
                            functions.push(func);
                        }
                        Err(err) => eprintln!("codegen error: {err}"),
                    }
                }

                TopLevel::Expression(expr) => {
                    anon_counter += 1;
                    let name = format!("__anon_expr_{anon_counter}");
                    let anon = Function {
                        proto: Prototype {
                            name: name.clone(),
                            args: vec![],
                        },
                        body: Some(expr),
                    };

                    let context = Context::create();
                    let mut compiler = build_world(&context, "repl_eval", &externs, &functions);
                    match compiler.compile_function(&anon) {
                        Ok(_) => {
                            let engine = compiler
                                .create_jit_execution_engine(OptimizationLevel::None)
                                .expect("failed to create JIT execution engine");

                            for proto in &externs {
                                if proto.name == "printd" {
                                    if let Some(function) = compiler.module.get_function("printd") {
                                        engine.add_global_mapping(
                                            &function,
                                            printd as *const () as usize,
                                        );
                                    }
                                }
                            }

                            type NullaryFn = unsafe extern "C" fn() -> f64;
                            unsafe {
                                match engine.get_function::<NullaryFn>(&name) {
                                    Ok(f) => println!("=> {}", f.call()),
                                    Err(err) => eprintln!("jit error: {err:?}"),
                                }
                            }
                        }
                        Err(err) => eprintln!("codegen error: {err}"),
                    }
                }
            }
        }
    }
}
