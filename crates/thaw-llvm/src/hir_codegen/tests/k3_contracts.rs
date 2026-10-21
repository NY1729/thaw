#[test]
fn k3_numeric_strict_equality_emits_no_closure_calls() {
    // A numeric `===` is a plain float compare; binding its operands into
    // call-argument closures allocated on every evaluation.
    let source = r#"
        function main(): void {
            let n = 0;
            let i = 0;
            while (i < 3) {
                if (i === 1) n++;
                i++;
            }
            console.log(n, 0 / 0 === 0 / 0, -0 === 0);
        }
    "#;
    let module = thaw_parser::parse_typescript(source).unwrap();
    let program = thaw_hir::lower_module(&module).unwrap();
    let context = Context::create();
    let mut compiler = HirCompiler::new(&context, "k3_numeric_strict_equality");
    compiler.compile_program(&program).unwrap();
    let ir = compiler.module.print_to_string().to_string();
    assert!(
        !ir.contains("__thaw_lambda"),
        "numeric === must not allocate operand closures:\n{ir}"
    );
}
