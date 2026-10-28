// Library mode (`thaw lib build`): C-callable wrappers instead of a C `main`.

fn library_mode_ir(source: &str, exports: Vec<LibraryExport>) -> Result<String, String> {
    let module = thaw_parser::parse_typescript(source).unwrap();
    let program = thaw_hir::lower_module(&module).unwrap();
    let context = Context::create();
    let mut compiler = HirCompiler::new(&context, "library_mode_contract");
    if !exports.is_empty() {
        compiler.set_library_mode(exports);
    }
    compiler.compile_program(&program)?;
    Ok(compiler.print_to_string())
}

const LIBRARY_CLAMP: &str = "function clamp(n: number, lo: number, hi: number): number { return n < lo ? lo : n > hi ? hi : n; }\n\
                             function label(s: string): string { return s; }\n";

fn library_clamp_export() -> Vec<LibraryExport> {
    vec![LibraryExport { hir_name: "clamp".into(), c_symbol: "thaw_pkg_t_clamp".into() }]
}

#[test]
fn library_mode_exports_a_c_wrapper_and_no_main() {
    let ir = library_mode_ir(LIBRARY_CLAMP, library_clamp_export()).unwrap();
    assert!(!ir.contains("define i32 @main"), "{ir}");
    assert!(ir.contains("define double @thaw_pkg_t_clamp(double %0, double %1, double %2)"), "{ir}");
    assert!(ir.contains("define internal double @clamp("), "{ir}");
    assert!(!ir.contains("define double @clamp("), "{ir}");
}

#[test]
fn library_mode_rejects_a_non_scalar_export() {
    let error = library_mode_ir(
        LIBRARY_CLAMP,
        vec![LibraryExport { hir_name: "label".into(), c_symbol: "thaw_pkg_t_label".into() }],
    )
    .unwrap_err();
    assert!(error.contains("not a scalar"), "{error}");
}

#[test]
fn library_mode_is_opt_in_the_default_path_keeps_main() {
    let ir = library_mode_ir(&format!("{LIBRARY_CLAMP}function main(): void {{}}\n"), Vec::new()).unwrap();
    assert!(ir.contains("define i32 @main"), "{ir}");
}
