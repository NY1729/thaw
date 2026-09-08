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
fn rejects_direct_forward_references_between_top_level_bindings() {
    let module = thaw_parser::parse_typescript(
        "const answer: number = base + 2; const base = 40; function main(): void {}",
    )
    .unwrap();
    let error = lower_module(&module).unwrap_err();
    assert!(error.contains("unknown variable `base`"), "{error}");
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
    assert!(matches!(
        &main.body[1],
        HirStmt::Expr(HirExpr::Call(_, args))
            if matches!(&args[0], HirExpr::Lit(HirLit::Str(value)) if value == "function")
    ));
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
    assert!(matches!(
        &program.functions[0].body[2],
        HirStmt::Let(_, HirType::F64, HirExpr::PropAssign(_, _, field, _)) if field == "value"
    ));
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
        interface B extends A { x: number; }
        function main(): void {}"#,
    )
    .unwrap();
    let err = lower_module(&module).unwrap_err();
    assert!(err.contains("collides"), "unexpected error: {err}");
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
    assert!(matches!(&then_body[0], HirStmt::Let(name, _, _) if name == "value__thaw_0"));
    assert!(
        matches!(&then_body[1], HirStmt::Expr(HirExpr::Assign(name, _)) if name == "value__thaw_0")
    );
    assert!(format!("{:?}", then_body[2]).contains("value__thaw_0"));
    assert!(format!("{:?}", body[2]).contains("Var(\"value\")"));
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
            name: "value__thaw_0".into(),
            ty: HirType::F64
        }]
    );
    assert!(matches!(
        lambda_body.as_ref(),
        HirExpr::BinOp(_, left, _) if matches!(left.as_ref(), HirExpr::Var(name) if name == "value__thaw_0")
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
