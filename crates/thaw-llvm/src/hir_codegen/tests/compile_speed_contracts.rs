fn compile_speed_ir(source: &str) -> String {
    let module = thaw_parser::parse_typescript(source).unwrap();
    let program = thaw_hir::lower_module(&module).unwrap();
    let context = Context::create();
    let mut compiler = HirCompiler::new(&context, "compile_speed");
    compiler.compile_program(&program).unwrap();
    compiler.module.verify().unwrap();
    compiler.print_to_string()
}

#[test]
fn identical_string_constants_are_emitted_once() {
    let ir = compile_speed_ir(
        r#"
        function main(): void {
            const a: Record<string, number> = {};
            console.log("shared literal text");
            console.log("shared literal text");
            console.log("shared literal text");
        }
    "#,
    );
    let copies = ir
        .lines()
        .filter(|line| line.starts_with('@') && line.contains("c\"shared literal text\\00\""))
        .count();
    assert_eq!(copies, 1, "{ir}");
}

#[test]
fn synthetic_error_stores_live_in_one_shared_function() {
    let ir = compile_speed_ir(
        r#"
        function call(f: (x: number) => number, g: (x: number) => number): number {
            return f(1) + g(2);
        }
        function main(): void {
            console.log(call((x) => x + 1, (x) => x * 2));
        }
    "#,
    );
    assert_eq!(ir.matches("define internal ptr @thaw_set_synthetic_error").count(), 1, "{ir}");
    // The stores into the pending-exception globals are no longer repeated per throw site.
    let setter_calls = ir.matches("call ptr @thaw_set_synthetic_error").count();
    let direct_frames = ir.matches("call ptr @thaw_error_frame").count();
    assert!(setter_calls >= 1, "{ir}");
    assert_eq!(direct_frames, 1, "only the shared setter calls thaw_error_frame:\n{ir}");
}
