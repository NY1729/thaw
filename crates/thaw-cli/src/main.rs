use std::path::{Path, PathBuf};
use std::process::Command;

use inkwell::context::Context;
use thaw_llvm::HirCompiler;

fn main() {
    let args: Vec<String> = std::env::args().collect();

    match args.get(1).map(String::as_str) {
        Some("build") => {
            if let Err(err) = run_build(&args[2..]) {
                eprintln!("error: {err}");
                std::process::exit(1);
            }
        }
        _ => {
            eprintln!("usage: thaw build <input.ts> [-o <output>]");
            std::process::exit(1);
        }
    }
}

fn run_build(args: &[String]) -> Result<(), String> {
    let mut input: Option<PathBuf> = None;
    let mut output: Option<PathBuf> = None;

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "-o" | "--output" => {
                i += 1;
                let value = args.get(i).ok_or("-o requires a path argument")?;
                output = Some(PathBuf::from(value));
            }
            other => {
                if input.is_some() {
                    return Err(format!("unexpected extra argument `{other}`"));
                }
                input = Some(PathBuf::from(other));
            }
        }
        i += 1;
    }

    let input = input.ok_or("missing input file (usage: thaw build <input.ts> [-o <output>])")?;
    let output = output.unwrap_or_else(|| {
        let stem = input.file_stem().unwrap_or_default();
        PathBuf::from(stem)
    });

    build(&input, &output)
}

fn build(input: &Path, output: &Path) -> Result<(), String> {
    let source = std::fs::read_to_string(input)
        .map_err(|e| format!("failed to read `{}`: {e}", input.display()))?;

    let module = thaw_parser::parse_typescript(&source)?;
    let program = thaw_hir::lower_module(&module)?;

    let context = Context::create();
    let mut compiler = HirCompiler::new(&context, input.to_string_lossy().as_ref());
    compiler.compile_program(&program)?;

    let obj_path = output.with_extension("o");
    compiler.write_object_file(&obj_path)?;

    let link_status = Command::new("cc")
        .arg(&obj_path)
        .arg("-o")
        .arg(output)
        .status()
        .map_err(|e| format!("failed to invoke system `cc` linker: {e}"))?;

    let _ = std::fs::remove_file(&obj_path);

    if !link_status.success() {
        return Err("linking failed".to_string());
    }

    println!("built `{}`", output.display());
    Ok(())
}
