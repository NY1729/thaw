#[test]
fn lowers_typed_function_with_binary_op() {
    let program = lower("function add(a: number, b: number): number { return a + b; }");
    assert_eq!(program.functions.len(), 1);
    let f = &program.functions[0];
    assert_eq!(f.name, "add");
    assert_eq!(
        f.params,
        vec![
            HirParam {
                name: "a".into(),
                ty: HirType::F64
            },
            HirParam {
                name: "b".into(),
                ty: HirType::F64
            },
        ]
    );
    assert_eq!(f.ret, HirType::F64);
    assert_eq!(
        f.body,
        vec![HirStmt::Return(Some(HirExpr::BinOp(
            BinOp::Add,
            Box::new(HirExpr::Var("a".into())),
            Box::new(HirExpr::Var("b".into())),
        )))]
    );
}

#[test]
fn free_function_defaults_have_linear_hir_growth() {
    let mut small_size = 0;
    for count in [8, 17, 65] {
        let parameters = (0..count)
            .map(|index| format!("p{index}: number = {index}"))
            .collect::<Vec<_>>()
            .join(", ");
        let program = lower(&format!(
            "function many({parameters}): number {{ return p0; }}"
        ));
        assert_eq!(program.functions.len(), 1, "{count} defaults must not emit arity or omission clones");
        let debug_size = format!("{program:#?}").len();
        if count == 8 {
            small_size = debug_size;
            assert!(debug_size < 100_000, "8 defaults should keep debug HIR bounded");
        } else {
            assert!(debug_size < small_size * count * 2 / 8, "{count} defaults should grow linearly: {debug_size} vs {small_size}");
        }
    }
}

#[test]
fn free_function_requires_syntactic_omission_permission_for_undefined_types() {
    let module = thaw_parser::parse_typescript(
        "function required(value: number | undefined): void {} function main(): void { required(); }",
    )
    .unwrap();
    assert!(lower_module(&module).is_err(), "a required undefined-capable parameter remains required");
}

#[test]
fn free_function_callable_values_do_not_expand_optional_masks() {
    for count in [8, 17, 65] {
        let parameters = (0..count)
            .map(|index| format!("p{index}: number = {index}"))
            .collect::<Vec<_>>()
            .join(", ");
        let program = lower(&format!(
            "function many({parameters}): number {{ return p0; }} function main(): number {{ const value = many; return value(); }}"
        ));
        assert_eq!(program.functions.len(), 2, "{count} optional named-value slots must not emit omission clones");
        assert!(format!("{program:#?}").len() < 1_000_000, "{count} callable defaults should keep debug HIR bounded");
    }
}

#[test]
fn keeps_dynamic_call_results_live_when_used_as_member_receivers() {
    lower(
        r#"
        declare function loader(): JsValue;
        function main(): void {
            const load: JsValue = loader();
            const module = load("example");
            module.run();
        }
        "#,
    );
}

#[test]
fn lowers_top_level_bindings_and_exposes_them_to_functions() {
    let program = lower(
        r#"
            const base = 40;
            let answer: number = base + 2;
            function read(): number { return answer; }
            function main(): void { console.log(read()); }
        "#,
    );
    assert_eq!(program.globals.len(), 2);
    assert_eq!(program.globals[0].name, "base");
    assert_eq!(program.globals[0].ty, HirType::F64);
    assert!(!program.globals[0].mutable);
    assert_eq!(program.globals[1].name, "answer");
    assert_eq!(program.globals[1].ty, HirType::F64);
    assert!(program.globals[1].mutable);
    let read = program
        .functions
        .iter()
        .find(|function| function.name == "read")
        .unwrap();
    assert!(matches!(
        &read.body[0],
        HirStmt::Return(Some(HirExpr::Var(name))) if name == "answer"
    ));
}

#[test]
fn infers_top_level_initializer_calls_to_forward_functions() {
    let program = lower(
        r#"
            const answer = makeAnswer();
            function makeAnswer(): number { return 42; }
            function main(): void { console.log(answer); }
        "#,
    );
    assert_eq!(program.globals[0].ty, HirType::F64);
    assert!(matches!(program.globals[0].init, HirExpr::Call(_, _)));
}

#[test]
fn rejects_top_level_const_reassignment() {
    let module = thaw_parser::parse_typescript(
        "const answer = 42; function main(): void { answer = 43; }",
    )
    .unwrap();
    let error = lower_module(&module).unwrap_err();
    assert_eq!(error, "cannot assign to constant `answer`");
}

#[test]
fn local_const_and_using_bindings_reject_writes_without_freezing_shadowed_let() {
    let rejected = [
        "function main(): void { const value = 1; value = 2; }",
        "function main(): void { const value = 1; value++; }",
        "function main(): void { const first = 1, second = (first = 2); }",
        "function main(): void { const [first] = [1]; first = 2; }",
        "function main(): void { const [first, second = (first = 2)] = [1, undefined]; }",
        "function main(): void { const { first } = { first: 1 }; first++; }",
        "function main(): void { const value: () => number = function value(): number { value = value; return 1; }; }",
        "function main(): void { const value: () => number = function inner(): number { value = value; return 1; }; }",
        "function main(): void { const value: () => void = (): void => { value = value; }; }",
        "function main(): void { const value: () => void = (): void => { const nested = (): void => { value = value; }; nested(); }; }",
        "declare function acquire(): JsValue; function main(): void { using resource = acquire(); resource = resource; }",
        "function main(): void { for (const item of [1]) { item = 2; } }",
        "function main(): void { for (const item in { a: 1 }) { item = 'b'; } }",
    ];
    for source in rejected {
        let module = thaw_parser::parse_typescript(source).expect("parse error");
        let error = lower_module(&module).unwrap_err();
        assert!(error.contains("constant"), "{source}: {error}");
    }
    lower("function main(): number { const value = 1; { let value = 2; value++; } return value; }");
    lower("function main(): number { let value = 1; value++; return value; }");
    lower("function main(): number { const value = 1; for (let value of [2]) { value++; } return value; }");
    lower("function main(): number { const value = 1; for (let value = 0; value < 1; value++) {} return value; }");
    lower("function main(): void { function local(): number { return 1; } local = local; }");
    lower("function main(): void { const value: (value: number) => number = (value: number): number => value; }");
    lower("function main(): void { const value: () => number = (): number => { let value = 1; value++; return value; }; }");
    lower("function main(): void { const value: () => number = (): number => { const item = { value: 1 }; return item.value; }; }");
    lower("function main(): void { const value: () => number = function inner(): number { return value(); }; }");
}

#[test]
fn rejects_direct_forward_references_between_top_level_bindings() {
    let module = thaw_parser::parse_typescript(
        "const answer: number = base + 2; const base = 40; function main(): void {}",
    )
    .unwrap();
    let error = lower_module(&module).unwrap_err();
    assert!(error.contains("unknown variable `base`"), "{error}");
}

#[test]
fn callback_initializers_can_capture_the_binding_being_initialized() {
    lower(
        r#"
            declare function schedule(callback: () => void): Json;
            declare function cancel(handle: Json): void;
            function main(): void {
                const handle = schedule((): void => cancel(handle));
            }
        "#,
    );
}

#[test]
fn preserves_top_level_executable_statements_in_initializer_order() {
    let program = lower(
        r#"
            let answer = 40;
            answer = answer + 1;
            if (true) { answer++; }
            function main(): void { console.log(answer); }
        "#,
    );
    assert_eq!(program.initializers.len(), 3);
    assert!(matches!(
        program.initializers[0],
        HirInitStep::StoreGlobal(ref name, _) if name == "answer"
    ));
    assert!(matches!(
        program.initializers[1],
        HirInitStep::Statement(HirStmt::Expr(HirExpr::Assign(ref name, _)))
            if name == "answer"
    ));
    assert!(matches!(
        program.initializers[2],
        HirInitStep::Statement(HirStmt::If(_, _, _))
    ));
}

#[test]
fn top_level_calls_constrain_unannotated_function_parameters() {
    let program = lower(
        r#"
            function configure(value): void { console.log(value); }
            configure(42);
            function main(): void {}
        "#,
    );
    let configure = program
        .functions
        .iter()
        .find(|function| function.name == "configure")
        .unwrap();
    assert_eq!(configure.params[0].ty, HirType::F64);
}

#[test]
fn lowers_console_log_of_a_string_literal() {
    let program = lower(r#"function main(): void { console.log("Hello, Thaw!"); }"#);
    let f = &program.functions[0];
    assert_eq!(
        f.body,
        vec![HirStmt::Expr(HirExpr::Call(
            Box::new(HirExpr::Var("console.log".into())),
            vec![HirExpr::Lit(HirLit::Str("Hello, Thaw!".into()))],
        ))]
    );
}

#[test]
fn rejects_missing_parameter_type_annotation() {
    let module = thaw_parser::parse_typescript("function f(a) { return a; }").unwrap();
    let error = lower_module(&module).unwrap_err();
    assert!(error.contains("cannot infer parameter"));
    assert!(error.contains("at bytes"));
}

/// A `let`/`var` with no initializer at all used to be rejected
/// unconditionally, regardless of its declared type -- see `thaw-llvm`'s
/// `compiles_uninitialized_declarations_assigned_later` for the real-
/// world motivating case (csv-parse's own streaming API) and the
/// end-to-end runtime behavior. Here: an untyped bare declaration
/// lowers to a real `Json`-typed `undefined` (the same encoding a
/// normal `let x: any = undefined` already produces), and a scalar
/// annotation with no default representation (`number`) is rejected
/// with a clear, specific reason rather than the old unconditional
/// message.
#[test]
fn lowers_an_untyped_uninitialized_declaration_as_undefined_json() {
    let program = lower(
        r#"
        function main(): void {
            let record;
            record = 1;
        }
        "#,
    );
    let f = &program.functions[0];
    let HirStmt::Let(_, ty, _) = &f.body[0] else {
        panic!("expected a `let` statement, got {:?}", f.body[0]);
    };
    assert_eq!(*ty, HirType::Json);
}

#[test]
fn rejects_an_uninitialized_declaration_with_no_default_value() {
    let module = thaw_parser::parse_typescript(
        "function main(): void { let x: number; console.log(x); }",
    )
    .unwrap();
    let error = lower_module(&module).unwrap_err();
    assert!(error.contains("`x` needs an initializer"));
    assert!(error.contains("declared type `F64` has no default value"));
}

#[test]
fn infers_unannotated_parameters_from_call_sites() {
    let program = lower(
        "function identity(value) { return value; } function main(): number { return identity(42); }",
    );
    let identity = &program.functions[0];
    assert_eq!(identity.params[0].ty, HirType::F64);
    assert_eq!(identity.ret, HirType::F64);
}

#[test]
fn structured_diagnostic_resolves_file_line_and_column() {
    let source = "function identity(value) { return value; }\nfunction main(): void { identity(1); identity(\"x\"); }";
    let (module, source_map) = thaw_parser::parse_typescript_with_source_map(source).unwrap();
    let diagnostic =
        lower_module_with_source_map(&module, &source_map, "example.ts").unwrap_err();
    assert!(diagnostic.message.contains("conflicting inferred types"));
    let range = diagnostic
        .range
        .as_ref()
        .expect("diagnostic should carry a range");
    assert_eq!(range.file, "example.ts");
    assert_eq!(range.line, 2);
    assert!(range.column > 1);
    assert!(diagnostic.to_string().starts_with("example.ts:2:"));
}

#[test]
fn deduplicates_multi_argument_instantiations_and_supports_forward_references() {
    let program = lower(
        r#"
        function main(): void {
            chooseFirst(1, "a");
            chooseFirst(2, "b");
            chooseFirst("x", 3);
        }
        function chooseFirst<T, U>(first: T, second: U): T { return first; }
        "#,
    );
    assert_eq!(
        program
            .functions
            .iter()
            .filter(|function| { function.name == "chooseFirst__thaw_f64__str" })
            .count(),
        1
    );
    assert_eq!(
        program
            .functions
            .iter()
            .filter(|function| { function.name == "chooseFirst__thaw_str__f64" })
            .count(),
        1
    );
}

#[test]
fn rejects_a_wrong_call_argument_type_and_arity() {
    let wrong_type = thaw_parser::parse_typescript(
        "function square(value: number): number { return value * value; } function main(): number { return square(\"x\"); }",
    )
    .unwrap();
    let error = lower_module(&wrong_type).unwrap_err();
    assert!(error.contains("argument 1"), "unexpected error: {error}");

    let wrong_arity = thaw_parser::parse_typescript(
        "function square(value: number): number { return value * value; } function main(): number { return square(); }",
    )
    .unwrap();
    let error = lower_module(&wrong_arity).unwrap_err();
    assert!(
        error.contains("expects 1 argument"),
        "unexpected error: {error}"
    );
}

#[test]
fn lowers_typed_unary_and_extended_comparison_operators() {
    let program = lower(
        r#"function main(): void {
            console.log(-1);
            console.log(+2);
            console.log(!false);
            console.log(1 <= 2);
            console.log(2 >= 2);
            console.log("a" !== "b");
        }"#,
    );
    assert_eq!(program.functions[0].body.len(), 6);
    for statement in &program.functions[0].body {
        let HirStmt::Expr(HirExpr::Call(_, arguments)) = statement else {
            panic!("expected console call");
        };
        assert_eq!(arguments.len(), 1);
        assert!(matches!(
            arguments[0],
            HirExpr::Call(..) | HirExpr::BinOp(..) | HirExpr::Lit(..)
        ));
    }
}

#[test]
fn lowers_remainder_exponentiation_and_compound_assignments() {
    let program = lower(
        r#"function main(): void {
            let value = 10;
            console.log(value % 3);
            console.log(2 ** 3);
            value %= 4;
            value **= 3;
            console.log(value);
        }"#,
    );
    assert!(matches!(
        &program.functions[0].body[1],
        HirStmt::Expr(HirExpr::Call(_, args))
            if matches!(&args[0], HirExpr::BinOp(BinOp::Mod, _, _))
    ));
    assert!(matches!(
        &program.functions[0].body[2],
        HirStmt::Expr(HirExpr::Call(_, args))
            if matches!(&args[0], HirExpr::BinOp(BinOp::Exp, _, _))
    ));
    assert!(matches!(
        &program.functions[0].body[3],
        HirStmt::Expr(HirExpr::Assign(_, value))
            if matches!(value.as_ref(), HirExpr::BinOp(BinOp::Mod, _, _))
    ));
    assert!(matches!(
        &program.functions[0].body[4],
        HirStmt::Expr(HirExpr::Assign(_, value))
            if matches!(value.as_ref(), HirExpr::BinOp(BinOp::Exp, _, _))
    ));
}

#[test]
fn lowers_bitwise_and_shift_operators() {
    let program = lower(
        r#"function main(): void {
            let value = 5;
            console.log(value | 2);
            console.log(value ^ 1);
            console.log(value & 3);
            value <<= 2;
            value >>= 1;
            value >>>= 1;
        }"#,
    );
    let expected = [BinOp::BitOr, BinOp::BitXor, BinOp::BitAnd];
    for (statement, expected) in program.functions[0].body[1..4].iter().zip(expected) {
        assert!(matches!(
            statement,
            HirStmt::Expr(HirExpr::Call(_, args))
                if matches!(&args[0], HirExpr::BinOp(op, _, _) if *op == expected)
        ));
    }
    let expected = [BinOp::LShift, BinOp::RShift, BinOp::ZeroFillRShift];
    for (statement, expected) in program.functions[0].body[4..7].iter().zip(expected) {
        assert!(matches!(
            statement,
            HirStmt::Expr(HirExpr::Assign(_, value))
                if matches!(value.as_ref(), HirExpr::BinOp(op, _, _) if *op == expected)
        ));
    }
}

#[test]
fn lowers_bitwise_not() {
    let program = lower(
        r#"function main(): void {
            console.log(~5);
        }"#,
    );
    let main = program
        .functions
        .iter()
        .find(|function| function.name == "main")
        .unwrap();
    assert!(matches!(
        &main.body[0],
        HirStmt::Expr(HirExpr::Call(_, args))
            if matches!(&args[0], HirExpr::BinOp(BinOp::BitXor, _, rhs)
                if matches!(rhs.as_ref(), HirExpr::Lit(HirLit::F64(value)) if *value == -1.0))
    ));
}

#[test]
fn lowers_typeof_to_an_evaluating_typed_closure() {
    let program = lower(
        r#"function value(): number { return 1; }
        function callback(value: number): number { return value; }
        function main(): void {
            console.log(typeof value());
            console.log(typeof callback);
        }"#,
    );
    let main = program
        .functions
        .iter()
        .find(|function| function.name == "main")
        .unwrap();
    assert!(matches!(
        &main.body[0],
        HirStmt::Expr(HirExpr::Call(_, args))
            if matches!(&args[0], HirExpr::Call(lambda, values)
                if matches!(lambda.as_ref(), HirExpr::Lambda(_, params, HirType::Str, body)
                    if params.len() == 1
                        && matches!(body.as_ref(), HirExpr::Lit(HirLit::Str(value)) if value == "number"))
                    && matches!(values.as_slice(), [HirExpr::Call(_, _)]))
    ));
    // A function value can carry `undefined` in the raw Function pointer ABI, so `typeof` of a
    // function reference keeps a runtime check: `undefined` when the value is the absent
    // sentinel, `"function"` otherwise (previously a bare string literal).
    assert!(matches!(
        &main.body[1],
        HirStmt::Expr(HirExpr::Call(_, args))
            if matches!(&args[0], HirExpr::Call(lambda, values)
                if matches!(lambda.as_ref(), HirExpr::Lambda(_, params, HirType::Str, body)
                    if params.len() == 1
                        && format!("{body:?}").contains("Lit(Str(\"function\"))")
                        && format!("{body:?}").contains("Lit(Str(\"undefined\"))"))
                    && matches!(values.as_slice(), [HirExpr::FunctionRef(name, _, _)] if name == "callback"))
    ), "{:?}", main.body[1]);
}

#[test]
fn distinguishes_prefix_and_postfix_update_values() {
    let program = lower(
        r#"function main(): void {
            let value = 1;
            const old = value++;
            const current = ++value;
            let values = [4];
            const element = values[0]--;
        }"#,
    );
    let main = &program.functions[0];
    assert!(matches!(
        &main.body[1],
        HirStmt::Let(_, HirType::F64, HirExpr::PostfixUpdate(_, BinOp::Add))
    ));
    assert!(matches!(
        &main.body[2],
        HirStmt::Let(_, HirType::F64, HirExpr::Assign(_, value))
            if matches!(value.as_ref(), HirExpr::BinOp(BinOp::Add, _, _))
    ));
    assert!(matches!(
        &main.body[4],
        HirStmt::Let(_, HirType::F64, HirExpr::Call(_, _))
    ));
}

#[test]
fn lowers_number_field_update_expressions() {
    let program = lower(
        r#"function main(): void {
            let point = { value: 2 };
            const old = point.value++;
            const current = --point.value;
        }"#,
    );
    assert!(matches!(
        &program.functions[0].body[1],
        HirStmt::Let(_, HirType::F64, HirExpr::Call(_, _))
    ));
    // The prefix update binds the object once, reads the old value, and assigns the new value
    // inside a closure (same staging as the postfix form) instead of a bare property assignment.
    assert!(matches!(
        &program.functions[0].body[2],
        HirStmt::Let(_, HirType::F64, HirExpr::Call(_, _))
    ));
    let lowered = format!("{:?}", program.functions[0].body[2]);
    assert!(lowered.contains("PropAssign(Var(\"__thaw_update_object_"), "{lowered}");
    assert!(lowered.contains("\"value\""), "{lowered}");
}

#[test]
fn binds_compound_assignment_references_once() {
    let program = lower(
        r#"function values(): number[] { return [1]; }
        function index(): number { return 0; }
        function point(): { value: number } { return { value: 1 }; }
        function main(): void {
            values()[index()] += 2;
            point().value *= 3;
        }"#,
    );
    let main = program
        .functions
        .iter()
        .find(|function| function.name == "main")
        .unwrap();
    assert!(main
        .body
        .iter()
        .all(|statement| matches!(statement, HirStmt::Expr(HirExpr::Call(_, _)))));
}

#[test]
fn lowers_sequence_expressions_to_ordered_closures() {
    let program = lower(
        r#"function effect(value: number): number { return value; }
        function main(): void {
            const result = (effect(1), effect(2), 3);
        }"#,
    );
    let main = program
        .functions
        .iter()
        .find(|function| function.name == "main")
        .unwrap();
    assert!(matches!(
        &main.body[0],
        HirStmt::Let(_, HirType::F64, HirExpr::Call(lambda, args))
            if args.is_empty()
                && matches!(lambda.as_ref(), HirExpr::Lambda(_, _, HirType::F64, body)
                    if matches!(body.as_ref(), HirExpr::Block(statements) if statements.len() == 3))
    ));
}

#[test]
fn lowers_void_to_an_evaluating_closure() {
    let program = lower(
        r#"function effect(): number { return 1; }
        function main(): void { void effect(); }"#,
    );
    let main = program
        .functions
        .iter()
        .find(|function| function.name == "main")
        .unwrap();
    assert!(matches!(
        &main.body[0],
        HirStmt::Expr(HirExpr::Call(lambda, args))
            if args.is_empty()
                && matches!(lambda.as_ref(), HirExpr::Lambda(_, params, HirType::Void, body)
                    if params.is_empty() && matches!(body.as_ref(), HirExpr::Block(_)))
    ));
}

#[test]
fn lowers_same_type_loose_equality() {
    let program = lower(
        r#"function main(): void {
            console.log(1 == 1);
            console.log("a" != "b");
            console.log(true == false);
        }"#,
    );
    assert_eq!(program.functions[0].body.len(), 3);
    for statement in &program.functions[0].body {
        assert!(matches!(
            statement,
            HirStmt::Expr(HirExpr::Call(_, args))
                if matches!(&args[0], HirExpr::BinOp(BinOp::EqEqEq, _, _))
        ));
    }
}

#[test]
fn lowers_cross_type_primitive_loose_equality() {
    let program = lower(
        r#"function main(): void {
            console.log(1 == "1");
            console.log(false != "1");
        }"#,
    );
    assert_eq!(program.functions[0].body.len(), 2);
}

#[test]
fn lowers_process_env_access() {
    let program = lower(r#"function main(): void { console.log(process.env.STAGE); }"#);
    let f = &program.functions[0];
    assert_eq!(
        f.body,
        vec![HirStmt::Expr(HirExpr::Call(
            Box::new(HirExpr::Var("console.log".into())),
            vec![HirExpr::EnvVar("STAGE".into())],
        ))]
    );
}

#[test]
fn lowers_conditional_expressions_with_matching_native_types() {
    let program = lower(
        r#"function main(): void {
            const chooseLeft: boolean = true;
            const value: number = chooseLeft ? 1 : 2;
            console.log(value);
        }"#,
    );
    let HirStmt::Let(
        _,
        HirType::F64,
        HirExpr::Conditional(condition, consequent, alternate, HirType::F64),
    ) =
        &program.functions[0].body[1]
    else {
        panic!("expected direct conditional expression");
    };
    assert!(matches!(condition.as_ref(), HirExpr::Var(name) if name == "chooseLeft"));
    assert!(matches!(consequent.as_ref(), HirExpr::Lit(HirLit::F64(1.0))));
    assert!(matches!(alternate.as_ref(), HirExpr::Lit(HirLit::F64(2.0))));
}

#[test]
fn lowers_load_script_and_call_dynamic() {
    let program = lower(
        r#"function main(): void {
            const ok: boolean = loadScript("function add(a,b){return a+b;}");
            const args = JSON.parse("[1,2]");
            const result = callDynamic("add", args);
            console.log(Number(result));
        }"#,
    );
    let f = &program.functions[0];
    assert_eq!(
        f.body[0],
        HirStmt::Let(
            "ok".into(),
            HirType::Bool,
            HirExpr::Call(
                Box::new(HirExpr::Var("loadScript".into())),
                vec![HirExpr::Lit(HirLit::Str(
                    "function add(a,b){return a+b;}".into()
                ))],
            ),
        )
    );
    assert_eq!(
        f.body[2],
        HirStmt::Let(
            "result".into(),
            HirType::Json,
            HirExpr::Call(
                Box::new(HirExpr::Var("callDynamic".into())),
                vec![
                    HirExpr::Lit(HirLit::Str("add".into())),
                    HirExpr::Var("args".into()),
                ],
            ),
        )
    );
}

#[test]
fn rejects_extends_field_name_collision() {
    let module = thaw_parser::parse_typescript(
        r#"interface A { x: number; }
        interface B extends A { x: string; }
        function main(): void {}"#,
    )
    .unwrap();
    let err = lower_module(&module).unwrap_err();
    assert!(err.contains("incompatible"), "unexpected error: {err}");
}

#[test]
fn extends_chains_work_transitively() {
    let program = lower(
        r#"interface A { a: number; }
        interface B extends A { b: number; }
        interface C extends B { c: number; }
        function main(): void {
            const v: C = { a: 1, b: 2, c: 3 };
            console.log(v.a);
        }"#,
    );
    let c_ty = HirType::Object(vec![
        ("a".into(), HirType::F64),
        ("b".into(), HirType::F64),
        ("c".into(), HirType::F64),
    ]);
    assert!(matches!(&program.functions[0].body[0], HirStmt::Let(_, ty, _) if *ty == c_ty));
}

#[test]
fn renames_shadowed_block_locals_and_restores_outer_binding() {
    let program = lower(
        r#"function main(): void {
            let value = 1;
            if (value < 2) {
                let value = 2;
                value = value + 1;
                console.log(value);
            }
            console.log(value);
        }"#,
    );
    let body = &program.functions[0].body;
    assert!(matches!(&body[0], HirStmt::Let(name, _, _) if name == "value"));
    let HirStmt::If(_, then_body, _) = &body[1] else {
        panic!("expected lowered if");
    };
    assert!(matches!(&then_body[0], HirStmt::Let(name, _, _) if name == "value__thaw_local_0"));
    assert!(
        matches!(&then_body[1], HirStmt::Expr(HirExpr::Assign(name, _)) if name == "value__thaw_local_0")
    );
    assert!(format!("{:?}", then_body[2]).contains("value__thaw_local_0"));
    assert!(format!("{:?}", body[2]).contains("Var(\"value\")"));
}

#[test]
fn renames_a_local_that_shadows_a_top_level_function() {
    let program = lower(
        r#"declare function template(value: Json): Json;
        declare function template__thaw_local_0(value: Json): Json;
        function main(): void {
            const template = (value: number): number => value + 1;
            console.log(template(1));
        }"#,
    );
    let main = program.functions.iter().find(|function| function.name == "main").unwrap();
    assert!(matches!(&main.body[0], HirStmt::Let(name, _, _) if name == "template__thaw_local_1"));
    assert!(format!("{:?}", main.body[1]).contains("template__thaw_local_1"));
}

#[test]
fn lowers_typed_arrow_functions_and_restores_the_outer_scope() {
    let program = lower(
        r#"function main(): void {
            const value: number = 10;
            const callback = (value: number): number => value + 1;
            console.log(value);
        }"#,
    );
    let body = &program.functions[0].body;
    let HirStmt::Let(
        _,
        HirType::Function(param_types, return_type),
        HirExpr::Lambda(captures, params, lambda_return, lambda_body),
    ) = &body[1]
    else {
        panic!("expected a lowered arrow function");
    };
    assert!(captures.is_empty());
    assert_eq!(param_types, &[HirType::F64]);
    assert_eq!(return_type.as_ref(), &HirType::F64);
    assert_eq!(lambda_return, &HirType::F64);
    assert_eq!(
        params,
        &[HirParam {
            name: "value__thaw_local_0".into(),
            ty: HirType::F64
        }]
    );
    assert!(matches!(
        lambda_body.as_ref(),
        HirExpr::BinOp(_, left, _) if matches!(left.as_ref(), HirExpr::Var(name) if name == "value__thaw_local_0")
    ));
    assert!(format!("{:?}", body[2]).contains("Var(\"value\")"));
}

#[test]
fn lowers_a_typed_arrow_block_body() {
    let program = lower(
        r#"function main(): void {
            const callback = (path: string): string => { return path; };
        }"#,
    );
    let HirStmt::Let(_, _, HirExpr::Lambda(captures, params, return_type, lambda_body)) =
        &program.functions[0].body[0]
    else {
        panic!("expected a lowered arrow function");
    };
    assert!(captures.is_empty());
    assert_eq!(params[0].ty, HirType::Str);
    assert_eq!(return_type, &HirType::Str);
    assert!(matches!(
        lambda_body.as_ref(),
        HirExpr::Block(stmts)
            if matches!(&stmts[0], HirStmt::Return(Some(HirExpr::Var(name))) if name == "path")
    ));
}

#[test]
fn lowers_function_type_annotations_and_calls_through_function_values() {
    let program = lower(
        r#"function main(): void {
            const increment: (value: number) => number =
                (value: number): number => value + 1;
            console.log(increment(41));
        }"#,
    );
    let function_type = HirType::Function(vec![HirType::F64], Box::new(HirType::F64));
    assert!(matches!(
        &program.functions[0].body[0],
        HirStmt::Let(name, ty, HirExpr::Lambda(_, _, _, _))
            if name == "increment" && ty == &function_type
    ));
    assert!(format!("{:?}", program.functions[0].body[1])
        .contains("Call(Var(\"increment\"), [Lit(F64(41.0))])"));
}

#[test]
fn generates_typed_inherited_member_wrappers_and_prefers_overrides() {
    let program = lower(
        r#"class Base {
            constructor(public value: number) {}
            answer(): number { return this.value; }
            get doubled(): number { return this.value * 2; }
            set current(next: number) { this.value = next; }
        }
        class Derived extends Base {
            constructor(value: number) { super(value); }
            answer(): number { return this.value + 1; }
        }
        function main(): number {
            const value = new Derived(20);
            value.current = 21;
            console.log(value.doubled);
            return value.answer();
        }"#,
    );
    for symbol in [
        "__thaw_class_Derived_instance_getter_doubled",
        "__thaw_class_Derived_instance_setter_current",
    ] {
        assert!(program
            .functions
            .iter()
            .any(|function| function.name == symbol));
    }
    let answer = program
        .functions
        .iter()
        .filter(|function| function.name == "__thaw_class_Derived_method_answer")
        .collect::<Vec<_>>();
    assert_eq!(answer.len(), 1, "override must suppress inherited wrapper");
}

#[test]
fn generic_object_inference_matches_required_field_names() {
    let pattern = GenericTypePattern::Object(vec![("a".into(), GenericTypePattern::Variable("T".into()))]);
    let actual = HirType::Object(vec![("b".into(), HirType::Bool), ("a".into(), HirType::F64)]);
    let mut inferred = HashMap::new();
    match_generic_pattern(&pattern, &actual, &mut inferred).unwrap();
    assert_eq!(inferred.get("T"), Some(&HirType::F64));
    assert!(match_generic_pattern(&pattern, &HirType::Object(vec![("b".into(), HirType::F64)]), &mut HashMap::new()).is_err());
    lower(r#"function get<T>(value: { a: T }): T { return value.a; }
        function main(): void { const value = { a: 3, b: true }; const result: number = get(value); console.log(result); }"#);
}

#[test]
fn collection_type_arguments_keep_generic_substitutions() {
    let module = thaw_parser::parse_typescript("type M<T> = Map<string, T>; type S<T> = Set<T>; type W<T> = WeakMap<T, number>; type V<T> = WeakSet<T>;").unwrap();
    let mut substitution = HashMap::new();
    substitution.insert("T".into(), HirType::F64);
    for item in &module.body {
        let ModuleItem::Stmt(Stmt::Decl(Decl::TsTypeAlias(alias))) = item else { continue; };
        let result = resolve_ts_type_with_substitution(&alias.type_ann, &substitution, &HashMap::new(), &GenericInterfaces::new(), &mut Vec::new());
        match alias.id.sym.as_ref() {
            "M" => assert_eq!(result.unwrap(), HirType::Map(Box::new(HirType::Str), Box::new(HirType::F64))),
            "S" => assert_eq!(result.unwrap(), HirType::Set(Box::new(HirType::F64))),
            _ => {
                assert!(result.is_err(), "weak keys must remain validated");
                let key = HirType::Object(vec![("x".into(), HirType::F64)]);
                substitution.insert("T".into(), key.clone());
                let valid = resolve_ts_type_with_substitution(&alias.type_ann, &substitution, &HashMap::new(), &GenericInterfaces::new(), &mut Vec::new()).unwrap();
                assert_eq!(valid, if alias.id.sym == *"W" { HirType::WeakMap(Box::new(key), Box::new(HirType::F64)) } else { HirType::WeakSet(Box::new(key)) });
                substitution.insert("T".into(), HirType::F64);
            }
        }
    }
    lower(r#"type X<T> = Map<string, T>; function main(): void { const m: X<number> = new Map<string, number>(); console.log(m.size); }"#);
}

#[test]
fn generic_undefined_unions_preserve_canonical_optional_types() {
    let module = thaw_parser::parse_typescript("type Maybe<T> = T | undefined;").unwrap();
    let ModuleItem::Stmt(Stmt::Decl(Decl::TsTypeAlias(alias))) = &module.body[0] else {
        panic!("expected type alias");
    };
    for (substituted, expected) in [
        (
            HirType::Optional(Box::new(HirType::F64)),
            HirType::Optional(Box::new(HirType::F64)),
        ),
        (
            HirType::Nullable(Box::new(HirType::F64)),
            HirType::Nullish(Box::new(HirType::F64)),
        ),
    ] {
        let substitution = HashMap::from([("T".into(), substituted)]);
        assert_eq!(
            resolve_ts_type_with_substitution(
                &alias.type_ann,
                &substitution,
                &HashMap::new(),
                &GenericInterfaces::new(),
                &mut Vec::new(),
            )
            .unwrap(),
            expected
        );
    }
}

#[test]
fn substituted_collection_names_keep_plain_interface_shadowing() {
    let module = thaw_parser::parse_typescript("type M = Map; type S = Set; type W = WeakMap; type V = WeakSet;").unwrap();
    let shape = HirType::Object(vec![("custom".into(), HirType::Bool)]);
    let interfaces = ["Map", "Set", "WeakMap", "WeakSet"].into_iter()
        .map(|name| (name.to_string(), shape.clone())).collect::<HashMap<_, _>>();
    for item in &module.body {
        let ModuleItem::Stmt(Stmt::Decl(Decl::TsTypeAlias(alias))) = item else { continue; };
        assert_eq!(resolve_ts_type_with_substitution(&alias.type_ann, &HashMap::new(), &interfaces, &GenericInterfaces::new(), &mut Vec::new()).unwrap(), shape);
    }
}

#[test]
fn generic_key_literal_strict_equality_lowers_as_string_content_comparison() {
    let program = lower(
        r#"function sameKey<T, K extends keyof T>(value: T, first: K, second: K): boolean {
            return first === second;
        }
        function main(): void {
            const value = { name: "thaw" };
            console.log(sameKey(value, "name", "name"));
        }"#,
    );
    fn has_string_comparison(expr: &HirExpr) -> bool {
        match expr {
            HirExpr::BinOp(BinOp::EqEqEq, left, right) => {
                matches!(left.as_ref(), HirExpr::TypedClosure(HirType::Str, _))
                    && matches!(right.as_ref(), HirExpr::TypedClosure(HirType::Str, _))
            }
            HirExpr::Call(callee, args) => {
                has_string_comparison(callee) || args.iter().any(has_string_comparison)
            }
            HirExpr::Lambda(_, _, _, body) | HirExpr::TypedClosure(_, body) => {
                has_string_comparison(body)
            }
            _ => false,
        }
    }
    let specialized = program.functions.iter().find(|function| function.name.starts_with("sameKey"))
        .expect("generic key comparison must specialize");
    assert!(specialized.body.iter().any(|statement| match statement {
        HirStmt::Return(Some(value)) => has_string_comparison(value),
        _ => false,
    }), "literal-typed equality must lower both operands as string content");
}

#[test]
fn strict_optional_json_comparison_branches_on_presence_and_projects_live_owner() {
    fn walk_expr(expression: &HirExpr, visit: &mut impl FnMut(&HirExpr)) {
        visit(expression);
        match expression {
            HirExpr::Call(callee, args) => {
                walk_expr(callee, visit);
                for arg in args { walk_expr(arg, visit); }
            }
            HirExpr::Conditional(test, yes, no, _) => {
                walk_expr(test, visit);
                walk_expr(yes, visit);
                walk_expr(no, visit);
            }
            HirExpr::Lambda(_, _, _, body) | HirExpr::TypedClosure(_, body)
            | HirExpr::NonArrowFunction(body)
            | HirExpr::NonArrowThis { payload: body, .. } => walk_expr(body, visit),
            HirExpr::Block(statements) => for statement in statements {
                match statement {
                    HirStmt::Expr(value) | HirStmt::Throw(value)
                    | HirStmt::Return(Some(value)) | HirStmt::Let(_, _, value) => walk_expr(value, visit),
                    HirStmt::If(test, yes, no) => {
                        walk_expr(test, visit);
                        for nested in yes.iter().chain(no) {
                            match nested {
                                HirStmt::Expr(value) | HirStmt::Throw(value)
                                | HirStmt::Return(Some(value)) | HirStmt::Let(_, _, value) => walk_expr(value, visit),
                                _ => {}
                            }
                        }
                    }
                    _ => {}
                }
            },
            HirExpr::BinOp(_, left, right) | HirExpr::EvalThen(left, right)
            | HirExpr::Index(left, right) | HirExpr::TypedIndex(left, right, _)
            | HirExpr::JsonKey(left, right) | HirExpr::JsonIndex(left, right)
            | HirExpr::ArraySetLen(left, right, _) => {
                walk_expr(left, visit);
                walk_expr(right, visit);
            }
            HirExpr::OptionalIsNone(value, _) | HirExpr::OptionalValue(value, _)
            | HirExpr::NullableIsNone(value, _) | HirExpr::NullableValue(value, _)
            | HirExpr::NullishIsNone(value, _) | HirExpr::NullishValue(value, _)
            | HirExpr::ArrayLen(value) | HirExpr::Await(value) | HirExpr::ArrayAlloc(value, _)
            | HirExpr::JsonAsBool(value) | HirExpr::JsonAsNumber(value) | HirExpr::JsonAsString(value)
            | HirExpr::JsValueAsJson(value) => walk_expr(value, visit),
            HirExpr::ArrayLit(values) | HirExpr::DynamicCall(_, values) => {
                for value in values { walk_expr(value, visit); }
            }
            HirExpr::ObjectLit(fields) | HirExpr::JsonObjectLit(fields, _) => {
                for (_, value) in fields { walk_expr(value, visit); }
            }
            HirExpr::PropAccess(value, _, _) | HirExpr::JsonGet(value, _)
            | HirExpr::JsonAsNative(value, _) => walk_expr(value, visit),
            HirExpr::IndexAssign(array, index, value) | HirExpr::JsonSet(array, index, value, _, _) => {
                walk_expr(array, visit);
                walk_expr(index, visit);
                walk_expr(value, visit);
            }
            HirExpr::PropAssign(object, _, _, value) => {
                walk_expr(object, visit);
                walk_expr(value, visit);
            }
            _ => {}
        }
    }

    fn has_variable(expression: &HirExpr, name: &str) -> bool {
        let mut found = false;
        walk_expr(expression, &mut |node| found |= matches!(node, HirExpr::Var(value) if value == name));
        found
    }

    fn variable_count(expression: &HirExpr, name: &str) -> usize {
        let mut count = 0;
        walk_expr(expression, &mut |node| if matches!(node, HirExpr::Var(value) if value == name) {
            count += 1;
        });
        count
    }

    fn named_call(expression: &HirExpr, name: &str) -> bool {
        let mut found = false;
        walk_expr(expression, &mut |node| found |= matches!(node,
            HirExpr::Call(callee, _) if matches!(callee.as_ref(), HirExpr::Var(value) if value == name)));
        found
    }

    fn named_call_with_argument(expression: &HirExpr, name: &str, predicate: impl Fn(&HirExpr) -> bool) -> bool {
        let mut found = false;
        walk_expr(expression, &mut |node| {
            if let HirExpr::Call(callee, args) = node {
                if matches!(callee.as_ref(), HirExpr::Var(value) if value == name)
                    && args.iter().any(&predicate) { found = true; }
            }
        });
        found
    }

    fn one_operand_binding<'a>(expression: &'a HirExpr, prefix: &str)
        -> Option<(&'a HirParam, &'a HirExpr, &'a HirExpr)> {
        let HirExpr::Call(callee, args) = expression else { return None };
        let HirExpr::Lambda(_, parameters, _, body) = callee.as_ref() else { return None };
        if parameters.len() != 1 || args.len() != 1 || !parameters[0].name.starts_with(prefix) {
            return None;
        }
        Some((&parameters[0], &args[0], body))
    }

    fn map_get_optional_binding(expression: &HirExpr, receiver_name: &str) -> bool {
        let Some((receiver, receiver_source, after_receiver)) =
            one_operand_binding(expression, "__thaw_map_get_receiver_") else { return false };
        if receiver.ty != HirType::Map(Box::new(HirType::Str), Box::new(HirType::Json))
            || !matches!(receiver_source, HirExpr::Var(name) if name == receiver_name) {
            return false;
        }
        let Some((key, key_source, result)) =
            one_operand_binding(after_receiver, "__thaw_map_get_key_") else { return false };
        if key.ty != HirType::Str
            || !matches!(key_source,
                HirExpr::Lit(HirLit::Str(value)) if value == "owner")
                && !matches!(key_source,
                    HirExpr::TypedClosure(HirType::Str, value)
                        if matches!(value.as_ref(), HirExpr::Lit(HirLit::Str(value)) if value == "owner")) {
            return false;
        }
        let HirExpr::Call(callee, args) = result else { return false };
        let HirExpr::Lambda(_, parameters, return_type, body) = callee.as_ref() else { return false };
        if !parameters.is_empty() || !args.is_empty()
            || *return_type != HirType::Optional(Box::new(HirType::Json)) {
            return false;
        }
        let HirExpr::Block(statements) = body.as_ref() else { return false };
        let has_pair_condition = |condition: &HirExpr, intrinsic: &str| -> bool {
            matches!(condition,
                HirExpr::Call(callee, arguments)
                    if matches!(callee.as_ref(), HirExpr::Var(name) if name == intrinsic)
                        && matches!(arguments.as_slice(), [HirExpr::Var(receiver_alias), HirExpr::Var(key_alias)]
                            if receiver_alias == &receiver.name && key_alias == &key.name))
        };
        let has_presence_test = statements.iter().any(|statement| matches!(statement,
            HirStmt::If(condition, _, otherwise)
                if has_pair_condition(condition, "__thaw_map_str_has")
                    && otherwise.iter().any(|statement| matches!(statement,
                        HirStmt::Return(Some(HirExpr::OptionalNone(HirType::Json)))))));
        let returns_optional_value = statements.iter().any(|statement| matches!(statement,
            HirStmt::Return(Some(HirExpr::OptionalSome(value, HirType::Json)))
                if matches!(value.as_ref(), HirExpr::TypedClosure(HirType::Json, raw)
                    if matches!(raw.as_ref(), HirExpr::Call(callee, arguments)
                    if matches!(callee.as_ref(), HirExpr::Var(name) if name == "__thaw_map_str_get_ptr")
                        && matches!(arguments.as_slice(), [HirExpr::Var(receiver_alias), HirExpr::Var(key_alias)]
                            if receiver_alias == &receiver.name && key_alias == &key.name)))));
        has_presence_test && returns_optional_value
    }

    fn selected_presence<'a>(expression: &'a HirExpr, optional_name: &str)
        -> Option<(&'a HirExpr, &'a HirExpr)> {
        let HirExpr::Conditional(test, absent, present, _) = expression else { return None };
        matches!(test.as_ref(), HirExpr::OptionalIsNone(value, HirType::Json)
            if matches!(value.as_ref(), HirExpr::Var(name) if name == optional_name))
            .then_some((present.as_ref(), absent.as_ref()))
    }

    fn has_exact_optional_value(expression: &HirExpr, name: &str) -> bool {
        let mut found = false;
        walk_expr(expression, &mut |node| found |= matches!(node,
            HirExpr::OptionalValue(value, HirType::Json)
                if matches!(value.as_ref(), HirExpr::Var(alias) if alias == name)));
        found
    }

    fn has_projection_of(expression: &HirExpr, owner_name: &str) -> bool {
        named_call_with_argument(expression, "__thaw_json_from_consumed_dynamic",
            |argument| has_variable(argument, owner_name))
    }

    fn has_undefined_literal(expression: &HirExpr) -> bool {
        let mut found = false;
        walk_expr(expression, &mut |node| found |= matches!(node,
            HirExpr::Lit(HirLit::Undefined)));
        found
    }

    fn has_bool_literal(expression: &HirExpr, expected: bool) -> bool {
        let mut found = false;
        walk_expr(expression, &mut |node| found |= matches!(node,
            HirExpr::Lit(HirLit::Bool(value)) if *value == expected));
        found
    }

    fn connected_trusted_json_pair(expression: &HirExpr, optional_name: &str, owner_name: &str) -> bool {
        let mut connected = false;
        walk_expr(expression, &mut |node| {
            let HirExpr::Call(callee, _) = node else { return };
            if !matches!(callee.as_ref(), HirExpr::Var(name) if name == "callDynamicValueMixed") {
                return;
            }
            let strict = named_call_with_argument(node, "getDynamicValue", |argument|
                matches!(argument, HirExpr::Lit(HirLit::Str(name)) if name == "__thaw_strict_equal_dynamic"));
            let json_payload = named_call_with_argument(node, "__thaw_json_borrowed_handle_id",
                |argument| matches!(argument, HirExpr::Var(alias) if alias == optional_name));
            let owner_payload = named_call_with_argument(node, "__thaw_json_borrowed_handle_id",
                |argument| has_projection_of(argument, owner_name));
            connected |= strict && json_payload && owner_payload;
        });
        connected
    }

    let program = lower(
        r#"
            interface Owner { value: number; }
            function left(values: Map<string, any>, owner: Owner): boolean { return values.get("owner") === owner; }
            function right(values: Map<string, any>, owner: Owner): boolean { return owner === values.get("owner"); }
            function nullLeft(values: Map<string, any>): boolean { return values.get("owner") === null; }
            function nullRight(values: Map<string, any>): boolean { return null === values.get("owner"); }
            class NominalOwner { value: number; constructor(value: number) { this.value = value; } }
            function nominal(values: Map<string, any>, owner: NominalOwner): boolean { return values.get("owner") === owner; }
            function methodOwner(values: Map<string, any>): boolean {
                const owner = { value: 7, run(): number { return this["value"]; } };
                return values.get("owner") === owner;
            }
        "#,
    );
    for (name, left_source, right_source, expects_owner) in [
        ("left", "map", "owner", true),
        ("right", "owner", "map", true),
        ("nullLeft", "map", "null", false),
        ("nullRight", "null", "map", false),
    ] {
        let function = program.functions.iter().find(|function| function.name == name).unwrap();
        let expression = function.body.iter().find_map(|statement| match statement {
            HirStmt::Return(Some(value)) => Some(value),
            _ => None,
        }).unwrap();
        let (left, left_actual, after_left) = one_operand_binding(expression, "__thaw_strict_left_")
            .expect("original left operand is bound first");
        let (right, right_actual, comparison) = one_operand_binding(after_left, "__thaw_strict_right_")
            .expect("original right operand is bound second");
        let optional = if left.ty == HirType::Optional(Box::new(HirType::Json)) { left }
            else { right };
        assert_eq!(optional.ty, HirType::Optional(Box::new(HirType::Json)),
            "{name} keeps the exact Optional<Json> input ABI");
        let source_kind = |actual: &HirExpr, expected: &str| match expected {
            "map" => assert!(map_get_optional_binding(actual, "values"),
                "{name} binds Map.get's exact Optional<Json> receiver, key, and result route: {actual:?}"),
            "owner" => assert!(matches!(actual, HirExpr::Var(source) if source == "owner"), "{name} binds the original owner: {actual:?}"),
            "null" => assert!(matches!(actual, HirExpr::Lit(HirLit::Null)), "{name} binds the original null literal: {actual:?}"),
            _ => unreachable!(),
        };
        source_kind(left_actual, left_source);
        source_kind(right_actual, right_source);
        for (parameter, label) in [(left, left_source), (right, right_source)] {
            match label {
                "map" => assert_eq!(parameter.ty, HirType::Optional(Box::new(HirType::Json)),
                    "{name} Map.get source is bound with its exact Optional<Json> result ABI"),
                "owner" => assert_eq!(parameter.ty,
                    function.params.iter().find(|param| param.name == "owner").unwrap().ty,
                    "{name} owner binding retains the original function parameter type"),
                "null" => assert_eq!(parameter.ty, HirType::Null,
                    "{name} null operand retains its original literal type"),
                _ => unreachable!(),
            }
        }
        for label in [left_source, right_source] {
            match label {
                "map" => assert_eq!(variable_count(expression, "values"), 1,
                    "{name} evaluates its Map receiver once in the original operand slot"),
                "owner" => assert_eq!(variable_count(expression, "owner"), 1,
                    "{name} evaluates the native owner once in the original operand slot"),
                _ => {}
            }
        }
        let (present, absent) = selected_presence(comparison, &optional.name)
            .expect("selected Optional<Json> operand has its exact presence Conditional");
        assert!(has_exact_optional_value(present, &optional.name),
            "{name} extracts the present Json from the same selected operand alias");
        assert!(!has_exact_optional_value(absent, &optional.name),
            "{name} does not extract an absent Json payload");
        assert!(!named_call(absent, "__thaw_json_from_consumed_dynamic"),
            "{name} does not project an owner on the absent branch");
        let (present_left, present_left_actual, after_present_left) =
            one_operand_binding(present, "__thaw_strict_left_")
                .expect("present comparison binds its first operand");
        let (present_right, present_right_actual, present_comparison) =
            one_operand_binding(after_present_left, "__thaw_strict_right_")
                .expect("present comparison binds its second operand");
        let optional_is_left = left.ty == HirType::Optional(Box::new(HirType::Json));
        let (present_json, present_json_actual, present_other, present_other_actual) =
            if optional_is_left {
                (present_left, present_left_actual, present_right, present_right_actual)
            } else {
                (present_right, present_right_actual, present_left, present_left_actual)
            };
        assert_eq!(present_json.ty, HirType::Json,
            "{name} inner Json alias has the present payload ABI");
        assert!(matches!(present_json_actual,
            HirExpr::OptionalValue(value, HirType::Json)
                if matches!(value.as_ref(), HirExpr::Var(alias) if alias == &optional.name)),
            "{name} inner Json alias comes from the selected outer Optional<Json>");
        if expects_owner {
            assert!(has_undefined_literal(absent),
                "{name} compares absent Optional<Json> as semantic undefined");
            let owner = if left.ty == HirType::Optional(Box::new(HirType::Json)) { right } else { left };
            assert!(matches!(&owner.ty, HirType::Object(_)), "{name} keeps the fixed owner type");
            assert_eq!(present_other.ty, owner.ty,
                "{name} inner owner alias retains the outer owner type");
            assert!(matches!(present_other_actual, HirExpr::Var(alias) if alias == &owner.name),
                "{name} inner owner alias reads the selected outer owner");
            assert!(has_projection_of(present_comparison, &present_other.name),
                "{name} projects the exact inner owner alias in the selected present branch");
            assert!(connected_trusted_json_pair(
                present_comparison, &present_json.name, &present_other.name),
                "{name} connects both inner aliases to the trusted strict comparator");
        } else {
            assert!(has_bool_literal(absent, false),
                "{name} keeps absent-versus-null false");
            assert_eq!(present_other.ty, HirType::Null,
                "{name} inner null alias retains its source literal type");
            let outer_null = if optional_is_left { right } else { left };
            assert!(matches!(present_other_actual, HirExpr::Var(alias) if alias == &outer_null.name),
                "{name} inner null alias reads the exact outer null binding sourced from the original literal");
            assert!(named_call_with_argument(present_comparison, "__thaw_json_is_null", |argument|
                matches!(argument, HirExpr::Var(alias) if alias == &present_json.name)),
                "{name} compares null against the exact present Json payload");
        }
    }

    for name in ["nominal", "methodOwner"] {
        let function = program.functions.iter().find(|function| function.name == name).unwrap();
        let expression = function.body.iter().find_map(|statement| match statement {
            HirStmt::Return(Some(value)) => Some(value), _ => None,
        }).unwrap();
        let (map_operand, map_actual, after_left) =
            one_operand_binding(expression, "__thaw_strict_left_").unwrap();
        assert_eq!(map_operand.ty, HirType::Optional(Box::new(HirType::Json)));
        assert!(map_get_optional_binding(map_actual, "values"),
            "{name} uses the same genuine Map.get Optional<Json> source route");
        let (_, _, comparison) = one_operand_binding(after_left, "__thaw_strict_right_").unwrap();
        assert!(named_call(comparison, "__thaw_json_from_consumed_dynamic"),
            "declared classes and computed plain-method receivers use the live actual-value projection");
        if name == "methodOwner" {
            let (owner_binding, owner_actual, _) =
                one_operand_binding(after_left, "__thaw_strict_right_").unwrap();
            let HirExpr::Var(owner_name) = owner_actual else {
                panic!("methodOwner binds its local owner as the original right operand: {owner_actual:?}");
            };
            let HirType::Object(fields) = &owner_binding.ty else {
                panic!("methodOwner remains a fixed object: {:?}", owner_binding.ty);
            };
            let (_, method_type) = fields.iter().find(|(field, _)| field == "run")
                .expect("methodOwner retains its source method field");
            let HirType::Function(parameters, _) = method_type else {
                panic!("methodOwner run retains its function type: {method_type:?}");
            };
            let Some(HirType::NonArrowReceiver(receiver)) = parameters.first() else {
                panic!("methodOwner run retains a private receiver parameter: {parameters:?}");
            };
            let HirType::Object(receiver) = receiver.as_ref() else {
                panic!("private method receiver retains its marked object layout: {receiver:?}");
            };
            assert_eq!(receiver, &vec![
                ("value".into(), HirType::F64),
                ("__thaw_object_method_receiver".into(), HirType::Undefined),
            ], "computed this access retains the actual non-Bool receiver marker");
            assert!(!has_projection_of(comparison, owner_name),
                "methodOwner's marked receiver type remains nonprojectable");
        }
    }
}

#[test]
fn explicit_this_default_function_uses_canonical_parameter_abi() {
    let program = lower(
        r#"
        function withReceiver(this: number, value: number = this): number {
            return this + value;
        }
        "#,
    );
    let function = program.functions.iter().find(|function| function.name == "withReceiver")
        .expect("source function");
    assert_eq!(function.params.len(), 2);
    assert_eq!(function.params[0].name, "__thaw_this");
    assert_eq!(function.params[0].ty, HirType::F64);
    assert_eq!(function.params[1].ty, HirType::Optional(Box::new(HirType::F64)));
    assert!(format!("{:?}", function.body).contains("OptionalIsNone"));
    assert!(!program.functions.iter().any(|function| function.name.starts_with("withReceiver__thaw")));
}

#[test]
fn switch_cases_do_not_inherit_presence_guards_from_other_entries() {
    for source in [
        "function read(x: number | undefined, key: number): number { switch (key) { case 0: if (x === undefined) return 0; break; default: return x; } return 0; }",
        "function read(x: number | null, key: number): number { switch (key) { case 0: if (x === null) return 0; break; default: return x; } return 0; }",
        "function read(x: number | null | undefined, key: number): number { switch (key) { case 0: if (x == null) return 0; break; default: return x; } return 0; }",
        "function read(x: number | undefined, key: number): number { switch (key) { case 0: if (x === undefined) return 0; default: return x; } }",
        "function read(x: number | undefined, key: number): number { switch (key) { case 0: if (x === undefined) return 0; break; } return x; }",
        "function read(x: number | undefined, key: number): number { if (x === undefined) return 0; switch (key) { case 0: x = undefined; break; } return x; }",
    ] {
        let module = thaw_parser::parse_typescript(source).unwrap();
        assert!(lower_module(&module).is_err(), "{source}");
    }
    lower("function read(x: number | undefined, key: number): number { if (x === undefined) return 0; switch (key) { case 0: return x; default: return x; } }");
    lower("function read(x: number | undefined, key: number): number { switch (key) { case 0: if (x === undefined) return 0; return x; default: if (x === undefined) return 0; return x; } }");
}

fn find_test_normalized_adapter(expr: &HirExpr) -> Option<(&[HirParam], &HirExpr)> {
    match expr {
        HirExpr::Lambda(_, params, _, body)
            if params.iter().any(|param| param.name.starts_with("__thaw_promise_argument_")) =>
        {
            Some((params, body))
        }
        HirExpr::Lambda(_, _, _, body) | HirExpr::TypedClosure(_, body)
        | HirExpr::EvalThen(_, body) => find_test_normalized_adapter(body),
        HirExpr::Call(callee, args) => find_test_normalized_adapter(callee)
            .or_else(|| args.iter().find_map(find_test_normalized_adapter)),
        HirExpr::FfiCall(_, args) | HirExpr::DynamicCall(_, args) => {
            args.iter().find_map(find_test_normalized_adapter)
        }
        HirExpr::Block(statements) => statements.iter().find_map(find_test_normalized_adapter_in_stmt),
        _ => None,
    }
}

fn find_test_callback_alias_in_expr(expr: &HirExpr) -> Option<(&[HirParam], &HirExpr, &HirExpr)> {
    if let HirExpr::Call(callee, args) = expr {
        if matches!(callee.as_ref(), HirExpr::Var(name) if name == "__thaw_closure_alias") {
            if let [adapted, original] = args.as_slice() {
                if let Some((params, body)) = find_test_normalized_adapter(adapted) {
                    return Some((params, body, original));
                }
            }
        }
    }
    match expr {
        HirExpr::Call(callee, args) => find_test_callback_alias_in_expr(callee)
            .or_else(|| args.iter().find_map(find_test_callback_alias_in_expr)),
        HirExpr::FfiCall(_, args) | HirExpr::DynamicCall(_, args) => {
            args.iter().find_map(find_test_callback_alias_in_expr)
        }
        HirExpr::PromiseThen(source, callback, ..) => find_test_callback_alias_in_expr(source)
            .or_else(|| find_test_callback_alias_in_expr(callback)),
        HirExpr::PromiseThenBoth(source, fulfilled, rejected, ..) => {
            find_test_callback_alias_in_expr(source)
                .or_else(|| fulfilled.as_deref().and_then(find_test_callback_alias_in_expr))
                .or_else(|| find_test_callback_alias_in_expr(rejected))
        }
        HirExpr::Lambda(_, _, _, body) | HirExpr::TypedClosure(_, body)
        | HirExpr::EvalThen(_, body) => find_test_callback_alias_in_expr(body),
        HirExpr::Block(statements) => statements.iter().find_map(find_test_callback_alias_in_stmt),
        _ => None,
    }
}

fn find_test_normalized_adapter_in_stmt(statement: &HirStmt) -> Option<(&[HirParam], &HirExpr)> {
    match statement {
        HirStmt::Expr(expr) | HirStmt::Return(Some(expr)) | HirStmt::Throw(expr)
        | HirStmt::Let(_, _, expr) => find_test_normalized_adapter(expr),
        HirStmt::If(condition, yes, no) => find_test_normalized_adapter(condition)
            .or_else(|| yes.iter().find_map(find_test_normalized_adapter_in_stmt))
            .or_else(|| no.iter().find_map(find_test_normalized_adapter_in_stmt)),
        HirStmt::While(condition, body) => find_test_normalized_adapter(condition)
            .or_else(|| body.iter().find_map(find_test_normalized_adapter_in_stmt)),
        HirStmt::Try(body, _, catch, _) => body.iter().find_map(find_test_normalized_adapter_in_stmt)
            .or_else(|| catch.iter().find_map(find_test_normalized_adapter_in_stmt)),
        HirStmt::Finally(body, _) => body.iter().find_map(find_test_normalized_adapter_in_stmt),
        _ => None,
    }
}

fn find_test_callback_alias_in_stmt(statement: &HirStmt) -> Option<(&[HirParam], &HirExpr, &HirExpr)> {
    match statement {
        HirStmt::Expr(expr) | HirStmt::Return(Some(expr)) | HirStmt::Throw(expr) => {
            find_test_callback_alias_in_expr(expr)
        }
        HirStmt::Let(_, _, expr) => find_test_callback_alias_in_expr(expr),
        HirStmt::If(condition, yes, no) => find_test_callback_alias_in_expr(condition)
            .or_else(|| yes.iter().find_map(find_test_callback_alias_in_stmt))
            .or_else(|| no.iter().find_map(find_test_callback_alias_in_stmt)),
        HirStmt::While(condition, body) => find_test_callback_alias_in_expr(condition)
            .or_else(|| body.iter().find_map(find_test_callback_alias_in_stmt)),
        HirStmt::Try(body, _, catch, _) => body.iter().find_map(find_test_callback_alias_in_stmt)
            .or_else(|| catch.iter().find_map(find_test_callback_alias_in_stmt)),
        HirStmt::Finally(body, _) => body.iter().find_map(find_test_callback_alias_in_stmt),
        _ => None,
    }
}

fn has_test_json_coercion(expr: &HirExpr, expected: &HirType) -> bool {
    match expr {
        HirExpr::JsonAsString(_) => expected == &HirType::Str,
        HirExpr::JsonAsNumber(_) => expected == &HirType::F64,
        HirExpr::Call(callee, args) => has_test_json_coercion(callee, expected)
            || args.iter().any(|arg| has_test_json_coercion(arg, expected)),
        HirExpr::Block(statements) => statements.iter().any(|statement| match statement {
            HirStmt::Expr(expr) | HirStmt::Return(Some(expr)) | HirStmt::Throw(expr)
            | HirStmt::Let(_, _, expr) => has_test_json_coercion(expr, expected),
            _ => false,
        }),
        HirExpr::TypedClosure(_, body) | HirExpr::Lambda(_, _, _, body)
        | HirExpr::EvalThen(_, body) => has_test_json_coercion(body, expected),
        HirExpr::FfiCall(_, args) | HirExpr::DynamicCall(_, args) => {
            args.iter().any(|arg| has_test_json_coercion(arg, expected))
        }
        _ => false,
    }
}

fn find_test_lambda_params_in_expr(expr: &HirExpr) -> Option<&[HirParam]> {
    match expr {
        HirExpr::Lambda(_, params, _, _) => Some(params),
        HirExpr::Call(callee, args) => find_test_lambda_params_in_expr(callee)
            .or_else(|| args.iter().find_map(find_test_lambda_params_in_expr)),
        HirExpr::FfiCall(_, args) | HirExpr::DynamicCall(_, args) => {
            args.iter().find_map(find_test_lambda_params_in_expr)
        }
        HirExpr::PromiseThen(source, callback, ..) => find_test_lambda_params_in_expr(source)
            .or_else(|| find_test_lambda_params_in_expr(callback)),
        HirExpr::PromiseThenBoth(source, fulfilled, rejected, ..) => {
            find_test_lambda_params_in_expr(source)
                .or_else(|| fulfilled.as_deref().and_then(find_test_lambda_params_in_expr))
                .or_else(|| find_test_lambda_params_in_expr(rejected))
        }
        HirExpr::TypedClosure(_, value) | HirExpr::EvalThen(_, value) => {
            find_test_lambda_params_in_expr(value)
        }
        HirExpr::Block(statements) => statements.iter().find_map(|statement| match statement {
            HirStmt::Expr(expr) | HirStmt::Return(Some(expr)) | HirStmt::Throw(expr)
            | HirStmt::Let(_, _, expr) => find_test_lambda_params_in_expr(expr),
            _ => None,
        }),
        _ => None,
    }
}

#[test]
fn contextual_rest_callback_adapts_named_fixed_arity_functions() {
    let program = lower(
        r#"
        declare function accept(callback: (...values: any[]) => void): void;
        function main(): void {
            const fixed: (value: string, count: number) => void = (value, count) => {};
            accept(fixed);
        }
        "#,
    );
    let main = program.functions.iter().find(|function| function.name == "main").unwrap();
    let (params, adapter_body, original) = main.body.iter()
        .find_map(find_test_callback_alias_in_stmt)
        .expect("fixed callback is captured by a typed adapter");
    assert_eq!(params.iter().map(|param| param.ty.clone()).collect::<Vec<_>>(),
        vec![HirType::Json, HirType::Json]);
    assert!(matches!(original, HirExpr::Var(name) if name == "fixed"));
    assert!(has_test_json_coercion(adapter_body, &HirType::Str));
    assert!(has_test_json_coercion(adapter_body, &HirType::F64));
}

#[test]
fn contextual_rest_callback_preserves_zero_and_fixed_prefix_rest_shapes() {
    let zero = lower(
        r#"
        declare function accept(callback: (...values: any[]) => void): void;
        function zero(): void {}
        function main(): void { accept(zero); }
        "#,
    );
    let main = zero.functions.iter().find(|function| function.name == "main").unwrap();
    assert!(format!("{:?}", main.body).contains("FunctionRef(\"zero\""));

    let rest = lower(
        r#"
        declare function accept(callback: (...values: any[]) => void): void;
        function rest(prefix: any, ...values: any[]): void {}
        function main(): void { accept(rest); }
        "#,
    );
    let main = rest.functions.iter().find(|function| function.name == "main").unwrap();
    let (params, _) = main.body.iter()
        .find_map(find_test_normalized_adapter_in_stmt)
        .expect("rest callback adapter");
    assert_eq!(params.len(), 2);
    assert_eq!(params[0].ty, HirType::Json);
    assert_eq!(params[1].ty, HirType::Array(Box::new(HirType::Json)));
    assert!(format!("{:?}", main.body).contains("FunctionRef(\"rest\""));
}

#[test]
fn named_optional_function_identifier_is_normalized_to_callable_metadata() {
    let program = lower(r#"
        function named(value?: number): number { return value ?? 0; }
        function main(): void { const alias = named; }
    "#);
    let main = program.functions.iter().find(|function| function.name == "main").unwrap();
    assert!(main.body.iter().any(|statement| matches!(statement,
        HirStmt::Let(name, HirType::CallableFunction(params, optional, rest, ret), _)
            if name == "alias" && params == &vec![HirType::Optional(Box::new(HirType::F64))]
                && optional.contains(0) && rest.is_none() && ret.as_ref() == &HirType::F64
    )), "identifier reads retain source omission metadata: {:?}", main.body);
}

#[test]
fn named_required_undefined_parameter_stays_required_through_alias() {
    let source = thaw_parser::parse_typescript(r#"
        function required(value: number | undefined): number { return value ?? 0; }
        function main(): void { const alias = required; alias(); }
    "#).unwrap();
    let error = lower_module(&source).unwrap_err();
    assert!(error.contains("expects argument 1") && error.contains("omitted"), "unexpected error: {error}");
}

#[test]
fn dynamic_storage_object_literal_keeps_contextual_rest_callback_metadata() {
    let program = lower(
        r#"
        declare function accept(options: { callback?: (...values: string[]) => void }): JsValue;
        function main(): void {
            accept({ callback: (...values: string[]) => console.log(values.length) });
        }
        "#,
    );
    let main = program.functions.iter().find(|function| function.name == "main").unwrap();
    let hir = format!("{:?}", main.body);
    assert!(
        hir.contains("CallableFunction([], Inline(0), Some(Str), Void)"),
        "the escaped callback retains its logical rest marker through object lowering: {hir}"
    );
}

#[test]
fn contextual_rest_callbacks_keep_inline_and_named_fixed_prefixes() {
    let program = lower(
        r#"
        declare function accept(callback: (prefix: string, ...values: any[]) => void): void;
        function named(prefix: string, count: number, ...values: any[]): void {}
        function main(): void {
            accept(named);
            const namedLocal: (prefix: string, count: number, ...values: any[]) => void = named;
            accept(namedLocal);
            accept((prefix: string, ...values: any[]) => {});
        }
        "#,
    );
    let main = program.functions.iter().find(|function| function.name == "main").unwrap();
    let (named_params, _) = main.body.iter()
        .find_map(find_test_normalized_adapter_in_stmt)
        .expect("named fixed-prefix rest adapter");
    assert_eq!(named_params.iter().map(|param| param.ty.clone()).collect::<Vec<_>>(),
        vec![HirType::Str, HirType::Json, HirType::Array(Box::new(HirType::Json))]);
    assert!(format!("{:?}", main.body).contains("FunctionRef(\"named\""));
    let accept_arguments = main.body.iter().filter_map(|statement| match statement {
        HirStmt::Expr(HirExpr::FfiCall(signature, args))
            if signature.symbol == "accept" =>
        {
            args.first()
        }
        _ => None,
    }).collect::<Vec<_>>();
    assert_eq!(accept_arguments.len(), 3, "named, named-local, and inline accept calls stay top-level FFI calls");
    let (final_named_type, final_named_value) = match accept_arguments[0] {
        HirExpr::TypedClosure(ty, value) => (ty.clone(), value.as_ref()),
        _ => panic!("final named callback argument retains its contextual rest ABI"),
    };
    assert_eq!(final_named_type, HirType::CallableFunction(
        vec![HirType::Str, HirType::Json],
        HirOptionalMask::default(),
        Some(Box::new(HirType::Json)),
        Box::new(HirType::Void),
    ));
    let direct_named_params = match final_named_value {
        HirExpr::Call(callee, args)
            if matches!(callee.as_ref(), HirExpr::Lambda(_, _, _, _))
                && matches!(args.as_slice(), [HirExpr::FunctionRef(name, _, _)] if name == "named") =>
        {
            find_test_normalized_adapter(callee)
                .expect("direct named binder contains its normalized adapter")
                .0
        }
        _ => panic!("direct named function reference is passed through its callback binder"),
    };
    assert_eq!(direct_named_params.iter().map(|param| param.ty.clone()).collect::<Vec<_>>(),
        vec![HirType::Str, HirType::Json, HirType::Array(Box::new(HirType::Json))]);
    let (local_type, local_value) = match accept_arguments[1] {
        HirExpr::TypedClosure(ty, value) => (ty, value.as_ref()),
        _ => panic!("named-local callback argument retains its contextual rest ABI"),
    };
    assert_eq!(local_type, &HirType::CallableFunction(
        vec![HirType::Str, HirType::Json],
        HirOptionalMask::default(),
        Some(Box::new(HirType::Json)),
        Box::new(HirType::Void),
    ));
    assert!(matches!(local_value,
        HirExpr::Call(callee, args)
            if matches!(callee.as_ref(), HirExpr::Var(name) if name == "__thaw_closure_alias")
                && args.len() == 2
                && find_test_normalized_adapter(&args[0]).is_some()
                && matches!(&args[1], HirExpr::Var(name) if name == "namedLocal")
    ), "named-local rest adapter is aliased exactly once to the selected local binding");
    let inline_params = main.body.iter().find_map(|statement| match statement {
        HirStmt::Expr(expr) => find_test_lambda_params_in_expr(expr)
            .filter(|params| params.len() == 2 && params[0].name == "prefix"),
        _ => None,
    }).expect("inline fixed-prefix rest lambda");
    assert_eq!(inline_params[0].ty, HirType::Str);
    assert_eq!(inline_params[1].ty, HirType::Array(Box::new(HirType::Json)));
}

#[test]
fn contextual_rest_callback_preserves_optional_metadata_and_shadowed_bindings() {
    let optional = lower(
        r#"
        declare function accept(callback: (...values: any[]) => void): void;
        function main(): void {
            const optional: (value?: string) => void = (value) => {};
            accept(optional);
        }
        "#,
    );
    let main = optional.functions.iter().find(|function| function.name == "main").unwrap();
    assert!(main.body.iter().any(|statement| matches!(statement,
        HirStmt::Let(name, HirType::CallableFunction(_, mask, _, _), _)
            if name == "optional" && mask.contains(0))));
    let (params, _, original) = main.body.iter()
        .find_map(find_test_callback_alias_in_stmt)
        .expect("optional callback adapter");
    assert_eq!(params.len(), 1);
    assert_eq!(params[0].ty, HirType::Json);
    assert!(matches!(original, HirExpr::Var(name) if name == "optional"));

    let shadowed = lower(
        r#"
        declare function accept(callback: (...values: any[]) => void): void;
        function callback(value: string): void {}
        function main(): void {
            const callback: (value: string, count: number) => void = (value, count) => {};
            accept(callback);
        }
        "#,
    );
    assert!(shadowed.functions.iter().any(|function| function.name == "callback"));
    let main = shadowed.functions.iter().find(|function| function.name == "main").unwrap();
    let local_callback = main.body.iter().find_map(|statement| match statement {
        HirStmt::Let(name, HirType::Function(params, _), _) if params.len() == 2 => Some(name.as_str()),
        HirStmt::Let(name, HirType::CallableFunction(params, _, _, _), _) if params.len() == 2 => Some(name.as_str()),
        _ => None,
    }).unwrap_or_else(|| panic!("two-parameter shadowing local binding not found in HIR: {:#?}", main.body));
    let (params, _, original) = main.body.iter()
        .find_map(find_test_callback_alias_in_stmt)
        .expect("shadowed callback adapter");
    assert_eq!(params.len(), 2);
    assert_eq!(params[0].ty, HirType::Json);
    assert_eq!(params[1].ty, HirType::Json);
    assert!(matches!(original, HirExpr::Var(name) if name == local_callback));
}

#[test]
fn named_function_bind_preserves_optional_and_default_metadata() {
    let program = lower(
        r#"
        function plain(value?: number): number | undefined { return value; }
        function defaulted(value: number = 7): number { return value; }
        function main(): void {
            const later = plain.bind(null);
            const laterDefault = defaulted.bind(null);
            console.log(later(), laterDefault());
        }
        "#,
    );
    let main = program.functions.iter().find(|function| function.name == "main").unwrap();
    let hir = format!("{:?}", main.body);
    assert!(hir.contains("CallableFunction([Optional(F64)], Inline(1), None, Optional(F64)"), "{hir}");
    assert!(hir.contains("CallableFunction([Optional(F64)], Inline(1), None, F64"), "{hir}");
}

#[test]
fn promise_then_keeps_single_value_callback_validation() {
    lower(
        r#"
        function single(value: number): void {}
        function optional(value?: number): void {}
        function main(): void {
            new Promise<number>((resolve) => resolve(1)).then(single);
            new Promise<number>((resolve) => resolve(1)).then(optional);
        }
        "#,
    );
    let invalid = thaw_parser::parse_typescript(
        r#"
        function required(value: number, count: number): void {}
        function main(): void {
            new Promise<number>((resolve) => resolve(1)).then(required);
        }
        "#,
    ).unwrap();
    assert!(lower_module(&invalid).is_err(), "Promise.then supplies one value");

    for source in [
        r#"
        declare function accept(callback: (prefix: string, ...values: any[]) => number): void;
        function badPrefix(prefix: number, count: number, ...values: any[]): number { return count; }
        function main(): void { accept(badPrefix); }
        "#,
        r#"
        declare function accept(callback: (prefix: string, ...values: any[]) => number): void;
        function badReturn(prefix: string, count: number, ...values: any[]): string { return "bad"; }
        function main(): void { accept(badReturn); }
        "#,
    ] {
        let module = thaw_parser::parse_typescript(source).unwrap();
        assert!(lower_module(&module).is_err(), "accepted incompatible fixed prefix/return: {source}");
    }
}
