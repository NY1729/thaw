#[test]
fn creates_distinct_native_instantiations_for_polymorphic_uses() {
    let program = lower(
        "function identity<T>(value: T): T { return value; } function main(): void { identity(1); identity(2); identity(\"x\"); }",
    );
    assert_eq!(
        program
            .functions
            .iter()
            .filter(|function| function.name == "identity__thaw_f64")
            .count(),
        1
    );
    assert_eq!(
        program
            .functions
            .iter()
            .filter(|function| function.name == "identity__thaw_str")
            .count(),
        1
    );
}

#[test]
fn infers_unannotated_function_return_types_through_forward_calls() {
    let program =
        lower("function first() { return second(); } function second() { return 42; }");
    assert_eq!(program.functions[0].ret, HirType::F64);
    assert_eq!(program.functions[1].ret, HirType::F64);
}

#[test]
fn infers_void_for_an_unannotated_function_without_value_returns() {
    let program = lower("function log() { console.log(1); }");
    assert_eq!(program.functions[0].ret, HirType::Void);
}

#[test]
fn infers_void_for_expression_bodied_console_log_arrow() {
    let program = lower(
        r#"async function main(): Promise<void> {
            await new Promise<void>((resolve, reject) => resolve())
                .finally(() => console.log("cleanup"));
        }"#,
    );
    let HirStmt::Expr(HirExpr::Await(inner)) = &program.functions[0].body[0] else {
        panic!("expected awaited finally chain");
    };
    let HirExpr::PromiseFinally(_, callback, HirType::Void, HirType::Void) = inner.as_ref()
    else {
        panic!("expected void finally callback");
    };
    assert!(matches!(
        callback.as_ref(),
        HirExpr::Lambda(_, _, HirType::Void, _)
    ));
}

#[test]
fn rejects_incompatible_return_types() {
    let module = thaw_parser::parse_typescript(
        "function choose(flag: boolean) { if (flag) return 1; return \"no\"; }",
    )
    .unwrap();
    let error = lower_module(&module).unwrap_err();
    assert!(
        error.contains("incompatible types"),
        "unexpected error: {error}"
    );
}

#[test]
fn lowers_boolean_logical_operators_to_short_circuit_closures() {
    let program = lower(
        r#"function main(): void {
            const a = true;
            const b = false;
            console.log(a && b);
            console.log(a || b);
        }"#,
    );
    for statement in &program.functions[0].body[2..] {
        let HirStmt::Expr(HirExpr::Call(_, arguments)) = statement else {
            panic!("expected console call");
        };
        let HirExpr::Call(callee, call_arguments) = &arguments[0] else {
            panic!("expected immediately invoked logical closure");
        };
        assert_eq!(call_arguments.len(), 1);
        assert!(matches!(
            callee.as_ref(),
            HirExpr::Lambda(_, params, HirType::Bool, _) if params.len() == 1
        ));
    }
}

#[test]
fn rejects_wrong_assignment_and_declared_return_types() {
    let assignment = thaw_parser::parse_typescript(
        "function main(): void { let value = 1; value = \"x\"; }",
    )
    .unwrap();
    assert!(lower_module(&assignment)
        .unwrap_err()
        .contains("expected F64"));

    let returned =
        thaw_parser::parse_typescript("function main(): number { return \"x\"; }").unwrap();
    assert!(lower_module(&returned)
        .unwrap_err()
        .contains("expected F64"));
}

#[test]
fn desugars_do_while_and_checks_condition_before_continue() {
    let program = lower(
        r#"function main(): void {
            let i = 0;
            do {
                i++;
                if (i < 2) continue;
                console.log(i);
            } while (i < 3);
        }"#,
    );
    let HirStmt::While(HirExpr::Lit(HirLit::Bool(true)), body) = &program.functions[0].body[1]
    else {
        panic!("expected unconditional desugared loop");
    };
    let guard_count = body
        .iter()
        .filter(|stmt| matches!(stmt, HirStmt::If(_, _, else_body) if else_body == &[HirStmt::Break]))
        .count();
    assert_eq!(guard_count, 1, "expected the ordinary tail guard");
    let HirStmt::If(_, continue_body, _) = &body[1] else {
        panic!("expected source if statement");
    };
    assert!(matches!(
        continue_body.as_slice(),
        [HirStmt::If(_, _, else_body), HirStmt::Continue]
            if else_body == &[HirStmt::Break]
    ));
}

#[test]
fn desugars_for_of_to_single_evaluation_index_loop() {
    let program = lower(
        r#"function values(): number[] { return [1, 2, 3]; }
           function main(): void {
               for (const value of values()) { console.log(value); }
           }"#,
    );
    let body = &program.functions[1].body;
    assert_eq!(body.len(), 3);
    assert!(matches!(
        &body[0],
        HirStmt::Let(_, HirType::Array(element), HirExpr::Call(_, _))
            if element.as_ref() == &HirType::F64
    ));
    let HirStmt::While(_, loop_body) = &body[2] else {
        panic!("expected indexed while loop");
    };
    assert!(matches!(
        &loop_body[0],
        HirStmt::Let(_, HirType::F64, HirExpr::TypedIndex(_, _, HirType::F64))
    ));
}

#[test]
fn desugars_for_of_assignment_to_existing_variable() {
    let program = lower(
        r#"function main(): void {
            let value = 0;
            for (value of [1, 2]) { console.log(value); }
            console.log(value);
        }"#,
    );
    let HirStmt::While(_, loop_body) = &program.functions[0].body[3] else {
        panic!("expected indexed while loop");
    };
    assert!(matches!(
        &loop_body[0],
        HirStmt::Expr(HirExpr::Assign(name, value))
            if name == "value" && matches!(value.as_ref(), HirExpr::TypedIndex(_, _, HirType::F64))
    ));
}

#[test]
fn lowers_switch_to_selected_case_state_without_switch_breaks() {
    fn contains_break(stmts: &[HirStmt]) -> bool {
        stmts.iter().any(|stmt| match stmt {
            HirStmt::Break => true,
            HirStmt::If(_, then_body, else_body) => {
                contains_break(then_body) || contains_break(else_body)
            }
            HirStmt::Try(body, _, catch_body) => {
                contains_break(body) || contains_break(catch_body)
            }
            HirStmt::While(_, _) => false,
            _ => false,
        })
    }
    let program = lower(
        r#"function main(): void {
            switch (2) {
                case 1: console.log("one"); break;
                default: console.log("default");
                case 2: console.log("two"); break;
            }
        }"#,
    );
    assert!(program.functions[0].body.len() > 4);
    assert!(!contains_break(&program.functions[0].body));
    assert!(matches!(
        &program.functions[0].body[0],
        HirStmt::Let(name, HirType::F64, HirExpr::Lit(HirLit::F64(2.0)))
            if name.starts_with("__thaw_switch_value_")
    ));
}

#[test]
fn lowers_try_catch() {
    let program = lower(
        r#"function main(): void {
            try {
                throw "boom";
            } catch (e) {
                console.log(e);
            }
        }"#,
    );
    let f = &program.functions[0];
    assert_eq!(
        f.body,
        vec![HirStmt::Try(
            vec![HirStmt::Throw(HirExpr::Lit(HirLit::Str("boom".into())))],
            "e".into(),
            vec![HirStmt::Expr(HirExpr::Call(
                Box::new(HirExpr::Var("console.log".into())),
                vec![HirExpr::Var("e".into())],
            ))],
        )]
    );
}

#[test]
fn lowers_finally_onto_normal_return_and_rethrow_paths() {
    let program = lower(
        r#"function f(): string {
            try {
                return "ok";
            } catch (e) {
                throw e;
            } finally {
                console.log("cleanup");
            }
        }
        function main(): void { console.log(f()); }"#,
    );
    let HirStmt::Try(body, _, catch_body) = &program.functions[0].body[0] else {
        panic!("expected lowered try");
    };
    assert!(matches!(body[0], HirStmt::Expr(_)));
    assert!(matches!(body[1], HirStmt::Return(_)));
    assert!(matches!(catch_body[0], HirStmt::Expr(_)));
    assert!(matches!(catch_body[1], HirStmt::Throw(_)));
    assert!(matches!(program.functions[0].body[1], HirStmt::Expr(_)));
}

#[test]
fn renames_catch_binding_that_shadows_an_outer_local() {
    let program = lower(
        r#"function main(): void {
            const error = "outer";
            try {
                throw "inner";
            } catch (error) {
                console.log(error);
            }
            console.log(error);
        }"#,
    );
    let body = &program.functions[0].body;
    let HirStmt::Try(_, catch_name, catch_body) = &body[1] else {
        panic!("expected lowered try");
    };
    assert_eq!(catch_name, "error__thaw_0");
    assert!(format!("{:?}", catch_body).contains("error__thaw_0"));
    assert!(format!("{:?}", body[2]).contains("Var(\"error\")"));
}

