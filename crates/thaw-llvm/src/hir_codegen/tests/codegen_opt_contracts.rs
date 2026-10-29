// Contract for the fast-compile backend (`HirCompiler::set_codegen_opt(None)`): the object
// file is non-empty and the program prints exactly what the default backend prints.

#[test]
fn codegen_opt_none_emits_object_with_identical_program_output() {
    let source = r##"function main(): void {
  let total = 0;
  for (let i = 0; i < 1000; i++) { total += i * 2; }
  console.log(total);
  const label = "fast" + "-" + "compile";
  console.log(label, total > 10 ? "big" : "small");
}
"##;
    let module = thaw_parser::parse_typescript(source).unwrap();
    let program = thaw_hir::lower_module(&module).unwrap();
    let context = Context::create();
    let mut compiler = HirCompiler::new(&context, "codegen_opt_none_object");
    compiler.set_codegen_opt(inkwell::OptimizationLevel::None);
    compiler.compile_program(&program).unwrap();
    let obj_path = std::env::temp_dir().join(format!(
        "thaw-codegen-opt-none-{}.o",
        std::process::id()
    ));
    compiler.write_object_file(&obj_path).unwrap();
    let object_len = std::fs::metadata(&obj_path).unwrap().len();
    let _ = std::fs::remove_file(&obj_path);
    assert!(object_len > 0, "fast-compile object file is empty");

    let default = compile_and_run_raw_with_opt(
        source,
        "codegen_opt_default_output",
        &[],
        false,
        inkwell::OptimizationLevel::Default,
    );
    let fast = compile_and_run_raw_with_opt(
        source,
        "codegen_opt_none_output",
        &[],
        false,
        inkwell::OptimizationLevel::None,
    );
    assert!(default.status.success() && fast.status.success());
    assert_eq!(
        String::from_utf8_lossy(&fast.stdout),
        String::from_utf8_lossy(&default.stdout)
    );
    assert_eq!(
        String::from_utf8_lossy(&fast.stdout),
        "999000\nfast-compile big\n"
    );
}
