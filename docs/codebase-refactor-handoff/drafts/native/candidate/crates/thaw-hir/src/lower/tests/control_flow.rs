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
    assert!(text.contains("@@thaw_trusted_exception_text"), "{text}");
    assert!(text.contains("boom"), "{text}");
    assert!(text.contains("console.log"), "{text}");
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
    let HirStmt::Try(body, _, catch_body, _) = &program.functions[0].body[0] else {
        panic!("expected lowered try");
    };
    assert!(matches!(body.as_slice(), [HirStmt::Let(..), HirStmt::Finally(cleanup, 1), HirStmt::Return(Some(HirExpr::Var(_)))]
        if matches!(cleanup.as_slice(), [HirStmt::Expr(_)])), "{body:?}");
    let [HirStmt::Try(guarded, _, failure, _)] = catch_body.as_slice() else {
        panic!("expected one synthetic source-catch guard: {catch_body:?}");
    };
    assert!(matches!(guarded.as_slice(),
        [.., HirStmt::Try(captured, _, failed, _), HirStmt::Finally(_, 1), HirStmt::Finally(rethrow, 1)]
            if matches!(captured.as_slice(), [HirStmt::Let(..)])
                && matches!(failed.as_slice(), [HirStmt::Finally(_, 1), HirStmt::Finally(throw, 1)]
                    if matches!(throw.as_slice(), [HirStmt::Throw(_)]))
                && matches!(rethrow.as_slice(), [HirStmt::Throw(_)])), "{guarded:?}");
    assert!(matches!(failure.as_slice(), [HirStmt::Let(..), HirStmt::Finally(_, 0), HirStmt::Throw(_)]), "{failure:?}");
    assert!(matches!(program.functions[0].body[1], HirStmt::Expr(_)));
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
    let HirStmt::Try(return_body, _, _, _) = &returned.body[1] else {
        panic!("expected return try");
    };
    assert!(matches!(return_body.as_slice(),
        [HirStmt::Let(_, _, HirExpr::Var(_)), HirStmt::Finally(_, 1), HirStmt::Return(Some(HirExpr::Var(_)))]),
        "{return_body:?}");
    let HirStmt::Try(_, _, throw_body, _) = &thrown.body[1] else {
        panic!("expected throw try");
    };
    assert!(matches!(throw_body.as_slice(), [HirStmt::Try(guarded, _, failure, _)]
        if matches!(guarded.as_slice(),
            [.., HirStmt::Try(captured, _, failed, _), HirStmt::Finally(_, 1), HirStmt::Finally(rethrow, 1)]
                if captured.len() == 7 && captured.iter().all(|stmt| matches!(stmt, HirStmt::Let(..)))
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
    let HirStmt::Try(_, _, catch_body, _) = &choose.body[0] else {
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
    let HirStmt::Try(_, _, catch_body, _) = &keep.body[0] else {
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
    let HirStmt::Try(body, _, _, _) = &chosen.body[1] else {
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
    let HirStmt::Try(_, catch_name, catch_body, _) = &body[1] else {
        panic!("expected lowered try");
    };
    assert_eq!(catch_name, "error__thaw_local_0");
    assert!(format!("{:?}", catch_body).contains("error__thaw_local_0"));
    assert!(format!("{:?}", body[2]).contains("Var(\"error\")"));
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
        assert_eq!(native_exception_tag(&ty), Some(expected_tag));
        assert!(native_exception_layout_token(&ty).is_some());
    }
    assert_ne!(
        native_exception_layout_token(&HirType::Array(Box::new(HirType::F64))),
        native_exception_layout_token(&HirType::Array(Box::new(HirType::Str))),
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

#[test]
fn rethrow_tag_preserves_object_text_in_catch_binding() {
    for source_name in ["error_object", "error_object_object"] {
        let source = format!("function main(): void {{ try {{ throw 42; }} catch ({source_name}) {{ throw {source_name}; }} }}");
        let program = lower(&source);
        let HirStmt::Try(_, binding, catch_body, _) = &program.functions[0].body[0] else {
            panic!("expected catch");
        };
        let body = format!("{catch_body:?}");
        assert!(body.contains(&format!("Var(\"{binding}__thaw_exception_object\")")), "{body}");
        assert!(body.contains(&format!("Var(\"{binding}__thaw_exception_tag\")")), "{body}");
    }
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
    for statement in &body[1..3] {
        let HirStmt::Try(_, name, catch_body, _) = statement else { panic!("expected catch"); };
        assert!(name.starts_with("@@thaw_anonymous_catch"), "{name}");
        assert_ne!(name, outer);
        assert!(format!("{catch_body:?}").contains(&format!("Var(\"{outer}\")")), "{catch_body:?}");
        catch_names.push(name);
    }
    assert_ne!(catch_names[0], catch_names[1]);
}
