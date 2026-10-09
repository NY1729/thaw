/// Non-empty and made only of `Let` statements.
fn only_lets(statements: &[HirStmt]) -> bool {
    !statements.is_empty() && statements.iter().all(|statement| matches!(statement, HirStmt::Let(..)))
}

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

/// An unannotated function whose `return` statements disagree on type
/// infers a `Union` of the distinct types seen (matching real
/// TypeScript's own inference for such a function -- it happily infers
/// `number | string`, no error), rather than rejecting the function
/// outright. See `infer_return_type` (`statements/narrowing.rs`).
#[test]
fn infers_a_union_return_type_for_disagreeing_return_statements() {
    let program = lower(
        "function choose(flag: boolean) { if (flag) return 1; return \"no\"; }
         function main(): void { console.log(choose(true)); }",
    );
    let choose = program
        .functions
        .iter()
        .find(|function| function.name == "choose")
        .expect("expected a lowered `choose` function");
    assert_eq!(choose.ret, HirType::Union(vec![HirType::F64, HirType::Str]));
}

/// Sibling of `infers_a_union_return_type_for_disagreeing_return_
/// statements`, for a class method instead of a free function -- the
/// fixed-point convergence loop (`module/pipeline.rs`) that resolves an
/// unannotated return type re-reads the signature fresh on every
/// re-lowering pass, so a method's own inferred `Union` converges the
/// same way a free function's does, with no separate handling needed.
#[test]
fn infers_a_union_return_type_for_a_class_method() {
    let program = lower(
        "class Choice { pick(flag: boolean) { if (flag) return 1; return \"no\"; } }
         function main(): void { console.log(new Choice().pick(true)); }",
    );
    let pick = program
        .functions
        .iter()
        .find(|function| function.name.contains("pick"))
        .expect("expected a lowered `Choice.pick` method");
    assert_eq!(pick.ret, HirType::Union(vec![HirType::F64, HirType::Str]));
}

/// Genuinely unresolvable mutual recursion between two unannotated
/// functions (neither has a non-recursive call site to bootstrap a
/// concrete type from) still gives the honest "cannot infer... add an
/// explicit return annotation" fallback -- confirms the convergence-
/// loop tolerance fix (above) doesn't paper over a real, permanent
/// non-convergence by looping forever or silently accepting a bogus
/// type.
#[test]
fn mutual_recursion_between_unannotated_functions_reports_a_clear_error() {
    let module = thaw_parser::parse_typescript(
        "function isEven(n: number) { if (n === 0) return true; return isOdd(n - 1); }
         function isOdd(n: number) { if (n === 0) return false; return isEven(n - 1); }",
    )
    .unwrap();
    let error = lower_module(&module).unwrap_err();
    assert!(
        error.contains("cannot infer the return type"),
        "unexpected error: {error}"
    );
}

/// Class-method sibling of the test above: `lower_class_methods`'s new
/// per-method tolerance (see `class_method_chain_with_an_out_of_order_
/// dependency_converges`, `hir_codegen/tests/classes.rs`) only skips a
/// method that fails *during the trial convergence loop*, retrying it on
/// a later pass -- a genuinely unresolvable case (mutual recursion
/// between two methods with no base case to bootstrap a concrete type
/// from) still correctly fails, not silently loops forever or accepts a
/// bogus type.
#[test]
fn mutual_recursion_between_unannotated_class_methods_reports_an_error() {
    let module = thaw_parser::parse_typescript(
        "class Bad {
             isEven(n: number) { if (n === 0) return true; return this.isOdd(n - 1); }
             isOdd(n: number) { if (n === 0) return false; return this.isEven(n - 1); }
         }
         function main(): void { console.log(new Bad().isEven(4)); }",
    )
    .unwrap();
    assert!(lower_module(&module).is_err());
}

#[test]
fn lowers_boolean_logical_operators_to_short_circuit_closures() {
    // Operands that are plain bindings have no side effects to sequence, so `&&`/`||`
    // lower straight to a short-circuiting conditional (`a && b` => `a ? b : a`).
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
        assert!(
            matches!(&arguments[0], HirExpr::Conditional(_, _, _, HirType::Bool)),
            "expected a direct short-circuit conditional: {:?}", arguments[0]
        );
    }

    // A left operand with side effects is evaluated once through an immediately invoked
    // closure (original assertion: one captured argument, Bool result).
    let program = lower(
        r#"function f(): boolean { return true; }
        function main(): void {
            console.log(f() && f());
            console.log(f() || f());
        }"#,
    );
    let main = program.functions.iter().find(|function| function.name == "main").unwrap();
    for statement in &main.body {
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
fn nullish_logical_and_preserves_falsy_left_and_lazy_right() {
    let program = lower(r#"
        function later(): number { return 9; }
        function numberAnd(value: number | undefined): number | undefined {
            return value && later();
        }
        function booleanAnd(value: boolean | null): boolean | null {
            return value && true;
        }
        function stringAnd(value: string | undefined): string | undefined {
            return value && "right";
        }
        function nullishAnd(value: number | null | undefined): number | null | undefined {
            return value && later();
        }
        function main(): void {
            numberAnd(0); numberAnd(NaN); booleanAnd(false);
            stringAnd(""); nullishAnd(null); nullishAnd(undefined);
        }
    "#);
    // Each falsy branch returns the captured left wrapper, preserving 0,
    // false, empty string, NaN, null and undefined. `later()` remains only
    // in the truthy branch of the generated IIFE.
    for function in &program.functions[1..5] {
        let lowered = format!("{:?}", function.body);
        assert!(lowered.contains("__thaw_nullish_and_left_"), "{lowered}");
        assert!(lowered.contains("Return(Some(Var(\"__thaw_nullish_and_left_"), "{lowered}");
        assert!(!lowered.contains("OptionalNone"), "{lowered}");
        assert!(!lowered.contains("NullableNone"), "{lowered}");
        assert!(!lowered.contains("NullishNull"), "{lowered}");
    }
}

#[test]
fn nullish_logical_and_supports_heterogeneous_results_without_widening_truthy_objects() {
    let program = lower(r#"
        function mixed(value: number | undefined): number | string | undefined {
            return value && "right";
        }
        function arrayAnd(value: number[] | undefined): string | undefined {
            return value && "right";
        }
        function objectAnd(value: { x: number } | null): boolean | null {
            return value && true;
        }
        function both(value: { x: number } | null | undefined): string | null | undefined {
            return value && "right";
        }
        function crossAbsence(value: number | undefined): number | null | undefined {
            return value && null;
        }
        function optionalRight(value: number[] | null, right: string | undefined): string | null | undefined {
            return value && right;
        }
        type Pair = number | string;
        function aliased(value: Pair | undefined): number | string | boolean | undefined {
            return value && true;
        }
        type Inner = number | undefined;
        function nestedLeft(value: Inner | undefined): number | undefined {
            return value && 1;
        }
        type MaybeNull = number | null;
        function nestedRight(value: number[] | undefined, right: MaybeNull | undefined): number | null | undefined {
            return value && right;
        }
    "#);
    assert!(matches!(program.functions[0].ret, HirType::Union(_)));
    assert_eq!(program.functions[1].ret, HirType::Optional(Box::new(HirType::Str)));
    assert_eq!(program.functions[2].ret, HirType::Nullable(Box::new(HirType::Bool)));
    assert_eq!(program.functions[3].ret, HirType::Nullish(Box::new(HirType::Str)));
    assert_eq!(program.functions[4].ret, HirType::Nullish(Box::new(HirType::F64)));
    assert_eq!(program.functions[5].ret, HirType::Nullish(Box::new(HirType::Str)));
    assert!(matches!(program.functions[6].ret, HirType::Union(_)));
    assert_eq!(program.functions[7].ret, HirType::Optional(Box::new(HirType::F64)));
    assert_eq!(program.functions[8].ret, HirType::Nullish(Box::new(HirType::F64)));
    let both = format!("{:?}", program.functions[3].body);
    assert!(both.contains("NullishIsUndefined"), "{both}");
    assert!(both.contains("NullishUndefined"), "{both}");
    assert!(both.contains("NullishNull"), "{both}");
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
    // The guard before `continue` leaves the loop with a depth-0 break (the innermost loop,
    // i.e. the same exit as a plain `Break`).
    assert!(matches!(
        continue_body.as_slice(),
        [HirStmt::If(_, _, else_body), HirStmt::Continue]
            if else_body == &[HirStmt::Break] || else_body == &[HirStmt::BreakDepth(0)]
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
            HirStmt::Try(body, _, catch_body, _) => {
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
    let text = format!("{:?}", f.body);
    assert!(text.contains("__thaw_clear_pending_exception_provenance"), "{text}");
    assert!(text.contains("__thaw_set_pending_exception_tag"), "{text}");
    assert!(text.contains("I64(4)"), "{text}");
    assert!(text.contains("@@thaw_published_exception_text"), "{text}");
    assert!(!text.contains("@@thaw_trusted_exception_text"), "{text}");
    assert!(text.contains("boom"), "{text}");
    assert!(text.contains("console.log"), "{text}");
}

#[test]
fn finally_separates_text_only_throws_from_fresh_published_tuples() {
    let program = lower(r#"
        function plain(input: any): void {
            try { throw "outer"; }
            catch (error) { const { value } = input; }
            finally { console.log("cleanup"); }
        }
        function fresh(): void {
            try { throw "outer"; }
            catch (error) { throw 17; }
            finally { console.log("cleanup"); }
        }
    "#);
    let plain = program.functions.iter().find(|function| function.name == "plain").unwrap();
    let plain_text = format!("{:?}", plain.body);
    assert!(plain_text.contains("@@thaw_trusted_exception_text"), "{plain_text}");
    assert!(plain_text.contains("@@thaw_published_exception_text"), "{plain_text}");
    assert!(!plain_text.contains("@@thaw_rethrow_pending_exception"), "{plain_text}");

    let fresh = program.functions.iter().find(|function| function.name == "fresh").unwrap();
    let fresh_text = format!("{:?}", fresh.body);
    assert!(fresh_text.contains("@@thaw_rethrow_pending_exception"), "{fresh_text}");
    assert!(fresh_text.contains("__thaw_pending_exception_native"), "{fresh_text}");
}

#[test]
fn throw_union_publishes_only_active_member_and_uses_safe_diagnostics() {
    let program = lower(r#"
        function main(reason: number | string | { value: number }): void {
            try { throw reason; } catch (error) { console.log(error); }
        }
    "#);
    let text = format!("{:?}", program.functions[0].body);
    assert!(text.contains("UnionTag"), "{text}");
    assert!(text.contains("UnionValue"), "{text}");
    assert!(text.contains("__thaw_set_pending_exception_f64"), "{text}");
    assert!(text.contains("__thaw_set_pending_exception_object"), "{text}");
    assert!(text.contains("[exception value]"), "{text}");
    assert!(!text.contains("__thaw_js_handle_to_string"), "{text}");
}

#[test]
fn promise_reject_uses_the_same_json_exception_owner_without_coercion() {
    let program = lower(r#"
        function main(reason: any): void { Promise.reject(reason); }
    "#);
    let text = format!("{:?}", program.functions[0].body);
    assert!(text.contains("__thaw_clear_pending_exception_provenance"), "{text}");
    assert!(text.contains("__thaw_set_pending_exception_json"), "{text}");
    assert!(text.contains("[exception value]"), "{text}");
    assert!(!text.contains("__thaw_js_handle_to_string"), "{text}");
}

#[test]
fn throw_borrowed_jsvalue_uses_exact_capture_instead_of_placeholder_json() {
    let program = lower(r#"
        declare function reason(): JsValue;
        function main(): void { try { throw reason(); } catch (error) { console.log(error); } }
    "#);
    let text = format!("{:?}", program.functions[0].body);
    assert!(text.contains("__thaw_capture_exception_js_value"), "{text}");
    assert!(text.contains("__thaw_set_pending_exception_json"), "{text}");
    assert!(!text.contains("JsValueAsJson"), "{text}");
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
    let HirStmt::Try(body, _, catch_body, _) = &program.functions[0].body[1] else {
        panic!("expected lowered try");
    };
    assert!(matches!(body.as_slice(), [HirStmt::Let(..), HirStmt::Finally(cleanup, 1), HirStmt::Return(Some(HirExpr::Var(_)))]
        if matches!(cleanup.as_slice(), [HirStmt::Expr(_)])), "{body:?}");
    let [HirStmt::Try(guarded, _, failure, _)] = catch_body.as_slice() else {
        panic!("expected one synthetic source-catch guard: {catch_body:?}");
    };
    assert!(matches!(guarded.as_slice(),
        [.., HirStmt::Try(captured, _, failed, _), HirStmt::Finally(_, 1), HirStmt::Finally(rethrow, 1)]
            if only_lets(captured)
                && matches!(failed.as_slice(), [HirStmt::Finally(_, 1), HirStmt::Finally(throw, 1)]
                    if matches!(throw.as_slice(), [HirStmt::Throw(_)]))
                && matches!(rethrow.as_slice(), [HirStmt::Throw(_)])), "{guarded:?}");
    // A caught exception publishes its provenance through a run of Lets before the finalizer.
    assert!(matches!(failure.as_slice(), [lets @ .., HirStmt::Finally(_, 0), HirStmt::Throw(_)] if only_lets(lets)), "{failure:?}");
    assert!(matches!(program.functions[0].body[2], HirStmt::Expr(_)));
}

#[test]
fn finally_snapshots_return_and_throw_values_before_mutation() {
    let program = lower(r#"
        function returned(): number {
            let value = 1;
            try { return value; } finally { value = 2; }
        }
        function thrown(): void {
            let value = "before";
            try { throw "trigger"; }
            catch (error) { throw value; }
            finally { value = "after"; }
        }
    "#);
    let returned = program.functions.iter().find(|function| function.name == "returned").unwrap();
    let thrown = program.functions.iter().find(|function| function.name == "thrown").unwrap();
    let HirStmt::Try(return_body, _, _, _) = &returned.body[2] else {
        panic!("expected return try");
    };
    assert!(matches!(return_body.as_slice(),
        [HirStmt::Let(_, _, HirExpr::Var(_)), HirStmt::Finally(_, 1), HirStmt::Return(Some(HirExpr::Var(_)))]),
        "{return_body:?}");
    let HirStmt::Try(_, _, throw_body, _) = &thrown.body[2] else {
        panic!("expected throw try");
    };
    assert!(matches!(throw_body.as_slice(), [HirStmt::Try(guarded, _, failure, _)]
        if matches!(guarded.as_slice(),
            [.., HirStmt::Try(captured, _, failed, _), HirStmt::Finally(_, 1), HirStmt::Finally(rethrow, 1)]
                if captured.len() == 8 && captured.iter().all(|stmt| matches!(stmt, HirStmt::Let(..)))
                    && matches!(failed.as_slice(), [HirStmt::Finally(_, 1), HirStmt::Finally(throw, 1)]
                        if matches!(throw.as_slice(), [HirStmt::Throw(_)]))
                    && matches!(rethrow.as_slice(), [HirStmt::Throw(HirExpr::Call(callee, args))]
                        if matches!(callee.as_ref(), HirExpr::Var(name)
                            if name == "@@thaw_rethrow_pending_exception") && args.len() == 10))
            && matches!(failure.as_slice(), [HirStmt::Let(..), HirStmt::Finally(_, 0), HirStmt::Throw(_)])),
        "{throw_body:?}");
}

#[test]
fn finally_runs_when_catch_exit_expression_itself_throws() {
    let program = lower(r#"
        function fail(): string { throw "expression failed"; }
        function choose(): string {
            try { throw "first"; }
            catch (error) { return fail(); }
            finally { console.log("cleanup"); }
        }
    "#);
    let choose = program.functions.iter().find(|function| function.name == "choose").unwrap();
    let HirStmt::Try(_, _, catch_body, _) = &choose.body[1] else {
        panic!("expected try");
    };
    assert!(matches!(catch_body.as_slice(), [HirStmt::Try(guarded, _, failure, _)]
        if matches!(guarded.as_slice(),
            [.., HirStmt::Try(captured, _, failed, _), HirStmt::Finally(_, 1), HirStmt::Return(Some(HirExpr::Var(_)))]
                if matches!(captured.as_slice(), [HirStmt::Let(_, _, HirExpr::Call(..))])
                    && matches!(failed.as_slice(), [HirStmt::Finally(_, 1), HirStmt::Finally(throw, 1)]
                        if matches!(throw.as_slice(), [HirStmt::Throw(_)])))
            && matches!(failure.as_slice(), [HirStmt::Let(..), HirStmt::Finally(_, 0), HirStmt::Throw(_)])),
        "{catch_body:?}");
}

#[test]
fn finally_preserves_pending_throw_tuple_after_handled_inner_throw() {
    let program = lower(r#"
        function keep(): void {
            try { throw "outer"; }
            catch (error) { throw 17; }
            finally { try { throw "inner"; } catch (ignored) {} }
        }
    "#);
    let keep = program.functions.iter().find(|function| function.name == "keep").unwrap();
    let HirStmt::Try(_, _, catch_body, _) = &keep.body[1] else {
        panic!("expected try");
    };
    let [HirStmt::Try(guarded, _, _, _)] = catch_body.as_slice() else {
        panic!("expected synthetic source-catch guard: {catch_body:?}");
    };
    let Some(HirStmt::Finally(rethrow, 1)) = guarded.last() else {
        panic!("expected catch-exit rethrow after finalizer: {guarded:?}");
    };
    let [HirStmt::Throw(HirExpr::Call(callee, args))] = rethrow.as_slice() else {
        panic!("expected pending-tuple rethrow: {rethrow:?}");
    };
    assert!(matches!(callee.as_ref(), HirExpr::Var(name)
        if name == "@@thaw_rethrow_pending_exception"));
    assert_eq!(args.len(), 10);
}

#[test]
fn abrupt_finalizer_precedes_captured_return() {
    let program = lower(r#"
        function chosen(): number {
            let value = 1;
            try { return value; } finally { value = 2; return 3; }
        }
    "#);
    let chosen = program.functions.iter().find(|function| function.name == "chosen").unwrap();
    let HirStmt::Try(body, _, _, _) = &chosen.body[2] else {
        panic!("expected try");
    };
    assert!(matches!(body.first(), Some(HirStmt::Let(_, _, HirExpr::Var(_)))), "{body:?}");
    assert!(matches!(body.get(1), Some(HirStmt::Finally(finalizer, 1))
        if matches!(finalizer.as_slice(), [HirStmt::Expr(_), HirStmt::Return(Some(HirExpr::Lit(HirLit::F64(_))))])),
        "{body:?}");
    assert!(matches!(body.last(), Some(HirStmt::Return(Some(HirExpr::Var(_))))), "{body:?}");
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
    let HirStmt::Try(_, catch_name, catch_body, _) = &body[2] else {
        panic!("expected lowered try");
    };
    assert!(catch_name.starts_with("error__thaw_local_"), "{catch_name}");
    assert!(format!("{:?}", catch_body).contains(catch_name.as_str()));
    assert!(format!("{:?}", body[3]).contains("Var(\"error\")"));
}

#[test]
fn explicit_null_throw_uses_distinct_exception_reason_tag() {
    let program = lower(r#"function main(): void { try { throw null; } catch (error) { console.log(error); } }"#);
    let text = format!("{:?}", program.functions[0].body);
    assert!(text.contains("__thaw_set_pending_exception_tag"), "{text}");
    assert!(text.contains("I64(6)"), "{text}");
}

#[test]
fn native_promise_throw_publishes_its_typed_identity_without_string_coercion() {
    let program = lower(r#"
        function rejectPromise(value: Promise<number>): void {
            try { throw value; } catch (error) { console.log(error); }
        }
    "#);
    let text = format!("{:?}", program.functions.iter()
        .find(|function| function.name == "rejectPromise").unwrap().body);
    assert!(text.contains("__thaw_set_pending_exception_native"), "{text}");
    assert!(text.contains("I64(29)"), "{text}");
    assert!(text.contains("Promise"), "{text}");
}

#[test]
fn native_exception_tags_cover_native_categories_and_exact_layouts() {
    let cases = [
        (HirType::Symbol, 20),
        (HirType::Array(Box::new(HirType::F64)), 21),
        (HirType::Bytes, 22),
        (HirType::Map(Box::new(HirType::Str), Box::new(HirType::F64)), 23),
        (HirType::WeakMap(Box::new(HirType::Str), Box::new(HirType::F64)), 24),
        (HirType::Set(Box::new(HirType::F64)), 25),
        (HirType::WeakSet(Box::new(HirType::F64)), 26),
        (HirType::Tuple(vec![HirType::F64, HirType::Str]), 27),
        (HirType::FunctionWithThis(Box::new(HirType::F64), Box::new(HirType::Str)), 28),
    ];
    for (ty, expected_tag) in cases {
        assert_eq!(crate::native_exception_tag(&ty), Some(expected_tag));
        assert!(crate::native_exception_layout_token(&ty).is_some());
    }
    assert_ne!(
        crate::native_exception_layout_token(&HirType::Array(Box::new(HirType::F64))),
        crate::native_exception_layout_token(&HirType::Array(Box::new(HirType::Str))),
    );
}

#[test]
fn promise_catch_direct_rethrow_uses_binding_snapshot_marker() {
    let program = lower(r#"function main(): void { Promise.reject<number>(1).catch(error => { throw error; }); }"#);
    let text = format!("{:?}", program);
    assert!(text.contains("@@thaw_rethrow_pending_exception"), "{text}");
    assert!(text.contains("@@thaw_promise_rejection:"), "{text}");
    assert!(text.contains("@@thaw_promise_rejection:") && text.contains(":native"), "{text}");
}

#[test]
fn inferred_return_includes_undefined_for_fallthrough_and_bare_return() {
    let program = lower(r#"
        function fallthrough(flag: boolean) { if (flag) return 1; }
        function bare(flag: boolean) { if (flag) return 2; return; }
        function shadowed(undefined: number, flag: boolean) { if (flag) return 4; }
        function switchBreak(mode: number) {
            switch (mode) { case 1: break; return 5; default: return 6; }
        }
        function doBreak(flag: boolean) {
            do { if (flag) return 7; break; } while (true);
        }
        function finalizer(flag: boolean) {
            try {} finally { if (flag) return 8; }
        }
        class Choice { pick(flag: boolean) { if (flag) return 3; } }
    "#);
    let expected = HirType::Union(vec![HirType::F64, HirType::Undefined]);
    for name in ["fallthrough", "bare", "shadowed", "switchBreak", "doBreak", "finalizer"] {
        let function = program.functions.iter().find(|function| function.name == name).unwrap();
        assert_eq!(function.ret, expected, "{name}");
    }
    let method = program.functions.iter().find(|function| function.name.contains("pick")).unwrap();
    assert_eq!(method.ret, expected);
}

#[test]
fn inferred_return_distinguishes_breakable_and_unbreakable_infinite_loops() {
    let program = lower(r#"
        function breakable(flag: boolean) {
            if (flag) return 1;
            while (true) { if (flag) break; }
        }
        function unbreakable(flag: boolean) {
            if (flag) return 1;
            while (true) { return 2; }
        }
        function nestedSwitch(flag: boolean) {
            if (flag) return 1;
            while (true) { switch (1) { default: break; } }
        }
    "#);
    let breakable = program.functions.iter().find(|function| function.name == "breakable").unwrap();
    assert_eq!(breakable.ret, HirType::Union(vec![HirType::F64, HirType::Undefined]));
    for name in ["unbreakable", "nestedSwitch"] {
        let function = program.functions.iter().find(|function| function.name == name).unwrap();
        assert_eq!(function.ret, HirType::F64, "{name}");
    }
}

#[test]
fn inferred_block_arrow_includes_undefined_without_absorbing_nested_returns() {
    let program = lower(r#"
        function main(): void {
            const callback = (flag: boolean) => {
                const nested = () => "text";
                if (flag) return 1;
                return;
            };
        }
    "#);
    let main = program.functions.iter().find(|function| function.name == "main").unwrap();
    let HirStmt::Let(_, _, HirExpr::Lambda(_, _, ret, _)) = &main.body[0] else {
        panic!("expected inferred block arrow");
    };
    assert_eq!(ret, &HirType::Union(vec![HirType::F64, HirType::Undefined]));
}

#[test]
fn nested_exit_finalizer_suspends_both_exited_catches() {
    fn contains_skip(stmts: &[HirStmt], expected: usize) -> bool {
        stmts.iter().any(|stmt| match stmt {
            HirStmt::Finally(_, count) if *count == expected => true,
            HirStmt::Finally(body, _) | HirStmt::While(_, body) => {
                contains_skip(body, expected)
            }
            HirStmt::If(_, yes, no) | HirStmt::Try(yes, _, no, _) => {
                contains_skip(yes, expected) || contains_skip(no, expected)
            }
            _ => false,
        })
    }
    let program = lower(r#"
        function value(): number {
            try {
                try { return 1; }
                catch (inner) { return 2; }
            } catch (outer) { return 3; }
            finally { throw "override"; }
        }
    "#);
    let value = program.functions.iter().find(|function| function.name == "value").unwrap();
    assert!(contains_skip(&value.body, 2), "{:#?}", value.body);
}

fn thaw_rethrow_cleanup_match_source_catch_pair(
    pair: &[HirStmt],
) -> Option<(&str, &[HirStmt])> {
    let [
        HirStmt::Let(
            prelude_name,
            prelude_type,
            HirExpr::UnionInject(seed, 4, prelude_members),
        ),
        HirStmt::Try(_, catch_name, catch_body, _),
    ] = pair else {
        return None;
    };
    let carrier = crate::caught_exception_carrier_type();
    let HirType::Union(members) = &carrier else {
        panic!("caught-exception carrier must be a union");
    };
    (prelude_type == &carrier
        && prelude_members == members
        && seed.as_ref() == &HirExpr::Lit(HirLit::Undefined)
        && catch_name == prelude_name)
        .then_some((catch_name.as_str(), catch_body))
}

fn thaw_rethrow_cleanup_collect_source_catches(
    statements: &[HirStmt],
) -> Vec<(&str, &[HirStmt])> {
    fn visit<'a>(
        statements: &'a [HirStmt],
        catches: &mut Vec<(&'a str, &'a [HirStmt])>,
    ) {
        for pair in statements.windows(2) {
            if let Some(catch) = thaw_rethrow_cleanup_match_source_catch_pair(pair) {
                catches.push(catch);
            }
        }
        for statement in statements {
            match statement {
                HirStmt::Try(body, _, catch_body, _) => {
                    visit(body, catches);
                    visit(catch_body, catches);
                }
                HirStmt::If(_, yes, no) => {
                    visit(yes, catches);
                    visit(no, catches);
                }
                HirStmt::While(_, body) | HirStmt::Finally(body, _) => visit(body, catches),
                _ => {}
            }
        }
    }

    let mut catches = Vec::new();
    visit(statements, &mut catches);
    catches
}

fn thaw_rethrow_cleanup_find_direct_source_catch(
    statements: &[HirStmt],
) -> (&str, &[HirStmt]) {
    let catches = statements
        .windows(2)
        .filter_map(thaw_rethrow_cleanup_match_source_catch_pair)
        .collect::<Vec<_>>();
    assert_eq!(catches.len(), 1, "expected one direct source catch pair in this scope");
    catches[0]
}

fn thaw_rethrow_cleanup_find_publisher<'a>(
    statements: &'a [HirStmt],
    expected_prefix: &[HirStmt],
    source_value: &HirExpr,
    source_type: &HirType,
) -> (&'a str, &'a [HirStmt]) {
    let mut selected = None;
    for (index, statement) in statements.iter().enumerate() {
        let HirStmt::Let(name, ty, initializer) = statement else {
            continue;
        };
        if name.starts_with("__thaw_thrown_value_")
            && ty == source_type
            && initializer == source_value
        {
            assert!(selected.is_none(), "more than one matching publication Let");
            selected = Some((index, name.as_str()));
        }
    }
    let (index, name) = selected.expect("expected the exact thrown-value Let");
    assert_eq!(index, expected_prefix.len(), "unexpected statements before publisher Let: {statements:#?}");
    assert_eq!(&statements[..index], expected_prefix, "publisher prefix was not fully accounted for");
    let tail = &statements[index..];
    assert!(matches!(tail.get(1), Some(HirStmt::Expr(HirExpr::Call(callee, args)))
        if callee.as_ref() == &HirExpr::Var("__thaw_clear_pending_exception_provenance".into())
            && args.is_empty()), "expected stale-provenance clear after the one-evaluation Let: {tail:?}");
    let Some(HirStmt::Throw(HirExpr::Call(callee, args))) = tail.last() else {
        panic!("expected published-display throw last: {tail:?}");
    };
    assert_eq!(callee.as_ref(), &HirExpr::Var("@@thaw_published_exception_text".into()));
    assert_eq!(args.len(), 1, "published display marker takes one display argument");
    (name, tail)
}

fn thaw_rethrow_cleanup_bool_count(value: bool) -> usize {
    if value { 1 } else { 0 }
}

fn thaw_rethrow_cleanup_merge_effect_counts(total: &mut [usize; 6], add: [usize; 6]) {
    for (slot, amount) in total.iter_mut().zip(add) {
        *slot += amount;
    }
}

/// Bounded visitor for the publisher's scalar display and the current native-object
/// registration prelude. It fails closed on any HIR form outside those emitted shapes.
/// Count slots: target calls, target-symbol references, outer reads, outer writes,
/// outer captures, and outer-name declarations.
fn thaw_rethrow_cleanup_scan_effect_expr(
    expression: &HirExpr,
    target: &str,
    outer: &str,
) -> [usize; 6] {
    match expression {
        HirExpr::Lit(_) => [0; 6],
        HirExpr::Var(name) => {
            let mut counts = [0; 6];
            counts[1] += thaw_rethrow_cleanup_bool_count(name == target);
            counts[2] += thaw_rethrow_cleanup_bool_count(name == outer);
            counts
        }
        HirExpr::Call(callee, args) => {
            let mut counts = [0; 6];
            if let HirExpr::Var(name) = callee.as_ref() {
                if name == target {
                    assert!(args.is_empty(), "the effectful control input must be zero-argument");
                    counts[0] += 1;
                }
            }
            thaw_rethrow_cleanup_merge_effect_counts(
                &mut counts,
                thaw_rethrow_cleanup_scan_effect_expr(callee, target, outer),
            );
            for argument in args {
                thaw_rethrow_cleanup_merge_effect_counts(
                    &mut counts,
                    thaw_rethrow_cleanup_scan_effect_expr(argument, target, outer),
                );
            }
            counts
        }
        HirExpr::Lambda(captures, _, _, body) => {
            let mut counts = [0; 6];
            for capture in captures {
                counts[1] += thaw_rethrow_cleanup_bool_count(capture.name == target);
                counts[4] += thaw_rethrow_cleanup_bool_count(capture.name == outer);
            }
            thaw_rethrow_cleanup_merge_effect_counts(
                &mut counts,
                thaw_rethrow_cleanup_scan_effect_expr(body, target, outer),
            );
            counts
        }
        HirExpr::BinOp(_, left, right)
        | HirExpr::EvalThen(left, right)
        | HirExpr::Index(left, right)
        | HirExpr::TypedIndex(left, right, _)
        | HirExpr::JsonKey(left, right)
        | HirExpr::JsonIndex(left, right)
        | HirExpr::JsonDelete(left, right)
        | HirExpr::DynamicPropAccess(left, right, _, _) => {
            let mut counts = thaw_rethrow_cleanup_scan_effect_expr(left, target, outer);
            thaw_rethrow_cleanup_merge_effect_counts(
                &mut counts,
                thaw_rethrow_cleanup_scan_effect_expr(right, target, outer),
            );
            counts
        }
        HirExpr::Conditional(test, yes, no, _) => {
            let mut counts = thaw_rethrow_cleanup_scan_effect_expr(test, target, outer);
            thaw_rethrow_cleanup_merge_effect_counts(
                &mut counts,
                thaw_rethrow_cleanup_scan_effect_expr(yes, target, outer),
            );
            thaw_rethrow_cleanup_merge_effect_counts(
                &mut counts,
                thaw_rethrow_cleanup_scan_effect_expr(no, target, outer),
            );
            counts
        }
        HirExpr::UnionInject(value, _, _)
        | HirExpr::UnionTag(value, _)
        | HirExpr::UnionValue(value, _, _)
        | HirExpr::ArrayLen(value)
        | HirExpr::JsonGet(value, _)
        | HirExpr::PropAccess(value, _, _)
        | HirExpr::JsonAsNumber(value)
        | HirExpr::JsonAsString(value)
        | HirExpr::JsonAsBool(value)
        | HirExpr::JsValueAsJson(value)
        | HirExpr::OptionalIsNone(value, _)
        | HirExpr::OptionalValue(value, _)
        | HirExpr::NullableIsNone(value, _)
        | HirExpr::NullableValue(value, _)
        | HirExpr::NullishIsNull(value, _)
        | HirExpr::NullishIsUndefined(value, _)
        | HirExpr::NullishIsNone(value, _)
        | HirExpr::NullishValue(value, _) => {
            thaw_rethrow_cleanup_scan_effect_expr(value, target, outer)
        }
        HirExpr::PropAssign(object, _, _, value) => {
            let mut counts = thaw_rethrow_cleanup_scan_effect_expr(object, target, outer);
            thaw_rethrow_cleanup_merge_effect_counts(
                &mut counts,
                thaw_rethrow_cleanup_scan_effect_expr(value, target, outer),
            );
            counts
        }
        HirExpr::JsonSet(object, key, value, _, _)
        | HirExpr::JsonIndexSet(object, key, value)
        | HirExpr::IndexAssign(object, key, value) => {
            let mut counts = thaw_rethrow_cleanup_scan_effect_expr(object, target, outer);
            thaw_rethrow_cleanup_merge_effect_counts(
                &mut counts,
                thaw_rethrow_cleanup_scan_effect_expr(key, target, outer),
            );
            thaw_rethrow_cleanup_merge_effect_counts(
                &mut counts,
                thaw_rethrow_cleanup_scan_effect_expr(value, target, outer),
            );
            counts
        }
        HirExpr::Assign(name, value) => {
            let mut counts = [0; 6];
            counts[1] += thaw_rethrow_cleanup_bool_count(name == target);
            counts[3] += thaw_rethrow_cleanup_bool_count(name == outer);
            thaw_rethrow_cleanup_merge_effect_counts(
                &mut counts,
                thaw_rethrow_cleanup_scan_effect_expr(value, target, outer),
            );
            counts
        }
        HirExpr::PostfixUpdate(name, _) => {
            let mut counts = [0; 6];
            counts[1] += thaw_rethrow_cleanup_bool_count(name == target);
            counts[2] += thaw_rethrow_cleanup_bool_count(name == outer);
            counts[3] += thaw_rethrow_cleanup_bool_count(name == outer);
            counts
        }
        HirExpr::Block(statements) => {
            thaw_rethrow_cleanup_scan_effect_stmts(statements, target, outer)
        }
        HirExpr::ArrayLit(elements)
        | HirExpr::ArrayConcat(elements, _)
        | HirExpr::DynamicCall(_, elements)
        | HirExpr::FfiCall(_, elements) => {
            let mut counts = [0; 6];
            for element in elements {
                thaw_rethrow_cleanup_merge_effect_counts(
                    &mut counts,
                    thaw_rethrow_cleanup_scan_effect_expr(element, target, outer),
                );
            }
            counts
        }
        HirExpr::ObjectLit(fields) | HirExpr::JsonObjectLit(fields, _) => {
            let mut counts = [0; 6];
            for (_, value) in fields {
                thaw_rethrow_cleanup_merge_effect_counts(
                    &mut counts,
                    thaw_rethrow_cleanup_scan_effect_expr(value, target, outer),
                );
            }
            counts
        }
        HirExpr::TypedClosure(_, value) | HirExpr::NonArrowFunction(value) => {
            thaw_rethrow_cleanup_scan_effect_expr(value, target, outer)
        }
        HirExpr::OptionalSome(value, _)
        | HirExpr::NullableSome(value, _)
        | HirExpr::NullishSome(value, _)
        | HirExpr::ArrayAlloc(value, _)
        | HirExpr::Await(value)
        | HirExpr::AwaitPromise(value, _) => {
            thaw_rethrow_cleanup_scan_effect_expr(value, target, outer)
        }
        HirExpr::ThrowValue(error, fallback) => {
            let mut counts = thaw_rethrow_cleanup_scan_effect_expr(error, target, outer);
            thaw_rethrow_cleanup_merge_effect_counts(
                &mut counts,
                thaw_rethrow_cleanup_scan_effect_expr(fallback, target, outer),
            );
            counts
        }
        HirExpr::JsonAsNative(value, _) => {
            thaw_rethrow_cleanup_scan_effect_expr(value, target, outer)
        }
        HirExpr::ArraySetLen(array, length, _) => {
            let mut counts = thaw_rethrow_cleanup_scan_effect_expr(array, target, outer);
            thaw_rethrow_cleanup_merge_effect_counts(
                &mut counts,
                thaw_rethrow_cleanup_scan_effect_expr(length, target, outer),
            );
            counts
        }
        HirExpr::FunctionCallWithThis(callee, this_arg, args, _, _)
        | HirExpr::FunctionBindThis(callee, this_arg, args, _, _) => {
            let mut counts = thaw_rethrow_cleanup_scan_effect_expr(callee, target, outer);
            thaw_rethrow_cleanup_merge_effect_counts(
                &mut counts,
                thaw_rethrow_cleanup_scan_effect_expr(this_arg, target, outer),
            );
            for argument in args {
                thaw_rethrow_cleanup_merge_effect_counts(
                    &mut counts,
                    thaw_rethrow_cleanup_scan_effect_expr(argument, target, outer),
                );
            }
            counts
        }
        HirExpr::ObjectAlloc(_)
        | HirExpr::ClassAlloc(_)
        | HirExpr::EnvVar(_)
        | HirExpr::FunctionRef(..)
        | HirExpr::FunctionRefThis(..)
        | HirExpr::MethodRef(..)
        | HirExpr::OptionalNone(_)
        | HirExpr::NullableNone(_)
        | HirExpr::NullishNull(_)
        | HirExpr::NullishUndefined(_) => [0; 6],
        other => panic!("unreviewed HIR in bounded rethrow effect scan: {other:?}"),
    }
}

fn thaw_rethrow_cleanup_scan_effect_stmts(
    statements: &[HirStmt],
    target: &str,
    outer: &str,
) -> [usize; 6] {
    let mut counts = [0; 6];
    for statement in statements {
        let next = match statement {
            HirStmt::Expr(expression) | HirStmt::Throw(expression) => {
                thaw_rethrow_cleanup_scan_effect_expr(expression, target, outer)
            }
            HirStmt::Return(value) => value.as_ref().map_or([0; 6], |value| {
                thaw_rethrow_cleanup_scan_effect_expr(value, target, outer)
            }),
            HirStmt::Let(name, _, initializer) => {
                let mut next = [0; 6];
                next[1] += thaw_rethrow_cleanup_bool_count(name == target);
                next[5] += thaw_rethrow_cleanup_bool_count(name == outer);
                thaw_rethrow_cleanup_merge_effect_counts(
                    &mut next,
                    thaw_rethrow_cleanup_scan_effect_expr(initializer, target, outer),
                );
                next
            }
            HirStmt::If(condition, yes, no) => {
                let mut next = thaw_rethrow_cleanup_scan_effect_expr(condition, target, outer);
                thaw_rethrow_cleanup_merge_effect_counts(
                    &mut next,
                    thaw_rethrow_cleanup_scan_effect_stmts(yes, target, outer),
                );
                thaw_rethrow_cleanup_merge_effect_counts(
                    &mut next,
                    thaw_rethrow_cleanup_scan_effect_stmts(no, target, outer),
                );
                next
            }
            HirStmt::While(condition, body) => {
                let mut next = thaw_rethrow_cleanup_scan_effect_expr(condition, target, outer);
                thaw_rethrow_cleanup_merge_effect_counts(
                    &mut next,
                    thaw_rethrow_cleanup_scan_effect_stmts(body, target, outer),
                );
                next
            }
            HirStmt::Try(body, _, catch_body, _) => {
                let mut next = thaw_rethrow_cleanup_scan_effect_stmts(body, target, outer);
                thaw_rethrow_cleanup_merge_effect_counts(
                    &mut next,
                    thaw_rethrow_cleanup_scan_effect_stmts(catch_body, target, outer),
                );
                next
            }
            HirStmt::Finally(body, _) => {
                thaw_rethrow_cleanup_scan_effect_stmts(body, target, outer)
            }
            other => panic!("unreviewed statement in bounded rethrow effect scan: {other:?}"),
        };
        thaw_rethrow_cleanup_merge_effect_counts(&mut counts, next);
    }
    counts
}

fn thaw_rethrow_cleanup_guard_arm<'a>(
    guard: &'a HirStmt,
    thrown_name: &str,
    index: usize,
    members: &[HirType],
) -> &'a [HirStmt] {
    let HirStmt::If(condition, then_body, else_body) = guard else {
        panic!("expected one active-member guard at index {index}: {guard:?}");
    };
    let expected_condition = HirExpr::BinOp(
        BinOp::EqEqEq,
        Box::new(HirExpr::UnionTag(
            Box::new(HirExpr::Var(thrown_name.into())),
            members.to_vec(),
        )),
        Box::new(HirExpr::Lit(HirLit::F64(index as f64))),
    );
    assert_eq!(condition, &expected_condition, "wrong active-member guard at {index}");
    assert!(else_body.is_empty(), "unexpected else arm at carrier member {index}: {else_body:?}");
    then_body
}

fn thaw_rethrow_cleanup_member_value(
    thrown_name: &str,
    index: usize,
    members: &[HirType],
) -> HirExpr {
    HirExpr::UnionValue(
        Box::new(HirExpr::Var(thrown_name.into())),
        index,
        members.to_vec(),
    )
}

fn thaw_rethrow_cleanup_call_statement(name: &str, args: Vec<HirExpr>) -> HirStmt {
    HirStmt::Expr(HirExpr::Call(Box::new(HirExpr::Var(name.into())), args))
}

fn thaw_rethrow_cleanup_expected_member_typeof(
    index: usize,
    parameter: &str,
    members: &[HirType],
) -> HirExpr {
    let value = thaw_rethrow_cleanup_member_value(parameter, index, members);
    match (&members[index], index) {
        (HirType::F64, 0) => HirExpr::Lit(HirLit::Str("number".into())),
        (HirType::I64, 1) => HirExpr::Lit(HirLit::Str("bigint".into())),
        (HirType::Bool, 2) => HirExpr::Lit(HirLit::Str("boolean".into())),
        (HirType::Str, 3) => HirExpr::Lit(HirLit::Str("string".into())),
        (HirType::Undefined, 4) => HirExpr::Lit(HirLit::Str("undefined".into())),
        (HirType::Null, 5) | (HirType::Object(_), 6) => {
            HirExpr::Lit(HirLit::Str("object".into()))
        }
        (HirType::Json, 7) => HirExpr::Call(
            Box::new(HirExpr::Var("__thaw_json_typeof".into())),
            vec![value],
        ),
        (HirType::JsValue, 8) => HirExpr::JsonAsString(Box::new(HirExpr::Call(
            Box::new(HirExpr::Var("callDynamicValueWithValue".into())),
            vec![
                HirExpr::Call(
                    Box::new(HirExpr::Var("getDynamicValue".into())),
                    vec![HirExpr::Lit(HirLit::Str("__thaw_typeof_dynamic_value".into()))],
                ),
                value,
            ],
        ))),
        (HirType::NativeException, 9) => HirExpr::Call(
            Box::new(HirExpr::Var("__thaw_exception_typeof".into())),
            vec![HirExpr::Call(
                Box::new(HirExpr::Var("__thaw_native_exception_tag".into())),
                vec![value],
            )],
        ),
        other => panic!("unexpected carrier typeof member {other:?} at {index}"),
    }
}

fn thaw_rethrow_cleanup_assert_typeof_member_chain(
    statements: &[HirStmt],
    parameter: &str,
    members: &[HirType],
    index: usize,
) {
    let result = thaw_rethrow_cleanup_expected_member_typeof(index, parameter, members);
    let returned = [HirStmt::Return(Some(result))];
    if index + 1 == members.len() {
        assert_eq!(statements, returned, "last carrier typeof category must be the terminal return");
        return;
    }
    let [HirStmt::If(condition, yes, no)] = statements else {
        panic!("expected one guarded typeof category branch at {index}: {statements:#?}");
    };
    let expected_condition = HirExpr::BinOp(
        BinOp::EqEqEq,
        Box::new(HirExpr::UnionTag(
            Box::new(HirExpr::Var(parameter.into())),
            members.to_vec(),
        )),
        Box::new(HirExpr::Lit(HirLit::F64(index as f64))),
    );
    assert_eq!(condition, &expected_condition, "typeof wrapper routed the wrong carrier category at {index}");
    assert_eq!(yes, &returned, "typeof carrier member {index} returned the wrong category");
    assert!(!no.is_empty(), "typeof category chain ended before all carrier members");
    thaw_rethrow_cleanup_assert_typeof_member_chain(no, parameter, members, index + 1);
}

fn thaw_rethrow_cleanup_assert_number_typeof_condition(
    condition: &HirExpr,
    catch_name: &str,
    members: &[HirType],
) {
    // Strict equality binds both operands once:
    // ((left) => ((right) => left === right)("number"))(typeof ...).
    let HirExpr::Call(wrapper, wrapper_args) = condition else {
        panic!("expected strict equality between typeof result and number literal: {condition:#?}");
    };
    let HirExpr::Lambda(_, left, HirType::Bool, wrapper_body) = wrapper.as_ref() else {
        panic!("expected the strict-equality left binding lambda: {wrapper:#?}");
    };
    let HirExpr::Call(compare_lambda, compare_args) = wrapper_body.as_ref() else {
        panic!("expected the strict-equality right binding call: {wrapper_body:#?}");
    };
    assert_eq!(compare_args, &vec![HirExpr::Lit(HirLit::Str("number".into()))]);
    let HirExpr::Lambda(_, right, HirType::Bool, compare) = compare_lambda.as_ref() else {
        panic!("expected the strict-equality comparison lambda: {compare_lambda:#?}");
    };
    assert_eq!(compare.as_ref(), &HirExpr::BinOp(
        BinOp::EqEqEq,
        Box::new(HirExpr::Var(left[0].name.clone())),
        Box::new(HirExpr::Var(right[0].name.clone())),
    ));
    let [value_typeof] = wrapper_args.as_slice() else {
        panic!("strict equality binds exactly one left operand: {wrapper_args:#?}");
    };
    let HirExpr::Call(callee, args) = value_typeof else {
        panic!("expected the source-backed one-evaluation carrier typeof wrapper: {value_typeof:#?}");
    };
    let HirExpr::Lambda(captures, params, HirType::Str, body) = callee.as_ref() else {
        panic!("expected a string-returning typeof lambda: {callee:#?}");
    };
    assert!(captures.is_empty(), "typeof wrapper should not capture a different binding: {captures:#?}");
    assert_eq!(params.len(), 1);
    let parameter = &params[0];
    assert_eq!(parameter.ty, HirType::Union(members.to_vec()));
    assert_eq!(args, &vec![HirExpr::Var(catch_name.into())]);
    let HirExpr::Block(branches) = body.as_ref() else {
        panic!("expected guarded per-member typeof branch chain: {body:#?}");
    };
    thaw_rethrow_cleanup_assert_typeof_member_chain(branches, &parameter.name, members, 0);
}

fn thaw_rethrow_cleanup_expected_carrier_display(
    thrown_name: &str,
    members: &[HirType],
) -> HirExpr {
    let mut display = HirExpr::Lit(HirLit::Str("[exception value]".into()));
    for index in (0..members.len()).rev() {
        let tag = HirExpr::BinOp(
            BinOp::EqEqEq,
            Box::new(HirExpr::UnionTag(
                Box::new(HirExpr::Var(thrown_name.into())),
                members.to_vec(),
            )),
            Box::new(HirExpr::Lit(HirLit::F64(index as f64))),
        );
        let member_value = thaw_rethrow_cleanup_member_value(thrown_name, index, members);
        let selected = match (&members[index], index) {
            (HirType::F64, 0) => HirExpr::Call(
                Box::new(HirExpr::Var("__thaw_number_to_string".into())),
                vec![member_value],
            ),
            (HirType::I64, 1) => HirExpr::Call(
                Box::new(HirExpr::Var("__thaw_i64_to_string".into())),
                vec![member_value],
            ),
            (HirType::Bool, 2) => HirExpr::Call(
                Box::new(HirExpr::Var("__thaw_bool_to_string".into())),
                vec![member_value],
            ),
            (HirType::Str, 3) => member_value,
            (HirType::Undefined, 4) => HirExpr::Lit(HirLit::Str("undefined".into())),
            (HirType::Null, 5) => HirExpr::Lit(HirLit::Str("null".into())),
            (HirType::Object(_), 6)
            | (HirType::Json, 7)
            | (HirType::JsValue, 8)
            | (HirType::NativeException, 9) => {
                HirExpr::Lit(HirLit::Str("[exception value]".into()))
            }
            other => panic!("unexpected carrier display member {other:?} at {index}"),
        };
        display = HirExpr::Conditional(
            Box::new(tag),
            Box::new(selected),
            Box::new(display),
            HirType::Str,
        );
    }
    display
}

fn thaw_rethrow_cleanup_assert_projection_prelude(
    prelude: &HirExpr,
    thrown_name: &str,
    members: &[HirType],
    effect_name: &str,
) {
    let expected_member = thaw_rethrow_cleanup_member_value(thrown_name, 6, members);
    let HirExpr::Call(wrapper_callee, args) = prelude else {
        panic!("expected one registration wrapper call in the guarded object arm: {prelude:#?}");
    };
    assert_eq!(args, &vec![expected_member]);
    let HirExpr::Lambda(wrapper_captures, wrapper_params, wrapper_return, wrapper_body) = wrapper_callee.as_ref() else {
        panic!("expected Lambda-wrapped owner registration: {wrapper_callee:#?}");
    };
    assert!(wrapper_captures.iter().all(|capture| capture.name != thrown_name));
    let [owner] = wrapper_params.as_slice() else {
        panic!("expected one private native owner parameter: {wrapper_params:#?}");
    };
    let HirType::Object(_) = &owner.ty else {
        panic!("projection owner must retain its fixed Object type: {:?}", owner.ty);
    };
    assert_eq!(owner.ty, members[6]);
    assert!(owner.name.starts_with("__thaw_full_native_owner_"));
    assert_ne!(owner.name, thrown_name);
    assert_eq!(wrapper_return, &owner.ty, "owner wrapper must return its same typed owner");

    let HirExpr::EvalThen(registrations, returned_owner) = wrapper_body.as_ref() else {
        panic!("expected registration EvalThen followed by the owner return: {wrapper_body:#?}");
    };
    assert_eq!(returned_owner.as_ref(), &HirExpr::Var(owner.name.clone()));
    let HirExpr::EvalThen(layout_registration, projector_registration) = registrations.as_ref() else {
        panic!("expected layout registration before projector registration: {registrations:#?}");
    };
    let HirExpr::Call(layout_callee, layout_args) = layout_registration.as_ref() else {
        panic!("expected native layout registration call: {layout_registration:#?}");
    };
    assert_eq!(layout_callee.as_ref(), &HirExpr::Var("__thaw_register_native_object_layout".into()));
    assert_eq!(layout_args.len(), 2);
    assert_eq!(layout_args[0], HirExpr::Var(owner.name.clone()));
    assert!(matches!(&layout_args[1], HirExpr::Lit(HirLit::Bool(_))));

    let HirExpr::Call(projector_callee, projector_args) = projector_registration.as_ref() else {
        panic!("expected native projector registration call: {projector_registration:#?}");
    };
    assert_eq!(projector_callee.as_ref(), &HirExpr::Var("__thaw_register_native_object_projector".into()));
    assert_eq!(projector_args.len(), 3);
    assert_eq!(projector_args[0], HirExpr::Var(owner.name.clone()));
    assert!(matches!(&projector_args[1], HirExpr::Lit(HirLit::Str(_))));
    let HirExpr::Lambda(factory_captures, factory_params, HirType::JsValue, factory_body) = &projector_args[2] else {
        panic!("expected lazy JsValue projector factory: {:#?}", projector_args[2]);
    };
    assert!(factory_params.is_empty(), "projector factory has no ordinary parameters");
    assert_eq!(factory_captures.len(), 1, "factory captures only its private owner");
    assert_eq!(factory_captures[0].name.as_str(), owner.name.as_str());
    assert_eq!(factory_captures[0].ty, owner.ty);
    let HirExpr::Call(inner_callee, inner_args) = factory_body.as_ref() else {
        panic!("expected projector factory to pass its captured owner to the accessor builder: {factory_body:#?}");
    };
    assert_eq!(inner_args, &vec![HirExpr::Var(owner.name.clone())]);
    let HirExpr::Lambda(_, inner_params, HirType::JsValue, inner_body) = inner_callee.as_ref() else {
        panic!("expected the source-backed accessor-builder Lambda: {inner_callee:#?}");
    };
    assert!(matches!(inner_body.as_ref(), HirExpr::Block(_)));
    assert_eq!(inner_params.len(), 1);
    assert_eq!(inner_params[0].ty, owner.ty);

    // All registration/projection HIR inside the wrapper and lazy factory is traversed.
    // Unknown nodes fail closed; no outer carrier binding may be read, captured, declared,
    // or assigned from that private owner route.
    let counts = thaw_rethrow_cleanup_scan_effect_expr(wrapper_callee, effect_name, thrown_name);
    assert_eq!(counts, [0; 6], "projection prelude reached the effect input or outer thrown slot");
}

fn thaw_rethrow_cleanup_assert_carrier_publication(
    thrown_name: &str,
    tail: &[HirStmt],
    effect_name: &str,
) {
    let carrier = crate::caught_exception_carrier_type();
    let HirType::Union(members) = &carrier else {
        panic!("caught-exception carrier must be a union");
    };
    assert_eq!(tail.len(), 13, "carrier publication is Let, clear, ten guards, marker: {tail:#?}");
    let HirStmt::Let(name, ty, _) = &tail[0] else {
        panic!("expected carrier thrown-value Let: {tail:#?}");
    };
    assert_eq!(name, thrown_name);
    assert_eq!(ty, &carrier);
    assert!(matches!(&tail[1], HirStmt::Expr(HirExpr::Call(callee, args))
        if callee.as_ref() == &HirExpr::Var("__thaw_clear_pending_exception_provenance".into())
            && args.is_empty()));
    for index in 0..members.len() {
        let arm = thaw_rethrow_cleanup_guard_arm(&tail[2 + index], thrown_name, index, members);
        let value = thaw_rethrow_cleanup_member_value(thrown_name, index, members);
        match index {
            0 => assert_eq!(arm, &[thaw_rethrow_cleanup_call_statement(
                "__thaw_set_pending_exception_f64", vec![value],
            )]),
            1 => assert_eq!(arm, &[thaw_rethrow_cleanup_call_statement(
                "__thaw_set_pending_exception_i64", vec![value],
            )]),
            2 => assert_eq!(arm, &[thaw_rethrow_cleanup_call_statement(
                "__thaw_set_pending_exception_bool", vec![value],
            )]),
            3 => assert_eq!(arm, &[thaw_rethrow_cleanup_call_statement(
                "__thaw_set_pending_exception_tag", vec![HirExpr::Lit(HirLit::I64(4))],
            )]),
            4 => assert_eq!(arm, &[thaw_rethrow_cleanup_call_statement(
                "__thaw_set_pending_exception_tag", vec![HirExpr::Lit(HirLit::I64(5))],
            )]),
            5 => assert_eq!(arm, &[thaw_rethrow_cleanup_call_statement(
                "__thaw_set_pending_exception_tag", vec![HirExpr::Lit(HirLit::I64(6))],
            )]),
            6 => {
                // The one registration expression is allowed only inside the matched guard.
                let [HirStmt::Expr(prelude), object_setter] = arm else {
                    panic!("expected accounted projection prelude followed by setter only: {arm:#?}");
                };
                thaw_rethrow_cleanup_assert_projection_prelude(
                    prelude, thrown_name, members, effect_name,
                );
                assert_eq!(object_setter, &thaw_rethrow_cleanup_call_statement(
                    "__thaw_set_pending_exception_object", vec![value],
                ));
            }
            7 => assert_eq!(arm, &[thaw_rethrow_cleanup_call_statement(
                "__thaw_set_pending_exception_json", vec![value],
            )]),
            8 => assert_eq!(arm, &[thaw_rethrow_cleanup_call_statement(
                "__thaw_set_pending_exception_json",
                vec![HirExpr::Call(
                    Box::new(HirExpr::Var("__thaw_capture_exception_js_value".into())),
                    vec![value],
                )],
            )]),
            9 => assert_eq!(arm, &[thaw_rethrow_cleanup_call_statement(
                "__thaw_set_pending_exception_native_value", vec![value],
            )]),
            _ => unreachable!("carrier member count is pinned by the source type"),
        }
    }
    let Some(HirStmt::Throw(HirExpr::Call(callee, args))) = tail.last() else {
        unreachable!("publisher helper checked the final throw");
    };
    assert_eq!(callee.as_ref(), &HirExpr::Var("@@thaw_published_exception_text".into()));
    assert_eq!(args, &vec![thaw_rethrow_cleanup_expected_carrier_display(thrown_name, members)]);
}

#[test]
fn thaw_rethrow_cleanup_named_suffix_catches_publish_the_visible_carrier() {
    for source_name in ["error_object", "error_object_object"] {
        let source = format!(
            "function main(): void {{ try {{ throw 42; }} catch ({source_name}) {{ throw {source_name}; }} }}"
        );
        let program = lower(&source);
        let catches = thaw_rethrow_cleanup_collect_source_catches(&program.functions[0].body);
        assert_eq!(catches.len(), 1, "expected one structurally paired source catch");
        let (binding, catch_body) = catches[0];
        let input = HirExpr::Var(binding.to_string());
        let carrier = crate::caught_exception_carrier_type();
        let (thrown_name, tail) =
            thaw_rethrow_cleanup_find_publisher(catch_body, &[], &input, &carrier);
        thaw_rethrow_cleanup_assert_carrier_publication(
            thrown_name, tail, "thaw_rethrow_cleanup_effectful_input",
        );
    }
}

#[test]
fn thaw_rethrow_cleanup_unannotated_alias_keeps_and_publishes_the_carrier() {
    let program = lower(
        "function main(): void { try { throw 42; } catch (e) { const alias = e; throw alias; } }",
    );
    let catches = thaw_rethrow_cleanup_collect_source_catches(&program.functions[0].body);
    assert_eq!(catches.len(), 1);
    let (binding, catch_body) = catches[0];
    let carrier = crate::caught_exception_carrier_type();
    let aliases = catch_body.iter().filter_map(|statement| match statement {
        HirStmt::Let(name, ty, HirExpr::Var(initializer))
            if ty == &carrier && initializer == binding => Some(name.as_str()),
        _ => None,
    }).collect::<Vec<_>>();
    assert_eq!(aliases.len(), 1, "expected one same-carrier alias initialized from the catch binding");
    let alias = aliases[0];
    let expected_prefix = [HirStmt::Let(
        alias.into(), carrier.clone(), HirExpr::Var(binding.into()),
    )];
    let (thrown_name, tail) = thaw_rethrow_cleanup_find_publisher(
        catch_body,
        &expected_prefix,
        &HirExpr::Var(alias.into()),
        &carrier,
    );
    thaw_rethrow_cleanup_assert_carrier_publication(
        thrown_name, tail, "thaw_rethrow_cleanup_effectful_input",
    );
}

#[test]
fn thaw_rethrow_cleanup_mutation_retags_and_publishes_the_current_binding() {
    let program = lower(
        "function main(): void { try { throw 1; } catch (e) { e = 42; throw e; } }",
    );
    let catches = thaw_rethrow_cleanup_collect_source_catches(&program.functions[0].body);
    assert_eq!(catches.len(), 1);
    let (binding, catch_body) = catches[0];
    let carrier = crate::caught_exception_carrier_type();
    let HirType::Union(members) = &carrier else { panic!("expected carrier union"); };
    let assignments = catch_body.iter().filter_map(|statement| match statement {
        HirStmt::Expr(HirExpr::Assign(name, value)) => Some((name.as_str(), value.as_ref())),
        _ => None,
    }).collect::<Vec<_>>();
    assert_eq!(assignments.len(), 1, "expected one direct write to the visible catch binding");
    assert_eq!(assignments[0].0, binding);
    let injected = HirExpr::UnionInject(
        Box::new(HirExpr::Lit(HirLit::F64(42.0))),
        0,
        members.clone(),
    );
    assert_eq!(assignments[0].1, &injected);
    let expected_prefix = [HirStmt::Expr(HirExpr::Assign(
        binding.into(), Box::new(injected),
    ))];
    // After `e = 42` the binding is narrowed to its F64 member, so the rethrow
    // publishes that member read from the retagged carrier as a scalar.
    let (thrown_name, tail) = thaw_rethrow_cleanup_find_publisher(
        catch_body,
        &expected_prefix,
        &HirExpr::UnionValue(Box::new(HirExpr::Var(binding.into())), 0, members.clone()),
        &HirType::F64,
    );
    assert_eq!(tail.len(), 4, "{tail:?}");
    assert!(matches!(&tail[2], HirStmt::Expr(HirExpr::Call(callee, args))
        if callee.as_ref() == &HirExpr::Var("__thaw_set_pending_exception_f64".into())
            && args.as_slice() == [HirExpr::Var(thrown_name.into())]), "{tail:?}");
}

#[test]
fn thaw_rethrow_cleanup_nested_shadow_publishes_the_outer_carrier_alias() {
    let program = lower(
        "function main(): void { try { throw 1; } catch (e) { const alias = e; try { throw 2; } catch (e) { throw alias; } } }",
    );
    let (outer_binding, outer_body) =
        thaw_rethrow_cleanup_find_direct_source_catch(&program.functions[0].body);
    let (inner_binding, inner_body) =
        thaw_rethrow_cleanup_find_direct_source_catch(outer_body);
    assert_ne!(outer_binding, inner_binding, "same source name must resolve to distinct catch slots");
    let carrier = crate::caught_exception_carrier_type();
    let aliases = outer_body.iter().filter_map(|statement| match statement {
        HirStmt::Let(name, ty, HirExpr::Var(initializer))
            if ty == &carrier && initializer == outer_binding => Some(name.as_str()),
        _ => None,
    }).collect::<Vec<_>>();
    assert_eq!(aliases.len(), 1, "expected the outer alias to retain its source binding");
    let alias = aliases[0];
    let (thrown_name, tail) = thaw_rethrow_cleanup_find_publisher(
        inner_body,
        &[],
        &HirExpr::Var(alias.into()),
        &carrier,
    );
    thaw_rethrow_cleanup_assert_carrier_publication(
        thrown_name, tail, "thaw_rethrow_cleanup_effectful_input",
    );
}

#[test]
fn thaw_rethrow_cleanup_number_typeof_narrowing_uses_member_zero_f64() {
    let program = lower(
        "function main(): void { try { throw 1; } catch (e) { if (typeof e === \"number\") throw e; } }",
    );
    let catches = thaw_rethrow_cleanup_collect_source_catches(&program.functions[0].body);
    assert_eq!(catches.len(), 1);
    let (binding, catch_body) = catches[0];
    let carrier = crate::caught_exception_carrier_type();
    let HirType::Union(members) = &carrier else { panic!("expected carrier union"); };
    let [HirStmt::If(condition, yes, no)] = catch_body else {
        panic!("expected exactly the source typeof branch in the catch body: {catch_body:#?}");
    };
    assert!(no.is_empty(), "typeof fixture's alternate branch must be empty: {no:#?}");
    thaw_rethrow_cleanup_assert_number_typeof_condition(condition, binding, members);
    let selected = HirExpr::UnionValue(
        Box::new(HirExpr::Var(binding.into())),
        0,
        members.clone(),
    );
    let (thrown_name, tail) = thaw_rethrow_cleanup_find_publisher(
        yes,
        &[],
        &selected,
        &HirType::F64,
    );
    assert_eq!(tail.len(), 4, "scalar publication has Let, clear, setter, marker: {tail:#?}");
    assert_eq!(tail[2], thaw_rethrow_cleanup_call_statement(
        "__thaw_set_pending_exception_f64",
        vec![HirExpr::Var(thrown_name.into())],
    ));
    let Some(HirStmt::Throw(HirExpr::Call(_, display))) = tail.last() else {
        unreachable!("publisher finder checked the marker");
    };
    assert_eq!(display, &vec![HirExpr::Call(
        Box::new(HirExpr::Var("__thaw_number_to_string".into())),
        vec![HirExpr::Var(thrown_name.into())],
    )]);
}

#[test]
fn thaw_rethrow_cleanup_common_publisher_checks_all_carrier_arms_structurally() {
    let signatures = HashMap::new();
    let interfaces = HashMap::new();
    let generic_interfaces = GenericInterfaces::new();
    let enum_values = EnumValues::new();
    let enum_reverse_values = EnumReverseValues::new();
    let mut lowerer = FnLowerer::new(
        &signatures,
        &interfaces,
        &generic_interfaces,
        &enum_values,
        &enum_reverse_values,
        HirType::Void,
        None,
    );
    let carrier = crate::caught_exception_carrier_type();
    let effectful_input = HirExpr::Call(
        Box::new(HirExpr::Var("thaw_rethrow_cleanup_effectful_input".into())),
        Vec::new(),
    );
    let statements = lowerer
        .lower_throw_value_statements(effectful_input.clone(), carrier.clone())
        .expect("the existing carrier publisher accepts the carrier type");
    let (thrown_name, tail) = thaw_rethrow_cleanup_find_publisher(
        &statements,
        &[],
        &effectful_input,
        &carrier,
    );
    thaw_rethrow_cleanup_assert_carrier_publication(
        thrown_name, tail, "thaw_rethrow_cleanup_effectful_input",
    );
    let counts = thaw_rethrow_cleanup_scan_effect_stmts(
        &statements, "thaw_rethrow_cleanup_effectful_input", thrown_name,
    );
    assert_eq!(counts[0], 1, "effectful input call must occur exactly once in the complete helper output");
    assert_eq!(counts[1], 1, "effectful input function symbol must occur only as that call target");
    assert_eq!(counts[3], 0, "publisher may not assign the thrown-value local");
    assert_eq!(counts[4], 0, "publisher may not capture the thrown-value local");
    assert_eq!(counts[5], 1, "the only thrown-value local declaration is the initial Let");
}

#[test]
fn thaw_rethrow_cleanup_member_six_routes_original_pointer_to_object_setter() {
    let signatures = HashMap::new();
    let interfaces = HashMap::new();
    let generic_interfaces = GenericInterfaces::new();
    let enum_values = EnumValues::new();
    let enum_reverse_values = EnumReverseValues::new();
    let mut lowerer = FnLowerer::new(
        &signatures,
        &interfaces,
        &generic_interfaces,
        &enum_values,
        &enum_reverse_values,
        HirType::Void,
        None,
    );
    let carrier = crate::caught_exception_carrier_type();
    let HirType::Union(members) = &carrier else { panic!("expected carrier union"); };
    let original_pointer = HirExpr::Var("thaw_rethrow_cleanup_original_native_pointer".into());
    let member_six_input = HirExpr::UnionInject(
        Box::new(original_pointer.clone()),
        6,
        members.clone(),
    );
    let statements = lowerer
        .lower_throw_value_statements(member_six_input.clone(), carrier.clone())
        .expect("the existing carrier publisher accepts a member-six carrier");
    let (thrown_name, tail) = thaw_rethrow_cleanup_find_publisher(
        &statements,
        &[],
        &member_six_input,
        &carrier,
    );
    thaw_rethrow_cleanup_assert_carrier_publication(
        thrown_name, tail, "thaw_rethrow_cleanup_effectful_input",
    );
    let counts = thaw_rethrow_cleanup_scan_effect_stmts(
        &statements, "thaw_rethrow_cleanup_effectful_input", thrown_name,
    );
    assert_eq!(counts[0], 0);
    assert_eq!(counts[1], 0);
    assert_eq!(counts[3], 0);
    assert_eq!(counts[4], 0);
    assert_eq!(counts[5], 1);
    let HirStmt::Let(_, ty, initializer) = &tail[0] else {
        panic!("expected carrier thrown-value Let");
    };
    assert_eq!(ty, &carrier);
    assert_eq!(initializer, &HirExpr::UnionInject(
        Box::new(original_pointer),
        6,
        members.clone(),
    ));
}

#[test]
fn anonymous_catch_preserves_outer_underscore_binding() {
    let program = lower(r#"function main(): string {
        const _ = "outer";
        try { throw "first"; } catch { console.log(_); }
        try { throw "second"; } catch { return _; }
        return _;
    }"#);
    let body = &program.functions[0].body;
    let HirStmt::Let(outer, _, _) = &body[0] else { panic!("expected outer binding"); };
    let mut catch_names = Vec::new();
    // Each catch is now preceded by its carrier `Let`, so select the `Try` statements.
    let tries = body.iter().filter(|statement| matches!(statement, HirStmt::Try(..))).collect::<Vec<_>>();
    assert_eq!(tries.len(), 2, "{body:?}");
    for statement in tries {
        let HirStmt::Try(_, name, catch_body, _) = statement else { panic!("expected catch"); };
        assert!(name.starts_with("@@thaw_anonymous_catch"), "{name}");
        assert_ne!(name, outer);
        assert!(format!("{catch_body:?}").contains(&format!("Var(\"{outer}\")")), "{catch_body:?}");
        catch_names.push(name);
    }
    assert_ne!(catch_names[0], catch_names[1]);
}


// --- native catch carrier: casts and Json projection (tags 20-29) ---

#[test]
fn caught_native_value_cast_lowers_for_each_native_category() {
    // `error as T` on a caught native value builds an unreachable placeholder of `T` for the
    // incompatible-category branch; every category must produce one (never "cannot synthesize").
    let mut problems = Vec::new();
    for (thrown, target) in [
        ("new Map<string, number>()", "Map<string, number>"),
        ("new Set<number>()", "Set<number>"),
        ("new Uint8Array(2)", "Uint8Array"),
        ("Promise.resolve(1)", "Promise<number>"),
    ] {
        let source = format!(
            "function main(): void {{ try {{ throw {thrown}; }} catch (error) {{ const value = error as {target}; console.log(value); }} }}"
        );
        let module = thaw_parser::parse_typescript(&source).expect("parse error");
        match lower_module(&module) {
            Err(error) => problems.push(format!("`{target}`: lowering failed: {error}")),
            Ok(program) => {
                let text = format!("{:?}", program.functions[0].body);
                if !text.contains("__thaw_native_exception_owner") || !text.contains("incompatible category or layout") {
                    problems.push(format!("`{target}`: not cast through its native owner"));
                }
            }
        }
    }
    assert!(problems.is_empty(), "{problems:#?}");
}

#[test]
fn caught_native_exception_json_projection_is_tag_driven() {
    let adapter = format!("{:?}", crate::caught_exception_json_adapter());
    assert!(adapter.contains("__thaw_caught_adapter_tag"), "{adapter}");
    assert!(adapter.contains("__thaw_native_exception_tag"), "{adapter}");
    // Symbol and Function project to undefined; Map/Set/WeakMap/WeakSet/Promise to `{}`.
    for tag in [20, 28, 23, 24, 25, 26, 29] {
        assert!(adapter.contains(&format!("I64({tag})")), "tag {tag} has no projection arm: {adapter}");
    }
    // Array/Bytes/Tuple contents would need a snapshot: no arm, so they hit the explicit error.
    for tag in [21, 22, 27] {
        assert!(!adapter.contains(&format!("I64({tag})")), "tag {tag} must not be projected: {adapter}");
    }
    assert!(adapter.contains("caught native value has no Json projection"), "{adapter}");
    assert!(adapter.contains("__thaw_json_object_from_json_entries"), "{adapter}");
}

#[test]
fn unreachable_placeholders_exist_for_every_native_value_type() {
    // An object-literal getter's declared return type gets an unreachable placeholder
    // (`unreachable_value`); each type below used to fail with "cannot synthesize".
    let mut problems = Vec::new();
    for (ty, value) in [
        ("bigint", "1n"),
        ("symbol", "Symbol(\"s\")"),
        ("Map<string, number>", "new Map<string, number>()"),
        ("Set<number>", "new Set<number>()"),
        ("Promise<number>", "Promise.resolve(1)"),
        ("Promise<void>", "Promise.resolve()"),
        ("[number, string]", "[1, \"a\"]"),
        ("() => number", "(): number => 1"),
    ] {
        let source = format!(
            "function main(): void {{ const holder = {{ get value(): {ty} {{ return {value}; }} }}; console.log(holder.value); }}"
        );
        let module = thaw_parser::parse_typescript(&source).expect("parse error");
        if let Err(error) = lower_module(&module) {
            problems.push(format!("`{ty}`: {error}"));
        }
    }
    // A setter's parameter type gets one too; its empty-ish body has no return check, which
    // `Uint8Array` and equal-member tuples would otherwise trip over (they infer as `Array(F64)`).
    for ty in ["Uint8Array", "[number, number]", "symbol", "Map<string, number>", "Promise<number>", "WeakMap<object, number>", "WeakSet<object>"] {
        let source = format!(
            "function main(): void {{ const holder = {{ set value(next: {ty}) {{ console.log(next); }} }}; }}"
        );
        let module = thaw_parser::parse_typescript(&source).expect("parse error");
        if let Err(error) = lower_module(&module) {
            problems.push(format!("setter `{ty}`: {error}"));
        }
    }
    assert!(problems.is_empty(), "{problems:#?}");
}

