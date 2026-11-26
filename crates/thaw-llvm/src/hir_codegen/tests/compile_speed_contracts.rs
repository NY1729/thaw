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

#[test]
fn call_free_leaf_functions_drop_the_stack_guard_and_may_inline() {
    let context = Context::create();
    let module = thaw_parser::parse_typescript(
        r#"
        function sq(x: number): number { return x * x + 1; }
        function fib(n: number): number { return n < 2 ? n : fib(n - 1) + fib(n - 2); }
        function main(): void { console.log(sq(3) + fib(5)); }
    "#,
    )
    .unwrap();
    let program = thaw_hir::lower_module(&module).unwrap();
    let mut compiler = HirCompiler::new(&context, "leaf_guard");
    compiler.compile_program(&program).unwrap();
    let noinline = inkwell::attributes::Attribute::get_named_enum_kind_id("noinline");
    let find = |suffix: &str| {
        compiler
            .module
            .get_functions()
            .find(|f| f.get_name().to_string_lossy().ends_with(suffix) && f.count_basic_blocks() > 0)
            .unwrap_or_else(|| panic!("no function ending in {suffix}"))
    };
    use inkwell::values::AnyValue;
    let (leaf, recursive) = (find("sq"), find("fib"));
    let attr = |f: inkwell::values::FunctionValue| {
        f.get_enum_attribute(inkwell::attributes::AttributeLoc::Function, noinline).is_some()
    };
    // Stack traces need the frame of a function that can run user code; a leaf has none to show.
    assert!(!attr(leaf) && attr(recursive));
    assert!(leaf.print_to_string().to_string().contains("br i1 false"), "{}", leaf.print_to_string());
    assert!(!recursive.print_to_string().to_string().contains("br i1 false"));
}
